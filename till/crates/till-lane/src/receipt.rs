//! The printed tax receipt, composed as ESC/POS bytes.
//!
//! The layout is mandated by the Rwanda Revenue Authority (CIS specification,
//! `docs/reference/rra/cis.txt`), so a missing field is a legal failure, not a
//! cosmetic one. Where this module has to choose between the spec's prose and
//! its sample receipts, the prose wins and the choice is noted at the site.
//!
//! Two rules shape everything below:
//!
//! * **Every figure comes from the types that already hold it.** The header
//!   tax is `BasketTotals::tot_tax_amt`, the sum of the per-line taxes. It is
//!   never recomputed from a band total: each line's tax is rounded on its own,
//!   so `18/118` of the band total can differ from that sum by a cent, and the
//!   figure on paper must equal the figure sent to the RRA.
//! * **A receipt that cannot be valid is not printed.** [`try_compose`] returns
//!   an error instead of a receipt with a blank QR, a garbled timestamp or a
//!   missing refund reference. [`compose`] is the infallible form the brief
//!   asks for and panics on those same conditions.

use rust_decimal::Decimal;
use till_core::codes::{PmtTyCd, RcptTyCd, SalesTyCd, TaxType};
use till_core::{BasketLine, BasketTotals, LineAmounts, Money, Settled, TenderSet};
use till_store::VsdcReceipt;

use crate::escpos::{Align, Esc};
use crate::fmt::{qty, rwf, tax};

/// Columns on an 80mm printer in the default font.
const WIDTH: usize = 42;

/// Everything a receipt prints.
///
/// `sales_ty` is not in the original brief: the brief's `rcpt_ty: RcptTyCd`
/// cannot express COPY, TRAINING or PROFORMA, because `RcptTyCd` is only
/// Sale/Refund (VSDC 4.9). Those are `SalesTyCd` (VSDC 4.8). The receipt label
/// (`NS`, `CR`, `PS`, ...) is the two codes together (CIS section 7.25).
pub struct ReceiptInput<'a> {
    pub shop_name: &'a str,
    pub shop_address: &'a str,
    pub shop_tin: &'a str,
    pub mrc: &'a str,
    pub sales_ty: SalesTyCd,
    pub rcpt_ty: RcptTyCd,
    pub lines: &'a [(BasketLine, LineAmounts)],
    pub totals: &'a BasketTotals,
    pub tenders: &'a TenderSet,
    pub settled: &'a Settled,
    pub invc_no: i64,
    pub vsdc: &'a VsdcReceipt,
}

/// Why a receipt was refused rather than printed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    /// A field of the QR payload is empty or contains `#`, so the payload
    /// would not split back into its six fields.
    #[error("QR field {0} is empty or contains '#'")]
    BadQrField(&'static str),
    /// The VSDC timestamp is not a real `yyyymmddhhmmss` instant.
    #[error("VSDC timestamp is not yyyymmddhhmmss: {0:?}")]
    BadTimestamp(String),
    /// Refund receipts must name the receipt they refund (CIS section 14), and
    /// the input has nowhere to carry that number yet.
    #[error(
        "a refund receipt needs the referenced receipt number, which ReceiptInput cannot carry"
    )]
    RefundReferenceMissing,
    /// The printed lines and the printed item count would disagree.
    #[error("{lines} lines supplied but the totals count {counted} items")]
    ItemCountMismatch { lines: usize, counted: u32 },
}

/// Inserts a dash after every fourth character (CIS 7.24.5 and 7.24.6). The
/// final group keeps whatever is left over; it is never padded.
pub fn dash_every_four(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// The receipt as printer bytes; panics where [`try_compose`] errs. A till that
/// reaches this with an invalid VSDC response has a bug upstream, and printing
/// anything would be worse than stopping.
pub fn compose(input: ReceiptInput<'_>) -> Vec<u8> {
    match try_compose(input) {
        Ok(bytes) => bytes,
        Err(e) => panic!("refusing to print an invalid receipt: {e}"),
    }
}

/// Composes the receipt, or says why it cannot be a valid one.
pub fn try_compose(input: ReceiptInput<'_>) -> Result<Vec<u8>, ReceiptError> {
    let ReceiptInput {
        shop_name,
        shop_address,
        shop_tin,
        mrc,
        sales_ty,
        rcpt_ty,
        lines,
        totals,
        tenders,
        settled,
        invc_no,
        vsdc,
    } = input;

    if rcpt_ty == RcptTyCd::Refund {
        return Err(ReceiptError::RefundReferenceMissing);
    }
    if u32::try_from(lines.len()).ok() != Some(totals.tot_item_cnt) {
        return Err(ReceiptError::ItemCountMismatch {
            lines: lines.len(),
            counted: totals.tot_item_cnt,
        });
    }
    let stamp = Stamp::parse(&vsdc.vsdc_rcpt_pbct_date)?;
    let label = format!("{}{}", sales_ty.wire(), rcpt_ty.wire());

    // Training and proforma receipts carry no digital signature (CIS sections
    // 16 and 17), so they print neither the signature block nor a QR: a QR on a
    // receipt that is not official would invite a customer to verify it.
    let signed = matches!(sales_ty, SalesTyCd::Normal | SalesTyCd::Copy);
    let qr = if signed {
        Some(qr_payload(&stamp, vsdc)?)
    } else {
        None
    };
    let banner = match sales_ty {
        SalesTyCd::Normal => None,
        SalesTyCd::Copy => Some("COPY"),
        SalesTyCd::Training => Some("TRAINING"),
        SalesTyCd::Proforma => Some("PROFORMA"),
    };

    // `init` is a bare reset: without an explicit code page the printer uses
    // whatever it powered up in and product names print as garbage.
    let mut e = Esc::new().init().code_page(0);

    // Header: trade name, address, TIN.
    e = e
        .align(Align::Centre)
        .bold(true)
        .line(&clean(shop_name))
        .bold(false);
    for l in wrap(&clean(shop_address)) {
        e = e.line(&l);
    }
    e = e.line(&format!("TIN: {}", clean(shop_tin)));
    // CIS 11: the designation sits below the header and above the items.
    if let Some(b) = banner {
        e = banner_lines(e, b);
    }
    e = e.align(Align::Left).line(&rule());

    // Items: name, then `price x qty  amount BAND`, then any discount.
    for (line, amt) in lines {
        for l in wrap(&clean(&line.item_nm)) {
            e = e.line(&l);
        }
        let band = line.tax_ty_cd.wire();
        e = e.line(&item_row(
            &format!("{}x", rwf(line.prc)),
            &qty(line.qty),
            &format!("{}{band}", rwf(amt.sply_amt)),
        ));
        if amt.dc_amt > Money::zero() {
            e = e.line(&pair(
                &format!("discount -{}%", amt.dc_rt.normalize()),
                &rwf(amt.tot_amt),
            ));
        }
    }
    e = e.line(&rule());

    // Totals. Each band shows the figures `BasketTotals` carries; the tax rows
    // are the sums of the line taxes and are never recomputed here.
    e = e
        .bold(true)
        .line(&pair("TOTAL", &rwf(totals.tot_amt)))
        .bold(false);
    for &band in TaxType::ALL {
        let (taxbl, rate, tax_amt) = totals.by_band(band);
        let used = lines.iter().any(|(l, _)| l.tax_ty_cd == band);
        // CIS 7.22: every rate above zero prints on every receipt. 7.23: a zero
        // rate prints only when an item of that band was sold.
        if rate > Decimal::ZERO || used {
            e = e.line(&pair(
                &format!("TOTAL {}", band_label(band, rate)),
                &rwf(taxbl),
            ));
        }
        if rate > Decimal::ZERO {
            e = e.line(&pair(&format!("TOTAL TAX {}", band.wire()), &tax(tax_amt)));
        }
    }
    e = e.line(&pair("TOTAL TAX", &tax(totals.tot_tax_amt)));
    // CIS 11: the warning goes below the amount totals.
    if banner.is_some() {
        e = e
            .bold(true)
            .line("THIS IS NOT AN OFFICIAL RECEIPT")
            .bold(false);
    }
    e = e.line(&rule());

    // Payment.
    for t in tenders.entries() {
        e = e.line(&pair(pmt_label(t.pmt_ty_cd), &rwf(t.amount)));
    }
    if settled.change > Money::zero() {
        e = e.line(&pair("CHANGE", &rwf(settled.change)));
    }
    e = e
        .line(&pair("ITEMS NUMBER", &totals.tot_item_cnt.to_string()))
        .line(&rule());

    // SDC INFORMATION (CIS 7.24). Non-normal receipts repeat their designation
    // above this block, as the specification's samples do.
    if let Some(b) = banner {
        e = banner_lines(e.align(Align::Centre), b)
            .align(Align::Left)
            .line(&rule());
    }
    e = e
        .align(Align::Centre)
        .line("SDC INFORMATION")
        .align(Align::Left)
        .line(&pair(
            &format!("Date: {}", stamp.date_printed()),
            &format!("Time: {}", stamp.time_printed()),
        ))
        .line(&pair("SDC ID:", &clean(&vsdc.sdc_id)))
        // 7.25: A = counter per receipt type, B = total counter, RT = label.
        .line(&pair(
            "RECEIPT NUMBER:",
            &format!("{}/{} {}", vsdc.rcpt_no, vsdc.tot_rcpt_no, label),
        ));
    if signed {
        e = e
            .align(Align::Centre)
            .line("Internal Data:")
            .line(&dash_every_four(&clean(&vsdc.intrl_data)))
            .line("Receipt Signature:")
            .line(&dash_every_four(&clean(&vsdc.rcpt_sign)));
        if let Some(payload) = &qr {
            // Never emit `qr("")`: it is accepted and prints nothing.
            e = e.feed(1).qr(payload).feed(1);
        }
        e = e.align(Align::Left);
    }
    e = e.line(&rule());

    // The CIS's own numbering and the machine registration code.
    e = e
        .line(&pair("RECEIPT NUMBER:", &invc_no.to_string()))
        .line(&pair("MRC:", &clean(mrc)))
        .line(&rule())
        .align(Align::Centre)
        .line("THANK YOU")
        .line("COME BACK AGAIN");

    // Bold, alignment and size are persistent printer state: put them back
    // before the cut or the next receipt inherits them.
    Ok(e.feed(4)
        .bold(false)
        .double_height(false)
        .align(Align::Left)
        .cut()
        .bytes())
}

/// `invoice_date(ddmmyyyy)#time(hhmmss)#sdc number#sdc receipt number#internal
/// data#receipt signature` (CIS 7.24.7). Internal data and signature go in raw:
/// the dashes are a print convenience, not part of the data.
///
/// "sdc number" is read as the SDC ID and "sdc receipt number" as `rcpt_no`
/// (the per-type counter, `A` of `A/B`); the specification does not say which of
/// the VSDC's two counters it means.
fn qr_payload(stamp: &Stamp, v: &VsdcReceipt) -> Result<String, ReceiptError> {
    let rcpt_no = v.rcpt_no.to_string();
    let fields: [(&'static str, String); 6] = [
        ("invoice_date", stamp.ddmmyyyy()),
        ("time", stamp.hhmmss()),
        ("sdc_number", v.sdc_id.clone()),
        ("sdc_receipt_number", rcpt_no),
        ("internal_data", v.intrl_data.clone()),
        ("receipt_signature", v.rcpt_sign.clone()),
    ];
    for (name, value) in &fields {
        if value.is_empty() || value.contains('#') {
            return Err(ReceiptError::BadQrField(name));
        }
    }
    Ok(fields.map(|(_, v)| v).join("#"))
}

/// The VSDC's receipt instant, `yyyymmddhhmmss` (VSDC sample responses).
struct Stamp {
    y: u32,
    mo: u32,
    d: u32,
    h: u32,
    mi: u32,
    s: u32,
}

impl Stamp {
    /// Non-digits are ignored, so the compact form and an ISO form both parse;
    /// anything that is not 14 digits forming a real clock time is refused.
    fn parse(raw: &str) -> Result<Stamp, ReceiptError> {
        let bad = || ReceiptError::BadTimestamp(raw.to_owned());
        let digits: Vec<u32> = raw.chars().filter_map(|c| c.to_digit(10)).collect();
        if digits.len() != 14 {
            return Err(bad());
        }
        let num = |a: usize, b: usize| {
            digits
                .get(a..b)
                .map(|d| d.iter().fold(0, |acc, x| acc * 10 + x))
        };
        let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) = (
            num(0, 4),
            num(4, 6),
            num(6, 8),
            num(8, 10),
            num(10, 12),
            num(12, 14),
        ) else {
            return Err(bad());
        };
        let ok = (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && s < 60;
        if ok {
            Ok(Stamp { y, mo, d, h, mi, s })
        } else {
            Err(bad())
        }
    }

    /// The sample receipt prints `25/5/2012`: day and month are not padded.
    fn date_printed(&self) -> String {
        format!("{}/{}/{}", self.d, self.mo, self.y)
    }
    fn time_printed(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.h, self.mi, self.s)
    }
    /// Fixed width, so zero-padded.
    fn ddmmyyyy(&self) -> String {
        format!("{:02}{:02}{:04}", self.d, self.mo, self.y)
    }
    fn hhmmss(&self) -> String {
        format!("{:02}{:02}{:02}", self.h, self.mi, self.s)
    }
}

/// Doubled-height, bold designation, centred; leaves size and weight reset.
fn banner_lines(e: Esc, text: &str) -> Esc {
    e.align(Align::Centre)
        .bold(true)
        .double_height(true)
        .line(text)
        .double_height(false)
        .bold(false)
}

fn rule() -> String {
    "-".repeat(WIDTH)
}

fn band_label(band: TaxType, rate: Decimal) -> String {
    match band {
        TaxType::A => "A-EX".to_owned(),
        other => format!("{}-{:.2}%", other.wire(), rate),
    }
}

fn pmt_label(code: PmtTyCd) -> &'static str {
    match code {
        PmtTyCd::Cash => "CASH",
        PmtTyCd::Credit => "CREDIT",
        PmtTyCd::CashCredit => "CASH/CREDIT",
        PmtTyCd::BankCheck => "BANK CHECK",
        PmtTyCd::Card => "CARD",
        PmtTyCd::MobileMoney => "MOBILE MONEY",
        PmtTyCd::Other => "OTHER",
    }
}

/// Free text as it may reach the printer. The encoder prints `\r` and `\t` as
/// `?` and `\n` as a line break that would wreck the layout, so every control
/// character becomes a space.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Breaks text into `WIDTH`-column lines. One byte is one column in CP437.
fn wrap(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars.chunks(WIDTH).map(|c| c.iter().collect()).collect()
}

/// `left` flush left, `right` flush right, in `WIDTH` columns. If both do not
/// fit, `left` is cut: the figure on the right must never be.
fn pair(left: &str, right: &str) -> String {
    let room = WIDTH.saturating_sub(right.chars().count() + 1);
    let left: String = left.chars().take(room).collect();
    let gap = WIDTH.saturating_sub(left.chars().count() + right.chars().count());
    format!("{left}{}{right}", " ".repeat(gap))
}

/// `price x`, quantity ending at column 26, amount flush right.
fn item_row(price: &str, quantity: &str, amount: &str) -> String {
    let mid_end = 26usize.max(price.chars().count() + quantity.chars().count());
    let head = format!("{price}{quantity:>w$}", w = mid_end - price.chars().count());
    pair(&head, amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use till_core::{Basket, LineDiscount, Qty, TenderEntry};

    fn line(name: &str, q: &str, prc: &str, band: TaxType) -> BasketLine {
        BasketLine {
            item_seq: 0,
            item_cd: "RW1".into(),
            item_cls_cd: "5020230602".into(),
            item_nm: name.into(),
            bcd: None,
            qty: Qty::try_from_str(q).unwrap(),
            prc: Money::try_from_str(prc).unwrap(),
            tax_ty_cd: band,
            discount: None,
        }
    }

    fn vsdc() -> VsdcReceipt {
        VsdcReceipt {
            rcpt_no: 168,
            intrl_data: "TE68SLA234J5EAV3N56988LJQ7".into(),
            rcpt_sign: "V249J39CFJ48HE2W".into(),
            sdc_id: "SDC001000001".into(),
            mrc_no: "AAACC123456".into(),
            vsdc_rcpt_pbct_date: "20120525110735".into(),
            tot_rcpt_no: 258,
        }
    }

    struct Fixture {
        lines: Vec<(BasketLine, LineAmounts)>,
        totals: BasketTotals,
        tenders: TenderSet,
        settled: Settled,
        vsdc: VsdcReceipt,
    }

    impl Fixture {
        fn new(lines: Vec<BasketLine>, paid: &str) -> Fixture {
            let mut basket = Basket::new();
            for l in lines {
                basket.push_line(l).unwrap();
            }
            let totals = basket.totals().unwrap();
            let mut tenders = TenderSet::default();
            tenders
                .push(TenderEntry {
                    pmt_ty_cd: PmtTyCd::Cash,
                    amount: Money::try_from_str(paid).unwrap(),
                })
                .unwrap();
            let settled = tenders.settle(totals.tot_amt).unwrap();
            Fixture {
                lines: basket.apportioned_lines().unwrap(),
                totals,
                tenders,
                settled,
                vsdc: vsdc(),
            }
        }

        fn input(&self, sales_ty: SalesTyCd, rcpt_ty: RcptTyCd) -> ReceiptInput<'_> {
            ReceiptInput {
                shop_name: "Simplex Mart",
                shop_address: "KN 4 Ave, Kigali",
                shop_tin: "123456789",
                mrc: "AAACC123456",
                sales_ty,
                rcpt_ty,
                lines: &self.lines,
                totals: &self.totals,
                tenders: &self.tenders,
                settled: &self.settled,
                invc_no: 152,
                vsdc: &self.vsdc,
            }
        }

        fn normal(&self) -> Vec<u8> {
            compose(self.input(SalesTyCd::Normal, RcptTyCd::Sale))
        }
    }

    fn mixed() -> Fixture {
        Fixture::new(
            vec![
                line("Plain Bread", "1", "1000", TaxType::A),
                line("Gouda cheese", "0.200", "33600", TaxType::B),
                line("Wriggly gum", "5", "60", TaxType::B),
            ],
            "10000",
        )
    }

    fn band_b_only() -> Fixture {
        Fixture::new(
            vec![
                line("Rice 5kg", "1", "2400", TaxType::B),
                line("Sugar 1kg", "1", "1500", TaxType::B),
                line("Oil 1L", "1", "1350", TaxType::B),
                line("Soap", "1", "1200", TaxType::B),
            ],
            "7000",
        )
    }

    /// Walks the byte stream the way a printer would: returns the printable
    /// text (LF kept) and the payload of every QR store command.
    fn decode(b: &[u8]) -> (String, Vec<String>) {
        let (mut text, mut qrs, mut i) = (String::new(), Vec::new(), 0);
        while i < b.len() {
            match b[i] {
                0x1B => {
                    i += match b.get(i + 1) {
                        Some(0x40) => 2,
                        _ => 3,
                    }
                }
                0x1D => match b.get(i + 1) {
                    Some(0x28) => {
                        let len = b[i + 3] as usize | (b[i + 4] as usize) << 8;
                        if b[i + 6] == 0x50 {
                            let data = &b[i + 8..i + 3 + len + 2];
                            qrs.push(String::from_utf8(data.to_vec()).unwrap());
                        }
                        i += 5 + len;
                    }
                    Some(0x56) => i += 4,
                    _ => i += 3,
                },
                0x0A => {
                    text.push('\n');
                    i += 1;
                }
                c => {
                    text.push(c as char);
                    i += 1;
                }
            }
        }
        (text, qrs)
    }

    fn text(b: &[u8]) -> String {
        decode(b).0
    }

    #[test]
    fn internal_data_is_dashed_every_four_characters() {
        assert_eq!(
            dash_every_four("A4F29C0177BD2E5018AA3C7F"),
            "A4F2-9C01-77BD-2E50-18AA-3C7F"
        );
        assert_eq!(dash_every_four("ABC"), "ABC");
        assert_eq!(dash_every_four("ABCD"), "ABCD");
        assert_eq!(dash_every_four("ABCDE"), "ABCD-E");
        assert_eq!(dash_every_four(""), "");
        // The CIS sample leaves a two-character final group.
        assert_eq!(
            dash_every_four("TE68SLA234J5EAV3N56988LJQ7"),
            "TE68-SLA2-34J5-EAV3-N569-88LJ-Q7"
        );
    }

    #[test]
    fn the_receipt_carries_every_mandated_sdc_field() {
        let t = text(&mixed().normal());
        for needle in [
            "SDC INFORMATION",
            "Date: 25/5/2012",
            "Time: 11:07:35",
            "SDC ID:",
            "SDC001000001",
            "RECEIPT NUMBER:",
            "168/258 NS",
            "Internal Data:",
            "TE68-SLA2-34J5-EAV3-N569-88LJ-Q7",
            "Receipt Signature:",
            "V249-J39C-FJ48-HE2W",
            "MRC:",
            "AAACC123456",
            "TIN: 123456789",
            "ITEMS NUMBER",
        ] {
            assert!(t.contains(needle), "missing {needle:?} in:\n{t}");
        }
        // The CIS's own invoice number is its own line, distinct from the VSDC's.
        let cis = t.lines().find(|l| l.ends_with("152")).expect("CIS number");
        assert!(cis.starts_with("RECEIPT NUMBER:"), "{cis:?}");
    }

    #[test]
    fn the_qr_payload_is_the_six_fields_joined_by_hashes() {
        let (_, qrs) = decode(&mixed().normal());
        assert_eq!(qrs.len(), 1);
        let parts: Vec<&str> = qrs[0].split('#').collect();
        assert_eq!(parts.len(), 6, "{}", qrs[0]);
        // ddmmyyyy, not the compact yyyymmdd the VSDC sends.
        assert_eq!(parts[0], "25052012");
        assert_eq!(parts[1], "110735");
        assert_eq!(parts[2], "SDC001000001");
        assert_eq!(parts[3], "168");
        assert_eq!(parts[4], "TE68SLA234J5EAV3N56988LJQ7");
        assert_eq!(parts[5], "V249J39CFJ48HE2W");
    }

    #[test]
    fn the_qr_date_is_zero_padded_though_the_printed_date_is_not() {
        let mut f = mixed();
        f.vsdc.vsdc_rcpt_pbct_date = "20260305090702".into();
        let bytes = f.normal();
        let (t, qrs) = decode(&bytes);
        assert!(t.contains("Date: 5/3/2026") && t.contains("Time: 09:07:02"));
        assert!(qrs[0].starts_with("05032026#090702#"));
    }

    #[test]
    fn header_tax_printed_is_the_sum_of_lines_not_a_recomputation() {
        let f = band_b_only();
        // Premise: the band total is 6,450 and its 18/118 would be 983.90.
        assert_eq!(f.totals.tot_amt, Money::try_from_str("6450").unwrap());
        let t = text(&f.normal());
        assert!(t.contains("983.89"), "{t}");
        assert!(!t.contains("983.90"), "{t}");
        let total_tax = t.lines().find(|l| l.starts_with("TOTAL TAX ")).unwrap();
        assert!(total_tax.ends_with("983.89"));
    }

    #[test]
    fn copy_training_and_proforma_are_visually_distinguishable_from_normal() {
        let f = mixed();
        let normal = text(&f.normal());
        for word in ["COPY", "TRAINING", "PROFORMA", "NOT AN OFFICIAL RECEIPT"] {
            assert!(!normal.contains(word), "normal receipt shows {word}");
        }
        for (ty, banner, label) in [
            (SalesTyCd::Copy, "COPY", "CS"),
            (SalesTyCd::Training, "TRAINING", "TS"),
            (SalesTyCd::Proforma, "PROFORMA", "PS"),
        ] {
            let bytes = compose(f.input(ty, RcptTyCd::Sale));
            let t = text(&bytes);
            assert!(t.contains(banner), "{banner}");
            assert!(t.contains("THIS IS NOT AN OFFICIAL RECEIPT"));
            assert!(t.contains(&format!("168/258 {label}")), "{t}");
            // Header banner precedes the items; the notice follows the totals.
            let at = |s: &str| t.find(s).unwrap();
            assert!(at(banner) < at("Plain Bread"));
            assert!(at("TOTAL TAX") < at("THIS IS NOT AN OFFICIAL RECEIPT"));
            // Doubled height is switched on for the banner.
            assert!(
                bytes.windows(3).any(|w| w == [0x1D, 0x21, 0x01]),
                "{banner} not doubled"
            );
        }
        // A normal receipt never doubles anything.
        assert!(!f.normal().windows(3).any(|w| w == [0x1D, 0x21, 0x01]));
    }

    #[test]
    fn training_and_proforma_carry_no_signature_or_qr_but_copy_does() {
        let f = mixed();
        for ty in [SalesTyCd::Training, SalesTyCd::Proforma] {
            let (t, qrs) = decode(&compose(f.input(ty, RcptTyCd::Sale)));
            assert!(qrs.is_empty() && !t.contains("Internal Data:"));
        }
        let (t, qrs) = decode(&compose(f.input(SalesTyCd::Copy, RcptTyCd::Sale)));
        assert_eq!(qrs.len(), 1);
        assert!(t.contains("Receipt Signature:"));
    }

    #[test]
    fn a_receipt_that_would_have_no_valid_qr_is_refused() {
        for (edit, field) in [
            (
                (|v: &mut VsdcReceipt| v.intrl_data.clear()) as fn(&mut VsdcReceipt),
                "internal_data",
            ),
            (|v| v.rcpt_sign.clear(), "receipt_signature"),
            (|v| v.sdc_id.clear(), "sdc_number"),
            (|v| v.rcpt_sign = "AB#CD".into(), "receipt_signature"),
        ] {
            let mut f = mixed();
            edit(&mut f.vsdc);
            assert_eq!(
                try_compose(f.input(SalesTyCd::Normal, RcptTyCd::Sale)),
                Err(ReceiptError::BadQrField(field))
            );
        }
    }

    #[test]
    fn a_malformed_vsdc_timestamp_is_refused() {
        for bad in ["", "2012", "20121325110735", "20120525250000", "abc"] {
            let mut f = mixed();
            f.vsdc.vsdc_rcpt_pbct_date = bad.into();
            assert!(
                matches!(
                    try_compose(f.input(SalesTyCd::Normal, RcptTyCd::Sale)),
                    Err(ReceiptError::BadTimestamp(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_refund_is_refused_until_it_can_name_the_receipt_it_refunds() {
        let f = mixed();
        assert_eq!(
            try_compose(f.input(SalesTyCd::Normal, RcptTyCd::Refund)),
            Err(ReceiptError::RefundReferenceMissing)
        );
    }

    #[test]
    fn lines_that_disagree_with_the_item_count_are_refused() {
        let mut f = mixed();
        f.lines.pop();
        assert!(matches!(
            try_compose(f.input(SalesTyCd::Normal, RcptTyCd::Sale)),
            Err(ReceiptError::ItemCountMismatch { .. })
        ));
    }

    #[test]
    fn it_opens_with_init_and_code_page_zero() {
        let b = mixed().normal();
        assert_eq!(b[..5], [0x1B, 0x40, 0x1B, 0x74, 0x00]);
    }

    #[test]
    fn printer_state_is_reset_immediately_before_the_cut() {
        for ty in [SalesTyCd::Normal, SalesTyCd::Proforma] {
            let b = compose(mixed().input(ty, RcptTyCd::Sale));
            let tail = [
                &[0x1B, 0x45, 0x00][..], // bold off
                &[0x1D, 0x21, 0x00],     // size normal
                &[0x1B, 0x61, 0x00],     // left
                &[0x1D, 0x56, 0x42, 0x00],
            ]
            .concat();
            assert!(b.ends_with(&tail));
        }
    }

    #[test]
    fn control_characters_in_names_never_print_as_question_marks() {
        let f = Fixture::new(
            vec![line("Milk\t1L\r\nFresh", "1", "1000", TaxType::A)],
            "1000",
        );
        let t = text(&f.normal());
        assert!(t.contains("Milk 1L  Fresh"), "{t}");
        assert!(!t.contains('?'));
    }

    #[test]
    fn long_names_wrap_and_no_line_exceeds_the_paper_width() {
        let long = "Extra long product name that goes on and on and on past the edge";
        let f = Fixture::new(vec![line(long, "1", "1000", TaxType::A)], "1000");
        let t = text(&f.normal());
        assert!(t.contains("Extra long product name that goes on and"));
        for l in t.lines() {
            assert!(l.chars().count() <= WIDTH, "too wide: {l:?}");
        }
    }

    #[test]
    fn a_discounted_line_shows_its_rate_and_net_amount() {
        let mut l = line("Gouda cheese", "0.200", "33600", TaxType::B);
        l.discount = Some(LineDiscount::Percent(Decimal::from(25)));
        let t = text(&Fixture::new(vec![l], "6000").normal());
        let d = t.lines().find(|l| l.starts_with("discount")).unwrap();
        assert!(
            d.starts_with("discount -25%") && d.ends_with("5,040"),
            "{d:?}"
        );
    }

    #[test]
    fn rate_above_zero_always_prints_but_a_zero_rate_only_when_used() {
        // 7.22 / 7.23: only exempt goods sold. B-18.00% still prints; A prints
        // because it was used; C and D do not.
        let only_a = Fixture::new(vec![line("Bread", "1", "1000", TaxType::A)], "1000");
        let t = text(&only_a.normal());
        // Band B carries no rate in the totals of this basket? It does: 18.
        assert!(
            t.contains("TOTAL A-EX") && t.contains("TOTAL B-18.00%"),
            "{t}"
        );
        assert!(!t.contains("TOTAL C") && !t.contains("TOTAL D"));
        // And with only band B sold, the unused zero-rate A is absent.
        let t = text(&band_b_only().normal());
        assert!(t.contains("TOTAL B-18.00%") && !t.contains("A-EX"), "{t}");
    }

    #[test]
    fn change_and_tender_are_printed() {
        let t = text(&mixed().normal());
        assert!(t.contains("CASH") && t.contains("10,000"));
        assert!(t.contains("CHANGE"));
    }

    #[test]
    fn money_is_never_printed_with_a_decimal_point_except_tax() {
        let t = text(&band_b_only().normal());
        let l = t.lines().find(|l| l.starts_with("TOTAL ")).unwrap();
        assert!(l.ends_with("6,450"), "{l:?}");
    }

    /// Not an assertion: `cargo test -- --nocapture rendered` shows the layout.
    #[test]
    fn rendered_example() {
        let t = text(&mixed().normal());
        println!("{t}");
        assert!(!t.is_empty());
    }
}
