//! RSP1B gain handling, band by band.
//!
//! Why this module exists
//! ----------------------
//! On an RSP, gain is not a single number of dB: it is set with TWO
//! parameters, `LNAstate` and `gRdB` (gain reduction of the IF stage,
//! 20..=59 dB). The meaning of an `LNAstate` DEPENDS ON THE BAND (API
//! specification v3.15, chapter 5, "Gain Reduction Tables"):
//!   * below 60 MHz    : 7 states (0..=6)
//!   * 60-420 MHz      : 10 states (0..=9)
//!   * 420-1000 MHz    : 10 states (0..=9), different attenuations
//!   * 1000-2000 MHz   : 9 states (0..=8)
//! A state that does not exist in the band is refused by the service
//! (OutOfRange), and a valid state does not attenuate by the same amount from
//! one band to another.
//!
//! A "dB -> (LNA, gRdB)" table measured at a single frequency is therefore only
//! valid in its own band. Here: 29 gain steps (like the R820T gain list rtl_tcp
//! clients expect), with one table PER BAND.
//!
//! Origin of the values: RSP1B tables from SDRplay's `RSPTCPServer` (rsp_tcp.c:
//! GNU GPL version 2 or later header, repository under GPL-3.0), same band
//! limits. Index 0 = minimum gain, index 28 = maximum gain (LNAstate 0,
//! gRdB 20).

/// Number of gain steps exposed to rtl_tcp clients.
pub const GAIN_STEPS: usize = 29;

/// Gain step at start-up: middle of the scale.
pub const DEFAULT_GAIN_INDEX: usize = 14;

// < 60 MHz
const AM_LNA: [u8; GAIN_STEPS] = [
    6, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 4, 4, 3, 3, 3, 3, 3, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
const AM_IFGR: [u8; GAIN_STEPS] = [
    59, 55, 52, 48, 45, 41, 57, 53, 49, 46, 42, 44, 40, 56, 52, 48, 45, 41, 44, 40, 43, 45, 41, 38,
    34, 31, 27, 24, 20,
];

// 60-120 MHz
const VHF_LNA: [u8; GAIN_STEPS] = [
    9, 9, 9, 9, 9, 9, 8, 7, 7, 7, 7, 7, 6, 6, 5, 5, 4, 3, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
const VHF_IFGR: [u8; GAIN_STEPS] = [
    59, 55, 52, 48, 45, 41, 42, 58, 54, 51, 47, 43, 46, 42, 44, 41, 43, 42, 44, 40, 43, 45, 42, 38,
    34, 31, 27, 24, 20,
];

// 120-250 MHz
const BAND3_LNA: [u8; GAIN_STEPS] = [
    9, 9, 9, 9, 9, 9, 8, 7, 7, 7, 7, 7, 6, 6, 5, 5, 4, 3, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
const BAND3_IFGR: [u8; GAIN_STEPS] = [
    59, 55, 52, 48, 45, 41, 42, 58, 54, 51, 47, 43, 46, 42, 44, 41, 43, 42, 44, 40, 43, 45, 42, 38,
    34, 31, 27, 24, 20,
];

// 250-420 MHz
const BANDX_LNA: [u8; GAIN_STEPS] = [
    9, 9, 9, 9, 9, 9, 8, 7, 7, 7, 7, 7, 6, 6, 5, 5, 4, 3, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
const BANDX_IFGR: [u8; GAIN_STEPS] = [
    59, 55, 52, 48, 45, 41, 42, 58, 54, 51, 47, 43, 46, 42, 44, 41, 43, 42, 44, 40, 43, 45, 42, 38,
    34, 31, 27, 24, 20,
];

// 420-1000 MHz
const BAND45_LNA: [u8; GAIN_STEPS] = [
    9, 9, 9, 9, 9, 9, 8, 8, 8, 8, 8, 7, 6, 6, 5, 5, 4, 4, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
const BAND45_IFGR: [u8; GAIN_STEPS] = [
    59, 55, 52, 48, 44, 41, 56, 52, 49, 45, 41, 44, 46, 42, 45, 41, 44, 40, 44, 40, 42, 46, 42, 38,
    35, 31, 27, 24, 20,
];

// 1000-2000 MHz
const LBAND_LNA: [u8; GAIN_STEPS] = [
    8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 6, 5, 5, 4, 4, 3, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0,
];
const LBAND_IFGR: [u8; GAIN_STEPS] = [
    59, 55, 52, 48, 45, 41, 56, 53, 49, 46, 42, 43, 46, 42, 44, 41, 43, 48, 44, 40, 43, 45, 42, 38,
    34, 31, 27, 24, 20,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Am,
    Vhf,
    Band3,
    BandX,
    Band45,
    LBand,
}

impl Band {
    /// Gain band for an RF frequency.
    pub fn from_hz(hz: u64) -> Band {
        match hz {
            0..=59_999_999 => Band::Am,
            60_000_000..=119_999_999 => Band::Vhf,
            120_000_000..=249_999_999 => Band::Band3,
            250_000_000..=419_999_999 => Band::BandX,
            420_000_000..=999_999_999 => Band::Band45,
            _ => Band::LBand,
        }
    }

    fn tables(self) -> (&'static [u8; GAIN_STEPS], &'static [u8; GAIN_STEPS]) {
        match self {
            Band::Am => (&AM_LNA, &AM_IFGR),
            Band::Vhf => (&VHF_LNA, &VHF_IFGR),
            Band::Band3 => (&BAND3_LNA, &BAND3_IFGR),
            Band::BandX => (&BANDX_LNA, &BANDX_IFGR),
            Band::Band45 => (&BAND45_LNA, &BAND45_IFGR),
            Band::LBand => (&LBAND_LNA, &LBAND_IFGR),
        }
    }

    /// Highest valid `LNAstate` in this band (RSP1B).
    pub fn max_lna_state(self) -> u8 {
        // Index 0 (minimum gain) always uses the highest LNA state.
        self.tables().0[0]
    }
}

/// `(LNAstate, gRdB)` to program for a gain step, in a given band.
pub fn settings(band: Band, index: usize) -> (u8, i32) {
    let index = index.min(GAIN_STEPS - 1);
    let (lna, ifgr) = band.tables();
    (lna[index], ifgr[index] as i32)
}

/// Converts an rtl_tcp gain (command 0x04, in tenths of dB, R820T scale
/// 0..=49.6 dB) into a gain step 0..=28. Same quantisation as rsp_tcp.
pub fn index_from_tenths_db(tenths: u32) -> usize {
    let p = ((9 + tenths) / 5).min(100);
    (((GAIN_STEPS - 1) as f32 / 100.0) * p as f32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANDS: [Band; 6] = [
        Band::Am,
        Band::Vhf,
        Band::Band3,
        Band::BandX,
        Band::Band45,
        Band::LBand,
    ];

    #[test]
    fn extremes_of_every_band() {
        for b in BANDS {
            assert_eq!(settings(b, GAIN_STEPS - 1), (0, 20), "{:?}", b);
            let (lna, gr) = settings(b, 0);
            assert_eq!(lna, b.max_lna_state(), "{:?}", b);
            assert_eq!(gr, 59, "{:?}", b);
        }
    }

    #[test]
    fn all_values_valid_for_rsp1b() {
        for b in BANDS {
            // API spec 3.15: number of RSP1B LNA states per band.
            let spec_max = match b {
                Band::Am => 6,
                Band::LBand => 8,
                _ => 9,
            };
            assert!(b.max_lna_state() <= spec_max, "{:?}", b);
            for i in 0..GAIN_STEPS {
                let (lna, gr) = settings(b, i);
                assert!(lna <= b.max_lna_state(), "{:?} {}", b, i);
                assert!((20..=59).contains(&gr), "{:?} {}", b, i);
            }
        }
    }

    #[test]
    fn band_edges() {
        assert_eq!(Band::from_hz(59_999_999), Band::Am);
        assert_eq!(Band::from_hz(60_000_000), Band::Vhf);
        assert_eq!(Band::from_hz(195_936_000), Band::Band3); // DAB 8A
        assert_eq!(Band::from_hz(419_999_999), Band::BandX);
        assert_eq!(Band::from_hz(420_000_000), Band::Band45);
        assert_eq!(Band::from_hz(1_000_000_000), Band::LBand);
    }

    #[test]
    fn rtl_gain_quantisation() {
        assert_eq!(index_from_tenths_db(0), 0);
        assert_eq!(index_from_tenths_db(496), GAIN_STEPS - 1);
        assert_eq!(index_from_tenths_db(9999), GAIN_STEPS - 1);
        assert_eq!(index_from_tenths_db(250), 14);
    }
}
