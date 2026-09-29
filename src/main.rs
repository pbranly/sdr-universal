mod backend;
mod core;
mod output;

use anyhow::Result;

use backend::Backend;
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
    println!("=================================");
    println!(" SDR Universal");
    println!(" Test Core -> SDRplay");
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

    let mut sdrplay = SdrplayBackend::new();

    // Récupération du flux IQ avant la connexion au RSP1B.
    // Le thread consommateur sera ainsi prêt avant l'arrivée
    // des premiers blocs IQ.
    let iq_rx = Backend::take_iq_receiver(&mut sdrplay)
    .expect("Receiver IQ indisponible");

    let (rtltcp_tx, rtltcp_commands) =
        RtltcpSink::start_server("0.0.0.0:1234");

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

    Backend::connect(&mut sdrplay)?;

    println!();
    println!("SDRplay connecté.");

    println!();
    println!("--- SetFrequency({}) ---", receiver.state().frequency_hz);
    let frequency_hz = receiver.state().frequency_hz;
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetFrequency(frequency_hz),
    )?;

    println!(">>> TEST SAMPLE RATE");
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetSampleRate(2_000_000),
    )?;

    println!(">>> TEST BANDWIDTH");
    let bandwidth_hz = receiver.state().bandwidth_hz;
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetBandwidth(bandwidth_hz),
    )?;

    println!(">>> TEST IF = Zero");
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetIfType(IfType::Zero),
    )?;

    println!(">>> TEST LO = Auto");
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetLoMode(LoMode::Auto),
    )?;

    // Démarrage prudent : AGC matériel du RSP actif (pas de surcharge à la
    // première connexion) et pas de gain médian de la bande courante.
    // Les clients rtl_tcp reprennent ensuite la main (0x03 / 0x04 / 0x0D).
    println!(">>> Gain initial : AGC RSP + pas {}", backend::sdrplay::gain::DEFAULT_GAIN_INDEX);
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetGainIndex(backend::sdrplay::gain::DEFAULT_GAIN_INDEX),
    )?;
    execute_command(
        &mut receiver,
        &mut sdrplay,
        Command::SetGainMode(GainMode::Automatic),
    )?;

println!(">>> TEST START IQ APRÈS CONFIGURATION");
    execute_command(
        &mut receiver,
        &mut sdrplay,
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
        sdrplay.service();

        match rtltcp_commands.try_recv() {
            Ok(command) => {
                println!(">>> CORE reçoit RTL-TCP : {:?}", command);

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
                    println!(">>> Command Core : {:?}", core_command);

                    if let Err(e) = execute_command(
                        &mut receiver,
                        &mut sdrplay,
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
    Backend::disconnect(&mut sdrplay);

println!("Attente de libération USB...");
std::thread::sleep(std::time::Duration::from_millis(2000));

Ok(())
}
