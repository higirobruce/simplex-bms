# The Counter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A shop owner opens the back office and sees the counter — what each shift took, whether the drawer balanced, and whether the figures can be trusted yet.

**Architecture:** A new `/api/[slug]/counter` route returns a per-shift roll-up; `/[slug]/counter` is a client page using `@tanstack/react-query` and `apiCall`, matching every other page under `[slug]`. The till's shift-close payload gains its own `saleCount` and `gross` so the cloud can tell a complete shift from one missing refused sales. Days are bounded in `Org.timezone`, which nothing in the app currently honours.

**Tech Stack:** Next.js 14.2 App Router, Prisma, Postgres, `@tanstack/react-query`, `Intl.DateTimeFormat` for zones; Rust 2024 + `sqlx` runtime API on the till.

**Spec:** `docs/superpowers/specs/2026-11-22-counter-view-design.md` (approved 2026-11-22)

## Global Constraints

- Money is exact decimals end to end. Read `Decimal` columns, return them as **strings**, do arithmetic server-side in `Decimal`. Never `Number(...)` a money value.
- RWF, Africa/Kigali. Days are bounded in `Org.timezone`, never the server's zone.
- Hand-authored Prisma migrations applied with `migrate deploy` only. **Never** `prisma migrate dev`, `migrate diff` or `migrate reset` (R25 — all need a shadow database this project lacks).
- Till migrations are hand-authored; `sqlx` runtime API only, the `macros` feature is disabled.
- `Money`/`Qty` expose no decimal accessor outside `till-core`; only `Display` (`{:.2}`, `{:.3}`).
- **Never show a variance that cannot be stood behind.** A shift whose data is incomplete shows no number.
- A backlog is normal, not an error. Nothing in this view treats waiting as a fault.
- Nothing in this view is editable. A fiscalised sale is a record.
- **Read routes use `getTenantContext()` alone.** This app has no read permissions — only `:write`, `:delete` and `:approve` — so the spec's "ADMIN and manager" has no mechanism. Introducing the first read permission is out of scope; any authenticated org member can see the counter, as with stock and products.

## Review Focus

Five conditions the spec implies that no happy path exercises. Each has a test pinned to the task that owns it.

1. **A shift spanning midnight in Kigali** — it belongs to the day it *opened*, and a UTC-bounded query would file it under the wrong day. (Task 1, Task 5)
2. **An over, not a short** — `declaredCash` above expected must display as prominently as a short but in the calmer colour, never as a negative short. (Task 6)
3. **A shift that sold nothing** — opened, nothing rung up, closed. Expected drawer is the float alone; the variance maths must not divide by or assume a sale. (Task 5)
4. **A sale where change was given** — tenders exceed `totAmt`, so a naive method split exceeds takings. The split must sum to takings with change shown as a deduction. (Task 5)
5. **A lane with no token, or only revoked ones** — `lastUsedAt` is null, so staleness must neither crash nor claim the lane is reachable. (Task 5)

---

## File Structure

- `src/lib/day.ts` — `dayBounds` and `kigaliParts`. Zone arithmetic, nothing else. Used by the counter and the dashboard.
- `src/lib/counter.ts` — the variance and state logic, pure and testable without a database.
- `src/app/api/[slug]/counter/route.ts` — the roll-up query.
- `src/app/api/[slug]/counter/[shiftId]/route.ts` — one shift's sales.
- `src/app/[slug]/counter/page.tsx` — the page.
- `prisma/migrations/<ts>_shift_totals/migration.sql` — `Shift.saleCount`, `Shift.gross`.
- `till/crates/till-store/src/shift.rs` — the close payload gains two fields.
- `src/app/api/till/shift/route.ts` — accepts and stores them.
- `src/app/api/[slug]/dashboard/stats/route.ts` — revenue split in two.

---

### Task 1: Days in the org's timezone

**Files:**
- Create: `src/lib/day.ts`, `tests/day.test.ts`

**Interfaces:**
- Produces: `dayBounds(day: Date, tz: string): { start: Date; end: Date }` — the UTC instants bounding that calendar day in `tz`, `start` inclusive and `end` exclusive.
- Produces: `startOfMonthIn(day: Date, tz: string): Date`.

- [ ] **Step 1: Write the failing tests**

```ts
test("a Kigali day is bounded at 22:00 UTC the previous evening", …)   // UTC+2, no DST
test("a sale at 01:30 Kigali belongs to that Kigali day, not the UTC one", …)
test("the month boundary is the first of the month in the zone, not the server's", …)
test("an unknown zone falls back to UTC rather than throwing", …)
```

- [ ] **Step 2: Run them to verify they fail**

Run: `env -u NODE_ENV npx vitest run tests/day.test.ts`

- [ ] **Step 3: Implement both functions in `src/lib/day.ts`**

Use `Intl.DateTimeFormat` with `timeZone` to read the zone's offset at that instant; add no dependency. Rwanda is UTC+2 year round with no DST, so a fixed-offset shortcut would pass every test here and silently break for any other org — do not take it.

- [ ] **Step 4: Run tests to verify they pass**

- [ ] **Step 5: Commit**

---

### Task 2: The cloud learns what a shift should contain

**Files:**
- Modify: `prisma/schema.prisma`
- Create: `prisma/migrations/<timestamp>_shift_totals/migration.sql`, `tests/schema/shift-totals.test.ts`

**Interfaces:**
- Produces: `Shift.saleCount Int?` and `Shift.gross Decimal? @db.Decimal(12, 2)`. Both null means *completeness unknown*.

- [ ] **Step 1: Add both fields to `prisma/schema.prisma`**

Nullable, because every shift closed before this ships has neither.

- [ ] **Step 2: Hand-author the migration**

Follow the layout of `prisma/migrations/20261122140000_saleline_amounts/`.

- [ ] **Step 3: Apply with `npx prisma migrate deploy`**

Expected: applied, no drift. **Never** `migrate dev`.

- [ ] **Step 4: Write and run the tests**

```ts
test("a shift can be created with neither field, meaning unknown", …)
test("both fields round-trip as exact decimals", …)
```

- [ ] **Step 5: Commit**

---

### Task 3: The till sends its own count and gross

**Files:**
- Modify: `till/crates/till-store/src/shift.rs` (the close payload, around line 282)

**Interfaces:**
- Consumes: `ShiftTotals { sale_count, gross, … }` from `shift.rs:40`, already computed for the close report.
- Produces: the close payload gains `"saleCount"` (number) and `"gross"` (exact decimal string).

- [ ] **Step 1: Write the failing test**

```rust
fn the_close_payload_carries_the_shifts_own_count_and_gross()
// opens a shift, rings up two confirmed sales, closes it, and asserts the
// queued close payload's saleCount is 2 and gross equals their summed tot_amt
fn an_open_payload_still_carries_neither()
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd till && cargo test -p till-store the_close_payload_carries`

- [ ] **Step 3: Add both fields to the close payload**

The totals are already computed for the close report — reuse them rather than recounting, so the number the cloud compares against is the same one the cashier's report showed. `gross` is a `Money`, so it reaches JSON via `Display`, never a float.

- [ ] **Step 4: Run `cargo test -p till-store`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`**

- [ ] **Step 5: Commit**

---

### Task 4: The ingest stores them

**Files:**
- Modify: `src/app/api/till/shift/route.ts`
- Modify: `tests/till-ingest-routes.test.ts`

**Interfaces:**
- Consumes: Task 2's columns, Task 3's payload fields.

- [ ] **Step 1: Write the failing tests**

```ts
test("a close carrying saleCount and gross stores both", …)
test("a close without them stores nulls and is still accepted", …)   // pre-change tills
test("they are refused on an OPEN payload with 422", …)              // only meaningful on a close
test("a replayed close writes them on the existing row", …)          // the upsert's update branch
test("a non-integer saleCount or a bad money string is 422", …)
```

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Accept and store both fields**

Validate `gross` with the route's existing money regex and `saleCount` as a non-negative integer. Accept them only alongside `closedAt`, and write them in the upsert's `update` branch as well as `create` — a replayed close must not leave them null.

- [ ] **Step 4: Run the suite and `tsc --noEmit`**

- [ ] **Step 5: Commit**

---

### Task 5: The roll-up

**Files:**
- Create: `src/lib/counter.ts`, `src/app/api/[slug]/counter/route.ts`, `tests/counter-logic.test.ts`, `tests/counter-route.test.ts`

**Interfaces:**
- Consumes: `dayBounds` (Task 1), `Shift.saleCount`/`gross` (Task 2).
- Produces:
  - `type ShiftState = "open" | "stale" | "complete" | "short_count" | "unknown" | "mismatch"`
  - `shiftState(input: { closedAt: Date | null; lastSeenAt: Date | null; saleCount: number | null; gross: string | null; heldCount: number; heldGross: string; now: Date }): ShiftState` — pure.
  - `variance(input: { openingFloat: string; cashTendered: string; changeGiven: string; declaredCash: string | null }): { expected: string; declared: string | null; diff: string | null }` — exact decimal strings throughout.
  - `GET /api/[slug]/counter?day=YYYY-MM-DD` → `{ day, shifts: [...], total, totalQualified: boolean }`.
- The staleness threshold is **20 minutes** of no contact for an open shift, or an open shift whose `openedAt` is before the current day's start.

- [ ] **Step 1: Write the failing logic tests** (`tests/counter-logic.test.ts`, no database)

```ts
test("an open shift seen two minutes ago is open, not stale", …)
test("an open shift not seen for 21 minutes is stale", …)
test("an open shift carried over from yesterday is stale however recently seen", …)
test("a lane that has never reported does not crash and is not called reachable", …)  // Review Focus 5
test("counts and gross matching is complete", …)
test("the till reporting more sales than we hold is short_count", …)
test("counts matching while gross differs is mismatch, never complete", …)
test("neither field present is unknown", …)
test("expected drawer is float plus cash tendered minus change given", …)
test("a shift that sold nothing expects exactly its float", …)                        // Review Focus 3
test("declared above expected yields a positive diff", …)                             // Review Focus 2
test("no declaredCash yields a null diff rather than a zero", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement `shiftState` and `variance` in `src/lib/counter.ts`**

Pure functions over strings and dates — no Prisma import, so they stay testable without a database.

- [ ] **Step 4: Write the failing route tests** (`tests/counter-route.test.ts`, guarded test database)

```ts
test("shifts are grouped by the day they OPENED, in the org's timezone", …)           // Review Focus 1
test("a shift opening 01:30 Kigali is not filed under the previous day", …)
test("the method split sums to takings, with change shown as a deduction", …)         // Review Focus 4
test("credit sales appear as their own line and are not hidden in the total", …)
test("the shop total is marked unqualified when any shift is not complete", …)
test("money is returned as strings, never numbers", …)
test("another org's shifts never appear", …)
test("an unauthenticated request is refused", …)
```

- [ ] **Step 5: Run them to verify they fail**

Run: `env -u NODE_ENV npx vitest run tests/counter-route.test.ts`
Expected: FAIL — the route does not exist.

- [ ] **Step 6: Implement the route**

`getTenantContext()` for the tenant — there is no read permission to use. `lastSeenAt` is the max `LaneToken.lastUsedAt` over the lane's non-revoked tokens; it means **reachable**, not "sent a sale", so name the field accordingly. Resolve cashier names in a second query, since `Shift.openedById` has no Prisma relation to `User`. Exclude `voidedAt IS NOT NULL`.

- [ ] **Step 7: Run both test files, then `tsc --noEmit`**

- [ ] **Step 8: Commit**

---

### Task 6: The page

**Files:**
- Create: `src/app/[slug]/counter/page.tsx`, `tests/counter-page.test.tsx`
- Modify: the sidebar nav (wherever `[slug]/layout.tsx` renders it)

**Interfaces:**
- Consumes: `GET /api/[slug]/counter` (Task 5).

- [ ] **Step 1: Write the failing tests**

```tsx
test("a short shows the shortfall, the expected drawer and the counted cash", …)
test("an over reads as over, never as a negative short", …)                           // Review Focus 2
test("a stale open shift is flagged and says when the lane was last reachable", …)
test("a short_count shift shows NO variance number and names the gap", …)
test("an unknown shift shows takings and no variance", …)
test("the shop total sits below the shifts and says when it is incomplete", …)
test("nothing on the page is an input or a button that mutates", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Build the page**

`"use client"`, `useTenant()` for the slug, `useQuery` with `refetchInterval: 45_000` and `refetchOnWindowFocus`. Layout and copy follow the approved mockup — https://claude.ai/artifact/FfZCGfvSan6dxDCDm2C3aL — including that the variance leads each shift and the shop total sits underneath. Add a "Counter" sidebar entry beside Sales.

- [ ] **Step 4: Run the tests**

- [ ] **Step 5: Commit**

---

### Task 7: Inside a shift

**Files:**
- Create: `src/app/api/[slug]/counter/[shiftId]/route.ts`, `tests/counter-shift-route.test.ts`
- Modify: `src/app/[slug]/counter/page.tsx`

**Interfaces:**
- Produces: `GET /api/[slug]/counter/[shiftId]` → that shift's sales, each with lines and tenders.

- [ ] **Step 1: Write the failing tests**

```ts
test("a shift's sales come back with their lines and tenders", …)
test("line amounts are the stored splyAmt, not a recomputation", …)
test("a shift from another org is 404", …)
test("the response is labelled as a back-office record, not a fiscal copy", …)
```

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement the route and the expanding row**

Show the stored `splyAmt`/`taxblAmt`, never `qty × prc` recomputed — the till rounds once at the line and a recomputation disagrees with the header. The product name shown is the product's *current* name; the response carries a flag saying this is a back-office record rather than the fiscal copy.

- [ ] **Step 4: Run the tests, then `tsc --noEmit`**

- [ ] **Step 5: Commit**

---

### Task 8: The dashboard stops ignoring the counter

**Files:**
- Modify: `src/app/api/[slug]/dashboard/stats/route.ts`, `src/app/[slug]/dashboard/page.tsx`
- Create: `tests/dashboard-revenue.test.ts`

**Interfaces:**
- Consumes: `startOfMonthIn` (Task 1).
- Produces: `revenueMTD` becomes `{ invoices: string; counter: string }`.

- [ ] **Step 1: Write the failing tests**

```ts
test("counter takings come from Sale.totAmt, not from tenders", …)   // tenders include change
test("invoice payments and counter takings are reported separately", …)
test("the month boundary is computed in the org's timezone", …)
test("expenses keep their own boundary variable and do not shift zone", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Split the figure and the boundary**

`startOfMonth` is currently one variable shared by revenue and expenses (`stats/route.ts:10`). Split it rather than repointing it, or expenses silently move to a different zone from revenue. Show two labelled lines on the dashboard, not one merged number: invoice payments and counter takings are different money with different timing, and adding them hides which one moved.

- [ ] **Step 4: Run the tests, then `tsc --noEmit`**

- [ ] **Step 5: Commit**
