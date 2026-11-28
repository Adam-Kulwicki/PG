// src/models/prediction.rs
//! Modele silników predykcyjnych i ich prognoz (tabele `prediction_models` i `predictions`).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};
use utoipa::ToSchema;

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
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Prediction {
    #[schema(example = 1)]
    pub id: i32,
    #[schema(example = 4)]
    pub match_id: i32,
    #[schema(example = 2)]
    pub model_id: i32,
    /// Prawdopodobieństwo wygranej gospodarza w przedziale `[0.0, 1.0]`.
    #[schema(example = 0.34)]
    pub prob_home: Decimal,
    /// Prawdopodobieństwo remisu (`0.0` dla sportów bez remisu: koszykówka, tenis, CS2).
    #[schema(example = 0.28)]
    pub prob_draw: Decimal,
    /// Prawdopodobieństwo wygranej gościa w przedziale `[0.0, 1.0]`.
    #[schema(example = 0.38)]
    pub prob_away: Decimal,
    /// Dodatkowe metryki modelu (np. `expected_goals_home`, `elo_diff`) w JSONB.
    #[schema(value_type = Object, example = json!({"expected_goals_home": 1.25, "expected_goals_away": 1.42}))]
    pub extra_metrics: Json<Value>,
    pub created_at: DateTime<Utc>,
}

/// Dane przyjmowane od silników predykcyjnych w `POST /api/v1/predictions`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePredictionDto {
    /// Identyfikator meczu z bazy danych.
    #[schema(example = 4)]
    pub match_id: i32,
    /// Nazwa modelu – model rejestrowany jest automatycznie przy pierwszej predykcji.
    #[schema(example = "XGBoost_Football")]
    pub model_name: String,
    /// Wersja modelu (opcjonalna, domyślnie `"1.0"`).
    #[schema(example = "1.0")]
    pub model_version: Option<String>,
    /// Prawdopodobieństwo wygranej gospodarza (`0.0..=1.0`).
    #[schema(example = 0.34)]
    pub prob_home: Decimal,
    /// Prawdopodobieństwo remisu (opcjonalne, traktowane jako `0` dla sportów bez remisu).
    #[schema(example = 0.28)]
    pub prob_draw: Option<Decimal>,
    /// Prawdopodobieństwo wygranej gościa (`0.0..=1.0`).
    #[schema(example = 0.38)]
    pub prob_away: Decimal,
    /// Opcjonalne dodatkowe metryki modelu zapisywane w JSONB (domyślnie `{}`).
    #[schema(value_type = Option<Object>, example = json!({"expected_goals_home": 1.25, "expected_goals_away": 1.42}))]
    pub extra_metrics: Option<Value>,
}
