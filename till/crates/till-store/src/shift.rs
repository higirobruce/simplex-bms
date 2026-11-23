//! A cashier's shift on one device. The cloud's `Sale.shiftId` is non-null, so
//! a sale can only be recorded (and later synced) inside an open shift.

use sqlx::SqliteConnection;
use till_core::Money;
use till_core::codes::PmtTyCd;

use crate::schema::{Store, StoreError, decode_money, decode_pmt_ty};

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

/// The one statement `shift_totals` runs. A const so the plan test checks
/// this exact text and cannot drift from it.
const TOTALS_SQL: &str = "select s.id, s.state, s.tot_amt, s.tot_tax_amt, t.pmt_ty_cd, t.amount \
             from sale s left join sale_tender t on t.sale_id = s.id \
             where s.shift_id = ?";

/// What a shift took: the figures a supervisor reconciles the drawer against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftTotals {
    /// Sales the VSDC confirmed. Only these are real sales.
    pub sale_count: u32,
    /// Sales abandoned before the VSDC confirmed them (state `Voided`). They
    /// took no money and are in no other figure.
    pub voided_count: u32,
    /// Sum of `tot_amt` over confirmed sales.
    pub gross: Money,
    /// Sum of `tot_tax_amt` over confirmed sales.
    pub tax: Money,
    /// Tendered amount per RRA payment code, ordered by wire code. Gross of
    /// change: a 5000 note against a 2380 sale shows 5000 under `01`.
    pub by_code: Vec<(PmtTyCd, Money)>,
    pub opening_float: Money,
    /// What should be physically in the drawer: opening float, plus code `01`
    /// tendered, minus change paid out. Code `03` is NOT cash.
    pub expected_cash: Money,
}

fn out_of_range(what: &str) -> StoreError {
    StoreError::Decode(format!("{what}: shift total exceeds the supported range"))
}

fn add(a: Money, b: Money, what: &str) -> Result<Money, StoreError> {
    a.checked_add(b).ok_or_else(|| out_of_range(what))
}

fn sub(a: Money, b: Money, what: &str) -> Result<Money, StoreError> {
    a.checked_sub(b).ok_or_else(|| out_of_range(what))
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
            // Queued in THIS transaction, so a shift is never open locally
            // yet unknown to the outbound queue. It is also queued before any
            // sale can exist in it, and the queue drains in id order, so the
            // cloud creates the Shift before the Sale that points at it.
            let payload = serde_json::to_string(&serde_json::json!({
                "id": row.0,
                "dvcSrlNo": dvc_srl_no,
                "openedById": row.1,
                "openedAt": row.2,
                "openingFloat": row.3,
            }))?;
            queue_shift(&mut *conn, payload).await?;
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

    /// What the shift took. Counts only `Confirmed` sales (`AwaitingVsdc` is
    /// unfinished and may yet be abandoned; `Voided` was abandoned; `Failed`
    /// is never written). `Voided` ones are counted in `voided_count` only.
    ///
    /// Change is not stored; it is each confirmed sale's tenders minus its
    /// `tot_amt`. `TenderSet::settle` only allows change up to the cash
    /// handed over, so it always leaves the drawer as cash. All arithmetic is
    /// exact `Money`; the single query filters on `sale.shift_id` (`sale_shift`) and reads
    /// state, totals and tenders in one statement, so it is one snapshot.
    pub async fn shift_totals(&self, shift_id: &str) -> Result<ShiftTotals, StoreError> {
        let mut conn = self.pool().acquire().await?;
        totals_on(&mut conn, shift_id).await
    }

    /// The device's most recently closed shift, recomposed from the store: the
    /// same [`ShiftClose`] that `close_shift` returned when it closed, so a close
    /// report lost to a jam and a restart can still be produced. Nothing is written.
    /// `None` if the device has never closed a shift.
    pub async fn last_closed_shift(
        &self,
        dvc_srl_no: &str,
    ) -> Result<Option<ClosedShift>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let row: Option<(String, String)> = sqlx::query_as(
            "select id, declared_cash from shift \
             where dvc_srl_no = ? and closed_at is not null and declared_cash is not null \
             order by closed_at desc, rowid desc limit 1",
        )
        .bind(dvc_srl_no)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((id, declared)) = row else {
            return Ok(None);
        };
        let declared_cash = decode_money("declared_cash", &declared)?;
        let totals = totals_on(&mut conn, &id).await?;
        let variance = sub(declared_cash, totals.expected_cash, "variance")?;
        Ok(Some(ClosedShift {
            id,
            close: ShiftClose {
                totals,
                declared_cash,
                variance,
            },
        }))
    }

    /// The MRC the VSDC reported on any confirmed sale of the shift, if there
    /// is one. A shift with no confirmed sale has none.
    pub async fn shift_mrc(&self, shift_id: &str) -> Result<Option<String>, StoreError> {
        let mrc: Option<(String,)> = sqlx::query_as(
            "select mrc_no from sale where shift_id = ? and mrc_no is not null \
             order by invc_no desc limit 1",
        )
        .bind(shift_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(mrc.map(|m| m.0))
    }

    /// Closes a shift: the cashier has counted the drawer and declares
    /// `declared_cash`. The variance is `declared - expected` and is NEVER
    /// refused: a short till (negative) closes like any other, because a till
    /// that will not close on a mismatch teaches cashiers to lie.
    ///
    /// Refused only with `ShiftAlreadyClosed` (a closed shift stays closed), `ShiftHasPendingSale`
    /// (an `AwaitingVsdc` sale holds a live invoice number that closing would
    /// orphan from its shift) and `ShiftNotFound`. All of it runs in one
    /// `BEGIN IMMEDIATE` transaction, the totals included, so what is recorded
    /// is exactly what the shift took at the instant it closed and no sale can
    /// confirm between the totals and the close.
    pub async fn close_shift(
        &self,
        shift_id: &str,
        declared_cash: Money,
    ) -> Result<ShiftClose, StoreError> {
        // A count cannot be below zero; a SHORT count is still fine (that is
        // the variance, not the count).
        if declared_cash < Money::zero() {
            return Err(StoreError::NegativeDeclaredCash);
        }
        let mut tx = self.begin_immediate().await?;
        let closed = async {
            let conn = tx.conn();
            let closed_at: Option<(Option<String>,)> =
                sqlx::query_as("select closed_at from shift where id = ?")
                    .bind(shift_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            match closed_at {
                None => {
                    return Err(StoreError::ShiftNotFound {
                        id: shift_id.to_string(),
                    });
                }
                Some((Some(_),)) => return Err(StoreError::ShiftAlreadyClosed),
                Some((None,)) => {}
            }
            let pending: i64 = sqlx::query_scalar(
                "select count(*) from sale where shift_id = ? and state = 'AwaitingVsdc'",
            )
            .bind(shift_id)
            .fetch_one(&mut *conn)
            .await?;
            if pending > 0 {
                return Err(StoreError::ShiftHasPendingSale);
            }
            let totals = totals_on(&mut *conn, shift_id).await?;
            // Signed on purpose: over and short are different problems.
            let variance = sub(declared_cash, totals.expected_cash, "variance")?;
            let row: (
                String,
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
            ) = sqlx::query_as(&format!(
                "update shift set closed_at = {NOW_SQL}, declared_cash = ? \
                     where id = ? and closed_at is null \
                     returning id, dvc_srl_no, opened_by, opened_at, opening_float, \
                     closed_at, declared_cash"
            ))
            .bind(declared_cash.to_string())
            .bind(shift_id)
            .fetch_one(&mut *conn)
            .await?;
            // The whole shift again, not a delta: the cloud upserts on `id`,
            // so the close is correct even if the open was never delivered.
            let payload = serde_json::to_string(&serde_json::json!({
                "id": row.0,
                "dvcSrlNo": row.1,
                "openedById": row.2,
                "openedAt": row.3,
                "openingFloat": row.4,
                "closedAt": row.5,
                "declaredCash": row.6,
                // The till's own tally, the same `totals` the cashier's close
                // report prints. The cloud sees only what arrived, so these
                // let it tell a complete shift from one missing a sale.
                // `gross` goes through `Display` (exact `{:.2}`), never a float.
                "saleCount": totals.sale_count,
                "gross": totals.gross.to_string(),
            }))?;
            queue_shift(&mut *conn, payload).await?;
            Ok(ShiftClose {
                totals,
                declared_cash,
                variance,
            })
        }
        .await;
        match closed {
            Ok(c) => {
                tx.commit().await?;
                Ok(c)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

/// Queues one `shift` row for the cloud. No `sale_id`: the row is about the
/// shift, which the payload names. Call only inside the transaction that
/// changed the shift.
async fn queue_shift(conn: &mut SqliteConnection, payload: String) -> Result<(), StoreError> {
    sqlx::query(&format!(
        "insert into outbound (kind, payload, created_at) values ('shift', ?, {NOW_SQL})"
    ))
    .bind(payload)
    .execute(conn)
    .await?;
    Ok(())
}

/// A closed shift and what closing it recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedShift {
    pub id: String,
    pub close: ShiftClose,
}

/// What closing a shift recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftClose {
    pub totals: ShiftTotals,
    pub declared_cash: Money,
    /// `declared_cash - totals.expected_cash`. Negative when the drawer is
    /// short, positive when it is over. Never clamped and never an absolute value.
    pub variance: Money,
}

async fn totals_on(conn: &mut SqliteConnection, shift_id: &str) -> Result<ShiftTotals, StoreError> {
    {
        let float: Option<String> =
            sqlx::query_scalar("select opening_float from shift where id = ?")
                .bind(shift_id)
                .fetch_optional(&mut *conn)
                .await?;
        let float = float.ok_or_else(|| StoreError::ShiftNotFound {
            id: shift_id.to_string(),
        })?;
        let opening_float = decode_money("opening_float", &float)?;

        // ONE statement, so one snapshot: a sale confirming mid-report is
        // seen whole (state, total and tenders together) or not at all. Two
        // reads could see its tenders but not its state, counting its cash
        // without subtracting its change.
        type Joined = (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        );
        let rows: Vec<Joined> = sqlx::query_as(TOTALS_SQL)
            .bind(shift_id)
            .fetch_all(&mut *conn)
            .await?;
        let mut seen = std::collections::HashSet::new();
        let (mut sale_count, mut voided_count) = (0u32, 0u32);
        let (mut gross, mut tax, mut change) = (Money::zero(), Money::zero(), Money::zero());
        let mut by_code: std::collections::BTreeMap<&'static str, (PmtTyCd, Money)> =
            std::collections::BTreeMap::new();
        let mut tendered_per_sale: std::collections::HashMap<String, Money> =
            std::collections::HashMap::new();
        let mut due = std::collections::HashMap::new();
        for (id, state, tot, tot_tax, code, amount) in rows {
            if !seen.insert(id.clone()) {
                // later row of a sale already counted: only its tender is new
            } else if state == "Voided" {
                voided_count += 1;
            } else if state == "Confirmed" {
                sale_count += 1;
                let tot = decode_money("tot_amt", &tot)?;
                gross = add(gross, tot, "gross")?;
                tax = add(tax, decode_money("tot_tax_amt", &tot_tax)?, "tax")?;
                due.insert(id.clone(), tot);
            }
            if state != "Confirmed" {
                continue;
            }
            if let (Some(code), Some(amount)) = (code, amount) {
                let code = decode_pmt_ty("pmt_ty_cd", &code)?;
                let amount = decode_money("amount", &amount)?;
                let slot = by_code.entry(code.wire()).or_insert((code, Money::zero()));
                slot.1 = add(slot.1, amount, "tender")?;
                let t = tendered_per_sale.entry(id).or_insert_with(Money::zero);
                *t = add(*t, amount, "tender")?;
            }
        }
        for (id, tot) in &due {
            let tendered = tendered_per_sale
                .get(id)
                .copied()
                .unwrap_or_else(Money::zero);
            change = add(change, sub(tendered, *tot, "change")?, "change")?;
        }
        let cash = by_code
            .get(PmtTyCd::Cash.wire())
            .map_or_else(Money::zero, |(_, m)| *m);
        let expected_cash = sub(
            add(opening_float, cash, "expected cash")?,
            change,
            "expected cash",
        )?;

        Ok(ShiftTotals {
            sale_count,
            voided_count,
            gross,
            tax,
            by_code: by_code.into_values().collect(),
            opening_float,
            expected_cash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> Money {
        Money::try_from_str(s).unwrap()
    }

    async fn shift_rows(s: &Store) -> Vec<(i64, serde_json::Value)> {
        let rows: Vec<(i64, String)> =
            sqlx::query_as("select id, payload from outbound where kind = 'shift' order by id")
                .fetch_all(s.pool())
                .await
                .unwrap();
        rows.into_iter()
            .map(|(i, p)| (i, serde_json::from_str(&p).unwrap()))
            .collect()
    }

    #[tokio::test]
    async fn opening_a_shift_queues_it_for_the_cloud() {
        let s = Store::open_in_memory().await.unwrap();
        let o = s.open_shift("d1", "alice", m("5000.00")).await.unwrap();
        let rows = shift_rows(&s).await;
        assert_eq!(rows.len(), 1);
        let v = &rows[0].1;
        assert_eq!(v["id"], o.id);
        assert_eq!(v["dvcSrlNo"], "d1");
        assert_eq!(v["openedById"], "alice");
        assert_eq!(v["openedAt"], o.opened_at);
        // an exact decimal string, never a JSON number
        assert_eq!(v["openingFloat"], "5000.00");
        assert!(v.get("closedAt").is_none());
        let sale_id: Option<String> = sqlx::query_scalar("select sale_id from outbound")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(sale_id, None);
    }

    #[tokio::test]
    async fn closing_a_shift_queues_the_close() {
        let s = Store::open_in_memory().await.unwrap();
        let o = s.open_shift("d1", "alice", m("5000.00")).await.unwrap();
        s.close_shift(&o.id, m("4990.50")).await.unwrap();
        let rows = shift_rows(&s).await;
        assert_eq!(rows.len(), 2, "open and close are two rows");
        let v = &rows[1].1;
        assert_eq!(v["id"], o.id);
        assert_eq!(v["dvcSrlNo"], "d1");
        assert_eq!(v["declaredCash"], "4990.50");
        let closed_at: String = sqlx::query_scalar("select closed_at from shift where id = ?")
            .bind(&o.id)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(v["closedAt"], closed_at);
        assert!(
            rows[0].1.get("closedAt").is_none(),
            "the open row is untouched"
        );
    }

    #[tokio::test]
    async fn the_last_closed_shift_is_recomposed_exactly_as_it_closed() {
        let s = Store::open_in_memory().await.unwrap();
        assert_eq!(s.last_closed_shift("d1").await.unwrap(), None);
        let first = s.open_shift("d1", "alice", m("5000.00")).await.unwrap();
        s.close_shift(&first.id, m("5000")).await.unwrap();
        let second = s.open_shift("d1", "bob", m("1000.00")).await.unwrap();
        // An open shift is not a closed one.
        assert_eq!(
            s.last_closed_shift("d1").await.unwrap().unwrap().id,
            first.id
        );
        let closed = s.close_shift(&second.id, m("900")).await.unwrap();
        let again = s.last_closed_shift("d1").await.unwrap().unwrap();
        assert_eq!(again.id, second.id);
        assert_eq!(again.close, closed);
        assert_eq!(again.close.variance, m("-100"));
        assert_eq!(s.last_closed_shift("other").await.unwrap(), None);
        assert_eq!(s.shift_mrc(&second.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_refused_open_or_close_queues_nothing() {
        let s = Store::open_in_memory().await.unwrap();
        let o = s.open_shift("d1", "alice", m("0")).await.unwrap();
        assert!(s.open_shift("d1", "bob", m("0")).await.is_err());
        assert!(s.close_shift(&o.id, m("-1")).await.is_err());
        assert!(s.close_shift("nope", m("0")).await.is_err());
        assert_eq!(shift_rows(&s).await.len(), 1);
        s.close_shift(&o.id, m("0")).await.unwrap();
        assert!(s.close_shift(&o.id, m("0")).await.is_err());
        assert_eq!(shift_rows(&s).await.len(), 2);
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

    // ---- shift_totals ----

    async fn seed_sale(
        s: &Store,
        shift: &str,
        invc: i64,
        state: &str,
        (tot, tax): (&str, &str),
        tenders: &[(&str, &str)],
    ) {
        sqlx::query("insert or ignore into invc_seq (dvc_srl_no, counter, last_invc_no) values ('d1','SaleInvc',0)")
            .execute(s.pool()).await.unwrap();
        let id = format!("sale-{shift}-{invc}");
        sqlx::query("insert into sale (id, shift_id, dvc_srl_no, counter, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, created_at) values (?, ?, 'd1', 'SaleInvc', '00', ?, ?, ?, ?, ?, '01', 't')")
            .bind(&id).bind(shift).bind(invc).bind(state).bind(tot).bind(tot).bind(tax)
            .execute(s.pool()).await.unwrap();
        for (i, (code, amount)) in tenders.iter().enumerate() {
            sqlx::query(
                "insert into sale_tender (sale_id, seq, pmt_ty_cd, amount) values (?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(i as i64 + 1)
            .bind(code)
            .bind(amount)
            .execute(s.pool())
            .await
            .unwrap();
        }
    }

    async fn shift_with_float(s: &Store, float: &str) -> String {
        s.open_shift("d1", "alice", m(float)).await.unwrap().id
    }

    #[tokio::test]
    async fn totals_count_confirmed_sales_and_exclude_abandoned_ones() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("2380.00", "180.00"),
            &[("01", "2380.00")],
        )
        .await;
        seed_sale(
            &s,
            &sh,
            2,
            "Confirmed",
            ("1000.00", "0.00"),
            &[("01", "1000.00")],
        )
        .await;
        seed_sale(
            &s,
            &sh,
            3,
            "Voided",
            ("9999.00", "999.00"),
            &[("01", "9999.00")],
        )
        .await;
        seed_sale(
            &s,
            &sh,
            4,
            "AwaitingVsdc",
            ("7777.00", "777.00"),
            &[("01", "7777.00")],
        )
        .await;
        seed_sale(
            &s,
            &sh,
            5,
            "Failed",
            ("5555.00", "555.00"),
            &[("01", "5555.00")],
        )
        .await;
        let t = s.shift_totals(&sh).await.unwrap();
        assert_eq!(t.sale_count, 2);
        assert_eq!(t.voided_count, 1);
        assert_eq!(t.gross, m("3380.00"));
        assert_eq!(t.tax, m("180.00"));
        assert_eq!(t.by_code, vec![(PmtTyCd::Cash, m("3380.00"))]);
        assert_eq!(t.expected_cash, m("3380.00"));
    }

    #[tokio::test]
    async fn totals_ignore_other_shifts_and_an_empty_shift_is_just_its_float() {
        let s = Store::open_in_memory().await.unwrap();
        let first = shift_with_float(&s, "100").await;
        seed_sale(
            &s,
            &first,
            1,
            "Confirmed",
            ("500.00", "0.00"),
            &[("01", "500.00")],
        )
        .await;
        sqlx::query("update shift set closed_at = 't', declared_cash = '0.00'")
            .execute(s.pool())
            .await
            .unwrap();
        let second = shift_with_float(&s, "250").await;
        let t = s.shift_totals(&second).await.unwrap();
        assert_eq!((t.sale_count, t.voided_count), (0, 0));
        assert_eq!(t.gross, Money::zero());
        assert!(t.by_code.is_empty());
        assert_eq!(t.opening_float, m("250"));
        assert_eq!(t.expected_cash, m("250"));
        assert!(matches!(
            s.shift_totals("nope").await,
            Err(StoreError::ShiftNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn totals_split_tenders_by_their_rra_code() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        // inserted out of code order, across sales
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("2380.00", "180.00"),
            &[("06", "1380.00"), ("01", "1000.00")],
        )
        .await;
        seed_sale(
            &s,
            &sh,
            2,
            "Confirmed",
            ("1000.00", "0.00"),
            &[("07", "400.00"), ("06", "600.00")],
        )
        .await;
        let t = s.shift_totals(&sh).await.unwrap();
        assert_eq!(
            t.by_code,
            vec![
                (PmtTyCd::Cash, m("1000.00")),
                (PmtTyCd::MobileMoney, m("1980.00")),
                (PmtTyCd::Other, m("400.00")),
            ]
        );
        assert_eq!(t.gross, m("3380.00"));
    }

    #[tokio::test]
    async fn expected_cash_is_float_plus_cash_taken_minus_change() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "5000.00").await;
        // 5000 note against 2380: 2620 change leaves the drawer
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("2380.00", "180.00"),
            &[("01", "5000.00")],
        )
        .await;
        // part card, part cash, no change
        seed_sale(
            &s,
            &sh,
            2,
            "Confirmed",
            ("3000.00", "0.00"),
            &[("05", "2000.00"), ("01", "1000.00")],
        )
        .await;
        // an abandoned sale's cash never reached the drawer
        seed_sale(
            &s,
            &sh,
            3,
            "Voided",
            ("800.00", "0.00"),
            &[("01", "800.00")],
        )
        .await;
        let t = s.shift_totals(&sh).await.unwrap();
        // 5000 + (5000 + 1000) - 2620
        assert_eq!(t.expected_cash, m("8380.00"));
        // by_code is what was handed over, before change
        assert_eq!(
            t.by_code,
            vec![(PmtTyCd::Cash, m("6000.00")), (PmtTyCd::Card, m("2000.00"))]
        );
    }

    #[tokio::test]
    async fn a_cash_credit_tender_does_not_count_toward_expected_cash() {
        // 03 is not 01. The same distinction till-core makes for change.
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "1000.00").await;
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("4000.00", "0.00"),
            &[("03", "4000.00")],
        )
        .await;
        let t = s.shift_totals(&sh).await.unwrap();
        assert_eq!(t.by_code, vec![(PmtTyCd::CashCredit, m("4000.00"))]);
        assert_eq!(t.expected_cash, m("1000.00"));
        seed_sale(
            &s,
            &sh,
            2,
            "Confirmed",
            ("500.00", "0.00"),
            &[("01", "500.00")],
        )
        .await;
        assert_eq!(
            s.shift_totals(&sh).await.unwrap().expected_cash,
            m("1500.00")
        );
    }

    #[tokio::test]
    async fn totals_do_not_round_through_a_float() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0.10").await;
        seed_sale(&s, &sh, 1, "Confirmed", ("0.20", "0.00"), &[("01", "0.20")]).await;
        seed_sale(&s, &sh, 2, "Confirmed", ("0.10", "0.00"), &[("01", "0.10")]).await;
        assert_eq!(s.shift_totals(&sh).await.unwrap().expected_cash, m("0.40"));
    }

    #[tokio::test]
    async fn totals_query_searches_the_sale_shift_index() {
        let s = Store::open_in_memory().await.unwrap();
        let plan: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&format!("explain query plan {TOTALS_SQL}"))
                .bind("x")
                .fetch_all(s.pool())
                .await
                .unwrap();
        assert!(
            plan.iter()
                .any(|r| r.3.contains("sale") && r.3.contains("sale_shift")),
            "{plan:?}"
        );
    }

    #[tokio::test]
    async fn a_confirmed_sale_with_no_tenders_still_counts() {
        // a fully discounted sale has no sale_tender rows; the join must keep it
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "10").await;
        seed_sale(&s, &sh, 1, "Confirmed", ("0.00", "0.00"), &[]).await;
        let t = s.shift_totals(&sh).await.unwrap();
        assert_eq!(t.sale_count, 1);
        assert!(t.by_code.is_empty());
        assert_eq!(t.expected_cash, m("10"));
    }

    // ---- close_shift ----

    #[tokio::test]
    async fn closing_records_the_declared_cash_and_the_variance() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "5000.00").await;
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("2380.00", "180.00"),
            &[("01", "5000.00")],
        )
        .await;
        // expected: 5000 + 5000 - 2620 = 7380
        let c = s.close_shift(&sh, m("7400.00")).await.unwrap();
        assert_eq!(c.totals.expected_cash, m("7380.00"));
        assert_eq!(c.totals.sale_count, 1);
        assert_eq!(c.declared_cash, m("7400.00"));
        assert_eq!(c.variance, m("20.00"));
        let (closed_at, declared): (Option<String>, Option<String>) =
            sqlx::query_as("select closed_at, declared_cash from shift where id = ?")
                .bind(&sh)
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert!(closed_at.is_some());
        assert_eq!(declared.as_deref(), Some("7400.00"));
    }

    #[tokio::test]
    async fn a_short_till_records_a_negative_variance_rather_than_failing() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "1000.00").await;
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("500.00", "0.00"),
            &[("01", "500.00")],
        )
        .await;
        // expected 1500; declare 1000 short
        let c = s.close_shift(&sh, m("500.00")).await.unwrap();
        assert_eq!(c.variance, m("-1000.00"));
        assert_eq!(s.current_shift("d1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn closing_a_shift_with_a_pending_sale_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        seed_sale(
            &s,
            &sh,
            1,
            "AwaitingVsdc",
            ("100.00", "0.00"),
            &[("01", "100.00")],
        )
        .await;
        let r = s.close_shift(&sh, m("0")).await;
        assert!(matches!(r, Err(StoreError::ShiftHasPendingSale)), "{r:?}");
        // nothing was written: still open
        assert!(s.current_shift("d1").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn closing_an_already_closed_shift_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        s.close_shift(&sh, m("0")).await.unwrap();
        let r = s.close_shift(&sh, m("999")).await;
        assert!(matches!(r, Err(StoreError::ShiftAlreadyClosed)), "{r:?}");
        let declared: Option<String> =
            sqlx::query_scalar("select declared_cash from shift where id = ?")
                .bind(&sh)
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!(declared.as_deref(), Some("0.00"));
        assert!(matches!(
            s.close_shift("nope", m("0")).await,
            Err(StoreError::ShiftNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn a_negative_declared_cash_is_refused_and_zero_is_a_valid_count() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "100").await;
        let r = s.close_shift(&sh, m("-1.00")).await;
        assert!(matches!(r, Err(StoreError::NegativeDeclaredCash)), "{r:?}");
        assert!(s.current_shift("d1").await.unwrap().is_some());
        // an empty drawer is a short till (variance -100), not a refusal
        assert_eq!(
            s.close_shift(&sh, m("0")).await.unwrap().variance,
            m("-100")
        );
    }

    #[tokio::test]
    async fn a_closed_shift_is_no_longer_current() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        assert!(s.current_shift("d1").await.unwrap().is_some());
        s.close_shift(&sh, m("0")).await.unwrap();
        assert_eq!(s.current_shift("d1").await.unwrap(), None);
        s.open_shift("d1", "bob", m("0")).await.unwrap();
    }

    #[tokio::test]
    async fn the_close_payload_carries_the_shifts_own_count_and_gross() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        seed_sale(
            &s,
            &sh,
            1,
            "Confirmed",
            ("2380.10", "180.00"),
            &[("01", "2380.10")],
        )
        .await;
        seed_sale(
            &s,
            &sh,
            2,
            "Confirmed",
            ("1000.20", "0.00"),
            &[("01", "1000.20")],
        )
        .await;
        // abandoned sales are in neither figure
        seed_sale(&s, &sh, 3, "Voided", ("9.00", "0.00"), &[("01", "9.00")]).await;
        let c = s.close_shift(&sh, m("3380.30")).await.unwrap();
        let rows = shift_rows(&s).await;
        let v = &rows[1].1;
        assert_eq!(v["saleCount"], 2);
        assert!(v["saleCount"].is_u64(), "a JSON number");
        assert_eq!(v["gross"], "3380.30");
        assert!(v["gross"].is_string(), "an exact decimal string");
        // the same figures the close report showed
        assert_eq!(c.totals.sale_count, 2);
        assert_eq!(c.totals.gross.to_string(), v["gross"].as_str().unwrap());
    }

    #[tokio::test]
    async fn a_close_with_no_sales_states_zero_not_absent() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        s.close_shift(&sh, m("0")).await.unwrap();
        let v = &shift_rows(&s).await[1].1;
        assert_eq!(v["saleCount"], 0);
        assert_eq!(v["gross"], "0.00");
    }

    #[tokio::test]
    async fn an_open_payload_still_carries_neither() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        seed_sale(&s, &sh, 1, "Confirmed", ("5.00", "0.00"), &[("01", "5.00")]).await;
        s.close_shift(&sh, m("5")).await.unwrap();
        let open = &shift_rows(&s).await[0].1;
        assert!(open.get("saleCount").is_none());
        assert!(open.get("gross").is_none());
    }

    #[tokio::test]
    async fn a_voided_sale_does_not_block_closing() {
        let s = Store::open_in_memory().await.unwrap();
        let sh = shift_with_float(&s, "0").await;
        seed_sale(
            &s,
            &sh,
            1,
            "Voided",
            ("100.00", "0.00"),
            &[("01", "100.00")],
        )
        .await;
        let c = s.close_shift(&sh, m("0")).await.unwrap();
        assert_eq!(c.totals.voided_count, 1);
        assert_eq!(c.variance, Money::zero());
    }
}
