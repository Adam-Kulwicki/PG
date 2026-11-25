// src/domain/betting_exchange.rs
//! Transakcyjne przyjmowanie zakładów (ACID) oraz blokada środków w portfelu agenta.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{FromRow, PgPool};

use crate::{
    domain::backtester::{BacktestAgentSeed, VirtualBet},
    error::{AppError, AppResult},
    models::bet::Bet,
    state::AppState,
};

/// Typ zakładu: zwycięstwo gospodarza, remis lub zwycięstwo gościa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Home,
    Draw,
    Away,
}

impl Selection {
    pub fn parse(raw: &str) -> AppResult<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "HOME" => Ok(Self::Home),
            "DRAW" => Ok(Self::Draw),
            "AWAY" => Ok(Self::Away),
            other => Err(AppError::BadRequest(format!(
                "Nieznany typ zakładu '{}' (dozwolone: HOME, DRAW, AWAY)",
                other
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::Draw => "DRAW",
            Self::Away => "AWAY",
        }
    }
}

/// Zapisany kupon – produkcyjny w bazie albo wirtualny w sesji backtestu.
#[derive(Debug, Clone)]
pub enum PlacedBetRecord {
    Live(Bet),
    Backtest(VirtualBet),
}

/// Wynik pomyślnie przyjętego zakładu.
#[derive(Debug, Clone)]
pub struct PlacedBet {
    pub bet: PlacedBetRecord,
    pub new_bankroll: Decimal,
}

#[derive(Debug, FromRow)]
struct BettableMatch {
    status: String,
    match_date: DateTime<Utc>,
    odds_home: Option<Decimal>,
    odds_draw: Option<Decimal>,
    odds_away: Option<Decimal>,
}

/// Czy mecz może w tej chwili przyjmować zakłady?
///
/// * Tryb produkcyjny: mecz `SCHEDULED`, który jeszcze się nie rozpoczął.
/// * Tryb backtestu: mecz historyczny (`FINISHED`), który jest w aktualnie "otwartym dniu"
///   symulacji (`backtest_open`). Wynik nigdy nie jest agentowi ujawniany przed rozliczeniem.
pub fn check_bettable(
    status: &str,
    match_date: DateTime<Utc>,
    now: DateTime<Utc>,
    backtest_open: bool,
) -> AppResult<()> {
    if backtest_open {
        return if status == "FINISHED" {
            Ok(())
        } else {
            Err(AppError::BetRejected(format!(
                "mecz w backteście ma nieoczekiwany status {}",
                status
            )))
        };
    }

    if status != "SCHEDULED" {
        return Err(AppError::BetRejected(format!(
            "mecz ma status {} – zakłady przyjmowane są wyłącznie na mecze SCHEDULED",
            status
        )));
    }
    if match_date <= now {
        return Err(AppError::BetRejected(
            "mecz już się rozpoczął – zakłady są zamknięte".to_string(),
        ));
    }
    Ok(())
}

/// Wartość oczekiwana zakładu: `EV = p * kurs - 1` (zaokrąglona do 4 miejsc, jak w kolumnie).
pub fn expected_value(prob: Decimal, odds: Decimal) -> Decimal {
    (prob * odds - Decimal::ONE).round_dp(4)
}

#[derive(Debug, FromRow)]
struct ActiveAgentSeedRow {
    id: i32,
    name: String,
    strategy_description: Option<String>,
    initial_bankroll: Decimal,
    is_active: bool,
}

async fn fetch_active_agent_seed(db: &PgPool, agent_id: i32) -> AppResult<BacktestAgentSeed> {
    let row = sqlx::query_as::<_, ActiveAgentSeedRow>(
        "SELECT id, name, strategy_description, initial_bankroll, is_active FROM agents WHERE id = $1",
    )
    .bind(agent_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Agent o ID {} nie istnieje", agent_id)))?;

    if !row.is_active {
        return Err(AppError::BetRejected("konto agenta jest nieaktywne".into()));
    }

    Ok(BacktestAgentSeed {
        id: row.id,
        name: row.name,
        strategy_description: row.strategy_description,
        initial_bankroll: row.initial_bankroll,
    })
}

async fn place_backtest_bet(
    state: &AppState,
    agent_id: i32,
    match_id: i32,
    selection: Selection,
    stake: Decimal,
    reasoning: Option<String>,
) -> AppResult<PlacedBet> {
    if !state.backtest.has_wallet(agent_id).await {
        let seed = fetch_active_agent_seed(&state.db, agent_id).await?;
        state.backtest.ensure_wallet(seed).await;
    }

    let (vbet, new_bankroll) = state
        .backtest
        .place_bet(agent_id, match_id, selection, stake, reasoning)
        .await?;

    tracing::info!(
        agent_id,
        match_id,
        selection = selection.as_str(),
        %stake,
        odds = %vbet.odds_taken,
        backtest = true,
        "Zakład wirtualny (backtest) przyjęty"
    );

    Ok(PlacedBet {
        bet: PlacedBetRecord::Backtest(vbet),
        new_bankroll,
    })
}

/// Przyjmuje zakład agenta:
/// * dla otwartego meczu backtestu — w izolowanym wirtualnym portfelu w pamięci (`BacktestStore`),
/// * w trybie produkcyjnym — w transakcji PostgreSQL z blokadą `SELECT ... FOR UPDATE`.
pub async fn place_bet(
    state: &AppState,
    agent_id: i32,
    match_id: i32,
    selection: &str,
    stake: Decimal,
    reasoning: Option<String>,
) -> AppResult<PlacedBet> {
    let selection = Selection::parse(selection)?;

    if stake <= Decimal::ZERO {
        return Err(AppError::BadRequest("Stawka musi być dodatnia".to_string()));
    }
    if stake.round_dp(2) != stake {
        return Err(AppError::BadRequest(
            "Stawka może mieć maksymalnie 2 miejsca po przecinku".to_string(),
        ));
    }

    if state.backtest.is_match_open(match_id).await {
        return place_backtest_bet(state, agent_id, match_id, selection, stake, reasoning).await;
    }

    let mut tx = state.db.begin().await?;

    // 1. Blokada portfela agenta na czas transakcji
    let (bankroll, is_active): (Decimal, bool) =
        sqlx::query_as("SELECT current_bankroll, is_active FROM agents WHERE id = $1 FOR UPDATE")
            .bind(agent_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Agent o ID {} nie istnieje", agent_id)))?;

    if !is_active {
        return Err(AppError::BetRejected("konto agenta jest nieaktywne".into()));
    }

    // 2. Mecz + kursy (blokada współdzielona – status nie zmieni się w trakcie transakcji)
    let m = sqlx::query_as::<_, BettableMatch>(
        r#"
        SELECT m.status, m.match_date, o.odds_home, o.odds_draw, o.odds_away
        FROM matches m
        LEFT JOIN match_odds o ON o.match_id = m.id AND o.bookmaker = 'MARKET_AVG'
        WHERE m.id = $1
        FOR SHARE OF m
        "#,
    )
    .bind(match_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Mecz o ID {} nie istnieje", match_id)))?;

    check_bettable(&m.status, m.match_date, Utc::now(), false)?;

    let odds = match selection {
        Selection::Home => m.odds_home,
        Selection::Draw => m.odds_draw,
        Selection::Away => m.odds_away,
    }
    .ok_or_else(|| {
        AppError::BetRejected(format!(
            "brak kursu dla typu {} w tym meczu",
            selection.as_str()
        ))
    })?;

    // Jeden zakład danego agenta na dany mecz (portfel zablokowany, więc brak wyścigu)
    let already: Option<(i32,)> =
        sqlx::query_as("SELECT id FROM bets WHERE agent_id = $1 AND match_id = $2")
            .bind(agent_id)
            .bind(match_id)
            .fetch_optional(&mut *tx)
            .await?;
    if already.is_some() {
        return Err(AppError::BetRejected(
            "agent postawił już zakład na ten mecz".to_string(),
        ));
    }

    // 3. Weryfikacja salda
    if bankroll < stake {
        return Err(AppError::InsufficientFunds {
            required: stake.to_string(),
            available: bankroll.to_string(),
        });
    }

    // EV wg średniej z predykcji wszystkich modeli (jeśli istnieją)
    let (avg_home, avg_draw, avg_away): (Option<Decimal>, Option<Decimal>, Option<Decimal>) =
        sqlx::query_as(
            "SELECT AVG(prob_home), AVG(prob_draw), AVG(prob_away) FROM predictions WHERE match_id = $1",
        )
        .bind(match_id)
        .fetch_one(&mut *tx)
        .await?;
    let prob = match selection {
        Selection::Home => avg_home,
        Selection::Draw => avg_draw,
        Selection::Away => avg_away,
    };
    let ev = prob.map(|p| expected_value(p, odds));

    // 4. Zapis kuponu i debet portfela
    let bet = sqlx::query_as::<_, Bet>(
        r#"
        INSERT INTO bets (agent_id, match_id, bet_selection, stake, odds_taken, expected_value, reasoning)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
        "#,
    )
    .bind(agent_id)
    .bind(match_id)
    .bind(selection.as_str())
    .bind(stake)
    .bind(odds)
    .bind(ev)
    .bind(reasoning)
    .fetch_one(&mut *tx)
    .await?;

    let (new_bankroll,): (Decimal,) = sqlx::query_as(
        "UPDATE agents SET current_bankroll = current_bankroll - $2 WHERE id = $1 RETURNING current_bankroll",
    )
    .bind(agent_id)
    .bind(stake)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(
        agent_id,
        match_id,
        selection = selection.as_str(),
        %stake,
        %odds,
        "Zakład przyjęty"
    );

    Ok(PlacedBet {
        bet: PlacedBetRecord::Live(bet),
        new_bankroll,
    })
}

/// Aktualne produkcyjne saldo portfela agenta w bazie danych.
pub async fn get_bankroll(db: &PgPool, agent_id: i32) -> AppResult<Decimal> {
    let (bankroll,): (Decimal,) =
        sqlx::query_as("SELECT current_bankroll FROM agents WHERE id = $1")
            .bind(agent_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Agent o ID {} nie istnieje", agent_id)))?;
    Ok(bankroll)
}

/// Aktualne saldo agenta z perspektywy WebSocketu: podczas trwającego backtestu
/// zwraca saldo jego wirtualnego portfela, a poza nim saldo produkcyjne z bazy.
pub async fn get_ws_bankroll(state: &AppState, agent_id: i32) -> AppResult<Decimal> {
    if state.backtest.is_running() {
        if let Some(bankroll) = state.backtest.get_bankroll(agent_id).await {
            return Ok(bankroll);
        }
        let seed = fetch_active_agent_seed(&state.db, agent_id).await?;
        if let Some(bankroll) = state.backtest.ensure_wallet(seed).await {
            return Ok(bankroll);
        }
    }
    get_bankroll(&state.db, agent_id).await
}
