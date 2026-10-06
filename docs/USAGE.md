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
| `--rsp-port N` | Optional rsp_tcp extended server (16-bit samples, RSP controls), see below |
| UDP `5353` (multicast) | mDNS advertisement, see below |

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

## Automatic discovery (mDNS)

Unless disabled, the gateway advertises the rtl_tcp server on the local network as a
`_rtl_tcp._tcp` service (mDNS / DNS-SD, the Bonjour / Avahi mechanism). Clients that
browse for rtl_tcp servers, such as NyxScope, list it automatically; the others still
connect by address. The advertisement carries the host name, the port and three
properties: `software`, `version` and `backend` (`sdrplay` or `mock`).

- The name shown to clients is `SDR Universal on <host>`; change it with `--name`.
- It is **not** advertised when the server listens on the loopback interface only
  (`--bind 127.0.0.1`), nor with `--no-mdns` (or `SDR_MDNS=0`).
- The service is withdrawn when the gateway stops with Ctrl+C.
- Advertising is best effort: if it cannot start (no multicast on the network
  interface, for instance) the gateway logs `mDNS advertisement unavailable` and keeps
  working.
- Discovery uses UDP multicast on port 5353: allow it in the firewall if clients do not
  see the server. This only helps discovery: connecting still needs the TCP ports.
- Advertising makes the server easier to find on the network, and it has no
  authentication (see above): disable it on networks you do not trust.

## Connecting a client

Any client that speaks rtl_tcp can connect. The header sent on connection announces
an **R820T tuner with 29 gain steps**, which is what lets RTL-SDR clients build a gain
list and send gain commands. See [PROTOCOL.md](PROTOCOL.md) for the supported
commands.

**AbracaDABra:** select the RTL-TCP input, enter the gateway's address and port
`1234`, and enable the control-port option so that it can show the RF level. In
its hardware-AGC mode AbracaDABra does not display an RF level; use its software AGC
or manual gain.

## rsp_tcp extended server (16-bit samples and native controls)

rtl_tcp was designed for RTL-SDR dongles: 8-bit samples and a gain list. An RSP samples at
12 to 14 bits and has its own controls, so SDRplay's `rsp_tcp` server adds an **extended
mode** to rtl_tcp, which clients such as **SDroxide** understand. Start the gateway with
a second port to offer it:

```bash
sdr-universal --rsp-port 1236          # 16-bit samples (default)
sdr-universal --rsp-port 1236 --rsp-bits 8
```

Then connect the client to the gateway's address and port `1236`. SDroxide uses the same
network source as for rtl_tcp servers: it recognises the extended block the server sends
and switches to the RSP's own controls and 16-bit samples. (Without the extended block, a
client has no way to know the samples are 16-bit and reads them as noise: that is why this
is a separate port.) Clients that only speak plain rtl_tcp keep using port `1234`; both
servers can run at the same time.

What the extended server adds:

| | rtl_tcp port (`1234`) | rsp_tcp extended port (`--rsp-port`) |
|---|---|---|
| Greeting | `RTL0` header | `RTL0` header, then a 45-byte `RSP0` capability block |
| Samples | unsigned 8-bit | signed 16-bit (default) or 8-bit |
| Gain | 29 steps (R820T emulation) | the same, **and** LNA state and IF gain reduction set directly |
| AGC | on / off | on / off, **and** its set-point (-72 to -20 dBFS) |
| Filters | none | FM broadcast notch and DAB notch |
| Other | bias-T, bandwidth | bias-T, bandwidth |

Things to know:

- **Data rate.** 16-bit samples are 4 bytes per complex sample: **8 MB/s at 2 MS/s**
  (64 Mbit/s), twice the 8-bit rate. Prefer a wired network for the highest rates.
- **One client at a time, across both servers.** The receiver has one frequency, gain and
  sample rate. If a client connects to one server while another is active on the other, it
  waits (connected, without a greeting) and is served when the active one leaves. The log
  shows `waiting for the active session to end`.
- **Slow clients.** Each client has a bounded queue. A client that stops reading loses IQ
  blocks (`client too slow: dropping IQ blocks`) instead of making the gateway use more
  and more memory.
- **Direct gain controls** are checked against the **current band**: an LNA state that
  does not exist there (for instance 9 in the L-band, whose highest is 8) is refused with
  a warning. See [GAIN.md](GAIN.md#direct-gain-controls-rsp_tcp-extended).
- An **RSP1B** has one antenna input, no reference-clock output and no AM or RF notch:
  the matching commands are accepted and ignored.
- The extended server is **not** advertised by mDNS (there is no standard service type for
  it): enter its address and port by hand.
- The extended opcodes are only decoded on this port; on the plain rtl_tcp port they are
  unknown commands and are ignored, as in the original servers.

## Sample rate and bandwidth

**Sample rate.** Any rate from **62.5 kHz to 10 MS/s** is accepted, which covers what
RTL-SDR clients ask for (225 kHz, 1.024, 1.4, 1.8, 2.048, 2.4, 3.2 MS/s...). The RSP's
ADC runs between 2 and 10 MS/s, so for a lower rate the gateway keeps the ADC at the
nearest power-of-two multiple that is at least 2 MS/s and lets the API's hardware
decimator (2 to 32) divide it down. The log shows the plan:

```
RSP1B sample rate set to 1024000 Hz (ADC 2048000 Hz, decimation 2)
```

This is the same method SDRplay's own `rsp_tcp` uses, and it gives properly filtered
samples. A rate outside the range is refused with a warning (`Core command failed`)
and the stream keeps its current rate.

**Bandwidth.** The analog IF filter has fixed widths: 200, 300, 600 kHz, 1.536, 5, 6, 7
and 8 MHz. When the sample rate changes, the gateway selects **the widest width that
fits inside the new rate** (a wider filter would alias; a narrower one would roll the
band edges off for nothing):

| Sample rate | Filter |
|---|---|
| 2.048 MS/s (DAB), 2.4 MS/s, 3.2 MS/s | 1.536 MHz |
| 1.024 MS/s | 600 kHz |
| 250 kHz | 200 kHz |
| 6 MS/s | 6 MHz |
| 10 MS/s | 8 MHz |

A client that wants another width sends it **after** the rate (rtl_tcp command `0x40`,
sent by recent AbracaDABra versions). The request is rounded **up** to a supported
width, but never above the widest one that fits the current rate.

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
| `mDNS: advertising '…'` / `mDNS advertisement unavailable: …` | INFO / WARN | Discovery state. |
| `RTL-TCP client connected` / `disconnected`, `RSP-TCP client connected` / `disconnected` | INFO | Client state, per server. |
| `RSP-TCP extended server listening on … (16-bit samples)` | INFO | The rsp_tcp server started. |
| `… client connected: waiting for the active session to end` | INFO | Another client is active: this one waits. |
| `client too slow: dropping IQ blocks (n so far)` | WARN | A client does not read fast enough; its oldest blocks are dropped. |
| `GAIN: band=… direct LNA=… gRdB=…` | INFO | LNA state or IF gain reduction set directly (rsp_tcp extended). |
| `RSP1B AGC set-point: … dBFS` | INFO | AGC set-point changed (rsp_tcp extended). |
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
- **RF notch (FM)** and **DAB notch**: rtl_tcp has no command for them, but the
  [rsp_tcp extended server](#rsp_tcp-extended-server-16-bit-samples-and-native-controls)
  does (`0x24`). Do **not** enable the DAB notch when receiving DAB: it attenuates band III.
- **Frequency correction** (`0x05`, ppm).
