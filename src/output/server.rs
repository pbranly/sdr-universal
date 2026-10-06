//! TCP servers shared by the rtl_tcp and rsp_tcp front ends.
//!
//! One [`ServerHub`] owns what the servers share:
//!
//! * the list of connected clients, each with its own **bounded** IQ queue, so
//!   a client that stops reading can never make the gateway's memory grow;
//! * the commands channel towards the core;
//! * a session lock: the receiver has a single frequency, gain and sample rate,
//!   so only one client at a time is served, whichever server it connected to.
//!   Another client waits (connected, without a greeting) until the active one
//!   leaves.
//!
//! Each [`ServerHub::listen`] call starts one listener: plain rtl_tcp (8-bit
//! samples) or SDRplay's rsp_tcp extended mode (capability block, optional
//! 16-bit samples, extra commands).

use super::rsp_tcp::{self, SampleFormat};
use super::rtltcp::RtltcpCommand;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// Blocks queued per client before new ones are dropped (about 2 seconds at
/// 2.4 MS/s, a few MB).
const CLIENT_QUEUE_BLOCKS: usize = 1024;

/// IQ data converted for each format that has clients.
#[derive(Default)]
pub struct IqFrame {
    pub u8_data: Option<Arc<Vec<u8>>>,
    pub i16_data: Option<Arc<Vec<u8>>>,
}

/// Which sample formats currently have clients: the sink only converts what is
/// wanted.
#[derive(Default)]
pub struct Demand {
    pub u8: AtomicBool,
    pub i16: AtomicBool,
}

/// Kind of server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// Plain rtl_tcp: `RTL0` greeting, 8-bit samples.
    RtlTcp,
    /// SDRplay rsp_tcp extended mode: `RTL0` greeting, `RSP0` capability block,
    /// extra commands, 8-bit or 16-bit samples.
    RspTcp(SampleFormat),
}

impl Flavor {
    pub fn name(self) -> &'static str {
        match self {
            Flavor::RtlTcp => "RTL-TCP",
            Flavor::RspTcp(_) => "RSP-TCP",
        }
    }

    pub fn format(self) -> SampleFormat {
        match self {
            Flavor::RtlTcp => SampleFormat::Uint8,
            Flavor::RspTcp(format) => format,
        }
    }
}

struct ClientSlot {
    id: u64,
    format: SampleFormat,
    tx: SyncSender<Arc<Vec<u8>>>,
    dropped: u64,
}

type Clients = Arc<Mutex<Vec<ClientSlot>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another thread panicked: the data is still
    // usable, and the gateway should keep serving.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn update_demand(clients: &[ClientSlot], demand: &Demand) {
    demand.u8.store(
        clients.iter().any(|c| c.format == SampleFormat::Uint8),
        Ordering::Relaxed,
    );
    demand.i16.store(
        clients.iter().any(|c| c.format == SampleFormat::Int16),
        Ordering::Relaxed,
    );
}

pub struct ServerHub {
    clients: Clients,
    demand: Arc<Demand>,
    session: Arc<Mutex<()>>,
    command_tx: Sender<RtltcpCommand>,
    next_id: Arc<AtomicU64>,
}

impl ServerHub {
    /// Creates the hub and starts the IQ distributor thread. Returns the hub,
    /// the channel the IQ sink feeds, the commands received from clients, and
    /// the demand flags the sink reads.
    pub fn start() -> (
        ServerHub,
        Sender<IqFrame>,
        Receiver<RtltcpCommand>,
        Arc<Demand>,
    ) {
        let (iq_tx, iq_rx) = mpsc::channel::<IqFrame>();
        let (command_tx, command_rx) = mpsc::channel::<RtltcpCommand>();

        let clients: Clients = Arc::new(Mutex::new(Vec::new()));
        let demand = Arc::new(Demand::default());

        // IQ distributor: the IQ receiver belongs to the hub for good and is
        // never handed over to a client.
        let distributor_clients = Arc::clone(&clients);
        let distributor_demand = Arc::clone(&demand);

        thread::spawn(move || {
            while let Ok(frame) = iq_rx.recv() {
                let mut clients = lock(&distributor_clients);
                let before = clients.len();

                clients.retain_mut(|client| {
                    let data = match client.format {
                        SampleFormat::Uint8 => frame.u8_data.as_ref(),
                        SampleFormat::Int16 => frame.i16_data.as_ref(),
                    };

                    let Some(data) = data else {
                        return true;
                    };

                    match client.tx.try_send(Arc::clone(data)) {
                        Ok(()) => true,
                        Err(TrySendError::Full(_)) => {
                            // A client that cannot keep up loses blocks instead
                            // of making the queue grow without limit.
                            client.dropped += 1;

                            if client.dropped == 1 || client.dropped % 1000 == 0 {
                                log::warn!(
                                    "client too slow: dropping IQ blocks ({} so far)",
                                    client.dropped
                                );
                            }
                            true
                        }
                        Err(TrySendError::Disconnected(_)) => false,
                    }
                });

                if clients.len() != before {
                    update_demand(&clients, &distributor_demand);
                }
            }

            log::debug!("IQ distributor stopped");
        });

        let hub = ServerHub {
            clients,
            demand: Arc::clone(&demand),
            session: Arc::new(Mutex::new(())),
            command_tx,
            next_id: Arc::new(AtomicU64::new(1)),
        };

        (hub, iq_tx, command_rx, demand)
    }

    /// Starts a listener thread on `addr`.
    pub fn listen(&self, addr: &str, flavor: Flavor) {
        let addr = addr.to_string();
        let clients = Arc::clone(&self.clients);
        let demand = Arc::clone(&self.demand);
        let session = Arc::clone(&self.session);
        let command_tx = self.command_tx.clone();
        let next_id = Arc::clone(&self.next_id);

        thread::spawn(move || {
            let listener = match TcpListener::bind(&addr) {
                Ok(listener) => {
                    match flavor {
                        Flavor::RtlTcp => {
                            log::info!("RTL-TCP server listening on {}", addr);
                        }
                        Flavor::RspTcp(format) => {
                            log::info!(
                                "RSP-TCP extended server listening on {} ({}-bit samples)",
                                addr,
                                if format == SampleFormat::Int16 { 16 } else { 8 }
                            );
                        }
                    }
                    listener
                }

                Err(err) => {
                    log::error!("{} bind {} failed: {}", flavor.name(), addr, err);
                    return;
                }
            };

            for stream in listener.incoming() {
                let stream = match stream {
                    Ok(stream) => stream,
                    Err(err) => {
                        log::warn!("{} connection failed: {}", flavor.name(), err);
                        continue;
                    }
                };

                // One session at a time, across all servers.
                let _session = match session.try_lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        log::info!(
                            "{} client connected: waiting for the active session to end",
                            flavor.name()
                        );
                        lock(&session)
                    }
                };

                log::info!("{} client connected", flavor.name());

                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let (client_tx, client_rx) =
                    mpsc::sync_channel::<Arc<Vec<u8>>>(CLIENT_QUEUE_BLOCKS);

                {
                    let mut clients = lock(&clients);
                    clients.push(ClientSlot {
                        id,
                        format: flavor.format(),
                        tx: client_tx,
                        dropped: 0,
                    });
                    update_demand(&clients, &demand);
                }

                handle_client(stream, flavor, client_rx, command_tx.clone());

                {
                    let mut clients = lock(&clients);
                    clients.retain(|client| client.id != id);
                    update_demand(&clients, &demand);
                }

                log::info!("{} client disconnected", flavor.name());
            }
        });
    }
}

fn handle_client(
    mut stream: TcpStream,
    flavor: Flavor,
    iq_rx: Receiver<Arc<Vec<u8>>>,
    command_tx: Sender<RtltcpCommand>,
) {
    if let Err(err) = send_greeting(&mut stream, flavor) {
        log::warn!("{} greeting failed: {}", flavor.name(), err);
        return;
    }

    // Two independent sockets: commands in, IQ out (a clone of the TCP socket).
    let command_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(err) => {
            log::error!("{} socket clone failed: {}", flavor.name(), err);
            return;
        }
    };

    let iq_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(err) => {
            log::error!("{} IQ socket clone failed: {}", flavor.name(), err);
            return;
        }
    };

    let connected = Arc::new(AtomicBool::new(true));

    let connected_commands = Arc::clone(&connected);
    let command_thread = thread::spawn(move || {
        handle_commands(command_stream, flavor, command_tx, connected_commands);
    });

    let connected_iq = Arc::clone(&connected);
    let iq_thread = thread::spawn(move || {
        handle_iq(iq_stream, flavor, iq_rx, connected_iq);
    });

    // The session ends when the command side sees the client leave.
    let _ = command_thread.join();
    connected.store(false, Ordering::SeqCst);
    let _ = iq_thread.join();

    // `stream` is kept up to here so that the main socket stays alive for the
    // duration of both threads.
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn handle_commands(
    mut stream: TcpStream,
    flavor: Flavor,
    command_tx: Sender<RtltcpCommand>,
    connected: Arc<AtomicBool>,
) {
    // A command is exactly 5 bytes: one opcode and a big-endian u32 value. TCP
    // may deliver them over several reads.
    let mut command_buffer = [0u8; 5];
    let mut command_len = 0usize;

    log::debug!("{} command thread started", flavor.name());

    while connected.load(Ordering::SeqCst) {
        match stream.read(&mut command_buffer[command_len..]) {
            Ok(0) => break,
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
                        matches!(flavor, Flavor::RspTcp(_)),
                    );

                    log::debug!("{} command: {:?}", flavor.name(), command);

                    if command_tx.send(command).is_err() {
                        break;
                    }

                    command_len = 0;
                }
            }
            Err(err) => {
                // A client closing its connection abruptly is normal.
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionAborted
                ) {
                    log::debug!("{} client closed the connection: {}", flavor.name(), err);
                } else {
                    log::warn!("{} command read failed: {}", flavor.name(), err);
                }
                break;
            }
        }
    }

    connected.store(false, Ordering::SeqCst);
}

fn handle_iq(
    mut stream: TcpStream,
    flavor: Flavor,
    iq_rx: Receiver<Arc<Vec<u8>>>,
    connected: Arc<AtomicBool>,
) {
    // The IQ socket stays BLOCKING: a momentarily full network must not throw
    // away pieces of blocks. The bounded queue upstream is what protects the
    // gateway's memory if the client stalls for good.
    while connected.load(Ordering::SeqCst) {
        match iq_rx.recv_timeout(Duration::from_millis(5000)) {
            Ok(data) => {
                if let Err(err) = stream.write_all(&data) {
                    // A client closing its connection is normal, not a problem.
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionAborted
                    ) {
                        log::debug!("{} client closed the connection: {}", flavor.name(), err);
                    } else {
                        log::warn!("{} IQ write failed: {}", flavor.name(), err);
                    }

                    connected.store(false, Ordering::SeqCst);
                    break;
                }
            }
            // Lets us check regularly whether the command thread has seen the
            // client leave.
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                connected.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
}

/// `RTL0` header, then (rsp_tcp extended mode only) the `RSP0` capability block.
fn send_greeting(stream: &mut TcpStream, flavor: Flavor) -> std::io::Result<()> {
    let mut greeting = Vec::with_capacity(12 + rsp_tcp::CAPABILITIES_LEN);

    // RTL-TCP header: "RTL0", tuner type, number of gain steps.
    greeting.extend_from_slice(b"RTL0");
    // Tuner type 5 = R820T: clients (AbracaDABra, SDR#...) derive a list of 29
    // gains from it. With "1" (E4000) and a gain count of 0 the list was empty
    // and AbracaDABra never sent any gain command.
    greeting.extend_from_slice(&5u32.to_be_bytes());
    greeting.extend_from_slice(&(crate::backend::gain::GAIN_STEPS as u32).to_be_bytes());

    if let Flavor::RspTcp(format) = flavor {
        greeting.extend_from_slice(&rsp_tcp::build_capabilities(format));
    }

    stream.write_all(&greeting)
}
