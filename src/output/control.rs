//! rtl_tcp "control port" server (rtl_tcp port + 1).
//!
//! It tells AbracaDABra the real gain of the RSP and the overload state, which
//! lets that client display an RF level. Without it, AbracaDABra shows
//! "Not available".

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use super::control_frame::{build_frame, client_gain_tenths};
use crate::core::telemetry;

/// Interval between two frames.
const PERIOD: Duration = Duration::from_millis(500);

pub fn start_server(addr: &str) {
    let addr = addr.to_string();

    thread::spawn(move || {
        let listener = match TcpListener::bind(&addr) {
            Ok(listener) => listener,
            Err(e) => {
                log::error!("control port {} unavailable: {}", addr, e);
                return;
            }
        };

        log::info!("control port (RF level) listening on {}", addr);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    log::info!("control client connected");
                    thread::spawn(move || serve_client(stream));
                }
                Err(e) => log::warn!("control port: accept failed: {}", e),
            }
        }
    });
}

fn serve_client(mut stream: TcpStream) {
    let _ = stream.set_nodelay(true);

    loop {
        // No frame until the API has reported a gain: the client would
        // display a wrong level.
        if let Some(total_gain_db) = telemetry::total_gain_db() {
            let frame = build_frame(client_gain_tenths(total_gain_db), telemetry::overload());

            if stream.write_all(&frame).is_err() {
                break;
            }
        }

        thread::sleep(PERIOD);
    }

    log::info!("control client disconnected");
}
