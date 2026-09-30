//! Mesures du récepteur partagées entre le backend (qui les écrit depuis les
//! callbacks de l'API SDRplay) et les sorties (rtl_tcp, futur hamlib).
//!
//! Ce sont de simples valeurs atomiques : aucune sortie ne dépend ainsi du
//! backend, et plusieurs sorties peuvent lire les mêmes mesures.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static TOTAL_GAIN_TENTHS_DB: AtomicI32 = AtomicI32::new(0);
static TOTAL_GAIN_VALID: AtomicBool = AtomicBool::new(false);
static OVERLOAD: AtomicBool = AtomicBool::new(false);

/// Gain total courant du RSP (LNA + IF), tel que calculé par l'API SDRplay.
pub fn set_total_gain_db(gain_db: f64) {
    if gain_db.is_finite() {
        TOTAL_GAIN_TENTHS_DB.store((gain_db * 10.0).round() as i32, Ordering::Relaxed);
        TOTAL_GAIN_VALID.store(true, Ordering::Relaxed);
    }
}

/// Gain total en dB, ou `None` tant que l'API n'en a signalé aucun.
pub fn total_gain_db() -> Option<f64> {
    if TOTAL_GAIN_VALID.load(Ordering::Relaxed) {
        Some(TOTAL_GAIN_TENTHS_DB.load(Ordering::Relaxed) as f64 / 10.0)
    } else {
        None
    }
}

/// Surcharge ADC signalée par le service SDRplay.
pub fn set_overload(overload: bool) {
    OVERLOAD.store(overload, Ordering::Relaxed);
}

pub fn overload() -> bool {
    OVERLOAD.load(Ordering::Relaxed)
}
