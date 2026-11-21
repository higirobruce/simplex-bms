//! Hardware abstraction layer for the lane: printer, drawer, scale, scanner.
//!
//! Each device is hidden behind a trait so that simulated implementations can run
//! in CI without real hardware. The simulated versions expose internal state for
//! testing: written bytes, kick count, queued scans. The printer's `fail_next` is
//! a one-shot failure mechanism that proves a fiscal failure after a sale is
//! confirmed does not undo the sale.

use std::collections::VecDeque;
use thiserror::Error;
use till_core::Qty;

/// Writes bytes to the ESC/POS printer (thermal receipt printer).
pub trait Printer {
    /// Sends bytes to the printer. Returns an error if the device is not connected,
    /// experiences I/O trouble, or times out waiting for readiness.
    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError>;
}

/// Opens the cash drawer.
pub trait Drawer {
    /// Sends a "drawer kick" pulse to open the drawer. Returns an error if the
    /// device is not connected or times out.
    fn kick(&mut self) -> Result<(), DeviceError>;
}

/// Reads weight from a serial scale.
pub trait Scale {
    /// Reads the current weight on the scale. Returns an error if the device is
    /// not connected or times out waiting for a reading. Nothing calls this in
    /// this build: the lane takes weights from scale labels, not a serial scale.
    /// It is part of the design spec and must exist.
    #[expect(dead_code, reason = "the serial scale is not wired into the lane yet")]
    fn read_weight(&mut self) -> Result<Qty, DeviceError>;
}

/// Polls a barcode scanner that acts as a HID keyboard wedge.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "nothing polls a scanner in this build; keystrokes arrive through the UI"
    )
)]
pub trait ScanSource {
    /// Returns the next scanned barcode, or None if no scan is queued. This trait
    /// is not called in this phase because the operating system delivers keyboard
    /// wedge input as ordinary keystrokes. Nothing polls it in this build; it is
    /// part of the design spec and must exist.
    fn poll(&mut self) -> Option<String>;
}

/// Errors raised by any device.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeviceError {
    /// The device is not connected or powered off.
    #[error("device not connected")]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no simulated device reports being disconnected yet"
        )
    )]
    NotConnected,
    /// I/O error communicating with the device.
    #[error("I/O error: {0}")]
    Io(String),
    /// The device did not respond within the timeout window.
    #[error("device timeout")]
    #[expect(
        dead_code,
        reason = "only the scale's timeout builds this, and the scale is unused"
    )]
    Timeout,
}

/// A simulated ESC/POS printer that records everything sent to it.
///
/// The `written` field is what later tasks assert receipt bytes against. The
/// `fail_next` flag is load-bearing for the proof that a printer failure after a
/// sale is fiscally confirmed does not undo the sale.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "tests use it; the running lane uses the devices in devices.rs"
    )
)]
pub struct SimPrinter {
    /// Every byte written to the printer since construction.
    pub written: Vec<u8>,
    /// If true, the next write fails and clears this flag. A one-shot mechanism so
    /// tests can provoke exactly one failure.
    pub fail_next: bool,
}

impl Printer for SimPrinter {
    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        if self.fail_next {
            self.fail_next = false;
            return Err(DeviceError::NotConnected);
        }
        self.written.extend_from_slice(bytes);
        Ok(())
    }
}

/// A simulated cash drawer that counts how many times it was kicked.
#[derive(Debug)]
pub struct SimDrawer {
    /// Number of times kick() succeeded.
    pub kicks: u32,
}

impl Drawer for SimDrawer {
    fn kick(&mut self) -> Result<(), DeviceError> {
        self.kicks += 1;
        Ok(())
    }
}

/// A simulated scale that returns a programmed weight.
///
/// Set `next` to `Some(qty)` before calling read_weight to simulate a reading.
/// Set it to `None` to simulate a timeout.
#[derive(Debug)]
pub struct SimScale {
    /// The weight to return on the next read, or None to trigger a timeout.
    #[expect(dead_code, reason = "read only by the unused Scale::read_weight")]
    pub next: Option<Qty>,
}

impl Scale for SimScale {
    fn read_weight(&mut self) -> Result<Qty, DeviceError> {
        self.next.ok_or(DeviceError::Timeout)
    }
}

/// A simulated barcode scanner with a queue of incoming scans.
///
/// Each call to poll removes and returns the front of the queue, or None if
/// empty. It models the behavior of a HID keyboard wedge, which the operating
/// system will also deliver as keystrokes in a real till.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "nothing polls a scanner in this build")
)]
pub struct SimScanner {
    /// A queue of barcodes waiting to be scanned. Each poll removes the front.
    pub queued: VecDeque<String>,
}

impl ScanSource for SimScanner {
    fn poll(&mut self) -> Option<String> {
        self.queued.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simulated_printer_records_what_it_was_given() {
        let mut printer = SimPrinter {
            written: vec![],
            fail_next: false,
        };

        // First write
        assert!(printer.write(b"hello").is_ok());
        assert_eq!(printer.written, b"hello");

        // Second write concatenates
        assert!(printer.write(b" world").is_ok());
        assert_eq!(printer.written, b"hello world");
    }

    #[test]
    fn a_simulated_printer_can_be_told_to_fail_once() {
        let mut printer = SimPrinter {
            written: vec![],
            fail_next: true,
        };

        // First write fails
        assert!(matches!(
            printer.write(b"test"),
            Err(DeviceError::NotConnected)
        ));

        // fail_next cleared automatically
        assert!(!printer.fail_next);

        // Second write succeeds
        assert!(printer.write(b"test").is_ok());
        assert_eq!(printer.written, b"test");
    }

    #[test]
    fn a_simulated_drawer_counts_kicks() {
        let mut drawer = SimDrawer { kicks: 0 };

        assert!(drawer.kick().is_ok());
        assert_eq!(drawer.kicks, 1);

        assert!(drawer.kick().is_ok());
        assert_eq!(drawer.kicks, 2);
    }

    #[test]
    fn a_simulated_scanner_yields_each_queued_scan_once_then_none() {
        let mut scanner = SimScanner {
            queued: vec!["scan1".to_string(), "scan2".to_string()]
                .into_iter()
                .collect(),
        };

        assert_eq!(scanner.poll(), Some("scan1".to_string()));
        assert_eq!(scanner.poll(), Some("scan2".to_string()));
        assert_eq!(scanner.poll(), None);
    }
}
