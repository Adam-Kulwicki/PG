// src/api/simulation.rs
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

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

/// Wynik rozliczenia zaległych kuponów (`POST /api/v1/simulation/settle`).
#[derive(Debug, Serialize, ToSchema)]
pub struct SettleResponse {
    pub settled_bets: usize,
}

/// POST /api/v1/simulation/settle
/// Rozlicza wszystkie nierozliczone zakłady na zakończone mecze i powiadamia agentów.
#[utoipa::path(
    post,
    path = "/api/v1/simulation/settle",
    tag = "Simulation",
    responses(
        (status = 200, description = "Liczba rozliczonych zakładów", body = SettleResponse),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn settle(State(state): State<AppState>) -> AppResult<Json<SettleResponse>> {
    let settled = settlement::settle_matches(&state.db, None).await?;
    let count = settled.len();
    settlement::publish(&state.ws_tx, settled);

    Ok(Json(SettleResponse {
        settled_bets: count,
    }))
}

/// POST /api/v1/simulation/backtest
/// Uruchamia symulację historyczną w tle – `202 Accepted` z planem przebiegu.
#[utoipa::path(
    post,
    path = "/api/v1/simulation/backtest",
    tag = "Simulation",
    request_body = BacktestParams,
    responses(
        (status = 202, description = "Symulacja historyczna uruchomiona w tle", body = BacktestPlan),
        (status = 400, description = "Niepoprawne parametry symulacji", body = crate::error::ErrorResponse),
        (status = 404, description = "Brak zakończonych meczów z kursami dla wybranego sezonu", body = crate::error::ErrorResponse),
        (status = 409, description = "Inny backtest już trwa", body = crate::error::ErrorResponse),
        (status = 500, description = "Błąd bazy danych", body = crate::error::ErrorResponse)
    )
)]
pub async fn backtest(
    State(state): State<AppState>,
    Json(params): Json<BacktestParams>,
) -> AppResult<(StatusCode, Json<BacktestPlan>)> {
    let plan = backtester::start(state, params).await?;
    Ok((StatusCode::ACCEPTED, Json(plan)))
}
