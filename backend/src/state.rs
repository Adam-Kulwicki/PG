// src/state.rs
//! Współdzielony stan aplikacji udostępniany każdemu handlerowi Axum.

use crate::config::Config;
use crate::ws::protocol::WsEnvelope;
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::{atomic::AtomicBool, Arc};
use tokio::sync::{broadcast, RwLock};

/// Stan klonowany do każdego handlera (wszystkie pola są tanie w klonowaniu).
#[derive(Clone)]
pub struct AppState {
    /// Pula połączeń z PostgreSQL.
    pub db: PgPool,
    /// Konfiguracja wczytana przy starcie.
    pub config: Arc<Config>,
    /// Klient HTTP do zewnętrznych API sportowych.
    pub http_client: reqwest::Client,
    /// Kanał broadcast do WebSocket Hub – każde połączenie agenta subskrybuje go przez `subscribe()`.
    pub ws_tx: broadcast::Sender<WsEnvelope>,
    /// Mecze historyczne (FINISHED), na które agenci mogą w tej chwili obstawiać w ramach
    /// trwającego backtestu (aktualny "dzień" symulacji). Poza backtestem zbiór jest pusty.
    pub open_backtest_matches: Arc<RwLock<HashSet<i32>>>,
    /// Flaga zabezpieczająca przed równoległym uruchomieniem dwóch backtestów.
    pub backtest_running: Arc<AtomicBool>,
}

impl AppState {
    /// Tworzy stan z gotowej puli bazy i konfiguracji (buduje też kanał broadcast i klienta HTTP).
    pub fn new(db: PgPool, config: Config) -> Self {
        // Bufor kanału broadcast mieści 1024 wiadomości; wolniejsi odbiorcy dostają `Lagged`.
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
            open_backtest_matches: Arc::new(RwLock::new(HashSet::new())),
            backtest_running: Arc::new(AtomicBool::new(false)),
        }
    }
}
