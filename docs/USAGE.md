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

The log is currently in French. These are the lines to look for:

| Log text | Meaning |
|---|---|
| `>>> GAIN : bande=… pas=…/28 -> LNA=… gRdB=…` | A gain step was applied: band, requested step, LNA state and IF gain reduction, then the values read back. |
| `Changement de bande X -> Y : gain réappliqué` | The frequency crossed a band boundary and the gain was re-applied with the new band's table. |
| `!!! SURCHARGE ADC (n fois)` | The ADC overloaded (at most one message every 2 s). Lower the gain or use the AGC. |
| `AGC RSP1B activé / désactivé` | The hardware AGC was switched. |
| `Bande passante RSP1B réglée à … Hz` | Analog bandwidth applied. |
| `Sample rate RSP1B réglé à … Hz` | Sample rate applied. |
| `>>> Bias-T`, `>>> RF notch (FM)`, `>>> DAB notch`, `>>> Correction fréquence` | Hardware option changed (logged only when the value changes). |
| `>>> ATTENTION : le notch DAB …` | The DAB notch was enabled while in band III, which degrades DAB reception. |
| `Port de contrôle (niveau RF) en écoute` / `Client de contrôle connecté` | Control port state. |
| `>>> RTL-TCP client connecté` / `<<< RTL-TCP client déconnecté` | rtl_tcp client state. |
| `Erreur`, `échoué` | A command or API call failed. |

Handy filters:

```bash
# Gain, overloads, band changes and errors only
./target/release/sdr-universal 2>&1 | grep --line-buffered -E ">>> GAIN|SURCHARGE|Changement de bande|Erreur|échoué"

# Keep the full log in a file and show only the essentials on screen
./target/release/sdr-universal 2>&1 | tee /tmp/sdr.log | grep --line-buffered -E ">>> GAIN|SURCHARGE|Erreur|échoué"
```

Run with `--verbose` for API events, per-block IQ statistics (`RTL: min=… max=…`
byte range) and raw commands.

## Hardware options

Applied only when the value actually changes, so clients that resend them at every
connection cause no traffic to the receiver:

- **Bias-T** (rtl_tcp `0x0E`) powers the antenna. Enable it only with an antenna that
  accepts DC on the coax.
- **RF notch (FM)** and **DAB notch** exist in the core and backend, but rtl_tcp
  has no standard command for them, so they are not reachable from an RTL client
  yet.
- **Frequency correction** (`0x05`, ppm).
