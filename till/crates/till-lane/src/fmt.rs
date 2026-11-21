//! Display formatting for money amounts and quantities on the lane.
//!
//! RWF (prices) have no circulating subunit and are always whole francs, so they
//! are shown grouped with no decimals (`7,450`). Tax keeps two decimal places
//! because the Rwanda Revenue Authority requires it. Quantity keeps three places
//! because weighed goods are sold by the kilogram.
//!
//! All three functions derive their output by parsing the [`Display`](std::fmt::Display)
//! impl of their inputs — that impl is the only public route to read the inner
//! decimal values, since `Money` and `Qty` expose no accessors.

use rust_decimal::Decimal;
use till_core::{Money, Qty, round_half_up};

/// Formats a price in RWF with thousands grouping and no decimal places.
///
/// The Rwanda Revenue Authority recognizes only whole francs as prices. Callers
/// pass whole francs by construction (via [`till_core::line::whole_francs`]), so
/// this function rounds using `round_half_up` as a safety net to ensure the
/// displayed amount matches what was recorded in the transaction ledger. Without
/// this matching, a figure could appear on screen a franc below the amount
/// actually charged.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(rwf(Money::try_from_str("7450.00").unwrap()), "7,450");
/// assert_eq!(rwf(Money::try_from_str("1234567.00").unwrap()), "1,234,567");
/// assert_eq!(rwf(Money::try_from_str("1234.49").unwrap()), "1,234");
/// assert_eq!(rwf(Money::try_from_str("1234.50").unwrap()), "1,235");
/// ```
pub fn rwf(m: Money) -> String {
    use std::str::FromStr;

    // Money formats as "{:.2}", e.g. "1234.56"
    let s = m.to_string();
    // Parse back to Decimal so we can round to whole francs
    let decimal = Decimal::from_str(&s).expect("Money's Display is always a valid decimal");
    // Round to 0 decimal places using half-up (away from zero)
    let rounded = round_half_up(decimal, 0);
    // Convert to string and ensure we never produce "-0"
    let rounded_str = rounded.to_string();
    let final_str = if rounded_str == "-0" {
        "0".to_string()
    } else {
        rounded_str
    };
    // Add thousands grouping
    add_thousands_separators(&final_str)
}

/// Formats tax (a money amount) with two decimal places.
///
/// Tax is the one money figure that keeps decimal places, because the Rwanda
/// Revenue Authority requires it in invoices and reports.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(tax(Money::try_from_str("983.89").unwrap()), "983.89");
/// assert_eq!(tax(Money::try_from_str("0.00").unwrap()), "0.00");
/// ```
pub fn tax(m: Money) -> String {
    // Money already formats as "{:.2}", e.g. "983.89"
    m.to_string()
}

/// Formats a quantity with three decimal places for weighed goods.
///
/// Quantity keeps three decimal places because goods in Rwanda are sold by the
/// kilogram, and accuracy to the gram is standard practice.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(qty(Qty::try_from_str("1.250").unwrap()), "1.250");
/// assert_eq!(qty(Qty::try_from_str("2.000").unwrap()), "2.000");
/// ```
pub fn qty(q: Qty) -> String {
    // Qty already formats as "{:.3}", e.g. "1.250"
    q.to_string()
}

/// Formats a quantity for the cashier's screen: a whole quantity as `2`,
/// anything else as `qty` does (`1.250`). Screen only. The receipt keeps
/// [`qty`]; what RRA expects there is an open question, not decided here.
pub fn qty_display(q: Qty) -> String {
    let full = qty(q);
    match full.strip_suffix(".000") {
        Some(whole) => whole.to_string(),
        None => full,
    }
}

/// Adds thousands separators (commas) to a numeric string from the right.
///
/// Takes a string of digits and inserts commas every three digits, starting
/// from the right. The result preserves the sign if present.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(add_thousands_separators("7450"), "7,450");
/// assert_eq!(add_thousands_separators("1234567"), "1,234,567");
/// assert_eq!(add_thousands_separators("983"), "983");
/// assert_eq!(add_thousands_separators("0"), "0");
/// ```
fn add_thousands_separators(s: &str) -> String {
    let (sign, num) = if let Some(stripped) = s.strip_prefix('-') {
        ("-", stripped)
    } else {
        ("", s)
    };

    let mut result = String::new();
    let chars: Vec<char> = num.chars().collect();
    let len = chars.len();

    for (i, c) in chars.iter().enumerate() {
        result.push(*c);
        // Insert a comma every three digits from the right, but not after the last digit
        let remaining = len - i - 1;
        if remaining > 0 && remaining % 3 == 0 {
            result.push(',');
        }
    }

    if !sign.is_empty() {
        format!("{}{}", sign, result)
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    fn q(s: &str) -> Qty {
        Qty::try_from_str(s).unwrap()
    }

    #[test]
    fn rwf_groups_thousands_and_rounds_to_whole_francs() {
        // Exact whole francs
        assert_eq!(rwf(m("7450.00")), "7,450");
        assert_eq!(rwf(m("983.00")), "983");
        assert_eq!(rwf(m("0.00")), "0");
        assert_eq!(rwf(m("1234567.00")), "1,234,567");
        // Rounding cases: round half-up (away from zero)
        assert_eq!(rwf(m("1234.49")), "1,234");
        assert_eq!(rwf(m("1234.50")), "1,235");
        assert_eq!(rwf(m("1234.99")), "1,235");
        assert_eq!(rwf(m("0.49")), "0");
        assert_eq!(rwf(m("0.50")), "1");
        assert_eq!(rwf(m("0.99")), "1");
    }

    #[test]
    fn rwf_handles_negative_amounts_without_producing_minus_zero() {
        // Negative amounts that don't round to zero
        assert_eq!(rwf(m("-1500.00")), "-1,500");
        assert_eq!(rwf(m("-1234.50")), "-1,235");
        assert_eq!(rwf(m("-1234.99")), "-1,235");
        // Negative sub-franc amounts that round to zero produce "0", not "-0"
        assert_eq!(rwf(m("-0.49")), "0");
        assert_eq!(rwf(m("-0.50")), "-1");
    }

    #[test]
    fn thousands_separator_preserves_sign() {
        // Positive and negative values both preserve their sign correctly
        assert_eq!(add_thousands_separators("1234567"), "1,234,567");
        assert_eq!(add_thousands_separators("-1234567"), "-1,234,567");
        assert_eq!(add_thousands_separators("1500"), "1,500");
        assert_eq!(add_thousands_separators("-1500"), "-1,500");
    }

    #[test]
    fn tax_keeps_two_places_because_rra_does() {
        assert_eq!(tax(m("983.89")), "983.89");
        assert_eq!(tax(m("0.00")), "0.00");
    }

    #[test]
    fn qty_display_drops_the_decimals_of_a_whole_quantity_only() {
        assert_eq!(qty_display(q("2")), "2");
        assert_eq!(qty_display(q("1.000")), "1");
        assert_eq!(qty_display(q("10")), "10");
        assert_eq!(qty_display(q("1.250")), "1.250");
        assert_eq!(qty_display(q("0.500")), "0.500");
        assert_eq!(qty_display(q("0.001")), "0.001");
    }

    #[test]
    fn qty_shows_three_places_for_weighed_goods() {
        assert_eq!(qty(q("1.250")), "1.250");
        assert_eq!(qty(q("2.000")), "2.000");
    }
}
