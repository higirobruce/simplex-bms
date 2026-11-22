-- Cloud sync groundwork: the cloud Product id on each sale line, a local cache
-- of the cloud's cashiers (so a PIN can be verified with the network down), and
-- removal of the unsent outbound queue rows that could never be delivered (see
-- section 1). No shift or sale is removed.
--
-- Conventions as in 0001_init.sql: ids and timestamps are TEXT, booleans are
-- INTEGER 0/1 with a CHECK. No money column is added here.

-- ---------------------------------------------------------------------------
-- 1. Clear the unsendable outbound queue entries. NO shift, sale, sale_line or
-- sale_tender row is deleted: those are fiscal records and stay.
--
-- shift.opened_by holds free text from testing ("alice", "cashier"). The
-- queued payloads for those shifts (and for the sales on them) carry
-- `openedBy` as that free text; the cloud refuses such a row with 422, the till
-- parks it, and the FIFO queue is then wedged forever behind it. Removing the
-- unsendable queue entries is what unblocks it. The orphaned local shifts are
-- harmless: nothing syncs them and nothing reads them.
--
-- Why scoping to "every shift that exists now" is complete, not a guess: when
-- this migration runs there is no `cashier` table and a till has never had any
-- way to obtain a cloud User.id, so EVERY shift existing at this point is
-- unmigratable by construction. Authorised by the project owner, 2026-11-22.
--
-- Scoped explicitly (not a bare DELETE FROM outbound) so the intent is legible
-- and it stays correct against a partly-migrated database. outbound is a leaf
-- table (nothing references it), so these deletes cannot violate a foreign key
-- whether or not enforcement is on (`PRAGMA foreign_keys` is a no-op inside the
-- transaction sqlx wraps this file in); the order is irrelevant to safety.
-- invc_seq is untouched and no sale is removed, so there is no numbering gap.
--
-- Only UNSENT rows (sent_at IS NULL) go. A row with sent_at set was accepted by
-- the cloud, so its openedBy was valid by definition; a delivered row cannot
-- wedge the queue, and deleting it would only destroy delivery history.
--
-- THIS IS NOT LOSSLESS. Deleting the unsent SALE queue rows means those sales
-- never reach the cloud, and nothing re-queues them. That is deliberate (owner
-- decision) and unavoidable: their shift's queue row is gone, so the cloud
-- would refuse a sale whose shift it has never seen, and that refusal would
-- wedge the FIFO queue. The sales themselves stay in the till's database.
-- ---------------------------------------------------------------------------
DELETE FROM outbound WHERE kind = 'shift' AND sent_at IS NULL;
DELETE FROM outbound
WHERE sent_at IS NULL
  AND sale_id IN (SELECT id FROM sale WHERE shift_id IN (SELECT id FROM shift));

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
