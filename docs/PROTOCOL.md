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
| `0x02` | Set sample rate (Hz) | Yes. See [USAGE.md](USAGE.md#sample-rate-and-bandwidth). |
| `0x03` | Set gain mode (`0` = auto, otherwise manual) | Yes. `0` selects the RSP hardware AGC. |
| `0x04` | Set gain (tenths of dB) | Yes. Converted to one of the 29 steps. |
| `0x05` | Set frequency correction (ppm, signed) | Yes. |
| `0x08` | Set (RTL2832) AGC mode | Accepted and ignored. |
| `0x0D` | Set gain by index (0–28) | Yes (extension used by AbracaDABra). |
| `0x0E` | Set bias-T | Yes. |
| `0x40` | Set bandwidth (Hz) | Yes (extension used by AbracaDABra). Rounded up to 200, 300, 600 or 1536 kHz. |
| others | (IF gain, test mode, direct sampling, offset tuning, crystal frequencies…) | Ignored. |

A refused or invalid command is logged (`Core command failed`) and does not
affect the stream.

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
