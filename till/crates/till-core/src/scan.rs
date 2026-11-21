//! Classifies what a cashier scanned or keyed: a barcode, a PLU code, or a
//! scale-printed label with a weight or price embedded in it.
//!
//! A scanner is an untrusted input device: it can emit partial reads, control
//! characters and arbitrary lengths. Every malformed input therefore becomes a
//! named [`ScanError`]; nothing here panics.
//!
//! # Why `classify` takes an [`EmbeddedMode`]
//!
//! Digits 8-12 of an in-store barcode carry *either* a weight in grams *or* a
//! price in francs, and nothing in the barcode says which. It depends on how
//! the shop's scales are configured, so the caller supplies it and `classify`
//! reports what it decoded under that mode. It never guesses.
//!
//! `classify` only *reports*. When a label carries a price, that price beats
//! the catalogue price (the scale already weighed the goods in the customer's
//! hand), but that precedence is applied by whatever builds the basket line.

use thiserror::Error;

use crate::money::{Money, MoneyError, Qty};

/// What a scan turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scanned {
    /// A plain EAN-8 or EAN-13 with a valid check digit.
    Barcode(String),
    /// A 4 or 5 digit price-look-up code keyed or scanned. Digits are kept
    /// verbatim, leading zeros included.
    Plu(String),
    /// An in-store EAN-13 from a scale. Exactly one of `qty` and `price` is
    /// `Some`, according to the [`EmbeddedMode`] it was decoded under.
    WeightEmbedded {
        item_ref: String,
        qty: Option<Qty>,
        price: Option<Money>,
    },
}

/// Whether this shop's scale labels carry grams or francs in digits 8-12.
/// Not recoverable from the barcode, so the caller passes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedMode {
    Weight,
    Price,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScanError {
    #[error("nothing was scanned")]
    Empty,
    #[error("scan contains characters other than digits")]
    NonNumeric,
    #[error("{0} digits is not a PLU (4-5), EAN-8 or EAN-13 length")]
    BadLength(usize),
    #[error("check digit is {got}, expected {expected}")]
    BadCheckDigit { expected: u8, got: u8 },
    /// The embedded digits could not be built into a quantity or price. Five
    /// digits always fit, so this is unreachable today; it exists so that
    /// fact is not enforced with a panic.
    #[error("embedded value rejected: {0}")]
    Embedded(#[from] MoneyError),
}

/// Empty and non-digit input are refused before anything looks at length, so
/// every later byte index is over ASCII digits only.
fn digits_of(input: &str) -> Result<&[u8], ScanError> {
    if input.is_empty() {
        return Err(ScanError::Empty);
    }
    let bytes = input.as_bytes();
    if !bytes.iter().all(u8::is_ascii_digit) {
        return Err(ScanError::NonNumeric);
    }
    Ok(bytes)
}

/// GS1 check digit of a payload of digits: the rightmost payload digit
/// weighs 3, then 1, 3, 1... leftwards. For 12 digits this is the familiar
/// "odd positions x1, even positions x3"; for EAN-8's 7 digits it is x3, x1.
fn check_digit_of(payload: &[u8]) -> u8 {
    let sum: u32 = payload
        .iter()
        .rev()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    ((10 - sum % 10) % 10) as u8
}

/// Check digit for the first 12 digits of an EAN-13.
pub fn ean13_check_digit(first12: &str) -> Result<u8, ScanError> {
    let bytes = digits_of(first12)?;
    if bytes.len() != 12 {
        return Err(ScanError::BadLength(bytes.len()));
    }
    Ok(check_digit_of(bytes))
}

/// True only for exactly 13 digits whose last digit is the right check digit.
pub fn ean13_is_valid(code: &str) -> bool {
    verify(code.as_bytes(), 13).is_ok()
}

/// Checks the last digit of `bytes` (all ASCII digits, `len` long) against
/// the rest.
fn verify(bytes: &[u8], len: usize) -> Result<(), ScanError> {
    if bytes.len() != len || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(ScanError::NonNumeric);
    }
    let Some((last, payload)) = bytes.split_last() else {
        return Err(ScanError::Empty);
    };
    let expected = check_digit_of(payload);
    let got = last - b'0';
    if expected == got {
        Ok(())
    } else {
        Err(ScanError::BadCheckDigit { expected, got })
    }
}

/// Canonical PLU form: leading zeros dropped, at least one digit kept. No
/// standard PLU begins with `0`, so this is lossless. Never apply to barcodes.
fn strip_leading_zeros(digits: &str) -> &str {
    match digits.trim_start_matches('0') {
        "" => "0",
        rest => rest,
    }
}

/// An in-store (variable measure) EAN-13 starts `02` or `20`-`29`.
fn is_in_store(bytes: &[u8]) -> bool {
    matches!(bytes, [b'2', ..] | [b'0', b'2', ..])
}

/// Classifies a scan. Length decides first: 4-5 digits is a PLU, 8 and 13 are
/// EANs, anything else is `BadLength`. For 13 digits the in-store prefix is
/// recognised after the check digit has been verified.
///
/// Surrounding ASCII whitespace (the Enter or CR/LF a scanner appends) is
/// trimmed first. Only the ends are trimmed, so `"400 6381333931"` is still
/// `NonNumeric` and no invalid code can become valid.
pub fn classify(input: &str, embedded: EmbeddedMode) -> Result<Scanned, ScanError> {
    let input = input.trim_matches(|c: char| c.is_ascii_whitespace());
    let bytes = digits_of(input)?;
    match bytes.len() {
        // Only PLUs and item refs are normalised (leading zeros stripped);
        // barcodes never are, they are matched byte-for-byte.
        4 | 5 => Ok(Scanned::Plu(strip_leading_zeros(input).to_string())),
        8 => {
            verify(bytes, 8)?;
            Ok(Scanned::Barcode(input.to_string()))
        }
        13 => {
            verify(bytes, 13)?;
            if is_in_store(bytes) {
                decode_embedded(input, embedded)
            } else {
                Ok(Scanned::Barcode(input.to_string()))
            }
        }
        n => Err(ScanError::BadLength(n)),
    }
}

/// `code` is 13 verified ASCII digits: item reference is digits 3-7, the
/// embedded value digits 8-12.
///
/// The layout is shop-dependent: some scales put a price check digit in digit
/// 8 and carry only 4 price digits. This is the one place to change if the
/// pilot shop's scales do so; [`EmbeddedMode`] is the extension point.
///
/// The item reference is returned without leading zeros (at least one digit
/// kept), the same canonical form a stored PLU has: no standard PLU begins
/// with `0`, so `"04011"` is PLU `"4011"` and the stripping is lossless.
fn decode_embedded(code: &str, embedded: EmbeddedMode) -> Result<Scanned, ScanError> {
    let item_ref = code.get(2..7).ok_or(ScanError::BadLength(code.len()))?;
    let item_ref = strip_leading_zeros(item_ref);
    let value = code.get(7..12).ok_or(ScanError::BadLength(code.len()))?;
    let (qty, price) = match embedded {
        EmbeddedMode::Weight => {
            // Grams to kilograms: 00734 is 0.734.
            let (kg, g) = value.split_at(2);
            (Some(Qty::try_from_str(&format!("{kg}.{g}"))?), None)
        }
        EmbeddedMode::Price => (None, Some(Money::try_from_str(value)?)),
    };
    Ok(Scanned::WeightEmbedded {
        item_ref: item_ref.to_string(),
        qty,
        price,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use EmbeddedMode::{Price, Weight};

    /// Sums from the opposite end to `check_digit_of`: here the weight comes
    /// from the digit's 1-based position counted from the left (odd x1, even
    /// x3), whereas the implementation counts from the right. Deliberately
    /// not a call into the implementation, so a shared mistake cannot hide.
    fn with_check_digit(first12: &str) -> String {
        let mut odd = 0;
        let mut even = 0;
        for (i, c) in first12.chars().enumerate() {
            let d = c.to_digit(10).unwrap();
            if (i + 1) % 2 == 1 {
                odd += d;
            } else {
                even += d;
            }
        }
        let check = (10 - (odd + 3 * even) % 10) % 10;
        format!("{first12}{check}")
    }

    #[test]
    fn a_plain_ean13_is_recognised_and_checked() {
        assert!(ean13_is_valid("4006381333931"));
        assert!(
            matches!(classify("4006381333931", Weight).unwrap(), Scanned::Barcode(b) if b == "4006381333931")
        );
    }
    #[test]
    fn a_bad_check_digit_is_refused_not_rounded_away() {
        match classify("4006381333932", Weight) {
            Err(ScanError::BadCheckDigit {
                expected: 1,
                got: 2,
            }) => {}
            other => panic!("expected BadCheckDigit, got {other:?}"),
        }
    }
    #[test]
    fn a_four_or_five_digit_code_is_a_plu() {
        assert!(matches!(classify("4011", Weight).unwrap(), Scanned::Plu(p) if p == "4011"));
        assert!(matches!(classify("94011", Weight).unwrap(), Scanned::Plu(p) if p == "94011"));
    }
    #[test]
    fn an_in_store_prefix_decodes_as_weight_embedded() {
        let code = with_check_digit("212345600734");
        assert_eq!(code, "2123456007343");
        match classify(&code, Weight).unwrap() {
            Scanned::WeightEmbedded {
                item_ref,
                qty,
                price,
            } => {
                assert_eq!(item_ref, "23456");
                assert_eq!(qty.unwrap().to_string(), "0.734");
                assert_eq!(price, None);
            }
            other => panic!("expected WeightEmbedded, got {other:?}"),
        }
    }
    #[test]
    fn the_same_barcode_decodes_differently_under_the_two_modes() {
        let code = with_check_digit("021234601500");
        assert_eq!(code, "0212346015000");
        let w = classify(&code, Weight).unwrap();
        let p = classify(&code, Price).unwrap();
        match (w, p) {
            (
                Scanned::WeightEmbedded {
                    qty: Some(q),
                    price: None,
                    item_ref: ref a,
                },
                Scanned::WeightEmbedded {
                    qty: None,
                    price: Some(m),
                    item_ref: ref b,
                },
            ) => {
                assert_eq!(a, b);
                assert_eq!(a, "12346");
                assert_eq!(q.to_string(), "1.500");
                assert_eq!(m.to_string(), "1500.00");
            }
            other => panic!("modes must differ, got {other:?}"),
        }
    }
    #[test]
    fn empty_and_non_numeric_input_are_refused() {
        assert!(matches!(classify("", Weight), Err(ScanError::Empty)));
        assert!(matches!(
            classify("40063813339AB", Weight),
            Err(ScanError::NonNumeric)
        ));
    }
    #[test]
    fn an_eight_digit_ean_is_a_barcode_not_a_plu() {
        assert!(matches!(
            classify("96385074", Weight).unwrap(),
            Scanned::Barcode(_)
        ));
    }
    #[test]
    fn a_length_we_do_not_handle_is_refused() {
        assert!(matches!(
            classify("123", Weight),
            Err(ScanError::BadLength(3))
        ));
        assert!(matches!(
            classify("1234567890", Weight),
            Err(ScanError::BadLength(10))
        ));
    }

    // Additions beyond the brief.
    #[test]
    fn a_bad_eight_digit_check_digit_is_refused() {
        assert!(matches!(
            classify("96385075", Weight),
            Err(ScanError::BadCheckDigit {
                expected: 4,
                got: 5
            })
        ));
    }
    #[test]
    fn an_in_store_code_with_a_bad_check_digit_is_refused() {
        assert!(matches!(
            classify("2123456007344", Price),
            Err(ScanError::BadCheckDigit {
                expected: 3,
                got: 4
            })
        ));
    }
    #[test]
    fn a_twelve_digit_code_is_refused_by_length() {
        assert!(matches!(
            classify("212345600734", Weight),
            Err(ScanError::BadLength(12))
        ));
    }
    #[test]
    fn prefix_boundaries_of_the_in_store_range() {
        for (first12, in_store) in [
            ("019999999999", false),
            ("029999999999", true),
            ("030000000000", false),
            ("199999999999", false),
            ("200000000000", true),
            ("299999999999", true),
            ("300000000000", false),
        ] {
            let code = with_check_digit(first12);
            let got = matches!(
                classify(&code, Weight).unwrap(),
                Scanned::WeightEmbedded { .. }
            );
            assert_eq!(got, in_store, "{code}");
        }
    }
    #[test]
    fn keyed_plu_drops_leading_zeros_and_all_zero_becomes_single_zero() {
        assert!(matches!(classify("0000", Weight).unwrap(), Scanned::Plu(p) if p == "0"));
        assert!(matches!(classify("01234", Weight).unwrap(), Scanned::Plu(p) if p == "1234"));
    }
    #[test]
    fn hostile_input_never_panics() {
        for s in [
            "\n",
            "40 11",
            "４０１１",
            "é",
            "٣٣٣٣",
            "\u{0}",
            "-4011",
            "4011.0",
            &"9".repeat(10_000),
        ] {
            assert!(classify(s, Weight).is_err(), "{s:?}");
            assert!(classify(s, Price).is_err(), "{s:?}");
        }
    }
    #[test]
    fn check_digit_helpers_refuse_bad_shapes() {
        assert_eq!(ean13_check_digit("400638133393"), Ok(1));
        assert_eq!(ean13_check_digit(""), Err(ScanError::Empty));
        assert_eq!(
            ean13_check_digit("4006381333"),
            Err(ScanError::BadLength(10))
        );
        assert_eq!(
            ean13_check_digit("40063813339x"),
            Err(ScanError::NonNumeric)
        );
        assert!(!ean13_is_valid("400638133393"));
        assert!(!ean13_is_valid("40063813339AB"));
        assert!(!ean13_is_valid(""));
    }
    #[test]
    fn leading_zero_barcode_is_returned_verbatim() {
        assert!(ean13_is_valid("0012345678905"));
        assert!(matches!(
            classify("0012345678905", Weight).unwrap(),
            Scanned::Barcode(b) if b == "0012345678905"
        ));
    }

    #[test]
    fn item_ref_drops_leading_zeros_but_keeps_a_five_digit_plu() {
        let code = with_check_digit("020401100734");
        assert!(matches!(
            classify(&code, Weight).unwrap(),
            Scanned::WeightEmbedded { item_ref, .. } if item_ref == "4011"
        ));
        let code = with_check_digit("209401100734");
        assert!(matches!(
            classify(&code, Weight).unwrap(),
            Scanned::WeightEmbedded { item_ref, .. } if item_ref == "94011"
        ));
        let code = with_check_digit("200000000734");
        assert!(matches!(
            classify(&code, Weight).unwrap(),
            Scanned::WeightEmbedded { item_ref, .. } if item_ref == "0"
        ));
    }
    #[test]
    fn surrounding_whitespace_is_trimmed_but_inner_is_refused() {
        assert!(matches!(classify("4011\r\n", Weight).unwrap(), Scanned::Plu(p) if p == "4011"));
        assert!(
            matches!(classify(" \t4006381333931\n", Weight).unwrap(), Scanned::Barcode(b) if b == "4006381333931")
        );
        assert!(matches!(
            classify("400 6381333931", Weight),
            Err(ScanError::NonNumeric)
        ));
        assert!(matches!(classify("\r\n", Weight), Err(ScanError::Empty)));
    }
    #[test]
    fn zero_embedded_values_decode_as_zero() {
        let code = with_check_digit("210000000000");
        assert!(matches!(
            classify(&code, Weight).unwrap(),
            Scanned::WeightEmbedded { qty: Some(q), .. } if q.to_string() == "0.000"
        ));
    }
}
