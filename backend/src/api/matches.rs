// src/api/matches.rs
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;

use crate::{
    error::{AppError, AppResult},
    models::match_model::MatchWithDetailsDto,
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_matches))
        .route("/:id", get(get_match_by_id))
}

#[derive(Debug, Deserialize)]
pub struct MatchFilter {
    pub sport: Option<String>,  // np. "FOOTBALL", "BASKETBALL", "TENNIS"
    pub status: Option<String>, // np. "SCHEDULED", "FINISHED"
    pub season: Option<String>, // np. "2026-2027"
}

/// GET /api/v1/matches?sport=...&status=...
async fn list_matches(
    State(state): State<AppState>,
    Query(filter): Query<MatchFilter>,
) -> AppResult<Json<Vec<MatchWithDetailsDto>>> {
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
        ORDER BY m.match_date ASC
        "#,
    )
    .bind(filter.sport)
    .bind(filter.status)
    .bind(filter.season)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(matches))
}

/// GET /api/v1/matches/:id
async fn get_match_by_id(
    State(state): State<AppState>,
    Path(id): Path<i32>,
) -> AppResult<Json<MatchWithDetailsDto>> {
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
