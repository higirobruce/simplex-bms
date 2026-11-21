//! Tender settlement: how a basket is paid for and what change is due.
//!
//! Every amount here is whole RWF by the time it arrives (RWF has no
//! circulating subunit), so change is whole too and nothing in this module
//! rounds.

use thiserror::Error;

use crate::codes::PmtTyCd;
use crate::money::Money;

/// Why a set of tenders could not settle a basket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TenderError {
    #[error("tendered {taken} does not cover {due}")]
    Insufficient { due: Money, taken: Money },
    #[error("no tender was given")]
    Empty,
    #[error("a tender must be a positive amount")]
    NonPositiveAmount,
    /// The overpayment is more than the cash handed over, so part of it would
    /// have to be returned against a card or transfer. That is a refund of
    /// money the till never received, not change.
    #[error("overpayment cannot be returned as change against a non-cash tender")]
    OverTenderOnNonCash,
    /// The 12-digit bound on [`Money`] was exceeded while summing.
    #[error("tender total exceeds the supported range")]
    OutOfRange,
}

/// One payment toward a basket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenderEntry {
    pub pmt_ty_cd: PmtTyCd,
    pub amount: Money,
}

/// The outcome of a successful settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settled {
    pub change: Money,
    pub header_pmt_ty_cd: PmtTyCd,
}

/// All the payments offered for one basket, in the order they were taken.
#[derive(Debug, Clone, Default)]
pub struct TenderSet(Vec<TenderEntry>);

impl TenderSet {
    pub fn push(&mut self, entry: TenderEntry) {
        self.0.push(entry);
    }

    pub fn entries(&self) -> &[TenderEntry] {
        &self.0
    }

    /// Sum of every entry, as the brief's signature requires. An empty set is
    /// zero, and so, deliberately, is a sum past the 12-digit bound: never a
    /// wrapped or clamped figure. Nothing should decide money from this
    /// alone; [`TenderSet::settle`] reports the overflow as `OutOfRange`.
    pub fn total(&self) -> Money {
        self.checked_total().unwrap_or_else(Money::zero)
    }

    fn checked_total(&self) -> Option<Money> {
        self.0
            .iter()
            .try_fold(Money::zero(), |acc, e| acc.checked_add(e.amount))
    }

    /// Settles a basket whose amount due is `due` (from `BasketTotals::tot_amt`).
    ///
    /// Checks run in a fixed order so the same input always gives the same
    /// error: empty, non-positive entry, out-of-range sum, shortfall, then
    /// change against non-cash.
    pub fn settle(&self, due: Money) -> Result<Settled, TenderError> {
        if self.0.is_empty() {
            return Err(TenderError::Empty);
        }
        if self.0.iter().any(|e| e.amount <= Money::zero()) {
            return Err(TenderError::NonPositiveAmount);
        }
        let taken = self.checked_total().ok_or(TenderError::OutOfRange)?;
        if taken < due {
            return Err(TenderError::Insufficient { due, taken });
        }
        let change = taken.checked_sub(due).ok_or(TenderError::OutOfRange)?;
        if change > Money::zero() {
            // Change is paid out of the drawer, so it can be at most the cash
            // actually handed over.
            let cash = self
                .0
                .iter()
                .filter(|e| e.pmt_ty_cd == PmtTyCd::Cash)
                .try_fold(Money::zero(), |acc, e| acc.checked_add(e.amount))
                .ok_or(TenderError::OutOfRange)?;
            if change > cash {
                return Err(TenderError::OverTenderOnNonCash);
            }
        }
        let header_pmt_ty_cd = self.header_code().ok_or(TenderError::Empty)?;
        Ok(Settled {
            change,
            header_pmt_ty_cd,
        })
    }

    /// The single code RRA's header can carry: the largest entry's, ties to
    /// the lower code number. Ordering by (amount descending, wire code
    /// ascending) is total, so equal amounts, equal codes and a single entry
    /// all resolve the same way every time.
    fn header_code(&self) -> Option<PmtTyCd> {
        self.0
            .iter()
            .min_by(|a, b| {
                b.amount
                    .cmp(&a.amount)
                    .then_with(|| a.pmt_ty_cd.wire().cmp(b.pmt_ty_cd.wire()))
            })
            .map(|e| e.pmt_ty_cd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }
    fn e(c: PmtTyCd, s: &str) -> TenderEntry {
        TenderEntry {
            pmt_ty_cd: c,
            amount: m(s),
        }
    }
    fn set(es: &[TenderEntry]) -> TenderSet {
        let mut t = TenderSet::default();
        for x in es {
            t.push(*x);
        }
        t
    }

    #[test]
    fn exact_cash_settles_with_no_change() {
        let s = set(&[e(PmtTyCd::Cash, "2380")]).settle(m("2380")).unwrap();
        assert_eq!(s.change, Money::zero());
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Cash);
    }
    #[test]
    fn over_tendered_cash_gives_change() {
        let s = set(&[e(PmtTyCd::Cash, "5000")]).settle(m("2380")).unwrap();
        assert_eq!(s.change.to_string(), "2620.00");
    }
    #[test]
    fn a_mixed_tender_reports_the_largest_entry_on_the_header() {
        let s = set(&[e(PmtTyCd::Cash, "1000"), e(PmtTyCd::MobileMoney, "1380")])
            .settle(m("2380"))
            .unwrap();
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::MobileMoney);
        assert_eq!(s.change, Money::zero());
    }
    #[test]
    fn the_largest_entry_wins_even_with_a_lower_code() {
        let s = set(&[e(PmtTyCd::Cash, "1380"), e(PmtTyCd::MobileMoney, "1000")])
            .settle(m("2380"))
            .unwrap();
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Cash);
    }
    #[test]
    fn equal_amounts_go_to_the_lower_code_in_either_order() {
        for es in [
            [e(PmtTyCd::Card, "1000"), e(PmtTyCd::Cash, "1000")],
            [e(PmtTyCd::Cash, "1000"), e(PmtTyCd::Card, "1000")],
        ] {
            let s = set(&es).settle(m("2000")).unwrap();
            assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Cash);
        }
    }
    #[test]
    fn the_header_is_independent_of_entry_order() {
        let a = [
            e(PmtTyCd::Other, "700"),
            e(PmtTyCd::Card, "700"),
            e(PmtTyCd::BankCheck, "700"),
            e(PmtTyCd::Cash, "100"),
        ];
        let mut b = a;
        b.reverse();
        let sa = set(&a).settle(m("2200")).unwrap();
        let sb = set(&b).settle(m("2200")).unwrap();
        assert_eq!(sa.header_pmt_ty_cd, PmtTyCd::BankCheck);
        assert_eq!(sa, sb);
    }
    #[test]
    fn underpayment_is_refused_with_both_figures() {
        match set(&[e(PmtTyCd::Cash, "2000")]).settle(m("2380")) {
            Err(TenderError::Insufficient { due, taken }) => {
                assert_eq!(due.to_string(), "2380.00");
                assert_eq!(taken.to_string(), "2000.00");
            }
            other => panic!("expected Insufficient, got {other:?}"),
        }
    }
    #[test]
    fn change_is_never_given_against_a_card() {
        assert!(matches!(
            set(&[e(PmtTyCd::Card, "5000")]).settle(m("2380")),
            Err(TenderError::OverTenderOnNonCash)
        ));
    }
    #[test]
    fn exact_card_and_mobile_money_settle() {
        let s = set(&[e(PmtTyCd::Card, "2380")]).settle(m("2380")).unwrap();
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Card);
        assert_eq!(s.change, Money::zero());
    }
    #[test]
    fn change_in_a_mixed_tender_is_bounded_by_the_cash_handed_over() {
        // 1000 cash + 2000 card against 2380: 620 change, drawer cash covers it
        let s = set(&[e(PmtTyCd::Cash, "1000"), e(PmtTyCd::Card, "2000")])
            .settle(m("2380"))
            .unwrap();
        assert_eq!(s.change.to_string(), "620.00");
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Card);
        // 100 cash + 5000 card against 2380: 2720 change exceeds the cash
        assert!(matches!(
            set(&[e(PmtTyCd::Cash, "100"), e(PmtTyCd::Card, "5000")]).settle(m("2380")),
            Err(TenderError::OverTenderOnNonCash)
        ));
    }
    #[test]
    fn a_zero_or_negative_tender_is_refused() {
        assert!(matches!(
            set(&[e(PmtTyCd::Cash, "0")]).settle(m("1")),
            Err(TenderError::NonPositiveAmount)
        ));
        assert!(matches!(
            set(&[e(PmtTyCd::Cash, "5000"), e(PmtTyCd::Card, "-1")]).settle(m("1")),
            Err(TenderError::NonPositiveAmount)
        ));
    }
    #[test]
    fn an_empty_set_is_refused() {
        assert!(matches!(
            TenderSet::default().settle(Money::zero()),
            Err(TenderError::Empty)
        ));
    }
    #[test]
    fn a_sum_beyond_twelve_digits_is_an_error_not_a_wrap() {
        let big = "9999999999.99";
        let t = set(&[e(PmtTyCd::Cash, big), e(PmtTyCd::Cash, big)]);
        assert!(matches!(t.settle(m("1")), Err(TenderError::OutOfRange)));
    }
    #[test]
    fn total_sums_entries() {
        let t = set(&[e(PmtTyCd::Cash, "1000"), e(PmtTyCd::Card, "1380")]);
        assert_eq!(t.total().to_string(), "2380.00");
    }
}
