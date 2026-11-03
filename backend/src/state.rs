// src/state.rs
use crate::config::Config;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::broadcast;

// Tymczasowy placeholder na potrzeby Fazy 1 (docelowo w src/ws/protocol.rs)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerMessage {
    Ping,
    // W Fazie 2 dodasz tu: Welcome, MatchOpportunity, BetResult, MatchSettled
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
    pub ws_tx: broadcast::Sender<ServerMessage>,
}

impl AppState {
    pub fn new(db: PgPool, config: Config) -> Self {
        // Pojemność bufora na 1024 wiadomości w kanale broadcastowym
        let (ws_tx, _ws_rx) = broadcast::channel(1024);

        let http_client = reqwest::Client::builder()
            .user_agent("SportsBettingExchange-StudentProject/1.0")
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Nie udało się utworzyć klienta HTTP reqwest");

        Self {
            db,
            config: Arc::new(config),
            http_client,
            ws_tx,
        }
    }
}
