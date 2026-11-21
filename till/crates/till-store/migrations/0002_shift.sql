-- A cashier's shift. The cloud's Sale.shiftId is non-null, so a sale with no
-- shift cannot be stored in the back office; until now the till had no shift
-- at all, which made every sale it recorded unsyncable. Mirrors the cloud's
-- `Shift` model (prisma/schema.prisma): the till's lane identity is its
-- `dvc_srl_no`, the cloud's `laneId` is resolved from it at sync time.
--
-- Money is TEXT holding an exact decimal string, as in 0001_init.sql (see the
-- comment at the top of that file for why).
CREATE TABLE shift (
    id             TEXT PRIMARY KEY,
    dvc_srl_no     TEXT NOT NULL,
    opened_by      TEXT NOT NULL,
    opened_at      TEXT NOT NULL,
    closed_at      TEXT,
    opening_float  TEXT NOT NULL,
    declared_cash  TEXT
);
-- "Is there an open shift on this device?" must not scan shift history.
CREATE INDEX shift_open ON shift (dvc_srl_no) WHERE closed_at IS NULL;

-- Rows that predate shifts need a shift to belong to. Created ONLY when sales
-- exist: an empty database must not gain a phantom closed shift.
INSERT INTO shift (id, dvc_srl_no, opened_by, opened_at, closed_at, opening_float, declared_cash)
SELECT 'migration-shift', MIN(dvc_srl_no), 'migration', MIN(created_at),
       strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), '0.00', NULL
FROM sale
HAVING COUNT(*) > 0;

-- sale.shift_id must be NOT NULL (a nullable one would permit exactly the
-- unsyncable sale this prevents), and SQLite cannot ADD COLUMN NOT NULL
-- without a default, so rebuild: create, copy, drop, rename.
--
-- The rebuild must also take sale's three children (sale_line, sale_tender,
-- outbound) down and bring them back. The textbook recipe turns foreign keys
-- OFF around the rebuild, but `PRAGMA foreign_keys` is a no-op inside a
-- transaction and sqlx 0.8's SQLite migrator always wraps a migration in one.
-- With keys ON, DROP TABLE sale runs an implicit DELETE that fails on child
-- rows, and `defer_foreign_keys` does not rescue it (SQLite counts the
-- violations at the DROP and never re-resolves them when the rebuilt table is
-- renamed into place). Dropping a CHILD is always allowed, so: stash the
-- children, drop them, rebuild the parent, recreate the children with their
-- 0001 definitions verbatim and refill them. Nothing is lost: all of it is
-- inside the migration's one transaction.
CREATE TABLE sale_new (
    id             TEXT PRIMARY KEY,
    shift_id       TEXT NOT NULL REFERENCES shift (id),
    dvc_srl_no     TEXT NOT NULL,
    counter        TEXT NOT NULL,
    bhf_id         TEXT NOT NULL,
    invc_no        INTEGER NOT NULL CHECK (invc_no >= 1),
    state          TEXT NOT NULL
                   CHECK (state IN ('AwaitingVsdc', 'Confirmed', 'Failed', 'Voided')),
    tot_amt        TEXT NOT NULL,
    tot_taxbl_amt  TEXT NOT NULL,
    tot_tax_amt    TEXT NOT NULL,
    pmt_ty_cd      TEXT NOT NULL,
    buyer_tin      TEXT,
    rcpt_no        INTEGER,
    intrl_data     TEXT,
    rcpt_sign      TEXT,
    sdc_id         TEXT,
    mrc_no         TEXT,
    vsdc_rcpt_pbct_date TEXT,
    tot_rcpt_no    INTEGER,
    created_at     TEXT NOT NULL,
    confirmed_at   TEXT,
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    FOREIGN KEY (dvc_srl_no, counter) REFERENCES invc_seq (dvc_srl_no, counter),
    UNIQUE (dvc_srl_no, counter, invc_no)
);

INSERT INTO sale_new (
    id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt,
    tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, buyer_tin, rcpt_no, intrl_data,
    rcpt_sign, sdc_id, mrc_no, vsdc_rcpt_pbct_date, tot_rcpt_no, created_at,
    confirmed_at, attempts, last_error
)
SELECT
    id, 'migration-shift', dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt,
    tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, buyer_tin, rcpt_no, intrl_data,
    rcpt_sign, sdc_id, mrc_no, vsdc_rcpt_pbct_date, tot_rcpt_no, created_at,
    confirmed_at, attempts, last_error
FROM sale;


CREATE TEMP TABLE stash_sale_line   AS SELECT * FROM sale_line;
CREATE TEMP TABLE stash_sale_tender AS SELECT * FROM sale_tender;
CREATE TEMP TABLE stash_outbound    AS SELECT * FROM outbound;
-- outbound is AUTOINCREMENT: its high-water mark can exceed max(id) and must
-- survive, or a recycled id could repeat one the cloud has already seen.
CREATE TEMP TABLE stash_outbound_seq AS
    SELECT seq FROM sqlite_sequence WHERE name = 'outbound';

DROP TABLE sale_line;
DROP TABLE sale_tender;
DROP TABLE outbound;
DROP TABLE sale;
ALTER TABLE sale_new RENAME TO sale;

-- The children, exactly as in 0001_init.sql.
CREATE TABLE sale_line (
    sale_id     TEXT NOT NULL REFERENCES sale (id),
    item_seq    INTEGER NOT NULL,
    item_cd     TEXT,
    item_cls_cd TEXT,
    item_nm     TEXT NOT NULL,
    qty         TEXT NOT NULL,
    prc         TEXT NOT NULL,
    sply_amt    TEXT NOT NULL,
    dc_rt       TEXT NOT NULL,
    dc_amt      TEXT NOT NULL,
    taxbl_amt   TEXT NOT NULL,
    tax_amt     TEXT NOT NULL,
    tax_ty_cd   TEXT NOT NULL,
    PRIMARY KEY (sale_id, item_seq)
);

CREATE TABLE sale_tender (
    sale_id    TEXT NOT NULL REFERENCES sale (id),
    seq        INTEGER NOT NULL,
    pmt_ty_cd  TEXT NOT NULL,
    amount     TEXT NOT NULL,
    PRIMARY KEY (sale_id, seq)
);

CREATE TABLE outbound (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,
    sale_id     TEXT REFERENCES sale (id),
    payload     TEXT NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  TEXT NOT NULL,
    sent_at     TEXT
);

INSERT INTO sale_line (sale_id, item_seq, item_cd, item_cls_cd, item_nm, qty, prc,
                       sply_amt, dc_rt, dc_amt, taxbl_amt, tax_amt, tax_ty_cd)
SELECT sale_id, item_seq, item_cd, item_cls_cd, item_nm, qty, prc,
       sply_amt, dc_rt, dc_amt, taxbl_amt, tax_amt, tax_ty_cd
FROM stash_sale_line;

INSERT INTO sale_tender (sale_id, seq, pmt_ty_cd, amount)
SELECT sale_id, seq, pmt_ty_cd, amount FROM stash_sale_tender;

INSERT INTO outbound (id, kind, sale_id, payload, attempts, last_error, created_at, sent_at)
SELECT id, kind, sale_id, payload, attempts, last_error, created_at, sent_at
FROM stash_outbound;

-- Restore the AUTOINCREMENT high-water mark (never lower it).
UPDATE sqlite_sequence
SET seq = MAX(seq, COALESCE((SELECT seq FROM stash_outbound_seq), 0))
WHERE name = 'outbound';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'outbound', seq FROM stash_outbound_seq
WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'outbound');

DROP TABLE stash_sale_line;
DROP TABLE stash_sale_tender;
DROP TABLE stash_outbound;
DROP TABLE stash_outbound_seq;

-- Indexes die with their tables; restore them.
CREATE INDEX sale_open_state ON sale (state) WHERE state IN ('AwaitingVsdc', 'Failed');
CREATE INDEX sale_shift ON sale (shift_id);
-- "The next unsent row" is ORDER BY id LIMIT 1 over unsent rows only.
CREATE INDEX outbound_unsent ON outbound (id) WHERE sent_at IS NULL;
CREATE INDEX outbound_sale ON outbound (sale_id) WHERE sale_id IS NOT NULL;
