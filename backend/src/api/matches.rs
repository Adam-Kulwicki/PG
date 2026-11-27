// src/api/matches.rs
//! Pobieranie i filtrowanie meczów dla silników ML oraz frontendu.

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::{
    error::{AppError, AppResult},
    models::match_model::MatchWithDetailsDto,
    state::AppState,
};

/// Maksymalna liczba meczów zwracana w pojedynczej odpowiedzi `GET /api/v1/matches`.
pub const MAX_MATCHES_LIMIT: i64 = 5000;

/// Trasy: `GET /` (lista z filtrami) i `GET /{id}` (szczegóły meczu).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_matches))
        .route("/{id}", get(get_match_by_id))
}

/// Opcjonalne filtry z query stringa; brak filtra = brak ograniczenia.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MatchFilter {
    /// Dyscyplina: "FOOTBALL", "BASKETBALL", "TENNIS" lub "CS2" (wielkość liter bez znaczenia).
    pub sport: Option<String>,
    /// Status meczu, np. "SCHEDULED" lub "FINISHED".
    pub status: Option<String>,
    /// Sezon w formacie używanym w bazie, np. "2026-2027".
    pub season: Option<String>,
    /// Maksymalna liczba wyników (ograniczona do [`MAX_MATCHES_LIMIT`]; brak = wszystkie).
    pub limit: Option<i64>,
    /// Przesunięcie wyników do paginacji (domyślnie `0`).
    pub offset: Option<i64>,
    /// Dolna granica daty meczu w UTC (`match_date >= date_from`).
    pub date_from: Option<DateTime<Utc>>,
    /// Górna granica daty meczu w UTC (`match_date <= date_to`).
    pub date_to: Option<DateTime<Utc>>,
}

/// Normalizuje parametry paginacji: `limit` mieści się w `1..=5000`, a `offset >= 0`.
pub fn normalize_pagination(limit: Option<i64>, offset: Option<i64>) -> (Option<i64>, i64) {
    (
        limit.map(|l| l.clamp(1, MAX_MATCHES_LIMIT)),
        offset.unwrap_or(0).max(0),
    )
}

/// GET /api/v1/matches – lista meczów, opcjonalnie filtrowana (`?sport=...&status=...&season=...&limit=...&offset=...&date_from=...&date_to=...`).
#[utoipa::path(
    get,
    path = "/api/v1/matches",
    tag = "Matches",
    params(MatchFilter),
    responses(
        (status = 200, description = "Lista meczów z kursami MARKET_AVG", body = [MatchWithDetailsDto]),
        (status = 400, description = "Niepoprawne parametry filtra", body = crate::error::ErrorResponse),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn list_matches(
    State(state): State<AppState>,
    Query(filter): Query<MatchFilter>,
) -> AppResult<Json<Vec<MatchWithDetailsDto>>> {
    let (limit, offset) = normalize_pagination(filter.limit, filter.offset);

    // Mecz + nazwy drużyn + kurs MARKET_AVG (LEFT JOIN – mecz może nie mieć jeszcze kursów).
    // Filtry `$n IS NULL OR ...` pozwalają użyć jednego zapytania dla każdej kombinacji parametrów.
    let matches = sqlx::query_as::<_, MatchWithDetailsDto>(
        r#"
        SELECT 
            m.id,
            c.sport,
            c.name AS competition_name,
            m.season,
            m.match_date,
            m.status,
            m.home_participant_id,
            p_home.name AS home_name,
            m.away_participant_id,
            p_away.name AS away_name,
            m.home_score,
            m.away_score,
            m.winner,
            m.match_details,
            o.odds_home,
            o.odds_draw,
            o.odds_away
        FROM matches m
        JOIN competitions c ON m.competition_id = c.id
        JOIN participants p_home ON m.home_participant_id = p_home.id
        JOIN participants p_away ON m.away_participant_id = p_away.id
        LEFT JOIN match_odds o ON o.match_id = m.id AND o.bookmaker = 'MARKET_AVG'
        WHERE ($1::VARCHAR IS NULL OR c.sport = UPPER($1))
          AND ($2::VARCHAR IS NULL OR m.status = UPPER($2))
          AND ($3::VARCHAR IS NULL OR m.season = $3)
          AND ($4::TIMESTAMPTZ IS NULL OR m.match_date >= $4)
          AND ($5::TIMESTAMPTZ IS NULL OR m.match_date <= $5)
        ORDER BY m.match_date ASC, m.id ASC
        LIMIT $6 OFFSET $7
        "#,
    )
    .bind(filter.sport)
    .bind(filter.status)
    .bind(filter.season)
    .bind(filter.date_from)
    .bind(filter.date_to)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(matches))
}

/// GET /api/v1/matches/{id} – szczegóły jednego meczu; nieistniejący ID daje 404.
#[utoipa::path(
    get,
    path = "/api/v1/matches/{id}",
    tag = "Matches",
    params(
        ("id" = i32, Path, description = "ID meczu")
    ),
    responses(
        (status = 200, description = "Szczegóły meczu", body = MatchWithDetailsDto),
        (status = 404, description = "Mecz nie istnieje", body = crate::error::ErrorResponse),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_match_by_id(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<MatchWithDetailsDto>> {
    // Takie samo złączenie jak w liście, ale dla jednego meczu; brak wiersza oznacza 404.
    let match_item = sqlx::query_as::<_, MatchWithDetailsDto>(
        r#"
        SELECT 
            m.id,
            c.sport,
            c.name AS competition_name,
            m.season,
            m.match_date,
            m.status,
            m.home_participant_id,
            p_home.name AS home_name,
            m.away_participant_id,
            p_away.name AS away_name,
            m.home_score,
            m.away_score,
            m.winner,
            m.match_details,
            o.odds_home,
            o.odds_draw,
            o.odds_away
        FROM matches m
        JOIN competitions c ON m.competition_id = c.id
        JOIN participants p_home ON m.home_participant_id = p_home.id
        JOIN participants p_away ON m.away_participant_id = p_away.id
        LEFT JOIN match_odds o ON o.match_id = m.id AND o.bookmaker = 'MARKET_AVG'
        WHERE m.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Mecz o ID {} nie istnieje", id)))?;

    Ok(Json(match_item))
}
