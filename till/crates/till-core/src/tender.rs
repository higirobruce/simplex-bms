//! Tender settlement: how a basket is paid for and what change is due.
//!
//! RWF has no circulating subunit, so this module refuses fractional amounts
//! rather than assuming they never arrive: [`TenderSet::push`] rejects a
//! non-whole tender and [`TenderSet::settle`] rejects a non-whole amount due.
//! Every figure that gets through is whole, so change is whole too and nothing
//! here rounds.

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
    /// A tender with a fractional part; RWF cannot be handed over in subunits.
    #[error("tender {0} is not a whole number of francs")]
    FractionalAmount(Money),
    /// The amount due has a fractional part; it was not rounded to whole francs.
    #[error("amount due {0} is not a whole number of francs")]
    FractionalDue(Money),
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
    /// Adds a tender, refusing anything that could not be handed over or
    /// settled. This is the entry gate: it fails at the keystroke, so a cashier
    /// sees it when keying rather than at settlement. Because the inner list is
    /// private, every entry in a set has passed it, and `settle` relies on that:
    /// every amount is positive and whole.
    ///
    /// RWF has no circulating subunit, so a fractional tender (`100.50`) would
    /// produce change that cannot be paid out.
    pub fn push(&mut self, entry: TenderEntry) -> Result<(), TenderError> {
        if entry.amount <= Money::zero() {
            return Err(TenderError::NonPositiveAmount);
        }
        if !is_whole(entry.amount) {
            return Err(TenderError::FractionalAmount(entry.amount));
        }
        self.0.push(entry);
        Ok(())
    }

    /// The entries in the order they were taken, read-only. The store writes
    /// one `sale_tender` row per entry; the list stays private so every entry
    /// in a set still arrives through [`TenderSet::push`]'s checks.
    pub fn entries(&self) -> &[TenderEntry] {
        &self.0
    }

    /// Sum of every entry. `None` if the sum passes the 12-digit bound, so an
    /// unrepresentable total is never mistaken for a plausible one. An empty
    /// set sums to zero.
    pub fn total(&self) -> Option<Money> {
        self.0
            .iter()
            .try_fold(Money::zero(), |acc, e| acc.checked_add(e.amount))
    }

    /// Sum of the entries carrying `code`. Every entry is positive, so this is
    /// a part of [`TenderSet::total`] and fits whenever the total does.
    fn code_total(&self, code: PmtTyCd) -> Option<Money> {
        self.0
            .iter()
            .filter(|e| e.pmt_ty_cd == code)
            .try_fold(Money::zero(), |acc, e| acc.checked_add(e.amount))
    }

    /// Settles a basket whose amount due is `due` (from `BasketTotals::tot_amt`).
    ///
    /// `due` is checked rather than trusted: today it is whole and non-negative
    /// by construction, but a refund path will pass amounts from elsewhere.
    ///
    /// Checks run in a fixed order so the same input always gives the same
    /// error: empty, negative due, fractional due, out-of-range sum, shortfall,
    /// then change against non-cash.
    pub fn settle(&self, due: Money) -> Result<Settled, TenderError> {
        let Some(first) = self.0.first() else {
            return Err(TenderError::Empty);
        };
        if due < Money::zero() {
            return Err(TenderError::NegativeDue(due));
        }
        if !is_whole(due) {
            return Err(TenderError::FractionalDue(due));
        }
        let taken = self.total().ok_or(TenderError::OutOfRange)?;
        if taken < due {
            return Err(TenderError::Insufficient { due, taken });
        }
        let change = taken.checked_sub(due).ok_or(TenderError::OutOfRange)?;
        if change > Money::zero() {
            // Change is paid out of the drawer, so it can be at most the cash
            // actually handed over. The sum is a part of `taken`, which fit;
            // the zero fallback is unreachable and fails safe (a refusal).
            let cash = self.code_total(PmtTyCd::Cash).unwrap_or_else(Money::zero);
            if change > cash {
                return Err(TenderError::OverTenderOnNonCash);
            }
        }
        Ok(Settled {
            change,
            header_pmt_ty_cd: self.header_code(first.pmt_ty_cd),
        })
    }

    /// The single code RRA's header can carry: how the sale was mostly paid.
    /// Amounts are summed per code first, then the largest total wins, ties to
    /// the lower wire code. Choosing the largest single entry instead would
    /// let Cash 600 + Cash 600 + Card 700 report Card although 1200 was cash.
    /// (total desc, wire code asc) is a total order over distinct codes, so
    /// the result never depends on entry order. `seed` is any code present in
    /// the set, so the result is always a code that was actually tendered.
    fn header_code(&self, seed: PmtTyCd) -> PmtTyCd {
        let mut best = (seed, self.code_total(seed).unwrap_or_else(Money::zero));
        for &code in PmtTyCd::ALL.iter() {
            // Absent codes sum to zero and can never beat a positive seed
            // total; a sum past the bound is unreachable (see `code_total`).
            let Some(sum) = self.code_total(code) else {
                continue;
            };
            if sum > best.1 || (sum == best.1 && code.wire() < best.0.wire()) {
                best = (code, sum);
            }
        }
        best.0
    }
}

/// True when `m` has no fractional part.
fn is_whole(m: Money) -> bool {
    m.amount().fract().is_zero()
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
            t.push(*x).unwrap();
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
    fn a_zero_or_negative_tender_is_refused_at_push() {
        let mut t = TenderSet::default();
        for amt in ["0", "-1"] {
            assert_eq!(
                t.push(e(PmtTyCd::Cash, amt)),
                Err(TenderError::NonPositiveAmount)
            );
        }
        assert!(matches!(t.settle(m("1")), Err(TenderError::Empty)));
    }
    #[test]
    fn a_fractional_tender_is_refused_at_push() {
        let mut t = TenderSet::default();
        assert_eq!(
            t.push(e(PmtTyCd::Cash, "100.50")),
            Err(TenderError::FractionalAmount(m("100.50")))
        );
        // a trailing .00 is whole
        assert!(t.push(e(PmtTyCd::Cash, "100.00")).is_ok());
    }
    #[test]
    fn a_fractional_due_is_refused_at_settle() {
        let t = set(&[e(PmtTyCd::Cash, "100")]);
        assert_eq!(
            t.settle(m("99.50")),
            Err(TenderError::FractionalDue(m("99.50")))
        );
    }
    #[test]
    fn whole_tender_and_whole_due_still_settle() {
        let s = set(&[e(PmtTyCd::Cash, "100.00")]).settle(m("99")).unwrap();
        assert_eq!(s.change.to_string(), "1.00");
    }
    #[test]
    fn credit_cash_is_not_cash_for_change() {
        // 03 is a split credit arrangement, not physical money: no change
        assert!(matches!(
            set(&[e(PmtTyCd::CashCredit, "5000")]).settle(m("2380")),
            Err(TenderError::OverTenderOnNonCash)
        ));
        // and it does not top up the cash available for change
        assert!(matches!(
            set(&[e(PmtTyCd::Cash, "100"), e(PmtTyCd::CashCredit, "5000")]).settle(m("2380")),
            Err(TenderError::OverTenderOnNonCash)
        ));
    }
    #[test]
    fn all_seven_codes_in_one_tender_pick_the_largest_total() {
        let codes = PmtTyCd::ALL;
        // 100, 200, ... 700: the highest code holds the largest total
        let es: Vec<TenderEntry> = codes
            .iter()
            .enumerate()
            .map(|(i, c)| TenderEntry {
                pmt_ty_cd: *c,
                amount: Money::try_from_minor((i as i64 + 1) * 10_000).unwrap(),
            })
            .collect();
        let s = set(&es).settle(m("2800")).unwrap();
        assert_eq!(s.header_pmt_ty_cd, PmtTyCd::Other);
        assert_eq!(s.change, Money::zero());
        // all seven equal: the lowest code wins
        let eq: Vec<TenderEntry> = codes
            .iter()
            .map(|c| TenderEntry {
                pmt_ty_cd: *c,
                amount: m("100"),
            })
            .collect();
        assert_eq!(
            set(&eq).settle(m("700")).unwrap().header_pmt_ty_cd,
            PmtTyCd::Cash
        );
    }
    #[test]
    fn check_order_negative_due_against_other_errors() {
        // empty beats negative due
        assert!(matches!(
            TenderSet::default().settle(m("-1")),
            Err(TenderError::Empty)
        ));
        // negative due beats fractional (-0.5 is both)
        assert!(matches!(
            set(&[e(PmtTyCd::Cash, "100")]).settle(m("-0.50")),
            Err(TenderError::NegativeDue(_))
        ));
        // negative due beats an out-of-range sum
        let t = set(&[
            e(PmtTyCd::Cash, "9999999999"),
            e(PmtTyCd::Card, "9999999999"),
        ]);
        assert!(t.total().is_none());
        assert!(matches!(
            t.settle(m("-1")),
            Err(TenderError::NegativeDue(_))
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
        let big = "9999999999";
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
        let big = "9999999999";
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
