//! Receiver measurements shared between the backend (which writes them from
//! the SDRplay API callbacks) and the outputs (rtl_tcp, future hamlib).
//!
//! They are plain atomic values: no output depends on the backend, and several
//! outputs can read the same measurements.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static TOTAL_GAIN_TENTHS_DB: AtomicI32 = AtomicI32::new(0);
static TOTAL_GAIN_VALID: AtomicBool = AtomicBool::new(false);
static OVERLOAD: AtomicBool = AtomicBool::new(false);

/// Current total gain of the RSP (LNA + IF), as computed by the SDRplay API.
pub fn set_total_gain_db(gain_db: f64) {
    if gain_db.is_finite() {
        TOTAL_GAIN_TENTHS_DB.store((gain_db * 10.0).round() as i32, Ordering::Relaxed);
        TOTAL_GAIN_VALID.store(true, Ordering::Relaxed);
    }
}

/// Total gain in dB, or `None` until the API has reported one.
pub fn total_gain_db() -> Option<f64> {
    if TOTAL_GAIN_VALID.load(Ordering::Relaxed) {
        Some(TOTAL_GAIN_TENTHS_DB.load(Ordering::Relaxed) as f64 / 10.0)
    } else {
        None
    }
}

/// ADC overload reported by the SDRplay service.
pub fn set_overload(overload: bool) {
    OVERLOAD.store(overload, Ordering::Relaxed);
}

pub fn overload() -> bool {
    OVERLOAD.load(Ordering::Relaxed)
}
