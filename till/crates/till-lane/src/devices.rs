//! The hardware the lane talks to, held as trait objects so the UI neither
//! knows nor cares whether a printer is a thermal unit or a simulation.

use crate::device::{DeviceError, Drawer, Printer, Scale, SimDrawer, SimScale};
use crate::fiscal::{FiscalPort, OfflineFlag, SimFiscal, SimMode, SwitchableSim};
use std::cell::Cell;
use std::io::Write;
use std::rc::Rc;

/// Everything the lane drives. No earlier task owns this type; the UI does.
pub struct Devices {
    pub printer: Box<dyn Printer>,
    pub drawer: Box<dyn Drawer>,
    /// Held for the design's weighed-goods flow; nothing reads it yet
    /// (`LaneState` takes weight from the scale label on the barcode).
    #[allow(dead_code)]
    pub scale: Box<dyn Scale>,
    pub fiscal: Box<dyn FiscalPort>,
    /// Demo only: the flag behind the simulated VSDC's reachability, so an
    /// operator can take it offline and exercise the C1 hard stop by hand.
    /// `None` for real devices, which makes `toggle_vsdc` a no-op there — a
    /// real VSDC's reachability is the network's business, not a keystroke's.
    vsdc_offline: Option<OfflineFlag>,
}

impl Devices {
    /// Simulated devices for `--demo`. The fiscal port invents receipts: it
    /// must never be wired up outside the demo.
    ///
    /// `vsdc_offline` is the VSDC's starting reachability, and it stays
    /// flippable while the lane runs (see [`Devices::toggle_vsdc`]).
    pub fn demo(vsdc_offline: bool) -> Devices {
        let flag: OfflineFlag = Rc::new(Cell::new(vsdc_offline));
        Devices {
            printer: Box::new(StdoutPrinter),
            drawer: Box::new(SimDrawer { kicks: 0 }),
            scale: Box::new(SimScale { next: None }),
            fiscal: Box::new(SwitchableSim::new(
                SimFiscal {
                    next_rcpt_no: 1,
                    mode: SimMode::Succeed,
                },
                Rc::clone(&flag),
            )),
            vsdc_offline: Some(flag),
        }
    }

    /// Takes the simulated VSDC offline, or puts it back. Returns the new
    /// state, or `None` when there is no simulator to switch.
    pub fn toggle_vsdc(&mut self) -> Option<bool> {
        let flag = self.vsdc_offline.as_ref()?;
        let now = !flag.get();
        flag.set(now);
        Some(now)
    }

    /// Whether the simulated VSDC is currently pretending to be unreachable.
    pub fn vsdc_offline(&self) -> bool {
        self.vsdc_offline.as_ref().is_some_and(|f| f.get())
    }
}

/// A printer that shows the bytes it is given on stdout: printable ASCII and
/// newlines as themselves, everything else (ESC/POS commands) as `<XX>`.
pub struct StdoutPrinter;

impl Printer for StdoutPrinter {
    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        let mut shown = String::with_capacity(bytes.len());
        for &b in bytes {
            match b {
                b'\n' | 0x20..=0x7E => shown.push(char::from(b)),
                _ => shown.push_str(&format!("<{b:02X}>")),
            }
        }
        let mut out = std::io::stdout().lock();
        writeln!(out, "--- printer ({} bytes) ---\n{shown}", bytes.len())
            .and_then(|()| out.flush())
            .map_err(|e| DeviceError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::device::{
        Drawer, Printer, Scale, ScanSource, SimDrawer, SimPrinter, SimScale, SimScanner,
    };
    use crate::fiscal::{FiscalPort, SimFiscal, SimMode};

    /// Dyn-compatibility is part of each trait's contract: `Devices` holds them
    /// as `Box<dyn _>`. Adding a generic method to any of them fails HERE, not
    /// somewhere unrelated. Compile-time only; the lets are the assertion.
    #[test]
    fn the_device_traits_are_dyn_compatible() {
        let _: &mut dyn Printer = &mut SimPrinter {
            written: vec![],
            fail_next: false,
        };
        let _: &mut dyn Drawer = &mut SimDrawer { kicks: 0 };
        let _: &mut dyn Scale = &mut SimScale { next: None };
        let _: &mut dyn ScanSource = &mut SimScanner {
            queued: Default::default(),
        };
        let _: &mut dyn FiscalPort = &mut SimFiscal {
            next_rcpt_no: 1,
            mode: SimMode::Succeed,
        };
    }
}
