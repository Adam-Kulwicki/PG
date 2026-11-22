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
    pub id: i32,
    pub name: String,
    #[serde(skip_serializing)]
    #[schema(ignore)]
    // Klucz API nie jest serializowany, więc nie wycieknie przez publiczne endpointy (np. ranking).
    pub api_key: String,
    /// Opis strategii, np. "Kelly Criterion 1/4".
    pub strategy_description: Option<String>,
    /// Kapitał początkowy – punkt odniesienia do liczenia zysku i ROI.
    pub initial_bankroll: Decimal,
    /// Aktualne saldo portfela (nigdy ujemne – pilnuje tego CHECK w bazie).
    pub current_bankroll: Decimal,
    /// Nieaktywny agent nie może się połączyć ani obstawiać.
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Pojedynczy kupon postawiony przez agenta na wynik meczu.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Bet {
    pub id: i32,
    pub agent_id: i32,
    pub match_id: i32,
    /// Obstawiany wynik: "HOME", "DRAW" lub "AWAY".
    pub bet_selection: String,
    /// Postawiona kwota (już odjęta z portfela w momencie przyjęcia zakładu).
    pub stake: Decimal,
    /// Kurs zamrożony w chwili obstawienia (późniejsze zmiany kursów go nie dotyczą).
    pub odds_taken: Decimal,
    /// Wartość oczekiwana (EV) wyliczona z predykcji modeli w chwili zakładu.
    pub expected_value: Option<Decimal>,
    /// Stan kuponu: "PENDING", "WON", "LOST" lub "VOID".
    pub status: String,
    /// Wynik netto: 0 dla PENDING, -stake dla LOST, stake * (kurs - 1) dla WON.
    pub profit_loss: Decimal,
    /// Uzasadnienie przesłane przez agenta w komendzie `PLACE_BET`.
    pub reasoning: Option<String>,
    pub placed_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}
