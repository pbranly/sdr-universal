//! End-to-end tests of the REAL SDRplay backend, without a receiver: the gateway
//! loads a fake `libsdrplay_api` (tests/fake_sdrplay_api/lib.rs) through
//! `SDRPLAY_API_LIB`.
//!
//! The fake keeps the API's parameter structures as raw bytes at the offsets of
//! the real headers and writes one trace line per call: reading it back checks,
//! independently of the gateway's own declarations, what the gateway really sent
//! to the "receiver". It can also unplug the device, hang the stream, or bring
//! the device back, to test the recovery logic.

#![cfg(all(target_os = "linux", feature = "sdrplay"))]

mod common;

#[path = "../src/backend/gain.rs"]
#[allow(dead_code)]
mod gain;

use common::*;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

fn trace_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Value of `name=` in a trace line (up to the next space).
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{}=", name);
    let start = line.find(&key)? + key.len();
    line[start..].split_whitespace().next()
}

/// The most recent `Update` line of the trace.
fn last_update(path: &Path) -> String {
    trace_lines(path)
        .into_iter()
        .rev()
        .find(|line| line.contains("Update reason="))
        .expect("no Update in the trace")
}

fn wait_trace(path: &Path, needle: &str, timeout: Duration) -> bool {
    let start = Instant::now();

    while start.elapsed() < timeout {
        if trace_lines(path).iter().any(|line| line.contains(needle)) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }

    false
}

fn count(path: &Path, needle: &str) -> usize {
    trace_lines(path)
        .iter()
        .filter(|line| line.contains(needle))
        .count()
}

fn start(name: &str, args: &[&str], envs: &[(&str, &str)]) -> (Gateway, std::path::PathBuf) {
    let trace = fake_trace_path(name);
    let mut all_envs = vec![("FAKE_SDRPLAY_LOG", trace.to_str().unwrap())];
    all_envs.extend_from_slice(envs);

    let mut all_args = vec!["--no-mdns"];
    all_args.extend_from_slice(args);

    (
        Gateway::start_real(&fake_sdrplay_api(), &all_args, &all_envs),
        trace,
    )
}

#[test]
fn the_receiver_is_configured_as_clients_ask() {
    let (gw, trace) = start("configure", &[], &[]);
    let mut client = gw.client();

    client.cmd(0x03, 1); // manual gain
    client.cmd(0x01, 195_936_000); // DAB 8A: band III
    client.cmd(0x02, 1_024_000); // below 2 MS/s: hardware decimation
    client.cmd(0x0D, 18); // gain step
    client.cmd(0x40, 1_530_000); // bandwidth, capped by the rate
    client.cmd(0x0E, 1); // bias-T
    client.cmd(0x05, 5); // 5 ppm

    assert!(
        wait_trace(&trace, "ppm=5.000", Duration::from_secs(10)),
        "{}\n{:?}",
        gw.dump(),
        trace_lines(&trace)
    );
    thread::sleep(Duration::from_millis(500));

    let update = last_update(&trace);
    let (lna, gr) = gain::settings(gain::Band::Band3, 18);

    assert_eq!(field(&update, "freq"), Some("195936000"), "{}", update);
    assert_eq!(
        field(&update, "fs"),
        Some("2048000"),
        "ADC rate: {}",
        update
    );
    assert_eq!(
        field(&update, "dec"),
        Some("1x2"),
        "decimation by 2: {}",
        update
    );
    assert_eq!(
        field(&update, "bw"),
        Some("600"),
        "filter capped by the rate: {}",
        update
    );
    assert_eq!(
        field(&update, "lna"),
        Some(lna.to_string().as_str()),
        "{}",
        update
    );
    assert_eq!(
        field(&update, "gr"),
        Some(gr.to_string().as_str()),
        "{}",
        update
    );
    assert_eq!(
        field(&update, "agc"),
        Some("0/-30"),
        "AGC off in manual mode: {}",
        update
    );
    assert_eq!(field(&update, "biasT"), Some("1"), "{}", update);
    assert_eq!(field(&update, "ppm"), Some("5.000"), "{}", update);

    // The data really comes at the requested rate (2 bytes per complex sample).
    let measured = throughput(&client, Duration::from_millis(1500));
    assert!(
        measured > 2_048_000.0 * 0.7 && measured < 2_048_000.0 * 1.3,
        "{:.0} bytes/s instead of about 2 MB/s",
        measured
    );

    // The log tells the same story.
    assert!(
        gw.log_contains("RSP1B sample rate set to 1024000 Hz (ADC 2048000 Hz, decimation 2)"),
        "{}",
        gw.dump()
    );
    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn rates_from_62_5_khz_to_10_msps_reach_the_receiver() {
    let (gw, trace) = start("rates", &[], &[]);
    let mut client = gw.client();

    // (requested rate, ADC rate, decimation field, filter in kHz)
    let cases = [
        (2_400_000u32, "2400000", "0x", "1536"),
        (250_000, "2000000", "1x8", "200"),
        (62_500, "2000000", "1x32", "200"),
        (6_000_000, "6000000", "0x", "6000"),
    ];

    for (rate, adc, dec, bandwidth) in cases {
        client.cmd(0x02, rate);

        assert!(
            gw.wait_log(
                &format!("RSP1B sample rate set to {} Hz", rate),
                Duration::from_secs(5)
            ),
            "{}",
            gw.dump()
        );
        thread::sleep(Duration::from_millis(700));

        let update = last_update(&trace);
        assert_eq!(field(&update, "fs"), Some(adc), "rate {}: {}", rate, update);
        assert!(
            field(&update, "dec").unwrap().starts_with(dec),
            "rate {}: {}",
            rate,
            update
        );
        assert_eq!(
            field(&update, "bw"),
            Some(bandwidth),
            "rate {}: {}",
            rate,
            update
        );
    }

    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn rsp_tcp_direct_controls_reach_the_receiver() {
    let (gw, trace) = start("direct", &["--rsp-port", "{rsp}"], &[]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x01, 195_936_000);
    client.cmd(0x22, 0); // hardware AGC off
    client.cmd(0x20, 5);
    client.cmd(0x21, 44);

    assert!(
        wait_trace(&trace, "lna=5 gr=44", Duration::from_secs(10)),
        "{:?}",
        trace_lines(&trace)
    );

    // Hardware AGC with its own set-point.
    client.cmd(0x22, 1);
    client.cmd(0x23, (-40i32) as u32);
    assert!(
        wait_trace(&trace, "agc=4/-40", Duration::from_secs(10)),
        "{:?}",
        trace_lines(&trace)
    );

    // Notch filters: broadcast (FM) and DAB, then none.
    client.cmd(0x24, 2 | 4);
    assert!(
        wait_trace(&trace, "fm_notch=1 dab_notch=1", Duration::from_secs(10)),
        "{:?}",
        trace_lines(&trace)
    );
    client.cmd(0x24, 0);
    assert!(
        wait_trace(&trace, "fm_notch=0 dab_notch=0", Duration::from_secs(10)),
        "{:?}",
        trace_lines(&trace)
    );

    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn a_device_removed_event_is_recovered_and_settings_are_restored() {
    // The fake removes the device 4 s after the stream starts and puts it back 3 s later.
    let (gw, trace) = start(
        "removed",
        &[],
        &[
            ("FAKE_SDRPLAY_REMOVE_AFTER_MS", "4000"),
            ("FAKE_SDRPLAY_REPLUG_AFTER_MS", "3000"),
        ],
    );
    let mut client = gw.client();

    client.cmd(0x03, 1);
    client.cmd(0x01, 178_352_000);
    client.cmd(0x0D, 14);
    thread::sleep(Duration::from_millis(1200));

    assert!(
        gw.wait_log("receiver lost", Duration::from_secs(15)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.log_contains("SDRplay API: device removed"),
        "{}",
        gw.dump()
    );

    // The client keeps choosing things while the receiver is away.
    client.cmd(0x01, 100_000_000);

    assert!(
        gw.wait_log("receiver recovered", Duration::from_secs(25)),
        "{}",
        gw.dump()
    );

    // The API session was reopened: one Open for the first start and one for each
    // recovery attempt (the early ones found no device), but a single Init per device.
    assert!(count(&trace, "Open") >= 2, "{:?}", trace_lines(&trace));
    assert_eq!(count(&trace, "Init"), 2, "{:?}", trace_lines(&trace));
    assert!(
        count(&trace, "GetDevices -> 0 devices") >= 1,
        "{:?}",
        trace_lines(&trace)
    );

    // The frequency chosen DURING the outage, and the gain, were applied after it.
    thread::sleep(Duration::from_millis(500));
    let update = last_update(&trace);
    assert_eq!(field(&update, "freq"), Some("100000000"), "{}", update);

    let init_second = trace_lines(&trace)
        .iter()
        .enumerate()
        .filter(|(_, l)| l.ends_with("Init"))
        .nth(1)
        .map(|(i, _)| i)
        .expect("second Init");
    let after: Vec<String> = trace_lines(&trace)[init_second..].to_vec();
    assert!(
        after
            .iter()
            .any(|l| l.contains("Update reason=") && field(l, "bw") == Some("1536")),
        "bandwidth re-applied: {:?}",
        after
    );

    // And the data flows again.
    let flowing = throughput(&client, Duration::from_millis(1000));
    assert!(flowing > 3_000_000.0, "stream back: {:.0} bytes/s", flowing);
}

#[test]
fn a_silent_stream_is_detected_by_the_watchdog() {
    // The fake stops calling the stream callback after 3 s, without any event.
    let (gw, trace) = start("hang", &[], &[("FAKE_SDRPLAY_HANG_AFTER_MS", "3000")]);
    let _client = gw.client();

    assert!(
        gw.wait_log("no IQ data for 5 s", Duration::from_secs(20)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.wait_log("receiver recovered", Duration::from_secs(20)),
        "{}",
        gw.dump()
    );

    // Recovery went through a full close and reopen of the API session.
    assert!(count(&trace, "Uninit") >= 1, "{:?}", trace_lines(&trace));
    assert_eq!(count(&trace, "Init"), 2, "{:?}", trace_lines(&trace));
}

#[test]
fn starting_without_a_device_fails_cleanly() {
    let output = run_cli(
        &["--port", "23999", "--no-mdns"],
        &[
            ("SDRPLAY_API_LIB", fake_sdrplay_api().to_str().unwrap()),
            ("FAKE_SDRPLAY_NO_DEVICE", "1"),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("No SDRplay device detected"), "{}", stderr);
    assert!(!stderr.contains("panicked"), "{}", stderr);
}
