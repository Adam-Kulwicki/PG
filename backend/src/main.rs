// src/main.rs
//! Punkt wejścia: konfiguracja, baza danych, migracje i start serwera Axum.

use axum::{extract::State, routing::get, Json, Router};
use backend::{api, config, error, ingest, state, ws};
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
    // 1. Logowanie: poziom domyślny jest ustawiony w kodzie, a zmienna `RUST_LOG` go nadpisuje.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=debug,tower_http=info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Startowanie serwisu Backend (Rust/Axum)...");

    // 2. Konfiguracja wczytywana ze zmiennych środowiskowych kontenera.
    let cfg = Config::from_env();
    let bind_addr = cfg.bind_address();

    // 3. Pula połączeń z PostgreSQL; brak bazy przy starcie jest błędem krytycznym.
    tracing::info!("Łączenie z bazą danych PostgreSQL...");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&cfg.database_url)
        .await
        .expect("Nie udało się połączyć z bazą PostgreSQL. Sprawdź DATABASE_URL oraz czy kontener db działa.");

    // 4. Migracje z katalogu `./migrations` (osadzone w binarium w czasie kompilacji).
    tracing::info!("Uruchamianie migracji bazy danych...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Błąd podczas wykonywania migracji SQLx!");
    tracing::info!("Migracje zakończone sukcesem.");

    // 5. Współdzielony stan aplikacji (pula bazy, konfiguracja, kanał broadcast).
    let ingest_enabled = cfg.ingest_enabled;
    let app_state = AppState::new(pool, cfg);

    // 5a. Cykliczny import danych sportowych (ETL) uruchamiany w tle.
    if ingest_enabled {
        ingest::spawn_scheduler(app_state.clone());
    } else {
        tracing::info!("Import danych sportowych wyłączony (INGEST_ENABLED=false)");
    }

    // 6. CORS otwarty dla wszystkich źródeł – wystarczy na czas developmentu.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 7. Główny router: diagnostyka, REST API pod `/api/v1` i WebSocket pod `/ws`.
    let app = Router::new()
        .route("/health", get(health_check))
        .nest("/api/v1", api::router())
        .nest("/ws", ws::router())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(app_state);

    // 8. Nasłuch na adresie z konfiguracji (domyślnie `0.0.0.0:3000`, dostępny dla innych kontenerów).
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("Serwer nasłuchuje na http://{}", bind_addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// GET /health – sprawdza połączenie z bazą i zwraca liczbę subskrybentów kanału WebSocket.
async fn health_check(State(state): State<AppState>) -> AppResult<Json<Value>> {
    // Proste zapytanie `SELECT 1` wystarcza, aby potwierdzić, że baza odpowiada.
    sqlx::query("SELECT 1").execute(&state.db).await?;

    Ok(Json(json!({
        "status": "ok",
        "database": "connected",
        "connected_ws_agents": state.ws_tx.receiver_count(),
    })))
}
