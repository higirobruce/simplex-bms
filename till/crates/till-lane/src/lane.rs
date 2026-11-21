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
//! * `void_line` removes a line; its sequence number stays spent;
//! * `add_tender` takes a payment toward the basket, and `settle` runs the
//!   fiscal sequence (below).
//!
//! # The fiscal sequence
//!
//! `settle` does, in this order and no other: check the tender covers the
//! total (nothing is allocated if it does not) -> `begin_sale` (allocates the
//! invoice number) -> `FiscalPort::save_sales` -> `confirm_sale` -> print ->
//! drawer kick. Nothing prints before `confirm_sale` has returned `Ok`. The
//! three ways the VSDC can refuse diverge from it as follows:
//!
//! * `Unreachable`: the sale is abandoned (number retired), the screen becomes
//!   `HardStop`, nothing prints. Selling while the device is unreachable is
//!   prohibited (CIS 24.7.9), so the lane stops instead of queueing.
//! * `Rejected`: `fail_sale` counts the attempt; the sale stays `AwaitingVsdc`
//!   and is held as the lane's pending sale, so the SAME number is retried by
//!   the next `settle`. While it is pending the basket and tenders are frozen
//!   (the stored sale must match what is retried); `abandon_pending` is the
//!   way out of a rejection that can never succeed.
//! * `Overlapped` (994): the VSDC already holds that number and no receipt can
//!   be recovered. The sale is abandoned and the basket cleared; the lane keeps
//!   selling (the VSDC is reachable) and the cashier re-rings the basket.
//!
//! If the VSDC answered but `confirm_sale` failed, the receipt is kept in the
//! pending sale and the next `settle` retries only `confirm_sale`: asking the
//! VSDC again would hit 994 on a number it has already recorded.
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

use crate::device::{Drawer, Printer};
use crate::fiscal::{FiscalError, FiscalPort, SalePayload};
use crate::latency::{Timer, Timings};
use crate::receipt::{ReceiptInput, compose};
use thiserror::Error;
use till_core::codes::{PmtTyCd, RcptTyCd, SalesTyCd};
use till_core::scan::{EmbeddedMode, ScanError, Scanned, classify};
use till_core::{
    Basket, BasketError, BasketLine, BasketTotals, LineAmounts, LineError, Money, Qty, Settled,
    TenderEntry, TenderError, TenderSet,
};
use till_store::{CachedProduct, Counter, NewSale, SaleHandle, Store, StoreError, VsdcReceipt};

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
    /// Printed on every receipt (CIS 7.29 block: name, address, TIN).
    pub shop_name: String,
    pub shop_address: String,
    pub shop_tin: String,
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
    #[error(
        "{name} is not a weighed product, but a scale label was scanned for it. Scan its own barcode, or ask the back office to check the product."
    )]
    NotWeighed { name: String },
    #[error("{0}")]
    Basket(#[from] BasketError),
    #[error("{name} cannot be sold as set up: {source}. Ask a supervisor to fix the catalogue.")]
    Line {
        name: String,
        #[source]
        source: LineError,
    },
    /// A store fault. The message depends on WHICH operation failed (see
    /// [`StoreOp`]): what the cashier must do after a failed scan is the
    /// opposite of what they must do after a failed confirm.
    #[error("{0}")]
    Store(StoreFault),
    #[error("The lane hit an internal fault ({0}). Call support.")]
    Internal(String),
    #[error("This sale is no longer open for scanning.")]
    NotSelling,
    #[error("{0}. Correct the payment and try again; nothing was recorded.")]
    Tender(#[from] TenderError),
    #[error("There is no receipt to reprint.")]
    NothingToReprint,
    #[error("There is nothing to pay for. Scan an item first.")]
    EmptyBasket,
    #[error(
        "The VSDC refused this sale: {code} {msg}. The basket and payment are held; press pay to retry the same invoice, or ask a supervisor to cancel it."
    )]
    Rejected { code: String, msg: String },
    #[error(
        "The VSDC already holds this invoice number, so this sale could not be recorded. Nothing was printed. Ring the basket up again."
    )]
    Overlapped,
    #[error(
        "The fiscal device (VSDC) cannot be reached. Selling is stopped until it is back. Nothing was printed. Call a supervisor."
    )]
    Unreachable,
    #[error(
        "A sale is waiting on the VSDC. Press pay to retry it, or ask a supervisor to cancel it, before changing the basket or payment."
    )]
    SalePending,
    #[error(
        "The VSDC has already recorded this sale, so it cannot be cancelled. Press pay to finish it."
    )]
    AlreadyRecorded,
    #[error(
        "A sale from a previous session is still outstanding with the VSDC. It cannot be retried safely; ask a supervisor to cancel it before trading continues."
    )]
    OutstandingSale,
    #[error("There is no completed sale to move on from.")]
    NotSettled,
    #[error("The lane is not stopped.")]
    NotStopped,
    #[error("There is no pending sale to cancel.")]
    NothingPending,
}

/// The store operation that failed. Each one has its own message, because the
/// right instruction differs: a failed scan is safe to repeat, a failed
/// confirm is a sale RRA already holds and must never be rung again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    /// Reading the pending sales when the lane opens.
    Startup,
    /// Looking a scanned code up in the catalogue.
    Scan,
    /// Allocating the invoice number, before the VSDC is asked anything.
    BeginSale,
    /// Recording the VSDC's answer after it has fiscalised the sale.
    ConfirmSale,
    /// Cancelling a held sale at a supervisor's request.
    CancelSale,
    /// Retiring the number after the VSDC proved unreachable.
    RetireUnreachable,
    /// Retiring the number after the VSDC answered 994.
    RetireOverlapped,
    /// Noting a VSDC rejection against a held sale.
    RecordRejection,
}

/// A [`StoreError`] together with the operation it interrupted.
#[derive(Debug, Error)]
#[error("{}", self.op.advice(&self.source))]
pub struct StoreFault {
    pub op: StoreOp,
    #[source]
    pub source: StoreError,
}

impl StoreOp {
    fn advice(self, e: &StoreError) -> String {
        match self {
            StoreOp::Startup => format!(
                "The lane could not read its own saved sales ({e}). It has not opened; call support."
            ),
            StoreOp::Scan => format!(
                "The catalogue could not be read ({e}). Try the scan again; if it repeats, call support."
            ),
            StoreOp::BeginSale => format!(
                "The sale could not be started ({e}). Nothing was sent to the VSDC and nothing was printed. Press pay to try again; if it repeats, call support."
            ),
            StoreOp::ConfirmSale => format!(
                "The VSDC has recorded this sale, but the till could not save its answer ({e}). Nothing was printed. Do not ring the items again. Press pay to finish the sale; if it fails again, call support."
            ),
            StoreOp::CancelSale => format!(
                "The sale could not be cancelled ({e}). It is still held. Try again; if it repeats, call support."
            ),
            StoreOp::RetireUnreachable => format!(
                "The VSDC cannot be reached, and the till could not retire this sale's invoice number ({e}). Selling is stopped and nothing was printed. Call support."
            ),
            StoreOp::RetireOverlapped => format!(
                "The VSDC already holds this invoice number, and the till could not retire it ({e}). Nothing was printed. Call support before ringing the basket again."
            ),
            StoreOp::RecordRejection => format!(
                "The VSDC refused this sale, and the till could not note the refusal ({e}). The sale is still held; press pay to retry it, or ask a supervisor to cancel it. If it repeats, call support."
            ),
        }
    }
}

impl LaneError {
    fn store(op: StoreOp, source: StoreError) -> LaneError {
        LaneError::Store(StoreFault { op, source })
    }
}

/// What the lane last learned from the VSDC. The lane does not poll it, so
/// this is the outcome of the last call and never a claim about right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VsdcOutcome {
    /// No sale has been sent to the VSDC this session.
    NoneYet,
    /// The last sale sent got an answer (a receipt, a refusal or a 994).
    Answered,
    /// The last sale sent could not reach the VSDC.
    Unreachable,
}

/// Everything needed to compose the sale's receipt again. Owned, so it
/// outlives the basket and tenders that `next_customer` clears.
#[derive(Debug)]
struct LastSale {
    lines: Vec<(BasketLine, LineAmounts)>,
    totals: BasketTotals,
    tenders: TenderSet,
    settled: Settled,
    invc_no: i64,
    /// Stamped once, when the sale was confirmed; a copy carries the same time.
    cis_datetime: String,
    vsdc: VsdcReceipt,
    /// The finished original, as first composed.
    original: Option<Vec<u8>>,
    /// An original has reached the printer. Until it has, a reprint IS the
    /// original (CIS 7.18: one original; the customer must get it).
    original_printed: bool,
}

impl LastSale {
    fn compose(&self, cfg: &LaneConfig, sales_ty: SalesTyCd) -> Result<Vec<u8>, String> {
        compose(ReceiptInput {
            shop_name: &cfg.shop_name,
            shop_address: &cfg.shop_address,
            shop_tin: &cfg.shop_tin,
            mrc: &self.vsdc.mrc_no,
            sales_ty,
            rcpt_ty: RcptTyCd::Sale,
            lines: &self.lines,
            totals: &self.totals,
            tenders: &self.tenders,
            settled: &self.settled,
            invc_no: self.invc_no,
            cis_datetime: &self.cis_datetime,
            vsdc: &self.vsdc,
        })
        .map_err(|e| e.to_string())
    }
}

/// A sale that holds an invoice number but is not yet confirmed.
#[derive(Debug)]
struct Pending {
    handle: SaleHandle,
    /// Set once the VSDC has answered, so a retry never asks it again.
    receipt: Option<VsdcReceipt>,
    /// Found in the store at startup rather than begun this session. It is
    /// never retried (see `LaneState::new`), only cancelled.
    recovered: bool,
}

pub struct LaneState {
    store: Store,
    cfg: LaneConfig,
    buffer: String,
    notice: Notice,
    screen: Screen,
    basket: Basket,
    tenders: TenderSet,
    pending: Option<Pending>,
    /// The last confirmed sale, kept so a reprint can be recomposed as a COPY.
    last_sale: Option<LastSale>,
    /// What the VSDC last told us (see [`VsdcOutcome`]).
    vsdc_outcome: VsdcOutcome,
    /// The last measurement of each latency path (see `crate::latency`).
    timings: Timings,
}

impl LaneState {
    pub async fn new(store: Store, cfg: LaneConfig) -> Result<LaneState, LaneError> {
        let timer = Timer::start();
        // A sale left `AwaitingVsdc` by a previous session (a crash between a
        // rejection and its retry) holds an invoice number, and the store
        // refuses to issue another while it exists: without this the till
        // would be wedged for good. The store has no "sent to the VSDC"
        // marker, so we cannot tell whether the VSDC received it; re-sending
        // risks a 994 or a duplicate fiscal record. So it is held as
        // `recovered` and can only be CANCELLED (`abandon_pending`), never
        // retried. Cancelling may leave a record at RRA with no local
        // counterpart. That is a reconciliation problem for Phase 3
        // (`Store::reconcile_counter_from_vsdc`), not illegal trading, and it
        // is strictly better than a till nobody can use.
        //
        // If that read itself fails the lane does not open: a till that cannot
        // read its own database may be hiding exactly such a sale, and would
        // look healthy while every sale failed. `Err`, not a notice and not a
        // HardStop (which would blame a VSDC that may be perfectly reachable).
        let mut pending = None;
        let mut notice = Notice::None;
        if let Some(p) = store
            .pending_sale(&cfg.dvc_srl_no, cfg.counter)
            .await
            .map_err(|e| LaneError::store(StoreOp::Startup, e))?
        {
            pending = Some(Pending {
                handle: SaleHandle {
                    id: p.id,
                    invc_no: p.invc_no,
                },
                receipt: None,
                recovered: true,
            });
            notice = Notice::Problem(LaneError::OutstandingSale.to_string());
        }
        let timings = Timings {
            cold_start: Some(timer.elapsed()),
            ..Timings::default()
        };
        Ok(LaneState {
            store,
            cfg,
            buffer: String::new(),
            notice,
            screen: Screen::Selling,
            basket: Basket::new(),
            tenders: TenderSet::default(),
            pending,
            last_sale: None,
            vsdc_outcome: VsdcOutcome::NoneYet,
            timings,
        })
    }

    /// The next customer: leaves `Settled`, clearing the finished basket and
    /// tenders so a fresh sale (and the next invoice number) can begin.
    pub fn next_customer(&mut self) -> Result<(), LaneError> {
        self.buffer.clear();
        if !matches!(self.screen, Screen::Settled { .. }) {
            return self.refuse(LaneError::NotSettled);
        }
        self.basket = Basket::new();
        self.tenders = TenderSet::default();
        self.screen = Screen::Selling;
        self.notice = Notice::None;
        Ok(())
    }

    /// A character from the wedge. Starting a new scan clears the previous notice.
    pub fn key(&mut self, c: char) {
        let timer = Timer::start();
        if self.screen != Screen::Selling
            || c.is_control()
            || self.buffer.chars().count() >= MAX_BUFFER
        {
            return;
        }
        if self.buffer.is_empty() {
            self.notice = Notice::None;
        }
        self.buffer.push(c);
        self.timings.key_to_screen = Some(timer.elapsed());
    }

    pub fn backspace(&mut self) {
        self.buffer.pop();
    }

    /// Terminates a scan. On refusal the notice is already set and the buffer
    /// already cleared; `Err` is reserved for faults the cashier cannot act on.
    pub async fn enter(&mut self) -> Result<(), LaneError> {
        let timer = Timer::start();
        let raw = std::mem::take(&mut self.buffer);
        match self.try_enter(&raw).await {
            Ok(()) => {
                self.timings.scan_to_line = Some(timer.elapsed());
                Ok(())
            }
            Err(e) => self.refuse(e),
        }
    }

    pub async fn void_line(&mut self, item_seq: u16) -> Result<(), LaneError> {
        self.buffer.clear();
        if let Err(e) = self.ensure_open() {
            return self.refuse(e);
        }
        match self.basket.void_line(item_seq) {
            Ok(()) => {
                self.notice = Notice::Info(format!("Line {item_seq} removed."));
                Ok(())
            }
            Err(e) => self.refuse(e.into()),
        }
    }

    /// Takes a payment toward the basket. A refused tender (zero, fractional)
    /// is a `Problem` notice and `Ok(())`, like every other cashier error.
    pub fn add_tender(&mut self, code: PmtTyCd, amount: Money) -> Result<(), LaneError> {
        self.buffer.clear();
        let added = self.ensure_open().and_then(|()| {
            self.tenders.push(TenderEntry {
                pmt_ty_cd: code,
                amount,
            })?;
            Ok(())
        });
        match added {
            Ok(()) => {
                self.notice = Notice::Info(format!("{amount} received."));
                Ok(())
            }
            Err(e) => self.refuse(e),
        }
    }

    /// Everything tendered so far; `None` if the sum passes the supported range.
    pub fn tender_total(&self) -> Option<Money> {
        self.tenders.total()
    }

    /// Completes the sale through the fiscal device. See the module docs for
    /// the sequence and how each failure diverges from it.
    ///
    /// `Ok(Some(settled))`: the sale is confirmed and the receipt printed.
    /// `Ok(None)`: the sale was NOT completed for a reason the cashier can act
    /// on; the notice (and, for `Unreachable`, the screen) says what. `Err`
    /// only for a fault they cannot act on (store I/O).
    pub async fn settle(
        &mut self,
        fiscal: &mut dyn FiscalPort,
        printer: &mut dyn Printer,
        drawer: &mut dyn Drawer,
    ) -> Result<Option<Settled>, LaneError> {
        let timer = Timer::start();
        self.buffer.clear();
        match self.try_settle(fiscal, printer, drawer, timer).await {
            Ok(settled) => Ok(Some(settled)),
            Err(e) => self.refuse(e).map(|()| None),
        }
    }

    /// Leaves a hard stop once a supervisor has cleared the fault (CIS 24.7.9:
    /// after the error is cleared, selling must be enabled again). The basket
    /// is kept; the tenders are cleared because the cashier takes payment
    /// again. This does NOT claim the VSDC is back: if it is still unreachable
    /// the next `settle` hard-stops again, which is the correct outcome.
    pub fn resume_from_hard_stop(&mut self) -> Result<(), LaneError> {
        self.buffer.clear();
        if !matches!(self.screen, Screen::HardStop { .. }) {
            return self.refuse(LaneError::NotStopped);
        }
        self.screen = Screen::Selling;
        self.tenders = TenderSet::default();
        self.notice = Notice::Info(
            "Selling resumed. Take payment again; the lane stops again if the VSDC is still down."
                .into(),
        );
        Ok(())
    }

    /// Gives up a rejected sale that can never succeed: retires its number and
    /// unfreezes the basket and tenders. Refused once the VSDC has recorded it.
    pub async fn abandon_pending(&mut self) -> Result<(), LaneError> {
        self.buffer.clear();
        match self.try_abandon_pending().await {
            Ok(()) => {
                self.notice = Notice::Info("Sale cancelled. Its invoice number is retired.".into());
                Ok(())
            }
            Err(e) => self.refuse(e),
        }
    }

    async fn try_abandon_pending(&mut self) -> Result<(), LaneError> {
        if self.screen != Screen::Selling {
            return Err(LaneError::NotSelling);
        }
        match &self.pending {
            None => Err(LaneError::NothingPending),
            Some(p) if p.receipt.is_some() => Err(LaneError::AlreadyRecorded),
            Some(p) => {
                let reason = if p.recovered {
                    "cancelled at startup: left AwaitingVsdc by a previous session; not known whether the VSDC received it, so not retried (may leave an RRA record with no local counterpart; reconcile in Phase 3)"
                } else {
                    "cancelled by supervisor after VSDC rejection"
                };
                self.store
                    .abandon_sale(&p.handle, reason)
                    .await
                    .map_err(|e| LaneError::store(StoreOp::CancelSale, e))?;
                self.pending = None;
                Ok(())
            }
        }
    }

    /// Scans, voids and tenders are only for an open, unfrozen sale.
    fn ensure_open(&self) -> Result<(), LaneError> {
        if self.screen != Screen::Selling {
            return Err(LaneError::NotSelling);
        }
        match &self.pending {
            Some(p) if p.recovered => Err(LaneError::OutstandingSale),
            Some(_) => Err(LaneError::SalePending),
            None => Ok(()),
        }
    }

    async fn try_settle(
        &mut self,
        fiscal: &mut dyn FiscalPort,
        printer: &mut dyn Printer,
        drawer: &mut dyn Drawer,
        timer: Timer,
    ) -> Result<Settled, LaneError> {
        if self.screen != Screen::Selling {
            return Err(LaneError::NotSelling);
        }
        if self.pending.as_ref().is_some_and(|p| p.recovered) {
            return Err(LaneError::OutstandingSale);
        }
        if self.basket.is_empty() {
            return Err(LaneError::EmptyBasket);
        }
        let totals = self.basket.totals()?;
        // Refused here, before an invoice number is taken, so a short tender
        // burns nothing. `begin_sale` re-checks what it is given.
        let settled = self.tenders.settle(totals.tot_amt)?;

        let mut pending = match self.pending.take() {
            Some(p) => p,
            None => {
                let handle = self
                    .store
                    .begin_sale(NewSale {
                        dvc_srl_no: &self.cfg.dvc_srl_no,
                        counter: self.cfg.counter,
                        bhf_id: &self.cfg.bhf_id,
                        basket: &self.basket,
                        tenders: &self.tenders,
                        settled: &settled,
                        buyer_tin: None,
                    })
                    .await
                    .map_err(|e| LaneError::store(StoreOp::BeginSale, e))?;
                Pending {
                    handle,
                    receipt: None,
                    recovered: false,
                }
            }
        };

        if pending.receipt.is_none() {
            let payload = SalePayload {
                invc_no: pending.handle.invc_no,
                bhf_id: self.cfg.bhf_id.clone(),
                tot_amt: totals.tot_amt.to_string(),
                tot_taxbl_amt: totals.tot_taxbl_amt.to_string(),
                tot_tax_amt: totals.tot_tax_amt.to_string(),
            };
            match fiscal.save_sales(&payload) {
                Ok(r) => {
                    self.vsdc_outcome = VsdcOutcome::Answered;
                    pending.receipt = Some(r);
                }
                Err(e) => {
                    self.vsdc_outcome = match e {
                        FiscalError::Unreachable => VsdcOutcome::Unreachable,
                        _ => VsdcOutcome::Answered,
                    };
                    return self.fiscal_refused(pending, e).await;
                }
            }
        }
        let Some(receipt) = pending.receipt.clone() else {
            // Unreachable: set just above. Fail safe, never panic.
            self.pending = Some(pending);
            return Err(LaneError::Internal("pending sale lost its receipt".into()));
        };

        // The VSDC has answered. Keep the answer until the store has it, so a
        // failed confirm is retried WITHOUT calling `save_sales` again. Do not
        // "simplify" a retry into a full re-send: the VSDC has already recorded
        // this invoice number, so a second save returns 994 (Overlapped), burns
        // the number and loses a sale that was in fact fiscalised.
        if let Err(e) = self
            .store
            .confirm_sale(&pending.handle, receipt.clone())
            .await
        {
            self.pending = Some(pending);
            return Err(LaneError::store(StoreOp::ConfirmSale, e));
        }

        // Confirmed: only now may anything print.
        self.screen = Screen::Settled {
            change: settled.change,
        };
        let mut faults: Vec<String> = Vec::new();
        // Never leave an earlier sale's receipt behind as "the last receipt".
        self.last_sale = None;
        match self.capture_sale(&settled, &totals, &pending.handle, &receipt) {
            Ok(sale) => match sale.compose(&self.cfg, SalesTyCd::Normal) {
                Ok(bytes) => {
                    // Retained BEFORE the write: a jam must not lose the sale.
                    let sale = self.last_sale.insert(sale);
                    let bytes = sale.original.insert(bytes);
                    match printer.write(bytes) {
                        Ok(()) => {
                            sale.original_printed = true;
                            self.timings.tender_to_bytes = Some(timer.elapsed());
                        }
                        Err(e) => faults.push(format!("the receipt did not print ({e})")),
                    }
                }
                Err(e) => {
                    // Keep the sale: the reprint will try again.
                    self.last_sale = Some(sale);
                    faults.push(format!("the receipt could not be composed ({e})"));
                }
            },
            Err(e) => faults.push(format!("the receipt could not be composed ({e})")),
        }
        // Cash was taken either way, so the drawer opens even if printing failed.
        if let Err(e) = drawer.kick() {
            faults.push(format!("the drawer did not open ({e})"));
        }
        self.notice = if faults.is_empty() {
            Notice::Info(format!(
                "Sale {} complete. Change {}.",
                pending.handle.invc_no, settled.change
            ))
        } else {
            Notice::Problem(format!(
                "Sale {} is recorded with the tax authority, but {}. Call support; do not ring it again.",
                pending.handle.invc_no,
                faults.join(" and ")
            ))
        };
        Ok(settled)
    }

    /// The original receipt composed for the last confirmed sale, byte for
    /// byte, if one could be composed.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by tests; the UI reprints through `reprint`")
    )]
    pub fn last_receipt(&self) -> Option<&[u8]> {
        self.last_sale.as_ref().and_then(|s| s.original.as_deref())
    }

    /// What the VSDC last told the lane. The lane does not poll the VSDC, so
    /// this is the outcome of the last sale sent, not current connectivity.
    pub fn vsdc_outcome(&self) -> VsdcOutcome {
        self.vsdc_outcome
    }

    /// Prints the last sale's receipt again. Printing is not the sale: this
    /// touches only the printer, never the store or the VSDC.
    ///
    /// CIS 7.18: print only one original; a reprint carries the COPY mark. So
    /// a reprint is RECOMPOSED as `SalesTyCd::Copy` (COPY banner, `CS` label),
    /// never replayed, or the customer would hold two identical originals. The
    /// one exception: if no original has reached the printer yet (it jammed
    /// or was out of paper), this first reprint IS the original, because
    /// otherwise the sale would have no original at all.
    ///
    /// A printer fault, like having nothing to reprint, is a `Problem` notice
    /// with `Ok(())`: the cashier can act on it.
    pub fn reprint(&mut self, printer: &mut dyn Printer) -> Result<(), LaneError> {
        self.buffer.clear();
        let Some(sale) = self.last_sale.as_mut() else {
            return self.refuse(LaneError::NothingToReprint);
        };
        let is_original = !sale.original_printed;
        let bytes = match (is_original, sale.original.as_ref()) {
            (true, Some(original)) => Ok(original.clone()),
            (true, None) => sale.compose(&self.cfg, SalesTyCd::Normal),
            (false, _) => sale.compose(&self.cfg, SalesTyCd::Copy),
        };
        let written = match bytes {
            Ok(bytes) => {
                let r = printer.write(&bytes).map_err(|e| e.to_string());
                if r.is_ok() && is_original {
                    sale.original_printed = true;
                }
                r.map_err(|e| format!("The receipt did not print ({e})"))
            }
            Err(e) => Err(format!("The receipt could not be composed ({e})")),
        };
        self.notice = match written {
            Ok(()) if is_original => Notice::Info("Receipt printed.".into()),
            Ok(()) => Notice::Info("Receipt reprinted (marked COPY).".into()),
            Err(m) => Notice::Problem(format!(
                "{m}. Check the printer and try the reprint again; the sale itself is safe."
            )),
        };
        Ok(())
    }

    /// Handles the VSDC's refusal. Always returns `Err`: the sale did not
    /// complete. The variant says what the cashier sees.
    async fn fiscal_refused(
        &mut self,
        pending: Pending,
        why: FiscalError,
    ) -> Result<Settled, LaneError> {
        match why {
            FiscalError::Unreachable => {
                // Stop first: even if the store write below fails the lane must
                // not sell.
                self.screen = Screen::HardStop {
                    reason: LaneError::Unreachable.to_string(),
                };
                match self
                    .store
                    .abandon_sale(&pending.handle, "VSDC unreachable")
                    .await
                {
                    Ok(()) => Err(LaneError::Unreachable),
                    Err(e) => {
                        self.pending = Some(pending);
                        Err(LaneError::store(StoreOp::RetireUnreachable, e))
                    }
                }
            }
            FiscalError::Rejected { code, msg } => {
                let reason = format!("VSDC rejected: {code} {msg}");
                let recorded = self.store.fail_sale(&pending.handle, &reason).await;
                // Held either way: the number is still ours and still unconfirmed.
                self.pending = Some(pending);
                recorded.map_err(|e| LaneError::store(StoreOp::RecordRejection, e))?;
                Err(LaneError::Rejected { code, msg })
            }
            FiscalError::Overlapped => {
                match self
                    .store
                    .abandon_sale(&pending.handle, "VSDC 994: invoice number already recorded")
                    .await
                {
                    Ok(()) => {
                        self.basket = Basket::new();
                        self.tenders = TenderSet::default();
                        Err(LaneError::Overlapped)
                    }
                    Err(e) => {
                        self.pending = Some(pending);
                        Err(LaneError::store(StoreOp::RetireOverlapped, e))
                    }
                }
            }
        }
    }

    /// Gathers what a receipt is composed from, owned, at the moment the sale
    /// is confirmed (the basket and tenders are cleared by `next_customer`).
    fn capture_sale(
        &self,
        settled: &Settled,
        totals: &BasketTotals,
        handle: &SaleHandle,
        vsdc: &VsdcReceipt,
    ) -> Result<LastSale, String> {
        Ok(LastSale {
            lines: self.basket.apportioned_lines().map_err(|e| e.to_string())?,
            totals: totals.clone(),
            tenders: self.tenders.clone(),
            settled: *settled,
            invc_no: handle.invc_no,
            cis_datetime: kigali_now(),
            vsdc: vsdc.clone(),
            original: None,
            original_printed: false,
        })
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
    /// The last measurement of each latency path.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by tests; no screen shows timings yet")
    )]
    pub fn timings(&self) -> &Timings {
        &self.timings
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
        self.ensure_open()?;
        let scanned = classify(raw, self.cfg.embedded)?;
        let product = self
            .store
            .lookup(&scanned)
            .await
            .map_err(|e| LaneError::store(StoreOp::Scan, e))?
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

/// The lane clock as the CIS block wants it: `yyyymmddhhmmss`, Kigali time.
/// Rwanda is UTC+2 all year (no daylight saving), so no zone database is needed.
fn kigali_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    kigali_stamp(secs)
}

fn kigali_stamp(unix_secs: u64) -> String {
    let local = unix_secs.saturating_add(2 * 3600);
    let days = i64::try_from(local / 86_400).unwrap_or(0);
    let rem = local % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
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
    // A scale label (weight or price) on a unit product would sell a fraction
    // of a packet, or override its price: refuse, as for the opposite case.
    if matches!(scanned, Scanned::WeightEmbedded { .. }) && !p.is_weighed {
        return Err(LaneError::NotWeighed {
            name: p.name.clone(),
        });
    }
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
    use crate::latency::Budget;
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
                shop_name: "Simplex Mart".into(),
                shop_address: "KG 7 Ave, Kigali".into(),
                shop_tin: "123456789".into(),
            },
        )
        .await
        .unwrap()
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

    #[tokio::test]
    async fn a_weight_label_on_a_unit_product_is_refused_by_name() {
        let mut l = lane_with(
            &[product("u", None, Some("23456"), false)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, WEIGHT_LABEL).await.unwrap();
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("Item u"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
    }

    #[tokio::test]
    async fn a_price_label_on_a_unit_product_is_refused_by_name() {
        let mut l = lane_with(
            &[product("u", None, Some("23456"), false)],
            EmbeddedMode::Price,
        )
        .await;
        scan(&mut l, "2123456015003").await.unwrap();
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("Item u"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
    }

    #[tokio::test]
    async fn a_store_fault_returns_err_sets_the_notice_and_clears_the_buffer() {
        let store = Store::open_in_memory().await.unwrap();
        let mut l = LaneState::new(
            store,
            LaneConfig {
                dvc_srl_no: "DVC1".into(),
                bhf_id: "00".into(),
                embedded: EmbeddedMode::Weight,
                counter: Counter::SaleInvc,
                shop_name: "Simplex Mart".into(),
                shop_address: "KG 7 Ave, Kigali".into(),
                shop_tin: "123456789".into(),
            },
        )
        .await
        .unwrap();
        l.store.pool().close().await; // a genuine failure: the pool is gone
        EAN.chars().for_each(|c| l.key(c));
        let r = l.enter().await;
        assert!(matches!(r, Err(LaneError::Store(_))), "{r:?}");
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.buffer(), "");
        assert!(l.basket().is_empty());
    }

    // ---- tender and the fiscal sequence ----

    use crate::device::{SimDrawer, SimPrinter};
    use crate::fiscal::{SimFiscal, SimMode};

    fn rwf(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }
    fn fiscal(mode: SimMode) -> SimFiscal {
        SimFiscal {
            next_rcpt_no: 1,
            mode,
        }
    }
    fn devices() -> (SimPrinter, SimDrawer) {
        (
            SimPrinter {
                written: vec![],
                fail_next: false,
            },
            SimDrawer { kicks: 0 },
        )
    }
    /// A lane with one item of 1250 in the basket.
    async fn lane_with_item() -> LaneState {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, EAN).await.unwrap();
        l
    }
    /// The invoice number as printed on the receipt.
    fn printed_invc(p: &SimPrinter) -> String {
        String::from_utf8_lossy(&p.written)
            .lines()
            // The SDC block has a "RECEIPT NUMBER:" too; the CIS one is last.
            .rfind(|l| l.contains("RECEIPT NUMBER:"))
            .and_then(|l| l.trim_end().rsplit(' ').next())
            .unwrap_or("")
            .to_string()
    }
    async fn awaiting(l: &LaneState) -> Option<i64> {
        l.store
            .pending_sale("DVC1", Counter::SaleInvc)
            .await
            .unwrap()
            .map(|p| p.invc_no)
    }

    #[tokio::test]
    async fn a_scan_becomes_a_line_within_budget() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        assert!(l.timings().scan_to_line.is_none());
        assert!(l.timings().key_to_screen.is_none());
        scan(&mut l, EAN).await.unwrap();
        let t = l.timings();
        assert!(t.scan_to_line.expect("scan measured").as_millis() < Budget::SCAN_TO_LINE_MS);
        assert!(t.key_to_screen.expect("key measured").as_millis() < Budget::KEY_TO_SCREEN_MS);
        assert!(t.cold_start.expect("start measured").as_millis() < Budget::COLD_START_MS);
        eprintln!("timings: {t:?}");
    }

    #[tokio::test]
    async fn a_refused_scan_records_no_scan_timing() {
        let mut l = lane_with(&[], EmbeddedMode::Weight).await;
        scan(&mut l, EAN).await.unwrap();
        assert!(l.timings().scan_to_line.is_none());
    }

    #[tokio::test]
    async fn settling_reaches_printer_bytes_within_budget() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("2000")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        assert!(!p.written.is_empty());
        let t = l.timings().tender_to_bytes.expect("settle measured");
        assert!(t.as_millis() < Budget::TENDER_TO_BYTES_MS);
        eprintln!("tender_to_bytes: {t:?}");
    }

    #[tokio::test]
    async fn a_settle_that_completes_no_sale_records_no_timing() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("100")).unwrap();
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(l.timings().tender_to_bytes.is_none());
    }

    #[tokio::test]
    async fn a_failed_print_records_no_timing() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("2000")).unwrap();
        let (mut p, mut d) = devices();
        p.fail_next = true;
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        assert!(l.timings().tender_to_bytes.is_none());
    }

    #[tokio::test]
    async fn a_complete_sale_prints_and_opens_the_drawer() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("2000")).unwrap();
        assert_eq!(l.tender_total(), Some(rwf("2000")));
        let (mut p, mut d) = devices();
        let s = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        assert_eq!(s.change, rwf("750"));
        assert_eq!(l.screen(), &Screen::Settled { change: rwf("750") });
        assert!(!p.written.is_empty());
        assert_eq!(d.kicks, 1);
        // Confirmed in the store: nothing is left awaiting, and the number is spent.
        assert_eq!(awaiting(&l).await, None);
        assert_eq!(printed_invc(&p), "1");
    }

    #[tokio::test]
    async fn tender_below_the_amount_due_does_not_settle() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1000")).unwrap();
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.screen(), &Screen::Selling);
        assert!(p.written.is_empty());
        assert_eq!(d.kicks, 0);
        assert_eq!(awaiting(&l).await, None);
        // No number was burned: topping up and settling takes invoice 1.
        l.add_tender(PmtTyCd::Cash, rwf("250")).unwrap();
        let mut f = fiscal(SimMode::Succeed);
        l.settle(&mut f, &mut p, &mut d).await.unwrap().unwrap();
        assert_eq!(printed_invc(&p), "1");
    }

    #[tokio::test]
    async fn over_tender_on_a_card_yields_no_change() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Card, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let s = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        assert_eq!(s.change, Money::zero());

        // A card tender ABOVE the total is refused, never paid out as cash.
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Card, rwf("2000")).unwrap();
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert!(p.written.is_empty());
        assert_eq!(d.kicks, 0);
        assert_eq!(awaiting(&l).await, None);
    }

    #[tokio::test]
    async fn an_unreachable_vsdc_stops_the_lane_and_abandons_the_number() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Unreachable), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(matches!(l.screen(), Screen::HardStop { .. }));
        assert!(p.written.is_empty());
        assert_eq!(d.kicks, 0);
        // Abandoned, not awaiting: the number is retired.
        assert_eq!(awaiting(&l).await, None);
        // Retired, not reused: the next sale on this store takes number 2.
        let mut l2 = LaneState::new(l.store.clone(), l.cfg.clone())
            .await
            .unwrap();
        scan(&mut l2, EAN).await.unwrap();
        l2.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        l2.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(printed_invc(&p), "2");
    }

    #[tokio::test]
    async fn a_hard_stop_can_be_cleared_and_stops_again_if_the_vsdc_is_still_down() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = fiscal(SimMode::Unreachable);
        l.settle(&mut f, &mut p, &mut d).await.unwrap();
        assert!(matches!(l.screen(), Screen::HardStop { .. }));

        l.resume_from_hard_stop().unwrap();
        assert_eq!(l.screen(), &Screen::Selling);
        assert_eq!(l.basket().len(), 1);
        assert_eq!(l.tender_total(), Some(Money::zero()));

        // Still unreachable: re-tender, settle, and it stops again, selling nothing.
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let r = l.settle(&mut f, &mut p, &mut d).await.unwrap();
        assert!(r.is_none());
        assert!(matches!(l.screen(), Screen::HardStop { .. }));
        assert!(p.written.is_empty());
        assert_eq!(d.kicks, 0);

        // Resuming when not stopped is refused, not a panic.
        l.resume_from_hard_stop().unwrap();
        l.resume_from_hard_stop().unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.screen(), &Screen::Selling);

        // Once the VSDC is back the same basket sells.
        f.mode = SimMode::Succeed;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        assert!(l.settle(&mut f, &mut p, &mut d).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_lane_that_cannot_read_its_own_pending_sales_does_not_open() {
        let store = Store::open_in_memory().await.unwrap();
        store.pool().close().await;
        let r = LaneState::new(
            store,
            LaneConfig {
                dvc_srl_no: "DVC1".into(),
                bhf_id: "00".into(),
                embedded: EmbeddedMode::Weight,
                counter: Counter::SaleInvc,
                shop_name: "Simplex Mart".into(),
                shop_address: "KG 7 Ave, Kigali".into(),
                shop_tin: "123456789".into(),
            },
        )
        .await;
        assert!(matches!(r, Err(LaneError::Store(_))), "{:?}", r.err());
    }

    #[tokio::test]
    async fn the_next_customer_rings_a_second_sale_on_the_next_number() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = fiscal(SimMode::Succeed);
        l.settle(&mut f, &mut p, &mut d).await.unwrap().unwrap();
        assert_eq!(printed_invc(&p), "1");
        // Terminal until told otherwise, and only from Settled.
        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), 1);

        l.next_customer().unwrap();
        assert_eq!(l.screen(), &Screen::Selling);
        assert!(l.basket().is_empty());
        assert_eq!(l.tender_total(), Some(Money::zero()));

        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), 1);
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        p.written.clear();
        l.settle(&mut f, &mut p, &mut d).await.unwrap().unwrap();
        assert_eq!(printed_invc(&p), "2");
        assert_eq!(d.kicks, 2);

        // Not allowed mid-sale.
        l.next_customer().unwrap();
        l.next_customer().unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.screen(), &Screen::Selling);
    }

    #[tokio::test]
    async fn a_sale_left_pending_by_a_crash_is_detected_and_can_only_be_cancelled() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(
            &mut fiscal(SimMode::Reject("894 No".into())),
            &mut p,
            &mut d,
        )
        .await
        .unwrap();
        assert_eq!(awaiting(&l).await, Some(1));
        let (store, cfg) = (l.store.clone(), l.cfg.clone());
        drop(l); // the "crash"

        let mut l2 = LaneState::new(store, cfg).await.unwrap();
        match l2.notice() {
            Notice::Problem(m) => assert!(m.contains("previous session"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        // Frozen, and never retried: settle does not reach the VSDC.
        scan(&mut l2, EAN).await.unwrap();
        assert!(l2.basket().is_empty());
        let r = l2
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(p.written.is_empty());
        assert_eq!(awaiting(&l2).await, Some(1));

        l2.abandon_pending().await.unwrap();
        assert_eq!(awaiting(&l2).await, None);
        scan(&mut l2, EAN).await.unwrap();
        l2.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        l2.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(printed_invc(&p), "2", "the retired number is not reused");
    }

    #[tokio::test]
    async fn keystrokes_during_a_hard_stop_do_not_build_a_sale() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Unreachable), &mut p, &mut d)
            .await
            .unwrap();
        let before = l.basket().len();
        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), before);
        assert_eq!(l.buffer(), "");
        assert!(matches!(l.screen(), Screen::HardStop { .. }));
        l.add_tender(PmtTyCd::Cash, rwf("5")).unwrap();
        assert_eq!(l.tender_total(), Some(rwf("1250")));
        l.void_line(1).await.unwrap();
        assert_eq!(l.basket().len(), before);
        // And settle cannot be driven through a hard stop.
        let r = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(p.written.is_empty());
    }

    #[tokio::test]
    async fn a_rejected_sale_counts_an_attempt_and_keeps_the_number() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = fiscal(SimMode::Reject("894 Bad item class".into()));
        let r = l.settle(&mut f, &mut p, &mut d).await.unwrap();
        assert!(r.is_none());
        assert_eq!(l.screen(), &Screen::Selling);
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("894") && m.contains("Bad item class"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        assert!(p.written.is_empty());
        assert_eq!(d.kicks, 0);
        let first = awaiting(&l).await.expect("still awaiting");

        // The basket and tenders are frozen while the sale is pending.
        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), 1);
        l.add_tender(PmtTyCd::Cash, rwf("5")).unwrap();
        assert_eq!(l.tender_total(), Some(rwf("1250")));

        // Retry takes the SAME number and completes.
        f.mode = SimMode::Succeed;
        l.settle(&mut f, &mut p, &mut d).await.unwrap().unwrap();
        assert_eq!(awaiting(&l).await, None);
        assert_eq!(first, 1);
        assert_eq!(printed_invc(&p), "1");
    }

    #[tokio::test]
    async fn a_rejection_that_can_never_succeed_can_be_cancelled() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(
            &mut fiscal(SimMode::Reject("899 No".into())),
            &mut p,
            &mut d,
        )
        .await
        .unwrap();
        l.abandon_pending().await.unwrap();
        assert_eq!(awaiting(&l).await, None);
        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), 2);
    }

    #[tokio::test]
    async fn an_overlapped_sale_is_abandoned_and_the_lane_keeps_selling() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Overlapped), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert_eq!(l.screen(), &Screen::Selling);
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("Ring the basket up again"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        assert!(p.written.is_empty());
        assert_eq!(d.kicks, 0);
        assert_eq!(awaiting(&l).await, None);
        assert!(l.basket().is_empty());
        assert_eq!(l.tender_total(), Some(Money::zero()));
        // Selling resumes on the next number.
        scan(&mut l, EAN).await.unwrap();
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .unwrap();
        assert!(!p.written.is_empty());
    }

    /// A fiscal port that records the call and then breaks the store, so the
    /// VSDC "answers" but `confirm_sale` cannot succeed.
    struct BreaksStore {
        pool: till_store::Store,
    }
    impl FiscalPort for BreaksStore {
        fn save_sales(&mut self, payload: &SalePayload) -> Result<VsdcReceipt, FiscalError> {
            let pool = self.pool.pool().clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(pool.close())
            });
            SimFiscal {
                next_rcpt_no: 7,
                mode: SimMode::Succeed,
            }
            .save_sales(payload)
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_prints_when_the_store_cannot_confirm() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = BreaksStore {
            pool: l.store.clone(),
        };
        let r = l.settle(&mut f, &mut p, &mut d).await;
        assert!(matches!(r, Err(LaneError::Store(_))), "{r:?}");
        assert!(
            p.written.is_empty(),
            "printed before confirm_sale succeeded"
        );
        assert_eq!(d.kicks, 0);
        assert_eq!(l.screen(), &Screen::Selling);
        // The VSDC's answer is kept, so a retry would not ask it again.
        assert!(l.pending.as_ref().is_some_and(|x| x.receipt.is_some()));
    }

    #[tokio::test]
    async fn a_printer_fault_after_confirmation_does_not_undo_the_sale() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        p.fail_next = true;
        let s = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(s.is_some());
        assert!(matches!(l.screen(), Screen::Settled { .. }));
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(d.kicks, 1);
        assert_eq!(awaiting(&l).await, None);
    }

    #[tokio::test]
    async fn a_printer_failure_does_not_undo_a_confirmed_sale() {
        // A paper jam is a stationery problem, not a fiscal one.
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("2000")).unwrap();
        let (mut p, mut d) = devices();
        p.fail_next = true;
        let s = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        // settle reports success.
        assert_eq!(s.expect("settled").change, rwf("750"));
        assert_eq!(l.screen(), &Screen::Settled { change: rwf("750") });
        // The store itself holds the sale as Confirmed: not Failed, not
        // Voided/abandoned, and nothing left awaiting.
        let rows: Vec<(i64, String)> = sqlx::query_as("select invc_no, state from sale")
            .fetch_all(l.store.pool())
            .await
            .unwrap();
        assert_eq!(rows, vec![(1, "Confirmed".to_string())]);
        assert_eq!(awaiting(&l).await, None);
        // The cashier is told about the printer.
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("did not print"), "{m}"),
            n => panic!("expected Problem, got {n:?}"),
        }
        // Nothing reached paper, but the bytes were kept.
        assert!(p.written.is_empty());
        let kept = l.last_receipt().expect("retained").to_vec();
        assert!(!kept.is_empty());

        // No original has reached paper, so the first reprint IS the original
        // (fail_next was one-shot): otherwise the sale would have none.
        l.reprint(&mut p).unwrap();
        assert_eq!(p.written, kept);
        assert_eq!(printed_invc(&p), "1");
        assert!(matches!(l.notice(), Notice::Info(_)));
        // The original is out; every later reprint is a COPY, not these bytes.
        l.reprint(&mut p).unwrap();
        let copy = p.written[kept.len()..].to_vec();
        assert_ne!(copy, kept);
        assert!(printable(&copy).contains("COPY"));
        assert_eq!(l.last_receipt(), Some(kept.as_slice()));
        // Reprinting never touched the store.
        let n: i64 = sqlx::query_scalar("select count(*) from sale")
            .fetch_one(l.store.pool())
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn a_reprint_that_fails_again_is_a_problem_and_keeps_the_receipt() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .unwrap();
        let kept = l.last_receipt().unwrap().to_vec();
        // The original print succeeded, so the paper holds exactly one copy.
        assert_eq!(p.written, kept);
        p.fail_next = true;
        l.reprint(&mut p).unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.last_receipt(), Some(kept.as_slice()));
        // The failed attempt wrote nothing.
        assert_eq!(p.written, kept);
        l.reprint(&mut p).unwrap();
        // The retry emitted a second document, and it is a COPY, not a replay.
        let copy = &p.written[kept.len()..];
        assert!(!copy.is_empty());
        assert_ne!(copy, kept.as_slice());
        assert!(printable(copy).contains("COPY"));
    }

    fn printable(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[tokio::test]
    async fn a_reprint_is_a_marked_copy_never_a_second_original() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .unwrap();
        let original = p.written.clone();
        l.reprint(&mut p).unwrap();
        let copy = p.written[original.len()..].to_vec();
        // CIS 7.18: not the same document, marked COPY, labelled CS not NS.
        assert_ne!(copy, original);
        let (o, c) = (printable(&original), printable(&copy));
        assert!(!o.contains("COPY") && o.contains("NS"), "{o}");
        assert!(c.contains("COPY"), "{c}");
        assert!(c.contains("CS") && !c.contains("NS"), "{c}");
        // The retained original is untouched, and a second copy matches the first.
        assert_eq!(l.last_receipt(), Some(original.as_slice()));
        l.reprint(&mut p).unwrap();
        assert_eq!(&p.written[original.len() + copy.len()..], copy.as_slice());
        // Still a reprint after the basket is cleared for the next customer.
        l.next_customer().unwrap();
        l.reprint(&mut p).unwrap();
        assert!(matches!(l.notice(), Notice::Info(_)));
    }

    #[tokio::test]
    async fn reprint_with_no_sale_is_a_problem_not_an_error() {
        let mut l = lane_with(&[], EmbeddedMode::Weight).await;
        let (mut p, _d) = devices();
        l.reprint(&mut p).unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert!(p.written.is_empty());
        assert!(l.last_receipt().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_confirm_failure_does_not_tell_the_cashier_to_rescan() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = BreaksStore {
            pool: l.store.clone(),
        };
        let e = l.settle(&mut f, &mut p, &mut d).await.unwrap_err();
        let m = e.to_string();
        assert!(!m.contains("catalogue") && !m.contains("scan again"), "{m}");
        assert!(m.contains("Do not ring the items again"), "{m}");
        assert_eq!(l.notice(), &Notice::Problem(m));
        // While a scan failure still says what it always did.
        let scan = LaneError::store(StoreOp::Scan, StoreError::Busy).to_string();
        assert!(scan.contains("catalogue could not be read"), "{scan}");
    }

    #[tokio::test]
    async fn the_vsdc_outcome_is_what_the_last_sale_learned() {
        let mut l = lane_with_item().await;
        assert_eq!(l.vsdc_outcome(), VsdcOutcome::NoneYet);
        l.add_tender(PmtTyCd::Cash, rwf("2500")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Unreachable), &mut p, &mut d)
            .await
            .unwrap();
        assert_eq!(l.vsdc_outcome(), VsdcOutcome::Unreachable);
        l.resume_from_hard_stop().unwrap();
        // Resuming claims nothing: the outcome stays what it was.
        assert_eq!(l.vsdc_outcome(), VsdcOutcome::Unreachable);
        l.add_tender(PmtTyCd::Cash, rwf("2500")).unwrap();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert_eq!(l.vsdc_outcome(), VsdcOutcome::Answered);
    }

    #[tokio::test]
    async fn settling_an_empty_basket_is_a_problem() {
        let mut l = lane_with(&[], EmbeddedMode::Weight).await;
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none());
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    #[test]
    fn the_cis_clock_is_kigali_time() {
        // 2026-11-21 22:30:05 UTC is 00:30:05 on the 22nd in Kigali.
        assert_eq!(kigali_stamp(1_795_300_205), "20261122003005");
        assert_eq!(kigali_stamp(0), "19700101020000");
        // 2024-02-29 (leap day) 12:00:00 UTC.
        assert_eq!(kigali_stamp(1_709_208_000), "20240229140000");
    }
}
