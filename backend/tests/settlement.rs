// tests/settlement.rs
use backend::domain::settlement::{payout, resolve_outcome, BetOutcome};
use rust_decimal::Decimal;

/// Skrót do tworzenia `Decimal` z napisu.
fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// Wygrana/przegrana zależy od zwycięzcy, a mecz bez rozstrzygnięcia daje VOID.
#[test]
fn resolves_outcomes() {
    assert_eq!(
        resolve_outcome("HOME", "FINISHED", Some("HOME")),
        BetOutcome::Won
    );
    assert_eq!(
        resolve_outcome("DRAW", "FINISHED", Some("AWAY")),
        BetOutcome::Lost
    );
    assert_eq!(resolve_outcome("HOME", "CANCELLED", None), BetOutcome::Void);
    assert_eq!(resolve_outcome("HOME", "FINISHED", None), BetOutcome::Void);
}

/// Wypłata: WON = stawka * kurs, LOST = 0, VOID = zwrot stawki.
#[test]
fn computes_payouts() {
    // stawka 50 @ 2.65 -> wypłata 132.50, zysk netto 82.50
    assert_eq!(
        payout(BetOutcome::Won, d("50"), d("2.65")),
        (d("82.50"), d("132.50"))
    );
    assert_eq!(
        payout(BetOutcome::Lost, d("50"), d("2.65")),
        (d("-50"), Decimal::ZERO)
    );
    assert_eq!(
        payout(BetOutcome::Void, d("50"), d("2.65")),
        (Decimal::ZERO, d("50"))
    );
}
