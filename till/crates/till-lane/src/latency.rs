//! Latency budgets for the four paths that decide whether the till feels fast.
//!
//! # What these numbers are, and are not
//!
//! The real quality bar is that scan-to-tender beats a competing product on the
//! same hardware. That cannot be measured yet: there is no pilot shop and no
//! competitor install. These budgets are a stand-in so there is a number to
//! argue with, and so the real comparison is a measurement and not a rebuild.
//!
//! They are generous on purpose. They catch an order-of-magnitude regression,
//! not jitter. The tests that use them run in a debug build on a developer's
//! machine against an in-memory store and simulated devices, which is the
//! slowest build on the fastest storage. A passing test is a sanity check. It
//! is NOT a performance guarantee and NOT a certification of the real counter.
//!
//! # Where each path starts and stops
//!
//! * key to screen: `LaneState::key` entry to return, for a character that was
//!   accepted. This is the state update only. Drawing the frame is the UI's
//!   cost and is not included, because there is no UI yet.
//! * scan to line: `LaneState::enter` entry to return, awaits included, for a
//!   scan that added a line. Refused scans are not recorded.
//! * tender to bytes: `LaneState::settle` entry to the moment the printer
//!   accepted the receipt bytes, for a sale that was confirmed and printed.
//!   The drawer kick that follows is not included. A settle that completes no
//!   sale, or whose print failed, records nothing, so the figure is never a
//!   time to a receipt that did not come out.
//! * cold start: `LaneState::new` entry to return, with the store already
//!   open. Process start, window creation and first frame are not included.

use std::time::{Duration, Instant};

/// The budgets, in milliseconds. See the module docs before trusting them.
pub struct Budget;

impl Budget {
    pub const KEY_TO_SCREEN_MS: u128 = 50;
    pub const SCAN_TO_LINE_MS: u128 = 100;
    pub const TENDER_TO_BYTES_MS: u128 = 300;
    pub const COLD_START_MS: u128 = 2000;
}

/// A running measurement. Started at the top of a path, read at its end.
#[derive(Debug, Clone, Copy)]
pub struct Timer(Instant);

impl Timer {
    pub fn start() -> Timer {
        Timer(Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

/// The most recent measurement of each path; `None` until it has happened.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub key_to_screen: Option<Duration>,
    pub scan_to_line: Option<Duration>,
    pub tender_to_bytes: Option<Duration>,
    pub cold_start: Option<Duration>,
}
