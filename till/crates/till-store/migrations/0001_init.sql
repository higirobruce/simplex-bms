-- The till's own durable memory. Every money and quantity column is TEXT
-- holding an exact decimal string ("1234567.89", "0.250"). REAL is an IEEE
-- float; NUMERIC is only an affinity and converts some inputs to float; an
-- integer of cents would not round-trip the cloud's Decimal(12,2)/(12,3)
-- strings unchanged. TEXT is the one type that cannot lose a franc. Values
-- are validated through till-core's Money/Qty at the Rust boundary.

CREATE TABLE product (
    id          TEXT PRIMARY KEY,
    sku         TEXT NOT NULL,
    name        TEXT NOT NULL,
    barcode     TEXT UNIQUE,
    plu         TEXT,
    unit_price  TEXT NOT NULL,
    tax_ty_cd   TEXT NOT NULL,
    is_weighed  INTEGER NOT NULL CHECK (is_weighed IN (0, 1)),
    item_cls_cd TEXT,
    department  TEXT,
    synced_at   TEXT NOT NULL
);
-- Offline scan lookup by keyed PLU must not scan the whole catalogue.
CREATE INDEX product_plu ON product (plu) WHERE plu IS NOT NULL;

-- One row per branch: the last invoice number handed out. Allocation (a later
-- task) must bump this inside the same transaction that inserts the sale.
CREATE TABLE invc_seq (
    bhf_id        TEXT PRIMARY KEY,
    last_invc_no  INTEGER NOT NULL CHECK (last_invc_no >= 0)
);

CREATE TABLE sale (
    id             TEXT PRIMARY KEY,
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
    created_at     TEXT NOT NULL,
    confirmed_at   TEXT,
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    UNIQUE (bhf_id, invc_no)
);
-- Finding sales that still need the VSDC's attention (resume after a crash).
CREATE INDEX sale_open_state ON sale (state) WHERE state IN ('AwaitingVsdc', 'Failed');

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
-- "The next unsent row" is ORDER BY id LIMIT 1 over unsent rows only; a
-- partial index keeps that O(1) however long sent history grows.
CREATE INDEX outbound_unsent ON outbound (id) WHERE sent_at IS NULL;
CREATE INDEX outbound_sale ON outbound (sale_id) WHERE sale_id IS NOT NULL;
