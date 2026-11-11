// src/api/predictions.rs
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use rust_decimal::Decimal;

use crate::{
    error::{AppError, AppResult},
    models::{
        match_model::Match,
        prediction::{CreatePredictionDto, Prediction},
    },
    state::AppState,
    ws::{protocol::WsEnvelope, session},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/", post(create_prediction))
}

/// Dopuszczalna odchyłka sumy prawdopodobieństw od 1.0 (zaokrąglenia modeli)
fn probability_tolerance() -> Decimal {
    Decimal::new(1, 2) // 0.01
}

pub fn validate_probabilities(home: Decimal, draw: Decimal, away: Decimal) -> AppResult<()> {
    for (name, p) in [
        ("prob_home", home),
        ("prob_draw", draw),
        ("prob_away", away),
    ] {
        if p < Decimal::ZERO || p > Decimal::ONE {
            return Err(AppError::BadRequest(format!(
                "{} musi być w przedziale [0, 1]",
                name
            )));
        }
    }
    if (home + draw + away - Decimal::ONE).abs() > probability_tolerance() {
        return Err(AppError::BadRequest(
            "Suma prawdopodobieństw musi wynosić 1.0 (+/- 0.01)".to_string(),
        ));
    }
    Ok(())
}

/// POST /api/v1/predictions
///
/// Zapisuje (upsert) predykcję silnika i – dla meczów SCHEDULED – rozgłasza
/// `MATCH_OPPORTUNITY` do wszystkich podłączonych agentów.
async fn create_prediction(
    State(state): State<AppState>,
    Json(payload): Json<CreatePredictionDto>,
) -> AppResult<(StatusCode, Json<Prediction>)> {
    let prob_draw = payload.prob_draw.unwrap_or(Decimal::ZERO);
    validate_probabilities(payload.prob_home, prob_draw, payload.prob_away)?;

    if payload.model_name.trim().is_empty() {
        return Err(AppError::BadRequest(
            "model_name nie może być pusty".to_string(),
        ));
    }

    let match_row = sqlx::query_as::<_, Match>("SELECT * FROM matches WHERE id = $1")
        .bind(payload.match_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("Mecz o ID {} nie istnieje", payload.match_id))
        })?;

    let (sport,): (String,) = sqlx::query_as("SELECT c.sport FROM competitions c WHERE c.id = $1")
        .bind(match_row.competition_id)
        .fetch_one(&state.db)
        .await?;

    let version = payload.model_version.unwrap_or_else(|| "1.0".to_string());
    let mut tx = state.db.begin().await?;

    // Model rejestrowany automatycznie przy pierwszej predykcji (name + version)
    let (model_id,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO prediction_models (name, version, sport)
        VALUES ($1, $2, $3)
        ON CONFLICT (name, version) DO UPDATE SET name = EXCLUDED.name
        RETURNING id
        "#,
    )
    .bind(payload.model_name.trim())
    .bind(&version)
    .bind(&sport)
    .fetch_one(&mut *tx)
    .await?;

    let prediction = sqlx::query_as::<_, Prediction>(
        r#"
        INSERT INTO predictions (match_id, model_id, prob_home, prob_draw, prob_away, extra_metrics)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (match_id, model_id) DO UPDATE SET
            prob_home = EXCLUDED.prob_home,
            prob_draw = EXCLUDED.prob_draw,
            prob_away = EXCLUDED.prob_away,
            extra_metrics = EXCLUDED.extra_metrics,
            created_at = NOW()
        RETURNING *
        "#,
    )
    .bind(payload.match_id)
    .bind(model_id)
    .bind(payload.prob_home)
    .bind(prob_draw)
    .bind(payload.prob_away)
    .bind(sqlx::types::Json(
        payload
            .extra_metrics
            .unwrap_or_else(|| serde_json::json!({})),
    ))
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    // Rozgłoszenie okazji agentom (tylko mecze, na które można jeszcze obstawiać)
    if match_row.status == "SCHEDULED" {
        if let Some(opportunity) = session::build_opportunity(&state.db, payload.match_id).await? {
            // Brak podłączonych agentów nie jest błędem – dostaną okazję jako Late Joiners
            let _ = state.ws_tx.send(WsEnvelope::to_all(opportunity));
        }
    }

    Ok((StatusCode::CREATED, Json(prediction)))
}
