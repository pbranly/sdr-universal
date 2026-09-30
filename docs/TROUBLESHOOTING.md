# Troubleshooting

## Build

**`cargo build` finishes instantly and the behaviour has not changed.**
Cargo compares file dates. After copying or extracting new sources whose dates are older
than the binary, force a rebuild:

```bash
find src -name '*.rs' -exec touch {} +
cargo build --release
```

To be sure which version is running, start the gateway with `--verbose` and look at
the first `RTL-TCP INPUT` line once a client is connected: it must contain
`facteur=512.0`. A line containing `peak_hold=` comes from an old build.

**`failed to parse lock file … lock file version 4 requires -Znext-lockfile-bump`.**
Your Cargo is too old for this `Cargo.lock`. Update it (`rustup update`).

**`cannot find -lsdrplay_api` or `libsdrplay_api.so: cannot open shared object file`.**
Install the SDRplay API with SDRplay's installer, or build without it:
`cargo build --release --no-default-features` and run with `--mock`. The build looks for
the library in `/usr/local/lib`.

## Start-up

**The RSP is not found.**
Check that the SDRplay API service (`sdrplay_apiService`) is running. An RSP is normally
used by one application at a time: close SDRuno, SDroxide or any other program that
holds it, then restart the gateway.

**`RTL-TCP bind … échoué` or the control port is unavailable.**
The port is already in use, possibly by another instance of the gateway. Pick another
one with `--port N` (the control port is `N + 1`).

## Reception

**Decoding works with the AGC but stops when the manual gain is raised.**
The signal saturates the ADC or the 16-bit output. Look for `!!! SURCHARGE ADC` in the
log. Lower the gain, or use the AGC. See [GAIN.md](GAIN.md#overload).

**Nothing changes when the client changes the gain.**
Check that the client is receiving the tuner header (`RTL-TCP client connecté`) and
that `>>> GAIN` lines appear when it moves the control. With an empty gain list a
client sends no gain commands at all; this gateway announces 29 steps to avoid that.

**AbracaDABra shows "RF level: not available".**
- Enable the control-port option in its RTL-TCP settings.
- Check that port `1235` (rtl_tcp port + 1) is reachable and not blocked by a firewall;
  the log should show `Client de contrôle connecté`.
- Use its software AGC or manual gain: no level is shown in hardware-AGC mode.

**The RF level is off by a constant amount.**
Expected until calibrated: set the RF level offset in AbracaDABra against a signal of
known level ([GAIN.md](GAIN.md#rf-level-dbm-in-abracadabra)). Variations are correct;
only the absolute reference is unknown.

**An overload appears on the FM band.**
Strong broadcast transmitters can overload the receiver at high gain. Lower the gain.
The RSP1B has an FM notch filter, implemented in the backend but not reachable from
rtl_tcp clients yet. Do **not** enable the DAB notch when receiving DAB: it attenuates
band III.

## Log too long

Filter it: see [USAGE.md](USAGE.md#reading-the-log). `--verbose` is off by default, and
adds a lot of output.
