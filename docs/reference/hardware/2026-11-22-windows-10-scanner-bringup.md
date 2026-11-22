# Windows 10 + barcode reader bring-up

**Date opened:** 2026-11-22
**Hardware:** one Windows 10 x64 laptop, one barcode reader. No printer, no scale,
no cash drawer.
**Build:** cross-compiled on macOS to `x86_64-pc-windows-msvc` via `cargo-xwin`.
The target triple is the one that will ship, so this is not a toolchain
substitute.

## What this hardware can and cannot settle

| Spec item | Status after this box |
|---|---|
| C9 — "runs on whatever Windows the shop already owns" | **Partly.** Proves Windows 10 x64. Does not prove the pilot's till, which is still unknown hardware. |
| Open question 6 — Windows version decides Tauri vs pure-Rust | **Moot.** We chose pure-Rust egui to remove the dependency. Nothing is waiting on the answer. |
| Phase 2b — scan source | **Yes.** This is the first real scanner. |
| Phase 2b — printer, scale, drawer | **No.** None present. The drawer usually hangs off the printer's RJ11, so no printer means no drawer either. |
| Open question 7 — pilot shop + Microinvest baseline | **No.** A dev laptop is not a pilot shop; the Phase 2 speed gate stays unmeasurable. |
| `latency.rs` budgets | **First real numbers.** They are currently a stand-in, by their own module docs. |

Windows 10 left support on 2025-10-14. For a dev box that does not matter, and
for C9 it is an advantage: it is closer to what a Kigali shop actually runs
than Windows 11 would be.

## Before you start

The scan path needs **no new code**. `device.rs:45` declares `ScanSource` and
nothing polls it, with the reason in the code: the OS delivers keyboard wedge
input as ordinary keystrokes. `ui.rs:178` accumulates `Event::Text` into the
buffer; `Enter` commits it. If the reader is a USB HID keyboard wedge — almost
all are — it should already work.

Two things to expect and not report as faults:

- **A black console window opens next to the till.** There is no
  `#![windows_subsystem = "windows"]` in the build, deliberately: the cheat
  sheet and every `eprintln!` land there, which is what makes this session
  debuggable. It gets suppressed when we build the Phase 2a installer.
- **Nothing persists.** `--demo` calls `Store::open_in_memory()`
  (`main.rs:69`), so the shift, the sales and the invoice sequence all vanish
  on exit. Each run is a clean till, which is good for repeatability and bad
  for one thing: this branch fixed "the close report survives a restart", and
  that fix **cannot be exercised** on an in-memory store. Testing it needs a
  `--db <path>` flag the binary does not have yet.

Run it as `simplex-lane.exe --demo`. Without `--demo` it refuses to start, on
purpose: there is no VSDC client yet and a simulator must never ring up a real
sale.

### What this build can and cannot reach

| Behaviour | Status |
|---|---|
| Close report surviving a restart | **Testable.** Pass `--db <path>`. |
| The C1 hard stop and F12 resume (CIS §24.7.8/§24.7.9) | **Testable.** `--vsdc-offline`, or Shift+F12 mid-shift. |
| Printer, scale, cash drawer | **Out of reach** — the hardware is not present. |

Run it one of two ways:

    simplex-lane.exe --demo
    simplex-lane.exe --demo --db till.db

With `--db`, two companion files appear next to it (`till.db-wal` and
`till.db-shm`). That is SQLite's write-ahead log, not a fault. Deleting the
`-wal` file between runs throws away the most recent writes, so leave all
three alone.

`--vsdc-offline` starts with the fiscal device unreachable, so the very first
sale hard stops. More useful mid-session is **Shift+F12**, which takes the
simulated VSDC away and gives it back while the lane runs. The status bar shows
`DEMO: VSDC SWITCHED OFF` while the switch is thrown — and says "demo", because
it reports how the *simulator* is set and must never be read as a claim about a
real device.

An unknown flag is refused rather than ignored, deliberately: a mistyped `--db`
that silently fell back to memory would look exactly like data loss at the end
of a shift.

## The checklist, in the order that fails cheapest first

### 1. It starts

- [ ] The window opens, maximised, and the status bar reads Lane 02 / M. Uwase.
- [ ] The console shows the demo cheat sheet.
- [ ] If it does not start at all, copy the console text verbatim — a startup
      fault is never a hard stop, and the message says which it is.

### 2. The scanner reaches the buffer

Scan anything at the "no shift" screen first — this is the guard test, and it
is the one most likely to find a real defect.

- [ ] Scan any 13-digit barcode with no shift open. **Expected:** the scan is
      rejected and no shift opens. `ui.rs:302` caps the opening float at
      7 digits precisely because the shortest barcode (EAN-8) has 8. If a
      shift opens with a float in the millions, that guard has failed.
- [ ] Open a shift properly: key a float such as `20000`, press Enter.
- [ ] Scan `5449000000996` — a real Coca-Cola GTIN, in the demo catalogue at
      500 RWF. An actual Coke bottle may ring up on the first scan, though
      Coca-Cola assigns different GTINs per market and size, so Rwandan
      packaging may not match. If it does not, type the digits by hand to
      separate "wrong barcode" from "scanner not working".
- [ ] Scan an unknown barcode. **Expected:** a notice naming the unknown code,
      not a crash and not a silent no-op.

Record for each scan:

- Does the reader send a trailing **Enter**? If the code appears in the buffer
  but never commits, it does not, and it needs reconfiguring (usually a
  programming barcode in the reader's manual).
- Does the **full** code arrive? Compare the buffer against the printed
  digits. Our side of this is already proven:
  `a_wedge_burst_in_one_frame_is_the_same_as_typing` (`ui.rs:1441`) fires a
  whole barcode plus Enter inside one frame and asserts one basket line, and
  `actions_from` is pure and order-preserving by construction. So a short
  buffer means the **reader** is dropping or stripping, not the till.
- Does it send the **check digit**? A 12-digit buffer from a 13-digit label
  means the reader is stripping it.

### 3. The function keys

F10 and F11 are the suspicious pair. Windows has historically reserved F10 for
menu activation, and our map puts destructive actions on both.

- [ ] F1 Void line
- [ ] F7 Shift report — read-only snapshot; must **not** close the shift
- [ ] F9 Reprint — must come out marked **Copy** (CIS §7.18)
- [ ] F8 Cash tender (also Ctrl+1 — it is dual-bound for cashiers who know F8)
- [ ] **F10 Cancel sale** — does it reach the app, or does Windows eat it?
- [ ] **F11 Undo tender** — same question; many apps bind F11 to fullscreen.
- [ ] **Shift+F7 Close shift** — a chord because closing is irreversible.
- [ ] **Shift+F12** — demo only: take the simulated VSDC offline or restore it.
- [ ] F2 Discount, F3 Suspend, F4 Resume, F5 Refund, F6 Buyer TIN should each
      say plainly that they are not built, rather than doing nothing.

Two keys both say "Resume" and they mean different things: **F4** is resuming a
suspended sale (not built), **F12** is resuming from a hard stop (built). They
never appear together — F12 is drawn only on the hard-stop screen, and labelled
there — so this is a vocabulary collision to keep an eye on, not a defect.

`ui.rs:146` already notes that "the operating system reserves Ctrl+F-key chords
on some machines" — which is why close is Shift+F7 and not Ctrl+F7. That same
reasoning is what makes F10 and F11 worth testing deliberately.

If F10 or F11 do not arrive, say so and the map gets changed. They are the
wrong keys for destructive actions if the OS is ambiguous about them.

### 4. The hard stop — the most important test here

This is constraint C1 and CIS §24.7.8/§24.7.9: a certified invoicing system
must stop operating when it cannot reach the fiscal device. Until now only the
test suite had ever seen it happen.

- [ ] With a shift open and an item in the basket, press **Shift+F12**. The
      status bar should show `DEMO: VSDC SWITCHED OFF`.
- [ ] Settle the sale. **Expected:** the lane stops, shows the hard-stop
      screen, and prints nothing. No receipt, no drawer kick.
- [ ] Check the basket is **held**, not lost — the screen should say so.
- [ ] Press **Shift+F12** again to bring the VSDC back, then **F12** to resume.
- [ ] Sell again. **Expected:** it settles normally.
- [ ] Now the subtle one, and worth doing with `--db`: after the refused sale,
      does the next successful sale take the **next** invoice number, or does it
      skip one? A sale the VSDC never saw must not consume a number. The suite
      asserts this, but this is the first time it runs on a real file.

### 5. Mixed tender, end to end

- [ ] Ring up three items by scanning.
- [ ] Pay part on Card (Ctrl+5), the rest in Cash (Ctrl+1). Note that methods
      are Ctrl+digit, not bare digits — a bare digit is wedge text
      (`ui.rs:119`).
- [ ] Over-tender the cash. **Expected:** change, cash only, capped by the cash
      received.
- [ ] Try to over-tender the *card*. **Expected:** a refusal the cashier can
      act on, not a silent overcharge.
- [ ] Close the shift (**Shift+F7**), declare a cash count deliberately 500
      short. **Expected:** the close report shows a signed variance of -500.

### 6. Does it feel fast

No stopwatch needed — the question is whether a cashier would notice.

- [ ] Scan five items as fast as the reader allows. Does the basket keep up?
- [ ] Hold the reader's trigger on one label for a continuous burst. Does the
      UI stay responsive, and does the basket match the scan count?

Anything that feels slow here is the first real datum against the `latency.rs`
budgets, which are currently invented.

## Findings

_To be filled in from the session. One line per result; defects get an issue._
