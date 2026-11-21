//! Exact money and quantity types.

use std::fmt;
use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};
use thiserror::Error;

/// Decimal places carried by [`Money`] (`Decimal(12,2)` in the cloud schema).
const MONEY_DP: u32 = 2;
/// Decimal places carried by [`Qty`] (`Decimal(12,3)` in the cloud schema).
const QTY_DP: u32 = 3;

/// Total digits allowed in money and quantity (`Decimal(12,s)`: `p` counts all digits).
const MAX_DIGITS: u32 = 12;

/// Errors raised when parsing money or quantity text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoneyError {
    #[error("not a decimal number: {0:?}")]
    Unparsable(String),
    #[error("{got} decimal places given, at most {max} allowed")]
    TooManyPlaces { got: u32, max: u32 },
    #[error("{got} digits given, at most {max} allowed")]
    TooManyDigits { got: u32, max: u32 },
}

/// Rounds half away from zero, matching the Rwanda Revenue Authority's own
/// sample payloads (`.966` becomes `.97`, `.2711` becomes `.27`). This is
/// deliberately not the `rust_decimal` default (banker's rounding).
pub fn round_half_up(d: Decimal, dp: u32) -> Decimal {
    d.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
}

/// Rejects a value with more than [`MAX_DIGITS`] digits in total: integer
/// digits plus `max_dp` places, which is what `Decimal(12,max_dp)` means.
/// Every construction path of [`Money`] and [`Qty`] goes through this.
fn check_digits(d: Decimal, max_dp: u32) -> Result<(), MoneyError> {
    let n = d.normalize();
    let mantissa_digits = n.mantissa().unsigned_abs().to_string().len() as u32;
    let integer_digits = mantissa_digits.saturating_sub(n.scale());
    let digits = integer_digits + max_dp;
    if digits > MAX_DIGITS {
        return Err(MoneyError::TooManyDigits {
            got: digits,
            max: MAX_DIGITS,
        });
    }
    Ok(())
}

/// Parses plain decimal text, rejecting more than `max` decimal places, then
/// more than [`MAX_DIGITS`] digits in total. Scientific notation and a leading
/// `+` are rejected; trailing and leading zeros are harmless, so `"1.500"` and
/// `"0005"` are valid.
fn parse_exact(s: &str, max: u32) -> Result<Decimal, MoneyError> {
    let t = s.trim();
    if t.starts_with('+') || t.contains(['e', 'E']) {
        return Err(MoneyError::Unparsable(s.to_string()));
    }
    let d = Decimal::from_str(t).map_err(|_| MoneyError::Unparsable(s.to_string()))?;
    let got = d.normalize().scale();
    if got > max {
        return Err(MoneyError::TooManyPlaces { got, max });
    }
    check_digits(d, max)?;
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

    /// Builds money from hundredths, so `try_from_minor(150)` is `1.50`.
    pub fn try_from_minor(minor: i64) -> Result<Money, MoneyError> {
        let d = Decimal::new(minor, MONEY_DP);
        check_digits(d, MONEY_DP)?;
        Ok(Money(d))
    }

    /// The inner decimal, read-only. Crate-private so a caller can compute with
    /// the value but can only build a new [`Money`] through a checked path,
    /// which keeps the 12-digit invariant intact.
    pub(crate) fn amount(self) -> Decimal {
        self.0
    }

    /// `None` if the sum overflows or exceeds 12 digits.
    pub fn checked_add(self, other: Money) -> Option<Money> {
        Self::bounded(self.0.checked_add(other.0)?)
    }

    /// `None` if the difference overflows or exceeds 12 digits.
    pub fn checked_sub(self, other: Money) -> Option<Money> {
        Self::bounded(self.0.checked_sub(other.0)?)
    }

    /// Wraps an already-computed decimal. `None` if it has more than two
    /// places or exceeds 12 digits. Built on [`Money::bounded`] so the bound
    /// has a single implementation.
    pub(crate) fn from_decimal(d: Decimal) -> Option<Money> {
        if d.normalize().scale() > MONEY_DP {
            return None;
        }
        Self::bounded(d)
    }

    fn bounded(d: Decimal) -> Option<Money> {
        check_digits(d, MONEY_DP).ok().map(|()| Money(d))
    }

    /// Unit price times quantity, rounded half-up to two places. `None` if
    /// the rounded product exceeds 12 digits.
    ///
    /// All four construction paths (`try_from_str`, `try_from_minor`,
    /// `checked_add`/`checked_sub`, and this one) refuse an out-of-range
    /// result, so no `Money` can exist that violates the bound. The multiply
    /// itself is guarded: bounded operands give a product of at most about
    /// 24 significant digits, inside `rust_decimal`'s 28, and `checked_mul`
    /// covers anything else.
    pub fn checked_times_qty(self, qty: Qty) -> Option<Money> {
        Self::bounded(round_half_up(self.0.checked_mul(qty.0)?, MONEY_DP))
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
            .checked_times_qty(Qty::try_from_str("0.734").unwrap())
            .unwrap();
        assert_eq!(got.to_string(), "1357.90");
        // 1 x 0.005 = 0.005, an exact midpoint: half-up gives 0.01, banker's would give 0.00
        let mid = Money::try_from_str("1")
            .unwrap()
            .checked_times_qty(Qty::try_from_str("0.005").unwrap())
            .unwrap();
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
    #[test]
    fn trailing_zeros_are_accepted_and_display_canonically() {
        assert_eq!(Money::try_from_str("1.500").unwrap().to_string(), "1.50");
    }
    #[test]
    fn thirteen_digits_are_rejected() {
        assert!(matches!(
            Money::try_from_str("12345678901.23"),
            Err(MoneyError::TooManyDigits { got: 13, max: 12 })
        ));
        assert!(matches!(
            Money::try_from_str("99999999999999999999999999.99"),
            Err(MoneyError::TooManyDigits { max: 12, .. })
        ));
        assert!(matches!(
            Qty::try_from_str("1234567890"),
            Err(MoneyError::TooManyDigits { got: 13, max: 12 })
        ));
        assert!(Money::try_from_str("9999999999.99").is_ok());
        assert!(Qty::try_from_str("999999999.999").is_ok());
    }
    #[test]
    fn unparsable_text_is_rejected() {
        assert!(matches!(
            Money::try_from_str("abc"),
            Err(MoneyError::Unparsable(_))
        ));
    }
    #[test]
    fn checked_add_and_sub() {
        let a = Money::try_from_str("10.25").unwrap();
        let b = Money::try_from_str("0.75").unwrap();
        assert_eq!(a.checked_add(b).unwrap().to_string(), "11.00");
        assert_eq!(a.checked_sub(b).unwrap().to_string(), "9.50");
        assert_eq!(b.checked_sub(a).unwrap().to_string(), "-9.50");
    }
    #[test]
    fn from_minor_and_is_positive_and_negative_display() {
        assert_eq!(Money::try_from_minor(150).unwrap().to_string(), "1.50");
        assert_eq!(
            Money::try_from_minor(-120050).unwrap().to_string(),
            "-1200.50"
        );
        assert_eq!(
            Money::try_from_str("-1200.5").unwrap().to_string(),
            "-1200.50"
        );
        assert!(Qty::try_from_str("0.001").unwrap().is_positive());
        assert!(!Qty::try_from_str("0").unwrap().is_positive());
        assert!(!Qty::try_from_str("-1").unwrap().is_positive());
    }
    #[test]
    fn try_from_minor_is_bounded() {
        assert_eq!(
            Money::try_from_minor(120050).unwrap().to_string(),
            "1200.50"
        );
        assert!(matches!(
            Money::try_from_minor(i64::MAX),
            Err(MoneyError::TooManyDigits { max: 12, .. })
        ));
    }
    #[test]
    fn checked_add_and_sub_are_none_beyond_twelve_digits() {
        let max = Money::try_from_str("9999999999.99").unwrap();
        let min = Money::try_from_str("-9999999999.99").unwrap();
        assert!(max.checked_add(max).is_none());
        assert!(min.checked_sub(max).is_none());
        assert_eq!(
            max.checked_add(Money::try_from_str("0.00").unwrap()),
            Some(max)
        );
        // the re-review's repro: doubling can no longer escape the bound
        let mut m = Money::try_from_str("2500000000").unwrap();
        let mut doublings = 0;
        while let Some(next) = m.checked_add(m) {
            m = next;
            doublings += 1;
        }
        assert_eq!(doublings, 1);
    }
    #[test]
    fn scientific_notation_and_plus_are_rejected_but_leading_zeros_are_not() {
        assert!(matches!(
            Money::try_from_str("1e5"),
            Err(MoneyError::Unparsable(_))
        ));
        assert!(matches!(
            Money::try_from_str("+5"),
            Err(MoneyError::Unparsable(_))
        ));
        assert!(matches!(
            Qty::try_from_str("1E2"),
            Err(MoneyError::Unparsable(_))
        ));
        assert_eq!(Money::try_from_str("0005").unwrap().to_string(), "5.00");
    }
    #[test]
    fn repeated_times_qty_terminates_instead_of_panicking() {
        let q = Qty::try_from_str("999999999.999").unwrap();
        let mut m = Money::try_from_str("9999999999.99").unwrap();
        let mut steps = 0;
        while let Some(next) = m.checked_times_qty(q) {
            m = next;
            steps += 1;
            assert!(steps < 10, "bound is not being enforced");
        }
        assert_eq!(steps, 0, "the very first product is out of range");
    }
    #[test]
    fn a_legitimate_line_still_works() {
        let m = Money::try_from_str("1850.00").unwrap();
        let got = m.checked_times_qty(Qty::try_from_str("0.734").unwrap());
        assert_eq!(got.unwrap().to_string(), "1357.90");
    }
    #[test]
    fn times_qty_boundary_is_exact() {
        let one = Qty::try_from_str("1").unwrap();
        let two = Qty::try_from_str("2").unwrap();
        let max = Money::try_from_str("9999999999.99").unwrap();
        assert_eq!(max.checked_times_qty(one), Some(max));
        let half = Money::try_from_str("5000000000").unwrap();
        assert!(half.checked_times_qty(two).is_none());
        let just_in = Money::try_from_str("4999999999.995").unwrap_err();
        assert!(matches!(just_in, MoneyError::TooManyPlaces { .. }));
    }
    #[test]
    fn times_qty_bound_refuses_negative_products_too() {
        let two = Qty::try_from_str("2").unwrap();
        let neg = Money::try_from_str("-9999999999.99").unwrap();
        assert!(neg.checked_times_qty(two).is_none());
        assert_eq!(
            neg.checked_times_qty(Qty::try_from_str("1").unwrap()),
            Some(neg)
        );
    }
}
