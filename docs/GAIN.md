# Gain, AGC, overload and RF level

## Why gain is not a single number on an RSP

An RSP is controlled with two settings, not one gain in dB:

- **LNA state** — steps of front-end (LNA) attenuation;
- **gRdB** — *gain reduction* of the IF stage, 20 to 59 dB (higher = less gain).

**The meaning of an LNA state depends on the band.** According to the SDRplay API
specification, the number of LNA states for the RSP1B differs per band, and a state
that exists in one band may not exist in another (the service refuses it). A table
that maps "gain in dB" to (LNA state, gRdB) measured at one frequency is therefore
only valid in that band. This gateway keeps one table per band.

## Bands

| Band | Frequency range | Highest LNA state |
|---|---|---|
| Am | below 60 MHz | 6 |
| Vhf | 60 – 120 MHz | 9 |
| Band3 | 120 – 250 MHz (includes DAB band III) | 9 |
| BandX | 250 – 420 MHz | 9 |
| Band45 | 420 – 1000 MHz | 9 |
| LBand | 1000 MHz and above | 8 |

## The 29 gain steps

The gateway exposes **29 steps, 0 to 28**: step 0 is minimum gain and step 28 is
maximum gain (LNA state 0, gRdB 20). Each band has its own table of
(LNA state, gRdB) for the 29 steps, taken from SDRplay's `rsp_tcp`
(`src/backend/gain.rs`).

When the frequency crosses a band boundary the **same step** is re-applied with the
new band's table. The LNA state is also clamped to what the new band supports before
the frequency change is sent, so the service never receives an invalid state.

## From rtl_tcp commands to a step

| Client command | Result |
|---|---|
| `0x0D` set gain **index** (AbracaDABra) | Step used directly (0–28). |
| `0x04` set gain in tenths of dB | Converted to a step with the same quantisation as `rsp_tcp`; the scale is the R820T's 0 – 49.6 dB. |
| `0x03` gain mode `0` | Hardware AGC on. |
| `0x03` gain mode `≠ 0` | Manual gain (AGC off; the last requested step is restored). |
| `0x08` RTL2832 digital AGC | Accepted and **ignored**: it has no RSP equivalent, and would otherwise fight the mode set with `0x03`. |

The gain announced by a client is the R820T's nominal gain, not the RSP's. The **real**
RSP gain is published on the control port.

## Direct gain controls (rsp_tcp extended)

Clients of the [rsp_tcp extended server](USAGE.md#rsp_tcp-extended-server-16-bit-samples-and-native-controls)
do not have to go through the 29-step scale: they set the RSP's own controls.

| Command | Control | Range |
|---|---|---|
| `0x20` | LNA state | 0 to the band's highest state (see [Bands](#bands)) |
| `0x21` | IF gain reduction (gRdB), in dB | 20 to 59 |
| `0x22` | Hardware AGC on / off | |
| `0x23` | AGC set-point, in dBFS | -72 to -20 |

Values outside these ranges are refused with a warning and nothing changes. The LNA state
is checked against the **current band** at the time of the command, since the same state
does not exist in every band. Setting one of the two gain values keeps the other as it is.
When the AGC is on, an LNA state change re-applies the AGC configuration (the AGC controls
gRdB only), as `rsp_tcp` does. The control port reports the resulting total gain, so RF
level estimates stay correct.

## Hardware AGC

Used with the same settings as `rsp_tcp`: control scheme *CTRL_EN* (slow, 500 ms
attack and decay, 200 ms decay delay), set-point **-30 dBFS** (changeable with `0x23` on
the rsp_tcp extended server). The AGC moves gRdB only;
the LNA state stays where it was set. This slow loop suits a wide-band signal such as
a DAB ensemble.

## Signal level sent to the client

IQ samples arrive from the API as 16-bit values normalised to ±1.0. They are sent to
the client as unsigned 8-bit values centred on 128 with a **fixed factor of 512**
(`byte = 128 + 512 × sample`), the same scaling as `rsp_tcp`. The factor is fixed on
purpose: the level the client sees must follow the receiver's gain, otherwise a
client's software AGC cannot see the effect of its own gain commands.

A sample above ±0.25 therefore clips in the 8-bit output. A client's software AGC
normally lowers the gain until its peaks sit well inside the range.

## Overload

The SDRplay API reports ADC overload events. The gateway acknowledges each one (the
API sends no further overload message until it is acknowledged) and logs
`ADC OVERLOAD` (a warning), at most once every 2 seconds. The overload flag is also sent on the
control port.

**Strong signals need less gain.** If a client's manual gain is raised until the
signal is clipped in the API's 16-bit output, decoding stops. In the author's setup on
DAB channel 8A (195.936 MHz) the raw signal reached full scale at step 15 and the
best range was around steps 9–12; your antenna and location will differ. Use the
AGC, or raise the manual gain until `ADC OVERLOAD` appears and then back off two or three
steps.

## RF level (dBm) in AbracaDABra

AbracaDABra estimates the RF level from the signal level it sees and the gain it is
told about on the control port:

```
RF level (dBm) = 20·log10(level in bytes) − gain − 46 + user offset
```

The gateway announces the **real total gain** of the RSP (from the API's gain callback)
plus a constant `20·log10(512) − 46 ≈ 8.19 dB`, which absorbs its 512 factor. The
client's estimate then equals *"digital level in dBFS minus RSP total gain"*, and it
stays consistent when the gain changes.

The **absolute** value in dBm still needs one constant that depends on the receiver
(the input power that gives full scale at zero gain). Calibrate it once: feed a signal
of known level (a generator or a reference receiver) and adjust AbracaDABra's RF level
offset setting until the reading matches. Until then, read the level as **relative**.
Only the software-AGC and manual-gain modes show an RF level; AbracaDABra shows none
in its hardware-AGC mode.
