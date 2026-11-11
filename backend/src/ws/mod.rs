// src/ws/mod.rs
pub mod handler;
pub mod protocol;
pub mod session;

use crate::state::AppState;
use axum::{routing::get, Router};

/// Router WebSocket montowany pod `/ws` (endpoint: `/ws/agent`).
pub fn router() -> Router<AppState> {
    Router::new().route("/agent", get(handler::ws_agent))
}
