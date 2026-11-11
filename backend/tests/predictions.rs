// tests/predictions.rs
use backend::api::predictions::validate_probabilities;
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn accepts_valid_probabilities() {
    assert!(validate_probabilities(d("0.34"), d("0.28"), d("0.38")).is_ok());
    assert!(validate_probabilities(d("0.68"), d("0"), d("0.32")).is_ok());
}

#[test]
fn rejects_invalid_probabilities() {
    assert!(validate_probabilities(d("0.5"), d("0.5"), d("0.5")).is_err());
    assert!(validate_probabilities(d("1.2"), d("0"), d("-0.2")).is_err());
}
