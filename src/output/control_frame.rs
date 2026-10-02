//! Frame of the rtl_tcp "control port" (rtl_tcp port + 1), as read by
//! AbracaDABra to estimate the RF level in dBm.
//!
//! Format (old-dab's rtl_tcp server, everything big-endian):
//!   bytes 0-1 : total frame length, including these 2 bytes
//!   0x00, length 0x0002, gain in tenths of dB (i16)   "gain indication"
//!   0x86, length 0x0001, overload (0 or 1)            "overload indication"
//! AbracaDABra only uses a frame longer than 10 bytes: both indications are
//! therefore required (11 bytes in total).
//!
//! Estimate on the client side:
//!   RF level (dBm) = 20·log10(level in bytes) - gain - 46 + user offset
//! The level in bytes is 512 × the floating-point sample (the gateway's
//! factor); so that the estimate comes out as "dBFS - total RSP gain", the
//! total gain is announced increased by `client_scale_db()`. The absolute,
//! receiver-specific constant still has to be calibrated once with
//! AbracaDABra's "RF level offset" setting.

/// Length of a complete frame.
pub const FRAME_LEN: usize = 11;

/// Float -> byte factor applied by the gateway (see rtltcp.rs).
const BYTE_SCALE: f64 = 512.0;

/// The "-46" constant of AbracaDABra's formula.
const CLIENT_CONSTANT_DB: f64 = 46.0;

/// Correction added to the announced gain: 20·log10(512) - 46 ≈ 8.19 dB.
pub fn client_scale_db() -> f64 {
    20.0 * BYTE_SCALE.log10() - CLIENT_CONSTANT_DB
}

/// Total RSP gain (dB) -> value announced to the client, in tenths of dB.
pub fn client_gain_tenths(total_gain_db: f64) -> i16 {
    ((total_gain_db + client_scale_db()) * 10.0)
        .round()
        .clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

/// Builds a control frame.
pub fn build_frame(gain_tenths: i16, overload: bool) -> [u8; FRAME_LEN] {
    let gain = gain_tenths.to_be_bytes();

    [
        0x00,
        FRAME_LEN as u8,
        0x00, // gain indication
        0x00,
        0x02,
        gain[0],
        gain[1],
        0x86, // overload indication
        0x00,
        0x01,
        overload as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Re-implementation of the parsing done by AbracaDABra
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

        let gain = abracadabra_read(&frame).expect("frame rejected by the client");
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
        // Signal: floating-point sample with a peak of 0.05, total gain 46.5 dB.
        let sample = 0.05_f64;
        let total_gain = 46.5_f64;

        let level_bytes = BYTE_SCALE * sample;
        let announced = client_gain_tenths(total_gain) as f64 / 10.0;
        let client_estimate = 20.0 * level_bytes.log10() - announced - CLIENT_CONSTANT_DB;

        let expected = 20.0 * sample.log10() - total_gain;
        assert!(
            (client_estimate - expected).abs() < 0.06,
            "{} vs {}",
            client_estimate,
            expected
        );
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
