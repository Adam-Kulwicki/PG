// tests/predictions.rs
use backend::{
    api::predictions::validate_probabilities,
    domain::odds::{calculate_market_odds, prob_to_odds},
};
use rust_decimal::Decimal;

/// Skrót do tworzenia `Decimal` z napisu.
fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// Poprawne rozkłady (także bez remisu) przechodzą walidację.
#[test]
fn accepts_valid_probabilities() {
    assert!(validate_probabilities(d("0.34"), d("0.28"), d("0.38")).is_ok());
    assert!(validate_probabilities(d("0.68"), d("0"), d("0.32")).is_ok());
}

/// Suma różna od 1 lub wartości poza [0, 1] są odrzucane.
#[test]
fn rejects_invalid_probabilities() {
    assert!(validate_probabilities(d("0.5"), d("0.5"), d("0.5")).is_err());
    assert!(validate_probabilities(d("1.2"), d("0"), d("-0.2")).is_err());
}

/// Wyliczanie kursów `MARKET_AVG` z prawdopodobieństw i marży (1X2 dla piłki, 12 dla koszykówki/tenisa).
#[test]
fn calculates_market_odds_with_margin_and_clamping() {
    let margin = d("0.05");

    // Piłka nożna (1X2): 1 / (0.34 * 1.05) = 2.8011... -> 2.80, 1 / (0.28 * 1.05) = 3.40, 1 / (0.38 * 1.05) = 2.51
    let football = calculate_market_odds(d("0.34"), d("0.28"), d("0.38"), true, margin);
    assert_eq!(football.odds_home, d("2.80"));
    assert_eq!(football.odds_draw, Some(d("3.40")));
    assert_eq!(football.odds_away, d("2.51"));

    // Koszykówka / tenis (12): brak kursu na remis oraz 0.50 z marżą 5% -> 1.90
    let nba = calculate_market_odds(d("0.50"), d("0"), d("0.50"), false, margin);
    assert_eq!(nba.odds_home, d("1.90"));
    assert_eq!(nba.odds_draw, None);
    assert_eq!(nba.odds_away, d("1.90"));

    // Brak marży (0.00): sprawiedliwe kursy 1 / p
    assert_eq!(prob_to_odds(d("0.50"), d("0.00")), d("2.00"));

    // Skrajne prawdopodobieństwa oraz ujemna marża są bezpiecznie przycinane do [1.01, 100.00]
    assert_eq!(prob_to_odds(d("0.0"), margin), d("100.00"));
    assert_eq!(prob_to_odds(d("0.0001"), margin), d("100.00"));
    assert_eq!(prob_to_odds(d("1.0"), margin), d("1.01"));
    assert_eq!(prob_to_odds(d("0.50"), d("-0.10")), d("2.00"));
}
