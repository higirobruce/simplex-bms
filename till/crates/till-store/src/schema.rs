//! Opening the store and running its migrations, plus the boundary decoders
//! that turn stored TEXT back into `till-core` types.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use std::borrow::Cow;

use sqlx::SqlitePool;
use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use thiserror::Error;
use till_core::codes::{PmtTyCd, TaxType};
use till_core::{Money, Qty};

/// Every way the store can fail. Nothing here panics; the caller decides.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlx(sqlx::Error),
    /// SQLite's write lock was still held by another connection when the
    /// `busy_timeout` ran out (`SQLITE_BUSY`). Nothing was written. It is
    /// transient by nature: a caller may retry ONCE, and if that also fails
    /// must tell the cashier something they can act on ("the till is busy
    /// syncing, try again in a moment"), never the raw database error.
    #[error("the till database is busy; try again")]
    Busy,
    /// Two products in one catalogue batch claim the same barcode. Caught in
    /// memory before the write lock is taken; names both so the cloud data can
    /// be fixed.
    #[error("products {first_id} and {second_id} both claim barcode {barcode}")]
    DuplicateBarcodeInBatch {
        barcode: String,
        first_id: String,
        second_id: String,
    },
    /// A text field over the VSDC's declared length (see `till-core`).
    #[error(transparent)]
    FieldTooLong(#[from] till_core::FieldTooLong),
    /// The sale is `Confirmed`: a fiscal record that must not be destroyed.
    #[error("sale {id} is confirmed; a confirmed sale cannot be abandoned")]
    SaleAlreadyConfirmed { id: String },
    /// Impossible-arm placeholder: a tender position that does not fit an
    /// `i64`. A sale has far fewer tenders than that, so this cannot happen;
    /// it exists so the conversion has an honest name instead of borrowing
    /// another variant's message.
    #[error("tender position out of range (impossible)")]
    TenderSeqOutOfRange,
    /// `shift_totals` was asked about a shift that does not exist.
    #[error("shift {id} does not exist")]
    ShiftNotFound { id: String },
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A stored column that will not parse back into its `till-core` type.
    #[error("undecodable stored value: {0}")]
    Decode(String),
    /// A new invoice number was requested while an `AwaitingVsdc` sale still
    /// holds `invc_no` in that series (Ruling R38). The caller must retry that
    /// sale with the same number, not allocate another.
    #[error("invoice number {invc_no} is still awaiting the VSDC; resolve it first")]
    SequenceBlocked { invc_no: i64 },
    /// `reconcile_counter_from_vsdc` was asked to lower a counter. Lowering
    /// would reissue invoice numbers this till has already recorded.
    #[error("counter {counter} is at {current}; refusing to lower it to {requested}")]
    CounterWouldDecrease {
        counter: &'static str,
        current: i64,
        requested: i64,
    },
    /// A counter value from outside is negative, which no VSDC counter is.
    #[error("counter value {value} is out of range")]
    CounterOutOfRange { value: i64 },
    /// A sale with no lines is a programming error, not a sale. SQLite has no
    /// deferred constraints, so the schema cannot enforce "a sale has lines";
    /// the sale transaction does.
    #[error("a sale must have at least one line")]
    EmptySale,
    #[error(transparent)]
    Basket(#[from] till_core::BasketError),
    #[error(transparent)]
    Tender(#[from] till_core::TenderError),
    /// The `Settled` handed to `begin_sale` is not what the tenders settle to
    /// for this basket, so the header payment type or change would be wrong.
    #[error("the settlement does not match the basket and tenders")]
    SettlementMismatch,
    #[error("no sale with id {id} and invoice number {invc_no}")]
    SaleNotFound { id: String, invc_no: i64 },
    /// The operation needs an `AwaitingVsdc` sale and this one is `state`.
    #[error("sale {id} is {state}, not AwaitingVsdc")]
    SaleNotAwaiting { id: String, state: String },
    /// The sale is already `Confirmed` with a DIFFERENT receipt. Two receipts
    /// for one invoice number is never acceptable; nothing is overwritten.
    #[error("sale {id} is already confirmed with a different receipt")]
    ReceiptConflict { id: String },
    /// A catalogue product the cache refuses to hold, so the whole sync is
    /// rolled back rather than caching something no scan could ever reach.
    #[error("catalogue product {id} rejected: {reason}")]
    CatalogueRejected { id: String, reason: String },
    /// A catalogue price below zero. `Money` allows negatives (refunds need
    /// them), but a product that pays the customer is a data error.
    #[error("catalogue product {id} has a negative price")]
    NegativeCataloguePrice { id: String },
    /// Several cached products share one PLU. A PLU is not unique in the
    /// schema, and guessing between them would price the wrong goods.
    #[error("PLU {plu} belongs to more than one cached product")]
    AmbiguousPlu { plu: String },
    /// This device already has an open shift; close it before opening another.
    #[error("device {dvc_srl_no} already has an open shift")]
    ShiftAlreadyOpen { dvc_srl_no: String },
    /// A sale needs an open shift on the device and there is none.
    #[error("device {dvc_srl_no} has no open shift")]
    NoOpenShift { dvc_srl_no: String },
    /// A closed shift's Z report is final; it cannot be closed again.
    #[error("this shift is already closed")]
    ShiftAlreadyClosed,
    /// A sale on this shift is still awaiting the VSDC and holds a live invoice number.
    #[error("a sale on this shift is still awaiting the VSDC")]
    ShiftHasPendingSale,
    /// An opening float below zero is a data-entry error, not a float.
    #[error("the opening float cannot be negative")]
    NegativeOpeningFloat,
    #[error("cloud payload could not be serialised: {0}")]
    Payload(#[from] serde_json::Error),
}

/// Maps SQLite's `SQLITE_BUSY` to [`StoreError::Busy`] and everything else to
/// [`StoreError::Sqlx`]. Every `?` on a sqlx call in this crate goes through
/// here, so `begin_immediate`, `begin_sale`, `confirm_sale` and
/// `sync_catalogue` all report a lock timeout the same, named way.
///
/// Uses the error's numeric code, not its message text. SQLite's code is
/// "extended": the low byte is the primary code and the upper bits refine it
/// (`SQLITE_BUSY_SNAPSHOT` is 517, `SQLITE_BUSY_RECOVERY` 261), so the test is
/// `code & 0xff == SQLITE_BUSY (5)`. `SQLITE_LOCKED` (6) is a different
/// condition (shared-cache table locks) and is deliberately not mapped.
impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> StoreError {
        const SQLITE_BUSY: i64 = 5;
        let busy = e
            .as_database_error()
            .and_then(|d| d.code())
            .and_then(|c| c.parse::<i64>().ok())
            .is_some_and(|c| c & 0xff == SQLITE_BUSY);
        if busy {
            StoreError::Busy
        } else {
            StoreError::Sqlx(e)
        }
    }
}

/// The migrations, compiled into the binary so a till needs no files beside
/// its executable.
///
/// `sqlx::migrate!` would do this, but it lives behind sqlx's `macros`
/// feature, which also enables the compile-time `query!` family that this
/// crate must not be able to use. So the list is built by hand with
/// `include_str!`; a test checks it covers every file in `migrations/`.
fn migrator() -> Migrator {
    let mut m = Migrator::DEFAULT;
    m.migrations = Cow::Owned(vec![
        Migration::new(
            1,
            Cow::Borrowed("init"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!("../migrations/0001_init.sql")),
            false,
        ),
        Migration::new(
            2,
            Cow::Borrowed("shift"),
            MigrationType::Simple,
            Cow::Borrowed(include_str!("../migrations/0002_shift.sql")),
            false,
        ),
    ]);
    m
}

/// How long a writer waits for SQLite's write lock before failing with
/// `SQLITE_BUSY`. Under `BEGIN IMMEDIATE` a second invoice allocator queues on
/// this.
///
/// Why 2 s: an allocation transaction is one UPDATE and one INSERT on WAL
/// (sub-millisecond); the sale transaction adds its line and tender rows (a
/// few milliseconds even at the 999-line ceiling). The one transaction that is
/// NOT that short is a catalogue sync, which scales with the catalogue (about
/// 2 s for 60,000 products measured on a development machine, several times
/// that on eMMC); it is kept to one statement per product and a handful of
/// batch statements, and a sale that still loses the race gets
/// [`StoreError::Busy`], which the caller may retry once. No transaction ever spans
/// the VSDC round trip: the `AwaitingVsdc` row is committed, the network call
/// runs with no lock held, and confirmation opens a fresh transaction. So real
/// contention clears in single-digit milliseconds and 2 s is roughly 200x
/// that, while a genuinely stuck lane surfaces to the cashier sooner than
/// sqlx's inherited 5 s would.
///
/// Deliberately NOT 5000: that is sqlx's default, and a value equal to the
/// default makes `busy_timeout_is_two_seconds` unable to notice the option
/// being dropped. Do not "tidy" this back to 5 s.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// Handle to the local database.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (creating if absent) the database at `path`, enabling foreign
    /// keys and WAL, and applies pending migrations.
    ///
    /// WAL because the till reads its catalogue while writing a sale; the
    /// default rollback journal would make the reader block the writer.
    pub async fn open(path: &Path) -> Result<Store, StoreError> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new().connect_with(opts).await?;
        Store::migrated(pool).await
    }

    /// A private in-memory database, for tests.
    ///
    /// Each SQLite connection to `:memory:` is its own empty database, so the
    /// pool is pinned to exactly one connection that is never recycled.
    pub async fn open_in_memory() -> Result<Store, StoreError> {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .busy_timeout(BUSY_TIMEOUT)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await?;
        Store::migrated(pool).await
    }

    async fn migrated(pool: SqlitePool) -> Result<Store, StoreError> {
        migrator().run(&pool).await?;
        Ok(Store { pool })
    }

    /// A file-backed store with a short busy timeout, so a lock test does not
    /// wait the production 2 s. Returns its temp directory for cleanup.
    #[cfg(test)]
    pub(crate) async fn open_with_busy_timeout_for_test(
        tag: &str,
        millis: u64,
    ) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("till-busy-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let opts = SqliteConnectOptions::new()
            .filename(dir.join("till.db"))
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(millis))
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        (Store::migrated(pool).await.unwrap(), dir)
    }

    /// The underlying pool, for the queries of later modules.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Parses a stored money string. Goes through `Money::try_from_str`, so the
/// 12-digit bound is enforced on the way out as well as the way in.
pub fn decode_money(column: &str, s: &str) -> Result<Money, StoreError> {
    Money::try_from_str(s).map_err(|e| StoreError::Decode(format!("{column}: {e}")))
}

/// Parses a stored quantity string.
pub fn decode_qty(column: &str, s: &str) -> Result<Qty, StoreError> {
    Qty::try_from_str(s).map_err(|e| StoreError::Decode(format!("{column}: {e}")))
}

/// Parses a stored tax-type code.
pub fn decode_tax_ty(column: &str, s: &str) -> Result<TaxType, StoreError> {
    TaxType::from_str(s).map_err(|e| StoreError::Decode(format!("{column}: {e}")))
}

/// Parses a stored payment-type code.
pub fn decode_pmt_ty(column: &str, s: &str) -> Result<PmtTyCd, StoreError> {
    PmtTyCd::from_str(s).map_err(|e| StoreError::Decode(format!("{column}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-21T00:00:00Z";

    /// Gives a device's `SaleInvc` series its sequence row, as the allocator
    /// will in production.
    async fn seed_seq(s: &Store, dvc: &str) {
        sqlx::query(
            "insert or ignore into invc_seq (dvc_srl_no, counter, last_invc_no) values (?, 'SaleInvc', 0)",
        )
        .bind(dvc)
        .execute(s.pool())
        .await
        .unwrap();
    }

    /// Every sale belongs to a shift; tests share one open shift, `sh1`.
    async fn seed_shift(s: &Store) {
        sqlx::query(
            "insert or ignore into shift (id, dvc_srl_no, opened_by, opened_at, opening_float) values ('sh1', 'd1', 'cashier', ?, '0.00')",
        )
        .bind(NOW)
        .execute(s.pool())
        .await
        .unwrap();
    }

    /// Inserts a sale for a device whose series already has a sequence row.
    /// Every sale here is on branch "00": lanes of one shop share it.
    async fn insert_sale(s: &Store, id: &str, dvc: &str, no: i64) -> Result<(), sqlx::Error> {
        seed_seq(s, dvc).await;
        seed_shift(s).await;
        sqlx::query(
            "insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) \
             values (?, 'sh1', ?, 'SaleInvc', '00', ?, 'AwaitingVsdc','0','0','0','01',?)",
        )
        .bind(id)
        .bind(dvc)
        .bind(no)
        .bind(NOW)
        .execute(s.pool())
        .await
        .map(|_| ())
    }

    #[tokio::test]
    async fn an_opened_store_has_every_table() {
        let s = Store::open_in_memory().await.unwrap();
        for t in [
            "product",
            "invc_seq",
            "sale",
            "sale_line",
            "sale_tender",
            "outbound",
            "shift",
        ] {
            let n: i64 = sqlx::query_scalar(
                "select count(*) from sqlite_master where type='table' and name=?",
            )
            .bind(t)
            .fetch_one(s.pool())
            .await
            .unwrap();
            assert_eq!(n, 1, "missing table {t}");
        }
    }

    #[tokio::test]
    async fn money_survives_a_round_trip_exactly() {
        // the reason every amount column is TEXT: 0.1+0.2 in a float column is not 0.3
        let s = Store::open_in_memory().await.unwrap();
        // (id, text, is a quantity-scale value)
        for (id, price, is_qty) in [
            ("p1", "1234567.89", false),
            ("p2", "0.10", false),
            ("p3", "10.00", false),
            ("p4", "0.001", true),
            ("p5", "9999999999.99", false),
        ] {
            sqlx::query("insert into product (id, sku, name, unit_price, tax_ty_cd, is_weighed, synced_at) values (?,?,?,?,?,?,?)")
                .bind(id).bind("S").bind("N").bind(price).bind("B").bind(0).bind(NOW)
                .execute(s.pool()).await.unwrap();
            let got: String = sqlx::query_scalar("select unit_price from product where id=?")
                .bind(id)
                .fetch_one(s.pool())
                .await
                .unwrap();
            assert_eq!(got, price);
            let back = if is_qty {
                decode_qty("unit_price", &got).unwrap().to_string()
            } else {
                decode_money("unit_price", &got).unwrap().to_string()
            };
            assert_eq!(back, price);
        }
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let s = Store::open_in_memory().await.unwrap();
        let line = sqlx::query("insert into sale_line (sale_id, item_seq, item_nm, qty, prc, sply_amt, dc_rt, dc_amt, taxbl_amt, tax_amt, tax_ty_cd) values ('nope',1,'x','1','1','1','0','0','1','0','B')")
            .execute(s.pool()).await;
        assert!(
            line.is_err(),
            "a line against a missing sale must be refused"
        );
        let tender = sqlx::query(
            "insert into sale_tender (sale_id, seq, pmt_ty_cd, amount) values ('nope',1,'01','1')",
        )
        .execute(s.pool())
        .await;
        assert!(
            tender.is_err(),
            "a tender against a missing sale must be refused"
        );
        let out = sqlx::query("insert into outbound (kind, sale_id, payload, created_at) values ('sale','nope','{}',?)")
            .bind(NOW)
            .execute(s.pool()).await;
        assert!(
            out.is_err(),
            "an outbound row against a missing sale must be refused"
        );
    }

    #[tokio::test]
    async fn one_invoice_number_cannot_be_used_twice_in_a_series() {
        let s = Store::open_in_memory().await.unwrap();
        insert_sale(&s, "s1", "d1", 1).await.unwrap();
        assert!(insert_sale(&s, "s2", "d1", 1).await.is_err());
        // another device has its own series, on the same branch "00"
        insert_sale(&s, "s3", "d2", 1).await.unwrap();
        // and another counter of the same device has its own too
        sqlx::query("insert into invc_seq values ('d1', 'TrainInvc', 0)")
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query("insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s4','sh1','d1','TrainInvc','00',1,'AwaitingVsdc','0','0','0','01',?)")
            .bind(NOW).execute(s.pool()).await.unwrap();
    }

    #[tokio::test]
    async fn a_sale_in_a_series_with_no_sequence_row_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        seed_shift(&s).await; // so the refusal is the series foreign key, not the shift's
        let r = sqlx::query("insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s','sh1','d1','SaleInvc','00',1,'AwaitingVsdc','0','0','0','01',?)")
            .bind(NOW).execute(s.pool()).await;
        assert!(r.is_err(), "no invc_seq row for the series");
        seed_seq(&s, "d1").await;
        insert_sale(&s, "s", "d1", 1).await.unwrap(); // same insert now succeeds
    }

    #[tokio::test]
    async fn an_unknown_sale_state_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        seed_seq(&s, "d1").await; // so the refusal is not the series foreign key
        seed_shift(&s).await; // nor the shift foreign key: only the CHECK is left
        let r = sqlx::query("insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s','sh1','d1','SaleInvc','00',1,'Bogus','0','0','0','01',?)")
            .bind(NOW).execute(s.pool()).await;
        assert!(r.unwrap_err().to_string().contains("CHECK"));
    }

    #[tokio::test]
    async fn a_numeric_looking_text_column_is_not_coerced() {
        // TEXT affinity must keep "10.00" as written, not turn it into 10.
        let s = Store::open_in_memory().await.unwrap();
        insert_sale(&s, "s", "d1", 1).await.unwrap();
        sqlx::query(
            "insert into sale_tender (sale_id, seq, pmt_ty_cd, amount) values ('s',1,'01','10.00')",
        )
        .execute(s.pool())
        .await
        .unwrap();
        let (got, ty): (String, String) =
            sqlx::query_as("select amount, typeof(amount) from sale_tender")
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!((got.as_str(), ty.as_str()), ("10.00", "text"));
    }

    #[tokio::test]
    async fn the_next_unsent_outbound_row_uses_the_partial_index() {
        let s = Store::open_in_memory().await.unwrap();
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "explain query plan select id from outbound where sent_at is null order by id limit 1",
        )
        .fetch_all(s.pool())
        .await
        .unwrap();
        let text: String = plan
            .iter()
            .map(|r| r.3.clone())
            .collect::<Vec<_>>()
            .join(";");
        assert!(text.contains("outbound_unsent"), "plan was: {text}");
    }

    #[tokio::test]
    async fn a_file_store_survives_reopen_and_uses_wal() {
        let dir = std::env::temp_dir().join(format!("till-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("till.db");
        {
            let s = Store::open(&path).await.unwrap();
            let mode: String = sqlx::query_scalar("pragma journal_mode")
                .fetch_one(s.pool())
                .await
                .unwrap();
            assert_eq!(mode, "wal");
            let fk: i64 = sqlx::query_scalar("pragma foreign_keys")
                .fetch_one(s.pool())
                .await
                .unwrap();
            assert_eq!(fk, 1);
            insert_sale(&s, "s1", "d1", 1).await.unwrap();
        }
        let s = Store::open(&path).await.unwrap(); // re-running migrations is a no-op
        let n: i64 = sqlx::query_scalar("select count(*) from sale")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(n, 1);
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn every_pooled_connection_of_a_file_store_has_fk_and_wal() {
        // foreign_keys defaults OFF and is per-connection: if pool construction
        // ever drops it, every FK silently stops being enforced. This uses a
        // FILE-backed store (not open_in_memory, which is pinned to one connection)
        // and holds several connections at once so each is a distinct one.
        let dir = std::env::temp_dir().join(format!("till-store-pragma-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(&dir.join("till.db")).await.unwrap();
        let mut conns = Vec::new();
        for _ in 0..4 {
            conns.push(s.pool().acquire().await.unwrap());
        }
        for c in conns.iter_mut() {
            let fk: i64 = sqlx::query_scalar("pragma foreign_keys")
                .fetch_one(&mut **c)
                .await
                .unwrap();
            assert_eq!(fk, 1);
            let mode: String = sqlx::query_scalar("pragma journal_mode")
                .fetch_one(&mut **c)
                .await
                .unwrap();
            assert_eq!(mode, "wal");
            let r = sqlx::query("insert into sale_line (sale_id, item_seq, item_nm, qty, prc, sply_amt, dc_rt, dc_amt, taxbl_amt, tax_amt, tax_ty_cd) values ('nope',1,'x','1','1','1','0','0','1','0','B')")
                .execute(&mut **c).await;
            assert!(
                r.is_err(),
                "a violating insert must be refused on every connection"
            );
        }
        drop(conns);
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Pins the busy timeout on both open paths. Because 2000 ms differs from
    /// sqlx's default (5000), deleting the `.busy_timeout(..)` call from either
    /// path makes this fail; the value was chosen off the default partly for
    /// that reason.
    #[tokio::test]
    async fn busy_timeout_is_two_seconds() {
        async fn ms(s: &Store) -> i64 {
            sqlx::query_scalar("pragma busy_timeout")
                .fetch_one(s.pool())
                .await
                .unwrap()
        }
        assert_eq!(ms(&Store::open_in_memory().await.unwrap()).await, 2000);
        let dir = std::env::temp_dir().join(format!("till-store-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(&dir.join("till.db")).await.unwrap();
        assert_eq!(ms(&s).await, 2000);
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_sale_cannot_exist_without_a_shift() {
        // Foreign keys are per-connection and default OFF, so first prove this
        // very connection has them on, then show the refusal is the shift FK
        // (not the series FK or a CHECK) by making the same insert succeed once
        // the shift exists.
        let s = Store::open_in_memory().await.unwrap();
        let fk: i64 = sqlx::query_scalar("pragma foreign_keys")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(fk, 1);
        seed_seq(&s, "d1").await;
        let insert = "insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s', 'ghost', 'd1', 'SaleInvc', '00', 1, 'AwaitingVsdc', '0', '0', '0', '01', ?)";
        let refused = sqlx::query(insert).bind(NOW).execute(s.pool()).await;
        let msg = refused.unwrap_err().to_string();
        assert!(msg.contains("FOREIGN KEY"), "unexpected refusal: {msg}");
        // NULL is refused too: the column is NOT NULL, not merely a nullable FK.
        let null = sqlx::query("insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s', NULL, 'd1', 'SaleInvc', '00', 1, 'AwaitingVsdc', '0', '0', '0', '01', ?)")
            .bind(NOW)
            .execute(s.pool())
            .await;
        assert!(null.unwrap_err().to_string().contains("NOT NULL"));
        sqlx::query("insert into shift (id, dvc_srl_no, opened_by, opened_at, opening_float) values ('ghost', 'd1', 'c', ?, '0.00')")
            .bind(NOW)
            .execute(s.pool())
            .await
            .unwrap();
        sqlx::query(insert)
            .bind(NOW)
            .execute(s.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_open_shift_has_no_closed_at() {
        let s = Store::open_in_memory().await.unwrap();
        sqlx::query("insert into shift (id, dvc_srl_no, opened_by, opened_at, opening_float) values ('a', 'd1', 'c', ?, '1000.00')")
            .bind(NOW)
            .execute(s.pool())
            .await
            .unwrap();
        let (closed, declared, float, ty): (Option<String>, Option<String>, String, String) =
            sqlx::query_as(
                "select closed_at, declared_cash, opening_float, typeof(opening_float) from shift where id = 'a'",
            )
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!((closed, declared), (None, None));
        assert_eq!((float.as_str(), ty.as_str()), ("1000.00", "text"));
    }

    /// Migrator limited to the first `n` migrations.
    fn migrator_upto(n: usize) -> Migrator {
        let mut m = migrator();
        m.migrations = Cow::Owned(m.migrations.iter().take(n).cloned().collect());
        m
    }

    async fn raw_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await
            .unwrap()
    }

    /// Column name, type, notnull, default, pk of a table.
    async fn columns(p: &SqlitePool, t: &str) -> Vec<(String, String, i64, Option<String>, i64)> {
        sqlx::query_as(&format!(
            "select name, type, \"notnull\", dflt_value, pk from pragma_table_info('{t}') order by name"
        ))
        .fetch_all(p)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn migrating_an_empty_database_creates_no_shift() {
        let p = raw_pool().await;
        migrator().run(&p).await.unwrap();
        let n: i64 = sqlx::query_scalar("select count(*) from shift")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn migration_0002_keeps_every_sale_and_child_row_and_the_sale_shape() {
        let p = raw_pool().await;
        migrator_upto(1).run(&p).await.unwrap();
        let before = columns(&p, "sale").await;
        let idx_before: Vec<(String, i64)> = sqlx::query_as(
            "select name, \"unique\" from pragma_index_list('sale') where origin = 'c' or origin = 'u' order by name",
        )
        .fetch_all(&p)
        .await
        .unwrap();
        let sql_before: String =
            sqlx::query_scalar("select sql from sqlite_master where name='sale_open_state'")
                .fetch_one(&p)
                .await
                .unwrap();

        sqlx::query("insert into invc_seq values ('d1', 'SaleInvc', 2)")
            .execute(&p)
            .await
            .unwrap();
        for (id, n, st) in [("s1", 1, "Confirmed"), ("s2", 2, "AwaitingVsdc")] {
            sqlx::query("insert into sale (id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, buyer_tin, rcpt_no, intrl_data, rcpt_sign, sdc_id, mrc_no, vsdc_rcpt_pbct_date, tot_rcpt_no, created_at, confirmed_at, attempts, last_error) values (?, 'd1', 'SaleInvc', '00', ?, ?, '118.00', '100.00', '18.00', '01', '123456789', 9, 'i', 'r', 'sdc', 'mrc', '20261121103000', 4, '2026-11-20T08:00:00Z', '2026-11-20T08:01:00Z', 3, 'e')")
                .bind(id).bind(n).bind(st).execute(&p).await.unwrap();
        }
        sqlx::query("insert into sale_line (sale_id, item_seq, item_nm, qty, prc, sply_amt, dc_rt, dc_amt, taxbl_amt, tax_amt, tax_ty_cd) values ('s1',1,'x','1','1','1','0','0','1','0','B')").execute(&p).await.unwrap();
        sqlx::query(
            "insert into sale_tender (sale_id, seq, pmt_ty_cd, amount) values ('s1',1,'01','1')",
        )
        .execute(&p)
        .await
        .unwrap();
        sqlx::query("insert into outbound (kind, sale_id, payload, created_at) values ('sale','s1','{}','2026-11-20T08:01:00Z')").execute(&p).await.unwrap();
        let old_rows: Vec<(String,)> = sqlx::query_as(
            "select id || tot_amt || buyer_tin || rcpt_no || intrl_data || rcpt_sign || sdc_id || mrc_no || vsdc_rcpt_pbct_date || tot_rcpt_no || created_at || confirmed_at || attempts || last_error || state || invc_no from sale order by id",
        ).fetch_all(&p).await.unwrap();

        // outbound's AUTOINCREMENT mark above its max id (rows were deleted)
        sqlx::query("update sqlite_sequence set seq = 5 where name = 'outbound'")
            .execute(&p)
            .await
            .unwrap();
        let master_before: Vec<(String, String)> = sqlx::query_as(
            "select name, sql from sqlite_master where name in ('sale_line','sale_tender','outbound','outbound_unsent','outbound_sale') order by name",
        ).fetch_all(&p).await.unwrap();

        migrator().run(&p).await.unwrap();
        let fk: i64 = sqlx::query_scalar("pragma foreign_keys")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(fk, 1, "the migration must leave foreign keys on");
        let seq_after: Vec<(i64,)> =
            sqlx::query_as("select seq from sqlite_sequence where name = 'outbound'")
                .fetch_all(&p)
                .await
                .unwrap();
        assert_eq!(seq_after, vec![(5,)], "AUTOINCREMENT high-water mark kept");
        let master_after: Vec<(String, String)> = sqlx::query_as(
            "select name, sql from sqlite_master where name in ('sale_line','sale_tender','outbound','outbound_unsent','outbound_sale') order by name",
        ).fetch_all(&p).await.unwrap();
        assert_eq!(
            master_before, master_after,
            "children are definition-identical"
        );

        // every row, every value, now on the one synthetic closed shift
        let new_rows: Vec<(String,)> = sqlx::query_as(
            "select id || tot_amt || buyer_tin || rcpt_no || intrl_data || rcpt_sign || sdc_id || mrc_no || vsdc_rcpt_pbct_date || tot_rcpt_no || created_at || confirmed_at || attempts || last_error || state || invc_no from sale order by id",
        ).fetch_all(&p).await.unwrap();
        assert_eq!(old_rows, new_rows);
        let shifts: Vec<(String, String, String, String, Option<String>, String)> = sqlx::query_as(
            "select id, dvc_srl_no, opened_by, opening_float, declared_cash, typeof(closed_at) from shift",
        ).fetch_all(&p).await.unwrap();
        assert_eq!(shifts.len(), 1);
        assert_eq!(shifts[0].2, "migration");
        assert_eq!(shifts[0].3, "0.00");
        assert_eq!(shifts[0].5, "text", "the synthetic shift is closed");
        let on_shift: i64 =
            sqlx::query_scalar("select count(*) from sale where shift_id = (select id from shift)")
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(on_shift, 2);
        for (t, n) in [("sale_line", 1), ("sale_tender", 1), ("outbound", 1)] {
            let c: i64 = sqlx::query_scalar(&format!("select count(*) from {t}"))
                .fetch_one(&p)
                .await
                .unwrap();
            assert_eq!(c, n, "{t} rows survive the rebuild");
        }

        // shape: the old columns are all still there, unchanged, plus shift_id
        let mut after = columns(&p, "sale").await;
        let added = after
            .iter()
            .position(|c| c.0 == "shift_id")
            .expect("shift_id");
        assert_eq!(
            after.remove(added),
            ("shift_id".into(), "TEXT".into(), 1, None, 0)
        );
        assert_eq!(before, after);
        // indexes and unique constraints
        let idx_after: Vec<(String, i64)> = sqlx::query_as(
            "select name, \"unique\" from pragma_index_list('sale') where origin = 'c' or origin = 'u' order by name",
        ).fetch_all(&p).await.unwrap();
        for old in &idx_before {
            if old.0.starts_with("sqlite_autoindex") {
                continue; // auto-named; the UNIQUE is checked below
            }
            assert!(idx_after.contains(old), "lost index {old:?}");
        }
        let sql_after: String =
            sqlx::query_scalar("select sql from sqlite_master where name='sale_open_state'")
                .fetch_one(&p)
                .await
                .unwrap();
        assert_eq!(sql_before, sql_after, "partial index definition");
        let table_sql: String =
            sqlx::query_scalar("select sql from sqlite_master where name='sale'")
                .fetch_one(&p)
                .await
                .unwrap();
        assert!(table_sql.contains("UNIQUE (dvc_srl_no, counter, invc_no)"));
        assert!(table_sql.contains("CHECK (invc_no >= 1)"));
        assert!(table_sql.contains("'AwaitingVsdc', 'Confirmed', 'Failed', 'Voided'"));
        // foreign keys: series, shift, and the three children still point at sale
        let fks: Vec<(String, String)> = sqlx::query_as(
            "select \"table\", \"from\" from pragma_foreign_key_list('sale') order by \"from\"",
        )
        .fetch_all(&p)
        .await
        .unwrap();
        assert!(fks.contains(&("invc_seq".into(), "dvc_srl_no".into())));
        assert!(fks.contains(&("invc_seq".into(), "counter".into())));
        assert!(fks.contains(&("shift".into(), "shift_id".into())));
        for child in ["sale_line", "sale_tender", "outbound"] {
            let to: Vec<(String,)> = sqlx::query_as(&format!(
                "select \"table\" from pragma_foreign_key_list('{child}')"
            ))
            .fetch_all(&p)
            .await
            .unwrap();
            assert_eq!(to, vec![("sale".to_string(),)]);
        }
        let violations: Vec<(String,)> =
            sqlx::query_as("select \"table\" from pragma_foreign_key_check")
                .fetch_all(&p)
                .await
                .unwrap();
        assert!(violations.is_empty());
    }

    #[test]
    fn every_migration_file_is_embedded() {
        // Compare names, not counts, and only *.sql: a stray .DS_Store must not
        // fail this, and a registered-but-wrong file must.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sql"))
            .collect();
        on_disk.sort();
        let mut registered: Vec<String> = migrator()
            .migrations
            .iter()
            .map(|m| format!("{:04}_{}.sql", m.version, m.description))
            .collect();
        registered.sort();
        assert_eq!(on_disk, registered);
    }

    #[test]
    fn decoders_name_the_bad_column() {
        let e = decode_money("tot_amt", "1.234").unwrap_err();
        assert!(matches!(&e, StoreError::Decode(m) if m.starts_with("tot_amt")));
        assert!(decode_qty("qty", "abc").is_err());
        assert!(decode_tax_ty("tax_ty_cd", "Z").is_err());
        assert!(decode_pmt_ty("pmt_ty_cd", "99").is_err());
        assert!(decode_tax_ty("tax_ty_cd", "B").is_ok());
    }

    #[tokio::test]
    async fn a_locked_database_gives_busy_from_every_write_entry_point() {
        let (s, dir) = Store::open_with_busy_timeout_for_test("busy", 50).await;
        // another connection holds the write lock, as a long catalogue sync would
        let holder = s.begin_immediate().await.unwrap();
        assert!(matches!(s.begin_immediate().await, Err(StoreError::Busy)));
        let p = crate::CachedProduct {
            id: "p".into(),
            sku: "S".into(),
            name: "N".into(),
            barcode: None,
            plu: None,
            unit_price: Money::try_from_str("1").unwrap(),
            tax_ty_cd: TaxType::B,
            is_weighed: false,
            item_cls_cd: None,
        };
        assert!(matches!(
            s.sync_catalogue(std::slice::from_ref(&p)).await,
            Err(StoreError::Busy)
        ));
        let h = crate::SaleHandle {
            id: "x".into(),
            invc_no: 1,
        };
        let r = crate::VsdcReceipt {
            rcpt_no: 1,
            intrl_data: "i".into(),
            rcpt_sign: "s".into(),
            sdc_id: "d".into(),
            mrc_no: "m".into(),
            vsdc_rcpt_pbct_date: "20261121103000".into(),
            tot_rcpt_no: 1,
        };
        assert!(matches!(s.confirm_sale(&h, r).await, Err(StoreError::Busy)));
        drop(holder);
        // and it is genuinely transient: once the lock is released it works
        s.sync_catalogue(std::slice::from_ref(&p)).await.unwrap();
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_sqlite_busy_codes_map_to_busy() {
        // not a database error at all
        assert!(matches!(
            StoreError::from(sqlx::Error::PoolTimedOut),
            StoreError::Sqlx(_)
        ));
    }
}
