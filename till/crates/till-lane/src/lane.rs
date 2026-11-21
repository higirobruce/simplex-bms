//! The sale itself: the scan buffer, the basket, and every refusal a cashier
//! can hit.
//!
//! # State machine
//!
//! `Screen::Selling` is the only state that accepts scans and voids. Later tasks
//! move it to `HardStop` (a fiscal fault that must be cleared by a supervisor)
//! and `Settled` (tender complete, change shown). In `Selling`:
//!
//! * `key` appends to the scan buffer (control characters are dropped and the
//!   buffer is capped, so a stuck scanner cannot grow it without bound);
//! * `enter` takes the buffer, classifies it, looks it up in the local
//!   catalogue, prices the line, and pushes it onto the basket;
//! * `void_line` removes a line; its sequence number stays spent.
//!
//! # Refusals
//!
//! Nothing here panics. Every refusal does three things: sets
//! [`Notice::Problem`] to a sentence the cashier can act on, clears the scan
//! buffer (so the next scan is not appended to the last one's debris), and
//! returns `Ok(())` when the cashier can act on it. Only faults they cannot
//! act on (a store I/O failure) return `Err(LaneError)`, so the UI can treat an
//! ordinary unknown barcode as a business outcome, not an error path. The basket is untouched by a refused scan; in particular a
//! line that fails pricing is rejected BEFORE `push_line`, so it does not spend
//! a sequence number.

use thiserror::Error;
use till_core::scan::{EmbeddedMode, ScanError, Scanned, classify};
use till_core::{Basket, BasketError, BasketLine, LineError, Money, Qty};
use till_store::{CachedProduct, Counter, Store, StoreError};

/// Longest scan buffer kept. Longer than any code (EAN-13 is 13); the excess
/// is dropped so the resulting `BadLength` still tells the cashier it was wrong.
const MAX_BUFFER: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Selling,
    HardStop { reason: String },
    Settled { change: Money },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    None,
    Info(String),
    Problem(String),
}

#[derive(Debug, Clone)]
pub struct LaneConfig {
    pub dvc_srl_no: String,
    pub bhf_id: String,
    pub embedded: EmbeddedMode,
    pub counter: Counter,
}

#[derive(Debug, Error)]
pub enum LaneError {
    #[error("{0}. Clear the entry and scan again, or key the code by hand.")]
    Scan(#[from] ScanError),
    #[error(
        "Code {0} is not in the catalogue. Check the item or ask a supervisor; nothing was added."
    )]
    UnknownCode(String),
    #[error("{name} is sold by weight. Weigh it and scan the scale label.")]
    NeedsWeightLabel { name: String },
    #[error("The scale label for {name} carries no weight. Weigh it again.")]
    ZeroWeight { name: String },
    #[error(
        "{name} is not fully set up (it has no item class). Do not sell it; ask the back office to fix the product."
    )]
    Unconfigured { name: String },
    #[error("{0}")]
    Basket(#[from] BasketError),
    #[error("{name} cannot be sold as set up: {source}. Ask a supervisor to fix the catalogue.")]
    Line {
        name: String,
        #[source]
        source: LineError,
    },
    #[error(
        "The catalogue could not be read ({0}). Try the scan again; if it repeats, call support."
    )]
    Store(#[from] StoreError),
    #[error("The lane hit an internal fault ({0}). Call support.")]
    Internal(String),
    #[error("This sale is no longer open for scanning.")]
    NotSelling,
}

pub struct LaneState {
    store: Store,
    cfg: LaneConfig,
    buffer: String,
    notice: Notice,
    screen: Screen,
    basket: Basket,
}

impl LaneState {
    pub async fn new(store: Store, cfg: LaneConfig) -> LaneState {
        LaneState {
            store,
            cfg,
            buffer: String::new(),
            notice: Notice::None,
            screen: Screen::Selling,
            basket: Basket::new(),
        }
    }

    /// A character from the wedge. Starting a new scan clears the previous notice.
    pub fn key(&mut self, c: char) {
        if c.is_control() || self.buffer.chars().count() >= MAX_BUFFER {
            return;
        }
        if self.buffer.is_empty() {
            self.notice = Notice::None;
        }
        self.buffer.push(c);
    }

    pub fn backspace(&mut self) {
        self.buffer.pop();
    }

    /// Terminates a scan. On refusal the notice is already set and the buffer
    /// already cleared; `Err` is reserved for faults the cashier cannot act on.
    pub async fn enter(&mut self) -> Result<(), LaneError> {
        let raw = std::mem::take(&mut self.buffer);
        match self.try_enter(&raw).await {
            Ok(()) => Ok(()),
            Err(e) => self.refuse(e),
        }
    }

    pub async fn void_line(&mut self, item_seq: u16) -> Result<(), LaneError> {
        self.buffer.clear();
        if self.screen != Screen::Selling {
            return self.refuse(LaneError::NotSelling);
        }
        match self.basket.void_line(item_seq) {
            Ok(()) => {
                self.notice = Notice::Info(format!("Line {item_seq} removed."));
                Ok(())
            }
            Err(e) => self.refuse(e.into()),
        }
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }
    pub fn notice(&self) -> &Notice {
        &self.notice
    }
    pub fn screen(&self) -> &Screen {
        &self.screen
    }
    pub fn basket(&self) -> &Basket {
        &self.basket
    }
    pub fn config(&self) -> &LaneConfig {
        &self.cfg
    }

    /// Shows the cashier the problem and clears the buffer. Business outcomes
    /// the cashier can act on return `Ok(())`; only faults they cannot act on
    /// (store I/O, internal) return `Err`.
    fn refuse(&mut self, e: LaneError) -> Result<(), LaneError> {
        self.buffer.clear();
        self.notice = Notice::Problem(e.to_string());
        match e {
            LaneError::Store(_) | LaneError::Internal(_) => Err(e),
            _ => Ok(()),
        }
    }

    async fn try_enter(&mut self, raw: &str) -> Result<(), LaneError> {
        if self.screen != Screen::Selling {
            return Err(LaneError::NotSelling);
        }
        let scanned = classify(raw, self.cfg.embedded)?;
        let product = self
            .store
            .lookup(&scanned)
            .await?
            .ok_or_else(|| LaneError::UnknownCode(raw.trim().to_string()))?;
        let line = line_for(&product, &scanned)?;

        // Price it first: a line that cannot be priced must not spend a number.
        line.amounts().map_err(|source| LaneError::Line {
            name: product.name.clone(),
            source,
        })?;
        let seq = self.basket.push_line(line)?;
        self.notice = Notice::Info(format!("{} added (line {seq}).", product.name));
        Ok(())
    }
}

/// Builds the line a scan stands for. A scale label's weight is the quantity;
/// a label's price (price mode) beats the catalogue price, at quantity 1.
fn line_for(p: &CachedProduct, scanned: &Scanned) -> Result<BasketLine, LaneError> {
    // itemClsCd is mandatory on the VSDC and drawn from RRA's controlled list;
    // a guess would put a false tax classification on a tax document.
    let item_cls_cd = match p.item_cls_cd.as_deref() {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => {
            return Err(LaneError::Unconfigured {
                name: p.name.clone(),
            });
        }
    };
    let (qty, prc) = match scanned {
        Scanned::WeightEmbedded { qty: Some(q), .. } => {
            if !q.is_positive() {
                return Err(LaneError::ZeroWeight {
                    name: p.name.clone(),
                });
            }
            (*q, p.unit_price)
        }
        Scanned::WeightEmbedded {
            price: Some(label_price),
            ..
        } => (one()?, *label_price),
        Scanned::WeightEmbedded { .. } => (one()?, p.unit_price),
        // A per-kg product with no weight would sell one kilo of it.
        _ if p.is_weighed => {
            return Err(LaneError::NeedsWeightLabel {
                name: p.name.clone(),
            });
        }
        _ => (one()?, p.unit_price),
    };
    Ok(BasketLine {
        item_seq: 0, // assigned by Basket::push_line
        item_cd: p.sku.clone(),
        item_cls_cd,
        item_nm: p.name.clone(),
        bcd: p.barcode.clone(),
        qty,
        prc,
        tax_ty_cd: p.tax_ty_cd,
        discount: None,
    })
}

fn one() -> Result<Qty, LaneError> {
    Qty::try_from_str("1").map_err(|e| LaneError::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use till_core::basket::MAX_ITEM_SEQ;
    use till_core::codes::TaxType;

    const EAN: &str = "4006381333931";
    const WEIGHT_LABEL: &str = "2123456007343"; // item 23456, 0.734 kg

    fn product(id: &str, barcode: Option<&str>, plu: Option<&str>, weighed: bool) -> CachedProduct {
        CachedProduct {
            id: id.into(),
            sku: format!("SKU-{id}"),
            name: format!("Item {id}"),
            barcode: barcode.map(Into::into),
            plu: plu.map(Into::into),
            unit_price: Money::try_from_str("1250").unwrap(),
            tax_ty_cd: TaxType::B,
            is_weighed: weighed,
            item_cls_cd: Some("5020230602".into()),
        }
    }

    async fn lane_with(products: &[CachedProduct], embedded: EmbeddedMode) -> LaneState {
        let store = Store::open_in_memory().await.unwrap();
        store.sync_catalogue(products).await.unwrap();
        LaneState::new(
            store,
            LaneConfig {
                dvc_srl_no: "DVC1".into(),
                bhf_id: "00".into(),
                embedded,
                counter: Counter::SaleInvc,
            },
        )
        .await
    }

    async fn scan(l: &mut LaneState, code: &str) -> Result<(), LaneError> {
        code.chars().for_each(|c| l.key(c));
        l.enter().await
    }

    #[tokio::test]
    async fn a_scanned_barcode_becomes_a_priced_line() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, EAN).await.unwrap();
        let lines: Vec<_> = l.basket().lines().collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].item_nm, "Item a");
        assert_eq!(lines[0].item_cd, "SKU-a");
        assert_eq!(lines[0].prc, Money::try_from_str("1250").unwrap());
        assert_eq!(lines[0].qty, Qty::try_from_str("1").unwrap());
        assert_eq!(lines[0].item_cls_cd, "5020230602");
        assert_eq!(l.buffer(), "");
    }

    #[tokio::test]
    async fn an_unknown_barcode_tells_the_cashier_and_adds_nothing() {
        let mut l = lane_with(&[], EmbeddedMode::Weight).await;
        scan(&mut l, EAN).await.unwrap();
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains(EAN), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
    }

    #[tokio::test]
    async fn a_malformed_scan_is_reported_not_swallowed() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        let bad = "4006381333932";
        scan(&mut l, bad).await.unwrap();
        let want = ScanError::BadCheckDigit {
            expected: 1,
            got: 2,
        }
        .to_string();
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains(&want), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        assert_eq!(l.buffer(), "");
        // The next scan is not corrupted by the last.
        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), 1);
    }

    #[tokio::test]
    async fn a_weighed_product_takes_its_quantity_from_the_label() {
        let mut l = lane_with(
            &[product("w", None, Some("23456"), true)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, WEIGHT_LABEL).await.unwrap();
        let (line, amounts) = l.basket().apportioned_lines().unwrap().remove(0);
        assert_eq!(line.qty, Qty::try_from_str("0.734").unwrap());
        // 0.734 x 1250 = 917.5, rounded once half-up to whole francs.
        assert_eq!(amounts.sply_amt, Money::try_from_str("918").unwrap());
    }

    #[tokio::test]
    async fn a_weighed_product_scanned_without_a_label_is_refused() {
        let mut l = lane_with(&[product("w", Some(EAN), None, true)], EmbeddedMode::Weight).await;
        scan(&mut l, EAN).await.unwrap();
        assert!(l.basket().is_empty());
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    #[tokio::test]
    async fn the_thousandth_line_is_refused_with_an_instruction() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        for _ in 0..MAX_ITEM_SEQ {
            scan(&mut l, EAN).await.unwrap();
        }
        assert_eq!(l.basket().len(), usize::from(MAX_ITEM_SEQ));
        scan(&mut l, EAN).await.unwrap();
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("start another"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        assert_eq!(l.basket().len(), usize::from(MAX_ITEM_SEQ));
    }

    #[tokio::test]
    async fn voiding_a_line_removes_it_without_reissuing_its_number() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        for _ in 0..3 {
            scan(&mut l, EAN).await.unwrap();
        }
        l.void_line(2).await.unwrap();
        scan(&mut l, EAN).await.unwrap();
        let seqs: Vec<u16> = l.basket().lines().map(|x| x.item_seq).collect();
        assert_eq!(seqs, vec![1, 3, 4]);
    }

    #[tokio::test]
    async fn voiding_a_missing_line_is_a_problem_not_a_panic() {
        let mut l = lane_with(&[], EmbeddedMode::Weight).await;
        l.void_line(7).await.unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    #[tokio::test]
    async fn a_line_that_cannot_be_priced_does_not_spend_a_sequence_number() {
        let mut long = product("l", Some(EAN), None, false);
        long.name = "x".repeat(300);
        let mut l = lane_with(
            &[long, product("b", Some("96385074"), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, EAN).await.unwrap();
        assert!(l.basket().is_empty());
        scan(&mut l, "96385074").await.unwrap();
        assert_eq!(l.basket().lines().next().unwrap().item_seq, 1);
    }

    #[tokio::test]
    async fn a_price_label_overrides_the_catalogue_price() {
        let mut l = lane_with(
            &[product("w", None, Some("23456"), true)],
            EmbeddedMode::Price,
        )
        .await;
        scan(&mut l, "2123456015003").await.unwrap();
        let line = l.basket().lines().next().unwrap();
        assert_eq!(line.prc, Money::try_from_str("1500").unwrap());
        assert_eq!(line.qty, Qty::try_from_str("1").unwrap());
    }

    #[tokio::test]
    async fn a_product_without_an_item_class_is_refused_by_name() {
        let mut p = product("m", Some(EAN), None, false);
        p.item_cls_cd = None;
        p.name = "Fresh Milk 1L".into();
        let mut l = lane_with(&[p], EmbeddedMode::Weight).await;
        scan(&mut l, EAN).await.unwrap();
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("Fresh Milk 1L"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        assert_eq!(l.buffer(), "");
    }
}
