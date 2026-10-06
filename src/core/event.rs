use super::{GainMode, IfType, LoMode, ReceiverMode};

/// Events produced by the core for the backend (and, later, other outputs).
/// Some variants are not produced yet.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Event {
    StateChanged,

    FrequencyChanged(u64),
    ModeChanged(ReceiverMode),
    BandwidthChanged(u32),
    IfTypeChanged(IfType),
    LoModeChanged(LoMode),
    SampleRateChanged(u32),

    GainChanged(f32),
    GainIndexChanged(usize),
    GainModeChanged(GainMode),
    AgcChanged(bool),
    LnaStateChanged(u8),
    IfGainReductionChanged(i32),
    AgcSetpointChanged(i32),

    AntennaChanged(String),
    PpmChanged(f32),

    BiasTeeChanged(bool),
    RfNotchChanged(bool),
    DabNotchChanged(bool),

    IqStarted,
    IqStopped,

    DeviceConnected,
    DeviceDisconnected,

    Overflow,
    Error(String),
}
