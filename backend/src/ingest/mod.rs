// src/ingest/mod.rs
//! Warstwa ETL: pobieranie danych sportowych z zewnętrznych API i zapis do PostgreSQL.
//!
//! Każde źródło (`football`, `basketball`, `tennis`) tłumaczy odpowiedź dostawcy na wspólny
//! typ [`IngestedMatch`], a [`store_matches`] zapisuje go idempotentnie
//! (`INSERT ... ON CONFLICT DO UPDATE`), więc import można bezpiecznie powtarzać.
//!
//! Uwaga: żadne ze źródeł nie dostarcza kursów bukmacherskich, dlatego zaimportowane mecze
//! nie mają wpisów w `match_odds` i nie można na nie obstawiać, dopóki kursy nie zostaną dodane.

pub mod basketball;
pub mod football;
pub mod tennis;

use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Datelike, Utc};
use serde_json::Value;
use sqlx::{types::Json, PgConnection, PgPool};

use crate::{
    error::{AppError, AppResult},
    models::match_model::MatchDetails,
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

/// Skrócona etykieta sezonu używana przez OpenFootball, np. `2024-25`.
pub fn short_season_label(start_year: i32) -> String {
    format!("{}-{:02}", start_year, (start_year + 1) % 100)
}

/// Pełna etykieta sezonu zapisywana w bazie, np. `2024-2025`.
pub fn full_season_label(start_year: i32) -> String {
    format!("{}-{}", start_year, start_year + 1)
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

/// Zapisuje mecze w jednej transakcji i zwraca liczbę przetworzonych rekordów.
///
/// Rozgrywki i uczestnicy są tworzeni w razie potrzeby, a mecze aktualizowane po kluczu
/// `(competition_id, season, match_date, home, away)`. Zakończony mecz nie jest cofany do
/// statusu niezakończonego przez starsze dane.
pub async fn store_matches(db: &PgPool, items: &[IngestedMatch]) -> AppResult<usize> {
    if items.is_empty() {
        return Ok(0);
    }

    let mut tx = db.begin().await?;
    // Bufory zapobiegają wielokrotnemu odpytywaniu bazy o te same rozgrywki i uczestników.
    let mut competitions: HashMap<String, i32> = HashMap::new();
    let mut participants: HashMap<(&'static str, String), i32> = HashMap::new();

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

    tx.commit().await?;
    Ok(items.len())
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
