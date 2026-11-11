// src/api/mod.rs
pub mod agents;
pub mod matches;
pub mod predictions;
pub mod simulation;

use crate::state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/matches", matches::router())
        .nest("/agents", agents::router())
        .nest("/predictions", predictions::router())
        .nest("/simulation", simulation::router())
}
