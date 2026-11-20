// src/ingest/football.rs
//! Piłka nożna (Premier League): OpenFootball dla sezonów historycznych
//! oraz Football-Data.org dla sezonu bieżącego.
//!
//! Oba źródła nie powinny obejmować tego samego sezonu – godziny meczów w nich się różnią,
//! więc mecz trafiłby do bazy dwa razy.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::Deserialize;
use serde_json::json;

pub use super::normalize_season_label;
use super::{
    fetch_text, season_start_year, short_season_label, store_matches, winner_from_scores,
    IngestedMatch, IngestedParticipant,
};
use crate::{
    error::{AppError, AppResult},
    models::match_model::MatchDetails,
    state::AppState,
};

/// Kod i nazwa rozgrywek.
const COMPETITION_CODE: &str = "PL";
const COMPETITION_NAME: &str = "Premier League";
const COMPETITION_COUNTRY: &str = "England";

/// Godzina przyjmowana dla meczów OpenFootball bez podanej godziny.
const DEFAULT_KICKOFF_HOUR: u32 = 15;

// ---------------------------------------------------------------------------
// OpenFootball (historia)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenFootballFile {
    matches: Vec<OpenFootballMatch>,
}

#[derive(Debug, Deserialize)]
struct OpenFootballMatch {
    round: Option<String>,
    /// Data w formacie `YYYY-MM-DD`.
    date: String,
    /// Godzina w formacie `HH:MM` (opcjonalna).
    time: Option<String>,
    team1: OpenFootballTeam,
    team2: OpenFootballTeam,
    score: Option<OpenFootballScore>,
}

/// Starsze pliki OpenFootball opisują drużynę obiektem, nowsze samą nazwą.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenFootballTeam {
    Name(String),
    Object { name: String, code: Option<String> },
}

impl OpenFootballTeam {
    fn into_participant(self) -> IngestedParticipant {
        match self {
            Self::Name(name) => IngestedParticipant::new(name, None),
            Self::Object { name, code } => IngestedParticipant::new(name, code),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenFootballScore {
    /// Wynik po 90 minutach `[gospodarze, goście]`.
    ft: Option<Vec<i32>>,
    /// Wynik do przerwy `[gospodarze, goście]`.
    ht: Option<Vec<i32>>,
}

/// Parsuje plik `en.1.json` z repozytorium OpenFootball.
/// Mecze z nieczytelną datą są pomijane; mecz bez wyniku ma status `SCHEDULED`.
pub fn parse_openfootball(json: &str, season_label: &str) -> AppResult<Vec<IngestedMatch>> {
    let file: OpenFootballFile = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("OpenFootball: {}", e)))?;
    let season = normalize_season_label(season_label);

    let mut result = Vec::with_capacity(file.matches.len());
    for m in file.matches {
        let Ok(date) = NaiveDate::parse_from_str(&m.date, "%Y-%m-%d") else {
            continue;
        };
        let time = m
            .time
            .as_deref()
            .and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
            .unwrap_or_else(|| NaiveTime::from_hms_opt(DEFAULT_KICKOFF_HOUR, 0, 0).unwrap());
        let match_date = date.and_time(time).and_utc();

        let (ft, ht) = match &m.score {
            Some(score) => (score.ft.as_deref(), score.ht.as_deref()),
            None => (None, None),
        };
        let full_time = ft.and_then(|v| match v {
            [h, a] => Some((*h, *a)),
            _ => None,
        });
        let half_time = ht.and_then(|v| match v {
            [h, a] => Some((*h, *a)),
            _ => None,
        });

        result.push(IngestedMatch {
            sport: "FOOTBALL",
            competition_code: COMPETITION_CODE.to_string(),
            competition_name: COMPETITION_NAME.to_string(),
            competition_country: Some(COMPETITION_COUNTRY.to_string()),
            season: season.clone(),
            match_date,
            status: if full_time.is_some() {
                "FINISHED"
            } else {
                "SCHEDULED"
            }
            .to_string(),
            home: m.team1.into_participant(),
            away: m.team2.into_participant(),
            home_score: full_time.map(|(h, _)| h),
            away_score: full_time.map(|(_, a)| a),
            winner: full_time
                .and_then(|(h, a)| winner_from_scores(h, a, true))
                .map(str::to_string),
            details: MatchDetails::Football {
                ht_home: half_time.map(|(h, _)| h),
                ht_away: half_time.map(|(_, a)| a),
                round: m.round.unwrap_or_default(),
            },
            external_ids: json!({}),
        });
    }
    Ok(result)
}

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

/// Parsuje odpowiedź `/v4/competitions/PL/matches`.
/// Wynik zapisujemy tylko dla zakończonych meczów, żeby trwający mecz nie wyglądał na rozstrzygnięty.
pub fn parse_football_data(json: &str) -> AppResult<Vec<IngestedMatch>> {
    let response: FootballDataResponse = serde_json::from_str(json)
        .map_err(|e| AppError::ExternalData(format!("Football-Data: {}", e)))?;

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

        result.push(IngestedMatch {
            sport: "FOOTBALL",
            competition_code: COMPETITION_CODE.to_string(),
            competition_name: COMPETITION_NAME.to_string(),
            competition_country: Some(COMPETITION_COUNTRY.to_string()),
            season: format!("{}-{}", start_year, end_year),
            match_date: match_date.with_timezone(&Utc),
            status: status.to_string(),
            home: IngestedParticipant::new(home_name, m.home_team.tla),
            away: IngestedParticipant::new(away_name, m.away_team.tla),
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
            },
            external_ids: json!({ "football_data": m.id }),
        });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Synchronizacja
// ---------------------------------------------------------------------------

/// Importuje piłkę nożną: opcjonalnie sezony historyczne (OpenFootball), zawsze sezon bieżący
/// (Football-Data.org, o ile ustawiono `FOOTBALL_DATA_API_KEY`).
pub async fn sync(state: &AppState, include_history: bool) -> AppResult<usize> {
    let current = season_start_year(Utc::now());
    let mut total = 0;

    if include_history {
        let seasons = if state.config.ingest_football_seasons.is_empty() {
            vec![
                short_season_label(current - 2),
                short_season_label(current - 1),
            ]
        } else {
            state.config.ingest_football_seasons.clone()
        };

        for season in seasons {
            // Brak pliku jednego sezonu (np. jeszcze nieopublikowanego) nie przerywa pozostałych.
            let url = format!(
                "https://raw.githubusercontent.com/openfootball/football.json/master/{}/en.1.json",
                season
            );
            let result = async {
                let body = fetch_text(&state.http_client, &url, &[]).await?;
                let matches = parse_openfootball(&body, &season)?;
                store_matches(&state.db, &matches).await
            }
            .await;

            match result {
                Ok(count) => {
                    tracing::info!(season, count, "OpenFootball: zapisano mecze");
                    total += count;
                }
                Err(err) => tracing::warn!(season, "OpenFootball: pominięto sezon: {}", err),
            }
        }
    }

    match &state.config.football_data_api_key {
        Some(key) => {
            let url = format!(
                "https://api.football-data.org/v4/competitions/{}/matches?season={}",
                COMPETITION_CODE, current
            );
            let body = fetch_text(&state.http_client, &url, &[("X-Auth-Token", key)]).await?;
            let matches = parse_football_data(&body)?;
            total += store_matches(&state.db, &matches).await?;
        }
        None => tracing::info!("Brak FOOTBALL_DATA_API_KEY – pomijam bieżący sezon piłki nożnej"),
    }

    Ok(total)
}
