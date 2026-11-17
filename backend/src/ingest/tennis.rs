// src/ingest/tennis.rs
//! Tenis: archiwum Jeffa Sackmanna (CSV, sezony zakończone) oraz publiczne API ESPN (bieżące drabinki).
//!
//! Sackmann zapisuje zwycięzcę zawsze jako pierwszą stronę meczu, więc przypisanie go do
//! gospodarza zdradzałoby wynik modelom ML (data leakage). Dlatego strony są losowane
//! deterministycznie na podstawie skrótu identyfikatora meczu.
//!
//! Archiwum importujemy tylko dla lat zakończonych, a ESPN obsługuje rok bieżący –
//! daty meczów w obu źródłach się różnią, więc nakładanie lat dawałoby duplikaty.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::{json, Value};

use super::{fetch_text, store_matches, IngestedMatch, IngestedParticipant};
use crate::{
    error::{AppError, AppResult},
    models::match_model::MatchDetails,
    state::AppState,
};

/// Godzina (UTC) przypisywana meczom Sackmanna, które mają tylko datę rozpoczęcia turnieju.
const DEFAULT_MATCH_HOUR: u32 = 12;

/// Kolejność rund turnieju; służy do rozłożenia meczów na kolejne dni.
const ROUND_ORDER: [&str; 8] = ["RR", "R128", "R64", "R32", "R16", "QF", "SF", "F"];

/// Nazwa i kod rozgrywek dla touru (`ATP` lub `WTA`); inne wartości zwracają `None`.
fn competition_for_tour(tour: &str) -> Option<(&'static str, &'static str)> {
    match tour {
        "ATP" => Some(("ATP", "ATP World Tour")),
        "WTA" => Some(("WTA", "WTA Tour")),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Sackmann (CSV)
// ---------------------------------------------------------------------------

/// Dzieli linię CSV na pola, obsługując cudzysłowy (pole w cudzysłowie może zawierać przecinki).
pub fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.trim_end_matches(['\r', '\n']).chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // Podwójny cudzysłów wewnątrz pola oznacza pojedynczy znak `"`.
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}

/// Skrót FNV-1a – prosty, deterministyczny i niezależny od wersji kompilatora.
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    // Wymieszanie bitów, aby zmiana ostatniego znaku wpływała na każdy bit skrótu
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51afd7ed558ccd);
    hash ^ (hash >> 33)
}

/// Czy zwycięzca meczu ma zostać zapisany jako gospodarz (losowanie niezależne od wyniku).
pub fn winner_is_home(match_key: &str) -> bool {
    (fnv1a(match_key) >> 24) & 1 == 0
}

/// Parsuje wynik w notacji Sackmanna (`6-4 3-6 7-6(5)`) z perspektywy zwycięzcy.
/// Pomija znaczniki typu `RET`; super tiebreak w nawiasach kwadratowych liczy się jako set.
pub fn parse_sets(score: &str) -> Vec<(u8, u8)> {
    score
        .split_whitespace()
        .filter_map(|token| {
            let token = token.trim_matches(|c| c == '[' || c == ']');
            // Wynik tiebreaka w nawiasie okrągłym (np. `7-6(5)`) nie jest potrzebny.
            let token = token.split('(').next()?;
            let (winner_games, loser_games) = token.split_once('-')?;
            Some((winner_games.parse().ok()?, loser_games.parse().ok()?))
        })
        .collect()
}

/// Liczba dni od rozpoczęcia turnieju, w których rozgrywana jest dana runda.
///
/// Pierwsza runda drabinki (zależna od jej wielkości) odpowiada dniowi 0, każda kolejna
/// przesuwa mecz o dzień. Dzięki temu backtest nie pokazuje agentom wszystkich rund naraz.
pub fn round_day_offset(round: &str, draw_size: Option<u32>) -> i64 {
    let first_round = match draw_size.unwrap_or(32) {
        128.. => "R128",
        48..=127 => "R64",
        28..=47 => "R32",
        16..=27 => "R16",
        _ => "RR",
    };
    let position = |name: &str| ROUND_ORDER.iter().position(|r| *r == name);

    // Mecz o brązowy medal rozgrywany jest razem z finałem.
    let round = if round == "BR" { "F" } else { round };
    match (position(round), position(first_round)) {
        (Some(current), Some(first)) => (current as i64 - first as i64).max(0),
        _ => 0,
    }
}

/// Pobiera pole wiersza po nazwie kolumny; puste pola traktuje jak brak wartości.
fn column<'a>(row: &'a [String], index: &HashMap<String, usize>, name: &str) -> Option<&'a str> {
    index
        .get(name)
        .and_then(|i| row.get(*i))
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
}

/// Parsuje plik `atp_matches_YYYY.csv` (lub WTA) do znormalizowanych meczów.
///
/// Pomijane są walkowery (`W/O`), dyskwalifikacje (`DEF`) i wiersze bez wyniku lub
/// z brakującymi kolumnami – nie są to rozegrane mecze.
pub fn parse_sackmann_csv(csv: &str, tour: &str) -> AppResult<Vec<IngestedMatch>> {
    let (code, name) = competition_for_tour(tour)
        .ok_or_else(|| AppError::BadRequest(format!("Nieznany tour tenisowy: {}", tour)))?;

    let mut lines = csv.lines();
    let header = lines
        .next()
        .ok_or_else(|| AppError::ExternalData("Sackmann: pusty plik CSV".to_string()))?;
    let index: HashMap<String, usize> = split_csv_line(header.trim_start_matches('\u{feff}'))
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, i))
        .collect();
    if !index.contains_key("winner_name") || !index.contains_key("tourney_date") {
        return Err(AppError::ExternalData(
            "Sackmann: nieoczekiwany nagłówek CSV".to_string(),
        ));
    }

    let mut result = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let row = split_csv_line(line);
        let get = |name: &str| column(&row, &index, name);

        let (Some(tourney_id), Some(match_num), Some(winner_name), Some(loser_name), Some(score)) = (
            get("tourney_id"),
            get("match_num"),
            get("winner_name"),
            get("loser_name"),
            get("score"),
        ) else {
            continue;
        };
        if score.contains("W/O") || score.contains("DEF") {
            continue;
        }
        let sets = parse_sets(score);
        if sets.is_empty() {
            continue;
        }

        let Some(tourney_date) =
            get("tourney_date").and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok())
        else {
            continue;
        };
        let offset = round_day_offset(
            get("round").unwrap_or(""),
            get("draw_size").and_then(|d| d.parse().ok()),
        );
        let match_date = (tourney_date + Duration::days(offset))
            .and_time(NaiveTime::from_hms_opt(DEFAULT_MATCH_HOUR, 0, 0).unwrap())
            .and_utc();

        let participant = |prefix: &str, player_name: &str| {
            let mut p = IngestedParticipant::new(player_name, None);
            p.metadata = json!({
                "hand": get(&format!("{}_hand", prefix)),
                "ioc": get(&format!("{}_ioc", prefix)),
                "sackmann_id": get(&format!("{}_id", prefix)),
            });
            p
        };
        let winner = participant("winner", winner_name);
        let loser = participant("loser", loser_name);
        let winner_aces = get("w_ace").and_then(|a| a.parse::<u16>().ok());
        let loser_aces = get("l_ace").and_then(|a| a.parse::<u16>().ok());

        // Losowanie stron: zwycięzca raz jest gospodarzem, raz gościem.
        let home_wins = winner_is_home(&format!("{}-{}", tourney_id, match_num));
        let (home, away, home_aces, away_aces, sets) = if home_wins {
            (winner, loser, winner_aces, loser_aces, sets)
        } else {
            // Wyniki setów odwracamy, aby były zapisane z perspektywy gospodarza.
            let flipped = sets.into_iter().map(|(w, l)| (l, w)).collect();
            (loser, winner, loser_aces, winner_aces, flipped)
        };

        let home_sets = sets.iter().filter(|(h, a)| h > a).count() as i32;
        let away_sets = sets.iter().filter(|(h, a)| a > h).count() as i32;

        result.push(IngestedMatch {
            sport: "TENNIS",
            competition_code: code.to_string(),
            competition_name: name.to_string(),
            competition_country: Some("International".to_string()),
            season: tourney_date.year().to_string(),
            match_date,
            status: "FINISHED".to_string(),
            home,
            away,
            home_score: Some(home_sets),
            away_score: Some(away_sets),
            winner: Some(if home_wins { "HOME" } else { "AWAY" }.to_string()),
            details: MatchDetails::Tennis {
                surface: get("surface").unwrap_or("Unknown").to_string(),
                best_of: get("best_of").and_then(|b| b.parse().ok()).unwrap_or(3),
                sets,
                home_aces,
                away_aces,
            },
            external_ids: json!({ "sackmann": format!("{}-{}", tourney_id, match_num) }),
        });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// ESPN (bieżące drabinki)
// ---------------------------------------------------------------------------

/// Parsuje datę ESPN; API zwraca zarówno pełny RFC 3339, jak i skrócony zapis bez sekund
/// (`2025-01-20T11:00Z`).
fn parse_espn_date(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%MZ")
                .ok()
                .map(|d| d.and_utc())
        })
}

/// Parsuje odpowiedź `/sports/tennis/{atp|wta}/scoreboard`.
///
/// Struktura: turniej (`events`) → grupa (`groupings`, bierzemy tylko single) → mecz
/// (`competitions`) → dwóch zawodników (`competitors`). Format jest nieoficjalny, dlatego
/// wszystko czytamy defensywnie, a rekordy o nieoczekiwanym kształcie pomijamy.
pub fn parse_espn_scoreboard(json: &str, tour: &str) -> AppResult<Vec<IngestedMatch>> {
    let (code, name) = competition_for_tour(tour)
        .ok_or_else(|| AppError::BadRequest(format!("Nieznany tour tenisowy: {}", tour)))?;
    let root: Value =
        serde_json::from_str(json).map_err(|e| AppError::ExternalData(format!("ESPN: {}", e)))?;

    let mut result = Vec::new();
    for event in root["events"].as_array().into_iter().flatten() {
        for grouping in event["groupings"].as_array().into_iter().flatten() {
            let slug = grouping["grouping"]["slug"].as_str().unwrap_or("");
            if !slug.contains("singles") {
                continue;
            }

            for competition in grouping["competitions"].as_array().into_iter().flatten() {
                let Some(competitors) = competition["competitors"].as_array() else {
                    continue;
                };
                let [first, second] = competitors.as_slice() else {
                    continue;
                };
                let player_name = |c: &Value| {
                    c["athlete"]["displayName"]
                        .as_str()
                        .or_else(|| c["athlete"]["fullName"].as_str())
                        .filter(|n| !n.eq_ignore_ascii_case("tbd"))
                        .map(str::to_string)
                };
                let (Some(home_name), Some(away_name)) = (player_name(first), player_name(second))
                else {
                    continue;
                };
                let Some(match_date) = competition["date"].as_str().and_then(parse_espn_date)
                else {
                    continue;
                };

                let status = match competition["status"]["type"]["state"].as_str() {
                    Some("in") => "LIVE",
                    Some("post") => "FINISHED",
                    _ => "SCHEDULED",
                };

                // Gemy w kolejnych setach z `linescores` obu zawodników.
                let games = |c: &Value| -> Vec<u8> {
                    c["linescores"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s["value"].as_f64())
                        .map(|v| v as u8)
                        .collect()
                };
                let (home_games, away_games) = (games(first), games(second));
                let sets: Vec<(u8, u8)> = home_games.into_iter().zip(away_games).collect();

                let winner = if status == "FINISHED" {
                    match (first["winner"].as_bool(), second["winner"].as_bool()) {
                        (Some(true), _) => Some("HOME"),
                        (_, Some(true)) => Some("AWAY"),
                        _ => None,
                    }
                } else {
                    None
                };
                // Mecz zakończony bez rozstrzygnięcia (np. brak danych) pozostaje niezapisany.
                if status == "FINISHED" && winner.is_none() {
                    continue;
                }

                let finished_sets = if status == "FINISHED" {
                    sets.clone()
                } else {
                    vec![]
                };
                let home_sets = finished_sets.iter().filter(|(h, a)| h > a).count() as i32;
                let away_sets = finished_sets.iter().filter(|(h, a)| a > h).count() as i32;
                let scores = (status == "FINISHED").then_some((home_sets, away_sets));

                let year = event["season"]["year"]
                    .as_i64()
                    .map(|y| y as i32)
                    .unwrap_or_else(|| match_date.year());

                result.push(IngestedMatch {
                    sport: "TENNIS",
                    competition_code: code.to_string(),
                    competition_name: name.to_string(),
                    competition_country: Some("International".to_string()),
                    season: year.to_string(),
                    match_date,
                    status: status.to_string(),
                    home: IngestedParticipant::new(home_name, None),
                    away: IngestedParticipant::new(away_name, None),
                    home_score: scores.map(|(h, _)| h),
                    away_score: scores.map(|(_, a)| a),
                    winner: winner.map(str::to_string),
                    details: MatchDetails::Tennis {
                        // Scoreboard nie podaje nawierzchni ani formatu meczu.
                        surface: "Unknown".to_string(),
                        best_of: 3,
                        sets: finished_sets,
                        home_aces: None,
                        away_aces: None,
                    },
                    external_ids: json!({ "espn": competition["id"] }),
                });
            }
        }
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Synchronizacja
// ---------------------------------------------------------------------------

/// Importuje tenis: opcjonalnie lata historyczne z archiwum Sackmanna, zawsze bieżące drabinki z ESPN
/// dla każdego touru z `INGEST_TENNIS_TOURS`.
pub async fn sync(state: &AppState, include_history: bool) -> AppResult<usize> {
    let current_year = Utc::now().year();
    let mut total = 0;

    for tour in &state.config.ingest_tennis_tours {
        if competition_for_tour(tour).is_none() {
            tracing::warn!(tour, "Nieznany tour tenisowy – pomijam");
            continue;
        }
        let lower = tour.to_ascii_lowercase();

        if include_history {
            let years = if state.config.ingest_tennis_years.is_empty() {
                vec![current_year - 2, current_year - 1]
            } else {
                state.config.ingest_tennis_years.clone()
            };

            for year in years {
                let url = format!(
                    "https://raw.githubusercontent.com/Aneeshers/tennis-sackmann-archive/main/{lower}/{lower}_matches_{year}.csv"
                );
                let result = async {
                    let body = fetch_text(&state.http_client, &url, &[]).await?;
                    let matches = parse_sackmann_csv(&body, tour)?;
                    store_matches(&state.db, &matches).await
                }
                .await;

                match result {
                    Ok(count) => {
                        tracing::info!(tour, year, count, "Sackmann: zapisano mecze");
                        total += count;
                    }
                    Err(err) => tracing::warn!(tour, year, "Sackmann: pominięto rok: {}", err),
                }
            }
        }

        let url =
            format!("https://site.api.espn.com/apis/site/v2/sports/tennis/{lower}/scoreboard");
        let body = fetch_text(&state.http_client, &url, &[]).await?;
        let matches = parse_espn_scoreboard(&body, tour)?;
        total += store_matches(&state.db, &matches).await?;
    }

    Ok(total)
}
