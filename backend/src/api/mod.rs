// src/api/mod.rs
pub mod agents;
pub mod matches;

use crate::state::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/matches", matches::router())
        .nest("/agents", agents::router())
}
