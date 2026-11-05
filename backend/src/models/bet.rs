// src/models/bet.rs
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Agent {
    pub id: i32,
    pub name: String,
    #[serde(skip_serializing)] // Nie wyciekamy klucza API w publicznych endpointach typu leaderboard!
    pub api_key: String,
    pub strategy_description: Option<String>,
    pub initial_bankroll: Decimal,
    pub current_bankroll: Decimal,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Bet {
    pub id: i32,
    pub agent_id: i32,
    pub match_id: i32,
    pub bet_selection: String, // "HOME", "DRAW", "AWAY"
    pub stake: Decimal,
    pub odds_taken: Decimal,
    pub expected_value: Option<Decimal>,
    pub status: String,        // "PENDING", "WON", "LOST", "VOID"
    pub profit_loss: Decimal,
    pub reasoning: Option<String>,
    pub placed_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}