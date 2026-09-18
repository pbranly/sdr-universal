#[derive(Debug, Clone)]
pub enum Event {
    StateChanged,
    FrequencyChanged(u64),
    ModeChanged,
    SampleRateChanged(u32),
    BandwidthChanged(u32),
    GainChanged(f32),
    GainModeChanged,
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
