//! The catalogue cache the till prices from, and the offline scan lookup.
//!
//! A scan must never wait on the network (design §7 step 1), so [`Store::lookup`]
//! reads SQLite and nothing else. [`Store::sync_catalogue`] takes products the
//! caller has already fetched and parsed; the HTTP call is not here.
//!
//! # Matching is plain equality, on purpose
//!
//! `till-core`'s `classify` strips leading zeros from a PLU and from a scale
//! label's item ref (the label zero-pads to five digits, a PLU is stored as
//! written) but never from a barcode. So the lookup compares the scanned string
//! to the stored one byte for byte and adds no normalisation of its own on
//! either side. The cost of that is that a PLU stored as `"04011"` could never
//! be reached, so [`Store::sync_catalogue`] REFUSES a non-canonical PLU instead
//! of caching an unscannable product.
//!
//! # What a sync does not do
//!
//! It never deletes. It cannot tell "the cloud delisted this" from "this page
//! of the catalogue was empty or partial", and wiping the cache on the second
//! would stop the shop trading. The price is that a delisted item stays
//! scannable until a deliberate full-replace operation exists (a follow-up that
//! needs the cloud to say the payload is complete).

use serde::{Deserialize, Deserializer};
use sqlx::{Row, SqliteConnection};
use std::str::FromStr;
use till_core::Money;
use till_core::codes::TaxType;
use till_core::scan::Scanned;

use crate::schema::{Store, StoreError, decode_money, decode_tax_ty};

/// UTC timestamp in SQL, so the store never needs a clock crate.
const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%SZ','now')";

/// One product as the till prices it.
///
/// `Money` and `TaxType` deliberately do not implement `Deserialize` (a derive
/// would bypass the 12-digit bound or accept a variant name instead of the RRA
/// code), so the two fields go through the same checked parsers as everything
/// else. Money must arrive as a JSON string: a JSON number would pass through
/// a binary float, which this project never allows.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CachedProduct {
    pub id: String,
    pub sku: String,
    pub name: String,
    pub barcode: Option<String>,
    pub plu: Option<String>,
    #[serde(deserialize_with = "money_from_str")]
    pub unit_price: Money,
    #[serde(deserialize_with = "tax_from_str")]
    pub tax_ty_cd: TaxType,
    pub is_weighed: bool,
    pub item_cls_cd: Option<String>,
}

fn money_from_str<'de, D: Deserializer<'de>>(d: D) -> Result<Money, D::Error> {
    let s = String::deserialize(d)?;
    Money::try_from_str(&s).map_err(serde::de::Error::custom)
}

fn tax_from_str<'de, D: Deserializer<'de>>(d: D) -> Result<TaxType, D::Error> {
    let s = String::deserialize(d)?;
    TaxType::from_str(&s).map_err(serde::de::Error::custom)
}

/// What one sync did. A product id repeated inside one batch counts once as
/// inserted and then as updated, which is what happened to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub inserted: usize,
    pub updated: usize,
}

/// A PLU as `classify` produces it: 1 to 5 digits, no leading zero unless it
/// is the single digit `0`. Anything else can never equal a scanned value.
fn is_canonical_plu(s: &str) -> bool {
    (1..=5).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_digit())
        && (s == "0" || !s.starts_with('0'))
}

fn reject(p: &CachedProduct, reason: &str) -> StoreError {
    StoreError::CatalogueRejected {
        id: p.id.clone(),
        reason: reason.to_string(),
    }
}

fn validate(p: &CachedProduct) -> Result<(), StoreError> {
    if p.id.is_empty() {
        return Err(reject(p, "empty id"));
    }
    if p.barcode.as_deref() == Some("") {
        return Err(reject(p, "empty barcode"));
    }
    if let Some(plu) = &p.plu
        && !is_canonical_plu(plu)
    {
        return Err(reject(
            p,
            "PLU must be 1-5 digits without leading zeros, as a scan produces it",
        ));
    }
    Ok(())
}

impl Store {
    /// Upserts `products` by `id` in ONE transaction: either the whole
    /// catalogue moves to the new prices or none of it does, so the till never
    /// prices some goods from today and some from last week.
    ///
    /// `BEGIN IMMEDIATE` takes the write lock first, which is also what makes
    /// the inserted/updated split exact: the existence check and the upsert
    /// cannot be interleaved with another writer. (`ON CONFLICT ... RETURNING`
    /// cannot tell an insert from an update in SQLite.) Any error, including a
    /// barcode claimed by two products, returns before `commit`, and dropping
    /// the transaction rolls everything back.
    pub async fn sync_catalogue(
        &self,
        products: &[CachedProduct],
    ) -> Result<SyncReport, StoreError> {
        for p in products {
            validate(p)?;
        }
        let mut tx = self.begin_immediate().await?;
        let report = upsert_all(tx.conn(), products).await?;
        tx.commit().await?;
        Ok(report)
    }

    /// Resolves a scan to a product from the local cache only.
    ///
    /// `None` means the cache holds nothing for it (a normal outcome: the
    /// cashier is told, nothing is guessed). A `WeightEmbedded` scan is matched
    /// by its `item_ref` against `plu`, exactly like a keyed PLU; its weight or
    /// price is the caller's business. Two products on one PLU is an error, not
    /// an arbitrary pick.
    pub async fn lookup(&self, scanned: &Scanned) -> Result<Option<CachedProduct>, StoreError> {
        match scanned {
            Scanned::Barcode(b) => {
                let rows = sqlx::query(&format!("{SELECT} where barcode = ?"))
                    .bind(b)
                    .fetch_all(self.pool())
                    .await?;
                // barcode is UNIQUE, so at most one row
                rows.first().map(decode_product).transpose()
            }
            Scanned::Plu(plu) | Scanned::WeightEmbedded { item_ref: plu, .. } => {
                // `plu is not null` restates the partial index's predicate so
                // the planner can always use it (a test checks the plan).
                let rows = sqlx::query(&format!(
                    "{SELECT} where plu = ? and plu is not null limit 2"
                ))
                .bind(plu)
                .fetch_all(self.pool())
                .await?;
                match rows.as_slice() {
                    [] => Ok(None),
                    [one] => decode_product(one).map(Some),
                    _ => Err(StoreError::AmbiguousPlu { plu: plu.clone() }),
                }
            }
        }
    }
}

const SELECT: &str = "select id, sku, name, barcode, plu, unit_price, tax_ty_cd, is_weighed, \
                      item_cls_cd from product";

async fn upsert_all(
    conn: &mut SqliteConnection,
    products: &[CachedProduct],
) -> Result<SyncReport, StoreError> {
    let (mut inserted, mut updated) = (0usize, 0usize);
    for p in products {
        let existed: i64 = sqlx::query_scalar("select count(*) from product where id = ?")
            .bind(&p.id)
            .fetch_one(&mut *conn)
            .await?;
        // `department` is not part of the cached product, so it is left alone
        // on update rather than blanked.
        sqlx::query(&format!(
            "insert into product (id, sku, name, barcode, plu, unit_price, tax_ty_cd, \
             is_weighed, item_cls_cd, synced_at) values (?, ?, ?, ?, ?, ?, ?, ?, ?, {NOW_SQL}) \
             on conflict (id) do update set sku = excluded.sku, name = excluded.name, \
             barcode = excluded.barcode, plu = excluded.plu, unit_price = excluded.unit_price, \
             tax_ty_cd = excluded.tax_ty_cd, is_weighed = excluded.is_weighed, \
             item_cls_cd = excluded.item_cls_cd, synced_at = excluded.synced_at"
        ))
        .bind(&p.id)
        .bind(&p.sku)
        .bind(&p.name)
        .bind(&p.barcode)
        .bind(&p.plu)
        .bind(p.unit_price.to_string())
        .bind(p.tax_ty_cd.wire())
        .bind(i64::from(p.is_weighed))
        .bind(&p.item_cls_cd)
        .execute(&mut *conn)
        .await?;
        if existed > 0 {
            updated += 1;
        } else {
            inserted += 1;
        }
    }
    // PLU is not UNIQUE in the schema (and cannot be changed), so the
    // invariant is enforced here, over the whole table, before commit.
    let dup: Option<String> = sqlx::query_scalar(
        "select plu from product where plu is not null group by plu having count(*) > 1 limit 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(plu) = dup {
        return Err(StoreError::AmbiguousPlu { plu });
    }
    Ok(SyncReport { inserted, updated })
}

fn decode_product(row: &sqlx::sqlite::SqliteRow) -> Result<CachedProduct, StoreError> {
    let is_weighed = match row.try_get::<i64, _>("is_weighed")? {
        0 => false,
        1 => true,
        n => return Err(StoreError::Decode(format!("is_weighed: {n}"))),
    };
    Ok(CachedProduct {
        id: row.try_get("id")?,
        sku: row.try_get("sku")?,
        name: row.try_get("name")?,
        barcode: row.try_get("barcode")?,
        plu: row.try_get("plu")?,
        unit_price: decode_money("unit_price", &row.try_get::<String, _>("unit_price")?)?,
        tax_ty_cd: decode_tax_ty("tax_ty_cd", &row.try_get::<String, _>("tax_ty_cd")?)?,
        is_weighed,
        item_cls_cd: row.try_get("item_cls_cd")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prod(
        id: &str,
        barcode: Option<&str>,
        plu: Option<&str>,
        price: &str,
        tax: TaxType,
    ) -> CachedProduct {
        CachedProduct {
            id: id.into(),
            sku: format!("SKU-{id}"),
            name: format!("Name {id}"),
            barcode: barcode.map(Into::into),
            plu: plu.map(Into::into),
            unit_price: Money::try_from_str(price).unwrap(),
            tax_ty_cd: tax,
            is_weighed: plu.is_some(),
            item_cls_cd: None,
        }
    }

    async fn count(s: &Store) -> i64 {
        sqlx::query_scalar("select count(*) from product")
            .fetch_one(s.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn syncing_inserts_then_updates_by_id() {
        let s = Store::open_in_memory().await.unwrap();
        let r = s
            .sync_catalogue(&[prod("p1", Some("4006381333931"), None, "1200", TaxType::A)])
            .await
            .unwrap();
        assert_eq!((r.inserted, r.updated), (1, 0));
        let r = s
            .sync_catalogue(&[prod("p1", Some("4006381333931"), None, "1300", TaxType::A)])
            .await
            .unwrap();
        assert_eq!((r.inserted, r.updated), (0, 1));
        let got = s
            .lookup(&Scanned::Barcode("4006381333931".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.unit_price.to_string(), "1300.00");
    }

    #[tokio::test]
    async fn a_plu_and_a_weight_embedded_ref_resolve_to_the_same_product() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("p2", None, Some("23456"), "1850", TaxType::B)])
            .await
            .unwrap();
        let by_plu = s
            .lookup(&Scanned::Plu("23456".into()))
            .await
            .unwrap()
            .unwrap();
        let by_embedded = s
            .lookup(&Scanned::WeightEmbedded {
                item_ref: "23456".into(),
                qty: None,
                price: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_plu.id, by_embedded.id);
    }

    #[tokio::test]
    async fn a_stripped_item_ref_reaches_the_plu_stored_as_written() {
        // classify turns a label's "04011" into "4011"; the cache holds "4011"
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("p", None, Some("4011"), "500", TaxType::B)])
            .await
            .unwrap();
        let w = Scanned::WeightEmbedded {
            item_ref: "4011".into(),
            qty: None,
            price: None,
        };
        assert_eq!(s.lookup(&w).await.unwrap().unwrap().id, "p");
        // the unstripped form is not a value classify ever produces, and is not matched
        let raw = Scanned::WeightEmbedded {
            item_ref: "04011".into(),
            qty: None,
            price: None,
        };
        assert!(s.lookup(&raw).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_unknown_scan_is_none_not_an_error() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(
            s.lookup(&Scanned::Barcode("9999999999999".into()))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            s.lookup(&Scanned::Plu("1234".into()))
                .await
                .unwrap()
                .is_none()
        );
        let w = Scanned::WeightEmbedded {
            item_ref: "1234".into(),
            qty: None,
            price: None,
        };
        assert!(s.lookup(&w).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_leading_zero_barcode_is_matched_verbatim_and_not_by_its_stripped_form() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("b", Some("0012345678905"), None, "100", TaxType::B)])
            .await
            .unwrap();
        assert!(
            s.lookup(&Scanned::Barcode("0012345678905".into()))
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            s.lookup(&Scanned::Barcode("12345678905".into()))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_rejected_product_rolls_back_the_whole_sync() {
        let s = Store::open_in_memory().await.unwrap();
        let dup = [
            prod("p3", Some("4006381333931"), None, "100", TaxType::B),
            prod("p4", Some("4006381333931"), None, "200", TaxType::B),
        ];
        assert!(s.sync_catalogue(&dup).await.is_err());
        assert_eq!(count(&s).await, 0);
    }

    #[tokio::test]
    async fn a_failed_sync_leaves_existing_rows_and_prices_untouched() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("p1", Some("4006381333931"), None, "1200", TaxType::A)])
            .await
            .unwrap();
        // p1 would be repriced and p9 added before p8 collides with p1's barcode
        let batch = [
            prod("p1", Some("4006381333931"), None, "9999", TaxType::B),
            prod("p9", Some("96385074"), None, "50", TaxType::B),
            prod("p8", Some("4006381333931"), None, "60", TaxType::B),
        ];
        assert!(matches!(
            s.sync_catalogue(&batch).await,
            Err(StoreError::Sqlx(_))
        ));
        assert_eq!(count(&s).await, 1);
        let p1 = s
            .lookup(&Scanned::Barcode("4006381333931".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (p1.unit_price.to_string().as_str(), p1.tax_ty_cd),
            ("1200.00", TaxType::A)
        );
        // and the store is usable afterwards: no transaction left open
        s.sync_catalogue(&[prod("p9", Some("96385074"), None, "50", TaxType::B)])
            .await
            .unwrap();
        assert_eq!(count(&s).await, 2);
    }

    #[tokio::test]
    async fn an_invalid_product_late_in_a_batch_lands_nothing() {
        let s = Store::open_in_memory().await.unwrap();
        let batch = [
            prod("ok", Some("96385074"), None, "50", TaxType::B),
            prod("bad", None, Some("04011"), "50", TaxType::B),
        ];
        assert!(matches!(
            s.sync_catalogue(&batch).await,
            Err(StoreError::CatalogueRejected { .. })
        ));
        assert_eq!(count(&s).await, 0);
    }

    #[tokio::test]
    async fn non_canonical_plus_and_empty_barcodes_are_refused() {
        let s = Store::open_in_memory().await.unwrap();
        for plu in ["04011", "", "123456", "12a4", "-123"] {
            let r = s
                .sync_catalogue(&[prod("x", None, Some(plu), "1", TaxType::B)])
                .await;
            assert!(
                matches!(r, Err(StoreError::CatalogueRejected { .. })),
                "{plu:?}"
            );
        }
        assert!(
            s.sync_catalogue(&[prod("z", None, Some("0"), "1", TaxType::B)])
                .await
                .is_ok()
        );
        let r = s
            .sync_catalogue(&[prod("y", Some(""), None, "1", TaxType::B)])
            .await;
        assert!(matches!(r, Err(StoreError::CatalogueRejected { .. })));
    }

    #[tokio::test]
    async fn two_products_on_one_plu_are_refused_at_sync_and_at_lookup() {
        let s = Store::open_in_memory().await.unwrap();
        let r = s
            .sync_catalogue(&[
                prod("a", None, Some("4011"), "1", TaxType::B),
                prod("b", None, Some("4011"), "2", TaxType::B),
            ])
            .await;
        assert!(matches!(r, Err(StoreError::AmbiguousPlu { .. })));
        assert_eq!(count(&s).await, 0);
        // a table written around the sync still cannot yield a guess
        for id in ["a", "b"] {
            sqlx::query("insert into product (id, sku, name, plu, unit_price, tax_ty_cd, is_weighed, synced_at) values (?, 'S', 'N', '4011', '1', 'B', 0, 'x')")
                .bind(id)
                .execute(s.pool())
                .await
                .unwrap();
        }
        assert!(matches!(
            s.lookup(&Scanned::Plu("4011".into())).await,
            Err(StoreError::AmbiguousPlu { .. })
        ));
    }

    #[tokio::test]
    async fn an_empty_sync_deletes_nothing() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("p1", Some("96385074"), None, "1", TaxType::B)])
            .await
            .unwrap();
        let r = s.sync_catalogue(&[]).await.unwrap();
        assert_eq!((r.inserted, r.updated), (0, 0));
        assert_eq!(count(&s).await, 1);
    }

    #[tokio::test]
    async fn an_update_keeps_the_department_it_does_not_carry() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("p1", Some("96385074"), None, "1", TaxType::B)])
            .await
            .unwrap();
        sqlx::query("update product set department = 'Dairy' where id = 'p1'")
            .execute(s.pool())
            .await
            .unwrap();
        s.sync_catalogue(&[prod("p1", Some("96385074"), None, "2", TaxType::B)])
            .await
            .unwrap();
        let d: Option<String> =
            sqlx::query_scalar("select department from product where id = 'p1'")
                .fetch_one(s.pool())
                .await
                .unwrap();
        assert_eq!(d.as_deref(), Some("Dairy"));
    }

    #[tokio::test]
    async fn an_unparsable_price_in_the_cache_is_reported_not_silently_zeroed() {
        let s = Store::open_in_memory().await.unwrap();
        sqlx::query("insert into product (id, sku, name, barcode, unit_price, tax_ty_cd, is_weighed, synced_at) values ('p5','S','N','96385074','not-a-number','B',0,'2026-11-21T00:00:00Z')")
            .execute(s.pool()).await.unwrap();
        assert!(matches!(
            s.lookup(&Scanned::Barcode("96385074".into())).await,
            Err(StoreError::Decode(_))
        ));
    }

    #[tokio::test]
    async fn an_unknown_stored_tax_code_is_reported() {
        let s = Store::open_in_memory().await.unwrap();
        sqlx::query("insert into product (id, sku, name, barcode, unit_price, tax_ty_cd, is_weighed, synced_at) values ('p6','S','N','96385074','1.00','Z',0,'x')")
            .execute(s.pool()).await.unwrap();
        assert!(matches!(
            s.lookup(&Scanned::Barcode("96385074".into())).await,
            Err(StoreError::Decode(_))
        ));
    }

    #[tokio::test]
    async fn the_plu_lookup_uses_the_partial_index_and_the_barcode_lookup_its_unique_index() {
        let s = Store::open_in_memory().await.unwrap();
        let plan = |sql: String| {
            let pool = s.pool().clone();
            async move {
                sqlx::query(&format!("explain query plan {sql}"))
                    .bind("1")
                    .fetch_all(&pool)
                    .await
                    .unwrap()
                    .iter()
                    .map(|r| r.get::<String, _>("detail"))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        };
        let p = plan(format!(
            "{SELECT} where plu = ? and plu is not null limit 2"
        ))
        .await;
        assert!(p.contains("product_plu"), "{p}");
        let b = plan(format!("{SELECT} where barcode = ?")).await;
        assert!(b.contains("USING INDEX") || b.contains("autoindex"), "{b}");
    }

    #[test]
    fn products_deserialise_through_the_checked_parsers() {
        let ok = r#"{"id":"p","sku":"s","name":"n","barcode":null,"plu":"4011",
            "unit_price":"1200.50","tax_ty_cd":"B","is_weighed":true,"item_cls_cd":null}"#;
        let p: CachedProduct = serde_json::from_str(ok).unwrap();
        assert_eq!(p.unit_price.to_string(), "1200.50");
        assert_eq!(p.tax_ty_cd, TaxType::B);
        // a JSON number would travel through a float: refused
        assert!(
            serde_json::from_str::<CachedProduct>(&ok.replace("\"1200.50\"", "1200.5")).is_err()
        );
        assert!(serde_json::from_str::<CachedProduct>(&ok.replace("\"B\"", "\"Z\"")).is_err());
        assert!(serde_json::from_str::<CachedProduct>(&ok.replace("1200.50", "1.234")).is_err());
    }
}
