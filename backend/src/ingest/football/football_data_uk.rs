// src/ingest/football/football_data_uk.rs
//! Klient i parser plików CSV z `football-data.co.uk`.
//!
//! Dostarcza historię i bieżący sezon dla wszystkich lig Top 5 (`E0`, `SP1`, `D1`, `I1`, `F1`)
//! wraz ze statystykami meczowymi (`xG`, strzały, strzały celne, rożne, faule, kartki, wynik HT)
//! oraz terminarz nadchodzących spotkań z `fixtures.csv`.

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde_json::json;

use super::teams::{league_spec, normalize_football_team_name, LeagueSpec};
use crate::{
    error::{AppError, AppResult},
    ingest::{
        fetch_text_with_timeout, full_season_label, winner_from_scores, IngestedMatch,
        IngestedParticipant,
    },
    models::match_model::{FootballTeamStats, MatchDetails},
};

/// Limit czasu pojedynczego zapytania do `football-data.co.uk`.
pub const FDUK_TIMEOUT: Duration = Duration::from_secs(30);

/// Krótka pauza między pobieraniem kolejnych plików CSV.
pub const FDUK_REQUEST_DELAY: Duration = Duration::from_millis(500);

const FDUK_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];

/// Publiczny CSV z nadchodzącymi meczami (używany jako fallback, gdy brak `FOOTBALL_DATA_API_KEY`).
pub const FOOTBALL_DATA_UK_FIXTURES_URL: &str = "https://football-data.co.uk/fixtures.csv";

/// Zamienia rok startu sezonu (np. `2026`) na kod katalogu `football-data.co.uk` (`2627`).
pub fn football_data_uk_season_code(season_start: i32) -> String {
    format!("{:02}{:02}", season_start % 100, (season_start + 1) % 100)
}

/// Buduje adres pliku CSV dla wskazanej dywizji i roku startu sezonu.
pub fn football_data_uk_season_url(div_code: &str, season_start: i32) -> String {
    let code = football_data_uk_season_code(season_start);
    format!("https://football-data.co.uk/mmz4281/{code}/{div_code}.csv")
}

/// Parsuje zapis sezonu z `INGEST_FOOTBALL_SEASONS` (`2023-24`, `2023/24`, `2023-2024` lub `2023`)
/// na rok startu sezonu.
pub fn parse_football_season_year(raw: &str) -> Option<i32> {
    let cleaned = raw.trim().replace('/', "-");
    let start_part = cleaned.split('-').next()?.trim();
    if start_part.len() != 4 {
        return None;
    }
    let year = start_part.parse::<i32>().ok()?;
    (1900..=2100).contains(&year).then_some(year)
}

fn build_header_index(header_line: &str) -> HashMap<String, usize> {
    header_line
        .trim_start_matches('\u{feff}')
        .split(',')
        .enumerate()
        .map(|(i, col)| (col.trim().to_string(), i))
        .collect()
}

fn col<'a>(fields: &[&'a str], idx: &HashMap<String, usize>, name: &str) -> Option<&'a str> {
    idx.get(name)
        .and_then(|&i| fields.get(i))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

fn col_i32(fields: &[&str], idx: &HashMap<String, usize>, name: &str) -> Option<i32> {
    col(fields, idx, name).and_then(|v| v.parse::<i32>().ok())
}

fn col_f64(fields: &[&str], idx: &HashMap<String, usize>, name: &str) -> Option<f64> {
    col(fields, idx, name)
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|f| f.is_finite())
}

/// Parsuje datę `DD/MM/YYYY` (lub starsze `DD/MM/YY`) oraz opcjonalną godzinę `HH:MM` (UTC).
fn parse_fduk_datetime(date_str: &str, time_str: Option<&str>) -> Option<DateTime<Utc>> {
    let trimmed = date_str.trim();
    // W `chrono` format `%Y` akceptuje też rok dwucyfrowy i zapisuje go jako `00XX` zamiast `20XX`,
    // dlatego dobieramy format jawnie na podstawie długości ostatniego członu daty.
    let date_format = match trimmed.rsplit('/').next().map(str::len) {
        Some(2) => "%d/%m/%y",
        Some(4) => "%d/%m/%Y",
        _ => return None,
    };
    let date = NaiveDate::parse_from_str(trimmed, date_format).ok()?;
    let time = time_str
        .and_then(|t| NaiveTime::parse_from_str(t.trim(), "%H:%M").ok())
        .unwrap_or(NaiveTime::MIN);
    Some(date.and_time(time).and_utc())
}

fn stats_if_any(stats: FootballTeamStats) -> Option<Box<FootballTeamStats>> {
    let has_any = stats.xg.is_some()
        || stats.shots.is_some()
        || stats.shots_on_target.is_some()
        || stats.corners.is_some()
        || stats.fouls.is_some()
        || stats.yellow_cards.is_some()
        || stats.red_cards.is_some();
    has_any.then(|| Box::new(stats))
}

/// Parsuje plik CSV sezonu z `football-data.co.uk` (np. `E0.csv`, `SP1.csv`).
pub fn parse_football_data_uk_csv(
    csv: &str,
    spec: &LeagueSpec,
    season_start: i32,
) -> AppResult<Vec<IngestedMatch>> {
    let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| AppError::ExternalData("football-data.co.uk: pusty plik CSV".to_string()))?;
    let idx = build_header_index(header);

    for required in ["Date", "HomeTeam", "AwayTeam", "FTHG", "FTAG"] {
        if !idx.contains_key(required) {
            return Err(AppError::ExternalData(format!(
                "football-data.co.uk ({}): brak kolumny '{required}'",
                spec.football_data_uk
            )));
        }
    }

    let season_label = full_season_label(season_start);
    let mut matches = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.split(',').collect();
        let (Some(raw_date), Some(raw_home), Some(raw_away), Some(hg), Some(ag)) = (
            col(&fields, &idx, "Date"),
            col(&fields, &idx, "HomeTeam"),
            col(&fields, &idx, "AwayTeam"),
            col_i32(&fields, &idx, "FTHG"),
            col_i32(&fields, &idx, "FTAG"),
        ) else {
            continue;
        };

        let Some(match_date) = parse_fduk_datetime(raw_date, col(&fields, &idx, "Time")) else {
            continue;
        };

        let home_canonical = normalize_football_team_name(raw_home).to_string();
        let away_canonical = normalize_football_team_name(raw_away).to_string();
        let ext_id = format!(
            "{}:{}:{}:{}",
            spec.football_data_uk,
            match_date.format("%Y-%m-%d"),
            home_canonical,
            away_canonical
        );

        let home_stats = stats_if_any(FootballTeamStats {
            xg: col_f64(&fields, &idx, "HxG"),
            shots: col_i32(&fields, &idx, "HS"),
            shots_on_target: col_i32(&fields, &idx, "HST"),
            corners: col_i32(&fields, &idx, "HC"),
            fouls: col_i32(&fields, &idx, "HF"),
            yellow_cards: col_i32(&fields, &idx, "HY"),
            red_cards: col_i32(&fields, &idx, "HR"),
        });
        let away_stats = stats_if_any(FootballTeamStats {
            xg: col_f64(&fields, &idx, "AxG"),
            shots: col_i32(&fields, &idx, "AS"),
            shots_on_target: col_i32(&fields, &idx, "AST"),
            corners: col_i32(&fields, &idx, "AC"),
            fouls: col_i32(&fields, &idx, "AF"),
            yellow_cards: col_i32(&fields, &idx, "AY"),
            red_cards: col_i32(&fields, &idx, "AR"),
        });

        matches.push(IngestedMatch {
            sport: "FOOTBALL",
            competition_code: spec.code.to_string(),
            competition_name: spec.name.to_string(),
            competition_country: Some(spec.country.to_string()),
            season: season_label.clone(),
            match_date,
            status: "FINISHED".to_string(),
            home: IngestedParticipant::new(home_canonical, None),
            away: IngestedParticipant::new(away_canonical, None),
            home_score: Some(hg),
            away_score: Some(ag),
            winner: winner_from_scores(hg, ag, true).map(str::to_string),
            details: MatchDetails::Football {
                ht_home: col_i32(&fields, &idx, "HTHG"),
                ht_away: col_i32(&fields, &idx, "HTAG"),
                round: String::new(),
                home_stats,
                away_stats,
                pre_match_elo: None,
            },
            external_ids: json!({ "football_data_uk": ext_id }),
        });
    }

    matches.sort_by(|a, b| {
        a.match_date.cmp(&b.match_date).then_with(|| {
            a.external_ids["football_data_uk"]
                .as_str()
                .cmp(&b.external_ids["football_data_uk"].as_str())
        })
    });
    Ok(matches)
}

/// Parsuje plik `fixtures.csv` z `football-data.co.uk` dla wybranych lig jako mecze `SCHEDULED`.
pub fn parse_football_data_uk_fixtures_csv(
    csv: &str,
    leagues: &[&LeagueSpec],
    season_start: i32,
) -> AppResult<Vec<IngestedMatch>> {
    let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or_else(|| {
        AppError::ExternalData("football-data.co.uk fixtures: pusty plik CSV".to_string())
    })?;
    let idx = build_header_index(header);

    for required in ["Div", "Date", "HomeTeam", "AwayTeam"] {
        if !idx.contains_key(required) {
            return Err(AppError::ExternalData(format!(
                "football-data.co.uk fixtures: brak kolumny '{required}'"
            )));
        }
    }

    let season_label = full_season_label(season_start);
    let mut matches = Vec::new();

    for line in lines {
        let fields: Vec<&str> = line.split(',').collect();
        let (Some(div), Some(raw_date), Some(raw_home), Some(raw_away)) = (
            col(&fields, &idx, "Div"),
            col(&fields, &idx, "Date"),
            col(&fields, &idx, "HomeTeam"),
            col(&fields, &idx, "AwayTeam"),
        ) else {
            continue;
        };

        let Some(spec) = league_spec(div).filter(|s| leagues.contains(s)) else {
            continue;
        };
        let Some(match_date) = parse_fduk_datetime(raw_date, col(&fields, &idx, "Time")) else {
            continue;
        };

        let home_canonical = normalize_football_team_name(raw_home).to_string();
        let away_canonical = normalize_football_team_name(raw_away).to_string();
        let ext_id = format!(
            "{}:{}:{}:{}",
            spec.football_data_uk,
            match_date.format("%Y-%m-%d"),
            home_canonical,
            away_canonical
        );

        matches.push(IngestedMatch {
            sport: "FOOTBALL",
            competition_code: spec.code.to_string(),
            competition_name: spec.name.to_string(),
            competition_country: Some(spec.country.to_string()),
            season: season_label.clone(),
            match_date,
            status: "SCHEDULED".to_string(),
            home: IngestedParticipant::new(home_canonical, None),
            away: IngestedParticipant::new(away_canonical, None),
            home_score: None,
            away_score: None,
            winner: None,
            details: MatchDetails::Football {
                ht_home: None,
                ht_away: None,
                round: String::new(),
                home_stats: None,
                away_stats: None,
                pre_match_elo: None,
            },
            external_ids: json!({ "football_data_uk": ext_id }),
        });
    }

    Ok(matches)
}

/// Pobiera i parsuje plik CSV sezonu dla jednej ligi z `football-data.co.uk`.
pub async fn fetch_league_season(
    client: &reqwest::Client,
    spec: &LeagueSpec,
    season_start: i32,
) -> AppResult<Vec<IngestedMatch>> {
    let url = football_data_uk_season_url(spec.football_data_uk, season_start);
    let csv = fetch_text_with_timeout(client, &url, &[], FDUK_TIMEOUT, &FDUK_RETRY_DELAYS).await?;
    parse_football_data_uk_csv(&csv, spec, season_start)
}

/// Pobiera i parsuje nadchodzące mecze z `football-data.co.uk/fixtures.csv`.
pub async fn fetch_fixtures(
    client: &reqwest::Client,
    leagues: &[&LeagueSpec],
    season_start: i32,
) -> AppResult<Vec<IngestedMatch>> {
    let csv = fetch_text_with_timeout(
        client,
        FOOTBALL_DATA_UK_FIXTURES_URL,
        &[],
        FDUK_TIMEOUT,
        &FDUK_RETRY_DELAYS,
    )
    .await?;
    parse_football_data_uk_fixtures_csv(&csv, leagues, season_start)
}
