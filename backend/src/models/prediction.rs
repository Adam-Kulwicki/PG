// src/models/prediction.rs
//! Modele silników predykcyjnych i ich prognoz (tabele `prediction_models` i `predictions`).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};

/// Zarejestrowany silnik predykcyjny (np. XGBoost, Elo, Poisson) w konkretnej wersji.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PredictionModel {
    pub id: i32,
    pub name: String,
    pub version: String,
    /// Dyscyplina, którą obsługuje model, lub "ALL".
    pub sport: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Prognoza modelu dla jednego meczu (jeden model = jedna predykcja na mecz).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Prediction {
    pub id: i32,
    pub match_id: i32,
    pub model_id: i32,
    /// Prawdopodobieństwa w przedziale 0.0–1.0; dla sportów bez remisu `prob_draw` = 0.
    pub prob_home: Decimal,
    pub prob_draw: Decimal,
    pub prob_away: Decimal,
    /// Dodatkowe metryki modelu (np. expected_goals) w JSONB.
    pub extra_metrics: Json<Value>,
    pub created_at: DateTime<Utc>,
}

/// Dane przyjmowane od silników predykcyjnych w `POST /api/v1/predictions`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePredictionDto {
    pub match_id: i32,
    /// Nazwa modelu – model rejestrowany jest automatycznie przy pierwszej predykcji.
    pub model_name: String,
    /// Wersja modelu (domyślnie "1.0").
    pub model_version: Option<String>,
    pub prob_home: Decimal,
    /// Pomijane dla sportów bez remisu (traktowane jako 0).
    pub prob_draw: Option<Decimal>,
    pub prob_away: Decimal,
    pub extra_metrics: Option<Value>,
}
