# Protocol reference

## rtl_tcp server (port 1234)

### Header

On connection the server sends 12 bytes, big-endian:

| Bytes | Value |
|---|---|
| 0–3 | `"RTL0"` |
| 4–7 | Tuner type: `5` (R820T) |
| 8–11 | Number of gain steps: `29` |

The IQ stream then follows: unsigned 8-bit `I, Q, I, Q, …`, centred on 128
(see [GAIN.md](GAIN.md) for the scaling).

### Commands

Each command is 5 bytes: one opcode byte and a 32-bit big-endian argument.

| Opcode | Command | Support |
|---|---|---|
| `0x01` | Set frequency (Hz) | Yes. Valid range 1 Hz – 2 GHz; other values are refused with an error in the log. |
| `0x02` | Set sample rate (Hz) | Yes. 62.5 kHz – 10 MS/s; lower rates use hardware decimation; the analog filter follows the rate. See [USAGE.md](USAGE.md#sample-rate-and-bandwidth). |
| `0x03` | Set gain mode (`0` = auto, otherwise manual) | Yes. `0` selects the RSP hardware AGC. |
| `0x04` | Set gain (tenths of dB) | Yes. Converted to one of the 29 steps. |
| `0x05` | Set frequency correction (ppm, signed) | Yes. |
| `0x08` | Set (RTL2832) AGC mode | Accepted and ignored. |
| `0x0D` | Set gain by index (0–28) | Yes (extension used by AbracaDABra). |
| `0x0E` | Set bias-T | Yes. |
| `0x40` | Set bandwidth (Hz) | Yes (extension used by AbracaDABra). Rounded up to a supported width, never above the widest that fits the current rate. |
| others | (IF gain, test mode, direct sampling, offset tuning, crystal frequencies…) | Ignored. |

A refused or invalid command is logged (`Core command failed`) and does not
affect the stream.

## rsp_tcp extended server (`--rsp-port`)

SDRplay's `rsp_tcp` extended mode, as implemented by SDRplay's own server (layout from its
`rsp_tcp_api.h`, cross-checked with the SDroxide client).

### Greeting

The 12-byte `RTL0` greeting is the same as above, followed by a **45-byte `RSP0` block**
(everything big-endian except where noted):

| Bytes | Content |
|---|---|
| 0–3 | `"RSP0"` |
| 4–7 | Version: `1` |
| 8–11 | Capability bits (below) |
| 12–15 | Reserved (0) |
| 16–19 | Hardware version: `6` (RSP1B) |
| 20–23 | Sample format: `1` = unsigned 8-bit, `2` = signed 16-bit |
| 24 | Number of antenna inputs: `1` |
| 25–37 | Name of a third antenna (empty) |
| 38–41 | Frequency limit of the third antenna (0; host byte order) |
| 42 | Number of tuners: `1` |
| 43 | Minimum IF gain reduction: `20` |
| 44 | Maximum IF gain reduction: `59` |

Capability bits announced by an RSP1B: bit 0 bias-T, bit 3 FM broadcast notch, bit 4 DAB
notch, bit 7 hardware AGC (`0x99`).

### Samples

With format `2`, each complex sample is **I then Q, each a signed 16-bit little-endian
integer**: the API's samples, unchanged (±32768 = full scale), 4 bytes per sample. With
format `1` the stream is the same as the rtl_tcp one (unsigned 8-bit).

### Extended commands

All the rtl_tcp commands above still work. These are added (they are unknown commands on the
plain rtl_tcp port):

| Opcode | Command | Argument | RSP1B |
|---|---|---|---|
| `0x1F` | Antenna input | 0 = A, 1 = B, 2 = Hi-Z | Only 0 exists: others ignored |
| `0x20` | LNA state | number | Checked against the current band |
| `0x21` | IF gain reduction | dB, 20–59 | |
| `0x22` | Hardware AGC | 0 or 1 | |
| `0x23` | AGC set-point | dBFS, **signed** 32-bit, -72 to -20 | |
| `0x24` | Notch filters | bit mask: 1 AM, 2 broadcast (FM), 4 DAB, 8 RF | Bits 2 and 4 only; the others are ignored |
| `0x25` | Bias-T | 0 or 1 | |
| `0x26` | Reference clock output | 0 or 1 | No such output: ignored |

Refused values are logged (`Core command failed`) and do not affect the stream.

## Control port (port 1235, i.e. rtl_tcp port + 1)

Read-only. Once a client connects, the server sends a frame **every 500 ms** — but only
after the SDRplay API has reported a gain, so a client never sees an unknown value.

Frame (11 bytes, big-endian):

| Bytes | Content |
|---|---|
| 0–1 | Total frame length, including these two bytes: `0x000B` |
| 2 | `0x00` — gain indication |
| 3–4 | Length of the indication's value: `0x0002` |
| 5–6 | Gain in tenths of dB (signed 16-bit) |
| 7 | `0x86` — overload indication |
| 8–9 | Length of the indication's value: `0x0001` |
| 10 | Overload: `0` or `1` |

AbracaDABra reads a frame only if it is longer than 10 bytes and starts with the
gain indication, so both indications are always sent.

The announced gain is the RSP's **total gain plus about 8.19 dB**, chosen so that
AbracaDABra's formula gives the level in dBFS minus the RSP gain
(see [GAIN.md](GAIN.md#rf-level-dbm-in-abracadabra)).
