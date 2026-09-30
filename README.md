# SDR Universal

A gateway that exposes a **SDRplay RSP** receiver (currently the **RSP1B**) as an
**rtl_tcp** server. Software written for RTL-SDR dongles — in particular the
DAB/DAB+ receiver [AbracaDABra](https://github.com/KejPi/AbracaDABra) — can then
use the RSP, with correct **gain handling in every band**, an **RF level
indication**, and **bandwidth selection**.

It is written in Rust and talks to the receiver through the official SDRplay API
(v3.x).

```
 RSP1B ──USB──▶ SDRplay API service ──▶ SDR Universal ──▶ rtl_tcp server   (TCP 1234)
                                        │  backend        │  IQ as 8-bit, gain, frequency...
                                        │  core           └─▶ control port  (TCP 1235)
                                        └─ mock backend                     RF level for AbracaDABra
                                           (no hardware, for tests)
```

> **Status.** Tested by its author with an RSP1B and AbracaDABra on DAB band III.
> Other clients and other RSP models have not been tested (see
> [Known limitations](#known-limitations)).

## Features

- **rtl_tcp server** compatible with RTL-SDR clients, including the extensions used
  by AbracaDABra (gain by index `0x0D`, bandwidth `0x40`).
- **Per-band gain control.** A gain step is translated into the RSP's two gain
  controls (*LNA state* and *IF gain reduction*) using a table **for the current
  band**; it is re-applied automatically when the frequency crosses a band
  boundary. See [docs/GAIN.md](docs/GAIN.md).
- **RSP hardware AGC** (client "automatic gain") with the same settings as SDRplay's
  own `rsp_tcp`.
- **RF level in AbracaDABra** through the rtl_tcp control port, using the real gain
  reported by the SDRplay API.
- **ADC overload detection and reporting** (acknowledged as the API requires).
- **Bias-T, FM/DAB notch filters, frequency correction (ppm)** applied to the
  hardware.
- **Mock backend** (`--mock`) and an end-to-end test suite that run without any
  hardware or SDRplay library. See [docs/TESTING.md](docs/TESTING.md).

## Requirements

- Linux.
- An **SDRplay RSP1B** and the **SDRplay API 3.x**, installed with SDRplay's
  installer (`install.sh` from <https://www.sdrplay.com/software/install.sh>). The
  API service (`sdrplay_apiService`) must be running.
- The [Rust toolchain](https://rustup.rs) (edition 2021).

Without a receiver you can still build, run in mock mode and run the tests (see
below).

## Build

```bash
git clone https://github.com/pbranly/sdr-universal.git
cd sdr-universal
cargo build --release
```

The binary is `target/release/sdr-universal`. It links against
`libsdrplay_api` in `/usr/local/lib` (where SDRplay's installer puts it).

To build **without** the SDRplay API (mock backend only):

```bash
cargo build --release --no-default-features
```

## Run

```bash
./target/release/sdr-universal
```

Then point your client at `HOST:1234` as an *RTL-TCP* input. For AbracaDABra:
choose the RTL-TCP input device, enter the address of the machine running the
gateway and port `1234`, and enable its control-port option to get the RF level
(the option name may vary between versions).

A typical, quieter way to watch what matters:

```bash
./target/release/sdr-universal 2>&1 | grep --line-buffered -E ">>> GAIN|SURCHARGE|Changement de bande|Erreur|échoué"
```

Try it without hardware:

```bash
./target/release/sdr-universal --mock --port 2234 --mock-level -70
```

## Command-line options

| Option | Environment variable | Default | Meaning |
|---|---|---|---|
| `--port N` | | `1234` | rtl_tcp port. The control port is `N + 1`. |
| `--verbose`, `-v` | `SDR_VERBOSE=1` | off | Detailed traces (API events, IQ statistics, raw commands). |
| `--mock` | `SDR_MOCK=1` | off | Simulated RSP1B, no hardware. |
| `--mock-level DBM` | `SDR_MOCK_LEVEL_DBM` | `-75` | Simulated antenna level, in dBm (mock only). |

## Documentation

| Document | Contents |
|---|---|
| [docs/USAGE.md](docs/USAGE.md) | Start-up behaviour, ports, clients, reading the log |
| [docs/GAIN.md](docs/GAIN.md) | How gain, AGC, overload and the RF level work |
| [docs/PROTOCOL.md](docs/PROTOCOL.md) | rtl_tcp commands and the control-port frame |
| [docs/TESTING.md](docs/TESTING.md) | Mock backend, automated tests, hardware checklist |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | Common problems and fixes |

## Known limitations

- **RSP1B only.** The first SDRplay device found is used and its model is not
  checked; the gain tables are the RSP1B's. Other models need their own tables.
- **One client at a time.** Extra connections wait until the current client
  disconnects.
- **No authentication and no bind-address option.** Both ports listen on all
  network interfaces. Use a firewall or run it on a trusted network.
- **Sample rate.** The RSP runs at the rate the client asks for when the API accepts
  it (for example 2.048 MS/s for DAB). Rates below 2 MS/s are produced by a simple
  linear resampler without an anti-aliasing filter; treat them as best-effort.
- **Log messages are in French** (the strings to search for are listed in
  [docs/USAGE.md](docs/USAGE.md)).
- **Absolute RF level needs a one-time calibration** (see [docs/GAIN.md](docs/GAIN.md)).

## License

This project is licensed under the **GNU General Public License v3.0 or later**
([GPL-3.0-or-later](https://www.gnu.org/licenses/gpl-3.0.html)).

See the [LICENSE](LICENSE) file for the full text.

The per-band gain tables in `src/backend/gain.rs` are derived from SDRplay's
[`rsp_tcp`](https://github.com/SDRplay/RSPTCPServer) (GPL-3.0). This project
is therefore distributed under a compatible license.

## Credits

- SDRplay `rsp_tcp` — gain tables and AGC settings.
- [AbracaDABra](https://github.com/KejPi/AbracaDABra) and old-dab's rtl_tcp server
  — the bandwidth command and the control-port protocol.
- [SDroxide](https://github.com/dividebysandwich/sdroxide) — used as a reference for
  how a native client drives an RSP through the SDRplay API.
