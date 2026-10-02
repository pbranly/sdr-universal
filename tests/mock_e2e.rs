//! End-to-end tests, without hardware: the gateway is started with `--mock`
//! and driven the way AbracaDABra would (rtl_tcp protocol on the chosen port,
//! control port on port + 1).
//!
//! Run with `cargo test`. The mock backend is always compiled, so this works
//! with or without the `sdrplay` feature, and without the SDRplay API.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_PORT: AtomicU16 = AtomicU16::new(0);

/// Simulated antenna level, in dBm.
const LEVEL_DBM: f64 = -75.0;

struct Gateway {
    child: Child,
    port: u16,
    logs: Arc<Mutex<Vec<String>>>,
}

impl Gateway {
    fn start() -> Gateway {
        Gateway::start_with(&[])
    }

    fn start_with(extra_args: &[&str]) -> Gateway {
        let port = 21000
            + (std::process::id() % 1000) as u16 * 20
            + NEXT_PORT.fetch_add(2, Ordering::SeqCst);

        let mut child = Command::new(env!("CARGO_BIN_EXE_sdr-universal"))
            .args([
                "--mock",
                "--port",
                &port.to_string(),
                "--mock-level",
                &LEVEL_DBM.to_string(),
            ])
            .args(extra_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starting the gateway");

        let logs = Arc::new(Mutex::new(Vec::new()));

        fn collect(stream: impl Read + Send + 'static, logs: Arc<Mutex<Vec<String>>>) {
            thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    logs.lock().unwrap().push(line);
                }
            });
        }

        collect(child.stdout.take().unwrap(), Arc::clone(&logs));
        collect(child.stderr.take().unwrap(), Arc::clone(&logs));

        let gw = Gateway { child, port, logs };
        assert!(
            gw.wait_log("waiting for RTL-TCP commands", Duration::from_secs(20)),
            "the gateway did not start:\n{}",
            gw.dump()
        );
        gw
    }

    fn log_contains(&self, needle: &str) -> bool {
        self.logs.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    fn wait_log(&self, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.log_contains(needle) {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn dump(&self) -> String {
        self.logs.lock().unwrap().join("\n")
    }

    fn client(&self) -> Client {
        Client::connect(self.port)
    }

    /// Latest frame of the control port: (announced gain in dB, overload).
    fn control(&self) -> (f64, bool) {
        let mut s = connect_retry(self.port + 1);
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut frame = [0u8; 11];
        s.read_exact(&mut frame).expect("control frame");
        assert_eq!(((frame[0] as usize) << 8) | frame[1] as usize, 11);
        assert_eq!(&frame[2..5], &[0, 0, 2], "indication de gain");
        assert_eq!(frame[7], 0x86, "indication de surcharge");
        let tenths = i16::from_be_bytes([frame[5], frame[6]]);
        (tenths as f64 / 10.0, frame[10] != 0)
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn connect_retry(port: u16) -> TcpStream {
    let start = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return s,
            Err(e) if start.elapsed() > Duration::from_secs(10) => {
                panic!("connexion au port {} : {}", port, e)
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

struct Client {
    stream: TcpStream,
    header: [u8; 12],
    window: Arc<Mutex<Vec<u8>>>,
    total: Arc<AtomicU64>,
}

impl Client {
    fn connect(port: u16) -> Client {
        let mut stream = connect_retry(port);
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();

        let mut header = [0u8; 12];
        stream.read_exact(&mut header).expect("RTL0 header");

        let window = Arc::new(Mutex::new(Vec::new()));
        let total = Arc::new(AtomicU64::new(0));

        let mut reader = stream.try_clone().unwrap();
        let (w, t) = (Arc::clone(&window), Arc::clone(&total));

        thread::spawn(move || {
            let mut buf = vec![0u8; 65536];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                t.fetch_add(n as u64, Ordering::Relaxed);
                let mut win = w.lock().unwrap();
                win.extend_from_slice(&buf[..n]);
                let len = win.len();
                if len > 65536 {
                    win.drain(..len - 65536);
                }
            }
        });

        Client {
            stream,
            header,
            window,
            total,
        }
    }

    fn cmd(&mut self, op: u8, value: u32) {
        let mut msg = [0u8; 5];
        msg[0] = op;
        msg[1..].copy_from_slice(&value.to_be_bytes());
        self.stream.write_all(&msg).expect("envoi de commande");
    }

    fn total_bytes(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// RMS of one component, in byte units (centred on 128).
    fn rms_bytes(&self) -> f64 {
        let win = self.window.lock().unwrap();
        let tail = &win[win.len().saturating_sub(32768)..];
        let power: f64 = tail
            .iter()
            .map(|&b| (b as f64 - 128.0).powi(2))
            .sum::<f64>()
            / tail.len() as f64;
        power.sqrt()
    }

    fn min_max(&self) -> (u8, u8) {
        let win = self.window.lock().unwrap();
        let tail = &win[win.len().saturating_sub(32768)..];
        (*tail.iter().min().unwrap(), *tail.iter().max().unwrap())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// RF level estimate as AbracaDABra computes it:
/// 20·log10(level in bytes) - announced gain - 46 (user offset of zero).
fn rf_estimate_dbm(rms_bytes: f64, announced_gain_db: f64) -> f64 {
    20.0 * rms_bytes.log10() - announced_gain_db - 46.0
}

const SETTLE: Duration = Duration::from_millis(900);

#[test]
fn header_and_realtime_stream() {
    let gw = Gateway::start();
    let client = gw.client();

    assert_eq!(&client.header[0..4], b"RTL0");
    assert_eq!(
        u32::from_be_bytes(client.header[4..8].try_into().unwrap()),
        5,
        "type de tuner R820T"
    );
    assert_eq!(
        u32::from_be_bytes(client.header[8..12].try_into().unwrap()),
        29,
        "nombre de gains"
    );

    thread::sleep(Duration::from_millis(1500));
    // 2 MS/s x 2 bytes = 4 MB/s in real time.
    let received = client.total_bytes();
    assert!(
        received > 3_000_000,
        "seulement {} octets en 1,5 s",
        received
    );
    assert!(
        received < 12_000_000,
        "flux trop rapide : {} octets",
        received
    );
}

#[test]
fn manual_gain_scales_level_and_rf_estimate_is_gain_independent() {
    let gw = Gateway::start();
    let mut client = gw.client();

    client.cmd(0x03, 1); // manual gain
    client.cmd(0x01, 195_936_000); // DAB 8A
    client.cmd(0x0D, 14);
    thread::sleep(SETTLE);

    let (rms_low, (g_low, _)) = (client.rms_bytes(), gw.control());

    client.cmd(0x0D, 18);
    thread::sleep(SETTLE);
    let (rms_high, (g_high, overload)) = (client.rms_bytes(), gw.control());

    assert!(!overload, "no overload expected at step 18");
    assert!(
        g_high > g_low,
        "the announced gain must rise: {} -> {}",
        g_low,
        g_high
    );
    assert!(
        rms_high > rms_low * 1.5,
        "the level must follow the gain: {} -> {}",
        rms_low,
        rms_high
    );

    // Two different gains, same estimated antenna level.
    for (rms, gain) in [(rms_low, g_low), (rms_high, g_high)] {
        let estimate = rf_estimate_dbm(rms, gain);
        assert!(
            (estimate - LEVEL_DBM).abs() < 2.0,
            "estimated RF level {:.1} dBm instead of {} (rms={:.2}, announced gain={:.1})",
            estimate,
            LEVEL_DBM,
            rms,
            gain
        );
    }
}

#[test]
fn hardware_agc_converges_and_rf_estimate_is_correct() {
    let gw = Gateway::start();
    let client = gw.client();

    // The gateway starts with the hardware AGC: let it converge.
    thread::sleep(Duration::from_millis(2500));

    let (gain, overload) = gw.control();
    assert!(!overload);

    // Set-point -30 dBFS -> component of about 0.032 -> 16 in byte units.
    let rms = client.rms_bytes();
    assert!((rms - 16.2).abs() < 4.0, "niveau AGC inattendu : {}", rms);

    let estimate = rf_estimate_dbm(rms, gain);
    assert!(
        (estimate - LEVEL_DBM).abs() < 2.0,
        "estimated RF level {:.1} dBm",
        estimate
    );
}

#[test]
fn gain_is_reapplied_when_the_band_changes() {
    let gw = Gateway::start();
    let mut client = gw.client();

    client.cmd(0x03, 1);
    client.cmd(0x0D, 14);
    client.cmd(0x01, 195_936_000);
    assert!(
        gw.wait_log("Band change Vhf -> Band3", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    client.cmd(0x01, 1_090_000_000);
    assert!(
        gw.wait_log("Band change Band3 -> LBand", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.wait_log("band=LBand step=14/28", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    client.cmd(0x01, 30_000_000);
    assert!(
        gw.wait_log("Band change LBand -> Am", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.wait_log("band=Am step=14/28", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    assert!(!gw.log_contains(" ERROR "), "{}", gw.dump());
    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn overload_is_reported_then_cleared() {
    let gw = Gateway::start();
    let mut client = gw.client();

    client.cmd(0x03, 1);
    client.cmd(0x01, 195_936_000);
    client.cmd(0x0D, 28); // maximum gain: the signal saturates
    thread::sleep(SETTLE);

    let (min, max) = client.min_max();
    assert!(
        min == 0 && max == 255,
        "clipping expected, bytes {}..{}",
        min,
        max
    );
    assert!(gw.control().1, "the control port must report the overload");
    assert!(
        gw.wait_log("ADC OVERLOAD", Duration::from_secs(6)),
        "{}",
        gw.dump()
    );

    client.cmd(0x0D, 8);
    thread::sleep(SETTLE);
    assert!(!gw.control().1, "the overload must clear at a lower gain");
}

#[test]
fn client_can_reconnect() {
    let gw = Gateway::start();

    for round in 1..=3 {
        let client = gw.client();
        assert_eq!(&client.header[0..4], b"RTL0", "connexion {}", round);
        thread::sleep(Duration::from_millis(1000));
        assert!(
            client.total_bytes() > 1_000_000,
            "connexion {} : {} octets",
            round,
            client.total_bytes()
        );
        drop(client);
        thread::sleep(Duration::from_millis(400));
    }
}

#[test]
fn invalid_commands_do_not_disturb_the_gateway() {
    let gw = Gateway::start();
    let mut client = gw.client();

    client.cmd(0x01, 3_000_000_000); // beyond 2 GHz
    client.cmd(0x99, 1); // unknown opcode
    client.cmd(0x40, 999_999_999); // bandwidth: rounded to the maximum
    client.cmd(0x0D, 400); // gain step out of range
    client.cmd(0x03, 1);
    client.cmd(0x0D, 14);

    assert!(
        gw.wait_log("Core command failed", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.wait_log("step=14/28", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    let before = client.total_bytes();
    thread::sleep(Duration::from_millis(1000));
    assert!(
        client.total_bytes() > before + 2_000_000,
        "the stream must keep going"
    );
}

#[test]
fn ctrl_c_stops_cleanly() {
    let mut gw = Gateway::start();
    let _client = gw.client();
    thread::sleep(Duration::from_millis(500));

    let status = Command::new("kill")
        .args(["-INT", &gw.child.id().to_string()])
        .status()
        .expect("kill");
    assert!(status.success());

    let start = Instant::now();
    let exit = loop {
        if let Some(status) = gw.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the gateway does not stop:\n{}",
            gw.dump()
        );
        thread::sleep(Duration::from_millis(100));
    };

    assert!(exit.success(), "exit code {:?}\n{}", exit.code(), gw.dump());
    assert!(
        gw.wait_log("clean shutdown", Duration::from_secs(1)),
        "{}",
        gw.dump()
    );
}

/// Starts the gateway with arguments and waits for it to finish (5 s at most:
/// a command that must fail or stop right away must never stay running).
fn run_cli(args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sdr-universal"));
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }

    let mut child = command.spawn().expect("starting the gateway");
    let start = Instant::now();

    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the command {:?} did not stop by itself", args);
        }
        thread::sleep(Duration::from_millis(50));
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();

    std::process::Output {
        status,
        stdout,
        stderr,
    }
}

#[test]
fn version_flag_reports_the_package_version() {
    for flag in ["--version", "-V"] {
        let output = run_cli(&[flag], &[]);
        assert!(output.status.success(), "{}", flag);

        let text = String::from_utf8_lossy(&output.stdout);
        let expected = format!("sdr-universal {} (git ", env!("CARGO_PKG_VERSION"));
        assert!(text.starts_with(&expected), "sortie inattendue : {}", text);
        assert_eq!(
            text.trim_end().lines().count(),
            1,
            "a single line is expected"
        );
        assert!(
            text.contains("-linux-"),
            "the target must appear in the version: {}",
            text
        );
    }
}

#[test]
fn help_lists_the_options() {
    for flag in ["--help", "-h"] {
        let output = run_cli(&[flag], &[]);
        assert!(output.status.success(), "{}", flag);

        let text = String::from_utf8_lossy(&output.stdout);
        for needle in [
            "--port",
            "--mock",
            "--mock-level",
            "--verbose",
            "--version",
            "SDRPLAY_API_LIB",
            "RUST_LOG",
            "--bind",
        ] {
            assert!(
                text.contains(needle),
                "{} missing from the help:\n{}",
                needle,
                text
            );
        }
    }
}

/// Without the SDRplay API: immediate failure, non-zero exit code, readable
/// message (and no panic). `SDRPLAY_API_LIB` forces a path that does not exist,
/// so that the test never touches a real RSP, even on a machine where the API
/// is installed.
#[cfg(feature = "sdrplay")]
#[test]
fn missing_sdrplay_api_fails_cleanly() {
    let output = run_cli(
        &[],
        &[("SDRPLAY_API_LIB", "/nonexistent/libsdrplay_api.so")],
    );

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("SDRplay API not found"), "{}", stderr);
    assert!(
        stderr.contains("--mock"),
        "the message must suggest --mock: {}",
        stderr
    );
    assert!(!stderr.contains("panicked"), "{}", stderr);
}

#[test]
fn bind_option_limits_the_listening_address() {
    let gw = Gateway::start_with(&["--bind", "127.0.0.1"]);

    assert!(
        gw.log_contains(&format!("127.0.0.1:{}", gw.port)),
        "listen address missing from the log:\n{}",
        gw.dump()
    );
    assert!(
        !gw.log_contains("all network interfaces"),
        "the warning must only appear for 0.0.0.0:\n{}",
        gw.dump()
    );

    // Both ports stay reachable on the loopback interface.
    let client = gw.client();
    assert_eq!(&client.header[0..4], b"RTL0");
    let _ = gw.control();
}

#[test]
fn default_bind_warns_about_all_interfaces() {
    let gw = Gateway::start();
    assert!(gw.log_contains("all network interfaces"), "{}", gw.dump());
}

#[test]
fn invalid_option_values_are_rejected_at_start_up() {
    let cases: [(&[&str], &str); 5] = [
        (&["--mock", "--bind", "not-an-ip"], "--bind"),
        (&["--mock", "--port", "abc"], "--port"),
        (&["--mock", "--port", "70000"], "--port"),
        (&["--mock", "--port", "65535"], "65535"),
        (&["--mock", "--mock-level", "abc"], "--mock-level"),
    ];

    for (args, expected) in cases {
        let output = run_cli(args, &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success(), "{:?} should have failed", args);
        assert!(stderr.contains(expected), "{:?} : {}", args, stderr);
        assert!(!stderr.contains("panicked"), "{:?} : {}", args, stderr);
    }
}
