//! The X and Z shift reports, composed as ESC/POS bytes.
//!
//! # These are not fiscal documents
//!
//! A receipt (`receipt.rs`) is a legal instrument: it carries the SDC block, a
//! QR code and the RRA's receipt signature, and every one is registered with the
//! VSDC. **These reports are none of that.** They are internal shop documents
//! for reconciling a drawer. They carry **no QR, no SDC block and no invoice
//! number, and nothing about them is sent to the VSDC.** So the receipt rules
//! (CIS layout, C5) do not apply, and nothing here should be "completed" to look
//! like a receipt: a report that resembled one could be mistaken for a tax
//! document. Each report says in print that it is not a tax receipt, and a test
//! asserts the QR and SDC block are absent.
//!
//! # X and Z
//!
//! * **X is a snapshot.** It reads the shift's totals and changes nothing, so a
//!   supervisor may run it as often as they like, mid-shift, mid-sale.
//! * **Z closes the shift and is final.** It is composed from the
//!   [`ShiftClose`] that closing returned, so it is printed only for a shift that
//!   really closed; the lane then has no open shift, so a second Z has nothing to
//!   close and is refused. One trading day therefore has one final figure.
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
pub fn x_report(totals: &ShiftTotals, cfg: &LaneConfig, at: &str) -> Vec<u8> {
    let head = header("X REPORT", "SHIFT STILL OPEN - NOT FINAL", cfg, at);
    finish(body(head, totals))
}

/// The closing report: the figures of [`x_report`] plus the declared cash and
/// the signed variance. Final.
pub fn z_report(close: &ShiftClose, cfg: &LaneConfig, at: &str) -> Vec<u8> {
    compose_z(close, cfg, at, false)
}

/// The Z again, for a Z that already printed once: the same figures and clock,
/// marked as a copy so two papers cannot pass for two "final" closings. A Z that
/// never printed is re-issued with [`z_report`], identical to the first attempt.
pub fn z_report_copy(close: &ShiftClose, cfg: &LaneConfig, at: &str) -> Vec<u8> {
    compose_z(close, cfg, at, true)
}

fn compose_z(close: &ShiftClose, cfg: &LaneConfig, at: &str, copy: bool) -> Vec<u8> {
    let status = if copy {
        "COPY - SHIFT CLOSED - FINAL"
    } else {
        "SHIFT CLOSED - FINAL"
    };
    let head = header("Z REPORT", status, cfg, at);
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

fn header(title: &str, status: &str, cfg: &LaneConfig, at: &str) -> Esc {
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
        .line("INTERNAL DOCUMENT - NOT A TAX RECEIPT");
    for l in wrap(&clean(&cfg.shop_address)) {
        e = e.line(&l);
    }
    e.align(Align::Left)
        .line(&rule())
        .line(&pair("LANE", &clean(&cfg.dvc_srl_no)))
        .line(&pair("PRINTED", &when))
        .line(&rule())
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

    #[test]
    fn variance_keeps_its_sign() {
        assert_eq!(variance(m("-2000")), "-2,000");
        assert_eq!(variance(m("2000")), "+2,000");
        assert_eq!(variance(m("0")), "0");
        let t = text(&z_report(&close("133000", "-2000"), &cfg(), AT));
        assert!(
            t.contains("VARIANCE (SHORT)") && t.contains("-2,000"),
            "{t}"
        );
        let t = text(&z_report(&close("137000", "2000"), &cfg(), AT));
        assert!(t.contains("VARIANCE (OVER)") && t.contains("+2,000"), "{t}");
    }

    #[test]
    fn tenders_print_in_the_order_the_store_gave() {
        let t = text(&x_report(&totals(), &cfg(), AT));
        let at = |s: &str| t.find(s).unwrap_or_else(|| panic!("{s} in\n{t}"));
        assert!(at("01 CASH") < at("05 CARD") && at("05 CARD") < at("06 MOBILE MONEY"));
    }

    #[test]
    fn x_and_z_say_which_they_are() {
        let x = text(&x_report(&totals(), &cfg(), AT));
        let z = text(&z_report(&close("135000", "0"), &cfg(), AT));
        assert!(x.contains("X REPORT") && x.contains("NOT FINAL") && !x.contains("DECLARED"));
        assert!(z.contains("Z REPORT") && z.contains("FINAL") && z.contains("DECLARED CASH"));
        assert!(!z.contains("NOT FINAL"));
        for t in [&x, &z] {
            assert!(t.contains("NOT A TAX RECEIPT"));
            assert!(t.contains("21/11/2026 18:30:05"), "{t}");
            assert!(t.contains("28144.07"), "tax keeps two decimals: {t}");
            assert!(t.contains("184,500"));
        }
    }

    #[test]
    fn a_copy_is_marked_and_otherwise_identical() {
        let c = close("133000", "-2000");
        let (z, copy) = (
            text(&z_report(&c, &cfg(), AT)),
            text(&z_report_copy(&c, &cfg(), AT)),
        );
        assert!(copy.contains("COPY") && !z.contains("COPY"));
        assert_eq!(z_report(&c, &cfg(), AT), z_report(&c, &cfg(), AT));
        assert_eq!(copy.replace("COPY - ", ""), z);
    }

    #[test]
    fn no_line_exceeds_the_paper_width() {
        for b in [
            x_report(&totals(), &cfg(), AT),
            z_report(&close("1", "-134999"), &cfg(), AT),
        ] {
            for l in text(&b).lines() {
                assert!(l.chars().count() <= crate::receipt::WIDTH, "{l:?}");
            }
        }
    }

    #[test]
    fn a_malformed_clock_still_prints_the_figures() {
        let t = text(&x_report(&totals(), &cfg(), "nonsense"));
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
        assert!(x_report(&totals(), &cfg(), AT).ends_with(&tail));
        assert!(z_report(&close("1", "0"), &cfg(), AT).ends_with(&tail));
        assert_eq!(
            x_report(&totals(), &cfg(), AT)[..5],
            [0x1B, 0x40, 0x1B, 0x74, 0x00]
        );
    }

    /// Not an assertion: `cargo test -- --nocapture rendered` shows the layout.
    #[test]
    fn rendered_examples() {
        println!("{}", text(&x_report(&totals(), &cfg(), AT)));
        println!("{}", text(&z_report(&close("133000", "-2000"), &cfg(), AT)));
    }
}
