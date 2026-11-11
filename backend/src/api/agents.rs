// src/api/agents.rs
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::{
    error::{AppError, AppResult},
    models::bet::{Agent, Bet},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_agents).post(register_agent))
        .route("/leaderboard", get(get_leaderboard))
        .route("/{id}/bets", get(get_agent_bets))
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentDto {
    pub name: String,
    pub api_key: String,
    pub strategy_description: Option<String>,
    pub initial_bankroll: Option<Decimal>,
}

#[derive(Debug, Serialize)]
pub struct AgentCreatedResponse {
    pub id: i32,
    pub name: String,
    pub api_key: String,
    pub current_bankroll: Decimal,
}

#[derive(Debug, Serialize, FromRow)]
pub struct LeaderboardEntryDto {
    pub id: i32,
    pub name: String,
    pub strategy_description: Option<String>,
    pub initial_bankroll: Decimal,
    pub current_bankroll: Decimal,
    pub total_profit: Decimal,
    pub roi_pct: Decimal,
    pub total_bets: i64,
    pub won_bets: i64,
}

/// GET /api/v1/agents
async fn list_agents(State(state): State<AppState>) -> AppResult<Json<Vec<Agent>>> {
    let agents = sqlx::query_as::<_, Agent>("SELECT * FROM agents ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;

    Ok(Json(agents))
}

/// POST /api/v1/agents
async fn register_agent(
    State(state): State<AppState>,
    Json(payload): Json<CreateAgentDto>,
) -> AppResult<(StatusCode, Json<AgentCreatedResponse>)> {
    if payload.name.trim().is_empty() || payload.api_key.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Nazwa agenta oraz api_key nie mogą być puste".to_string(),
        ));
    }

    let bankroll = payload
        .initial_bankroll
        .unwrap_or_else(|| Decimal::from(1000));

    let agent = sqlx::query_as::<_, Agent>(
        r#"
        INSERT INTO agents (name, api_key, strategy_description, initial_bankroll, current_bankroll)
        VALUES ($1, $2, $3, $4, $4)
        RETURNING *
        "#,
    )
    .bind(payload.name.trim())
    .bind(payload.api_key.trim())
    .bind(payload.strategy_description)
    .bind(bankroll)
    .fetch_one(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(AgentCreatedResponse {
            id: agent.id,
            name: agent.name,
            api_key: agent.api_key,
            current_bankroll: agent.current_bankroll,
        }),
    ))
}

/// GET /api/v1/agents/leaderboard
async fn get_leaderboard(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<LeaderboardEntryDto>>> {
    let leaderboard = sqlx::query_as::<_, LeaderboardEntryDto>(
        r#"
        SELECT 
            a.id,
            a.name,
            a.strategy_description,
            a.initial_bankroll,
            a.current_bankroll,
            (a.current_bankroll - a.initial_bankroll) AS total_profit,
            ROUND(((a.current_bankroll - a.initial_bankroll) / NULLIF(a.initial_bankroll, 0)) * 100, 2) AS roi_pct,
            COUNT(b.id) AS total_bets,
            COUNT(b.id) FILTER (WHERE b.status = 'WON') AS won_bets
        FROM agents a
        LEFT JOIN bets b ON b.agent_id = a.id
        GROUP BY a.id
        ORDER BY a.current_bankroll DESC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(leaderboard))
}

/// GET /api/v1/agents/:id/bets
async fn get_agent_bets(
    State(state): State<AppState>,
    Path(agent_id): Path<i32>,
) -> AppResult<Json<Vec<Bet>>> {
    let bets =
        sqlx::query_as::<_, Bet>("SELECT * FROM bets WHERE agent_id = $1 ORDER BY placed_at DESC")
            .bind(agent_id)
            .fetch_all(&state.db)
            .await?;

    Ok(Json(bets))
}
