# Testing

## Mock backend

The mock backend simulates an RSP1B so that the gateway can be run and tested on any
machine, with no receiver and no SDRplay library:

- the antenna signal is Gaussian noise (like a DAB multiplex) at a configurable level,
  **-75 dBm** by default;
- total gain = 105.6 dB − LNA attenuation − gRdB (values observed on an RSP1B);
- digital level (dBFS, ±1.0 scale as in the API) = antenna level + total gain;
- the AGC drives the level towards -30 dBFS by changing gRdB;
- above full scale the signal is clipped and an overload is reported;
- it emits the same log lines as the real backend, so the same `grep` filters work.

```bash
cargo build --release
./target/release/sdr-universal --mock --port 2234 --mock-level -70
```

A client can connect, adjust gain and show an RF level, but there is no DAB ensemble
to find: the signal is noise.

**Limits of the model.** LNA attenuations are approximated with the 60–420 MHz table
for every band, so gain behaviour is faithful there but not in the Am and 420–1000 MHz
bands. The mock validates the gateway's logic, not the receiver's RF behaviour.

## Automated tests

```bash
cargo test      # no receiver and no SDRplay API needed
```

They take about 20 seconds.

**Unit tests:** gain tables and band edges, bandwidth rounding, the control-port frame
(including a re-implementation of AbracaDABra's parsing), and the mock's radio model.

**End-to-end tests** (`tests/mock_e2e.rs`) start the gateway with `--mock` and drive it
like AbracaDABra would:

| Test | Checks |
|---|---|
| `header_and_realtime_stream` | `RTL0` header, R820T tuner, 29 gains, real-time data rate |
| `manual_gain_scales_level_and_rf_estimate_is_gain_independent` | Level follows the gain; estimated RF level stays at the antenna level |
| `hardware_agc_converges_and_rf_estimate_is_correct` | AGC settles at its set-point; RF level estimate correct |
| `gain_is_reapplied_when_the_band_changes` | Band change logged and gain re-applied (Vhf, Band3, LBand, Am) |
| `overload_is_reported_then_cleared` | Clipping, overload log and control-port flag, then recovery |
| `client_can_reconnect` | Three successive connections, each with a header and data |
| `invalid_commands_do_not_disturb_the_gateway` | Out-of-range frequency, unknown opcode, out-of-range step: errors logged, stream continues |
| `ctrl_c_stops_cleanly` | SIGINT gives a clean exit with status 0 |
| `bind_option_limits_the_listening_address` | `--bind 127.0.0.1`: address in the log, both ports reachable, no all-interfaces warning |
| `default_bind_warns_about_all_interfaces` | Default start-up warns that all interfaces are exposed |
| `invalid_option_values_are_rejected_at_start_up` | Bad `--bind`, `--port` (not a number, out of range, 65535) and `--mock-level` stop at once with a message naming the option |
| `rates_below_2_msps_use_decimation_and_a_matching_bandwidth` | 1.024 MS/s, 250 kHz and 2.4 MS/s: ADC rate and decimation in the log, matching analog filter, data rate close to the requested rate |
| `unsupported_sample_rates_are_refused_and_the_stream_keeps_its_rate` | 12 MS/s and 10 kHz are refused with a warning; the stream stays at 2.048 MS/s |
| `bandwidth_requests_are_capped_by_the_sample_rate` | A 1.536 MHz request at 1.024 MS/s is capped to 600 kHz, and honoured at 2.048 MS/s |
| `mdns_can_be_disabled_and_is_skipped_on_loopback` | `--no-mdns` and `--bind 127.0.0.1` do not advertise |
| `mdns_failure_is_never_fatal` | The gateway advertises, or only warns, and keeps serving clients |
| `empty_service_name_is_rejected` | `--name` with an empty value stops at start-up |
| `version_flag_reports_the_package_version` | `--version` / `-V` print one line with the package version and target |
| `help_lists_the_options` | `--help` / `-h` list the options |
| `missing_sdrplay_api_fails_cleanly` | Without the SDRplay API: non-zero exit, readable message, no panic (never touches a real RSP) |

The tests catch real regressions: reintroducing an old bug (wrong RF-level scaling, or
the old tuner header) makes the corresponding tests fail.

One more test needs a network interface with multicast, which CI runners and containers do
not always have, so it is ignored by default:

```bash
cargo test --test mock_e2e -- --ignored mdns
```

`mdns_advertisement_is_discoverable` browses for `_rtl_tcp._tcp` like a client would and
checks the port and properties of the advertisement, and that it is withdrawn when the
gateway stops.

**Not covered:** the SDRplay backend itself is not executed by these tests. Hardware
tests are still needed.

## Hardware checklist

Run with a real RSP1B:

- [ ] DAB decoding on band III with the client's hardware AGC, software AGC and manual gain.
- [ ] Manual gain raised step by step: `ADC OVERLOAD` appears before decoding is lost.
- [ ] Channel changes inside band III: no `ERROR` or `WARN` lines.
- [ ] Crossing bands (FM ↔ DAB ↔ UHF ↔ L-band): `Band change` logged and gain re-applied.
- [ ] Client disconnect and reconnect without restarting the gateway.
- [ ] Sample rates 1.024, 1.4 and 2.4 MS/s with a real client: spectrum shows no aliasing and the log reports the expected decimation.
- [ ] Rate change while streaming: the stream resumes at the new rate (no stall; no `sample rate change not confirmed` warning).
- [ ] NyxScope (or another mDNS-aware client) lists the gateway without typing an address.
- [ ] Ctrl+C, then an immediate restart: the RSP is released and found again.
- [ ] Unplug and replug the RSP; restart the SDRplay API service.
- [ ] Endurance: one hour on a stable channel (CPU use, no dropped audio).
- [ ] RF level calibration against a signal of known level.
