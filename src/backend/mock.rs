//! Mock backend: simulates an RSP1B without hardware (`--mock` option).
//!
//! It is used to test the gateway end to end (rtl_tcp protocol, control port,
//! per-band gain, AGC, overload, reconnections, clean shutdown) on any machine,
//! with no SDRplay library and no RSP plugged in.
//!
//! Radio model, deliberately simple but consistent with the rest of the
//! gateway:
//!   * the antenna signal is Gaussian noise (like a DAB multiplex) whose power
//!     is set by `level_dbm` (-75 dBm by default);
//!   * total gain = GAIN_BASE_DB - LNA attenuation - gRdB (values observed on
//!     an RSP1B; LNA attenuations per band are approximated by the 60-420 MHz
//!     table: enough to test the gateway's logic, but not representative of
//!     the real RF, especially in Am and 420-1000 MHz);
//!   * digital level (dBFS, ±1.0 scale like the SDRplay API) =
//!     level_dbm + total gain. The receiver-specific constant is therefore
//!     zero, which is also what the RF level computation of
//!     `output::control_frame` assumes;
//!   * the AGC drives the level towards -30 dBFS by changing gRdB;
//!   * above full scale the signal is clipped and an overload is reported.

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::backend::bandwidth;
use crate::backend::gain::{self, Band};
use crate::backend::Backend;
use crate::core::{telemetry, Event, GainMode, IqBlock, IqSample};

/// Total gain for LNAstate 0 and gRdB 0 (observed: 85.58 dB at gRdB 20).
const GAIN_BASE_DB: f64 = 105.6;

/// LNA attenuation per state (approximation, 60-420 MHz table).
const LNA_GR_DB: [f64; 10] = [0.0, 6.0, 12.0, 18.0, 20.0, 26.0, 32.0, 38.0, 57.0, 62.0];

/// AGC set-point in dBFS (like rsp_tcp).
const AGC_TARGET_DBFS: f64 = -30.0;

/// gRdB limits of the RSP1B.
const GR_MIN: i32 = 20;
const GR_MAX: i32 = 59;

/// Per-component (I or Q) level above which an overload is reported.
const OVERLOAD_RMS: f64 = 0.30;

/// Filter widths accepted by the SDRplay API (Hz).
const SUPPORTED_BANDWIDTHS_HZ: [u32; 8] = [
    200_000, 300_000, 600_000, 1_536_000, 5_000_000, 6_000_000, 7_000_000, 8_000_000,
];

const BLOCK_SAMPLES: usize = 4096;
const NOISE_LEN: usize = 1 << 16;

/// Total gain (dB) for an LNA state and a gRdB.
pub fn total_gain_db(lna_state: u8, gr_db: i32) -> f64 {
    let lna = LNA_GR_DB[(lna_state as usize).min(LNA_GR_DB.len() - 1)];
    GAIN_BASE_DB - lna - gr_db as f64
}

/// Digital level in dBFS for an antenna level and a total gain.
pub fn dbfs(level_dbm: f64, total_gain_db: f64) -> f64 {
    level_dbm + total_gain_db
}

/// One AGC step: ±1 dB of gRdB to bring the level closer to the set-point.
pub fn agc_step(gr_db: i32, level_dbfs: f64) -> i32 {
    if level_dbfs > AGC_TARGET_DBFS + 1.0 {
        (gr_db + 1).min(GR_MAX)
    } else if level_dbfs < AGC_TARGET_DBFS - 1.0 {
        (gr_db - 1).max(GR_MIN)
    } else {
        gr_db
    }
}

struct Shared {
    frequency_hz: u64,
    band: Band,
    sample_rate: u32,
    bandwidth_hz: u32,
    lna_state: u8,
    gr_db: i32,
    gain_index: usize,
    agc_on: bool,
    level_dbm: f64,
    overload: bool,
    overload_events: u64,
    bias_t: bool,
    rf_notch: bool,
    dab_notch: bool,
    ppm: f64,
}

pub struct MockBackend {
    shared: Arc<Mutex<Shared>>,
    tx: Option<SyncSender<IqBlock>>,
    rx: Option<Receiver<IqBlock>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    connected: bool,
    last_overload_log: Instant,
}

impl MockBackend {
    pub fn new(level_dbm: f64) -> Self {
        let (tx, rx) = sync_channel(1024);
        let band = Band::from_hz(100_000_000);
        let (lna_state, gr_db) = gain::settings(band, gain::DEFAULT_GAIN_INDEX);

        Self {
            shared: Arc::new(Mutex::new(Shared {
                frequency_hz: 100_000_000,
                band,
                sample_rate: 2_000_000,
                bandwidth_hz: 200_000,
                lna_state,
                gr_db,
                gain_index: gain::DEFAULT_GAIN_INDEX,
                agc_on: false,
                level_dbm,
                overload: false,
                overload_events: 0,
                bias_t: false,
                rf_notch: false,
                dab_notch: false,
                ppm: 0.0,
            })),
            tx: Some(tx),
            rx: Some(rx),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            connected: false,
            last_overload_log: Instant::now(),
        }
    }

    fn set_gain_index(&mut self, index: usize) -> Result<()> {
        let mut s = self.shared.lock().unwrap();
        let index = index.min(gain::GAIN_STEPS - 1);
        let (lna_state, gr_db) = gain::settings(s.band, index);

        s.lna_state = lna_state;
        s.gr_db = gr_db;
        s.gain_index = index;

        let total = total_gain_db(lna_state, gr_db);
        telemetry::set_total_gain_db(total);

        log::info!(
            "GAIN: band={:?} step={}/{} -> LNA={} gRdB={} | actual LNA={} gRdB={} curr={:.2} dB (AGC={})",
            s.band,
            index,
            gain::GAIN_STEPS - 1,
            lna_state,
            gr_db,
            lna_state,
            gr_db,
            total,
            s.agc_on
        );

        Ok(())
    }

    fn start_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }

        let Some(tx) = self.tx.clone() else { return };
        let shared = Arc::clone(&self.shared);
        let stop = Arc::clone(&self.stop);
        stop.store(false, Ordering::SeqCst);

        self.worker = Some(thread::spawn(move || generate(shared, tx, stop)));
    }

    fn stop_worker(&mut self) {
        self.stop.store(true, Ordering::SeqCst);

        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

/// Complex Gaussian noise generator (unit variance per component).
fn noise_table() -> Vec<(f32, f32)> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let uniform = |x: u64| ((x >> 11) as f64 + 0.5) / (1u64 << 53) as f64;

    (0..NOISE_LEN)
        .map(|_| {
            let u1 = uniform(next());
            let u2 = uniform(next());
            let r = (-2.0 * u1.ln()).sqrt();
            let t = 2.0 * std::f64::consts::PI * u2;
            ((r * t.cos()) as f32, (r * t.sin()) as f32)
        })
        .collect()
}

fn generate(shared: Arc<Mutex<Shared>>, tx: SyncSender<IqBlock>, stop: Arc<AtomicBool>) {
    let noise = noise_table();
    let mut sequence: u64 = 0;
    let mut offset: usize = 0;
    let mut next_deadline = Instant::now();
    const FULL_SCALE: f32 = 32767.0 / 32768.0;

    while !stop.load(Ordering::SeqCst) {
        let (frequency_hz, sample_rate, rms) = {
            let mut s = shared.lock().unwrap();

            let mut total = total_gain_db(s.lna_state, s.gr_db);
            let mut level = dbfs(s.level_dbm, total);

            if s.agc_on {
                let new_gr = agc_step(s.gr_db, level);

                if new_gr != s.gr_db {
                    s.gr_db = new_gr;
                    total = total_gain_db(s.lna_state, s.gr_db);
                    level = dbfs(s.level_dbm, total);
                    telemetry::set_total_gain_db(total);
                }
            }

            let rms = 10f64.powf(level / 20.0);
            let overload = rms > OVERLOAD_RMS;

            if overload != s.overload {
                s.overload = overload;
                telemetry::set_overload(overload);

                if overload {
                    s.overload_events += 1;
                }
            }

            (s.frequency_hz, s.sample_rate, rms)
        };

        let a = rms as f32;
        let mut samples = Vec::with_capacity(BLOCK_SAMPLES);

        for i in 0..BLOCK_SAMPLES {
            let (ni, nq) = noise[(offset + i) & (NOISE_LEN - 1)];
            samples.push(IqSample {
                i: (ni * a).clamp(-1.0, FULL_SCALE),
                q: (nq * a).clamp(-1.0, FULL_SCALE),
            });
        }

        offset = (offset + 12_345) & (NOISE_LEN - 1);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        let block = IqBlock::new(sequence, timestamp, frequency_hz, sample_rate, samples);
        sequence += 1;

        match tx.try_send(block) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => break,
        }

        // Real-time pacing: one block every 4096 / fs seconds.
        next_deadline += Duration::from_secs_f64(BLOCK_SAMPLES as f64 / sample_rate.max(1) as f64);
        let now = Instant::now();

        if next_deadline > now {
            thread::sleep(next_deadline - now);
        } else if now - next_deadline > Duration::from_millis(200) {
            next_deadline = now;
        }
    }
}

impl Backend for MockBackend {
    fn connect(&mut self) -> Result<()> {
        self.connected = true;
        log::info!("mock backend connected (no hardware, simulated antenna level)");

        let s = self.shared.lock().unwrap();
        telemetry::set_total_gain_db(total_gain_db(s.lna_state, s.gr_db));

        Ok(())
    }

    fn disconnect(&mut self) {
        self.stop_worker();
        self.connected = false;
    }

    fn apply_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::FrequencyChanged(frequency_hz) => {
                let (old_band, new_band, index) = {
                    let mut s = self.shared.lock().unwrap();
                    let old = s.band;
                    let new = Band::from_hz(*frequency_hz);

                    s.frequency_hz = *frequency_hz;
                    s.band = new;

                    if s.lna_state > new.max_lna_state() {
                        s.lna_state = new.max_lna_state();
                    }

                    (old, new, s.gain_index)
                };

                if new_band != old_band {
                    log::info!(
                        "Band change {:?} -> {:?}: gain re-applied (step {})",
                        old_band,
                        new_band,
                        index
                    );
                    self.set_gain_index(index)?;
                }
            }

            Event::GainChanged(gain_db) => {
                let tenths = (gain_db.max(0.0) * 10.0).round() as u32;
                self.set_gain_index(gain::index_from_tenths_db(tenths))?;
            }

            Event::GainIndexChanged(index) => {
                self.set_gain_index(*index)?;
            }

            Event::GainModeChanged(mode) => match mode {
                GainMode::Automatic => {
                    self.shared.lock().unwrap().agc_on = true;
                    log::info!("RSP1B AGC enabled");
                }
                GainMode::Manual => {
                    let (was_agc, index) = {
                        let mut s = self.shared.lock().unwrap();
                        let was = s.agc_on;
                        s.agc_on = false;
                        (was, s.gain_index)
                    };

                    if was_agc {
                        log::info!("RSP1B AGC disabled");
                        self.set_gain_index(index)?;
                    }
                }
            },

            Event::AgcChanged(enabled) => {
                log::debug!("RTL digital AGC (0x08) ignored: enabled={}", enabled);
            }

            Event::BandwidthChanged(hz) => {
                if !SUPPORTED_BANDWIDTHS_HZ.contains(hz) {
                    return Err(anyhow::anyhow!("Unsupported bandwidth: {} Hz", hz));
                }

                self.shared.lock().unwrap().bandwidth_hz = *hz;
                log::info!("RSP1B bandwidth set to {} Hz", hz);
            }

            Event::SampleRateChanged(hz) => {
                self.shared.lock().unwrap().sample_rate = *hz;
                log::info!("RSP1B sample rate set to {} Hz", hz);
            }

            Event::BiasTeeChanged(enabled) => {
                let mut s = self.shared.lock().unwrap();
                if s.bias_t != *enabled {
                    s.bias_t = *enabled;
                    log::info!(
                        "Bias-T: {}",
                        if *enabled {
                            "enabled (antenna power)"
                        } else {
                            "disabled"
                        }
                    );
                }
            }

            Event::RfNotchChanged(enabled) => {
                let mut s = self.shared.lock().unwrap();
                if s.rf_notch != *enabled {
                    s.rf_notch = *enabled;
                    log::info!(
                        "RF notch (FM): {}",
                        if *enabled { "enabled" } else { "disabled" }
                    );
                }
            }

            Event::DabNotchChanged(enabled) => {
                let mut s = self.shared.lock().unwrap();
                if s.dab_notch != *enabled {
                    if *enabled && s.band == Band::Band3 {
                        log::warn!(
                            "DAB notch enabled in band III (174-240 MHz): \
                             DAB reception will be degraded"
                        );
                    }
                    s.dab_notch = *enabled;
                    log::info!(
                        "DAB notch: {}",
                        if *enabled { "enabled" } else { "disabled" }
                    );
                }
            }

            Event::PpmChanged(ppm) => {
                let mut s = self.shared.lock().unwrap();
                let ppm = *ppm as f64;
                if (s.ppm - ppm).abs() > 1e-9 {
                    s.ppm = ppm;
                    log::info!("Frequency correction: {:.3} ppm", ppm);
                }
            }

            Event::IqStarted => self.start_worker(),
            Event::IqStopped => self.stop_worker(),

            _ => {}
        }

        Ok(())
    }

    fn take_iq_receiver(&mut self) -> Option<Receiver<IqBlock>> {
        self.rx.take()
    }

    /// Same overload message as the real backend (at most every 2 s).
    fn service(&mut self) {
        let mut s = self.shared.lock().unwrap();

        if s.overload_events > 0 && self.last_overload_log.elapsed() >= Duration::from_secs(2) {
            log::warn!(
                "ADC OVERLOAD ({} times) band={:?} gain step={}: lower the gain \
                 (higher LNA state) or enable the AGC",
                s.overload_events,
                s.band,
                s.gain_index
            );
            s.overload_events = 0;
            self.last_overload_log = Instant::now();
        }
    }
}

impl Drop for MockBackend {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

#[allow(dead_code)]
fn _uses(_: u32) -> u32 {
    bandwidth::DEFAULT_HZ
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_gain_matches_observed_rsp1b_values() {
        // Observed: LNA 0 / gRdB 20 -> 85.58 dB; LNA 0 / gRdB 38 -> 67.58 dB.
        assert!((total_gain_db(0, 20) - 85.6).abs() < 0.1);
        assert!((total_gain_db(0, 38) - 67.6).abs() < 0.1);
        // LNA 5 (26 dB) / gRdB 44 -> about 35.6 dB.
        assert!((total_gain_db(5, 44) - 35.6).abs() < 0.1);
    }

    #[test]
    fn total_gain_never_decreases_with_the_gain_step() {
        // Bands where the model's "60-420 MHz LNA table" approximation is
        // representative. Not Am nor 420-1000 MHz: their real LNA attenuations
        // differ, and the model is not monotonic there.
        for band in [Band::Vhf, Band::Band3, Band::BandX, Band::LBand] {
            let mut previous = f64::MIN;
            for index in 0..gain::GAIN_STEPS {
                let (lna, gr) = gain::settings(band, index);
                let g = total_gain_db(lna, gr);
                assert!(
                    g >= previous - 0.01,
                    "{:?} step {}: {} < {}",
                    band,
                    index,
                    g,
                    previous
                );
                previous = g;
            }
        }
    }

    #[test]
    fn agc_converges_to_target() {
        let level_dbm = -75.0;
        let (lna, mut gr) = gain::settings(Band::Band3, 14);

        for _ in 0..100 {
            let level = dbfs(level_dbm, total_gain_db(lna, gr));
            gr = agc_step(gr, level);
        }

        let level = dbfs(level_dbm, total_gain_db(lna, gr));
        assert!(
            (level - AGC_TARGET_DBFS).abs() <= 1.0,
            "niveau final {}",
            level
        );
        assert!((GR_MIN..=GR_MAX).contains(&gr));
    }

    #[test]
    fn agc_respects_limits() {
        assert_eq!(agc_step(GR_MAX, 10.0), GR_MAX);
        assert_eq!(agc_step(GR_MIN, -90.0), GR_MIN);
    }

    #[test]
    fn noise_table_is_unit_variance() {
        let t = noise_table();
        let n = t.len() as f64;
        let power: f64 = t
            .iter()
            .map(|(i, q)| (*i as f64).powi(2) + (*q as f64).powi(2))
            .sum::<f64>()
            / (2.0 * n);
        assert!((power - 1.0).abs() < 0.03, "variance {}", power);
    }
}
