//! Helpers shared by the end-to-end test files: starting the gateway (with the
//! mock backend or with the real one over a fake SDRplay API), reading its log,
//! and a minimal rtl_tcp / rsp_tcp client.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub static NEXT_PORT: AtomicU16 = AtomicU16::new(0);

/// Simulated antenna level, in dBm.
pub const LEVEL_DBM: f64 = -75.0;

pub struct Gateway {
    pub child: Child,
    pub port: u16,
    /// Port of the optional rsp_tcp server (`--rsp-port {rsp}`).
    pub rsp_port: u16,
    pub logs: Arc<Mutex<Vec<String>>>,
}

impl Gateway {
    pub fn start() -> Gateway {
        Gateway::start_with(&[])
    }

    pub fn start_with(extra_args: &[&str]) -> Gateway {
        Gateway::launch(true, extra_args, &[])
    }

    /// Starts the gateway with the REAL SDRplay backend, which loads `library`
    /// (a fake `libsdrplay_api`) instead of the vendor's.
    pub fn start_real(
        library: &std::path::Path,
        extra_args: &[&str],
        envs: &[(&str, &str)],
    ) -> Gateway {
        let mut all: Vec<(&str, &str)> = vec![("SDRPLAY_API_LIB", library.to_str().unwrap())];
        all.extend_from_slice(envs);
        Gateway::launch(false, extra_args, &all)
    }

    fn launch(mock: bool, extra_args: &[&str], envs: &[(&str, &str)]) -> Gateway {
        let port = 21000
            + (std::process::id() % 1000) as u16 * 20
            + NEXT_PORT.fetch_add(4, Ordering::SeqCst);

        let mut child = Command::new(env!("CARGO_BIN_EXE_sdr-universal"))
            .args(["--port", &port.to_string()])
            .args(if mock {
                vec![
                    "--mock".to_string(),
                    "--mock-level".to_string(),
                    LEVEL_DBM.to_string(),
                ]
            } else {
                Vec::new()
            })
            .envs(envs.iter().copied())
            .args(extra_args.iter().map(|arg| {
                if *arg == "{rsp}" {
                    (port + 2).to_string()
                } else {
                    arg.to_string()
                }
            }))
            // Tests must not advertise services on the real network: only the
            // ones that pass `--name` exercise the mDNS advertisement.
            .args(if extra_args.contains(&"--name") {
                None
            } else {
                Some("--no-mdns")
            })
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

        let gw = Gateway {
            child,
            port,
            rsp_port: port + 2,
            logs,
        };
        assert!(
            gw.wait_log("waiting for RTL-TCP commands", Duration::from_secs(20)),
            "the gateway did not start:\n{}",
            gw.dump()
        );
        gw
    }

    pub fn log_contains(&self, needle: &str) -> bool {
        self.logs.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    pub fn wait_log(&self, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.log_contains(needle) {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Latest log line containing `needle`.
    pub fn last_log(&self, needle: &str) -> Option<String> {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|line| line.contains(needle))
            .cloned()
    }

    pub fn dump(&self) -> String {
        self.logs.lock().unwrap().join("\n")
    }

    pub fn client(&self) -> Client {
        Client::connect(self.port)
    }

    /// Latest frame of the control port: (announced gain in dB, overload).
    pub fn control(&self) -> (f64, bool) {
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

pub fn connect_retry(port: u16) -> TcpStream {
    let start = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return s,
            Err(e) if start.elapsed() > Duration::from_secs(10) => {
                panic!("connecting to port {}: {}", port, e)
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

pub struct Client {
    pub stream: TcpStream,
    pub header: [u8; 12],
    pub window: Arc<Mutex<Vec<u8>>>,
    pub total: Arc<AtomicU64>,
}

impl Client {
    pub fn connect(port: u16) -> Client {
        Client::connect_with_greeting(port, 0).0
    }

    /// Connects to the rsp_tcp server: also returns the 45-byte `RSP0` block.
    pub fn connect_rsp(port: u16) -> (Client, [u8; 45]) {
        let (client, extra) = Client::connect_with_greeting(port, 45);
        (client, extra.try_into().expect("45 bytes"))
    }

    pub fn connect_with_greeting(port: u16, extra_len: usize) -> (Client, Vec<u8>) {
        let mut stream = connect_retry(port);
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();

        let mut header = [0u8; 12];
        stream.read_exact(&mut header).expect("RTL0 header");

        let mut extra = vec![0u8; extra_len];
        stream.read_exact(&mut extra).expect("RSP0 block");

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
                    // Keep whole samples (up to 4 bytes) at the start of the window.
                    let excess = len - 65536;
                    win.drain(..excess - excess % 4);
                }
            }
        });

        (
            Client {
                stream,
                header,
                window,
                total,
            },
            extra,
        )
    }

    pub fn cmd(&mut self, op: u8, value: u32) {
        let mut msg = [0u8; 5];
        msg[0] = op;
        msg[1..].copy_from_slice(&value.to_be_bytes());
        self.stream.write_all(&msg).expect("sending a command");
    }

    pub fn total_bytes(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// RMS of one component, in byte units (centred on 128).
    pub fn rms_bytes(&self) -> f64 {
        let win = self.window.lock().unwrap();
        let tail = &win[win.len().saturating_sub(32768)..];
        let power: f64 = tail
            .iter()
            .map(|&b| (b as f64 - 128.0).powi(2))
            .sum::<f64>()
            / tail.len() as f64;
        power.sqrt()
    }

    /// RMS of one component for signed 16-bit little-endian samples.
    pub fn rms_i16(&self) -> f64 {
        let win = self.window.lock().unwrap();
        let tail = &win[win.len().saturating_sub(32768)..];
        let values: Vec<f64> = tail
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f64)
            .collect();
        (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt()
    }

    pub fn min_max(&self) -> (u8, u8) {
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
pub fn rf_estimate_dbm(rms_bytes: f64, announced_gain_db: f64) -> f64 {
    20.0 * rms_bytes.log10() - announced_gain_db - 46.0
}

pub const SETTLE: Duration = Duration::from_millis(900);

/// Starts the gateway with arguments and waits for it to finish (5 s at most:
/// a command that must fail or stop right away must never stay running).
pub fn run_cli(args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
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

/// Bytes per second received over `window`.
pub fn throughput(client: &Client, window: Duration) -> f64 {
    let before = client.total_bytes();
    let start = Instant::now();
    thread::sleep(window);
    (client.total_bytes() - before) as f64 / start.elapsed().as_secs_f64()
}

/// Decoded `RSP0` capability block.
pub struct RspBlock {
    pub version: u32,
    pub capabilities: u32,
    pub hardware_version: u32,
    pub sample_format: u32,
    pub antennas: u8,
    pub tuners: u8,
    pub ifgr_min: u8,
    pub ifgr_max: u8,
}

pub fn parse_rsp_block(block: &[u8; 45]) -> RspBlock {
    assert_eq!(&block[..4], b"RSP0", "magic of the capability block");
    let be = |i: usize| u32::from_be_bytes([block[i], block[i + 1], block[i + 2], block[i + 3]]);

    RspBlock {
        version: be(4),
        capabilities: be(8),
        hardware_version: be(16),
        sample_format: be(20),
        antennas: block[24],
        tuners: block[42],
        ifgr_min: block[43],
        ifgr_max: block[44],
    }
}

/// Component RMS expected for a digital level, in dBFS, on the API's ±32768 scale.
pub fn rms_for_dbfs(dbfs: f64) -> f64 {
    10f64.powf(dbfs / 20.0) * 32768.0
}

pub fn assert_close(measured: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (measured - expected).abs() <= expected * tolerance,
        "{}: measured {:.0}, expected {:.0} (±{:.0}%)",
        what,
        measured,
        expected,
        tolerance * 100.0
    );
}

/// Index of the first log line containing `needle`.
pub fn log_position(gw: &Gateway, needle: &str) -> Option<usize> {
    gw.logs
        .lock()
        .unwrap()
        .iter()
        .position(|l| l.contains(needle))
}

/// Compiles the fake `libsdrplay_api` (tests/fake_sdrplay_api/lib.rs) once and
/// returns the path of the shared library.
pub fn fake_sdrplay_api() -> std::path::PathBuf {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();

    LIBRARY
        .get_or_init(|| {
            let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join("libfake_sdrplay_api.so");
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_sdrplay_api/lib.rs");

            // The compiler that builds the tests, found next to `cargo`.
            let rustc = std::env::var_os("CARGO")
                .map(|cargo| Path::new(&cargo).with_file_name("rustc"))
                .filter(|path| path.exists())
                .unwrap_or_else(|| PathBuf::from("rustc"));

            let result = Command::new(rustc)
                .args(["--edition", "2021", "-O", "--crate-type", "cdylib"])
                .arg(&source)
                .arg("-o")
                .arg(&output)
                .output()
                .expect("running rustc");

            assert!(
                result.status.success(),
                "the fake SDRplay API did not compile:\n{}",
                String::from_utf8_lossy(&result.stderr)
            );

            output
        })
        .clone()
}

/// A fresh trace file for the fake API.
pub fn fake_trace_path(name: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fake-sdrplay-{}-{}.log",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}
