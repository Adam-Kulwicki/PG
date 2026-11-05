// src/models/match_model.rs
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{types::Json, FromRow};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "sport_type", rename_all = "snake_case")]
pub enum MatchDetails {
    Football {
        ht_home: Option<i32>,
        ht_away: Option<i32>,
        round: String,
    },
    Basketball {
        quarters: Vec<(i32, i32)>,
        overtime: bool,
    },
    Tennis {
        surface: String,
        best_of: u8,
        sets: Vec<(u8, u8)>,
        home_aces: Option<u16>,
        away_aces: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Match {
    pub id: i32,
    pub competition_id: i32,
    pub season: String,
    pub match_date: DateTime<Utc>,
    pub status: String,
    pub home_participant_id: i32,
    pub away_participant_id: i32,
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    pub winner: Option<String>,
    pub match_details: Json<MatchDetails>,
    pub external_ids: Json<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MatchOdds {
    pub id: i32,
    pub match_id: i32,
    pub bookmaker: String,
    pub odds_home: Decimal,
    pub odds_draw: Option<Decimal>,
    pub odds_away: Decimal,
    pub updated_at: DateTime<Utc>,
}

/// Wzbogacony widok meczu zwracany przez GET /api/v1/matches (z nazwami drużyn i kursami)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MatchWithDetailsDto {
    pub id: i32,
    pub sport: String,
    pub competition_name: String,
    pub season: String,
    pub match_date: DateTime<Utc>,
    pub status: String,
    pub home_participant_id: i32,
    pub home_name: String,
    pub away_participant_id: i32,
    pub away_name: String,
    pub home_score: Option<i32>,
    pub away_score: Option<i32>,
    pub winner: Option<String>,
    pub match_details: Json<MatchDetails>,
    pub odds_home: Option<Decimal>,
    pub odds_draw: Option<Decimal>,
    pub odds_away: Option<Decimal>,
}