// src/ingest/mod.rs
//! Warstwa ETL: pobieranie danych sportowych z zewnętrznych API i zapis do PostgreSQL.
//!
//! Każde źródło (`football`, `basketball`, `tennis`) tłumaczy odpowiedź dostawcy na wspólny
//! typ [`IngestedMatch`], a [`store_matches`] zapisuje go idempotentnie
//! (`INSERT ... ON CONFLICT DO UPDATE` oraz dopasowanie po `external_ids` i oknie czasowym),
//! więc import można bezpiecznie powtarzać.
//!
//! Uwaga: żadne ze źródeł nie dostarcza kursów bukmacherskich, dlatego zaimportowane mecze
//! nie mają wpisów w `match_odds` i nie można na nie obstawiać, dopóki kursy nie zostaną dodane.

pub mod basketball;
pub mod football;
pub mod tennis;

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveTime, Utc};
use serde_json::Value;
use sqlx::{types::Json, PgConnection, PgPool};

use crate::{
    error::{AppError, AppResult},
    models::match_model::{FootballPreMatchElo, FootballTeamStats, MatchDetails},
    state::AppState,
};

/// Obsługiwane dyscypliny w kolejności, w jakiej są importowane w każdym cyklu.
pub const SPORTS: [&str; 3] = ["football", "basketball", "tennis"];

/// Ile razy ponawiamy zapytanie po odpowiedzi `429 Too Many Requests`.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;

/// Czas oczekiwania przed ponowieniem zapytania po `429` (limity darmowych API liczone są na minutę).
const RATE_LIMIT_WAIT: Duration = Duration::from_secs(60);

/// Uczestnik meczu w postaci niezależnej od dostawcy danych.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestedParticipant {
    pub name: String,
    pub short_code: Option<String>,
    /// Dodatkowe dane zapisywane w JSONB, np. `{"hand": "R", "ioc": "ESP"}`.
    pub metadata: Value,
}

impl IngestedParticipant {
    /// Tworzy uczestnika bez dodatkowych metadanych.
    pub fn new(name: impl Into<String>, short_code: Option<String>) -> Self {
        Self {
            name: name.into(),
            short_code,
            metadata: serde_json::json!({}),
        }
    }
}

/// Mecz znormalizowany do schematu bazy, gotowy do zapisu przez [`store_matches`].
#[derive(Debug, Clone)]
pub struct IngestedMatch {
    /// "FOOTBALL", "BASKETBALL" lub "TENNIS".
    pub sport: &'static str,
    /// Unikalny kod rozgrywek (`competitions.external_code`), np. "PL".
    pub competition_code: String,
    pub competition_name: String,
    pub competition_country: Option<String>,
    pub season: String,
    pub match_date: DateTime<Utc>,
    /// "SCHEDULED", "LIVE", "FINISHED", "POSTPONED" lub "CANCELLED".
    pub status: String,
    pub home: IngestedParticipant,
    pub away: IngestedParticipant,
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    /// "HOME", "AWAY" lub "DRAW" – tylko dla zakończonych meczów.
    pub winner: Option<String>,
    pub details: MatchDetails,
    /// Identyfikatory meczu u dostawcy, np. `{"football_data": 123}`.
    pub external_ids: Value,
}

/// Wyznacza zwycięzcę na podstawie wyniku; remis jest możliwy tylko w dyscyplinach, które go dopuszczają.
pub fn winner_from_scores(home: i32, away: i32, allow_draw: bool) -> Option<&'static str> {
    match home.cmp(&away) {
        std::cmp::Ordering::Greater => Some("HOME"),
        std::cmp::Ordering::Less => Some("AWAY"),
        std::cmp::Ordering::Equal => allow_draw.then_some("DRAW"),
    }
}

/// Rok rozpoczęcia aktualnego sezonu europejskiego (od sierpnia zaczyna się nowy sezon).
pub fn season_start_year(now: DateTime<Utc>) -> i32 {
    if now.month() >= 8 {
        now.year()
    } else {
        now.year() - 1
    }
}

/// Skrócona etykieta sezonu, np. `2024-25` (używana m.in. przez `stats.nba.com`).
pub fn short_season_label(start_year: i32) -> String {
    format!("{}-{:02}", start_year, (start_year + 1) % 100)
}

/// Pełna etykieta sezonu zapisywana w bazie, np. `2024-2025`.
pub fn full_season_label(start_year: i32) -> String {
    format!("{}-{}", start_year, start_year + 1)
}

/// Przekształca etykietę sezonu na format zapisywany w bazie: `2023-24` lub `2023/24` → `2023-2024`.
/// Etykiety, których nie rozpoznajemy, zwracane są bez zmian.
pub fn normalize_season_label(raw: &str) -> String {
    let normalized = raw.trim().replace('/', "-");
    let Some((start, end)) = normalized.split_once('-') else {
        return normalized;
    };
    match (start.parse::<i32>(), end.len()) {
        (Ok(year), 2) | (Ok(year), 4) => full_season_label(year),
        _ => normalized,
    }
}

/// Pobiera treść odpowiedzi HTTP, ponawiając zapytanie po `429` (limity darmowych kluczy API).
pub async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
) -> AppResult<String> {
    let mut attempt = 0;
    loop {
        let mut request = client.get(url);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
            && attempt < MAX_RATE_LIMIT_RETRIES
        {
            attempt += 1;
            tracing::warn!(
                url,
                attempt,
                "Limit zapytań (429) – czekam przed ponowieniem"
            );
            tokio::time::sleep(RATE_LIMIT_WAIT).await;
            continue;
        }

        return Ok(response.error_for_status()?.text().await?);
    }
}

/// Pobiera treść odpowiedzi HTTP z własnym limitem czasu i zdefiniowaną ścieżką ponowień.
///
/// Ponawia zapytanie przy timeoutcie, błędzie połączenia, statusie `429` lub błędzie
/// serwera (`5xx`), czekając kolejne interwały z `retry_delays`.
pub async fn fetch_text_with_timeout(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
    retry_delays: &[Duration],
) -> AppResult<String> {
    let mut attempt = 0;
    loop {
        let mut request = client.get(url).timeout(timeout);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let retry_reason = match request.send().await {
            Ok(response) => {
                let status = response.status();
                if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
                    && attempt < retry_delays.len()
                {
                    format!("HTTP {}", status.as_u16())
                } else {
                    match response.error_for_status()?.text().await {
                        Ok(body) => return Ok(body),
                        Err(err) if err.is_timeout() && attempt < retry_delays.len() => {
                            err.to_string()
                        }
                        Err(err) => return Err(err.into()),
                    }
                }
            }
            Err(err) if (err.is_timeout() || err.is_connect()) && attempt < retry_delays.len() => {
                err.to_string()
            }
            Err(err) => return Err(err.into()),
        };

        let delay = retry_delays[attempt];
        attempt += 1;
        tracing::warn!(
            url,
            attempt,
            delay_secs = delay.as_secs_f64(),
            reason = %retry_reason,
            "Błąd zapytania HTTP – ponawiam po przerwie"
        );
        tokio::time::sleep(delay).await;
    }
}

// ---------------------------------------------------------------------------
// Deduplikacja i zapis do bazy
// ---------------------------------------------------------------------------

/// Istniejący rekord meczu pobrany z bazy podczas deduplikacji w [`store_matches`].
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExistingMatchRow {
    pub id: i32,
    pub match_date: DateTime<Utc>,
    pub status: String,
    pub home_participant_id: i32,
    pub away_participant_id: i32,
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    pub winner: Option<String>,
    pub match_details: Json<MatchDetails>,
    pub external_ids: Json<Value>,
}

/// Pola meczu po scaleniu istniejącego wiersza z nowymi danymi z API.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedMatchUpdate {
    pub match_date: DateTime<Utc>,
    pub status: String,
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    pub winner: Option<String>,
    pub details: MatchDetails,
}

/// Wybiera dokładniejszy czas rozpoczęcia meczu: godzina różna od `00:00:00` UTC wygrywa
/// z samą datą (np. `datetime` z BALLDONTLIE nad `GAME_DATE` ze `stats.nba.com`).
pub fn choose_match_date(existing: DateTime<Utc>, incoming: DateTime<Utc>) -> DateTime<Utc> {
    if existing.time() != NaiveTime::MIN && incoming.time() == NaiveTime::MIN {
        existing
    } else {
        incoming
    }
}

/// Scala opcjonalne statystyki drużyny piłkarskiej na poziomie poszczególnych pól.
fn merge_football_team_stats(
    existing: Option<Box<FootballTeamStats>>,
    incoming: Option<Box<FootballTeamStats>>,
) -> Option<Box<FootballTeamStats>> {
    match (existing, incoming) {
        (None, None) => None,
        (Some(old), None) => Some(old),
        (None, Some(new)) => Some(new),
        (Some(old), Some(new)) => Some(Box::new(FootballTeamStats {
            xg: new.xg.or(old.xg),
            shots: new.shots.or(old.shots),
            shots_on_target: new.shots_on_target.or(old.shots_on_target),
            corners: new.corners.or(old.corners),
            fouls: new.fouls.or(old.fouls),
            yellow_cards: new.yellow_cards.or(old.yellow_cards),
            red_cards: new.red_cards.or(old.red_cards),
        })),
    }
}

/// Scala przedmeczowe ratingi ClubElo na poziomie poszczególnych pól.
fn merge_football_pre_match_elo(
    existing: Option<Box<FootballPreMatchElo>>,
    incoming: Option<Box<FootballPreMatchElo>>,
) -> Option<Box<FootballPreMatchElo>> {
    match (existing, incoming) {
        (None, None) => None,
        (Some(old), None) => Some(old),
        (None, Some(new)) => Some(new),
        (Some(old), Some(new)) => {
            let home_elo = new.home_elo.or(old.home_elo);
            let away_elo = new.away_elo.or(old.away_elo);
            let elo_diff = new
                .elo_diff
                .or(old.elo_diff)
                .or_else(|| home_elo.zip(away_elo).map(|(h, a)| h - a));
            Some(Box::new(FootballPreMatchElo {
                home_elo,
                away_elo,
                elo_diff,
                home_rank: new.home_rank.or(old.home_rank),
                away_rank: new.away_rank.or(old.away_rank),
                clubelo_prob_home: new.clubelo_prob_home.or(old.clubelo_prob_home),
                clubelo_prob_draw: new.clubelo_prob_draw.or(old.clubelo_prob_draw),
                clubelo_prob_away: new.clubelo_prob_away.or(old.clubelo_prob_away),
            }))
        }
    }
}

/// Scala szczegóły meczu z wielu źródeł (np. BALLDONTLIE + `stats.nba.com` albo
/// Football-Data.org + `football-data.co.uk` + ClubElo) niezależnie od kolejności ich uruchomienia.
pub fn merge_match_details(existing: MatchDetails, incoming: MatchDetails) -> MatchDetails {
    match (existing, incoming) {
        (
            MatchDetails::Football {
                ht_home: old_ht_home,
                ht_away: old_ht_away,
                round: old_round,
                home_stats: old_home_stats,
                away_stats: old_away_stats,
                pre_match_elo: old_pre_match_elo,
            },
            MatchDetails::Football {
                ht_home: new_ht_home,
                ht_away: new_ht_away,
                round: new_round,
                home_stats: new_home_stats,
                away_stats: new_away_stats,
                pre_match_elo: new_pre_match_elo,
            },
        ) => MatchDetails::Football {
            ht_home: new_ht_home.or(old_ht_home),
            ht_away: new_ht_away.or(old_ht_away),
            round: if new_round.is_empty() {
                old_round
            } else {
                new_round
            },
            home_stats: merge_football_team_stats(old_home_stats, new_home_stats),
            away_stats: merge_football_team_stats(old_away_stats, new_away_stats),
            pre_match_elo: merge_football_pre_match_elo(old_pre_match_elo, new_pre_match_elo),
        },
        (
            MatchDetails::Basketball {
                quarters: old_quarters,
                overtime: old_overtime,
                season_type: old_season_type,
                home_stats: old_home_stats,
                away_stats: old_away_stats,
            },
            MatchDetails::Basketball {
                quarters: new_quarters,
                overtime: new_overtime,
                season_type: new_season_type,
                home_stats: new_home_stats,
                away_stats: new_away_stats,
            },
        ) => MatchDetails::Basketball {
            quarters: if new_quarters.is_empty() {
                old_quarters
            } else {
                new_quarters
            },
            overtime: old_overtime || new_overtime,
            season_type: new_season_type.or(old_season_type),
            home_stats: new_home_stats.or(old_home_stats),
            away_stats: new_away_stats.or(old_away_stats),
        },
        (_, incoming) => incoming,
    }
}

/// Odwraca wskazanie zwycięzcy przy zamianie stron gospodarz ↔ gość.
fn flip_winner(winner: Option<&str>) -> Option<String> {
    winner.map(|w| match w {
        "HOME" => "AWAY".to_string(),
        "AWAY" => "HOME".to_string(),
        other => other.to_string(),
    })
}

/// Zamienia statystyki i punkty kwart między gospodarzem a gościem (mecze na terenie neutralnym).
fn flip_details_sides(details: MatchDetails) -> MatchDetails {
    match details {
        MatchDetails::Basketball {
            quarters,
            overtime,
            season_type,
            home_stats,
            away_stats,
        } => MatchDetails::Basketball {
            quarters: quarters.into_iter().map(|(h, a)| (a, h)).collect(),
            overtime,
            season_type,
            home_stats: away_stats,
            away_stats: home_stats,
        },
        other => other,
    }
}

/// Sprawdza, czy dwa rekordy mają ten sam klucz zewnętrzny (np. `nba_game_id`) o różnych wartościach.
fn has_conflicting_external_id(existing: &Value, incoming: &Value) -> bool {
    let (Some(a), Some(b)) = (existing.as_object(), incoming.as_object()) else {
        return false;
    };
    a.iter().any(|(key, val_a)| {
        if val_a.is_null() {
            return false;
        }
        match b.get(key) {
            Some(val_b) if !val_b.is_null() => val_a != val_b,
            _ => false,
        }
    })
}

/// Wybiera najlepiej pasujący rekord z okna ±36h, chroniąc sąsiednie mecze back-to-back
/// przed przypadkowym scaleniem.
pub fn select_matching_row(
    candidates: &[ExistingMatchRow],
    incoming: &IngestedMatch,
    home_id: i32,
    away_id: i32,
) -> Option<usize> {
    let mut best: Option<(usize, (i64, bool))> = None;

    for (idx, cand) in candidates.iter().enumerate() {
        if has_conflicting_external_id(&cand.external_ids.0, &incoming.external_ids) {
            continue;
        }

        let same_sides = cand.home_participant_id == home_id && cand.away_participant_id == away_id;
        let swapped = incoming.sport == "BASKETBALL"
            && cand.home_participant_id == away_id
            && cand.away_participant_id == home_id;
        if !same_sides && !swapped {
            continue;
        }

        // Gdy oba rekordy znają wynik, nie pozwalamy scalić dwóch różnych spotkań.
        let existing_scores = cand.home_score.zip(cand.away_score);
        let incoming_scores =
            incoming.home_score.zip(incoming.away_score).map(
                |(h, a)| {
                    if swapped {
                        (a, h)
                    } else {
                        (h, a)
                    }
                },
            );
        if let (Some(es), Some(is)) = (existing_scores, incoming_scores) {
            if es != is {
                continue;
            }
        }

        let cand_midnight = cand.match_date.time() == NaiveTime::MIN;
        let inc_midnight = incoming.match_date.time() == NaiveTime::MIN;
        let (dist, max_dist) = if incoming.sport == "BASKETBALL" && cand_midnight != inc_midnight {
            // Mecze NBA rozgrywane wieczorem czasu USA mają w UTC datę z następnej doby;
            // przesunięcie północy o +23h i okno ±12h rozdziela mecze rozgrywane dzień po dniu.
            let shift = ChronoDuration::hours(23);
            let cand_adj = if cand_midnight {
                cand.match_date + shift
            } else {
                cand.match_date
            };
            let inc_adj = if inc_midnight {
                incoming.match_date + shift
            } else {
                incoming.match_date
            };
            ((cand_adj - inc_adj).num_seconds().abs(), 12 * 3600)
        } else {
            (
                (cand.match_date - incoming.match_date).num_seconds().abs(),
                36 * 3600,
            )
        };
        if dist > max_dist {
            continue;
        }

        let candidate_key = (dist, swapped);
        let is_better = match best {
            None => true,
            Some((_, best_key)) => candidate_key < best_key,
        };
        if is_better {
            best = Some((idx, candidate_key));
        }
    }

    best.map(|(idx, _)| idx)
}

/// Przygotowuje scalone pola do `UPDATE`, zachowując kierunek stron istniejącego rekordu.
pub fn merge_existing_match(
    existing: &ExistingMatchRow,
    incoming: &IngestedMatch,
    incoming_home_id: i32,
    incoming_away_id: i32,
) -> MergedMatchUpdate {
    let swapped = existing.home_participant_id == incoming_away_id
        && existing.away_participant_id == incoming_home_id;

    let (inc_home_score, inc_away_score, inc_winner, inc_details) = if swapped {
        (
            incoming.away_score,
            incoming.home_score,
            flip_winner(incoming.winner.as_deref()),
            flip_details_sides(incoming.details.clone()),
        )
    } else {
        (
            incoming.home_score,
            incoming.away_score,
            incoming.winner.clone(),
            incoming.details.clone(),
        )
    };

    // Football-Data.org ma oficjalną godzinę `utcDate`; źródło CSV nie powinno jej nadpisywać.
    let match_date = if incoming.sport == "FOOTBALL"
        && existing.external_ids.0.get("football_data").is_some()
        && incoming.external_ids.get("football_data").is_none()
        && existing.match_date.time() != NaiveTime::MIN
    {
        existing.match_date
    } else {
        choose_match_date(existing.match_date, incoming.match_date)
    };

    // Zakończony mecz nie cofa się do starszego statusu ani wyniku, ale może przyjąć dokładniejszą
    // godzinę i brakujące szczegóły wzbogacające (np. HT, kolejkę lub rating Elo).
    let details = merge_match_details(existing.match_details.0.clone(), inc_details);
    if existing.status == "FINISHED" && incoming.status != "FINISHED" {
        return MergedMatchUpdate {
            match_date,
            status: existing.status.clone(),
            home_score: existing.home_score,
            away_score: existing.away_score,
            winner: existing.winner.clone(),
            details,
        };
    }

    MergedMatchUpdate {
        match_date,
        status: incoming.status.clone(),
        home_score: inc_home_score.or(existing.home_score),
        away_score: inc_away_score.or(existing.away_score),
        winner: inc_winner.or_else(|| existing.winner.clone()),
        details,
    }
}

/// Rozbija obiekt `external_ids` na pojedyncze pary `{klucz: wartość}` do zapytań `@>`.
fn external_id_probes(external_ids: &Value) -> Vec<Value> {
    let Some(map) = external_ids.as_object() else {
        return Vec::new();
    };
    map.iter()
        .filter(|(_, val)| !val.is_null() && !matches!(val, Value::String(s) if s.is_empty()))
        .map(|(k, v)| serde_json::json!({ k: v }))
        .collect()
}

/// Zapisuje mecze w jednej transakcji i zwraca liczbę przetworzonych rekordów.
///
/// Rozgrywki i uczestnicy są tworzeni w razie potrzeby. Przed wstawieniem nowego meczu
/// funkcja szuka istniejącego rekordu po `external_ids`, a następnie po drużynach i oknie ±36h,
/// scalając dane z różnych źródeł (np. `stats.nba.com` i BALLDONTLIE) bez duplikatów.
pub async fn store_matches(db: &PgPool, items: &[IngestedMatch]) -> AppResult<usize> {
    if items.is_empty() {
        return Ok(0);
    }

    let mut tx = db.begin().await?;
    // Bufory zapobiegają wielokrotnemu odpytywaniu bazy o te same rozgrywki i uczestników.
    let mut competitions: HashMap<String, i32> = HashMap::new();
    let mut participants: HashMap<(&'static str, String), i32> = HashMap::new();
    let mut saved = 0;

    for m in items {
        let competition_id = match competitions.get(&m.competition_code) {
            Some(id) => *id,
            None => {
                let (id,): (i32,) = sqlx::query_as(
                    r#"
                    INSERT INTO competitions (sport, name, country_or_category, external_code)
                    VALUES ($1, $2, $3, $4)
                    ON CONFLICT (external_code) DO UPDATE SET external_code = EXCLUDED.external_code
                    RETURNING id
                    "#,
                )
                .bind(m.sport)
                .bind(&m.competition_name)
                .bind(&m.competition_country)
                .bind(&m.competition_code)
                .fetch_one(&mut *tx)
                .await?;
                competitions.insert(m.competition_code.clone(), id);
                id
            }
        };

        let home_id = upsert_participant(&mut tx, &mut participants, m.sport, &m.home).await?;
        let away_id = upsert_participant(&mut tx, &mut participants, m.sport, &m.away).await?;
        if home_id == away_id {
            // Dane źródłowe z tą samą nazwą po obu stronach są błędne – pomijamy taki rekord.
            tracing::warn!(home = %m.home.name, "Pominięto mecz z identycznymi uczestnikami");
            continue;
        }

        // Krok A: dopasowanie po znanym identyfikatorze zewnętrznego dostawcy.
        let mut existing: Option<ExistingMatchRow> = None;
        for probe in external_id_probes(&m.external_ids) {
            let found: Option<ExistingMatchRow> = sqlx::query_as(
                r#"
                SELECT
                    id, match_date, status,
                    home_participant_id, away_participant_id,
                    home_score, away_score, winner,
                    match_details, external_ids
                FROM matches
                WHERE competition_id = $1
                  AND external_ids @> $2::jsonb
                LIMIT 1
                "#,
            )
            .bind(competition_id)
            .bind(Json(&probe))
            .fetch_optional(&mut *tx)
            .await?;

            if found.is_some() {
                existing = found;
                break;
            }
        }

        // Krok B: dopasowanie po parze drużyn i oknie czasowym ±36h (np. pierwszy zapis z drugiego źródła).
        if existing.is_none() {
            let candidates: Vec<ExistingMatchRow> = sqlx::query_as(
                r#"
                SELECT
                    id, match_date, status,
                    home_participant_id, away_participant_id,
                    home_score, away_score, winner,
                    match_details, external_ids
                FROM matches
                WHERE competition_id = $1
                  AND season = $2
                  AND match_date BETWEEN ($3 - INTERVAL '36 hours') AND ($3 + INTERVAL '36 hours')
                  AND (
                      (home_participant_id = $4 AND away_participant_id = $5)
                      OR ($6 AND home_participant_id = $5 AND away_participant_id = $4)
                  )
                "#,
            )
            .bind(competition_id)
            .bind(&m.season)
            .bind(m.match_date)
            .bind(home_id)
            .bind(away_id)
            .bind(m.sport == "BASKETBALL")
            .fetch_all(&mut *tx)
            .await?;

            if let Some(idx) = select_matching_row(&candidates, m, home_id, away_id) {
                existing = Some(candidates[idx].clone());
            }
        }

        if let Some(existing_row) = existing {
            let merged = merge_existing_match(&existing_row, m, home_id, away_id);
            sqlx::query(
                r#"
                UPDATE matches
                SET match_date = $1,
                    status = $2,
                    home_score = $3,
                    away_score = $4,
                    winner = $5,
                    match_details = $6,
                    external_ids = matches.external_ids || $7,
                    updated_at = NOW()
                WHERE id = $8
                "#,
            )
            .bind(merged.match_date)
            .bind(&merged.status)
            .bind(merged.home_score)
            .bind(merged.away_score)
            .bind(&merged.winner)
            .bind(Json(&merged.details))
            .bind(Json(&m.external_ids))
            .bind(existing_row.id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                r#"
                INSERT INTO matches (
                    competition_id, season, match_date, status,
                    home_participant_id, away_participant_id,
                    home_score, away_score, winner, match_details, external_ids
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                ON CONFLICT (competition_id, season, match_date, home_participant_id, away_participant_id)
                DO UPDATE SET
                    status = CASE WHEN matches.status = 'FINISHED' AND EXCLUDED.status <> 'FINISHED'
                                  THEN matches.status ELSE EXCLUDED.status END,
                    home_score = COALESCE(EXCLUDED.home_score, matches.home_score),
                    away_score = COALESCE(EXCLUDED.away_score, matches.away_score),
                    winner = COALESCE(EXCLUDED.winner, matches.winner),
                    match_details = CASE WHEN matches.status = 'FINISHED' AND EXCLUDED.status <> 'FINISHED'
                                         THEN matches.match_details ELSE EXCLUDED.match_details END,
                    external_ids = matches.external_ids || EXCLUDED.external_ids,
                    updated_at = NOW()
                "#,
            )
            .bind(competition_id)
            .bind(&m.season)
            .bind(m.match_date)
            .bind(&m.status)
            .bind(home_id)
            .bind(away_id)
            .bind(m.home_score)
            .bind(m.away_score)
            .bind(&m.winner)
            .bind(Json(&m.details))
            .bind(Json(&m.external_ids))
            .execute(&mut *tx)
            .await?;
        }
        saved += 1;
    }

    tx.commit().await?;
    Ok(saved)
}

/// Zwraca ID uczestnika, tworząc go lub uzupełniając metadane, jeśli już istnieje.
async fn upsert_participant(
    conn: &mut PgConnection,
    cache: &mut HashMap<(&'static str, String), i32>,
    sport: &'static str,
    p: &IngestedParticipant,
) -> AppResult<i32> {
    let key = (sport, p.name.clone());
    if let Some(id) = cache.get(&key) {
        return Ok(*id);
    }

    let participant_type = if sport == "TENNIS" { "PLAYER" } else { "TEAM" };
    let (id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO participants (sport, name, short_code, participant_type, metadata)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (sport, name) DO UPDATE SET
            short_code = COALESCE(EXCLUDED.short_code, participants.short_code),
            metadata = participants.metadata || EXCLUDED.metadata
        RETURNING id
        "#,
    )
    .bind(sport)
    .bind(&p.name)
    .bind(&p.short_code)
    .bind(participant_type)
    .bind(Json(&p.metadata))
    .fetch_one(conn)
    .await?;

    cache.insert(key, id);
    Ok(id)
}

/// Importuje wskazaną dyscyplinę (`football`, `basketball`, `tennis`).
/// `include_history` dodatkowo pobiera sezony historyczne.
pub async fn run_sport(state: &AppState, sport: &str, include_history: bool) -> AppResult<usize> {
    match sport {
        "football" => football::sync(state, include_history).await,
        "basketball" => basketball::sync(state, include_history).await,
        "tennis" => tennis::sync(state, include_history).await,
        other => Err(AppError::BadRequest(format!(
            "Nieznana dyscyplina '{}' (dozwolone: football, basketball, tennis)",
            other
        ))),
    }
}

/// Uruchamia w tle pętlę importu: co `INGEST_INTERVAL_MINUTES` minut synchronizuje wszystkie sporty.
///
/// Dane historyczne pobierane są raz na dyscyplinę (do pierwszego udanego importu), a awaria
/// jednego źródła nie blokuje pozostałych.
pub fn spawn_scheduler(state: AppState) {
    tokio::spawn(async move {
        let interval = Duration::from_secs(state.config.ingest_interval_minutes.max(1) * 60);
        let mut history_done = [false; SPORTS.len()];

        loop {
            for (i, sport) in SPORTS.iter().enumerate() {
                match run_sport(&state, sport, !history_done[i]).await {
                    Ok(count) => {
                        history_done[i] = true;
                        tracing::info!(sport, count, "Import zakończony");
                    }
                    Err(err) => tracing::error!(sport, "Import nieudany: {}", err),
                }
            }
            tokio::time::sleep(interval).await;
        }
    });
}
