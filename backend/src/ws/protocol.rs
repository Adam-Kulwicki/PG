// src/ws/protocol.rs
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPredictionSummary {
    pub model_name: String,
    pub prob_home: Decimal,
    pub prob_draw: Decimal,
    pub prob_away: Decimal,
    pub extra_metrics: Value,
}

/// Wiadomości wysyłane przez Twój serwer Axum do Agenta
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerMessage {
    Welcome {
        agent_id: i32,
        agent_name: String,
        current_bankroll: Decimal,
    },
    MatchOpportunity {
        match_id: i32,
        sport: String,
        competition: String,
        home_name: String,
        away_name: String,
        odds_home: Decimal,
        odds_draw: Option<Decimal>,
        odds_away: Decimal,
        predictions: Vec<ModelPredictionSummary>,
    },
    BetResult {
        match_id: i32,
        accepted: bool,
        new_bankroll: Decimal,
        message: String,
    },
    MatchSettled {
        match_id: i32,
        winner: String,
        bet_status: String, // "WON" lub "LOST"
        profit_loss: Decimal,
        current_bankroll: Decimal,
    },
}

/// Komendy przysyłane przez Agenta do Twojego serwera Axum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "payload",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum AgentCommand {
    PlaceBet {
        match_id: i32,
        selection: String, // "HOME", "DRAW", "AWAY"
        stake: Decimal,
        reasoning: Option<String>,
    },
    GetBankroll,
}
