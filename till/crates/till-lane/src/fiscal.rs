//! The boundary to the fiscal device (VSDC).
//!
//! The till must save every sale to the Rwanda Revenue Authority's VSDC before
//! printing or opening the drawer. This module defines the trait that fiscal
//! clients must implement, the error cases it can produce, and a simulated
//! implementation for CI testing. Phase 3 will write the real client against
//! this trait.
//!
//! The [`FiscalPort`] trait hides the network round trip behind a synchronous
//! boundary: the caller builds the sale, calls `save_sales`, and handles the
//! error cases. No queuing or retry logic lives here — that belongs in later
//! tasks after the VSDC client is wired.

use thiserror::Error;
use till_store::VsdcReceipt;

/// The fiscal port: how the lane saves a sale to the VSDC.
///
/// The VSDC is the Rwanda Revenue Authority's device (C5 register in RRA terms).
/// Every sale must be recorded there before the till moves on. This trait lets
/// simulated and real implementations coexist.
pub trait FiscalPort {
    /// Saves a sale to the VSDC. Returns the receipt if successful, or an error
    /// if the VSDC is unreachable, rejects the sale, or has already seen the
    /// invoice number.
    ///
    /// The return value is the VSDC's answer, unchanged — not synthesised, not
    /// computed, not defaulted. If the VSDC is unreachable, the call must fail;
    /// Rwandan law prohibits offline selling, and fabricating a receipt would
    /// be fraud.
    fn save_sales(&mut self, payload: &SalePayload) -> Result<VsdcReceipt, FiscalError>;
}

/// Data the lane sends to the VSDC to record a sale.
///
/// Money fields are exact decimal strings, never floats — `till-core` owns the
/// totals and has already validated them. This struct is a carrier only; the
/// fiscal port does not compute or validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalePayload {
    /// The invoice number this sale holds (from `till-store`).
    pub invc_no: i64,
    /// The branch id (shared by every lane of the shop).
    pub bhf_id: String,
    /// The sale total, as an exact decimal string (never computed here).
    pub tot_amt: String,
    /// The taxable portion of the total, as an exact decimal string.
    pub tot_taxbl_amt: String,
    /// The tax on the taxable portion, as an exact decimal string.
    pub tot_tax_amt: String,
}

/// Errors the fiscal port can return.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FiscalError {
    /// The VSDC is not reachable: network is down, the device is offline,
    /// or the connection timed out. This is a genuine failure, never
    /// fabricated into a receipt — Rwandan law prohibits offline selling.
    #[error("VSDC unreachable")]
    Unreachable,

    /// The VSDC rejected the sale with a code and message. The till must not
    /// retry with the same invoice number; some rejections (bad item class,
    /// deregistered device) are permanent.
    #[error("VSDC rejected: {code} {msg}")]
    Rejected { code: String, msg: String },

    /// The VSDC has already recorded a sale with this invoice number (RRA
    /// response code 994). The specification offers no way to re-read the
    /// original receipt, and no endpoint to report the duplicate. The sale
    /// cannot be recovered, and the invoice number must be abandoned; a later
    /// task will handle the escape hatch. This variant carries no receipt
    /// because there is genuinely nothing to recover.
    #[error("VSDC: invoice number already in use")]
    Overlapped,
}

/// Mode the simulated fiscal port can run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimMode {
    /// Return a successful receipt with an incrementing receipt number.
    Succeed,
    /// Return an `Unreachable` error.
    Unreachable,
    /// Return a `Rejected` error with the given code and message.
    Reject(String),
    /// Return an `Overlapped` error (invoice number already in use).
    Overlapped,
}

/// A simulated VSDC for CI testing.
///
/// Each call to `save_sales` increments `next_rcpt_no` and returns a receipt with
/// that number, unless `mode` says otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimFiscal {
    /// The receipt number to return on the next successful save.
    pub next_rcpt_no: i64,
    /// How to respond: succeed, fail with Unreachable, reject with a code/msg,
    /// or return Overlapped.
    pub mode: SimMode,
}

impl FiscalPort for SimFiscal {
    fn save_sales(&mut self, payload: &SalePayload) -> Result<VsdcReceipt, FiscalError> {
        match self.mode {
            SimMode::Succeed => {
                let receipt = VsdcReceipt {
                    rcpt_no: self.next_rcpt_no,
                    intrl_data: format!("sim-{}", payload.invc_no),
                    rcpt_sign: "SIM_SIGNATURE".to_string(),
                    sdc_id: "SIM_SDC_ID".to_string(),
                    mrc_no: "SIM_MRC".to_string(),
                    vsdc_rcpt_pbct_date: "20261121000000".to_string(),
                    tot_rcpt_no: 1,
                };
                self.next_rcpt_no += 1;
                Ok(receipt)
            }
            SimMode::Unreachable => Err(FiscalError::Unreachable),
            SimMode::Reject(ref msg) => {
                let parts: Vec<&str> = msg.splitn(2, ' ').collect();
                let code = parts.first().copied().unwrap_or("999").to_string();
                let msg_text = parts.get(1).copied().unwrap_or("rejection").to_string();
                Err(FiscalError::Rejected {
                    code,
                    msg: msg_text,
                })
            }
            SimMode::Overlapped => Err(FiscalError::Overlapped),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_simulated_port_returns_an_ascending_receipt_number() {
        let mut fiscal = SimFiscal {
            next_rcpt_no: 100,
            mode: SimMode::Succeed,
        };
        let payload = SalePayload {
            invc_no: 1,
            bhf_id: "00".to_string(),
            tot_amt: "1000.00".to_string(),
            tot_taxbl_amt: "1000.00".to_string(),
            tot_tax_amt: "0.00".to_string(),
        };

        let first = fiscal.save_sales(&payload).unwrap();
        assert_eq!(first.rcpt_no, 100);

        let second = fiscal.save_sales(&payload).unwrap();
        assert_eq!(second.rcpt_no, 101);

        let third = fiscal.save_sales(&payload).unwrap();
        assert_eq!(third.rcpt_no, 102);

        assert_eq!(fiscal.next_rcpt_no, 103);
    }

    #[test]
    fn an_unreachable_port_is_an_error_not_an_empty_receipt() {
        let mut fiscal = SimFiscal {
            next_rcpt_no: 1,
            mode: SimMode::Unreachable,
        };
        let payload = SalePayload {
            invc_no: 1,
            bhf_id: "00".to_string(),
            tot_amt: "1000.00".to_string(),
            tot_taxbl_amt: "1000.00".to_string(),
            tot_tax_amt: "0.00".to_string(),
        };

        let result = fiscal.save_sales(&payload);
        assert!(matches!(result, Err(FiscalError::Unreachable)));
        // Critically: the receipt number is NOT incremented on failure.
        assert_eq!(fiscal.next_rcpt_no, 1);
    }

    #[test]
    fn a_rejection_carries_the_code_the_vsdc_gave() {
        let mut fiscal = SimFiscal {
            next_rcpt_no: 1,
            mode: SimMode::Reject("894 Communication timeout".to_string()),
        };
        let payload = SalePayload {
            invc_no: 1,
            bhf_id: "00".to_string(),
            tot_amt: "1000.00".to_string(),
            tot_taxbl_amt: "1000.00".to_string(),
            tot_tax_amt: "0.00".to_string(),
        };

        let result = fiscal.save_sales(&payload);
        assert!(matches!(result, Err(FiscalError::Rejected { .. })));
        if let Err(FiscalError::Rejected { code, msg }) = result {
            assert_eq!(code, "894");
            assert_eq!(msg, "Communication timeout");
        }
    }

    #[test]
    fn overlapped_carries_no_receipt() {
        let mut fiscal = SimFiscal {
            next_rcpt_no: 1,
            mode: SimMode::Overlapped,
        };
        let payload = SalePayload {
            invc_no: 1,
            bhf_id: "00".to_string(),
            tot_amt: "1000.00".to_string(),
            tot_taxbl_amt: "1000.00".to_string(),
            tot_tax_amt: "0.00".to_string(),
        };

        let result = fiscal.save_sales(&payload);
        assert!(matches!(result, Err(FiscalError::Overlapped)));
        // No receipt is fabricated; the number is not incremented.
        assert_eq!(fiscal.next_rcpt_no, 1);
    }
}
