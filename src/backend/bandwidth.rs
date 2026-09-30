//! Bande passante (filtre IF) demandée par les clients rtl_tcp.
//!
//! AbracaDABra (version récente) envoie la commande 0x40 « SET_BANDWIDTH »
//! avec une valeur en Hz (1 530 000 par défaut, ou la valeur choisie par
//! l'utilisateur). Le RSP1B n'accepte que des largeurs discrètes.

/// Largeurs de filtre IF du RSP1B utilisables par la passerelle, en Hz,
/// dans l'ordre croissant.
///
/// Les largeurs de 5 à 8 MHz existent sur le RSP1B, mais elles supposent une
/// fréquence d'échantillonnage plus élevée que les 2,048 MHz fixés par cette
/// passerelle : on plafonne donc à 1,536 MHz, la largeur d'un ensemble DAB.
pub const SUPPORTED_HZ: [u32; 4] = [200_000, 300_000, 600_000, 1_536_000];

/// Largeur par défaut (0 ou valeur absente).
pub const DEFAULT_HZ: u32 = 1_536_000;

/// Plus petite largeur supportée supérieure ou égale à la demande, pour ne
/// jamais couper une partie du signal demandé ; au-delà du maximum, le
/// maximum. Une demande de 0 signifie « largeur par défaut ».
pub fn snap_hz(requested: u32) -> u32 {
    if requested == 0 {
        return DEFAULT_HZ;
    }

    SUPPORTED_HZ
        .iter()
        .copied()
        .find(|&bw| bw >= requested)
        .unwrap_or(DEFAULT_HZ)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abracadabra_default_maps_to_dab_width() {
        assert_eq!(snap_hz(1_530_000), 1_536_000);
        assert_eq!(snap_hz(1_536_000), 1_536_000);
    }

    #[test]
    fn rounds_up_to_next_supported_width() {
        assert_eq!(snap_hz(100_000), 200_000);
        assert_eq!(snap_hz(200_000), 200_000);
        assert_eq!(snap_hz(250_000), 300_000);
        assert_eq!(snap_hz(500_000), 600_000);
        assert_eq!(snap_hz(1_000_000), 1_536_000);
    }

    #[test]
    fn caps_and_default() {
        assert_eq!(snap_hz(0), DEFAULT_HZ);
        assert_eq!(snap_hz(8_000_000), 1_536_000);
        assert_eq!(snap_hz(u32::MAX), 1_536_000);
    }

    #[test]
    fn every_result_is_supported() {
        for hz in (0..3_000_000).step_by(10_000) {
            assert!(SUPPORTED_HZ.contains(&snap_hz(hz)), "{}", hz);
        }
    }
}
