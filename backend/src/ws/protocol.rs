// src/ws/protocol.rs
//! Kontrakt JSON protokołu WebSocket: wiadomości serwera (`ServerMessage`)
//! oraz komendy agenta (`AgentCommand`).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::backtester::BacktestLeaderboardEntry;

/// Skrót predykcji jednego modelu dołączany do `MATCH_OPPORTUNITY`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPredictionSummary {
    pub model_name: String,
    pub prob_home: Decimal,
    pub prob_draw: Decimal,
    pub prob_away: Decimal,
    pub extra_metrics: Value,
}

/// Wiadomości wysyłane przez serwer do agenta; w JSON mają postać `{"event": ..., "data": {...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerMessage {
    /// Powitanie wysyłane zaraz po nawiązaniu połączenia.
    Welcome {
        agent_id: i32,
        agent_name: String,
        current_bankroll: Decimal,
    },
    /// Nowa okazja inwestycyjna: mecz z kursami i predykcjami modeli.
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
    /// Potwierdzenie (lub odrzucenie) zakładu postawionego komendą `PLACE_BET`.
    BetResult {
        match_id: i32,
        accepted: bool,
        new_bankroll: Decimal,
        message: String,
    },
    /// Rozliczenie kuponu agenta po zakończeniu meczu.
    MatchSettled {
        match_id: i32,
        winner: String,
        /// Wynik kuponu: "WON", "LOST" lub "VOID".
        bet_status: String,
        profit_loss: Decimal,
        current_bankroll: Decimal,
    },
    /// Odpowiedź na komendę `GET_BANKROLL`.
    Bankroll { current_bankroll: Decimal },
    /// Początek symulacji historycznej (backtestu).
    BacktestStarted {
        run_id: u64,
        season: String,
        sport: Option<String>,
        total_matches: usize,
        total_days: usize,
        initial_bankroll: Option<Decimal>,
    },
    /// Koniec symulacji historycznej (backtestu).
    BacktestFinished {
        run_id: u64,
        season: String,
        total_matches: usize,
        settled_bets: usize,
        leaderboard: Vec<BacktestLeaderboardEntry>,
    },
    /// Błąd protokołu, np. niepoprawny JSON komendy.
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
    /// Wiadomość dla wszystkich podłączonych agentów.
    pub fn to_all(message: ServerMessage) -> Self {
        Self {
            target_agent: None,
            message,
        }
    }

    /// Wiadomość tylko dla wskazanego agenta.
    pub fn to_agent(agent_id: i32, message: ServerMessage) -> Self {
        Self {
            target_agent: Some(agent_id),
            message,
        }
    }

    /// Czy koperta jest przeznaczona dla danego agenta (broadcast lub adresowana do niego).
    pub fn is_for(&self, agent_id: i32) -> bool {
        self.target_agent.is_none_or(|id| id == agent_id)
    }
}

/// Komendy wysyłane przez agenta do serwera; w JSON mają postać `{"action": ..., "payload": {...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "payload",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum AgentCommand {
    /// Postawienie zakładu na mecz.
    PlaceBet {
        match_id: i32,
        /// Obstawiany wynik: "HOME", "DRAW" lub "AWAY".
        selection: String,
        stake: Decimal,
        /// Opcjonalne uzasadnienie decyzji agenta (zapisywane przy kuponie).
        reasoning: Option<String>,
    },
    /// Zapytanie o aktualne saldo portfela.
    GetBankroll,
}
