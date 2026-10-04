use crate::backend::Backend;
use anyhow::{anyhow, Result};
use libloading::Library;
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::{rates, Event, IfType, IqBlock, IqReblocker, IqSample, LoMode};

use crate::backend::gain;
use gain::Band;
use std::time::Instant;

const MAX_DEVICES: usize = 16;
const MAX_SER_NO_LEN: usize = 64;

static IQ_CALLBACK_RECEIVED: AtomicBool = AtomicBool::new(false);
static IQ_CALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayDevice {
    ser_no: [u8; MAX_SER_NO_LEN],
    hw_ver: u8,
    tuner: c_int,
    rsp_duo_mode: c_int,
    valid: u8,
    rsp_duo_sample_freq: f64,
    dev: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayFsFreq {
    fs_hz: f64,
    sync_update: u8,
    re_cal: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplaySyncUpdate {
    sample_num: u32,
    period: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayResetFlags {
    reset_gain_update: u8,
    reset_rf_update: u8,
    reset_fs_update: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayGainValues {
    curr: f32,
    max: f32,
    min: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayGain {
    gr_db: c_int,
    lna_state: u8,
    sync_update: u8,
    min_gr: c_int,
    gain_vals: SdrplayGainValues,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRfFreq {
    rf_hz: f64,
    sync_update: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayDcOffsetTuner {
    dc_cal: u8,
    speed_up: u8,
    track_time: c_int,
    refresh_rate_time: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayTunerParams {
    bw_type: c_int,
    if_type: c_int,
    lo_mode: c_int,
    gain: SdrplayGain,
    rf_freq: SdrplayRfFreq,
    dc_offset_tuner: SdrplayDcOffsetTuner,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayDcOffset {
    dc_enable: u8,
    iq_enable: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayDecimation {
    enable: u8,
    decimation_factor: u8,
    wide_band_signal: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayAgc {
    enable: c_int,
    set_point_dbfs: c_int,
    attack_ms: u16,
    decay_ms: u16,
    decay_delay_ms: u16,
    decay_threshold_db: u16,
    sync_update: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayControlParams {
    dc_offset: SdrplayDcOffset,
    decimation: SdrplayDecimation,
    agc: SdrplayAgc,
    adsb_mode: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRsp1aParams {
    rf_notch_enable: u8,
    rf_dab_notch_enable: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRsp1aTunerParams {
    bias_t_enable: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRsp2Params {
    ext_ref_output_en: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRsp2TunerParams {
    bias_t_enable: u8,
    am_port_sel: c_int,
    antenna_sel: c_int,
    rf_notch_enable: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRspDuoParams {
    ext_ref_output_en: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRspDuoResetSlaveFlags {
    reset_gain_update: u8,
    reset_rf_update: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRspDuoTunerParams {
    bias_t_enable: u8,
    tuner1_am_port_sel: c_int,
    tuner1_am_notch_enable: u8,
    rf_notch_enable: u8,
    rf_dab_notch_enable: u8,
    reset_slave_flags: SdrplayRspDuoResetSlaveFlags,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRspDxParams {
    hdr_enable: u8,
    bias_t_enable: u8,
    antenna_sel: c_int,
    rf_notch_enable: u8,
    rf_dab_notch_enable: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRspDxTunerParams {
    hdr_bw: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayDevParams {
    ppm: f64,
    fs_freq: SdrplayFsFreq,
    sync_update: SdrplaySyncUpdate,
    reset_flags: SdrplayResetFlags,
    mode: c_int,
    samples_per_pkt: u32,
    rsp1a_params: SdrplayRsp1aParams,
    rsp2_params: SdrplayRsp2Params,
    rsp_duo_params: SdrplayRspDuoParams,
    rsp_dx_params: SdrplayRspDxParams,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRxChannelParams {
    tuner_params: SdrplayTunerParams,
    ctrl_params: SdrplayControlParams,
    rsp1a_tuner_params: SdrplayRsp1aTunerParams,
    rsp2_tuner_params: SdrplayRsp2TunerParams,
    rsp_duo_tuner_params: SdrplayRspDuoTunerParams,
    rsp_dx_tuner_params: SdrplayRspDxTunerParams,
}

#[repr(C)]
struct SdrplayDeviceParams {
    dev_params: *mut SdrplayDevParams,
    rx_channel_a: *mut SdrplayRxChannelParams,
    rx_channel_b: *mut SdrplayRxChannelParams,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayStreamCbParams {
    first_sample_num: u32,
    gr_changed: c_int,
    rf_changed: c_int,
    fs_changed: c_int,
    num_samples: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayGainCbParam {
    gr_db: u32,
    lna_gr_db: u32,
    curr_gain: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayPowerOverloadCbParam {
    power_overload_change_type: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct SdrplayRspDuoModeCbParam {
    mode_change_type: c_int,
}

#[repr(C)]
union SdrplayEventParams {
    gain_params: SdrplayGainCbParam,
    power_overload_params: SdrplayPowerOverloadCbParam,
    rsp_duo_mode_cb_param: SdrplayRspDuoModeCbParam,
}

type SdrplayStreamCallback = unsafe extern "C" fn(
    xi: *mut i16,
    xq: *mut i16,
    params: *mut SdrplayStreamCbParams,
    num_samples: u32,
    reset: u32,
    cb_context: *mut c_void,
);

type SdrplayEventCallback = unsafe extern "C" fn(
    event_id: c_int,
    tuner: c_int,
    params: *mut SdrplayEventParams,
    cb_context: *mut c_void,
);

#[repr(C)]
struct SdrplayCallbackFns {
    stream_a_cb_fn: Option<SdrplayStreamCallback>,
    stream_b_cb_fn: Option<SdrplayStreamCallback>,
    event_cb_fn: Option<SdrplayEventCallback>,
}

struct CallbackContext {
    iq_tx: SyncSender<IqBlock>,
    reblocker: Mutex<IqReblocker>,
    center_frequency_hz: AtomicU64,
    sample_rate: AtomicU64,
    dropped_blocks: AtomicU64,
    lna_gr_db: AtomicU64,
    curr_gain_milli_db: AtomicU64,
    callback_gr_db: AtomicU64,
    iq_enabled: AtomicBool,
    /// ADC overload reported by the service (and not yet "corrected").
    overload: AtomicBool,
    /// The service is waiting for an acknowledgement (required to receive further messages).
    overload_ack_pending: AtomicBool,
    /// Number of overload entries since the last message was logged.
    overload_events: AtomicU64,
    /// Set by the stream callback when the service reports a sample-rate change.
    fs_changed: AtomicBool,
}

const _: () = {
    assert!(std::mem::size_of::<SdrplayDevice>() == 96);

    assert!(std::mem::size_of::<SdrplayDevParams>() == 64);
    assert!(std::mem::size_of::<SdrplayRxChannelParams>() == 144);
    assert!(std::mem::size_of::<SdrplayDeviceParams>() == 24);

    assert!(std::mem::size_of::<SdrplayTunerParams>() == 72);
    assert!(std::mem::size_of::<SdrplayControlParams>() == 32);

    assert!(std::mem::size_of::<SdrplayRsp1aParams>() == 2);
    assert!(std::mem::size_of::<SdrplayRsp1aTunerParams>() == 1);

    assert!(std::mem::size_of::<SdrplayRsp2Params>() == 1);
    assert!(std::mem::size_of::<SdrplayRsp2TunerParams>() == 16);

    assert!(std::mem::size_of::<SdrplayRspDuoParams>() == 4);
    assert!(std::mem::size_of::<SdrplayRspDuoTunerParams>() == 16);

    assert!(std::mem::size_of::<SdrplayRspDxParams>() == 12);
    assert!(std::mem::size_of::<SdrplayRspDxTunerParams>() == 4);

    assert!(std::mem::size_of::<SdrplayStreamCbParams>() == 20);
    assert!(std::mem::size_of::<SdrplayGainCbParam>() == 16);
    assert!(std::mem::size_of::<SdrplayEventParams>() == 16);
    assert!(std::mem::size_of::<SdrplayCallbackFns>() == 24);
};

type SdrplayApiOpen = unsafe extern "C" fn() -> c_int;
type SdrplayApiClose = unsafe extern "C" fn() -> c_int;
type SdrplayApiApiVersion = unsafe extern "C" fn(*mut f32) -> c_int;
type SdrplayApiLockDeviceApi = unsafe extern "C" fn() -> c_int;
type SdrplayApiUnlockDeviceApi = unsafe extern "C" fn() -> c_int;
type SdrplayApiGetDevices = unsafe extern "C" fn(*mut SdrplayDevice, *mut u32, u32) -> c_int;
type SdrplayApiSelectDevice = unsafe extern "C" fn(*mut SdrplayDevice) -> c_int;
type SdrplayApiReleaseDevice = unsafe extern "C" fn(*mut c_void) -> c_int;
type SdrplayApiGetDeviceParams =
    unsafe extern "C" fn(*mut c_void, *mut *mut SdrplayDeviceParams) -> c_int;
type SdrplayApiInit =
    unsafe extern "C" fn(*mut c_void, *mut SdrplayCallbackFns, *mut c_void) -> c_int;
type SdrplayApiUninit = unsafe extern "C" fn(*mut c_void) -> c_int;
type SdrplayApiUpdate = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int;

struct SdrplayApi {
    _library: Library,

    open: SdrplayApiOpen,
    close: SdrplayApiClose,
    api_version: SdrplayApiApiVersion,
    lock_device_api: SdrplayApiLockDeviceApi,
    unlock_device_api: SdrplayApiUnlockDeviceApi,
    get_devices: SdrplayApiGetDevices,
    select_device: SdrplayApiSelectDevice,
    release_device: SdrplayApiReleaseDevice,
    get_device_params: SdrplayApiGetDeviceParams,
    init: SdrplayApiInit,
    uninit: SdrplayApiUninit,
    update: SdrplayApiUpdate,
}

static SDRPLAY_API: OnceLock<SdrplayApi> = OnceLock::new();

/// Names tried, in order, to load the SDRplay API. The `SDRPLAY_API_LIB`
/// environment variable (full path) replaces this list: useful for a specific
/// location, and for tests.
fn library_candidates() -> Vec<String> {
    if let Ok(path) = std::env::var("SDRPLAY_API_LIB") {
        if !path.is_empty() {
            return vec![path];
        }
    }

    [
        "libsdrplay_api.so",
        "libsdrplay_api.so.3",
        "/usr/local/lib/libsdrplay_api.so",
        "/usr/local/lib/libsdrplay_api.so.3",
    ]
    .iter()
    .map(|name| name.to_string())
    .collect()
}

fn open_library() -> Result<Library> {
    let mut attempts = Vec::new();

    for name in library_candidates() {
        match unsafe { Library::new(&name) } {
            Ok(library) => return Ok(library),
            Err(err) => attempts.push(format!("  - {} : {}", name, err)),
        }
    }

    Err(anyhow!(
        "SDRplay API not found (libsdrplay_api.so). Tried:\n{}\n\
         Install the SDRplay API 3.x (https://www.sdrplay.com/software/install.sh), \
         set SDRPLAY_API_LIB to its location, or use --mock.",
        attempts.join("\n")
    ))
}

/// Loads the SDRplay API (once); call it before any use of the backend to
/// fail early with a clear message.
pub fn ensure_api_loaded() -> Result<()> {
    load_sdrplay_api()
}

fn load_sdrplay_api() -> Result<()> {
    if SDRPLAY_API.get().is_some() {
        return Ok(());
    }

    let library = open_library()?;

    unsafe {
        let open = *library.get::<SdrplayApiOpen>(b"sdrplay_api_Open\0")?;
        let close = *library.get::<SdrplayApiClose>(b"sdrplay_api_Close\0")?;
        let api_version = *library.get::<SdrplayApiApiVersion>(b"sdrplay_api_ApiVersion\0")?;
        let lock_device_api =
            *library.get::<SdrplayApiLockDeviceApi>(b"sdrplay_api_LockDeviceApi\0")?;
        let unlock_device_api =
            *library.get::<SdrplayApiUnlockDeviceApi>(b"sdrplay_api_UnlockDeviceApi\0")?;
        let get_devices = *library.get::<SdrplayApiGetDevices>(b"sdrplay_api_GetDevices\0")?;
        let select_device =
            *library.get::<SdrplayApiSelectDevice>(b"sdrplay_api_SelectDevice\0")?;
        let release_device =
            *library.get::<SdrplayApiReleaseDevice>(b"sdrplay_api_ReleaseDevice\0")?;
        let get_device_params =
            *library.get::<SdrplayApiGetDeviceParams>(b"sdrplay_api_GetDeviceParams\0")?;
        let init = *library.get::<SdrplayApiInit>(b"sdrplay_api_Init\0")?;
        let uninit = *library.get::<SdrplayApiUninit>(b"sdrplay_api_Uninit\0")?;
        let update = *library.get::<SdrplayApiUpdate>(b"sdrplay_api_Update\0")?;

        let api = SdrplayApi {
            _library: library,

            open,
            close,
            api_version,
            lock_device_api,
            unlock_device_api,
            get_devices,
            select_device,
            release_device,
            get_device_params,
            init,
            uninit,
            update,
        };

        SDRPLAY_API
            .set(api)
            .map_err(|_| anyhow!("SDRplay API already loaded"))?;
    }

    log::info!("SDRplay API loaded dynamically");

    Ok(())
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_Open() -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API not loaded").open)()
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_Close() -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API not loaded").close)()
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_ApiVersion(api_ver: *mut f32) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .api_version)(api_ver)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_LockDeviceApi() -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .lock_device_api)()
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_UnlockDeviceApi() -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .unlock_device_api)()
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_GetDevices(
    devices: *mut SdrplayDevice,
    num_devs: *mut u32,
    max_devs: u32,
) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .get_devices)(devices, num_devs, max_devs)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_SelectDevice(device: *mut SdrplayDevice) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .select_device)(device)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_ReleaseDevice(device: *mut c_void) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .release_device)(device)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_GetDeviceParams(
    dev: *mut c_void,
    device_params: *mut *mut SdrplayDeviceParams,
) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API not loaded")
        .get_device_params)(dev, device_params)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_Init(
    dev: *mut c_void,
    callback_fns: *mut SdrplayCallbackFns,
    cb_context: *mut c_void,
) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API not loaded").init)(dev, callback_fns, cb_context)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_Uninit(dev: *mut c_void) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API not loaded").uninit)(dev)
}

#[inline]
#[allow(non_snake_case)] // mirrors the C function names
unsafe fn sdrplay_api_Update(
    dev: *mut c_void,
    tuner: c_int,
    reason_for_update: c_int,
    ext1_reason_for_update: c_int,
) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API not loaded").update)(
        dev,
        tuner,
        reason_for_update,
        ext1_reason_for_update,
    )
}

fn timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros() as u64)
        .unwrap_or(0)
}

unsafe extern "C" fn stream_a_callback(
    xi: *mut i16,
    xq: *mut i16,
    params: *mut SdrplayStreamCbParams,
    num_samples: u32,
    reset: u32,
    cb_context: *mut c_void,
) {
    if cb_context.is_null() || xi.is_null() || xq.is_null() {
        return;
    }

    let context = &*(cb_context as *const CallbackContext);

    if num_samples == 0 {
        return;
    }
    if !context.iq_enabled.load(Ordering::Relaxed) {
        return;
    }
    let callback_count = IQ_CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;

    if !params.is_null() && (*params).fs_changed != 0 {
        context.fs_changed.store(true, Ordering::Relaxed);
    }

    if !params.is_null()
        && ((*params).gr_changed != 0
            || (*params).rf_changed != 0
            || (*params).fs_changed != 0
            || reset != 0)
    {
        log::debug!(
            "SDRplay CALLBACK CHANGE #{}: first={} gr={} rf={} fs={} reset={} num={}",
            callback_count,
            (*params).first_sample_num,
            (*params).gr_changed,
            (*params).rf_changed,
            (*params).fs_changed,
            reset,
            num_samples
        );

        // Raw RMS of the exact callback that reported the change.
        let mut sum_i_sq = 0.0f64;
        let mut sum_q_sq = 0.0f64;

        for index in 0..num_samples as usize {
            let i = *xi.add(index) as f64 / 32768.0;
            let q = *xq.add(index) as f64 / 32768.0;
            sum_i_sq += i * i;
            sum_q_sq += q * q;
        }

        let n = num_samples.max(1) as f64;
        let rms_i = (sum_i_sq / n).sqrt();
        let rms_q = (sum_q_sq / n).sqrt();

        log::debug!("CALLBACK RAW RMS: I={:.6} Q={:.6}", rms_i, rms_q);
    }

    if callback_count <= 3 && !params.is_null() {
        let raw = std::slice::from_raw_parts(
            params as *const u8,
            std::mem::size_of::<SdrplayStreamCbParams>(),
        );

        log::debug!(
            "PARAMS #{}: first={} gr={} rf={} fs={} num={} xi={:p} xq={:p}",
            callback_count,
            (*params).first_sample_num,
            (*params).gr_changed,
            (*params).rf_changed,
            (*params).fs_changed,
            (*params).num_samples,
            xi,
            xq
        );

        log::debug!("PARAMS RAW #{}: {:02x?}", callback_count, raw);
    }

    if callback_count <= 5 {
        let mut q_min = i16::MAX;
        let mut q_max = i16::MIN;
        let mut q_nonzero = 0u32;
        let mut q_sum_sq = 0.0f64;

        for index in 0..num_samples as usize {
            let q = *xq.add(index);
            q_min = q_min.min(q);
            q_max = q_max.max(q);

            if q != 0 {
                q_nonzero += 1;
            }

            let qf = q as f64;
            q_sum_sq += qf * qf;
        }

        let q_rms = if num_samples > 0 {
            (q_sum_sq / num_samples as f64).sqrt()
        } else {
            0.0
        };

        log::debug!(
            "IQCHECK #{} reset={} num={}: Qmin={} Qmax={} Qrms={:.1} Qnonzero={}/{}",
            callback_count,
            reset,
            num_samples,
            q_min,
            q_max,
            q_rms,
            q_nonzero,
            num_samples
        );
    }

    // One log line per callback (several hundred per second) would block the API's
    // real-time thread on terminal output and cause USB packet loss: one trace
    // every 2000 callbacks is enough.
    if callback_count % 2000 == 1 {
        log::debug!(
            "SDRplay IQ callback #{}: {} samples",
            callback_count,
            num_samples
        );
    }

    let center_frequency_hz = context.center_frequency_hz.load(Ordering::Relaxed);

    let sample_rate = context.sample_rate.load(Ordering::Relaxed) as u32;

    let mut samples = Vec::with_capacity(num_samples as usize);

    for index in 0..num_samples as usize {
        let i = *xi.add(index) as f32 / 32768.0;

        let q = *xq.add(index) as f32 / 32768.0;

        samples.push(IqSample { i, q });
    }

    let blocks = {
        let mut reblocker = match context.reblocker.lock() {
            Ok(reblocker) => reblocker,

            Err(_) => {
                log::error!("IqReblocker mutex poisoned");

                return;
            }
        };

        reblocker.push(&samples, timestamp_now(), center_frequency_hz, sample_rate)
    };

    for block in blocks {
        match context.iq_tx.try_send(block) {
            Ok(()) => {}

            Err(TrySendError::Full(_)) => {
                let dropped = context.dropped_blocks.fetch_add(1, Ordering::Relaxed) + 1;

                // Rate-limited: this runs in the API's real-time thread, and a
                // full queue would otherwise log once per block.
                if dropped == 1 || dropped % 1000 == 0 {
                    log::warn!(
                        "IQ block dropped: IQ queue full ({} blocks dropped so far)",
                        dropped
                    );
                }
            }

            Err(TrySendError::Disconnected(_)) => {
                // The core is no longer listening to the IQ stream.
            }
        }
    }

    if !IQ_CALLBACK_RECEIVED.swap(true, Ordering::Relaxed) {
        log::info!("IQ stream -> core active");

        log::debug!("numSamples: {}", num_samples);

        log::debug!("reset: {}", reset);

        if !params.is_null() {
            log::debug!("firstSampleNum: {}", (*params).first_sample_num);

            log::debug!("grChanged: {}", (*params).gr_changed);

            log::debug!("rfChanged: {}", (*params).rf_changed);

            log::debug!("fsChanged: {}", (*params).fs_changed);
        }

        let first_i = *xi as f32 / 32768.0;

        let first_q = *xq as f32 / 32768.0;

        log::debug!("first I: {:.6}", first_i);

        log::debug!("first Q: {:.6}", first_q);

        log::debug!("core block size: {}", crate::core::IQ_BLOCK_SIZE);
    }
}

unsafe extern "C" fn event_callback(
    event_id: c_int,
    tuner: c_int,
    params: *mut SdrplayEventParams,
    cb_context: *mut c_void,
) {
    log::debug!("SDRplay event: eventId={} tuner={}", event_id, tuner);

    // sdrplay_api_GainChange = 0
    if event_id == 0 {
        if !params.is_null() {
            let gain = (*params).gain_params;
            if !cb_context.is_null() {
                let context = &*(cb_context as *const CallbackContext);

                context
                    .callback_gr_db
                    .store(gain.gr_db as u64, Ordering::Relaxed);

                context
                    .lna_gr_db
                    .store(gain.lna_gr_db as u64, Ordering::Relaxed);

                context
                    .curr_gain_milli_db
                    .store((gain.curr_gain * 1000.0) as u64, Ordering::Relaxed);

                // Real total gain (LNA + IF) for the outputs: AbracaDABra's RF level.
                crate::core::telemetry::set_total_gain_db(gain.curr_gain);
            }

            log::debug!(
                "gain callback: gRdB={} lnaGRdB={} currGain={:.2} dB",
                gain.gr_db,
                gain.lna_gr_db,
                gain.curr_gain
            );
        }
    }

    // sdrplay_api_PowerOverloadChange = 1
    // The service sends no further overload message until the previous one has
    // been acknowledged (sdrplay_api_Update_Ctrl_OverloadMsgAck): the
    // acknowledgement is done outside this callback, by SdrplayBackend::service().
    if event_id == 1 && !params.is_null() && !cb_context.is_null() {
        let context = &*(cb_context as *const CallbackContext);

        // 0 = Overload_Detected, 1 = Overload_Corrected
        let detected = (*params).power_overload_params.power_overload_change_type == 0;

        let was = context.overload.swap(detected, Ordering::Relaxed);

        crate::core::telemetry::set_overload(detected);

        if detected && !was {
            context.overload_events.fetch_add(1, Ordering::Relaxed);
        }

        context.overload_ack_pending.store(true, Ordering::Relaxed);
    }
}

pub struct SdrplayBackend {
    connected: bool,
    device_selected: bool,
    initialized: bool,
    serial: Option<String>,
    selected_device: Option<SdrplayDevice>,
    iq_rx: Option<Receiver<IqBlock>>,
    callback_context: Option<Box<CallbackContext>>,
    gain_mode: crate::core::GainMode,
    /// Current gain band (depends on the RF frequency).
    band: Band,
    /// Last gain step requested (0..=28), re-applied at every band change.
    gain_index: usize,
    /// SDRplay hardware AGC enabled.
    agc_on: bool,
    last_overload_log: Instant,
}

impl SdrplayBackend {
    pub fn new() -> Self {
        let (iq_tx, iq_rx) = sync_channel(1024);

        let callback_context = Box::new(CallbackContext {
            iq_tx,

            reblocker: Mutex::new(IqReblocker::new()),

            center_frequency_hz: AtomicU64::new(200_000_000),

            sample_rate: AtomicU64::new(2_000_000),

            dropped_blocks: AtomicU64::new(0),
            lna_gr_db: AtomicU64::new(0),
            curr_gain_milli_db: AtomicU64::new(0),
            callback_gr_db: AtomicU64::new(0),
            iq_enabled: AtomicBool::new(false),
            overload: AtomicBool::new(false),
            overload_ack_pending: AtomicBool::new(false),
            overload_events: AtomicU64::new(0),
            fs_changed: AtomicBool::new(false),
        });

        Self {
            connected: false,
            device_selected: false,
            initialized: false,
            serial: None,
            selected_device: None,
            iq_rx: Some(iq_rx),
            callback_context: Some(callback_context),
            gain_mode: crate::core::GainMode::Manual,
            band: Band::from_hz(200_000_000),
            gain_index: gain::DEFAULT_GAIN_INDEX,
            agc_on: false,
            last_overload_log: Instant::now(),
        }
    }

    pub fn take_iq_receiver(&mut self) -> Option<Receiver<IqBlock>> {
        self.iq_rx.take()
    }

    pub fn start_iq(&self) -> Result<()> {
        let context = self
            .callback_context
            .as_ref()
            .ok_or_else(|| anyhow!("IQ callback context missing"))?;

        context.iq_enabled.store(true, Ordering::Relaxed);
        log::info!("IQ stream started");

        Ok(())
    }

    pub fn stop_iq(&self) -> Result<()> {
        let context = self
            .callback_context
            .as_ref()
            .ok_or_else(|| anyhow!("IQ callback context missing"))?;

        context.iq_enabled.store(false, Ordering::Relaxed);
        log::info!("IQ stream stopped");

        Ok(())
    }

    pub fn apply_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::FrequencyChanged(frequency_hz) => {
                let old_band = self.band;

                self.set_frequency(*frequency_hz)?;

                // The meaning of an LNAstate changes with the band: re-apply the gain step
                // with the new band's table.
                if self.band != old_band {
                    log::info!(
                        "Band change {:?} -> {:?}: gain re-applied (step {})",
                        old_band,
                        self.band,
                        self.gain_index
                    );
                    self.set_gain_index(self.gain_index)?;
                }
            }

            Event::SampleRateChanged(sample_rate_hz) => {
                self.set_sample_rate(*sample_rate_hz)?;

                // rsp_tcp re-applies its AGC settings after a rate change.
                if self.agc_on {
                    self.set_agc(true)?;
                }
            }

            Event::BandwidthChanged(bandwidth_hz) => {
                self.set_bandwidth(*bandwidth_hz)?;
            }

            Event::IfTypeChanged(if_type) => {
                self.set_if_type(*if_type)?;
            }

            Event::LoModeChanged(lo_mode) => {
                self.set_lo_mode_core(*lo_mode)?;
            }

            Event::GainChanged(gain_db) => {
                self.set_gain(*gain_db)?;
            }

            Event::GainIndexChanged(index) => {
                self.set_gain_index(*index)?;
            }

            Event::GainModeChanged(gain_mode) => {
                self.gain_mode = *gain_mode;

                match gain_mode {
                    crate::core::GainMode::Automatic => {
                        self.set_agc(true)?;
                    }
                    crate::core::GainMode::Manual => {
                        // Some clients send "manual gain" before EVERY gain change: touch the
                        // hardware only if the AGC was really on (otherwise a useless Update and a
                        // brief return to the old gain before the new one).
                        if self.agc_on {
                            self.set_agc(false)?;
                            // The AGC may have moved gRdB: back to the requested step.
                            self.set_gain_index(self.gain_index)?;
                        }
                    }
                }
            }

            Event::AgcChanged(enabled) => {
                // rtl_tcp 0x08 = RTL2832 digital AGC: no equivalent on an RSP. The RSP's
                // AGC only follows the gain mode (0x03).
                log::debug!(
                    "RTL digital AGC (0x08) ignored: enabled={} gain_mode={:?}",
                    enabled,
                    self.gain_mode
                );
            }

            Event::BiasTeeChanged(enabled) => {
                self.set_bias_t(*enabled)?;
            }

            Event::RfNotchChanged(enabled) => {
                self.set_rf_notch(*enabled)?;
            }

            Event::DabNotchChanged(enabled) => {
                self.set_dab_notch(*enabled)?;
            }

            Event::PpmChanged(ppm) => {
                self.set_ppm(*ppm as f64)?;
            }

            Event::IqStarted => {
                self.start_iq()?;
            }

            Event::IqStopped => {
                self.stop_iq()?;
            }

            _ => {}
        }

        Ok(())
    }

    pub fn connect(&mut self) -> Result<()> {
        load_sdrplay_api()?;

        unsafe {
            let result = sdrplay_api_Open();

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Open() failed: {}", result));
            }

            self.connected = true;

            let mut api_version = 0.0f32;

            let result = sdrplay_api_ApiVersion(&mut api_version);

            if result != 0 {
                self.disconnect();

                return Err(anyhow!("sdrplay_api_ApiVersion() failed: {}", result));
            }

            log::info!("SDRplay API version: {:.2}", api_version);

            if let Err(err) = self.select_first_device() {
                self.disconnect();

                return Err(err);
            }
        }

        Ok(())
    }

    unsafe fn select_first_device(&mut self) -> Result<()> {
        let result = sdrplay_api_LockDeviceApi();

        if result != 0 {
            return Err(anyhow!("LockDeviceApi() failed: {}", result));
        }

        let mut devices = [SdrplayDevice {
            ser_no: [0; MAX_SER_NO_LEN],

            hw_ver: 0,
            tuner: 0,
            rsp_duo_mode: 0,
            valid: 0,
            rsp_duo_sample_freq: 0.0,
            dev: std::ptr::null_mut(),
        }; MAX_DEVICES];

        let mut num_devices = 0u32;

        let result =
            sdrplay_api_GetDevices(devices.as_mut_ptr(), &mut num_devices, MAX_DEVICES as u32);

        if result != 0 {
            sdrplay_api_UnlockDeviceApi();

            return Err(anyhow!("GetDevices() failed: {}", result));
        }

        log::info!("SDRplay devices found: {}", num_devices);

        if num_devices == 0 {
            sdrplay_api_UnlockDeviceApi();

            return Err(anyhow!("No SDRplay device detected"));
        }

        let device = &mut devices[0];

        let serial_end = device
            .ser_no
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(MAX_SER_NO_LEN);

        let serial = String::from_utf8_lossy(&device.ser_no[..serial_end]).to_string();

        log::info!("Selected device:");

        log::info!("  serial: {}", serial);

        log::info!("  hardware version: {}", device.hw_ver);

        // sdrplay_api_DisableHeartbeat() is not called: the heartbeat is the safety net
        // that lets the sdrplay service release the device automatically if the
        // client exits without being able to propagate Uninit/Close properly.

        let result = sdrplay_api_SelectDevice(device);

        if result != 0 {
            sdrplay_api_UnlockDeviceApi();

            return Err(anyhow!("SelectDevice() failed: {}", result));
        }

        let selected_device = *device;

        self.device_selected = true;

        self.serial = Some(serial);

        self.selected_device = Some(selected_device);

        let result = sdrplay_api_UnlockDeviceApi();

        if result != 0 {
            return Err(anyhow!("UnlockDeviceApi() failed: {}", result));
        }

        log::info!("Device selected");

        self.get_device_params(selected_device.dev)?;

        self.initialize_device(selected_device.dev)?;

        Ok(())
    }

    unsafe fn get_device_params(&mut self, dev: *mut c_void) -> Result<()> {
        let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let result = sdrplay_api_GetDeviceParams(dev, &mut params);

        if result != 0 {
            return Err(anyhow!("GetDeviceParams() failed: {}", result));
        }

        if params.is_null() {
            return Err(anyhow!(
                "GetDeviceParams() succeeded but returned a NULL pointer"
            ));
        }

        log::debug!("device parameters retrieved");

        log::debug!("deviceParams: {:p}", params);

        log::debug!("devParams: {:p}", (*params).dev_params);

        log::debug!("rxChannelA: {:p}", (*params).rx_channel_a);

        log::debug!("rxChannelB: {:p}", (*params).rx_channel_b);

        let mut sample_rate = 2_000_000u64;

        let mut frequency = 200_000_000u64;

        if !(*params).dev_params.is_null() {
            let dev_params = &*(*params).dev_params;

            sample_rate = dev_params.fs_freq.fs_hz.round() as u64;

            log::debug!("device parameters:");

            log::debug!("  sample rate: {:.0} Hz", dev_params.fs_freq.fs_hz);

            log::debug!("  PPM: {:.3}", dev_params.ppm);

            log::debug!("  transfer mode: {}", dev_params.mode);

            log::debug!("  samples/packet: {}", dev_params.samples_per_pkt);

            log::debug!("  RF notch: {}", dev_params.rsp1a_params.rf_notch_enable);

            log::debug!(
                "  DAB notch: {}",
                dev_params.rsp1a_params.rf_dab_notch_enable
            );
        }

        if !(*params).rx_channel_a.is_null() {
            let rx = &*(*params).rx_channel_a;

            frequency = rx.tuner_params.rf_freq.rf_hz.round() as u64;

            log::debug!("RX A parameters:");

            log::debug!("  frequency: {:.0} Hz", rx.tuner_params.rf_freq.rf_hz);

            log::debug!("  bandwidth: {} kHz", rx.tuner_params.bw_type);

            log::debug!("  IF: {} kHz", rx.tuner_params.if_type);

            log::debug!("  LO mode: {}", rx.tuner_params.lo_mode);

            log::debug!("  gain reduction: {} dB", rx.tuner_params.gain.gr_db);

            log::debug!("  LNA state: {}", rx.tuner_params.gain.lna_state);

            log::debug!("  min gain reduction: {} dB", rx.tuner_params.gain.min_gr);

            log::debug!(
                "  gain values: curr={:.2} max={:.2} min={:.2}",
                rx.tuner_params.gain.gain_vals.curr,
                rx.tuner_params.gain.gain_vals.max,
                rx.tuner_params.gain.gain_vals.min
            );

            log::debug!("  LNA state: {}", rx.tuner_params.gain.lna_state);

            log::debug!("  AGC: {}", rx.ctrl_params.agc.enable);

            log::debug!(
                "  AGC set-point: {} dBFS",
                rx.ctrl_params.agc.set_point_dbfs
            );

            log::debug!("  DC offset: {}", rx.ctrl_params.dc_offset.dc_enable);

            log::debug!("  IQ balance: {}", rx.ctrl_params.dc_offset.iq_enable);

            log::debug!("  bias-T: {}", rx.rsp1a_tuner_params.bias_t_enable);
        }

        if let Some(context) = self.callback_context.as_ref() {
            context.sample_rate.store(sample_rate, Ordering::Relaxed);

            context
                .center_frequency_hz
                .store(frequency, Ordering::Relaxed);
        }

        Ok(())
    }

    unsafe fn initialize_device(&mut self, dev: *mut c_void) -> Result<()> {
        IQ_CALLBACK_RECEIVED.store(false, Ordering::Relaxed);

        IQ_CALLBACK_COUNT.store(0, Ordering::Relaxed);

        let context = self
            .callback_context
            .as_mut()
            .ok_or_else(|| anyhow!("IQ callback context missing"))?;

        let context_ptr = context.as_mut() as *mut CallbackContext as *mut c_void;

        let mut callbacks = SdrplayCallbackFns {
            stream_a_cb_fn: Some(stream_a_callback),

            stream_b_cb_fn: None,

            event_cb_fn: Some(event_callback),
        };

        log::info!("Initialising the receiver...");

        let result = sdrplay_api_Init(dev, &mut callbacks, context_ptr);

        if result != 0 {
            return Err(anyhow!("sdrplay_api_Init() failed: {}", result));
        }

        self.initialized = true;

        let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();
        let verify_result = sdrplay_api_GetDeviceParams(dev, &mut verify_params);

        if verify_result == 0 && !verify_params.is_null() {
            if !(*verify_params).dev_params.is_null() {
                let dp = &*(*verify_params).dev_params;
                log::debug!(
                    "POST-INIT DEV: mode={} samplesPkt={} fs={:.0}",
                    dp.mode,
                    dp.samples_per_pkt,
                    dp.fs_freq.fs_hz
                );
            }

            if !(*verify_params).rx_channel_a.is_null() {
                let rx = &*(*verify_params).rx_channel_a;
                log::debug!(
                    "POST-INIT RX: freq={:.0} IQenable={} DCenable={} AGC={} DECenable={} DECfactor={} WBS={} ADSB={}",
                    rx.tuner_params.rf_freq.rf_hz,
                    rx.ctrl_params.dc_offset.iq_enable,
                    rx.ctrl_params.dc_offset.dc_enable,
                    rx.ctrl_params.agc.enable,
                    rx.ctrl_params.decimation.enable,
                    rx.ctrl_params.decimation.decimation_factor,
                    rx.ctrl_params.decimation.wide_band_signal,
                    rx.ctrl_params.adsb_mode
                );
            }
        }

        log::info!("Receiver initialised");

        Ok(())
    }

    pub fn set_frequency(&mut self, frequency_hz: u64) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B not initialised"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("SDRplay device missing"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() failed: {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() returned NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA is NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            rx.tuner_params.rf_freq.rf_hz = frequency_hz as f64;

            const UPDATE_TUNER_FRF: c_int = 0x00020000;
            const UPDATE_TUNER_GR: c_int = 0x00008000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            // An LNAstate that does not exist in the new band is refused by the service
            // (OutOfRange): bring it down to the band's maximum in the same update.
            let new_band = Band::from_hz(frequency_hz);
            let mut reason = UPDATE_TUNER_FRF;

            if rx.tuner_params.gain.lna_state > new_band.max_lna_state() {
                log::info!(
                    "LNA state {} invalid in band {:?}: reduced to {}",
                    rx.tuner_params.gain.lna_state,
                    new_band,
                    new_band.max_lna_state()
                );
                rx.tuner_params.gain.lna_state = new_band.max_lna_state();
                reason |= UPDATE_TUNER_GR;
            }

            self.band = new_band;

            let result = sdrplay_api_Update(device.dev, TUNER_A, reason, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(FRF) failed: {}", result));
            }

            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            if sdrplay_api_GetDeviceParams(device.dev, &mut verify_params) == 0
                && !verify_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let verify_rx = &*(*verify_params).rx_channel_a;

                log::debug!(
                    "RF VERIFY: RF={:.0} Hz IF={} kHz BW={} kHz LO={} LNA={} gRdB={} curr={:.2} dB",
                    verify_rx.tuner_params.rf_freq.rf_hz,
                    verify_rx.tuner_params.if_type,
                    verify_rx.tuner_params.bw_type,
                    verify_rx.tuner_params.lo_mode,
                    verify_rx.tuner_params.gain.lna_state,
                    verify_rx.tuner_params.gain.gr_db,
                    verify_rx.tuner_params.gain.gain_vals.curr
                );
            }

            if let Some(context) = self.callback_context.as_ref() {
                context
                    .center_frequency_hz
                    .store(frequency_hz, Ordering::Relaxed);
            }

            log::info!("RSP1B frequency set to {} Hz", frequency_hz);
        }

        Ok(())
    }

    /// Sets the OUTPUT sample rate. Below 2 MS/s the ADC stays at the nearest
    /// power-of-two multiple (at least 2 MS/s) and the API's decimator divides
    /// it down (see `core::rates`), like SDRplay's rsp_tcp.
    pub fn set_sample_rate(&mut self, sample_rate_hz: u32) -> Result<()> {
        let plan = rates::plan(sample_rate_hz)
            .ok_or_else(|| anyhow!("Unsupported sample rate: {} Hz", sample_rate_hz))?;

        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B not initialised"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("SDRplay device missing"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() failed: {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() returned NULL"));
            }

            if (*params).dev_params.is_null() {
                return Err(anyhow!("devParams is NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA is NULL"));
            }

            let dev_params = &mut *(*params).dev_params;
            let rx = &mut *(*params).rx_channel_a;

            dev_params.fs_freq.fs_hz = plan.adc_hz as f64;

            if plan.decimation > 1 {
                rx.ctrl_params.decimation.enable = 1;
                rx.ctrl_params.decimation.decimation_factor = plan.decimation as u8;
                rx.ctrl_params.decimation.wide_band_signal = 1;
            } else {
                rx.ctrl_params.decimation.enable = 0;
            }

            const UPDATE_DEV_FS: c_int = 0x00000001;
            const UPDATE_CTRL_DECIMATION: c_int = 0x00800000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            // Cleared before the update, set again by the stream callback once
            // the service reports the new rate.
            let waiting_for_callback = self
                .callback_context
                .as_ref()
                .map(|context| {
                    context.fs_changed.store(false, Ordering::Relaxed);
                    context.iq_enabled.load(Ordering::Relaxed)
                })
                .unwrap_or(false);

            let result = sdrplay_api_Update(
                device.dev,
                TUNER_A,
                UPDATE_DEV_FS | UPDATE_CTRL_DECIMATION,
                EXT1_NONE,
            );

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(FS) failed: {}", result));
            }

            // Like rsp_tcp, wait (at most 500 ms) until the new rate is in
            // effect before touching anything else. Only possible while the
            // stream runs: the callback is what reports the change.
            if waiting_for_callback {
                let started = std::time::Instant::now();

                loop {
                    let changed = self
                        .callback_context
                        .as_ref()
                        .map(|context| context.fs_changed.load(Ordering::Relaxed))
                        .unwrap_or(true);

                    if changed {
                        log::debug!(
                            "sample rate in effect after {} ms",
                            started.elapsed().as_millis()
                        );
                        break;
                    }

                    if started.elapsed() > std::time::Duration::from_millis(500) {
                        log::warn!("sample rate change not confirmed by the service after 500 ms");
                        break;
                    }

                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }

            // Diagnostic: read the parameters back after Update(FS)
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();
            let verify_result = sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

            if verify_result == 0
                && !verify_params.is_null()
                && !(*verify_params).dev_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let verify_dev = &*(*verify_params).dev_params;
                let verify_rx = &*(*verify_params).rx_channel_a;

                log::debug!(
                    "FS VERIFY: fsHz={} samplesPerPkt={} IF={} kHz BW={}",
                    verify_dev.fs_freq.fs_hz,
                    verify_dev.samples_per_pkt,
                    verify_rx.tuner_params.if_type,
                    verify_rx.tuner_params.bw_type
                );
            } else {
                log::warn!("FS VERIFY: GetDeviceParams() invalid after Update");
            }

            if let Some(context) = self.callback_context.as_ref() {
                context
                    .sample_rate
                    .store(plan.output_hz as u64, Ordering::Relaxed);
            }

            log::info!(
                "RSP1B sample rate set to {} Hz (ADC {} Hz, decimation {})",
                plan.output_hz,
                plan.adc_hz,
                plan.decimation
            );
        }

        Ok(())
    }

    pub fn set_bandwidth(&mut self, bandwidth_hz: u32) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B not initialised"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("SDRplay device missing"))?;

        let bw_type: c_int = match bandwidth_hz {
            200_000 => 200,
            300_000 => 300,
            600_000 => 600,
            1_536_000 => 1536,
            5_000_000 => 5000,
            6_000_000 => 6000,
            7_000_000 => 7000,
            8_000_000 => 8000,

            _ => {
                return Err(anyhow!("Unsupported bandwidth: {} Hz", bandwidth_hz));
            }
        };

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() failed: {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() returned NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA is NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            // rtl_tcp clients resend the bandwidth at every connection: no need to
            // reprogram the RSP if it is already right.
            if rx.tuner_params.bw_type == bw_type {
                return Ok(());
            }

            rx.tuner_params.bw_type = bw_type;

            const UPDATE_TUNER_BW_TYPE: c_int = 0x00040000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_TUNER_BW_TYPE, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(BW_TYPE) failed: {}", result));
            }
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let verify_result = sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

            if verify_result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() after BW failed: {}",
                    verify_result
                ));
            }

            if verify_params.is_null() {
                return Err(anyhow!("GetDeviceParams() after BW returned NULL"));
            }

            if (*verify_params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA after BW is NULL"));
            }

            let verify_rx = &*(*verify_params).rx_channel_a;

            log::debug!(
                "bandwidth after Update = {} kHz",
                verify_rx.tuner_params.bw_type
            );
            log::info!("RSP1B bandwidth set to {} Hz", bandwidth_hz);
        }

        Ok(())
    }
    pub fn set_if_type(&mut self, if_type: IfType) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B not initialised"));
        }

        let if_khz: c_int = match if_type {
            IfType::Zero => 0,
            IfType::KHz450 => 450,
            IfType::KHz1620 => 1620,
            IfType::KHz2048 => 2048,
        };

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("SDRplay device missing"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() failed: {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() returned NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA is NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            rx.tuner_params.if_type = if_khz;

            const UPDATE_TUNER_IF_TYPE: c_int = 0x00080000;
            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_TUNER_IF_TYPE, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(IF_TYPE) failed: {}", result));
            }

            // Read the parameters back after Update
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let verify_result = sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

            if verify_result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() after Update IF failed: {}",
                    verify_result
                ));
            }

            if verify_params.is_null() {
                return Err(anyhow!("GetDeviceParams() after Update IF returned NULL"));
            }

            if (*verify_params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA after Update IF is NULL"));
            }

            let verify_rx = &*(*verify_params).rx_channel_a;

            log::debug!("IF after Update = {} kHz", verify_rx.tuner_params.if_type);

            log::info!("RSP1B IF set to {} kHz", if_khz);
        }

        Ok(())
    }

    pub fn set_lo_mode_core(&mut self, lo_mode: LoMode) -> Result<()> {
        let value = match lo_mode {
            LoMode::Auto => 1,
            LoMode::MHz120 => 2,
            LoMode::MHz144 => 3,
            LoMode::MHz168 => 4,
        };

        self.set_lo_mode(value)
    }

    pub fn set_lo_mode(&mut self, lo_mode: i32) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B not initialised"));
        }

        let lo_mode: c_int = match lo_mode {
            1 | 2 | 3 | 4 => lo_mode,

            _ => {
                return Err(anyhow!("Unsupported LO mode: {}", lo_mode));
            }
        };

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("SDRplay device missing"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() failed: {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() returned NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA is NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            rx.tuner_params.lo_mode = lo_mode;

            const UPDATE_TUNER_LO_MODE: c_int = 0x00200000;
            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_TUNER_LO_MODE, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(LO_MODE) failed: {}", result));
            }

            // Read the parameters back after Update
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let verify_result = sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

            if verify_result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() after Update LO failed: {}",
                    verify_result
                ));
            }

            if verify_params.is_null() {
                return Err(anyhow!("GetDeviceParams() after Update LO returned NULL"));
            }

            if (*verify_params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA after Update LO is NULL"));
            }

            let verify_rx = &*(*verify_params).rx_channel_a;

            log::debug!("LO after Update = {}", verify_rx.tuner_params.lo_mode);

            log::info!("RSP1B LO mode set to {}", lo_mode);
        }

        Ok(())
    }
    /// rtl_tcp gain (command 0x04, R820T scale 0..49.6 dB) -> gain step.
    pub fn set_gain(&mut self, gain_db: f32) -> Result<()> {
        if !gain_db.is_finite() {
            return Err(anyhow!("Invalid gain: {}", gain_db));
        }

        let tenths = (gain_db.max(0.0) * 10.0).round() as u32;

        self.set_gain_index(gain::index_from_tenths_db(tenths))
    }

    /// Applies a gain step (0..=28) with the current band's table: LNAstate and
    /// gRdB are sent together in a single update.
    pub fn set_gain_index(&mut self, index: usize) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("SDRplay not initialised"));
        }

        let index = index.min(gain::GAIN_STEPS - 1);
        let band = self.band;
        let (lna_state, gr_db) = gain::settings(band, index);

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Selected SDRplay device not found"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_GetDeviceParams failed: {}", result));
            }

            if params.is_null() || (*params).rx_channel_a.is_null() {
                return Err(anyhow!("RX channel A parameters unavailable"));
            }

            let rx = &mut *(*params).rx_channel_a;

            rx.tuner_params.gain.lna_state = lna_state;
            rx.tuner_params.gain.gr_db = gr_db;

            const UPDATE_TUNER_GR: c_int = 0x00008000;
            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_TUNER_GR, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!(
                    "sdrplay_api_Update(GR) failed (band {:?}, step {}, LNA={}, gRdB={}): {}",
                    band,
                    index,
                    lna_state,
                    gr_db,
                    result
                ));
            }

            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            if sdrplay_api_GetDeviceParams(device.dev, &mut verify_params) == 0
                && !verify_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let v = &*(*verify_params).rx_channel_a;

                log::info!(
                "GAIN: band={:?} step={}/{} -> LNA={} gRdB={} | actual LNA={} gRdB={} curr={:.2} dB (AGC={})",
                band,
                index,
                gain::GAIN_STEPS - 1,
                lna_state,
                gr_db,
                v.tuner_params.gain.lna_state,
                v.tuner_params.gain.gr_db,
                v.tuner_params.gain.gain_vals.curr,
                self.agc_on
            );
            }
        }

        self.gain_index = index;

        // Like rsp_tcp: after an LNAstate change, re-apply the AGC configuration
        // (the AGC takes control of gRdB again).
        if self.agc_on {
            self.set_agc(true)?;
        }

        Ok(())
    }

    /// Sets a setting specific to the RSP1A/RSP1B (bias-T, notch, PPM): modifies
    /// the parameter structure, then sends the matching Update. `apply` returns
    /// `true` if the value really changed: otherwise no Update is sent (rtl_tcp
    /// clients resend these commands at every connection, even at their default
    /// value).
    fn update_rsp1_setting(
        &mut self,
        label: &str,
        reason: c_int,
        apply: impl FnOnce(&mut SdrplayDevParams, &mut SdrplayRxChannelParams) -> bool,
    ) -> Result<bool> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Selected SDRplay device not found"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_GetDeviceParams failed: {}", result));
            }

            if params.is_null()
                || (*params).dev_params.is_null()
                || (*params).rx_channel_a.is_null()
            {
                return Err(anyhow!("Device parameters unavailable"));
            }

            let dev = &mut *(*params).dev_params;
            let rx = &mut *(*params).rx_channel_a;

            if !apply(dev, rx) {
                return Ok(false);
            }

            // Before sdrplay_api_Init the structure will be read at initialisation: no
            // Update is possible or needed.
            if self.initialized {
                const TUNER_A: c_int = 1;
                const EXT1_NONE: c_int = 0;

                let result = sdrplay_api_Update(device.dev, TUNER_A, reason, EXT1_NONE);

                if result != 0 {
                    return Err(anyhow!("sdrplay_api_Update({}) failed: {}", label, result));
                }
            }
        }

        Ok(true)
    }

    pub fn set_bias_t(&mut self, enabled: bool) -> Result<()> {
        // sdrplay_api_Update_Rsp1a_BiasTControl
        let changed = self.update_rsp1_setting("Bias-T", 0x0000_0010, |_, rx| {
            let value = enabled as u8;
            let changed = rx.rsp1a_tuner_params.bias_t_enable != value;
            rx.rsp1a_tuner_params.bias_t_enable = value;
            changed
        })?;

        if changed {
            log::info!(
                "Bias-T: {}",
                if enabled {
                    "enabled (antenna power)"
                } else {
                    "disabled"
                }
            );
        }

        Ok(())
    }

    pub fn set_rf_notch(&mut self, enabled: bool) -> Result<()> {
        // sdrplay_api_Update_Rsp1a_RfNotchControl: FM broadcast notch (88-108 MHz)
        let changed = self.update_rsp1_setting("RF notch (FM)", 0x0000_0020, |dev, _| {
            let value = enabled as u8;
            let changed = dev.rsp1a_params.rf_notch_enable != value;
            dev.rsp1a_params.rf_notch_enable = value;
            changed
        })?;

        if changed {
            log::info!(
                "RF notch (FM): {}",
                if enabled { "enabled" } else { "disabled" }
            );
        }

        Ok(())
    }

    pub fn set_dab_notch(&mut self, enabled: bool) -> Result<()> {
        // sdrplay_api_Update_Rsp1a_RfDabNotchControl: DAB notch (band III)
        if enabled && self.band == Band::Band3 {
            log::warn!(
                "DAB notch enabled in band III (174-240 MHz): DAB reception will be degraded"
            );
        }

        let changed = self.update_rsp1_setting("DAB notch", 0x0000_0040, |dev, _| {
            let value = enabled as u8;
            let changed = dev.rsp1a_params.rf_dab_notch_enable != value;
            dev.rsp1a_params.rf_dab_notch_enable = value;
            changed
        })?;

        if changed {
            log::info!(
                "DAB notch: {}",
                if enabled { "enabled" } else { "disabled" }
            );
        }

        Ok(())
    }

    pub fn set_ppm(&mut self, ppm: f64) -> Result<()> {
        if !ppm.is_finite() || ppm.abs() > 1000.0 {
            return Err(anyhow!("Invalid PPM correction: {}", ppm));
        }

        // sdrplay_api_Update_Dev_Ppm
        let changed = self.update_rsp1_setting("PPM", 0x0000_0002, |dev, _| {
            let changed = (dev.ppm - ppm).abs() > 1e-9;
            dev.ppm = ppm;
            changed
        })?;

        if changed {
            log::info!("Frequency correction: {:.3} ppm", ppm);
        }

        Ok(())
    }

    /// To be called regularly from the main loop (outside the API callbacks):
    /// acknowledges overload messages and reports ADC overloads.
    pub fn service(&mut self) {
        let ack = match self.callback_context.as_ref() {
            Some(context) => context.overload_ack_pending.swap(false, Ordering::Relaxed),
            None => return,
        };

        if ack && self.initialized {
            if let Some(device) = self.selected_device {
                const UPDATE_CTRL_OVERLOAD_ACK: c_int = 0x04000000;
                const TUNER_A: c_int = 1;
                const EXT1_NONE: c_int = 0;

                let result = unsafe {
                    sdrplay_api_Update(device.dev, TUNER_A, UPDATE_CTRL_OVERLOAD_ACK, EXT1_NONE)
                };

                if result != 0 {
                    log::warn!("overload acknowledgement: Update failed ({})", result);
                }
            }
        }

        if let Some(context) = self.callback_context.as_ref() {
            let events = context.overload_events.load(Ordering::Relaxed);

            if events > 0 && self.last_overload_log.elapsed() >= std::time::Duration::from_secs(2) {
                context.overload_events.store(0, Ordering::Relaxed);
                self.last_overload_log = Instant::now();

                log::warn!(
                "ADC OVERLOAD ({} times) band={:?} gain step={}: lower the gain (higher LNA state) or enable the AGC",
                events, self.band, self.gain_index
            );
            }
        }
    }
    pub fn set_agc(&mut self, enabled: bool) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay not connected"));
        }

        if !self.device_selected {
            return Err(anyhow!("No SDRplay device selected"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B not initialised"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("SDRplay device missing"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() failed: {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() returned NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA is NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            /*
             * SDRplay AGC :
             *   0 = AGC_DISABLE
             *   1 = AGC_100HZ
             *   2 = AGC_50HZ
             *   3 = AGC_5HZ
             *   4 = AGC_CTRL_EN
             */
            if enabled {
                // Same setting as rsp_tcp (SDRplay): slow "CTRL_EN" AGC scheme (500 ms time
                // constants), suited to a wide-band signal such as DAB, set-point -30 dBFS
                // (valid range -72..-20). The old "1" was AGC_100HZ, a fast loop (100 Hz)
                // with the default -60 dBFS set-point.
                rx.ctrl_params.agc.enable = 4;
                rx.ctrl_params.agc.set_point_dbfs = -30;
                rx.ctrl_params.agc.attack_ms = 500;
                rx.ctrl_params.agc.decay_ms = 500;
                rx.ctrl_params.agc.decay_delay_ms = 200;
                rx.ctrl_params.agc.decay_threshold_db = 5;
            } else {
                rx.ctrl_params.agc.enable = 0;
            }

            const UPDATE_CTRL_AGC: c_int = 0x01000000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_CTRL_AGC, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(AGC) failed: {}", result));
            }

            // Check the AGC value SDRplay actually kept
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            if sdrplay_api_GetDeviceParams(device.dev, &mut verify_params) == 0
                && !verify_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let verify_rx = &*(*verify_params).rx_channel_a;

                log::debug!(
                    "AGC VERIFY: requested={} agc.enable={}",
                    enabled,
                    verify_rx.ctrl_params.agc.enable
                );
            }

            log::info!("RSP1B AGC {}", if enabled { "enabled" } else { "disabled" });
        }

        self.agc_on = enabled;

        Ok(())
    }
    pub fn disconnect(&mut self) {
        if !self.connected {
            return;
        }

        unsafe {
            if self.initialized {
                if let Some(device) = self.selected_device {
                    let result = sdrplay_api_Uninit(device.dev);

                    if result != 0 {
                        log::error!("sdrplay_api_Uninit() failed: {}", result);
                    } else {
                        log::info!("Receiver uninitialised");
                    }
                }

                self.initialized = false;
            }

            if let Some(device) = self.selected_device.take() {
                let result = sdrplay_api_ReleaseDevice(device.dev);

                if result != 0 {
                    log::error!("ReleaseDevice() failed: {}", result);
                } else {
                    log::info!("Receiver released");
                }
            }

            let result = sdrplay_api_Close();

            if result != 0 {
                log::error!("sdrplay_api_Close() failed: {}", result);
            } else {
                log::info!("SDRplay API session closed");
            }
        }

        self.connected = false;
        self.device_selected = false;
        self.initialized = false;
        self.serial = None;
        self.callback_context = None;
    }
}

impl Backend for SdrplayBackend {
    fn connect(&mut self) -> Result<()> {
        SdrplayBackend::connect(self)
    }

    fn disconnect(&mut self) {
        SdrplayBackend::disconnect(self)
    }

    fn apply_event(&mut self, event: &Event) -> Result<()> {
        SdrplayBackend::apply_event(self, event)
    }
    fn take_iq_receiver(&mut self) -> Option<Receiver<IqBlock>> {
        SdrplayBackend::take_iq_receiver(self)
    }

    fn service(&mut self) {
        SdrplayBackend::service(self)
    }
}

impl Drop for SdrplayBackend {
    fn drop(&mut self) {
        self.disconnect();
    }
}
