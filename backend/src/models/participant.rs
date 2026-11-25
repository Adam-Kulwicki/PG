// src/models/participant.rs
//! Modele rozgrywek i uczestników (tabele `competitions` i `participants`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};

/// Rozgrywki lub turniej, np. Premier League, NBA, ATP Tour.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Competition {
    pub id: i32,
    /// "FOOTBALL", "BASKETBALL", "TENNIS" lub "CS2".
    pub sport: String,
    pub name: String,
    pub country_or_category: Option<String>,
    /// Unikalny kod używany przy imporcie danych, np. "PL", "NBA", "ATP", "CS2".
    pub external_code: String,
    pub created_at: DateTime<Utc>,
}

/// Uczestnik meczu: drużyna (piłka, koszykówka) albo zawodnik (tenis).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Participant {
    pub id: i32,
    pub sport: String,
    pub name: String,
    pub short_code: Option<String>,
    /// "TEAM" (drużyna) lub "PLAYER" (zawodnik indywidualny).
    pub participant_type: String,
    /// Dodatkowe dane w JSONB, np. `{"hand": "R", "ioc": "POL"}`.
    pub metadata: Json<Value>,
    pub created_at: DateTime<Utc>,
}
