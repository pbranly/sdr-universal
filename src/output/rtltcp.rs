use crate::core::{Command, GainMode, IqBlock};
use crate::core::iq::IqSink;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
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
    SetAgc(bool),
    Unknown(u8, u32),
}

impl RtltcpCommand {
    pub fn parse(command: u8, value: u32) -> Self {
        match command {
            0x01 => Self::SetFrequency(value),
            0x02 => Self::SetSampleRate(value),
            0x03 => Self::SetGainMode(value),
            0x04 => Self::SetGain(value),
            0x08 => Self::SetAgc(value != 0),
            _ => Self::Unknown(command, value),
        }
    }

    pub fn to_core_command(self) -> Option<Command> {
        match self {
            Self::SetFrequency(value) => {
                Some(Command::SetFrequency(value as u64))
            }

            Self::SetSampleRate(value) => {
                Some(Command::SetSampleRate(value))
            }

            Self::SetGainMode(value) => {
                let mode = if value == 0 {
					GainMode::Automatic
				} else {
					GainMode::Manual
				};

                Some(Command::SetGainMode(mode))
            }

            Self::SetGain(value) => {
				Some(Command::SetGain(value as f32 / 10.0))
			}

            Self::SetAgc(value) => {
                Some(Command::SetAgc(value))
            }

            Self::Unknown(_, _) => None,
        }
    }
}

pub struct RtltcpSink {
    samples_processed: u64,
    blocks_processed: u64,
    iq_tx: Sender<Vec<u8>>,
}

impl RtltcpSink {
    pub fn new(iq_tx: Sender<Vec<u8>>) -> Self {
        Self {
            samples_processed: 0,
            blocks_processed: 0,
            iq_tx,
        }
    }

    pub fn samples_processed(&self) -> u64 {
        self.samples_processed
    }

    fn convert_sample(value: f32) -> u8 {
        let value = value.clamp(-1.0, 1.0);

        ((value + 1.0) * 127.5) as u8
    }

    pub fn convert_block(block: &IqBlock) -> Vec<u8> {
    static DEBUG_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    let debug_count =
        DEBUG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut output = Vec::with_capacity(block.samples.len() * 2);

    for sample in &block.samples {
        output.push(Self::convert_sample(sample.i));
        output.push(Self::convert_sample(sample.q));
    }

//    for sample in &block.samples {
//		output.push(Self::convert_sample(sample.q));  // Q d'abord
//		output.push(Self::convert_sample(sample.i));  // puis I
//	}
    if debug_count < 3 && output.len() >= 8 {
        println!(
            ">>> RTL-TCP INPUT : I/Q=({:.4}, {:.4})",
            block.samples[0].i,
            block.samples[0].q
        );

        println!(
            ">>> RTL-TCP OUTPUT : {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
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

    pub fn start_server(
        addr: &str,
    ) -> (Sender<Vec<u8>>, Receiver<RtltcpCommand>) {
        let (iq_tx, iq_rx) = mpsc::channel::<Vec<u8>>();
        let (command_tx, command_rx) = mpsc::channel::<RtltcpCommand>();

        let addr = addr.to_string();

        thread::spawn(move || {
            let listener = match TcpListener::bind(&addr) {
                Ok(listener) => {
                    println!(">>> RTL-TCP serveur : {}", addr);
                    listener
                }

                Err(err) => {
                    eprintln!(
                        "RTL-TCP bind {} échoué : {}",
                        addr,
                        err
                    );
                    return;
                }
            };

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        println!(">>> RTL-TCP client connecté");

                        /*
                         * On supprime les anciens blocs IQ éventuellement
                         * accumulés pendant qu'aucun client n'était connecté.
                         *
                         * Cela évite qu'une reconnexion reçoive plusieurs
                         * secondes d'anciens IQ avant d'arriver au temps réel.
                         */
                        loop {
                            match iq_rx.try_recv() {
                                Ok(_) => {}

                                Err(mpsc::TryRecvError::Empty) => {
                                    break;
                                }

                                Err(mpsc::TryRecvError::Disconnected) => {
                                    return;
                                }
                            }
                        }

                        let command_tx = command_tx.clone();

                        Self::handle_client(
                            stream,
                            iq_rx,
                            command_tx,
                        );

                        println!("<<< RTL-TCP client déconnecté");

                        /*
                         * Le Receiver IQ est unique et est réutilisé
                         * pour le prochain client.
                         */
                        return;
                    }

                    Err(err) => {
                        eprintln!(
                            "RTL-TCP connexion échouée : {}",
                            err
                        );
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
            eprintln!(
                "RTL-TCP header échoué : {}",
                err
            );
            return;
        }

        /*
         * Deux sockets indépendants :
         *
         * - command_stream : réception des commandes
         * - iq_stream      : émission des IQ
         *
         * TCP permet cela avec un clone du socket.
         */
        let command_stream = match stream.try_clone() {
            Ok(stream) => stream,

            Err(err) => {
                eprintln!(
                    "RTL-TCP clone socket échoué : {}",
                    err
                );
                return;
            }
        };

        let iq_stream = match stream.try_clone() {
            Ok(stream) => stream,

            Err(err) => {
                eprintln!(
                    "RTL-TCP clone IQ échoué : {}",
                    err
                );
                return;
            }
        };

        let connected = Arc::new(AtomicBool::new(true));

        /*
         * ------------------------------------------------------------
         * THREAD COMMANDES
         * ------------------------------------------------------------
         */
        let connected_commands = Arc::clone(&connected);

        let command_thread = thread::spawn(move || {
            Self::handle_commands(
                command_stream,
                command_tx,
                connected_commands,
            );
        });

        /*
         * ------------------------------------------------------------
         * THREAD IQ
         * ------------------------------------------------------------
         */
        let connected_iq = Arc::clone(&connected);

        let iq_thread = thread::spawn(move || {
            Self::handle_iq(
                iq_stream,
                iq_rx,
                connected_iq,
            );
        });

        /*
         * Le thread principal du client attend les deux threads.
         */
        let _ = command_thread.join();

        connected.store(false, Ordering::SeqCst);

        let _ = iq_thread.join();

        /*
         * On conserve stream jusqu'ici pour maintenir le socket
         * principal vivant pendant les deux threads.
         */
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }

    fn handle_commands(
        mut stream: TcpStream,
        command_tx: Sender<RtltcpCommand>,
        connected: Arc<AtomicBool>,
    ) {
        /*
         * Une commande RTL-TCP fait exactement 5 octets :
         *
         * octet 0     : commande
         * octets 1-4  : valeur u32 big-endian
         *
         * TCP peut toutefois fournir ces octets en plusieurs lectures.
         */
        let mut command_buffer = [0u8; 5];
        let mut command_len = 0usize;
		println!(">>> RTL-TCP IQ THREAD démarré");
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

                        println!(
                            ">>> RTL-TCP commande : {:?}",
                            command
                        );

                        if command_tx.send(command).is_err() {
                            break;
                        }

                        command_len = 0;
                    }
                }

                Err(err) => {
                    eprintln!(
                        "RTL-TCP lecture commande : {}",
                        err
                    );
                    break;
                }
            }
        }

        connected.store(false, Ordering::SeqCst);
    }

    fn handle_iq(
        mut stream: TcpStream,
        iq_rx: Receiver<Vec<u8>>,
        connected: Arc<AtomicBool>,
    ) {
        /*
         * Le socket IQ reste BLOQUANT.
         *
         * C'est volontaire :
         * nous ne voulons pas jeter des morceaux de blocs IQ
         * simplement parce que le réseau est momentanément plein.
         */
        while connected.load(Ordering::SeqCst) {
        match iq_rx.recv_timeout(Duration::from_millis(5000)) {
                Ok(data) => {
                    static RECV_DEBUG_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

let recv_count =
    RECV_DEBUG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

if recv_count % 100 == 0 {
    println!(
        ">>> RTLTCP RECV #{} : len={} first={:02x} {:02x} {:02x} {:02x} | non7f={}",
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
                        eprintln!(
                            "RTL-TCP écriture IQ : {}",
                            err
                        );

                        connected.store(false, Ordering::SeqCst);
                        break;
                    }
                }

                Err(mpsc::RecvTimeoutError::Timeout) => {
                    /*
                     * Permet de vérifier régulièrement si le thread
                     * de commandes a détecté une déconnexion.
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
         * RTL-TCP header :
         *
         * 4 octets : "RTL0"
         * 4 octets : tuner type
         * 4 octets : gain index
         */
        header.extend_from_slice(b"RTL0");
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(&0u32.to_be_bytes());

        stream.write_all(&header)
    }
}

impl Default for RtltcpSink {
    fn default() -> Self {
        let (iq_tx, _iq_rx) = mpsc::channel();

        Self::new(iq_tx)
    }
}

impl IqSink for RtltcpSink {
    fn push(&mut self, block: &IqBlock) {
        let rtl_iq = Self::convert_block(block);

        self.blocks_processed += 1;
        self.samples_processed += block.samples.len() as u64;

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

            println!(
                ">>> RTL IQ #{} | seq={} | freq={} Hz | rate={} Hz",
                self.blocks_processed,
                block.sequence,
                block.center_frequency_hz,
                block.sample_rate
            );

            println!(
                "    I: min={:.4} max={:.4} mean={:.4} rms={:.4}",
                i_min,
                i_max,
                i_mean,
                i_rms
            );

            println!(
                "    Q: min={:.4} max={:.4} mean={:.4} rms={:.4}",
                q_min,
                q_max,
                q_mean,
                q_rms
            );

            println!(
                "    RTL: min={} max={} non7f={}/{}",
                rtl_iq.iter().copied().min().unwrap_or(0),
                rtl_iq.iter().copied().max().unwrap_or(0),
                non7f,
                rtl_iq.len()
            );

            println!(
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
            println!(">>> RTLTCP SEND ERROR : Receiver déconnecté");
        }
    }
}