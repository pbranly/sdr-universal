mod backend;
mod core;

use anyhow::Result;

use backend::sdrplay::SdrplayBackend;
use core::{
    Capabilities,
    Command,
    CommandResult,
	 Event,
    IfType,
    LoMode,
    Receiver,
    ReceiverMode,
    ReceiverState,
};

fn main() -> Result<()> {
    println!("=================================");
    println!(" SDR Universal");
    println!(" Test Core -> SDRplay");
    println!("=================================");
    println!();

    let state = ReceiverState {
        frequency_hz: 100_000_000,
        sample_rate: 2_000_000,
        bandwidth_hz: 200_000,
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



    match receiver.handle_command(
    Command::SetFrequency(receiver.state().frequency_hz)
)? {
    CommandResult::Event(event) => {
        println!("Événement Core : {:?}", event);
        sdrplay.apply_event(&event)?;
    }
    other => {
        println!("Résultat Core : {:?}", other);
    }
}

println!(">>> TEST SAMPLE RATE");

match receiver.handle_command(
    Command::SetSampleRate(receiver.state().sample_rate)
)? {
    CommandResult::Event(event) => {
        println!("Événement Core : {:?}", event);
        sdrplay.apply_event(&event)?;
    }
    other => {
        println!("Résultat Core : {:?}", other);
    }
}

// Le sample rate est déjà appliqué par apply_event().


println!(">>> AVANT set_bandwidth()");

// La bande passante est appliquée par apply_event().

println!(">>> APRES set_bandwidth()");

for if_type in [
    IfType::Zero,
    IfType::KHz450,
    IfType::KHz1620,
    IfType::KHz2048,
] {
    println!(">>> TEST IF = {:?}", if_type);

    match receiver.handle_command(Command::SetIfType(if_type))? {
        CommandResult::Event(event) => {
            println!("Événement Core : {:?}", event);
            sdrplay.apply_event(&event)?;
        }
        other => {
            println!("Résultat Core : {:?}", other);
        }
    }

    // L'IF est appliquée par apply_event().
}

println!(">>> TEST LO = Auto");

match receiver.handle_command(Command::SetLoMode(LoMode::Auto))? {
    CommandResult::Event(event) => {
        println!("Événement Core : {:?}", event);
    }
    other => {
        println!("Résultat Core : {:?}", other);
    }
}

match receiver.handle_command(Command::SetLoMode(LoMode::Auto))? {
    CommandResult::Event(event) => {
        println!("Événement Core : {:?}", event);
        sdrplay.apply_event(&event)?;
    }
    other => {
        println!("Résultat Core : {:?}", other);
    }
}
println!(">>> Désactivation AGC AVANT TEST GAIN");
match receiver.handle_command(Command::SetAgc(false))? {
    CommandResult::Event(event) => {
        println!("Événement Core : {:?}", event);
        sdrplay.apply_event(&event)?;
    }
    other => {
        println!("Résultat Core : {:?}", other);
    }
}
println!(">>> Réglage du gain à {} dB", receiver.state().gain);
println!(">>> TEST CORE GAIN");

match receiver.handle_command(
    Command::SetGain(receiver.state().gain)
)? {
    CommandResult::Event(event) => {
        println!("Événement Core : {:?}", event);
    }
    other => {
        println!("Résultat Core : {:?}", other);
    }
}

// Le gain est appliqué par apply_event().

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
    println!("Réception IQ pendant 5 secondes...");

    std::thread::sleep(std::time::Duration::from_secs(5));

    println!();
    println!("Fin du test IQ.");

    sdrplay.disconnect();

    Ok(())
}
