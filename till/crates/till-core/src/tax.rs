//! VAT extraction from VAT-inclusive prices.

use crate::codes::TaxType;
use rust_decimal::Decimal;

use crate::money::{Money, round_half_up};

/// The VAT contained *within* `taxbl`, rounded half-up to two places.
///
/// Rwandan shelf prices include VAT, so tax is extracted, never added on top:
/// `taxbl x rate / (100 + rate)`, which for band B is `x 18/118`. RRA's own
/// samples confirm it: `1,800,000` gives `274,576.271`, shown as `274576.27`,
/// and `660,000` gives `100,677.966`, shown as `100677.97`.
///
/// Rounding is half-up ([`round_half_up`]), not `rust_decimal`'s default
/// banker's rounding, which differs at exact midpoints.
///
/// Tax keeps two places even though payable amounts are whole francs: it is a
/// derived figure nobody hands over, and RRA's samples carry two places there.
/// This is the single place to change if that ever has to move.
///
/// A zero rate short-circuits to zero, so exempt lines need no division.
pub fn extract_tax(taxbl: Money, band: TaxType) -> Money {
    let rate = band.rate();
    if rate.is_zero() {
        return Money::zero();
    }
    // `Money` holds at most 12 digits, so x rate (at most 100) cannot overflow
    // a Decimal, and the divisor is at least 100.
    let tax = round_half_up(taxbl.amount() * rate / (rate + Decimal::ONE_HUNDRED), 2);
    // Rounded to two places and smaller in magnitude than a gross that already
    // passed the bound, so `None` is unreachable. Zero, not a panic, if that
    // ever stops being true.
    Money::from_decimal(tax).unwrap_or_else(Money::zero)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    #[test]
    fn band_b_extracts_eighteen_over_one_eighteen() {
        let cases = [
            ("200000", "30508.47"),
            ("50000", "7627.12"),
            ("1800000", "274576.27"),
            ("660000", "100677.97"),
        ];
        for (gross, tax) in cases {
            let got = extract_tax(money(gross), TaxType::B);
            assert_eq!(got.to_string(), money(tax).to_string(), "gross {gross}");
        }
    }

    #[test]
    fn exempt_and_unrated_bands_extract_nothing() {
        for band in [TaxType::A, TaxType::C, TaxType::D] {
            assert_eq!(extract_tax(money("250000"), band), Money::zero());
        }
    }

    #[test]
    fn tax_is_never_added_on_top() {
        // the extracted tax is a part of the gross, so it is always smaller
        let gross = money("1180");
        let tax = extract_tax(gross, TaxType::B);
        assert_eq!(tax.to_string(), "180.00");
        assert_eq!(gross.checked_sub(tax).unwrap().to_string(), "1000.00");
    }

    #[test]
    fn rounds_to_the_nearest_cent() {
        // x 9/59 has no exact midpoint at two places for a two-place gross (59 is
        // prime), so half-up and banker's cannot differ for band B; this pins
        // ordinary nearest-cent rounding in both directions.
        assert_eq!(extract_tax(money("0.60"), TaxType::B).to_string(), "0.09");
        assert_eq!(extract_tax(money("0.66"), TaxType::B).to_string(), "0.10");
    }

    #[test]
    fn a_refund_extracts_negative_tax() {
        assert_eq!(
            extract_tax(money("-1180"), TaxType::B).to_string(),
            "-180.00"
        );
    }

    #[test]
    fn the_largest_money_does_not_overflow() {
        let max = money("9999999999.99");
        assert_eq!(extract_tax(max, TaxType::B).to_string(), "1525423728.81");
    }

    #[test]
    fn a_tiny_refund_rounds_to_plain_zero_not_negative_zero() {
        let tax = extract_tax(money("-0.01"), TaxType::B);
        assert_eq!(tax.to_string(), "0.00");
        assert_eq!(tax, Money::zero());
    }
}
