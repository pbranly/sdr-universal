use super::{GainMode, IfType, LoMode, ReceiverMode};

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
    GainModeChanged(GainMode),
    AgcChanged(bool),

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