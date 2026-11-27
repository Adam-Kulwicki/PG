// src/ingest/cs2/kaggle_csv.rs
//! Archiwalny i przyrostowy import CS2 z datasetu Kaggle (`ektarr/counter-strike-pro-matches`).
//!
//! Gdy ustawiono `KAGGLE_API_TOKEN`, moduł sprawdza w każdym cyklu numer wersji
//! datasetu przez Kaggle REST API i pobiera nowy plik CSV tylko przy zmianie wersji lub braku cache.

use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Read},
    path::Path,
    time::Duration,
};

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::Deserialize;
use serde_json::json;
use zip::ZipArchive;

use super::teams::normalize_cs2_team_name;
use crate::{
    error::{AppError, AppResult},
    ingest::{tennis::split_csv_line, winner_from_scores, IngestedMatch, IngestedParticipant},
    models::match_model::{Cs2MapResult, Cs2Player, Cs2PlayerStats, MatchDetails},
    state::AppState,
};

/// Identyfikator datasetu Kaggle ze zbioru `ektarr/counter-strike-pro-matches`.
pub const KAGGLE_CS2_DATASET: &str = "ektarr/counter-strike-pro-matches";

/// Nazwa pliku wewnątrz datasetu Kaggle przeznaczonego dla rozgrywek Tier 1.
pub const KAGGLE_CS2_FILE: &str = "cs2_tier1_games.csv";

/// Ścieżka lokalnego cache pobranego (lub ręcznie umieszczonego) pliku CSV.
pub const CS2_CACHE_CSV_PATH: &str = "data/cs2_games.csv";

/// Ścieżka pliku przechowującego numer ostatnio przetworzonej wersji datasetu Kaggle.
pub const CS2_CACHE_VERSION_PATH: &str = "data/cs2_games.csv.version";

const KAGGLE_API_BASE_URL: &str = "https://www.kaggle.com/api/v1/datasets";
const KAGGLE_DOWNLOAD_TIMEOUT_SECS: u64 = 120;
const ZIP_MAGIC_BYTES: [u8; 2] = [0x50, 0x4B];
const MAX_CS2_PLAYERS_PER_TEAM: usize = 5;

#[derive(Debug, Deserialize)]
struct KaggleDatasetMetadata {
    #[serde(rename = "currentVersionNumber")]
    current_version_number: Option<u64>,
}

/// Parsuje numer wersji datasetu (`currentVersionNumber`) z odpowiedzi `/datasets/view/...`.
pub fn parse_kaggle_dataset_version(json_body: &str) -> AppResult<u64> {
    let metadata: KaggleDatasetMetadata = serde_json::from_str(json_body)
        .map_err(|err| AppError::ExternalData(format!("Kaggle metadata: {err}")))?;
    match metadata.current_version_number {
        Some(version) if version > 0 => Ok(version),
        _ => Err(AppError::ExternalData(
            "Kaggle metadata: brak poprawnego pola currentVersionNumber".to_string(),
        )),
    }
}

/// Wydobywa surowe bajty CSV z odpowiedzi Kaggle (obsługuje zarówno czysty CSV, jak i archiwum ZIP).
pub fn extract_csv_bytes(payload: &[u8], expected_file: &str) -> AppResult<Vec<u8>> {
    if payload.is_empty() {
        return Err(AppError::ExternalData(
            "Kaggle download: otrzymano pustą odpowiedź".to_string(),
        ));
    }

    if !payload.starts_with(&ZIP_MAGIC_BYTES) {
        return Ok(payload.to_vec());
    }

    let cursor = Cursor::new(payload);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|err| AppError::ExternalData(format!("Kaggle ZIP: {err}")))?;

    let mut target_index = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|err| AppError::ExternalData(format!("Kaggle ZIP entry: {err}")))?;
        let entry_name = entry
            .name()
            .map_err(|err| AppError::ExternalData(format!("Kaggle ZIP entry name: {err}")))?;
        if entry_name.eq_ignore_ascii_case(expected_file) {
            target_index = Some(index);
            break;
        }
        if target_index.is_none() && entry_name.to_ascii_lowercase().ends_with(".csv") {
            target_index = Some(index);
        }
    }

    let index = target_index.ok_or_else(|| {
        AppError::ExternalData("Kaggle ZIP: brak pliku CSV w archiwum".to_string())
    })?;
    let mut file = archive
        .by_index(index)
        .map_err(|err| AppError::ExternalData(format!("Kaggle ZIP read: {err}")))?;
    let mut extracted = Vec::new();
    file.read_to_end(&mut extracted)
        .map_err(|err| AppError::ExternalData(format!("Kaggle ZIP decompress: {err}")))?;
    Ok(extracted)
}

/// Wyznacza poziom rozgrywek (`tier1`/`tier2`/`tier3`) na podstawie nazwy pliku z Kaggle.
pub fn infer_tier_from_filename(file_name: &str) -> Option<String> {
    let lowered = file_name.to_ascii_lowercase();
    if lowered.contains("tier1") {
        Some("tier1".to_string())
    } else if lowered.contains("tier2") {
        Some("tier2".to_string())
    } else if lowered.contains("tier3") {
        Some("tier3".to_string())
    } else {
        None
    }
}

/// Zwraca wartość niepustego pola CSV po nazwie kolumny (bez rozróżniania wielkości liter).
fn field_str<'a>(
    fields: &'a [String],
    header_index: &HashMap<String, usize>,
    column: &str,
) -> Option<&'a str> {
    header_index
        .get(&column.to_ascii_lowercase())
        .and_then(|&idx| fields.get(idx))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

fn field_i32(
    fields: &[String],
    header_index: &HashMap<String, usize>,
    column: &str,
) -> Option<i32> {
    let raw = field_str(fields, header_index, column)?;
    raw.parse::<i32>()
        .ok()
        .or_else(|| raw.parse::<f64>().ok().map(|v| v.round() as i32))
}

fn field_i64(
    fields: &[String],
    header_index: &HashMap<String, usize>,
    column: &str,
) -> Option<i64> {
    let raw = field_str(fields, header_index, column)?;
    raw.parse::<i64>()
        .ok()
        .or_else(|| raw.parse::<f64>().ok().map(|v| v.round() as i64))
}

fn field_f64(
    fields: &[String],
    header_index: &HashMap<String, usize>,
    column: &str,
) -> Option<f64> {
    field_str(fields, header_index, column)?.parse::<f64>().ok()
}

fn is_truthy_flag(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
}

fn parse_kaggle_datetime(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(dt.and_utc());
        }
    }
    raw.get(..10)
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d.and_time(NaiveTime::MIN).and_utc())
}

/// Wczytuje listę do 5 zawodników oraz opcjonalne statystyki pomeczowe dla jednej strony meczu.
fn extract_team_roster_and_stats(
    fields: &[String],
    header_index: &HashMap<String, usize>,
    team_prefix: &str,
    team_side: &str,
) -> (Vec<Cs2Player>, Vec<Cs2PlayerStats>) {
    let mut roster = Vec::new();
    let mut stats = Vec::new();

    for slot in 1..=MAX_CS2_PLAYERS_PER_TEAM {
        let base_col = format!("{team_prefix}_player{slot}");
        let id_col = format!("{base_col}_id");
        let player_id = field_i64(fields, header_index, &id_col);
        let nickname = field_str(fields, header_index, &base_col).map(str::to_string);

        let Some(display_name) = nickname.or_else(|| player_id.map(|id| format!("player_{id}")))
        else {
            continue;
        };

        roster.push(Cs2Player {
            external_id: player_id,
            name: display_name.clone(),
        });

        let kills = field_i32(fields, header_index, &format!("{base_col}_kills"));
        let deaths = field_i32(fields, header_index, &format!("{base_col}_deaths"));
        let assists = field_i32(fields, header_index, &format!("{base_col}_assists"));
        let adr = field_f64(fields, header_index, &format!("{base_col}_adr"));
        let kast = field_f64(fields, header_index, &format!("{base_col}_kast"));
        let kddiff = field_i32(fields, header_index, &format!("{base_col}_kddiff"));

        let has_any_stat = kills.is_some()
            || deaths.is_some()
            || assists.is_some()
            || adr.is_some()
            || kast.is_some()
            || kddiff.is_some();

        if has_any_stat {
            stats.push(Cs2PlayerStats {
                team_side: team_side.to_string(),
                player_id,
                name: display_name,
                kills,
                deaths,
                assists,
                adr,
                kast,
                kddiff,
            });
        }
    }

    (roster, stats)
}

#[derive(Debug)]
struct ParsedCsvRow {
    fields: Vec<String>,
    game_id: i64,
    is_total: bool,
}

/// Parsuje treść pliku CSV z datasetu `ektarr/counter-strike-pro-matches`.
///
/// Wiersze grupowane są po `match_id`: rekordy z `is_total == false` dostarczają wyniki rundowe
/// poszczególnych map, natomiast wiersz z `is_total == true` dostarcza wynik serii, składy i statystyki.
pub fn parse_kaggle_cs2_csv(
    csv_content: &str,
    default_tier: Option<&str>,
) -> AppResult<Vec<IngestedMatch>> {
    let trimmed_content = csv_content.trim_start_matches('\u{feff}');
    let mut lines = trimmed_content
        .lines()
        .filter(|line| !line.trim().is_empty());

    let Some(header_line) = lines.next() else {
        return Ok(Vec::new());
    };
    let header_index: HashMap<String, usize> = split_csv_line(header_line)
        .into_iter()
        .enumerate()
        .map(|(idx, col)| (col.trim().to_ascii_lowercase(), idx))
        .collect();

    for required in ["match_id", "datetime", "team1", "team2"] {
        if !header_index.contains_key(required) {
            return Err(AppError::ExternalData(format!(
                "Kaggle CS2 CSV: brak wymaganej kolumny '{required}'"
            )));
        }
    }

    // BTreeMap porządkuje grupy po match_id deterministycznie przed sortowaniem po dacie.
    let mut grouped_matches: BTreeMap<String, Vec<ParsedCsvRow>> = BTreeMap::new();
    for line in lines {
        let fields = split_csv_line(line);
        let Some(match_id) = field_str(&fields, &header_index, "match_id").map(str::to_string)
        else {
            continue;
        };
        let game_id = field_i64(&fields, &header_index, "game_id").unwrap_or(0);
        let is_total = is_truthy_flag(field_str(&fields, &header_index, "is_total"));
        grouped_matches
            .entry(match_id)
            .or_default()
            .push(ParsedCsvRow {
                fields,
                game_id,
                is_total,
            });
    }

    let mut ingested = Vec::with_capacity(grouped_matches.len());
    for (match_id, mut rows) in grouped_matches {
        rows.sort_by_key(|row| row.game_id);

        let summary_row = rows
            .iter()
            .find(|row| row.is_total)
            .or_else(|| rows.first())
            .expect("grupa match_id zawiera co najmniej jeden wiersz");
        let summary_fields = &summary_row.fields;

        let (Some(raw_home), Some(raw_away), Some(match_date)) = (
            field_str(summary_fields, &header_index, "team1"),
            field_str(summary_fields, &header_index, "team2"),
            field_str(summary_fields, &header_index, "datetime").and_then(parse_kaggle_datetime),
        ) else {
            continue;
        };

        let home_name = normalize_cs2_team_name(raw_home);
        let away_name = normalize_cs2_team_name(raw_away);
        if home_name.is_empty() || away_name.is_empty() {
            continue;
        }

        let mut home_participant = IngestedParticipant::new(home_name, None);
        if let Some(team_id) = field_i64(summary_fields, &header_index, "team1_id") {
            home_participant.metadata = json!({ "cs2_kaggle_id": team_id });
        }
        let mut away_participant = IngestedParticipant::new(away_name, None);
        if let Some(team_id) = field_i64(summary_fields, &header_index, "team2_id") {
            away_participant.metadata = json!({ "cs2_kaggle_id": team_id });
        }

        let mut maps = Vec::new();
        for row in rows.iter().filter(|r| !r.is_total) {
            let Some(map_name) =
                field_str(&row.fields, &header_index, "map_name").map(str::to_string)
            else {
                continue;
            };
            let home_map_score = field_i32(&row.fields, &header_index, "score1_game");
            let away_map_score = field_i32(&row.fields, &header_index, "score2_game");
            let map_winner = home_map_score
                .zip(away_map_score)
                .and_then(|(h, a)| winner_from_scores(h, a, true))
                .map(str::to_string);
            maps.push(Cs2MapResult {
                map_name,
                home_score: home_map_score,
                away_score: away_map_score,
                winner: map_winner,
            });
        }

        // Gdy CSV ma wynik serii w `score1_match`/`score2_match`, traktujemy go priorytetowo.
        let series_scores = field_i32(summary_fields, &header_index, "score1_match")
            .zip(field_i32(summary_fields, &header_index, "score2_match"))
            .or_else(|| {
                if maps.is_empty() {
                    None
                } else {
                    let home_wins = maps
                        .iter()
                        .filter(|m| m.winner.as_deref() == Some("HOME"))
                        .count() as i32;
                    let away_wins = maps
                        .iter()
                        .filter(|m| m.winner.as_deref() == Some("AWAY"))
                        .count() as i32;
                    Some((home_wins, away_wins))
                }
            });

        let winner = series_scores
            .and_then(|(h, a)| winner_from_scores(h, a, true))
            .map(str::to_string)
            .or_else(|| {
                field_i32(summary_fields, &header_index, "team1_win1").map(|flag| {
                    if flag == 1 {
                        "HOME".to_string()
                    } else {
                        "AWAY".to_string()
                    }
                })
            });

        let (home_roster, mut player_stats) =
            extract_team_roster_and_stats(summary_fields, &header_index, "team1", "HOME");
        let (away_roster, away_stats) =
            extract_team_roster_and_stats(summary_fields, &header_index, "team2", "AWAY");
        player_stats.extend(away_stats);

        let best_of = field_i32(summary_fields, &header_index, "bestOf")
            .and_then(|value| u8::try_from(value).ok())
            .filter(|&value| value > 0);
        let tournament_name =
            field_str(summary_fields, &header_index, "tournament").map(str::to_string);
        let tournament_tier = field_str(summary_fields, &header_index, "tier")
            .map(str::to_string)
            .or_else(|| default_tier.map(str::to_string));

        let external_match_id = match match_id.parse::<i64>() {
            Ok(numeric_id) => json!(numeric_id),
            Err(_) => json!(match_id),
        };

        ingested.push(IngestedMatch {
            sport: "CS2",
            competition_code: "CS2".to_string(),
            competition_name: "Counter-Strike 2".to_string(),
            competition_country: Some("International".to_string()),
            season: match_date.year().to_string(),
            match_date,
            status: "FINISHED".to_string(),
            home: home_participant,
            away: away_participant,
            home_score: series_scores.map(|(h, _)| h),
            away_score: series_scores.map(|(_, a)| a),
            winner,
            details: MatchDetails::Cs2 {
                best_of,
                tournament_name,
                tournament_tier,
                maps,
                home_roster,
                away_roster,
                player_stats,
                veto: Vec::new(),
            },
            external_ids: json!({ "cs2_kaggle": external_match_id }),
        });
    }

    ingested.sort_by_key(|m| m.match_date);
    Ok(ingested)
}

async fn fetch_kaggle_version(client: &reqwest::Client, api_token: &str) -> AppResult<u64> {
    let url = format!("{KAGGLE_API_BASE_URL}/view/{KAGGLE_CS2_DATASET}");
    let response = client
        .get(&url)
        .bearer_auth(api_token)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_kaggle_dataset_version(&response)
}

async fn download_kaggle_csv(client: &reqwest::Client, api_token: &str) -> AppResult<String> {
    let url = format!("{KAGGLE_API_BASE_URL}/download/{KAGGLE_CS2_DATASET}/{KAGGLE_CS2_FILE}");
    let bytes = client
        .get(&url)
        .timeout(Duration::from_secs(KAGGLE_DOWNLOAD_TIMEOUT_SECS))
        .bearer_auth(api_token)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let extracted = extract_csv_bytes(&bytes, KAGGLE_CS2_FILE)?;
    String::from_utf8(extracted)
        .map_err(|err| AppError::ExternalData(format!("Kaggle CSV UTF-8: {err}")))
}

fn read_cached_version(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

fn write_cache_files(csv_path: &Path, version_path: &Path, csv_content: &str, version: u64) {
    if let Some(parent) = csv_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                path = %parent.display(),
                "Kaggle CS2: nie udało się utworzyć katalogu cache: {err}"
            );
            return;
        }
    }
    if let Err(err) = std::fs::write(csv_path, csv_content) {
        tracing::warn!(
            path = %csv_path.display(),
            "Kaggle CS2: nie udało się zapisać pliku cache CSV: {err}"
        );
        return;
    }
    if let Err(err) = std::fs::write(version_path, version.to_string()) {
        tracing::warn!(
            path = %version_path.display(),
            "Kaggle CS2: nie udało się zapisać wersji cache: {err}"
        );
    }
}

/// Synchronizuje historyczne mecze CS2 z Kaggle CSV (`ektarr/counter-strike-pro-matches`).
///
/// Gdy podano `KAGGLE_API_TOKEN`, sprawdza wersję w każdym cyklu i pobiera plik
/// ponownie wyłącznie po publikacji nowej wersji, co automatycznie wzbogaca mecze z PandaScore.
pub async fn sync_kaggle_csv(state: &AppState, include_history: bool) -> AppResult<usize> {
    let cache_csv_path = Path::new(CS2_CACHE_CSV_PATH);
    let cache_version_path = Path::new(CS2_CACHE_VERSION_PATH);
    let default_tier = infer_tier_from_filename(KAGGLE_CS2_FILE);

    let csv_content = match state.config.kaggle_api_token.as_deref() {
        Some(api_token) => {
            let remote_version = fetch_kaggle_version(&state.http_client, api_token).await?;
            let cached_version = read_cached_version(cache_version_path);
            let has_cache_file = cache_csv_path.exists();

            if cached_version == Some(remote_version) && has_cache_file {
                if !include_history {
                    tracing::debug!(
                        version = remote_version,
                        "Kaggle CS2: brak nowej wersji datasetu – pomijam import CSV"
                    );
                    return Ok(0);
                }
                tokio::fs::read_to_string(cache_csv_path)
                    .await
                    .map_err(|err| AppError::ExternalData(format!("Kaggle cache read: {err}")))?
            } else {
                tracing::info!(
                    previous_version = ?cached_version,
                    remote_version,
                    "Kaggle CS2: pobieranie datasetu CSV"
                );
                let downloaded = download_kaggle_csv(&state.http_client, api_token).await?;
                write_cache_files(
                    cache_csv_path,
                    cache_version_path,
                    &downloaded,
                    remote_version,
                );
                downloaded
            }
        }
        None => {
            if !include_history || !cache_csv_path.exists() {
                return Ok(0);
            }
            tokio::fs::read_to_string(cache_csv_path)
                .await
                .map_err(|err| AppError::ExternalData(format!("Kaggle local CSV: {err}")))?
        }
    };

    let matches = parse_kaggle_cs2_csv(&csv_content, default_tier.as_deref())?;
    let saved_count = crate::ingest::store_matches(&state.db, &matches).await?;
    tracing::info!(saved_count, "Kaggle CS2: zaimportowano/wzbogacono mecze");
    Ok(saved_count)
}
