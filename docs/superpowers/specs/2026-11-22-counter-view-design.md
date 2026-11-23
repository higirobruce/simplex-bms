# The counter: till sales in the back office — design

**Date:** 2026-11-22
**Status:** awaiting sign-off
**Phase:** 2f (new). Follows Phase 2e, which lands the data this reads.

## Intent

A shop owner opens the back office in the morning and sees the counter: what
each shift took, whether the drawer balanced, and whether the figures can be
trusted yet.

## Why now

Phase 2e lands `Sale`, `SaleLine`, `Tender`, `Shift` and `StockMovement` from
the till into Postgres, verified end to end against a live server. **Nothing in
`src/` reads any of them.** The only code touching those tables is the two
write routes under `src/app/api/till/`. The data arrives correctly and no
person can see a single sale.

The stock effect *is* visible — `/[slug]/stock` reads `StockLevel`, which the
ingest decrements — but a sale, a shift, a tender and a cashier are all
invisible.

**And the existing revenue figure is wrong for a retail shop.** The dashboard
computes revenue from `Payment`, which exists only against an `Invoice`. A till
sale never creates one. Today's development database holds **2 `Payment` rows against 79
`Tender` rows** (counted directly against the running Postgres, not derivable
from the repo), so the headline revenue a shop owner sees is blind to
essentially every franc that crossed the counter. Building the counter view and
leaving that alone would put two disagreeing numbers in one app, with the wrong
one on the front page.

## Goals

- Yesterday's takings, per shift, split by RRA payment method.
- The cash variance per shift: what the drawer should hold against what was
  counted.
- An honest statement of whether a shift's data is complete.
- Open shifts visible as trading-now, distinct from closed ones.
- A sale list per shift, opening to the receipt as the customer got it.
- The dashboard's revenue figure stops ignoring the counter.

## Non-goals

- Editing anything. See the anti-goals.
- Refunds, voids, suspended sales — not built on the till.
- The RRA daily X and Z reports. Those are statutory documents with mandated
  content (CIS §18.1/§19.1) and are their own slice.
- Multi-shop roll-ups across orgs.
- Live push (websockets). Polling on an interval is enough and is what the rest
  of the app does.

## Anti-goals — what would make this a failure even if it worked

- **Showing a variance that cannot be stood behind.** A shift closed on the
  till whose sales have not all arrived would print a confident shortfall that
  is really a network delay. Someone would go and accuse an honest cashier.
  This is the single thing the design exists to prevent.
- **A headline total that hides a dead till.** A shop-wide figure with a green
  dot somewhere else is how a silent lane goes unnoticed for a day.
- **An editable till receipt.** A fiscalised sale is a record. A back office
  that lets someone quietly amend one is worse than a back office that shows
  nothing.

## Constraints carried

- Money is exact decimals end to end. Read `Decimal` columns; never round for
  display in a way that changes a reconciliation figure.
- RWF, Africa/Kigali. `Org.timezone` exists and defaults to `Africa/Kigali`.
- The till keeps selling when the cloud is down; a backlog is normal, not an
  error state.
- Tenders record what the customer **handed over**; `change = Σ(tenders) −
  totAmt`, paid in cash and capped by the cash taken (verified in
  `till-core/src/tender.rs:137-150`).

## Approaches

**A. A new API route plus a client page, following the existing pattern. ← chosen**
`/api/[slug]/counter` returns the shift roll-up; `/[slug]/counter` is a client
component using `@tanstack/react-query` and `apiCall`, exactly like
`/[slug]/stock` and `/[slug]/products`. Every page under `/[slug]/` already
works this way; only `layout.tsx` touches Prisma directly.
*Trades:* one more route. Gains consistency, a reusable endpoint, and
react-query's refetch, which matters here because a stale page showing
"balanced" for a shift that has since gone short is the failure this view
exists to prevent.

**B. A server component reading Prisma directly.**
*Rejected.* Simpler on paper, but it would be the only page in the app doing
it, and the refresh behaviour this view needs is awkward in a server component.
A second data-access pattern for one page is a poor trade.

**C. Fold it into the existing dashboard.**
*Rejected.* The dashboard is built around invoices and B2B revenue. The counter
is a different income stream with a different accounting unit (the shift). One
screen answering both questions answers neither well. The dashboard's revenue
figure still gets fixed — see below — but it does not become this page.

## What the human decided

1. **The shift is the unit.** Shops count the drawer and reconcile per shift,
   so a shift is what a person is accountable for and what the page is built
   around. Yesterday is two or three shifts, each with a name on it.
2. **The variance leads, the total sits underneath.** "Short 2,500" is what
   makes someone act; "took 356,500" is context. The shop-wide total goes below
   the shifts, not above them.
3. **Sync status qualifies the number rather than sitting beside it.** A shift
   whose data is incomplete says so where its variance would be.
4. **The till carries its own totals in the shift close payload**, so the cloud
   can tell complete from incomplete. See below.
5. **The dashboard revenue fix is in this slice**, not deferred.

## The design

### The shift roll-up

Per shift: lane, cashier name, open and close times, sale count, the tender
split by RRA payment code, change given, takings, and the variance.

**Takings** = Σ(`Sale.totAmt`) over the shift's sales.
**Expected drawer** = `openingFloat` + Σ(cash tenders) − Σ(change given).
**Change given**, per sale = Σ(that sale's tenders) − `totAmt`. It is cash-only
and capped by the cash taken, so it never produces a negative drawer.
**Variance** = `declaredCash` − expected drawer. Negative is short, positive is
over.

An **over** is shown as prominently as a short but in a calmer colour: it is a
counting error or an unrecorded sale rather than a suspicion, and treating the
two identically trains people to ignore both.

### Five states, which must not look alike

An earlier draft of this design named three, and got the dangerous one
backwards. **Delivery is ordered.** The till's outbox drains `order by o.id`
(`till-store/src/outbox.rs:62`), the close row is queued after that shift's
sale rows, and the till refuses to proceed with a sale still pending
(`LaneError::SalePending`, `lane.rs:981`). So the cloud receives every sale
*before* it receives the close, and a network delay delays the close too.

"Closed on the till but its sales are still arriving" is therefore largely
unreachable. The states that actually occur:

| State | Shows |
|---|---|
| **Open, reporting** | Takings so far, no variance. Normal; must not read as a problem. |
| **Open, stale** | The lane has not reported for longer than the threshold, or the shift opened more than a working day ago. **This is the dangerous one** — a shift closed on the till whose close has not arrived appears here, looking healthy. A shift that ended at 21:00 still showing "trading now" at 09:00 is the alarm. |
| **Closed, complete** | The variance, stated plainly. |
| **Closed, counts short** | The till reports more sales than the cloud holds — a sale was refused and parked. Louder than lateness, and it names the gap: "Lane 02 reports 121 sales, we hold 118". |
| **Closed, completeness unknown** | A shift closed before the till carried its totals. **No variance is shown**, only takings and a plain statement that this shift predates completeness tracking. Showing a caveated number would breach the anti-goal; showing nothing without saying why would look broken. |

A sixth condition — counts agree but totals disagree — is a data fault rather
than a delivery problem. It shows as its own state and must never resolve to
"complete".

### Staleness gives the "dead till" anti-goal its mechanism

The anti-goal "a headline total that hides a dead till" had no mechanism in the
earlier draft. The staleness rule is it: a lane whose shift is open and whose
last contact is older than the threshold is flagged on the shift itself. Last
contact comes from `LaneToken.lastUsedAt` (max over the lane's non-revoked
tokens). Note what that actually means — it is bumped by any authenticated lane
request and throttled to 60 s, so it says **reachable**, not **sent a sale**.
The copy must not overclaim it.

The shop-wide total states what it excludes whenever any shift is pending,
unknown, mismatched or stale.

### Completeness — the part that needs a till change

The back office cannot currently tell these apart. The close payload carries
only `id`, `dvcSrlNo`, `openedById`, `openedAt`, `openingFloat`, `closedAt`,
`declaredCash`. The cloud sees what arrived and cannot know three more sales
sit in the till's outbox.

**The till already computes what is needed.** `ShiftTotals`
(`till-store/src/shift.rs:40`) carries `sale_count`, `gross`, `tax` and the
per-code tender split for its own close report.

**Change:** the shift close payload gains `saleCount` and `gross`. The cloud
compares them against what it holds and reports the shift complete only when
both match. If the counts agree and the totals do not, that is a louder problem
than lateness and must say so rather than resolving to "complete".

Shifts closed before this change have neither field. They are reported as
**"completeness unknown"** — not complete, not pending — because claiming
either would be a guess.

### The dashboard revenue fix

`revenueMTD` currently sums `Payment.amount`. It becomes **two lines, not one
merged figure**: invoice payments and counter takings are genuinely different
money with different timing, and adding them hides which one moved. Counter
takings come from Σ(`Sale.totAmt`) over the period, not from tenders, because
tenders include change handed back.

### Days are bounded in the org's timezone

`Org.timezone` is stored and **nothing currently honours it** — the dashboard
bounds its month with `new Date(now.getFullYear(), now.getMonth(), 1)`, which
is the *server's* zone. On a UTC host, a Kigali sale at 01:30 Saturday is 23:30
Friday UTC and lands in the wrong day. For a view whose whole purpose is "what
happened yesterday", that is a correctness bug at exactly the boundary a late
shift sits on. The counter bounds its days in `Org.timezone`. Fixing the
dashboard's existing month boundary is in scope only where it touches the
revenue figure being changed — which in practice means `revenueMTD`'s month
start **and** the six-month trend, since they share the same boundary.
`expensesMTD` reads the same `startOfMonth` variable, so that variable must be
split rather than repointed, or expenses silently move to a different zone from
revenue.

### Refresh

react-query with a modest interval (30–60 s) and refetch on window focus. No
websockets. A backlog is normal, so the UI never treats "waiting" as an error.

### The sale detail

**A reconstructed view, labelled as a back-office record and not the fiscal
copy.** The cloud holds no `rcptSign`, SDC id, internal data or QR payload —
those live only on the till's printed receipt — so a true fiscal copy is
impossible without extending the payload, and that is better justified once
Phase 3's real VSDC client exists. Today those fields come from a simulator.

**A known limitation, recorded rather than discovered:** lines store
`productId`, so a sale shows the product's *current* name. Renaming a product
rewrites the name on every historical sale. The till already sends `itemNm` in
every line and the ingest discards it, so one nullable column on `SaleLine`
closes this whenever it is wanted. Decided against for now.

Nothing in the view is editable. A fiscalised sale is a record.

## Decisions made by the builder, for the record

These are mine, not the human's. Overrule any of them freely.

- **Day membership:** a shift belongs to the day it *opened*, in `Org.timezone`.
  A shift still open from a previous day also appears under today as stale.
- **Method split:** net cash (cash tendered minus change) plus each non-cash
  code as tendered, so the split sums to takings. Gross cash appears only in
  the expected-drawer breakdown. Without this the split exceeds takings by the
  change given and nothing adds up on screen.
- **Credit sales:** RRA code 02 (`CREDIT`) is money owed, not received. It gets
  its own line rather than being hidden inside the total.
- **New cloud fields:** `Shift.saleCount Int?` and `Shift.gross Decimal(12,2)?`,
  hand-authored migration, accepted only alongside `closedAt`, and written on a
  replayed close as well as a first one. Both null means unknown.
- **Cashier name:** `Shift.openedById` has no Prisma relation to `User`, so the
  roll-up resolves names in a second query. Deactivated cashiers still resolve —
  the row stays and only `pinHash` is cleared.
- **Completeness rule:** complete only when `saleCount` *and* `gross` both
  match. More sales held than reported is the mismatch state, not complete.
- **Voided sales:** excluded from counts and sums. The till never queues them,
  so this is defensive only.
- **API shape:** decimals returned as strings, arithmetic done server-side in
  `Decimal`. Avoids the `Number(...)` pattern in `stats/route.ts`.
- **Refresh:** `refetchInterval` 45 s plus `refetchOnWindowFocus`, overriding
  the global `staleTime`.
- **Permission:** an existing read permission (ADMIN and manager). Cashiers
  cannot log into the web at all.
- **Sidebar:** a "Counter" entry beside Sales; rename the existing "Sales" to
  "Sales orders" if that is cheap, since the two are different things.

## What is left to the builder

- The exact SQL shape of the roll-up: one grouped query per shift versus a
  join, and whether tender splits come back in the same round trip.
- Pagination once a shop has months of shifts; yesterday and today need none.
- Whether the sale list inside a shift paginates or lazy-loads.
- The date-picker affordance for looking at a day other than yesterday.
- Where the counter sits in the sidebar, and whether the B2B `sales` entry is
  renamed to avoid confusion with it.
- Timezone library choice (`Intl` with a time zone is likely enough; no new
  dependency unless it earns itself).

## Risks

| Risk | Consequence | Mitigation |
|---|---|---|
| A pre-change shift shows "unknown" forever | Owners learn to ignore the state | Backfill is impossible; the state is temporary by nature and disappears as shifts close after the change |
| Counts match but totals disagree | A silent data fault reads as healthy | Surface it louder than lateness, as its own state |
| Timezone handled per query | Day boundaries drift between views | One helper, used by every query in the slice |
| Variance maths misread | An honest cashier is accused | Expected drawer, counted cash and the difference are all shown, never just the difference |
