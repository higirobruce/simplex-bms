//! One-shot end-to-end harness: a real sale through the till's Rust code
//! against a LIVE back office.
//!
//! Gated twice so `cargo test` never touches it: it is `#[ignore]`, and it
//! skips unless `SIMPLEX_E2E_URL` and `SIMPLEX_E2E_TOKEN` are set. It lives in
//! the crate (not in `tests/`) because `till-lane` is a binary crate, whose
//! `sync` module an external integration test cannot import.
//!
//! Run (from `till/`):
//!   SIMPLEX_E2E_URL=http://localhost:3000 SIMPLEX_E2E_TOKEN=<lane token> \
//!     cargo test -p till-lane e2e -- --ignored --nocapture

use till_core::codes::PmtTyCd;
use till_core::scan::{Scanned, classify};
use till_core::{Basket, BasketLine, TenderEntry, TenderSet};
use till_store::{Counter, DrainOutcome, NewSale, Store, VsdcReceipt};

use crate::config::SyncConfig;
use crate::demo;
use crate::sync::CloudSync;

const OPENING_FLOAT: &str = "10000";
const CASH: &str = "2000";
const CARD: &str = "2200";

async fn line_for(store: &Store, code: &str) -> BasketLine {
    let scanned = classify(code, demo::config().embedded).expect("scan");
    let p = store
        .lookup(&scanned)
        .await
        .expect("lookup")
        .unwrap_or_else(|| panic!("{code} not in catalogue"));
    let qty = match &scanned {
        Scanned::WeightEmbedded { qty: Some(q), .. } => *q,
        _ => till_core::Qty::try_from_str("1").expect("qty"),
    };
    BasketLine {
        item_seq: 0,
        item_cd: p.sku.clone(),
        item_cls_cd: p.item_cls_cd.clone().expect("item class"),
        item_nm: p.name.clone(),
        bcd: p.barcode.clone(),
        product_id: Some(p.id.clone()),
        qty,
        prc: p.unit_price,
        tax_ty_cd: p.tax_ty_cd,
        discount: None,
    }
}

async fn counts(store: &Store) -> (i64, i64, i64) {
    let q = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(store.pool())
            .await
            .expect("count")
    };
    (
        q("select count(*) from outbound where sent_at is null and parked_at is null").await,
        q("select count(*) from outbound where parked_at is not null").await,
        q("select count(*) from outbound where sent_at is not null").await,
    )
}

#[tokio::test]
#[ignore = "needs a live back office: set SIMPLEX_E2E_URL and SIMPLEX_E2E_TOKEN"]
async fn a_real_sale_lands_in_the_live_back_office_and_replays_idempotently() {
    let (Ok(url), Ok(token)) = (
        std::env::var("SIMPLEX_E2E_URL"),
        std::env::var("SIMPLEX_E2E_TOKEN"),
    ) else {
        eprintln!("SIMPLEX_E2E_URL / SIMPLEX_E2E_TOKEN not set: skipping");
        return;
    };

    // 1. A real file, not memory.
    let dir = std::env::temp_dir().join(format!("till-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("till.db");
    let store = Store::open(&path).await.expect("open store");
    eprintln!("till db: {}", path.display());

    // 2. Seed as main.rs's seed_demo does.
    store
        .sync_catalogue(&demo::products())
        .await
        .expect("catalogue");
    store
        .sync_cashiers(&demo::cashiers())
        .await
        .expect("cashiers");

    // 3. Shift.
    let cfg = demo::config();
    let shift = store
        .open_shift(
            &cfg.dvc_srl_no,
            "demo-cashier",
            till_core::Money::try_from_str(OPENING_FLOAT).expect("float"),
        )
        .await
        .expect("open shift");
    eprintln!("shift {}", shift.id);

    // 4. Four lines, one the weighed tomato via its scale label.
    let mut basket = Basket::new();
    for code in [
        "5612345000017".to_string(),
        "5612345000024".to_string(),
        demo::tomato_label(1250),
        "5449000000996".to_string(),
    ] {
        assert!(code.len() == 13);
        basket
            .push_line(line_for(&store, &code).await)
            .expect("push");
    }
    assert_eq!(demo::tomato_label(1250), "2004087012503");
    let totals = basket.totals().expect("totals");
    eprintln!("total {} (tax {})", totals.tot_amt, totals.tot_tax_amt);
    assert_eq!(totals.tot_amt.to_string(), "4200.00");

    // 5. Split tender.
    let mut tenders = TenderSet::default();
    for (ty, amt) in [(PmtTyCd::Cash, CASH), (PmtTyCd::Card, CARD)] {
        tenders
            .push(TenderEntry {
                pmt_ty_cd: ty,
                amount: till_core::Money::try_from_str(amt).expect("amount"),
            })
            .expect("tender");
    }
    let settled = tenders.settle(totals.tot_amt).expect("settle");
    let handle = store
        .begin_sale(NewSale {
            dvc_srl_no: &cfg.dvc_srl_no,
            shift_id: &shift.id,
            counter: Counter::SaleInvc,
            bhf_id: &cfg.bhf_id,
            basket: &basket,
            tenders: &tenders,
            settled: &settled,
            buyer_tin: None,
        })
        .await
        .expect("begin sale");
    // 6. Confirm (a simulated VSDC receipt, as the demo fiscal port gives).
    store
        .confirm_sale(
            &handle,
            VsdcReceipt {
                rcpt_no: 1,
                intrl_data: format!("e2e-{}", handle.invc_no),
                rcpt_sign: "E2E_SIGNATURE".into(),
                sdc_id: "E2E_SDC".into(),
                mrc_no: "E2E_MRC".into(),
                vsdc_rcpt_pbct_date: "20261122000000".into(),
                tot_rcpt_no: 1,
            },
        )
        .await
        .expect("confirm");
    eprintln!("sale {} invc {}", handle.id, handle.invc_no);

    let rows = store.unsent(50).await.expect("unsent");
    for r in &rows {
        eprintln!("QUEUED #{} {}: {}", r.id, r.kind, r.payload);
    }
    assert_eq!(rows.iter().filter(|r| r.kind == "shift").count(), 1);
    assert_eq!(rows.iter().filter(|r| r.kind == "sale").count(), 1);

    // 7. Real drain.
    let cloud = CloudSync::new(
        store.clone(),
        SyncConfig {
            base_url: url,
            token,
        },
    )
    .expect("client");
    let first = cloud.drain_once().await;
    let after = counts(&store).await;
    eprintln!("DRAIN 1: {first:?}; (unsent, parked, sent) = {after:?}");
    for r in sqlx::query_as::<_, (i64, String, Option<String>)>(
        "select id, kind, last_error from outbound order by id",
    )
    .fetch_all(store.pool())
    .await
    .expect("rows")
    {
        eprintln!("  outbound #{} {} last_error={:?}", r.0, r.1, r.2);
    }
    assert_eq!(
        first,
        DrainOutcome::Sent(2),
        "cloud did not accept both rows"
    );
    assert_eq!(after, (0, 0, 2));

    // 8. Genuine replay: clear sent_at so the same payloads are POSTed again.
    sqlx::query("update outbound set sent_at = null")
        .execute(store.pool())
        .await
        .expect("reset");
    let second = cloud.drain_once().await;
    let after2 = counts(&store).await;
    eprintln!("DRAIN 2 (replay): {second:?}; (unsent, parked, sent) = {after2:?}");
    assert_eq!(second, DrainOutcome::Sent(2), "replay was refused");
    assert_eq!(after2, (0, 0, 2));
    eprintln!("SALE_ID={} SHIFT_ID={}", handle.id, shift.id);
}
