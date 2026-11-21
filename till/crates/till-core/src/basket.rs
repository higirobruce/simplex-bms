//! A basket of lines and the per-tax-band totals that head a `saveSales` payload.

use rust_decimal::Decimal;
use thiserror::Error;

use crate::codes::TaxType;
use crate::line::LineAmounts;
use crate::line::{BasketLine, LineDiscount, LineError};
use crate::money::{Money, round_half_up};

/// Highest `item_seq` a sale can carry. In the VSDC `TrnsSalesSaveWrItem`,
/// `itemSeq` is a required NUMBER of data length 3, so a 1000th line has no
/// representable sequence number. Not arbitrary: do not raise it without an RRA
/// spec change. The cap counts numbers *issued*, not lines surviving, because
/// a voided number is already spent and is never reissued.
pub const MAX_ITEM_SEQ: u16 = 999;

/// Why a basket could not be changed or totalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BasketError {
    /// An empty basket has no fiscal meaning; RRA would reject the payload.
    #[error("the basket has no lines")]
    Empty,
    #[error("no line with that item sequence")]
    NoSuchLine,
    /// The sequence numbers of a sale are used up (see [`MAX_ITEM_SEQ`]).
    /// Refused at the till, while the cashier can still split the basket into
    /// two sales, rather than at the VSDC after the invoice number is spent.
    #[error("a sale holds at most {max} lines; finish this sale and start another")]
    TooManyLines { max: u16 },
    /// Summing many lines can exceed the 12-digit bound every `Money` carries.
    /// Refused rather than saturated: a clipped total is a wrong fiscal total.
    #[error("basket total is outside the supported range")]
    TotalOutOfRange,
    /// The basket discount is larger than the basket, or would take more than
    /// one line's supply amount. Refused, never clamped.
    #[error("the basket discount is larger than the basket it applies to")]
    DiscountExceedsBasket,
    /// Rounding the earlier shares up left a negative residual for the last
    /// line, which would be a surcharge. Refused rather than hidden.
    #[error("the basket discount cannot be split into whole francs across these lines")]
    DiscountNotSplittable,
    #[error(transparent)]
    Line(#[from] LineError),
}

/// Lines being rung up. `item_seq` is issued by [`Basket::push_line`] from a
/// counter that voids never rewind: a payload's `itemSeq` identifies a line,
/// so reusing a voided number would misattribute it.
#[derive(Debug, Clone, Default)]
pub struct Basket {
    lines: Vec<BasketLine>,
    /// Whole-basket discount. RRA has no header-level discount, so it is
    /// pushed onto the lines by [`Basket::apportioned_lines`].
    pub basket_discount: Option<LineDiscount>,
    last_seq: u16,
}

/// Header totals, indexed `A=0 B=1 C=2 D=3`, on the RRA wire names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasketTotals {
    /// Number of lines, not units: a 0.734 kg line is one item, and RRA's
    /// sample shows two one-unit lines as `totItemCnt: 2`.
    pub tot_item_cnt: u32,
    pub taxbl_amt: [Money; 4],
    pub tax_rt: [Decimal; 4],
    pub tax_amt: [Money; 4],
    pub tot_taxbl_amt: Money,
    pub tot_tax_amt: Money,
    pub tot_amt: Money,
}

/// Slot of a band in the totals arrays. An exhaustive `match`, not a cast, so
/// adding a band fails to compile here instead of panicking at runtime.
fn slot(band: TaxType) -> usize {
    match band {
        TaxType::A => 0,
        TaxType::B => 1,
        TaxType::C => 2,
        TaxType::D => 3,
    }
}

impl BasketTotals {
    /// `(taxbl_amt, tax_rt, tax_amt)` for one band.
    pub fn by_band(&self, band: TaxType) -> (Money, Decimal, Money) {
        let i = slot(band);
        // `slot` yields 0..=3 for arrays of 4, so `get` always hits; the
        // fallback keeps this panic-free if that ever stops being true.
        match (
            self.taxbl_amt.get(i),
            self.tax_rt.get(i),
            self.tax_amt.get(i),
        ) {
            (Some(t), Some(r), Some(a)) => (*t, *r, *a),
            _ => (Money::zero(), Decimal::ZERO, Money::zero()),
        }
    }
}

fn add(a: Money, b: Money) -> Result<Money, BasketError> {
    a.checked_add(b).ok_or(BasketError::TotalOutOfRange)
}

impl Basket {
    pub fn new() -> Basket {
        Basket::default()
    }

    /// Adds a line, overwriting its `item_seq` with the next unused number
    /// (from 1), which is returned.
    pub fn push_line(&mut self, mut line: BasketLine) -> Result<u16, BasketError> {
        let seq = self
            .last_seq
            .checked_add(1)
            .filter(|&n| n <= MAX_ITEM_SEQ)
            .ok_or(BasketError::TooManyLines { max: MAX_ITEM_SEQ })?;
        self.last_seq = seq;
        line.item_seq = seq;
        self.lines.push(line);
        Ok(seq)
    }

    /// Removes a line. Later lines keep their `item_seq`.
    pub fn void_line(&mut self, item_seq: u16) -> Result<(), BasketError> {
        let pos = self
            .lines
            .iter()
            .position(|l| l.item_seq == item_seq)
            .ok_or(BasketError::NoSuchLine)?;
        self.lines.remove(pos);
        Ok(())
    }

    pub fn set_basket_discount(&mut self, discount: LineDiscount) {
        self.basket_discount = Some(discount);
    }

    /// Each line with its final amounts, the basket discount pushed down.
    ///
    /// The total discount is a whole-franc figure: an amount rounds half-up, a
    /// percentage is taken of the summed `sply_amt` (the lines' own discounts
    /// do not shrink the base) and rounds half-up. Each line's share is
    /// `total x sply_amt / sum(sply_amt)` rounded half-up to whole francs, and
    /// the LAST line takes `total - sum(earlier shares)`, so the shares sum
    /// exactly to the total. It is added to the line's own discount. With no
    /// basket discount the lines come back untouched.
    pub fn apportioned_lines(&self) -> Result<Vec<(BasketLine, LineAmounts)>, BasketError> {
        let mut priced = Vec::with_capacity(self.lines.len());
        let mut sum_sply = Money::zero();
        for line in &self.lines {
            let a = line.amounts()?;
            sum_sply = add(sum_sply, a.sply_amt)?;
            priced.push((line.clone(), a));
        }
        let Some(discount) = self.basket_discount else {
            return Ok(priced);
        };
        let total = match discount {
            LineDiscount::Amount(m) => round_half_up(m.amount(), 0),
            LineDiscount::Percent(p) => sum_sply
                .amount()
                .checked_mul(round_half_up(p, 2))
                .and_then(|x| x.checked_div(Decimal::ONE_HUNDRED))
                .map(|x| round_half_up(x, 0))
                .ok_or(BasketError::TotalOutOfRange)?,
        };
        if total < Decimal::ZERO {
            return Err(LineError::NegativeDiscount.into());
        }
        if total > sum_sply.amount() {
            return Err(BasketError::DiscountExceedsBasket);
        }
        if total.is_zero() {
            return Ok(priced);
        }
        let last = priced.len().saturating_sub(1);
        let mut given = Decimal::ZERO;
        let mut out = Vec::with_capacity(priced.len());
        for (i, (mut line, a)) in priced.into_iter().enumerate() {
            let share = if i == last {
                total - given
            } else {
                total
                    .checked_mul(a.sply_amt.amount())
                    .and_then(|x| x.checked_div(sum_sply.amount()))
                    .map(|x| round_half_up(x, 0))
                    .ok_or(BasketError::TotalOutOfRange)?
            };
            if share < Decimal::ZERO {
                return Err(BasketError::DiscountNotSplittable);
            }
            given += share;
            let combined = Money::from_decimal(a.dc_amt.amount() + share)
                .ok_or(BasketError::TotalOutOfRange)?;
            if combined > a.sply_amt {
                return Err(BasketError::DiscountExceedsBasket);
            }
            line.discount = Some(LineDiscount::Amount(combined));
            let amounts = line.amounts()?;
            out.push((line, amounts));
        }
        Ok(out)
    }

    pub fn lines(&self) -> impl Iterator<Item = &BasketLine> {
        self.lines.iter()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Per-band totals. The header tax is the SUM OF THE LINE TAXES, never a
    /// fresh extraction from the basket total: each line's tax is rounded on
    /// its own, and RRA's sample header (`38135`) is the sum of its lines'
    /// `30508` and `7627`. The two differ once rounding bites.
    ///
    /// Line amounts arrive already rounded, so nothing is rounded here.
    pub fn totals(&self) -> Result<BasketTotals, BasketError> {
        if self.lines.is_empty() {
            return Err(BasketError::Empty);
        }
        let mut taxbl = [Money::zero(); 4];
        let mut tax = [Money::zero(); 4];
        let mut tot_amt = Money::zero();
        for (line, a) in self.apportioned_lines()? {
            let i = slot(line.tax_ty_cd);
            if let (Some(t), Some(x)) = (taxbl.get_mut(i), tax.get_mut(i)) {
                *t = add(*t, a.taxbl_amt)?;
                *x = add(*x, a.tax_amt)?;
            }
            tot_amt = add(tot_amt, a.tot_amt)?;
        }
        let mut tax_rt = [Decimal::ZERO; 4];
        for &band in TaxType::ALL {
            if let Some(r) = tax_rt.get_mut(slot(band)) {
                *r = band.rate();
            }
        }
        let mut tot_taxbl_amt = Money::zero();
        let mut tot_tax_amt = Money::zero();
        for i in 0..4 {
            if let (Some(t), Some(x)) = (taxbl.get(i), tax.get(i)) {
                tot_taxbl_amt = add(tot_taxbl_amt, *t)?;
                tot_tax_amt = add(tot_tax_amt, *x)?;
            }
        }
        Ok(BasketTotals {
            tot_item_cnt: u32::try_from(self.lines.len())
                .map_err(|_| BasketError::TotalOutOfRange)?,
            taxbl_amt: taxbl,
            tax_rt,
            tax_amt: tax,
            tot_taxbl_amt,
            tot_tax_amt,
            tot_amt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Qty;

    fn dec(n: i64) -> Decimal {
        Decimal::from(n)
    }
    use crate::tax::extract_tax;

    fn line(
        item_cd: &str,
        qty: &str,
        prc: &str,
        band: TaxType,
        discount: Option<LineDiscount>,
    ) -> BasketLine {
        BasketLine {
            item_seq: 0,
            item_cd: item_cd.to_string(),
            item_cls_cd: "5020230602".to_string(),
            item_nm: "Test item".to_string(),
            bcd: None,
            qty: Qty::try_from_str(qty).unwrap(),
            prc: Money::try_from_str(prc).unwrap(),
            tax_ty_cd: band,
            discount,
        }
    }

    #[test]
    fn the_vsdc_sample_basket_reproduces_its_header() {
        let mut b = Basket::new();
        b.push_line(line("RW1NTXU0000001", "1", "200000", TaxType::B, None))
            .unwrap();
        b.push_line(line("RW1NTXU0000002", "1", "50000", TaxType::B, None))
            .unwrap();
        let t = b.totals().unwrap();
        assert_eq!(t.tot_item_cnt, 2);
        assert_eq!(t.taxbl_amt[1].to_string(), "250000.00");
        assert_eq!(t.tax_rt[1].to_string(), "18");
        assert_eq!(t.tot_taxbl_amt.to_string(), "250000.00");
        assert_eq!(t.tot_tax_amt.to_string(), "38135.59"); // 30508.47 + 7627.12
        assert_eq!(t.tot_amt, t.tot_taxbl_amt);
    }

    #[test]
    fn header_tax_is_the_sum_of_lines_not_a_recomputation() {
        // Three 5 RWF lines: each tax rounds to 0.76, summing to 2.28, while
        // extracting from the 15 RWF gross gives 2.29. The lines win.
        let mut b = Basket::new();
        for _ in 0..3 {
            b.push_line(line("P", "1", "5", TaxType::B, None)).unwrap();
        }
        let t = b.totals().unwrap();
        let gross = Money::try_from_str("15").unwrap();
        assert_eq!(t.tot_tax_amt.to_string(), "2.28");
        assert_eq!(extract_tax(gross, TaxType::B).to_string(), "2.29");
        assert_ne!(t.tot_tax_amt, extract_tax(gross, TaxType::B));
        assert_eq!(t.tot_amt.to_string(), "15.00");
    }

    #[test]
    fn exempt_and_standard_rated_goods_split_across_bands() {
        let mut b = Basket::new();
        b.push_line(line("P-MILK", "1", "1200", TaxType::A, None))
            .unwrap();
        b.push_line(line("P-SOAP", "1", "1180", TaxType::B, None))
            .unwrap();
        let t = b.totals().unwrap();
        assert_eq!(t.taxbl_amt[0].to_string(), "1200.00");
        assert_eq!(t.tax_amt[0].to_string(), "0.00");
        assert_eq!(t.tax_rt[0].to_string(), "0");
        assert_eq!(t.taxbl_amt[1].to_string(), "1180.00");
        assert_eq!(t.tax_amt[1].to_string(), "180.00");
        assert_eq!(t.tot_taxbl_amt.to_string(), "2380.00");
        assert_eq!(t.tot_tax_amt.to_string(), "180.00");
        assert_eq!(t.tot_amt.to_string(), "2380.00");
        let (tb, r, ta) = t.by_band(TaxType::B);
        assert_eq!((tb, r, ta), (t.taxbl_amt[1], t.tax_rt[1], t.tax_amt[1]));
    }

    #[test]
    fn an_empty_basket_has_no_totals() {
        assert!(matches!(Basket::new().totals(), Err(BasketError::Empty)));
    }

    #[test]
    fn voiding_a_line_renumbers_nothing_and_drops_it_from_the_totals() {
        let mut b = Basket::new();
        b.push_line(line("A", "1", "1000", TaxType::B, None))
            .unwrap();
        b.push_line(line("B", "1", "2000", TaxType::B, None))
            .unwrap();
        b.void_line(1).unwrap();
        let t = b.totals().unwrap();
        assert_eq!(t.tot_item_cnt, 1);
        assert_eq!(t.tot_amt.to_string(), "2000.00");
        assert_eq!(b.lines().next().unwrap().item_seq, 2);
    }

    #[test]
    fn a_voided_item_seq_is_never_reissued() {
        let mut b = Basket::new();
        b.push_line(line("A", "1", "1000", TaxType::B, None))
            .unwrap();
        b.void_line(1).unwrap();
        let seq = b
            .push_line(line("B", "1", "1000", TaxType::B, None))
            .unwrap();
        assert_eq!(seq, 2);
        assert_eq!(b.void_line(1), Err(BasketError::NoSuchLine));
    }

    #[test]
    fn the_thousandth_line_is_refused_even_after_a_void() {
        let mut b = Basket::new();
        let mut last = 0;
        for _ in 0..999 {
            last = b.push_line(line("P", "1", "10", TaxType::A, None)).unwrap();
        }
        assert_eq!(last, 999);
        let full = Err(BasketError::TooManyLines { max: 999 });
        assert_eq!(b.push_line(line("P", "1", "10", TaxType::A, None)), full);
        // Issued numbers count, not surviving lines: a void frees nothing.
        b.void_line(1).unwrap();
        assert_eq!(b.push_line(line("P", "1", "10", TaxType::A, None)), full);
    }

    #[test]
    fn item_count_is_lines_not_units() {
        let mut b = Basket::new();
        b.push_line(line("W", "0.734", "3000", TaxType::B, None))
            .unwrap();
        assert_eq!(b.totals().unwrap().tot_item_cnt, 1);
    }

    #[test]
    fn a_total_past_twelve_digits_is_refused_not_clipped() {
        let mut b = Basket::new();
        for _ in 0..2 {
            b.push_line(line("BIG", "1", "9000000000", TaxType::A, None))
                .unwrap();
        }
        assert_eq!(b.totals(), Err(BasketError::TotalOutOfRange));
    }

    fn m(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    #[test]
    fn a_basket_discount_apportions_exactly_with_a_residual() {
        let mut b = Basket::new();
        for _ in 0..3 {
            b.push_line(line("P", "1", "1000", TaxType::B, None))
                .unwrap();
        }
        b.set_basket_discount(LineDiscount::Amount(m("100")));
        let apportioned = b.apportioned_lines().unwrap();
        let shares: Vec<String> = apportioned
            .iter()
            .map(|(_, a)| a.dc_amt.to_string())
            .collect();
        assert_eq!(shares, vec!["33.00", "33.00", "34.00"]);
        let taxbl: Vec<String> = apportioned
            .iter()
            .map(|(_, a)| a.taxbl_amt.to_string())
            .collect();
        assert_eq!(taxbl, vec!["967.00", "967.00", "966.00"]);
        assert_eq!(b.totals().unwrap().tot_taxbl_amt.to_string(), "2900.00");
    }

    #[test]
    fn a_percent_basket_discount_applies_to_every_line() {
        let mut b = Basket::new();
        b.push_line(line("A", "1", "1000", TaxType::B, None))
            .unwrap();
        b.push_line(line("B", "1", "3000", TaxType::B, None))
            .unwrap();
        b.set_basket_discount(LineDiscount::Percent(dec(10)));
        assert_eq!(b.totals().unwrap().tot_taxbl_amt.to_string(), "3600.00");
    }

    #[test]
    fn apportionment_respects_band_boundaries() {
        let mut b = Basket::new();
        b.push_line(line("MILK", "1", "1000", TaxType::A, None))
            .unwrap();
        b.push_line(line("SOAP", "1", "1000", TaxType::B, None))
            .unwrap();
        b.set_basket_discount(LineDiscount::Amount(m("200")));
        let t = b.totals().unwrap();
        assert_eq!(t.taxbl_amt[0].to_string(), "900.00");
        assert_eq!(t.taxbl_amt[1].to_string(), "900.00");
        assert_eq!(t.tax_amt[0].to_string(), "0.00");
        assert_eq!(t.tax_amt[1].to_string(), "137.29");
    }

    #[test]
    fn no_line_can_be_driven_negative() {
        let mut b = Basket::new();
        b.push_line(line("P", "1", "100", TaxType::B, None))
            .unwrap();
        b.set_basket_discount(LineDiscount::Amount(m("101")));
        assert!(matches!(
            b.totals(),
            Err(BasketError::DiscountExceedsBasket)
        ));
    }

    #[test]
    fn a_line_discount_and_a_basket_discount_compose() {
        let mut b = Basket::new();
        let d = Some(LineDiscount::Amount(m("100")));
        b.push_line(line("P", "1", "1000", TaxType::B, d)).unwrap();
        b.set_basket_discount(LineDiscount::Percent(dec(10)));
        assert_eq!(b.totals().unwrap().tot_taxbl_amt.to_string(), "800.00");
    }

    #[test]
    fn the_combined_discount_on_one_line_cannot_exceed_its_supply() {
        // Shares are 50 / 50, but line 1 already has 80 off its 100.
        let mut b = Basket::new();
        let d = Some(LineDiscount::Amount(m("80")));
        b.push_line(line("A", "1", "100", TaxType::B, d)).unwrap();
        b.push_line(line("B", "1", "100", TaxType::B, None))
            .unwrap();
        b.set_basket_discount(LineDiscount::Amount(m("100")));
        assert!(matches!(
            b.totals(),
            Err(BasketError::DiscountExceedsBasket)
        ));
    }

    #[test]
    fn a_rounding_overshoot_is_refused_not_turned_into_a_surcharge() {
        // 2 off four equal lines: each share 0.5 rounds up to 1, so the last
        // line's residual would be -1.
        let mut b = Basket::new();
        for _ in 0..4 {
            b.push_line(line("P", "1", "100", TaxType::B, None))
                .unwrap();
        }
        b.set_basket_discount(LineDiscount::Amount(m("2")));
        assert_eq!(b.totals(), Err(BasketError::DiscountNotSplittable));
    }
}
