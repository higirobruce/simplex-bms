//! A cashier's shift on one device. The cloud's `Sale.shiftId` is non-null, so
//! a sale can only be recorded (and later synced) inside an open shift.

use till_core::Money;

use crate::schema::{Store, StoreError, decode_money};

const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%SZ','now')";

/// A shift that has not been closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShift {
    pub id: String,
    pub opened_by: String,
    pub opened_at: String,
    pub opening_float: Money,
}

type Row = (String, String, String, String);

fn decode(r: Row) -> Result<OpenShift, StoreError> {
    Ok(OpenShift {
        id: r.0,
        opened_by: r.1,
        opened_at: r.2,
        opening_float: decode_money("opening_float", &r.3)?,
    })
}

impl Store {
    /// Opens a shift on `dvc_srl_no`. Refused with `ShiftAlreadyOpen` if the
    /// device has one: checked under `BEGIN IMMEDIATE`, and backed by the
    /// partial unique index `shift_one_open_per_device`, so two racing
    /// connections cannot both succeed. Other devices are unaffected.
    pub async fn open_shift(
        &self,
        dvc_srl_no: &str,
        opened_by: &str,
        opening_float: Money,
    ) -> Result<OpenShift, StoreError> {
        if opening_float < Money::zero() {
            return Err(StoreError::NegativeOpeningFloat);
        }
        let mut tx = self.begin_immediate().await?;
        let written = async {
            let conn = tx.conn();
            let open: i64 = sqlx::query_scalar(
                "select count(*) from shift where dvc_srl_no = ? and closed_at is null",
            )
            .bind(dvc_srl_no)
            .fetch_one(&mut *conn)
            .await?;
            if open > 0 {
                return Err(StoreError::ShiftAlreadyOpen {
                    dvc_srl_no: dvc_srl_no.to_string(),
                });
            }
            let id: String = sqlx::query_scalar("select lower(hex(randomblob(16)))")
                .fetch_one(&mut *conn)
                .await?;
            let row: Row = sqlx::query_as(&format!(
                "insert into shift (id, dvc_srl_no, opened_by, opened_at, opening_float) \
                 values (?, ?, ?, {NOW_SQL}, ?) \
                 returning id, opened_by, opened_at, opening_float"
            ))
            .bind(&id)
            .bind(dvc_srl_no)
            .bind(opened_by)
            .bind(opening_float.to_string())
            .fetch_one(&mut *conn)
            .await?;
            decode(row)
        }
        .await;
        match written {
            Ok(shift) => {
                tx.commit().await?;
                Ok(shift)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// The device's open shift, if any.
    pub async fn current_shift(&self, dvc_srl_no: &str) -> Result<Option<OpenShift>, StoreError> {
        let row: Option<Row> = sqlx::query_as(
            "select id, opened_by, opened_at, opening_float from shift \
             where dvc_srl_no = ? and closed_at is null",
        )
        .bind(dvc_srl_no)
        .fetch_optional(self.pool())
        .await?;
        row.map(decode).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    #[tokio::test]
    async fn an_opened_shift_is_the_current_one() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.current_shift("d1").await.unwrap(), None);
        let o = s.open_shift("d1", "alice", m("5000.00")).await.unwrap();
        assert_eq!(o.opened_by, "alice");
        assert_eq!(o.opening_float, m("5000.00"));
        assert_eq!(s.current_shift("d1").await.unwrap(), Some(o));
    }

    #[tokio::test]
    async fn a_second_open_shift_on_one_device_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        let first = s.open_shift("d1", "alice", m("0")).await.unwrap();
        let r = s.open_shift("d1", "bob", m("0")).await;
        assert!(
            matches!(r, Err(StoreError::ShiftAlreadyOpen { ref dvc_srl_no }) if dvc_srl_no == "d1")
        );
        assert_eq!(s.current_shift("d1").await.unwrap(), Some(first));
    }

    #[tokio::test]
    async fn the_database_itself_refuses_a_second_open_shift() {
        // bypasses the application check: the index is what makes it true
        let s = Store::open_in_memory().await.unwrap();
        s.open_shift("d1", "alice", m("0")).await.unwrap();
        let r = sqlx::query("insert into shift (id, dvc_srl_no, opened_by, opened_at, opening_float) values ('x','d1','b','t','0.00')")
            .execute(s.pool())
            .await;
        assert!(r.unwrap_err().to_string().contains("UNIQUE"));
    }

    #[tokio::test]
    async fn two_devices_each_hold_their_own_open_shift() {
        let s = Store::open_in_memory().await.unwrap();
        let a = s.open_shift("d1", "alice", m("100")).await.unwrap();
        let b = s.open_shift("d2", "bob", m("200")).await.unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(s.current_shift("d1").await.unwrap(), Some(a));
        assert_eq!(s.current_shift("d2").await.unwrap(), Some(b));
    }

    #[tokio::test]
    async fn a_negative_opening_float_is_refused_and_nothing_is_opened() {
        let s = Store::open_in_memory().await.unwrap();
        let r = s.open_shift("d1", "alice", m("-1.00")).await;
        assert!(matches!(r, Err(StoreError::NegativeOpeningFloat)));
        assert_eq!(s.current_shift("d1").await.unwrap(), None);
        // zero is a legitimate float
        s.open_shift("d1", "alice", m("0")).await.unwrap();
    }

    #[tokio::test]
    async fn a_closed_shift_is_not_current_and_frees_the_device() {
        let s = Store::open_in_memory().await.unwrap();
        s.open_shift("d1", "alice", m("0")).await.unwrap();
        sqlx::query("update shift set closed_at = 't', declared_cash = '0.00'")
            .execute(s.pool())
            .await
            .unwrap();
        assert_eq!(s.current_shift("d1").await.unwrap(), None);
        s.open_shift("d1", "bob", m("0")).await.unwrap();
    }
}
