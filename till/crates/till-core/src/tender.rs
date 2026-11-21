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
    /// A negative amount due is a refund, which is not a settlement.
    #[error("amount due {0} is negative; a refund is not settled as a sale")]
    NegativeDue(Money),
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

    /// Sum of every entry. `None` if the sum passes the 12-digit bound, so an
    /// unrepresentable total is never mistaken for a plausible one. An empty
    /// set sums to zero.
    pub fn total(&self) -> Option<Money> {
        self.checked_total()
    }

    fn checked_total(&self) -> Option<Money> {
        self.0
            .iter()
            .try_fold(Money::zero(), |acc, e| acc.checked_add(e.amount))
    }

    /// Settles a basket whose amount due is `due` (from `BasketTotals::tot_amt`).
    ///
    /// Checks run in a fixed order so the same input always gives the same
    /// error: empty, non-positive entry, negative due, out-of-range sum, shortfall, then
    /// change against non-cash.
    pub fn settle(&self, due: Money) -> Result<Settled, TenderError> {
        if self.0.is_empty() {
            return Err(TenderError::Empty);
        }
        if self.0.iter().any(|e| e.amount <= Money::zero()) {
            return Err(TenderError::NonPositiveAmount);
        }
        if due < Money::zero() {
            return Err(TenderError::NegativeDue(due));
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
        let header_pmt_ty_cd = self.header_code()?.ok_or(TenderError::Empty)?;
        Ok(Settled {
            change,
            header_pmt_ty_cd,
        })
    }

    /// The single code RRA's header can carry: how the sale was mostly paid.
    /// Amounts are summed per code first, then the largest total wins, ties to
    /// the lower wire code. Choosing the largest single entry instead would
    /// let Cash 600 + Cash 600 + Card 700 report Card although 1200 was cash.
    /// (total desc, wire code asc) is a total order over distinct codes, so
    /// the result never depends on entry order.
    fn header_code(&self) -> Result<Option<PmtTyCd>, TenderError> {
        let mut best: Option<(PmtTyCd, Money)> = None;
        for &code in PmtTyCd::ALL.iter() {
            let sum = self
                .0
                .iter()
                .filter(|e| e.pmt_ty_cd == code)
                .try_fold(Money::zero(), |acc, e| acc.checked_add(e.amount))
                .ok_or(TenderError::OutOfRange)?;
            if sum == Money::zero() {
                continue;
            }
            let better = match best {
                None => true,
                Some((bc, bs)) => sum > bs || (sum == bs && code.wire() < bc.wire()),
            };
            if better {
                best = Some((code, sum));
            }
        }
        Ok(best.map(|(c, _)| c))
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
        assert_eq!(t.total().unwrap().to_string(), "2380.00");
    }
    #[test]
    fn total_is_none_past_the_bound() {
        let big = "9999999999.99";
        let t = set(&[e(PmtTyCd::Cash, big), e(PmtTyCd::Card, big)]);
        assert_eq!(t.total(), None);
        assert_eq!(TenderSet::default().total(), Some(Money::zero()));
    }
    #[test]
    fn the_header_is_chosen_on_per_code_totals_not_single_entries() {
        // Per-entry selection would say Card (700 is the biggest keystroke);
        // the customer actually paid 1200 in cash, so the header is 01.
        let s = set(&[
            e(PmtTyCd::Cash, "600"),
            e(PmtTyCd::Cash, "600"),
            e(PmtTyCd::Card, "700"),
        ])
        .settle(m("1900"))
        .unwrap();
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Cash);
        assert_eq!(s.header_pmt_ty_cd.wire(), "01");
    }
    #[test]
    fn per_code_totals_tie_to_the_lower_code() {
        let s = set(&[
            e(PmtTyCd::Card, "500"),
            e(PmtTyCd::Card, "500"),
            e(PmtTyCd::Cash, "1000"),
        ])
        .settle(m("2000"))
        .unwrap();
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Cash);
    }
    #[test]
    fn a_negative_due_is_refused() {
        assert!(matches!(
            set(&[e(PmtTyCd::Cash, "100")]).settle(m("-1")),
            Err(TenderError::NegativeDue(_))
        ));
    }
}
