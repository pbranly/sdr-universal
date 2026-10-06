use super::rsp_tcp::{self, RspCommand};
use super::server::{Demand, IqFrame};
use crate::core::iq::IqSink;
use crate::core::{Command, GainMode, IqBlock};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum RtltcpCommand {
    SetFrequency(u32),
    SetSampleRate(u32),
    SetGainMode(u32),
    SetGain(u32),
    SetGainIndex(u32),
    SetAgc(bool),
    /// 0x05: frequency correction in ppm (signed integer).
    SetPpm(i32),
    /// 0x0E: bias-T.
    SetBiasTee(bool),
    /// 0x40: bandwidth in Hz (extension used by AbracaDABra).
    SetBandwidth(u32),
    /// An rsp_tcp extended command (opcodes 0x1F-0x26), only decoded on the
    /// rsp_tcp server.
    Rsp(RspCommand),
    Unknown(u8, u32),
}

impl RtltcpCommand {
    /// Decodes a command. The rsp_tcp extended opcodes are only recognised when
    /// `rsp_extended` is set (the rsp_tcp server): on plain rtl_tcp they are
    /// unknown commands, as in the original servers.
    pub fn parse(command: u8, value: u32, rsp_extended: bool) -> Self {
        if rsp_extended {
            if let Some(rsp) = RspCommand::parse(command, value) {
                return Self::Rsp(rsp);
            }
        }

        match command {
            0x01 => Self::SetFrequency(value),
            0x02 => Self::SetSampleRate(value),
            0x03 => Self::SetGainMode(value),
            0x04 => Self::SetGain(value),
            0x0D => Self::SetGainIndex(value),
            0x08 => Self::SetAgc(value != 0),
            0x05 => Self::SetPpm(value as i32),
            0x0E => Self::SetBiasTee(value != 0),
            0x40 => Self::SetBandwidth(value),
            _ => Self::Unknown(command, value),
        }
    }

    /// Converts to core commands (usually one; some rsp_tcp commands give two
    /// or none). `sample_rate_hz` is the receiver's current output rate: it
    /// limits the analog bandwidth that can be requested.
    pub fn to_core_commands(self, sample_rate_hz: u32) -> Vec<Command> {
        match self {
            Self::SetFrequency(value) => vec![Command::SetFrequency(value as u64)],

            Self::SetSampleRate(value) => vec![Command::SetSampleRate(value)],

            Self::SetGainMode(value) => {
                let mode = if value == 0 {
                    GainMode::Automatic
                } else {
                    GainMode::Manual
                };

                vec![Command::SetGainMode(mode)]
            }

            Self::SetGain(value) => vec![Command::SetGain(value as f32 / 10.0)],

            Self::SetGainIndex(value) => vec![Command::SetGainIndex(value as usize)],

            Self::SetAgc(value) => vec![Command::SetAgc(value)],

            Self::SetPpm(value) => vec![Command::SetPpm(value as f32)],

            Self::SetBiasTee(value) => vec![Command::SetBiasTee(value)],

            // The RSP only has a few filter widths: take the nearest one above
            // the request (AbracaDABra's 1.53 MHz -> 1.536 MHz), never wider
            // than the current rate allows.
            Self::SetBandwidth(value) => vec![Command::SetBandwidth(
                crate::core::rates::snap_bandwidth_hz(value, sample_rate_hz),
            )],

            Self::Rsp(command) => command.to_core_commands(),

            Self::Unknown(opcode, value) => {
                log::debug!(
                    "ignoring unsupported rtl_tcp command 0x{:02X} (value {})",
                    opcode,
                    value
                );
                Vec::new()
            }
        }
    }
}

pub struct RtltcpSink {
    samples_processed: u64,
    blocks_processed: u64,
    iq_tx: Sender<IqFrame>,
    demand: Arc<Demand>,
    requested_sample_rate: Arc<AtomicU32>,

    // State kept between two IQ blocks to keep the resampling
    // continuous.
    resample_buffer: Vec<crate::core::iq::IqSample>,
    resample_position: f64,
}

impl RtltcpSink {
    pub fn new(
        iq_tx: Sender<IqFrame>,
        demand: Arc<Demand>,
        requested_sample_rate: Arc<AtomicU32>,
    ) -> Self {
        Self {
            samples_processed: 0,
            blocks_processed: 0,
            iq_tx,
            demand,
            requested_sample_rate,
            resample_buffer: Vec::new(),
            resample_position: 0.0,
        }
    }

    /// Converts a floating-point sample to an rtl_tcp byte (unsigned,
    /// centred on 128 like rsp_tcp and rtl_tcp clients).
    fn convert_sample(value: f32, gain: f32) -> u8 {
        (value * gain + 128.0).round().clamp(0.0, 255.0) as u8
    }

    fn resample_block(&mut self, block: &IqBlock) -> Vec<crate::core::iq::IqSample> {
        let input_rate = block.sample_rate as f64;
        let output_rate = self.requested_sample_rate.load(Ordering::Relaxed) as f64;

        if input_rate <= 0.0 || output_rate <= 0.0 {
            return Vec::new();
        }

        self.resample_buffer.extend_from_slice(&block.samples);

        let step = input_rate / output_rate;
        let mut output = Vec::new();

        while self.resample_position + 1.0 < self.resample_buffer.len() as f64 {
            let index = self.resample_position.floor() as usize;
            let fraction = (self.resample_position - index as f64) as f32;

            let a = &self.resample_buffer[index];
            let b = &self.resample_buffer[index + 1];

            output.push(crate::core::iq::IqSample {
                i: a.i + (b.i - a.i) * fraction,
                q: a.q + (b.q - a.q) * fraction,
            });

            self.resample_position += step;
        }

        let consumed = self.resample_position.floor() as usize;

        if consumed > 0 {
            self.resample_buffer.drain(0..consumed);
            self.resample_position -= consumed as f64;
        }

        output
    }

    /// Float -> byte factor, identical to rsp_tcp (SDRplay): `(xi << 2) >> 8`
    /// on i16 values is xi/64, i.e. 32768/64 = 512 for a value normalised to
    /// ±1.0. The factor is FIXED: the level sent to the client must follow the
    /// RSP gain, otherwise the client's software AGC (AbracaDABra) never sees
    /// the effect of its gain commands. Values that exceed the range are
    /// clipped to 0..=255.
    const NOMINAL_GAIN: f32 = 512.0;

    pub fn convert_block(&mut self, block: &IqBlock) -> Vec<u8> {
        static DEBUG_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

        let debug_count = DEBUG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let gain = Self::NOMINAL_GAIN;

        let mut output = Vec::with_capacity(block.samples.len() * 2);

        for sample in &block.samples {
            output.push(Self::convert_sample(sample.i, gain));
            output.push(Self::convert_sample(sample.q, gain));
        }

        if debug_count < 3 && output.len() >= 8 {
            log::debug!(
                "RTL-TCP INPUT: I/Q=({:.4}, {:.4}) factor={:.1}",
                block.samples[0].i,
                block.samples[0].q,
                gain
            );

            log::debug!(
                "RTL-TCP OUTPUT: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                output[0],
                output[1],
                output[2],
                output[3],
                output[4],
                output[5],
                output[6],
                output[7],
            );
        }

        output
    }
}

impl IqSink for RtltcpSink {
    fn push(&mut self, block: &IqBlock) {
        let resampled_samples = self.resample_block(block);

        let resampled_block = IqBlock {
            sequence: block.sequence,
            timestamp: block.timestamp,
            center_frequency_hz: block.center_frequency_hz,
            sample_rate: self.requested_sample_rate.load(Ordering::Relaxed),
            samples: resampled_samples,
        };

        let debug_due = self.blocks_processed % 100 == 99 && log::log_enabled!(log::Level::Debug);

        // Convert only the formats that have clients (plus the 8-bit view when
        // the periodic debug statistics are due).
        let u8_data = (self.demand.u8.load(Ordering::Relaxed) || debug_due)
            .then(|| self.convert_block(&resampled_block));

        let i16_data = self
            .demand
            .i16
            .load(Ordering::Relaxed)
            .then(|| rsp_tcp::samples_to_i16_le(&resampled_block.samples));

        let rtl_iq: &[u8] = u8_data.as_deref().unwrap_or(&[]);

        self.blocks_processed += 1;
        self.samples_processed += resampled_block.samples.len() as u64;
        if self.blocks_processed % 500 == 0 {
            log::trace!(
                "RTL-TCP PUSH #{}: freq={} Hz samples={}",
                self.blocks_processed,
                block.center_frequency_hz,
                block.samples.len()
            );
        }
        if self.blocks_processed % 100 == 0 && log::log_enabled!(log::Level::Debug) {
            let mut i_min = f32::INFINITY;
            let mut i_max = f32::NEG_INFINITY;
            let mut q_min = f32::INFINITY;
            let mut q_max = f32::NEG_INFINITY;

            let mut i_sum = 0.0f64;
            let mut q_sum = 0.0f64;
            let mut i_sq_sum = 0.0f64;
            let mut q_sq_sum = 0.0f64;

            for sample in &block.samples {
                i_min = i_min.min(sample.i);
                i_max = i_max.max(sample.i);
                q_min = q_min.min(sample.q);
                q_max = q_max.max(sample.q);

                i_sum += sample.i as f64;
                q_sum += sample.q as f64;

                i_sq_sum += (sample.i as f64) * (sample.i as f64);
                q_sq_sum += (sample.q as f64) * (sample.q as f64);
            }

            let count = block.samples.len() as f64;

            let i_mean = i_sum / count;
            let q_mean = q_sum / count;

            let i_rms = (i_sq_sum / count).sqrt();
            let q_rms = (q_sq_sum / count).sqrt();

            let non7f = rtl_iq.iter().filter(|&&v| v != 0x7f).count();

            let resampled_i_rms = if resampled_block.samples.is_empty() {
                0.0
            } else {
                (resampled_block
                    .samples
                    .iter()
                    .map(|s| (s.i as f64) * (s.i as f64))
                    .sum::<f64>()
                    / resampled_block.samples.len() as f64)
                    .sqrt()
            };

            let resampled_q_rms = if resampled_block.samples.is_empty() {
                0.0
            } else {
                (resampled_block
                    .samples
                    .iter()
                    .map(|s| (s.q as f64) * (s.q as f64))
                    .sum::<f64>()
                    / resampled_block.samples.len() as f64)
                    .sqrt()
            };

            log::debug!(
                "RTL IQ #{} | seq={} | freq={} Hz | input={} Hz | output={} Hz | samples={} | bytes={} | RMSout=({:.4},{:.4})",
                self.blocks_processed,
                block.sequence,
                block.center_frequency_hz,
                block.sample_rate,
                resampled_block.sample_rate,
                resampled_block.samples.len(),
                rtl_iq.len(),
                resampled_i_rms,
                resampled_q_rms
            );

            log::debug!(
                "    I: min={:.4} max={:.4} mean={:.4} rms={:.4}",
                i_min,
                i_max,
                i_mean,
                i_rms
            );

            log::debug!(
                "    Q: min={:.4} max={:.4} mean={:.4} rms={:.4}",
                q_min,
                q_max,
                q_mean,
                q_rms
            );

            log::debug!(
                "    RTL: min={} max={} non7f={}/{}",
                rtl_iq.iter().copied().min().unwrap_or(0),
                rtl_iq.iter().copied().max().unwrap_or(0),
                non7f,
                rtl_iq.len()
            );

            log::debug!(
                "    bytes: {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
                rtl_iq.first().copied().unwrap_or(0),
                rtl_iq.get(1).copied().unwrap_or(0),
                rtl_iq.get(2).copied().unwrap_or(0),
                rtl_iq.get(3).copied().unwrap_or(0),
                rtl_iq.get(4).copied().unwrap_or(0),
                rtl_iq.get(5).copied().unwrap_or(0),
                rtl_iq.get(6).copied().unwrap_or(0),
                rtl_iq.get(7).copied().unwrap_or(0),
            );
        }

        let frame = IqFrame {
            // Only keep the 8-bit data if a client wants it.
            u8_data: if self.demand.u8.load(Ordering::Relaxed) {
                u8_data.map(Arc::new)
            } else {
                None
            },
            i16_data: i16_data.map(Arc::new),
        };

        if self.iq_tx.send(frame).is_err() {
            // Warn once: the same failure would otherwise repeat for every block.
            static WARNED: AtomicBool = AtomicBool::new(false);

            if !WARNED.swap(true, Ordering::Relaxed) {
                log::warn!("RTL-TCP: IQ receiver disconnected");
            }
        }
    }
}
