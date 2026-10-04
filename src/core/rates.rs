//! Sample rates and analog bandwidths of the RSP.
//!
//! Sample rates
//! ------------
//! The RSP's ADC runs between 2 and 10 MS/s. For a lower output rate the API's
//! decimator divides the ADC rate by a power of two (2 to 32), so the ADC is
//! kept at the nearest power-of-two multiple that is at least 2 MS/s. This is
//! what SDRplay's own `rsp_tcp` does, and it gives properly filtered samples,
//! unlike interpolating in software.
//!
//! Bandwidths
//! ----------
//! The analog IF filter has a few discrete widths. By default the widest one
//! that fits inside the output rate is used (a wider filter would alias, a
//! narrower one would roll the band edges off for no reason). A client can ask
//! for another width (rtl_tcp command 0x40, sent by recent AbracaDABra
//! versions); it is rounded up to a supported width, never above the widest
//! one that fits the current rate.

/// Lowest ADC rate of the RSP.
pub const MIN_ADC_HZ: u32 = 2_000_000;

/// Highest output rate offered (the API accepts up to 10.66 MS/s).
pub const MAX_OUTPUT_HZ: u32 = 10_000_000;

/// Highest decimation factor of the API.
pub const MAX_DECIMATION: u32 = 32;

/// Lowest output rate: the ADC floor divided by the highest decimation.
pub const MIN_OUTPUT_HZ: u32 = MIN_ADC_HZ / MAX_DECIMATION;

/// Analog IF filter widths of the RSP, in Hz, in increasing order.
pub const SUPPORTED_BANDWIDTHS_HZ: [u32; 8] = [
    200_000, 300_000, 600_000, 1_536_000, 5_000_000, 6_000_000, 7_000_000, 8_000_000,
];

/// How to obtain an output rate: ADC rate and decimation factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatePlan {
    /// Rate of the samples delivered to the gateway.
    pub output_hz: u32,
    /// Rate the ADC must be programmed to.
    pub adc_hz: u32,
    /// Decimation factor applied by the API (1 = none).
    pub decimation: u32,
}

/// Plans an output rate, or `None` if the RSP cannot deliver it.
pub fn plan(output_hz: u32) -> Option<RatePlan> {
    if !(MIN_OUTPUT_HZ..=MAX_OUTPUT_HZ).contains(&output_hz) {
        return None;
    }

    let mut decimation = 1u32;

    while u64::from(output_hz) * u64::from(decimation) < u64::from(MIN_ADC_HZ) {
        decimation *= 2;
    }

    Some(RatePlan {
        output_hz,
        adc_hz: output_hz * decimation,
        decimation,
    })
}

/// Widest analog filter that fits inside an output rate (at least 200 kHz).
pub fn default_bandwidth_hz(output_hz: u32) -> u32 {
    SUPPORTED_BANDWIDTHS_HZ
        .iter()
        .copied()
        .filter(|&bandwidth| bandwidth <= output_hz)
        .max()
        .unwrap_or(SUPPORTED_BANDWIDTHS_HZ[0])
}

/// Smallest supported width greater than or equal to a request, so that no
/// part of the requested signal is cut, but never above the widest width that
/// fits `output_hz`. A request of 0 means "default width for this rate".
pub fn snap_bandwidth_hz(requested_hz: u32, output_hz: u32) -> u32 {
    let widest = default_bandwidth_hz(output_hz);

    if requested_hz == 0 {
        return widest;
    }

    SUPPORTED_BANDWIDTHS_HZ
        .iter()
        .copied()
        .find(|&bandwidth| bandwidth >= requested_hz)
        .unwrap_or(widest)
        .min(widest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_at_or_above_the_adc_floor_are_not_decimated() {
        for rate in [
            2_000_000, 2_048_000, 2_400_000, 3_200_000, 6_000_000, 10_000_000,
        ] {
            assert_eq!(
                plan(rate),
                Some(RatePlan {
                    output_hz: rate,
                    adc_hz: rate,
                    decimation: 1
                }),
                "{}",
                rate
            );
        }
    }

    #[test]
    fn lower_rates_use_the_smallest_power_of_two_decimation() {
        // (output, ADC rate, decimation)
        let cases = [
            (1_024_000, 2_048_000, 2),
            (1_400_000, 2_800_000, 2),
            (1_800_000, 3_600_000, 2),
            (1_920_000, 3_840_000, 2),
            (1_000_000, 2_000_000, 2),
            (250_000, 2_000_000, 8),
            (225_000, 3_600_000, 16),
            (62_500, 2_000_000, 32),
        ];

        for (output, adc, decimation) in cases {
            let p = plan(output).unwrap();
            assert_eq!(p.output_hz, output);
            assert_eq!(p.decimation, decimation, "{}", output);
            assert_eq!(p.adc_hz, adc, "{}", output);
        }
    }

    #[test]
    fn adc_rate_always_stays_in_range() {
        let mut rate = MIN_OUTPUT_HZ;

        while rate <= MAX_OUTPUT_HZ {
            let p = plan(rate).unwrap();
            assert!(p.adc_hz >= MIN_ADC_HZ && p.adc_hz <= 10_660_000, "{}", rate);
            assert!(p.decimation <= MAX_DECIMATION, "{}", rate);
            assert_eq!(p.adc_hz % p.decimation, 0);
            rate += 12_345;
        }
    }

    #[test]
    fn unsupported_rates_are_refused() {
        assert_eq!(plan(0), None);
        assert_eq!(plan(MIN_OUTPUT_HZ - 1), None);
        assert_eq!(plan(MAX_OUTPUT_HZ + 1), None);
        assert_eq!(plan(u32::MAX), None);
        assert!(plan(MIN_OUTPUT_HZ).is_some());
        assert!(plan(MAX_OUTPUT_HZ).is_some());
    }

    #[test]
    fn default_bandwidth_is_the_widest_that_fits() {
        assert_eq!(default_bandwidth_hz(2_048_000), 1_536_000); // DAB
        assert_eq!(default_bandwidth_hz(2_400_000), 1_536_000);
        assert_eq!(default_bandwidth_hz(1_536_000), 1_536_000);
        assert_eq!(default_bandwidth_hz(1_024_000), 600_000);
        assert_eq!(default_bandwidth_hz(600_000), 600_000);
        assert_eq!(default_bandwidth_hz(500_000), 300_000);
        assert_eq!(default_bandwidth_hz(250_000), 200_000);
        assert_eq!(default_bandwidth_hz(62_500), 200_000);
        assert_eq!(default_bandwidth_hz(5_000_000), 5_000_000);
        assert_eq!(default_bandwidth_hz(7_500_000), 7_000_000);
        assert_eq!(default_bandwidth_hz(10_000_000), 8_000_000);
    }

    #[test]
    fn requested_bandwidth_is_rounded_up_but_capped_by_the_rate() {
        // AbracaDABra's default request at the DAB rate.
        assert_eq!(snap_bandwidth_hz(1_530_000, 2_048_000), 1_536_000);
        assert_eq!(snap_bandwidth_hz(1_536_000, 2_048_000), 1_536_000);
        assert_eq!(snap_bandwidth_hz(100_000, 2_048_000), 200_000);
        assert_eq!(snap_bandwidth_hz(250_000, 2_048_000), 300_000);
        assert_eq!(snap_bandwidth_hz(500_000, 2_048_000), 600_000);
        // Never wider than the rate allows.
        assert_eq!(snap_bandwidth_hz(8_000_000, 2_048_000), 1_536_000);
        assert_eq!(snap_bandwidth_hz(1_536_000, 1_024_000), 600_000);
        // Wide rates allow wide filters.
        assert_eq!(snap_bandwidth_hz(5_500_000, 10_000_000), 6_000_000);
        assert_eq!(snap_bandwidth_hz(9_000_000, 10_000_000), 8_000_000);
        // 0 = default for the rate.
        assert_eq!(snap_bandwidth_hz(0, 2_048_000), 1_536_000);
        assert_eq!(snap_bandwidth_hz(0, 250_000), 200_000);
    }

    #[test]
    fn every_result_is_a_supported_width() {
        for requested in (0..10_000_000).step_by(37_000) {
            for output in [62_500, 250_000, 1_024_000, 2_048_000, 6_000_000, 10_000_000] {
                assert!(
                    SUPPORTED_BANDWIDTHS_HZ.contains(&snap_bandwidth_hz(requested, output)),
                    "{} {}",
                    requested,
                    output
                );
            }
        }
    }
}
