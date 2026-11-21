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
use crate::lane::{LaneState, Notice, Screen};
use crate::theme::{self, TOKENS};
use eframe::egui::{
    self, Align, Align2, Color32, Context, CornerRadius, FontFamily, FontId, Frame, Layout, Margin,
    Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2,
};
use till_core::Money;
use till_core::codes::{PmtTyCd, TaxType};
use till_core::line::LineDiscount;

/// Who is at the lane. `LaneConfig` carries none of this, so `main` supplies it
/// through the context (see [`set_session`]).
#[derive(Debug, Clone)]
pub struct Session {
    pub lane: String,
    pub cashier: String,
    pub shift: String,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            lane: "LANE".into(),
            cashier: "NO CASHIER".into(),
            shift: "NO SHIFT".into(),
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
    Cash,
    Reprint,
    CancelSale,
    Resume,
    NotBuilt(&'static str),
}

/// Function keys: (egui key, label, name, action). The first seven are the
/// mockup's; the lane has no discount, suspend, refund, buyer-TIN or report
/// flow yet, so those say so rather than doing nothing silently.
const FKEYS: [(egui::Key, &str, &str, Action); 10] = [
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
    (
        egui::Key::F7,
        "F7",
        "X / Z report",
        Action::NotBuilt("X / Z report"),
    ),
    (egui::Key::F8, "F8", "Cash", Action::Cash),
    (egui::Key::F9, "F9", "Reprint", Action::Reprint),
    (egui::Key::F10, "F10", "Cancel sale", Action::CancelSale),
];

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
                ..
            } => match key {
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

fn apply(action: Action, state: &mut LaneState, devices: &mut Devices, local: &mut Local) {
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
        Action::Cash => tender_cash(state, local),
        Action::Reprint => state.reprint(devices.printer.as_mut()),
        Action::CancelSale => block_on(state.abandon_pending()),
        Action::Resume => state.resume_from_hard_stop(),
        Action::NotBuilt(name) => {
            local.note = Some(format!("{name} is not available in this build."));
            Ok(())
        }
    };
    local.fault = result.err().map(|e| e.to_string());
}

/// F8: what is keyed is the cash handed over; nothing keyed means exact cash.
fn tender_cash(state: &mut LaneState, local: &mut Local) -> Result<(), crate::lane::LaneError> {
    if *state.screen() != Screen::Selling {
        return Ok(());
    }
    let amount = if state.buffer().is_empty() {
        let due = state.basket().totals().map(|t| t.tot_amt).ok();
        let paid = state.tender_total();
        match (due, paid) {
            (Some(d), Some(p)) => d.checked_sub(p).filter(|m| *m > Money::zero()),
            _ => None,
        }
    } else {
        Money::try_from_str(state.buffer()).ok()
    };
    match amount {
        Some(a) => state.add_tender(PmtTyCd::Cash, a),
        None => {
            local.note = Some("Key the cash amount first, then press F8.".into());
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
        apply(a, state, devices, &mut local);
    }

    let mut tapped = Vec::new();
    status_bar(ctx, state, &session);
    match state.screen().clone() {
        Screen::HardStop { reason } => {
            if let Some(a) = hard_stop(ctx, state, &reason) {
                tapped.push(a);
            }
        }
        _ => {
            if let Some(a) = rail(ctx, state, &local) {
                tapped.push(a);
            }
            basket_panel(ctx, state, &local);
        }
    }
    for a in &tapped {
        apply(*a, state, devices, &mut local);
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

fn status_bar(ctx: &Context, state: &LaneState, session: &Session) {
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
                    // The VSDC is "online" unless the lane has hard-stopped.
                    match state.screen() {
                        Screen::HardStop { .. } => {
                            chip(ui, "VSDC unreachable", TOKENS.danger, TOKENS.surface)
                        }
                        _ => chip(ui, "VSDC online", TOKENS.hi_vis, TOKENS.ink),
                    }
                    ui.label(text(clock(), mono(14.0), TOKENS.line));
                    ui.label(text(session.shift.to_uppercase(), sans(14.0), TOKENS.line));
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

/// One row of the basket grid: `#`, item (takes what is left), qty, price,
/// band, amount.
fn row(ui: &mut Ui, height: f32, cells: [&dyn Fn(&mut Ui); 6]) {
    let width = ui.available_width();
    let item_w =
        width - 2.0 * ROW_PAD - COL_SEQ - COL_QTY - COL_PRICE - COL_BAND - COL_AMOUNT - 5.0 * GAP;
    let widths = [
        COL_SEQ,
        item_w.max(80.0),
        COL_QTY,
        COL_PRICE,
        COL_BAND,
        COL_AMOUNT,
    ];
    ui.horizontal(|ui| {
        ui.add_space(ROW_PAD);
        ui.spacing_mut().item_spacing.x = GAP;
        for (i, cell) in cells.iter().enumerate() {
            let layout = match i {
                0 | 1 => Layout::left_to_right(Align::Center),
                4 => Layout::centered_and_justified(egui::Direction::LeftToRight),
                _ => Layout::right_to_left(Align::Center),
            };
            ui.allocate_ui_with_layout(vec2(widths[i], height), layout, |ui| cell(ui));
        }
    });
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
                    ui.label(text(fmt::qty(line.qty), mono_bold(15.0), TOKENS.ink));
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
        .exact_width(440.0)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(TOKENS.canvas)
                .stroke(Stroke::new(1.0_f32, TOKENS.line))
                .inner_margin(Margin::same(12)),
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
                "Tendered · 01 Cash",
                tendered,
                TOKENS.surface,
                TOKENS.ink,
                TOKENS.ink_mute,
            );
            if settled.is_none() {
                let change = tendered
                    .checked_sub(due)
                    .filter(|m| *m > Money::zero())
                    .unwrap_or(Money::zero());
                amount_tile(
                    ui,
                    "Change",
                    change,
                    TOKENS.surface,
                    TOKENS.ink,
                    TOKENS.ink_mute,
                );
            }

            scan_field(ui, state);
            tapped = keys(ui, state);
        });
    tapped
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
                let hint = if selling { "Scan or key PLU ▏" } else { "" };
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
    let mut all: Vec<(&str, &str, Action, bool, bool)> = FKEYS
        .iter()
        .map(|(_, k, n, a)| (*k, *n, *a, false, matches!(a, Action::NotBuilt(_))))
        .collect();
    all.push((enter.0, enter.1, Action::Enter, enter.2, false));

    let cols = 4;
    let gap = 6.0;
    let w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    for chunk in all.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (fk, name, action, hot, muted) in chunk {
                if key_cap(ui, w, fk, name, *hot, *muted) {
                    tapped = Some(*action);
                }
            }
        });
    }
    tapped
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
    p.text(
        rect.center_bottom() - vec2(0.0, 15.0),
        Align2::CENTER_CENTER,
        name,
        sans_semibold(13.0),
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
            let rt = tokio::runtime::Runtime::new().unwrap();
            let state = rt.block_on(async {
                let store = Store::open_in_memory().await.unwrap();
                store.sync_catalogue(&demo::products()).await.unwrap();
                LaneState::new(store, demo::config()).await.unwrap()
            });
            let ctx = Context::default();
            theme::apply(&ctx);
            Rig {
                rt,
                ctx,
                state,
                devices: Devices::demo(),
            }
        }

        /// One frame with these events.
        fn frame(&mut self, events: Vec<Event>) {
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
            let _ = self.ctx.run(input, |ctx| lane(ctx, state, devices));
        }
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
    fn fatal_screen_renders() {
        let ctx = Context::default();
        theme::apply(&ctx);
        let _ = ctx.run(RawInput::default(), |ctx| fatal(ctx, "database unreadable"));
    }
}
