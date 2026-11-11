// src/domain/settlement.rs
//! Rozliczanie zakończonych spotkań i wypłaty do portfeli agentów.

use rust_decimal::Decimal;
use sqlx::FromRow;
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::{
    error::AppResult,
    models::bet::Bet,
    ws::protocol::{ServerMessage, WsEnvelope},
};

/// Wynik rozliczenia pojedynczego kuponu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BetOutcome {
    Won,
    Lost,
    Void,
}

impl BetOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Won => "WON",
            Self::Lost => "LOST",
            Self::Void => "VOID",
        }
    }
}

/// Zdarzenie rozliczenia kuponu (do rozesłania agentowi przez WebSocket).
#[derive(Debug, Clone)]
pub struct SettledBet {
    pub agent_id: i32,
    pub match_id: i32,
    pub winner: String,
    pub outcome: BetOutcome,
    pub profit_loss: Decimal,
    pub current_bankroll: Decimal,
}

impl SettledBet {
    pub fn into_envelope(self) -> WsEnvelope {
        WsEnvelope::to_agent(
            self.agent_id,
            ServerMessage::MatchSettled {
                match_id: self.match_id,
                winner: self.winner,
                bet_status: self.outcome.as_str().to_string(),
                profit_loss: self.profit_loss,
                current_bankroll: self.current_bankroll,
            },
        )
    }
}

/// Ustala wynik kuponu na podstawie statusu meczu i zwycięzcy.
/// Mecz bez rozstrzygnięcia (np. odwołany) unieważnia kupon (zwrot stawki).
pub fn resolve_outcome(selection: &str, match_status: &str, winner: Option<&str>) -> BetOutcome {
    match (match_status, winner) {
        ("FINISHED", Some(w)) if w == selection => BetOutcome::Won,
        ("FINISHED", Some(_)) => BetOutcome::Lost,
        _ => BetOutcome::Void,
    }
}

/// Zwraca `(zysk/strata netto, kwota do wypłaty na portfel)`.
pub fn payout(outcome: BetOutcome, stake: Decimal, odds: Decimal) -> (Decimal, Decimal) {
    match outcome {
        BetOutcome::Won => {
            let gross = (stake * odds).round_dp(2);
            (gross - stake, gross)
        }
        BetOutcome::Lost => (-stake, Decimal::ZERO),
        BetOutcome::Void => (Decimal::ZERO, stake),
    }
}

#[derive(Debug, FromRow)]
struct MatchResultRow {
    status: String,
    winner: Option<String>,
}

/// Rozlicza wszystkie nierozliczone zakłady dla zakończonych (lub odwołanych) meczów.
/// Gdy `only_matches` jest `Some`, rozliczane są tylko wskazane mecze (używane w backteście).
///
/// Każdy mecz rozliczany jest w osobnej transakcji (`FOR UPDATE` na kuponach), więc
/// równoległe wywołania nie wypłacą tego samego kuponu dwukrotnie.
pub async fn settle_matches(
    db: &PgPool,
    only_matches: Option<&[i32]>,
) -> AppResult<Vec<SettledBet>> {
    let match_ids: Vec<(i32,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT m.id
        FROM matches m
        JOIN bets b ON b.match_id = m.id
        WHERE b.status = 'PENDING'
          AND m.status IN ('FINISHED', 'CANCELLED')
          AND ($1::INT[] IS NULL OR m.id = ANY($1))
        ORDER BY m.id
        "#,
    )
    .bind(only_matches.map(|ids| ids.to_vec()))
    .fetch_all(db)
    .await?;

    let mut settled = Vec::new();
    for (match_id,) in match_ids {
        settled.extend(settle_match(db, match_id).await?);
    }

    if !settled.is_empty() {
        tracing::info!("Rozliczono {} kuponów", settled.len());
    }
    Ok(settled)
}

async fn settle_match(db: &PgPool, match_id: i32) -> AppResult<Vec<SettledBet>> {
    let mut tx = db.begin().await?;

    let result =
        sqlx::query_as::<_, MatchResultRow>("SELECT status, winner FROM matches WHERE id = $1")
            .bind(match_id)
            .fetch_one(&mut *tx)
            .await?;

    // Kolejność po agent_id ogranicza ryzyko deadlocków między równoległymi rozliczeniami
    let bets = sqlx::query_as::<_, Bet>(
        "SELECT * FROM bets WHERE match_id = $1 AND status = 'PENDING' ORDER BY agent_id FOR UPDATE",
    )
    .bind(match_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut events = Vec::with_capacity(bets.len());
    for bet in bets {
        let outcome = resolve_outcome(&bet.bet_selection, &result.status, result.winner.as_deref());
        let (profit_loss, credit) = payout(outcome, bet.stake, bet.odds_taken);

        sqlx::query(
            "UPDATE bets SET status = $2, profit_loss = $3, settled_at = NOW() WHERE id = $1",
        )
        .bind(bet.id)
        .bind(outcome.as_str())
        .bind(profit_loss)
        .execute(&mut *tx)
        .await?;

        let (bankroll,): (Decimal,) = sqlx::query_as(
            "UPDATE agents SET current_bankroll = current_bankroll + $2 WHERE id = $1 RETURNING current_bankroll",
        )
        .bind(bet.agent_id)
        .bind(credit)
        .fetch_one(&mut *tx)
        .await?;

        events.push(SettledBet {
            agent_id: bet.agent_id,
            match_id,
            winner: result.winner.clone().unwrap_or_else(|| "VOID".to_string()),
            outcome,
            profit_loss,
            current_bankroll: bankroll,
        });
    }

    tx.commit().await?;
    Ok(events)
}

/// Rozsyła zdarzenia rozliczenia do podłączonych agentów (każdy dostaje tylko swoje).
pub fn publish(tx: &broadcast::Sender<WsEnvelope>, events: Vec<SettledBet>) {
    for event in events {
        // Brak odbiorców (żaden agent nie jest online) nie jest błędem
        let _ = tx.send(event.into_envelope());
    }
}
