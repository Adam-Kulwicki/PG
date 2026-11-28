// src/ingest/football/mod.rs
//! Piłka nożna (Top 5 lig Europy):
//! - **`football-data.co.uk`** — historia i bieżący sezon CSV (xG, strzały, strzały celne, rożne, faule, kartki, HT) oraz fallback terminarza `fixtures.csv`,
//! - **Football-Data.org v4** *(opcjonalnie z kluczem API)* — dokładne `utcDate`, statusy `LIVE`/`POSTPONED`/`CANCELLED` i numery kolejek,
//! - **`clubelo.com`** — przedmeczowe ratingi Elo (`D - 1`), prawdopodobieństwa `/Fixtures` dla meczów `SCHEDULED` oraz skalibrowany backfill archiwalny `EloRatings.csv`.

pub mod clubelo;
pub mod clubelo_archive;
pub mod football_data_uk;
pub mod teams;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

pub use super::normalize_season_label;
use super::{
    fetch_text, season_start_year, store_matches, winner_from_scores, IngestedMatch,
    IngestedParticipant,
};
use crate::{
    config::FootballIngestSource,
    error::{AppError, AppResult},
    models::match_model::MatchDetails,
    state::AppState,
};
pub use clubelo::{
    build_pre_match_elo, enrich_matches_with_clubelo, lookup_pre_match_elo_point,
    parse_clubelo_fixtures_html, parse_clubelo_home_rankings, parse_clubelo_team_page_history,
    resolve_historical_team_elo, ClubEloPoint,
};
pub use clubelo_archive::{
    collect_live_calibration_samples, compute_league_calibration_offsets,
    load_or_fetch_clubelo_archive, median, parse_clubelo_archive_csv, ClubEloArchiveMap,
    CLUBELO_ARCHIVE_CACHE_PATH, CLUBELO_ARCHIVE_MIRROR_URL, CLUBELO_ARCHIVE_PRIMARY_URL,
};
pub use football_data_uk::{
    football_data_uk_season_code, football_data_uk_season_url, parse_football_data_uk_csv,
    parse_football_data_uk_fixtures_csv, parse_football_season_year,
};
pub use teams::{
    clubelo_slug, league_spec, normalize_football_team_name, LeagueSpec, TOP5_LEAGUES,
};

// ---------------------------------------------------------------------------
// Football-Data.org (sezon bieżący)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FootballDataResponse {
    matches: Vec<FootballDataMatch>,
}

#[derive(Debug, Deserialize)]
struct FootballDataMatch {
    id: i64,
    #[serde(rename = "utcDate")]
    utc_date: String,
    status: String,
    matchday: Option<i32>,
    season: FootballDataSeason,
    #[serde(rename = "homeTeam")]
    home_team: FootballDataTeam,
    #[serde(rename = "awayTeam")]
    away_team: FootballDataTeam,
    score: FootballDataScore,
}

#[derive(Debug, Deserialize)]
struct FootballDataSeason {
    #[serde(rename = "startDate")]
    start_date: String,
    #[serde(rename = "endDate")]
    end_date: String,
}

/// Drużyna nierozstrzygniętego meczu (np. puchar) może nie mieć jeszcze nazwy.
#[derive(Debug, Deserialize)]
struct FootballDataTeam {
    name: Option<String>,
    tla: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FootballDataScore {
    winner: Option<String>,
    #[serde(rename = "fullTime")]
    full_time: Option<FootballDataGoals>,
    #[serde(rename = "halfTime")]
    half_time: Option<FootballDataGoals>,
}

#[derive(Debug, Deserialize)]
struct FootballDataGoals {
    home: Option<i32>,
    away: Option<i32>,
}

/// Mapuje status Football-Data.org na statusy używane w bazie.
fn map_football_data_status(status: &str) -> &'static str {
    match status {
        "FINISHED" | "AWARDED" => "FINISHED",
        "IN_PLAY" | "PAUSED" => "LIVE",
        "POSTPONED" | "SUSPENDED" => "POSTPONED",
        "CANCELLED" => "CANCELLED",
        _ => "SCHEDULED",
    }
}

/// Parsuje odpowiedź `/v4/competitions/PL/matches` (zachowane dla kompatybilności testów).
pub fn parse_football_data(json: &str) -> AppResult<Vec<IngestedMatch>> {
    parse_football_data_for_league(json, &TOP5_LEAGUES[0])
}

/// Parsuje odpowiedź `/v4/competitions/{CODE}/matches` dla wskazanej ligi z [`TOP5_LEAGUES`].
/// Wynik zapisujemy tylko dla zakończonych meczów, żeby trwający mecz nie wyglądał na rozstrzygnięty.
pub fn parse_football_data_for_league(
    json: &str,
    spec: &LeagueSpec,
) -> AppResult<Vec<IngestedMatch>> {
    let response: FootballDataResponse = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("Football-Data ({}): {}", spec.code, e)))?;

    let mut result = Vec::with_capacity(response.matches.len());
    for m in response.matches {
        let (Some(home_name), Some(away_name)) = (m.home_team.name, m.away_team.name) else {
            continue;
        };
        let Ok(match_date) = DateTime::parse_from_rfc3339(&m.utc_date) else {
            continue;
        };

        let start_year = m
            .season
            .start_date
            .get(..4)
            .and_then(|y| y.parse::<i32>().ok());
        let end_year = m
            .season
            .end_date
            .get(..4)
            .and_then(|y| y.parse::<i32>().ok());
        let (Some(start_year), Some(end_year)) = (start_year, end_year) else {
            continue;
        };

        let status = map_football_data_status(&m.status);
        let finished = status == "FINISHED";
        let goals = |g: &Option<FootballDataGoals>| g.as_ref().and_then(|g| g.home.zip(g.away));
        let full_time = if finished {
            goals(&m.score.full_time)
        } else {
            None
        };
        let half_time = if finished {
            goals(&m.score.half_time)
        } else {
            None
        };

        // Zwycięzcę bierzemy z API, a gdy go brak – liczymy z wyniku.
        let winner = if finished {
            match m.score.winner.as_deref() {
                Some("HOME_TEAM") => Some("HOME"),
                Some("AWAY_TEAM") => Some("AWAY"),
                Some("DRAW") => Some("DRAW"),
                _ => full_time.and_then(|(h, a)| winner_from_scores(h, a, true)),
            }
        } else {
            None
        };

        let home_canonical = normalize_football_team_name(&home_name).to_string();
        let away_canonical = normalize_football_team_name(&away_name).to_string();

        result.push(IngestedMatch {
            sport: "FOOTBALL",
            competition_code: spec.code.to_string(),
            competition_name: spec.name.to_string(),
            competition_country: Some(spec.country.to_string()),
            season: format!("{}-{}", start_year, end_year),
            match_date: match_date.with_timezone(&Utc),
            status: status.to_string(),
            home: IngestedParticipant::new(home_canonical, m.home_team.tla),
            away: IngestedParticipant::new(away_canonical, m.away_team.tla),
            home_score: full_time.map(|(h, _)| h),
            away_score: full_time.map(|(_, a)| a),
            winner: winner.map(str::to_string),
            details: MatchDetails::Football {
                ht_home: half_time.map(|(h, _)| h),
                ht_away: half_time.map(|(_, a)| a),
                round: m
                    .matchday
                    .map(|d| format!("Matchday {}", d))
                    .unwrap_or_default(),
                home_stats: None,
                away_stats: None,
                pre_match_elo: None,
            },
            external_ids: json!({ "football_data": m.id }),
        });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Synchronizacja
// ---------------------------------------------------------------------------

fn configured_leagues(raw: &[String]) -> Vec<&'static LeagueSpec> {
    let mut specs = Vec::new();
    for code in raw {
        match league_spec(code) {
            Some(spec) if !specs.contains(&spec) => specs.push(spec),
            Some(_) => {}
            None => tracing::warn!(league = %code, "Nieznany kod ligi w INGEST_FOOTBALL_LEAGUES"),
        }
    }
    if specs.is_empty() {
        specs.push(&TOP5_LEAGUES[0]);
    }
    specs
}

fn configured_history_years(raw: &[String], current: i32) -> Vec<i32> {
    let candidates: Vec<i32> = if raw.is_empty() {
        vec![current - 2, current - 1]
    } else {
        raw.iter()
            .filter_map(|s| {
                let parsed = parse_football_season_year(s);
                if parsed.is_none() {
                    tracing::warn!(season = %s, "Pominięto niepoprawny format INGEST_FOOTBALL_SEASONS");
                }
                parsed
            })
            .collect()
    };

    let mut years = Vec::new();
    for y in candidates {
        if y != current && !years.contains(&y) {
            years.push(y);
        }
    }
    years
}

/// Importuje piłkę nożną wg `INGEST_FOOTBALL_SOURCE`:
/// 1) `football-data.co.uk` (historia, statystyki bieżącego sezonu i opcjonalnie `fixtures.csv`),
/// 2) Football-Data.org (jeśli ustawiono `FOOTBALL_DATA_API_KEY`),
/// 3) `clubelo.com` (wzbogacenie pre-match Elo z dnia `D - 1` i `/Fixtures`).
pub async fn sync(state: &AppState, include_history: bool) -> AppResult<usize> {
    let current = season_start_year(Utc::now());
    let leagues = configured_leagues(&state.config.ingest_football_leagues);
    let history_years = if include_history {
        configured_history_years(&state.config.ingest_football_seasons, current)
    } else {
        Vec::new()
    };

    let mut total = 0;
    let source = state.config.ingest_football_source;

    // 1. football-data.co.uk (Auto lub FootballDataUk)
    if matches!(
        source,
        FootballIngestSource::Auto | FootballIngestSource::FootballDataUk
    ) {
        let mut seasons = history_years;
        seasons.push(current);

        let mut sent_any = false;
        for spec in &leagues {
            for &year in &seasons {
                if sent_any {
                    tokio::time::sleep(football_data_uk::FDUK_REQUEST_DELAY).await;
                }
                sent_any = true;

                let result = async {
                    let matches =
                        football_data_uk::fetch_league_season(&state.http_client, spec, year)
                            .await?;
                    store_matches(&state.db, &matches).await
                }
                .await;

                match result {
                    Ok(count) => {
                        tracing::info!(
                            league = spec.code,
                            season = year,
                            count,
                            "football-data.co.uk: zapisano mecze"
                        );
                        total += count;
                    }
                    Err(err) => tracing::warn!(
                        league = spec.code,
                        season = year,
                        "football-data.co.uk: pominięto sezon: {}",
                        err
                    ),
                }
            }
        }

        // Gdy nie mamy klucza Football-Data.org (albo wymuszono tryb CSV), pobieramy też terminarz.
        if state.config.football_data_api_key.is_none()
            || source == FootballIngestSource::FootballDataUk
        {
            match football_data_uk::fetch_fixtures(&state.http_client, &leagues, current).await {
                Ok(fixtures) => match store_matches(&state.db, &fixtures).await {
                    Ok(count) => {
                        tracing::info!(
                            count,
                            "football-data.co.uk: zapisano terminarz fixtures.csv"
                        );
                        total += count;
                    }
                    Err(err) => tracing::warn!("football-data.co.uk fixtures: błąd zapisu: {err}"),
                },
                Err(err) => tracing::warn!("football-data.co.uk fixtures: pominięto: {err}"),
            }
        }
    }

    // 2. Football-Data.org (Auto lub FootballData)
    if matches!(
        source,
        FootballIngestSource::Auto | FootballIngestSource::FootballData
    ) {
        match &state.config.football_data_api_key {
            Some(key) => {
                for spec in &leagues {
                    let url = format!(
                        "https://api.football-data.org/v4/competitions/{}/matches?season={}",
                        spec.code, current
                    );
                    let result = async {
                        let body =
                            fetch_text(&state.http_client, &url, &[("X-Auth-Token", key)]).await?;
                        let matches = parse_football_data_for_league(&body, spec)?;
                        store_matches(&state.db, &matches).await
                    }
                    .await;

                    match result {
                        Ok(count) => {
                            tracing::info!(
                                league = spec.code,
                                season = current,
                                count,
                                "Football-Data: zapisano mecze"
                            );
                            total += count;
                        }
                        Err(err) if source == FootballIngestSource::Auto => tracing::warn!(
                            league = spec.code,
                            season = current,
                            "Football-Data: pominięto ligę: {}",
                            err
                        ),
                        Err(err) => return Err(err),
                    }
                }
            }
            None => {
                if source == FootballIngestSource::FootballData {
                    tracing::warn!(
                        "INGEST_FOOTBALL_SOURCE=football_data wymaga FOOTBALL_DATA_API_KEY – pomijam"
                    );
                } else {
                    tracing::info!(
                        "Brak FOOTBALL_DATA_API_KEY – bieżący sezon obsługuje football-data.co.uk + ClubElo"
                    );
                }
            }
        }
    }

    // 3. ClubElo (wzbogacanie istniejących meczów o pre_match_elo z dnia D - 1)
    if state.config.ingest_football_clubelo_enabled {
        let league_codes: Vec<&'static str> = leagues.iter().map(|s| s.code).collect();
        match enrich_matches_with_clubelo(state, &league_codes).await {
            Ok(enriched) => {
                tracing::info!(enriched, "ClubElo: zaktualizowano ratingi przedmeczowe");
                total += enriched;
            }
            Err(err) => tracing::warn!("ClubElo: pominięto wzbogacanie meczów: {}", err),
        }
    }

    Ok(total)
}
