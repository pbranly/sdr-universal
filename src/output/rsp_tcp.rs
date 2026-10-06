//! SDRplay `rsp_tcp` extended protocol.
//!
//! `rsp_tcp` is SDRplay's own rtl_tcp server. Started in "extended mode" it
//! adds three things on top of rtl_tcp, which clients such as SDroxide use to
//! drive an RSP natively instead of through an emulated RTL dongle:
//!
//! * a 45-byte `RSP0` capability block, sent right after the 12-byte `RTL0`
//!   greeting, which also announces the sample format;
//! * optional signed 16-bit samples, the RSP's native resolution, instead of
//!   8-bit ones;
//! * extra commands (opcodes `0x1F` to `0x26`) for the RSP-specific controls:
//!   LNA state, IF gain reduction, hardware AGC and its set-point, notch
//!   filters, bias-T and antenna.
//!
//! The layout follows SDRplay's `rsp_tcp_api.h` (GPL), and was cross-checked
//! with the parser of the SDroxide client.

use crate::core::capabilities::{IF_GAIN_REDUCTION_RANGE_DB, MAX_LNA_STATE};
use crate::core::{Command, GainMode};

/// Magic of the capability block.
pub const CAPABILITIES_MAGIC: &[u8; 4] = b"RSP0";

/// Version of the capability block.
pub const CAPABILITIES_VERSION: u32 = 1;

/// Length of the capability block on the wire.
pub const CAPABILITIES_LEN: usize = 45;

/// Hardware version reported by the SDRplay API for an RSP1B.
pub const RSP1B_HARDWARE_VERSION: u32 = 6;

// Capability bits (rsp_tcp_api.h).
pub const CAP_BIAS_T: u32 = 1 << 0;
pub const CAP_BROADCAST_NOTCH: u32 = 1 << 3;
pub const CAP_DAB_NOTCH: u32 = 1 << 4;
pub const CAP_AGC: u32 = 1 << 7;

/// What the RSP1B offers: hardware AGC, bias-T, FM broadcast notch, DAB notch.
pub const RSP1B_CAPABILITIES: u32 = CAP_AGC | CAP_BIAS_T | CAP_DAB_NOTCH | CAP_BROADCAST_NOTCH;

// Notch bits of command 0x24.
pub const NOTCH_AM: u32 = 1 << 0;
pub const NOTCH_BROADCAST: u32 = 1 << 1;
pub const NOTCH_DAB: u32 = 1 << 2;
pub const NOTCH_RF: u32 = 1 << 3;

/// Format of the IQ samples on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8-bit, centred on 128 (plain rtl_tcp).
    Uint8,
    /// Signed 16-bit little-endian: the API's samples, unchanged.
    Int16,
}

impl SampleFormat {
    /// Code announced in the capability block.
    pub fn code(self) -> u32 {
        match self {
            SampleFormat::Uint8 => 1,
            SampleFormat::Int16 => 2,
        }
    }
}

/// Builds the capability block for an RSP1B.
///
/// Packed, 45 bytes. Everything is big-endian except `third_antenna_freq_limit`,
/// which `rsp_tcp` sends in host order (it is zero here: the RSP1B has a single
/// antenna input).
pub fn build_capabilities(format: SampleFormat) -> [u8; CAPABILITIES_LEN] {
    let mut block = [0u8; CAPABILITIES_LEN];

    block[0..4].copy_from_slice(CAPABILITIES_MAGIC);
    block[4..8].copy_from_slice(&CAPABILITIES_VERSION.to_be_bytes());
    block[8..12].copy_from_slice(&RSP1B_CAPABILITIES.to_be_bytes());
    // 12..16: reserved.
    block[16..20].copy_from_slice(&RSP1B_HARDWARE_VERSION.to_be_bytes());
    block[20..24].copy_from_slice(&format.code().to_be_bytes());
    block[24] = 1; // antenna inputs
                   // 25..38: third antenna name (none), 38..42: its frequency limit (0).
    block[42] = 1; // tuners
    block[43] = IF_GAIN_REDUCTION_RANGE_DB.0 as u8;
    block[44] = IF_GAIN_REDUCTION_RANGE_DB.1 as u8;

    block
}

/// Converts floating-point samples (±1.0 = the API's full scale) to signed
/// 16-bit little-endian bytes.
pub fn samples_to_i16_le(samples: &[crate::core::iq::IqSample]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);

    for sample in samples {
        for value in [sample.i, sample.q] {
            let v = (value * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    out
}

/// An extended command, decoded from its opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RspCommand {
    /// 0x1F: antenna input (0 = A, 1 = B, 2 = Hi-Z).
    Antenna(u32),
    /// 0x20: LNA state.
    LnaState(u32),
    /// 0x21: IF gain reduction, in dB.
    IfGainReduction(u32),
    /// 0x22: hardware AGC on or off.
    Agc(bool),
    /// 0x23: hardware AGC set-point, in dBFS (signed).
    AgcSetpoint(i32),
    /// 0x24: notch filter bit mask.
    Notch(u32),
    /// 0x25: bias-T.
    BiasT(bool),
    /// 0x26: reference clock output.
    RefOut(bool),
}

impl RspCommand {
    /// Decodes an extended opcode, or `None` if it is not one.
    pub fn parse(opcode: u8, value: u32) -> Option<Self> {
        Some(match opcode {
            0x1F => Self::Antenna(value),
            0x20 => Self::LnaState(value),
            0x21 => Self::IfGainReduction(value),
            0x22 => Self::Agc(value != 0),
            0x23 => Self::AgcSetpoint(value as i32),
            0x24 => Self::Notch(value),
            0x25 => Self::BiasT(value != 0),
            0x26 => Self::RefOut(value != 0),
            _ => return None,
        })
    }

    /// The core commands that carry out this command. Controls the RSP1B does
    /// not have (second antenna, reference output, AM and RF notches) give none.
    pub fn to_core_commands(self) -> Vec<Command> {
        match self {
            Self::Antenna(0) => vec![Command::SetAntenna("A".to_string())],
            Self::Antenna(other) => {
                log::debug!("antenna input {} does not exist on an RSP1B", other);
                Vec::new()
            }

            // Out-of-range values are refused by the core with a message.
            Self::LnaState(value) => vec![Command::SetLnaState(
                u8::try_from(value).unwrap_or(MAX_LNA_STATE.saturating_add(1)),
            )],
            Self::IfGainReduction(value) => vec![Command::SetIfGainReduction(
                i32::try_from(value).unwrap_or(i32::MAX),
            )],

            Self::Agc(enabled) => vec![Command::SetGainMode(if enabled {
                GainMode::Automatic
            } else {
                GainMode::Manual
            })],
            Self::AgcSetpoint(value) => vec![Command::SetAgcSetpoint(value)],

            Self::Notch(mask) => {
                if mask & (NOTCH_AM | NOTCH_RF) != 0 {
                    log::debug!(
                        "AM and RF notch filters do not exist on an RSP1B (mask {mask:#x})"
                    );
                }

                // Both are set every time: the mask says which ones are on.
                vec![
                    Command::SetRfNotch(mask & NOTCH_BROADCAST != 0),
                    Command::SetDabNotch(mask & NOTCH_DAB != 0),
                ]
            }

            Self::BiasT(enabled) => vec![Command::SetBiasTee(enabled)],

            Self::RefOut(_) => {
                log::debug!("the RSP1B has no reference clock output");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::capabilities::AGC_SETPOINT_RANGE_DBFS;
    use crate::core::iq::IqSample;

    /// What a client reads from the block.
    struct ClientView {
        version: u32,
        capabilities: u32,
        hardware_version: u32,
        sample_format: u32,
        antennas: u8,
        tuners: u8,
        ifgr_min: u8,
        ifgr_max: u8,
    }

    /// Re-implementation of the client-side parsing (SDroxide's
    /// `RspCapabilities::parse`), to check our block against the same offsets.
    fn client_parse(b: &[u8]) -> Option<ClientView> {
        if b.len() < CAPABILITIES_LEN || &b[..4] != CAPABILITIES_MAGIC {
            return None;
        }
        let be = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Some(ClientView {
            version: be(4),
            capabilities: be(8),
            hardware_version: be(16),
            sample_format: be(20),
            antennas: b[24],
            tuners: b[42],
            ifgr_min: b[43],
            ifgr_max: b[44],
        })
    }

    #[test]
    fn capability_block_has_the_documented_layout() {
        let block = build_capabilities(SampleFormat::Int16);
        assert_eq!(block.len(), 45);

        let view = client_parse(&block).expect("parsed");

        assert_eq!(view.version, 1);
        assert_eq!(view.capabilities, RSP1B_CAPABILITIES);
        assert_eq!(view.capabilities, 0b1001_1001);
        assert_eq!(view.hardware_version, 6);
        assert_eq!(view.sample_format, 2);
        assert_eq!(view.antennas, 1);
        assert_eq!(view.tuners, 1);
        assert_eq!((view.ifgr_min, view.ifgr_max), (20, 59));

        // Reserved word, third antenna name and its frequency limit stay zero.
        assert!(block[12..16].iter().all(|&b| b == 0));
        assert!(block[25..42].iter().all(|&b| b == 0));
    }

    #[test]
    fn sample_format_is_announced() {
        assert_eq!(
            client_parse(&build_capabilities(SampleFormat::Uint8))
                .unwrap()
                .sample_format,
            1
        );
        assert_eq!(
            client_parse(&build_capabilities(SampleFormat::Int16))
                .unwrap()
                .sample_format,
            2
        );
    }

    #[test]
    fn int16_samples_round_trip_exactly() {
        // Every value of the API's range survives the float conversion.
        let originals: Vec<i16> = vec![0, 1, -1, 1234, -1234, 32767, -32768, 16384, -16384];
        let samples: Vec<IqSample> = originals
            .chunks(2)
            .map(|pair| IqSample {
                i: pair[0] as f32 / 32768.0,
                q: pair.get(1).copied().unwrap_or(0) as f32 / 32768.0,
            })
            .collect();

        let bytes = samples_to_i16_le(&samples);
        assert_eq!(bytes.len(), samples.len() * 4);

        let decoded: Vec<i16> = bytes
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();

        let mut expected = originals.clone();
        expected.push(0); // the last pair has no Q
        assert_eq!(decoded, expected);
    }

    #[test]
    fn int16_conversion_clips_and_is_little_endian() {
        let bytes = samples_to_i16_le(&[IqSample { i: 2.0, q: -2.0 }]);
        assert_eq!(bytes, vec![0xFF, 0x7F, 0x00, 0x80]);

        let bytes = samples_to_i16_le(&[IqSample { i: 0.5, q: 0.0 }]);
        assert_eq!(&bytes[0..2], &[0x00, 0x40]);
    }

    #[test]
    fn opcodes_are_decoded() {
        assert_eq!(RspCommand::parse(0x1F, 2), Some(RspCommand::Antenna(2)));
        assert_eq!(RspCommand::parse(0x20, 5), Some(RspCommand::LnaState(5)));
        assert_eq!(
            RspCommand::parse(0x21, 44),
            Some(RspCommand::IfGainReduction(44))
        );
        assert_eq!(RspCommand::parse(0x22, 1), Some(RspCommand::Agc(true)));
        assert_eq!(RspCommand::parse(0x22, 0), Some(RspCommand::Agc(false)));
        assert_eq!(RspCommand::parse(0x24, 6), Some(RspCommand::Notch(6)));
        assert_eq!(RspCommand::parse(0x25, 1), Some(RspCommand::BiasT(true)));
        assert_eq!(RspCommand::parse(0x26, 0), Some(RspCommand::RefOut(false)));

        // Not extended opcodes.
        for opcode in [0x01u8, 0x0D, 0x1E, 0x27, 0x40, 0xFF] {
            assert_eq!(RspCommand::parse(opcode, 0), None, "{opcode:#x}");
        }
    }

    #[test]
    fn agc_setpoint_is_signed() {
        let wire = (-40i32) as u32; // two's complement, as sent by the client
        assert_eq!(
            RspCommand::parse(0x23, wire),
            Some(RspCommand::AgcSetpoint(-40))
        );
        assert_eq!(AGC_SETPOINT_RANGE_DBFS, (-72, -20));
    }

    #[test]
    fn commands_map_to_the_core() {
        assert!(matches!(
            RspCommand::LnaState(5).to_core_commands()[..],
            [Command::SetLnaState(5)]
        ));
        assert!(matches!(
            RspCommand::IfGainReduction(44).to_core_commands()[..],
            [Command::SetIfGainReduction(44)]
        ));
        assert!(matches!(
            RspCommand::Agc(true).to_core_commands()[..],
            [Command::SetGainMode(GainMode::Automatic)]
        ));
        assert!(matches!(
            RspCommand::Agc(false).to_core_commands()[..],
            [Command::SetGainMode(GainMode::Manual)]
        ));
        assert!(matches!(
            RspCommand::AgcSetpoint(-40).to_core_commands()[..],
            [Command::SetAgcSetpoint(-40)]
        ));
        assert!(matches!(
            RspCommand::BiasT(true).to_core_commands()[..],
            [Command::SetBiasTee(true)]
        ));
        assert!(matches!(
            RspCommand::Antenna(0).to_core_commands()[..],
            [Command::SetAntenna(_)]
        ));
    }

    #[test]
    fn controls_the_rsp1b_does_not_have_give_nothing() {
        assert!(RspCommand::Antenna(1).to_core_commands().is_empty());
        assert!(RspCommand::Antenna(2).to_core_commands().is_empty());
        assert!(RspCommand::RefOut(true).to_core_commands().is_empty());
    }

    #[test]
    fn notch_mask_selects_the_filters() {
        let flags = |mask: u32| match RspCommand::Notch(mask).to_core_commands()[..] {
            [Command::SetRfNotch(fm), Command::SetDabNotch(dab)] => (fm, dab),
            _ => panic!("two commands expected"),
        };

        assert_eq!(flags(0), (false, false));
        assert_eq!(flags(NOTCH_BROADCAST), (true, false));
        assert_eq!(flags(NOTCH_DAB), (false, true));
        assert_eq!(flags(NOTCH_BROADCAST | NOTCH_DAB), (true, true));
        // AM and RF notches do not exist on an RSP1B: ignored.
        assert_eq!(flags(NOTCH_AM | NOTCH_RF), (false, false));
    }

    #[test]
    fn out_of_range_values_reach_the_core_to_be_refused() {
        // 300 does not fit a u8: it must still be refused, not wrapped to 44.
        assert!(matches!(
            RspCommand::LnaState(300).to_core_commands()[..],
            [Command::SetLnaState(n)] if n > MAX_LNA_STATE
        ));
        assert!(matches!(
            RspCommand::IfGainReduction(u32::MAX).to_core_commands()[..],
            [Command::SetIfGainReduction(g)] if g > IF_GAIN_REDUCTION_RANGE_DB.1
        ));
    }
}
