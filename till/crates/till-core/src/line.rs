//! Per-line amounts for the receipt and the RRA `saveSales` payload.

use rust_decimal::Decimal;
use thiserror::Error;

use crate::codes::TaxType;
use crate::money::{Money, Qty, round_half_up};
use crate::tax::extract_tax;

/// Why a line could not be priced. Every failure is named so the till can show
/// the cashier a reason instead of crashing mid-shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LineError {
    #[error("quantity must be greater than zero")]
    NonPositiveQty,
    #[error("discount is larger than the line it applies to")]
    DiscountExceedsSupply,
    /// A discount below zero would raise the price, which is a surcharge, not a
    /// discount. Refused rather than silently priced.
    #[error("discount must not be negative")]
    NegativeDiscount,
    /// An amount beyond `Decimal(12,2)` could not be stored by the cloud's
    /// Postgres column either, so it is refused by name rather than carried
    /// forward as a value nothing downstream can hold.
    #[error("line amount is outside the supported range")]
    AmountOutOfRange,
    /// A text field longer than the VSDC accepts. Caught here, before the
    /// invoice number is consumed, instead of at the VSDC after the customer
    /// has paid.
    #[error(transparent)]
    FieldTooLong(#[from] FieldTooLong),
}

/// `itemClsCd`: CHAR 10 in `TrnsSalesSaveWrItem` (`vsdc.txt` line 1577).
pub const ITEM_CLS_CD_MAX: usize = 10;
/// `itemCd`: CHAR 20 (`vsdc.txt` line 1579).
pub const ITEM_CD_MAX: usize = 20;
/// `itemNm`: CHAR 200 (`vsdc.txt` line 1581).
pub const ITEM_NM_MAX: usize = 200;
/// `bcd`: CHAR 20 (`vsdc.txt` line 1583).
pub const BCD_MAX: usize = 20;
/// `custTin`: CHAR 9 (`vsdc.txt` line 1466).
pub const CUST_TIN_MAX: usize = 9;

/// A text field over the VSDC's declared length, named by its wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{field} is {got} characters; the VSDC accepts at most {max}")]
pub struct FieldTooLong {
    pub field: &'static str,
    pub max: usize,
    pub got: usize,
}

/// Refuses `value` if it is longer than `max`. Length is counted in characters,
/// not bytes: the spec says CHAR and does not say which, and counting bytes
/// would refuse a legitimate accented product name the VSDC might accept.
/// If the VSDC turns out to count bytes, only this function needs to change.
fn check_len(field: &'static str, value: &str, max: usize) -> Result<(), FieldTooLong> {
    let got = value.chars().count();
    if got > max {
        Err(FieldTooLong { field, max, got })
    } else {
        Ok(())
    }
}

/// Validates a buyer TIN (`custTin`) before a sale is begun. Length only: the
/// authority declares CHAR 9 and this does not guess at a digit grammar.
pub fn validate_buyer_tin(tin: &str) -> Result<(), FieldTooLong> {
    check_len("custTin", tin, CUST_TIN_MAX)
}

/// A discount on one line. Mirrors the cloud's `DiscountKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineDiscount {
    /// A percentage of the line's supply amount.
    Percent(Decimal),
    /// A fixed amount off the line.
    Amount(Money),
}

/// One line of a basket, as scanned.
///
/// Field names are the VSDC wire names on purpose: a later phase serialises
/// this straight into a payload, and a rename in between would be a defect
/// waiting to happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasketLine {
    pub item_seq: u16,
    pub item_cd: String,
    pub item_cls_cd: String,
    pub item_nm: String,
    pub bcd: Option<String>,
    /// The cloud `Product.id` this line was rung against, from the catalogue
    /// cache. `None` only for lines written before it was recorded; the cloud
    /// then falls back to resolving `item_cd` against the SKU.
    pub product_id: Option<String>,
    pub qty: Qty,
    pub prc: Money,
    pub tax_ty_cd: TaxType,
    pub discount: Option<LineDiscount>,
}

/// The money figures of one line, on the RRA wire names.
///
/// Everything but `tax_amt` is whole francs: RWF has no circulating subunit, so
/// a line a cashier cannot take in cash would be wrong. `tax_amt` is derived and
/// nobody hands it over, so it keeps two places as RRA's samples do. `dc_rt` is
/// a rate, not money, and keeps two places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineAmounts {
    pub sply_amt: Money,
    pub dc_rt: Decimal,
    pub dc_amt: Money,
    pub taxbl_amt: Money,
    pub tax_amt: Money,
    pub tot_amt: Money,
}

/// Rounds to whole francs and wraps, `None` if the result is out of range.
fn whole_francs(d: Decimal) -> Option<Money> {
    Money::from_decimal(round_half_up(d, 0))
}

impl BasketLine {
    /// Checks every text field against the VSDC's declared length. Called first
    /// by [`BasketLine::amounts`], which every sale prices through before
    /// `begin_sale` opens its transaction, so an over-long name is refused
    /// while no invoice number has been taken.
    pub fn validate_fields(&self) -> Result<(), FieldTooLong> {
        check_len("itemCd", &self.item_cd, ITEM_CD_MAX)?;
        check_len("itemClsCd", &self.item_cls_cd, ITEM_CLS_CD_MAX)?;
        check_len("itemNm", &self.item_nm, ITEM_NM_MAX)?;
        if let Some(bcd) = &self.bcd {
            check_len("bcd", bcd, BCD_MAX)?;
        }
        Ok(())
    }

    /// Prices the line: supply amount, discount, taxable amount and extracted
    /// VAT. The discount applies to `sply_amt`, and the tax is extracted from
    /// what remains after it.
    pub fn amounts(&self) -> Result<LineAmounts, LineError> {
        self.validate_fields()?;
        // Refused before anything is multiplied or divided.
        if !self.qty.is_positive() {
            return Err(LineError::NonPositiveQty);
        }
        // One rounding of the exact product. Going through `checked_times_qty`
        // would round to 2 dp first and then to 0 dp, which is wrong by a franc
        // whenever the first step lands on a midpoint (0.499 x 1001 = 499.499
        // -> 499.50 -> 500, where rounding once gives 499).
        let sply_amt = self
            .prc
            .amount()
            .checked_mul(self.qty.amount())
            .and_then(whole_francs)
            .ok_or(LineError::AmountOutOfRange)?;

        let (dc_rt, dc_amt) = match self.discount {
            None => (Decimal::ZERO, Money::zero()),
            Some(LineDiscount::Percent(p)) => {
                // Rate first, so the stored rate and the amount agree.
                let rate = round_half_up(p, 2);
                let off = sply_amt
                    .amount()
                    .checked_mul(rate)
                    .and_then(|x| x.checked_div(Decimal::ONE_HUNDRED))
                    .and_then(whole_francs)
                    .ok_or(LineError::AmountOutOfRange)?;
                (rate, off)
            }
            Some(LineDiscount::Amount(m)) => {
                let off = whole_francs(m.amount()).ok_or(LineError::AmountOutOfRange)?;
                (derive_rate(off, sply_amt), off)
            }
        };

        if dc_amt < Money::zero() {
            return Err(LineError::NegativeDiscount);
        }
        if dc_amt > sply_amt {
            return Err(LineError::DiscountExceedsSupply);
        }
        let taxbl_amt = sply_amt
            .checked_sub(dc_amt)
            .ok_or(LineError::AmountOutOfRange)?;
        Ok(LineAmounts {
            sply_amt,
            dc_rt,
            dc_amt,
            taxbl_amt,
            tax_amt: extract_tax(taxbl_amt, self.tax_ty_cd),
            tot_amt: taxbl_amt,
        })
    }
}

/// `dc_amt / sply_amt x 100`, half-up to two places. `sply_amt` can still be
/// zero here, because a tiny quantity at a small price rounds to zero francs
/// even though the quantity itself is positive, so the division is guarded and
/// a zero line reports a zero rate. (A discount on such a line is then refused
/// for exceeding it, unless it is zero too.)
fn derive_rate(dc_amt: Money, sply_amt: Money) -> Decimal {
    dc_amt
        .amount()
        .checked_mul(Decimal::ONE_HUNDRED)
        .and_then(|x| x.checked_div(sply_amt.amount()))
        .map_or(Decimal::ZERO, |r| round_half_up(r, 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(n: i64) -> Decimal {
        Decimal::from(n)
    }

    fn line(
        item_cd: &str,
        qty: &str,
        prc: &str,
        band: TaxType,
        discount: Option<LineDiscount>,
    ) -> BasketLine {
        BasketLine {
            item_seq: 1,
            item_cd: item_cd.to_string(),
            item_cls_cd: "5020230602".to_string(),
            item_nm: "Test item".to_string(),
            bcd: None,
            product_id: None,
            qty: Qty::try_from_str(qty).unwrap(),
            prc: Money::try_from_str(prc).unwrap(),
            tax_ty_cd: band,
            discount,
        }
    }

    fn money(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    #[test]
    fn an_undiscounted_line_matches_the_vsdc_sample() {
        let l = line("RW1NTXU0000001", "1", "200000", TaxType::B, None);
        let a = l.amounts().unwrap();
        assert_eq!(a.sply_amt.to_string(), "200000.00");
        assert_eq!(a.dc_amt.to_string(), "0.00");
        assert_eq!(a.taxbl_amt.to_string(), "200000.00");
        assert_eq!(a.tax_amt.to_string(), "30508.47");
        assert_eq!(a.tot_amt, a.taxbl_amt);
    }
    #[test]
    fn a_weighed_line_rounds_to_a_payable_whole_franc() {
        // 0.734 kg of tomatoes at 1850 RWF/kg = 1357.900 exactly, which no cashier
        // can take. Ruling R57: the line rounds to whole francs before tax.
        let a = line("P-TOM", "0.734", "1850", TaxType::B, None)
            .amounts()
            .unwrap();
        assert_eq!(a.sply_amt.to_string(), "1358.00");
        assert_eq!(a.taxbl_amt.to_string(), "1358.00");
        assert_eq!(a.tax_amt.to_string(), "207.15"); // 1358 x 18/118, kept at 2dp
    }
    #[test]
    fn rounding_to_whole_francs_goes_up_at_the_half() {
        // 0.5 kg at 1001 RWF/kg = 500.50 -> 501, not 500
        let a = line("P-X", "0.5", "1001", TaxType::B, None)
            .amounts()
            .unwrap();
        assert_eq!(a.sply_amt.to_string(), "501.00");
    }
    #[test]
    fn the_product_is_rounded_once_not_twice() {
        // 0.499 kg x 1001 = 499.499 exactly. Rounding to 2 dp first gives
        // 499.50, which then rounds up to 500; rounding once gives 499.
        let a = line("P-X", "0.499", "1001", TaxType::B, None)
            .amounts()
            .unwrap();
        assert_eq!(a.sply_amt.to_string(), "499.00");
    }
    #[test]
    fn a_percent_discount_sets_both_dc_rt_and_dc_amt() {
        let a = line(
            "P",
            "2",
            "5000",
            TaxType::B,
            Some(LineDiscount::Percent(dec(10))),
        )
        .amounts()
        .unwrap();
        assert_eq!(a.sply_amt.to_string(), "10000.00");
        assert_eq!(a.dc_amt.to_string(), "1000.00");
        assert_eq!(a.dc_rt.to_string(), "10");
        assert_eq!(a.taxbl_amt.to_string(), "9000.00");
    }
    #[test]
    fn an_amount_discount_derives_dc_rt() {
        let a = line(
            "P",
            "1",
            "800",
            TaxType::B,
            Some(LineDiscount::Amount(money("200"))),
        )
        .amounts()
        .unwrap();
        assert_eq!(a.dc_rt.to_string(), "25");
    }
    #[test]
    fn an_exempt_line_is_taxable_but_untaxed() {
        let a = line("P-MILK", "1", "1200", TaxType::A, None)
            .amounts()
            .unwrap();
        assert_eq!(a.taxbl_amt.to_string(), "1200.00");
        assert_eq!(a.tax_amt.to_string(), "0.00");
    }
    #[test]
    fn zero_or_negative_quantity_is_refused() {
        assert!(matches!(
            line("P", "0", "100", TaxType::B, None).amounts(),
            Err(LineError::NonPositiveQty)
        ));
        assert!(matches!(
            line("P", "-1", "100", TaxType::B, None).amounts(),
            Err(LineError::NonPositiveQty)
        ));
    }
    #[test]
    fn a_discount_larger_than_the_line_is_refused() {
        let d = Some(LineDiscount::Amount(money("101")));
        assert!(matches!(
            line("P", "1", "100", TaxType::B, d).amounts(),
            Err(LineError::DiscountExceedsSupply)
        ));
    }
    #[test]
    fn a_percent_over_one_hundred_is_refused() {
        let d = Some(LineDiscount::Percent(dec(101)));
        assert!(matches!(
            line("P", "1", "100", TaxType::B, d).amounts(),
            Err(LineError::DiscountExceedsSupply)
        ));
    }
    #[test]
    fn a_negative_discount_is_refused_not_applied_as_a_surcharge() {
        for d in [
            LineDiscount::Percent(dec(-10)),
            LineDiscount::Amount(money("-5")),
        ] {
            assert!(matches!(
                line("P", "1", "100", TaxType::B, Some(d)).amounts(),
                Err(LineError::NegativeDiscount)
            ));
        }
    }
    #[test]
    fn a_full_discount_is_allowed_and_leaves_nothing_taxable() {
        let a = line(
            "P",
            "1",
            "100",
            TaxType::B,
            Some(LineDiscount::Percent(dec(100))),
        )
        .amounts()
        .unwrap();
        assert_eq!(a.taxbl_amt.to_string(), "0.00");
        assert_eq!(a.tax_amt.to_string(), "0.00");
    }
    #[test]
    fn an_amount_discount_rate_rounds_half_up_to_two_places() {
        // 1 / 3 x 100 = 33.333.. -> 33.33 ; 2 / 3 x 100 = 66.666.. -> 66.67
        let third = |off: &str| {
            line(
                "P",
                "1",
                "3",
                TaxType::B,
                Some(LineDiscount::Amount(money(off))),
            )
            .amounts()
            .unwrap()
            .dc_rt
            .to_string()
        };
        assert_eq!(third("1"), "33.33");
        assert_eq!(third("2"), "66.67");
    }
    #[test]
    fn a_discount_is_whole_francs_too() {
        // 12.5% of 1001 = 125.125 -> 125 ; the stored amount is payable
        let a = line(
            "P",
            "1",
            "1001",
            TaxType::B,
            Some(LineDiscount::Percent(Decimal::new(125, 1))),
        )
        .amounts()
        .unwrap();
        assert_eq!(a.dc_amt.to_string(), "125.00");
        assert_eq!(a.dc_rt.to_string(), "12.5");
        assert_eq!(a.taxbl_amt.to_string(), "876.00");
    }
    #[test]
    fn a_line_that_rounds_to_zero_francs_does_not_divide_by_zero() {
        // 0.001 x 1 = 0.001 -> 0.00 -> 0 francs, though the quantity is positive
        let none = line(
            "P",
            "0.001",
            "1",
            TaxType::B,
            Some(LineDiscount::Amount(Money::zero())),
        )
        .amounts()
        .unwrap();
        assert_eq!(none.sply_amt.to_string(), "0.00");
        assert_eq!(none.dc_rt.to_string(), "0");
        let some = Some(LineDiscount::Amount(money("1")));
        assert!(matches!(
            line("P", "0.001", "1", TaxType::B, some).amounts(),
            Err(LineError::DiscountExceedsSupply)
        ));
    }
    #[test]
    fn an_unstorable_line_is_refused_by_name() {
        let r = line("P", "2", "9999999999.99", TaxType::B, None).amounts();
        assert!(matches!(r, Err(LineError::AmountOutOfRange)));
        // rounds up past the bound only at the whole-franc step
        let r = line("P", "1", "9999999999.50", TaxType::B, None).amounts();
        assert!(matches!(r, Err(LineError::AmountOutOfRange)));
    }
    #[test]
    fn an_absurd_percent_is_refused_without_panicking() {
        let d = Some(LineDiscount::Percent(Decimal::MAX));
        assert!(line("P", "1", "100", TaxType::B, d).amounts().is_err());
    }

    #[test]
    fn each_text_field_is_refused_one_over_its_limit_and_accepted_at_it() {
        let cases: [(&str, usize); 4] = [
            ("itemCd", ITEM_CD_MAX),
            ("itemClsCd", ITEM_CLS_CD_MAX),
            ("itemNm", ITEM_NM_MAX),
            ("bcd", BCD_MAX),
        ];
        for (field, max) in cases {
            let set = |l: &mut BasketLine, n: usize| {
                let v = "x".repeat(n);
                match field {
                    "itemCd" => l.item_cd = v,
                    "itemClsCd" => l.item_cls_cd = v,
                    "itemNm" => l.item_nm = v,
                    _ => l.bcd = Some(v),
                }
            };
            let mut l = line("A", "1", "100", TaxType::B, None);
            set(&mut l, max);
            assert!(l.amounts().is_ok(), "{field} at the limit");
            set(&mut l, max + 1);
            assert_eq!(
                l.amounts(),
                Err(LineError::FieldTooLong(FieldTooLong {
                    field,
                    max,
                    got: max + 1
                })),
                "{field} one over"
            );
        }
    }

    #[test]
    fn length_counts_characters_not_bytes() {
        let mut l = line("A", "1", "100", TaxType::B, None);
        l.item_nm = "é".repeat(ITEM_NM_MAX);
        assert!(l.amounts().is_ok());
    }

    #[test]
    fn buyer_tin_over_nine_is_refused() {
        assert!(validate_buyer_tin("100110263").is_ok());
        assert_eq!(
            validate_buyer_tin("1001102634"),
            Err(FieldTooLong {
                field: "custTin",
                max: 9,
                got: 10
            })
        );
    }
}
