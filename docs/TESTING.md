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
cargo build --release --no-default-features
./target/release/sdr-universal --mock --port 2234 --mock-level -70
```

A client can connect, adjust gain and show an RF level, but there is no DAB ensemble
to find: the signal is noise.

**Limits of the model.** LNA attenuations are approximated with the 60–420 MHz table
for every band, so gain behaviour is faithful there but not in the Am and 420–1000 MHz
bands. The mock validates the gateway's logic, not the receiver's RF behaviour.

## Automated tests

```bash
cargo test --no-default-features   # without the SDRplay API
cargo test                         # with it (the receiver is not used)
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

The tests catch real regressions: reintroducing an old bug (wrong RF-level scaling, or
the old tuner header) makes the corresponding tests fail.

**Not covered:** the SDRplay backend itself is not executed by these tests. Hardware
tests are still needed.

## Hardware checklist

Run with a real RSP1B:

- [ ] DAB decoding on band III with the client's hardware AGC, software AGC and manual gain.
- [ ] Manual gain raised step by step: `SURCHARGE` appears before decoding is lost.
- [ ] Channel changes inside band III: no `Erreur` or `échoué` lines.
- [ ] Crossing bands (FM ↔ DAB ↔ UHF ↔ L-band): `Changement de bande` logged and gain re-applied.
- [ ] Client disconnect and reconnect without restarting the gateway.
- [ ] Ctrl+C, then an immediate restart: the RSP is released and found again.
- [ ] Unplug and replug the RSP; restart the SDRplay API service.
- [ ] Endurance: one hour on a stable channel (CPU use, no dropped audio).
- [ ] RF level calibration against a signal of known level.
