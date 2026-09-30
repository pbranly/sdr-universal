//! Backend factice : simule un RSP1B sans matériel (option `--mock`).
//!
//! Il sert à tester la passerelle de bout en bout (protocole rtl_tcp, port de
//! contrôle, gain par bande, AGC, surcharge, reconnexions, arrêt propre) sur
//! n'importe quelle machine, sans bibliothèque SDRplay ni RSP branché.
//!
//! Modèle radio, volontairement simple mais cohérent avec le reste de la
//! passerelle :
//!   * le signal d'antenne est un bruit gaussien (comme un multiplex DAB) dont
//!     la puissance est fixée par `level_dbm` (-75 dBm par défaut) ;
//!   * gain total = GAIN_BASE_DB - atténuation LNA - gRdB (valeurs relevées
//!     sur un RSP1B ; les atténuations LNA par bande sont approchées par la
//!     table 60-420 MHz : suffisant pour tester la logique de la passerelle,
//!     mais pas représentatif de la RF réelle, surtout en Am et 420-1000 MHz) ;
//!   * niveau numérique (dBFS, échelle ±1.0 comme l'API SDRplay) =
//!     level_dbm + gain total. La constante propre au RSP reste donc nulle, ce
//!     que suppose aussi le calcul du niveau RF de `output::control_frame` ;
//!   * l'AGC ramène le niveau vers -30 dBFS en agissant sur gRdB ;
//!   * au-delà de la pleine échelle, le signal est écrêté et une surcharge est
//!     signalée.

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

/// Gain total pour LNAstate 0 et gRdB 0 (relevé : 85,58 dB à gRdB 20).
const GAIN_BASE_DB: f64 = 105.6;

/// Atténuation LNA par état (approximation, table 60-420 MHz).
const LNA_GR_DB: [f64; 10] = [0.0, 6.0, 12.0, 18.0, 20.0, 26.0, 32.0, 38.0, 57.0, 62.0];

/// Consigne de l'AGC en dBFS (comme rsp_tcp).
const AGC_TARGET_DBFS: f64 = -30.0;

/// Limites de gRdB du RSP1B.
const GR_MIN: i32 = 20;
const GR_MAX: i32 = 59;

/// Niveau par composante (I ou Q) au-dessus duquel on signale une surcharge.
const OVERLOAD_RMS: f64 = 0.30;

/// Largeurs de filtre acceptées par l'API SDRplay (Hz).
const SUPPORTED_BANDWIDTHS_HZ: [u32; 8] = [
    200_000, 300_000, 600_000, 1_536_000, 5_000_000, 6_000_000, 7_000_000, 8_000_000,
];

const BLOCK_SAMPLES: usize = 4096;
const NOISE_LEN: usize = 1 << 16;

/// Gain total (dB) pour un état LNA et un gRdB.
pub fn total_gain_db(lna_state: u8, gr_db: i32) -> f64 {
    let lna = LNA_GR_DB[(lna_state as usize).min(LNA_GR_DB.len() - 1)];
    GAIN_BASE_DB - lna - gr_db as f64
}

/// Niveau numérique en dBFS pour un niveau d'antenne et un gain total.
pub fn dbfs(level_dbm: f64, total_gain_db: f64) -> f64 {
    level_dbm + total_gain_db
}

/// Un pas d'AGC : ±1 dB de gRdB pour rapprocher le niveau de la consigne.
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

        println!(
            ">>> GAIN : bande={:?} pas={}/{} -> LNA={} gRdB={} | réel LNA={} gRdB={} curr={:.2} dB (AGC={})",
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

/// Générateur de bruit gaussien complexe (variance unité par composante).
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

        // Cadence temps réel : un bloc toutes les 4096 / fs secondes.
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
        println!(">>> Backend FACTICE connecté (aucun matériel, niveau d'antenne simulé)");

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
                    println!(
                        ">>> Changement de bande {:?} -> {:?} : gain réappliqué (pas {})",
                        old_band, new_band, index
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
                    println!("AGC RSP1B activé");
                }
                GainMode::Manual => {
                    let (was_agc, index) = {
                        let mut s = self.shared.lock().unwrap();
                        let was = s.agc_on;
                        s.agc_on = false;
                        (was, s.gain_index)
                    };

                    if was_agc {
                        println!("AGC RSP1B désactivé");
                        self.set_gain_index(index)?;
                    }
                }
            },

            Event::AgcChanged(enabled) => {
                println!(">>> BACKEND AGC RTL (0x08) ignoré : enabled={}", enabled);
            }

            Event::BandwidthChanged(hz) => {
                if !SUPPORTED_BANDWIDTHS_HZ.contains(hz) {
                    return Err(anyhow::anyhow!("Bande passante non supportée : {} Hz", hz));
                }

                self.shared.lock().unwrap().bandwidth_hz = *hz;
                println!("Bande passante RSP1B réglée à {} Hz", hz);
            }

            Event::SampleRateChanged(hz) => {
                self.shared.lock().unwrap().sample_rate = *hz;
                println!("Sample rate RSP1B réglé à {} Hz", hz);
            }

            Event::BiasTeeChanged(enabled) => {
                let mut s = self.shared.lock().unwrap();
                if s.bias_t != *enabled {
                    s.bias_t = *enabled;
                    println!(">>> Bias-T : {}", if *enabled { "activé (alimentation antenne)" } else { "désactivé" });
                }
            }

            Event::RfNotchChanged(enabled) => {
                let mut s = self.shared.lock().unwrap();
                if s.rf_notch != *enabled {
                    s.rf_notch = *enabled;
                    println!(">>> RF notch (FM) : {}", if *enabled { "activé" } else { "désactivé" });
                }
            }

            Event::DabNotchChanged(enabled) => {
                let mut s = self.shared.lock().unwrap();
                if s.dab_notch != *enabled {
                    if *enabled && s.band == Band::Band3 {
                        println!(
                            ">>> ATTENTION : le notch DAB atténue la bande III (174-240 MHz) : \
                             la réception DAB sera dégradée"
                        );
                    }
                    s.dab_notch = *enabled;
                    println!(">>> DAB notch : {}", if *enabled { "activé" } else { "désactivé" });
                }
            }

            Event::PpmChanged(ppm) => {
                let mut s = self.shared.lock().unwrap();
                let ppm = *ppm as f64;
                if (s.ppm - ppm).abs() > 1e-9 {
                    s.ppm = ppm;
                    println!(">>> Correction fréquence : {:.3} ppm", ppm);
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

    /// Même message de surcharge que le vrai backend (au plus toutes les 2 s).
    fn service(&mut self) {
        let mut s = self.shared.lock().unwrap();

        if s.overload_events > 0 && self.last_overload_log.elapsed() >= Duration::from_secs(2) {
            println!(
                "!!! SURCHARGE ADC ({} fois) bande={:?} pas de gain={} : réduire le gain \
                 (LNAstate plus élevé) ou activer l'AGC",
                s.overload_events, s.band, s.gain_index
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
        // Relevés : LNA 0 / gRdB 20 -> 85,58 dB ; LNA 0 / gRdB 38 -> 67,58 dB.
        assert!((total_gain_db(0, 20) - 85.6).abs() < 0.1);
        assert!((total_gain_db(0, 38) - 67.6).abs() < 0.1);
        // LNA 5 (26 dB) / gRdB 44 -> environ 35,6 dB.
        assert!((total_gain_db(5, 44) - 35.6).abs() < 0.1);
    }

    #[test]
    fn total_gain_never_decreases_with_the_gain_step() {
        // Bandes où l'approximation « table LNA 60-420 MHz » du modèle est
        // représentative. Pas Am ni 420-1000 MHz : leurs atténuations LNA
        // réelles diffèrent, et le modèle n'y est pas monotone.
        for band in [Band::Vhf, Band::Band3, Band::BandX, Band::LBand] {
            let mut previous = f64::MIN;
            for index in 0..gain::GAIN_STEPS {
                let (lna, gr) = gain::settings(band, index);
                let g = total_gain_db(lna, gr);
                assert!(g >= previous - 0.01, "{:?} pas {} : {} < {}", band, index, g, previous);
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
        assert!((level - AGC_TARGET_DBFS).abs() <= 1.0, "niveau final {}", level);
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
        let power: f64 = t.iter().map(|(i, q)| (*i as f64).powi(2) + (*q as f64).powi(2)).sum::<f64>() / (2.0 * n);
        assert!((power - 1.0).abs() < 0.03, "variance {}", power);
    }
}
