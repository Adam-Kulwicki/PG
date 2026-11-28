// src/domain/backtester.rs
//! Silnik backtestingu („Wehikuł czasu”): odtwarza historyczny sezon dzień po dniu
//! na w pełni izolowanych, wirtualnych portfelach w pamięci serwera.
//!
//! Dla każdego dnia kalendarzowego (UTC):
//! 1. mecze tego dnia są "otwierane" na zakłady (`BacktestStore::open_day`),
//! 2. agenci dostają `MATCH_OPPORTUNITY` (kursy + predykcje, **bez wyniku**),
//! 3. serwer czeka `bet_window_ms` na komendy `PLACE_BET`,
//! 4. okno jest zamykane, a wirtualne kupony dnia są rozliczane z zapisanym wynikiem (`MATCH_SETTLED`).
//!
//! Backtester nigdy nie modyfikuje produkcyjnych tabel `agents` ani `bets`.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tokio::sync::RwLock;
use utoipa::ToSchema;

use crate::{
    domain::{
        betting_exchange::{self, Selection},
        settlement::{self, SettledBet},
    },
    error::{AppError, AppResult},
    state::AppState,
    ws::{
        protocol::{ServerMessage, WsEnvelope},
        session,
    },
};

const DEFAULT_BET_WINDOW_MS: u64 = 500;
const MAX_BET_WINDOW_MS: u64 = 30_000;
/// Maksymalna liczba zakończonych/przerwanych przebiegów backtestu trzymanych w pamięci.
pub const MAX_STORED_RUNS: usize = 20;

/// Parametry uruchomienia symulacji historycznej (`POST /api/v1/simulation/backtest`).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BacktestParams {
    /// Sezon do odtworzenia, np. `"2024-2025"` lub `"2026"` (wymagane, niepuste).
    #[schema(example = "2024-2025")]
    pub season: String,
    /// Opcjonalny filtr dyscypliny: `"FOOTBALL"`, `"BASKETBALL"`, `"TENNIS"` lub `"CS2"`.
    #[schema(example = "FOOTBALL")]
    pub sport: Option<String>,
    /// Czas (ms) na zakłady agentów w każdym dniu symulacji (domyślnie `500`, maks. `30000`).
    #[schema(example = 500)]
    pub bet_window_ms: Option<u64>,
    /// Opcjonalny wspólny kapitał początkowy wirtualnych portfeli dla tej symulacji (`> 0`, maks. 2 miejsca po przecinku).
    /// Gdy `None`, każdy agent startuje ze swoim `agents.initial_bankroll` z bazy.
    #[schema(example = 1000.00)]
    pub initial_bankroll: Option<Decimal>,
}

/// Plan uruchomionej symulacji historycznej zwracany z kodem `202 Accepted`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BacktestPlan {
    #[schema(example = 1)]
    pub run_id: u64,
    #[schema(example = "2024-2025")]
    pub season: String,
    #[schema(example = "FOOTBALL")]
    pub sport: Option<String>,
    #[schema(example = 380)]
    pub total_matches: usize,
    #[schema(example = 112)]
    pub total_days: usize,
    #[schema(example = 500)]
    pub bet_window_ms: u64,
    #[schema(example = 1000.00)]
    pub initial_bankroll: Option<Decimal>,
    /// Szacowany czas trwania symulacji w sekundach (`total_days * bet_window_ms / 1000`).
    #[schema(example = 56.0)]
    pub estimated_seconds: f64,
}

/// Stan pojedynczego przebiegu backtestu (`RUNNING`, `COMPLETED`, `FAILED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BacktestStatus {
    Running,
    Completed,
    Failed,
}

/// Wirtualny portfel agenta istniejący wyłącznie w ramach jednego przebiegu backtestu.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VirtualWallet {
    pub agent_id: i32,
    pub agent_name: String,
    pub strategy_description: Option<String>,
    pub initial_bankroll: Decimal,
    pub current_bankroll: Decimal,
}

/// Wirtualny kupon postawiony podczas symulacji backtestera.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VirtualBet {
    #[schema(example = 1)]
    pub id: u64,
    #[schema(example = 1)]
    pub agent_id: i32,
    #[schema(example = 42)]
    pub match_id: i32,
    /// Data historycznego meczu (ułatwia budowanie wykresów krzywej kapitału).
    pub match_date: DateTime<Utc>,
    /// Obstawiony wynik: `"HOME"`, `"DRAW"` lub `"AWAY"`.
    #[schema(example = "HOME")]
    pub bet_selection: String,
    #[schema(example = 50.00)]
    pub stake: Decimal,
    #[schema(example = 2.15)]
    pub odds_taken: Decimal,
    #[schema(example = 0.075)]
    pub expected_value: Option<Decimal>,
    /// Stan kuponu: `"PENDING"`, `"WON"`, `"LOST"` lub `"VOID"`.
    #[schema(example = "WON")]
    pub status: String,
    #[schema(example = 57.50)]
    pub profit_loss: Decimal,
    /// Saldo wirtualnego portfela po rozliczeniu tego kuponu (`None`, dopóki `PENDING`).
    #[schema(example = 1057.50)]
    pub bankroll_after: Option<Decimal>,
    #[schema(example = "Dodatnie EV z ułamkowego Kelly'ego")]
    pub reasoning: Option<String>,
    pub placed_at: DateTime<Utc>,
    pub settled_at: Option<DateTime<Utc>>,
}

/// Wiersz rankingu dla pojedynczego przebiegu backtestu.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct BacktestLeaderboardEntry {
    #[schema(example = 1)]
    pub agent_id: i32,
    #[schema(example = "Agent_Kelly_Value")]
    pub agent_name: String,
    #[schema(example = "Stawia wg ułamkowego Kryterium Kelly'ego przy EV > 5%")]
    pub strategy_description: Option<String>,
    #[schema(example = 1000.00)]
    pub initial_bankroll: Decimal,
    #[schema(example = 1240.50)]
    pub current_bankroll: Decimal,
    #[schema(example = 240.50)]
    pub total_profit: Decimal,
    #[schema(example = 24.05)]
    pub roi_pct: Decimal,
    #[schema(example = 85)]
    pub total_bets: usize,
    #[schema(example = 85)]
    pub settled_bets: usize,
    #[schema(example = 48)]
    pub won_bets: usize,
    #[schema(example = 37)]
    pub lost_bets: usize,
    #[schema(example = 0)]
    pub void_bets: usize,
}

/// Skrócone podsumowanie przebiegu backtestu (bez pełnej listy kuponów).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestRunSummary {
    #[schema(example = 1)]
    pub run_id: u64,
    pub status: BacktestStatus,
    #[schema(example = "2024-2025")]
    pub season: String,
    #[schema(example = "FOOTBALL")]
    pub sport: Option<String>,
    #[schema(example = 380)]
    pub total_matches: usize,
    #[schema(example = 112)]
    pub total_days: usize,
    #[schema(example = 112)]
    pub completed_days: usize,
    #[schema(example = 500)]
    pub bet_window_ms: u64,
    #[schema(example = 1000.00)]
    pub initial_bankroll: Option<Decimal>,
    #[schema(example = 140)]
    pub total_bets: usize,
    #[schema(example = 140)]
    pub settled_bets: usize,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// Pełny raport pojedynczego przebiegu backtestu wraz z rankingiem i wirtualnymi kuponami.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestReport {
    #[schema(example = 1)]
    pub run_id: u64,
    pub status: BacktestStatus,
    #[schema(example = "2024-2025")]
    pub season: String,
    #[schema(example = "FOOTBALL")]
    pub sport: Option<String>,
    #[schema(example = 380)]
    pub total_matches: usize,
    #[schema(example = 112)]
    pub total_days: usize,
    #[schema(example = 112)]
    pub completed_days: usize,
    #[schema(example = 500)]
    pub bet_window_ms: u64,
    #[schema(example = 1000.00)]
    pub initial_bankroll: Option<Decimal>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub leaderboard: Vec<BacktestLeaderboardEntry>,
    pub bets: Vec<VirtualBet>,
}

impl BacktestReport {
    /// Zwraca skrócone podsumowanie tego przebiegu (m.in. dla listy historii uruchomień).
    pub fn summary(&self) -> BacktestRunSummary {
        let settled_bets = self.bets.iter().filter(|b| b.status != "PENDING").count();
        BacktestRunSummary {
            run_id: self.run_id,
            status: self.status,
            season: self.season.clone(),
            sport: self.sport.clone(),
            total_matches: self.total_matches,
            total_days: self.total_days,
            completed_days: self.completed_days,
            bet_window_ms: self.bet_window_ms,
            initial_bankroll: self.initial_bankroll,
            total_bets: self.bets.len(),
            settled_bets,
            started_at: self.started_at,
            finished_at: self.finished_at,
            error: self.error.clone(),
        }
    }
}

/// Niemodyfikowalny snapshot meczu historycznego wczytywany z bazy przed startem symulacji.
#[derive(Debug, Clone, FromRow)]
pub struct BacktestMatchSnapshot {
    pub match_id: i32,
    pub match_date: DateTime<Utc>,
    pub status: String,
    pub winner: Option<String>,
    pub odds_home: Option<Decimal>,
    pub odds_draw: Option<Decimal>,
    pub odds_away: Option<Decimal>,
    pub avg_prob_home: Option<Decimal>,
    pub avg_prob_draw: Option<Decimal>,
    pub avg_prob_away: Option<Decimal>,
}

/// Dane startowe aktywnego agenta potrzebne do utworzenia wirtualnego portfela.
#[derive(Debug, Clone, FromRow)]
pub struct BacktestAgentSeed {
    pub id: i32,
    pub name: String,
    pub strategy_description: Option<String>,
    pub initial_bankroll: Decimal,
}

/// RAII: zwalnia flagę "backtest trwa" także przy błędzie lub panice w tle.
pub(crate) struct RunningGuard(Arc<AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Izolowana sesja pojedynczego przebiegu backtestu (czysta logika domenowa bez dostępu do DB).
#[derive(Debug, Clone)]
pub struct BacktestSession {
    run_id: u64,
    status: BacktestStatus,
    season: String,
    sport: Option<String>,
    total_matches: usize,
    total_days: usize,
    completed_days: usize,
    bet_window_ms: u64,
    initial_bankroll: Option<Decimal>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    error: Option<String>,
    open_matches: HashSet<i32>,
    matches: HashMap<i32, BacktestMatchSnapshot>,
    wallets: BTreeMap<i32, VirtualWallet>,
    bets: Vec<VirtualBet>,
    placed_keys: HashSet<(i32, i32)>,
    next_bet_id: u64,
}

impl BacktestSession {
    /// Tworzy nową sesję z pre-inicjalizowanymi wirtualnymi portfelami aktywnych agentów.
    pub fn new(
        plan: &BacktestPlan,
        matches: Vec<BacktestMatchSnapshot>,
        agents: Vec<BacktestAgentSeed>,
    ) -> Self {
        let mut wallets = BTreeMap::new();
        for agent in agents {
            let start_bankroll = plan.initial_bankroll.unwrap_or(agent.initial_bankroll);
            wallets.insert(
                agent.id,
                VirtualWallet {
                    agent_id: agent.id,
                    agent_name: agent.name,
                    strategy_description: agent.strategy_description,
                    initial_bankroll: start_bankroll,
                    current_bankroll: start_bankroll,
                },
            );
        }

        let match_map = matches
            .into_iter()
            .map(|snapshot| (snapshot.match_id, snapshot))
            .collect();

        Self {
            run_id: plan.run_id,
            status: BacktestStatus::Running,
            season: plan.season.clone(),
            sport: plan.sport.clone(),
            total_matches: plan.total_matches,
            total_days: plan.total_days,
            completed_days: 0,
            bet_window_ms: plan.bet_window_ms,
            initial_bankroll: plan.initial_bankroll,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
            open_matches: HashSet::new(),
            matches: match_map,
            wallets,
            bets: Vec::new(),
            placed_keys: HashSet::new(),
            next_bet_id: 1,
        }
    }

    /// Otwiera okno zakładów na mecze bieżącego dnia symulacji.
    pub fn open_day(&mut self, day_ids: &[i32]) {
        self.open_matches.clear();
        self.open_matches.extend(day_ids.iter().copied());
    }

    /// Zamyka aktualne okno zakładów.
    pub fn close_window(&mut self) {
        self.open_matches.clear();
    }

    /// Czy podany mecz znajduje się aktualnie w otwartym oknie backtestu.
    pub fn is_match_open(&self, match_id: i32) -> bool {
        self.open_matches.contains(&match_id)
    }

    /// Czy agent ma już utworzony wirtualny portfel w tej sesji.
    pub fn has_wallet(&self, agent_id: i32) -> bool {
        self.wallets.contains_key(&agent_id)
    }

    /// Tworzy wirtualny portfel dla agenta, jeśli jeszcze nie istnieje (np. dla agenta dodanego w trakcie),
    /// i zwraca jego aktualne saldo.
    pub fn ensure_wallet(&mut self, seed: BacktestAgentSeed) -> Decimal {
        let initial = self.initial_bankroll.unwrap_or(seed.initial_bankroll);
        let wallet = self
            .wallets
            .entry(seed.id)
            .or_insert_with(|| VirtualWallet {
                agent_id: seed.id,
                agent_name: seed.name,
                strategy_description: seed.strategy_description,
                initial_bankroll: initial,
                current_bankroll: initial,
            });
        wallet.current_bankroll
    }

    /// Zwraca aktualne saldo wirtualnego portfela agenta.
    pub fn get_bankroll(&self, agent_id: i32) -> Option<Decimal> {
        self.wallets.get(&agent_id).map(|w| w.current_bankroll)
    }

    /// Zwraca listę `(agent_id, current_bankroll)` wszystkich portfeli w sesji.
    pub fn wallet_balances(&self) -> Vec<(i32, Decimal)> {
        self.wallets
            .iter()
            .map(|(&agent_id, w)| (agent_id, w.current_bankroll))
            .collect()
    }

    /// Przyjmuje wirtualny zakład w pamięci (bez zapisu do tabel produkcyjnych).
    pub fn place_bet(
        &mut self,
        agent_id: i32,
        match_id: i32,
        selection: Selection,
        stake: Decimal,
        reasoning: Option<String>,
    ) -> AppResult<(VirtualBet, Decimal)> {
        if stake <= Decimal::ZERO {
            return Err(AppError::BadRequest("Stawka musi być dodatnia".to_string()));
        }
        if stake.round_dp(2) != stake {
            return Err(AppError::BadRequest(
                "Stawka może mieć maksymalnie 2 miejsca po przecinku".to_string(),
            ));
        }

        if !self.open_matches.contains(&match_id) {
            return Err(AppError::BetRejected(
                "okno zakładów dla tego meczu w backteście zostało zamknięte".to_string(),
            ));
        }

        let snapshot = self
            .matches
            .get(&match_id)
            .ok_or_else(|| AppError::NotFound(format!("Mecz o ID {} nie istnieje", match_id)))?;

        betting_exchange::check_bettable(&snapshot.status, snapshot.match_date, Utc::now(), true)?;

        let odds = match selection {
            Selection::Home => snapshot.odds_home,
            Selection::Draw => snapshot.odds_draw,
            Selection::Away => snapshot.odds_away,
        }
        .ok_or_else(|| {
            AppError::BetRejected(format!(
                "brak kursu dla typu {} w tym meczu",
                selection.as_str()
            ))
        })?;

        if self.placed_keys.contains(&(agent_id, match_id)) {
            return Err(AppError::BetRejected(
                "agent postawił już zakład na ten mecz".to_string(),
            ));
        }

        let wallet = self
            .wallets
            .get_mut(&agent_id)
            .ok_or_else(|| AppError::NotFound(format!("Agent o ID {} nie istnieje", agent_id)))?;

        if wallet.current_bankroll < stake {
            return Err(AppError::InsufficientFunds {
                required: stake.to_string(),
                available: wallet.current_bankroll.to_string(),
            });
        }

        let prob = match selection {
            Selection::Home => snapshot.avg_prob_home,
            Selection::Draw => snapshot.avg_prob_draw,
            Selection::Away => snapshot.avg_prob_away,
        };
        let ev = prob.map(|p| betting_exchange::expected_value(p, odds));

        wallet.current_bankroll -= stake;
        let new_bankroll = wallet.current_bankroll;

        self.placed_keys.insert((agent_id, match_id));
        let bet_id = self.next_bet_id;
        self.next_bet_id += 1;

        let bet = VirtualBet {
            id: bet_id,
            agent_id,
            match_id,
            match_date: snapshot.match_date,
            bet_selection: selection.as_str().to_string(),
            stake,
            odds_taken: odds,
            expected_value: ev,
            status: "PENDING".to_string(),
            profit_loss: Decimal::ZERO,
            bankroll_after: None,
            reasoning,
            placed_at: Utc::now(),
            settled_at: None,
        };

        self.bets.push(bet.clone());
        Ok((bet, new_bankroll))
    }

    /// Zamyka okno zakładów i rozlicza wszystkie oczekujące wirtualne kupony dla meczów danego dnia.
    pub fn settle_day(&mut self, day_ids: &[i32]) -> Vec<SettledBet> {
        self.open_matches.clear();

        let day_set: HashSet<i32> = day_ids.iter().copied().collect();
        let now = Utc::now();
        let mut events = Vec::new();

        for bet in &mut self.bets {
            if bet.status != "PENDING" || !day_set.contains(&bet.match_id) {
                continue;
            }
            let Some(snapshot) = self.matches.get(&bet.match_id) else {
                continue;
            };

            let outcome = settlement::resolve_outcome(
                &bet.bet_selection,
                &snapshot.status,
                snapshot.winner.as_deref(),
            );
            let (profit_loss, credit) = settlement::payout(outcome, bet.stake, bet.odds_taken);

            let new_bankroll = if let Some(wallet) = self.wallets.get_mut(&bet.agent_id) {
                wallet.current_bankroll += credit;
                wallet.current_bankroll
            } else {
                Decimal::ZERO
            };

            bet.status = outcome.as_str().to_string();
            bet.profit_loss = profit_loss;
            bet.bankroll_after = Some(new_bankroll);
            bet.settled_at = Some(now);

            events.push(SettledBet {
                agent_id: bet.agent_id,
                match_id: bet.match_id,
                winner: snapshot
                    .winner
                    .clone()
                    .unwrap_or_else(|| "VOID".to_string()),
                outcome,
                profit_loss,
                current_bankroll: new_bankroll,
            });
        }

        self.completed_days += 1;
        events
    }

    /// Wylicza ranking wirtualnych portfeli posortowany malejąco po `current_bankroll`.
    pub fn leaderboard(&self) -> Vec<BacktestLeaderboardEntry> {
        let mut entries: Vec<BacktestLeaderboardEntry> = self
            .wallets
            .values()
            .map(|wallet| {
                let mut total_bets = 0usize;
                let mut settled_bets = 0usize;
                let mut won_bets = 0usize;
                let mut lost_bets = 0usize;
                let mut void_bets = 0usize;

                for bet in self.bets.iter().filter(|b| b.agent_id == wallet.agent_id) {
                    total_bets += 1;
                    match bet.status.as_str() {
                        "WON" => {
                            settled_bets += 1;
                            won_bets += 1;
                        }
                        "LOST" => {
                            settled_bets += 1;
                            lost_bets += 1;
                        }
                        "VOID" => {
                            settled_bets += 1;
                            void_bets += 1;
                        }
                        _ => {}
                    }
                }

                let total_profit = wallet.current_bankroll - wallet.initial_bankroll;
                let roi_pct = if wallet.initial_bankroll > Decimal::ZERO {
                    ((total_profit / wallet.initial_bankroll) * Decimal::from(100)).round_dp(2)
                } else {
                    Decimal::ZERO
                };

                BacktestLeaderboardEntry {
                    agent_id: wallet.agent_id,
                    agent_name: wallet.agent_name.clone(),
                    strategy_description: wallet.strategy_description.clone(),
                    initial_bankroll: wallet.initial_bankroll,
                    current_bankroll: wallet.current_bankroll,
                    total_profit,
                    roi_pct,
                    total_bets,
                    settled_bets,
                    won_bets,
                    lost_bets,
                    void_bets,
                }
            })
            .collect();

        entries.sort_by(|a, b| {
            b.current_bankroll
                .cmp(&a.current_bankroll)
                .then_with(|| a.agent_id.cmp(&b.agent_id))
        });
        entries
    }

    /// Buduje pełny raport z posortowanymi chronologicznie kuponami (`match_date ASC, id ASC`).
    pub fn report(&self) -> BacktestReport {
        let mut bets = self.bets.clone();
        bets.sort_by(|a, b| {
            a.match_date
                .cmp(&b.match_date)
                .then_with(|| a.id.cmp(&b.id))
        });

        BacktestReport {
            run_id: self.run_id,
            status: self.status,
            season: self.season.clone(),
            sport: self.sport.clone(),
            total_matches: self.total_matches,
            total_days: self.total_days,
            completed_days: self.completed_days,
            bet_window_ms: self.bet_window_ms,
            initial_bankroll: self.initial_bankroll,
            started_at: self.started_at,
            finished_at: self.finished_at,
            error: self.error.clone(),
            leaderboard: self.leaderboard(),
            bets,
        }
    }

    /// Skrócone podsumowanie aktualnego stanu sesji.
    pub fn summary(&self) -> BacktestRunSummary {
        let settled_bets = self.bets.iter().filter(|b| b.status != "PENDING").count();
        BacktestRunSummary {
            run_id: self.run_id,
            status: self.status,
            season: self.season.clone(),
            sport: self.sport.clone(),
            total_matches: self.total_matches,
            total_days: self.total_days,
            completed_days: self.completed_days,
            bet_window_ms: self.bet_window_ms,
            initial_bankroll: self.initial_bankroll,
            total_bets: self.bets.len(),
            settled_bets,
            started_at: self.started_at,
            finished_at: self.finished_at,
            error: self.error.clone(),
        }
    }
}

/// Magazyn w pamięci serwera izolujący wirtualne portfele i historię przebiegów backtestu.
/// Zaprojektowany jako punkt styku pod przyszłą persystencję w PostgreSQL.
pub struct BacktestStore {
    running: Arc<AtomicBool>,
    next_run_id: AtomicU64,
    active: RwLock<Option<BacktestSession>>,
    history: RwLock<VecDeque<BacktestReport>>,
    max_stored_runs: usize,
}

impl Default for BacktestStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BacktestStore {
    pub fn new() -> Self {
        Self::with_max_runs(MAX_STORED_RUNS)
    }

    pub fn with_max_runs(max_stored_runs: usize) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            next_run_id: AtomicU64::new(1),
            active: RwLock::new(None),
            history: RwLock::new(VecDeque::new()),
            max_stored_runs,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub(crate) fn try_acquire_run(&self) -> Option<RunningGuard> {
        self.running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| RunningGuard(self.running.clone()))
    }

    pub fn allocate_run_id(&self) -> u64 {
        self.next_run_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn set_active(&self, session: BacktestSession) {
        *self.active.write().await = Some(session);
    }

    pub async fn open_day(&self, day_ids: &[i32]) {
        if let Some(session) = self.active.write().await.as_mut() {
            session.open_day(day_ids);
        }
    }

    pub async fn close_window(&self) {
        if let Some(session) = self.active.write().await.as_mut() {
            session.close_window();
        }
    }

    pub async fn is_match_open(&self, match_id: i32) -> bool {
        self.active
            .read()
            .await
            .as_ref()
            .is_some_and(|session| session.is_match_open(match_id))
    }

    pub async fn has_wallet(&self, agent_id: i32) -> bool {
        self.active
            .read()
            .await
            .as_ref()
            .is_some_and(|session| session.has_wallet(agent_id))
    }

    pub async fn ensure_wallet(&self, seed: BacktestAgentSeed) -> Option<Decimal> {
        let mut active = self.active.write().await;
        active.as_mut().map(|session| session.ensure_wallet(seed))
    }

    pub async fn get_bankroll(&self, agent_id: i32) -> Option<Decimal> {
        self.active
            .read()
            .await
            .as_ref()
            .and_then(|session| session.get_bankroll(agent_id))
    }

    pub async fn place_bet(
        &self,
        agent_id: i32,
        match_id: i32,
        selection: Selection,
        stake: Decimal,
        reasoning: Option<String>,
    ) -> AppResult<(VirtualBet, Decimal)> {
        let mut active = self.active.write().await;
        let session = active.as_mut().ok_or_else(|| {
            AppError::BetRejected(
                "okno zakładów dla tego meczu w backteście zostało zamknięte".to_string(),
            )
        })?;
        session.place_bet(agent_id, match_id, selection, stake, reasoning)
    }

    pub async fn settle_day(&self, day_ids: &[i32]) -> Vec<SettledBet> {
        let mut active = self.active.write().await;
        match active.as_mut() {
            Some(session) => session.settle_day(day_ids),
            None => Vec::new(),
        }
    }

    /// Zamyka aktywną sesję jako zakończoną sukcesem i przenosi jej raport do historii.
    pub async fn finish_active(&self) -> Option<BacktestReport> {
        self.archive_active(BacktestStatus::Completed, None).await
    }

    /// Zamyka aktywną sesję jako przerwaną błędem i zapisuje komunikat w historii.
    pub async fn fail_active(&self, err: String) -> Option<BacktestReport> {
        self.archive_active(BacktestStatus::Failed, Some(err)).await
    }

    async fn archive_active(
        &self,
        status: BacktestStatus,
        error: Option<String>,
    ) -> Option<BacktestReport> {
        let mut session = self.active.write().await.take()?;
        session.close_window();
        session.status = status;
        session.finished_at = Some(Utc::now());
        session.error = error;

        let report = session.report();
        if self.max_stored_runs > 0 {
            let mut history = self.history.write().await;
            history.push_front(report.clone());
            while history.len() > self.max_stored_runs {
                history.pop_back();
            }
        }
        Some(report)
    }

    /// Zwraca raport aktywnego przebiegu lub ostatniego zakończonego przebiegu.
    pub async fn latest_report(&self) -> Option<BacktestReport> {
        if let Some(session) = self.active.read().await.as_ref() {
            return Some(session.report());
        }
        self.history.read().await.front().cloned()
    }

    /// Zwraca pełny raport wybranego przebiegu po `run_id`.
    pub async fn get_report(&self, run_id: u64) -> Option<BacktestReport> {
        if let Some(session) = self.active.read().await.as_ref() {
            if session.run_id == run_id {
                return Some(session.report());
            }
        }
        self.history
            .read()
            .await
            .iter()
            .find(|r| r.run_id == run_id)
            .cloned()
    }

    /// Zwraca listę przebiegów od najnowszego (aktywny na początku, jeśli trwa).
    pub async fn list_runs(&self) -> Vec<BacktestRunSummary> {
        let mut runs = Vec::new();
        if let Some(session) = self.active.read().await.as_ref() {
            runs.push(session.summary());
        }
        let history = self.history.read().await;
        runs.extend(history.iter().map(BacktestReport::summary));
        runs
    }

    /// Zwraca ranking wirtualnych portfeli dla wybranego `run_id`.
    pub async fn get_leaderboard(&self, run_id: u64) -> Option<Vec<BacktestLeaderboardEntry>> {
        if let Some(session) = self.active.read().await.as_ref() {
            if session.run_id == run_id {
                return Some(session.leaderboard());
            }
        }
        self.history
            .read()
            .await
            .iter()
            .find(|r| r.run_id == run_id)
            .map(|r| r.leaderboard.clone())
    }

    /// Zwraca listę wirtualnych kuponów dla wybranego `run_id` (opcjonalnie przefiltrowaną po `agent_id`).
    pub async fn get_bets(&self, run_id: u64, agent_id: Option<i32>) -> Option<Vec<VirtualBet>> {
        let report = self.get_report(run_id).await?;
        let bets = match agent_id {
            Some(id) => report
                .bets
                .into_iter()
                .filter(|b| b.agent_id == id)
                .collect(),
            None => report.bets,
        };
        Some(bets)
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

/// Waliduje parametry, przygotowuje izolowane portfele i uruchamia symulację w tle (`tokio::spawn`).
pub async fn start(state: AppState, params: BacktestParams) -> AppResult<BacktestPlan> {
    if params.season.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Pole season jest wymagane".to_string(),
        ));
    }
    if let Some(bankroll) = params.initial_bankroll {
        if bankroll <= Decimal::ZERO {
            return Err(AppError::BadRequest(
                "Pole initial_bankroll musi być dodatnie".to_string(),
            ));
        }
        if bankroll.round_dp(2) != bankroll {
            return Err(AppError::BadRequest(
                "Pole initial_bankroll może mieć maksymalnie 2 miejsca po przecinku".to_string(),
            ));
        }
    }

    let bet_window_ms = params
        .bet_window_ms
        .unwrap_or(DEFAULT_BET_WINDOW_MS)
        .min(MAX_BET_WINDOW_MS);

    let guard = state.backtest.try_acquire_run().ok_or_else(|| {
        AppError::Conflict("Backtest już trwa – poczekaj na jego zakończenie".to_string())
    })?;

    // Tylko zakończone mecze z rozstrzygnięciem i kursami (bez kursu nie da się obstawiać)
    let snapshots: Vec<BacktestMatchSnapshot> = sqlx::query_as(
        r#"
        SELECT
            m.id AS match_id,
            m.match_date,
            m.status,
            m.winner,
            o.odds_home,
            o.odds_draw,
            o.odds_away,
            AVG(p.prob_home) AS avg_prob_home,
            AVG(p.prob_draw) AS avg_prob_draw,
            AVG(p.prob_away) AS avg_prob_away
        FROM matches m
        JOIN competitions c ON c.id = m.competition_id
        JOIN match_odds o ON o.match_id = m.id AND o.bookmaker = 'MARKET_AVG'
        LEFT JOIN predictions p ON p.match_id = m.id
        WHERE m.season = $1
          AND m.status = 'FINISHED'
          AND m.winner IS NOT NULL
          AND ($2::VARCHAR IS NULL OR c.sport = UPPER($2))
        GROUP BY m.id, m.match_date, m.status, m.winner, o.odds_home, o.odds_draw, o.odds_away
        ORDER BY m.match_date, m.id
        "#,
    )
    .bind(params.season.trim())
    .bind(params.sport.as_deref())
    .fetch_all(&state.db)
    .await?;

    if snapshots.is_empty() {
        return Err(AppError::NotFound(format!(
            "Brak zakończonych meczów z kursami dla sezonu {}",
            params.season
        )));
    }

    let agents: Vec<BacktestAgentSeed> = sqlx::query_as(
        r#"
        SELECT id, name, strategy_description, initial_bankroll
        FROM agents
        WHERE is_active = TRUE
        ORDER BY id ASC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    let match_dates: Vec<(i32, DateTime<Utc>)> = snapshots
        .iter()
        .map(|s| (s.match_id, s.match_date))
        .collect();
    let days = group_by_day(&match_dates);

    let run_id = state.backtest.allocate_run_id();
    let plan = BacktestPlan {
        run_id,
        season: params.season.trim().to_string(),
        sport: params.sport.clone(),
        total_matches: snapshots.len(),
        total_days: days.len(),
        bet_window_ms,
        initial_bankroll: params.initial_bankroll,
        estimated_seconds: days.len() as f64 * bet_window_ms as f64 / 1000.0,
    };

    let session = BacktestSession::new(&plan, snapshots, agents);
    let initial_wallets = session.wallet_balances();
    state.backtest.set_active(session).await;

    let task_plan = plan.clone();
    tokio::spawn(async move {
        let _guard = guard; // zwolni flagę po zakończeniu zadania
        match run(&state, task_plan.clone(), days, initial_wallets).await {
            Ok(settled_total) => {
                let leaderboard = state
                    .backtest
                    .finish_active()
                    .await
                    .map(|report| report.leaderboard)
                    .unwrap_or_default();

                broadcast(
                    &state,
                    ServerMessage::BacktestFinished {
                        run_id: task_plan.run_id,
                        season: task_plan.season.clone(),
                        total_matches: task_plan.total_matches,
                        settled_bets: settled_total,
                        leaderboard,
                    },
                );
                tracing::info!(
                    run_id = task_plan.run_id,
                    season = %task_plan.season,
                    settled_bets = settled_total,
                    "Backtest zakończony"
                );
            }
            Err(err) => {
                tracing::error!(
                    run_id = task_plan.run_id,
                    "Backtest przerwany błędem: {}",
                    err
                );
                state.backtest.fail_active(err.to_string()).await;
            }
        }
    });

    Ok(plan)
}

async fn run(
    state: &AppState,
    plan: BacktestPlan,
    days: Vec<Vec<i32>>,
    initial_wallets: Vec<(i32, Decimal)>,
) -> AppResult<usize> {
    tracing::info!(
        run_id = plan.run_id,
        season = %plan.season,
        matches = plan.total_matches,
        days = plan.total_days,
        "Backtest wystartował na izolowanych portfelach"
    );

    broadcast(
        state,
        ServerMessage::BacktestStarted {
            run_id: plan.run_id,
            season: plan.season.clone(),
            sport: plan.sport.clone(),
            total_matches: plan.total_matches,
            total_days: plan.total_days,
            initial_bankroll: plan.initial_bankroll,
        },
    );

    for (agent_id, bankroll) in initial_wallets {
        let _ = state.ws_tx.send(WsEnvelope::to_agent(
            agent_id,
            ServerMessage::Bankroll {
                current_bankroll: bankroll,
            },
        ));
    }

    let mut settled_total = 0usize;

    for day_ids in days {
        // 1. Otwarcie okna zakładów dla meczów tego dnia
        state.backtest.open_day(&day_ids).await;

        // 2. Rozesłanie okazji (bez wyniku meczu!)
        for match_id in &day_ids {
            if let Some(opportunity) = session::build_opportunity(&state.db, *match_id).await? {
                broadcast(state, opportunity);
            }
        }

        // 3. Czas na PLACE_BET
        tokio::time::sleep(Duration::from_millis(plan.bet_window_ms)).await;

        // 4. Zamknięcie okna i rozliczenie wirtualnych zakładów dnia w pamięci
        let settled = state.backtest.settle_day(&day_ids).await;
        settled_total += settled.len();
        settlement::publish(&state.ws_tx, settled);
    }

    Ok(settled_total)
}

fn broadcast(state: &AppState, message: ServerMessage) {
    // Brak odbiorców nie jest błędem – symulacja i tak rozlicza się w wirtualnych portfelach
    let _ = state.ws_tx.send(WsEnvelope::to_all(message));
}
