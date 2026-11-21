//! The gapless per-device, per-receipt-type invoice-number sequence (Ruling R38).
//!
//! A gap in a fiscal invoice sequence is what a tax audit looks for. The
//! number is consumed before the VSDC is contacted, so the guarantee rests on
//! three rules:
//!
//! 1. Allocating the number and inserting the sale row (state `AwaitingVsdc`)
//!    happen in ONE transaction, so a crash leaves both or neither. Task 12
//!    does that by calling [`allocate_on`] inside its own transaction, opened
//!    with [`Store::begin_immediate`].
//! 2. A VSDC failure leaves the sale `AwaitingVsdc` and the SAME number is
//!    retried. A retry answered `994` ("There is an overlapped Data") means
//!    the VSDC already holds that invoice, so the number must NEVER be
//!    reallocated. But 994 is NOT a path to `Confirmed`: it carries no
//!    receipt, and `saveSales` is the only sales endpoint (the one "select"
//!    endpoint is the buyer-side purchases lookup), so there is no documented
//!    way to re-read a receipt already issued. The sale stays unconfirmed and
//!    needs reconciliation: an operator must obtain the receipt from the
//!    VSDC's own records (Phase 3 builds that). Phase 3 must therefore
//!    minimise ambiguous retries: retrying after a timeout-post-send is
//!    exactly what produces a 994.
//! 3. While an `AwaitingVsdc` sale exists in a series no new number is issued
//!    ([`StoreError::SequenceBlocked`]); [`Store::pending_sale`] finds it at
//!    startup.
//!
//! 4. A sale that can never complete is retired with [`Store::abandon_sale`]
//!    (`Voided`, number never reused), which is the only way out of rule 3.
//!
//! # Counters are per DEVICE and per receipt type
//!
//! The VSDC keeps its invoice counters per device (`dvcSrlNo`), seven of them,
//! one per receipt type: `vsdc.txt` 3.3.1.1 returns them under `InitDevice`
//! in the `selectInitInfo` response, and `InitBranch` carries none. RRA also
//! requires "an uninterrupted ascending number series per receipt type"
//! (`cis.txt` 7.3). So `invc_seq` is keyed on `(dvc_srl_no, counter)` with a
//! [`Counter`] naming one of the seven series. `bhfId` is issued per branch, so
//! every lane in a shop legitimately shares it; it is NOT part of the key, and
//! the lanes cannot collide because each has its own device serial.
//!
//! # Recovery
//!
//! Allocation is local and never asks the VSDC. A till whose database was
//! lost would therefore restart at 1 and have every sale refused with `994`.
//! [`Store::reconcile_counter_from_vsdc`] resumes from the VSDC's own value.
//!
//! Every transaction here is `BEGIN IMMEDIATE`. SQLite's default `BEGIN` is
//! deferred: it reads first and only then tries to upgrade to a writer, and in
//! WAL mode that upgrade fails at once (`SQLITE_BUSY_SNAPSHOT`) if another
//! connection committed in between. `BEGIN IMMEDIATE` takes the write lock up
//! front, so contenders queue on the busy timeout instead of failing.

use sqlx::{Sqlite, SqliteConnection, Transaction};

use crate::schema::{Store, StoreError};

/// One of the seven invoice counters the VSDC keeps per device. The variants
/// map one-to-one onto the `selectInitInfo` fields (`vsdc.txt` 3.3.1.1):
/// `lastInvcNo`, `lastSaleInvcNo`, `lastSaleRcptNo`, `lastPchsInvcNo`,
/// `lastTrainInvcNo`, `lastProfrmInvcNo`, `lastCopyInvcNo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Counter {
    Invc,
    SaleInvc,
    SaleRcpt,
    PchsInvc,
    TrainInvc,
    ProfrmInvc,
    CopyInvc,
}

impl Counter {
    /// The value stored in `invc_seq.counter` (checked by the schema).
    pub fn as_str(self) -> &'static str {
        match self {
            Counter::Invc => "Invc",
            Counter::SaleInvc => "SaleInvc",
            Counter::SaleRcpt => "SaleRcpt",
            Counter::PchsInvc => "PchsInvc",
            Counter::TrainInvc => "TrainInvc",
            Counter::ProfrmInvc => "ProfrmInvc",
            Counter::CopyInvc => "CopyInvc",
        }
    }
}

/// A write transaction opened with `BEGIN IMMEDIATE`, and the only kind
/// [`allocate_on`] accepts.
///
/// The field is private to this module and the only constructor is
/// [`Store::begin_immediate`], so a deferred transaction or a bare pooled
/// connection cannot reach `allocate_on`: the wrong call does not compile.
/// Dropping it without `commit` rolls back.
pub(crate) struct ImmediateTx(Transaction<'static, Sqlite>);

impl ImmediateTx {
    /// The connection inside the transaction, for the caller's own statements
    /// (the sale, its lines and tenders) so they share the allocation's fate.
    pub(crate) fn conn(&mut self) -> &mut SqliteConnection {
        &mut self.0
    }

    pub(crate) async fn commit(self) -> Result<(), StoreError> {
        Ok(self.0.commit().await?)
    }

    pub(crate) async fn rollback(self) -> Result<(), StoreError> {
        Ok(self.0.rollback().await?)
    }
}

/// A sale holding an invoice number that the VSDC has not yet confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSale {
    pub id: String,
    pub invc_no: i64,
}

impl Store {
    /// Starts a write transaction that holds SQLite's write lock from the
    /// first statement. Anything that calls [`allocate_on`] must use this.
    /// NEVER use `pool().begin()` for an allocation: it issues a deferred
    /// `BEGIN`, which reintroduces the read-then-upgrade race and can collide
    /// or fail under concurrency.
    pub(crate) async fn begin_immediate(&self) -> Result<ImmediateTx, StoreError> {
        Ok(ImmediateTx(
            self.pool().begin_with("BEGIN IMMEDIATE").await?,
        ))
    }

    /// Allocates the next invoice number for `(dvc_srl_no, counter)` in its own transaction.
    ///
    /// Caution: this commits the bump by itself, so a number allocated here and
    /// not followed by a sale insert is a gap. It is crate-private and has no
    /// production caller: production code allocates inside the sale transaction
    /// via [`allocate_on`]. Kept for tests, hence the dead-code allowance.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn allocate_invc_no(
        &self,
        dvc_srl_no: &str,
        counter: Counter,
    ) -> Result<i64, StoreError> {
        let mut tx = self.begin_immediate().await?;
        let n = allocate_on(&mut tx, dvc_srl_no, counter).await?;
        tx.commit().await?;
        Ok(n)
    }

    /// The oldest `AwaitingVsdc` sale in the series, if any: what to resume at
    /// startup. Normally there is at most one (allocation refuses while one
    /// exists), but the schema does not forbid several, so the lowest number
    /// is returned for a deterministic, in-order resume.
    pub async fn pending_sale(
        &self,
        dvc_srl_no: &str,
        counter: Counter,
    ) -> Result<Option<PendingSale>, StoreError> {
        pending_on(self.pool(), dvc_srl_no, counter).await
    }

    /// Sets one of the device's counters to the VSDC's own value, so a
    /// reinstalled till, or one whose SQLite was lost, resumes from the VSDC's
    /// number instead of from 1 (which would reissue numbers the VSDC holds and
    /// have every sale refused `994`, an unrecoverable answer: it carries no
    /// receipt).
    ///
    /// `last_invc_no` MUST be the matching `lastXxxInvcNo` value from the
    /// `selectInitInfo` response (`vsdc.txt` 3.3.1.1, under `InitDevice`) and
    /// nothing else. Phase 3's initialisation sequence owns calling it; the
    /// store cannot verify the origin of the number. A `null` from the VSDC
    /// means "none issued" and is passed as 0.
    ///
    /// Monotonic only: a value below the stored counter is refused with
    /// [`StoreError::CounterWouldDecrease`], because lowering it would reissue
    /// numbers this till has already recorded. An absent counter is created;
    /// an equal value is a no-op.
    pub async fn reconcile_counter_from_vsdc(
        &self,
        dvc_srl_no: &str,
        counter: Counter,
        last_invc_no: i64,
    ) -> Result<(), StoreError> {
        if last_invc_no < 0 {
            return Err(StoreError::CounterOutOfRange {
                value: last_invc_no,
            });
        }
        let mut tx = self.begin_immediate().await?;
        let current: Option<i64> = sqlx::query_scalar(
            "select last_invc_no from invc_seq where dvc_srl_no = ? and counter = ?",
        )
        .bind(dvc_srl_no)
        .bind(counter.as_str())
        .fetch_optional(tx.conn())
        .await?;
        if let Some(current) = current {
            if last_invc_no < current {
                let _ = tx.rollback().await;
                return Err(StoreError::CounterWouldDecrease {
                    counter: counter.as_str(),
                    current,
                    requested: last_invc_no,
                });
            }
        }
        sqlx::query(
            "insert into invc_seq (dvc_srl_no, counter, last_invc_no) values (?, ?, ?) \
             on conflict (dvc_srl_no, counter) do update set last_invc_no = excluded.last_invc_no",
        )
        .bind(dvc_srl_no)
        .bind(counter.as_str())
        .bind(last_invc_no)
        .execute(tx.conn())
        .await?;
        tx.commit().await
    }
}

async fn pending_on<'e, E>(
    exec: E,
    dvc_srl_no: &str,
    counter: Counter,
) -> Result<Option<PendingSale>, StoreError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row: Option<(String, i64)> = sqlx::query_as(
        "select id, invc_no from sale where dvc_srl_no = ? and counter = ? \
         and state = 'AwaitingVsdc' \
         order by invc_no limit 1",
    )
    .bind(dvc_srl_no)
    .bind(counter.as_str())
    .fetch_optional(exec)
    .await?;
    Ok(row.map(|(id, invc_no)| PendingSale { id, invc_no }))
}

/// Issues the next number in the `(dvc_srl_no, counter)` series inside `tx`,
/// which the type guarantees is a `BEGIN IMMEDIATE` transaction; the caller
/// commits. Creates the series' `invc_seq` row on first use.
///
/// Refuses with [`StoreError::SequenceBlocked`] while an `AwaitingVsdc` sale
/// exists in the series. Series are independent: only this device's sales and
/// this counter's row are consulted.
pub(crate) async fn allocate_on(
    tx: &mut ImmediateTx,
    dvc_srl_no: &str,
    counter: Counter,
) -> Result<i64, StoreError> {
    let conn = tx.conn();
    if let Some(p) = pending_on(&mut *conn, dvc_srl_no, counter).await? {
        return Err(StoreError::SequenceBlocked { invc_no: p.invc_no });
    }
    let n: i64 = sqlx::query_scalar(
        "insert into invc_seq (dvc_srl_no, counter, last_invc_no) values (?, ?, 1) \
         on conflict (dvc_srl_no, counter) do update set last_invc_no = last_invc_no + 1 \
         returning last_invc_no",
    )
    .bind(dvc_srl_no)
    .bind(counter.as_str())
    .fetch_one(&mut *conn)
    .await?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const NOW: &str = "2026-11-21T00:00:00Z";

    const A: &str = "dvc-A";
    const B: &str = "dvc-B";
    const SALE: Counter = Counter::SaleInvc;

    async fn insert_awaiting(s: &Store, id: &str, dvc: &str, counter: Counter, no: i64) {
        sqlx::query(
            "insert into sale (id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) \
             values (?, ?, ?, '00', ?, 'AwaitingVsdc','0','0','0','01',?)",
        )
        .bind(id)
        .bind(dvc)
        .bind(counter.as_str())
        .bind(no)
        .bind(NOW)
        .execute(s.pool())
        .await
        .unwrap();
    }

    async fn set_state(s: &Store, id: &str, state: &str) {
        sqlx::query("update sale set state = ? where id = ?")
            .bind(state)
            .bind(id)
            .execute(s.pool())
            .await
            .unwrap();
    }

    /// A file-backed store: unlike `open_in_memory` (pinned to one connection,
    /// which serialises everything in the pool) this has several real
    /// connections, so a concurrency test can actually race.
    async fn file_store(tag: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("till-seq-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Store::open(&dir.join("till.db")).await.unwrap();
        (s, dir)
    }

    #[tokio::test]
    async fn the_first_number_in_a_series_is_one() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn numbers_ascend_without_gaps_and_per_device() {
        let s = Store::open_in_memory().await.unwrap();
        for expected in 1..=5 {
            assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), expected);
        }
        assert_eq!(s.allocate_invc_no(B, SALE).await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), 6);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_allocations_never_collide() {
        // File-backed, multi-connection, multi-thread runtime: with a plain
        // BEGIN this fails (SQLITE_BUSY_SNAPSHOT on the read-to-write upgrade).
        let (s, dir) = file_store("conc").await;
        let s = Arc::new(s);
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let s = s.clone();
            set.spawn(async move { s.allocate_invc_no(A, SALE).await.unwrap() });
        }
        let mut got = Vec::new();
        while let Some(r) = set.join_next().await {
            got.push(r.unwrap());
        }
        got.sort_unstable();
        assert_eq!(
            got,
            (1..=20).collect::<Vec<_>>(),
            "every number once, no gaps"
        );
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_allocations_on_one_in_memory_pool_also_hold() {
        let s = Arc::new(Store::open_in_memory().await.unwrap());
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let s = s.clone();
            set.spawn(async move { s.allocate_invc_no(A, SALE).await.unwrap() });
        }
        let mut got = Vec::new();
        while let Some(r) = set.join_next().await {
            got.push(r.unwrap());
        }
        got.sort_unstable();
        assert_eq!(got, (1..=20).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_devices_allocating_concurrently_each_stay_gapless() {
        let (s, dir) = file_store("two").await;
        let s = Arc::new(s);
        let mut set = tokio::task::JoinSet::new();
        for i in 0..20 {
            let s = s.clone();
            let dvc = if i % 2 == 0 { A } else { B };
            set.spawn(async move { (dvc, s.allocate_invc_no(dvc, SALE).await.unwrap()) });
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        while let Some(r) = set.join_next().await {
            let (dvc, n) = r.unwrap();
            if dvc == A { a.push(n) } else { b.push(n) }
        }
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, (1..=10).collect::<Vec<_>>());
        assert_eq!(b, (1..=10).collect::<Vec<_>>());
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_unresolved_sale_blocks_the_next_allocation() {
        let s = Store::open_in_memory().await.unwrap();
        let n = s.allocate_invc_no(A, SALE).await.unwrap();
        insert_awaiting(&s, "s1", A, SALE, n).await;
        match s.allocate_invc_no(A, SALE).await {
            Err(StoreError::SequenceBlocked { invc_no }) => assert_eq!(invc_no, n),
            other => panic!("expected SequenceBlocked, got {other:?}"),
        }
        assert_eq!(s.pending_sale(A, SALE).await.unwrap().unwrap().invc_no, n);
        // the refused attempt must not have bumped the counter
        set_state(&s, "s1", "Confirmed").await;
        assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), n + 1);
    }

    #[tokio::test]
    async fn a_blocked_device_does_not_block_another() {
        let s = Store::open_in_memory().await.unwrap();
        let n = s.allocate_invc_no(A, SALE).await.unwrap();
        insert_awaiting(&s, "s1", A, SALE, n).await;
        assert_eq!(s.allocate_invc_no(B, SALE).await.unwrap(), 1);
        assert!(s.pending_sale(B, SALE).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn only_an_awaiting_sale_blocks() {
        let s = Store::open_in_memory().await.unwrap();
        let n = s.allocate_invc_no(A, SALE).await.unwrap();
        insert_awaiting(&s, "s1", A, SALE, n).await;
        for state in ["Confirmed", "Failed", "Voided"] {
            set_state(&s, "s1", state).await;
            assert!(s.pending_sale(A, SALE).await.unwrap().is_none(), "{state}");
            let next = s.allocate_invc_no(A, SALE).await.unwrap();
            // keep the sequence honest for the next loop iteration
            insert_awaiting(&s, &format!("x{next}"), A, SALE, next).await;
            set_state(&s, &format!("x{next}"), "Confirmed").await;
        }
    }

    #[tokio::test]
    async fn pending_sale_returns_the_lowest_of_several() {
        let s = Store::open_in_memory().await.unwrap();
        s.allocate_invc_no(A, SALE).await.unwrap();
        s.allocate_invc_no(A, SALE).await.unwrap();
        insert_awaiting(&s, "b", A, SALE, 2).await;
        insert_awaiting(&s, "a", A, SALE, 1).await;
        let p = s.pending_sale(A, SALE).await.unwrap().unwrap();
        assert_eq!((p.id.as_str(), p.invc_no), ("a", 1));
    }

    #[tokio::test]
    async fn allocation_and_sale_insert_commit_or_roll_back_together() {
        let s = Store::open_in_memory().await.unwrap();
        // rolled back: neither the number nor the sale survives
        let mut tx = s.begin_immediate().await.unwrap();
        let n = allocate_on(&mut tx, A, SALE).await.unwrap();
        assert_eq!(n, 1);
        tx.rollback().await.unwrap();
        assert!(s.pending_sale(A, SALE).await.unwrap().is_none());
        assert_eq!(
            s.allocate_invc_no(A, SALE).await.unwrap(),
            1,
            "number not burned"
        );
        // dropped without commit (crash / cancelled future) behaves the same
        {
            let mut tx = s.begin_immediate().await.unwrap();
            assert_eq!(allocate_on(&mut tx, A, SALE).await.unwrap(), 2);
        }
        assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn each_receipt_type_has_its_own_gapless_series() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.allocate_invc_no(A, Counter::SaleInvc).await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no(A, Counter::SaleInvc).await.unwrap(), 2);
        // a Training receipt starts its own series and does not touch Sale
        assert_eq!(s.allocate_invc_no(A, Counter::TrainInvc).await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no(A, Counter::ProfrmInvc).await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no(A, Counter::CopyInvc).await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no(A, Counter::SaleInvc).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn an_unresolved_sale_blocks_only_its_own_counter() {
        let s = Store::open_in_memory().await.unwrap();
        let n = s.allocate_invc_no(A, SALE).await.unwrap();
        insert_awaiting(&s, "s1", A, SALE, n).await;
        assert!(matches!(
            s.allocate_invc_no(A, SALE).await,
            Err(StoreError::SequenceBlocked { .. })
        ));
        assert_eq!(s.allocate_invc_no(A, Counter::TrainInvc).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_same_number_may_exist_on_two_devices_of_one_branch() {
        // both lanes share bhfId "00" (insert_awaiting hardcodes it) and each
        // holds invoice 1: legitimate, since the VSDC counts per device.
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no(B, SALE).await.unwrap(), 1);
        insert_awaiting(&s, "a1", A, SALE, 1).await;
        insert_awaiting(&s, "b1", B, SALE, 1).await;
    }

    async fn counter_value(s: &Store, dvc: &str, c: Counter) -> Option<i64> {
        sqlx::query_scalar("select last_invc_no from invc_seq where dvc_srl_no = ? and counter = ?")
            .bind(dvc)
            .bind(c.as_str())
            .fetch_optional(s.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reconcile_raises_a_counter_and_allocation_resumes_after_it() {
        let s = Store::open_in_memory().await.unwrap();
        s.allocate_invc_no(A, SALE).await.unwrap();
        s.reconcile_counter_from_vsdc(A, SALE, 41).await.unwrap();
        assert_eq!(counter_value(&s, A, SALE).await, Some(41));
        assert_eq!(s.allocate_invc_no(A, SALE).await.unwrap(), 42);
        // equal is a no-op, not an error
        s.reconcile_counter_from_vsdc(A, SALE, 42).await.unwrap();
        assert_eq!(counter_value(&s, A, SALE).await, Some(42));
    }

    #[tokio::test]
    async fn reconcile_refuses_to_lower_a_counter_by_name() {
        let s = Store::open_in_memory().await.unwrap();
        s.reconcile_counter_from_vsdc(A, SALE, 10).await.unwrap();
        match s.reconcile_counter_from_vsdc(A, SALE, 9).await {
            Err(StoreError::CounterWouldDecrease {
                counter: "SaleInvc",
                current: 10,
                requested: 9,
            }) => {}
            other => panic!("expected CounterWouldDecrease, got {other:?}"),
        }
        assert_eq!(counter_value(&s, A, SALE).await, Some(10), "untouched");
    }

    #[tokio::test]
    async fn reconcile_creates_an_absent_counter_and_touches_no_other() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(counter_value(&s, A, Counter::CopyInvc).await, None);
        s.reconcile_counter_from_vsdc(A, Counter::CopyInvc, 7)
            .await
            .unwrap();
        assert_eq!(counter_value(&s, A, Counter::CopyInvc).await, Some(7));
        assert_eq!(counter_value(&s, A, SALE).await, None);
        assert_eq!(counter_value(&s, B, Counter::CopyInvc).await, None);
        s.reconcile_counter_from_vsdc(A, Counter::TrainInvc, 0)
            .await
            .unwrap();
        assert_eq!(counter_value(&s, A, Counter::TrainInvc).await, Some(0));
    }

    #[tokio::test]
    async fn reconcile_refuses_a_negative_value() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(matches!(
            s.reconcile_counter_from_vsdc(A, SALE, -1).await,
            Err(StoreError::CounterOutOfRange { value: -1 })
        ));
        assert_eq!(counter_value(&s, A, SALE).await, None);
    }

    #[tokio::test]
    async fn every_counter_name_is_accepted_by_the_schema_and_a_stranger_is_not() {
        let s = Store::open_in_memory().await.unwrap();
        for c in [
            Counter::Invc,
            Counter::SaleInvc,
            Counter::SaleRcpt,
            Counter::PchsInvc,
            Counter::TrainInvc,
            Counter::ProfrmInvc,
            Counter::CopyInvc,
        ] {
            s.reconcile_counter_from_vsdc(A, c, 1).await.unwrap();
        }
        let r = sqlx::query("insert into invc_seq values ('x', 'Bogus', 0)")
            .execute(s.pool())
            .await;
        assert!(r.is_err());
    }
}
