// src/main.rs
//! Punkt wejścia: konfiguracja, baza danych, migracje i start serwera Axum.

use axum::{routing::get, Router};
use backend::{api, config, ingest, state, ws};
use config::{Config, LogConfig};
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
    // Opcjonalnie zapisujemy również do dziennych plików `backend.log.YYYY-MM-DD` (LOG_TO_FILE=true).
    let log_cfg = LogConfig::from_env();
    let mut _file_guard = None;
    let mut file_log_error = None;
    let file_layer = if log_cfg.to_file {
        match log_cfg.build_file_appender() {
            Ok(appender) => {
                let (non_blocking, guard) = tracing_appender::non_blocking(appender);
                _file_guard = Some(guard);
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(non_blocking),
                )
            }
            Err(err) => {
                file_log_error = Some(err.to_string());
                None
            }
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backend=debug,tower_http=info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .init();

    if let Some(err) = file_log_error {
        tracing::warn!(
            dir = %log_cfg.dir,
            "Nie udało się włączyć logowania do pliku – używam tylko stdout: {err}"
        );
    } else if log_cfg.to_file {
        tracing::info!(
            dir = %log_cfg.dir,
            max_files = ?log_cfg.max_files,
            "Włączono logowanie do dziennych plików backend.log.YYYY-MM-DD"
        );
    }

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

    // 7. Główny router: diagnostyka, REST API pod `/api/v1`, WebSocket pod `/ws` i OpenAPI pod `/docs`.
    let app = Router::new()
        .route("/health", get(api::health::health_check))
        .nest("/api/v1", api::router())
        .nest("/ws", ws::router())
        .with_state(app_state)
        .merge(api::docs::swagger_ui())
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    // 8. Nasłuch na adresie z konfiguracji (domyślnie `0.0.0.0:3000`, dostępny dla innych kontenerów).
    let listener = TcpListener::bind(&bind_addr).await?;
    tracing::info!("Serwer nasłuchuje na http://{}", bind_addr);
    tracing::info!(
        "Dokumentacja OpenAPI (Swagger UI): http://{}/docs",
        bind_addr
    );

    axum::serve(listener, app).await?;

    Ok(())
}
