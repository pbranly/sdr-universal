
use super::{IfType, LoMode, ReceiverMode};

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
    SetGainMode(super::GainMode),
    SetAgc(bool),

    SetAntenna(String),
    SetPpm(f32),

    SetBiasTee(bool),
    SetRfNotch(bool),
    SetDabNotch(bool),

    StartIq,
    StopIq,
}


