// src/models/participant.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Competition {
    pub id: i32,
    pub sport: String,
    pub name: String,
    pub country_or_category: Option<String>,
    pub external_code: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Participant {
    pub id: i32,
    pub sport: String,
    pub name: String,
    pub short_code: Option<String>,
    pub participant_type: String, // "TEAM" lub "PLAYER"
    pub metadata: Json<Value>,
    pub created_at: DateTime<Utc>,
}
