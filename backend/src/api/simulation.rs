// src/api/simulation.rs
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde_json::{json, Value};

use crate::{
    domain::{
        backtester::{self, BacktestParams, BacktestPlan},
        settlement,
    },
    error::AppResult,
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settle", post(settle))
        .route("/backtest", post(backtest))
}

/// POST /api/v1/simulation/settle
/// Rozlicza wszystkie nierozliczone zakłady na zakończone mecze i powiadamia agentów.
async fn settle(State(state): State<AppState>) -> AppResult<Json<Value>> {
    let settled = settlement::settle_matches(&state.db, None).await?;
    let count = settled.len();
    settlement::publish(&state.ws_tx, settled);

    Ok(Json(json!({ "settled_bets": count })))
}

/// POST /api/v1/simulation/backtest
/// Uruchamia symulację historyczną w tle – `202 Accepted` z planem przebiegu.
async fn backtest(
    State(state): State<AppState>,
    Json(params): Json<BacktestParams>,
) -> AppResult<(StatusCode, Json<BacktestPlan>)> {
    let plan = backtester::start(state, params).await?;
    Ok((StatusCode::ACCEPTED, Json(plan)))
}
