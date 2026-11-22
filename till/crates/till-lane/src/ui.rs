//! The screen: rendering and keyboard wiring only. Every decision already
//! happened in `LaneState`; this file draws its answer and forwards keys.
//!
//! The update loop is synchronous and `LaneState`'s transitions are `async`
//! (the store's API is), so they are driven with `Handle::current().block_on`.
//! `main` enters the runtime before the window opens. The calls are local
//! SQLite, sub-millisecond, so blocking the UI thread for them is acceptable.
//!
//! Colours come from `theme::TOKENS` and numbers from `fmt`, nowhere else.
//! `TOKENS.accent` is the only azure dark enough behind white text;
//! `TOKENS.hi_vis` is a ground for dark text only.

use crate::devices::Devices;
use crate::fmt;
use crate::lane::{LaneState, Notice, Screen, VsdcOutcome};
use crate::theme::{self, TOKENS};
use eframe::egui::{
    self, Align, Align2, Color32, Context, CornerRadius, FontFamily, FontId, Frame, Layout, Margin,
    Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2,
};
use till_core::codes::{PmtTyCd, TaxType};
use till_core::line::LineDiscount;
use till_core::{Money, TenderError};

/// Who is at the lane. `LaneConfig` carries none of this, so `main` supplies it
/// through the context (see [`set_session`]).
#[derive(Debug, Clone)]
pub struct Session {
    pub lane: String,
    pub cashier: String,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            lane: "LANE".into(),
            cashier: "NO CASHIER".into(),
        }
    }
}

pub fn set_session(ctx: &Context, session: Session) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("lane-session"), session));
}

/// UI-only state: things the lane does not own.
#[derive(Debug, Clone, Default)]
struct Local {
    /// A message from this file (a key that is not built, an unreadable amount).
    note: Option<String>,
    /// A fault the cashier cannot act on (store I/O). Cleared by the next action that works.
    fault: Option<String>,
}

fn local_id() -> egui::Id {
    egui::Id::new("lane-local")
}

/// What a keystroke or a tap asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Char(char),
    Backspace,
    Enter,
    VoidLine,
    /// Take a payment under this RRA code: the keyed amount, or the balance.
    Tender(PmtTyCd),
    /// Take back the last payment keyed (a mis-key).
    UndoTender,
    /// Take back every payment; the basket stays.
    ClearTenders,
    Reprint,
    /// Print a shift report: a read-only snapshot of the open shift.
    ReportX,
    /// Close the shift with the keyed cash count and print the shift close report.
    ReportZ,
    /// Print the last shift close report again (a jam is not a lost closing).
    ReprintZ,
    CancelSale,
    Resume,
    /// Demo only: take the simulated VSDC offline, or put it back, so the C1
    /// hard stop can be exercised by hand. A no-op with real devices.
    ToggleVsdc,
    /// Take the nth product a name search offered, counting from 1. Only a tap
    /// produces this; keying the number goes through the buffer like a scan.
    Pick(usize),
    NotBuilt(&'static str),
}

/// Function keys: (egui key, label, name, action). The first seven are the
/// mockup's; the lane has no discount, suspend, refund or buyer-TIN flow yet,
/// so those say so rather than doing nothing silently.
const FKEYS: [(egui::Key, &str, &str, Action); 11] = [
    (egui::Key::F1, "F1", "Void line", Action::VoidLine),
    (
        egui::Key::F2,
        "F2",
        "Discount",
        Action::NotBuilt("Discount"),
    ),
    (egui::Key::F3, "F3", "Suspend", Action::NotBuilt("Suspend")),
    (egui::Key::F4, "F4", "Resume", Action::NotBuilt("Resume")),
    (egui::Key::F5, "F5", "Refund", Action::NotBuilt("Refund")),
    (
        egui::Key::F6,
        "F6",
        "Buyer TIN",
        Action::NotBuilt("Buyer TIN"),
    ),
    (egui::Key::F7, "F7", "Shift report", Action::ReportX),
    (
        egui::Key::F8,
        "F8 · Ctrl 1",
        "Cash",
        Action::Tender(PmtTyCd::Cash),
    ),
    (egui::Key::F9, "F9", "Reprint", Action::Reprint),
    (egui::Key::F10, "F10", "Cancel sale", Action::CancelSale),
    (egui::Key::F11, "F11", "Undo tender", Action::UndoTender),
];

/// The payment methods, one per RRA code (C5b): code, key digit, name, the
/// label on the on-screen cap. Cash also answers to F8, which the cashiers
/// already know. Letters cannot be method keys: the scanner wedge types them
/// into the buffer. A modified digit is never wedge text, so Ctrl+1..7 is safe,
/// and the digit is the RRA code's own number.
const METHODS: [(PmtTyCd, egui::Key, &str, &str); 7] = [
    (PmtTyCd::Cash, egui::Key::Num1, "Cash", "Cash"),
    (PmtTyCd::Credit, egui::Key::Num2, "Credit", "Credit"),
    (
        PmtTyCd::CashCredit,
        egui::Key::Num3,
        "Cash / credit",
        "Cash/credit",
    ),
    (
        PmtTyCd::BankCheck,
        egui::Key::Num4,
        "Bank check",
        "Bank check",
    ),
    (PmtTyCd::Card, egui::Key::Num5, "Card", "Card"),
    (
        PmtTyCd::MobileMoney,
        egui::Key::Num6,
        "Mobile money",
        "MoMo",
    ),
    (PmtTyCd::Other, egui::Key::Num7, "Other", "Other"),
];

/// Shift-chorded report keys: key, name, action. Shift+F7 closes the shift and
/// prints its close report, beside F7's harmless shift report: a chord because
/// closing is irreversible, and Shift rather than Ctrl
/// because the operating system reserves Ctrl+F-key chords on some machines.
const SHIFT_KEYS: [(egui::Key, &str, Action); 2] = [
    (egui::Key::F7, "Close shift", Action::ReportZ),
    // "Reprint close report" does not fit a cap even at the smallest size, and
    // a clipped label reads as broken. Not "Reprint Z": these are shift
    // reports, not RRA's daily Z.
    (egui::Key::F9, "Reprint close", Action::ReprintZ),
];

fn shift_cap(key: egui::Key) -> String {
    format!("Shift {}", key.name())
}

/// The key cap of a Ctrl chord, derived from the key itself so a cap can never
/// disagree with the binding that `actions_from` really reads.
fn ctrl_cap(key: egui::Key) -> String {
    format!("Ctrl {}", key.name())
}

fn method_name(code: PmtTyCd) -> &'static str {
    METHODS
        .iter()
        .find(|(c, ..)| *c == code)
        .map(|(_, _, name, _)| *name)
        .unwrap_or("Other")
}

/// Translates raw input into actions, in order. Pure, so it can be tested.
pub fn actions_from(events: &[egui::Event]) -> Vec<Action> {
    let mut out = Vec::new();
    for ev in events {
        match ev {
            egui::Event::Text(t) => out.extend(t.chars().map(Action::Char)),
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => match key {
                // Ctrl+Backspace is not a wedge character and not a method key.
                egui::Key::Backspace if modifiers.ctrl => out.push(Action::ClearTenders),
                k if modifiers.ctrl => {
                    if let Some((code, ..)) = METHODS.iter().find(|(_, mk, ..)| mk == k) {
                        out.push(Action::Tender(*code));
                    }
                }
                // Demo only, and deliberately next to F12's resume: Shift+F12
                // takes the simulated VSDC away, F12 brings the lane back.
                // A chord, so a scanner wedge can never type it.
                egui::Key::F12 if modifiers.shift => out.push(Action::ToggleVsdc),
                k if modifiers.shift && SHIFT_KEYS.iter().any(|(sk, ..)| sk == k) => {
                    out.extend(SHIFT_KEYS.iter().filter(|(sk, ..)| sk == k).map(|t| t.2));
                }
                egui::Key::Enter => out.push(Action::Enter),
                egui::Key::Backspace => out.push(Action::Backspace),
                egui::Key::F12 => out.push(Action::Resume),
                k => {
                    if let Some((_, _, _, a)) = FKEYS.iter().find(|(fk, ..)| fk == k) {
                        out.push(*a);
                    }
                }
            },
            _ => {}
        }
    }
    out
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(f)
}

fn apply(
    action: Action,
    state: &mut LaneState,
    devices: &mut Devices,
    local: &mut Local,
    session: &Session,
) {
    local.note = None;
    let result = match action {
        Action::Char(c) => {
            state.key(c);
            Ok(())
        }
        Action::Backspace => {
            state.backspace();
            Ok(())
        }
        Action::Enter => match state.screen() {
            Screen::NoShift => open_shift(state, local, session),
            Screen::Selling if !state.buffer().is_empty() => block_on(state.enter()),
            Screen::Selling => block_on(state.settle(
                devices.fiscal.as_mut(),
                devices.printer.as_mut(),
                devices.drawer.as_mut(),
            ))
            .map(|_| ()),
            Screen::Settled { .. } => state.next_customer(),
            // Nothing to do: a supervisor clears a hard stop with F12.
            Screen::HardStop { .. } => Ok(()),
        },
        Action::VoidLine => {
            let seq = state
                .buffer()
                .parse::<u16>()
                .ok()
                .or_else(|| state.basket().lines().last().map(|l| l.item_seq));
            match seq {
                Some(s) => block_on(state.void_line(s)),
                None => Ok(()),
            }
        }
        Action::Tender(code) => tender(state, local, code),
        Action::UndoTender => state.remove_last_tender(),
        Action::ClearTenders => state.clear_tenders(),
        Action::Reprint => state.reprint(devices.printer.as_mut()),
        Action::ReportX => block_on(state.print_x(devices.printer.as_mut())),
        Action::ReportZ => close_with_z(state, local, devices),
        Action::ReprintZ => block_on(state.reprint_z(devices.printer.as_mut())),
        Action::CancelSale => block_on(state.abandon_pending()),
        Action::Resume => state.resume_from_hard_stop(),
        Action::Pick(n) => block_on(state.pick_nth(n)),
        Action::ToggleVsdc => {
            local.note = Some(match devices.toggle_vsdc() {
                Some(true) => {
                    "Demo: the VSDC is now unreachable. The next sale will hard stop.".to_owned()
                }
                Some(false) => "Demo: the VSDC is reachable again.".to_owned(),
                None => "There is no simulated VSDC to switch in this build.".to_owned(),
            });
            Ok(())
        }
        Action::NotBuilt(name) => {
            local.note = Some(format!("{name} is not available in this build."));
            Ok(())
        }
    };
    local.fault = result.err().map(|e| e.to_string());
}

/// Shift+F7: what is keyed is the cash the cashier counted. Unlike the opening
/// float, nothing keyed is NOT taken as zero: closing the shift is irreversible, and
/// a count of zero must be keyed on purpose, never by a stray key press.
fn close_with_z(
    state: &mut LaneState,
    local: &mut Local,
    devices: &mut Devices,
) -> Result<(), crate::lane::LaneError> {
    let digits = state.buffer().chars().filter(char::is_ascii_digit).count();
    let counted = if state.buffer().is_empty() || digits > MAX_FLOAT_DIGITS {
        None
    } else {
        Money::try_from_str(state.buffer()).ok()
    };
    let Some(counted) = counted else {
        // Never close on a scanned barcode or a mis-key.
        while !state.buffer().is_empty() {
            state.backspace();
        }
        local.note = Some(
            "Key the cash you counted in the drawer (0 if empty), then press Shift F7 to close the shift."
                .into(),
        );
        return Ok(());
    };
    block_on(state.print_z(counted, devices.printer.as_mut()))
}

/// The longest opening float the till will take from the keyboard.
const MAX_FLOAT_DIGITS: usize = 7;

/// Enter on the no-shift screen: what is keyed is the opening float; nothing
/// keyed means a float of zero. The cashier's name is the session's, because
/// the lane has no sign-in of its own yet.
fn open_shift(
    state: &mut LaneState,
    local: &mut Local,
    session: &Session,
) -> Result<(), crate::lane::LaneError> {
    // A scanner types its code and Enter exactly like a cashier keys a float,
    // so a barcode scanned on this screen must never become an opening float.
    // No drawer opens with more than seven digits.
    // Digits, not characters: `10000.50` is a legitimate float of eight
    // characters and seven digits. An EAN-8 is the shortest barcode and has
    // eight digits, which is why the limit is seven.
    let digits = state.buffer().chars().filter(char::is_ascii_digit).count();
    if digits > MAX_FLOAT_DIGITS {
        while !state.buffer().is_empty() {
            state.backspace();
        }
        local.note = Some("That looks like a barcode. Key the opening float instead.".into());
        return Ok(());
    }
    let float = if state.buffer().is_empty() {
        Some(Money::zero())
    } else {
        Money::try_from_str(state.buffer()).ok()
    };
    match float {
        Some(f) => block_on(state.open_shift(&session.cashier, f)),
        None => {
            local.note = Some("Key the opening float as a plain amount, then press Enter.".into());
            Ok(())
        }
    }
}

/// A payment key: what is keyed is the amount handed over; nothing keyed means
/// the exact balance. Which code goes where, and what change is owed, is
/// `till-core`'s: this only passes the amount and the code on.
fn tender(
    state: &mut LaneState,
    local: &mut Local,
    code: PmtTyCd,
) -> Result<(), crate::lane::LaneError> {
    if *state.screen() != Screen::Selling {
        return Ok(());
    }
    let amount = if state.buffer().is_empty() {
        state
            .basket()
            .totals()
            .ok()
            .and_then(|t| state.remaining(t.tot_amt))
    } else {
        Money::try_from_str(state.buffer()).ok()
    };
    match amount {
        Some(a) => state.add_tender(code, a),
        None => {
            local.note = Some(if state.buffer().is_empty() {
                "Nothing is left to pay. Press Enter to settle.".to_owned()
            } else {
                format!(
                    "Key the {} amount as a plain number first.",
                    method_name(code).to_lowercase()
                )
            });
            Ok(())
        }
    }
}

/// The whole lane screen. Call once per frame.
pub fn lane(ctx: &Context, state: &mut LaneState, devices: &mut Devices) {
    let mut local: Local = ctx.data(|d| d.get_temp(local_id())).unwrap_or_default();
    let session: Session = ctx
        .data(|d| d.get_temp(egui::Id::new("lane-session")))
        .unwrap_or_default();

    for a in ctx.input(|i| actions_from(&i.events)) {
        apply(a, state, devices, &mut local, &session);
    }

    let mut tapped = Vec::new();
    status_bar(ctx, state, &session, devices.vsdc_offline());
    match state.screen().clone() {
        Screen::HardStop { reason } => {
            if let Some(a) = hard_stop(ctx, state, &reason) {
                tapped.push(a);
            }
        }
        Screen::NoShift => no_shift(ctx, state, &local, &session),
        _ => {
            if let Some(a) = rail(ctx, state, &local) {
                tapped.push(a);
            }
            basket_panel(ctx, state, &local);
        }
    }
    for a in &tapped {
        apply(*a, state, devices, &mut local, &session);
    }
    if !tapped.is_empty() {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(local_id(), local));
    // The clock.
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
}

// ---------------------------------------------------------------- fonts

fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
fn mono_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(theme::MONO_BOLD.into()))
}
fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
fn sans_semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(theme::INTER_SEMIBOLD.into()))
}
fn sans_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(theme::INTER_BOLD.into()))
}

fn text(s: impl Into<String>, font: FontId, color: Color32) -> RichText {
    RichText::new(s).font(font).color(color)
}

fn corner() -> CornerRadius {
    CornerRadius::same(theme::RADIUS as u8)
}

// ---------------------------------------------------------------- status bar

/// `HH:MM` in Kigali (UTC+2, no daylight saving).
fn clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let local = secs + 2 * 3600;
    format!("{:02}:{:02}", local % 86_400 / 3600, local % 3600 / 60)
}

fn chip(ui: &mut Ui, label: &str, fill: Color32, ink: Color32) {
    Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(text(label.to_uppercase(), mono_bold(12.0), ink));
        });
}

/// What the status bar says about the shift: the real opening time (Kigali
/// clock) and who opened it, read from the open shift itself.
fn shift_label(shift: Option<&till_store::OpenShift>) -> String {
    match shift {
        Some(s) => {
            let at = crate::lane::kigali_hhmm(&s.opened_at).unwrap_or_else(|| s.opened_at.clone());
            format!("Shift open {at} · {}", s.opened_by).to_uppercase()
        }
        None => "NO SHIFT".to_string(),
    }
}

fn status_bar(ctx: &Context, state: &LaneState, session: &Session, vsdc_switched_off: bool) {
    egui::TopBottomPanel::top("status")
        .frame(
            Frame::new()
                .fill(TOKENS.ink)
                .inner_margin(Margin::symmetric(14, 10)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (mark, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                ui.painter()
                    .rect_filled(mark, CornerRadius::same(7), TOKENS.hi_vis);
                ui.label(text(
                    session.lane.to_uppercase(),
                    sans_bold(15.0),
                    TOKENS.canvas,
                ));
                ui.label(text(
                    session.cashier.to_uppercase(),
                    sans(14.0),
                    TOKENS.line,
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // The demo's own switch is a different claim from the one
                    // below: it reports how the SIMULATOR is set, which is
                    // known without asking anything, and says "demo" so it can
                    // never be read as the state of a real device.
                    if vsdc_switched_off {
                        chip(ui, "DEMO: VSDC switched off", TOKENS.danger, TOKENS.surface);
                    }
                    // The lane does not poll the VSDC; it only learns at
                    // settlement. So the chip reports the last outcome, and
                    // never claims the device is reachable right now.
                    match (state.screen(), state.vsdc_outcome()) {
                        (Screen::HardStop { .. }, _) | (_, VsdcOutcome::Unreachable) => chip(
                            ui,
                            "VSDC unreachable at last sale",
                            TOKENS.danger,
                            TOKENS.surface,
                        ),
                        (_, VsdcOutcome::Answered) => {
                            chip(ui, "VSDC answered last sale", TOKENS.hi_vis, TOKENS.ink)
                        }
                        (_, VsdcOutcome::NoneYet) => {
                            chip(ui, "VSDC not contacted yet", TOKENS.line, TOKENS.ink)
                        }
                    }
                    ui.label(text(clock(), mono(14.0), TOKENS.line));
                    let shift = shift_label(state.shift());
                    ui.label(text(shift, sans(14.0), TOKENS.line));
                });
            });
        });
}

// ---------------------------------------------------------------- basket

const COL_SEQ: f32 = 44.0;
const COL_QTY: f32 = 84.0;
const COL_PRICE: f32 = 90.0;
const COL_BAND: f32 = 40.0;
const COL_AMOUNT: f32 = 110.0;
const GAP: f32 = 8.0;
const ROW_PAD: f32 = 12.0;

/// The six cell rects of a basket row occupying `row`: `#`, item (takes what
/// is left), qty, price, band, amount. Pure arithmetic, so a test can pin it:
/// every column gets exactly its width on every row, whatever the text is.
fn column_rects(row: Rect) -> [Rect; 6] {
    let item_w = row.width()
        - 2.0 * ROW_PAD
        - COL_SEQ
        - COL_QTY
        - COL_PRICE
        - COL_BAND
        - COL_AMOUNT
        - 5.0 * GAP;
    let widths = [
        COL_SEQ,
        item_w.max(80.0),
        COL_QTY,
        COL_PRICE,
        COL_BAND,
        COL_AMOUNT,
    ];
    let mut x = row.left() + ROW_PAD;
    widths.map(|w| {
        let r = Rect::from_min_size(egui::pos2(x, row.top()), vec2(w, row.height()));
        x += w + GAP;
        r
    })
}

/// One row of the basket grid. The row is reserved at full width first and
/// each cell is placed in its own rect: `allocate_ui_with_layout` only sets a
/// MAXIMUM, so the cursor would advance by content width and columns drift.
fn row(ui: &mut Ui, height: f32, cells: [&dyn Fn(&mut Ui); 6]) {
    let (row_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    for (i, (cell, rect)) in cells.iter().zip(column_rects(row_rect)).enumerate() {
        let layout = match i {
            0 | 1 => Layout::left_to_right(Align::Center),
            4 => Layout::centered_and_justified(egui::Direction::LeftToRight),
            _ => Layout::right_to_left(Align::Center),
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        cell(&mut child);
    }
}

fn rule(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::ZERO, color);
}

fn basket_panel(ctx: &Context, state: &LaneState, local: &Local) {
    egui::CentralPanel::default()
        .frame(Frame::new().fill(TOKENS.surface))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let head = |s: &'static str| {
                move |ui: &mut Ui| {
                    ui.label(text(s.to_uppercase(), sans_semibold(11.0), TOKENS.ink_mute));
                }
            };
            ui.add_space(6.0);
            row(
                ui,
                24.0,
                [
                    &head("#"),
                    &head("Item"),
                    &head("Qty"),
                    &head("Price"),
                    &head("Band"),
                    &head("Amount"),
                ],
            );
            rule(ui, TOKENS.line);

            let lines = state.basket().apportioned_lines();
            let foot_h = 92.0;
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - foot_h)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| match &lines {
                    Err(e) => {
                        ui.add_space(12.0);
                        ui.label(text(
                            format!("The basket cannot be priced: {e}. Call a supervisor."),
                            sans(15.0),
                            TOKENS.danger,
                        ));
                    }
                    Ok(lines) => {
                        let last = lines.len().saturating_sub(1);
                        for (i, (line, amt)) in lines.iter().enumerate() {
                            let live = i == last && *state.screen() == Screen::Selling;
                            basket_row(ui, line, amt, live);
                        }
                    }
                });

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                basket_foot(ui, state, local);
            });
        });
}

fn basket_row(
    ui: &mut Ui,
    line: &till_core::BasketLine,
    amt: &till_core::line::LineAmounts,
    live: bool,
) {
    let ground = if live {
        TOKENS.accent_soft
    } else {
        TOKENS.surface
    };
    let frame = Frame::new()
        .fill(ground)
        .inner_margin(Margin::symmetric(0, 6));
    let resp = frame.show(ui, |ui| {
        let sub = match &line.bcd {
            Some(b) => format!("{} · {b}", line.item_cd),
            None => line.item_cd.clone(),
        };
        row(
            ui,
            38.0,
            [
                &|ui| {
                    ui.label(text(
                        format!("{:03}", line.item_seq),
                        mono(13.0),
                        TOKENS.ink_mute,
                    ));
                },
                &|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(text(&line.item_nm, sans_semibold(16.0), TOKENS.ink));
                        ui.label(text(
                            &sub,
                            mono(11.0),
                            if live {
                                TOKENS.ink_soft
                            } else {
                                TOKENS.ink_mute
                            },
                        ));
                    });
                },
                &|ui| {
                    ui.label(text(
                        fmt::qty_display(line.qty),
                        mono_bold(15.0),
                        TOKENS.ink,
                    ));
                },
                &|ui| {
                    ui.label(text(fmt::rwf(line.prc), mono_bold(15.0), TOKENS.ink));
                },
                &|ui| {
                    ui.label(text(
                        line.tax_ty_cd.wire(),
                        mono_bold(13.0),
                        TOKENS.ink_soft,
                    ));
                },
                &|ui| {
                    ui.label(text(fmt::rwf(amt.sply_amt), mono_bold(15.0), TOKENS.ink));
                },
            ],
        );
        if amt.dc_amt > Money::zero() {
            let label = match line.discount {
                Some(LineDiscount::Percent(p)) => format!("↳ line discount {}%", p.normalize()),
                _ => "↳ line discount".to_owned(),
            };
            ui.horizontal(|ui| {
                ui.add_space(ROW_PAD + COL_SEQ + GAP);
                ui.label(text(label, sans(13.0), TOKENS.danger));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(ROW_PAD);
                    ui.label(text(
                        format!("−{}", fmt::rwf(amt.dc_amt)),
                        mono_bold(13.0),
                        TOKENS.danger,
                    ));
                });
            });
        }
    });
    if live {
        let r = resp.response.rect;
        ui.painter().rect_filled(
            Rect::from_min_size(r.min, vec2(3.0, r.height())),
            CornerRadius::ZERO,
            TOKENS.accent,
        );
    }
    rule(ui, TOKENS.line);
}

fn basket_foot(ui: &mut Ui, state: &LaneState, local: &Local) {
    Frame::new()
        .fill(TOKENS.surface)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                notice_line(ui, state, local);
                ui.add_space(6.0);
                let cfg = state.config();
                ui.label(text(
                    format!(
                        "{} · DVC {} · BHF {} · {} lines",
                        cfg.shop_name,
                        cfg.dvc_srl_no,
                        cfg.bhf_id,
                        state.basket().len()
                    ),
                    mono(11.0),
                    TOKENS.ink_mute,
                ));
            });
        });
    rule(ui, TOKENS.line);
}

/// The lane's notice, or a UI-side fault or note, whichever matters most.
fn notice_line(ui: &mut Ui, state: &LaneState, local: &Local) {
    let (msg, color) = if let Some(f) = &local.fault {
        (f.clone(), TOKENS.danger)
    } else if let Some(n) = &local.note {
        (n.clone(), TOKENS.warning)
    } else {
        match state.notice() {
            Notice::Problem(m) => (m.clone(), TOKENS.danger),
            Notice::Info(m) => (m.clone(), TOKENS.ink_soft),
            Notice::None => (String::new(), TOKENS.ink_soft),
        }
    };
    ui.set_min_height(44.0);
    if !msg.is_empty() {
        ui.add(egui::Label::new(text(msg, sans_semibold(15.0), color)).wrap());
    }
}

// ---------------------------------------------------------------- rail

fn tile(ui: &mut Ui, fill: Color32, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, TOKENS.line))
        .corner_radius(corner())
        .inner_margin(Margin::symmetric(15, 13))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn amount_tile(
    ui: &mut Ui,
    label: &str,
    amount: Money,
    fill: Color32,
    ink: Color32,
    mute: Color32,
) {
    tile(ui, fill, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.label(text(label.to_uppercase(), mono_bold(11.0), mute));
        ui.horizontal(|ui| {
            ui.label(text(fmt::rwf(amount), mono_bold(38.0), ink));
            ui.label(text("RWF", sans_semibold(13.0), mute));
        });
    });
}

fn rail(ctx: &Context, state: &LaneState, _local: &Local) -> Option<Action> {
    let mut tapped = None;
    egui::SidePanel::right("rail")
        .exact_width(RAIL_WIDTH)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(TOKENS.canvas)
                .stroke(Stroke::new(1.0_f32, TOKENS.line))
                .inner_margin(Margin::same(RAIL_MARGIN as i8)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            let totals = state.basket().totals();
            let due = totals.as_ref().map(|t| t.tot_amt).unwrap_or(Money::zero());
            let tendered = state.tender_total().unwrap_or(Money::zero());
            let settled = match state.screen() {
                Screen::Settled { change } => Some(*change),
                _ => None,
            };

            match settled {
                // The change is the thing the cashier needs; it takes the accent.
                Some(change) => {
                    amount_tile(
                        ui,
                        "Change due",
                        change,
                        TOKENS.accent,
                        TOKENS.surface,
                        TOKENS.surface,
                    );
                    amount_tile(
                        ui,
                        "Total",
                        due,
                        TOKENS.surface,
                        TOKENS.ink,
                        TOKENS.ink_mute,
                    );
                }
                None => amount_tile(
                    ui,
                    "Total due",
                    due,
                    TOKENS.accent,
                    TOKENS.surface,
                    TOKENS.surface,
                ),
            }

            tile(ui, TOKENS.surface, |ui| {
                ui.label(text("TAX BANDS", mono_bold(11.0), TOKENS.ink_mute));
                match &totals {
                    Ok(t) => band_table(ui, t),
                    Err(_) => {
                        ui.label(text("Not available", sans(14.0), TOKENS.danger));
                    }
                }
            });

            amount_tile(
                ui,
                "Tendered",
                tendered,
                TOKENS.surface,
                TOKENS.ink,
                TOKENS.ink_mute,
            );
            tender_lines(ui, state);
            if settled.is_none() {
                balance_tile(ui, state, due);
            }

            if let Some(picked) = match_list(ui, state) {
                tapped = Some(picked);
            }
            scan_field(ui, state);
            let from_keys = keys(ui, state);
            tapped = tapped.or(from_keys);
        });
    tapped
}

/// One line per tender taken, each with its RRA code, so a split is visible.
fn tender_lines(ui: &mut Ui, state: &LaneState) {
    let entries = state.tenders().entries();
    if entries.is_empty() {
        return;
    }
    tile(ui, TOKENS.surface, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for e in entries {
            ui.horizontal(|ui| {
                ui.label(text(e.pmt_ty_cd.wire(), mono_bold(14.0), TOKENS.accent));
                ui.label(text(
                    method_name(e.pmt_ty_cd),
                    sans_semibold(14.0),
                    TOKENS.ink,
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(text(fmt::rwf(e.amount), mono_bold(14.0), TOKENS.ink));
                });
            });
        }
    });
}

/// Change, or what is still owed, before settlement. Asks `till-core` rather
/// than subtracting: change is capped by the cash handed over, so a card
/// overpayment owes none, and the cashier is told so instead of being shown
/// a change figure the till will refuse to settle on.
fn balance_tile(ui: &mut Ui, state: &LaneState, due: Money) {
    match state.tenders().settle(due) {
        Ok(s) => amount_tile(
            ui,
            "Change",
            s.change,
            TOKENS.surface,
            TOKENS.ink,
            TOKENS.ink_mute,
        ),
        // Nothing tendered yet is the same state as a shortfall: all of it is
        // still to pay. `remaining` does the sum; the UI never subtracts.
        Err(TenderError::Insufficient { .. } | TenderError::Empty) => amount_tile(
            ui,
            "Still to pay",
            state.remaining(due).unwrap_or(due),
            TOKENS.surface,
            TOKENS.ink,
            TOKENS.ink_mute,
        ),
        Err(TenderError::OverTenderOnNonCash) => balance_alert(
            ui,
            "NO CHANGE ON NON-CASH. The payment is more than is due and cash received does not cover the difference. Take the last payment back (F11) and key it again.",
        ),
        // The basket's own total or the tender sum is unusable; the cashier
        // must see that rather than a blank panel.
        Err(
            TenderError::NegativeDue(_)
            | TenderError::FractionalDue(_)
            | TenderError::OutOfRange
            | TenderError::NonPositiveAmount
            | TenderError::FractionalAmount(_),
        ) => balance_alert(
            ui,
            "THE BALANCE CANNOT BE WORKED OUT. Take the payments back and check the basket, or ask a supervisor.",
        ),
    }
}

fn balance_alert(ui: &mut Ui, message: &str) {
    tile(ui, TOKENS.surface, |ui| {
        ui.label(text(message, sans_semibold(14.0), TOKENS.danger));
    });
}

fn band_table(ui: &mut Ui, t: &till_core::BasketTotals) {
    let cell = |ui: &mut Ui, s: String, bold: bool, right: bool, color: Color32| {
        let font = if bold { mono_bold(14.0) } else { mono(14.0) };
        let layout = if right {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        };
        ui.with_layout(layout, |ui| ui.label(text(s, font, color)));
    };
    egui::Grid::new("bands")
        .num_columns(3)
        .min_col_width(80.0)
        .spacing([8.0, 5.0])
        .show(ui, |ui| {
            for h in ["BAND", "TAXABLE", "VAT"] {
                ui.label(text(h, sans_semibold(10.5), TOKENS.ink_mute));
            }
            ui.end_row();
            for band in TaxType::ALL {
                let (taxable, rate, vat) = t.by_band(*band);
                if taxable == Money::zero() && vat == Money::zero() {
                    continue;
                }
                let name = if rate.is_zero() {
                    format!("{} EX", band.wire())
                } else {
                    format!("{} {}%", band.wire(), rate.normalize())
                };
                cell(ui, name, false, false, TOKENS.ink);
                cell(ui, fmt::rwf(taxable), false, true, TOKENS.ink);
                cell(ui, fmt::tax(vat), false, true, TOKENS.ink);
                ui.end_row();
            }
            cell(ui, "Total".into(), true, false, TOKENS.ink);
            cell(ui, fmt::rwf(t.tot_taxbl_amt), true, true, TOKENS.ink);
            cell(ui, fmt::tax(t.tot_tax_amt), true, true, TOKENS.ink);
            ui.end_row();
        });
}

/// The products a name search is offering, numbered the way `LaneState::pick`
/// counts them: from 1. Returns the action for a row that was tapped, so a
/// touch screen reaches the same code path as keying the number.
fn match_list(ui: &mut Ui, state: &LaneState) -> Option<Action> {
    let matches = state.search_matches();
    if matches.is_empty() {
        return None;
    }
    let mut tapped = None;
    tile(ui, TOKENS.surface, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let term = state.search_term().unwrap_or_default();
        let heading = match state.search_times() {
            Some(n) => format!("PICK AN ITEM · \"{term}\" · {n} OF IT"),
            None => format!("PICK AN ITEM · \"{term}\""),
        };
        ui.label(text(heading, sans_bold(11.0), TOKENS.ink_mute));
        for (i, p) in matches.iter().enumerate() {
            let n = i + 1;
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::CLICK);
            if resp.hovered() {
                ui.painter().rect_filled(rect, corner(), TOKENS.accent_soft);
            }
            let painter = ui.painter().with_clip_rect(rect);
            painter.text(
                rect.left_center() + vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{n}"),
                mono_bold(14.0),
                TOKENS.accent,
            );
            painter.text(
                rect.left_center() + vec2(34.0, 0.0),
                Align2::LEFT_CENTER,
                &p.name,
                sans(14.0),
                TOKENS.ink,
            );
            painter.text(
                rect.right_center() - vec2(8.0, 0.0),
                Align2::RIGHT_CENTER,
                fmt::rwf(p.unit_price),
                mono(14.0),
                TOKENS.ink_soft,
            );
            if resp.clicked() {
                tapped = Some(Action::Pick(n));
            }
        }
    });
    tapped
}

fn scan_field(ui: &mut Ui, state: &LaneState) {
    Frame::new()
        .fill(TOKENS.surface)
        .stroke(Stroke::new(1.0_f32, TOKENS.line))
        .corner_radius(corner())
        .inner_margin(Margin::symmetric(11, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let selling = *state.screen() == Screen::Selling;
            if state.buffer().is_empty() {
                let hint = if state.search_matches().is_empty() {
                    // The three things the buffer accepts, in the order a
                    // cashier reaches for them.
                    "Scan, key a PLU, type a name, or 3*code ▏"
                } else {
                    "Key the item's number ▏"
                };
                let hint = if selling { hint } else { "" };
                ui.label(text(hint, mono(17.0), TOKENS.ink_mute));
            } else {
                ui.label(text(
                    format!("{}▏", state.buffer()),
                    mono_bold(17.0),
                    TOKENS.ink,
                ));
            }
        });
}

/// The function keys; returns the one tapped, if any. Keys that are not built
/// are drawn muted so a cashier does not expect them to work.
fn keys(ui: &mut Ui, state: &LaneState) -> Option<Action> {
    let mut tapped = None;
    let enter = match state.screen() {
        Screen::Settled { .. } => ("ENTER", "Next customer", true),
        _ => ("ENTER", "Scan / Pay", true),
    };
    let mut all: Vec<(String, &str, Action, bool, bool)> = FKEYS
        .iter()
        .map(|(_, k, n, a)| {
            (
                (*k).to_owned(),
                *n,
                *a,
                false,
                matches!(a, Action::NotBuilt(_)),
            )
        })
        .collect();
    // Cash already has F8; its cap carries the Ctrl+1 alias too. Every label is
    // derived from `METHODS`, the table `actions_from` reads, so they cannot drift.
    for (code, key, _, cap) in METHODS.iter().filter(|(c, ..)| *c != PmtTyCd::Cash) {
        all.push((ctrl_cap(*key), *cap, Action::Tender(*code), false, false));
    }
    for (key, name, action) in SHIFT_KEYS {
        all.push((shift_cap(key), name, action, false, false));
    }
    all.push((
        "Ctrl Bksp".to_owned(),
        "Clear tenders",
        Action::ClearTenders,
        false,
        false,
    ));
    all.push((enter.0.to_owned(), enter.1, Action::Enter, enter.2, false));

    let w = cap_width();
    for chunk in all.chunks(CAP_COLS) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CAP_GAP;
            for (fk, name, action, hot, muted) in chunk {
                if key_cap(ui, w, fk.as_str(), name, *hot, *muted) {
                    tapped = Some(*action);
                }
            }
        });
    }
    tapped
}

/// The function-key rail's exact width, its inner margin, and how the caps are
/// laid out inside it. Named because the cap width derives from them, and the
/// test that proves every label fits must not be able to drift from the UI.
const RAIL_WIDTH: f32 = 440.0;
const RAIL_MARGIN: f32 = 12.0;
const CAP_COLS: usize = 4;
const CAP_GAP: f32 = 6.0;

/// The width of one key cap, given the rail's geometry.
fn cap_width() -> f32 {
    let inner = RAIL_WIDTH - RAIL_MARGIN * 2.0;
    (inner - CAP_GAP * (CAP_COLS as f32 - 1.0)) / CAP_COLS as f32
}

/// The font sizes a key cap will try for its name, largest first. A label that
/// does not fit at 13 is shrunk rather than painted over its neighbours.
const CAP_SIZES: [f32; 4] = [13.0, 12.0, 11.0, 10.0];

/// How much of a cap's width its name may use, leaving a visual margin.
const CAP_TEXT_INSET: f32 = 10.0;

/// Picks the largest cap size whose single-line width fits `max_w`.
///
/// Pure in `measure` so it can be tested against both synthetic widths and the
/// real font. Falls back to the smallest size: a label that does not fit even
/// there is clipped by the caller, never allowed to escape the cap.
fn cap_font_size(max_w: f32, measure: impl Fn(f32) -> f32) -> f32 {
    CAP_SIZES
        .into_iter()
        .find(|&size| measure(size) <= max_w)
        .unwrap_or(CAP_SIZES[CAP_SIZES.len() - 1])
}

fn key_cap(ui: &mut Ui, w: f32, fk: &str, name: &str, hot: bool, muted: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 54.0), Sense::CLICK);
    let (fill, fk_c, name_c, stroke) = if hot {
        (TOKENS.accent, TOKENS.surface, TOKENS.surface, TOKENS.accent)
    } else if resp.hovered() {
        (TOKENS.accent_soft, TOKENS.accent, TOKENS.ink, TOKENS.accent)
    } else {
        (
            TOKENS.surface,
            TOKENS.accent,
            if muted { TOKENS.ink_mute } else { TOKENS.ink },
            TOKENS.line,
        )
    };
    let p = ui.painter();
    p.rect_filled(rect, corner(), fill);
    p.rect_stroke(
        rect,
        corner(),
        Stroke::new(1.0_f32, stroke),
        StrokeKind::Inside,
    );
    p.text(
        rect.center_top() + vec2(0.0, 10.0),
        Align2::CENTER_CENTER,
        fk,
        mono_bold(11.0),
        fk_c,
    );
    // The name is shrunk to fit and then clipped to the cap. Painting it at a
    // fixed size let the longest label ("Reprint close report") run past the
    // cap's edge.
    let max_w = rect.width() - CAP_TEXT_INSET;
    let size = cap_font_size(max_w, |size| {
        ui.fonts(|f| {
            f.layout_no_wrap(name.to_owned(), sans_semibold(size), name_c)
                .size()
                .x
        })
    });
    ui.painter().with_clip_rect(rect).text(
        rect.center_bottom() - vec2(0.0, 15.0),
        Align2::CENTER_CENTER,
        name,
        sans_semibold(size),
        name_c,
    );
    resp.clicked()
}

// ---------------------------------------------------------------- hard stop

/// C1: the VSDC is unreachable and offline selling is prohibited. A full
/// ground in `ink`, the held basket, and no way to sell: the only control is
/// the supervisor's resume, which does not itself claim the VSDC is back.
fn hard_stop(ctx: &Context, state: &LaneState, reason: &str) -> Option<Action> {
    let mut tapped = None;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(TOKENS.ink).inner_margin(Margin::same(40)))
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(text("SELLING SUSPENDED", sans_bold(13.0), TOKENS.hi_vis));
                ui.add_space(12.0);
                ui.label(text("No connection to the VSDC", sans_bold(40.0), TOKENS.canvas));
                ui.add_space(14.0);
                ui.label(text(
                    "This till cannot issue a receipt until the connection is restored. The basket is held; nothing is lost.",
                    sans(18.0),
                    TOKENS.line,
                ));
                ui.add_space(8.0);
                ui.label(text(reason, sans(15.0), TOKENS.line));
                ui.add_space(8.0);
                ui.label(text("Call the supervisor if this persists.", sans(18.0), TOKENS.line));
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    // Centre the chip row.
                    let n = state.basket().len();
                    ui.add_space((ui.available_width() - 640.0).max(0.0) / 2.0);
                    chip(ui, "VSDC unreachable", TOKENS.danger, TOKENS.surface);
                    ghost_chip(ui, &format!("Basket held · {n} lines"));
                    ghost_chip(ui, "Sale not issued");
                });
                ui.add_space(28.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(360.0, 52.0), Sense::CLICK);
                let fill = if resp.hovered() { TOKENS.accent_hover } else { TOKENS.accent };
                ui.painter().rect_filled(rect, corner(), fill);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "F12  Supervisor: resume selling",
                    sans_semibold(16.0),
                    TOKENS.surface,
                );
                if resp.clicked() {
                    tapped = Some(Action::Resume);
                }
                ui.add_space(24.0);
                ui.label(text(
                    "CIS spec §24.7.9: a certified invoicing system must stop operating whenever a communication error appears.",
                    mono(12.0),
                    TOKENS.line,
                ));
            });
        });
    tapped
}

/// Shown until a cashier opens a shift. The float is keyed on the same wedge
/// buffer the scans use, and Enter opens the shift.
fn no_shift(ctx: &Context, state: &LaneState, local: &Local, session: &Session) {
    egui::CentralPanel::default()
        .frame(Frame::new().fill(TOKENS.ink).inner_margin(Margin::same(40)))
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(text("NO SHIFT OPEN", sans_bold(13.0), TOKENS.hi_vis));
                ui.add_space(12.0);
                ui.label(text("Open a shift to start selling", sans_bold(40.0), TOKENS.canvas));
                ui.add_space(14.0);
                ui.label(text(
                    format!(
                        "Count the drawer, key the opening float, and press Enter. The shift opens in the name of {}.",
                        session.cashier
                    ),
                    sans(18.0),
                    TOKENS.line,
                ));
                ui.add_space(24.0);
                let buffer = if state.buffer().is_empty() {
                    "Opening float ▏".to_string()
                } else {
                    format!("{} ▏", state.buffer())
                };
                ui.label(text(buffer, mono(28.0), TOKENS.canvas));
                ui.add_space(12.0);
                // A problem and a confirmation must look different on this dark
                // ground: the closing figures appear here, because closing the
                // shift is what puts the lane on this screen.
                match state.notice() {
                    Notice::Problem(m) => {
                        ui.label(text(m, sans(16.0), TOKENS.hi_vis));
                    }
                    Notice::Info(m) => {
                        ui.label(text(m, sans_bold(18.0), TOKENS.canvas));
                    }
                    Notice::None => {}
                }
                if let Some(n) = &local.note {
                    ui.label(text(n, sans(16.0), TOKENS.hi_vis));
                }
                if let Some(f) = &local.fault {
                    ui.label(text(f, sans(16.0), TOKENS.hi_vis));
                }
                ui.add_space(20.0);
                ui.label(text("ENTER  Open shift", mono(14.0), TOKENS.line));
            });
        });
}

fn ghost_chip(ui: &mut Ui, label: &str) {
    Frame::new()
        .stroke(Stroke::new(1.0_f32, TOKENS.ink_soft))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(text(label.to_uppercase(), mono_bold(12.0), TOKENS.line));
        });
}

// ---------------------------------------------------------------- fatal

/// A startup failure: the lane could not open, and must not look healthy.
/// Deliberately not the hard-stop screen, which means "wait for the VSDC".
pub fn fatal(ctx: &Context, message: &str) {
    egui::CentralPanel::default()
        .frame(Frame::new().fill(TOKENS.ink).inner_margin(Margin::same(40)))
        .show(ctx, |ui| {
            ui.add_space(60.0);
            ui.label(text(
                "THE LANE COULD NOT START",
                sans_bold(13.0),
                TOKENS.hi_vis,
            ));
            ui.add_space(12.0);
            ui.label(text("Call support", sans_bold(40.0), TOKENS.canvas));
            ui.add_space(14.0);
            ui.label(text(message, sans(18.0), TOKENS.line));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::lane::LaneState;
    use egui::{Event, Key, Modifiers, RawInput};
    use till_store::Store;

    fn key_event(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// A keystroke as a keyboard sends it: press, then release. egui drops a
    /// second press of a key it still believes is down.
    fn tap(key: Key) -> Vec<Event> {
        let mut up = key_event(key);
        if let Event::Key { pressed, .. } = &mut up {
            *pressed = false;
        }
        vec![key_event(key), up]
    }

    /// Ctrl+key, press then release.
    fn tap_ctrl(key: Key) -> Vec<Event> {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        [true, false]
            .into_iter()
            .map(|pressed| Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: ctrl,
            })
            .collect()
    }

    fn type_text(s: &str) -> Vec<Event> {
        vec![Event::Text(s.to_owned())]
    }

    struct Rig {
        rt: tokio::runtime::Runtime,
        ctx: Context,
        state: LaneState,
        devices: Devices,
    }

    impl Rig {
        fn new() -> Rig {
            Rig::build(true)
        }

        /// A till as it starts in the morning: no shift yet.
        fn without_shift() -> Rig {
            Rig::build(false)
        }

        fn build(with_shift: bool) -> Rig {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let state = rt.block_on(async {
                let store = Store::open_in_memory().await.unwrap();
                store.sync_catalogue(&demo::products()).await.unwrap();
                if with_shift {
                    store
                        .open_shift(&demo::config().dvc_srl_no, "cashier", Money::zero())
                        .await
                        .unwrap();
                }
                LaneState::new(store, demo::config()).await.unwrap()
            });
            let ctx = Context::default();
            theme::apply(&ctx);
            Rig {
                rt,
                ctx,
                state,
                devices: Devices::demo(false),
            }
        }

        /// One frame with these events; returns every string it painted.
        fn frame(&mut self, events: Vec<Event>) -> String {
            let _g = self.rt.enter();
            let input = RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    vec2(1280.0, 1024.0),
                )),
                ..Default::default()
            };
            let (state, devices) = (&mut self.state, &mut self.devices);
            let out = self.ctx.run(input, |ctx| lane(ctx, state, devices));
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// Scans these barcodes, one Enter each.
        fn scan(&mut self, codes: &[&str]) {
            for c in codes {
                self.frame(type_text(c));
                self.frame(tap(Key::Enter));
            }
        }

        fn due(&self) -> Money {
            self.state.basket().totals().unwrap().tot_amt
        }

        fn tenders(&self) -> Vec<(PmtTyCd, String)> {
            self.state
                .tenders()
                .entries()
                .iter()
                .map(|e| (e.pmt_ty_cd, e.amount.to_string()))
                .collect()
        }
    }

    #[test]
    fn the_cashier_opens_a_shift_from_the_no_shift_screen() {
        let mut r = Rig::without_shift();
        set_session(
            &r.ctx,
            Session {
                lane: "Lane 02".into(),
                cashier: "M. Uwase".into(),
            },
        );
        r.frame(vec![]); // the no-shift screen renders
        assert_eq!(*r.state.screen(), Screen::NoShift);
        // A barcode scanned here is not an opening float.
        let mut scan = type_text("5612345000017");
        scan.extend(tap(Key::Enter));
        r.frame(scan);
        assert_eq!(*r.state.screen(), Screen::NoShift);
        assert_eq!(r.state.buffer(), "");
        assert!(r.state.basket().is_empty());
        // Key the float; Enter opens the shift in the session's name.
        r.frame(type_text("5000"));
        assert_eq!(r.state.buffer(), "5000");
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::Selling);
        assert_eq!(
            r.state.shift().unwrap().opening_float.to_string(),
            "5000.00"
        );
        assert_eq!(r.state.shift().unwrap().opened_by, "M. Uwase");
        r.frame(type_text("5612345000017"));
        r.frame(tap(Key::Enter));
        assert_eq!(r.state.basket().len(), 1);
    }

    #[test]
    fn digits_and_enter_ring_up_a_scan() {
        let mut r = Rig::new();
        r.frame(type_text("5612345000017"));
        assert_eq!(r.state.buffer(), "5612345000017");
        r.frame(tap(Key::Enter));
        assert_eq!(r.state.basket().len(), 1);
        assert_eq!(r.state.buffer(), "");
    }

    #[test]
    fn a_wedge_burst_in_one_frame_is_the_same_as_typing() {
        let mut r = Rig::new();
        let mut ev = type_text("5449000000996");
        ev.extend(tap(Key::Enter));
        r.frame(ev);
        assert_eq!(r.state.basket().len(), 1);
    }

    #[test]
    fn sell_tender_settle_print_and_next_customer() {
        let mut r = Rig::new();
        r.frame(type_text("5612345000017"));
        r.frame(tap(Key::Enter));
        // Nothing keyed, F8: exact cash.
        r.frame(tap(Key::F8));
        assert_eq!(r.state.tender_total().unwrap().to_string(), "1200.00");
        // Enter with an empty buffer pays.
        r.frame(tap(Key::Enter));
        assert!(matches!(r.state.screen(), Screen::Settled { .. }));
        assert!(r.state.last_receipt().is_some());
        r.frame(vec![]); // the settled screen renders
        // Enter leaves Settled.
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::Selling);
        assert!(r.state.basket().is_empty());
    }

    #[test]
    fn keyed_cash_amount_and_void() {
        let mut r = Rig::new();
        r.frame(type_text("5612345000017"));
        r.frame(tap(Key::Enter));
        r.frame(type_text("5612345000024"));
        r.frame(tap(Key::Enter));
        r.frame(tap(Key::F1)); // voids the last line
        assert_eq!(r.state.basket().len(), 1);
        r.frame(type_text("2000"));
        r.frame(tap(Key::F8));
        assert_eq!(r.state.tender_total().unwrap().to_string(), "2000.00");
    }

    /// 7,100 due: two milks, two sugars, a Mutzig and a cola.
    const BASKET: [&str; 6] = [
        "5612345000017",
        "5612345000024",
        "5612345000031",
        "5449000000996",
        "5612345000017",
        "5612345000024",
    ];

    #[test]
    fn each_payment_key_adds_a_tender_under_its_own_rra_code() {
        let digits = [
            (Key::Num1, PmtTyCd::Cash),
            (Key::Num2, PmtTyCd::Credit),
            (Key::Num3, PmtTyCd::CashCredit),
            (Key::Num4, PmtTyCd::BankCheck),
            (Key::Num5, PmtTyCd::Card),
            (Key::Num6, PmtTyCd::MobileMoney),
            (Key::Num7, PmtTyCd::Other),
        ];
        for (key, code) in digits {
            let mut r = Rig::new();
            r.scan(&["5612345000017"]);
            r.frame(type_text("500"));
            r.frame(tap_ctrl(key));
            assert_eq!(r.tenders(), vec![(code, "500.00".to_owned())], "{code:?}");
            // Nothing keyed: the exact balance, under the same code.
            r.frame(tap_ctrl(key));
            assert_eq!(r.tenders()[1], (code, "700.00".to_owned()), "{code:?}");
        }
        // F8 is still cash, and the wire codes are the RRA's own.
        let mut r = Rig::new();
        r.scan(&["5612345000017"]);
        r.frame(tap(Key::F8));
        assert_eq!(r.tenders()[0].0.wire(), "01");
        assert_eq!(
            METHODS.map(|(c, ..)| c.wire()),
            ["01", "02", "03", "04", "05", "06", "07"]
        );
    }

    #[test]
    fn a_digit_without_ctrl_is_still_a_scan_digit() {
        let mut r = Rig::new();
        r.scan(&["5612345000017"]);
        r.frame(type_text("5"));
        assert_eq!(r.state.buffer(), "5");
        assert!(r.tenders().is_empty());
    }

    #[test]
    fn a_split_tender_across_two_methods_settles_when_the_total_covers_it() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        assert_eq!(r.due().to_string(), "7100.00");
        r.frame(type_text("5000"));
        r.frame(tap_ctrl(Key::Num5)); // card
        // Short: Enter must not settle.
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::Selling);
        r.frame(tap_ctrl(Key::Num1)); // cash, the exact remaining 2,100
        assert_eq!(
            r.tenders(),
            vec![
                (PmtTyCd::Card, "5000.00".to_owned()),
                (PmtTyCd::Cash, "2100.00".to_owned())
            ]
        );
        r.frame(tap(Key::Enter));
        assert_eq!(
            *r.state.screen(),
            Screen::Settled {
                change: Money::zero()
            }
        );
    }

    #[test]
    fn change_is_capped_by_cash_received_in_a_split_tender() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        r.frame(type_text("8000"));
        r.frame(tap_ctrl(Key::Num5)); // card 8,000 against 7,100
        let shown = r.frame(tap(Key::Enter));
        assert_eq!(
            *r.state.screen(),
            Screen::Selling,
            "card overpay must not settle"
        );
        assert!(r.state.last_receipt().is_none());
        assert!(shown.contains("NO CHANGE ON NON-CASH"), "{shown}");
        // A token cash tender does not cover 900 of change either.
        r.frame(type_text("100"));
        r.frame(tap_ctrl(Key::Num1));
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::Selling);
        // Mobile money is non-cash too.
        let mut m = Rig::new();
        m.scan(&BASKET);
        m.frame(type_text("8000"));
        m.frame(tap_ctrl(Key::Num6));
        m.frame(tap(Key::Enter));
        assert_eq!(*m.state.screen(), Screen::Selling);
        // The cap is not a blanket refusal: cash received can cover change.
        let mut ok = Rig::new();
        ok.scan(&["5612345000017"]);
        ok.frame(type_text("1000"));
        ok.frame(tap_ctrl(Key::Num5));
        ok.frame(type_text("500"));
        ok.frame(tap_ctrl(Key::Num1));
        ok.frame(tap(Key::Enter));
        assert_eq!(
            *ok.state.screen(),
            Screen::Settled {
                change: Money::try_from_str("300").unwrap()
            }
        );
    }

    #[test]
    fn the_rail_shows_each_tender_line_with_its_code() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        r.frame(type_text("5000"));
        r.frame(tap_ctrl(Key::Num5));
        let shown = r.frame(type_text("1000"));
        assert!(shown.contains("05") && shown.contains("Card"), "{shown}");
        assert!(shown.contains(&fmt::rwf(Money::try_from_str("5000").unwrap())));
        assert!(shown.contains("STILL TO PAY"), "{shown}");
        r.frame(tap_ctrl(Key::Num6));
        let shown = r.frame(vec![]);
        assert!(
            shown.contains("06") && shown.contains("Mobile money"),
            "{shown}"
        );
        assert!(shown.contains(&fmt::rwf(Money::try_from_str("1000").unwrap())));
        assert!(shown.contains("STILL TO PAY"));
        assert!(shown.contains("1,100"), "{shown}");
    }

    #[test]
    fn the_balance_is_shown_before_the_first_tender_is_taken() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        let shown = r.frame(vec![]);
        assert!(shown.contains("STILL TO PAY"), "{shown}");
        assert!(shown.contains("7,100"), "{shown}");
    }

    #[test]
    fn f11_takes_back_a_mis_keyed_card_tender_and_the_sale_can_then_settle() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        r.frame(type_text("8000"));
        r.frame(tap_ctrl(Key::Num5)); // card 8,000 against 7,100: a mis-key
        let shown = r.frame(vec![]);
        assert!(shown.contains("NO CHANGE ON NON-CASH"), "{shown}");
        let shown = r.frame(tap(Key::F11));
        assert!(r.tenders().is_empty());
        assert!(!shown.contains("NO CHANGE ON NON-CASH"), "{shown}");
        assert!(shown.contains("STILL TO PAY"), "{shown}");
        r.frame(tap_ctrl(Key::Num5)); // card, the exact balance
        r.frame(tap(Key::Enter));
        assert_eq!(
            *r.state.screen(),
            Screen::Settled {
                change: Money::zero()
            }
        );
    }

    #[test]
    fn ctrl_backspace_clears_every_tender_and_keeps_the_basket() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        r.frame(type_text("1000"));
        r.frame(tap_ctrl(Key::Num5));
        r.frame(type_text("500"));
        r.frame(tap_ctrl(Key::Num6));
        assert_eq!(r.tenders().len(), 2);
        let lines = r.state.basket().len();
        r.frame(tap_ctrl(Key::Backspace));
        assert!(r.tenders().is_empty());
        assert_eq!(r.state.basket().len(), lines);
        assert_eq!(r.due().to_string(), "7100.00");
    }

    #[test]
    fn the_undo_keys_are_on_screen() {
        let mut r = Rig::new();
        r.scan(&BASKET);
        let shown = r.frame(vec![]);
        assert!(
            shown.contains("Undo tender") && shown.contains("F11"),
            "{shown}"
        );
        assert!(shown.contains("Clear tenders"), "{shown}");
    }

    #[test]
    fn an_opening_float_with_cents_is_not_mistaken_for_a_barcode() {
        let mut r = Rig::without_shift();
        r.frame(type_text("10000.50"));
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::Selling);
        assert_eq!(
            r.state.shift().unwrap().opening_float.to_string(),
            "10000.50"
        );
    }

    #[test]
    fn an_eight_digit_entry_is_refused_as_a_barcode() {
        // EAN-8 is the shortest barcode, and the case the guard exists for.
        let mut r = Rig::without_shift();
        r.frame(type_text("96385074"));
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::NoShift);
        assert_eq!(r.state.buffer(), "");
        let local: Local = r.ctx.data(|d| d.get_temp(local_id())).unwrap();
        assert!(local.note.unwrap().contains("barcode"));
        // Seven digits is the longest float accepted.
        r.frame(type_text("9999999"));
        r.frame(tap(Key::Enter));
        assert_eq!(*r.state.screen(), Screen::Selling);
    }

    #[test]
    fn the_hard_stop_renders_and_cannot_be_sold_through() {
        let mut r = Rig::new();
        r.devices.fiscal = Box::new(crate::fiscal::SimFiscal {
            next_rcpt_no: 1,
            mode: crate::fiscal::SimMode::Unreachable,
        });
        r.frame(type_text("5612345000017"));
        r.frame(tap(Key::Enter));
        r.frame(tap(Key::F8));
        r.frame(tap(Key::Enter));
        assert!(matches!(r.state.screen(), Screen::HardStop { .. }));
        r.frame(vec![]); // it renders
        // Scans and Enter do nothing while stopped.
        r.frame(type_text("5612345000024"));
        r.frame(tap(Key::Enter));
        assert_eq!(r.state.basket().len(), 1);
        assert!(matches!(r.state.screen(), Screen::HardStop { .. }));
        // The basket is held; a supervisor resumes.
        r.frame(tap(Key::F12));
        assert_eq!(*r.state.screen(), Screen::Selling);
        assert_eq!(r.state.basket().len(), 1);
    }

    #[test]
    fn unbuilt_function_keys_say_so_and_change_nothing() {
        let mut r = Rig::new();
        r.frame(tap(Key::F2));
        let local: Local = r.ctx.data(|d| d.get_temp(local_id())).unwrap();
        assert!(local.note.unwrap().contains("Discount"));
        assert!(r.state.basket().is_empty());
    }

    #[test]
    fn actions_from_ignores_key_repeat_and_releases() {
        let ev = vec![
            Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: Modifiers::NONE,
            },
        ];
        assert!(actions_from(&ev).is_empty());
    }

    #[test]
    fn basket_columns_tile_the_row_exactly_at_any_width() {
        for width in [640.0_f32, 840.0, 1280.0] {
            let row = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(width, 38.0));
            let cols = column_rects(row);
            assert!(cols[1].width() > 0.0, "item column at {width}");
            // Fixed columns keep their width; the item column flexes.
            let fixed = [
                (0, COL_SEQ),
                (2, COL_QTY),
                (3, COL_PRICE),
                (4, COL_BAND),
                (5, COL_AMOUNT),
            ];
            for (i, w) in fixed {
                assert_eq!(cols[i].width(), w);
            }
            // Gapped, in order, and the amount ends ROW_PAD short of the edge.
            for pair in cols.windows(2) {
                assert!((pair[1].left() - pair[0].right() - GAP).abs() < 0.01);
            }
            assert!((cols[5].right() - (width - ROW_PAD)).abs() < 0.01);
        }
    }

    /// Whatever the text, rows (header and data) must lay out identically:
    /// the rect each cell is given must not depend on its content.
    #[test]
    fn row_cells_do_not_move_with_their_content() {
        let ctx = Context::default();
        theme::apply(&ctx);
        let mut seen: Vec<Vec<f32>> = Vec::new();
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                vec2(840.0, 400.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for name in ["Inyange Fresh Milk 1L", "Kabuye Sugar 1kg", "I"] {
                    let xs = std::cell::RefCell::new(Vec::new());
                    let probe = |i: usize| {
                        let xs = &xs;
                        move |ui: &mut Ui| {
                            // Measured before the content goes in.
                            xs.borrow_mut().push(ui.max_rect().left());
                            ui.label(name);
                            let _ = i;
                        }
                    };
                    let (p0, p1, p2, p3, p4, p5) =
                        (probe(0), probe(1), probe(2), probe(3), probe(4), probe(5));
                    row(ui, 38.0, [&p0, &p1, &p2, &p3, &p4, &p5]);
                    seen.push(xs.into_inner());
                }
            });
        });
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[0], seen[2]);
    }

    #[test]
    fn fatal_screen_renders() {
        let ctx = Context::default();
        theme::apply(&ctx);
        let _ = ctx.run(RawInput::default(), |ctx| fatal(ctx, "database unreadable"));
    }

    fn tap_shift_f7() -> Vec<Event> {
        [true, false]
            .into_iter()
            .map(|pressed| Event::Key {
                key: Key::F7,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            })
            .collect()
    }

    /// Every label a key cap can show, from the same tables the UI draws from.
    fn every_cap_label() -> Vec<&'static str> {
        let mut v: Vec<&str> = FKEYS.iter().map(|(_, _, n, _)| *n).collect();
        v.extend(METHODS.iter().map(|(_, _, _, cap)| *cap));
        v.extend(SHIFT_KEYS.iter().map(|(_, n, _)| *n));
        v.extend(["Clear tenders", "Scan / Pay", "Next customer"]);
        v
    }

    /// The defect: `key_cap` painted its name at a fixed 13pt with no width
    /// limit, so a label wider than the cap ran over the edge. Shrinking is the
    /// fix; this test is what proves every real label actually lands inside its
    /// cap at the size the shrinker picks, measured with the real font.
    #[test]
    fn no_key_cap_label_paints_wider_than_its_cap() {
        let ctx = Context::default();
        theme::apply(&ctx);
        // Fonts are only available once a frame has run.
        let _ = ctx.run(RawInput::default(), |_| {});

        let max_w = cap_width() - CAP_TEXT_INSET;
        let mut too_wide = vec![];
        for name in every_cap_label() {
            let size = cap_font_size(max_w, |size| {
                ctx.fonts(|f| {
                    f.layout_no_wrap(name.to_owned(), sans_semibold(size), TOKENS.ink)
                        .size()
                        .x
                })
            });
            let w = ctx.fonts(|f| {
                f.layout_no_wrap(name.to_owned(), sans_semibold(size), TOKENS.ink)
                    .size()
                    .x
            });
            if w > max_w {
                too_wide.push(format!(
                    "{name:?} is {w:.1}px at {size}pt, cap allows {max_w:.1}px"
                ));
            }
        }
        assert!(
            too_wide.is_empty(),
            "these labels do not fit a key cap even at the smallest size:\n  {}",
            too_wide.join("\n  ")
        );
    }

    #[test]
    fn cap_font_size_takes_the_largest_that_fits_and_never_goes_below_the_floor() {
        // A measure that is proportional to size: 10px per point.
        let measure = |size: f32| size * 10.0;
        assert_eq!(cap_font_size(130.0, measure), 13.0, "13pt fits at 130px");
        assert_eq!(
            cap_font_size(125.0, measure),
            12.0,
            "13pt does not, 12pt does"
        );
        assert_eq!(cap_font_size(115.0, measure), 11.0);
        assert_eq!(cap_font_size(105.0, measure), 10.0);
        // Nothing fits: the floor, so the caller clips rather than growing.
        assert_eq!(cap_font_size(1.0, measure), 10.0);
    }

    /// A name search draws its list, numbered from 1 the way `pick` counts.
    #[test]
    fn a_name_search_draws_a_numbered_pick_list() {
        let mut r = Rig::new();
        // The demo catalogue has no two names sharing a word except these.
        let painted = r.frame(type_text("ka"));
        assert!(!painted.contains("PICK AN ITEM"), "no list before Enter");
        let painted = r.frame(tap(Key::Enter));
        // "Kabuye Sugar 1kg" is the only "ka", so it is added with no list.
        assert!(
            !painted.contains("PICK AN ITEM"),
            "a single match needs no list: {painted}"
        );
        assert_eq!(r.state.basket().len(), 1);
    }

    #[test]
    fn several_matches_draw_the_list_and_a_number_picks_from_it() {
        let mut r = Rig::new();
        // "ml" is in exactly two demo names: Coca-Cola 500ml and Mutzig 650ml.
        // Neither starts with it, so they are offered alphabetically.
        r.frame(type_text("ml"));
        let painted = r.frame(tap(Key::Enter));
        assert_eq!(r.state.search_matches().len(), 2);
        assert!(painted.contains("PICK AN ITEM"), "{painted}");
        assert!(painted.contains("Coca-Cola 500ml"), "{painted}");
        assert!(painted.contains("Mutzig 650ml"), "{painted}");
        assert!(r.state.basket().is_empty(), "offering must add nothing");

        r.frame(type_text("2"));
        r.frame(tap(Key::Enter));
        assert_eq!(
            r.state.basket().lines().last().unwrap().item_nm,
            "Mutzig 650ml",
            "2 must take the second row offered"
        );
        let painted = r.frame(vec![]);
        assert!(!painted.contains("PICK AN ITEM"), "the list closes");
    }

    /// A tap on a row must reach the same place as keying its number.
    #[test]
    fn tapping_a_row_picks_it() {
        let mut r = Rig::new();
        r.frame(type_text("ml"));
        r.frame(tap(Key::Enter));
        assert_eq!(r.state.search_matches().len(), 2);
        r.rt.block_on(r.state.pick_nth(1)).unwrap();
        assert_eq!(
            r.state.basket().lines().last().unwrap().item_nm,
            "Coca-Cola 500ml"
        );
    }

    #[test]
    fn a_keyed_count_reaches_the_basket_through_the_buffer() {
        let mut r = Rig::new();
        r.frame(type_text("3*5612345000017"));
        r.frame(tap(Key::Enter));
        assert_eq!(r.state.basket().len(), 1, "one line, not three");
        assert_eq!(
            r.state.basket().lines().last().unwrap().qty.to_string(),
            "3.000"
        );
    }

    /// The star must reach the buffer as ordinary text, like every other
    /// character a wedge or a keyboard produces.
    #[test]
    fn the_count_separator_is_plain_buffer_text() {
        assert_eq!(
            actions_from(&type_text("3*")),
            vec![Action::Char('3'), Action::Char('*')]
        );
    }

    #[test]
    fn the_scan_hint_names_every_way_in_and_changes_while_picking() {
        let mut r = Rig::new();
        let painted = r.frame(vec![]);
        assert!(
            painted.contains("3*code"),
            "the hint must teach the count: {painted}"
        );
        assert!(painted.contains("type a name"), "{painted}");
    }

    fn tap_shift(key: Key) -> Vec<Event> {
        [true, false]
            .into_iter()
            .map(|pressed| Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            })
            .collect()
    }

    #[test]
    fn shift_f12_is_the_vsdc_switch_and_plain_f12_is_still_resume() {
        assert_eq!(
            actions_from(&tap_shift(Key::F12)),
            vec![Action::ToggleVsdc],
            "Shift+F12 must switch the simulated VSDC"
        );
        assert_eq!(
            actions_from(&tap(Key::F12)),
            vec![Action::Resume],
            "plain F12 must still resume from a hard stop"
        );
    }

    /// The whole point of the switch: a hard stop can be produced by hand, and
    /// the lane comes back from it. This is CIS §24.7.8 and §24.7.9 end to end,
    /// driven the way an operator at a till would drive it.
    #[test]
    fn the_vsdc_switch_produces_a_hard_stop_and_the_lane_recovers() {
        let mut r = Rig::new();

        // Take the VSDC away.
        r.frame(tap_shift(Key::F12));
        assert!(r.devices.vsdc_offline());

        // Sell, and try to settle.
        r.frame(type_text("5612345000017"));
        r.frame(tap(Key::Enter));
        r.frame(tap(Key::F8));
        r.frame(tap(Key::Enter));

        assert!(
            matches!(r.state.screen(), Screen::HardStop { .. }),
            "an unreachable VSDC must hard stop the lane, got {:?}",
            r.state.screen()
        );
        // Nothing was printed and no number was burned on a sale the VSDC
        // never saw.
        assert!(r.state.last_receipt().is_none());

        // The screen says so, and says why.
        let painted = r.frame(vec![]);
        assert!(painted.contains("F12"), "the resume key must be on screen");

        // Put the VSDC back, then resume.
        r.frame(tap_shift(Key::F12));
        assert!(!r.devices.vsdc_offline());
        r.frame(tap(Key::F12));
        assert_eq!(*r.state.screen(), Screen::Selling);

        // And the lane sells again.
        r.frame(type_text("5612345000017"));
        r.frame(tap(Key::Enter));
        r.frame(tap(Key::F8));
        r.frame(tap(Key::Enter));
        assert!(
            matches!(r.state.screen(), Screen::Settled { .. }),
            "after the VSDC returns the lane must settle, got {:?}",
            r.state.screen()
        );
        assert!(r.state.last_receipt().is_some());
    }

    #[test]
    fn the_status_bar_says_when_the_demo_vsdc_is_switched_off() {
        let mut r = Rig::new();
        // `chip` upper-cases every label, as the other status chips show.
        assert!(!r.frame(vec![]).contains("DEMO: VSDC SWITCHED OFF"));
        r.frame(tap_shift(Key::F12));
        let painted = r.frame(vec![]);
        assert!(
            painted.contains("DEMO: VSDC SWITCHED OFF"),
            "an operator must be able to see the switch is thrown. painted:\n{painted}"
        );
    }

    #[test]
    fn f7_prints_an_x_report_and_leaves_the_shift_open() {
        let mut r = Rig::new();
        r.frame(tap(Key::F7));
        r.frame(tap(Key::F7));
        assert!(r.state.shift().is_some());
        assert_eq!(*r.state.screen(), Screen::Selling);
        match r.state.notice() {
            Notice::Info(m) => assert!(m.contains("Shift report printed"), "{m}"),
            n => panic!("{n:?}"),
        }
    }

    #[test]
    fn z_needs_a_keyed_count_and_a_stray_press_closes_nothing() {
        let mut r = Rig::new();
        let shown = r.frame(tap_shift_f7());
        assert!(shown.contains("Key the cash you counted"), "{shown}");
        assert!(r.state.shift().is_some());
        // A scanned barcode is not a cash count either.
        r.frame(type_text("6001234567890"));
        r.frame(tap_shift_f7());
        assert!(r.state.shift().is_some());
        assert_eq!(r.state.buffer(), "");
        // Plain F7 is the harmless shift report, never a close.
        r.frame(type_text("1000"));
        r.frame(tap(Key::F7));
        assert!(r.state.shift().is_some());
    }

    #[test]
    fn shift_f7_with_a_counted_amount_closes_the_shift() {
        let mut r = Rig::new();
        r.frame(type_text("1000"));
        let painted = r.frame(tap_shift_f7());
        assert!(r.state.shift().is_none());
        assert_eq!(*r.state.screen(), Screen::NoShift);
        // What the cashier is shown, not what the state holds: the closing
        // figures are on the frame that follows a successful close.
        assert!(
            painted.contains("Close report printed") && painted.contains("variance +1,000"),
            "{painted}"
        );
        assert!(
            r.frame(vec![]).contains("variance +1,000"),
            "and they stay painted"
        );
    }

    #[test]
    fn every_ctrl_key_cap_is_derived_from_the_binding_it_labels() {
        let mut r = Rig::new();
        let shown = r.frame(vec![]);
        for (_, key, ..) in METHODS.iter().filter(|(c, ..)| *c != PmtTyCd::Cash) {
            assert!(shown.contains(&ctrl_cap(*key)), "{key:?} cap in {shown}");
        }
        assert!(shown.contains(&shift_cap(Key::F7)) && shown.contains("Close shift"));
        // Derived from SHIFT_KEYS rather than spelled out, so shortening a
        // label to fit its cap cannot silently leave this test behind.
        let f9 = SHIFT_KEYS
            .iter()
            .find(|(k, ..)| *k == Key::F9)
            .expect("F9 is a shift key");
        assert!(
            shown.contains("Shift F9") && shown.contains(f9.1),
            "{shown}"
        );
        assert!(shown.contains("Shift report"));
    }

    #[test]
    fn the_status_bar_shows_the_real_shift_not_a_placeholder() {
        let mut r = Rig::new();
        let shown = r.frame(vec![]);
        let opened = r.state.shift().unwrap().clone();
        let at = crate::lane::kigali_hhmm(&opened.opened_at).unwrap();
        let want = format!("SHIFT OPEN {at} · {}", opened.opened_by.to_uppercase());
        assert!(shown.contains(&want), "{want} in {shown}");
        assert!(
            !shown.contains("06:58") || at == "06:58",
            "no hardcoded time"
        );
    }
}
