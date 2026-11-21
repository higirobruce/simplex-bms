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

use till_core::{Money, Qty};

/// Formats a price in RWF with thousands grouping and no decimal places.
///
/// The Rwanda Revenue Authority recognizes only whole francs as prices. This
/// formats the price for the till screen and printed receipt.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(rwf(Money::try_from_str("7450.00").unwrap()), "7,450");
/// assert_eq!(rwf(Money::try_from_str("1234567.00").unwrap()), "1,234,567");
/// ```
pub fn rwf(m: Money) -> String {
    // Money formats as "{:.2}", e.g. "7450.00"
    let s = m.to_string();
    // Split at the decimal and take the integer part
    let integer_part = s.split('.').next().unwrap_or("0");
    // Add thousands grouping
    add_thousands_separators(integer_part)
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
    fn rwf_groups_thousands_and_drops_the_subunit() {
        assert_eq!(rwf(m("7450.00")), "7,450");
        assert_eq!(rwf(m("983.00")), "983");
        assert_eq!(rwf(m("0.00")), "0");
        assert_eq!(rwf(m("1234567.00")), "1,234,567");
    }

    #[test]
    fn tax_keeps_two_places_because_rra_does() {
        assert_eq!(tax(m("983.89")), "983.89");
        assert_eq!(tax(m("0.00")), "0.00");
    }

    #[test]
    fn qty_shows_three_places_for_weighed_goods() {
        assert_eq!(qty(q("1.250")), "1.250");
        assert_eq!(qty(q("2.000")), "2.000");
    }
}
