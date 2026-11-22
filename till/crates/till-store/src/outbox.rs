//! The outbound queue as the cloud sync sees it: what is waiting, what was
//! delivered, what failed, and what the cloud refused for good.
//!
//! The queue is strictly ordered by `id`. A row is `sent` only when the cloud
//! answered 2xx (the caller decides that; this module just records it). A row
//! the cloud refused permanently is `parked`: kept, readable, never lost, but
//! no longer offered, and neither is anything that depends on it.

use sqlx::Row;

use crate::schema::{Store, StoreError};

const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%SZ','now')";

/// One row waiting for the cloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub id: i64,
    /// `"sale"` or `"shift"`.
    pub kind: String,
    /// The exact JSON text to POST, as stored when the row was queued.
    pub payload: String,
    pub attempts: i64,
}

/// What one pass of the drain did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainOutcome {
    /// Nothing was waiting.
    Idle,
    /// This many rows were delivered and nothing was refused.
    Sent(u32),
    /// The cloud refused something retrying cannot fix. `id` is the row (the
    /// first one refused, or the oldest waiting row when the lane token was
    /// rejected and the whole queue is held).
    Parked { id: i64, reason: String },
    /// The cloud could not be reached, or failed in a way worth retrying.
    Offline,
}

impl Store {
    /// Rows waiting for delivery, oldest first, at most `limit`.
    ///
    /// Excludes rows already sent, rows parked, and every row that belongs to a
    /// parked shift OPEN: a sale (or the close) of a shift the cloud never
    /// accepted would only be refused in turn, and `Sale.shiftId` must resolve.
    /// A parked shift CLOSE (payload carries `closedAt`) withholds nothing: the
    /// shift already exists in the cloud, so its sales land fine. Other shifts'
    /// rows are unaffected.
    pub async fn unsent(&self, limit: u32) -> Result<Vec<OutboxRow>, StoreError> {
        let rows = sqlx::query(
            "select o.id, o.kind, o.payload, o.attempts from outbound o \
             where o.sent_at is null and o.parked_at is null \
             and not exists ( \
                 select 1 from outbound p \
                 where p.kind = 'shift' and p.parked_at is not null and p.sent_at is null \
                 and json_valid(p.payload) and json_valid(o.payload) \
                 and json_extract(p.payload, '$.closedAt') is null \
                 and json_extract(p.payload, '$.id') = \
                     case o.kind when 'sale' then json_extract(o.payload, '$.shiftId') \
                                 else json_extract(o.payload, '$.id') end) \
             order by o.id limit ?",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(OutboxRow {
                    id: r.try_get("id")?,
                    kind: r.try_get("kind")?,
                    payload: r.try_get("payload")?,
                    attempts: r.try_get("attempts")?,
                })
            })
            .collect()
    }

    /// Marks a row delivered. Idempotent: the first delivery time stands.
    pub async fn mark_sent(&self, id: i64) -> Result<(), StoreError> {
        sqlx::query(&format!(
            "update outbound set sent_at = {NOW_SQL}, last_error = null \
             where id = ? and sent_at is null"
        ))
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Records a failed attempt: bumps `attempts`, keeps the row unsent. The
    /// text is stored verbatim, so callers must never pass anything secret.
    pub async fn record_failure(&self, id: i64, error: &str) -> Result<(), StoreError> {
        sqlx::query(
            "update outbound set attempts = attempts + 1, last_error = ? \
             where id = ? and sent_at is null",
        )
        .bind(error)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Parks a row the cloud refused for good. It is kept, with `error` as its
    /// `last_error`, and is no longer offered by [`Store::unsent`].
    pub async fn park(&self, id: i64, error: &str) -> Result<(), StoreError> {
        sqlx::query(&format!(
            "update outbound set attempts = attempts + 1, last_error = ?, \
             parked_at = coalesce(parked_at, {NOW_SQL}) \
             where id = ? and sent_at is null"
        ))
        .bind(error)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// How many rows have not been delivered (parked ones included: they are
    /// still not in the cloud) and the oldest one's `created_at`.
    pub async fn queue_depth(&self) -> Result<(u32, Option<String>), StoreError> {
        let row = sqlx::query(
            "select count(*) as n, min(created_at) as oldest from outbound where sent_at is null",
        )
        .fetch_one(self.pool())
        .await?;
        let n: i64 = row.try_get("n")?;
        Ok((u32::try_from(n).unwrap_or(u32::MAX), row.try_get("oldest")?))
    }

    /// How many undelivered rows the cloud refused for good.
    pub async fn parked_count(&self) -> Result<u32, StoreError> {
        let n: i64 = sqlx::query_scalar(
            "select count(*) from outbound where sent_at is null and parked_at is not null",
        )
        .fetch_one(self.pool())
        .await?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn queue(s: &Store, kind: &str, payload: &str, at: &str) -> i64 {
        sqlx::query_scalar(
            "insert into outbound (kind, payload, created_at) values (?, ?, ?) returning id",
        )
        .bind(kind)
        .bind(payload)
        .bind(at)
        .fetch_one(s.pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn unsent_returns_rows_in_id_order_and_excludes_sent() {
        let s = Store::open_in_memory().await.unwrap();
        let a = queue(&s, "shift", "{}", "2026-11-22T08:00:00Z").await;
        let b = queue(&s, "sale_x", "{}", "2026-11-22T08:01:00Z").await;
        let c = queue(&s, "shift", "{}", "2026-11-22T08:02:00Z").await;
        s.mark_sent(b).await.unwrap();
        let ids: Vec<i64> = s.unsent(10).await.unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![a, c]);
        assert_eq!(s.unsent(1).await.unwrap().len(), 1, "the limit is honoured");
    }

    #[tokio::test]
    async fn mark_sent_is_idempotent() {
        let s = Store::open_in_memory().await.unwrap();
        let a = queue(&s, "shift", "{}", "2026-11-22T08:00:00Z").await;
        s.mark_sent(a).await.unwrap();
        let first: String = sqlx::query_scalar("select sent_at from outbound where id = ?")
            .bind(a)
            .fetch_one(s.pool())
            .await
            .unwrap();
        s.mark_sent(a).await.unwrap();
        s.mark_sent(a).await.unwrap();
        let again: String = sqlx::query_scalar("select sent_at from outbound where id = ?")
            .bind(a)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(first, again);
        assert!(s.unsent(10).await.unwrap().is_empty());
        // Marking a row that does not exist is not an error either.
        s.mark_sent(9999).await.unwrap();
    }

    #[tokio::test]
    async fn record_failure_increments_attempts_and_keeps_the_row_unsent() {
        let s = Store::open_in_memory().await.unwrap();
        let a = queue(&s, "shift", "{}", "2026-11-22T08:00:00Z").await;
        s.record_failure(a, "HTTP 503").await.unwrap();
        s.record_failure(a, "HTTP 502").await.unwrap();
        let rows = s.unsent(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].attempts, 2);
        let err: Option<String> = sqlx::query_scalar("select last_error from outbound")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(err.as_deref(), Some("HTTP 502"));
    }

    #[tokio::test]
    async fn queue_depth_reports_the_count_and_the_oldest_unsent() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.queue_depth().await.unwrap(), (0, None));
        let a = queue(&s, "shift", "{}", "2026-11-22T08:00:00Z").await;
        queue(&s, "shift", "{}", "2026-11-22T08:05:00Z").await;
        queue(&s, "shift", "{}", "2026-11-22T08:09:00Z").await;
        assert_eq!(
            s.queue_depth().await.unwrap(),
            (3, Some("2026-11-22T08:00:00Z".into()))
        );
        s.mark_sent(a).await.unwrap();
        assert_eq!(
            s.queue_depth().await.unwrap(),
            (2, Some("2026-11-22T08:05:00Z".into()))
        );
    }

    #[tokio::test]
    async fn a_parked_row_is_kept_but_not_offered_and_still_counts_as_undelivered() {
        let s = Store::open_in_memory().await.unwrap();
        let a = queue(&s, "sale", "{}", "2026-11-22T08:00:00Z").await;
        let b = queue(&s, "sale", "{}", "2026-11-22T08:01:00Z").await;
        s.park(a, "HTTP 422: nope").await.unwrap();
        let ids: Vec<i64> = s.unsent(10).await.unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![b]);
        assert_eq!(s.queue_depth().await.unwrap().0, 2);
        assert_eq!(s.parked_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_parked_shift_hides_its_own_sales_and_close_but_not_other_shifts() {
        let s = Store::open_in_memory().await.unwrap();
        let open = queue(&s, "shift", r#"{"id":"s1"}"#, "2026-11-22T08:00:00Z").await;
        queue(&s, "sale", r#"{"shiftId":"s1"}"#, "2026-11-22T08:01:00Z").await;
        queue(
            &s,
            "shift",
            r#"{"id":"s1","closedAt":"x"}"#,
            "2026-11-22T08:02:00Z",
        )
        .await;
        let other = queue(&s, "shift", r#"{"id":"s2"}"#, "2026-11-22T08:03:00Z").await;
        let other_sale = queue(&s, "sale", r#"{"shiftId":"s2"}"#, "2026-11-22T08:04:00Z").await;
        s.park(open, "Unknown cashier").await.unwrap();
        let ids: Vec<i64> = s.unsent(10).await.unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![other, other_sale]);
    }

    #[tokio::test]
    async fn a_parked_shift_close_does_not_withhold_the_shifts_sales_but_a_parked_open_does() {
        let s = Store::open_in_memory().await.unwrap();
        // Close parked: the open was delivered, so the sale must still flow.
        let open = queue(&s, "shift", r#"{"id":"s1"}"#, "2026-11-22T08:00:00Z").await;
        s.mark_sent(open).await.unwrap();
        let close = queue(
            &s,
            "shift",
            r#"{"id":"s1","closedAt":"x"}"#,
            "2026-11-22T08:02:00Z",
        )
        .await;
        let sale = queue(&s, "sale", r#"{"shiftId":"s1"}"#, "2026-11-22T08:03:00Z").await;
        s.park(close, "HTTP 422").await.unwrap();
        let ids: Vec<i64> = s.unsent(10).await.unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![sale]);

        // Open parked: the cloud has never seen shift s2, so its sales wait.
        let open2 = queue(&s, "shift", r#"{"id":"s2"}"#, "2026-11-22T08:04:00Z").await;
        queue(&s, "sale", r#"{"shiftId":"s2"}"#, "2026-11-22T08:05:00Z").await;
        s.park(open2, "Unknown cashier").await.unwrap();
        let ids: Vec<i64> = s.unsent(10).await.unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![sale]);
    }
}
