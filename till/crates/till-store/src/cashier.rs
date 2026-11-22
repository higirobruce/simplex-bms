//! The cashier cache and offline PIN verification.
//!
//! A till that cannot open a shift offline cannot trade, so [`Store::verify_pin`]
//! reads SQLite and nothing else: there is no network code in this module by
//! construction. [`Store::sync_cashiers`] takes the list the caller has already
//! fetched; the HTTP call is not here.
//!
//! # The hash is an interop contract
//!
//! The cloud stores each PIN as an argon2id PHC string
//! (`$argon2id$v=19$m=131072,t=3,p=1$<salt>$<hash>`). The string carries its own
//! cost parameters and the verifier reads them FROM the string, so nothing here
//! hardcodes a cost: the cloud can raise it without a till release. Memory
//! cost at the current parameters is about 128 MiB per verification, which is
//! why verification is only expected once per shift.
//!
//! # Secrets
//!
//! A PIN or a PIN hash is never logged, printed, or put in an error. `Cashier`
//! has a hand-written `Debug` that omits the hash, and [`Store::active_cashiers`]
//! returns [`CashierEntry`], which has no hash field at all.
//!
//! # Denied by default
//!
//! An inactive cashier, an empty hash and a hash that does not parse all refuse
//! verification. The cloud deactivates a cashier by nulling the hash, so an
//! absent hash must never mean "no PIN required".

use std::fmt;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use sqlx::Row;

use crate::schema::{Store, StoreError};

/// UTC timestamp in SQL, so the store never needs a clock crate.
const NOW_SQL: &str = "strftime('%Y-%m-%dT%H:%M:%SZ','now')";

/// One cashier as the cloud describes them. `id` is the cloud `User.id`.
/// `pin_hash` is an argon2id PHC string; it is input to a sync, never output.
#[derive(Clone, PartialEq, Eq)]
pub struct Cashier {
    pub id: String,
    pub name: String,
    pub pin_hash: String,
    pub active: bool,
}

// Not derived: a derive would print `pin_hash` into any log line or test
// failure that formats a `Cashier`.
impl fmt::Debug for Cashier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cashier")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("pin_hash", &"<redacted>")
            .field("active", &self.active)
            .finish()
    }
}

/// A cashier as the picker UI sees them: no hash, by type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashierEntry {
    pub id: String,
    pub name: String,
}

/// The most memory (KiB) a stored hash may ask argon2 to allocate: 1 GiB, eight
/// times the cloud's current 128 MiB (`m=131072`), so a legitimate retune still
/// verifies. It exists because a failed allocation ABORTS the process rather
/// than returning an error, and a corrupt row or tampered database file could
/// otherwise carry an absurd `m`. The cost still comes from the hash; this only
/// declines to honour an absurd one.
const MAX_MEMORY_KIB: u32 = 1_048_576;

// Compile-time guard: the ceiling must stay well above the cloud's m=131072.
const _: () = assert!(MAX_MEMORY_KIB >= 131_072 * 4);

/// Verifies `pin` against a PHC string, parameters taken from the string.
/// Any failure, including a malformed hash or an over-ceiling `m`, is a
/// refusal; nothing panics.
fn pin_matches(pin: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    match parsed.params.get_decimal("m") {
        Some(m) if m <= MAX_MEMORY_KIB => {}
        _ => return false,
    }
    Argon2::default()
        .verify_password(pin.as_bytes(), &parsed)
        .is_ok()
}

impl Store {
    /// Replaces the cached cashier list with `cashiers`, in ONE transaction.
    ///
    /// "Replaces" means a cashier absent from the list is deleted, so someone
    /// removed upstream stops verifying here. The delete and the upserts commit
    /// together or not at all, so a failure part way cannot leave half a list.
    ///
    /// Unlike `sync_catalogue` (upsert-only, never deletes, no empty-list
    /// rule), this DELETES rows absent from the list and treats an empty list
    /// as a no-op. A failed FETCH must not reach this function (the caller
    /// returns before it), and an empty list is ignored, because
    /// the cloud keeps a deactivated cashier in the list with no hash, so an
    /// empty list means a bad or partial response, and a till with no cashiers
    /// cannot open a shift. Rows with an empty id are refused before the write
    /// lock is taken.
    pub async fn sync_cashiers(&self, cashiers: &[Cashier]) -> Result<(), StoreError> {
        if cashiers.is_empty() {
            return Ok(());
        }
        if cashiers.iter().any(|c| c.id.is_empty()) {
            return Err(StoreError::CashierRejected {
                reason: "empty id".to_string(),
            });
        }
        let ids = serde_json::to_string(&cashiers.iter().map(|c| &c.id).collect::<Vec<_>>())?;
        let mut tx = self.begin_immediate().await?;
        sqlx::query("delete from cashier where id not in (select value from json_each(?))")
            .bind(&ids)
            .execute(&mut *tx.conn())
            .await?;
        for c in cashiers {
            sqlx::query(&format!(
                "insert into cashier (id, name, pin_hash, active, synced_at) \
                 values (?, ?, ?, ?, {NOW_SQL}) \
                 on conflict (id) do update set name = excluded.name, \
                 pin_hash = excluded.pin_hash, active = excluded.active, \
                 synced_at = excluded.synced_at"
            ))
            .bind(&c.id)
            .bind(&c.name)
            .bind(&c.pin_hash)
            .bind(i64::from(c.active))
            .execute(&mut *tx.conn())
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Checks `pin` for the active cashier whose id or name is `name_or_id`,
    /// from the local database only. Returns the cashier's cloud id on success.
    ///
    /// `None` covers every refusal: unknown cashier, inactive, no hash, wrong
    /// PIN, unparsable hash. The caller cannot and need not tell them apart.
    /// Several cashiers may share a name; the first whose PIN matches wins.
    ///
    /// This is CPU- and memory-heavy by design (see the module docs) and runs
    /// on the calling thread, so an async caller that cares should run it where
    /// blocking is acceptable.
    pub async fn verify_pin(
        &self,
        name_or_id: &str,
        pin: &str,
    ) -> Result<Option<String>, StoreError> {
        let rows = sqlx::query(
            "select id, pin_hash from cashier \
             where active = 1 and pin_hash <> '' and (id = ?1 or name = ?1) order by id",
        )
        .bind(name_or_id)
        .fetch_all(self.pool())
        .await?;
        for row in &rows {
            let hash: String = row.try_get("pin_hash")?;
            if pin_matches(pin, &hash) {
                return Ok(Some(row.try_get("id")?));
            }
        }
        Ok(None)
    }

    /// The cashiers who may open a shift, by name, for the picker. Carries no
    /// hash: see [`CashierEntry`].
    pub async fn active_cashiers(&self) -> Result<Vec<CashierEntry>, StoreError> {
        let rows = sqlx::query(
            "select id, name from cashier where active = 1 and pin_hash <> '' \
             order by name, id",
        )
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|r| {
                Ok(CashierEntry {
                    id: r.try_get("id")?,
                    name: r.try_get("name")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::{Algorithm, Params, PasswordHasher, Version};
    use std::time::Instant;

    /// A hash the cloud's `argon2` npm package produced for PIN "4821" with
    /// the production parameters (m=131072, t=3, p=1). The interop proof.
    const CLOUD_HASH: &str = "$argon2id$v=19$m=131072,t=3,p=1$tmtSLo0Z/83mPOveszHRVw$QgZ64XnOvO8C5cX3ItVDDI0je1YVUU+GShAZWIWr0PI";

    /// A cheap hash (8 KiB, 1 pass) so the logic tests do not each pay 128 MiB.
    /// The parameters differ from the cloud's on purpose: verification must
    /// read them from the string.
    fn hash(pin: &str) -> String {
        let params = Params::new(8, 1, 1, None).unwrap();
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_with_salt(pin.as_bytes(), b"0123456789abcdef")
            .unwrap()
            .to_string()
    }

    fn cashier(id: &str, name: &str, pin: &str, active: bool) -> Cashier {
        Cashier {
            id: id.into(),
            name: name.into(),
            pin_hash: hash(pin),
            active,
        }
    }

    async fn store_with(cs: &[Cashier]) -> Store {
        let s = Store::open_in_memory().await.unwrap();
        s.sync_cashiers(cs).await.unwrap();
        s
    }

    #[tokio::test]
    async fn a_correct_pin_returns_the_cashier_id() {
        let s = store_with(&[cashier("u1", "Alice", "1234", true)]).await;
        assert_eq!(
            s.verify_pin("Alice", "1234").await.unwrap().as_deref(),
            Some("u1")
        );
        assert_eq!(
            s.verify_pin("u1", "1234").await.unwrap().as_deref(),
            Some("u1"),
            "id works as well as name"
        );
    }

    #[tokio::test]
    async fn a_wrong_pin_returns_none() {
        let s = store_with(&[cashier("u1", "Alice", "1234", true)]).await;
        assert_eq!(s.verify_pin("Alice", "9999").await.unwrap(), None);
        assert_eq!(s.verify_pin("Alice", "").await.unwrap(), None);
        assert_eq!(s.verify_pin("Nobody", "1234").await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_inactive_cashier_never_verifies() {
        let s = store_with(&[cashier("u1", "Alice", "1234", false)]).await;
        assert_eq!(s.verify_pin("Alice", "1234").await.unwrap(), None);
        assert!(s.active_cashiers().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_absent_or_garbage_hash_is_denied_not_waved_through() {
        let mut empty = cashier("u1", "Alice", "1234", true);
        empty.pin_hash = String::new();
        let mut garbage = cashier("u2", "Bob", "1234", true);
        garbage.pin_hash = "not-a-phc-string".into();
        let s = store_with(&[empty, garbage]).await;
        for who in ["Alice", "Bob"] {
            assert_eq!(s.verify_pin(who, "").await.unwrap(), None, "{who}");
            assert_eq!(s.verify_pin(who, "1234").await.unwrap(), None, "{who}");
        }
        // the empty-hash row is not offered to the picker at all
        let listed: Vec<String> = s
            .active_cashiers()
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(listed, vec!["Bob"], "garbage is listed but still denied");
    }

    /// Reopens the database file and verifies without any sync.
    ///
    /// What it proves is that the cached data is sufficient and persistent. The
    /// no-network guarantee itself is structural: `till-store` has no HTTP
    /// dependency, so a network call could not compile.
    #[tokio::test]
    async fn verification_needs_only_the_reopened_file() {
        let dir = std::env::temp_dir().join(format!("till-cashier-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("till.db");
        {
            let s = Store::open(&path).await.unwrap();
            s.sync_cashiers(&[cashier("u1", "Alice", "1234", true)])
                .await
                .unwrap();
        }
        // A fresh process-equivalent: reopen from disk, no sync, no fetch.
        let s = Store::open(&path).await.unwrap();
        assert_eq!(
            s.verify_pin("Alice", "1234").await.unwrap().as_deref(),
            Some("u1")
        );
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_resync_that_deactivates_a_cashier_takes_effect() {
        let s = store_with(&[cashier("u1", "Alice", "1234", true)]).await;
        assert!(s.verify_pin("Alice", "1234").await.unwrap().is_some());
        s.sync_cashiers(&[cashier("u1", "Alice", "1234", false)])
            .await
            .unwrap();
        assert_eq!(s.verify_pin("Alice", "1234").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_resync_that_drops_a_cashier_removes_them_and_a_new_pin_replaces_the_old() {
        let s = store_with(&[
            cashier("u1", "Alice", "1234", true),
            cashier("u2", "Bob", "5555", true),
        ])
        .await;
        s.sync_cashiers(&[cashier("u2", "Bob", "6666", true)])
            .await
            .unwrap();
        assert_eq!(s.verify_pin("Alice", "1234").await.unwrap(), None);
        assert_eq!(s.verify_pin("Bob", "5555").await.unwrap(), None);
        assert_eq!(
            s.verify_pin("Bob", "6666").await.unwrap().as_deref(),
            Some("u2")
        );
    }

    #[tokio::test]
    async fn a_failed_sync_leaves_the_cached_list_intact() {
        let s = store_with(&[cashier("u1", "Alice", "1234", true)]).await;
        // refused before the lock
        let r = s
            .sync_cashiers(&[cashier("u2", "Bob", "1", true), cashier("", "X", "1", true)])
            .await;
        assert!(matches!(r, Err(StoreError::CashierRejected { .. })));
        // an empty list (a bad fetch that reached us) does not empty the table
        s.sync_cashiers(&[]).await.unwrap();
        // a failure INSIDE the transaction, after the delete has run: a NULL
        // name violates NOT NULL only if bound as NULL, so force it with a
        // trigger that aborts the second upsert.
        sqlx::query(
            "create trigger boom before insert on cashier when new.id = 'u3' \
             begin select raise(abort, 'boom'); end",
        )
        .execute(s.pool())
        .await
        .unwrap();
        let r = s
            .sync_cashiers(&[
                cashier("u2", "Bob", "1", true),
                cashier("u3", "C", "1", true),
            ])
            .await;
        assert!(matches!(r, Err(StoreError::Sqlx(_))));
        let n: i64 = sqlx::query_scalar("select count(*) from cashier")
            .fetch_one(s.pool())
            .await
            .unwrap();
        assert_eq!(n, 1, "the delete and the u2 upsert rolled back");
        assert_eq!(
            s.verify_pin("Alice", "1234").await.unwrap().as_deref(),
            Some("u1")
        );
    }

    #[tokio::test]
    async fn a_pin_hash_is_never_logged_or_returned_by_active_cashiers() {
        let c = cashier("u1", "Alice", "1234", true);
        let s = store_with(std::slice::from_ref(&c)).await;
        let listed = s.active_cashiers().await.unwrap();
        assert_eq!(listed.len(), 1);
        // CashierEntry has no hash field; belt and braces on its text form too.
        assert!(!format!("{listed:?}").contains("argon2"));
        assert!(!format!("{c:?}").contains("argon2"), "Debug redacts");
        assert!(!format!("{c:?}").contains(&c.pin_hash));
    }

    /// Proves interop with the cloud's hash and MEASURES one production-cost
    /// verification. Run with `--nocapture` to see the time.
    #[tokio::test]
    async fn a_hash_from_the_cloud_verifies_and_its_cost_is_measured() {
        let s = store_with(&[Cashier {
            id: "u1".into(),
            name: "Alice".into(),
            pin_hash: CLOUD_HASH.into(),
            active: true,
        }])
        .await;
        let t = Instant::now();
        let ok = s.verify_pin("Alice", "4821").await.unwrap();
        let ms = t.elapsed().as_millis();
        assert_eq!(ok.as_deref(), Some("u1"));
        println!("verify_pin (m=131072,t=3,p=1) took {ms} ms");
        assert_eq!(s.verify_pin("Alice", "4822").await.unwrap(), None);
    }

    /// A valid argon2id PHC string asking for ~4 TiB. Honoured, it would abort
    /// the process on allocation; it must be denied before argon2 runs.
    #[tokio::test]
    async fn a_hash_demanding_absurd_memory_is_denied_and_the_process_survives() {
        let absurd = "$argon2id$v=19$m=4294967295,t=1,p=1$MDEyMzQ1Njc4OWFiY2RlZg$MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY";
        assert!(PasswordHash::new(absurd).is_ok(), "the fixture must parse");
        let s = store_with(&[Cashier {
            id: "u1".into(),
            name: "Alice".into(),
            pin_hash: absurd.into(),
            active: true,
        }])
        .await;
        assert_eq!(s.verify_pin("Alice", "1234").await.unwrap(), None);
        assert!(!pin_matches("1234", absurd));
    }
}
