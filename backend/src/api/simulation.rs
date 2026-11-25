// src/api/simulation.rs
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    domain::{
        backtester::{
            self, BacktestLeaderboardEntry, BacktestParams, BacktestPlan, BacktestReport,
            BacktestRunSummary, VirtualBet,
        },
        settlement,
    },
    error::{AppError, AppResult},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settle", post(settle))
        .route("/backtest", post(backtest).get(get_latest_backtest))
        .route("/backtest/runs", get(list_backtest_runs))
        .route("/backtest/runs/{run_id}", get(get_backtest_run))
        .route(
            "/backtest/runs/{run_id}/leaderboard",
            get(get_backtest_leaderboard),
        )
        .route("/backtest/runs/{run_id}/bets", get(get_backtest_bets))
}

/// Wynik rozliczenia zaległych kuponów (`POST /api/v1/simulation/settle`).
#[derive(Debug, Serialize, ToSchema)]
pub struct SettleResponse {
    pub settled_bets: usize,
}

/// Opcjonalny filtr po agencie dla `GET /api/v1/simulation/backtest/runs/{run_id}/bets`.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BacktestBetsQuery {
    /// Opcjonalny identyfikator agenta filtrujący zwrócone kupony.
    pub agent_id: Option<i32>,
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
/// Uruchamia symulację historyczną w tle na izolowanych wirtualnych portfelach – `202 Accepted` z planem przebiegu.
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

/// GET /api/v1/simulation/backtest
/// Zwraca raport aktualnego lub ostatniego przebiegu backtestu (z rankingiem i kuponami wirtualnymi).
#[utoipa::path(
    get,
    path = "/api/v1/simulation/backtest",
    tag = "Simulation",
    responses(
        (status = 200, description = "Raport aktualnego lub ostatniego backtestu", body = BacktestReport),
        (status = 404, description = "Nie uruchomiono jeszcze żadnego backtestu", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_latest_backtest(State(state): State<AppState>) -> AppResult<Json<BacktestReport>> {
    let report = state.backtest.latest_report().await.ok_or_else(|| {
        AppError::NotFound("Nie uruchomiono jeszcze żadnego backtestu".to_string())
    })?;
    Ok(Json(report))
}

/// GET /api/v1/simulation/backtest/runs
/// Zwraca listę zapisanych w pamięci przebiegów backtestu (od najnowszego).
#[utoipa::path(
    get,
    path = "/api/v1/simulation/backtest/runs",
    tag = "Simulation",
    responses(
        (status = 200, description = "Lista przebiegów backtestu", body = [BacktestRunSummary])
    )
)]
pub async fn list_backtest_runs(
    State(state): State<AppState>,
) -> AppResult<Json<Vec<BacktestRunSummary>>> {
    Ok(Json(state.backtest.list_runs().await))
}

/// GET /api/v1/simulation/backtest/runs/{run_id}
/// Zwraca pełny raport wskazanego przebiegu backtestu.
#[utoipa::path(
    get,
    path = "/api/v1/simulation/backtest/runs/{run_id}",
    tag = "Simulation",
    params(
        ("run_id" = u64, Path, description = "ID uruchomienia backtestu")
    ),
    responses(
        (status = 200, description = "Pełny raport backtestu", body = BacktestReport),
        (status = 404, description = "Backtest o podanym run_id nie istnieje", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_backtest_run(
    State(state): State<AppState>,
    Path(run_id): Path<u64>,
) -> AppResult<Json<BacktestReport>> {
    let report =
        state.backtest.get_report(run_id).await.ok_or_else(|| {
            AppError::NotFound(format!("Backtest o run_id {} nie istnieje", run_id))
        })?;
    Ok(Json(report))
}

/// GET /api/v1/simulation/backtest/runs/{run_id}/leaderboard
/// Zwraca ranking wirtualnych portfeli agentów dla wskazanego przebiegu backtestu.
#[utoipa::path(
    get,
    path = "/api/v1/simulation/backtest/runs/{run_id}/leaderboard",
    tag = "Simulation",
    params(
        ("run_id" = u64, Path, description = "ID uruchomienia backtestu")
    ),
    responses(
        (status = 200, description = "Ranking wirtualnych portfeli dla przebiegu backtestu", body = [BacktestLeaderboardEntry]),
        (status = 404, description = "Backtest o podanym run_id nie istnieje", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_backtest_leaderboard(
    State(state): State<AppState>,
    Path(run_id): Path<u64>,
) -> AppResult<Json<Vec<BacktestLeaderboardEntry>>> {
    let leaderboard = state
        .backtest
        .get_leaderboard(run_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("Backtest o run_id {} nie istnieje", run_id)))?;
    Ok(Json(leaderboard))
}

/// GET /api/v1/simulation/backtest/runs/{run_id}/bets
/// Zwraca wirtualne zakłady postawione w danym przebiegu backtestu (opcjonalnie dla jednego agenta).
#[utoipa::path(
    get,
    path = "/api/v1/simulation/backtest/runs/{run_id}/bets",
    tag = "Simulation",
    params(
        ("run_id" = u64, Path, description = "ID uruchomienia backtestu"),
        BacktestBetsQuery
    ),
    responses(
        (status = 200, description = "Lista wirtualnych kuponów z backtestu", body = [VirtualBet]),
        (status = 404, description = "Backtest o podanym run_id nie istnieje", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_backtest_bets(
    State(state): State<AppState>,
    Path(run_id): Path<u64>,
    Query(query): Query<BacktestBetsQuery>,
) -> AppResult<Json<Vec<VirtualBet>>> {
    let bets = state
        .backtest
        .get_bets(run_id, query.agent_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("Backtest o run_id {} nie istnieje", run_id)))?;
    Ok(Json(bets))
}
