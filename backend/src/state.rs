// src/state.rs
//! Współdzielony stan aplikacji udostępniany każdemu handlerowi Axum.

use crate::config::Config;
use crate::domain::backtester::BacktestStore;
use crate::ws::protocol::WsEnvelope;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Stan klonowany do każdego handlera (wszystkie pola są tanie w klonowaniu).
#[derive(Clone)]
pub struct AppState {
    /// Pula połączeń z PostgreSQL.
    pub db: PgPool,
    /// Konfiguracja wczytana przy starcie.
    pub config: Arc<Config>,
    /// Klient HTTP do zewnętrznych API sportowych.
    pub http_client: reqwest::Client,
    /// Dedykowany klient HTTP dla `stats.nba.com` (z opcjonalnym proxy `NBA_API_PROXY`).
    pub nba_stats_client: reqwest::Client,
    /// Kanał broadcast do WebSocket Hub – każde połączenie agenta subskrybuje go przez `subscribe()`.
    pub ws_tx: broadcast::Sender<WsEnvelope>,
    /// Izolowany magazyn wirtualnych portfeli i wyników backtestingu (w pamięci serwera).
    pub backtest: Arc<BacktestStore>,
}

/// Buduje klienta HTTP dla `stats.nba.com` z opcjonalnym proxy (`NBA_API_PROXY`).
/// W razie niepoprawnego URL proxy wypisuje ostrzeżenie (bez danych uwierzytelniających) i wraca do klienta bazowego.
pub fn build_nba_stats_client(
    proxy_url: Option<&str>,
    fallback: &reqwest::Client,
) -> reqwest::Client {
    let Some(raw_url) = proxy_url.map(str::trim).filter(|s| !s.is_empty()) else {
        return fallback.clone();
    };

    let proxy = match reqwest::Proxy::all(raw_url) {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(
                "Niepoprawny adres NBA_API_PROXY – używam połączenia bezpośredniego: {err}"
            );
            return fallback.clone();
        }
    };

    match reqwest::Client::builder()
        .user_agent("SportsBettingExchange-StudentProject/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .proxy(proxy)
        .build()
    {
        Ok(client) => {
            tracing::info!("NBA Stats: skonfigurowano proxy z NBA_API_PROXY");
            client
        }
        Err(err) => {
            tracing::warn!("Nie udało się utworzyć klienta proxy dla NBA Stats: {err}");
            fallback.clone()
        }
    }
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

        let nba_stats_client =
            build_nba_stats_client(config.nba_api_proxy.as_deref(), &http_client);

        Self {
            db,
            config: Arc::new(config),
            http_client,
            nba_stats_client,
            ws_tx,
            backtest: Arc::new(BacktestStore::new()),
        }
    }
}
