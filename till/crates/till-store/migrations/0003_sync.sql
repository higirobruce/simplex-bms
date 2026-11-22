-- Cloud sync groundwork: the cloud Product id on each sale line, a local cache
-- of the cloud's cashiers (so a PIN can be verified with the network down), and
-- removal of the demo shifts that predate any of this.
--
-- Conventions as in 0001_init.sql: ids and timestamps are TEXT, booleans are
-- INTEGER 0/1 with a CHECK. No money column is added here.

-- ---------------------------------------------------------------------------
-- 1. Remove the demo shifts, the sales on them, and those sales' children.
--
-- shift.opened_by holds free text from testing ("alice", "cashier"); from now
-- on it is the cloud User.id, so those rows could never be migrated, and
-- nothing has ever synced them. Authorised by the project owner, 2026-11-22.
--
-- ORDER MATTERS. sqlx runs this file in one transaction and `PRAGMA
-- foreign_keys` is a no-op inside a transaction, so FK enforcement may be on
-- or off and this file must be correct either way. Deleting strictly
-- child-first means no row is ever deleted while another row still references
-- it, so with keys ON nothing is refused and with keys OFF nothing is orphaned:
--   sale_line, sale_tender, outbound  ->  sale  ->  shift
-- All of it is one transaction, so a failure rolls the whole delete back.
-- invc_seq is deliberately untouched: the invoice counters must never go
-- backwards.
-- ---------------------------------------------------------------------------
DELETE FROM sale_line
WHERE sale_id IN (SELECT id FROM sale WHERE shift_id IN (SELECT id FROM shift));
DELETE FROM sale_tender
WHERE sale_id IN (SELECT id FROM sale WHERE shift_id IN (SELECT id FROM shift));
DELETE FROM outbound
WHERE sale_id IN (SELECT id FROM sale WHERE shift_id IN (SELECT id FROM shift));
DELETE FROM sale WHERE shift_id IN (SELECT id FROM shift);
DELETE FROM shift;

-- ---------------------------------------------------------------------------
-- 2. The cloud Product.id of a sale line. Nullable: lines written before this
-- migration have none and the cloud resolves those by SKU (item_cd). A plain
-- ADD COLUMN, no rebuild needed.
-- ---------------------------------------------------------------------------
ALTER TABLE sale_line ADD COLUMN product_id TEXT;

-- ---------------------------------------------------------------------------
-- 3. Cashiers cached from the cloud. `id` is the cloud User.id (what
-- shift.opened_by will hold); `pin_hash` is an argon2id PHC string.
-- ---------------------------------------------------------------------------
CREATE TABLE cashier (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    pin_hash   TEXT NOT NULL,
    active     INTEGER NOT NULL CHECK (active IN (0, 1)),
    synced_at  TEXT NOT NULL
);
