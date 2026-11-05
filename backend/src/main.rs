// src/main.rs
pub mod api;
pub mod config;
pub mod error;
pub mod models;
pub mod state;
pub mod ws;

use axum::{extract::State, routing::get, Json, Router};
use config::Config;
use error::AppResult;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use state::AppState;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Inicjalizacja systemu logowania (domyślnie poziom INFO, można zmienić przez zmienną RUST_LOG)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=debug,tower_http=info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Startowanie serwisu Backend (Rust/Axum)...");

    // 2. Wczytanie konfiguracji ze zmiennych środowiskowych kontenera
    let cfg = Config::from_env();
    let bind_addr = cfg.bind_address();

    // 3. Połączenie z bazą PostgreSQL
    tracing::info!("Łączenie z bazą danych PostgreSQL...");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&cfg.database_url)
        .await
        .expect("Nie udało się połączyć z bazą PostgreSQL. Sprawdź DATABASE_URL oraz czy kontener db działa.");

    // 4. Automatyczne uruchomienie migracji z folderu ./migrations
    tracing::info!("Uruchamianie migracji bazy danych...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Błąd podczas wykonywania migracji SQLx!");
    tracing::info!("Migracje zakończone sukcesem.");

    // 5. Zbudowanie współdzielonego stanu AppState
    let app_state = AppState::new(pool, cfg);

    // 6. Konfiguracja CORS (otwarta na czas developmentu w docker-compose)
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 7. Definicja głównego Routera
    let app = Router::new()
        .route("/health", get(health_check))
        // W kolejnych fazach podepniesz tu pod-routery:
        .nest("/api/v1", api::router())
        // .nest("/ws", ws::router())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(app_state);

    // 8. Bindowanie na 0.0.0.0:3000 (dostępne dla innych kontenerów w sieci Docker)
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("Serwer nasłuchuje na http://{}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Endpoint diagnostyczny sprawdzający połączenie z bazą i liczbę aktywnych agentów na WS
async fn health_check(State(state): State<AppState>) -> AppResult<Json<Value>> {
    // Sprawdzamy, czy baza odpowiada prostym zapytaniem
    sqlx::query("SELECT 1").execute(&state.db).await?;

    Ok(Json(json!({
        "status": "ok",
        "database": "connected",
        "connected_ws_agents": state.ws_tx.receiver_count(),
    })))
}
