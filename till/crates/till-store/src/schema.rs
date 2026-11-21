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
    Sqlx(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A stored column that will not parse back into its `till-core` type.
    #[error("undecodable stored value: {0}")]
    Decode(String),
    /// A new invoice number was requested while an `AwaitingVsdc` sale still
    /// holds `invc_no` on that branch (Ruling R38). The caller must retry that
    /// sale with the same number, not allocate another.
    #[error("invoice number {invc_no} is still awaiting the VSDC; resolve it first")]
    SequenceBlocked { invc_no: i64 },
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
    m.migrations = Cow::Owned(vec![Migration::new(
        1,
        Cow::Borrowed("init"),
        MigrationType::Simple,
        Cow::Borrowed(include_str!("../migrations/0001_init.sql")),
        false,
    )]);
    m
}

/// How long a writer waits for SQLite's write lock before failing with
/// `SQLITE_BUSY`. Under `BEGIN IMMEDIATE` a second invoice allocator queues on
/// this. Set explicitly (it equals sqlx's default) so it is a recorded
/// decision: 5 s outlasts any sale transaction by orders of magnitude, yet a
/// genuinely wedged lock surfaces at the counter instead of hanging the lane.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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

    /// Gives a branch its sequence row, as the allocator will in production.
    async fn seed_seq(s: &Store, bhf: &str) {
        sqlx::query("insert or ignore into invc_seq (bhf_id, last_invc_no) values (?, 0)")
            .bind(bhf)
            .execute(s.pool())
            .await
            .unwrap();
    }

    /// Inserts a sale for a branch that already has a sequence row.
    async fn insert_sale(s: &Store, id: &str, bhf: &str, no: i64) -> Result<(), sqlx::Error> {
        seed_seq(s, bhf).await;
        sqlx::query(
            "insert into sale (id, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) \
             values (?, ?, ?, 'AwaitingVsdc','0','0','0','01',?)",
        )
        .bind(id)
        .bind(bhf)
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
    async fn one_invoice_number_cannot_be_used_twice_on_a_branch() {
        let s = Store::open_in_memory().await.unwrap();
        insert_sale(&s, "s1", "00", 1).await.unwrap();
        assert!(insert_sale(&s, "s2", "00", 1).await.is_err());
        // another branch has its own sequence
        insert_sale(&s, "s3", "01", 1).await.unwrap();
    }

    #[tokio::test]
    async fn a_sale_for_a_branch_with_no_sequence_row_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        let r = sqlx::query("insert into sale (id, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s','00',1,'AwaitingVsdc','0','0','0','01',?)")
            .bind(NOW).execute(s.pool()).await;
        assert!(r.is_err(), "no invc_seq row for the branch");
        seed_seq(&s, "00").await;
        insert_sale(&s, "s", "00", 1).await.unwrap(); // same insert now succeeds
    }

    #[tokio::test]
    async fn an_unknown_sale_state_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        seed_seq(&s, "00").await; // so the refusal is the CHECK, not the branch foreign key
        let r = sqlx::query("insert into sale (id, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values ('s','00',1,'Bogus','0','0','0','01',?)")
            .bind(NOW).execute(s.pool()).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn a_numeric_looking_text_column_is_not_coerced() {
        // TEXT affinity must keep "10.00" as written, not turn it into 10.
        let s = Store::open_in_memory().await.unwrap();
        insert_sale(&s, "s", "00", 1).await.unwrap();
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
            insert_sale(&s, "s1", "00", 1).await.unwrap();
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

    /// Guards against the busy timeout being LOWERED on either path. It does
    /// NOT prove the option is set explicitly: 5000 ms is also sqlx's default,
    /// so deleting the `.busy_timeout(..)` call would still pass.
    #[tokio::test]
    async fn busy_timeout_is_five_seconds() {
        async fn ms(s: &Store) -> i64 {
            sqlx::query_scalar("pragma busy_timeout")
                .fetch_one(s.pool())
                .await
                .unwrap()
        }
        assert_eq!(ms(&Store::open_in_memory().await.unwrap()).await, 5000);
        let dir = std::env::temp_dir().join(format!("till-store-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(&dir.join("till.db")).await.unwrap();
        assert_eq!(ms(&s).await, 5000);
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
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
}
