//! Trame du « port de contrôle » rtl_tcp (port rtl_tcp + 1), telle que la
//! lit AbracaDABra pour estimer le niveau RF en dBm.
//!
//! Format (serveur rtl_tcp d'old-dab, tout en gros-boutiste) :
//!   octets 0-1 : longueur totale de la trame, ces 2 octets compris
//!   0x00, longueur 0x0002, gain en dixièmes de dB (i16)   « indication de gain »
//!   0x86, longueur 0x0001, surcharge (0 ou 1)             « indication de surcharge »
//! AbracaDABra n'exploite une trame que si elle fait plus de 10 octets : les
//! deux indications sont donc nécessaires (11 octets au total).
//!
//! Estimation côté client :
//!   niveau RF (dBm) = 20·log10(niveau octets) - gain - 46 + décalage utilisateur
//! Le niveau en octets vaut 512 × l'échantillon flottant (facteur de la
//! passerelle) ; pour que l'estimation revienne à « dBFS - gain total du RSP »,
//! on annonce le gain total augmenté de `CLIENT_SCALE_DB`. La constante
//! absolue propre au RSP reste à étalonner une fois avec le réglage
//! « décalage du niveau RF » d'AbracaDABra.

/// Longueur d'une trame complète.
pub const FRAME_LEN: usize = 11;

/// Facteur flottant -> octets appliqué par la passerelle (voir rtltcp.rs).
const BYTE_SCALE: f64 = 512.0;

/// Constante « -46 » de la formule d'AbracaDABra.
const CLIENT_CONSTANT_DB: f64 = 46.0;

/// Correction à ajouter au gain annoncé : 20·log10(512) - 46 ≈ 8,19 dB.
pub fn client_scale_db() -> f64 {
    20.0 * BYTE_SCALE.log10() - CLIENT_CONSTANT_DB
}

/// Gain total du RSP (dB) -> valeur annoncée au client, en dixièmes de dB.
pub fn client_gain_tenths(total_gain_db: f64) -> i16 {
    ((total_gain_db + client_scale_db()) * 10.0)
        .round()
        .clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

/// Construit une trame de contrôle.
pub fn build_frame(gain_tenths: i16, overload: bool) -> [u8; FRAME_LEN] {
    let gain = gain_tenths.to_be_bytes();

    [
        0x00,
        FRAME_LEN as u8,
        0x00, // indication de gain
        0x00,
        0x02,
        gain[0],
        gain[1],
        0x86, // indication de surcharge
        0x00,
        0x01,
        overload as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduction de la lecture faite par AbracaDABra
    /// (RtlTcpInput::readControlSocketData).
    fn abracadabra_read(data: &[u8]) -> Option<f32> {
        if data.len() > 10 && data[2] == 0 && data[3] == 0 && data[4] == 2 {
            let val = (((data[5] as u16) << 8) | data[6] as u16) as i16;
            Some(val as f32 * 0.1)
        } else {
            None
        }
    }

    #[test]
    fn client_accepts_frame() {
        let frame = build_frame(345, false);
        assert_eq!(frame.len(), FRAME_LEN);
        assert_eq!(((frame[0] as usize) << 8) | frame[1] as usize, FRAME_LEN);

        let gain = abracadabra_read(&frame).expect("trame refusée par le client");
        assert!((gain - 34.5).abs() < 1e-3);
    }

    #[test]
    fn negative_gain_survives_round_trip() {
        let frame = build_frame(-18, true);
        let gain = abracadabra_read(&frame).unwrap();
        assert!((gain + 1.8).abs() < 1e-3);
        assert_eq!(frame[10], 1);
    }

    #[test]
    fn client_estimate_equals_dbfs_minus_total_gain() {
        // Signal : échantillon flottant de crête 0.05, gain total 46.5 dB.
        let sample = 0.05_f64;
        let total_gain = 46.5_f64;

        let level_bytes = BYTE_SCALE * sample;
        let announced = client_gain_tenths(total_gain) as f64 / 10.0;
        let client_estimate = 20.0 * level_bytes.log10() - announced - CLIENT_CONSTANT_DB;

        let expected = 20.0 * sample.log10() - total_gain;
        assert!((client_estimate - expected).abs() < 0.06, "{} vs {}", client_estimate, expected);
    }

    #[test]
    fn scale_constant() {
        assert!((client_scale_db() - 8.19).abs() < 0.01);
    }

    #[test]
    fn extreme_gains_are_clamped() {
        assert_eq!(client_gain_tenths(1e9), i16::MAX);
        assert_eq!(client_gain_tenths(-1e9), i16::MIN);
    }
}
