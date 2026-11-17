// src/api/ingest.rs
//! Ręczne uruchamianie importu danych sportowych (poza harmonogramem).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    ingest::{self, SPORTS},
    state::AppState,
};

/// Trasa: `POST /{sport}` (`football`, `basketball`, `tennis` lub `all`).
pub fn router() -> Router<AppState> {
    Router::new().route("/{sport}", post(trigger_ingest))
}

/// Parametry zapytania ręcznego importu.
#[derive(Debug, Deserialize)]
pub struct IngestQuery {
    /// Czy pobrać także sezony historyczne (domyślnie `false`).
    #[serde(default)]
    pub history: bool,
}

/// POST /api/v1/ingest/{sport}?history=true – uruchamia import w tle i od razu zwraca `202`.
async fn trigger_ingest(
    State(state): State<AppState>,
    Path(sport): Path<String>,
    Query(query): Query<IngestQuery>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let sport = sport.to_ascii_lowercase();
    let sports: Vec<&'static str> = if sport == "all" {
        SPORTS.to_vec()
    } else {
        match SPORTS.iter().find(|s| **s == sport) {
            Some(s) => vec![*s],
            None => {
                return Err(AppError::BadRequest(format!(
                    "Nieznana dyscyplina '{}' (dozwolone: football, basketball, tennis, all)",
                    sport
                )))
            }
        }
    };

    let started = sports.clone();
    let include_history = query.history;
    tokio::spawn(async move {
        for sport in sports {
            match ingest::run_sport(&state, sport, include_history).await {
                Ok(count) => tracing::info!(sport, count, "Ręczny import zakończony"),
                Err(err) => tracing::error!(sport, "Ręczny import nieudany: {}", err),
            }
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "started": started, "history": include_history })),
    ))
}
