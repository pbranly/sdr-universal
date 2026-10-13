//! End-to-end tests, without hardware: the gateway is started with `--mock`
//! and driven the way AbracaDABra would (rtl_tcp protocol on the chosen port,
//! control port on port + 1).
//!
//! Run with `cargo test`. The mock backend is always compiled, so this works
//! with or without the `sdrplay` feature, and without the SDRplay API.

mod common;

use common::*;
use std::io::Read;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

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
        "number of gains"
    );

    thread::sleep(Duration::from_millis(1500));
    // 2 MS/s x 2 bytes = 4 MB/s in real time.
    let received = client.total_bytes();
    assert!(received > 3_000_000, "only {} bytes in 1.5 s", received);
    assert!(received < 12_000_000, "stream too fast: {} bytes", received);
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
    assert!((rms - 16.2).abs() < 4.0, "unexpected AGC level: {}", rms);

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
        assert_eq!(&client.header[0..4], b"RTL0", "connection {}", round);
        thread::sleep(Duration::from_millis(1000));
        assert!(
            client.total_bytes() > 1_000_000,
            "connection {}: {} bytes",
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

#[test]
fn version_flag_reports_the_package_version() {
    for flag in ["--version", "-V"] {
        let output = run_cli(&[flag], &[]);
        assert!(output.status.success(), "{}", flag);

        let text = String::from_utf8_lossy(&output.stdout);
        let expected = format!("sdr-universal {} (git ", env!("CARGO_PKG_VERSION"));
        assert!(text.starts_with(&expected), "unexpected output: {}", text);
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
            "--no-mdns",
            "--name",
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
    let cases: [(&[&str], &str); 12] = [
        (&["--mock", "--bind", "not-an-ip"], "--bind"),
        (&["--mock", "--port", "abc"], "--port"),
        (&["--mock", "--port", "70000"], "--port"),
        (&["--mock", "--port", "65535"], "65535"),
        (&["--mock", "--mock-level", "abc"], "--mock-level"),
        (&["--mock", "--rsp-port", "abc"], "--rsp-port"),
        (&["--mock", "--rsp-bits", "12"], "--rsp-bits"),
        (
            &["--mock", "--port", "23456", "--rsp-port", "23456"],
            "--rsp-port",
        ),
        (
            &["--mock", "--port", "23456", "--rsp-port", "23457"],
            "--rsp-port",
        ),
        (
            &["--mock", "--mock-replug-after", "3"],
            "--mock-replug-after",
        ),
        (
            &["--mock", "--mock-unplug-after", "-1"],
            "--mock-unplug-after",
        ),
        (&["--mock-unplug-after", "1"], "--mock-unplug-after"),
    ];

    for (args, expected) in cases {
        let output = run_cli(args, &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success(), "{:?} should have failed", args);
        assert!(stderr.contains(expected), "{:?} : {}", args, stderr);
        assert!(!stderr.contains("panicked"), "{:?} : {}", args, stderr);
    }
}

#[test]
fn rates_below_2_msps_use_decimation_and_a_matching_bandwidth() {
    let gw = Gateway::start();
    let mut client = gw.client();

    // (requested rate, expected ADC rate, decimation, expected bandwidth)
    let cases: [(u32, u32, u32, u32); 3] = [
        (1_024_000, 2_048_000, 2, 600_000),
        (250_000, 2_000_000, 8, 200_000),
        (2_400_000, 2_400_000, 1, 1_536_000),
    ];

    for (rate, adc, decimation, bandwidth) in cases {
        client.cmd(0x02, rate);

        let expected = format!(
            "RSP1B sample rate set to {} Hz (ADC {} Hz, decimation {})",
            rate, adc, decimation
        );
        assert!(
            gw.wait_log(&expected, Duration::from_secs(5)),
            "{}\n{}",
            expected,
            gw.dump()
        );

        thread::sleep(Duration::from_millis(700));
        let line = gw
            .last_log("RSP1B bandwidth set to")
            .expect("bandwidth log");
        assert!(
            line.contains(&format!("set to {} Hz", bandwidth)),
            "rate {}: expected a {} Hz filter, got: {}",
            rate,
            bandwidth,
            line
        );

        // 2 bytes per complex sample, at the requested rate.
        let measured = throughput(&client, Duration::from_millis(1500));
        let expected_rate = rate as f64 * 2.0;
        assert!(
            measured > expected_rate * 0.7 && measured < expected_rate * 1.3,
            "rate {}: {:.0} bytes/s instead of about {:.0}",
            rate,
            measured,
            expected_rate
        );
    }

    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn unsupported_sample_rates_are_refused_and_the_stream_keeps_its_rate() {
    let gw = Gateway::start();
    let mut client = gw.client();

    client.cmd(0x02, 2_048_000);
    assert!(
        gw.wait_log("decimation 1)", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    for rate in [12_000_000u32, 10_000] {
        client.cmd(0x02, rate);
    }

    assert!(
        gw.wait_log("Core command failed", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(500));

    let measured = throughput(&client, Duration::from_millis(1500));
    assert!(
        measured > 2_048_000.0 * 2.0 * 0.7 && measured < 2_048_000.0 * 2.0 * 1.3,
        "the stream must stay at 2.048 MS/s: {:.0} bytes/s",
        measured
    );
}

#[test]
fn bandwidth_requests_are_capped_by_the_sample_rate() {
    let gw = Gateway::start();
    let mut client = gw.client();

    client.cmd(0x02, 1_024_000);
    assert!(
        gw.wait_log("decimation 2)", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(500));

    // A 1.536 MHz filter would alias at 1.024 MS/s: the request is capped.
    client.cmd(0x40, 1_536_000);
    thread::sleep(Duration::from_millis(500));

    let line = gw
        .last_log("RSP1B bandwidth set to")
        .expect("bandwidth log");
    assert!(line.contains("set to 600000 Hz"), "{}", line);

    // At the DAB rate the same request is honoured.
    client.cmd(0x02, 2_048_000);
    thread::sleep(Duration::from_millis(500));
    client.cmd(0x40, 1_530_000);
    thread::sleep(Duration::from_millis(500));

    let line = gw
        .last_log("RSP1B bandwidth set to")
        .expect("bandwidth log");
    assert!(line.contains("set to 1536000 Hz"), "{}", line);
}

#[test]
fn mdns_can_be_disabled_and_is_skipped_on_loopback() {
    let gw = Gateway::start_with(&["--no-mdns"]);
    assert!(
        gw.log_contains("mDNS advertisement disabled"),
        "{}",
        gw.dump()
    );
    assert!(!gw.log_contains("mDNS: advertising"), "{}", gw.dump());
    drop(gw);

    // `--name` keeps mDNS enabled in the test harness, so that the decision
    // taken for the loopback address is what is being tested.
    let gw = Gateway::start_with(&["--bind", "127.0.0.1", "--name", "e2e-loopback"]);
    assert!(
        gw.log_contains("mDNS advertisement skipped"),
        "{}",
        gw.dump()
    );
}

/// Advertising is best effort: on a machine without multicast the gateway only
/// warns, and must keep working.
#[test]
fn mdns_failure_is_never_fatal() {
    let gw = Gateway::start_with(&["--name", "e2e-test"]);

    assert!(
        gw.log_contains("mDNS: advertising 'e2e-test'")
            || gw.log_contains("mDNS advertisement unavailable"),
        "{}",
        gw.dump()
    );

    let client = gw.client();
    assert_eq!(&client.header[0..4], b"RTL0");
}

#[test]
fn empty_service_name_is_rejected() {
    let output = run_cli(&["--mock", "--name", "  "], &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("--name"), "{}", stderr);
}

/// The server is really discoverable by a DNS-SD browser, and withdrawn when it
/// stops. Needs a network interface with multicast, which CI runners and
/// containers do not always have: run it explicitly with
/// `cargo test -- --ignored mdns`.
#[test]
#[ignore = "needs multicast on the local network"]
fn mdns_advertisement_is_discoverable() {
    use mdns_sd::{ServiceDaemon, ServiceEvent};

    let gw = Gateway::start_with(&["--name", "sdr-universal-e2e"]);

    assert!(
        gw.log_contains("mDNS: advertising 'sdr-universal-e2e'"),
        "{}",
        gw.dump()
    );

    let browser = ServiceDaemon::new().expect("mDNS browser");
    let events = browser.browse("_rtl_tcp._tcp.local.").expect("browse");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut found = None;

    while Instant::now() < deadline && found.is_none() {
        if let Ok(ServiceEvent::ServiceResolved(info)) =
            events.recv_timeout(Duration::from_millis(500))
        {
            if info.get_fullname().starts_with("sdr-universal-e2e.") {
                found = Some(info);
            }
        }
    }

    let info = found.expect("the service was not discovered");
    assert_eq!(info.get_port(), gw.port);
    assert_eq!(info.get_property_val_str("software"), Some("sdr-universal"));
    assert_eq!(
        info.get_property_val_str("version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(info.get_property_val_str("backend"), Some("mock"));
    assert!(!info.get_addresses().is_empty());

    // Stopping the gateway withdraws the service.
    let status = Command::new("kill")
        .args(["-INT", &gw.child.id().to_string()])
        .status()
        .expect("kill");
    assert!(status.success());

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut removed = false;

    while Instant::now() < deadline && !removed {
        if let Ok(ServiceEvent::ServiceRemoved(_, name)) =
            events.recv_timeout(Duration::from_millis(500))
        {
            removed = name.starts_with("sdr-universal-e2e.");
        }
    }

    let _ = browser.shutdown();
    assert!(
        removed,
        "the service was not withdrawn when the gateway stopped"
    );
}

#[test]
fn rsp_tcp_server_sends_the_capability_block_and_16_bit_samples() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    assert!(
        gw.log_contains("RSP-TCP extended server listening on"),
        "{}",
        gw.dump()
    );

    let (client, block) = Client::connect_rsp(gw.rsp_port);

    // The 12-byte RTL0 greeting is unchanged.
    assert_eq!(&client.header[0..4], b"RTL0");
    assert_eq!(
        u32::from_be_bytes(client.header[4..8].try_into().unwrap()),
        5
    );
    assert_eq!(
        u32::from_be_bytes(client.header[8..12].try_into().unwrap()),
        29
    );

    let rsp = parse_rsp_block(&block);
    assert_eq!(rsp.version, 1);
    assert_eq!(rsp.hardware_version, 6, "RSP1B");
    assert_eq!(rsp.sample_format, 2, "signed 16-bit");
    // AGC | bias-T | DAB notch | broadcast notch.
    assert_eq!(rsp.capabilities, (1 << 7) | (1 << 0) | (1 << 4) | (1 << 3));
    assert_eq!((rsp.antennas, rsp.tuners), (1, 1));
    assert_eq!((rsp.ifgr_min, rsp.ifgr_max), (20, 59));

    // 4 bytes per complex sample at 2 MS/s.
    let measured = throughput(&client, Duration::from_millis(1500));
    assert!(
        measured > 8_000_000.0 * 0.7 && measured < 8_000_000.0 * 1.3,
        "{:.0} bytes/s instead of about 8 MB/s",
        measured
    );

    // The hardware AGC drives the level to its -30 dBFS set-point.
    thread::sleep(Duration::from_millis(1500));
    assert_close(client.rms_i16(), rms_for_dbfs(-30.0), 0.4, "AGC level");
}

#[test]
fn rsp_tcp_server_can_send_8_bit_samples() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}", "--rsp-bits", "8"]);

    let (client, block) = Client::connect_rsp(gw.rsp_port);
    assert_eq!(parse_rsp_block(&block).sample_format, 1, "unsigned 8-bit");

    let measured = throughput(&client, Duration::from_millis(1500));
    assert!(
        measured > 4_000_000.0 * 0.7 && measured < 4_000_000.0 * 1.3,
        "{:.0} bytes/s instead of about 4 MB/s",
        measured
    );
}

#[test]
fn rsp_tcp_direct_gain_controls_drive_the_receiver() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x01, 195_936_000); // band III
    client.cmd(0x22, 0); // hardware AGC off
    client.cmd(0x20, 5); // LNA state
    client.cmd(0x21, 44); // IF gain reduction

    assert!(
        gw.wait_log("direct LNA=5 gRdB=44", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(1000));

    // Total gain 105.6 - 26 - 44 = 35.6 dB, antenna at -75 dBm: -39.4 dBFS.
    let before = client.rms_i16();
    assert_close(before, rms_for_dbfs(-39.4), 0.2, "level at LNA 5 / gRdB 44");

    // 14 dB less IF gain reduction = 14 dB more signal, a factor of 5.
    client.cmd(0x21, 30);
    assert!(
        gw.wait_log("direct LNA=5 gRdB=30", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(1000));

    let after = client.rms_i16();
    assert_close(after / before, 10f64.powf(14.0 / 20.0), 0.2, "gain ratio");

    // The control port announces the new total gain (35.6 + 14 = 49.6 dB).
    let (announced, _) = gw.control();
    assert!(announced > 49.6, "announced gain {}", announced);

    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn rsp_tcp_agc_set_point_is_honoured() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x22, 1); // hardware AGC on
    client.cmd(0x23, (-40i32) as u32); // set-point, signed
    assert!(
        gw.wait_log("RSP1B AGC set-point: -40 dBFS", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(2500));
    assert_close(
        client.rms_i16(),
        rms_for_dbfs(-40.0),
        0.4,
        "level at -40 dBFS",
    );

    client.cmd(0x23, (-25i32) as u32);
    thread::sleep(Duration::from_millis(2500));
    assert_close(
        client.rms_i16(),
        rms_for_dbfs(-25.0),
        0.4,
        "level at -25 dBFS",
    );
}

#[test]
fn rsp_tcp_notch_bias_t_and_missing_controls() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x24, 2 | 4); // broadcast (FM) and DAB notch
    assert!(
        gw.wait_log("RF notch (FM): enabled", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.wait_log("DAB notch: enabled", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    client.cmd(0x24, 0);
    assert!(
        gw.wait_log("RF notch (FM): disabled", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.wait_log("DAB notch: disabled", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    client.cmd(0x25, 1);
    assert!(
        gw.wait_log("Bias-T: enabled", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );

    // An RSP1B has one antenna and no reference output: ignored, not an error.
    client.cmd(0x1F, 1);
    client.cmd(0x26, 1);
    client.cmd(0x24, 1 | 8); // AM and RF notches do not exist either
    thread::sleep(Duration::from_millis(500));

    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn rsp_tcp_invalid_values_are_refused_and_the_stream_goes_on() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x20, 50); // no such LNA state anywhere
    client.cmd(0x21, 10); // below the 20 dB minimum
    client.cmd(0x23, 0); // set-point above -20 dBFS
    client.cmd(0x23, (-100i32) as u32); // below -72 dBFS

    // LNA state 9 exists in band III but not in L-band (highest: 8).
    client.cmd(0x01, 1_090_000_000);
    assert!(
        gw.wait_log("Band change", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    client.cmd(0x20, 9);

    thread::sleep(Duration::from_millis(1000));

    let failures = gw
        .logs
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.contains("Core command failed"))
        .count();
    assert_eq!(failures, 5, "{}", gw.dump());
    assert!(
        gw.log_contains("does not exist in band LBand"),
        "{}",
        gw.dump()
    );

    let measured = throughput(&client, Duration::from_millis(1000));
    assert!(
        measured > 4_000_000.0,
        "the stream must keep going: {:.0}",
        measured
    );
}

#[test]
fn extended_commands_are_unknown_on_the_plain_rtl_tcp_port() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    let mut client = gw.client();

    client.cmd(0x20, 5);
    client.cmd(0x21, 44);
    client.cmd(0x24, 6);
    thread::sleep(Duration::from_millis(1000));

    assert!(!gw.log_contains("direct LNA"), "{}", gw.dump());
    assert!(!gw.log_contains("RF notch"), "{}", gw.dump());
    assert!(!gw.log_contains("Core command failed"), "{}", gw.dump());
}

#[test]
fn one_client_at_a_time_across_both_servers() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);

    let first = gw.client();
    assert_eq!(&first.header[0..4], b"RTL0");

    // A second client, on the other server, connects but gets nothing yet.
    let mut second = connect_retry(gw.rsp_port);
    second
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .unwrap();

    let mut greeting = [0u8; 12];
    assert!(
        second.read_exact(&mut greeting).is_err(),
        "the second client must wait for the active session"
    );
    assert!(
        gw.log_contains("waiting for the active session to end"),
        "{}",
        gw.dump()
    );

    // As soon as the first client leaves, the second one is served.
    drop(first);
    second
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    second
        .read_exact(&mut greeting)
        .expect("greeting after the first client left");
    assert_eq!(&greeting[0..4], b"RTL0");

    let mut block = [0u8; 45];
    second.read_exact(&mut block).expect("capability block");
    assert_eq!(&block[0..4], b"RSP0");
}

/// A client that stops reading must not make the gateway's memory grow without
/// limit: its queue is bounded and the oldest data is dropped.
#[test]
fn a_stalled_client_cannot_exhaust_the_gateway_memory() {
    let gw = Gateway::start();

    // A raw connection that reads the greeting and then never reads again: the
    // socket buffers fill up, then the gateway's per-client queue.
    let mut raw = connect_retry(gw.port);
    raw.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut header = [0u8; 12];
    raw.read_exact(&mut header).unwrap();

    assert!(
        gw.wait_log(
            "client too slow: dropping IQ blocks",
            Duration::from_secs(25)
        ),
        "{}",
        gw.dump()
    );

    // Resident memory of the gateway stays small.
    let status = std::fs::read_to_string(format!("/proc/{}/status", gw.child.id())).unwrap();
    let rss_kb: u64 = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .expect("VmRSS");
    assert!(rss_kb < 200_000, "gateway memory: {} kB", rss_kb);

    drop(raw);
}

#[test]
fn a_lost_receiver_is_recovered_with_the_settings_clients_chose() {
    // The receiver disappears 6 s after the stream starts and comes back 4 s later.
    let gw = Gateway::start_with(&[
        "--rsp-port",
        "{rsp}",
        "--mock-unplug-after",
        "6",
        "--mock-replug-after",
        "4",
    ]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x01, 195_936_000);
    client.cmd(0x22, 0); // hardware AGC off
    client.cmd(0x20, 5); // LNA state
    client.cmd(0x21, 44); // IF gain reduction
    thread::sleep(Duration::from_millis(1200));

    // Healthy: total gain 35.6 dB, antenna at -75 dBm: -39.4 dBFS.
    let before = client.rms_i16();
    assert_close(before, rms_for_dbfs(-39.4), 0.2, "level before the loss");

    assert!(
        gw.wait_log("receiver lost", Duration::from_secs(15)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.log_contains(" ERROR "),
        "the loss must be an error: {}",
        gw.dump()
    );

    // While the receiver is away the client keeps talking to the gateway: its
    // choices are kept and applied when the receiver is back.
    client.cmd(0x21, 30); // 14 dB more signal
    client.cmd(0x01, 178_352_000);
    thread::sleep(Duration::from_millis(800));

    let silent = throughput(&client, Duration::from_millis(1000));
    assert!(
        silent < 200_000.0,
        "no data expected while lost: {:.0} bytes/s",
        silent
    );
    assert!(
        !gw.log_contains("Core command failed"),
        "commands during the outage are kept, not errors:\n{}",
        gw.dump()
    );

    assert!(
        gw.wait_log("receiver recovered", Duration::from_secs(20)),
        "{}",
        gw.dump()
    );
    assert!(
        gw.log_contains("recovery attempt 1 failed"),
        "the first attempt comes before the replug: {}",
        gw.dump()
    );

    // The settings made DURING the outage were applied after the recovery.
    let back = log_position(&gw, "mock receiver is back").expect("replay start");
    let replayed = gw.logs.lock().unwrap()[back..]
        .iter()
        .any(|line| line.contains("direct LNA=5 gRdB=30"));
    assert!(replayed, "{}", gw.dump());

    thread::sleep(Duration::from_millis(1500));
    let after = client.rms_i16();
    assert_close(after, rms_for_dbfs(-25.4), 0.25, "level after the recovery");

    // The stream is back at full rate, and the control port reports the new gain.
    let flowing = throughput(&client, Duration::from_millis(1000));
    assert!(flowing > 6_000_000.0, "stream back: {:.0} bytes/s", flowing);
    let (announced, _) = gw.control();
    assert!(announced > 49.6, "announced gain {}", announced);
}

#[test]
fn direct_gain_survives_a_band_change_but_a_gain_step_is_re_applied() {
    let gw = Gateway::start_with(&["--rsp-port", "{rsp}"]);
    let (mut client, _) = Client::connect_rsp(gw.rsp_port);

    client.cmd(0x22, 0); // hardware AGC off
    client.cmd(0x01, 100_000_000); // VHF
    client.cmd(0x20, 4);
    client.cmd(0x21, 40);
    assert!(
        gw.wait_log("direct LNA=4 gRdB=40", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(1000));
    let vhf = client.rms_i16();

    // Crossing into band III: the client's own LNA state and IF gain reduction stay.
    client.cmd(0x01, 195_936_000);
    assert!(
        gw.wait_log(
            "Band change Vhf -> Band3: direct gain kept",
            Duration::from_secs(5)
        ),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(1000));
    assert_close(
        client.rms_i16() / vhf,
        1.0,
        0.1,
        "level across the band change",
    );

    // An LNA state that the next band does not have is brought down to its highest.
    client.cmd(0x20, 9);
    thread::sleep(Duration::from_millis(500));
    let lna9 = client.rms_i16();
    client.cmd(0x01, 1_090_000_000); // L-band: highest state 8
    assert!(
        gw.wait_log(
            "Band change Band3 -> LBand: direct gain kept",
            Duration::from_secs(5)
        ),
        "{}",
        gw.dump()
    );
    thread::sleep(Duration::from_millis(1000));
    // State 8 attenuates 5 dB less than state 9 in the mock's LNA table.
    assert_close(
        client.rms_i16() / lna9,
        10f64.powf(5.0 / 20.0),
        0.15,
        "level after the LNA state was brought down",
    );

    // A client that chooses a step of the 29-step scale gets it re-applied.
    client.cmd(0x0D, 14);
    assert!(
        gw.wait_log("step=14/28", Duration::from_secs(5)),
        "{}",
        gw.dump()
    );
    client.cmd(0x01, 195_936_000);
    assert!(
        gw.wait_log(
            "Band change LBand -> Band3: gain re-applied (step 14)",
            Duration::from_secs(5)
        ),
        "{}",
        gw.dump()
    );
}
