// src/api/mod.rs
//! Warstwa REST API (`/api/v1/...`) – składa routery poszczególnych zasobów.
pub mod agents;
pub mod ingest;
pub mod matches;
pub mod predictions;
pub mod simulation;

use crate::state::AppState;
use axum::Router;

/// Główny router REST; każdy zasób jest zagnieżdżony pod własnym prefiksem.
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/matches", matches::router())
        .nest("/agents", agents::router())
        .nest("/ingest", ingest::router())
        .nest("/predictions", predictions::router())
        .nest("/simulation", simulation::router())
}
