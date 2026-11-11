// tests/betting_exchange.rs
use backend::domain::betting_exchange::{check_bettable, expected_value, Selection};
use chrono::{Duration, Utc};
use rust_decimal::Decimal;

fn dec(v: f64) -> Decimal {
    Decimal::try_from(v).unwrap()
}

#[test]
fn parses_selection_case_insensitively() {
    assert_eq!(Selection::parse("home").unwrap(), Selection::Home);
    assert_eq!(Selection::parse(" DRAW ").unwrap(), Selection::Draw);
    assert_eq!(Selection::parse("Away").unwrap(), Selection::Away);
    assert!(Selection::parse("x").is_err());
}

#[test]
fn rejects_started_or_finished_matches_outside_backtest() {
    let now = Utc::now();
    assert!(check_bettable("SCHEDULED", now + Duration::hours(1), now, false).is_ok());
    assert!(check_bettable("SCHEDULED", now - Duration::hours(1), now, false).is_err());
    assert!(check_bettable("FINISHED", now - Duration::days(1), now, false).is_err());
    assert!(check_bettable("LIVE", now - Duration::minutes(5), now, false).is_err());
}

#[test]
fn backtest_window_allows_only_finished_matches() {
    let now = Utc::now();
    assert!(check_bettable("FINISHED", now - Duration::days(30), now, true).is_ok());
    assert!(check_bettable("SCHEDULED", now + Duration::days(1), now, true).is_err());
}

#[test]
fn computes_expected_value() {
    assert_eq!(expected_value(dec(0.5), dec(2.2)), dec(0.1));
    assert!(expected_value(dec(0.3), dec(2.0)) < Decimal::ZERO);
}
