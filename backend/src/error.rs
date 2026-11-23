// src/error.rs
//! Globalny typ błędu aplikacji i jego mapowanie na odpowiedzi HTTP w formacie JSON.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

/// Standardowa odpowiedź błędu zwracana przez REST API.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    #[schema(example = "Mecz o ID 999 nie istnieje")]
    pub error: String,
    #[schema(example = "NOT_FOUND")]
    pub code: String,
}

/// Wszystkie błędy, które mogą wystąpić w handlerach REST, WebSocket i logice domenowej.
#[derive(Error, Debug)]
pub enum AppError {
    /// Błąd SQLx (zapytanie, połączenie, transakcja) – klientowi zwracamy ogólny komunikat.
    #[error("Błąd bazy danych: {0}")]
    Database(#[from] sqlx::Error),

    /// Błąd komunikacji z zewnętrznym dostawcą danych (reqwest).
    #[error("Błąd zewnętrznego API: {0}")]
    ExternalApi(#[from] reqwest::Error),

    /// Zewnętrzny dostawca zwrócił dane w nieoczekiwanym formacie (nie da się ich sparsować).
    #[error("Nieprawidłowe dane z zewnętrznego źródła: {0}")]
    ExternalData(String),

    /// Żądany zasób (mecz, agent) nie istnieje.
    #[error("Nie znaleziono zasobu: {0}")]
    NotFound(String),

    /// Stawka przekracza saldo portfela agenta.
    #[error(
        "Niewystarczające środki w portfelu agenta (wymagane: {required}, dostępne: {available})"
    )]
    InsufficientFunds { required: String, available: String },

    /// Zakład odrzucony regułami giełdy (np. mecz już trwa, duplikat zakładu).
    #[error("Nie można obstawić tego meczu: {0}")]
    BetRejected(String),

    /// Niepoprawne dane wejściowe od klienta.
    #[error("Nieprawidłowe dane wejściowe: {0}")]
    BadRequest(String),

    /// Błędne dane logowania agenta (np. zły `api_key`).
    #[error("Brak autoryzacji: {0}")]
    Unauthorized(String),

    /// Operacja koliduje z aktualnym stanem (np. backtest już trwa).
    #[error("Konflikt: {0}")]
    Conflict(String),
}

/// Zamienia błąd na odpowiedź `{"error": ..., "code": ...}` z odpowiednim kodem HTTP.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            AppError::Database(err) => {
                // Pełny błąd SQL trafia tylko do logów, a klient dostaje ogólny komunikat.
                tracing::error!("SQLx error: {:?}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "DATABASE_ERROR",
                    "Wystąpił wewnętrzny błąd bazy danych".to_string(),
                )
            }
            AppError::ExternalApi(err) => {
                tracing::error!("Reqwest error: {:?}", err);
                (
                    StatusCode::BAD_GATEWAY,
                    "EXTERNAL_API_ERROR",
                    "Błąd komunikacji z zewnętrznym dostawcą danych sportowych".to_string(),
                )
            }
            AppError::ExternalData(msg) => {
                tracing::error!("Błąd formatu danych zewnętrznych: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    "EXTERNAL_DATA_ERROR",
                    "Zewnętrzny dostawca zwrócił dane w nieoczekiwanym formacie".to_string(),
                )
            }
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg.clone()),
            AppError::InsufficientFunds { .. } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "INSUFFICIENT_FUNDS",
                self.to_string(),
            ),
            AppError::BetRejected(msg) => (StatusCode::CONFLICT, "BET_REJECTED", msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg.clone()),
        };

        let body = Json(ErrorResponse {
            error: message,
            code: error_code.to_string(),
        });

        (status, body).into_response()
    }
}

/// Skrót `Result` z błędem `AppError`, używany w sygnaturach handlerów i funkcji domenowych.
pub type AppResult<T> = Result<T, AppError>;
