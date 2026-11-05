// src/models/prediction.rs
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PredictionModel {
    pub id: i32,
    pub name: String,
    pub version: String,
    pub sport: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Prediction {
    pub id: i32,
    pub match_id: i32,
    pub model_id: i32,
    pub prob_home: Decimal,
    pub prob_draw: Decimal,
    pub prob_away: Decimal,
    pub extra_metrics: Json<Value>,
    pub created_at: DateTime<Utc>,
}

/// Payload przyjmowany od silników predykcyjnych w POST /api/v1/predictions
#[derive(Debug, Clone, Deserialize)]
pub struct CreatePredictionDto {
    pub match_id: i32,
    pub model_name: String,
    pub model_version: Option<String>,
    pub prob_home: Decimal,
    pub prob_draw: Option<Decimal>,
    pub prob_away: Decimal,
    pub extra_metrics: Option<Value>,
}
