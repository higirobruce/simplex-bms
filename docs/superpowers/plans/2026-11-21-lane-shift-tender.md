# Shift, Mixed Tender and X/Z Reports Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The lane can trade a full day — open a shift, take any RRA payment
method, split a tender across methods, and close the shift with a cash
declaration and an auditable report.

**Architecture:** A `shift` table in `till-store` mirroring the cloud's existing
Prisma `Shift`, with `sale.shift_id` as a non-null foreign key. `LaneState` gains
shift state and refuses to sell without one. X and Z reports compose through the
existing `escpos::Esc` builder. Mixed tender needs no core work — `TenderSet`
already accepts every code and derives the header code from per-code totals.

**Tech Stack:** Rust 2024, `sqlx` (runtime API only), `egui`/`eframe` 0.32, the
merged `till-core`, `till-store` and `till-lane`.

**Spec:** `docs/superpowers/specs/2026-11-20-supermarket-retail-design.md` §8
Phase 2d and C5b. Also read `docs/reference/rra/OPEN-QUESTIONS.md`.

## Why this slice exists

The cloud's `Sale.shiftId` is **non-null** (`prisma/schema.prisma:713`) and a
`Shift` model already exists there. `till-store` has no shift concept at all and
`cloud_payload` emits no `shiftId`, so **every sale the till records today is
unsyncable**. Shift is a prerequisite for the back office to receive anything, not
a reporting nicety. That is what drives the ordering below.

## Global Constraints

- Whole-franc rounding at the **line**; tax keeps 2 decimal places.
- Header tax is the **sum of the line taxes**, never recomputed from a band total.
- `sqlx`'s `macros` feature stays disabled.
- Currency RWF, timezone Africa/Kigali.
- **C1:** the lane hard-stops when the fiscal port is unreachable.
- Nothing panics on a path reachable mid-sale.
- **The error contract:** `Err(LaneError)` only for faults the cashier cannot act
  on (store I/O); `Ok(..)` with a `Notice::Problem` for anything they can.
- Money never passes through a float. `Money`/`Qty` expose no decimal accessor
  outside `till-core`; format through `fmt::rwf`, `fmt::tax`, `fmt::qty`.
- **Mirror the cloud's `Shift` shape**: `opened_by`, `opened_at`, `closed_at`,
  `opening_float`, `declared_cash`. Do not invent a different model.
- **This slice MAY modify `till-store`.** The previous plan froze it; this one
  changes it deliberately. `till-core` stays frozen — it already does everything
  this slice needs.

## Out of Scope — state this, do not build it

- Suspend and resume; refunds (CIS §14 needs a referenced receipt number);
  discounts; buyer TIN capture. Later 2d slices.
- The real VSDC client. Phase 3.
- MoMo or card terminal integration. C5b decided: the till **records** the tender
  under its RRA code and takes the cashier's confirmation that money arrived,
  exactly as it does for cash.

## Review Focus

Five failure modes the spec implies that no happy path exercises. Each has a test
assigned to the task that owns the code.

1. **Selling with no shift open.** Must refuse, and must not create a sale row
   pointing at a shift that does not exist. (Task 4)
2. **Closing a shift while a sale is still `AwaitingVsdc`.** Must refuse — closing
   would orphan a live invoice number from its shift. (Task 7)
3. **A Z report on an already-closed shift.** Must refuse; a Z is final. (Task 7)
4. **A declared cash count far from expected.** The variance is recorded, never
   rejected — shrinkage is data the shop needs, not an error. (Task 7)
5. **A split tender where cash is less than the change due.** Change is cash-only
   and capped by cash received, so a card overpayment yields no change. (Task 5)

---

## File Structure

```
till/crates/till-store/migrations/0002_shift.sql   shift table; sale.shift_id NOT NULL
till/crates/till-store/src/shift.rs                open/current/close, shift totals
till/crates/till-store/src/sale.rs                 NewSale.shift_id; cloud_payload emits shiftId
till/crates/till-lane/src/lane.rs                  shift state, gating, tender by code
till/crates/till-lane/src/report.rs                X and Z report composition
till/crates/till-lane/src/ui.rs                    tender keys, shift screens, report keys
```

---

### Task 1: The shift schema

**Files:**
- Create: `till/crates/till-store/migrations/0002_shift.sql`
- Modify: `till/crates/till-store/src/schema.rs` (register the migration)

**Interfaces:**
- Produces: tables `shift` and a rebuilt `sale` carrying `shift_id`.

The migrator is hand-built from `include_str!` and a test compares the sorted
`*.sql` filenames against the registered names — so a new file that is not
registered fails that test. Read `schema.rs` before adding the file.

**The `sale` rebuild.** SQLite cannot `ALTER TABLE ADD COLUMN ... NOT NULL`
without a default, and a nullable `shift_id` would permit exactly the unsyncable
sale this slice exists to prevent. So `0002` follows SQLite's table-rebuild idiom:
create the new `sale`, copy rows across, drop the old, rename. **If any sale rows
exist, the migration first inserts one synthetic shift** (`opened_by =
'migration'`, `opening_float = '0.00'`, closed at the migration time) and assigns
the orphans to it — a correct migration must not lose data even though no
deployed till has any yet.

`shift` columns, mirroring the cloud: `id TEXT PRIMARY KEY`, `dvc_srl_no TEXT NOT
NULL`, `opened_by TEXT NOT NULL`, `opened_at TEXT NOT NULL`, `closed_at TEXT`,
`opening_float TEXT NOT NULL`, `declared_cash TEXT`. Money is **TEXT holding an
exact decimal string**, as everywhere else in this schema — read the comment at
the top of `0001_init.sql` for why.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test] async fn a_sale_cannot_exist_without_a_shift() {
    // Insert a sale row with a shift_id that matches no shift; the FK refuses it.
    // Foreign keys are per-connection in SQLite and the store enables them, so
    // this proves the constraint is live, not merely declared.
}
#[tokio::test] async fn an_open_shift_has_no_closed_at() { }
```

- [ ] **Step 2: Run them and watch them fail** — `cargo test -p till-store shift`

- [ ] **Step 3: Write `0002_shift.sql` and register it**

- [ ] **Step 4: Run the migrator test too**

Run: `cargo test -p till-store` — the filename/registration test must still pass.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(store): shift table and sale.shift_id"
```

---

### Task 2: Opening and reading a shift

**Files:**
- Create: `till/crates/till-store/src/shift.rs`
- Modify: `till/crates/till-store/src/lib.rs` (declare and re-export)

**Interfaces:**
- Produces:
```rust
pub struct OpenShift { pub id: String, pub opened_by: String,
                       pub opened_at: String, pub opening_float: Money }
impl Store {
    pub async fn open_shift(&self, dvc_srl_no: &str, opened_by: &str,
                            opening_float: Money) -> Result<OpenShift, StoreError>;
    pub async fn current_shift(&self, dvc_srl_no: &str)
        -> Result<Option<OpenShift>, StoreError>;
}
```
- New `StoreError` variants: `ShiftAlreadyOpen`, `NoOpenShift`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn opening_a_shift_returns_it_as_current() { }
#[tokio::test] async fn a_second_open_shift_on_one_device_is_refused() {
    // ShiftAlreadyOpen. One device trades one shift at a time.
}
#[tokio::test] async fn two_devices_each_hold_their_own_open_shift() {
    // The constraint is per device, not global — two lanes trade at once.
}
#[tokio::test] async fn a_negative_opening_float_is_refused() { }
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement `shift.rs`**

Open under `BEGIN IMMEDIATE`, like every other write in this crate — the check for
an existing open shift and the insert must not race.

- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 3: Every sale belongs to a shift

**Files:**
- Modify: `till/crates/till-store/src/sale.rs`

**Interfaces:**
- Consumes: Task 2's `OpenShift`.
- Produces: `NewSale` gains `pub shift_id: &'a str`; `cloud_payload` emits
  `"shiftId"`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn the_cloud_payload_carries_the_shift_id() {
    // The whole point of the slice: without this the cloud rejects the sale,
    // because Prisma's Sale.shiftId is non-null.
}
#[tokio::test] async fn beginning_a_sale_against_a_closed_shift_is_refused() {
    // No invoice number is allocated — check the counter did not move.
}
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Add the field, the check and the payload key**

The shift check belongs **before** the number allocation, so a refused sale spends
no invoice number.

- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 4: The lane refuses to sell without a shift

**Files:**
- Modify: `till/crates/till-lane/src/lane.rs`

**Interfaces:**
- Produces, on `LaneState`:
```rust
pub async fn open_shift(&mut self, opened_by: &str, opening_float: Money)
    -> Result<(), LaneError>;
pub fn shift(&self) -> Option<&OpenShift>;
```
- `Screen` gains `NoShift`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn a_lane_with_no_open_shift_starts_on_the_no_shift_screen() { }
#[tokio::test] async fn scanning_with_no_shift_open_is_refused_and_adds_nothing() {
    // Review Focus 1. Ok(()) with a Problem naming what to do; basket empty.
}
#[tokio::test] async fn opening_a_shift_moves_the_lane_to_selling() { }
#[tokio::test] async fn a_lane_resumes_an_existing_open_shift_on_startup() {
    // A till restarted mid-shift must not demand a second opening float.
}
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement**

`new` calls `current_shift` alongside its existing pending-sale check. A store
failure there is already `Err`; an absent shift is `Screen::NoShift`, not an error.

**Where `opened_by` comes from:** `LaneConfig` has no cashier field, and the UI
currently passes the session's cashier name through `ui::set_session` with a
hardcoded value. Take `opened_by` as an argument on `open_shift` and let the UI
supply that same session name. Do **not** add a cashier to `LaneConfig` — real
sign-in is a later slice, and a config field would look like identity without
being it.

- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 5: Mixed tender

**Files:**
- Modify: `till/crates/till-lane/src/ui.rs`

**Interfaces:**
- Consumes: `LaneState::add_tender(PmtTyCd, Money)`, which already accepts every
  code — this task is UI only.

Keys, one per RRA code from C5b: cash `01`, credit `02`, cash/credit `03`, bank
check `04`, card `05`, mobile money `06`, other `07`. Keying an amount then
pressing a method key adds that tender; pressing a method key with nothing keyed
tenders the exact remaining balance.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn each_payment_key_adds_a_tender_under_its_own_rra_code() { }
#[test] fn a_split_tender_across_two_methods_settles_when_the_total_covers_it() {
    // e.g. 5,000 card + 2,450 cash against 7,450.
}
#[test] fn change_is_capped_by_cash_received_in_a_split_tender() {
    // Review Focus 5. 8,000 card + 0 cash against 7,450 yields NO change —
    // till-core enforces it; this proves the UI does not route around it.
}
#[test] fn the_rail_shows_each_tender_line_with_its_code() { }
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement the keys and the tender list**

Use `fmt::rwf` for every amount and `TOKENS` for every colour. `TOKENS.hi_vis` is
a ground for dark text only.

- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 6: Shift totals

**Files:**
- Modify: `till/crates/till-store/src/shift.rs`

**Interfaces:**
- Produces:
```rust
pub struct ShiftTotals {
    pub sale_count: u32, pub voided_count: u32,
    pub gross: Money, pub tax: Money,
    pub by_code: Vec<(PmtTyCd, Money)>,
    pub opening_float: Money, pub expected_cash: Money,
}
impl Store { pub async fn shift_totals(&self, shift_id: &str)
    -> Result<ShiftTotals, StoreError>; }
```

`expected_cash` is `opening_float` plus cash tendered minus change given. Only
code `01` counts as cash — `03` (cash/credit) does not, matching how `till-core`
caps change.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn totals_count_confirmed_sales_and_exclude_abandoned_ones() { }
#[tokio::test] async fn totals_split_tenders_by_their_rra_code() { }
#[tokio::test] async fn expected_cash_is_float_plus_cash_taken_minus_change() { }
#[tokio::test] async fn a_cash_credit_tender_does_not_count_toward_expected_cash() {
    // 03 is not 01. The same distinction till-core makes for change.
}
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 7: Closing a shift

**Files:**
- Modify: `till/crates/till-store/src/shift.rs`, `till/crates/till-lane/src/lane.rs`

**Interfaces:**
- Produces:
```rust
pub struct ShiftClose { pub totals: ShiftTotals, pub declared_cash: Money,
                        pub variance: Money }   // declared - expected; may be negative
impl Store { pub async fn close_shift(&self, shift_id: &str, declared_cash: Money)
    -> Result<ShiftClose, StoreError>; }
```
- On `LaneState`: `pub async fn close_shift(&mut self, declared_cash: Money) -> Result<ShiftClose, LaneError>`.
- New `StoreError` variants: `ShiftAlreadyClosed`, `ShiftHasPendingSale`.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn closing_records_the_declared_cash_and_the_variance() { }
#[tokio::test] async fn a_short_till_records_a_negative_variance_rather_than_failing() {
    // Review Focus 4. Shrinkage is data the shop needs. Declare 1,000 short:
    // the close succeeds and variance is -1,000.
}
#[tokio::test] async fn closing_a_shift_with_a_pending_sale_is_refused() {
    // Review Focus 2. A sale still AwaitingVsdc holds a live invoice number;
    // closing would orphan it from its shift.
}
#[tokio::test] async fn closing_an_already_closed_shift_is_refused() {
    // Review Focus 3. A Z is final.
}
#[tokio::test] async fn a_closed_shift_is_no_longer_current() { }
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement**

Close under `BEGIN IMMEDIATE`. After closing, the lane returns to `Screen::NoShift`.

- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 8: X and Z reports

**Files:**
- Create: `till/crates/till-lane/src/report.rs`
- Modify: `till/crates/till-lane/src/ui.rs`, `till/crates/till-lane/src/lane.rs`

**Interfaces:**
- Consumes: `escpos::Esc`, `fmt::*`, `ShiftTotals`, `ShiftClose`.
- Produces: `report::x_report(totals: &ShiftTotals, cfg: &LaneConfig, at: &str) -> Vec<u8>`
  and `report::z_report(close: &ShiftClose, cfg: &LaneConfig, at: &str) -> Vec<u8>`.
- On `LaneState`: `pub async fn print_x(&mut self, printer: &mut dyn Printer) -> Result<(), LaneError>`
  and `pub async fn print_z(&mut self, declared_cash: Money, printer: &mut dyn Printer) -> Result<(), LaneError>`.

An **X report is a mid-shift snapshot and changes nothing.** A **Z report closes
the shift and is final.** Both are internal shop documents, not fiscal receipts —
they carry no QR, no SDC block and no invoice number, and nothing about them goes
to the VSDC. Say so in the module doc, because the next reader will wonder whether
C5 applies.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn an_x_report_does_not_close_the_shift() {
    // Print X twice, then still sell. The shift is open throughout.
}
#[tokio::test] async fn a_z_report_closes_the_shift_and_a_second_z_is_refused() { }
#[tokio::test] async fn both_reports_show_every_tender_code_that_was_used() { }
#[tokio::test] async fn the_z_report_prints_the_variance_with_its_sign() { }
#[tokio::test] async fn neither_report_carries_a_qr_or_an_sdc_block() {
    // They are not fiscal documents. Assert the absence, so nobody "helpfully"
    // adds a QR later and makes an internal report look like a receipt.
}
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement, and wire F7 to X and a separate key to Z**

Reset bold, size and alignment before `cut()`, as `receipt.rs` does — printer
state persists across documents.

- [ ] **Step 4: Run the whole suite and the lints**

Run: `cargo test --workspace`, `cargo fmt --all --check`,
`cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

---

### Task 9: The shift reaches the cloud

**Added during execution.** Task 3's review found that `cloud_payload` carries
`shiftId` but nothing identifying the lane, while the cloud's `Sale.laneId` is
non-null — and, more seriously, **the till never sends shifts to the cloud at
all**. A queued sale would therefore fail on a foreign key to a `Shift` row the
back office has never seen, which defeats the stated purpose of this whole slice.

**Files:**
- Modify: `till/crates/till-store/src/shift.rs`, `till/crates/till-store/src/sale.rs`

**Interfaces:**
- Consumes: Task 2's `open_shift`, Task 7's `close_shift`.
- Produces: outbound rows of `kind = 'shift'`.

**How the cloud resolves the lane.** The till knows its device serial, not the
cloud's `Lane.id`. `Lane.dvcSrlNo` is unique per org, so the shift payload carries
`dvcSrlNo` and the cloud resolves `laneId` from it. The sale payload then needs
nothing further — the cloud reaches `laneId` through the sale's shift. The till
never learns cloud primary keys, which is the right direction for that dependency.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn opening_a_shift_queues_it_for_the_cloud() {
    // One outbound row, kind 'shift', carrying the shift id, dvcSrlNo, openedBy,
    // openedAt and openingFloat.
}
#[tokio::test] async fn closing_a_shift_queues_the_close() {
    // A second outbound row carrying closedAt and declaredCash.
}
#[tokio::test] async fn the_shift_is_queued_before_any_sale_that_references_it() {
    // Ordering matters: the cloud must be able to create the Shift before the
    // Sale that points at it. Assert the shift row's id is lower.
}
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement**

Queue inside the same transaction that opens or closes the shift, exactly as
`confirm_sale` queues its sale — a shift that exists locally but was never queued
is the bug this task exists to prevent.

- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**

---

### Task 10: A mis-keyed tender can be taken back

**Added during execution.** Task 5's implementer identified a hazard that Task 5
itself created. With cash the only method, an overpayment always produced change,
so a mis-key was self-correcting. Once a tender can be non-cash, it is not: change
is cash-only and capped by cash received, so a cashier who taps card for 8,000 on a
7,450 sale cannot produce the 550 — and cannot undo the tender either. The sale
settles and the customer is **silently overcharged**. Mis-keys happen constantly on
a real counter, so this closes a money-losing hole in the slice that opened it.

**Files:**
- Modify: `till/crates/till-lane/src/lane.rs`, `till/crates/till-lane/src/ui.rs`

**Interfaces:**
- Produces, on `LaneState`:
```rust
pub fn remove_last_tender(&mut self) -> Result<(), LaneError>;
pub fn clear_tenders(&mut self) -> Result<(), LaneError>;
```

Both operate only before settlement — a tender on a sale already sent to the VSDC
is history, not a draft. `TenderSet`'s field is private and `till-core` is frozen,
so rebuild the set from the entries you keep rather than reaching into it.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn the_last_tender_can_be_taken_back_before_settling() { }
#[tokio::test] async fn clearing_tenders_leaves_the_basket_untouched() {
    // The lines are the customer's shopping; only the payment was wrong.
}
#[tokio::test] async fn a_mis_keyed_card_overpayment_can_be_corrected() {
    // The hazard, end to end: card 8,000 against 7,450 due, remove it,
    // tender 7,450 by card instead, settle. No overcharge.
}
#[tokio::test] async fn tenders_cannot_be_removed_once_the_sale_is_settled() { }
#[tokio::test] async fn removing_from_an_empty_tender_set_is_a_problem_not_an_error() { }
```

- [ ] **Step 2: Run and watch fail**
- [ ] **Step 3: Implement, and wire a key for each**
- [ ] **Step 4: Run and watch pass**
- [ ] **Step 5: Commit**
