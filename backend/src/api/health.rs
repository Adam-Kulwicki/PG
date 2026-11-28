// src/api/health.rs
//! Endpoint diagnostyczny `/health`.

use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{error::AppResult, state::AppState};

/// Odpowiedź diagnostyki serwera (`GET /health`).
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    /// Ogólny status usługi (`"ok"`).
    #[schema(example = "ok")]
    pub status: String,
    /// Stan połączenia z bazą PostgreSQL (`"connected"`).
    #[schema(example = "connected")]
    pub database: String,
    /// Liczba aktywnych połączeń WebSocket subskrybujących kanał broadcast (`/ws/agent`).
    #[schema(example = 2)]
    pub connected_ws_agents: usize,
}

/// Diagnostyka serwera i połączenia z bazą danych.
///
/// Wykonuje testowe zapytanie `SELECT 1` w PostgreSQL i zwraca aktualną liczbę odbiorców
/// kanału broadcast WebSocket (`/ws/agent`).
#[utoipa::path(
    get,
    path = "/health",
    tag = "Health",
    responses(
        (status = 200, description = "Status serwera i bazy danych", body = HealthResponse),
        (status = 500, description = "Błąd połączenia z bazą", body = crate::error::ErrorResponse)
    )
)]
pub async fn health_check(State(state): State<AppState>) -> AppResult<Json<HealthResponse>> {
    // Proste zapytanie `SELECT 1` wystarcza, aby potwierdzić, że baza odpowiada.
    sqlx::query("SELECT 1").execute(&state.db).await?;

    Ok(Json(HealthResponse {
        status: "ok".to_string(),
        database: "connected".to_string(),
        connected_ws_agents: state.ws_tx.receiver_count(),
    }))
}
