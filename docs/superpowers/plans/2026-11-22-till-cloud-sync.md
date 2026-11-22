# Till ↔ Back Office Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sale rung up on a till appears in the back office within seconds, with its lines, tenders, stock movements and shift, attributed to the cashier who rang it.

**Architecture:** The till drains its existing `outbound` outbox over HTTPS to a new ingest route, authenticated per Lane by a bearer token rather than a browser session. Idempotency rides on constraints that already exist (`Sale @@unique([laneId, invcNo])`, `SaleLine @@unique([saleId, itemSeq])`) plus one new partial index on sale-sourced stock movements, so a retried POST is harmless and no idempotency-key table is needed. Ordering is free: the shift's row is enqueued before its sales and the drain is strictly FIFO.

**Tech Stack:** Next.js 14.2 App Router, Prisma, Postgres; Rust 2024, `sqlx` 0.8 (runtime API only), `reqwest` (rustls), `argon2`, `keyring`.

**Spec:** `docs/superpowers/specs/2026-11-22-till-cloud-sync-design.md` (signed off 2026-11-22)

## Global Constraints

- The cloud being unreachable must never stop a sale. C1's hard stop is about the **VSDC**, not the cloud.
- Money is exact decimal strings end to end, never a float. `Money` and `Qty` expose no decimal accessor outside `till-core`.
- `sqlx` macros stay disabled. Till migrations are hand-authored; applied with `migrate deploy` only. **Never** `prisma migrate dev`, `migrate diff` or `migrate reset` (R25) — all need a shadow database.
- Prisma migrations are hand-authored SQL under `prisma/migrations/<timestamp>_<name>/migration.sql`, matching the existing layout.
- `Err` only for faults the cashier cannot act on; `Ok` with a `Notice::Problem` for those they can. **Sync faults are neither** — they are invisible to the cashier and belong in a status indicator, never a notice.
- A queue row is marked sent **only** on a 2xx.
- The drain is **strictly ordered**: no parallel requests, no skipping a row ahead of its dependents.
- Batch size **50**; heartbeat **15 s**; backoff 1, 2, 4 … capped at **60 s**.
- Target: enqueue to visible under **2 s** on a healthy link. A target, not a guarantee.
- The till's base URL is **configuration**, changeable without a rebuild.
- No secrets in plaintext on disk. The lane token lives in the OS credential store.
- **Token hashing is SHA-256; PIN hashing is argon2id.** This is deliberate: a lane token is 256 bits of randomness and needs no slow KDF, while a 6-digit PIN is brute-forceable and needs a memory-hard one. Do not unify them.
- PINs are **6 digits minimum**. A cashier's `pinHash` must never be usable as a web login credential.
- RWF, Africa/Kigali. `soldAt` comes from the till's record, never the server's clock.
- Build the web app where `NODE_ENV` is unset (`env -u NODE_ENV npx next build`); a stale `NODE_ENV=development` makes Next load both app-page runtimes and fail every prerender.

## Review Focus

Five failure modes the spec implies that no task's happy path exercises. Each has a test pinned to the task that owns the code.

1. **A lane token revoked or rotated while rows are queued** — the till must park, never lose a row and never mark one sent. (Task 11)
2. **A parked shift row blocking its own sales** — those sales must stay unsent and in order, not be skipped. (Task 11)
3. **A re-POST of an already-accepted sale** — no duplicate `Tender` rows and no duplicate `StockMovement`. (Task 6)
4. **A cashier deactivated in the cloud while the till holds a stale list** — opening a shift offline must still work, and the cloud must reject the shift rather than silently accept an inactive cashier. (Task 5)
5. **A product hard-deleted in the cloud after a sale was rung** — ingest must park that row with a clear error, not 500 forever or drop the sale. (Task 6)

---

## File Structure

**Cloud**
- `prisma/schema.prisma` — `LaneToken`, `Role.CASHIER`, `User.pinHash`.
- `prisma/migrations/<ts>_till_sync/migration.sql` — the above plus the partial index.
- `src/lib/lane-auth.ts` — `getLaneContext`, token minting and hashing. Device auth only; never touches sessions.
- `src/lib/cashier.ts` — synthetic email generation, PIN hashing and policy.
- `src/app/api/till/shift/route.ts` — shift ingest.
- `src/app/api/till/sale/route.ts` — sale ingest.
- `src/app/api/till/cashiers/route.ts` — the list a till pulls.
- `src/app/api/[slug]/lanes/[id]/tokens/route.ts` — issue and revoke, ADMIN only.
- `src/app/api/[slug]/cashiers/route.ts` — create, list, deactivate, ADMIN only.

**Till**
- `till/crates/till-store/migrations/0003_sync.sql` — `product_id`, `cashier`, drop demo shifts.
- `till/crates/till-store/src/outbox.rs` — the drain API. New module; `sale.rs` and `shift.rs` keep writing the queue.
- `till/crates/till-store/src/cashier.rs` — cashier cache and local PIN verification.
- `till/crates/till-lane/src/sync.rs` — the HTTP sender, batching and backoff.
- `till/crates/till-lane/src/config.rs` — base URL and token lookup.

---

### Task 1: Cloud schema and migration

**Files:**
- Modify: `prisma/schema.prisma`
- Create: `prisma/migrations/<timestamp>_till_sync/migration.sql`

**Interfaces:**
- Produces: `LaneToken { id, laneId, tokenHash, label, createdAt, lastUsedAt, revokedAt, orgId }`; `Role.CASHIER`; `User.pinHash String?`.

- [ ] **Step 1: Add the models and fields to `prisma/schema.prisma`**

`LaneToken` with `@@index([laneId])` and `@@unique([tokenHash])`. `Role` gains `CASHIER`. `User` gains `pinHash String?`. Add a comment above `StockMovement` recording that its sale-uniqueness is a partial index in SQL because Prisma cannot express one.

- [ ] **Step 2: Hand-author `migration.sql`**

Follow the existing `prisma/migrations/` layout. Include, as the last statement:

```sql
CREATE UNIQUE INDEX "StockMovement_sale_ref_product_key"
  ON "StockMovement" ("referenceId", "productId")
  WHERE "type" = 'SALE';
```

It **must** be partial. `referenceId` is already used by GRN approval (`src/app/api/[slug]/grns/[id]/approve/route.ts:58`) and delivery-note approval, and a GRN can hold two movements for one product in different batches — a global unique would break goods receipt.

- [ ] **Step 3: Apply with `npx prisma migrate deploy` and regenerate the client**

Expected: applied, no drift. Never `migrate dev`.

- [ ] **Step 4: Write the constraint tests**

```ts
test("two SALE movements for one product and one sale are refused", …)
test("two GRN movements for one product in different batches are still allowed", …)
test("a lane token hash is unique", …)
```

- [ ] **Step 5: Run them, then commit**

---

### Task 2: Device authentication

**Files:**
- Create: `src/lib/lane-auth.ts`, `tests/lane-auth.test.ts`

**Interfaces:**
- Consumes: `LaneToken` (Task 1).
- Produces:
  - `mintLaneToken(): { token: string; tokenHash: string }` — 32 random bytes, base64url; hash is SHA-256 hex.
  - `getLaneContext(req: Request): Promise<{ laneId: string; orgId: string; locationId: string }>` — throws `AuthError` on a missing, unknown or revoked token.

- [ ] **Step 1: Write the failing tests**

```ts
test("a valid token yields its lane, org and location", …)
test("a revoked token is refused", …)
test("an unknown token is refused", …)
test("a missing Authorization header is refused", …)
test("a browser session is NOT accepted by getLaneContext", …)
test("the token is never stored in plaintext", …)   // only the hash reaches the DB
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement `src/lib/lane-auth.ts`**

Read `Authorization: Bearer <token>`, hash with SHA-256, look up by `tokenHash`, reject when `revokedAt` is set, and derive `orgId` and `locationId` from the Lane. Update `lastUsedAt`. `getLaneContext` must not call `getServerSession` — a till route must be unreachable by a browser session, and a browser route unreachable by a lane token.

- [ ] **Step 4: Run tests, then commit**

---

### Task 3: Issue and revoke lane tokens

**Files:**
- Create: `src/app/api/[slug]/lanes/[id]/tokens/route.ts`

**Interfaces:**
- Consumes: `mintLaneToken` (Task 2), `requirePermission` / `getTenantContext` (`src/lib/api.ts`).
- Produces: `POST` returns `{ id, token }` — the plaintext token **once**. `GET` lists tokens without their values. `DELETE ?id=` sets `revokedAt`.

- [ ] **Step 1: Write the failing tests**

```ts
test("an ADMIN can issue a token and sees its value exactly once", …)
test("listing tokens never returns a token value", …)
test("a non-ADMIN cannot issue a token", …)
test("a revoked token stops authenticating", …)   // via getLaneContext
test("a lane in another org cannot be issued a token", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement the route**

Use `handleRouteError` and the existing route conventions. The plaintext token appears in the POST response body and nowhere else — not in the audit log, not in `GET`.

- [ ] **Step 4: Run tests, then commit**

---

### Task 4: Cashier accounts

**Files:**
- Create: `src/lib/cashier.ts`, `src/app/api/[slug]/cashiers/route.ts`

**Interfaces:**
- Produces:
  - `syntheticEmail(name: string, slug: string): string` — stable, unique per org, never shown to anyone.
  - `hashPin(pin: string): Promise<string>` — argon2id PHC string.
  - `POST /api/[slug]/cashiers` — creates a `User` with `role: CASHIER`, a synthetic email and a `pinHash`. `GET` lists them. `PATCH` deactivates and sets a new PIN.

- [ ] **Step 1: Write the failing tests**

```ts
test("a PIN shorter than 6 digits is refused", …)
test("a non-numeric PIN is refused", …)
test("the stored pinHash is argon2id and is not the password column", …)
test("a cashier cannot log in to the web app with their PIN", …)
test("two cashiers with the same name in one org get different emails", …)
test("the same name in two orgs is allowed", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement `src/lib/cashier.ts` and the route**

The cashier's `password` column must be set to a value that cannot authenticate — not a hash of the PIN. **Do not reuse the signup path** (`src/app/api/auth/signup`), which creates a web-login account.

- [ ] **Step 4: Run tests, then commit**

---

### Task 5: Shift ingest and the cashier list

**Files:**
- Create: `src/app/api/till/shift/route.ts`, `src/app/api/till/cashiers/route.ts`

**Interfaces:**
- Consumes: `getLaneContext` (Task 2).
- Produces:
  - `POST /api/till/shift` — body `{ id, openedById, openedAt, openingFloat, closedAt?, declaredCash? }`. Upserts `Shift` on its primary key, with `laneId` and `orgId` from the lane context. 2xx on success.
  - `GET /api/till/cashiers` — `[{ id, name, pinHash, active }]` for the lane's org.

- [ ] **Step 1: Write the failing tests**

```ts
test("a shift is created with the lane and org from the token, not the body", …)
test("re-posting the same shift id is idempotent", …)
test("a close updates closedAt and declaredCash on the existing row", …)
test("openedById must be a CASHIER in this org", …)
test("an inactive cashier's shift is refused with a non-retryable status", …)  // Review Focus 4
test("a shift naming a cashier from another org is refused", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement both routes**

`laneId`, `orgId` and `locationId` come **only** from the token. A body field naming another org must not be honoured. Refusals that the till can never fix (an unknown or inactive cashier) return **422**, which the till parks; transient faults return 5xx, which the till retries.

- [ ] **Step 4: Run tests, then commit**

---

### Task 6: Sale ingest

**Files:**
- Create: `src/app/api/till/sale/route.ts`

**Interfaces:**
- Consumes: `getLaneContext` (Task 2); the shift must already exist (Task 5).
- Produces: `POST /api/till/sale` — body is the till's `cloud_payload` plus `productId` per line. Writes `Sale`, `SaleLine`, `Tender`, `StockMovement` and advances `Lane.lastInvcNo`, **all in one transaction**.

- [ ] **Step 1: Write the failing tests**

```ts
test("a sale lands with its lines, tenders and one SALE movement per line", …)
test("the movement's fromLocationId is the lane's location", …)
test("re-posting the same sale creates no duplicate tender or movement", …)   // Review Focus 3
test("a sale for an unknown shift is refused and nothing is written", …)
test("lastInvcNo advances, and never goes backwards on an older sale", …)
test("a line naming a hard-deleted product is refused with 422", …)           // Review Focus 5
test("a line naming a soft-deleted product is accepted", …)
test("money arrives as exact decimals: 1200.00 stays 1200.00", …)
test("soldAt comes from the payload, not the server clock", …)
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement the route**

Upsert `Sale` on `(laneId, invcNo)`, `SaleLine` on `(saleId, itemSeq)`, replace `Tender` rows for the sale, and create one `StockMovement` per line with `type: SALE`, `referenceId` = the sale id, `fromLocationId` = the lane's location — the partial index from Task 1 makes a re-POST safe. A missing `Product` row is a 422, not a 500: the till can never fix it by retrying.

- [ ] **Step 4: Run tests, then commit**

---

### Task 7: Till migration 0003

**Files:**
- Create: `till/crates/till-store/migrations/0003_sync.sql`

**Interfaces:**
- Produces: `sale_line.product_id TEXT`; table `cashier (id, name, pin_hash, active, synced_at)`; demo shifts removed.

- [ ] **Step 1: Copy the database file before writing the migration**

R112: take a copy of any `--db` file in use before a destructive migration is authored or run. Record where it went.

- [ ] **Step 2: Write `0003_sync.sql`**

Adding a nullable column is a plain `ALTER TABLE`. Deleting the demo shifts and the sales referencing them happens in the **same** migration (sqlx wraps each migration in a transaction and ignores `no_tx`, so `PRAGMA foreign_keys` is a no-op inside one — rely on statement order, not on FK enforcement). Dropping the demo rows was authorised explicitly on 2026-11-22.

- [ ] **Step 3: Write the migration tests**

```rust
fn the_cashier_table_exists_and_is_empty_on_a_fresh_database()
fn product_id_is_nullable_so_existing_lines_survive()
fn demo_shifts_and_their_sales_are_gone_but_other_rows_remain()
fn the_migration_is_idempotent_under_migrate_deploy()
```

- [ ] **Step 4: Run `cargo test -p till-store`, then commit**

---

### Task 8: Carry the cloud product id onto sale lines

**Files:**
- Modify: `till/crates/till-core/src/line.rs` (or wherever `BasketLine` lives), `till/crates/till-lane/src/lane.rs:line_for`, `till/crates/till-store/src/sale.rs:cloud_payload`

**Interfaces:**
- Consumes: `CachedProduct.id` — already the cloud `Product.id`.
- Produces: `BasketLine.product_id: Option<String>`; `cloud_payload` emits `productId` per line.

- [ ] **Step 1: Write the failing tests**

```rust
fn a_scanned_line_carries_the_cloud_product_id()
fn a_line_picked_by_name_carries_it_too()
fn the_cloud_payload_emits_product_id_per_line()
fn a_line_written_before_0003_still_serialises_with_a_null_product_id()
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Thread `product_id` from `CachedProduct` through `line_for` to the stored line and the payload**

`Option<String>`, because rows written before Task 7 have none. The ingest resolves those by SKU — the only place that fragile path is acceptable, because those sales are already in the past.

- [ ] **Step 4: Run tests, then commit**

---

### Task 9: Cashier cache and local PIN verification

**Files:**
- Create: `till/crates/till-store/src/cashier.rs`
- Modify: `till/crates/till-store/src/lib.rs`

**Interfaces:**
- Consumes: the `cashier` table (Task 7).
- Produces:
  - `pub struct Cashier { pub id: String, pub name: String, pub pin_hash: String, pub active: bool }` — `id` is the cloud `User.id`.
  - `Store::sync_cashiers(&[Cashier]) -> Result<(), StoreError>`
  - `Store::verify_pin(name_or_id: &str, pin: &str) -> Result<Option<String>, StoreError>` — the cashier's cloud id on success.
  - `Store::active_cashiers() -> Result<Vec<Cashier>, StoreError>`

- [ ] **Step 1: Write the failing tests**

```rust
fn a_correct_pin_returns_the_cashier_id()
fn a_wrong_pin_returns_none()
fn an_inactive_cashier_never_verifies()
fn verification_works_with_no_network()      // the whole point
fn a_resync_that_deactivates_a_cashier_takes_effect()
fn a_pin_hash_is_never_logged_or_returned_by_active_cashiers()
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Implement with the `argon2` crate, parsing the PHC string the cloud produced**

Verification is local and offline by design: a till that cannot open a shift offline cannot trade. `active_cashiers` must not expose `pin_hash`.

- [ ] **Step 4: Run `cargo test -p till-store`, then commit**

---

### Task 10: A PIN on shift open

**Files:**
- Modify: `till/crates/till-lane/src/lane.rs`, `till/crates/till-lane/src/ui.rs`

**Interfaces:**
- Consumes: `Store::verify_pin`, `Store::active_cashiers` (Task 9).
- Produces: `shift.opened_by` holds the cloud `User.id`; the shift payload's key becomes `openedById`.

- [ ] **Step 1: Write the failing tests**

```rust
fn opening_a_shift_requires_a_cashier_and_a_pin()
fn a_wrong_pin_is_a_problem_the_cashier_can_act_on_not_an_err()
fn the_opened_shift_records_the_cloud_user_id()
fn the_shift_payload_sends_openedById()
fn a_pin_is_never_drawn_on_screen_or_left_in_the_buffer()
fn lockout_after_repeated_wrong_pins()
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Extend the no-shift screen to pick a cashier and key a PIN**

The PIN uses the existing wedge buffer but must be masked and cleared on success, failure and screen change. A wrong PIN is `Ok` with a `Notice::Problem`.

- [ ] **Step 4: Run `cargo test -p till-lane`, then commit**

---

### Task 11: The outbox drain

**Files:**
- Create: `till/crates/till-store/src/outbox.rs`, `till/crates/till-lane/src/sync.rs`, `till/crates/till-lane/src/config.rs`
- Modify: `till/crates/till-lane/Cargo.toml` (`reqwest` with `default-features = false, features = ["json", "rustls-tls"]`, `keyring`)

**Interfaces:**
- Produces:
  - `pub struct OutboxRow { pub id: i64, pub kind: String, pub payload: String, pub attempts: i64 }` — `kind` is `"sale"` or `"shift"`.
  - `pub enum DrainOutcome { Idle, Sent(u32), Parked { id: i64, reason: String }, Offline }`
  - `Store::unsent(limit: u32) -> Result<Vec<OutboxRow>, StoreError>` — `id` order, oldest first.
  - `Store::mark_sent(id: i64) -> Result<(), StoreError>`
  - `Store::record_failure(id: i64, error: &str) -> Result<(), StoreError>` — increments `attempts`.
  - `Store::queue_depth() -> Result<(u32, Option<String>), StoreError>` — unsent count and oldest `created_at`.
  - `SyncConfig { base_url: String, token: String }` — base URL from config, token from the OS credential store via `keyring`.
  - `Sync::drain_once() -> DrainOutcome`

- [ ] **Step 1: Write the failing store tests**

```rust
fn unsent_returns_rows_in_id_order_and_excludes_sent()
fn mark_sent_is_idempotent()
fn record_failure_increments_attempts_and_keeps_the_row_unsent()
fn queue_depth_reports_the_count_and_the_oldest_unsent()
```

- [ ] **Step 2: Write the failing sync tests, against a stub HTTP server**

```rust
fn a_2xx_marks_the_row_sent()
fn a_non_2xx_leaves_the_row_unsent_and_records_the_error()
fn a_422_parks_the_row_without_blocking_unrelated_rows()
fn a_401_parks_the_whole_queue_and_loses_nothing()        // Review Focus 1
fn a_parked_shift_keeps_its_sales_unsent_and_in_order()   // Review Focus 2
fn rows_are_sent_in_id_order_and_never_in_parallel()
fn backoff_grows_1_2_4_and_caps_at_60s()
fn a_batch_never_exceeds_50_rows()
fn the_token_is_never_written_to_a_log_or_to_disk()
```

- [ ] **Step 3: Run them to verify they fail**

- [ ] **Step 4: Implement the store API, then `config.rs`, then `sync.rs`**

A row is marked sent **only** on a 2xx. The drain is strictly sequential. A 401/403 parks the whole queue — a revoked token is not a per-row problem. The base URL is read from configuration so it can be re-pointed without a rebuild.

- [ ] **Step 5: Pull the cashier list on the same client**

`GET /api/till/cashiers` into `Store::sync_cashiers` (Task 9). Without this
nothing ever populates the cashier table and Task 10 has no one to verify a PIN
against. A failed pull is never fatal: the till keeps the list it has, because a
stale list still opens a shift and an empty one cannot.

```rust
fn a_cashier_pull_replaces_the_local_list()
fn a_failed_pull_keeps_the_previous_list_rather_than_emptying_it()
```

- [ ] **Step 6: Wire the drain to fire on enqueue and on a 15 s heartbeat, off the UI thread**

- [ ] **Step 7: Run `cargo test --workspace`, then commit**

---

### Task 12: Sync status on screen

**Files:**
- Modify: `till/crates/till-lane/src/ui.rs`

**Interfaces:**
- Consumes: `Store::queue_depth` (Task 11).

- [ ] **Step 1: Write the failing tests**

```rust
fn a_drained_queue_shows_nothing_alarming()
fn an_unsent_backlog_shows_its_depth()
fn a_parked_queue_says_so_and_names_no_cashier_action()
fn the_oldest_unsent_age_is_shown_once_it_passes_a_threshold()
```

- [ ] **Step 2: Run them to verify they fail**

- [ ] **Step 3: Add a status chip beside the VSDC chip**

A sync fault is **not** a cashier problem: it never becomes a `Notice`. It reports depth and oldest-unsent age, because with the Mac-and-tunnel topology those are the only way anyone knows whether the shop's data is current. It must not claim the cloud is reachable "now" — like the VSDC chip, it reports the last outcome.

- [ ] **Step 4: Run `cargo test -p till-lane`, run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets`, then commit**

---

## Deployment (not a code task)

Settle before the hostname goes live; it blocks nothing in the plan, because the base URL is configuration.

- Postgres on **Neon free** (commercial use permitted). Prisma needs **two** connection strings: a direct one for `migrate deploy`, a pooled one for the app — migrations do not work correctly through transaction pooling.
- The app on **Vercel Pro** or **this Mac behind a named Cloudflare tunnel with a purchased domain**. Vercel Hobby is non-commercial-only and does not cover this project.
- Point **both** `NEXTAUTH_URL` and the till's base URL at the stable hostname. A rotating hostname breaks the operator's own login, not just sync.
- Put Cloudflare Access in front of the browser routes, or confirm no seeded credentials remain.
