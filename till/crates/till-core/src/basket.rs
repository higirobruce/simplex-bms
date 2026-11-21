//! A basket of lines and the per-tax-band totals that head a `saveSales` payload.

use rust_decimal::Decimal;
use thiserror::Error;

use crate::codes::TaxType;
use crate::line::{BasketLine, LineDiscount, LineError};
use crate::money::Money;

/// Why a basket could not be changed or totalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BasketError {
    /// An empty basket has no fiscal meaning; RRA would reject the payload.
    #[error("the basket has no lines")]
    Empty,
    #[error("no line with that item sequence")]
    NoSuchLine,
    /// `item_seq` is a `u16` on the wire and is never reused, so the counter
    /// can run out. Refused by name rather than wrapped onto a used number.
    #[error("too many lines have been added to this basket")]
    TooManyLines,
    /// Summing many lines can exceed the 12-digit bound every `Money` carries.
    /// Refused rather than saturated: a clipped total is a wrong fiscal total.
    #[error("basket total is outside the supported range")]
    TotalOutOfRange,
    #[error(transparent)]
    Line(#[from] LineError),
}

/// Lines being rung up. `item_seq` is issued by [`Basket::push_line`] from a
/// counter that voids never rewind: a payload's `itemSeq` identifies a line,
/// so reusing a voided number would misattribute it.
#[derive(Debug, Clone, Default)]
pub struct Basket {
    lines: Vec<BasketLine>,
    /// Whole-basket discount; applied by a later task, held here so `Basket`
    /// keeps one shape.
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
            .ok_or(BasketError::TooManyLines)?;
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
        for line in &self.lines {
            let a = line.amounts()?;
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
}
