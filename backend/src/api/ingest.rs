// src/api/ingest.rs
//! Ręczne uruchamianie importu danych sportowych (poza harmonogramem).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    error::{AppError, AppResult},
    ingest::{self, SPORTS},
    state::AppState,
};

/// Trasa: `POST /{sport}` (`football`, `basketball`, `tennis`, `cs2` lub `all`).
pub fn router() -> Router<AppState> {
    Router::new().route("/{sport}", post(trigger_ingest))
}

/// Parametry zapytania ręcznego importu.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IngestQuery {
    /// Czy pobrać także sezony historyczne (domyślnie `false` — tylko bieżący sezon / przyrostowe aktualizacje).
    #[serde(default)]
    #[param(example = false)]
    pub history: bool,
}

/// Potwierdzenie uruchomienia importu w tle (`POST /api/v1/ingest/{sport}`).
#[derive(Debug, Serialize, ToSchema)]
pub struct IngestStartedResponse {
    /// Lista dyscyplin zakolejkowanych do synchronizacji w tle.
    #[schema(example = json!(["football", "basketball", "tennis", "cs2"]))]
    pub started: Vec<String>,
    /// Czy uruchomiono synchronizację wraz z sezonami historycznymi.
    #[schema(example = true)]
    pub history: bool,
}

/// Ręczne uruchomienie importu danych sportowych (ETL) w tle.
///
/// Natychmiast zwraca `202 Accepted` i wykonuje synchronizację asynchronicznie:
/// * `football`: `football-data.co.uk` (Top 5 lig + `fixtures.csv`), opcjonalnie Football-Data.org v4 oraz `clubelo.com` (`D - 1`, `/Fixtures` i skalibrowany backfill `EloRatings.csv`),
/// * `basketball`: `stats.nba.com` (`Base` + `Advanced`), opcjonalnie BALLDONTLIE (`v1/games`) i TheSportsDB (Euroliga),
/// * `tennis`: archiwum Sackmanna (`history=true`) + ESPN Scoreboard (single ATP/WTA),
/// * `cs2`: PandaScore Free (`/csgo/*` + składy turniejowe) + Kaggle CSV (`ektarr/counter-strike-pro-matches`, kontrola wersji i cache `data/cs2_games.csv`),
/// * `all`: uruchamia kolejno wszystkie 4 dyscypliny.
#[utoipa::path(
    post,
    path = "/api/v1/ingest/{sport}",
    tag = "Ingest",
    params(
        ("sport" = String, Path, description = "Dyscyplina: `football`, `basketball`, `tennis`, `cs2` lub `all` (wielkość liter bez znaczenia)", example = "all"),
        IngestQuery
    ),
    responses(
        (status = 202, description = "Import uruchomiony w tle", body = IngestStartedResponse),
        (status = 400, description = "Nieznana dyscyplina (dozwolone: football, basketball, tennis, cs2, all)", body = crate::error::ErrorResponse)
    )
)]
pub async fn trigger_ingest(
    State(state): State<AppState>,
    Path(sport): Path<String>,
    Query(query): Query<IngestQuery>,
) -> AppResult<(StatusCode, Json<IngestStartedResponse>)> {
    let sport = sport.to_ascii_lowercase();
    let sports: Vec<&'static str> = if sport == "all" {
        SPORTS.to_vec()
    } else {
        match SPORTS.iter().find(|s| **s == sport) {
            Some(s) => vec![*s],
            None => {
                return Err(AppError::BadRequest(format!(
                    "Nieznana dyscyplina '{}' (dozwolone: football, basketball, tennis, cs2, all)",
                    sport
                )))
            }
        }
    };

    let started: Vec<String> = sports.iter().map(|s| (*s).to_string()).collect();
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
        Json(IngestStartedResponse {
            started,
            history: include_history,
        }),
    ))
}
