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
use crate::fmt;
use crate::latency::{Timer, Timings};
use crate::receipt::{ReceiptInput, compose};
use crate::report::{self, shift_close_report, shift_close_report_copy, shift_report};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use thiserror::Error;
use till_core::codes::{PmtTyCd, RcptTyCd, SalesTyCd};
use till_core::scan::{EmbeddedMode, ScanError, Scanned, classify};
use till_core::{
    Basket, BasketError, BasketLine, BasketTotals, LineAmounts, LineError, Money, Qty, Settled,
    TenderEntry, TenderError, TenderSet,
};
use till_store::{
    CachedProduct, CashierEntry, Counter, MIN_SEARCH_LEN, NewSale, OpenShift, SaleHandle,
    ShiftClose, Store, StoreError, VsdcReceipt,
};
use tokio::runtime::Handle;

/// Longest scan buffer kept. Longer than any code (EAN-13 is 13); the excess
/// is dropped so the resulting `BadLength` still tells the cashier it was wrong.
const MAX_BUFFER: usize = 32;

/// Wrong PINs in a row the till tolerates before it locks sign-in.
const PIN_FREE_TRIES: u32 = 5;

/// The first lockout. Each further wrong PIN doubles it, up to the cap below,
/// so a guesser is held to a handful of tries per hour but a cashier who
/// fumbled once is never stuck for long.
const PIN_LOCK_BASE: Duration = Duration::from_secs(30);

/// The longest a lockout grows to.
const PIN_LOCK_MAX: Duration = Duration::from_secs(15 * 60);

/// The most digits a PIN entry holds. The cloud's policy is 6 to 12 digits
/// (`PIN_MIN_LENGTH` and `PIN_MAX_LENGTH` in `src/lib/cashier.ts`), so 12 must
/// be accepted or a cashier with a long PIN could never sign in. A 13-digit
/// EAN-13 is still "too long" and not counted as a guess; an 8 to 12 digit
/// barcode scanned here is counted as a wrong guess, a known and accepted trade.
const MAX_PIN_DIGITS: usize = 12;

/// How long sign-in is locked after `failures` wrong PINs in a row, or `None`
/// while the cashier still has tries left.
fn lockout_for(failures: u32) -> Option<Duration> {
    let over = failures.checked_sub(PIN_FREE_TRIES)?;
    let factor = 2u32.checked_pow(over.min(16)).unwrap_or(u32::MAX);
    Some(PIN_LOCK_BASE.saturating_mul(factor).min(PIN_LOCK_MAX))
}

/// A wait in words a cashier can act on.
fn wait_words(d: Duration) -> String {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    if secs >= 120 {
        format!("{} minutes", secs.div_ceil(60))
    } else if secs == 1 {
        "1 second".into()
    } else {
        format!("{secs} seconds")
    }
}

/// The character that separates a count from the item it multiplies: `3*1234`.
/// A symbol, not a letter, because a letter in the buffer means a name search
/// and because the scanner wedge types letters.
pub const TIMES: char = '*';

/// The largest count a cashier may key. A till sells cases, not pallets, and a
/// mis-keyed 10000 should be refused rather than rung up.
pub const MAX_TIMES: u32 = 999;

/// How many name matches are offered at once. The list is read off a screen
/// and picked by a one- or two-digit number, never paged.
pub const MAX_MATCHES: u32 = 12;

/// An open name search: what was typed, the count waiting to be applied to
/// whatever is picked, and the products offered.
#[derive(Debug, Clone)]
pub struct NameSearch {
    pub term: String,
    pub times: Option<u32>,
    pub matches: Vec<CachedProduct>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// No shift is open on this device, so nothing can be sold. A normal
    /// state: every till starts its day here until a cashier opens a shift.
    NoShift,
    Selling,
    HardStop {
        reason: String,
    },
    Settled {
        change: Money,
    },
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
    #[error("No shift is open. Open a shift with the opening float before selling.")]
    NoShift,
    #[error("A shift is already open on this till.")]
    ShiftAlreadyOpen,
    #[error("The opening float cannot be negative. Count the drawer again.")]
    NegativeFloat,
    #[error("Choose your name and key your PIN before opening the shift.")]
    NoCashier,
    #[error("That is not a number on the list. Key the number beside your name, then press Enter.")]
    NotACashier,
    #[error(
        "No cashier can sign in on this till yet. Ask the back office to set up cashiers and PINs; the till fetches them when it next reaches the cloud."
    )]
    NoCashiersSaved,
    #[error("Key your PIN, then press Enter.")]
    EmptyPin,
    #[error("That is too long to be a PIN, so it was not checked. Key just your PIN.")]
    PinTooLong,
    #[error(
        "Wrong PIN for {name}. {left} more wrong tries and sign-in locks for a while. Key it again, or press Esc to choose another name."
    )]
    WrongPin { name: String, left: u32 },
    #[error(
        "Wrong PIN for {name}. Too many wrong PINs: sign-in is locked for {wait}. Ask a supervisor if you are locked out of your own PIN."
    )]
    WrongPinLocked { name: String, wait: String },
    #[error("Sign-in is locked after too many wrong PINs. Wait {wait}, then key your PIN again.")]
    PinLocked { wait: String },
    #[error("{0}. Correct the payment and try again; nothing was recorded.")]
    Tender(#[from] TenderError),
    #[error("There is no receipt to reprint.")]
    NothingToReprint,
    #[error("No shift has been closed on this till, so there is no close report to reprint.")]
    NoZToReprint,
    #[error("There is nothing to pay for. Scan an item first.")]
    EmptyBasket,
    #[error(
        "{0} is not a usable count. Key a whole number from 1 to {max}, then the item.",
        max = MAX_TIMES
    )]
    BadMultiplier(String),
    #[error(
        "{name} is sold by weight, so a count means nothing. Weigh it and scan the scale label."
    )]
    MultiplierOnWeighed { name: String },
    #[error("Type at least {MIN_SEARCH_LEN} letters of the name, then press Enter.")]
    SearchTooShort,
    #[error("Nothing in the catalogue matches \"{0}\". Check the spelling, or scan the item.")]
    NoNameMatch(String),
    #[error("There is no item {0} in the list. Key a number from the list, or scan the item.")]
    BadPick(String),
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
    #[error("No payment has been taken, so there is none to take back.")]
    NoTenderToRemove,
    #[error(
        "A sale is still waiting on the VSDC, so the shift cannot be closed. Finish or cancel that sale first; it must be accounted for to RRA."
    )]
    ShiftHasPendingSale,
    #[error(
        "A sale is in progress. Finish it or void its lines and take back its payments before closing the shift."
    )]
    SaleInProgress,
    #[error("The declared cash cannot be negative. Count the drawer again.")]
    NegativeDeclaredCash,
    #[error("This shift was already closed. Open a new shift to sell.")]
    ShiftAlreadyClosed,
}

/// The store operation that failed. Each one has its own message, because the
/// right instruction differs: a failed scan is safe to repeat, a failed
/// confirm is a sale RRA already holds and must never be rung again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    /// Reading the pending sales and the open shift when the lane opens.
    Startup,
    /// Writing a new shift.
    OpenShift,
    /// Closing the shift and recording the declared cash.
    CloseShift,
    /// Reading the shift's totals for a shift report.
    ShiftTotals,
    /// Recomposing a closed shift's report from the store.
    ClosedShift,
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
    /// Reading the list of cashiers for the sign-in screen.
    Cashiers,
    /// Checking a PIN against the saved cashiers.
    VerifyPin,
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
            StoreOp::OpenShift => format!(
                "The shift could not be opened ({e}). Nothing was recorded. Try again; if it repeats, call support."
            ),
            StoreOp::CloseShift => format!(
                "The shift could not be closed ({e}). Nothing was recorded and the shift is still open. Try again; if it repeats, call support."
            ),
            StoreOp::ShiftTotals => format!(
                "The shift's figures could not be read ({e}). Nothing was printed and nothing was changed. Try again; if it repeats, call support."
            ),
            StoreOp::ClosedShift => format!(
                "The closed shift's figures could not be read ({e}). Nothing was printed and nothing was changed. Try again; if it repeats, call support."
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
            StoreOp::Cashiers => format!(
                "The list of cashiers could not be read ({e}). Try again; if it repeats, call support."
            ),
            StoreOp::VerifyPin => format!(
                "The PIN could not be checked ({e}). It was not counted as wrong. Key it again; if it repeats, call support."
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

/// What a closed shift recorded, and whether its close report has reached paper.
struct LastZ {
    close: ShiftClose,
    mrc: Option<String>,
    at: String,
    printed: bool,
}

/// A PIN being keyed. It is its own buffer, never the scan buffer, so no
/// screen that draws the scan buffer can ever draw it. The digits are private
/// and only a mask leaves this type.
struct PinEntry {
    cashier: CashierEntry,
    digits: String,
    /// More digits were keyed than any PIN has (a barcode, a stuck key).
    overflow: bool,
}

impl PinEntry {
    fn new(cashier: CashierEntry) -> PinEntry {
        PinEntry {
            cashier,
            digits: String::new(),
            overflow: false,
        }
    }
}

// Not derived: a derive would print the digits into any log or test failure.
impl std::fmt::Debug for PinEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinEntry")
            .field("cashier", &self.cashier)
            .field("digits", &format_args!("<{} hidden>", self.digits.len()))
            .finish()
    }
}

/// A PIN check running off the UI thread.
struct Checking {
    cashier: CashierEntry,
    rx: Receiver<Result<Option<String>, StoreError>>,
}

/// Where the no-shift screen is in signing a cashier in.
enum SignIn {
    /// Picking a name from the list.
    Choosing,
    Pin(PinEntry),
    Checking(Checking),
    /// The PIN was right; the opening float is next. Carries the cloud id.
    Float {
        id: String,
        name: String,
    },
}

/// What the screen may know about sign-in. No digit of a PIN, only a mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInView<'a> {
    Choosing { cashiers: &'a [CashierEntry] },
    Pin { name: &'a str, mask: String },
    Checking { name: &'a str },
    Float { name: &'a str },
}

pub struct LaneState {
    store: Store,
    cfg: LaneConfig,
    buffer: String,
    /// An open name search, offering products to pick from. Not a `Screen`:
    /// the till is still selling, and a scan while the list is open rings up
    /// normally and closes it.
    search: Option<NameSearch>,
    notice: Notice,
    screen: Screen,
    basket: Basket,
    tenders: TenderSet,
    pending: Option<Pending>,
    /// The shift this till is selling in. `None` exactly when the screen is
    /// `NoShift`.
    shift: Option<OpenShift>,
    /// The last confirmed sale, kept so a reprint can be recomposed as a COPY.
    last_sale: Option<LastSale>,
    /// The last close report of this till, kept so a jam is a stationery problem:
    /// the shift stays closed and its report can be printed again. The fast
    /// path only: after a restart it is recomposed from the store.
    last_z: Option<LastZ>,
    /// What the VSDC last told us (see [`VsdcOutcome`]).
    vsdc_outcome: VsdcOutcome,
    /// The last measurement of each latency path (see `crate::latency`).
    timings: Timings,
    signin: SignIn,
    /// Cashiers who may sign in, read from the local cache.
    cashiers: Vec<CashierEntry>,
    cashiers_loaded: bool,
    /// Wrong PINs in a row on this till, across every cashier.
    pin_failures: u32,
    locked_until: Option<Instant>,
    /// The name of whoever opened the open shift, for the screen.
    shift_cashier: Option<String>,
    /// Test only: the PIN check waits (up to 5 s) for this before verifying,
    /// so a test can hold the worker and prove the caller did not wait on it.
    #[cfg(test)]
    pin_gate: Option<Receiver<()>>,
    /// Pokes the cloud sync after a row is queued, so it drains now rather
    /// than at its next heartbeat. `None` when sync is not configured.
    sync_wake: Option<std::sync::Arc<tokio::sync::Notify>>,
}

impl LaneState {
    /// Connects the lane to the cloud sync's wake-up signal.
    pub fn set_sync_wake(&mut self, wake: std::sync::Arc<tokio::sync::Notify>) {
        self.sync_wake = Some(wake);
    }

    /// A row was just committed to the outbound queue. Never blocks, never fails.
    fn wake_sync(&self) {
        if let Some(w) = &self.sync_wake {
            w.notify_one();
        }
    }

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
        // An absent shift is not a fault: it is how a till starts every
        // morning. A shift already open for this device is resumed silently,
        // so a restarted till does not ask for a second opening float.
        let shift = store
            .current_shift(&cfg.dvc_srl_no)
            .await
            .map_err(|e| LaneError::store(StoreOp::Startup, e))?;
        let screen = if shift.is_some() {
            Screen::Selling
        } else {
            Screen::NoShift
        };
        let cashiers = store
            .active_cashiers()
            .await
            .map_err(|e| LaneError::store(StoreOp::Startup, e))?;
        let shift_cashier = shift.as_ref().and_then(|s| {
            cashiers
                .iter()
                .find(|c| c.id == s.opened_by)
                .map(|c| c.name.clone())
        });
        let timings = Timings {
            cold_start: Some(timer.elapsed()),
            ..Timings::default()
        };
        Ok(LaneState {
            store,
            cfg,
            buffer: String::new(),
            search: None,
            notice,
            screen,
            basket: Basket::new(),
            tenders: TenderSet::default(),
            pending,
            shift,
            last_sale: None,
            last_z: None,
            vsdc_outcome: VsdcOutcome::NoneYet,
            timings,
            signin: SignIn::Choosing,
            cashiers,
            cashiers_loaded: true,
            pin_failures: 0,
            locked_until: None,
            shift_cashier,
            #[cfg(test)]
            pin_gate: None,
            sync_wake: None,
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
        if self.screen == Screen::NoShift {
            match &mut self.signin {
                // A PIN goes to its own masked entry, never the scan buffer.
                SignIn::Pin(p) => {
                    if c.is_ascii_digit() {
                        if p.digits.is_empty() {
                            self.notice = Notice::None;
                        }
                        if p.digits.len() >= MAX_PIN_DIGITS {
                            p.overflow = true;
                        } else {
                            p.digits.push(c);
                        }
                    }
                    return;
                }
                // The check is already running; keys are not queued behind it.
                SignIn::Checking(_) => return,
                SignIn::Choosing | SignIn::Float { .. } => {}
            }
        }
        if !matches!(self.screen, Screen::Selling | Screen::NoShift)
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
        if self.screen == Screen::NoShift {
            match &mut self.signin {
                SignIn::Pin(p) => {
                    p.digits.pop();
                    p.overflow = false;
                    return;
                }
                SignIn::Checking(_) => return,
                SignIn::Choosing | SignIn::Float { .. } => {}
            }
        }
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

    /// Takes the nth product a name search offered, counting from 1. For a tap
    /// on the list; keying the number goes through the buffer like a scan, and
    /// both land in the same place.
    pub async fn pick_nth(&mut self, n: usize) -> Result<(), LaneError> {
        self.buffer.clear();
        if let Err(e) = self.ensure_open() {
            return self.refuse(e);
        }
        match self.pick_at(n).await {
            Ok(()) => Ok(()),
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

    /// Takes back the last payment keyed, for a mis-key. Before settlement
    /// only: once a sale is settled or pending at the VSDC its tenders are
    /// history. Nothing to remove is a `Problem`, like any cashier error.
    ///
    /// `TenderSet` has no removal, so the set is rebuilt from the entries kept;
    /// each goes back through `push`, the same gate it first passed.
    pub fn remove_last_tender(&mut self) -> Result<(), LaneError> {
        self.buffer.clear();
        let removed = self.ensure_open().and_then(|()| {
            let entries = self.tenders.entries();
            let Some((last, kept)) = entries.split_last() else {
                return Err(LaneError::NoTenderToRemove);
            };
            let last = *last;
            self.tenders = rebuild(kept)?;
            Ok(last)
        });
        match removed {
            Ok(e) => {
                self.notice = Notice::Info(format!(
                    "{} tender of {} taken back.",
                    e.pmt_ty_cd.wire(),
                    e.amount
                ));
                Ok(())
            }
            Err(e) => self.refuse(e),
        }
    }

    /// Takes back every payment, leaving the basket as it is: the lines are the
    /// customer's shopping, only the payment was wrong. Same limits as
    /// [`LaneState::remove_last_tender`].
    pub fn clear_tenders(&mut self) -> Result<(), LaneError> {
        self.buffer.clear();
        let cleared = self.ensure_open().and_then(|()| {
            if self.tenders.entries().is_empty() {
                return Err(LaneError::NoTenderToRemove);
            }
            self.tenders = TenderSet::default();
            Ok(())
        });
        match cleared {
            Ok(()) => {
                self.notice = Notice::Info("All payments taken back.".to_owned());
                Ok(())
            }
            Err(e) => self.refuse(e),
        }
    }

    /// What is still owed on `due` after the payments so far. `None` when
    /// nothing is left to pay (or the sum cannot be represented), so callers
    /// never do arithmetic on money themselves.
    pub fn remaining(&self, due: Money) -> Option<Money> {
        let paid = self.tenders.total()?;
        due.checked_sub(paid).filter(|m| *m > Money::zero())
    }

    /// The payments taken so far, in the order taken. Read-only: every entry
    /// came through `add_tender`, and settlement rules stay in `till-core`.
    pub fn tenders(&self) -> &TenderSet {
        &self.tenders
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
        // Cancelling needs no shift: a sale left outstanding by a crash must
        // be clearable on a till that has since started on `NoShift`.
        if !matches!(self.screen, Screen::Selling | Screen::NoShift) {
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
        if self.screen == Screen::NoShift {
            return Err(LaneError::NoShift);
        }
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
        if self.screen == Screen::NoShift {
            return Err(LaneError::NoShift);
        }
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
                // The lane's own shift names the sale. The store re-checks it
                // inside its write transaction, which is what catches a shift
                // closed mid-sale.
                let shift = self
                    .shift
                    .as_ref()
                    .ok_or_else(|| LaneError::Internal("selling without a shift".into()))?;
                let handle = self
                    .store
                    .begin_sale(NewSale {
                        dvc_srl_no: &self.cfg.dvc_srl_no,
                        shift_id: &shift.id,
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
        self.wake_sync();

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
    /// The products an open name search is offering, in the order they are
    /// numbered on screen. Empty when no search is open.
    pub fn search_matches(&self) -> &[CachedProduct] {
        self.search.as_ref().map_or(&[], |s| &s.matches)
    }
    /// The count a name search is holding, to be applied to whatever is picked.
    pub fn search_times(&self) -> Option<u32> {
        self.search.as_ref().and_then(|s| s.times)
    }
    /// What was typed to produce the offered list, shown so a cashier can see
    /// whether a miss was a spelling slip.
    pub fn search_term(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.term.as_str())
    }
    pub fn notice(&self) -> &Notice {
        &self.notice
    }
    pub fn screen(&self) -> &Screen {
        &self.screen
    }
    /// The shift this till is selling in, if one is open.
    pub fn shift(&self) -> Option<&OpenShift> {
        self.shift.as_ref()
    }

    /// Opens a shift in the name of the cashier who has signed in (chosen from
    /// the list and PIN verified) and moves the lane from `NoShift` to
    /// `Selling`. Without that sign-in nothing opens. A refusal the cashier can
    /// act on (no sign-in, a negative float, a shift already open) is `Ok(())`
    /// with a `Problem` notice; `Err` is a store fault.
    pub async fn open_shift(&mut self, opening_float: Money) -> Result<(), LaneError> {
        self.buffer.clear();
        let who = match &self.signin {
            SignIn::Float { id, name } => (id.clone(), name.clone()),
            _ => return self.refuse(LaneError::NoCashier),
        };
        match self.try_open_shift(&who.0, opening_float).await {
            Ok(()) => {
                self.shift_cashier = Some(who.1);
                self.signin = SignIn::Choosing;
                Ok(())
            }
            Err(e) => self.refuse(e),
        }
    }

    /// Test fixture: a shift opened in this id's name without the sign-in.
    #[cfg(test)]
    pub async fn open_shift_as(
        &mut self,
        opened_by: &str,
        opening_float: Money,
    ) -> Result<(), LaneError> {
        self.buffer.clear();
        match self.try_open_shift(opened_by, opening_float).await {
            Ok(()) => Ok(()),
            Err(e) => self.refuse(e),
        }
    }

    async fn try_open_shift(
        &mut self,
        opened_by: &str,
        opening_float: Money,
    ) -> Result<(), LaneError> {
        if self.shift.is_some() {
            return Err(LaneError::ShiftAlreadyOpen);
        }
        let shift = self
            .store
            .open_shift(&self.cfg.dvc_srl_no, opened_by, opening_float)
            .await
            .map_err(|e| match e {
                StoreError::NegativeOpeningFloat => LaneError::NegativeFloat,
                StoreError::ShiftAlreadyOpen { .. } => LaneError::ShiftAlreadyOpen,
                e => LaneError::store(StoreOp::OpenShift, e),
            })?;
        self.wake_sync();
        self.shift = Some(shift);
        self.screen = Screen::Selling;
        self.notice = Notice::Info("Shift opened. Ready to sell.".into());
        Ok(())
    }

    /// What the sign-in screen shows. Only a mask of a PIN ever leaves here.
    pub fn signin(&self) -> SignInView<'_> {
        match &self.signin {
            SignIn::Choosing => SignInView::Choosing {
                cashiers: &self.cashiers,
            },
            SignIn::Pin(p) => SignInView::Pin {
                name: &p.cashier.name,
                mask: "\u{2022}".repeat(p.digits.len()),
            },
            SignIn::Checking(c) => SignInView::Checking {
                name: &c.cashier.name,
            },
            SignIn::Float { name, .. } => SignInView::Float { name },
        }
    }

    /// The name of whoever opened the open shift, for the status bar. Falls
    /// back to the stored id for a shift whose cashier is no longer cached.
    pub fn shift_cashier(&self) -> Option<&str> {
        self.shift_cashier
            .as_deref()
            .or_else(|| self.shift.as_ref().map(|s| s.opened_by.as_str()))
    }

    /// A PIN check is running in the background.
    pub fn is_checking_pin(&self) -> bool {
        matches!(self.signin, SignIn::Checking(_))
    }

    /// Reads the cashier list again if a shift has closed since it was read.
    pub async fn ensure_cashiers(&mut self) -> Result<(), LaneError> {
        if self.cashiers_loaded {
            return Ok(());
        }
        self.cashiers = self
            .store
            .active_cashiers()
            .await
            .map_err(|e| LaneError::store(StoreOp::Cashiers, e))?;
        self.cashiers_loaded = true;
        Ok(())
    }

    /// Enter while choosing a name: the keyed number picks a cashier. Nothing
    /// keyed re-reads the list, in case the cloud has synced new cashiers.
    pub async fn choose_cashier(&mut self) -> Result<(), LaneError> {
        if !matches!(self.signin, SignIn::Choosing) {
            return Ok(());
        }
        let raw = std::mem::take(&mut self.buffer);
        let raw = raw.trim();
        if raw.is_empty() {
            self.cashiers_loaded = false;
            self.ensure_cashiers().await?;
            self.notice = if self.cashiers.is_empty() {
                Notice::Problem(LaneError::NoCashiersSaved.to_string())
            } else {
                Notice::None
            };
            return Ok(());
        }
        let picked = raw
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| self.cashiers.get(i).cloned());
        match picked {
            Some(c) => {
                self.signin = SignIn::Pin(PinEntry::new(c));
                self.notice = Notice::None;
                Ok(())
            }
            None => self.refuse(LaneError::NotACashier),
        }
    }

    /// Esc: back to the list of names, throwing away any PIN keyed so far.
    pub fn back_to_cashiers(&mut self) {
        if self.screen == Screen::NoShift && !matches!(self.signin, SignIn::Checking(_)) {
            self.signin = SignIn::Choosing;
            self.buffer.clear();
            self.notice = Notice::None;
        }
    }

    /// Enter on the PIN: starts the check and returns at once. The argon2
    /// verification is deliberately slow (about 290 ms in a release build), so
    /// it runs on the runtime's blocking pool and the UI thread keeps drawing;
    /// [`LaneState::poll_pin_check`] collects the answer on a later frame.
    ///
    /// A PIN that is empty, too long, or keyed while locked out is refused here
    /// with a `Problem` notice and is not counted as a guess.
    pub fn begin_pin_check(&mut self) -> Result<(), LaneError> {
        let SignIn::Pin(entry) = &mut self.signin else {
            return Ok(());
        };
        let cashier = entry.cashier.clone();
        let digits = std::mem::take(&mut entry.digits);
        let overflow = std::mem::take(&mut entry.overflow);
        self.buffer.clear();
        if let Some(until) = self.locked_until {
            let now = Instant::now();
            if now < until {
                return self.refuse(LaneError::PinLocked {
                    wait: wait_words(until - now),
                });
            }
        }
        if overflow {
            return self.refuse(LaneError::PinTooLong);
        }
        if digits.is_empty() {
            return self.refuse(LaneError::EmptyPin);
        }
        let Ok(handle) = Handle::try_current() else {
            return self.refuse(LaneError::Internal(
                "no async runtime for the PIN check".into(),
            ));
        };
        let (tx, rx) = mpsc::channel();
        let store = self.store.clone();
        let who = cashier.id.clone();
        let rt = handle.clone();
        #[cfg(test)]
        let gate = self.pin_gate.take();
        // The JoinHandle is dropped on purpose: the result comes back on `tx`,
        // and a panic drops `tx`, which `poll_pin_check` reads as a fault.
        drop(handle.spawn_blocking(move || {
            #[cfg(test)]
            if let Some(g) = gate {
                let _ = g.recv_timeout(Duration::from_secs(5));
            }
            let _ = tx.send(rt.block_on(store.verify_pin(&who, &digits)));
        }));
        self.notice = Notice::None;
        self.signin = SignIn::Checking(Checking { cashier, rx });
        Ok(())
    }

    /// Collects a finished PIN check, if there is one. Cheap and non-blocking,
    /// so the UI calls it every frame while [`LaneState::is_checking_pin`].
    ///
    /// A wrong PIN is `Ok` with a `Problem` notice; `Err` is only a fault the
    /// cashier cannot act on, and does not count against them.
    pub fn poll_pin_check(&mut self) -> Result<(), LaneError> {
        let SignIn::Checking(c) = &self.signin else {
            return Ok(());
        };
        let answer = match c.rx.try_recv() {
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                self.signin = SignIn::Choosing;
                return self.refuse(LaneError::Internal("the PIN check did not finish".into()));
            }
            Ok(a) => a,
        };
        let cashier = c.cashier.clone();
        match answer {
            Ok(Some(id)) => {
                self.pin_failures = 0;
                self.locked_until = None;
                self.signin = SignIn::Float {
                    id,
                    name: cashier.name.clone(),
                };
                self.notice = Notice::Info(format!(
                    "Welcome, {}. Count the drawer and key the opening float.",
                    cashier.name
                ));
                Ok(())
            }
            Ok(None) => {
                self.pin_failures = self.pin_failures.saturating_add(1);
                let name = cashier.name.clone();
                let problem = match lockout_for(self.pin_failures) {
                    Some(wait) => {
                        self.locked_until = Some(Instant::now() + wait);
                        LaneError::WrongPinLocked {
                            name,
                            wait: wait_words(wait),
                        }
                    }
                    None => LaneError::WrongPin {
                        name,
                        left: PIN_FREE_TRIES - self.pin_failures,
                    },
                };
                self.signin = SignIn::Pin(PinEntry::new(cashier));
                self.refuse(problem)
            }
            Err(e) => {
                self.signin = SignIn::Pin(PinEntry::new(cashier));
                self.refuse(LaneError::store(StoreOp::VerifyPin, e))
            }
        }
    }

    /// Test fixture: the lockout has run out.
    #[cfg(test)]
    fn expire_lockout(&mut self) {
        self.locked_until = Some(Instant::now());
    }

    /// Closes the shift with the cash the cashier counted. A short (or over)
    /// drawer is NOT refused: the variance is recorded and the shift closes.
    /// The lane then returns to `NoShift`, as a till is left overnight.
    ///
    /// `Ok(Some(..))`: closed. `Ok(None)`: refused for a reason the cashier can
    /// act on (no shift, negative count, a sale in progress or awaiting the
    /// VSDC); the `Problem` notice says which and the shift stays open. `Err`
    /// only for a store fault. Same contract as `settle`.
    pub async fn close_shift(
        &mut self,
        declared_cash: Money,
    ) -> Result<Option<ShiftClose>, LaneError> {
        self.buffer.clear();
        match self.try_close_shift(declared_cash).await {
            Ok(closed) => {
                self.notice = Notice::Info(format!(
                    "Shift closed. Expected {}, declared {}, variance {}.",
                    closed.totals.expected_cash, closed.declared_cash, closed.variance
                ));
                Ok(Some(closed))
            }
            Err(e) => self.refuse(e).map(|()| None),
        }
    }

    async fn try_close_shift(&mut self, declared_cash: Money) -> Result<ShiftClose, LaneError> {
        let Some(shift) = self.shift.as_ref() else {
            return Err(LaneError::NoShift);
        };
        if declared_cash < Money::zero() {
            return Err(LaneError::NegativeDeclaredCash);
        }
        if self.pending.is_some() {
            return Err(LaneError::ShiftHasPendingSale);
        }
        // A settled sale is finished (its basket is only kept for the receipt);
        // anything else holding lines or payments is a customer mid-sale.
        let settled = matches!(self.screen, Screen::Settled { .. });
        if !settled && (!self.basket.is_empty() || !self.tenders.entries().is_empty()) {
            return Err(LaneError::SaleInProgress);
        }
        let closed = match self.store.close_shift(&shift.id, declared_cash).await {
            Ok(c) => c,
            Err(StoreError::ShiftHasPendingSale) => return Err(LaneError::ShiftHasPendingSale),
            Err(StoreError::NegativeDeclaredCash) => return Err(LaneError::NegativeDeclaredCash),
            Err(StoreError::ShiftAlreadyClosed) => {
                // Closed elsewhere: this lane's shift is stale.
                self.end_shift();
                return Err(LaneError::ShiftAlreadyClosed);
            }
            Err(e) => return Err(LaneError::store(StoreOp::CloseShift, e)),
        };
        self.wake_sync();
        self.end_shift();
        Ok(closed)
    }

    /// Prints a shift report: a snapshot of the open shift (per shift, not RRA's
    /// daily X; see `report.rs`). **Changes nothing**:
    /// it only reads the shift's totals, so it may be run as often as wanted,
    /// mid-shift and even mid-sale. It does not touch the scan buffer, the
    /// basket, the tenders or the screen; only the notice reports the outcome.
    ///
    /// Same contract as `reprint`: `Ok` for anything the cashier can act on
    /// (no shift, printer trouble, the `Problem` notice says which); `Err` only
    /// for a store fault.
    pub async fn print_x(&mut self, printer: &mut dyn Printer) -> Result<(), LaneError> {
        let Some(shift) = self.shift.as_ref() else {
            return self.refuse_keeping_buffer(LaneError::NoShift);
        };
        let totals = match self.store.shift_totals(&shift.id).await {
            Ok(t) => t,
            Err(e) => {
                return self.refuse_keeping_buffer(LaneError::store(StoreOp::ShiftTotals, e));
            }
        };
        let mrc = match self.store.shift_mrc(&shift.id).await {
            Ok(m) => m,
            Err(e) => {
                return self.refuse_keeping_buffer(LaneError::store(StoreOp::ShiftTotals, e));
            }
        };
        let bytes = shift_report(&totals, &self.cfg, mrc.as_deref(), &kigali_now());
        self.notice = match printer.write(&bytes) {
            Ok(()) => Notice::Info("Shift report printed. The shift is still open.".into()),
            Err(e) => Notice::Problem(format!(
                "The shift report did not print ({e}). Check the printer and try again; nothing was changed."
            )),
        };
        Ok(())
    }

    /// Closes the shift with the cash the cashier counted and prints the shift
    /// close report. Closing is irreversible: the shift is closed first, and the
    /// report is composed from what closing recorded, so it exists only for a
    /// shift that really closed. The lane then has no open shift, so a second
    /// close is refused (`NoShift`) rather than printing a second closing figure.
    /// This is a per-shift document, not RRA's daily Z (see `report.rs`).
    ///
    /// Refusals are those of [`close_shift`](Self::close_shift) (nothing is
    /// printed, the shift stays open). If the printer then fails, the shift
    /// stays closed, and the notice says so and repeats the figures.
    pub async fn print_z(
        &mut self,
        declared_cash: Money,
        printer: &mut dyn Printer,
    ) -> Result<(), LaneError> {
        // Read before closing: a failure here changes nothing, whereas after the
        // close the shift is gone from `self`.
        let mrc = match self.shift.as_ref() {
            Some(shift) => match self.store.shift_mrc(&shift.id).await {
                Ok(m) => m,
                Err(e) => return self.refuse(LaneError::store(StoreOp::ShiftTotals, e)),
            },
            None => None,
        };
        let Some(closed) = self.close_shift(declared_cash).await? else {
            return Ok(()); // refused: close_shift left the Problem notice
        };
        // Retained before anything prints: the close is the truth, paper follows.
        let at = kigali_now();
        let bytes = shift_close_report(&closed, &self.cfg, mrc.as_deref(), &at);
        let figures = format!(
            "Expected {}, declared {}, variance {}.",
            fmt::rwf(closed.totals.expected_cash),
            fmt::rwf(closed.declared_cash),
            report::variance(closed.variance)
        );
        let last = self.last_z.insert(LastZ {
            close: closed,
            mrc,
            at,
            printed: false,
        });
        self.notice = match printer.write(&bytes) {
            Ok(()) => {
                last.printed = true;
                Notice::Info(format!("Shift closed. Close report printed. {figures}"))
            }
            Err(e) => Notice::Problem(format!(
                "The shift is closed and its figures are final, but the close report did not print ({e}). {figures} Check the printer and reprint it."
            )),
        };
        Ok(())
    }

    /// Prints the last close report again. The first time it reaches paper it is
    /// the original; any later one is marked COPY. Nothing in the store is
    /// written. Normally from memory; after a restart (a jam, then a quit) it is
    /// recomposed from the store's last closed shift on this device, and since
    /// nothing says whether the first attempt reached paper, that one is marked
    /// COPY too.
    pub async fn reprint_z(&mut self, printer: &mut dyn Printer) -> Result<(), LaneError> {
        self.buffer.clear();
        if self.last_z.is_none() {
            let dvc = self.cfg.dvc_srl_no.clone();
            let closed = match self.store.last_closed_shift(&dvc).await {
                Ok(c) => c,
                Err(e) => return self.refuse(LaneError::store(StoreOp::ClosedShift, e)),
            };
            let Some(closed) = closed else {
                return self.refuse(LaneError::NoZToReprint);
            };
            let mrc = match self.store.shift_mrc(&closed.id).await {
                Ok(m) => m,
                Err(e) => return self.refuse(LaneError::store(StoreOp::ClosedShift, e)),
            };
            self.last_z = Some(LastZ {
                close: closed.close,
                mrc,
                at: kigali_now(),
                printed: true,
            });
        }
        let Some(last) = self.last_z.as_mut() else {
            return self.refuse(LaneError::NoZToReprint);
        };
        let mrc = last.mrc.as_deref();
        let bytes = if last.printed {
            shift_close_report_copy(&last.close, &self.cfg, mrc, &last.at)
        } else {
            shift_close_report(&last.close, &self.cfg, mrc, &last.at)
        };
        self.notice = match printer.write(&bytes) {
            Ok(()) => {
                let msg = if last.printed {
                    "Close report reprinted (marked COPY)."
                } else {
                    "Close report printed."
                };
                last.printed = true;
                Notice::Info(msg.into())
            }
            Err(e) => Notice::Problem(format!(
                "The close report did not print ({e}). Check the printer and try again; the shift is closed and its figures are safe."
            )),
        };
        Ok(())
    }

    /// `refuse`, for a read-only report: the half-keyed scan is not lost.
    fn refuse_keeping_buffer(&mut self, e: LaneError) -> Result<(), LaneError> {
        let keep = std::mem::take(&mut self.buffer);
        let r = self.refuse(e);
        self.buffer = keep;
        r
    }

    /// Back to the start of the day: no shift, nothing in the basket.
    fn end_shift(&mut self) {
        self.shift = None;
        self.shift_cashier = None;
        // Whatever was half-keyed is gone with the screen.
        self.signin = SignIn::Choosing;
        self.cashiers_loaded = false;
        self.screen = Screen::NoShift;
        self.basket = Basket::new();
        self.tenders = TenderSet::default();
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
        let raw = raw.trim();

        // A pick from an open list. A one- or two-digit entry is never a valid
        // code (a PLU is 4 or 5 digits, a barcode 8 or 13), so picking can
        // never shadow a scan; anything else falls through and the list closes.
        if self.search.is_some() && looks_like_a_pick(raw) {
            // `looks_like_a_pick` has already proved these are ASCII digits.
            let n = raw
                .parse()
                .map_err(|_| LaneError::BadPick(raw.to_string()))?;
            return self.pick_at(n).await;
        }
        self.search = None;

        let (times, code) = split_times(raw)?;

        // A letter is never part of a code, so it is unambiguously a name.
        if code.chars().any(char::is_alphabetic) {
            return self.search_by_name(code, times).await;
        }
        self.add_by_code(code, times).await
    }

    /// Looks a product up by barcode or PLU and puts it in the basket.
    async fn add_by_code(&mut self, code: &str, times: Option<u32>) -> Result<(), LaneError> {
        let scanned = classify(code, self.cfg.embedded)?;
        let product = self
            .store
            .lookup(&scanned)
            .await
            .map_err(|e| LaneError::store(StoreOp::Scan, e))?
            .ok_or_else(|| LaneError::UnknownCode(code.to_string()))?;
        self.add_product(&product, Some(&scanned), times)
    }

    /// Offers the products whose name matches, or adds the only one that does.
    async fn search_by_name(&mut self, term: &str, times: Option<u32>) -> Result<(), LaneError> {
        if term.chars().count() < MIN_SEARCH_LEN {
            return Err(LaneError::SearchTooShort);
        }
        let matches = self
            .store
            .search_by_name(term, MAX_MATCHES)
            .await
            .map_err(|e| LaneError::store(StoreOp::Scan, e))?;
        match matches.len() {
            0 => Err(LaneError::NoNameMatch(term.to_string())),
            // Exactly one match needs no choosing.
            1 => {
                let p = matches.into_iter().next().expect("one match");
                self.add_product(&p, None, times)
            }
            n => {
                self.notice = Notice::Info(format!(
                    "{n} items match \"{term}\". Key its number and press Enter, or scan the item."
                ));
                self.search = Some(NameSearch {
                    term: term.to_string(),
                    times,
                    matches,
                });
                Ok(())
            }
        }
    }

    /// Takes the nth offered product, counting from 1 as the screen numbers it.
    async fn pick_at(&mut self, n: usize) -> Result<(), LaneError> {
        let search = self.search.take().ok_or(LaneError::NotSelling)?;
        let Some(product) = n.checked_sub(1).and_then(|i| search.matches.get(i)) else {
            return Err(LaneError::BadPick(n.to_string()));
        };
        let product = product.clone();
        self.add_product(&product, None, search.times)
    }

    /// The one place a line reaches the basket, whether it arrived by scan, by
    /// PLU, or by being picked out of a name search.
    fn add_product(
        &mut self,
        product: &CachedProduct,
        scanned: Option<&Scanned>,
        times: Option<u32>,
    ) -> Result<(), LaneError> {
        let line = line_for(product, scanned, times)?;

        // Price it first: a line that cannot be priced must not spend a number.
        line.amounts().map_err(|source| LaneError::Line {
            name: product.name.clone(),
            source,
        })?;
        let qty = line.qty;
        let seq = self.basket.push_line(line)?;
        self.notice = Notice::Info(match times {
            Some(_) => format!("{} x {} added (line {seq}).", qty, product.name),
            None => format!("{} added (line {seq}).", product.name),
        });
        Ok(())
    }
}

/// Whether an entry is a pick from an offered list rather than a code. One or
/// two digits only: no real code is that short.
fn looks_like_a_pick(raw: &str) -> bool {
    matches!(raw.len(), 1 | 2) && raw.chars().all(|c| c.is_ascii_digit())
}

/// Splits `3*1234` into a count and the rest. No separator means no count.
///
/// The count must be a whole number from 1 to [`MAX_TIMES`]; anything else is
/// refused rather than guessed at, because a wrong count is a wrong bill.
fn split_times(raw: &str) -> Result<(Option<u32>, &str), LaneError> {
    let Some((count, rest)) = raw.split_once(TIMES) else {
        return Ok((None, raw));
    };
    let count = count.trim();
    let times: u32 = count
        .parse()
        .map_err(|_| LaneError::BadMultiplier(count.to_string()))?;
    if times == 0 || times > MAX_TIMES {
        return Err(LaneError::BadMultiplier(count.to_string()));
    }
    Ok((Some(times), rest.trim()))
}

/// The lane clock as the CIS block wants it: `yyyymmddhhmmss`, Kigali time.
/// Rwanda is UTC+2 all year (no daylight saving), so no zone database is needed.
/// A tender set holding exactly `entries`, each re-validated by `push`.
fn rebuild(entries: &[TenderEntry]) -> Result<TenderSet, LaneError> {
    let mut set = TenderSet::default();
    for e in entries {
        set.push(*e)?;
    }
    Ok(set)
}

/// The Kigali clock time (`HH:MM`) of a store timestamp (`2026-11-21T04:58:00Z`,
/// UTC). `None` if it is not in that shape.
pub fn kigali_hhmm(utc: &str) -> Option<String> {
    let t = utc.split('T').nth(1)?;
    let (h, rest) = t.split_once(':')?;
    let m = rest.get(..2)?;
    let h: u32 = h.parse().ok().filter(|h| *h < 24)?;
    let m: u32 = m.parse().ok().filter(|m| *m < 60)?;
    Some(format!("{:02}:{:02}", (h + 2) % 24, m))
}

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
/// Builds the basket line for a product.
///
/// `scanned` is `None` when the product was chosen from a name search rather
/// than scanned. That is not the same as a bare PLU and must not pretend to be
/// one: a weighed product picked from a list still needs its scale label, and
/// the check below is what says so.
fn line_for(
    p: &CachedProduct,
    scanned: Option<&Scanned>,
    times: Option<u32>,
) -> Result<BasketLine, LaneError> {
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
    if matches!(scanned, Some(Scanned::WeightEmbedded { .. })) && !p.is_weighed {
        return Err(LaneError::NotWeighed {
            name: p.name.clone(),
        });
    }
    // A count on a weighed product would multiply a weight: three times
    // 1.250 kg is not something a cashier means to key, and the scale label
    // already carries the quantity.
    if times.is_some() && p.is_weighed {
        return Err(LaneError::MultiplierOnWeighed {
            name: p.name.clone(),
        });
    }
    let (qty, prc) = match scanned {
        Some(Scanned::WeightEmbedded { qty: Some(q), .. }) => {
            if !q.is_positive() {
                return Err(LaneError::ZeroWeight {
                    name: p.name.clone(),
                });
            }
            (*q, p.unit_price)
        }
        Some(Scanned::WeightEmbedded {
            price: Some(label_price),
            ..
        }) => (one()?, *label_price),
        Some(Scanned::WeightEmbedded { .. }) => (one()?, p.unit_price),
        // A per-kg product with no weight would sell one kilo of it.
        _ if p.is_weighed => {
            return Err(LaneError::NeedsWeightLabel {
                name: p.name.clone(),
            });
        }
        // A keyed count, or one of the item. Built from a string because Qty
        // owns its own precision and money never passes through a float.
        _ => (count_qty(times)?, p.unit_price),
    };
    Ok(BasketLine {
        item_seq: 0, // assigned by Basket::push_line
        item_cd: p.sku.clone(),
        item_cls_cd,
        item_nm: p.name.clone(),
        bcd: p.barcode.clone(),
        product_id: Some(p.id.clone()),
        qty,
        prc,
        tax_ty_cd: p.tax_ty_cd,
        discount: None,
    })
}

fn one() -> Result<Qty, LaneError> {
    Qty::try_from_str("1").map_err(|e| LaneError::Internal(e.to_string()))
}

/// A keyed count as a quantity, or one when none was keyed.
fn count_qty(times: Option<u32>) -> Result<Qty, LaneError> {
    match times {
        None => one(),
        Some(n) => {
            Qty::try_from_str(&n.to_string()).map_err(|e| LaneError::Internal(e.to_string()))
        }
    }
}

/// Cashiers with cheap argon2 hashes, so tests do not each pay the cloud's
/// 128 MiB; verification reads the parameters from the string.
#[cfg(test)]
pub(crate) mod test_support {
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    use till_store::Cashier;

    pub fn cashier(id: &str, name: &str, pin: &str) -> Cashier {
        let params = Params::new(8, 1, 1, None).unwrap();
        let hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_with_salt(pin.as_bytes(), b"0123456789abcdef")
            .unwrap()
            .to_string();
        Cashier {
            id: id.into(),
            name: name.into(),
            pin_hash: hash,
            active: true,
        }
    }

    /// Alice (PIN 1234) and Bob (PIN 5678).
    pub fn cashiers() -> Vec<Cashier> {
        vec![
            cashier("u-alice", "Alice", "1234"),
            cashier("u-bob", "Bob", "5678"),
        ]
    }
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
        // every sale belongs to an open shift
        store
            .open_shift("DVC1", "cashier", Money::zero())
            .await
            .unwrap();
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

    /// A product with a chosen name, for the name-search tests.
    fn named(id: &str, name: &str, plu: Option<&str>, weighed: bool) -> CachedProduct {
        CachedProduct {
            name: name.into(),
            ..product(id, None, plu, weighed)
        }
    }

    /// Keys `raw` and presses Enter. Returns what `enter` returned, so a test
    /// about a refusal can assert on it rather than unwrapping a panic.
    async fn key(l: &mut LaneState, raw: &str) -> Result<(), LaneError> {
        for c in raw.chars() {
            l.key(c);
        }
        l.enter().await
    }

    // ---- a keyed count (3*CODE) ----

    #[tokio::test]
    async fn a_keyed_count_puts_one_line_on_the_basket_with_that_quantity() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, &format!("3*{EAN}")).await;
        assert_eq!(l.basket().len(), 1, "a count is one line, not three");
        let line = l.basket().lines().last().unwrap();
        assert_eq!(line.qty.to_string(), "3.000");
        // 3 x 1250, priced by the basket and not by the till's arithmetic.
        assert_eq!(line.amounts().unwrap().sply_amt.to_string(), "3750.00");
    }

    #[tokio::test]
    async fn the_count_separator_accepts_space_around_it() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, &format!("2* {EAN}")).await;
        assert_eq!(l.basket().lines().last().unwrap().qty.to_string(), "2.000");
    }

    #[tokio::test]
    async fn a_count_that_is_not_a_whole_positive_number_is_refused() {
        for bad in ["0", "-1", "1.5", "abc", "", &(MAX_TIMES + 1).to_string()] {
            let mut l = lane_with(
                &[product("a", Some(EAN), None, false)],
                EmbeddedMode::Weight,
            )
            .await;
            let _ = key(&mut l, &format!("{bad}*{EAN}")).await;
            assert!(
                l.basket().is_empty(),
                "{bad:?} must not reach the basket, notice was {:?}",
                l.notice()
            );
            assert!(
                matches!(l.notice(), Notice::Problem(_)),
                "{bad:?} must tell the cashier, got {:?}",
                l.notice()
            );
        }
    }

    #[tokio::test]
    async fn the_largest_allowed_count_is_accepted() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, &format!("{MAX_TIMES}*{EAN}")).await;
        assert_eq!(
            l.basket().lines().last().unwrap().qty.to_string(),
            format!("{MAX_TIMES}.000")
        );
    }

    /// Three times a weight is not something a cashier means to key, and the
    /// scale label already carries the quantity.
    #[tokio::test]
    async fn a_count_on_a_weighed_product_is_refused() {
        let mut l = lane_with(
            &[product("23456", None, Some("23456"), true)],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, &format!("3*{WEIGHT_LABEL}")).await;
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("sold by weight"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    // ---- name search ----

    #[tokio::test]
    async fn one_name_match_is_added_without_choosing() {
        let mut l = lane_with(
            &[
                named("a", "Inyange Fresh Milk 1L", None, false),
                named("b", "Kabuye Sugar 1kg", None, false),
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "milk").await;
        assert_eq!(l.basket().len(), 1);
        assert_eq!(
            l.basket().lines().last().unwrap().item_nm,
            "Inyange Fresh Milk 1L"
        );
        assert!(l.search_matches().is_empty(), "nothing left to pick");
    }

    #[tokio::test]
    async fn a_single_name_match_added_directly_carries_the_cloud_product_id() {
        let mut l = lane_with(
            &[
                named("cloud-m", "Inyange Fresh Milk 1L", None, false),
                named("cloud-s", "Kabuye Sugar 1kg", None, false),
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "milk").await;
        assert!(l.search_matches().is_empty(), "no picker for one match");
        let line = l.basket().lines().last().unwrap();
        assert_eq!(line.product_id.as_deref(), Some("cloud-m"));
    }

    #[tokio::test]
    async fn several_name_matches_are_offered_and_picked_by_number() {
        let mut l = lane_with(
            &[
                named("a", "Milk Full Cream 1L", None, false),
                named("b", "Milk Skimmed 1L", None, false),
                named("c", "Kabuye Sugar 1kg", None, false),
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "milk").await;
        assert_eq!(l.search_matches().len(), 2, "two matches must be offered");
        assert!(l.basket().is_empty(), "offering must not add anything");

        // Both names begin with "Milk", so they are offered alphabetically:
        // Full Cream is 1, Skimmed is 2. Picking 2 must take Skimmed.
        assert_eq!(l.search_matches()[1].name, "Milk Skimmed 1L");
        let _ = key(&mut l, "2").await;
        assert_eq!(l.basket().len(), 1);
        assert_eq!(
            l.basket().lines().last().unwrap().item_nm,
            "Milk Skimmed 1L",
            "picking 2 must take the second item offered, not the first"
        );
        assert!(l.search_matches().is_empty(), "the list closes on a pick");
    }

    #[tokio::test]
    async fn a_pick_outside_the_list_is_refused_and_adds_nothing() {
        let mut l = lane_with(
            &[
                named("a", "Milk Full Cream 1L", None, false),
                named("b", "Milk Skimmed 1L", None, false),
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "milk").await;
        let _ = key(&mut l, "9").await;
        assert!(l.basket().is_empty());
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    /// Scanning while a list is open must ring the scan up, not be read as a
    /// pick. This is why a pick is only one or two digits.
    #[tokio::test]
    async fn a_scan_while_a_list_is_open_rings_up_and_closes_the_list() {
        let mut l = lane_with(
            &[
                named("a", "Milk Full Cream 1L", None, false),
                named("b", "Milk Skimmed 1L", None, false),
                CachedProduct {
                    barcode: Some(EAN.into()),
                    ..named("c", "Something Else", None, false)
                },
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "milk").await;
        assert_eq!(l.search_matches().len(), 2);
        let _ = key(&mut l, EAN).await;
        assert_eq!(l.basket().len(), 1);
        assert_eq!(l.basket().lines().last().unwrap().item_nm, "Something Else");
        assert!(l.search_matches().is_empty());
    }

    #[tokio::test]
    async fn a_name_that_matches_nothing_is_a_problem_the_cashier_can_act_on() {
        let mut l = lane_with(&[named("a", "Milk", None, false)], EmbeddedMode::Weight).await;
        let _ = key(&mut l, "zzzz").await;
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("zzzz"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn one_letter_is_not_a_search() {
        let mut l = lane_with(&[named("a", "Milk", None, false)], EmbeddedMode::Weight).await;
        let _ = key(&mut l, "m").await;
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("at least"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    /// The two features compose: a count keyed before a name is held and
    /// applied to whatever comes out of the list.
    #[tokio::test]
    async fn a_count_survives_a_name_search_and_applies_to_the_pick() {
        let mut l = lane_with(
            &[
                named("a", "Milk Full Cream 1L", None, false),
                named("b", "Milk Skimmed 1L", None, false),
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "4*milk").await;
        assert_eq!(l.search_matches().len(), 2);
        assert_eq!(l.search_times(), Some(4), "the count must be held");
        let _ = key(&mut l, "1").await;
        assert_eq!(l.basket().lines().last().unwrap().qty.to_string(), "4.000");
    }

    #[tokio::test]
    async fn a_count_with_a_single_name_match_needs_no_pick() {
        let mut l = lane_with(&[named("a", "Milk", None, false)], EmbeddedMode::Weight).await;
        let _ = key(&mut l, "5*milk").await;
        assert_eq!(l.basket().lines().last().unwrap().qty.to_string(), "5.000");
    }

    /// A weighed product found by name still needs its scale label: picking it
    /// off a list is not a substitute for weighing it.
    #[tokio::test]
    async fn a_weighed_product_picked_by_name_still_asks_for_the_scale_label() {
        let mut l = lane_with(
            &[named("t", "Tomatoes", Some("4087"), true)],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "tomato").await;
        assert!(l.basket().is_empty());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("Weigh it"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn selling_by_name_needs_an_open_shift_like_any_other_sale() {
        let mut l = lane_without_shift().await;
        let _ = key(&mut l, "item").await;
        assert!(l.basket().is_empty());
        assert!(matches!(l.screen(), Screen::NoShift));
    }

    fn test_config() -> LaneConfig {
        LaneConfig {
            dvc_srl_no: "DVC1".into(),
            bhf_id: "00".into(),
            embedded: EmbeddedMode::Weight,
            counter: Counter::SaleInvc,
            shop_name: "Simplex Mart".into(),
            shop_address: "KG 7 Ave, Kigali".into(),
            shop_tin: "123456789".into(),
        }
    }

    /// A lane over a catalogue with one product and NO shift open.
    async fn lane_without_shift() -> LaneState {
        let store = Store::open_in_memory().await.unwrap();
        store
            .sync_catalogue(&[product("a", Some(EAN), None, false)])
            .await
            .unwrap();
        LaneState::new(store, test_config()).await.unwrap()
    }

    #[tokio::test]
    async fn a_lane_with_no_open_shift_starts_on_the_no_shift_screen() {
        let l = lane_without_shift().await;
        assert_eq!(l.screen(), &Screen::NoShift);
        assert!(l.shift().is_none());
        assert_eq!(l.notice(), &Notice::None);
    }

    #[tokio::test]
    async fn scanning_with_no_shift_open_is_refused_and_adds_nothing() {
        let mut l = lane_without_shift().await;
        // Ok(()), not Err: the cashier can act on this.
        scan(&mut l, EAN).await.unwrap();
        assert!(l.basket().is_empty());
        assert_eq!(l.buffer(), "");
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("Open a shift"), "{m}"),
            n => panic!("expected a problem, got {n:?}"),
        }
        assert_eq!(l.screen(), &Screen::NoShift);
    }

    #[tokio::test]
    async fn opening_a_shift_moves_the_lane_to_selling() {
        let mut l = lane_without_shift().await;
        l.open_shift_as("alice", rwf("5000")).await.unwrap();
        assert_eq!(l.screen(), &Screen::Selling);
        let s = l.shift().expect("the lane holds its shift");
        assert_eq!(s.opened_by, "alice");
        assert_eq!(s.opening_float, rwf("5000"));
        scan(&mut l, EAN).await.unwrap();
        assert_eq!(l.basket().len(), 1);
    }

    #[tokio::test]
    async fn a_sale_is_recorded_against_the_lanes_own_shift() {
        let mut l = lane_without_shift().await;
        l.open_shift_as("alice", Money::zero()).await.unwrap();
        let id = l.shift().unwrap().id.clone();
        scan(&mut l, EAN).await.unwrap();
        l.add_tender(PmtTyCd::Cash, rwf("5000")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(matches!(l.screen(), Screen::Settled { .. }));
        let sale_shift: String = sqlx::query_scalar("select shift_id from sale")
            .fetch_one(l.store.pool())
            .await
            .unwrap();
        assert_eq!(sale_shift, id);
    }

    #[tokio::test]
    async fn a_negative_opening_float_is_a_problem_and_the_lane_stays_closed() {
        let mut l = lane_without_shift().await;
        l.open_shift_as("alice", rwf("-1")).await.unwrap();
        assert_eq!(l.screen(), &Screen::NoShift);
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    #[tokio::test]
    async fn opening_a_second_shift_is_a_problem_and_keeps_the_first() {
        let mut l = lane_without_shift().await;
        l.open_shift_as("alice", rwf("100")).await.unwrap();
        l.open_shift_as("bob", rwf("200")).await.unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.shift().unwrap().opened_by, "alice");
    }

    #[tokio::test]
    async fn a_store_fault_opening_a_shift_is_err() {
        let mut l = lane_without_shift().await;
        l.store.pool().close().await;
        let r = l.open_shift_as("alice", Money::zero()).await;
        assert!(matches!(r, Err(LaneError::Store(_))), "{r:?}");
        assert_eq!(l.screen(), &Screen::NoShift);
    }

    #[tokio::test]
    async fn a_lane_resumes_an_existing_open_shift_on_startup() {
        let store = Store::open_in_memory().await.unwrap();
        let opened = store
            .open_shift("DVC1", "alice", rwf("5000"))
            .await
            .unwrap();
        let l = LaneState::new(store, test_config()).await.unwrap();
        assert_eq!(l.screen(), &Screen::Selling);
        assert_eq!(l.shift(), Some(&opened));
        assert_eq!(l.notice(), &Notice::None);
    }

    #[tokio::test]
    async fn closing_a_short_till_succeeds_and_returns_to_no_shift() {
        let mut l = lane_without_shift().await;
        l.open_shift_as("alice", rwf("1000")).await.unwrap();
        scan(&mut l, EAN).await.unwrap();
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        // expected 2250; the cashier counts 250
        let c = l
            .close_shift(rwf("250"))
            .await
            .unwrap()
            .expect("a short till closes");
        assert_eq!(c.totals.expected_cash, rwf("2250"));
        assert_eq!(c.variance, rwf("-2000"));
        assert_eq!(l.screen(), &Screen::NoShift);
        assert!(l.shift().is_none());
        assert!(matches!(l.notice(), Notice::Info(_)));
        // the next morning: a new shift opens
        l.open_shift_as("bob", rwf("0")).await.unwrap();
        assert_eq!(l.screen(), &Screen::Selling);
    }

    #[tokio::test]
    async fn closing_with_a_sale_awaiting_the_vsdc_is_refused_and_the_shift_stays_open() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = fiscal(SimMode::Reject("894 No".into()));
        l.settle(&mut f, &mut p, &mut d).await.unwrap();
        let r = l.close_shift(rwf("0")).await;
        assert!(matches!(r, Ok(None)), "{r:?}");
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("waiting on the VSDC"), "{m}"),
            n => panic!("expected a problem, got {n:?}"),
        }
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(l.screen(), &Screen::Selling);
        assert!(l.shift().is_some());
    }

    #[tokio::test]
    async fn closing_with_no_shift_or_a_negative_count_is_refused() {
        let mut l = lane_without_shift().await;
        assert!(matches!(l.close_shift(rwf("0")).await, Ok(None)));
        assert!(matches!(l.notice(), Notice::Problem(_)));
        l.open_shift_as("alice", rwf("0")).await.unwrap();
        assert!(matches!(l.close_shift(rwf("-1")).await, Ok(None)));
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert!(l.shift().is_some());
    }

    #[tokio::test]
    async fn closing_over_a_half_rung_basket_is_refused_and_keeps_it() {
        let mut l = lane_with_item().await;
        assert!(matches!(l.close_shift(rwf("0")).await, Ok(None)));
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("in progress"), "{m}"),
            n => panic!("expected a problem, got {n:?}"),
        }
        assert_eq!(l.basket().len(), 1);
        assert_eq!(l.screen(), &Screen::Selling);
        // payments keyed against an empty basket also count as a sale in progress
        l.void_line(1).await.unwrap();
        l.add_tender(PmtTyCd::Cash, rwf("5")).unwrap();
        assert!(matches!(l.close_shift(rwf("0")).await, Ok(None)));
        // once cleared, a short count still closes
        l.clear_tenders().unwrap();
        let c = l.close_shift(rwf("0")).await.unwrap().unwrap();
        assert_eq!(c.variance, Money::zero());
    }

    #[tokio::test]
    async fn closing_from_a_settled_screen_is_allowed_and_the_receipt_survives() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(l.close_shift(rwf("1000")).await.unwrap().is_some());
        assert_eq!(l.screen(), &Screen::NoShift);
        assert!(l.last_receipt().is_some());
    }

    #[tokio::test]
    async fn a_store_fault_closing_a_shift_leaves_the_lane_open() {
        let mut l = lane_without_shift().await;
        l.open_shift_as("alice", rwf("0")).await.unwrap();
        l.store.pool().close().await;
        let r = l.close_shift(rwf("0")).await;
        assert!(matches!(r, Err(LaneError::Store(_))), "{r:?}");
        assert!(l.shift().is_some());
        assert_eq!(l.screen(), &Screen::Selling);
    }

    // ---- shift reports ----

    /// The report text as printed, shorn of printer commands.
    fn printed(p: &SimPrinter) -> String {
        crate::report::printable(&p.written)
    }

    /// One 1250 sale paid with `code`, then on to the next customer.
    async fn sell_one(l: &mut LaneState, code: PmtTyCd) {
        scan(l, EAN).await.unwrap();
        l.add_tender(code, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        l.next_customer().unwrap();
    }

    #[tokio::test]
    async fn an_x_report_does_not_close_the_shift() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        let id = l.shift().unwrap().id.clone();
        sell_one(&mut l, PmtTyCd::Cash).await;
        let before = l.store.shift_totals(&id).await.unwrap();
        let (mut p, _) = devices();
        l.print_x(&mut p).await.unwrap();
        l.print_x(&mut p).await.unwrap();
        // Open throughout, and the store is exactly as it was.
        assert_eq!(l.screen(), &Screen::Selling);
        assert_eq!(l.shift().map(|s| s.id.clone()), Some(id.clone()));
        assert_eq!(l.store.shift_totals(&id).await.unwrap(), before);
        assert!(l.store.current_shift("DVC1").await.unwrap().is_some());
        // Two reports, identical, because nothing moved between them.
        // (Minus the PRINTED clock line, which may tick between the two.)
        let both = printed(&p);
        assert!(both.contains("SHIFT REPORT"));
        let figures = |t: &str| {
            t.lines()
                .filter(|l| !l.starts_with("PRINTED"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (first, second) = both.split_at(both.len() / 2);
        assert_eq!(figures(first), figures(second));
        // And the shift still sells.
        sell_one(&mut l, PmtTyCd::Card).await;
        assert_eq!(l.store.shift_totals(&id).await.unwrap().sale_count, 2);
    }

    #[tokio::test]
    async fn an_x_report_mid_sale_keeps_the_basket_and_the_half_keyed_scan() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("500")).unwrap();
        "12".chars().for_each(|c| l.key(c));
        let (mut p, _) = devices();
        l.print_x(&mut p).await.unwrap();
        assert_eq!(l.basket().len(), 1);
        assert_eq!(l.tenders().entries().len(), 1);
        assert_eq!(l.buffer(), "12");
        assert!(!p.written.is_empty());
    }

    #[tokio::test]
    async fn an_x_report_with_no_shift_is_refused_and_prints_nothing() {
        let mut l = lane_without_shift().await;
        let (mut p, _) = devices();
        l.print_x(&mut p).await.unwrap();
        assert!(p.written.is_empty());
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    #[tokio::test]
    async fn a_z_report_closes_the_shift_and_a_second_z_is_refused() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        sell_one(&mut l, PmtTyCd::Cash).await;
        let (mut p, _) = devices();
        l.print_z(rwf("1250"), &mut p).await.unwrap();
        assert!(printed(&p).contains("SHIFT CLOSE REPORT"));
        assert_eq!(l.screen(), &Screen::NoShift);
        assert!(l.shift().is_none());
        assert!(l.store.current_shift("DVC1").await.unwrap().is_none());
        let once = p.written.clone();
        // A second close has no shift to close: refused, nothing printed.
        l.print_z(rwf("999"), &mut p).await.unwrap();
        assert_eq!(p.written, once);
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("No shift is open"), "{m}"),
            n => panic!("expected a problem, got {n:?}"),
        }
        // Nor does an X find a shift to report on.
        l.print_x(&mut p).await.unwrap();
        assert_eq!(p.written, once);
    }

    #[tokio::test]
    async fn a_refused_z_prints_nothing_and_leaves_the_shift_open() {
        let mut l = lane_with_item().await; // a sale is in progress
        let (mut p, _) = devices();
        l.print_z(rwf("0"), &mut p).await.unwrap();
        assert!(p.written.is_empty());
        assert!(l.shift().is_some());
        l.void_line(1).await.unwrap();
        l.print_z(rwf("-1"), &mut p).await.unwrap();
        assert!(p.written.is_empty() && l.shift().is_some());
    }

    #[tokio::test]
    async fn a_z_whose_printer_fails_still_closes_the_shift_and_says_so() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        let (mut p, _) = devices();
        p.fail_next = true;
        l.print_z(rwf("0"), &mut p).await.unwrap();
        assert!(l.shift().is_none());
        match l.notice() {
            Notice::Problem(m) => assert!(m.contains("closed") && m.contains("variance"), "{m}"),
            n => panic!("expected a problem, got {n:?}"),
        }
        assert!(p.written.is_empty());
        assert!(l.store.current_shift("DVC1").await.unwrap().is_none());
        // The close report is not lost: a reprint emits the very bytes the first attempt
        // would have (it never reached paper, so it is the original).
        let wanted = shift_close_report(
            &l.last_z.as_ref().unwrap().close,
            &l.cfg,
            l.last_z.as_ref().unwrap().mrc.as_deref(),
            &l.last_z.as_ref().unwrap().at,
        );
        l.reprint_z(&mut p).await.unwrap();
        assert_eq!(p.written, wanted);
        assert!(matches!(l.notice(), Notice::Info(_)));
        // Once on paper, a further one is a marked COPY, and the shift stays closed.
        l.reprint_z(&mut p).await.unwrap();
        assert!(printed(&p).contains("COPY - SHIFT CLOSED"));
        assert!(l.shift().is_none());
        // A second close is still refused.
        let n = p.written.len();
        l.print_z(rwf("0"), &mut p).await.unwrap();
        assert_eq!(p.written.len(), n);
    }

    #[tokio::test]
    async fn a_jammed_close_report_survives_a_restart() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        sell_one(&mut l, PmtTyCd::Cash).await;
        let (mut p, _) = devices();
        p.fail_next = true;
        l.print_z(rwf("1000"), &mut p).await.unwrap();
        assert!(p.written.is_empty());
        // Quit and start again: nothing is held in memory, only the store.
        let mut again = LaneState::new(l.store.clone(), l.cfg.clone())
            .await
            .unwrap();
        assert!(again.last_z.is_none() && again.shift().is_none());
        let (mut q, _) = devices();
        again.reprint_z(&mut q).await.unwrap();
        let t = printed(&q);
        assert!(
            t.contains("SHIFT CLOSE REPORT") && t.contains("COPY - SHIFT CLOSED"),
            "{t}"
        );
        assert!(t.contains("VARIANCE (SHORT)") && t.contains("-250"), "{t}");
        assert!(t.contains("DECLARED CASH") && t.contains("1,000"), "{t}");
        assert!(
            !t.contains("NONE YET"),
            "the shift had a confirmed sale: {t}"
        );
        assert!(matches!(again.notice(), Notice::Info(_)));
    }

    /// Records, rather than guards, that rebuilding the tenders in another order
    /// cannot misreport the payment method to RRA: `header_code` sums per code
    /// and breaks ties on the lower wire code, so it is order-independent.
    #[test]
    fn the_header_payment_code_does_not_depend_on_tender_order() {
        use till_core::tender::{TenderEntry, TenderSet};
        type Case<'a> = (&'a [(PmtTyCd, &'a str)], &'a str, PmtTyCd);
        let cases: [Case; 3] = [
            // 1200 cash beats the largest single entry (card 700).
            (
                &[
                    (PmtTyCd::Cash, "600"),
                    (PmtTyCd::Cash, "600"),
                    (PmtTyCd::Card, "700"),
                ],
                "1900",
                PmtTyCd::Cash,
            ),
            // A tie goes to the lower wire code (01 before 05).
            (
                &[(PmtTyCd::Card, "500"), (PmtTyCd::Cash, "500")],
                "1000",
                PmtTyCd::Cash,
            ),
            (
                &[
                    (PmtTyCd::MobileMoney, "800"),
                    (PmtTyCd::Card, "800"),
                    (PmtTyCd::Cash, "400"),
                ],
                "2000",
                PmtTyCd::Card,
            ),
        ];
        for (entries, due, want) in cases {
            let mut orders = vec![entries.to_vec()];
            // Every permutation, by Heap's algorithm.
            let mut v = entries.to_vec();
            let mut c = vec![0usize; v.len()];
            let mut i = 0;
            while i < v.len() {
                if c[i] < i {
                    v.swap(if i % 2 == 0 { 0 } else { c[i] }, i);
                    orders.push(v.clone());
                    c[i] += 1;
                    i = 0;
                } else {
                    c[i] = 0;
                    i += 1;
                }
            }
            assert!(orders.len() >= 2);
            for order in orders {
                let mut set = TenderSet::default();
                for (code, amount) in &order {
                    set.push(TenderEntry {
                        pmt_ty_cd: *code,
                        amount: rwf(amount),
                    })
                    .unwrap();
                }
                let settled = set.settle(rwf(due)).unwrap();
                assert_eq!(settled.header_pmt_ty_cd, want, "order {order:?}");
            }
        }
    }

    #[test]
    fn the_opening_time_is_shown_on_the_kigali_clock() {
        assert_eq!(
            kigali_hhmm("2026-11-21T04:58:00Z").as_deref(),
            Some("06:58")
        );
        assert_eq!(
            kigali_hhmm("2026-11-21T23:05:09Z").as_deref(),
            Some("01:05")
        );
        assert_eq!(kigali_hhmm("garbage"), None);
    }

    #[tokio::test]
    async fn reprinting_a_z_with_none_closed_is_a_problem() {
        let mut l = lane_without_shift().await;
        let (mut p, _) = devices();
        l.reprint_z(&mut p).await.unwrap();
        assert!(p.written.is_empty() && matches!(l.notice(), Notice::Problem(_)));
    }

    #[tokio::test]
    async fn both_reports_show_every_tender_code_that_was_used() {
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        sell_one(&mut l, PmtTyCd::Cash).await;
        sell_one(&mut l, PmtTyCd::Card).await;
        sell_one(&mut l, PmtTyCd::MobileMoney).await;
        let (mut x, _) = devices();
        l.print_x(&mut x).await.unwrap();
        let (mut z, _) = devices();
        l.print_z(rwf("2250"), &mut z).await.unwrap();
        for p in [&x, &z] {
            let t = printed(p);
            for needle in ["01 CASH", "05 CARD", "06 MOBILE MONEY"] {
                assert!(t.contains(needle), "{needle} in\n{t}");
            }
            // Codes nobody used do not appear.
            assert!(!t.contains("02 CREDIT") && !t.contains("07 OTHER"), "{t}");
            assert!(t.find("01 CASH") < t.find("05 CARD"));
            assert!(t.find("05 CARD") < t.find("06 MOBILE MONEY"));
        }
    }

    #[tokio::test]
    async fn the_z_report_prints_the_variance_with_its_sign() {
        for (counted, want) in [("250", "-2,000"), ("4250", "+2,000"), ("2250", "0")] {
            let mut l = lane_without_shift().await;
            l.open_shift_as("alice", rwf("1000")).await.unwrap();
            scan(&mut l, EAN).await.unwrap();
            l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
            let (mut p, mut d) = devices();
            l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
                .await
                .unwrap();
            let (mut z, _) = devices();
            l.print_z(rwf(counted), &mut z).await.unwrap();
            let t = printed(&z);
            let row = t.lines().find(|r| r.starts_with("VARIANCE")).expect(&t);
            assert!(row.ends_with(want), "{row:?}");
            assert_eq!(row.contains('-'), want.starts_with('-'), "{row:?}");
        }
    }

    #[tokio::test]
    async fn neither_report_carries_a_qr_or_an_sdc_block() {
        // Not fiscal documents: assert the absence, so nobody "helpfully" adds a
        // QR later and makes an internal report look like a tax receipt.
        let mut l = lane_with(
            &[product("a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        sell_one(&mut l, PmtTyCd::Cash).await;
        let (mut x, _) = devices();
        l.print_x(&mut x).await.unwrap();
        let (mut z, _) = devices();
        l.print_z(rwf("1250"), &mut z).await.unwrap();
        for p in [&x, &z] {
            // GS ( k is the QR store/print command family.
            assert!(!p.written.windows(3).any(|w| w == [0x1D, 0x28, 0x6B]));
            let t = printed(p);
            for banned in [
                "SDC",
                "Internal Data",
                "Receipt Signature",
                "RECEIPT NUMBER",
                "THANK YOU",
            ] {
                assert!(!t.contains(banned), "{banned} in\n{t}");
            }
            assert!(t.contains("NOT A TAX RECEIPT"));
            // The MRC is required content (CIS 18.1.4/19.1.4), not part of the
            // SDC block: matched on its value, so punctuation may change.
            assert!(t.contains("SIM_MRC"), "MRC value in\n{t}");
        }
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
    async fn a_scanned_line_carries_the_cloud_product_id() {
        let mut l = lane_with(
            &[product("cloud-a", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, EAN).await.unwrap();
        let line = l.basket().lines().last().unwrap();
        assert_eq!(line.product_id.as_deref(), Some("cloud-a"));
    }

    #[tokio::test]
    async fn a_line_keyed_by_plu_carries_it_too() {
        let mut l = lane_with(
            &[product("cloud-p", None, Some("4001"), false)],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "4001").await;
        let line = l.basket().lines().last().unwrap();
        assert_eq!(line.product_id.as_deref(), Some("cloud-p"));
    }

    #[tokio::test]
    async fn a_line_picked_by_name_carries_it_too() {
        let mut l = lane_with(
            &[
                named("cloud-a", "Milk Full Cream 1L", None, false),
                named("cloud-b", "Milk Skimmed 1L", None, false),
            ],
            EmbeddedMode::Weight,
        )
        .await;
        let _ = key(&mut l, "milk").await;
        let _ = key(&mut l, "2").await;
        let line = l.basket().lines().last().unwrap();
        assert_eq!(line.product_id.as_deref(), Some("cloud-b"));
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
        // A scan only reaches the catalogue when a shift is open.
        store
            .open_shift("DVC1", "cashier", Money::zero())
            .await
            .unwrap();
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
    async fn a_scanned_product_reaches_the_queued_payload_with_its_cloud_id() {
        // Crosses the lane -> store seam: scan, settle, then read the payload
        // the sync queue actually holds.
        let mut l = lane_with(
            &[product("cloud-e2e", Some(EAN), None, false)],
            EmbeddedMode::Weight,
        )
        .await;
        scan(&mut l, EAN).await.unwrap();
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        let payload: String =
            sqlx::query_scalar("select payload from outbound where kind = 'sale'")
                .fetch_one(l.store.pool())
                .await
                .unwrap();
        assert!(
            payload.contains(r#""productId":"cloud-e2e""#),
            "the scanned product's cloud id must be in the payload: {payload}"
        );
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

    fn kept(l: &LaneState) -> Vec<(PmtTyCd, Money)> {
        l.tenders()
            .entries()
            .iter()
            .map(|e| (e.pmt_ty_cd, e.amount))
            .collect()
    }

    #[tokio::test]
    async fn the_last_tender_can_be_taken_back_before_settling() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Card, rwf("500")).unwrap();
        l.add_tender(PmtTyCd::MobileMoney, rwf("300")).unwrap();
        l.remove_last_tender().unwrap();
        assert_eq!(kept(&l), vec![(PmtTyCd::Card, rwf("500"))]);
        assert_eq!(l.tender_total(), Some(rwf("500")));
        assert!(matches!(l.notice(), Notice::Info(_)));
        assert_eq!(l.remaining(rwf("1250")), Some(rwf("750")));
    }

    #[tokio::test]
    async fn clearing_tenders_leaves_the_basket_untouched() {
        // The lines are the customer's shopping; only the payment was wrong.
        let mut l = lane_with_item().await;
        scan(&mut l, EAN).await.unwrap();
        l.add_tender(PmtTyCd::Card, rwf("500")).unwrap();
        l.add_tender(PmtTyCd::Cash, rwf("100")).unwrap();
        let lines_before = l.basket().len();
        l.clear_tenders().unwrap();
        assert!(l.tenders().entries().is_empty());
        assert_eq!(l.tender_total(), Some(Money::zero()));
        assert_eq!(l.basket().len(), lines_before);
        assert_eq!(l.screen(), &Screen::Selling);
    }

    #[tokio::test]
    async fn a_mis_keyed_card_overpayment_can_be_corrected() {
        // The hazard, end to end: card 8,000 against 7,450 due, remove it,
        // tender 7,450 by card instead, settle. No overcharge.
        let mut l = lane_with_item().await;
        // Six lines of 1,250: the figures differ from the brief's 7,450 but the
        // shape is the same, a card overpayment of 550 that cannot be paid out.
        for _ in 0..5 {
            scan(&mut l, EAN).await.unwrap();
        }
        let due = l.basket().totals().unwrap().tot_amt;
        let over = due.checked_add(rwf("550")).unwrap();
        l.add_tender(PmtTyCd::Card, over).unwrap();
        let (mut p, mut d) = devices();
        let r = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap();
        assert!(r.is_none(), "an overpayment on card must not settle");
        l.remove_last_tender().unwrap();
        l.add_tender(PmtTyCd::Card, due).unwrap();
        let s = l
            .settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        assert_eq!(s.change, Money::zero());
        assert_eq!(kept(&l), vec![(PmtTyCd::Card, due)]);
    }

    #[tokio::test]
    async fn tenders_cannot_be_removed_once_the_sale_is_settled() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        l.settle(&mut fiscal(SimMode::Succeed), &mut p, &mut d)
            .await
            .unwrap()
            .expect("settled");
        l.remove_last_tender().unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        l.clear_tenders().unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(kept(&l), vec![(PmtTyCd::Cash, rwf("1250"))]);
        assert!(matches!(l.screen(), Screen::Settled { .. }));
    }

    #[tokio::test]
    async fn tenders_cannot_be_removed_while_the_sale_is_pending_at_the_vsdc() {
        let mut l = lane_with_item().await;
        l.add_tender(PmtTyCd::Cash, rwf("1250")).unwrap();
        let (mut p, mut d) = devices();
        let mut f = fiscal(SimMode::Reject("894 Bad item class".into()));
        assert!(l.settle(&mut f, &mut p, &mut d).await.unwrap().is_none());
        l.remove_last_tender().unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        l.clear_tenders().unwrap();
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert_eq!(kept(&l), vec![(PmtTyCd::Cash, rwf("1250"))]);
    }

    #[tokio::test]
    async fn removing_from_an_empty_tender_set_is_a_problem_not_an_error() {
        let mut l = lane_with_item().await;
        assert!(l.remove_last_tender().is_ok());
        assert!(matches!(l.notice(), Notice::Problem(_)));
        assert!(l.clear_tenders().is_ok());
        assert!(matches!(l.notice(), Notice::Problem(_)));
    }

    #[tokio::test]
    async fn remaining_is_what_is_still_owed_and_none_once_covered() {
        let mut l = lane_with_item().await;
        assert_eq!(l.remaining(rwf("1250")), Some(rwf("1250")));
        l.add_tender(PmtTyCd::Card, rwf("1000")).unwrap();
        assert_eq!(l.remaining(rwf("1250")), Some(rwf("250")));
        l.add_tender(PmtTyCd::Cash, rwf("250")).unwrap();
        assert_eq!(l.remaining(rwf("1250")), None);
    }

    #[test]
    fn the_cis_clock_is_kigali_time() {
        // 2026-11-21 22:30:05 UTC is 00:30:05 on the 22nd in Kigali.
        assert_eq!(kigali_stamp(1_795_300_205), "20261122003005");
        assert_eq!(kigali_stamp(0), "19700101020000");
        // 2024-02-29 (leap day) 12:00:00 UTC.
        assert_eq!(kigali_stamp(1_709_208_000), "20240229140000");
    }

    // ------------------------------------------------------------ sign-in

    /// A lane with no shift and two cashiers cached: Alice (1234), Bob (5678).
    async fn lane_with_cashiers() -> LaneState {
        let store = Store::open_in_memory().await.unwrap();
        store
            .sync_catalogue(&[product("a", Some(EAN), None, false)])
            .await
            .unwrap();
        store
            .sync_cashiers(&crate::lane::test_support::cashiers())
            .await
            .unwrap();
        LaneState::new(store, test_config()).await.unwrap()
    }

    fn type_digits(l: &mut LaneState, s: &str) {
        s.chars().for_each(|c| l.key(c));
    }

    /// Waits for the background PIN check, as the UI does frame by frame.
    fn finish_check(l: &mut LaneState) -> Result<(), LaneError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while l.is_checking_pin() {
            assert!(Instant::now() < deadline, "the PIN check never finished");
            l.poll_pin_check()?;
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    /// Choose Alice (number 1), key a PIN, wait for the answer.
    async fn try_pin(l: &mut LaneState, who: &str, pin: &str) -> Result<(), LaneError> {
        l.back_to_cashiers();
        type_digits(l, who);
        l.choose_cashier().await?;
        type_digits(l, pin);
        l.begin_pin_check()?;
        finish_check(l)
    }

    fn problem(l: &LaneState) -> String {
        match l.notice() {
            Notice::Problem(m) => m.clone(),
            n => panic!("expected a problem, got {n:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn opening_a_shift_requires_a_cashier_and_a_pin() {
        let mut l = lane_with_cashiers().await;
        // No name chosen, no PIN: nothing opens.
        l.open_shift(Money::zero()).await.unwrap();
        assert_eq!(l.screen(), &Screen::NoShift);
        assert!(l.shift().is_none());
        assert!(problem(&l).contains("PIN"), "{}", problem(&l));
        // A name alone is not enough.
        type_digits(&mut l, "1");
        l.choose_cashier().await.unwrap();
        assert!(matches!(l.signin(), SignInView::Pin { name: "Alice", .. }));
        l.open_shift(Money::zero()).await.unwrap();
        assert_eq!(l.screen(), &Screen::NoShift);
        // Name and PIN: the float step opens, and only then does the shift.
        type_digits(&mut l, "1234");
        l.begin_pin_check().unwrap();
        finish_check(&mut l).unwrap();
        assert!(matches!(l.signin(), SignInView::Float { name: "Alice" }));
        l.open_shift(rwf("5000")).await.unwrap();
        assert_eq!(l.screen(), &Screen::Selling);
        assert_eq!(l.shift_cashier(), Some("Alice"));
        // After the shift closes the next cashier starts from the list again.
        l.close_shift(rwf("5000")).await.unwrap();
        assert!(matches!(l.signin(), SignInView::Choosing { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_pin_is_a_problem_the_cashier_can_act_on_not_an_err() {
        let mut l = lane_with_cashiers().await;
        let r = try_pin(&mut l, "1", "9999").await;
        assert!(r.is_ok(), "a wrong PIN is not an Err: {r:?}");
        let m = problem(&l);
        assert!(m.contains("Wrong PIN") && m.contains("Alice"), "{m}");
        assert!(!m.contains("9999"), "the message never echoes the PIN: {m}");
        assert_eq!(l.screen(), &Screen::NoShift);
        // The cashier can simply try again with the right one.
        type_digits(&mut l, "1234");
        l.begin_pin_check().unwrap();
        finish_check(&mut l).unwrap();
        assert!(matches!(l.signin(), SignInView::Float { .. }));
        // An empty or overlong entry is a problem too, and costs no attempt.
        let mut l = lane_with_cashiers().await;
        type_digits(&mut l, "1");
        l.choose_cashier().await.unwrap();
        assert!(l.begin_pin_check().is_ok());
        assert!(problem(&l).contains("Key your PIN"));
        type_digits(&mut l, "5612345000017");
        assert!(l.begin_pin_check().is_ok());
        assert!(problem(&l).contains("too long"), "{}", problem(&l));
        assert_eq!(l.pin_failures, 0, "neither was counted as a guess");
        // A cashier number that is not on the list is a problem, not an Err.
        l.back_to_cashiers();
        type_digits(&mut l, "9");
        assert!(l.choose_cashier().await.is_ok());
        assert!(problem(&l).contains("not a number on the list"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_opened_shift_records_the_cloud_user_id() {
        let mut l = lane_with_cashiers().await;
        try_pin(&mut l, "2", "5678").await.unwrap();
        l.open_shift(rwf("100")).await.unwrap();
        // The id, not the name and not free text.
        assert_eq!(l.shift().unwrap().opened_by, "u-bob");
        let stored: String = sqlx::query_scalar("select opened_by from shift")
            .fetch_one(l.store.pool())
            .await
            .unwrap();
        assert_eq!(stored, "u-bob");
        assert_eq!(l.shift_cashier(), Some("Bob"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[expect(non_snake_case, reason = "named for the JSON key it checks")]
    async fn the_shift_payload_sends_openedById() {
        let mut l = lane_with_cashiers().await;
        try_pin(&mut l, "1", "1234").await.unwrap();
        l.open_shift(rwf("100")).await.unwrap();
        let (by_id, old_key): (Option<String>, Option<String>) = sqlx::query_as(
            "select json_extract(payload, '$.openedById'), json_extract(payload, '$.openedBy') \
             from outbound where kind = 'shift' order by id limit 1",
        )
        .fetch_one(l.store.pool())
        .await
        .unwrap();
        assert_eq!(by_id.as_deref(), Some("u-alice"));
        assert_eq!(old_key, None, "the old key is gone");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pin_is_never_drawn_on_screen_or_left_in_the_buffer() {
        let mut l = lane_with_cashiers().await;
        type_digits(&mut l, "1");
        l.choose_cashier().await.unwrap();
        type_digits(&mut l, "1234");
        // Not in the scan buffer, which other screens draw.
        assert_eq!(l.buffer(), "");
        // The screen is given a mask, and only a mask.
        match l.signin() {
            SignInView::Pin { mask, .. } => {
                assert_eq!(mask.chars().count(), 4);
                assert!(mask.chars().all(|c| c == '\u{2022}'), "{mask}");
            }
            v => panic!("{v:?}"),
        }
        // Nor does a Debug of what the screen sees carry it.
        assert!(!format!("{:?}", l.signin()).contains("1234"));
        // A wrong PIN leaves nothing behind.
        l.begin_pin_check().unwrap();
        finish_check(&mut l).unwrap();
        l.back_to_cashiers();
        type_digits(&mut l, "2");
        l.choose_cashier().await.unwrap();
        type_digits(&mut l, "0000");
        l.begin_pin_check().unwrap();
        finish_check(&mut l).unwrap();
        assert_eq!(l.buffer(), "");
        assert!(matches!(l.signin(), SignInView::Pin { mask, .. } if mask.is_empty()));
        // A half-keyed PIN does not survive leaving the screen.
        type_digits(&mut l, "55");
        l.back_to_cashiers();
        type_digits(&mut l, "2");
        l.choose_cashier().await.unwrap();
        assert!(matches!(l.signin(), SignInView::Pin { mask, .. } if mask.is_empty()));
        // Nor a correct one once the shift opens and closes again.
        type_digits(&mut l, "5678");
        l.begin_pin_check().unwrap();
        finish_check(&mut l).unwrap();
        l.open_shift(Money::zero()).await.unwrap();
        assert_eq!(l.buffer(), "");
        l.close_shift(Money::zero()).await.unwrap();
        assert!(matches!(l.signin(), SignInView::Choosing { .. }));
        assert_eq!(l.buffer(), "");
    }

    #[test]
    fn the_lockout_backs_off_from_thirty_seconds_to_a_cap() {
        for n in 0..PIN_FREE_TRIES {
            assert_eq!(lockout_for(n), None, "{n} wrong PINs is still free");
        }
        let secs = |n| lockout_for(n).unwrap().as_secs();
        assert_eq!(secs(5), 30);
        assert_eq!(secs(6), 60);
        assert_eq!(secs(7), 120);
        assert_eq!(secs(40), 15 * 60, "capped, and no overflow");
        assert_eq!(secs(u32::MAX), 15 * 60);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lockout_after_repeated_wrong_pins() {
        let mut l = lane_with_cashiers().await;
        for n in 1..PIN_FREE_TRIES {
            try_pin(&mut l, "1", "0000").await.unwrap();
            assert!(!problem(&l).contains("locked"), "try {n}: {}", problem(&l));
        }
        // The fifth wrong PIN locks sign-in, and says for how long.
        try_pin(&mut l, "1", "0000").await.unwrap();
        let m = problem(&l);
        assert!(m.contains("locked for 30 seconds"), "{m}");
        // While locked even the RIGHT PIN is refused, and is not checked.
        try_pin(&mut l, "1", "1234").await.unwrap();
        let m = problem(&l);
        assert!(m.contains("locked") && m.contains("Wait"), "{m}");
        assert!(matches!(l.signin(), SignInView::Pin { .. }));
        // Another cashier does not get around it: the lock is the till's.
        try_pin(&mut l, "2", "5678").await.unwrap();
        assert!(problem(&l).contains("locked"));
        assert_eq!(l.screen(), &Screen::NoShift);
        // When it runs out, the right PIN works, and clears the count.
        l.expire_lockout();
        try_pin(&mut l, "1", "1234").await.unwrap();
        assert!(matches!(l.signin(), SignInView::Float { .. }));
        assert_eq!(l.pin_failures, 0);
        // A wrong PIN after a lockout backs off for longer.
        let mut l = lane_with_cashiers().await;
        for _ in 0..PIN_FREE_TRIES {
            try_pin(&mut l, "1", "0000").await.unwrap();
        }
        l.expire_lockout();
        try_pin(&mut l, "1", "0000").await.unwrap();
        assert!(
            problem(&l).contains("locked for 60 seconds"),
            "{}",
            problem(&l)
        );
    }

    /// The worker is held at a gate the test controls. If `begin_pin_check`
    /// waited for the check (ran it inline), it would sit at the gate for the
    /// full 5 s timeout and this would fail.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_pin_check_does_not_wait_for_the_verification() {
        let mut l = lane_with_cashiers().await;
        type_digits(&mut l, "1");
        l.choose_cashier().await.unwrap();
        type_digits(&mut l, "1234");
        let (open_gate, gate) = mpsc::channel();
        l.pin_gate = Some(gate);
        let started = Instant::now();
        l.begin_pin_check().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "begin_pin_check waited for the verification: {:?}",
            started.elapsed()
        );
        // The gate is still held: the check is pending and polling does not block.
        std::thread::sleep(Duration::from_millis(150));
        l.poll_pin_check().unwrap();
        assert!(l.is_checking_pin(), "the answer cannot exist while gated");
        open_gate.send(()).unwrap();
        finish_check(&mut l).unwrap();
        assert!(matches!(l.signin(), SignInView::Float { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_twelve_digit_pin_verifies_and_opens_a_shift() {
        let store = Store::open_in_memory().await.unwrap();
        store
            .sync_cashiers(&[crate::lane::test_support::cashier(
                "u-long",
                "Long",
                "123456789012",
            )])
            .await
            .unwrap();
        let mut l = LaneState::new(store, test_config()).await.unwrap();
        try_pin(&mut l, "1", "123456789012").await.unwrap();
        l.open_shift(Money::zero()).await.unwrap();
        assert_eq!(l.shift().unwrap().opened_by, "u-long");
        // A 13-digit barcode is still too long, and is not a guess.
        l.close_shift(Money::zero()).await.unwrap();
        type_digits(&mut l, "1");
        l.choose_cashier().await.unwrap();
        type_digits(&mut l, "5612345000017");
        l.begin_pin_check().unwrap();
        assert!(problem(&l).contains("too long"));
        assert_eq!(l.pin_failures, 0);
    }
}
