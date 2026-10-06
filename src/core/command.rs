use super::{IfType, LoMode, ReceiverMode};

/// The core's command surface. Not every command is used by the current
/// outputs yet; they are the common vocabulary for future ones (hamlib...).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Command {
    GetState,
    GetCapabilities,

    SetFrequency(u64),
    SetMode(ReceiverMode),
    SetBandwidth(u32),
    SetIfType(IfType),
    SetLoMode(LoMode),
    SetSampleRate(u32),

    SetGain(f32),
    /// Gain step 0..=28 (rtl_tcp command 0x0D): LNAstate + gRdB of the current band.
    SetGainIndex(usize),
    SetGainMode(super::GainMode),
    SetAgc(bool),
    /// Direct LNA state (rsp_tcp extended command 0x20).
    SetLnaState(u8),
    /// Direct IF gain reduction in dB (rsp_tcp extended command 0x21).
    SetIfGainReduction(i32),
    /// Hardware AGC set-point in dBFS (rsp_tcp extended command 0x23).
    SetAgcSetpoint(i32),

    SetAntenna(String),
    SetPpm(f32),

    SetBiasTee(bool),
    SetRfNotch(bool),
    SetDabNotch(bool),

    StartIq,
    StopIq,
}
