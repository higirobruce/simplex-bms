//! Joins the till to the cloud back office: drains the outbound queue over
//! HTTPS and pulls the cashier list.
//!
//! The contract with the cloud routes is the status code, and a wrong reading
//! corrupts data, so it is stated once, here:
//!
//! | answer                         | meaning                       | the till                              |
//! |--------------------------------|-------------------------------|---------------------------------------|
//! | 2xx                            | accepted                      | marks the row sent (the ONLY way)     |
//! | 422                            | can never succeed             | parks the row, keeps the error        |
//! | 401 / 403                      | lane token revoked or wrong   | holds the WHOLE queue, loses nothing  |
//! | 5xx, timeout, refused, other   | transient (or unknown)        | keeps the row, retries with backoff   |
//!
//! "Other" includes 400, 404, 408, 429 and redirects: a wrong tunnel hostname
//! answers 404, and parking every sale because of it would be far worse than
//! waiting. Only an explicit 422 declares a row dead.
//!
//! The drain is strictly sequential and in `id` order: a shift must land
//! before its sales (`Sale.shiftId` is a foreign key in the cloud). A shift the
//! cloud refuses is parked and the pass stops; its sales are then withheld by
//! `Store::unsent`, while other shifts' rows still flow.
//!
//! Sync faults are never cashier-facing. They are recorded in the store
//! (`last_error`, `parked_at`) and in [`CloudSync::halted`]; the status surface
//! that shows them is built separately.
//!
//! The token is only ever placed in the `Authorization` header (marked
//! sensitive by reqwest). Error text stored or returned here is composed by
//! this module, never copied from a request, so it cannot contain the token.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use thiserror::Error;
use till_store::{Cashier, DrainOutcome, OutboxRow, Store};
use tokio::sync::Notify;

use crate::config::SyncConfig;
use crate::sync_status::{SharedSyncStatus, SyncStatus, now_unix};

/// The most rows one pass sends.
pub const BATCH_LIMIT: u32 = 50;
/// How often the drain runs when nothing wakes it.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// How often the cashier list is refreshed.
const CASHIER_REFRESH: Duration = Duration::from_secs(300);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The wait after `failures` consecutive failed passes: 1, 2, 4 ... capped at 60 s.
pub fn backoff(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(6);
    Duration::from_secs((1u64 << exp).min(60))
}

/// What the cloud's answer means for one request.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    Accepted,
    /// 422: never going to succeed.
    Refused(String),
    /// 401 / 403: the token is revoked or wrong.
    AuthRejected(u16),
    /// Anything else: try again later.
    Retry(String),
}

fn classify(status: u16, body: &str) -> Reply {
    match status {
        200..=299 => Reply::Accepted,
        422 => Reply::Refused(format!("HTTP 422: {}", clip(body))),
        401 | 403 => Reply::AuthRejected(status),
        other => Reply::Retry(format!("HTTP {other}")),
    }
}

/// The cloud's own message, trimmed. It is the server's text, not ours, but it
/// is bounded so a hostile or broken answer cannot bloat the database.
fn clip(s: &str) -> String {
    s.chars().take(200).collect()
}

/// A transport failure described without echoing the request (whose headers
/// hold the token).
fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "request timed out".into()
    } else if e.is_connect() {
        "could not connect to the cloud".into()
    } else {
        "request failed".into()
    }
}

/// Why a cashier pull changed nothing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PullError {
    #[error("cashier pull failed: {0}")]
    Http(String),
    #[error("the lane token was rejected")]
    AuthRejected,
    #[error("the cashier list could not be read")]
    Malformed,
    #[error("the cashier list could not be saved")]
    Store,
}

/// What a successful cashier pull did.
#[derive(Debug, PartialEq, Eq)]
pub struct Pulled {
    /// Cashiers written to the local list.
    pub synced: usize,
    /// Records the till could not read. Each one that still had a readable id
    /// keeps whatever row the till already holds; a record with no readable id
    /// cannot be matched to anyone, so it is simply not represented.
    pub skipped: usize,
}

/// One cashier as `GET /api/till/cashiers` sends it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudCashier {
    id: String,
    /// The cloud's `User.name` is nullable.
    name: Option<String>,
    /// `null` for a deactivated cashier.
    pin_hash: Option<String>,
    active: bool,
}

impl From<CloudCashier> for Cashier {
    fn from(c: CloudCashier) -> Cashier {
        // A missing hash becomes "", which `verify_pin` never accepts, and the
        // cashier is also inactive whatever the cloud said.
        let pin_hash = c.pin_hash.unwrap_or_default();
        // A nameless cashier is shown (and typed) by id rather than dropped.
        let name = c
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| c.id.clone());
        Cashier {
            active: c.active && !pin_hash.is_empty(),
            id: c.id,
            name,
            pin_hash,
        }
    }
}

/// The cloud client: one per till.
pub struct CloudSync {
    store: Store,
    base_url: String,
    token: String,
    client: reqwest::Client,
    /// Set when the token was rejected. While set, nothing is sent.
    halted: Mutex<Option<String>>,
    /// Records the last successful cashier pull could not read.
    last_skipped: AtomicUsize,
    /// Published after every pass for the status bar.
    status: SharedSyncStatus,
    wake: Arc<Notify>,
}

impl CloudSync {
    pub fn new(store: Store, cfg: SyncConfig) -> Result<CloudSync, String> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            // A redirect is not an acceptance, and following one would resend
            // the bearer token to wherever it points.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "the HTTP client could not be built".to_owned())?;
        Ok(CloudSync {
            store,
            base_url: cfg.base_url,
            token: cfg.token,
            client,
            halted: Mutex::new(None),
            last_skipped: AtomicUsize::new(0),
            // Stamped at start so a loop that never reports goes stale too.
            status: Arc::new(Mutex::new(SyncStatus {
                published_at: Some(now_unix()),
                ..SyncStatus::default()
            })),
            wake: Arc::new(Notify::new()),
        })
    }

    /// Signal this after queuing a row to drain now instead of at the next beat.
    pub fn waker(&self) -> Arc<Notify> {
        Arc::clone(&self.wake)
    }

    /// The snapshot the loop keeps current, for the screen to read.
    pub fn status_handle(&self) -> SharedSyncStatus {
        Arc::clone(&self.status)
    }

    /// Re-reads the queue and publishes it.
    async fn publish_status(&self) {
        let reading = match (
            self.store.queue_depth().await,
            self.store.parked_count().await,
        ) {
            (Ok((depth, oldest)), Ok(parked)) => Some((depth, oldest, parked)),
            _ => None,
        };
        self.publish_reading(reading, now_unix());
    }

    /// Publishes what is known. The hold and the unreadable-cashier count live
    /// in memory, so they are refreshed even when the queue could not be read
    /// (a revoked token must show during a database fault). A failed read
    /// keeps the previous depth rather than a made-up zero, and does NOT renew
    /// `published_at`, so those figures age honestly into "no report since".
    fn publish_reading(&self, reading: Option<(u32, Option<String>, u32)>, now: u64) {
        let Ok(mut g) = self.status.lock() else {
            return;
        };
        g.held = self.halted().is_some();
        g.skipped_cashiers = self.skipped_cashiers();
        if let Some((depth, oldest, parked)) = reading {
            g.ran = true;
            g.depth = depth;
            g.oldest = oldest;
            g.parked = parked;
            g.published_at = Some(now);
        }
    }

    /// Why the whole queue is held, if it is (the token was rejected).
    pub fn halted(&self) -> Option<String> {
        self.halted.lock().ok().and_then(|g| g.clone())
    }

    /// How many records the last cashier pull could not read (0 when it read
    /// them all). For the status surface.
    pub fn skipped_cashiers(&self) -> usize {
        self.last_skipped.load(Ordering::Relaxed)
    }

    fn halt(&self, status: u16) -> String {
        let reason = format!(
            "the cloud rejected this lane's token (HTTP {status}); nothing is sent until it is replaced"
        );
        if let Ok(mut g) = self.halted.lock() {
            *g = Some(reason.clone());
        }
        reason
    }

    async fn request(&self, req: reqwest::RequestBuilder) -> Result<(u16, String), String> {
        match req.bearer_auth(&self.token).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Ok((status, body))
            }
            Err(e) => Err(describe(&e)),
        }
    }

    /// Posts the stored payload text as it is. It is not re-serialised: what
    /// was queued is what the cloud was built to accept.
    async fn post(&self, row: &OutboxRow) -> Reply {
        let path = if row.kind == "shift" { "shift" } else { "sale" };
        let req = self
            .client
            .post(format!("{}/api/till/{path}", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(row.payload.clone());
        match self.request(req).await {
            Ok((status, body)) => classify(status, &body),
            Err(why) => Reply::Retry(why),
        }
    }

    /// One pass: sends up to [`BATCH_LIMIT`] waiting rows, one at a time, in
    /// order. A store fault reads as `Offline` (retry), never a panic.
    pub async fn drain_once(&self) -> DrainOutcome {
        if let Some(reason) = self.halted() {
            let id = self
                .store
                .unsent(1)
                .await
                .ok()
                .and_then(|r| r.first().map(|r| r.id));
            return DrainOutcome::Parked {
                id: id.unwrap_or(0),
                reason,
            };
        }
        let rows = match self.store.unsent(BATCH_LIMIT).await {
            Ok(rows) => rows,
            Err(_) => return DrainOutcome::Offline,
        };
        if rows.is_empty() {
            return DrainOutcome::Idle;
        }
        let mut sent = 0u32;
        let mut first_parked: Option<(i64, String)> = None;
        for row in &rows {
            match self.post(row).await {
                Reply::Accepted => {
                    if self.store.mark_sent(row.id).await.is_err() {
                        // The cloud has it; the row will be re-sent, and the
                        // routes are idempotent on the ids the till supplies.
                        return DrainOutcome::Offline;
                    }
                    sent += 1;
                }
                Reply::Refused(reason) => {
                    if self.store.park(row.id, &reason).await.is_err() {
                        // Not recorded: back off like any other failure rather
                        // than spin on a database that will not take a write.
                        return DrainOutcome::Offline;
                    }
                    first_parked.get_or_insert((row.id, reason));
                    if row.kind == "shift" {
                        // Its sales would be refused in turn. Stop and surface it.
                        break;
                    }
                }
                Reply::AuthRejected(status) => {
                    let reason = self.halt(status);
                    let _ = self.store.record_failure(row.id, &reason).await;
                    return DrainOutcome::Parked { id: row.id, reason };
                }
                Reply::Retry(why) => {
                    let _ = self.store.record_failure(row.id, &why).await;
                    return DrainOutcome::Offline;
                }
            }
        }
        match first_parked {
            Some((id, reason)) => DrainOutcome::Parked { id, reason },
            None => DrainOutcome::Sent(sent),
        }
    }

    /// Fetches the cashier list and replaces the local one with it. On ANY
    /// failure the list the till has is left exactly as it was: a stale list
    /// still opens a shift, an empty one cannot.
    pub async fn pull_cashiers(&self) -> Result<Pulled, PullError> {
        if self.halted().is_some() {
            return Err(PullError::AuthRejected);
        }
        let req = self
            .client
            .get(format!("{}/api/till/cashiers", self.base_url));
        let (status, body) = self.request(req).await.map_err(PullError::Http)?;
        match classify(status, &body) {
            Reply::Accepted => {}
            Reply::AuthRejected(s) => {
                self.halt(s);
                return Err(PullError::AuthRejected);
            }
            Reply::Refused(why) | Reply::Retry(why) => return Err(PullError::Http(why)),
        }
        // Row by row: one bad record costs that cashier an update, never the
        // shop's list. But `sync_cashiers` deletes ids it was not given, so the
        // id of EVERY record with a readable one is passed along to be spared,
        // whether or not the rest of the record parsed.
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&body).map_err(|_| PullError::Malformed)?;
        let total = rows.len();
        let mut keep: Vec<String> = Vec::new();
        let mut cashiers: Vec<Cashier> = Vec::new();
        for v in rows {
            if let Some(id) = v
                .get("id")
                .and_then(|i| i.as_str())
                .filter(|i| !i.is_empty())
            {
                keep.push(id.to_owned());
            }
            if let Ok(c) = serde_json::from_value::<CloudCashier>(v) {
                if !c.id.is_empty() {
                    cashiers.push(Cashier::from(c));
                }
            }
        }
        let skipped = total - cashiers.len();
        self.last_skipped.store(skipped, Ordering::Relaxed);
        if cashiers.is_empty() {
            return Ok(Pulled { synced: 0, skipped });
        }
        self.store
            .sync_cashiers_keeping(&cashiers, &keep)
            .await
            .map_err(|_| PullError::Store)?;
        Ok(Pulled {
            synced: cashiers.len(),
            skipped,
        })
    }

    /// The background loop: pull cashiers now and every few minutes, drain on
    /// every heartbeat, on every wake-up, and (with backoff) after a failure.
    /// Runs on the runtime, never on the UI thread.
    pub async fn run(self: Arc<Self>) {
        let mut failures = 0u32;
        let mut last_pull: Option<(Instant, bool)> = None;
        loop {
            let due = match last_pull {
                None => true,
                Some((at, ok)) => !ok || at.elapsed() >= CASHIER_REFRESH,
            };
            if due && self.halted().is_none() {
                let pulled = self.pull_cashiers().await;
                if pulled.is_ok() && self.skipped_cashiers() > 0 {
                    // A log line for the operator, never a cashier notice.
                    eprintln!(
                        "Cloud sync: {} cashier record(s) could not be read and were left as they were.",
                        self.skipped_cashiers()
                    );
                }
                last_pull = Some((Instant::now(), pulled.is_ok()));
            }
            let outcome = self.drain_once().await;
            self.publish_status().await;
            let wait = match outcome {
                DrainOutcome::Offline => {
                    failures = failures.saturating_add(1);
                    backoff(failures)
                }
                DrainOutcome::Sent(n) if n >= BATCH_LIMIT => {
                    failures = 0;
                    continue; // more is waiting
                }
                DrainOutcome::Parked { .. } if self.halted().is_none() => {
                    failures = 0;
                    continue; // a row was parked; the rest may still go
                }
                _ => {
                    failures = 0;
                    HEARTBEAT
                }
            };
            // Wake early when something is queued; otherwise wait it out.
            let _ = tokio::time::timeout(wait, self.wake.notified()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const TOKEN: &str = "SECRET-LANE-TOKEN-9f3a";

    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        path: String,
        auth: Option<String>,
        body: String,
    }

    type Handler = Arc<dyn Fn(&Seen) -> (u16, String) + Send + Sync>;

    struct Stub {
        url: String,
        seen: Arc<Mutex<Vec<Seen>>>,
        max_in_flight: Arc<AtomicUsize>,
    }

    impl Stub {
        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    async fn stub(handler: impl Fn(&Seen) -> (u16, String) + Send + Sync + 'static) -> Stub {
        let handler: Handler = Arc::new(handler);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let (s2, i2, m2) = (seen.clone(), in_flight, max_in_flight.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (handler, seen, in_flight, max) =
                    (handler.clone(), s2.clone(), i2.clone(), m2.clone());
                tokio::spawn(async move {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(now, Ordering::SeqCst);
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head_end, len) = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            in_flight.fetch_sub(1, Ordering::SeqCst);
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            break (p + 4, len);
                        }
                    };
                    while buf.len() < head_end + len {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let mut first = head.lines().next().unwrap_or("").split(' ');
                    let req = Seen {
                        method: first.next().unwrap_or("").into(),
                        path: first.next().unwrap_or("").into(),
                        auth: head.lines().find_map(|l| {
                            l.to_lowercase()
                                .starts_with("authorization:")
                                .then(|| l[14..].trim().to_owned())
                        }),
                        body: String::from_utf8_lossy(&buf[head_end..]).into_owned(),
                    };
                    seen.lock().unwrap().push(req.clone());
                    // Long enough that two overlapping requests would overlap here.
                    tokio::time::sleep(Duration::from_millis(15)).await;
                    let (status, body) = handler(&req);
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Stub {
            url,
            seen,
            max_in_flight,
        }
    }

    fn ok(_: &Seen) -> (u16, String) {
        (200, r#"{"id":"x"}"#.into())
    }

    async fn sync_for(url: &str) -> (CloudSync, Store) {
        let store = Store::open_in_memory().await.unwrap();
        let cfg = SyncConfig {
            base_url: url.into(),
            token: TOKEN.into(),
        };
        (CloudSync::new(store.clone(), cfg).unwrap(), store)
    }

    async fn queue(s: &Store, kind: &str, payload: &str) -> i64 {
        sqlx::query_scalar(
            "insert into outbound (kind, payload, created_at) values (?, ?, '2026-11-22T08:00:00Z') returning id",
        )
        .bind(kind)
        .bind(payload)
        .fetch_one(s.pool())
        .await
        .unwrap()
    }

    async fn ids(s: &Store) -> Vec<i64> {
        s.unsent(1000).await.unwrap().iter().map(|r| r.id).collect()
    }

    async fn last_error(s: &Store, id: i64) -> Option<String> {
        sqlx::query_scalar("select last_error from outbound where id = ?")
            .bind(id)
            .fetch_one(s.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_2xx_marks_the_row_sent() {
        let stub = stub(ok).await;
        let (sync, store) = sync_for(&stub.url).await;
        let payload = r#"{"id":"abc","openedById":"u1"}"#;
        queue(&store, "shift", payload).await;
        assert_eq!(sync.drain_once().await, DrainOutcome::Sent(1));
        assert!(ids(&store).await.is_empty());
        let seen = stub.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/api/till/shift");
        assert_eq!(
            seen[0].auth.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(seen[0].body, payload, "the stored text is posted verbatim");
        assert_eq!(sync.drain_once().await, DrainOutcome::Idle);
    }

    #[tokio::test]
    async fn a_sale_row_goes_to_the_sale_route() {
        let stub = stub(ok).await;
        let (sync, store) = sync_for(&stub.url).await;
        queue(&store, "sale", r#"{"saleId":"s"}"#).await;
        sync.drain_once().await;
        assert_eq!(stub.seen()[0].path, "/api/till/sale");
    }

    #[tokio::test]
    async fn a_non_2xx_leaves_the_row_unsent_and_records_the_error() {
        for status in [500u16, 502, 503, 404, 429, 400, 302] {
            let stub = stub(move |_| (status, "boom".into())).await;
            let (sync, store) = sync_for(&stub.url).await;
            let id = queue(&store, "sale", "{}").await;
            assert_eq!(sync.drain_once().await, DrainOutcome::Offline, "{status}");
            assert_eq!(
                ids(&store).await,
                vec![id],
                "{status}: still unsent and not parked"
            );
            assert_eq!(last_error(&store, id).await, Some(format!("HTTP {status}")));
            assert_eq!(store.unsent(1).await.unwrap()[0].attempts, 1);
            assert!(sync.halted().is_none(), "{status}");
            // And it is retried: the next pass sends it again.
            sync.drain_once().await;
            assert_eq!(stub.seen().len(), 2, "{status}");
        }
    }

    #[tokio::test]
    async fn a_refused_connection_and_a_timeout_are_transient() {
        // Nothing listens on this port any more.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (sync, store) = sync_for(&dead).await;
        let id = queue(&store, "shift", "{}").await;
        assert_eq!(sync.drain_once().await, DrainOutcome::Offline);
        assert_eq!(ids(&store).await, vec![id]);
        assert_eq!(
            last_error(&store, id).await.as_deref(),
            Some("could not connect to the cloud")
        );
    }

    #[tokio::test]
    async fn a_422_parks_the_row_without_blocking_unrelated_rows() {
        let stub = stub(|r| {
            if r.body.contains("bad") {
                (422, r#"{"error":"Unknown cashier"}"#.into())
            } else {
                ok(r)
            }
        })
        .await;
        let (sync, store) = sync_for(&stub.url).await;
        let a = queue(&store, "sale", r#"{"shiftId":"s1","n":"first"}"#).await;
        let b = queue(&store, "sale", r#"{"shiftId":"s1","n":"bad"}"#).await;
        let c = queue(&store, "sale", r#"{"shiftId":"s1","n":"third"}"#).await;
        match sync.drain_once().await {
            DrainOutcome::Parked { id, reason } => {
                assert_eq!(id, b);
                assert!(reason.contains("Unknown cashier"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        assert!(ids(&store).await.is_empty(), "a and c delivered, b parked");
        assert_eq!(store.parked_count().await.unwrap(), 1);
        assert_eq!(
            store.queue_depth().await.unwrap().0,
            1,
            "only the parked row is undelivered"
        );
        assert!(
            last_error(&store, b)
                .await
                .unwrap()
                .contains("Unknown cashier")
        );
        assert_eq!(stub.seen().len(), 3, "c was sent after b was parked");
        let _ = (a, c);
        // The parked row is not retried forever.
        assert_eq!(sync.drain_once().await, DrainOutcome::Idle);
        assert_eq!(stub.seen().len(), 3);
    }

    #[tokio::test]
    async fn a_401_parks_the_whole_queue_and_loses_nothing() {
        for status in [401u16, 403] {
            let stub = stub(move |_| (status, r#"{"error":"nope"}"#.into())).await;
            let (sync, store) = sync_for(&stub.url).await;
            let a = queue(&store, "shift", r#"{"id":"s1"}"#).await;
            let b = queue(&store, "sale", r#"{"shiftId":"s1"}"#).await;
            let c = queue(&store, "sale", r#"{"shiftId":"s1"}"#).await;
            match sync.drain_once().await {
                DrainOutcome::Parked { id, reason } => {
                    assert_eq!(id, a);
                    assert!(reason.contains(&status.to_string()), "{reason}");
                }
                other => panic!("{status}: {other:?}"),
            }
            assert_eq!(
                stub.seen().len(),
                1,
                "{status}: it stopped at the first rejection"
            );
            assert_eq!(
                ids(&store).await,
                vec![a, b, c],
                "{status}: every row is still waiting, in order"
            );
            assert_eq!(
                store.parked_count().await.unwrap(),
                0,
                "no row is individually parked"
            );
            assert!(sync.halted().is_some());
            // The queue stays held: no further request goes out.
            assert!(matches!(sync.drain_once().await, DrainOutcome::Parked { id, .. } if id == a));
            assert_eq!(
                stub.seen().len(),
                1,
                "{status}: a held queue makes no requests"
            );
        }
    }

    #[tokio::test]
    async fn a_revoked_token_reaches_the_published_status() {
        let stub = stub(|_| (401, "{}".into())).await;
        let (sync, store) = sync_for(&stub.url).await;
        queue(&store, "sale", "{}").await;
        let handle = sync.status_handle();
        sync.drain_once().await;
        sync.publish_status().await;
        let s = handle.lock().unwrap().clone();
        assert!(s.held, "the hold must reach the screen");
        assert_eq!(s.depth, 1);
        assert!(s.ran);
    }

    #[tokio::test]
    async fn unreadable_cashiers_reach_the_published_status() {
        let body = r#"[{"id":"u1","name":"A","pinHash":42,"active":true}]"#;
        let stub = stub(move |_| (200, body.into())).await;
        let (sync, _store) = sync_for(&stub.url).await;
        sync.pull_cashiers().await.unwrap();
        sync.publish_status().await;
        assert_eq!(sync.status_handle().lock().unwrap().skipped_cashiers, 1);
    }

    #[tokio::test]
    async fn a_hold_is_published_even_when_the_queue_cannot_be_read() {
        let stub = stub(|_| (401, "{}".into())).await;
        let (sync, store) = sync_for(&stub.url).await;
        queue(&store, "sale", "{}").await;
        sync.publish_status().await;
        let before = sync.status_handle().lock().unwrap().clone();
        assert!(before.ran && before.depth == 1 && !before.held);
        sync.pull_cashiers().await.unwrap_err(); // the 401 halts
        sync.publish_reading(None, before.published_at.unwrap() + 500);
        let s = sync.status_handle().lock().unwrap().clone();
        assert!(s.held, "a hold must show during a database fault");
        assert_eq!(s.depth, 1, "the last known depth is kept, not zeroed");
        assert_eq!(
            s.published_at, before.published_at,
            "a failed read is not a report"
        );
        // A first read that fails leaves "not run", never a made-up state.
        let (fresh, _s2) = sync_for(&stub.url).await;
        fresh.publish_reading(None, 0);
        assert!(!fresh.status_handle().lock().unwrap().ran);
    }

    #[tokio::test]
    async fn a_parked_shift_keeps_its_sales_unsent_and_in_order() {
        let stub = stub(|r| {
            if r.path == "/api/till/shift" && r.body.contains("\"s1\"") {
                (422, r#"{"error":"Unknown cashier"}"#.into())
            } else {
                ok(r)
            }
        })
        .await;
        let (sync, store) = sync_for(&stub.url).await;
        let open = queue(&store, "shift", r#"{"id":"s1","openedById":"ghost"}"#).await;
        let sale1 = queue(&store, "sale", r#"{"shiftId":"s1","n":1}"#).await;
        let sale2 = queue(&store, "sale", r#"{"shiftId":"s1","n":2}"#).await;
        let close = queue(&store, "shift", r#"{"id":"s1","closedAt":"x"}"#).await;
        match sync.drain_once().await {
            DrainOutcome::Parked { id, .. } => assert_eq!(id, open),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            stub.seen().len(),
            1,
            "no sale was sent past the parked shift"
        );
        // Every later pass, including after a restart (a new CloudSync on the same store).
        let again = CloudSync::new(
            store.clone(),
            SyncConfig {
                base_url: stub.url.clone(),
                token: TOKEN.into(),
            },
        )
        .unwrap();
        assert_eq!(again.drain_once().await, DrainOutcome::Idle);
        assert_eq!(stub.seen().len(), 1);
        // Nothing was lost: all four rows are undelivered, the sales untouched.
        let (n, _) = store.queue_depth().await.unwrap();
        assert_eq!(n, 4);
        let waiting: Vec<(i64, Option<String>)> =
            sqlx::query_as("select id, parked_at from outbound where sent_at is null order by id")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert_eq!(
            waiting.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec![open, sale1, sale2, close]
        );
        assert!(waiting[0].1.is_some() && waiting[1].1.is_none() && waiting[2].1.is_none());
        // A later, healthy shift still flows.
        let open2 = queue(&store, "shift", r#"{"id":"s2","openedById":"u1"}"#).await;
        let sale3 = queue(&store, "sale", r#"{"shiftId":"s2"}"#).await;
        assert_eq!(sync.drain_once().await, DrainOutcome::Sent(2));
        let seen = stub.seen();
        assert_eq!(seen[1].path, "/api/till/shift");
        assert_eq!(seen[2].path, "/api/till/sale");
        assert_eq!(ids(&store).await, Vec::<i64>::new());
        let _ = (open2, sale3);
    }

    #[tokio::test]
    async fn rows_are_sent_in_id_order_and_never_in_parallel() {
        let stub = stub(ok).await;
        let (sync, store) = sync_for(&stub.url).await;
        queue(&store, "shift", r#"{"id":"s1","seq":0}"#).await;
        for i in 1..=8 {
            queue(&store, "sale", &format!(r#"{{"shiftId":"s1","seq":{i}}}"#)).await;
        }
        assert_eq!(sync.drain_once().await, DrainOutcome::Sent(9));
        let order: Vec<String> = stub.seen().iter().map(|s| s.body.clone()).collect();
        assert!(order[0].contains(r#""seq":0"#));
        for (i, body) in order.iter().enumerate() {
            assert!(
                body.contains(&format!(r#""seq":{i}"#)),
                "position {i}: {body}"
            );
        }
        assert_eq!(
            stub.max_in_flight.load(Ordering::SeqCst),
            1,
            "never two requests at once"
        );
    }

    #[test]
    fn backoff_grows_1_2_4_and_caps_at_60s() {
        let secs: Vec<u64> = (1..=9).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff(0).as_secs(), 1);
        assert_eq!(backoff(u32::MAX).as_secs(), 60);
    }

    #[tokio::test]
    async fn a_batch_never_exceeds_50_rows() {
        let stub = stub(ok).await;
        let (sync, store) = sync_for(&stub.url).await;
        for _ in 0..120 {
            queue(&store, "sale", "{}").await;
        }
        assert_eq!(sync.drain_once().await, DrainOutcome::Sent(50));
        assert_eq!(stub.seen().len(), 50);
        assert_eq!(store.queue_depth().await.unwrap().0, 70);
    }

    #[tokio::test]
    async fn the_token_is_never_written_to_a_log_or_to_disk() {
        // pid + clock: a reused pid must not see a stale directory.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("till-sync-token-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("till.db");
        let store = Store::open(&db).await.unwrap();
        // Every failure path, each of which writes `last_error`.
        for status in [401u16, 422, 500] {
            let stub = stub(move |_| (status, "x".into())).await;
            let cfg = SyncConfig {
                base_url: stub.url.clone(),
                token: TOKEN.into(),
            };
            let sync = CloudSync::new(store.clone(), cfg.clone()).unwrap();
            queue(&store, "sale", "{}").await;
            let outcome = sync.drain_once().await;
            assert!(!format!("{outcome:?}{cfg:?}").contains(TOKEN));
            assert!(!sync.halted().unwrap_or_default().contains(TOKEN));
        }
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let sync = CloudSync::new(
            store.clone(),
            SyncConfig {
                base_url: dead,
                token: TOKEN.into(),
            },
        )
        .unwrap();
        queue(&store, "sale", "{}").await;
        let _ = sync.drain_once().await;
        let _ = sync.pull_cashiers().await;
        let errors: Vec<Option<String>> = sqlx::query_scalar("select last_error from outbound")
            .fetch_all(store.pool())
            .await
            .unwrap();
        assert!(errors.iter().any(Option::is_some));
        assert!(
            errors.iter().flatten().all(|e| !e.contains(TOKEN)),
            "{errors:?}"
        );
        // Checkpoint, then scan every byte the database and its journal hold.
        let _ = sqlx::query("pragma wal_checkpoint(truncate)")
            .execute(store.pool())
            .await;
        store.pool().close().await;
        // read_dir is lazy and SQLite removes the -wal/-shm files as the pool
        // closes (Windows reports them late), so an entry may name a file that
        // is already gone. A file that no longer exists cannot hold the token,
        // so NotFound alone is skipped; every other error fails the test.
        let mut scanned = 0;
        for entry in std::fs::read_dir(&dir).expect("the test directory is readable") {
            let path = entry.expect("a directory entry is readable").path();
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => panic!("could not scan {}: {e}", path.display()),
            };
            scanned += 1;
            assert!(
                !bytes.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()),
                "the token is on disk in {}",
                path.display()
            );
        }
        assert!(scanned >= 1, "the scan must have read the database file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn cashier_json() -> String {
        serde_json::json!([
            {"id":"u1","name":"Alice","pinHash":"$argon2id$fake","active":true},
            {"id":"u2","name":"Bob","pinHash":null,"active":false},
        ])
        .to_string()
    }

    async fn names(store: &Store) -> Vec<(String, String, i64)> {
        sqlx::query_as("select id, pin_hash, active from cashier order by id")
            .fetch_all(store.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_cashier_pull_replaces_the_local_list() {
        let stub = stub(|_| (200, cashier_json())).await;
        let (sync, store) = sync_for(&stub.url).await;
        store
            .sync_cashiers(&[Cashier {
                id: "old".into(),
                name: "Old".into(),
                pin_hash: "h".into(),
                active: true,
            }])
            .await
            .unwrap();
        assert_eq!(
            sync.pull_cashiers().await,
            Ok(Pulled {
                synced: 2,
                skipped: 0
            })
        );
        let rows = names(&store).await;
        assert_eq!(rows.len(), 2, "the old cashier is gone");
        assert_eq!(rows[0], ("u1".into(), "$argon2id$fake".into(), 1));
        // A null hash is stored as "" (never verifies) and the cashier is inactive.
        assert_eq!(rows[1], ("u2".into(), String::new(), 0));
        let seen = stub.seen();
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].path, "/api/till/cashiers");
        assert_eq!(
            seen[0].auth.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
    }

    #[tokio::test]
    async fn a_failed_pull_keeps_the_previous_list_rather_than_emptying_it() {
        let before = [Cashier {
            id: "u1".into(),
            name: "Alice".into(),
            pin_hash: "h".into(),
            active: true,
        }];
        let cases: Vec<(u16, &str)> = vec![
            (500, "down"),
            (404, "no"),
            (200, "not json"),
            (200, "[]"),
            (200, r#"[{"id":"u1"}]"#),
            (200, r#"[{"id":"","name":"x","pinHash":"h","active":true}]"#),
            (200, r#"{"not":"an array"}"#),
        ];
        for (status, body) in cases {
            let stub = stub(move |_| (status, body.to_owned())).await;
            let (sync, store) = sync_for(&stub.url).await;
            store.sync_cashiers(&before).await.unwrap();
            let _ = sync.pull_cashiers().await;
            let rows = names(&store).await;
            assert_eq!(rows, vec![("u1".into(), "h".into(), 1)], "{status} {body}");
        }
        // And with the cloud unreachable.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (sync, store) = sync_for(&dead).await;
        store.sync_cashiers(&before).await.unwrap();
        assert!(matches!(
            sync.pull_cashiers().await,
            Err(PullError::Http(_))
        ));
        assert_eq!(names(&store).await.len(), 1);
    }

    #[tokio::test]
    async fn one_malformed_cashier_costs_only_that_cashier() {
        let body = serde_json::json!([
            {"id":"u1","name":"Alice","pinHash":"h1","active":true},
            {"id":"u2","name":null,"pinHash":"h2","active":true},
            {"id":"u3","name":"Cara","pinHash":42,"active":true},
            {"name":"NoId","pinHash":"h","active":true},
            "garbage",
            {"id":"","name":"Empty","pinHash":"h","active":true},
            {"id":"u4","name":"Dan","pinHash":"h4","active":true},
        ])
        .to_string();
        let stub = stub(move |_| (200, body.clone())).await;
        let (sync, store) = sync_for(&stub.url).await;
        assert_eq!(
            sync.pull_cashiers().await,
            Ok(Pulled {
                synced: 3,
                skipped: 4
            })
        );
        assert_eq!(sync.skipped_cashiers(), 4);
        let rows: Vec<(String, String)> =
            sqlx::query_as("select id, name from cashier order by id")
                .fetch_all(store.pool())
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                ("u1".into(), "Alice".into()),
                ("u2".into(), "u2".into()), // null name falls back to the id
                ("u4".into(), "Dan".into()),
            ]
        );
    }

    #[tokio::test]
    async fn an_unreadable_record_does_not_delete_the_cashier_it_names() {
        use crate::lane::test_support::cashier;
        let (alice, bob) = (cashier("u1", "Alice", "1111"), cashier("u2", "Bob", "2222"));
        // The cloud sends Alice well-formed and Bob with a pinHash of the wrong type.
        let body = serde_json::json!([
            {"id":"u1","name":"Alice","pinHash":alice.pin_hash,"active":true},
            {"id":"u2","name":"Bob","pinHash":42,"active":true},
        ])
        .to_string();
        let stub = stub(move |_| (200, body.clone())).await;
        let (sync, store) = sync_for(&stub.url).await;
        store
            .sync_cashiers(&[alice, bob, cashier("u3", "Carl", "3333")])
            .await
            .unwrap();
        assert_eq!(
            sync.pull_cashiers().await,
            Ok(Pulled {
                synced: 1,
                skipped: 1
            })
        );
        assert_eq!(sync.skipped_cashiers(), 1);
        // Bob is still in the cloud, so he is still here and can still sign in.
        assert_eq!(
            store.verify_pin("u2", "2222").await.unwrap(),
            Some("u2".into())
        );
        assert_eq!(
            store.verify_pin("u1", "1111").await.unwrap(),
            Some("u1".into())
        );
        // A cashier genuinely absent from the response is still removed.
        assert_eq!(store.verify_pin("u3", "3333").await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_transient_failure_on_a_shift_holds_its_sales() {
        for status in [503u16, 500] {
            let stub = stub(move |r| {
                if r.path == "/api/till/shift" {
                    (status, "down".into())
                } else {
                    ok(r)
                }
            })
            .await;
            let (sync, store) = sync_for(&stub.url).await;
            let shift = queue(&store, "shift", r#"{"id":"s1"}"#).await;
            let sale = queue(&store, "sale", r#"{"shiftId":"s1"}"#).await;
            assert_eq!(sync.drain_once().await, DrainOutcome::Offline);
            let seen = stub.seen();
            assert_eq!(seen.len(), 1, "{status}: only the shift was tried");
            assert_eq!(seen[0].path, "/api/till/shift");
            assert_eq!(
                ids(&store).await,
                vec![shift, sale],
                "{status}: both still unsent, in order"
            );
            assert_eq!(
                store.parked_count().await.unwrap(),
                0,
                "{status}: nothing parked"
            );
        }
        // A dead connection holds it just the same.
        let dead = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (sync, store) = sync_for(&dead).await;
        let shift = queue(&store, "shift", r#"{"id":"s1"}"#).await;
        let sale = queue(&store, "sale", r#"{"shiftId":"s1"}"#).await;
        assert_eq!(sync.drain_once().await, DrainOutcome::Offline);
        assert_eq!(ids(&store).await, vec![shift, sale]);
    }

    #[tokio::test]
    async fn a_401_on_the_cashier_pull_holds_the_queue_too() {
        let stub = stub(|_| (401, "{}".into())).await;
        let (sync, store) = sync_for(&stub.url).await;
        queue(&store, "sale", "{}").await;
        assert_eq!(sync.pull_cashiers().await, Err(PullError::AuthRejected));
        assert!(sync.halted().is_some());
        sync.drain_once().await;
        assert_eq!(stub.seen().len(), 1, "the drain sent nothing");
    }

    #[tokio::test]
    async fn the_run_loop_drains_on_a_wake_up() {
        let stub = stub(ok).await;
        let (sync, store) = sync_for(&stub.url).await;
        let sync = Arc::new(sync);
        let waker = sync.waker();
        let task = tokio::spawn(sync.clone().run());
        tokio::time::sleep(Duration::from_millis(100)).await; // first pass: pull + idle
        queue(&store, "shift", r#"{"id":"s1"}"#).await;
        waker.notify_one();
        for _ in 0..50 {
            if ids(&store).await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        task.abort();
        assert!(
            ids(&store).await.is_empty(),
            "the wake-up drained the row long before the heartbeat"
        );
    }
}
