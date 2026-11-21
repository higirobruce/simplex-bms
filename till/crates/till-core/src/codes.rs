//! The Rwanda Revenue Authority's code tables (VSDC specification, section 4).
//!
//! Every fiscal payload and receipt carries these codes. Each table is
//! exhaustive over the published list and has no catch-all variant: an
//! unrecognised code in a VSDC response is an error, never a value, so a
//! future code the till does not understand cannot be silently treated as
//! some default.
//!
//! The RRA code string is the single external representation of every table:
//! `wire()`, `FromStr` and `Serialize` all produce and accept it identically,
//! generated from one literal per variant so they cannot drift apart. A
//! payload struct that derives `Serialize` therefore emits `"01"`, not
//! `"Cash"`, and stored values survive a Rust rename of a variant.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

/// An unrecognised code for one of the RRA tables.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown {table} code: {code:?}")]
pub struct CodeError {
    table: &'static str,
    code: String,
}

/// Declares a code table: the enum, `wire`, `ALL` and `FromStr` all come from
/// the one variant list, so they cannot drift apart.
macro_rules! code_table {
    ($(#[$meta:meta])* $name:ident, $table:literal, { $($(#[$vmeta:meta])* $variant:ident => $code:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $code)] $variant),+
        }

        impl $name {
            /// Every variant, in specification order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The code string the VSDC sends and expects.
            pub fn wire(self) -> &'static str {
                match self {
                    $($name::$variant => $code),+
                }
            }
        }

        impl FromStr for $name {
            type Err = CodeError;

            fn from_str(s: &str) -> Result<Self, CodeError> {
                match s {
                    $($code => Ok($name::$variant),)+
                    _ => Err(CodeError { table: $table, code: s.to_owned() }),
                }
            }
        }
    };
}

code_table! {
    /// Tax type (VSDC 4.1). `A` is exempt (`A-EX`), `B` is `B-18.00%`.
    TaxType, "tax type", { A => "A", B => "B", C => "C", D => "D" }
}

impl TaxType {
    /// Rate in percent. `C` and `D` exist in the table but carry no rate in
    /// any sample payload (`saveSales` sends `taxRtC: 0, taxRtD: 0`), so they
    /// are zero here rather than an invented figure.
    pub fn rate(self) -> Decimal {
        match self {
            TaxType::B => Decimal::from(18),
            TaxType::A | TaxType::C | TaxType::D => Decimal::ZERO,
        }
    }
}

code_table! {
    /// Payment method (VSDC 4.10).
    PmtTyCd, "payment method", {
        Cash => "01",
        Credit => "02",
        CashCredit => "03",
        BankCheck => "04",
        Card => "05",
        MobileMoney => "06",
        Other => "07",
    }
}

code_table! {
    /// Transaction type (VSDC 4.8).
    SalesTyCd, "transaction type", {
        Copy => "C",
        Normal => "N",
        Proforma => "P",
        Training => "T",
    }
}

code_table! {
    /// Sales receipt type (VSDC 4.9).
    RcptTyCd, "receipt type", { Sale => "S", Refund => "R" }
}

code_table! {
    /// Transaction progress (VSDC 4.11).
    SalesSttsCd, "transaction progress", {
        WaitForApproval => "01",
        Approved => "02",
        CancelRequested => "03",
        Canceled => "04",
        Refunded => "05",
        Transferred => "06",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exhaustive matches with no wildcard: adding a variant to either enum
    // fails to compile here until the count below is revisited.
    fn pmt_index(v: PmtTyCd) -> usize {
        match v {
            PmtTyCd::Cash => 0,
            PmtTyCd::Credit => 1,
            PmtTyCd::CashCredit => 2,
            PmtTyCd::BankCheck => 3,
            PmtTyCd::Card => 4,
            PmtTyCd::MobileMoney => 5,
            PmtTyCd::Other => 6,
        }
    }

    fn tax_index(v: TaxType) -> usize {
        match v {
            TaxType::A => 0,
            TaxType::B => 1,
            TaxType::C => 2,
            TaxType::D => 3,
        }
    }

    #[test]
    fn payment_method_has_exactly_seven_variants() {
        assert_eq!(PmtTyCd::ALL.len(), 7);
        for (i, v) in PmtTyCd::ALL.iter().enumerate() {
            assert_eq!(pmt_index(*v), i);
        }
    }

    #[test]
    fn tax_type_has_exactly_four_variants() {
        assert_eq!(TaxType::ALL.len(), 4);
        for (i, v) in TaxType::ALL.iter().enumerate() {
            assert_eq!(tax_index(*v), i);
        }
    }

    #[test]
    fn tax_rates_match_vsdc_4_1() {
        assert_eq!(TaxType::A.rate(), Decimal::ZERO);
        assert_eq!(TaxType::B.rate(), Decimal::from(18));
        assert_eq!(TaxType::C.rate(), Decimal::ZERO);
        assert_eq!(TaxType::D.rate(), Decimal::ZERO);
    }

    #[test]
    fn all_seven_payment_codes_round_trip() {
        for (code, v) in [
            ("01", PmtTyCd::Cash),
            ("02", PmtTyCd::Credit),
            ("03", PmtTyCd::CashCredit),
            ("04", PmtTyCd::BankCheck),
            ("05", PmtTyCd::Card),
            ("06", PmtTyCd::MobileMoney),
            ("07", PmtTyCd::Other),
        ] {
            assert_eq!(v.wire(), code);
            assert_eq!(code.parse::<PmtTyCd>().unwrap(), v);
        }
    }

    #[test]
    fn other_tables_round_trip() {
        for (code, v) in [
            ("A", TaxType::A),
            ("B", TaxType::B),
            ("C", TaxType::C),
            ("D", TaxType::D),
        ] {
            assert_eq!((v.wire(), code.parse::<TaxType>().unwrap()), (code, v));
        }
        for (code, v) in [
            ("C", SalesTyCd::Copy),
            ("N", SalesTyCd::Normal),
            ("P", SalesTyCd::Proforma),
            ("T", SalesTyCd::Training),
        ] {
            assert_eq!((v.wire(), code.parse::<SalesTyCd>().unwrap()), (code, v));
        }
        for (code, v) in [("S", RcptTyCd::Sale), ("R", RcptTyCd::Refund)] {
            assert_eq!((v.wire(), code.parse::<RcptTyCd>().unwrap()), (code, v));
        }
        for (code, v) in [
            ("01", SalesSttsCd::WaitForApproval),
            ("02", SalesSttsCd::Approved),
            ("03", SalesSttsCd::CancelRequested),
            ("04", SalesSttsCd::Canceled),
            ("05", SalesSttsCd::Refunded),
            ("06", SalesSttsCd::Transferred),
        ] {
            assert_eq!((v.wire(), code.parse::<SalesSttsCd>().unwrap()), (code, v));
        }
        assert_eq!(SalesSttsCd::ALL.len(), 6);
        assert_eq!(SalesTyCd::ALL.len(), 4);
        assert_eq!(RcptTyCd::ALL.len(), 2);
    }

    #[test]
    fn serialize_emits_the_wire_code_for_every_variant() {
        fn check<T: Serialize + Copy>(all: &[T], wire: fn(T) -> &'static str) {
            for v in all {
                assert_eq!(
                    serde_json::to_string(v).unwrap(),
                    format!("\"{}\"", wire(*v))
                );
            }
        }
        check(TaxType::ALL, TaxType::wire);
        check(PmtTyCd::ALL, PmtTyCd::wire);
        check(SalesTyCd::ALL, SalesTyCd::wire);
        check(RcptTyCd::ALL, RcptTyCd::wire);
        check(SalesSttsCd::ALL, SalesSttsCd::wire);
    }

    #[test]
    fn an_unknown_code_is_an_error_not_a_default() {
        assert!("08".parse::<PmtTyCd>().is_err());
        assert!("E".parse::<TaxType>().is_err());
        assert!("".parse::<RcptTyCd>().is_err());
        assert!("n".parse::<SalesTyCd>().is_err());
        assert!("07".parse::<SalesSttsCd>().is_err());
    }
}
