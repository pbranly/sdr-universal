mod backend;
mod core;
mod logging;
mod output;

use anyhow::Result;

use backend::mock::MockBackend;
#[cfg(feature = "sdrplay")]
use backend::sdrplay::SdrplayBackend;
use backend::Backend;
use output::RtltcpSink;

use core::{
    Capabilities, Command, CommandResult, GainMode, IfType, LoMode, Receiver, ReceiverMode,
    ReceiverState,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Value of a `--name value` command-line option.
fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Value of an option (command line, otherwise environment variable):
/// absent -> `None`; present but invalid -> explicit error (instead of being
/// silently ignored).
fn option_value<T>(name: &str, env_var: Option<&str>) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = arg_value(name).or_else(|| {
        env_var
            .and_then(|var| std::env::var(var).ok())
            .filter(|value| !value.is_empty())
    });

    match raw {
        None => Ok(None),
        Some(text) => text
            .parse::<T>()
            .map(Some)
            .map_err(|err| anyhow::anyhow!("{}: invalid value '{}' ({})", name, text, err)),
    }
}

/// "sdr-universal 0.0.3 (git v0.0.3, x86_64-unknown-linux-gnu)"
fn version_line() -> String {
    format!(
        "{} {} (git {}, {})",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("SDR_UNIVERSAL_GIT"),
        env!("SDR_UNIVERSAL_TARGET"),
    )
}

fn print_help() {
    println!("{}", version_line());
    println!("{}", env!("CARGO_PKG_DESCRIPTION"));
    println!();
    println!("USAGE:");
    println!("    sdr-universal [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --port N           rtl_tcp port (default 1234); the control port is N+1");
    println!("    --bind ADDR        listen address (default 0.0.0.0, all interfaces; SDR_BIND)");
    println!(
        "        --no-mdns      do not advertise the server on the local network (SDR_MDNS=0)"
    );
    println!("        --name NAME    name shown to clients that discover the server (SDR_NAME)");
    println!("    -v, --verbose      detailed traces (same as SDR_VERBOSE=1)");
    println!("        --mock         simulated RSP1B, no hardware (same as SDR_MOCK=1)");
    println!(
        "        --mock-level D simulated antenna level in dBm (default -75, SDR_MOCK_LEVEL_DBM)"
    );
    println!("    -V, --version      print the version and exit");
    println!("    -h, --help         print this help and exit");
    println!();
    println!("ENVIRONMENT:");
    println!("    SDRPLAY_API_LIB    full path of libsdrplay_api.so (default: system search)");
    println!("    RUST_LOG           log filter, e.g. warn or info,backend::sdrplay=debug (default: info)");
    println!();
    println!("Documentation: {}", env!("CARGO_PKG_REPOSITORY"));
}

/// Real (SDRplay) or mock backend (--mock / SDR_MOCK=1, no hardware).
fn create_backend(mock: bool, mock_level_dbm: f64) -> Result<Box<dyn Backend>> {
    if mock {
        log::info!(
            "MOCK MODE: no hardware, simulated antenna level {} dBm",
            mock_level_dbm
        );
        return Ok(Box::new(MockBackend::new(mock_level_dbm)));
    }

    #[cfg(feature = "sdrplay")]
    {
        // Fail immediately, with a readable message, if the SDRplay API is not
        // installed: the library is loaded at run time, not at build time.
        backend::sdrplay::ensure_api_loaded()?;
        Ok(Box::new(SdrplayBackend::new()))
    }

    #[cfg(not(feature = "sdrplay"))]
    {
        Err(anyhow::anyhow!(
            "Built without the SDRplay backend: run again with --mock"
        ))
    }
}

fn execute_command(
    receiver: &mut Receiver,
    backend: &mut dyn Backend,
    command: Command,
) -> Result<()> {
    match receiver.handle_command(command)? {
        CommandResult::Event(event) => {
            backend.apply_event(&event)?;
        }
        CommandResult::Events(events) => {
            for event in &events {
                backend.apply_event(event)?;
            }
        }
        other => {
            log::debug!("Core result: {:?}", other);
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    if std::env::args()
        .skip(1)
        .any(|a| a == "--version" || a == "-V")
    {
        println!("{}", version_line());
        return Ok(());
    }

    if std::env::args().skip(1).any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    let verbose = std::env::args().any(|a| a == "--verbose" || a == "-v")
        || std::env::var("SDR_VERBOSE")
            .map(|v| v == "1")
            .unwrap_or(false);

    logging::init(if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });

    log::info!(
        "SDR Universal {} (rtl_tcp gateway for SDRplay)",
        env!("CARGO_PKG_VERSION")
    );

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = Arc::clone(&running);
        ctrlc::set_handler(move || {
            log::info!("Ctrl+C received: clean shutdown in progress...");
            running.store(false, Ordering::SeqCst);
        })
        .expect("cannot install the Ctrl+C handler");
    }

    let state = ReceiverState {
        frequency_hz: 100_000_000,
        sample_rate: 2_000_000,
        bandwidth_hz: 1_536_000,
        mode: ReceiverMode::Nfm,
        gain: 50.0,
        ..ReceiverState::default()
    };

    let capabilities = Capabilities::default();

    let mut receiver = Receiver::new(state, capabilities);

    log::debug!("initialising...");

    let use_mock = std::env::args().any(|a| a == "--mock")
        || std::env::var("SDR_MOCK").map(|v| v == "1").unwrap_or(false);

    let mock_level_dbm: f64 =
        option_value("--mock-level", Some("SDR_MOCK_LEVEL_DBM"))?.unwrap_or(-75.0);

    if !mock_level_dbm.is_finite() {
        return Err(anyhow::anyhow!(
            "--mock-level: invalid value '{}' (finite number expected)",
            mock_level_dbm
        ));
    }

    let rtltcp_port: u16 = option_value("--port", None)?.unwrap_or(1234);

    let control_port = rtltcp_port.checked_add(1).ok_or_else(|| {
        anyhow::anyhow!(
            "--port {}: the control port (port + 1) would exceed 65535",
            rtltcp_port
        )
    })?;

    // Listen address: all interfaces by default (historical behaviour), or a
    // specific address, for example 127.0.0.1 to restrict access to this
    // machine.
    let bind_ip: std::net::IpAddr = option_value("--bind", Some("SDR_BIND"))?
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

    let mdns_disabled = std::env::args().any(|a| a == "--no-mdns")
        || std::env::var("SDR_MDNS").map(|v| v == "0").unwrap_or(false);

    let service_name: Option<String> = option_value("--name", Some("SDR_NAME"))?;

    if let Some(name) = &service_name {
        if name.trim().is_empty() {
            return Err(anyhow::anyhow!("--name: the name must not be empty"));
        }
    }

    let rtltcp_addr = std::net::SocketAddr::new(bind_ip, rtltcp_port).to_string();
    let control_addr = std::net::SocketAddr::new(bind_ip, control_port).to_string();

    let mut radio = create_backend(use_mock, mock_level_dbm)?;

    // Take the IQ stream before connecting to the receiver, so that the
    // consumer thread is ready before the first IQ blocks arrive.
    let iq_rx = radio.take_iq_receiver().expect("IQ receiver unavailable");

    let (rtltcp_tx, rtltcp_commands) = RtltcpSink::start_server(&rtltcp_addr);

    // Control port (rtl_tcp + 1): real RSP gain, for AbracaDABra's RF level.
    output::control::start_server(&control_addr);

    if bind_ip.is_unspecified() {
        log::warn!(
            "listening on all network interfaces, without authentication; \
             --bind 127.0.0.1 restricts access to this machine"
        );
    }

    // Let clients on the local network discover the server (mDNS / DNS-SD).
    let advertisement = match output::mdns::decide(bind_ip, mdns_disabled) {
        output::mdns::Decision::Advertise => {
            let instance = output::mdns::truncate_label(&service_name.unwrap_or_else(|| {
                output::mdns::default_instance_name(&output::mdns::local_hostname())
            }));

            match output::mdns::Advertisement::start(
                &instance,
                bind_ip,
                rtltcp_port,
                if use_mock { "mock" } else { "sdrplay" },
            ) {
                Ok(advertisement) => {
                    log::info!(
                        "mDNS: advertising '{}' ({}, port {})",
                        instance,
                        output::mdns::SERVICE_TYPE,
                        rtltcp_port
                    );
                    Some(advertisement)
                }
                Err(e) => {
                    log::warn!("mDNS advertisement unavailable: {}", e);
                    None
                }
            }
        }
        output::mdns::Decision::Disabled => {
            log::info!("mDNS advertisement disabled");
            None
        }
        output::mdns::Decision::Loopback => {
            log::info!("mDNS advertisement skipped: listening on the loopback interface only");
            None
        }
    };

    let requested_sample_rate = Arc::new(AtomicU32::new(2_000_000));

    let rtltcp = RtltcpSink::new(rtltcp_tx, Arc::clone(&requested_sample_rate));

    std::thread::spawn(move || {
        let mut processor = core::IqProcessor::new();

        processor.distributor_mut().add_sink(Box::new(rtltcp));

        while let Ok(block) = iq_rx.recv() {
            processor.process(block);
        }

        log::debug!(
            "core IQ processing stopped: blocks={} samples={}",
            processor.blocks_processed(),
            processor.samples_processed()
        );
    });

    radio.connect()?;

    log::info!("Receiver connected");

    log::debug!(
        "configuring: frequency {} Hz",
        receiver.state().frequency_hz
    );
    let frequency_hz = receiver.state().frequency_hz;
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetFrequency(frequency_hz),
    )?;

    log::debug!("configuring: sample rate");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetSampleRate(2_000_000),
    )?;

    log::debug!("configuring: bandwidth");
    let bandwidth_hz = receiver.state().bandwidth_hz;
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetBandwidth(bandwidth_hz),
    )?;

    log::debug!("configuring: IF = zero");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetIfType(IfType::Zero),
    )?;

    log::debug!("configuring: LO = auto");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetLoMode(LoMode::Auto),
    )?;

    // Cautious start: the RSP hardware AGC is on (no overload on the first
    // connection) and the gain step is the middle one of the current band.
    // rtl_tcp clients then take over (0x03 / 0x04 / 0x0D).
    log::info!(
        "Initial gain: RSP hardware AGC + step {}",
        backend::gain::DEFAULT_GAIN_INDEX
    );
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetGainIndex(backend::gain::DEFAULT_GAIN_INDEX),
    )?;
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetGainMode(GainMode::Automatic),
    )?;

    log::debug!("starting the IQ stream");
    execute_command(&mut receiver, radio.as_mut(), Command::StartIq)?;

    std::thread::sleep(std::time::Duration::from_millis(300));

    log::debug!("core state: {:#?}", receiver.state());

    log::info!(
        "Receiver configured: {} Hz, sample rate {} Hz, bandwidth {} Hz",
        receiver.state().frequency_hz,
        receiver.state().sample_rate,
        receiver.state().bandwidth_hz
    );
    log::info!("Ready: waiting for RTL-TCP commands...");

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Acknowledge ADC overloads (outside the API callbacks).
        radio.service();

        match rtltcp_commands.try_recv() {
            Ok(command) => {
                log::trace!("core received RTL-TCP command: {:?}", command);

                if let output::rtltcp::RtltcpCommand::SetSampleRate(rate) = command {
                    log::info!("RTL-TCP sample rate requested: {} Hz", rate);
                }

                if let Some(core_command) = command.to_core_command(receiver.state().sample_rate) {
                    log::trace!("core command: {:?}", core_command);

                    match execute_command(&mut receiver, radio.as_mut(), core_command.clone()) {
                        Ok(()) => {
                            // The receiver now delivers exactly this rate: the
                            // output stage has nothing to resample.
                            if let Command::SetSampleRate(rate) = core_command {
                                requested_sample_rate.store(rate, Ordering::Relaxed);
                            }
                        }
                        Err(e) => log::warn!("Core command failed: {}", e),
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::warn!("RTL-TCP channel disconnected");
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    if let Some(advertisement) = advertisement {
        advertisement.stop();
    }

    log::info!("Releasing the receiver before exit...");
    radio.disconnect();

    log::debug!("waiting for the USB device to be released...");
    std::thread::sleep(std::time::Duration::from_millis(2000));

    Ok(())
}
