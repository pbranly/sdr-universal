/// Traces détaillées (événements API, statistiques IQ, commandes brutes).
/// Activées par `--verbose` ou la variable d'environnement SDR_VERBOSE=1.
/// Les lignes utiles au diagnostic (GAIN, SURCHARGE, erreurs...) restent
/// toujours affichées.
pub static VERBOSE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

macro_rules! vprintln {
    ($($arg:tt)*) => {
        if $crate::VERBOSE.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($arg)*);
        }
    };
}

mod backend;
mod core;
mod output;

use anyhow::Result;

use backend::mock::MockBackend;
use backend::Backend;
#[cfg(feature = "sdrplay")]
use backend::sdrplay::SdrplayBackend;
use output::RtltcpSink;

use core::{
    Capabilities,
    Command,
    CommandResult,
    GainMode,
    IfType,
    LoMode,
    Receiver,
    ReceiverMode,
    ReceiverState,
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Valeur d'une option `--nom valeur` de la ligne de commande.
fn arg_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Valeur d'une option (ligne de commande, sinon variable d'environnement) :
/// absente -> `None` ; présente mais invalide -> erreur explicite (au lieu
/// d'être ignorée en silence).
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
        Some(text) => text.parse::<T>().map(Some).map_err(|err| {
            anyhow::anyhow!("{} : valeur invalide « {} » ({})", name, text, err)
        }),
    }
}

/// « sdr-universal 0.0.3 (git v0.0.3, x86_64-unknown-linux-gnu) »
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
    println!("    -v, --verbose      detailed traces (same as SDR_VERBOSE=1)");
    println!("        --mock         simulated RSP1B, no hardware (same as SDR_MOCK=1)");
    println!("        --mock-level D simulated antenna level in dBm (default -75, SDR_MOCK_LEVEL_DBM)");
    println!("    -V, --version      print the version and exit");
    println!("    -h, --help         print this help and exit");
    println!();
    println!("ENVIRONMENT:");
    println!("    SDRPLAY_API_LIB    full path of libsdrplay_api.so (default: system search)");
    println!();
    println!("Documentation: {}", env!("CARGO_PKG_REPOSITORY"));
}

/// Backend réel (SDRplay) ou factice (--mock / SDR_MOCK=1, sans matériel).
fn create_backend(mock: bool, mock_level_dbm: f64) -> Result<Box<dyn Backend>> {
    if mock {
        println!(">>> MODE FACTICE : aucun matériel, niveau d'antenne simulé {} dBm", mock_level_dbm);
        return Ok(Box::new(MockBackend::new(mock_level_dbm)));
    }

    #[cfg(feature = "sdrplay")]
    {
        // Échec immédiat et lisible si l'API SDRplay n'est pas installée :
        // la bibliothèque est chargée à l'exécution, pas à la compilation.
        backend::sdrplay::ensure_api_loaded()?;
        Ok(Box::new(SdrplayBackend::new()))
    }

    #[cfg(not(feature = "sdrplay"))]
    {
        Err(anyhow::anyhow!(
            "Compilé sans le backend SDRplay : relancez avec --mock"
        ))
    }
}

fn execute_command(receiver: &mut Receiver, backend: &mut dyn Backend, command: Command) -> Result<()> {
    match receiver.handle_command(command)? {
        CommandResult::Event(event) => {
            backend.apply_event(&event)?;
        }
        other => {
            println!("Résultat Core : {:?}", other);
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{}", version_line());
        return Ok(());
    }

    if std::env::args().skip(1).any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if std::env::args().any(|a| a == "--verbose" || a == "-v")
        || std::env::var("SDR_VERBOSE").map(|v| v == "1").unwrap_or(false)
    {
        VERBOSE.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    println!("=================================");
    println!(" SDR Universal {}", env!("CARGO_PKG_VERSION"));
    println!(" rtl_tcp gateway for SDRplay");
    println!("=================================");
    println!();

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = Arc::clone(&running);
        ctrlc::set_handler(move || {
            println!("\n>>> Ctrl+C reçu, arrêt propre en cours...");
            running.store(false, Ordering::SeqCst);
        })
        .expect("Impossible d'installer le handler Ctrl+C");
    }

    let state = ReceiverState {
        frequency_hz: 100_000_000,
        sample_rate: 2_000_000,
//        bandwidth_hz: 200_000,
		bandwidth_hz: 1_536_000,
        mode: ReceiverMode::Nfm,
        gain: 50.0,
        ..ReceiverState::default()
    };

    let capabilities = Capabilities::default();

    let mut receiver = Receiver::new(state, capabilities);

println!(">>> TEST START IQ");



    println!("Connexion au SDRplay...");

    let use_mock = std::env::args().any(|a| a == "--mock")
        || std::env::var("SDR_MOCK").map(|v| v == "1").unwrap_or(false);

    let mock_level_dbm: f64 =
        option_value("--mock-level", Some("SDR_MOCK_LEVEL_DBM"))?.unwrap_or(-75.0);

    if !mock_level_dbm.is_finite() {
        return Err(anyhow::anyhow!(
            "--mock-level : valeur invalide « {} » (nombre fini attendu)",
            mock_level_dbm
        ));
    }

    let rtltcp_port: u16 = option_value("--port", None)?.unwrap_or(1234);

    let control_port = rtltcp_port.checked_add(1).ok_or_else(|| {
        anyhow::anyhow!(
            "--port {} : le port de contrôle (port + 1) dépasserait 65535",
            rtltcp_port
        )
    })?;

    // Adresse d'écoute : toutes les interfaces par défaut (comportement
    // historique), ou une adresse précise, par exemple 127.0.0.1 pour limiter
    // l'accès à cette machine.
    let bind_ip: std::net::IpAddr = option_value("--bind", Some("SDR_BIND"))?
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

    let rtltcp_addr = std::net::SocketAddr::new(bind_ip, rtltcp_port).to_string();
    let control_addr = std::net::SocketAddr::new(bind_ip, control_port).to_string();

    let mut radio = create_backend(use_mock, mock_level_dbm)?;

    // Récupération du flux IQ avant la connexion au RSP1B.
    // Le thread consommateur sera ainsi prêt avant l'arrivée
    // des premiers blocs IQ.
    let iq_rx = radio
        .take_iq_receiver()
        .expect("Receiver IQ indisponible");

    let (rtltcp_tx, rtltcp_commands) =
        RtltcpSink::start_server(&rtltcp_addr);

    // Port de contrôle (rtl_tcp + 1) : gain réel du RSP pour le niveau RF
    // d'AbracaDABra.
    output::control::start_server(&control_addr);

    if bind_ip.is_unspecified() {
        println!(
            ">>> ATTENTION : écoute sur toutes les interfaces réseau, sans authentification ; \
             --bind 127.0.0.1 limite l'accès à cette machine."
        );
    }

    let requested_sample_rate = Arc::new(AtomicU32::new(2_000_000));

    let rtltcp = RtltcpSink::new(
        rtltcp_tx,
        Arc::clone(&requested_sample_rate),
    );

    std::thread::spawn(move || {
        let mut processor = core::IqProcessor::new();

        processor.distributor_mut().add_sink(Box::new(rtltcp));

        while let Ok(block) = iq_rx.recv() {
            processor.process(block);
        }

        println!(
            "<<< CORE IQ arrêté : blocs={} samples={}",
            processor.blocks_processed(),
            processor.samples_processed()
        );
    });

    radio.connect()?;

    println!();
    println!("SDRplay connecté.");

    println!();
    println!("--- SetFrequency({}) ---", receiver.state().frequency_hz);
    let frequency_hz = receiver.state().frequency_hz;
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetFrequency(frequency_hz),
    )?;

    println!(">>> TEST SAMPLE RATE");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetSampleRate(2_000_000),
    )?;

    println!(">>> TEST BANDWIDTH");
    let bandwidth_hz = receiver.state().bandwidth_hz;
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetBandwidth(bandwidth_hz),
    )?;

    println!(">>> TEST IF = Zero");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetIfType(IfType::Zero),
    )?;

    println!(">>> TEST LO = Auto");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::SetLoMode(LoMode::Auto),
    )?;

    // Démarrage prudent : AGC matériel du RSP actif (pas de surcharge à la
    // première connexion) et pas de gain médian de la bande courante.
    // Les clients rtl_tcp reprennent ensuite la main (0x03 / 0x04 / 0x0D).
    println!(">>> Gain initial : AGC RSP + pas {}", backend::gain::DEFAULT_GAIN_INDEX);
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

println!(">>> TEST START IQ APRÈS CONFIGURATION");
    execute_command(
        &mut receiver,
        radio.as_mut(),
        Command::StartIq,
    )?;

std::thread::sleep(std::time::Duration::from_millis(300));





println!();
println!("=================================");
println!(" État Core");
println!("=================================");

println!("{:#?}", receiver.state());
    println!();
    println!("Le RSP1B doit maintenant être à :");
    println!("  {} Hz", receiver.state().frequency_hz);
    println!("  {} Hz de sample rate", receiver.state().sample_rate);
    println!("  {} Hz de bande passante", receiver.state().bandwidth_hz);

    println!();
    println!("Flux IQ toujours disponible.");
    println!();
    println!("Test Core -> SDRplay terminé.");

    println!();
    println!("=================================");
    println!(" Attente du flux IQ");
	println!("=================================");
	println!("Core actif - attente des commandes RTL-TCP...");

	loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        // Acquittement des surcharges ADC (hors callbacks de l'API).
        radio.service();

        match rtltcp_commands.try_recv() {
            Ok(command) => {
                vprintln!(">>> CORE reçoit RTL-TCP : {:?}", command);

                if let output::rtltcp::RtltcpCommand::SetSampleRate(rate) = command {
                    requested_sample_rate.store(rate, Ordering::Relaxed);
                    println!(">>> RTL-TCP sample rate demandé : {} Hz", rate);
                }

                if let output::rtltcp::RtltcpCommand::SetSampleRate(rate) = command {
                    if rate < 2_000_000 {
                        println!(
                            ">>> RTL-TCP : {} Hz traité uniquement par le resampler",
                            rate
                        );
                        continue;
                    }
                }

                if let Some(core_command) = command.to_core_command() {
                    vprintln!(">>> Command Core : {:?}", core_command);

                    if let Err(e) = execute_command(
                        &mut receiver,
                        radio.as_mut(),
                        core_command,
                    ) {
                        println!(">>> Erreur commande Core RTL-TCP : {}", e);
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                println!(">>> Canal RTL-TCP déconnecté");
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    println!();
    println!("Fin du test IQ.");

    println!("Libération du RSP1B avant fermeture...");
    radio.disconnect();

println!("Attente de libération USB...");
std::thread::sleep(std::time::Duration::from_millis(2000));

Ok(())
}
