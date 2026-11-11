// src/error.rs
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Błąd bazy danych: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Błąd zewnętrznego API: {0}")]
    ExternalApi(#[from] reqwest::Error),

    #[error("Nie znaleziono zasobu: {0}")]
    NotFound(String),

    #[error(
        "Niewystarczające środki w portfelu agenta (wymagane: {required}, dostępne: {available})"
    )]
    InsufficientFunds { required: String, available: String },

    #[error("Nie można obstawić tego meczu: {0}")]
    BetRejected(String),

    #[error("Nieprawidłowe dane wejściowe: {0}")]
    BadRequest(String),

    #[error("Brak autoryzacji: {0}")]
    Unauthorized(String),

    #[error("Konflikt: {0}")]
    Conflict(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            AppError::Database(err) => {
                // Logujemy pełny błąd SQL w konsoli Dockera, ale klientowi zwracamy bezpieczny komunikat
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

        let body = Json(json!({
            "error": message,
            "code": error_code,
        }));

        (status, body).into_response()
    }
}

// Alias ułatwiający pisanie sygnatur funkcji w handlerach Axum
pub type AppResult<T> = Result<T, AppError>;
