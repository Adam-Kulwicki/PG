// src/domain/odds.rs
//! Wyliczanie kursów bukmacherskich (`MARKET_AVG`) na podstawie prawdopodobieństw
//! przesłanych przez silniki predykcyjne oraz konfigurowalnej marży bukmachera.

use rust_decimal::Decimal;

/// Minimalny kurs zapisywany w tabeli `match_odds` (spełnia `CHECK (odds > 1.00)`).
pub fn min_odds() -> Decimal {
    Decimal::new(101, 2) // 1.01
}

/// Maksymalny kurs zapisywany w `match_odds` (bezpiecznie mieści się w `DECIMAL(6,2)`).
pub fn max_odds() -> Decimal {
    Decimal::new(10000, 2) // 100.00
}

/// Domyślna marża bukmacherska (5%).
pub fn default_bookmaker_margin() -> Decimal {
    Decimal::new(5, 2) // 0.05
}

/// Wyliczone kursy gotowe do zapisu w `match_odds` jako `MARKET_AVG`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalculatedOdds {
    pub odds_home: Decimal,
    /// `None` dla dyscyplin bez remisu (koszykówka, tenis, CS2).
    pub odds_draw: Option<Decimal>,
    pub odds_away: Decimal,
}

/// Przelicza pojedyncze prawdopodobieństwo `prob` na kurs z marżą `margin`:
/// `O = 1 / (prob * (1 + margin))`, zaokrąglone do 2 miejsc i przycięte do `[1.01, 100.00]`.
pub fn prob_to_odds(prob: Decimal, margin: Decimal) -> Decimal {
    if prob <= Decimal::ZERO {
        return max_odds();
    }
    let safe_margin = margin.max(Decimal::ZERO);
    let raw = Decimal::ONE / (prob * (Decimal::ONE + safe_margin));
    raw.round_dp(2).clamp(min_odds(), max_odds())
}

/// Wylicza komplet kursów `1X2` (dla piłki nożnej) lub `12` (dla koszykówki, tenisa i CS2).
pub fn calculate_market_odds(
    prob_home: Decimal,
    prob_draw: Decimal,
    prob_away: Decimal,
    allow_draw: bool,
    margin: Decimal,
) -> CalculatedOdds {
    CalculatedOdds {
        odds_home: prob_to_odds(prob_home, margin),
        odds_draw: allow_draw.then(|| prob_to_odds(prob_draw, margin)),
        odds_away: prob_to_odds(prob_away, margin),
    }
}
