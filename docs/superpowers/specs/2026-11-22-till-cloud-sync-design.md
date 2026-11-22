# Till ↔ back office sync — design

**Date:** 2026-11-22
**Status:** awaiting sign-off
**Phase:** 2e (new). Sits between Phase 2d (merged) and Phase 3 (the VSDC client).

## Intent

Make a sale rung up on a till appear in the back office within seconds, with
its lines, its tenders, its stock movements and its shift, so that inventory
and financial reporting are about what the shop actually did today rather than
what someone typed in later.

## Why now

Nothing connects the two. This is not "partly wired" — it is two missing
halves, and the till has been accumulating unsyncable sales since Phase 2d
merged.

**What exists on the till.** A well-built outbox. `outbound`
(`till/crates/till-store/migrations/0001_init.sql:112`) holds `kind`,
`sale_id`, `payload`, `attempts`, `last_error`, `created_at`, `sent_at`, with a
partial index (`:124`) so "next unsent row" stays O(1) however long sent
history grows. Sales and shift open/close are enqueued **in the same
transaction** as the write, so a queued row can never describe a sale that did
not happen. The payload is already cloud-shaped and carries full line detail.

**What does not exist.**

| Half | State |
|---|---|
| A sender on the till | No HTTP client anywhere in the Rust workspace — no `reqwest`, `hyper` or `ureq`. No module drains the queue. `till-store` exports no way to read unsent rows or mark one sent. |
| A receiver in the cloud | No till-facing route. `src/app/api/[slug]/` has products, stock, invoices, sales-orders and the rest; nothing a till can POST to. |
| A way for a till to authenticate | `getTenantContext()` (`src/lib/api.ts:91`) requires a NextAuth **browser session**. A headless Rust process has no cookies. There is no device credential concept. |

So the till writes to a queue nothing reads, aimed at a door that does not
exist.

## Goals

- A confirmed sale, its lines and its tenders reach the cloud and are visible
  in the back office within a few seconds on a healthy link.
- A sale produces the stock movements that make inventory correct.
- A shift open and close reach the cloud, attributed to the cashier who did it.
- Delivery survives a flat network, a killed process and a reboot, and never
  duplicates or loses a sale.
- The till keeps selling when the cloud is unreachable.

## Non-goals

- Cloud → till catalogue push. The till already has `sync_catalogue`; how it is
  triggered is a separate slice.
- Real-time *display* in the back office (websockets, live tiles). This slice
  lands the data; whether a page polls or subscribes is a UI decision.
- Refunds, voids and suspended sales. Not built on the till yet.
- The VSDC client. Phase 3. This slice carries `rcptNo` when the till has one
  and leaves it null when it does not.
- Multi-shop roll-up reporting.

## Anti-goals — what would make this a failure even if it shipped

- **A till that stops selling because the cloud is down.** The outbox exists
  precisely so that cannot happen. Any design where a sale waits on an HTTP
  round trip is wrong.
- **A duplicated sale.** Double-counted revenue is worse than late revenue: it
  is wrong in the books and in the tax position, and nobody notices for weeks.
- **A sale attributed to the wrong product.** Silently remapping history by a
  mutable key would corrupt inventory while looking healthy.

## Constraints carried

- Offline-first: C1's hard stop is about the **VSDC**, not the cloud. The cloud
  being unreachable must never stop a sale.
- Money is exact decimal strings end to end, never a float. `Money` and `Qty`
  expose no decimal accessor outside `till-core`.
- `sqlx` macros stay disabled; migrations are hand-authored and applied with
  `migrate deploy` only (R25 — no `migrate dev`, `diff` or `reset`).
- `Err` only for faults the cashier cannot act on; `Ok` with a `Notice::Problem`
  for those they can. Sync faults are neither — they are invisible to the
  cashier and belong in a status indicator, not a notice.
- No secrets in plaintext on disk.
- Africa/Kigali, RWF.

## Approaches

**A. The till pushes to an HTTPS ingest endpoint. ← chosen**
The till drains its outbox and POSTs batches to a new route. Authenticated as a
device, idempotent on a natural key.
*Trades:* needs a device credential concept that does not exist, and the cloud
must be reachable from the shop. Gives offline tolerance for free, since the
outbox already is the buffer.

**B. The back office polls each till.**
*Rejected.* Requires inbound connectivity to every till — NAT traversal, static
addresses or a tunnel per shop. A non-starter for shops on consumer
connections, and it inverts the trust boundary: the cloud would need credentials
for every till rather than each till holding its own.

**C. A message broker between them.**
*Rejected for v1.* Extra infrastructure, extra cost and a third thing that can
be down. The outbox already provides durability and ordering; a broker would
duplicate it. Worth revisiting only if fan-out to several consumers appears.

**D. The till writes to Postgres directly.**
*Rejected.* Database credentials on every till in every shop, no validation
layer, and nothing enforcing the org boundary. One compromised till would reach
every shop's data.

## The design

### Transport and cadence

The till gains a `sync` module with an HTTP client (`reqwest`, rustls, no
OpenSSL) running on the existing Tokio runtime, off the UI thread.

- Drain loop: take up to **50** unsent rows in `id` order, POST, mark sent.
- Triggered on **enqueue** and by a **heartbeat** every 15 seconds, so a sale
  normally leaves within a second and a missed trigger costs at most 15.
- Backoff on failure: 1s, 2s, 4s … capped at 60s, per-row `attempts` and
  `last_error` recorded (the columns already exist).
- **Target:** enqueue to visible in the back office under 2 seconds on a healthy
  link. A target, not a guarantee — the honest promise is *near-real-time while
  connected, eventually consistent when not*.

A row is marked sent **only** on a 2xx. A 4xx that is not retryable (a malformed
payload, an unknown lane) parks the row with `last_error` set and does not block
the queue behind it; anything else retries forever. Parked rows are what the
status indicator counts.

### Ordering — already correct, and worth not breaking

`Sale.shiftId` is a non-null FK to `Shift` (`prisma/schema.prisma:713`), so a
shift must exist in the cloud before any of its sales. The outbox gives this for
free: the shift's open row is enqueued before any sale of that shift, and the
drain is strictly FIFO by `id`. **The drain must stay strictly ordered** — no
parallel requests, no skipping a stuck row ahead of its dependents. A parked
`shift` row must block the sales of that shift specifically; the simplest
correct rule is to park the whole queue on a non-retryable `shift` failure and
surface it loudly.

### Idempotency — a natural key already exists

`Sale` carries `@@unique([laneId, invcNo])` (`:732`). The till's invoice
sequence is gapless per device, so `(lane, invcNo)` identifies a sale uniquely
and permanently. The ingest upserts on it. A retried POST after a timeout that
actually succeeded is therefore harmless — the second write finds the first.

`Shift` has no such natural key, but the till generates the shift `id` itself,
and the cloud's `Shift.id` is a plain `String` primary key. The till's id is
used verbatim, making shift upserts idempotent on the primary key. Sale `id` is
treated the same way.

The till's ids are `lower(hex(randomblob(16)))` — 32 hex characters, 128 bits of
randomness (`sale.rs:120`, `shift.rs:99`), not cuids. That is fine: the columns
are plain `String @id` with no format requirement, and Prisma's
`@default(cuid())` applies only when the client supplies no id. Collision risk
at 128 bits is not worth a sentence beyond this one.

This means **no idempotency-key table and no request ids** are needed.

### Authentication — a device credential

A new `LaneToken`: a random 32-byte secret, shown once at issue, stored as a
hash in the cloud and verified on every request.

- Issued from the back office, per Lane, by an ADMIN. Revocable. Rotatable.
- Sent as `Authorization: Bearer <token>`.
- The token identifies the Lane, which yields `orgId` and `locationId`. The
  till never sends an org slug and cannot address another org — the tenant is
  derived from the credential, not from the request.
- Stored on the till in the **Windows Credential Manager** (DPAPI), not in a
  file beside the database. This is a long-lived device credential, not a
  session token.

A new `getLaneContext()` sits beside `getTenantContext()` rather than inside it:
the till route must not be reachable by a browser session, and a browser route
must not be reachable by a lane token.

### What the ingest writes

One transaction per sale, so a half-written sale cannot be observed:

1. `Sale` upsert on `(laneId, invcNo)`.
2. `SaleLine` rows, upsert on `(saleId, itemSeq)` — the existing unique.
3. `Tender` rows, replaced wholesale for the sale.
4. `StockMovement` per line: `type: SALE` (the enum already has it,
   `:43`), `productId`, `qty`, `fromLocationId` = the Lane's `locationId`,
   `referenceId` = the sale id, so a movement is traceable to its sale and a
   re-POST can be made idempotent on `(referenceId, productId)`.
5. `Lane.lastInvcNo` advanced when the payload carries a higher one.

Financial reporting needs no new tables: it is derived from `Sale` and `Tender`
once they land.

### The productId gap — the one real data-model decision

The cloud's `SaleLine.productId` is a **required FK to `Product`**
(`:741`). The till's `sale_line` has **no product id**
(`0001_init.sql:87-102`) — only `item_cd` (the SKU), `item_cls_cd` and
`item_nm`, which are the RRA/VSDC fields.

`Product` does carry `@@unique([sku, orgId])`, so the cloud *could* resolve
`item_cd` → `productId` at ingest. **It should not.**

A sale is a historical fact. SKUs are editable in the back office and `Product`
has `deletedAt`, so a sale synced an hour later could resolve to a product whose
SKU has since been reassigned, or to a soft-deleted row — silently attributing
revenue and stock to the wrong item, with nothing to show anything went wrong.

**Decision (mine):** the till records the cloud `Product.id` on the line at
scan time and sends it. It already has it — `CachedProduct.id` *is* the cloud
id, held in the catalogue cache. This needs till-store migration `0003`: add
`product_id TEXT` to `sale_line`, populated from the cached product. Resolution
by SKU remains the fallback for rows written before the migration, which is
the only place the fragile path is acceptable, because those sales are already
in the past.

### Cashier identity — chosen: PIN against synced users

`Shift.openedById` is a non-null FK to `User` (`:699`). The till records
`opened_by` as free text (`"alice"`, `0002_shift.sql:12`) and has no notion of
cloud users.

Each cashier becomes a real `User`, so a shift and its cash variance are
attributable to a named person — which is the entire point of recording a
variance. This needs:

**Cloud schema:**
- `Role` gains `CASHIER` (currently ADMIN, MANAGER, ACCOUNTANT, VIEWER only).
- `User` gains `pinHash String?`. **Not** reusing `password`: a 4–6 digit PIN as
  a web login password would be a serious regression.
- `User.email` is currently required and unique per org. Cashiers may have no
  email; this needs a decision — see Open questions.

**Till:**
- A cashier list synced to the till: `id`, `name`, `pin_hash`, `active`.
- Shift open asks for a PIN, verified **locally** against the synced hash, so
  opening a shift works with the network down. This is required: a till that
  cannot open a shift offline cannot trade.
- `opened_by` carries the cloud `User.id`; the payload's `openedBy` becomes
  `openedById`.

**The security trade, stated plainly.** Syncing PIN hashes to the till means a
stolen till database allows offline brute force, and a 4-digit PIN has 10,000
candidates. Mitigations: **argon2id** with per-user salt and deliberately high
cost, a **6-digit** minimum, lockout after repeated failures, and PIN hashes
that are useless for web login. The alternative — verifying PINs online — breaks
offline shift open, which is not acceptable for a till. This is a real
trade-off, taken knowingly, and it is why the PIN must not be the password.

## Schema changes summary

**Cloud (`prisma/schema.prisma`), hand-authored migration:**
- `model LaneToken` — id, laneId, tokenHash, label, createdAt, lastUsedAt, revokedAt, orgId.
- `Role` += `CASHIER`.
- `User` += `pinHash String?`.
- `StockMovement` — a **partial** unique index on sale-sourced rows only:
  `CREATE UNIQUE INDEX ... ON "StockMovement" ("referenceId", "productId") WHERE "type" = 'SALE'`.
  It must be partial, and this is not a detail: `referenceId` is already used by
  GRN approval (`src/app/api/[slug]/grns/[id]/approve/route.ts:58`) and delivery-note
  approval (`.../delivery-notes/[id]/approve/route.ts:95`), and a GRN can
  legitimately produce two movements for the same product in **different
  batches**. A global unique on `(referenceId, productId)` would break goods
  receipt. Prisma cannot express a partial unique index, so this goes in the
  migration as raw SQL and `prisma/schema.prisma` carries a comment saying why.

**Till (`till-store`), migration `0003`:**
- `sale_line` += `product_id TEXT`.
- `cashier` table — id, name, pin_hash, active, synced_at.
- `shift.opened_by` becomes the cloud user id (data already free text; existing rows need a decision — see Open questions).

Both migrations are hand-authored and applied with `migrate deploy`.

## What is left to the builder

- Retry/backoff constants and whether backoff is per-row or per-queue.
- Batch framing: one request per row, or an array. Recommend an array with a
  per-row result so one bad row does not fail its batch.
- Where the sync status surfaces on the till screen (a chip, like the VSDC one)
  and what it says.
- Whether the heartbeat lives in the UI's runtime or its own thread.
- The cashier-sync trigger and how a deactivated cashier is revoked mid-shift.
- Payload versioning: a `v` field now costs nothing and buys a migration path.

## Open questions for sign-off

1. **Cashier emails.** `User.email` is required and `@@unique([email, orgId])`.
   Do cashiers get real emails, synthetic ones (`alice.cashier@<slug>`), or
   should `email` become nullable? Nullable touches NextAuth's credential login
   and is the riskiest; synthetic is ugly but contained. **I would use
   synthetic, generated at creation and never shown.**
2. **Existing shift rows.** The till's `opened_by` currently holds free text
   from testing. Backfill to a placeholder user, or drop pre-migration shifts as
   test data? **I would drop them** — they are demo data and nothing depends on
   them. This needs explicit authorisation under R112.
3. **Where is the back office hosted, and does the pilot shop have reliable
   internet?** Assumed cloud-hosted and reachable over the internet. If a shop
   can be offline for hours, the queue depth and the status indicator matter more
   than the latency target.
4. **Who issues lane tokens, and what happens on loss?** Assumed ADMIN issues
   and revokes from the back office; a lost token is revoked and reissued, and
   the till re-enrols by hand.

## Risks

| Risk | Consequence | Mitigation |
|---|---|---|
| A parked `shift` row blocks its sales | Sales invisible in the back office until someone looks | Park loudly, surface queue depth and oldest-unsent age on the till and in the back office |
| PIN hashes on a stolen till | Offline brute force of cashier PINs | argon2id, 6 digits, lockout, hashes useless for web login |
| SKU-resolved legacy lines | Wrong product on a historical sale | Carry `product_id` going forward; fallback only for pre-migration rows |
| Stock double-counted on re-POST | Inventory drifts silently | Uniqueness on the movement's sale reference, enforced in the database |
| Clock skew between till and cloud | Sales dated wrongly in reports | `soldAt` comes from the till's record, not the server's clock; reports use it as written |
