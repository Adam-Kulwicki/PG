// tests/backtester.rs
use backend::{
    domain::{
        backtester::{
            group_by_day, BacktestAgentSeed, BacktestMatchSnapshot, BacktestPlan, BacktestSession,
            BacktestStatus, BacktestStore,
        },
        betting_exchange::Selection,
        settlement::BetOutcome,
    },
    error::AppError,
};
use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn sample_plan(run_id: u64, initial_bankroll: Option<Decimal>) -> BacktestPlan {
    BacktestPlan {
        run_id,
        season: "2025-2026".to_string(),
        sport: Some("FOOTBALL".to_string()),
        total_matches: 2,
        total_days: 2,
        bet_window_ms: 500,
        initial_bankroll,
        estimated_seconds: 1.0,
    }
}

fn sample_matches() -> Vec<BacktestMatchSnapshot> {
    vec![
        BacktestMatchSnapshot {
            match_id: 101,
            match_date: Utc.with_ymd_and_hms(2025, 8, 10, 15, 0, 0).unwrap(),
            status: "FINISHED".to_string(),
            winner: Some("HOME".to_string()),
            odds_home: Some(d("2.50")),
            odds_draw: Some(d("3.20")),
            odds_away: Some(d("2.80")),
            avg_prob_home: Some(d("0.50")),
            avg_prob_draw: Some(d("0.25")),
            avg_prob_away: Some(d("0.25")),
        },
        BacktestMatchSnapshot {
            match_id: 102,
            match_date: Utc.with_ymd_and_hms(2025, 8, 11, 18, 0, 0).unwrap(),
            status: "CANCELLED".to_string(),
            winner: None,
            odds_home: Some(d("1.90")),
            odds_draw: None,
            odds_away: Some(d("2.00")),
            avg_prob_home: Some(d("0.55")),
            avg_prob_draw: Some(d("0.00")),
            avg_prob_away: Some(d("0.45")),
        },
    ]
}

fn sample_agents() -> Vec<BacktestAgentSeed> {
    vec![
        BacktestAgentSeed {
            id: 1,
            name: "Agent_Kelly".to_string(),
            strategy_description: Some("Kelly".to_string()),
            initial_bankroll: d("1000.00"),
        },
        BacktestAgentSeed {
            id: 2,
            name: "Agent_Flat".to_string(),
            strategy_description: Some("Flat".to_string()),
            initial_bankroll: d("500.00"),
        },
    ]
}

/// Mecze trafiają do paczek według dnia UTC, w kolejności chronologicznej.
#[test]
fn groups_matches_by_utc_day_in_order() {
    let dt = |day, hour| Utc.with_ymd_and_hms(2025, 8, day, hour, 0, 0).unwrap();
    let matches = vec![
        (1, dt(1, 12)),
        (2, dt(1, 20)),
        (3, dt(2, 15)),
        (4, dt(4, 9)),
    ];
    assert_eq!(group_by_day(&matches), vec![vec![1, 2], vec![3], vec![4]]);
}

/// Wirtualne portfele izolują zakłady i poprawnie rozliczają WON, LOST oraz VOID.
#[test]
fn virtual_wallets_isolate_bets_and_settle_won_lost_void() {
    // 1. Gdy initial_bankroll = None, agenci biorą własne initial_bankroll z bazy
    let session_default =
        BacktestSession::new(&sample_plan(1, None), sample_matches(), sample_agents());
    assert_eq!(session_default.get_bankroll(1), Some(d("1000.00")));
    assert_eq!(session_default.get_bankroll(2), Some(d("500.00")));

    // 2. Gdy initial_bankroll = Some(2000), nadpisuje kapitał obu wirtualnych portfeli
    let mut session = BacktestSession::new(
        &sample_plan(2, Some(d("2000.00"))),
        sample_matches(),
        sample_agents(),
    );
    assert_eq!(session.get_bankroll(1), Some(d("2000.00")));
    assert_eq!(session.get_bankroll(2), Some(d("2000.00")));

    // Dzień 1: mecz 101 (FINISHED, winner = HOME, odds_home = 2.50, odds_away = 2.80)
    session.open_day(&[101]);
    let (bet1, bal1_after_place) = session
        .place_bet(
            1,
            101,
            Selection::Home,
            d("100.00"),
            Some("Value".to_string()),
        )
        .unwrap();
    assert_eq!(bal1_after_place, d("1900.00"));
    assert_eq!(bet1.status, "PENDING");
    assert_eq!(bet1.bankroll_after, None);
    // EV = 0.50 * 2.50 - 1 = 0.2500
    assert_eq!(bet1.expected_value, Some(d("0.2500")));

    let (bet2, bal2_after_place) = session
        .place_bet(2, 101, Selection::Away, d("50.00"), None)
        .unwrap();
    assert_eq!(bal2_after_place, d("1950.00"));
    // EV = 0.25 * 2.80 - 1 = -0.3000
    assert_eq!(bet2.expected_value, Some(d("-0.3000")));

    let settled_day1 = session.settle_day(&[101]);
    assert_eq!(settled_day1.len(), 2);
    assert!(!session.is_match_open(101));

    assert_eq!(settled_day1[0].agent_id, 1);
    assert_eq!(settled_day1[0].outcome, BetOutcome::Won);
    assert_eq!(settled_day1[0].profit_loss, d("150.00"));
    assert_eq!(settled_day1[0].current_bankroll, d("2150.00"));

    assert_eq!(settled_day1[1].agent_id, 2);
    assert_eq!(settled_day1[1].outcome, BetOutcome::Lost);
    assert_eq!(settled_day1[1].profit_loss, d("-50.00"));
    assert_eq!(settled_day1[1].current_bankroll, d("1950.00"));

    // Dzień 2: mecz 102 (CANCELLED -> VOID, zwrot stawki)
    let mut matches_with_day2_finished = sample_matches();
    matches_with_day2_finished[1].status = "FINISHED".to_string();
    let mut session_void = BacktestSession::new(
        &sample_plan(3, Some(d("2000.00"))),
        matches_with_day2_finished,
        sample_agents(),
    );
    session_void.open_day(&[101]);
    session_void
        .place_bet(1, 101, Selection::Home, d("100.00"), None)
        .unwrap();
    session_void
        .place_bet(2, 101, Selection::Away, d("50.00"), None)
        .unwrap();
    session_void.settle_day(&[101]);

    session_void.open_day(&[102]);
    let (_bet_void, bal_void_place) = session_void
        .place_bet(1, 102, Selection::Home, d("100.00"), None)
        .unwrap();
    assert_eq!(bal_void_place, d("2050.00"));
    let settled_day2 = session_void.settle_day(&[102]);
    assert_eq!(settled_day2.len(), 1);
    assert_eq!(settled_day2[0].outcome, BetOutcome::Void);
    assert_eq!(settled_day2[0].profit_loss, Decimal::ZERO);
    assert_eq!(settled_day2[0].current_bankroll, d("2150.00"));

    let lb = session_void.leaderboard();
    assert_eq!(lb.len(), 2);
    assert_eq!(lb[0].agent_id, 1);
    assert_eq!(lb[0].current_bankroll, d("2150.00"));
    assert_eq!(lb[0].total_profit, d("150.00"));
    assert_eq!(lb[0].roi_pct, d("7.50"));
    assert_eq!(lb[0].total_bets, 2);
    assert_eq!(lb[0].settled_bets, 2);
    assert_eq!(lb[0].won_bets, 1);
    assert_eq!(lb[0].lost_bets, 0);
    assert_eq!(lb[0].void_bets, 1);

    assert_eq!(lb[1].agent_id, 2);
    assert_eq!(lb[1].current_bankroll, d("1950.00"));
    assert_eq!(lb[1].total_profit, d("-50.00"));
    assert_eq!(lb[1].roi_pct, d("-2.50"));
    assert_eq!(lb[1].won_bets, 0);
    assert_eq!(lb[1].lost_bets, 1);
    assert_eq!(lb[1].void_bets, 0);

    let report = session_void.report();
    assert_eq!(report.bets.len(), 3);
    assert_eq!(report.bets[0].bankroll_after, Some(d("2150.00")));
    assert_eq!(report.bets[1].bankroll_after, Some(d("1950.00")));
    assert_eq!(report.bets[2].bankroll_after, Some(d("2150.00")));
}

/// Odrzucanie niepoprawnych stawek, zamkniętych okien, duplikatów, brakujących kursów i przekroczenia salda.
#[test]
fn rejects_invalid_duplicate_closed_and_over_budget_virtual_bets() {
    let mut matches = sample_matches();
    matches[1].status = "FINISHED".to_string();

    let mut session =
        BacktestSession::new(&sample_plan(1, Some(d("100.00"))), matches, sample_agents());

    // Okno zamknięte
    let err = session
        .place_bet(1, 101, Selection::Home, d("10.00"), None)
        .unwrap_err();
    assert!(matches!(err, AppError::BetRejected(_)));

    session.open_day(&[101, 102]);

    // Niepoprawna stawka (<= 0 i > 2 miejsca po przecinku)
    assert!(matches!(
        session.place_bet(1, 101, Selection::Home, Decimal::ZERO, None),
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        session.place_bet(1, 101, Selection::Home, d("10.123"), None),
        Err(AppError::BadRequest(_))
    ));

    // Brak kursu DRAW w meczu 102
    assert!(matches!(
        session.place_bet(1, 102, Selection::Draw, d("10.00"), None),
        Err(AppError::BetRejected(_))
    ));

    // Przekroczenie salda nie zmienia portfela
    let err = session
        .place_bet(1, 101, Selection::Home, d("150.00"), None)
        .unwrap_err();
    assert!(matches!(err, AppError::InsufficientFunds { .. }));
    assert_eq!(session.get_bankroll(1), Some(d("100.00")));

    // Poprawny zakład i duplikat na ten sam mecz
    session
        .place_bet(1, 101, Selection::Home, d("40.00"), None)
        .unwrap();
    assert_eq!(session.get_bankroll(1), Some(d("60.00")));

    let dup_err = session
        .place_bet(1, 101, Selection::Away, d("10.00"), None)
        .unwrap_err();
    assert!(matches!(dup_err, AppError::BetRejected(_)));
    assert_eq!(session.get_bankroll(1), Some(d("60.00")));
}

/// Kolejne przebiegi w BacktestStore są w pełni od siebie odizolowane i zachowują historię.
#[tokio::test]
async fn backtest_store_runs_are_isolated_and_preserve_history() {
    let store = BacktestStore::new();

    // Run 1: zysk agenta 1
    let run1 = store.allocate_run_id();
    let plan1 = sample_plan(run1, Some(d("1000.00")));
    store
        .set_active(BacktestSession::new(
            &plan1,
            sample_matches(),
            sample_agents(),
        ))
        .await;
    store.open_day(&[101]).await;
    store
        .place_bet(1, 101, Selection::Home, d("100.00"), None)
        .await
        .unwrap();
    store
        .place_bet(2, 101, Selection::Away, d("100.00"), None)
        .await
        .unwrap();
    store.settle_day(&[101]).await;
    let report1 = store.finish_active().await.unwrap();
    assert_eq!(report1.status, BacktestStatus::Completed);
    assert_eq!(report1.leaderboard[0].current_bankroll, d("1150.00"));

    // Run 2: świeże portfele (niezależne od wyniku Run 1!)
    let run2 = store.allocate_run_id();
    let plan2 = sample_plan(run2, Some(d("1000.00")));
    store
        .set_active(BacktestSession::new(
            &plan2,
            sample_matches(),
            sample_agents(),
        ))
        .await;
    assert_eq!(store.get_bankroll(1).await, Some(d("1000.00")));
    assert_eq!(store.get_bankroll(2).await, Some(d("1000.00")));

    store.open_day(&[101]).await;
    store
        .place_bet(1, 101, Selection::Away, d("200.00"), None)
        .await
        .unwrap();
    store.settle_day(&[101]).await;
    let report2 = store.finish_active().await.unwrap();
    assert_eq!(report2.status, BacktestStatus::Completed);

    let runs = store.list_runs().await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_id, run2);
    assert_eq!(runs[1].run_id, run1);

    let lb1 = store.get_leaderboard(run1).await.unwrap();
    assert_eq!(lb1[0].agent_id, 1);
    assert_eq!(lb1[0].current_bankroll, d("1150.00"));

    let lb2 = store.get_leaderboard(run2).await.unwrap();
    assert_eq!(lb2[0].agent_id, 2);
    assert_eq!(lb2[0].current_bankroll, d("1000.00"));
    assert_eq!(lb2[1].agent_id, 1);
    assert_eq!(lb2[1].current_bankroll, d("800.00"));

    let bets_run1_agent1 = store.get_bets(run1, Some(1)).await.unwrap();
    assert_eq!(bets_run1_agent1.len(), 1);
    assert_eq!(bets_run1_agent1[0].agent_id, 1);

    let latest = store.latest_report().await.unwrap();
    assert_eq!(latest.run_id, run2);
}

/// BacktestStore zapisuje przerwane przebiegi jako FAILED i respektuje limit historii.
#[tokio::test]
async fn backtest_store_tracks_failed_runs_and_caps_history() {
    let store = BacktestStore::with_max_runs(2);

    for i in 1..=3u64 {
        let run_id = store.allocate_run_id();
        assert_eq!(run_id, i);
        let plan = sample_plan(run_id, Some(d("500.00")));
        store
            .set_active(BacktestSession::new(
                &plan,
                sample_matches(),
                sample_agents(),
            ))
            .await;

        if i == 2 {
            let failed = store
                .fail_active("utracono połączenie z bazą".to_string())
                .await
                .unwrap();
            assert_eq!(failed.status, BacktestStatus::Failed);
            assert_eq!(failed.error.as_deref(), Some("utracono połączenie z bazą"));
        } else {
            store.finish_active().await.unwrap();
        }
    }

    let runs = store.list_runs().await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_id, 3);
    assert_eq!(runs[0].status, BacktestStatus::Completed);
    assert_eq!(runs[1].run_id, 2);
    assert_eq!(runs[1].status, BacktestStatus::Failed);
    assert!(store.get_report(1).await.is_none());
}
