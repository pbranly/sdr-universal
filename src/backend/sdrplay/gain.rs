//! Gestion du gain du RSP1B, bande par bande.
//!
//! Pourquoi ce module existe
//! -------------------------
//! Sur un RSP, le gain n'est pas un simple nombre de dB : il se règle avec DEUX
//! paramètres, `LNAstate` et `gRdB` (gain reduction de l'étage IF, 20..=59 dB).
//! La signification d'un `LNAstate` DÉPEND DE LA BANDE (spécification API v3.15,
//! chapitre 5, « Gain Reduction Tables ») :
//!   * 0-50 MHz (RSP1B)   : 7 états (0..=6)
//!   * 60-420 MHz         : 10 états (0..=9)
//!   * 420-1000 MHz       : 10 états (0..=9), atténuations différentes
//!   * 1000-2000 MHz      : 9 états (0..=8)
//! Un état qui n'existe pas dans la bande est refusé par le service (OutOfRange),
//! et un état valide n'atténue pas la même quantité d'une bande à l'autre.
//!
//! Une table « dB -> (LNA, gRdB) » mesurée à une seule fréquence n'est donc valable
//! que dans sa bande. Ici : 29 pas de gain (comme la liste R820T attendue par les
//! clients rtl_tcp), avec une table PAR BANDE.
//!
//! Origine des valeurs : tables RSP1B de SDRplay `RSPTCPServer` (rsp_tcp.c,
//! licence GPL v2+), bornes de bandes identiques. Index 0 = gain minimal,
//! index 28 = gain maximal (LNAstate 0, gRdB 20).

/// Nombre de pas de gain exposés aux clients rtl_tcp.
pub const GAIN_STEPS: usize = 29;

/// Pas de gain au démarrage : milieu de l'échelle.
pub const DEFAULT_GAIN_INDEX: usize = 14;

// < 60 MHz
const AM_LNA: [u8; GAIN_STEPS] = [ 6,  6,  6,  6,  6,  6,  5,  5,  5,  5,  5,  4,  4,  3,  3,  3,  3,  3,  2,  2,  1,  0,  0,  0,  0,  0,  0,  0,  0];
const AM_IFGR: [u8; GAIN_STEPS] = [59, 55, 52, 48, 45, 41, 57, 53, 49, 46, 42, 44, 40, 56, 52, 48, 45, 41, 44, 40, 43, 45, 41, 38, 34, 31, 27, 24, 20];

// 60-120 MHz
const VHF_LNA: [u8; GAIN_STEPS] = [ 9,  9,  9,  9,  9,  9,  8,  7,  7,  7,  7,  7,  6,  6,  5,  5,  4,  3,  2,  2,  1,  0,  0,  0,  0,  0,  0,  0,  0];
const VHF_IFGR: [u8; GAIN_STEPS] = [59, 55, 52, 48, 45, 41, 42, 58, 54, 51, 47, 43, 46, 42, 44, 41, 43, 42, 44, 40, 43, 45, 42, 38, 34, 31, 27, 24, 20];

// 120-250 MHz
const BAND3_LNA: [u8; GAIN_STEPS] = [ 9,  9,  9,  9,  9,  9,  8,  7,  7,  7,  7,  7,  6,  6,  5,  5,  4,  3,  2,  2,  1,  0,  0,  0,  0,  0,  0,  0,  0];
const BAND3_IFGR: [u8; GAIN_STEPS] = [59, 55, 52, 48, 45, 41, 42, 58, 54, 51, 47, 43, 46, 42, 44, 41, 43, 42, 44, 40, 43, 45, 42, 38, 34, 31, 27, 24, 20];

// 250-420 MHz
const BANDX_LNA: [u8; GAIN_STEPS] = [ 9,  9,  9,  9,  9,  9,  8,  7,  7,  7,  7,  7,  6,  6,  5,  5,  4,  3,  2,  2,  1,  0,  0,  0,  0,  0,  0,  0,  0];
const BANDX_IFGR: [u8; GAIN_STEPS] = [59, 55, 52, 48, 45, 41, 42, 58, 54, 51, 47, 43, 46, 42, 44, 41, 43, 42, 44, 40, 43, 45, 42, 38, 34, 31, 27, 24, 20];

// 420-1000 MHz
const BAND45_LNA: [u8; GAIN_STEPS] = [ 9,  9,  9,  9,  9,  9,  8,  8,  8,  8,  8,  7,  6,  6,  5,  5,  4,  4,  2,  2,  1,  0,  0,  0,  0,  0,  0,  0,  0];
const BAND45_IFGR: [u8; GAIN_STEPS] = [59, 55, 52, 48, 44, 41, 56, 52, 49, 45, 41, 44, 46, 42, 45, 41, 44, 40, 44, 40, 42, 46, 42, 38, 35, 31, 27, 24, 20];

// 1000-2000 MHz
const LBAND_LNA: [u8; GAIN_STEPS] = [ 8,  8,  8,  8,  8,  8,  7,  7,  7,  7,  7,  6,  5,  5,  4,  4,  3,  2,  2,  2,  1,  0,  0,  0,  0,  0,  0,  0,  0];
const LBAND_IFGR: [u8; GAIN_STEPS] = [59, 55, 52, 48, 45, 41, 56, 53, 49, 46, 42, 43, 46, 42, 44, 41, 43, 48, 44, 40, 43, 45, 42, 38, 34, 31, 27, 24, 20];

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
    /// Bande de gain correspondant à une fréquence RF.
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

    /// Plus grand `LNAstate` valide dans cette bande (RSP1B).
    pub fn max_lna_state(self) -> u8 {
        // L'index 0 (gain minimal) utilise toujours l'état LNA le plus élevé.
        self.tables().0[0]
    }
}

/// `(LNAstate, gRdB)` à programmer pour un pas de gain, dans une bande donnée.
pub fn settings(band: Band, index: usize) -> (u8, i32) {
    let index = index.min(GAIN_STEPS - 1);
    let (lna, ifgr) = band.tables();
    (lna[index], ifgr[index] as i32)
}

/// Convertit un gain rtl_tcp (commande 0x04, en dixièmes de dB, échelle R820T
/// 0..=49.6 dB) en pas de gain 0..=28. Même quantification que rsp_tcp.
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
            // Spec API 3.15 : nombre d'états LNA du RSP1B par bande.
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
