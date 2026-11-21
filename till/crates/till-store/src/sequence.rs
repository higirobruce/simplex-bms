//! The gapless per-branch invoice-number sequence (Ruling R38).
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
//!    the VSDC already holds that invoice: that is success, not an error.
//! 3. While an `AwaitingVsdc` sale exists on a branch no new number is issued
//!    ([`StoreError::SequenceBlocked`]); [`Store::pending_sale`] finds it at
//!    startup.
//!
//! Every transaction here is `BEGIN IMMEDIATE`. SQLite's default `BEGIN` is
//! deferred: it reads first and only then tries to upgrade to a writer, and in
//! WAL mode that upgrade fails at once (`SQLITE_BUSY_SNAPSHOT`) if another
//! connection committed in between. `BEGIN IMMEDIATE` takes the write lock up
//! front, so contenders queue on the busy timeout instead of failing.

use sqlx::{Sqlite, SqliteConnection, Transaction};

use crate::schema::{Store, StoreError};

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

    // Used by tests now and by the sale transaction (Task 12) next.
    #[cfg_attr(not(test), allow(dead_code))]
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

    /// Allocates the next invoice number for `bhf_id` in its own transaction.
    ///
    /// Caution: this commits the bump by itself, so a number allocated here and
    /// not followed by a sale insert is a gap. It is crate-private and has no
    /// production caller: production code allocates inside the sale transaction
    /// via [`allocate_on`]. Kept for tests, hence the dead-code allowance.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn allocate_invc_no(&self, bhf_id: &str) -> Result<i64, StoreError> {
        let mut tx = self.begin_immediate().await?;
        let n = allocate_on(&mut tx, bhf_id).await?;
        tx.commit().await?;
        Ok(n)
    }

    /// The oldest `AwaitingVsdc` sale on the branch, if any: what to resume at
    /// startup. Normally there is at most one (allocation refuses while one
    /// exists), but the schema does not forbid several, so the lowest number
    /// is returned for a deterministic, in-order resume.
    pub async fn pending_sale(&self, bhf_id: &str) -> Result<Option<PendingSale>, StoreError> {
        pending_on(self.pool(), bhf_id).await
    }
}

async fn pending_on<'e, E>(exec: E, bhf_id: &str) -> Result<Option<PendingSale>, StoreError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row: Option<(String, i64)> = sqlx::query_as(
        "select id, invc_no from sale where bhf_id = ? and state = 'AwaitingVsdc' \
         order by invc_no limit 1",
    )
    .bind(bhf_id)
    .fetch_optional(exec)
    .await?;
    Ok(row.map(|(id, invc_no)| PendingSale { id, invc_no }))
}

/// Issues the next number for `bhf_id` inside `tx`, which the type guarantees
/// is a `BEGIN IMMEDIATE` transaction; the caller commits. Creates the branch's `invc_seq` row on first use.
///
/// Refuses with [`StoreError::SequenceBlocked`] while an `AwaitingVsdc` sale
/// exists on the branch. Branches are independent: only `bhf_id`'s own sales
/// and sequence row are consulted.
pub(crate) async fn allocate_on(tx: &mut ImmediateTx, bhf_id: &str) -> Result<i64, StoreError> {
    let conn = tx.conn();
    if let Some(p) = pending_on(&mut *conn, bhf_id).await? {
        return Err(StoreError::SequenceBlocked { invc_no: p.invc_no });
    }
    let n: i64 = sqlx::query_scalar(
        "insert into invc_seq (bhf_id, last_invc_no) values (?, 1) \
         on conflict (bhf_id) do update set last_invc_no = last_invc_no + 1 \
         returning last_invc_no",
    )
    .bind(bhf_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const NOW: &str = "2026-11-21T00:00:00Z";

    async fn insert_awaiting(s: &Store, id: &str, bhf: &str, no: i64) {
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
    async fn the_first_number_on_a_branch_is_one() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.allocate_invc_no("00").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn numbers_ascend_without_gaps_and_per_branch() {
        let s = Store::open_in_memory().await.unwrap();
        for expected in 1..=5 {
            assert_eq!(s.allocate_invc_no("00").await.unwrap(), expected);
        }
        assert_eq!(s.allocate_invc_no("01").await.unwrap(), 1);
        assert_eq!(s.allocate_invc_no("00").await.unwrap(), 6);
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
            set.spawn(async move { s.allocate_invc_no("00").await.unwrap() });
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
            set.spawn(async move { s.allocate_invc_no("00").await.unwrap() });
        }
        let mut got = Vec::new();
        while let Some(r) = set.join_next().await {
            got.push(r.unwrap());
        }
        got.sort_unstable();
        assert_eq!(got, (1..=20).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_branches_allocating_concurrently_each_stay_gapless() {
        let (s, dir) = file_store("two").await;
        let s = Arc::new(s);
        let mut set = tokio::task::JoinSet::new();
        for i in 0..20 {
            let s = s.clone();
            let bhf = if i % 2 == 0 { "00" } else { "01" };
            set.spawn(async move { (bhf, s.allocate_invc_no(bhf).await.unwrap()) });
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        while let Some(r) = set.join_next().await {
            let (bhf, n) = r.unwrap();
            if bhf == "00" { a.push(n) } else { b.push(n) }
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
        let n = s.allocate_invc_no("00").await.unwrap();
        insert_awaiting(&s, "s1", "00", n).await;
        match s.allocate_invc_no("00").await {
            Err(StoreError::SequenceBlocked { invc_no }) => assert_eq!(invc_no, n),
            other => panic!("expected SequenceBlocked, got {other:?}"),
        }
        assert_eq!(s.pending_sale("00").await.unwrap().unwrap().invc_no, n);
        // the refused attempt must not have bumped the counter
        set_state(&s, "s1", "Confirmed").await;
        assert_eq!(s.allocate_invc_no("00").await.unwrap(), n + 1);
    }

    #[tokio::test]
    async fn a_blocked_branch_does_not_block_another() {
        let s = Store::open_in_memory().await.unwrap();
        let n = s.allocate_invc_no("00").await.unwrap();
        insert_awaiting(&s, "s1", "00", n).await;
        assert_eq!(s.allocate_invc_no("01").await.unwrap(), 1);
        assert!(s.pending_sale("01").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn only_an_awaiting_sale_blocks() {
        let s = Store::open_in_memory().await.unwrap();
        let n = s.allocate_invc_no("00").await.unwrap();
        insert_awaiting(&s, "s1", "00", n).await;
        for state in ["Confirmed", "Failed", "Voided"] {
            set_state(&s, "s1", state).await;
            assert!(s.pending_sale("00").await.unwrap().is_none(), "{state}");
            let next = s.allocate_invc_no("00").await.unwrap();
            // keep the sequence honest for the next loop iteration
            insert_awaiting(&s, &format!("x{next}"), "00", next).await;
            set_state(&s, &format!("x{next}"), "Confirmed").await;
        }
    }

    #[tokio::test]
    async fn pending_sale_returns_the_lowest_of_several() {
        let s = Store::open_in_memory().await.unwrap();
        s.allocate_invc_no("00").await.unwrap();
        s.allocate_invc_no("00").await.unwrap();
        insert_awaiting(&s, "b", "00", 2).await;
        insert_awaiting(&s, "a", "00", 1).await;
        let p = s.pending_sale("00").await.unwrap().unwrap();
        assert_eq!((p.id.as_str(), p.invc_no), ("a", 1));
    }

    #[tokio::test]
    async fn allocation_and_sale_insert_commit_or_roll_back_together() {
        let s = Store::open_in_memory().await.unwrap();
        // rolled back: neither the number nor the sale survives
        let mut tx = s.begin_immediate().await.unwrap();
        let n = allocate_on(&mut tx, "00").await.unwrap();
        assert_eq!(n, 1);
        tx.rollback().await.unwrap();
        assert!(s.pending_sale("00").await.unwrap().is_none());
        assert_eq!(
            s.allocate_invc_no("00").await.unwrap(),
            1,
            "number not burned"
        );
        // dropped without commit (crash / cancelled future) behaves the same
        {
            let mut tx = s.begin_immediate().await.unwrap();
            assert_eq!(allocate_on(&mut tx, "00").await.unwrap(), 2);
        }
        assert_eq!(s.allocate_invc_no("00").await.unwrap(), 2);
    }
}
