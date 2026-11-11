// src/domain/backtester.rs
//! Silnik backtestingu („Wehikuł czasu”): odtwarza historyczny sezon dzień po dniu.
//!
//! Dla każdego dnia kalendarzowego (UTC):
//! 1. mecze tego dnia są "otwierane" na zakłady (`open_backtest_matches`),
//! 2. agenci dostają `MATCH_OPPORTUNITY` (kursy + predykcje, **bez wyniku**),
//! 3. serwer czeka `bet_window_ms` na komendy `PLACE_BET`,
//! 4. okno jest zamykane, a mecze dnia są rozliczane z zapisanym wynikiem (`MATCH_SETTLED`).
//!
//! Dzięki temu agent nigdy nie widzi wyniku meczu przed rozliczeniem (brak data leakage).

use std::{
    collections::BTreeMap,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

use crate::{
    domain::settlement,
    error::{AppError, AppResult},
    state::AppState,
    ws::{
        protocol::{ServerMessage, WsEnvelope},
        session,
    },
};

const DEFAULT_BET_WINDOW_MS: u64 = 500;
const MAX_BET_WINDOW_MS: u64 = 30_000;

#[derive(Debug, Clone, Deserialize)]
pub struct BacktestParams {
    /// Sezon do odtworzenia, np. "2025-2026"
    pub season: String,
    /// Opcjonalny filtr dyscypliny: FOOTBALL / BASKETBALL / TENNIS
    pub sport: Option<String>,
    /// Czas (ms) na zakłady agentów w każdym dniu symulacji (domyślnie 500, max 30000)
    pub bet_window_ms: Option<u64>,
    /// Usuwa wcześniejsze (rozliczone) zakłady na mecze tego backtestu i cofa ich wpływ na
    /// salda – pozwala powtarzać symulację tego samego sezonu.
    #[serde(default)]
    pub reset_agents: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestPlan {
    pub season: String,
    pub sport: Option<String>,
    pub total_matches: usize,
    pub total_days: usize,
    pub bet_window_ms: u64,
    pub estimated_seconds: f64,
}

/// RAII: zwalnia flagę "backtest trwa" także przy błędzie lub panice w tle.
struct RunningGuard(Arc<AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Grupuje mecze (posortowane chronologicznie) w paczki po dniu kalendarzowym UTC.
pub fn group_by_day(matches: &[(i32, DateTime<Utc>)]) -> Vec<Vec<i32>> {
    let mut days: BTreeMap<NaiveDate, Vec<i32>> = BTreeMap::new();
    for (id, date) in matches {
        days.entry(date.date_naive()).or_default().push(*id);
    }
    days.into_values().collect()
}

/// Waliduje parametry, przygotowuje plan i uruchamia symulację w tle (`tokio::spawn`).
pub async fn start(state: AppState, params: BacktestParams) -> AppResult<BacktestPlan> {
    if params.season.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Pole season jest wymagane".to_string(),
        ));
    }
    let bet_window_ms = params
        .bet_window_ms
        .unwrap_or(DEFAULT_BET_WINDOW_MS)
        .min(MAX_BET_WINDOW_MS);

    if state
        .backtest_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(AppError::Conflict(
            "Backtest już trwa – poczekaj na jego zakończenie".to_string(),
        ));
    }
    let guard = RunningGuard(state.backtest_running.clone());

    // Tylko zakończone mecze z rozstrzygnięciem i kursami (bez kursu nie da się obstawiać)
    let matches: Vec<(i32, DateTime<Utc>)> = sqlx::query_as(
        r#"
        SELECT m.id, m.match_date
        FROM matches m
        JOIN competitions c ON c.id = m.competition_id
        JOIN match_odds o ON o.match_id = m.id AND o.bookmaker = 'MARKET_AVG'
        WHERE m.season = $1
          AND m.status = 'FINISHED'
          AND m.winner IS NOT NULL
          AND ($2::VARCHAR IS NULL OR c.sport = UPPER($2))
        ORDER BY m.match_date, m.id
        "#,
    )
    .bind(params.season.trim())
    .bind(params.sport.as_deref())
    .fetch_all(&state.db)
    .await?;

    if matches.is_empty() {
        return Err(AppError::NotFound(format!(
            "Brak zakończonych meczów z kursami dla sezonu {}",
            params.season
        )));
    }

    if params.reset_agents {
        let ids: Vec<i32> = matches.iter().map(|(id, _)| *id).collect();
        reset_previous_bets(&state, &ids).await?;
    }

    let days = group_by_day(&matches);
    let plan = BacktestPlan {
        season: params.season.trim().to_string(),
        sport: params.sport.clone(),
        total_matches: matches.len(),
        total_days: days.len(),
        bet_window_ms,
        estimated_seconds: days.len() as f64 * bet_window_ms as f64 / 1000.0,
    };

    let task_plan = plan.clone();
    tokio::spawn(async move {
        let _guard = guard; // zwolni flagę po zakończeniu zadania
        if let Err(err) = run(&state, task_plan, days).await {
            tracing::error!("Backtest przerwany błędem: {}", err);
        }
        state.open_backtest_matches.write().await.clear();
    });

    Ok(plan)
}

/// Cofa rozliczone zakłady z poprzednich przebiegów: usuwa kupony i odejmuje ich zysk/stratę
/// od sald (atomowo, jednym zapytaniem).
async fn reset_previous_bets(state: &AppState, match_ids: &[i32]) -> AppResult<()> {
    sqlx::query(
        r#"
        WITH deleted AS (
            DELETE FROM bets
            WHERE match_id = ANY($1) AND status <> 'PENDING'
            RETURNING agent_id, profit_loss
        ),
        totals AS (
            SELECT agent_id, SUM(profit_loss) AS total FROM deleted GROUP BY agent_id
        )
        UPDATE agents a
        SET current_bankroll = a.current_bankroll - t.total
        FROM totals t
        WHERE a.id = t.agent_id
        "#,
    )
    .bind(match_ids)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn run(state: &AppState, plan: BacktestPlan, days: Vec<Vec<i32>>) -> AppResult<()> {
    tracing::info!(
        season = %plan.season,
        matches = plan.total_matches,
        days = plan.total_days,
        "Backtest wystartował"
    );

    broadcast(
        state,
        ServerMessage::BacktestStarted {
            season: plan.season.clone(),
            sport: plan.sport.clone(),
            total_matches: plan.total_matches,
            total_days: plan.total_days,
        },
    );

    let mut settled_total = 0usize;

    for day_ids in days {
        // 1. Otwarcie okna zakładów dla meczów tego dnia
        {
            let mut open = state.open_backtest_matches.write().await;
            open.clear();
            open.extend(day_ids.iter().copied());
        }

        // 2. Rozesłanie okazji (bez wyniku meczu!)
        for match_id in &day_ids {
            if let Some(opportunity) = session::build_opportunity(&state.db, *match_id).await? {
                broadcast(state, opportunity);
            }
        }

        // 3. Czas na PLACE_BET
        tokio::time::sleep(Duration::from_millis(plan.bet_window_ms)).await;

        // 4. Zamknięcie okna i rozliczenie dnia
        state.open_backtest_matches.write().await.clear();
        let settled = settlement::settle_matches(&state.db, Some(&day_ids)).await?;
        settled_total += settled.len();
        settlement::publish(&state.ws_tx, settled);
    }

    broadcast(
        state,
        ServerMessage::BacktestFinished {
            season: plan.season.clone(),
            total_matches: plan.total_matches,
            settled_bets: settled_total,
        },
    );
    tracing::info!(season = %plan.season, settled_bets = settled_total, "Backtest zakończony");
    Ok(())
}

fn broadcast(state: &AppState, message: ServerMessage) {
    // Brak odbiorców nie jest błędem – symulacja i tak rozlicza się w bazie
    let _ = state.ws_tx.send(WsEnvelope::to_all(message));
}
