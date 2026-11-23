//! What the till knows about its own cloud sync, ready to be shown.
//!
//! The sync loop publishes a [`SyncStatus`] after every pass; the screen only
//! reads it. The UI therefore never touches the database, and the figures are
//! normally one heartbeat (15 s) old, or newer when a sale wakes the loop. While
//! the cloud is unreachable a pass can take up to ~100 s (cashier pull timeout
//! 20 s + drain timeout 20 s + 60 s maximum backoff).
//!
//! Everything here reports what LAST HAPPENED. Nothing is tested live, so no
//! label claims the cloud is reachable now. A backlog is not a fault (it is the
//! normal state while the back office's tunnel is down); a held or refused
//! queue is, because it never clears by itself. None of this is a cashier
//! `Notice`: it is ambient, never blocks a sale, and the strings below name no
//! action for the cashier.

use std::sync::{Arc, Mutex};

/// The shared snapshot: written by the sync loop, read by the screen.
pub type SharedSyncStatus = Arc<Mutex<SyncStatus>>;

/// The oldest unsent row must be this old (seconds) before its age is shown.
/// Under it, a queue is just the normal gap between a sale and the next beat.
pub const AGE_SHOWN_AFTER_SECS: u64 = 15 * 60;
/// A snapshot older than this (seconds) is not reported as current: a sync
/// task that stalled or died must not read as "nothing waiting" forever.
/// `publish_status` runs after the cashier pull and the drain, so while the
/// cloud is down the gap between publishes is up to ~100 s (a 20 s pull
/// timeout, a 20 s drain timeout and a 60 s maximum backoff). The threshold must clear that
/// with slack, or ordinary offline trading would raise a warning nobody
/// believes. It catches a stall that matters over hours, not seconds.
pub const STALE_AFTER_SECS: u64 = 180;
/// No label is longer than this many characters; a test measures it.
pub const LABEL_MAX_CHARS: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncStatus {
    /// At least one drain pass has finished since the lane started.
    pub ran: bool,
    /// Unsent rows, parked ones included.
    pub depth: u32,
    /// `created_at` of the oldest unsent row.
    pub oldest: Option<String>,
    /// Unsent rows the cloud refused for good.
    pub parked: u32,
    /// The cloud rejected this lane's token; nothing is sent until restart.
    pub held: bool,
    /// Cashier records the last pull could not read.
    pub skipped_cashiers: usize,
    /// Unix seconds of the last time the queue was read (or, before the first
    /// read, when sync started). `None` is never reported stale.
    pub published_at: Option<u64>,
}

/// How loudly a chip should be drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Nothing to see: draining, or waiting for the next beat.
    Quiet,
    /// Worth a look, not a fault.
    Notice,
    /// Needs someone: never clears by itself.
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncChip {
    pub label: String,
    pub tone: Tone,
}

impl SyncStatus {
    /// The chips to draw, most important first. `now` is Unix seconds.
    pub fn chips(&self, now: u64) -> Vec<SyncChip> {
        let mut out = Vec::new();
        // The cap is enforced, not just tested: a label is cut, never grown.
        let chip = |label: String, tone| SyncChip {
            label: label.chars().take(LABEL_MAX_CHARS).collect(),
            tone,
        };
        if self.held {
            out.push(chip("Sync stopped: token rejected".into(), Tone::Stopped));
        } else if self.parked > 0 {
            out.push(chip(
                format!("Sync: {} refused by cloud", count(self.parked)),
                Tone::Stopped,
            ));
        } else if let Some(at) = self
            .published_at
            .filter(|at| now.saturating_sub(*at) > STALE_AFTER_SECS)
        {
            // The loop has not reported lately; the figures below would be
            // history, so none of them is shown as the current state.
            out.push(chip(
                format!("Sync: no report since {}", hhmm(at)),
                Tone::Notice,
            ));
        } else if !self.ran {
            out.push(chip("Sync: not run yet".into(), Tone::Quiet));
        } else if self.depth == 0 {
            out.push(chip("Sync: nothing waiting".into(), Tone::Quiet));
        } else {
            let age = self
                .oldest
                .as_deref()
                .and_then(unix_of)
                .map(|t| now.saturating_sub(t))
                .filter(|a| *a >= AGE_SHOWN_AFTER_SECS);
            let label = match age {
                Some(a) => format!(
                    "Sync: {} waiting · oldest {}",
                    count(self.depth),
                    age_text(a)
                ),
                None => format!("Sync: {} waiting", count(self.depth)),
            };
            out.push(chip(label, Tone::Quiet));
        }
        if self.skipped_cashiers > 0 {
            let n = count_usize(self.skipped_cashiers);
            let noun = if self.skipped_cashiers == 1 {
                "record"
            } else {
                "records"
            };
            out.push(chip(format!("{n} cashier {noun} unread"), Tone::Notice));
        }
        out
    }
}

/// Kigali clock time (UTC+2, as the rest of the lane shows it) of a Unix instant.
fn hhmm(unix: u64) -> String {
    let local = (unix + 2 * 3600) % 86_400;
    format!("{:02}:{:02}", local / 3600, local % 3600 / 60)
}

fn count(n: u32) -> String {
    if n > 999 {
        "999+".into()
    } else {
        n.to_string()
    }
}

fn count_usize(n: usize) -> String {
    count(u32::try_from(n).unwrap_or(u32::MAX))
}

/// `47m`, `3h 05m`, `2d 4h`, capped at `99d+`.
fn age_text(secs: u64) -> String {
    let mins = secs / 60;
    if mins < 60 {
        format!("{mins}m")
    } else if mins < 48 * 60 {
        format!("{}h {:02}m", mins / 60, mins % 60)
    } else {
        let days = mins / (24 * 60);
        if days > 99 {
            "99d+".into()
        } else {
            format!("{days}d {}h", (mins / 60) % 24)
        }
    }
}

/// Unix seconds of a store instant (`2026-11-22T13:45:00Z`, UTC); `None` if it
/// is not in that shape.
pub fn unix_of(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: i64 = d.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let day: i64 = d.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
    let mut t = time.get(..8)?.split(':');
    let h: i64 = t.next()?.parse().ok().filter(|h| *h < 24)?;
    let mi: i64 = t.next()?.parse().ok().filter(|m| *m < 60)?;
    let sec: i64 = t.next()?.parse().ok().filter(|s| *s < 61)?;
    // Days since 1970-01-01 (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + sec).ok()
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_795_000_000;

    fn at(secs_ago: u64) -> String {
        // Inverse of `unix_of`, via a fixed reference and the same function.
        let t = NOW - secs_ago;
        let days = t / 86_400;
        let rem = t % 86_400;
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    }

    fn st() -> SyncStatus {
        SyncStatus {
            ran: true,
            ..Default::default()
        }
    }

    #[test]
    fn instants_parse_and_round_trip() {
        assert_eq!(unix_of("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_of("2026-11-22T13:45:00Z"), Some(1_795_355_100));
        assert_eq!(unix_of(&at(1234)), Some(NOW - 1234));
        assert_eq!(unix_of("garbage"), None);
        assert_eq!(unix_of("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn a_drained_queue_shows_nothing_alarming() {
        let c = st().chips(NOW);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].label, "Sync: nothing waiting");
        assert_eq!(c[0].tone, Tone::Quiet);
        let fresh = SyncStatus::default().chips(NOW);
        assert_eq!(fresh[0].tone, Tone::Quiet);
    }

    #[test]
    fn an_unsent_backlog_shows_its_depth_and_is_not_an_error() {
        let s = SyncStatus {
            depth: 7,
            oldest: Some(at(60)),
            ..st()
        };
        let c = s.chips(NOW);
        assert_eq!(c[0].label, "Sync: 7 waiting");
        assert_eq!(c[0].tone, Tone::Quiet);
        let huge = SyncStatus {
            depth: 50_000,
            ..st()
        };
        assert_eq!(huge.chips(NOW)[0].label, "Sync: 999+ waiting");
    }

    #[test]
    fn the_oldest_unsent_age_is_shown_once_it_passes_a_threshold() {
        let mk = |ago| SyncStatus {
            depth: 3,
            oldest: Some(at(ago)),
            ..st()
        };
        assert_eq!(
            mk(AGE_SHOWN_AFTER_SECS - 1).chips(NOW)[0].label,
            "Sync: 3 waiting"
        );
        assert_eq!(
            mk(AGE_SHOWN_AFTER_SECS).chips(NOW)[0].label,
            "Sync: 3 waiting · oldest 15m"
        );
        assert_eq!(
            mk(3 * 3600 + 5 * 60).chips(NOW)[0].label,
            "Sync: 3 waiting · oldest 3h 05m"
        );
        assert_eq!(
            mk(3 * 86_400 + 4 * 3600).chips(NOW)[0].label,
            "Sync: 3 waiting · oldest 3d 4h"
        );
        // An age is still only a backlog: quiet, however old.
        assert_eq!(mk(5 * 86_400).chips(NOW)[0].tone, Tone::Quiet);
        // An unreadable timestamp drops the age, never the depth.
        let bad = SyncStatus {
            depth: 3,
            oldest: Some("???".into()),
            ..st()
        };
        assert_eq!(bad.chips(NOW)[0].label, "Sync: 3 waiting");
    }

    #[test]
    fn a_parked_queue_says_so_and_names_no_cashier_action() {
        let s = SyncStatus {
            depth: 4,
            parked: 2,
            ..st()
        };
        let c = s.chips(NOW);
        assert_eq!(c[0].label, "Sync: 2 refused by cloud");
        assert_eq!(c[0].tone, Tone::Stopped);
        let l = c[0].label.to_lowercase();
        assert!(!l.contains("retry") && !l.contains("please") && !l.contains("press"));
    }

    #[test]
    fn a_rejected_token_is_stopped_and_outranks_everything() {
        let s = SyncStatus {
            depth: 4,
            parked: 2,
            held: true,
            oldest: Some(at(9999)),
            ..st()
        };
        let c = s.chips(NOW);
        assert_eq!(c[0].label, "Sync stopped: token rejected");
        assert_eq!(c[0].tone, Tone::Stopped);
    }

    #[test]
    fn unreadable_cashier_records_get_their_own_chip() {
        let s = SyncStatus {
            skipped_cashiers: 2,
            ..st()
        };
        let c = s.chips(NOW);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].label, "2 cashier records unread");
        let one = SyncStatus {
            skipped_cashiers: 1,
            ..st()
        };
        assert_eq!(one.chips(NOW)[1].label, "1 cashier record unread");
        assert_eq!(c[1].tone, Tone::Notice);
    }

    #[test]
    fn a_silent_sync_is_not_reported_as_nothing_waiting() {
        let mk = |ago| SyncStatus {
            published_at: Some(NOW - ago),
            ..st()
        };
        assert_eq!(
            mk(STALE_AFTER_SECS).chips(NOW)[0].label,
            "Sync: nothing waiting"
        );
        let c = mk(STALE_AFTER_SECS + 1).chips(NOW);
        assert_eq!(c[0].tone, Tone::Notice);
        assert_eq!(
            c[0].label,
            format!("Sync: no report since {}", hhmm(NOW - STALE_AFTER_SECS - 1))
        );
        // A backlog that has gone silent is stale too, not "N waiting".
        let b = SyncStatus {
            depth: 9,
            published_at: Some(NOW - 600),
            ..st()
        };
        assert!(b.chips(NOW)[0].label.starts_with("Sync: no report since"));
        // A sync that never reported at all goes stale from its start time.
        let never = SyncStatus {
            ran: false,
            published_at: Some(NOW - 600),
            ..Default::default()
        };
        assert!(
            never.chips(NOW)[0]
                .label
                .starts_with("Sync: no report since")
        );
        // A hold never clears by itself, so staleness does not hide it.
        let held = SyncStatus {
            held: true,
            published_at: Some(NOW - 600),
            ..st()
        };
        assert_eq!(held.chips(NOW)[0].label, "Sync stopped: token rejected");
    }

    #[test]
    fn a_parked_row_beats_the_stale_state() {
        let s = SyncStatus {
            parked: 2,
            published_at: Some(NOW - 600),
            ..st()
        };
        let c = s.chips(NOW);
        assert_eq!(c[0].label, "Sync: 2 refused by cloud");
        assert_eq!(c[0].tone, Tone::Stopped);
    }

    #[test]
    fn the_widest_backlog_label_is_the_one_measured() {
        let s = SyncStatus {
            depth: u32::MAX,
            published_at: Some(NOW),
            oldest: Some("1970-01-01T00:00:00Z".into()),
            ..st()
        };
        assert_eq!(s.chips(NOW)[0].label, "Sync: 999+ waiting · oldest 99d+");
    }

    #[test]
    fn the_stale_time_is_the_kigali_clock() {
        assert_eq!(hhmm(1_795_355_100), "15:45");
    }

    #[test]
    fn no_label_can_outgrow_its_cap() {
        let worst = [
            SyncStatus {
                held: true,
                skipped_cashiers: usize::MAX,
                ..st()
            },
            SyncStatus {
                parked: u32::MAX,
                ..st()
            },
            SyncStatus {
                depth: u32::MAX,
                published_at: Some(NOW),
                oldest: Some("1970-01-01T00:00:00Z".into()),
                ..st()
            },
        ];
        for s in worst {
            for c in s.chips(NOW) {
                assert!(
                    !c.label.contains("no report"),
                    "{:?}: a stale row measures nothing",
                    c.label
                );
                assert!(
                    c.label.chars().count() <= LABEL_MAX_CHARS,
                    "{:?} is too long",
                    c.label
                );
            }
        }
    }
}
