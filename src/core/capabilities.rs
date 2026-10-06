use super::{rates, ReceiverMode};

/// Range of the IF gain reduction (gRdB), in dB: higher means less gain.
pub const IF_GAIN_REDUCTION_RANGE_DB: (i32, i32) = (20, 59);

/// Highest LNA state of any band (the backend also checks the band's own limit).
pub const MAX_LNA_STATE: u8 = 9;

/// Range of the hardware AGC set-point, in dBFS.
pub const AGC_SETPOINT_RANGE_DBFS: (i32, i32) = (-72, -20);

/// AGC set-point used until a client chooses another (same as SDRplay's rsp_tcp).
pub const DEFAULT_AGC_SETPOINT_DBFS: i32 = -30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfType {
    Zero,
    KHz450,
    KHz1620,
    KHz2048,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoMode {
    Auto,
    MHz120,
    MHz144,
    MHz168,
}

#[derive(Debug, Clone)]
pub struct Capabilities {
    pub frequency_min_hz: u64,
    pub frequency_max_hz: u64,

    /// Output sample-rate range (hardware decimation covers the low end).
    pub sample_rate_min_hz: u32,
    pub sample_rate_max_hz: u32,
    pub modes: Vec<ReceiverMode>,
    pub bandwidths_hz: Vec<u32>,

    pub if_types: Vec<IfType>,
    pub lo_modes: Vec<LoMode>,

    pub gain_min_db: f32,
    pub gain_max_db: f32,
    /// Number of gain steps exposed (0..gain_steps).
    pub gain_steps: usize,

    pub antennas: Vec<String>,

    pub bias_tee: bool,
    pub rf_notch: bool,
    pub dab_notch: bool,

    pub iq: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            frequency_min_hz: 1,
            frequency_max_hz: 2_000_000_000,

            sample_rate_min_hz: rates::MIN_OUTPUT_HZ,
            sample_rate_max_hz: rates::MAX_OUTPUT_HZ,

            modes: vec![
                ReceiverMode::Am,
                ReceiverMode::Nfm,
                ReceiverMode::Wfm,
                ReceiverMode::Usb,
                ReceiverMode::Lsb,
                ReceiverMode::Cw,
                ReceiverMode::Cwr,
                ReceiverMode::Raw,
            ],

            bandwidths_hz: rates::SUPPORTED_BANDWIDTHS_HZ.to_vec(),

            if_types: vec![
                IfType::Zero,
                IfType::KHz450,
                IfType::KHz1620,
                IfType::KHz2048,
            ],

            lo_modes: vec![LoMode::Auto, LoMode::MHz120, LoMode::MHz144, LoMode::MHz168],

            gain_min_db: 0.0,
            gain_max_db: 59.0,
            gain_steps: 29,

            antennas: vec!["A".to_string()],

            bias_tee: true,
            rf_notch: true,
            dab_notch: true,

            iq: true,
        }
    }
}
