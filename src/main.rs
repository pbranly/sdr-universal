mod backend;
mod core;

use anyhow::Result;

use backend::sdrplay::SdrplayBackend;
use core::{Capabilities, Command, CommandResult, Receiver, ReceiverMode, ReceiverState};

fn main() -> Result<()> {
    println!("=================================");
    println!(" SDR Universal");
    println!(" Test Core -> SDRplay");
    println!("=================================");
    println!();

    let state = ReceiverState {
        frequency_hz: 200_000_000,
        sample_rate: 2_000_000,
        bandwidth_hz: 200_000,
        mode: ReceiverMode::Nfm,
        gain: 50.0,
        ..ReceiverState::default()
    };

    let capabilities = Capabilities::default();

    let mut receiver = Receiver::new(state, capabilities);

    println!("Connexion au SDRplay...");

    let mut sdrplay = SdrplayBackend::new();

    // Récupération du flux IQ avant la connexion au RSP1B.
    // Le thread consommateur sera ainsi prêt avant l'arrivée
    // des premiers blocs IQ.
    let iq_rx = sdrplay
        .take_iq_receiver()
        .expect("Receiver IQ indisponible");

    std::thread::spawn(move || {
        while let Ok(block) = iq_rx.recv() {
            println!(
                "<<< IQ CORE RX : sequence={} samples={} freq={} rate={}",
                block.sequence,
                block.samples.len(),
                block.center_frequency_hz,
                block.sample_rate
            );
        }

        println!("<<< IQ CORE RX arrêté");
    });

    sdrplay.connect()?;

    println!();
    println!("SDRplay connecté.");

    println!();
    println!("--- SetFrequency(145000000) ---");

    match receiver.handle_command(Command::SetFrequency(145_000_000))? {
        CommandResult::Event(event) => {
            println!("Événement Core : {:?}", event);
        }
        other => {
            println!("Résultat Core : {:?}", other);
        }
    }

    sdrplay.set_frequency(receiver.state().frequency_hz)?;

    sdrplay.set_sample_rate(receiver.state().sample_rate)?;

    println!(">>> AVANT set_bandwidth()");

    sdrplay.set_bandwidth(receiver.state().bandwidth_hz)?;

    println!(">>> APRES set_bandwidth()");

println!(">>> Réglage du gain à {} dB", receiver.state().gain);

sdrplay.set_gain(receiver.state().gain)?;

println!(">>> Désactivation de l'AGC");
sdrplay.set_agc(false)?;

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
    println!("Réception IQ pendant 5 secondes...");

    std::thread::sleep(std::time::Duration::from_secs(5));

    println!();
    println!("Fin du test IQ.");

    sdrplay.disconnect();

    Ok(())
}
