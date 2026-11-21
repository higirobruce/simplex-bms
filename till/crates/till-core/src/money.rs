//! Exact money and quantity types.

use std::fmt;
use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};
use thiserror::Error;

/// Decimal places carried by [`Money`] (`Decimal(12,2)` in the cloud schema).
const MONEY_DP: u32 = 2;
/// Decimal places carried by [`Qty`] (`Decimal(12,3)` in the cloud schema).
const QTY_DP: u32 = 3;

/// Errors raised when parsing money or quantity text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoneyError {
    #[error("not a decimal number: {0:?}")]
    Unparsable(String),
    #[error("{got} decimal places given, at most {max} allowed")]
    TooManyPlaces { got: u32, max: u32 },
}

/// Rounds half away from zero, matching the Rwanda Revenue Authority's own
/// sample payloads (`.966` becomes `.97`, `.2711` becomes `.27`). This is
/// deliberately not the `rust_decimal` default (banker's rounding).
pub fn round_half_up(d: Decimal, dp: u32) -> Decimal {
    d.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
}

/// Parses text, rejecting more than `max` significant decimal places.
/// Trailing zeros are not significant, so `"1.500"` is valid money.
fn parse_exact(s: &str, max: u32) -> Result<Decimal, MoneyError> {
    let d = Decimal::from_str(s.trim()).map_err(|_| MoneyError::Unparsable(s.to_string()))?;
    let got = d.normalize().scale();
    if got > max {
        return Err(MoneyError::TooManyPlaces { got, max });
    }
    Ok(d)
}

/// An amount of RWF, exact to two decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money(Decimal);

impl Money {
    pub fn zero() -> Money {
        Money(Decimal::ZERO)
    }

    pub fn try_from_str(s: &str) -> Result<Money, MoneyError> {
        parse_exact(s, MONEY_DP).map(Money)
    }

    /// Builds money from hundredths, so `from_minor(150)` is `1.50`.
    pub fn from_minor(minor: i64) -> Money {
        Money(Decimal::new(minor, MONEY_DP))
    }

    pub fn checked_add(self, other: Money) -> Option<Money> {
        self.0.checked_add(other.0).map(Money)
    }

    pub fn checked_sub(self, other: Money) -> Option<Money> {
        self.0.checked_sub(other.0).map(Money)
    }

    /// Unit price times quantity, rounded half-up to two places.
    pub fn times_qty(self, qty: Qty) -> Money {
        Money(round_half_up(self.0 * qty.0, MONEY_DP))
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

/// A quantity (units or kilograms), exact to three decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Qty(Decimal);

impl Qty {
    pub fn try_from_str(s: &str) -> Result<Qty, MoneyError> {
        parse_exact(s, QTY_DP).map(Qty)
    }

    pub fn is_positive(self) -> bool {
        self.0.is_sign_positive() && !self.0.is_zero()
    }
}

impl fmt::Display for Qty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn money_keeps_two_places() {
        assert_eq!(Money::try_from_str("1200").unwrap().to_string(), "1200.00");
        assert_eq!(
            Money::try_from_str("1200.5").unwrap().to_string(),
            "1200.50"
        );
    }
    #[test]
    fn money_rejects_a_third_place() {
        assert!(matches!(
            Money::try_from_str("1.005"),
            Err(MoneyError::TooManyPlaces { got: 3, max: 2 })
        ));
    }
    #[test]
    fn qty_keeps_three_places() {
        assert_eq!(Qty::try_from_str("0.734").unwrap().to_string(), "0.734");
    }
    #[test]
    fn qty_times_price_rounds_half_up_to_money() {
        // 0.734 kg x 1850 RWF/kg = 1357.900
        let got = Money::try_from_str("1850")
            .unwrap()
            .times_qty(Qty::try_from_str("0.734").unwrap());
        assert_eq!(got.to_string(), "1357.90");
        // 1 x 0.005 = 0.005, an exact midpoint: half-up gives 0.01, banker's would give 0.00
        let mid = Money::try_from_str("1")
            .unwrap()
            .times_qty(Qty::try_from_str("0.005").unwrap());
        assert_eq!(mid.to_string(), "0.01");
    }
    #[test]
    fn round_half_up_goes_away_from_zero_at_the_midpoint() {
        // from_str, never from f64: 2.345 as an f64 is 2.34499999... and would round down
        assert_eq!(
            round_half_up(Decimal::from_str("2.345").unwrap(), 2).to_string(),
            "2.35"
        );
        assert_eq!(
            round_half_up(Decimal::from_str("-2.345").unwrap(), 2).to_string(),
            "-2.35"
        );
    }
}
