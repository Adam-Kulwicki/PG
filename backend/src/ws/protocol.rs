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
        bet_status: String, // "WON", "LOST" lub "VOID"
        profit_loss: Decimal,
        current_bankroll: Decimal,
    },
    /// Odpowiedź na komendę `GET_BANKROLL`
    Bankroll { current_bankroll: Decimal },
    /// Początek symulacji historycznej (backtest)
    BacktestStarted {
        season: String,
        sport: Option<String>,
        total_matches: usize,
        total_days: usize,
    },
    /// Koniec symulacji historycznej (backtest)
    BacktestFinished {
        season: String,
        total_matches: usize,
        settled_bets: usize,
    },
    /// Błąd protokołu (np. niepoprawny JSON komendy)
    Error { code: String, message: String },
}

/// Koperta kanału broadcast: `target_agent = None` oznacza wiadomość do wszystkich agentów,
/// `Some(id)` – wiadomość adresowaną tylko do wskazanego agenta (np. rozliczenie jego kuponu).
#[derive(Debug, Clone)]
pub struct WsEnvelope {
    pub target_agent: Option<i32>,
    pub message: ServerMessage,
}

impl WsEnvelope {
    pub fn to_all(message: ServerMessage) -> Self {
        Self {
            target_agent: None,
            message,
        }
    }

    pub fn to_agent(agent_id: i32, message: ServerMessage) -> Self {
        Self {
            target_agent: Some(agent_id),
            message,
        }
    }

    pub fn is_for(&self, agent_id: i32) -> bool {
        self.target_agent.is_none_or(|id| id == agent_id)
    }
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
