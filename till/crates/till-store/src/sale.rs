//! Writing a sale to the till's store, and queueing it for the cloud once it
//! is complete.
//!
//! A sale has three moments, each its own short transaction:
//!
//! 1. [`Store::begin_sale`] allocates the invoice number and writes the sale,
//!    its lines and its tenders as `AwaitingVsdc`, then COMMITS and returns.
//! 2. The caller talks to the VSDC. No transaction is open, so no lock is held
//!    across the network round trip: the store's 2 s `busy_timeout` assumes
//!    that, and holding the write lock for a VSDC timeout would stall every
//!    other lane operation. Do not fold this step into either transaction.
//! 3. [`Store::confirm_sale`] (VSDC answered, receipt printed) opens a fresh
//!    transaction, records the receipt and queues exactly one `outbound` row.
//!    [`Store::fail_sale`] instead counts the failed attempt and leaves the
//!    number for the retry.
//!
//! The tax authority, not our server, is the legal counterparty, so nothing is
//! queued for the cloud until step 3. And `confirm_sale` is the only path that
//! completes a sale: its [`VsdcReceipt`] must be a real VSDC response, and
//! nothing in this crate synthesises one.

use sqlx::{Row, SqliteConnection};
use till_core::codes::PmtTyCd;
use till_core::{Basket, Settled, TenderSet};

use crate::schema::{Store, StoreError, decode_money, decode_pmt_ty, decode_qty, decode_tax_ty};
use crate::sequence::allocate_on;

/// Everything `begin_sale` needs. `settled` must be what `tenders.settle`
/// returned for this basket's total; it is re-checked, not trusted.
#[derive(Debug, Clone, Copy)]
pub struct NewSale<'a> {
    pub bhf_id: &'a str,
    pub basket: &'a Basket,
    pub tenders: &'a TenderSet,
    pub settled: &'a Settled,
    pub buyer_tin: Option<&'a str>,
}

/// A begun sale: its id and the invoice number it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaleHandle {
    pub id: String,
    pub invc_no: i64,
}

/// The VSDC's answer for a saved sale. Must come from a real response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VsdcReceipt {
    pub rcpt_no: i64,
    pub intrl_data: String,
    pub rcpt_sign: String,
    pub sdc_id: String,
    pub mrc_no: String,
    /// The VSDC's receipt timestamp: the date and time of the C5 receipt QR.
    pub vsdc_rcpt_pbct_date: String,
    /// `saveSales`' `totRcptNo`. Stored beside `rcpt_no` because the QR's
    /// "sdc_receipt_number" could mean either.
    pub tot_rcpt_no: i64,
}

/// UTC timestamp in SQL, so the store never needs a clock crate.
const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%SZ','now')";

impl Store {
    /// Allocates the next invoice number and writes the sale, its lines and
    /// its tenders in ONE `BEGIN IMMEDIATE` transaction, state `AwaitingVsdc`.
    /// A crash or error leaves all of it or none of it, and a rolled-back
    /// allocation rewinds the counter, so a failed sale is not a gap.
    ///
    /// Everything that can be computed or refused is done BEFORE the
    /// transaction opens, so the write lock is held only for the inserts.
    pub async fn begin_sale(&self, new: NewSale<'_>) -> Result<SaleHandle, StoreError> {
        if new.basket.is_empty() {
            return Err(StoreError::EmptySale);
        }
        let totals = new.basket.totals()?;
        if new.tenders.settle(totals.tot_amt)? != *new.settled {
            return Err(StoreError::SettlementMismatch);
        }
        let lines = new.basket.apportioned_lines()?;
        if lines.is_empty() {
            return Err(StoreError::EmptySale);
        }

        let mut tx = self.begin_immediate().await?;
        let written = async {
            let invc_no = allocate_on(&mut tx, new.bhf_id).await?;
            let conn = tx.conn();
            let id: String = sqlx::query_scalar("select lower(hex(randomblob(16)))")
                .fetch_one(&mut *conn)
                .await?;
            sqlx::query(&format!(
                "insert into sale (id, bhf_id, invc_no, state, tot_amt, tot_taxbl_amt, \
                 tot_tax_amt, pmt_ty_cd, buyer_tin, created_at) \
                 values (?, ?, ?, 'AwaitingVsdc', ?, ?, ?, ?, ?, {NOW_SQL})"
            ))
            .bind(&id)
            .bind(new.bhf_id)
            .bind(invc_no)
            .bind(totals.tot_amt.to_string())
            .bind(totals.tot_taxbl_amt.to_string())
            .bind(totals.tot_tax_amt.to_string())
            .bind(new.settled.header_pmt_ty_cd.wire())
            .bind(new.buyer_tin)
            .execute(&mut *conn)
            .await?;
            for (line, a) in &lines {
                sqlx::query(
                    "insert into sale_line (sale_id, item_seq, item_cd, item_cls_cd, item_nm, \
                     qty, prc, sply_amt, dc_rt, dc_amt, taxbl_amt, tax_amt, tax_ty_cd) \
                     values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(i64::from(line.item_seq))
                .bind(&line.item_cd)
                .bind(&line.item_cls_cd)
                .bind(&line.item_nm)
                .bind(line.qty.to_string())
                .bind(line.prc.to_string())
                .bind(a.sply_amt.to_string())
                .bind(a.dc_rt.to_string())
                .bind(a.dc_amt.to_string())
                .bind(a.taxbl_amt.to_string())
                .bind(a.tax_amt.to_string())
                .bind(line.tax_ty_cd.wire())
                .execute(&mut *conn)
                .await?;
            }
            for (i, t) in new.tenders.entries().iter().enumerate() {
                sqlx::query(
                    "insert into sale_tender (sale_id, seq, pmt_ty_cd, amount) values (?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(i64::try_from(i + 1).map_err(|_| StoreError::EmptySale)?)
                .bind(t.pmt_ty_cd.wire())
                .bind(t.amount.to_string())
                .execute(&mut *conn)
                .await?;
            }
            Ok::<_, StoreError>(SaleHandle { id, invc_no })
        }
        .await;
        match written {
            Ok(handle) => {
                tx.commit().await?;
                Ok(handle)
            }
            Err(e) => {
                // The original error is the one worth reporting; a failed
                // rollback leaves the connection to be discarded by the pool.
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Records the VSDC's receipt, moves the sale to `Confirmed` and queues
    /// exactly one `outbound` row, in one transaction.
    ///
    /// Idempotent for a repeat of the SAME receipt on an already `Confirmed`
    /// sale (a retry after a crash between commit and return): returns `Ok`
    /// and queues nothing more. A DIFFERENT receipt on a confirmed sale is
    /// [`StoreError::ReceiptConflict`]; a `Failed` or `Voided` sale is
    /// [`StoreError::SaleNotAwaiting`].
    ///
    /// A VSDC `994` ("overlapped data") is NOT a path to here: it carries no
    /// receipt, and the spec has no endpoint to re-read one, so nothing may be
    /// faked. See the `sequence` module docs.
    pub async fn confirm_sale(
        &self,
        handle: &SaleHandle,
        r: VsdcReceipt,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin_immediate().await?;
        let done = confirm_on(tx.conn(), handle, &r).await;
        match done {
            Ok(()) => tx.commit().await,
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Counts a failed VSDC attempt and records why, leaving the sale
    /// `AwaitingVsdc` so the SAME number is retried (Ruling R44). May be
    /// called repeatedly; each call adds one attempt. Refused with
    /// [`StoreError::SaleNotAwaiting`] once the sale is no longer awaiting, so
    /// a late failure cannot overwrite a confirmed sale's record.
    pub async fn fail_sale(&self, handle: &SaleHandle, reason: &str) -> Result<(), StoreError> {
        let mut tx = self.begin_immediate().await?;
        let done = async {
            let n = sqlx::query(
                "update sale set attempts = attempts + 1, last_error = ? \
                 where id = ? and invc_no = ? and state = 'AwaitingVsdc'",
            )
            .bind(reason)
            .bind(&handle.id)
            .bind(handle.invc_no)
            .execute(tx.conn())
            .await?
            .rows_affected();
            if n == 1 {
                Ok(())
            } else {
                Err(refusal(tx.conn(), handle).await)
            }
        }
        .await;
        match done {
            Ok(()) => tx.commit().await,
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

/// Why an update that matched no `AwaitingVsdc` row did nothing.
async fn refusal(conn: &mut SqliteConnection, h: &SaleHandle) -> StoreError {
    let state: Result<Option<String>, sqlx::Error> =
        sqlx::query_scalar("select state from sale where id = ? and invc_no = ?")
            .bind(&h.id)
            .bind(h.invc_no)
            .fetch_optional(conn)
            .await;
    match state {
        Err(e) => StoreError::Sqlx(e),
        Ok(None) => StoreError::SaleNotFound {
            id: h.id.clone(),
            invc_no: h.invc_no,
        },
        Ok(Some(state)) => StoreError::SaleNotAwaiting {
            id: h.id.clone(),
            state,
        },
    }
}

async fn confirm_on(
    conn: &mut SqliteConnection,
    h: &SaleHandle,
    r: &VsdcReceipt,
) -> Result<(), StoreError> {
    let n = sqlx::query(&format!(
        "update sale set state = 'Confirmed', rcpt_no = ?, intrl_data = ?, rcpt_sign = ?, \
         sdc_id = ?, mrc_no = ?, vsdc_rcpt_pbct_date = ?, tot_rcpt_no = ?, confirmed_at = {NOW_SQL} \
         where id = ? and invc_no = ? and state = 'AwaitingVsdc'"
    ))
    .bind(r.rcpt_no)
    .bind(&r.intrl_data)
    .bind(&r.rcpt_sign)
    .bind(&r.sdc_id)
    .bind(&r.mrc_no)
    .bind(&r.vsdc_rcpt_pbct_date)
    .bind(r.tot_rcpt_no)
    .bind(&h.id)
    .bind(h.invc_no)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if n == 0 {
        // Not awaiting. A repeat of the identical receipt is a success.
        let same: Option<i64> = sqlx::query_scalar(
            "select 1 from sale where id = ? and invc_no = ? and state = 'Confirmed' \
             and rcpt_no = ? and intrl_data = ? and rcpt_sign = ? and sdc_id = ? and mrc_no = ? \
             and vsdc_rcpt_pbct_date = ? and tot_rcpt_no = ?",
        )
        .bind(&h.id)
        .bind(h.invc_no)
        .bind(r.rcpt_no)
        .bind(&r.intrl_data)
        .bind(&r.rcpt_sign)
        .bind(&r.sdc_id)
        .bind(&r.mrc_no)
        .bind(&r.vsdc_rcpt_pbct_date)
        .bind(r.tot_rcpt_no)
        .fetch_optional(&mut *conn)
        .await?;
        if same.is_some() {
            return Ok(());
        }
        return Err(match refusal(conn, h).await {
            StoreError::SaleNotAwaiting { id, state } if state == "Confirmed" => {
                StoreError::ReceiptConflict { id }
            }
            other => other,
        });
    }
    let payload = cloud_payload(conn, h, r).await?;
    sqlx::query(&format!(
        "insert into outbound (kind, sale_id, payload, created_at) \
         values ('sale', ?, ?, {NOW_SQL})"
    ))
    .bind(&h.id)
    .bind(payload)
    .execute(conn)
    .await?;
    Ok(())
}

/// The sale as the CLOUD receives it, rebuilt from what was committed.
///
/// The cloud is not the tax authority: its Postgres enum names payment types
/// `CASH`, `MOBILE_MONEY`, ..., so they go through `cloud_label()`, never
/// `wire()` (the RRA code). Amounts stay exact decimal strings, re-validated
/// through `Money`/`Qty` on the way out.
async fn cloud_payload(
    conn: &mut SqliteConnection,
    h: &SaleHandle,
    r: &VsdcReceipt,
) -> Result<String, StoreError> {
    let head = sqlx::query(
        "select bhf_id, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd, buyer_tin, \
         created_at, confirmed_at from sale where id = ?",
    )
    .bind(&h.id)
    .fetch_one(&mut *conn)
    .await?;
    let money = |col: &str| -> Result<String, StoreError> {
        Ok(decode_money(col, &head.try_get::<String, _>(col)?)?.to_string())
    };
    let header_pmt: PmtTyCd = decode_pmt_ty("pmt_ty_cd", &head.try_get::<String, _>("pmt_ty_cd")?)?;

    let mut lines = Vec::new();
    for l in sqlx::query(
        "select item_seq, item_cd, item_cls_cd, item_nm, qty, prc, sply_amt, dc_rt, dc_amt, \
         taxbl_amt, tax_amt, tax_ty_cd from sale_line where sale_id = ? order by item_seq",
    )
    .bind(&h.id)
    .fetch_all(&mut *conn)
    .await?
    {
        let m = |c: &str| -> Result<String, StoreError> {
            Ok(decode_money(c, &l.try_get::<String, _>(c)?)?.to_string())
        };
        lines.push(serde_json::json!({
            "itemSeq": l.try_get::<i64, _>("item_seq")?,
            "itemCd": l.try_get::<Option<String>, _>("item_cd")?,
            "itemClsCd": l.try_get::<Option<String>, _>("item_cls_cd")?,
            "itemNm": l.try_get::<String, _>("item_nm")?,
            "qty": decode_qty("qty", &l.try_get::<String, _>("qty")?)?.to_string(),
            "prc": m("prc")?,
            "splyAmt": m("sply_amt")?,
            "dcRt": l.try_get::<String, _>("dc_rt")?,
            "dcAmt": m("dc_amt")?,
            "taxblAmt": m("taxbl_amt")?,
            "taxAmt": m("tax_amt")?,
            "taxTyCd": decode_tax_ty("tax_ty_cd", &l.try_get::<String, _>("tax_ty_cd")?)?.wire(),
        }));
    }

    let mut tenders = Vec::new();
    for t in
        sqlx::query("select seq, pmt_ty_cd, amount from sale_tender where sale_id = ? order by seq")
            .bind(&h.id)
            .fetch_all(&mut *conn)
            .await?
    {
        let pmt = decode_pmt_ty("pmt_ty_cd", &t.try_get::<String, _>("pmt_ty_cd")?)?;
        tenders.push(serde_json::json!({
            "seq": t.try_get::<i64, _>("seq")?,
            "pmtTyCd": pmt.cloud_label(),
            "amount": decode_money("amount", &t.try_get::<String, _>("amount")?)?.to_string(),
        }));
    }

    Ok(serde_json::to_string(&serde_json::json!({
        "saleId": h.id,
        "bhfId": head.try_get::<String, _>("bhf_id")?,
        "invcNo": h.invc_no,
        "buyerTin": head.try_get::<Option<String>, _>("buyer_tin")?,
        "totAmt": money("tot_amt")?,
        "totTaxblAmt": money("tot_taxbl_amt")?,
        "totTaxAmt": money("tot_tax_amt")?,
        "pmtTyCd": header_pmt.cloud_label(),
        "createdAt": head.try_get::<String, _>("created_at")?,
        "confirmedAt": head.try_get::<Option<String>, _>("confirmed_at")?,
        "lines": lines,
        "tenders": tenders,
        "receipt": {
            "rcptNo": r.rcpt_no,
            "intrlData": r.intrl_data,
            "rcptSign": r.rcpt_sign,
            "sdcId": r.sdc_id,
            "mrcNo": r.mrc_no,
            "vsdcRcptPbctDate": r.vsdc_rcpt_pbct_date,
            "totRcptNo": r.tot_rcpt_no,
        },
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use till_core::codes::TaxType;
    use till_core::{BasketLine, Money, Qty, TenderEntry};

    fn line(cd: &str, prc: &str, band: TaxType) -> BasketLine {
        BasketLine {
            item_seq: 0,
            item_cd: cd.to_string(),
            item_cls_cd: "5020230602".to_string(),
            item_nm: cd.to_string(),
            bcd: None,
            qty: Qty::try_from_str("1").unwrap(),
            prc: Money::try_from_str(prc).unwrap(),
            tax_ty_cd: band,
            discount: None,
        }
    }

    /// Exempt milk at 1200 plus standard-rated soap at 1180: 2380.00, the
    /// figures `till-core`'s basket tests assert.
    fn basket() -> Basket {
        let mut b = Basket::new();
        b.push_line(line("MILK", "1200", TaxType::A)).unwrap();
        b.push_line(line("SOAP", "1180", TaxType::B)).unwrap();
        b
    }

    /// Owns what `NewSale` borrows.
    struct Fixture {
        basket: Basket,
        tenders: TenderSet,
        settled: Settled,
    }

    impl Fixture {
        fn new(tenders: &[(PmtTyCd, &str)]) -> Fixture {
            let basket = basket();
            let mut set = TenderSet::default();
            for (c, a) in tenders {
                set.push(TenderEntry {
                    pmt_ty_cd: *c,
                    amount: Money::try_from_str(a).unwrap(),
                })
                .unwrap();
            }
            let settled = set.settle(basket.totals().unwrap().tot_amt).unwrap();
            Fixture {
                basket,
                tenders: set,
                settled,
            }
        }
        fn sale<'a>(&'a self, bhf: &'a str) -> NewSale<'a> {
            NewSale {
                bhf_id: bhf,
                basket: &self.basket,
                tenders: &self.tenders,
                settled: &self.settled,
                buyer_tin: None,
            }
        }
    }

    fn cash() -> Fixture {
        Fixture::new(&[(PmtTyCd::Cash, "2380")])
    }

    fn receipt() -> VsdcReceipt {
        VsdcReceipt {
            rcpt_no: 42,
            intrl_data: "INTRL".into(),
            rcpt_sign: "ABCD1234EFGH5678".into(),
            sdc_id: "SDC010000001".into(),
            mrc_no: "WIS00000001".into(),
            vsdc_rcpt_pbct_date: "20261121103000".into(),
            tot_rcpt_no: 7,
        }
    }

    async fn count(s: &Store, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(s.pool()).await.unwrap()
    }

    #[tokio::test]
    async fn a_begun_sale_stores_its_lines_tenders_and_totals() {
        let s = Store::open_in_memory().await.unwrap();
        let f = cash();
        let h = s.begin_sale(f.sale("00")).await.unwrap();
        assert_eq!(h.invc_no, 1);
        let (state, tot, taxbl, tax, pmt): (String, String, String, String, String) =
            sqlx::query_as(
                "select state, tot_amt, tot_taxbl_amt, tot_tax_amt, pmt_ty_cd from sale where id=?",
            )
            .bind(&h.id)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(state, "AwaitingVsdc");
        assert_eq!(tot, "2380.00");
        assert_eq!(taxbl, "2380.00");
        assert_eq!(tax, "180.00");
        assert_eq!(pmt, "01");
        assert_eq!(count(&s, "select count(*) from sale_line").await, 2);
        let (amount,): (String,) = sqlx::query_as("select amount from sale_tender where sale_id=?")
            .bind(&h.id)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(amount, "2380.00");
    }

    #[tokio::test]
    async fn beginning_a_sale_queues_nothing_yet() {
        // C1: the sale is not complete, so the cloud must not hear about it
        let s = Store::open_in_memory().await.unwrap();
        s.begin_sale(cash().sale("00")).await.unwrap();
        assert_eq!(count(&s, "select count(*) from outbound").await, 0);
    }

    #[tokio::test]
    async fn confirming_stores_the_receipt_and_queues_exactly_one_row() {
        let s = Store::open_in_memory().await.unwrap();
        let h = s.begin_sale(cash().sale("00")).await.unwrap();
        s.confirm_sale(&h, receipt()).await.unwrap();
        let (state, rcpt, sign): (String, i64, String) =
            sqlx::query_as("select state, rcpt_no, rcpt_sign from sale where id=?")
                .bind(&h.id)
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!(state, "Confirmed");
        assert_eq!(rcpt, 42);
        assert_eq!(sign, "ABCD1234EFGH5678");
        let q: i64 = sqlx::query_scalar("select count(*) from outbound where sale_id=?")
            .bind(&h.id)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(q, 1);
    }

    #[tokio::test]
    async fn the_receipt_qr_fields_are_readable_from_the_sale_row_alone() {
        // A COPY reprint must rebuild the C5 QR without the outbound payload.
        let s = Store::open_in_memory().await.unwrap();
        let h = s.begin_sale(cash().sale("00")).await.unwrap();
        let before: (Option<String>, Option<i64>) =
            sqlx::query_as("select vsdc_rcpt_pbct_date, tot_rcpt_no from sale where id=?")
                .bind(&h.id)
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!(before, (None, None));
        s.confirm_sale(&h, receipt()).await.unwrap();
        let row = sqlx::query(
            "select vsdc_rcpt_pbct_date, tot_rcpt_no, rcpt_no, sdc_id, intrl_data, rcpt_sign \
             from sale where id=?",
        )
        .bind(&h.id)
        .fetch_one(s.pool())
        .await
        .unwrap();
        assert_eq!(
            row.get::<String, _>("vsdc_rcpt_pbct_date"),
            "20261121103000"
        );
        assert_eq!(row.get::<i64, _>("tot_rcpt_no"), 7);
        assert_eq!(row.get::<i64, _>("rcpt_no"), 42);
        assert_eq!(row.get::<String, _>("sdc_id"), "SDC010000001");
        assert_eq!(row.get::<String, _>("intrl_data"), "INTRL");
        assert_eq!(row.get::<String, _>("rcpt_sign"), "ABCD1234EFGH5678");
    }

    #[tokio::test]
    async fn the_outbound_payload_uses_cloud_labels_and_exact_decimal_strings() {
        let s = Store::open_in_memory().await.unwrap();
        // Two tenders: the header takes the larger total (MobileMoney), tenders keep their own.
        let f = Fixture::new(&[(PmtTyCd::Cash, "1000"), (PmtTyCd::MobileMoney, "1380")]);
        let h = s.begin_sale(f.sale("00")).await.unwrap();
        s.confirm_sale(&h, receipt()).await.unwrap();
        let (kind, payload): (String, String) =
            sqlx::query_as("select kind, payload from outbound where sale_id=?")
                .bind(&h.id)
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!(kind, "sale");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["pmtTyCd"], "MOBILE_MONEY");
        assert_eq!(v["tenders"][0]["pmtTyCd"], "CASH");
        assert_eq!(v["tenders"][1]["pmtTyCd"], "MOBILE_MONEY");
        assert_eq!(v["tenders"][1]["amount"], "1380.00");
        assert_eq!(v["totAmt"], "2380.00");
        assert_eq!(v["lines"].as_array().unwrap().len(), 2);
        assert_eq!(v["lines"][1]["taxAmt"], "180.00");
        assert_eq!(v["lines"][0]["qty"], "1.000");
        assert_eq!(v["receipt"]["vsdcRcptPbctDate"], "20261121103000");
        assert!(!payload.contains("\"01\"") && !payload.contains("\"06\""));
    }

    #[tokio::test]
    async fn a_failed_sale_keeps_its_number_for_the_retry() {
        // Review Focus 4: the refusal case must not burn the number.
        let s = Store::open_in_memory().await.unwrap();
        let h = s.begin_sale(cash().sale("00")).await.unwrap();
        s.fail_sale(&h, "894 communication error").await.unwrap();
        let (state, attempts, last_error): (String, i64, Option<String>) =
            sqlx::query_as("select state, attempts, last_error from sale where id = ?")
                .bind(&h.id)
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!(state, "AwaitingVsdc", "a refusal must not change the state");
        assert_eq!(attempts, 1);
        assert_eq!(last_error.as_deref(), Some("894 communication error"));
        assert_eq!(
            s.pending_sale("00").await.unwrap().unwrap().invc_no,
            h.invc_no
        );
        // a second failure counts up without losing the number
        s.fail_sale(&h, "894 again").await.unwrap();
        let attempts: i64 = sqlx::query_scalar("select attempts from sale where id = ?")
            .bind(&h.id)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(attempts, 2);
        // and confirming the same handle afterwards still works, on the same number
        s.confirm_sale(&h, receipt()).await.unwrap();
        assert_eq!(s.pending_sale("00").await.unwrap(), None);
        let invc: i64 = sqlx::query_scalar("select invc_no from sale where id = ?")
            .bind(&h.id)
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(invc, h.invc_no, "the retried sale kept its original number");
    }

    #[tokio::test]
    async fn confirming_twice_with_the_same_receipt_queues_one_row_only() {
        let s = Store::open_in_memory().await.unwrap();
        let h = s.begin_sale(cash().sale("00")).await.unwrap();
        s.confirm_sale(&h, receipt()).await.unwrap();
        s.confirm_sale(&h, receipt()).await.unwrap();
        assert_eq!(count(&s, "select count(*) from outbound").await, 1);
    }

    #[tokio::test]
    async fn confirming_a_confirmed_sale_with_another_receipt_is_refused_and_changes_nothing() {
        let s = Store::open_in_memory().await.unwrap();
        let h = s.begin_sale(cash().sale("00")).await.unwrap();
        s.confirm_sale(&h, receipt()).await.unwrap();
        let other = VsdcReceipt {
            rcpt_no: 43,
            ..receipt()
        };
        let e = s.confirm_sale(&h, other).await.unwrap_err();
        assert!(matches!(e, StoreError::ReceiptConflict { .. }), "{e:?}");
        assert_eq!(count(&s, "select rcpt_no from sale").await, 42);
        assert_eq!(count(&s, "select count(*) from outbound").await, 1);
    }

    #[tokio::test]
    async fn a_confirmed_sale_cannot_be_failed() {
        let s = Store::open_in_memory().await.unwrap();
        let h = s.begin_sale(cash().sale("00")).await.unwrap();
        s.confirm_sale(&h, receipt()).await.unwrap();
        let e = s.fail_sale(&h, "late").await.unwrap_err();
        assert!(matches!(e, StoreError::SaleNotAwaiting { .. }), "{e:?}");
        let last: Option<String> = sqlx::query_scalar("select last_error from sale")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(last, None);
    }

    #[tokio::test]
    async fn an_unknown_handle_is_not_found_not_silently_ok() {
        let s = Store::open_in_memory().await.unwrap();
        let ghost = SaleHandle {
            id: "nope".into(),
            invc_no: 1,
        };
        assert!(matches!(
            s.confirm_sale(&ghost, receipt()).await,
            Err(StoreError::SaleNotFound { .. })
        ));
        assert!(matches!(
            s.fail_sale(&ghost, "x").await,
            Err(StoreError::SaleNotFound { .. })
        ));
        assert_eq!(count(&s, "select count(*) from outbound").await, 0);
    }

    #[tokio::test]
    async fn a_sale_without_lines_is_refused_and_takes_no_number() {
        let s = Store::open_in_memory().await.unwrap();
        let f = cash();
        let empty = Basket::new();
        let e = s
            .begin_sale(NewSale {
                basket: &empty,
                ..f.sale("00")
            })
            .await
            .unwrap_err();
        assert!(matches!(e, StoreError::EmptySale), "{e:?}");
        assert_eq!(count(&s, "select count(*) from sale").await, 0);
        assert_eq!(s.allocate_invc_no("00").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_settlement_that_is_not_this_baskets_is_refused() {
        let s = Store::open_in_memory().await.unwrap();
        let f = cash();
        let wrong = Settled {
            header_pmt_ty_cd: PmtTyCd::Card,
            ..f.settled
        };
        let e = s
            .begin_sale(NewSale {
                settled: &wrong,
                ..f.sale("00")
            })
            .await
            .unwrap_err();
        assert!(matches!(e, StoreError::SettlementMismatch), "{e:?}");
        assert_eq!(count(&s, "select count(*) from sale").await, 0);
    }

    #[tokio::test]
    async fn a_second_sale_is_blocked_until_the_first_is_resolved() {
        let s = Store::open_in_memory().await.unwrap();
        let f = cash();
        let h = s.begin_sale(f.sale("00")).await.unwrap();
        let e = s.begin_sale(f.sale("00")).await.unwrap_err();
        assert!(
            matches!(e, StoreError::SequenceBlocked { invc_no: 1 }),
            "{e:?}"
        );
        s.confirm_sale(&h, receipt()).await.unwrap();
        assert_eq!(s.begin_sale(f.sale("00")).await.unwrap().invc_no, 2);
    }

    #[tokio::test]
    async fn a_failure_part_way_through_writes_no_partial_sale() {
        // The basket API cannot produce a duplicate item_seq (push_line issues
        // them), so the failure is injected: a trigger that aborts the insert
        // of line 2. By then the number is allocated, the sale row written and
        // line 1 inserted, so this really does exercise a rollback.
        let s = Store::open_in_memory().await.unwrap();
        sqlx::query(
            "create trigger boom before insert on sale_line when new.item_seq = 2 \
             begin select raise(abort, 'injected failure'); end",
        )
        .execute(s.pool())
        .await
        .unwrap();
        let e = s.begin_sale(cash().sale("00")).await.unwrap_err();
        assert!(e.to_string().contains("injected failure"), "{e:?}");
        for t in ["sale", "sale_line", "sale_tender", "outbound", "invc_seq"] {
            assert_eq!(
                count(&s, &format!("select count(*) from {t}")).await,
                0,
                "{t}"
            );
        }
        // and the number it would have taken is still available
        assert_eq!(s.allocate_invc_no("00").await.unwrap(), 1);
    }
}
