//! A cashier's shift on one device. The cloud's `Sale.shiftId` is non-null, so
//! a sale can only be recorded (and later synced) inside an open shift.

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
    /// exact `Money`; both queries filter on `sale.shift_id` (`sale_shift`).
    pub async fn shift_totals(&self, shift_id: &str) -> Result<ShiftTotals, StoreError> {
        let float: Option<String> =
            sqlx::query_scalar("select opening_float from shift where id = ?")
                .bind(shift_id)
                .fetch_optional(self.pool())
                .await?;
        let float = float.ok_or_else(|| StoreError::ShiftNotFound {
            id: shift_id.to_string(),
        })?;
        let opening_float = decode_money("opening_float", &float)?;

        let sales: Vec<(String, String, String, String)> =
            sqlx::query_as("select id, state, tot_amt, tot_tax_amt from sale where shift_id = ?")
                .bind(shift_id)
                .fetch_all(self.pool())
                .await?;
        let (mut sale_count, mut voided_count) = (0u32, 0u32);
        let (mut gross, mut tax) = (Money::zero(), Money::zero());
        // sale id -> its tot_amt, for confirmed sales only
        let mut due = std::collections::HashMap::new();
        for (id, state, tot, tot_tax) in sales {
            match state.as_str() {
                "Confirmed" => {
                    sale_count += 1;
                    let tot = decode_money("tot_amt", &tot)?;
                    gross = add(gross, tot, "gross")?;
                    tax = add(tax, decode_money("tot_tax_amt", &tot_tax)?, "tax")?;
                    due.insert(id, tot);
                }
                "Voided" => voided_count += 1,
                _ => {}
            }
        }

        let tenders: Vec<(String, String, String)> = sqlx::query_as(
            "select t.sale_id, t.pmt_ty_cd, t.amount from sale_tender t \
             join sale s on s.id = t.sale_id \
             where s.shift_id = ? and s.state = 'Confirmed'",
        )
        .bind(shift_id)
        .fetch_all(self.pool())
        .await?;
        let mut by_code: std::collections::BTreeMap<&'static str, (PmtTyCd, Money)> =
            std::collections::BTreeMap::new();
        let mut tendered_per_sale: std::collections::HashMap<String, Money> =
            std::collections::HashMap::new();
        for (sale_id, code, amount) in tenders {
            let code = decode_pmt_ty("pmt_ty_cd", &code)?;
            let amount = decode_money("amount", &amount)?;
            let slot = by_code.entry(code.wire()).or_insert((code, Money::zero()));
            slot.1 = add(slot.1, amount, "tender")?;
            let t = tendered_per_sale.entry(sale_id).or_insert_with(Money::zero);
            *t = add(*t, amount, "tender")?;
        }
        let mut change = Money::zero();
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
    async fn totals_queries_search_the_sale_shift_index() {
        let s = Store::open_in_memory().await.unwrap();
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "explain query plan select id, state, tot_amt, tot_tax_amt from sale where shift_id = ?",
        )
        .bind("x")
        .fetch_all(s.pool())
        .await
        .unwrap();
        assert!(plan.iter().any(|r| r.3.contains("sale_shift")), "{plan:?}");
    }
}
