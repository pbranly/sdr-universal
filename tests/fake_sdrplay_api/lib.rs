//! A fake `libsdrplay_api`, compiled as a shared library by the end-to-end tests
//! and loaded by the gateway through `SDRPLAY_API_LIB`.
//!
//! It exports the functions the gateway calls, keeps the parameter structures
//! as raw byte buffers read and written at the offsets pinned by the real
//! `sdrplay_api*.h` headers (so the gateway's own struct declarations are
//! checked against an independent view), streams synthetic IQ through the
//! callbacks and can simulate the receiver being unplugged, going silent, or
//! coming back. Everything it receives is written, one line per call, to the
//! file named by `FAKE_SDRPLAY_LOG`.
//!
//! Environment:
//!   FAKE_SDRPLAY_LOG               file receiving the trace (appended)
//!   FAKE_SDRPLAY_LEVEL_DBM         antenna level, default -75
//!   FAKE_SDRPLAY_REMOVE_AFTER_MS   remove the device this long after the stream starts
//!   FAKE_SDRPLAY_REPLUG_AFTER_MS   ...and make it available again this long after
//!   FAKE_SDRPLAY_HANG_AFTER_MS     stop calling the stream callback, without any event
//!   FAKE_SDRPLAY_NO_DEVICE         no device at all (GetDevices finds none)

use std::ffi::c_void;
use std::io::Write;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

// Offsets in the API's structures (checked against the real headers).
const DEV_PPM: usize = 0; // f64
const DEV_FS_HZ: usize = 8; // f64
const DEV_RF_NOTCH: usize = 44; // u8
const DEV_DAB_NOTCH: usize = 45; // u8
const RX_BW_TYPE: usize = 0; // c_int
const RX_IF_TYPE: usize = 4; // c_int
const RX_LO_MODE: usize = 8; // c_int
const RX_GR_DB: usize = 12; // c_int
const RX_LNA_STATE: usize = 16; // u8
const RX_MIN_GR: usize = 20; // c_int
const RX_GAIN_CURR: usize = 24; // f32
const RX_GAIN_MAX: usize = 28; // f32
const RX_GAIN_MIN: usize = 32; // f32
const RX_RF_HZ: usize = 40; // f64
const RX_DEC_ENABLE: usize = 74; // u8
const RX_DEC_FACTOR: usize = 75; // u8
const RX_AGC_ENABLE: usize = 80; // c_int
const RX_AGC_SETPOINT: usize = 84; // c_int
const RX_BIAS_T: usize = 104; // u8

const UPDATE_DEV_FS: c_int = 0x0000_0001;
const UPDATE_TUNER_GR: c_int = 0x0000_8000;
const UPDATE_TUNER_FRF: c_int = 0x0002_0000;
const UPDATE_CTRL_DECIMATION: c_int = 0x0080_0000;

const LNA_GR_DB: [f64; 10] = [0.0, 6.0, 12.0, 18.0, 20.0, 26.0, 32.0, 38.0, 57.0, 62.0];

pub type StreamCb =
    unsafe extern "C" fn(*mut i16, *mut i16, *mut StreamParams, u32, u32, *mut c_void);
pub type EventCb = unsafe extern "C" fn(c_int, c_int, *mut EventParams, *mut c_void);

#[repr(C)]
pub struct StreamParams {
    first_sample_num: u32,
    gr_changed: c_int,
    rf_changed: c_int,
    fs_changed: c_int,
    num_samples: u32,
}

#[repr(C)]
pub struct EventParams {
    // gain event: gRdB, lnaGRdB, currGain (16 bytes in total)
    a: u32,
    b: u32,
    curr_gain: f64,
}

#[repr(C)]
pub struct CallbackFns {
    stream_a: Option<StreamCb>,
    stream_b: Option<StreamCb>,
    event: Option<EventCb>,
}

#[repr(C)]
pub struct Device {
    ser_no: [u8; 64],
    hw_ver: u8,
    _pad1: [u8; 3],
    tuner: c_int,
    rsp_duo_mode: c_int,
    valid: u8,
    _pad2: [u8; 3],
    rsp_duo_sample_freq: f64,
    dev: *mut c_void,
}

#[repr(C)]
pub struct DeviceParams {
    dev_params: *mut u8,
    rx_a: *mut u8,
    rx_b: *mut u8,
}

struct Buffers {
    dev: Box<[u8; 64]>,
    rx_a: Box<[u8; 144]>,
    rx_b: Box<[u8; 144]>,
    params: Box<DeviceParams>,
}

// The buffers are only touched under the STATE mutex or by the single stream
// thread; raw pointers handed to the gateway are stable (boxed, never freed).
unsafe impl Send for Buffers {}

struct State {
    buffers: Option<Buffers>,
    callbacks: Option<(Option<StreamCb>, Option<EventCb>, usize)>,
    removed: bool,
    removed_at: Option<Instant>,
    init_at: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State {
    buffers: None,
    callbacks: None,
    removed: false,
    removed_at: None,
    init_at: None,
});
static STREAMING: AtomicBool = AtomicBool::new(false);
static GR_CHANGED: AtomicBool = AtomicBool::new(false);
static RF_CHANGED: AtomicBool = AtomicBool::new(false);
static FS_CHANGED: AtomicBool = AtomicBool::new(false);
static FIRST_SAMPLE: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();

fn env_ms(name: &str) -> Option<Duration> {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
}

fn trace(message: &str) {
    let Ok(path) = std::env::var("FAKE_SDRPLAY_LOG") else {
        return;
    };
    let ms = START.get_or_init(Instant::now).elapsed().as_millis();

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{:>7} ms {}", ms, message);
    }
}

unsafe fn rd<T: Copy>(buffer: *const u8, offset: usize) -> T {
    std::ptr::read_unaligned(buffer.add(offset) as *const T)
}

unsafe fn wr<T: Copy>(buffer: *mut u8, offset: usize, value: T) {
    std::ptr::write_unaligned(buffer.add(offset) as *mut T, value)
}

fn new_buffers() -> Buffers {
    let mut dev = Box::new([0u8; 64]);
    let mut rx_a = Box::new([0u8; 144]);
    let rx_b = Box::new([0u8; 144]);

    unsafe {
        wr(dev.as_mut_ptr(), DEV_FS_HZ, 6_000_000.0f64);
        wr(rx_a.as_mut_ptr(), RX_BW_TYPE, 1536 as c_int);
        wr(rx_a.as_mut_ptr(), RX_LO_MODE, 1 as c_int);
        wr(rx_a.as_mut_ptr(), RX_GR_DB, 50 as c_int);
        wr(rx_a.as_mut_ptr(), RX_MIN_GR, 20 as c_int);
        wr(rx_a.as_mut_ptr(), RX_RF_HZ, 200_000_000.0f64);
        wr(rx_a.as_mut_ptr(), RX_AGC_SETPOINT, -60 as c_int);
    }

    let mut buffers = Buffers {
        dev,
        rx_a,
        rx_b,
        params: Box::new(DeviceParams {
            dev_params: std::ptr::null_mut(),
            rx_a: std::ptr::null_mut(),
            rx_b: std::ptr::null_mut(),
        }),
    };

    buffers.params.dev_params = buffers.dev.as_mut_ptr();
    buffers.params.rx_a = buffers.rx_a.as_mut_ptr();
    buffers.params.rx_b = buffers.rx_b.as_mut_ptr();
    buffers
}

/// The device has been removed and not yet put back.
fn unavailable(state: &State) -> bool {
    if std::env::var("FAKE_SDRPLAY_NO_DEVICE").is_ok() {
        return true;
    }

    if !state.removed {
        return false;
    }

    match (state.removed_at, env_ms("FAKE_SDRPLAY_REPLUG_AFTER_MS")) {
        (Some(at), Some(back)) => at.elapsed() < back,
        _ => true,
    }
}

fn total_gain_db(lna: u8, gr: i32) -> f64 {
    105.6 - LNA_GR_DB[(lna as usize).min(9)] - gr as f64
}

#[no_mangle]
pub extern "C" fn sdrplay_api_Open() -> c_int {
    trace("Open");
    0
}

#[no_mangle]
pub extern "C" fn sdrplay_api_Close() -> c_int {
    trace("Close");
    0
}

#[no_mangle]
pub unsafe extern "C" fn sdrplay_api_ApiVersion(version: *mut f32) -> c_int {
    *version = 3.15;
    0
}

#[no_mangle]
pub extern "C" fn sdrplay_api_LockDeviceApi() -> c_int {
    0
}

#[no_mangle]
pub extern "C" fn sdrplay_api_UnlockDeviceApi() -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn sdrplay_api_GetDevices(
    devices: *mut Device,
    count: *mut u32,
    max: u32,
) -> c_int {
    let state = STATE.lock().unwrap();

    if unavailable(&state) || max == 0 {
        *count = 0;
        trace("GetDevices -> 0 devices");
        return 0;
    }

    let mut device = Device {
        ser_no: [0; 64],
        hw_ver: 6, // RSP1B
        _pad1: [0; 3],
        tuner: 1,
        rsp_duo_mode: 0,
        valid: 1,
        _pad2: [0; 3],
        rsp_duo_sample_freq: 0.0,
        dev: 0x1000 as *mut c_void,
    };
    device.ser_no[..8].copy_from_slice(b"FAKE0001");

    std::ptr::write(devices, device);
    *count = 1;
    trace("GetDevices -> 1 device (RSP1B FAKE0001)");
    0
}

#[no_mangle]
pub unsafe extern "C" fn sdrplay_api_SelectDevice(_device: *mut Device) -> c_int {
    let state = STATE.lock().unwrap();

    if unavailable(&state) {
        trace("SelectDevice -> failed");
        return 1;
    }

    drop(state);
    // A freshly selected device has its default settings.
    STATE.lock().unwrap().buffers = Some(new_buffers());

    trace("SelectDevice");
    0
}

#[no_mangle]
pub extern "C" fn sdrplay_api_ReleaseDevice(_device: *mut c_void) -> c_int {
    trace("ReleaseDevice");
    0
}

#[no_mangle]
pub unsafe extern "C" fn sdrplay_api_GetDeviceParams(
    _dev: *mut c_void,
    out: *mut *mut DeviceParams,
) -> c_int {
    let mut state = STATE.lock().unwrap();

    if unavailable(&state) {
        return 1;
    }

    match state.buffers.as_mut() {
        Some(buffers) => {
            *out = &mut *buffers.params as *mut DeviceParams;
            0
        }
        None => 1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn sdrplay_api_Init(
    _dev: *mut c_void,
    callbacks: *mut CallbackFns,
    context: *mut c_void,
) -> c_int {
    let mut state = STATE.lock().unwrap();

    if unavailable(&state) {
        trace("Init -> failed");
        return 1;
    }

    let fns = &*callbacks;
    state.callbacks = Some((fns.stream_a, fns.event, context as usize));
    state.init_at = Some(Instant::now());
    state.removed = false;
    drop(state);

    trace("Init");

    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    STREAMING.store(true, Ordering::SeqCst);
    GR_CHANGED.store(true, Ordering::SeqCst);
    RF_CHANGED.store(true, Ordering::SeqCst);
    FS_CHANGED.store(true, Ordering::SeqCst);
    FIRST_SAMPLE.store(0, Ordering::SeqCst);

    thread::spawn(move || stream_thread(generation));
    0
}

#[no_mangle]
pub extern "C" fn sdrplay_api_Uninit(_dev: *mut c_void) -> c_int {
    STREAMING.store(false, Ordering::SeqCst);
    GENERATION.fetch_add(1, Ordering::SeqCst);
    STATE.lock().unwrap().callbacks = None;
    trace("Uninit");
    0
}

unsafe fn fire_event(event_id: c_int, a: u32, b: u32, curr_gain: f64) {
    let (event_cb, context) = {
        let state = STATE.lock().unwrap();
        match state.callbacks {
            Some((_, Some(cb), ctx)) => (cb, ctx),
            _ => return,
        }
    };

    let mut params = EventParams { a, b, curr_gain };
    event_cb(event_id, 1, &mut params, context as *mut c_void);
}

unsafe fn announce_gain(rx: *mut u8) {
    let lna: u8 = rd(rx, RX_LNA_STATE);
    let gr: c_int = rd(rx, RX_GR_DB);
    let curr = total_gain_db(lna, gr);

    wr(rx, RX_GAIN_CURR, curr as f32);
    wr(rx, RX_GAIN_MAX, 85.6f32);
    wr(rx, RX_GAIN_MIN, -15.4f32);

    fire_event(0, gr as u32, LNA_GR_DB[(lna as usize).min(9)] as u32, curr);
}

#[no_mangle]
pub unsafe extern "C" fn sdrplay_api_Update(
    _dev: *mut c_void,
    _tuner: c_int,
    reason: c_int,
    _ext1: c_int,
) -> c_int {
    let (dev, rx, removed) = {
        let state = STATE.lock().unwrap();
        match state.buffers.as_ref() {
            Some(b) => (
                b.dev.as_ptr() as *mut u8,
                b.rx_a.as_ptr() as *mut u8,
                unavailable(&state) || state.removed,
            ),
            None => return 1,
        }
    };

    if removed {
        trace(&format!(
            "Update reason={:#x} -> failed (device removed)",
            reason
        ));
        return 1;
    }

    // One line with everything the gateway has set: the tests read it back.
    trace(&format!(
        "Update reason={:#x} fs={:.0} dec={}x{} freq={:.0} bw={} if={} lo={} lna={} gr={} agc={}/{} biasT={} fm_notch={} dab_notch={} ppm={:.3}",
        reason,
        rd::<f64>(dev, DEV_FS_HZ),
        rd::<u8>(rx, RX_DEC_ENABLE),
        rd::<u8>(rx, RX_DEC_FACTOR),
        rd::<f64>(rx, RX_RF_HZ),
        rd::<c_int>(rx, RX_BW_TYPE),
        rd::<c_int>(rx, RX_IF_TYPE),
        rd::<c_int>(rx, RX_LO_MODE),
        rd::<u8>(rx, RX_LNA_STATE),
        rd::<c_int>(rx, RX_GR_DB),
        rd::<c_int>(rx, RX_AGC_ENABLE),
        rd::<c_int>(rx, RX_AGC_SETPOINT),
        rd::<u8>(rx, RX_BIAS_T),
        rd::<u8>(dev, DEV_RF_NOTCH),
        rd::<u8>(dev, DEV_DAB_NOTCH),
        rd::<f64>(dev, DEV_PPM),
    ));

    if reason & UPDATE_TUNER_GR != 0 {
        GR_CHANGED.store(true, Ordering::SeqCst);
        announce_gain(rx);
    }
    if reason & UPDATE_TUNER_FRF != 0 {
        RF_CHANGED.store(true, Ordering::SeqCst);
    }
    if reason & (UPDATE_DEV_FS | UPDATE_CTRL_DECIMATION) != 0 {
        FS_CHANGED.store(true, Ordering::SeqCst);
    }

    0
}

/// Pseudo-random Gaussian-ish noise, cheap and deterministic.
fn noise_table() -> Vec<(f32, f32)> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let unit = |v: u64| ((v >> 11) as f64 + 0.5) / (1u64 << 53) as f64;

    (0..65536)
        .map(|_| {
            let (u1, u2) = (unit(next()), unit(next()));
            let r = (-2.0 * u1.ln()).sqrt();
            let t = 2.0 * std::f64::consts::PI * u2;
            ((r * t.cos()) as f32, (r * t.sin()) as f32)
        })
        .collect()
}

fn stream_thread(generation: u64) {
    let noise = noise_table();
    let block = 4096usize;
    let mut xi = vec![0i16; block];
    let mut xq = vec![0i16; block];
    let mut offset = 0usize;
    let mut next_deadline = Instant::now();

    let level_dbm: f64 = std::env::var("FAKE_SDRPLAY_LEVEL_DBM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(-75.0);

    let started = Instant::now();

    while STREAMING.load(Ordering::SeqCst) && GENERATION.load(Ordering::SeqCst) == generation {
        // Simulated removal.
        if let Some(after) = env_ms("FAKE_SDRPLAY_REMOVE_AFTER_MS") {
            if started.elapsed() >= after {
                {
                    let mut state = STATE.lock().unwrap();
                    state.removed = true;
                    state.removed_at = Some(Instant::now());
                }
                trace("device removed (simulated)");
                unsafe { fire_event(2, 0, 0, 0.0) };
                STREAMING.store(false, Ordering::SeqCst);
                break;
            }
        }

        // Simulated hang: the callback just stops being called.
        if let Some(after) = env_ms("FAKE_SDRPLAY_HANG_AFTER_MS") {
            if started.elapsed() >= after {
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        }

        let (rx, dev, callback, context) = {
            let state = STATE.lock().unwrap();
            let Some(buffers) = state.buffers.as_ref() else {
                break;
            };
            let Some((Some(cb), _, ctx)) = state.callbacks else {
                break;
            };
            (
                buffers.rx_a.as_ptr() as *mut u8,
                buffers.dev.as_ptr() as *mut u8,
                cb,
                ctx,
            )
        };

        let (lna, mut gr, agc_enable, agc_setpoint, adc_hz, decimation, dec_enable) = unsafe {
            (
                rd::<u8>(rx, RX_LNA_STATE),
                rd::<c_int>(rx, RX_GR_DB),
                rd::<c_int>(rx, RX_AGC_ENABLE),
                rd::<c_int>(rx, RX_AGC_SETPOINT),
                rd::<f64>(dev, DEV_FS_HZ),
                rd::<u8>(rx, RX_DEC_FACTOR).max(1) as f64,
                rd::<u8>(rx, RX_DEC_ENABLE),
            )
        };

        // The AGC moves gRdB towards its set-point, one dB per block.
        if agc_enable != 0 {
            let level = level_dbm + total_gain_db(lna, gr);
            let target = agc_setpoint as f64;
            let new_gr = if level > target + 1.0 {
                (gr + 1).min(59)
            } else if level < target - 1.0 {
                (gr - 1).max(20)
            } else {
                gr
            };

            if new_gr != gr {
                gr = new_gr;
                unsafe {
                    wr(rx, RX_GR_DB, gr);
                    announce_gain(rx);
                }
            }
        }

        let rms = 10f64.powf((level_dbm + total_gain_db(lna, gr)) / 20.0) * 32768.0;

        for i in 0..block {
            let (ni, nq) = noise[(offset + i) & 65535];
            xi[i] = (ni as f64 * rms).clamp(-32768.0, 32767.0) as i16;
            xq[i] = (nq as f64 * rms).clamp(-32768.0, 32767.0) as i16;
        }
        offset = (offset + 12_345) & 65535;

        let mut params = StreamParams {
            first_sample_num: FIRST_SAMPLE.fetch_add(block as u64, Ordering::SeqCst) as u32,
            gr_changed: GR_CHANGED.swap(false, Ordering::SeqCst) as c_int,
            rf_changed: RF_CHANGED.swap(false, Ordering::SeqCst) as c_int,
            fs_changed: FS_CHANGED.swap(false, Ordering::SeqCst) as c_int,
            num_samples: block as u32,
        };

        unsafe {
            callback(
                xi.as_mut_ptr(),
                xq.as_mut_ptr(),
                &mut params,
                block as u32,
                0,
                context as *mut c_void,
            );
        }

        let output_hz = if dec_enable != 0 {
            adc_hz / decimation
        } else {
            adc_hz
        };
        next_deadline += Duration::from_secs_f64(block as f64 / output_hz.max(1.0));
        let now = Instant::now();

        if next_deadline > now {
            thread::sleep(next_deadline - now);
        } else if now - next_deadline > Duration::from_millis(200) {
            next_deadline = now;
        }
    }
}
