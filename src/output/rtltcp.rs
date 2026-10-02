use crate::core::iq::IqSink;
use crate::core::{Command, GainMode, IqBlock};

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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
    Unknown(u8, u32),
}

impl RtltcpCommand {
    pub fn parse(command: u8, value: u32) -> Self {
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

    pub fn to_core_command(self) -> Option<Command> {
        match self {
            Self::SetFrequency(value) => Some(Command::SetFrequency(value as u64)),

            Self::SetSampleRate(value) => Some(Command::SetSampleRate(value)),

            Self::SetGainMode(value) => {
                let mode = if value == 0 {
                    GainMode::Automatic
                } else {
                    GainMode::Manual
                };

                Some(Command::SetGainMode(mode))
            }

            Self::SetGain(value) => Some(Command::SetGain(value as f32 / 10.0)),

            Self::SetGainIndex(value) => Some(Command::SetGainIndex(value as usize)),

            Self::SetAgc(value) => Some(Command::SetAgc(value)),

            Self::SetPpm(value) => Some(Command::SetPpm(value as f32)),

            Self::SetBiasTee(value) => Some(Command::SetBiasTee(value)),

            Self::SetBandwidth(value) => {
                // The RSP only has a few filter widths: take the nearest one
                // above the request (AbracaDABra's 1.53 MHz -> 1.536 MHz).
                Some(Command::SetBandwidth(crate::backend::bandwidth::snap_hz(
                    value,
                )))
            }

            Self::Unknown(_, _) => None,
        }
    }
}

pub struct RtltcpSink {
    samples_processed: u64,
    blocks_processed: u64,
    iq_tx: Sender<Vec<u8>>,
    requested_sample_rate: Arc<AtomicU32>,

    // State kept between two IQ blocks to keep the resampling
    // continuous.
    resample_buffer: Vec<crate::core::iq::IqSample>,
    resample_position: f64,
}

impl RtltcpSink {
    pub fn new(iq_tx: Sender<Vec<u8>>, requested_sample_rate: Arc<AtomicU32>) -> Self {
        Self {
            samples_processed: 0,
            blocks_processed: 0,
            iq_tx,
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

    pub fn start_server(addr: &str) -> (Sender<Vec<u8>>, Receiver<RtltcpCommand>) {
        let (iq_tx, iq_rx) = mpsc::channel::<Vec<u8>>();
        let (command_tx, command_rx) = mpsc::channel::<RtltcpCommand>();

        let addr = addr.to_string();

        /*
         * List of the RTL-TCP clients currently connected.
         *
         * The IQ core keeps a single permanent Sender to the server. The
         * server then distributes each IQ block to the clients.
         */
        let clients: Arc<std::sync::Mutex<Vec<Sender<Vec<u8>>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        /*
         * IQ DISTRIBUTOR THREAD
         *
         * The IQ Receiver belongs to the server for good: it is never handed
         * over to a client.
         */
        let clients_iq = Arc::clone(&clients);

        thread::spawn(move || {
            while let Ok(data) = iq_rx.recv() {
                let mut clients = match clients_iq.lock() {
                    Ok(clients) => clients,
                    Err(_) => {
                        log::error!("RTL-TCP: client list lock poisoned");
                        break;
                    }
                };

                clients.retain(|client_tx| client_tx.send(data.clone()).is_ok());
            }

            log::debug!("RTL-TCP IQ distributor stopped");
        });

        thread::spawn(move || {
            let listener = match TcpListener::bind(&addr) {
                Ok(listener) => {
                    log::info!("RTL-TCP server listening on {}", addr);
                    listener
                }

                Err(err) => {
                    log::error!("RTL-TCP bind {} failed: {}", addr, err);
                    return;
                }
            };

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        log::info!("RTL-TCP client connected");

                        /*
                         * Each client now has its own IQ channel. The main
                         * Receiver stays in the distributor thread.
                         */
                        let (client_iq_tx, client_iq_rx) = mpsc::channel::<Vec<u8>>();

                        if let Ok(mut clients) = clients.lock() {
                            clients.push(client_iq_tx);
                        } else {
                            log::error!("RTL-TCP: cannot register the client");
                            continue;
                        }

                        let command_tx = command_tx.clone();

                        Self::handle_client(stream, client_iq_rx, command_tx);

                        log::info!("RTL-TCP client disconnected");
                    }

                    Err(err) => {
                        log::warn!("RTL-TCP connection failed: {}", err);
                    }
                }
            }
        });

        (iq_tx, command_rx)
    }

    fn handle_client(
        mut stream: TcpStream,
        iq_rx: Receiver<Vec<u8>>,
        command_tx: Sender<RtltcpCommand>,
    ) {
        if let Err(err) = Self::send_header(&mut stream) {
            log::warn!("RTL-TCP header failed: {}", err);
            return;
        }

        /*
         * Two independent sockets:
         *
         * - command_stream: receives the commands
         * - iq_stream:      sends the IQ data
         *
         * TCP allows this with a clone of the socket.
         */
        let command_stream = match stream.try_clone() {
            Ok(stream) => stream,

            Err(err) => {
                log::error!("RTL-TCP socket clone failed: {}", err);
                return;
            }
        };

        let iq_stream = match stream.try_clone() {
            Ok(stream) => stream,

            Err(err) => {
                log::error!("RTL-TCP IQ socket clone failed: {}", err);
                return;
            }
        };

        let connected = Arc::new(AtomicBool::new(true));

        /*
         * ------------------------------------------------------------
         * COMMAND THREAD
         * ------------------------------------------------------------
         */
        let connected_commands = Arc::clone(&connected);

        let command_thread = thread::spawn(move || {
            Self::handle_commands(command_stream, command_tx, connected_commands);
        });

        /*
         * ------------------------------------------------------------
         * IQ THREAD
         * ------------------------------------------------------------
         */
        let connected_iq = Arc::clone(&connected);

        let iq_thread = thread::spawn(move || {
            Self::handle_iq(iq_stream, iq_rx, connected_iq);
        });

        /*
         * The client's main thread waits for both threads.
         */
        let _ = command_thread.join();

        connected.store(false, Ordering::SeqCst);

        let _ = iq_thread.join();

        /*
         * `stream` is kept up to here so that the main socket stays alive
         * for the duration of both threads.
         */
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }

    fn handle_commands(
        mut stream: TcpStream,
        command_tx: Sender<RtltcpCommand>,
        connected: Arc<AtomicBool>,
    ) {
        /*
         * An RTL-TCP command is exactly 5 bytes:
         *
         * byte 0     : command
         * bytes 1-4  : big-endian u32 value
         *
         * TCP may however deliver these bytes over several reads.
         */
        let mut command_buffer = [0u8; 5];
        let mut command_len = 0usize;
        log::debug!("RTL-TCP command thread started");
        while connected.load(Ordering::SeqCst) {
            match stream.read(&mut command_buffer[command_len..]) {
                Ok(0) => {
                    break;
                }

                Ok(n) => {
                    command_len += n;

                    if command_len == 5 {
                        let command = RtltcpCommand::parse(
                            command_buffer[0],
                            u32::from_be_bytes([
                                command_buffer[1],
                                command_buffer[2],
                                command_buffer[3],
                                command_buffer[4],
                            ]),
                        );

                        log::debug!("RTL-TCP command: {:?}", command);

                        if command_tx.send(command).is_err() {
                            break;
                        }

                        command_len = 0;
                    }
                }

                Err(err) => {
                    log::warn!("RTL-TCP command read failed: {}", err);
                    break;
                }
            }
        }

        connected.store(false, Ordering::SeqCst);
    }

    fn handle_iq(mut stream: TcpStream, iq_rx: Receiver<Vec<u8>>, connected: Arc<AtomicBool>) {
        /*
         * The IQ socket stays BLOCKING.
         *
         * This is deliberate: we do not want to throw away pieces of IQ
         * blocks just because the network is momentarily full.
         */
        while connected.load(Ordering::SeqCst) {
            match iq_rx.recv_timeout(Duration::from_millis(5000)) {
                Ok(data) => {
                    static RECV_DEBUG_COUNT: std::sync::atomic::AtomicUsize =
                        std::sync::atomic::AtomicUsize::new(0);

                    let recv_count =
                        RECV_DEBUG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if recv_count % 100 == 0 {
                        log::trace!(
                            "RTL-TCP RECV #{}: len={} first={:02x} {:02x} {:02x} {:02x} | non7f={}",
                            recv_count,
                            data.len(),
                            data.get(0).copied().unwrap_or(0),
                            data.get(1).copied().unwrap_or(0),
                            data.get(2).copied().unwrap_or(0),
                            data.get(3).copied().unwrap_or(0),
                            data.iter().filter(|&&v| v != 0x7f).count(),
                        );
                    }
                    if let Err(err) = stream.write_all(&data) {
                        // A client closing its connection is normal, not a problem.
                        if matches!(
                            err.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::ConnectionAborted
                        ) {
                            log::debug!("RTL-TCP client closed the connection: {}", err);
                        } else {
                            log::warn!("RTL-TCP IQ write failed: {}", err);
                        }

                        connected.store(false, Ordering::SeqCst);
                        break;
                    }
                }

                Err(mpsc::RecvTimeoutError::Timeout) => {
                    /*
                     * Lets us check regularly whether the command thread
                     * has detected a disconnection.
                     */
                }

                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    connected.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }
    }

    fn send_header(stream: &mut TcpStream) -> std::io::Result<()> {
        let mut header = Vec::with_capacity(12);

        /*
         * RTL-TCP header:
         *
         * 4 bytes: "RTL0"
         * 4 bytes: tuner type
         * 4 bytes: number of gain steps
         */
        header.extend_from_slice(b"RTL0");
        // Tuner type 5 = R820T: clients (AbracaDABra, SDR#...) derive a list
        // of 29 gains from it. With "1" (E4000) and a gain count of 0 the list
        // was empty and AbracaDABra never sent any gain command.
        header.extend_from_slice(&5u32.to_be_bytes());
        header.extend_from_slice(&(crate::backend::gain::GAIN_STEPS as u32).to_be_bytes());

        stream.write_all(&header)
    }
}

impl Default for RtltcpSink {
    fn default() -> Self {
        let (iq_tx, _iq_rx) = mpsc::channel();
        let requested_sample_rate = Arc::new(AtomicU32::new(2_000_000));

        Self::new(iq_tx, requested_sample_rate)
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

        let rtl_iq = self.convert_block(&resampled_block);

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
        if self.blocks_processed % 100 == 0 {
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
                rtl_iq.get(0).copied().unwrap_or(0),
                rtl_iq.get(1).copied().unwrap_or(0),
                rtl_iq.get(2).copied().unwrap_or(0),
                rtl_iq.get(3).copied().unwrap_or(0),
                rtl_iq.get(4).copied().unwrap_or(0),
                rtl_iq.get(5).copied().unwrap_or(0),
                rtl_iq.get(6).copied().unwrap_or(0),
                rtl_iq.get(7).copied().unwrap_or(0),
            );
        }

        if self.iq_tx.send(rtl_iq).is_err() {
            // Warn once: the same failure would otherwise repeat for every block.
            static WARNED: AtomicBool = AtomicBool::new(false);

            if !WARNED.swap(true, Ordering::Relaxed) {
                log::warn!("RTL-TCP: IQ receiver disconnected");
            }
        }
    }
}
