#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverMode {
    Am,
    Nfm,
    Wfm,
    Usb,
    Lsb,
    Cw,
    Cwr,
    Raw,
}

#[derive(Debug, Clone)]
pub struct ReceiverState {
    pub frequency_hz: u64,
    pub sample_rate: u32,
    pub bandwidth_hz: u32,

    pub mode: ReceiverMode,

    pub gain: f32,
    pub gain_mode: GainMode,

    pub agc: bool,

    pub antenna: Option<String>,

    pub ppm: f32,

    pub bias_tee: bool,
    pub rf_notch: bool,
    pub dab_notch: bool,

    pub streaming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GainMode {
    Manual,
    Automatic,
}

impl Default for ReceiverState {
    fn default() -> Self {
        Self {
            frequency_hz: 200_000_000,
            sample_rate: 2_000_000,
            bandwidth_hz: 200_000,

            mode: ReceiverMode::Nfm,

            gain: 50.0,
            gain_mode: GainMode::Manual,

            agc: true,

            antenna: Some("A".to_string()),

            ppm: 0.0,

            bias_tee: false,
            rf_notch: false,
            dab_notch: false,

            streaming: false,
        }
    }
}
