//! Serveur du « port de contrôle » rtl_tcp (port rtl_tcp + 1).
//!
//! Il annonce à AbracaDABra le gain réel du RSP et l'état de surcharge, ce qui
//! permet à ce client d'afficher un niveau RF. Sans lui, AbracaDABra affiche
//! « Non disponible ».

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use super::control_frame::{build_frame, client_gain_tenths};
use crate::core::telemetry;

/// Intervalle entre deux trames.
const PERIOD: Duration = Duration::from_millis(500);

pub fn start_server(addr: &str) {
    let addr = addr.to_string();

    thread::spawn(move || {
        let listener = match TcpListener::bind(&addr) {
            Ok(listener) => listener,
            Err(e) => {
                println!(">>> Port de contrôle {} indisponible : {}", addr, e);
                return;
            }
        };

        println!(">>> Port de contrôle (niveau RF) en écoute sur {}", addr);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    println!(">>> Client de contrôle connecté");
                    thread::spawn(move || serve_client(stream));
                }
                Err(e) => println!(">>> Port de contrôle : accept a échoué : {}", e),
            }
        }
    });
}

fn serve_client(mut stream: TcpStream) {
    let _ = stream.set_nodelay(true);

    loop {
        // Pas de trame tant que l'API n'a signalé aucun gain : le client
        // afficherait un niveau faux.
        if let Some(total_gain_db) = telemetry::total_gain_db() {
            let frame = build_frame(
                client_gain_tenths(total_gain_db),
                telemetry::overload(),
            );

            if stream.write_all(&frame).is_err() {
                break;
            }
        }

        thread::sleep(PERIOD);
    }

    println!(">>> Client de contrôle déconnecté");
}
