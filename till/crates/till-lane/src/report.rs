//! The shift reports, composed as ESC/POS bytes: `SHIFT REPORT` (mid-shift,
//! changes nothing) and `SHIFT CLOSE REPORT` (printed when a shift closes).
//!
//! # These are internal shift documents, not RRA's X and Z reports
//!
//! CIS section 18 and 19 define the X and Z reports as **daily** reports with
//! mandatory content, and section 19 calls a Z "a business and accounting
//! record". **These documents are neither.** They are per **shift**: two
//! cashiers on one lane produce two of them in a day, and neither is RRA's Z.
//! They are deliberately not headed "X report", "Z report" or "FINAL", so a
//! supervisor or an auditor cannot mistake one for the other.
//!
//! ## Known gap: the CIS daily X and Z are not implemented
//!
//! The daily reports need aggregation across every shift of a trading day, and
//! taxable and tax amounts **per applicable tax rate** (CIS 18.1.10-11), which
//! `ShiftTotals` does not gather (it carries one gross and one tax figure). That
//! needs a `till-store` change and a different query, and is a separate slice.
//! Until it lands this till has no RRA daily X or Z report.
//!
//! What is printed from the CIS list is the trade name, the TIN (18.1.1) and the
//! MRC (18.1.4, from the VSDC's receipts: a shift with no confirmed sale has
//! none and says so). The CIS designation, per-rate amounts and the rest of
//! 18.1/19.1 are absent.
//!
//! # Not a receipt either
//!
//! A receipt (`receipt.rs`) is a legal instrument: SDC block, QR code, the RRA's
//! signature, registered with the VSDC. These carry **no QR, no SDC block and no
//! invoice number, and nothing about them is sent to the VSDC.** Nothing here
//! should be "completed" to look like a receipt. Each says in print that it is
//! not a tax receipt, and a test asserts the QR and SDC block are absent.
//!
//! # The two documents
//!
//! * **`SHIFT REPORT` is a snapshot.** It reads the shift's totals and changes
//!   nothing, so a supervisor may run it as often as they like.
//! * **`SHIFT CLOSE REPORT` closes the shift.** It is composed from the
//!   [`ShiftClose`] that closing returned (or recomposed from the store by
//!   `Store::last_closed_shift`), so it exists only for a shift that really
//!   closed; a second close has nothing to close and is refused.
//!
//! Money is formatted only by `fmt::rwf` (and `fmt::tax` for tax). The variance
//! keeps its sign: over and short are different problems.

use till_core::Money;
use till_store::{ShiftClose, ShiftTotals};

use crate::escpos::{Align, Esc};
use crate::fmt::{rwf, tax};
use crate::lane::LaneConfig;
use crate::receipt::{Stamp, clean, pair, pmt_label, rule, wrap};

/// Mid-shift snapshot. Reads `totals` and nothing else; prints no state change.
/// `mrc` is the VSDC's MRC if any sale of the shift was confirmed.
pub fn shift_report(
    totals: &ShiftTotals,
    cfg: &LaneConfig,
    mrc: Option<&str>,
    at: &str,
) -> Vec<u8> {
    let head = header("SHIFT REPORT", "SHIFT STILL OPEN", cfg, mrc, at);
    finish(body(head, totals))
}

/// The closing report: the figures of [`shift_report`] plus the declared cash
/// and the signed variance.
pub fn shift_close_report(
    close: &ShiftClose,
    cfg: &LaneConfig,
    mrc: Option<&str>,
    at: &str,
) -> Vec<u8> {
    compose_close(close, cfg, mrc, at, false)
}

/// The close report again, for one that already printed (or whose print status
/// is unknown, as after a restart): the same figures, marked as a copy so two
/// papers cannot pass for two closings. One that never printed is re-issued with
/// [`shift_close_report`], identical to the first attempt.
pub fn shift_close_report_copy(
    close: &ShiftClose,
    cfg: &LaneConfig,
    mrc: Option<&str>,
    at: &str,
) -> Vec<u8> {
    compose_close(close, cfg, mrc, at, true)
}

fn compose_close(
    close: &ShiftClose,
    cfg: &LaneConfig,
    mrc: Option<&str>,
    at: &str,
    copy: bool,
) -> Vec<u8> {
    let status = if copy {
        "COPY - SHIFT CLOSED"
    } else {
        "SHIFT CLOSED"
    };
    let head = header("SHIFT CLOSE REPORT", status, cfg, mrc, at);
    let e = body(head, &close.totals)
        .line(&pair("DECLARED CASH", &rwf(close.declared_cash)))
        .bold(true)
        .line(&pair(
            &format!("VARIANCE {}", verdict(close.variance)),
            &variance(close.variance),
        ))
        .bold(false);
    finish(e)
}

/// A variance with its sign: `+2,000` over, `-2,000` short, `0` exact. Never an
/// absolute value, so over and short cannot be confused.
pub fn variance(v: Money) -> String {
    if v > Money::zero() {
        format!("+{}", rwf(v))
    } else {
        rwf(v)
    }
}

fn verdict(v: Money) -> &'static str {
    if v > Money::zero() {
        "(OVER)"
    } else if v < Money::zero() {
        "(SHORT)"
    } else {
        "(EXACT)"
    }
}

fn header(title: &str, status: &str, cfg: &LaneConfig, mrc: Option<&str>, at: &str) -> Esc {
    // Printed time is the lane clock, in the receipt's own format. A malformed
    // clock must not stop a supervisor getting their figures: print it raw.
    let when = match Stamp::parse(at) {
        Ok(s) => format!("{} {}", s.date_printed(), s.time_printed()),
        Err(_) => clean(at),
    };
    // `init` + an explicit code page, as the receipt does.
    let mut e = Esc::new()
        .init()
        .code_page(0)
        .align(Align::Centre)
        .bold(true)
        .line(&clean(&cfg.shop_name))
        .double_height(true)
        .line(title)
        .double_height(false)
        .line(status)
        .bold(false)
        .line("INTERNAL SHIFT DOCUMENT")
        .line("NOT A TAX RECEIPT")
        .line("NOT AN RRA DAILY REPORT");
    for l in wrap(&clean(&cfg.shop_address)) {
        e = e.line(&l);
    }
    e.align(Align::Left)
        .line(&rule())
        .line(&pair("TIN", &clean(&cfg.shop_tin)))
        .line(&pair("MRC", &mrc_text(mrc)))
        .line(&pair("LANE", &clean(&cfg.dvc_srl_no)))
        .line(&pair("PRINTED", &when))
        .line(&rule())
}

/// The MRC, or a placeholder that says why there is none: the MRC arrives on a
/// VSDC receipt, so a shift with no confirmed sale has not seen one.
fn mrc_text(mrc: Option<&str>) -> String {
    match mrc.map(clean).filter(|m| !m.is_empty()) {
        Some(m) => m,
        None => "NONE YET (NO SALE CONFIRMED)".to_owned(),
    }
}

/// The figures both reports share. `by_code` is iterated as given: the store
/// already orders it by RRA wire code.
fn body(mut e: Esc, t: &ShiftTotals) -> Esc {
    e = e
        .line(&pair("SALES", &t.sale_count.to_string()))
        .line(&pair("VOIDED SALES", &t.voided_count.to_string()))
        .bold(true)
        .line(&pair("GROSS TOTAL", &rwf(t.gross)))
        .bold(false)
        .line(&pair("TOTAL TAX", &tax(t.tax)))
        .line(&rule())
        .line("TAKEN BY PAYMENT TYPE")
        .line("(as handed over, before change)");
    for (code, amount) in &t.by_code {
        e = e.line(&pair(
            &format!("{} {}", code.wire(), pmt_label(*code)),
            &rwf(*amount),
        ));
    }
    e.line(&rule())
        .line(&pair("OPENING FLOAT", &rwf(t.opening_float)))
        .bold(true)
        .line(&pair("EXPECTED CASH", &rwf(t.expected_cash)))
        .bold(false)
}

/// Bold, alignment and size are persistent printer state: put them back before
/// the cut or the next customer's receipt inherits them.
fn finish(e: Esc) -> Vec<u8> {
    e.line(&rule())
        .feed(4)
        .bold(false)
        .double_height(false)
        .align(Align::Left)
        .cut()
        .bytes()
}

/// The text a printer would lay out from `bytes`, printer commands skipped. For
/// tests, here and in `lane`, that assert on what a supervisor would read.
#[cfg(test)]
pub(crate) fn printable(b: &[u8]) -> String {
    let (mut out, mut i) = (String::new(), 0);
    while i < b.len() {
        match b[i] {
            0x1B => i += if b.get(i + 1) == Some(&0x40) { 2 } else { 3 },
            0x1D => match b.get(i + 1) {
                Some(0x28) => i += 5 + (b[i + 3] as usize | (b[i + 4] as usize) << 8),
                Some(0x56) => i += 4,
                _ => i += 3,
            },
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &[u8]) -> String {
        printable(b)
    }
    use till_core::codes::PmtTyCd as P;

    fn m(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    pub(crate) fn cfg() -> LaneConfig {
        LaneConfig {
            dvc_srl_no: "DVC1".into(),
            bhf_id: "00".into(),
            embedded: till_core::scan::EmbeddedMode::Weight,
            counter: till_store::Counter::SaleInvc,
            shop_name: "Simplex Mart".into(),
            shop_address: "KG 7 Ave, Kigali".into(),
            shop_tin: "123456789".into(),
        }
    }

    pub(crate) fn totals() -> ShiftTotals {
        ShiftTotals {
            sale_count: 12,
            voided_count: 1,
            gross: m("184500"),
            tax: m("28144.07"),
            by_code: vec![
                (P::Cash, m("120000")),
                (P::Card, m("50500")),
                (P::MobileMoney, m("14000")),
            ],
            opening_float: m("20000"),
            expected_cash: m("135000"),
        }
    }

    fn close(declared: &str, variance: &str) -> ShiftClose {
        ShiftClose {
            totals: totals(),
            declared_cash: m(declared),
            variance: m(variance),
        }
    }

    const AT: &str = "20261121183005";
    const MRC: &str = "WIS00000001";

    #[test]
    fn variance_keeps_its_sign() {
        assert_eq!(variance(m("-2000")), "-2,000");
        assert_eq!(variance(m("2000")), "+2,000");
        assert_eq!(variance(m("0")), "0");
        let t = text(&shift_close_report(
            &close("133000", "-2000"),
            &cfg(),
            Some(MRC),
            AT,
        ));
        assert!(
            t.contains("VARIANCE (SHORT)") && t.contains("-2,000"),
            "{t}"
        );
        let t = text(&shift_close_report(
            &close("137000", "2000"),
            &cfg(),
            Some(MRC),
            AT,
        ));
        assert!(t.contains("VARIANCE (OVER)") && t.contains("+2,000"), "{t}");
    }

    #[test]
    fn tenders_print_in_the_order_the_store_gave() {
        let t = text(&shift_report(&totals(), &cfg(), Some(MRC), AT));
        let at = |s: &str| t.find(s).unwrap_or_else(|| panic!("{s} in\n{t}"));
        assert!(at("01 CASH") < at("05 CARD") && at("05 CARD") < at("06 MOBILE MONEY"));
    }

    #[test]
    fn the_reports_say_which_they_are_and_claim_to_be_no_rra_report() {
        let x = text(&shift_report(&totals(), &cfg(), Some(MRC), AT));
        let z = text(&shift_close_report(
            &close("135000", "0"),
            &cfg(),
            Some(MRC),
            AT,
        ));
        assert!(
            x.contains("SHIFT REPORT") && x.contains("SHIFT STILL OPEN") && !x.contains("DECLARED")
        );
        assert!(z.contains("SHIFT CLOSE REPORT") && z.contains("DECLARED CASH"));
        for t in [&x, &z] {
            // Not RRA's X or Z, and not "final" in the RRA sense.
            for banned in ["X REPORT", "Z REPORT", "FINAL"] {
                assert!(!t.contains(banned), "{banned} in\n{t}");
            }
            assert!(t.contains("NOT A TAX RECEIPT") && t.contains("NOT AN RRA DAILY REPORT"));
            assert!(t.contains("21/11/2026 18:30:05"), "{t}");
            assert!(t.contains("28144.07"), "tax keeps two decimals: {t}");
            assert!(t.contains("184,500"));
        }
    }

    #[test]
    fn the_trade_name_tin_and_mrc_are_printed() {
        for b in [
            shift_report(&totals(), &cfg(), Some(MRC), AT),
            shift_close_report(&close("1", "0"), &cfg(), Some(MRC), AT),
        ] {
            let t = text(&b);
            assert!(t.contains("Simplex Mart"), "{t}");
            assert!(t.contains("TIN") && t.contains("123456789"), "{t}");
            assert!(t.contains("MRC") && t.contains(MRC), "{t}");
        }
    }

    #[test]
    fn a_shift_with_no_confirmed_sale_prints_a_clear_mrc_placeholder() {
        for mrc in [None, Some("  ")] {
            let t = text(&shift_report(&totals(), &cfg(), mrc, AT));
            assert!(
                t.contains("MRC") && t.contains("NONE YET (NO SALE CONFIRMED)"),
                "{t}"
            );
        }
    }

    #[test]
    fn a_copy_is_marked_and_otherwise_identical() {
        let c = close("133000", "-2000");
        let (z, copy) = (
            text(&shift_close_report(&c, &cfg(), Some(MRC), AT)),
            text(&shift_close_report_copy(&c, &cfg(), Some(MRC), AT)),
        );
        assert!(copy.contains("COPY") && !z.contains("COPY"));
        assert_eq!(
            shift_close_report(&c, &cfg(), Some(MRC), AT),
            shift_close_report(&c, &cfg(), Some(MRC), AT)
        );
        assert_eq!(copy.replace("COPY - ", ""), z);
    }

    #[test]
    fn no_line_exceeds_the_paper_width() {
        for b in [
            shift_report(&totals(), &cfg(), Some(MRC), AT),
            shift_close_report(&close("1", "-134999"), &cfg(), Some(MRC), AT),
        ] {
            for l in text(&b).lines() {
                assert!(l.chars().count() <= crate::receipt::WIDTH, "{l:?}");
            }
        }
    }

    #[test]
    fn a_malformed_clock_still_prints_the_figures() {
        let t = text(&shift_report(&totals(), &cfg(), Some(MRC), "nonsense"));
        assert!(t.contains("nonsense") && t.contains("184,500"));
    }

    #[test]
    fn printer_state_is_reset_immediately_before_the_cut() {
        let tail = [
            &[0x1B, 0x45, 0x00][..], // bold off
            &[0x1D, 0x21, 0x00],     // size normal
            &[0x1B, 0x61, 0x00],     // left
            &[0x1D, 0x56, 0x42, 0x00],
        ]
        .concat();
        assert!(shift_report(&totals(), &cfg(), Some(MRC), AT).ends_with(&tail));
        assert!(shift_close_report(&close("1", "0"), &cfg(), Some(MRC), AT).ends_with(&tail));
        assert_eq!(
            shift_report(&totals(), &cfg(), Some(MRC), AT)[..5],
            [0x1B, 0x40, 0x1B, 0x74, 0x00]
        );
    }

    /// Not an assertion: `cargo test -- --nocapture rendered` shows the layout.
    #[test]
    fn rendered_examples() {
        println!("{}", text(&shift_report(&totals(), &cfg(), Some(MRC), AT)));
        println!(
            "{}",
            text(&shift_close_report(
                &close("133000", "-2000"),
                &cfg(),
                Some(MRC),
                AT
            ))
        );
    }
}
