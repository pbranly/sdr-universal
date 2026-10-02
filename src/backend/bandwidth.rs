//! Bandwidth (IF filter) requested by rtl_tcp clients.
//!
//! Recent AbracaDABra versions send command 0x40 "SET_BANDWIDTH" with a value
//! in Hz (1,530,000 by default, or the value chosen by the user). The RSP1B
//! only accepts discrete widths.

/// RSP1B IF filter widths usable by the gateway, in Hz, in increasing order.
///
/// Widths of 5 to 8 MHz exist on the RSP1B, but they assume a higher sample
/// rate than the 2.048 MHz this gateway uses: the list is therefore capped at
/// 1.536 MHz, the width of a DAB ensemble.
pub const SUPPORTED_HZ: [u32; 4] = [200_000, 300_000, 600_000, 1_536_000];

/// Default width (0 or no value).
pub const DEFAULT_HZ: u32 = 1_536_000;

/// Smallest supported width greater than or equal to the request, so that no
/// part of the requested signal is cut; above the maximum, the maximum. A
/// request of 0 means "default width".
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
