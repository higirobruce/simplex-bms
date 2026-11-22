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

use std::collections::{HashMap, HashSet};

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
    // Zero is allowed: a free item is legitimate.
    if p.unit_price < Money::zero() {
        return Err(StoreError::NegativeCataloguePrice { id: p.id.clone() });
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
    /// the inserted/updated split exact: the existence pre-pass and the upsert
    /// cannot be interleaved with another writer. (`ON CONFLICT ... RETURNING`
    /// cannot tell an insert from an update in SQLite.) Any error returns
    /// before `commit`, and dropping the transaction rolls everything back.
    ///
    /// The write lock is the lane's: a sale cannot begin while it is held, and
    /// a sale that waits past the 2 s busy timeout fails with
    /// [`StoreError::Busy`]. So everything that can be decided without the
    /// database (validation, two batch products claiming one barcode) is done
    /// BEFORE the transaction, and inside it the batch costs a constant number
    /// of statements, not several per product. It is still ONE transaction:
    /// all-or-nothing matters more than throughput.
    pub async fn sync_catalogue(
        &self,
        products: &[CachedProduct],
    ) -> Result<SyncReport, StoreError> {
        for p in products {
            validate(p)?;
        }
        check_batch_barcodes(products)?;
        let mut tx = self.begin_immediate().await?;
        let (report, _statements) = upsert_all(tx.conn(), products).await?;
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

/// The shortest search term accepted. One letter matches most of a catalogue
/// and tells the cashier nothing.
pub const MIN_SEARCH_LEN: usize = 2;

/// Escapes a cashier's typing so LIKE treats it as literal text. Without this
/// a name containing `%` would match the whole catalogue, and `_` would match
/// any character.
fn escape_like(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Store {
    /// Products whose name contains `term`, for the cashier's name search.
    ///
    /// Names starting with the term come first, then the rest alphabetically,
    /// because a cashier typing "co" wants Coca-Cola before Chocolate Cocoa.
    /// At most `limit` rows: the list is read off a screen, not paged.
    ///
    /// Matching is SQLite's ASCII-only case folding. Good enough for a Rwandan
    /// catalogue in practice; a name differing only by non-ASCII case will not
    /// be found, which is a miss, never a wrong product.
    pub async fn search_by_name(
        &self,
        term: &str,
        limit: u32,
    ) -> Result<Vec<CachedProduct>, StoreError> {
        let term = term.trim();
        if term.chars().count() < MIN_SEARCH_LEN {
            return Ok(vec![]);
        }
        let escaped = escape_like(term);
        let contains = format!("%{escaped}%");
        let starts = format!("{escaped}%");
        let rows = sqlx::query(&format!(
            "{SELECT} where name like ? escape '\\' \
             order by (case when name like ? escape '\\' then 0 else 1 end), name limit ?"
        ))
        .bind(&contains)
        .bind(&starts)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(decode_product).collect()
    }
}

/// Finds two DIFFERENT products in one batch claiming one barcode, in memory,
/// before the write lock is taken. Mirrors what the statements would do in
/// order: an id that appears again replaces its own earlier claim (so
/// `x:b1, x:b2, y:b1` is legal, exactly as sequential upserts would leave it),
/// and only a barcode held by another id is a conflict.
fn check_batch_barcodes(products: &[CachedProduct]) -> Result<(), StoreError> {
    let mut owner: HashMap<&str, &str> = HashMap::new();
    let mut current: HashMap<&str, &str> = HashMap::new();
    for p in products {
        if let Some(old) = current.remove(p.id.as_str()) {
            owner.remove(old);
        }
        if let Some(b) = p.barcode.as_deref() {
            if let Some(other) = owner.get(b) {
                return Err(StoreError::DuplicateBarcodeInBatch {
                    barcode: b.to_string(),
                    first_id: (*other).to_string(),
                    second_id: p.id.clone(),
                });
            }
            owner.insert(b, p.id.as_str());
            current.insert(p.id.as_str(), b);
        }
    }
    Ok(())
}

/// The whole batch in a CONSTANT number of statements. Each takes the batch as
/// one JSON array bound once and unpacked with `json_each`, which also avoids
/// SQLite's bound-variable limit. A per-product loop (the previous shape) held
/// the write lock for seconds at tens of thousands of products and could make
/// a sale fail with `SQLITE_BUSY`; the per-row work now happens inside SQLite
/// instead of one round trip per row.
///
/// Returns the number of statements it ran (counted beside each one) so a test
/// can assert that it is the same for 1 product and for 500.
async fn upsert_all(
    conn: &mut SqliteConnection,
    products: &[CachedProduct],
) -> Result<(SyncReport, usize), StoreError> {
    let ids = serde_json::to_string(&products.iter().map(|p| &p.id).collect::<Vec<_>>())?;
    let rows = serde_json::to_string(
        &products
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id, "sku": p.sku, "name": p.name, "barcode": p.barcode,
                    "plu": p.plu, "unit_price": p.unit_price.to_string(),
                    "tax_ty_cd": p.tax_ty_cd.wire(), "is_weighed": i64::from(p.is_weighed),
                    "item_cls_cd": p.item_cls_cd,
                })
            })
            .collect::<Vec<_>>(),
    )?;
    let mut statements = 0usize;

    // Statement 1: which ids already exist. One pre-pass replaces a
    // `select count(*)` per product and keeps inserted/updated exact: the lock
    // is held, so nothing can change between this and the upsert.
    statements += 1;
    let existing: HashSet<String> =
        sqlx::query_scalar("select id from product where id in (select value from json_each(?))")
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();
    let (mut inserted, mut updated) = (0usize, 0usize);
    let mut seen: HashSet<&str> = HashSet::new();
    for p in products {
        // A repeated id is an insert the first time and an update after.
        if existing.contains(&p.id) || !seen.insert(p.id.as_str()) {
            updated += 1;
        } else {
            inserted += 1;
        }
    }

    // Statement 2, phase 1: release every barcode the batch's products hold.
    // `barcode` is UNIQUE and rows are checked one at a time, so two products
    // exchanging barcodes (or a longer rotation) would otherwise collide
    // mid-statement, and no retry could ever succeed. Two batch products
    // claiming one barcode were refused in memory before the lock; a collision
    // with a product OUTSIDE the batch is still refused by the upsert. A later
    // failure rolls this clear back with everything else.
    statements += 1;
    sqlx::query("update product set barcode = null where id in (select value from json_each(?))")
        .bind(&ids)
        .execute(&mut *conn)
        .await?;

    // Statement 3: the upsert. `department` is not part of the cached product,
    // so it is left alone on update rather than blanked. (`where true` is what
    // lets SQLite parse `on conflict` after an `insert ... select`.)
    statements += 1;
    sqlx::query(&format!(
        "insert into product (id, sku, name, barcode, plu, unit_price, tax_ty_cd, \
             is_weighed, item_cls_cd, synced_at) \
             select json_extract(value, '$.id'), json_extract(value, '$.sku'), \
             json_extract(value, '$.name'), json_extract(value, '$.barcode'), \
             json_extract(value, '$.plu'), json_extract(value, '$.unit_price'), \
             json_extract(value, '$.tax_ty_cd'), json_extract(value, '$.is_weighed'), \
             json_extract(value, '$.item_cls_cd'), {NOW_SQL} from json_each(?) where true \
             on conflict (id) do update set sku = excluded.sku, name = excluded.name, \
             barcode = excluded.barcode, plu = excluded.plu, unit_price = excluded.unit_price, \
             tax_ty_cd = excluded.tax_ty_cd, is_weighed = excluded.is_weighed, \
             item_cls_cd = excluded.item_cls_cd, synced_at = excluded.synced_at"
    ))
    .bind(&rows)
    .execute(&mut *conn)
    .await?;

    // Statement 4: PLU is not UNIQUE in the schema (and cannot be changed), so
    // the invariant is enforced here, over the whole table, before commit.
    statements += 1;
    let dup: Option<String> = sqlx::query_scalar(
        "select plu from product where plu is not null group by plu having count(*) > 1 limit 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(plu) = dup {
        return Err(StoreError::AmbiguousPlu { plu });
    }
    Ok((SyncReport { inserted, updated }, statements))
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

    /// A product with a chosen name, for the search tests.
    fn named(id: &str, name: &str) -> CachedProduct {
        CachedProduct {
            name: name.into(),
            ..prod(id, None, None, "1000", TaxType::B)
        }
    }

    async fn search(s: &Store, term: &str) -> Vec<String> {
        s.search_by_name(term, 20)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    #[tokio::test]
    async fn a_name_search_matches_anywhere_in_the_name_ignoring_case() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[
            named("p1", "Inyange Fresh Milk 1L"),
            named("p2", "Kabuye Sugar 1kg"),
            named("p3", "Powdered milk 400g"),
        ])
        .await
        .unwrap();

        assert_eq!(
            search(&s, "milk").await,
            vec!["Inyange Fresh Milk 1L", "Powdered milk 400g"],
            "case must not matter and the term may sit anywhere in the name"
        );
        assert_eq!(search(&s, "MILK").await.len(), 2);
        assert_eq!(search(&s, "sugar").await, vec!["Kabuye Sugar 1kg"]);
        assert!(search(&s, "zzz").await.is_empty());
    }

    #[tokio::test]
    async fn names_starting_with_the_term_come_first() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[
            named("p1", "Chocolate with Cocoa"),
            named("p2", "Coca-Cola 500ml"),
        ])
        .await
        .unwrap();
        // "Chocolate" sorts first alphabetically, but a cashier typing "co"
        // wants the product whose name begins with it.
        assert_eq!(
            search(&s, "co").await,
            vec!["Coca-Cola 500ml", "Chocolate with Cocoa"]
        );
    }

    /// A cashier typing a name that happens to contain LIKE's wildcards must
    /// get a literal search. Unescaped, "%" would match the whole catalogue.
    #[tokio::test]
    async fn like_wildcards_typed_by_a_cashier_are_literal_text() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[
            named("p1", "Juice 100% Orange"),
            named("p2", "Kabuye Sugar 1kg"),
            named("p3", "Soap_bar"),
        ])
        .await
        .unwrap();

        assert_eq!(
            search(&s, "100%").await,
            vec!["Juice 100% Orange"],
            "a literal percent must not become a wildcard"
        );
        assert_eq!(
            search(&s, "p_b").await,
            vec!["Soap_bar"],
            "a literal underscore must not match any character"
        );
        // Proof the underscore really is literal: this would match "Soap_bar"
        // if `_` were still a single-character wildcard.
        assert!(search(&s, "pXb").await.is_empty());
    }

    #[tokio::test]
    async fn a_term_shorter_than_the_minimum_returns_nothing() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[named("p1", "Milk")]).await.unwrap();
        assert!(
            search(&s, "m").await.is_empty(),
            "one letter is not a search"
        );
        assert!(search(&s, " ").await.is_empty());
        assert_eq!(search(&s, "mi").await, vec!["Milk"]);
    }

    #[tokio::test]
    async fn a_search_never_returns_more_than_the_limit() {
        let s = Store::open_in_memory().await.unwrap();
        let many: Vec<CachedProduct> = (0..30)
            .map(|i| named(&format!("p{i}"), &format!("Milk {i:02}")))
            .collect();
        s.sync_catalogue(&many).await.unwrap();
        assert_eq!(s.search_by_name("milk", 5).await.unwrap().len(), 5);
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
        // Changed: this used to assert only `is_err()` (the failure was an
        // opaque UNIQUE violation mid-transaction). The batch is now refused in
        // memory before the lock, with a named error naming both products.
        match s.sync_catalogue(&dup).await {
            Err(StoreError::DuplicateBarcodeInBatch {
                barcode,
                first_id,
                second_id,
            }) => {
                assert_eq!(barcode, "4006381333931");
                assert_eq!((first_id.as_str(), second_id.as_str()), ("p3", "p4"));
            }
            other => panic!("expected DuplicateBarcodeInBatch, got {other:?}"),
        }
        assert_eq!(count(&s).await, 0);
    }

    #[tokio::test]
    async fn a_failed_sync_leaves_existing_rows_and_prices_untouched() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("p1", Some("4006381333931"), None, "1200", TaxType::A)])
            .await
            .unwrap();
        // Changed: p8 used to collide with p1 INSIDE the batch, which is now
        // refused in memory before any write, so that no longer exercised the
        // rollback. The collision is now with a product OUTSIDE the batch, so
        // p1 is repriced and p9 added before the upsert statement fails.
        s.sync_catalogue(&[prod("out", Some("5449000000996"), None, "10", TaxType::B)])
            .await
            .unwrap();
        let batch = [
            prod("p1", Some("4006381333931"), None, "9999", TaxType::B),
            prod("p9", Some("96385074"), None, "50", TaxType::B),
            prod("p8", Some("5449000000996"), None, "60", TaxType::B),
        ];
        assert!(matches!(
            s.sync_catalogue(&batch).await,
            Err(StoreError::Sqlx(_))
        ));
        assert_eq!(count(&s).await, 2);
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
        assert_eq!(count(&s).await, 3);
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
    async fn two_products_can_exchange_barcodes_in_one_sync() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[
            prod("a", Some("4006381333931"), None, "100", TaxType::B),
            prod("b", Some("96385074"), None, "200", TaxType::B),
        ])
        .await
        .unwrap();
        let r = s
            .sync_catalogue(&[
                prod("a", Some("96385074"), None, "100", TaxType::B),
                prod("b", Some("4006381333931"), None, "200", TaxType::B),
            ])
            .await
            .unwrap();
        assert_eq!((r.inserted, r.updated), (0, 2));
        let a = s
            .lookup(&Scanned::Barcode("96385074".into()))
            .await
            .unwrap()
            .unwrap();
        let b = s
            .lookup(&Scanned::Barcode("4006381333931".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((a.id.as_str(), b.id.as_str()), ("a", "b"));
    }

    #[tokio::test]
    async fn a_three_way_barcode_rotation_succeeds() {
        let s = Store::open_in_memory().await.unwrap();
        let codes = ["4006381333931", "96385074", "5901234123457"];
        let ids = ["a", "b", "c"];
        let at = |shift: usize| -> Vec<CachedProduct> {
            (0..3)
                .map(|i| prod(ids[i], Some(codes[(i + shift) % 3]), None, "1", TaxType::B))
                .collect()
        };
        s.sync_catalogue(&at(0)).await.unwrap();
        s.sync_catalogue(&at(1)).await.unwrap();
        for i in 0..3 {
            let got = s
                .lookup(&Scanned::Barcode(codes[(i + 1) % 3].into()))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(got.id, ids[i]);
        }
    }

    #[tokio::test]
    async fn a_failure_after_the_barcode_clear_restores_the_original_barcodes() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[
            prod("a", Some("4006381333931"), None, "100", TaxType::B),
            prod("b", Some("96385074"), None, "200", TaxType::B),
        ])
        .await
        .unwrap();
        // swap (clears both barcodes first), then an invalid late product:
        // two products claiming one PLU fails at the end-of-sync check
        let batch = [
            prod("a", Some("96385074"), Some("4011"), "999", TaxType::B),
            prod("b", Some("4006381333931"), Some("4011"), "999", TaxType::B),
        ];
        assert!(matches!(
            s.sync_catalogue(&batch).await,
            Err(StoreError::AmbiguousPlu { .. })
        ));
        let a = s
            .lookup(&Scanned::Barcode("4006381333931".into()))
            .await
            .unwrap()
            .unwrap();
        let b = s
            .lookup(&Scanned::Barcode("96385074".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (a.id.as_str(), a.unit_price.to_string().as_str()),
            ("a", "100.00")
        );
        assert_eq!(
            (b.id.as_str(), b.unit_price.to_string().as_str()),
            ("b", "200.00")
        );
        let nulls: i64 = sqlx::query_scalar("select count(*) from product where barcode is null")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(nulls, 0);
    }

    #[tokio::test]
    async fn a_barcode_held_by_a_product_outside_the_batch_is_still_refused() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("a", Some("4006381333931"), None, "100", TaxType::B)])
            .await
            .unwrap();
        let r = s
            .sync_catalogue(&[prod("z", Some("4006381333931"), None, "1", TaxType::B)])
            .await;
        assert!(matches!(r, Err(StoreError::Sqlx(_))));
    }

    #[tokio::test]
    async fn a_negative_catalogue_price_is_refused_by_name_and_the_cache_untouched() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("ok", Some("96385074"), None, "50", TaxType::B)])
            .await
            .unwrap();
        let batch = [
            prod("ok", Some("96385074"), None, "99", TaxType::B),
            prod("neg", None, None, "-5", TaxType::B),
        ];
        assert!(matches!(
            s.sync_catalogue(&batch).await,
            Err(StoreError::NegativeCataloguePrice { ref id }) if id == "neg"
        ));
        assert_eq!(count(&s).await, 1);
        let p = s
            .lookup(&Scanned::Barcode("96385074".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(p.unit_price.to_string(), "50.00");
    }

    #[tokio::test]
    async fn a_zero_catalogue_price_is_accepted() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[prod("free", Some("96385074"), None, "0", TaxType::B)])
            .await
            .unwrap();
        let p = s
            .lookup(&Scanned::Barcode("96385074".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(p.unit_price.to_string(), "0.00");
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

    #[tokio::test]
    async fn the_same_id_twice_in_one_batch_is_not_a_duplicate_barcode() {
        // x:b1 then x:b2 then y:b1 leaves x on b2 and y on b1 when applied in
        // order, so it must not be refused; x:b1 then y:b1 must be.
        let s = Store::open_in_memory().await.unwrap();
        let r = s
            .sync_catalogue(&[
                prod("x", Some("4006381333931"), None, "1", TaxType::B),
                prod("x", Some("96385074"), None, "1", TaxType::B),
                prod("y", Some("4006381333931"), None, "1", TaxType::B),
            ])
            .await
            .unwrap();
        assert_eq!((r.inserted, r.updated), (2, 1));
        let x = s
            .lookup(&Scanned::Barcode("96385074".into()))
            .await
            .unwrap()
            .unwrap();
        let y = s
            .lookup(&Scanned::Barcode("4006381333931".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((x.id.as_str(), y.id.as_str()), ("x", "y"));
    }

    #[tokio::test]
    async fn counts_stay_exact_across_a_mixed_batch() {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_catalogue(&[
            prod("a", Some("4006381333931"), None, "1", TaxType::B),
            prod("b", None, None, "1", TaxType::B),
        ])
        .await
        .unwrap();
        let r = s
            .sync_catalogue(&[
                prod("a", Some("4006381333931"), None, "2", TaxType::B),
                prod("c", None, None, "1", TaxType::B),
                prod("d", None, None, "1", TaxType::B),
            ])
            .await
            .unwrap();
        assert_eq!((r.inserted, r.updated), (2, 1));
    }

    #[tokio::test]
    async fn special_characters_and_nulls_survive_the_json_batch() {
        let s = Store::open_in_memory().await.unwrap();
        let mut p = prod("q", None, None, "12.50", TaxType::B);
        p.name = "Inyama \"y'inka\" \\ é 🥩".into();
        p.item_cls_cd = Some("5020230602".into());
        s.sync_catalogue(&[p.clone()]).await.unwrap();
        let (name, bc, plu, w, cls): (String, Option<String>, Option<String>, i64, Option<String>) =
            sqlx::query_as(
                "select name, barcode, plu, is_weighed, item_cls_cd from product where id='q'",
            )
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(name, p.name);
        assert_eq!((bc, plu, w), (None, None, 0));
        assert_eq!(cls.as_deref(), Some("5020230602"));
    }

    #[tokio::test]
    async fn statements_under_the_lock_do_not_grow_with_the_batch() {
        let s = Store::open_in_memory().await.unwrap();
        let batch = |n: usize, tag: &str| -> Vec<CachedProduct> {
            (0..n)
                .map(|i| {
                    prod(
                        &format!("{tag}{i}"),
                        Some(&format!("{tag}-bc-{i}")),
                        None,
                        "10",
                        TaxType::B,
                    )
                })
                .collect()
        };
        let mut counts = Vec::new();
        for (n, tag) in [(1usize, "a"), (500, "b")] {
            let mut tx = s.begin_immediate().await.unwrap();
            let (r, stmts) = upsert_all(tx.conn(), &batch(n, tag)).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(r.inserted, n);
            counts.push(stmts);
        }
        assert_eq!(counts, vec![4, 4], "1 product and 500 cost the same");
    }
}
