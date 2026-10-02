# Usage

## What happens at start-up

The gateway configures the receiver, then waits for rtl_tcp clients:

| Setting | Value |
|---|---|
| Frequency | 100 MHz |
| Sample rate | 2 MS/s |
| Analog bandwidth | 1.536 MHz |
| IF | zero-IF, LO mode auto |
| Gain | step 14 of 28, with the **hardware AGC on** |

A client normally overrides all of this as soon as it connects (frequency,
sample rate, gain mode, gain, bandwidth). The hardware AGC at start-up simply avoids
overloading the receiver before a client takes control.

Stop the gateway with **Ctrl+C**: it stops the stream, releases the RSP and exits
after a short pause (about two seconds) that lets the USB device be released.

## Network

| Port | Purpose |
|---|---|
| `1234` (`--port N`) | rtl_tcp server: header, IQ stream, commands |
| `1235` (`N + 1`) | Control port: real gain and overload state, used for the RF level |

By default both listen on `0.0.0.0` (all interfaces) and have **no authentication**:
anyone who can reach them can retune the receiver, and the gateway prints a warning at
start-up. Limit exposure with `--bind` (or `SDR_BIND`):

```bash
sdr-universal --bind 127.0.0.1        # only clients on this machine
sdr-universal --bind 192.168.1.10     # only through this network interface
```

and/or a firewall. Invalid values for `--bind`, `--port` or `--mock-level` stop the
gateway at start-up with an explicit message instead of being ignored.

The rtl_tcp server serves **one client at a time**; another client that connects
waits until the first one disconnects. A client can disconnect and reconnect at any
time without restarting the gateway. The control port accepts several connections.

## Connecting a client

Any client that speaks rtl_tcp can connect. The header sent on connection announces
an **R820T tuner with 29 gain steps**, which is what lets RTL-SDR clients build a gain
list and send gain commands. See [PROTOCOL.md](PROTOCOL.md) for the supported
commands.

**AbracaDABra:** select the RTL-TCP input, enter the gateway's address and port
`1234`, and enable the control-port option so that it can show the RF level. In
its hardware-AGC mode AbracaDABra does not display an RF level; use its software AGC
or manual gain.

## Sample rate and bandwidth

- Sample rates of **2 MS/s and above** are applied to the RSP when the core accepts
  them (2, 2.048, 4, 6, 8, 10 MS/s). DAB clients use 2.048 MS/s.
- Rates **below 2 MS/s** are produced by a linear resampler from the RSP stream
  (no anti-aliasing filter).
- The analog filter bandwidth requested by the client (`0x40`) is rounded **up** to
  the nearest supported width — 200, 300, 600 or 1536 kHz — and capped at
  1.536 MHz, the width of a DAB ensemble. Wider RSP filters exist but need higher
  sample rates than the gateway uses.

## Reading the log

Log lines go to **stderr** (use `2>&1` to pipe them), one per line, with a UTC
timestamp, the level and the module that logged them:

```
2026-10-02T20:00:22.720Z INFO  backend::sdrplay: Band change Vhf -> Band3: gain re-applied (step 14)
```

| Level | Used for |
|---|---|
| `ERROR` | A failure the gateway cannot work around (API call failed, port unavailable) |
| `WARN` | Something to look at: ADC overload, refused command, exposed network interfaces |
| `INFO` | Default. State changes: gain, band, bandwidth, clients, start-up and shutdown |
| `DEBUG` | `--verbose`. API events, receiver parameters, raw commands, IQ statistics |
| `TRACE` | Per-block and per-command traces (very verbose) |

**Choosing what to show.** `--verbose` (or `SDR_VERBOSE=1`) selects `DEBUG`. The `RUST_LOG`
environment variable overrides it, with a global level and/or per-module levels:

```bash
RUST_LOG=warn sdr-universal                          # warnings and errors only
RUST_LOG=info,backend::sdrplay=debug sdr-universal   # details for the SDRplay backend only
RUST_LOG=output=trace sdr-universal                  # everything from the rtl_tcp / control outputs
```

Modules: `backend::sdrplay`, `backend::mock`, `output::rtltcp`, `output::control`, `core`
and `sdr_universal` (start-up and main loop).

The messages to look for:

| Log text | Level | Meaning |
|---|---|---|
| `GAIN: band=… step=…/28 -> LNA=… gRdB=…` | INFO | A gain step was applied: band, requested step, LNA state and IF gain reduction, then the values read back. |
| `Band change X -> Y: gain re-applied (step N)` | INFO | The frequency crossed a band boundary and the gain was re-applied with the new band's table. |
| `ADC OVERLOAD (n times) …` | WARN | The ADC overloaded (at most one message every 2 s). Lower the gain or use the AGC. |
| `RSP1B AGC enabled` / `disabled` | INFO | The hardware AGC was switched. |
| `RSP1B bandwidth set to … Hz`, `RSP1B sample rate set to … Hz`, `RSP1B frequency set to … Hz` | INFO | Receiver setting applied. |
| `Bias-T: …`, `RF notch (FM): …`, `DAB notch: …`, `Frequency correction: … ppm` | INFO | Hardware option changed (logged only when the value changes). |
| `DAB notch enabled in band III …` | WARN | The DAB notch degrades DAB reception. |
| `control port (RF level) listening on …` / `control client connected` | INFO | Control port state. |
| `RTL-TCP client connected` / `RTL-TCP client disconnected` | INFO | rtl_tcp client state. |
| `Core command failed: …` | WARN | A command from a client was refused (for example a value out of range). |

Handy filters:

```bash
# Gain, overloads, band changes, warnings and errors
sdr-universal 2>&1 | grep --line-buffered -E "GAIN|OVERLOAD|Band change|WARN|ERROR"

# Keep the full log in a file and show only the essentials on screen
sdr-universal 2>&1 | tee /tmp/sdr.log | grep --line-buffered -E "GAIN|OVERLOAD|WARN|ERROR"
```

## Hardware options

Applied only when the value actually changes, so clients that resend them at every
connection cause no traffic to the receiver:

- **Bias-T** (rtl_tcp `0x0E`) powers the antenna. Enable it only with an antenna that
  accepts DC on the coax.
- **RF notch (FM)** and **DAB notch** exist in the core and backend, but rtl_tcp
  has no standard command for them, so they are not reachable from an RTL client
  yet.
- **Frequency correction** (`0x05`, ppm).
