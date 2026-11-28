// src/models/bet.rs
//! Modele agentów obstawiających oraz ich zakładów (tabele `agents` i `bets`).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

/// Agent obstawiający mecze przez WebSocket – posiada własny portfel (`bankroll`).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Agent {
    #[schema(example = 1)]
    pub id: i32,
    #[schema(example = "Agent_Kelly_Value")]
    pub name: String,
    #[serde(skip_serializing)]
    #[schema(ignore)]
    // Klucz API nie jest serializowany, więc nie wycieknie przez publiczne endpointy (np. ranking).
    pub api_key: String,
    /// Opis strategii, np. `"Kelly Criterion 1/4"`.
    #[schema(example = "Stawia wg ułamkowego Kryterium Kelly'ego przy EV > 5%")]
    pub strategy_description: Option<String>,
    /// Kapitał początkowy – punkt odniesienia do liczenia zysku i ROI.
    #[schema(example = 1000.00)]
    pub initial_bankroll: Decimal,
    /// Aktualne saldo portfela (nigdy ujemne – pilnuje tego CHECK w bazie).
    #[schema(example = 1050.00)]
    pub current_bankroll: Decimal,
    /// Nieaktywny agent nie może się połączyć ani obstawiać.
    #[schema(example = true)]
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Pojedynczy kupon postawiony przez agenta na wynik meczu.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Bet {
    #[schema(example = 1)]
    pub id: i32,
    #[schema(example = 1)]
    pub agent_id: i32,
    #[schema(example = 4)]
    pub match_id: i32,
    /// Obstawiany wynik: `"HOME"`, `"DRAW"` lub `"AWAY"`.
    #[schema(example = "AWAY")]
    pub bet_selection: String,
    /// Postawiona kwota (już odjęta z portfela w momencie przyjęcia zakładu).
    #[schema(example = 50.00)]
    pub stake: Decimal,
    /// Kurs zamrożony w chwili obstawienia (późniejsze zmiany kursów go nie dotyczą).
    #[schema(example = 2.60)]
    pub odds_taken: Decimal,
    /// Wartość oczekiwana (`EV = prob * odds - 1`) wyliczona ze średniej predykcji modeli w chwili zakładu.
    #[schema(example = 0.088)]
    pub expected_value: Option<Decimal>,
    /// Stan kuponu: `"PENDING"`, `"WON"`, `"LOST"` lub `"VOID"`.
    #[schema(example = "PENDING")]
    pub status: String,
    /// Wynik netto: `0` dla `PENDING`/`VOID`, `-stake` dla `LOST`, `stake * (kurs - 1)` dla `WON`.
    #[schema(example = 0.00)]
    pub profit_loss: Decimal,
    /// Uzasadnienie przesłane przez agenta w komendzie `PLACE_BET`.
    #[schema(example = "Dodatnie EV wg modelu XGBoost")]
    pub reasoning: Option<String>,
    pub placed_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}
