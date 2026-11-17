// src/ingest/basketball.rs
//! Koszykówka: NBA z BALLDONTLIE (wymaga klucza API) oraz opcjonalnie Euroliga z TheSportsDB.

//TODO: IMPLEMENT NBA_API FOR NOW ONLY BALLDONTLIE IS USED

use std::time::Duration;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    fetch_text, full_season_label, season_start_year, store_matches, winner_from_scores,
    IngestedMatch, IngestedParticipant,
};
use crate::{
    error::{AppError, AppResult},
    models::match_model::MatchDetails,
    state::AppState,
};

/// Przerwa między zapytaniami do BALLDONTLIE – darmowy klucz pozwala na 5 zapytań na minutę.
const BALLDONTLIE_REQUEST_DELAY: Duration = Duration::from_secs(12);

/// Liczba kwart w regulaminowym czasie gry; `period` powyżej tej wartości oznacza dogrywkę.
const REGULATION_PERIODS: i32 = 4;

// ---------------------------------------------------------------------------
// BALLDONTLIE (NBA)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BalldontliePage {
    data: Vec<BalldontlieGame>,
    meta: Option<BalldontlieMeta>,
}

#[derive(Debug, Deserialize)]
struct BalldontlieMeta {
    /// Kursor następnej strony; brak oznacza koniec danych.
    next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct BalldontlieGame {
    id: i64,
    date: String,
    datetime: Option<String>,
    status: String,
    period: Option<i32>,
    home_team_score: Option<i32>,
    visitor_team_score: Option<i32>,
    home_team: BalldontlieTeam,
    visitor_team: BalldontlieTeam,
}

#[derive(Debug, Deserialize)]
struct BalldontlieTeam {
    full_name: String,
    abbreviation: Option<String>,
    conference: Option<String>,
}

impl BalldontlieTeam {
    fn into_participant(self) -> IngestedParticipant {
        let mut participant = IngestedParticipant::new(self.full_name, self.abbreviation);
        if let Some(conference) = self.conference {
            participant.metadata = json!({ "conference": conference });
        }
        participant
    }
}

/// Mapuje pole `status` BALLDONTLIE: zakończone mecze mają `Final`, niezaczęte `period` równe 0.
fn map_balldontlie_status(status: &str, period: Option<i32>) -> &'static str {
    let lowered = status.to_ascii_lowercase();
    if lowered == "final" {
        "FINISHED"
    } else if lowered.contains("postponed") {
        "POSTPONED"
    } else if lowered.contains("cancel") {
        "CANCELLED"
    } else if period.unwrap_or(0) == 0 {
        "SCHEDULED"
    } else {
        "LIVE"
    }
}

/// Parsuje jedną stronę odpowiedzi `/v1/games` i zwraca mecze oraz kursor następnej strony.
pub fn parse_balldontlie_page(json: &str) -> AppResult<(Vec<IngestedMatch>, Option<i64>)> {
    let page: BalldontliePage = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("BALLDONTLIE: {}", e)))?;
    let next_cursor = page.meta.and_then(|m| m.next_cursor);

    let mut result = Vec::with_capacity(page.data.len());
    for game in page.data {
        // Dokładny czas startu jest w `datetime`; gdy go brak, zostaje sama data (północ UTC).
        let match_date = game
            .datetime
            .as_deref()
            .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
            .map(|d| d.with_timezone(&Utc))
            .or_else(|| {
                game.date
                    .get(..10)
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .map(|d| d.and_time(NaiveTime::MIN).and_utc())
            });
        let Some(match_date) = match_date else {
            continue;
        };

        let status = map_balldontlie_status(&game.status, game.period);
        let finished = status == "FINISHED";
        let scores = if finished {
            game.home_team_score.zip(game.visitor_team_score)
        } else {
            None
        };

        // Sezon NBA zapisany rokiem startu (2024) odpowiada etykiecie 2024-2025.
        let season_start = season_start_year(match_date);

        result.push(IngestedMatch {
            sport: "BASKETBALL",
            competition_code: "NBA".to_string(),
            competition_name: "NBA".to_string(),
            competition_country: Some("USA".to_string()),
            season: full_season_label(season_start),
            match_date,
            status: status.to_string(),
            home: game.home_team.into_participant(),
            away: game.visitor_team.into_participant(),
            home_score: scores.map(|(h, _)| h),
            away_score: scores.map(|(_, a)| a),
            winner: scores
                .and_then(|(h, a)| winner_from_scores(h, a, false))
                .map(str::to_string),
            details: MatchDetails::Basketball {
                // API nie udostępnia punktów w poszczególnych kwartach.
                quarters: vec![],
                overtime: finished && game.period.unwrap_or(0) > REGULATION_PERIODS,
            },
            external_ids: json!({ "balldontlie": game.id }),
        });
    }
    Ok((result, next_cursor))
}

/// Pobiera i zapisuje wszystkie strony meczów jednego sezonu NBA (strona po stronie).
async fn sync_nba_season(state: &AppState, key: &str, season_start: i32) -> AppResult<usize> {
    let mut total = 0;
    let mut cursor: Option<i64> = None;

    loop {
        let mut url = format!(
            "https://api.balldontlie.io/v1/games?seasons[]={}&per_page=100",
            season_start
        );
        if let Some(c) = cursor {
            url.push_str(&format!("&cursor={}", c));
        }

        let body = fetch_text(&state.http_client, &url, &[("Authorization", key)]).await?;
        let (matches, next_cursor) = parse_balldontlie_page(&body)?;
        total += store_matches(&state.db, &matches).await?;

        match next_cursor {
            Some(next) => {
                cursor = Some(next);
                tokio::time::sleep(BALLDONTLIE_REQUEST_DELAY).await;
            }
            None => break,
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// TheSportsDB (Euroliga)
// ---------------------------------------------------------------------------

/// Zamienia wartość JSON (liczbę lub napis, bo API bywa niespójne) na `i32`.
fn json_to_i32(value: &Value) -> Option<i32> {
    value
        .as_i64()
        .map(|n| n as i32)
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Parsuje odpowiedź `eventsseason.php`. Pole `events` bywa `null`, gdy sezon nie ma meczów.
pub fn parse_thesportsdb_events(json: &str, season: &str) -> AppResult<Vec<IngestedMatch>> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("TheSportsDB: {}", e)))?;

    let mut result = Vec::new();
    for event in root["events"].as_array().into_iter().flatten() {
        let (Some(home), Some(away)) =
            (event["strHomeTeam"].as_str(), event["strAwayTeam"].as_str())
        else {
            continue;
        };

        // Preferujemy pełny znacznik czasu (UTC), a w razie jego braku składamy datę i godzinę.
        let match_date = event["strTimestamp"]
            .as_str()
            .and_then(|t| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S").ok())
            .map(|t| t.and_utc())
            .or_else(|| {
                let date = event["dateEvent"]
                    .as_str()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())?;
                let time = event["strTime"]
                    .as_str()
                    .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M:%S").ok())
                    .unwrap_or(NaiveTime::MIN);
                Some(date.and_time(time).and_utc())
            });
        let Some(match_date) = match_date else {
            continue;
        };

        let scores = json_to_i32(&event["intHomeScore"]).zip(json_to_i32(&event["intAwayScore"]));
        let postponed = event["strPostponed"]
            .as_str()
            .is_some_and(|p| p.eq_ignore_ascii_case("yes"));
        let status = if postponed {
            "POSTPONED"
        } else if scores.is_some() {
            "FINISHED"
        } else {
            "SCHEDULED"
        };
        let scores = if status == "FINISHED" { scores } else { None };

        result.push(IngestedMatch {
            sport: "BASKETBALL",
            competition_code: "EUROLEAGUE".to_string(),
            competition_name: "Euroleague".to_string(),
            competition_country: Some("Europe".to_string()),
            season: season.to_string(),
            match_date,
            status: status.to_string(),
            home: IngestedParticipant::new(home, None),
            away: IngestedParticipant::new(away, None),
            home_score: scores.map(|(h, _)| h),
            away_score: scores.map(|(_, a)| a),
            winner: scores
                .and_then(|(h, a)| winner_from_scores(h, a, false))
                .map(str::to_string),
            details: MatchDetails::Basketball {
                quarters: vec![],
                overtime: false,
            },
            external_ids: json!({ "thesportsdb": event["idEvent"] }),
        });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Synchronizacja
// ---------------------------------------------------------------------------

/// Importuje koszykówkę: NBA (gdy ustawiono `BALLDONTLIE_API_KEY`) i Euroligę (gdy podano
/// `THESPORTSDB_EUROLEAGUE_ID`). Sezon bieżący jest pobierany zawsze, historyczne tylko na żądanie.
pub async fn sync(state: &AppState, include_history: bool) -> AppResult<usize> {
    let current = season_start_year(Utc::now());
    let mut seasons: Vec<i32> = Vec::new();
    if include_history {
        seasons = if state.config.ingest_nba_seasons.is_empty() {
            vec![current - 2, current - 1]
        } else {
            state.config.ingest_nba_seasons.clone()
        };
    }
    if !seasons.contains(&current) {
        seasons.push(current);
    }

    let mut total = 0;

    match &state.config.balldontlie_api_key {
        Some(key) => {
            for season in &seasons {
                // Błąd jednego sezonu (np. limit zapytań) nie blokuje kolejnych.
                match sync_nba_season(state, key, *season).await {
                    Ok(count) => {
                        tracing::info!(season, count, "BALLDONTLIE: zapisano mecze");
                        total += count;
                    }
                    Err(err) => tracing::warn!(season, "BALLDONTLIE: pominięto sezon: {}", err),
                }
            }
        }
        None => tracing::info!("Brak BALLDONTLIE_API_KEY – pomijam NBA"),
    }

    if let Some(league_id) = &state.config.thesportsdb_euroleague_id {
        for season in &seasons {
            let label = full_season_label(*season);
            let url = format!(
                "https://www.thesportsdb.com/api/v1/json/123/eventsseason.php?id={}&s={}",
                league_id, label
            );
            let result = async {
                let body = fetch_text(&state.http_client, &url, &[]).await?;
                let matches = parse_thesportsdb_events(&body, &label)?;
                store_matches(&state.db, &matches).await
            }
            .await;

            match result {
                Ok(count) => {
                    tracing::info!(season = %label, count, "TheSportsDB: zapisano mecze");
                    total += count;
                }
                Err(err) => {
                    tracing::warn!(season = %label, "TheSportsDB: pominięto sezon: {}", err)
                }
            }
        }
    }

    Ok(total)
}
