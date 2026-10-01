use anyhow::{anyhow, Result};
use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use libloading::Library;
use std::sync::OnceLock;
use crate::backend::Backend;

use crate::core::{Event, IfType, IqBlock, IqReblocker, IqSample, LoMode};

use crate::backend::gain;
use gain::Band;
use std::time::Instant;

const MAX_DEVICES: usize = 16;
const MAX_SER_NO_LEN: usize = 64;

static IQ_CALLBACK_RECEIVED: AtomicBool = AtomicBool::new(false);
static IQ_CALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);
static IQ_BLOCK_COUNT: AtomicU64 = AtomicU64::new(0);

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
    /// Surcharge ADC signalée par le service (et pas encore « corrigée »).
    overload: AtomicBool,
    /// Le service attend un acquittement (obligatoire pour recevoir la suite).
    overload_ack_pending: AtomicBool,
    /// Nombre d'entrées en surcharge depuis le dernier message affiché.
    overload_events: AtomicU64,
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

type SdrplayApiOpen =
    unsafe extern "C" fn() -> c_int;
type SdrplayApiClose =
    unsafe extern "C" fn() -> c_int;
type SdrplayApiApiVersion =
    unsafe extern "C" fn(*mut f32) -> c_int;
type SdrplayApiLockDeviceApi =
    unsafe extern "C" fn() -> c_int;
type SdrplayApiUnlockDeviceApi =
    unsafe extern "C" fn() -> c_int;
type SdrplayApiGetDevices =
    unsafe extern "C" fn(*mut SdrplayDevice, *mut u32, u32) -> c_int;
type SdrplayApiDisableHeartbeat =
    unsafe extern "C" fn() -> c_int;
type SdrplayApiSelectDevice =
    unsafe extern "C" fn(*mut SdrplayDevice) -> c_int;
type SdrplayApiReleaseDevice =
    unsafe extern "C" fn(*mut c_void) -> c_int;
type SdrplayApiGetDeviceParams =
    unsafe extern "C" fn(*mut c_void, *mut *mut SdrplayDeviceParams) -> c_int;
type SdrplayApiInit =
    unsafe extern "C" fn(
        *mut c_void,
        *mut SdrplayCallbackFns,
        *mut c_void,
    ) -> c_int;
type SdrplayApiUninit =
    unsafe extern "C" fn(*mut c_void) -> c_int;
type SdrplayApiUpdate =
    unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int;

struct SdrplayApi {
    _library: Library,

    open: SdrplayApiOpen,
    close: SdrplayApiClose,
    api_version: SdrplayApiApiVersion,
    lock_device_api: SdrplayApiLockDeviceApi,
    unlock_device_api: SdrplayApiUnlockDeviceApi,
    get_devices: SdrplayApiGetDevices,
    disable_heartbeat: SdrplayApiDisableHeartbeat,
    select_device: SdrplayApiSelectDevice,
    release_device: SdrplayApiReleaseDevice,
    get_device_params: SdrplayApiGetDeviceParams,
    init: SdrplayApiInit,
    uninit: SdrplayApiUninit,
    update: SdrplayApiUpdate,
}

static SDRPLAY_API: OnceLock<SdrplayApi> = OnceLock::new();

/// Noms essayés, dans l'ordre, pour charger l'API SDRplay. La variable
/// d'environnement `SDRPLAY_API_LIB` (chemin complet) remplace cette liste :
/// utile pour un emplacement particulier, et pour les tests.
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
        "API SDRplay introuvable (libsdrplay_api.so). Essais :\n{}\n\
         Installez l'API SDRplay 3.x (https://www.sdrplay.com/software/install.sh), \
         indiquez son emplacement avec SDRPLAY_API_LIB, ou utilisez --mock.",
        attempts.join("\n")
    ))
}

/// Charge l'API SDRplay (une seule fois) ; à appeler avant toute utilisation
/// du backend pour échouer tôt avec un message clair.
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
        let api_version =
            *library.get::<SdrplayApiApiVersion>(b"sdrplay_api_ApiVersion\0")?;
        let lock_device_api =
            *library.get::<SdrplayApiLockDeviceApi>(b"sdrplay_api_LockDeviceApi\0")?;
        let unlock_device_api =
            *library.get::<SdrplayApiUnlockDeviceApi>(b"sdrplay_api_UnlockDeviceApi\0")?;
        let get_devices =
            *library.get::<SdrplayApiGetDevices>(b"sdrplay_api_GetDevices\0")?;
        let disable_heartbeat =
            *library.get::<SdrplayApiDisableHeartbeat>(b"sdrplay_api_DisableHeartbeat\0")?;
        let select_device =
            *library.get::<SdrplayApiSelectDevice>(b"sdrplay_api_SelectDevice\0")?;
        let release_device =
            *library.get::<SdrplayApiReleaseDevice>(b"sdrplay_api_ReleaseDevice\0")?;
        let get_device_params =
            *library.get::<SdrplayApiGetDeviceParams>(b"sdrplay_api_GetDeviceParams\0")?;
        let init =
            *library.get::<SdrplayApiInit>(b"sdrplay_api_Init\0")?;
        let uninit =
            *library.get::<SdrplayApiUninit>(b"sdrplay_api_Uninit\0")?;
        let update =
            *library.get::<SdrplayApiUpdate>(b"sdrplay_api_Update\0")?;

        let api = SdrplayApi {
            _library: library,

            open,
            close,
            api_version,
            lock_device_api,
            unlock_device_api,
            get_devices,
            disable_heartbeat,
            select_device,
            release_device,
            get_device_params,
            init,
            uninit,
            update,
        };

        SDRPLAY_API
            .set(api)
            .map_err(|_| anyhow!("API SDRplay déjà chargée"))?;
    }

    println!(">>> SDRplay API chargée dynamiquement");

    Ok(())
}

#[inline]
unsafe fn sdrplay_api_Open() -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").open)()
}

#[inline]
unsafe fn sdrplay_api_Close() -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").close)()
}

#[inline]
unsafe fn sdrplay_api_ApiVersion(api_ver: *mut f32) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").api_version)(api_ver)
}

#[inline]
unsafe fn sdrplay_api_LockDeviceApi() -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").lock_device_api)()
}

#[inline]
unsafe fn sdrplay_api_UnlockDeviceApi() -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").unlock_device_api)()
}

#[inline]
unsafe fn sdrplay_api_GetDevices(
    devices: *mut SdrplayDevice,
    num_devs: *mut u32,
    max_devs: u32,
) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").get_devices)(
        devices,
        num_devs,
        max_devs,
    )
}

#[inline]
unsafe fn sdrplay_api_DisableHeartbeat() -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API non chargée")
        .disable_heartbeat)()
}

#[inline]
unsafe fn sdrplay_api_SelectDevice(device: *mut SdrplayDevice) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").select_device)(device)
}

#[inline]
unsafe fn sdrplay_api_ReleaseDevice(device: *mut c_void) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API non chargée")
        .release_device)(device)
}

#[inline]
unsafe fn sdrplay_api_GetDeviceParams(
    dev: *mut c_void,
    device_params: *mut *mut SdrplayDeviceParams,
) -> c_int {
    (SDRPLAY_API
        .get()
        .expect("SDRplay API non chargée")
        .get_device_params)(dev, device_params)
}

#[inline]
unsafe fn sdrplay_api_Init(
    dev: *mut c_void,
    callback_fns: *mut SdrplayCallbackFns,
    cb_context: *mut c_void,
) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").init)(
        dev,
        callback_fns,
        cb_context,
    )
}

#[inline]
unsafe fn sdrplay_api_Uninit(dev: *mut c_void) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").uninit)(dev)
}

#[inline]
unsafe fn sdrplay_api_Update(
    dev: *mut c_void,
    tuner: c_int,
    reason_for_update: c_int,
    ext1_reason_for_update: c_int,
) -> c_int {
    (SDRPLAY_API.get().expect("SDRplay API non chargée").update)(
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

    if !params.is_null()
        && ((*params).gr_changed != 0
            || (*params).rf_changed != 0
            || (*params).fs_changed != 0
            || reset != 0)
    {
        vprintln!(
            ">>> SDRplay CALLBACK CHANGE #{} : first={} gr={} rf={} fs={} reset={} num={}",
            callback_count,
            (*params).first_sample_num,
            (*params).gr_changed,
            (*params).rf_changed,
            (*params).fs_changed,
            reset,
            num_samples
        );

        // RMS brut du callback exact ayant signalé le changement.
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

        vprintln!(
            ">>> CALLBACK RAW RMS : I={:.6} Q={:.6}",
            rms_i,
            rms_q
        );
    }

    if callback_count <= 3 && !params.is_null() {
        let raw = std::slice::from_raw_parts(
            params as *const u8,
            std::mem::size_of::<SdrplayStreamCbParams>(),
        );

        println!(
            ">>> PARAMS #{} : first={} gr={} rf={} fs={} num={} xi={:p} xq={:p}",
            callback_count,
            (*params).first_sample_num,
            (*params).gr_changed,
            (*params).rf_changed,
            (*params).fs_changed,
            (*params).num_samples,
            xi,
            xq
        );

        println!(">>> PARAMS RAW #{} : {:02x?}", callback_count, raw);
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

        println!(
            ">>> IQCHECK #{} reset={} num={} : Qmin={} Qmax={} Qrms={:.1} Qnonzero={}/{}",
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

    // Un println! par callback (plusieurs centaines par seconde) bloque le
    // thread temps réel de l'API sur la sortie terminal et provoque des pertes
    // de paquets USB : une trace toutes les 2000 callbacks suffit.
    if callback_count % 2000 == 1 {
        vprintln!(
            ">>> SDRplay IQ callback #{} : {} samples",
            callback_count, num_samples
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
                eprintln!("Erreur : mutex IqReblocker empoisonné");

                return;
            }
        };

        reblocker.push(&samples, timestamp_now(), center_frequency_hz, sample_rate)
    };

    for block in blocks {
        let block_count = IQ_BLOCK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;


        match context.iq_tx.try_send(block) {
            Ok(()) => {}

            Err(TrySendError::Full(_)) => {
                context.dropped_blocks.fetch_add(1, Ordering::Relaxed);

                println!("!!! IQ BLOCK DROP : file IQ pleine");
            }

            Err(TrySendError::Disconnected(_)) => {
                // Le Core n'écoute plus le flux IQ.
            }
        }
    }

    if !IQ_CALLBACK_RECEIVED.swap(true, Ordering::Relaxed) {
        println!();
        println!("=================================");
        println!(" Flux IQ -> Core actif !");
        println!("=================================");

        println!("  numSamples      : {}", num_samples);

        println!("  reset           : {}", reset);

        if !params.is_null() {
            println!("  firstSampleNum  : {}", (*params).first_sample_num);

            println!("  grChanged       : {}", (*params).gr_changed);

            println!("  rfChanged       : {}", (*params).rf_changed);

            println!("  fsChanged       : {}", (*params).fs_changed);
        }

        let first_i = *xi as f32 / 32768.0;

        let first_q = *xq as f32 / 32768.0;

        println!("  premier I       : {:.6}", first_i);

        println!("  premier Q       : {:.6}", first_q);

        println!("  taille bloc Core: {}", crate::core::IQ_BLOCK_SIZE);

        println!();
    }
}

unsafe extern "C" fn event_callback(
    event_id: c_int,
    tuner: c_int,
    params: *mut SdrplayEventParams,
    cb_context: *mut c_void,
) {
    vprintln!(
        "SDRplay event : eventId={} tuner={}",
        event_id,
        tuner
    );

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

    // Gain total réel (LNA + IF) pour les sorties : niveau RF d'AbracaDABra.
    crate::core::telemetry::set_total_gain_db(gain.curr_gain);
}

            println!(
                "  Gain callback : gRdB={} lnaGRdB={} currGain={:.2} dB",
                gain.gr_db,
                gain.lna_gr_db,
                gain.curr_gain
            );
        }
    }

    // sdrplay_api_PowerOverloadChange = 1
    // Le service n'envoie plus aucun message de surcharge tant que le
    // précédent n'a pas été acquitté (sdrplay_api_Update_Ctrl_OverloadMsgAck) :
    // l'acquittement est fait hors de ce callback, par SdrplayBackend::service().
    if event_id == 1 && !params.is_null() && !cb_context.is_null() {
        let context = &*(cb_context as *const CallbackContext);

        // 0 = Overload_Detected, 1 = Overload_Corrected
        let detected =
            (*params).power_overload_params.power_overload_change_type == 0;

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
    /// Bande de gain courante (dépend de la fréquence RF).
    band: Band,
    /// Dernier pas de gain demandé (0..=28), réappliqué à chaque changement de bande.
    gain_index: usize,
    /// AGC matériel SDRplay actif.
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
            .ok_or_else(|| anyhow!("Contexte callback IQ absent"))?;

        context.iq_enabled.store(true, Ordering::Relaxed);
		println!(">>> Backend IQ activé");

        Ok(())
    }

    pub fn stop_iq(&self) -> Result<()> {
        let context = self
            .callback_context
            .as_ref()
            .ok_or_else(|| anyhow!("Contexte callback IQ absent"))?;

        context.iq_enabled.store(false, Ordering::Relaxed);
		println!(">>> Backend IQ désactivé");

        Ok(())
    }

    pub fn apply_event(&mut self, event: &Event) -> Result<()> {
        match event {
            Event::FrequencyChanged(frequency_hz) => {
                let old_band = self.band;

                self.set_frequency(*frequency_hz)?;

                // La signification d'un LNAstate change avec la bande :
                // on réapplique le pas de gain avec la table de la nouvelle bande.
                if self.band != old_band {
                    println!(
                        ">>> Changement de bande {:?} -> {:?} : gain réappliqué (pas {})",
                        old_band, self.band, self.gain_index
                    );
                    self.set_gain_index(self.gain_index)?;
                }
            }

            Event::SampleRateChanged(sample_rate_hz) => {
                self.set_sample_rate(*sample_rate_hz)?;
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
                        // Certains clients renvoient « gain manuel » avant CHAQUE
                        // réglage de gain : on ne touche au matériel que si l'AGC
                        // était réellement active (sinon Update inutile et
                        // retour bref à l'ancien gain avant le nouveau).
                        if self.agc_on {
                            self.set_agc(false)?;
                            // L'AGC a pu déplacer gRdB : retour au pas demandé.
                            self.set_gain_index(self.gain_index)?;
                        }
                    }
                }
            }

            Event::AgcChanged(enabled) => {
                // rtl_tcp 0x08 = AGC numérique du RTL2832 : sans équivalent sur
                // un RSP. L'AGC du RSP suit uniquement le mode de gain (0x03).
                println!(
                    ">>> BACKEND AGC RTL (0x08) ignoré : enabled={} gain_mode={:?}",
                    enabled, self.gain_mode
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

    pub fn dropped_iq_blocks(&self) -> u64 {
        self.callback_context
            .as_ref()
            .map(|context| context.dropped_blocks.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn connect(&mut self) -> Result<()> {
        load_sdrplay_api()?;

        unsafe {
            let result = sdrplay_api_Open();

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Open() a échoué : {}", result));
            }

            self.connected = true;

            let mut api_version = 0.0f32;

            let result = sdrplay_api_ApiVersion(&mut api_version);

            if result != 0 {
                self.disconnect();

                return Err(anyhow!("sdrplay_api_ApiVersion() a échoué : {}", result));
            }

            println!("SDRplay API : {:.2}", api_version);

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
            return Err(anyhow!("LockDeviceApi() a échoué : {}", result));
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

            return Err(anyhow!("GetDevices() a échoué : {}", result));
        }

        println!("Nombre de périphériques SDRplay : {}", num_devices);

        if num_devices == 0 {
            sdrplay_api_UnlockDeviceApi();

            return Err(anyhow!("Aucun périphérique SDRplay détecté"));
        }

        let device = &mut devices[0];

        let serial_end = device
            .ser_no
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(MAX_SER_NO_LEN);

        let serial = String::from_utf8_lossy(&device.ser_no[..serial_end]).to_string();

        println!("Périphérique sélectionné :");

        println!("  série : {}", serial);

        println!("  hwVer : {}", device.hw_ver);

       // sdrplay_api_DisableHeartbeat() désactivé : le heartbeat est le
// filet de sécurité qui permet au service sdrplay de libérer le
// device automatiquement si le client se ferme sans avoir pu
// propager Uninit/Close proprement.

let result = sdrplay_api_SelectDevice(device);

        if result != 0 {
            sdrplay_api_UnlockDeviceApi();

            return Err(anyhow!("SelectDevice() a échoué : {}", result));
        }

        let selected_device = *device;

        self.device_selected = true;

        self.serial = Some(serial);

        self.selected_device = Some(selected_device);

        let result = sdrplay_api_UnlockDeviceApi();

        if result != 0 {
            return Err(anyhow!("UnlockDeviceApi() a échoué : {}", result));
        }

        println!("RSP1B sélectionné.");

        self.get_device_params(selected_device.dev)?;

        self.initialize_device(selected_device.dev)?;

        Ok(())
    }

    unsafe fn get_device_params(&mut self, dev: *mut c_void) -> Result<()> {
        let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let result = sdrplay_api_GetDeviceParams(dev, &mut params);

        if result != 0 {
            return Err(anyhow!("GetDeviceParams() a échoué : {}", result));
        }

        if params.is_null() {
            return Err(anyhow!(
                "GetDeviceParams() a réussi mais retourne un pointeur NULL"
            ));
        }

        println!();
        println!("Paramètres SDRplay récupérés.");

        println!("  deviceParams : {:p}", params);

        println!("  devParams    : {:p}", (*params).dev_params);

        println!("  rxChannelA   : {:p}", (*params).rx_channel_a);

        println!("  rxChannelB   : {:p}", (*params).rx_channel_b);

        let mut sample_rate = 2_000_000u64;

        let mut frequency = 200_000_000u64;

        if !(*params).dev_params.is_null() {
            let dev_params = &*(*params).dev_params;

            sample_rate = dev_params.fs_freq.fs_hz.round() as u64;

            println!();
            println!("Paramètres Device :");

            println!("  sample rate : {:.0} Hz", dev_params.fs_freq.fs_hz);

            println!("  PPM         : {:.3}", dev_params.ppm);

            println!("  transfert   : {}", dev_params.mode);

            println!("  samples/pkt : {}", dev_params.samples_per_pkt);

            println!(
                "  RF notch    : {}",
                dev_params.rsp1a_params.rf_notch_enable
            );

            println!(
                "  DAB notch   : {}",
                dev_params.rsp1a_params.rf_dab_notch_enable
            );
        }

        if !(*params).rx_channel_a.is_null() {
            let rx = &*(*params).rx_channel_a;

            frequency = rx.tuner_params.rf_freq.rf_hz.round() as u64;

            println!();
            println!("Paramètres RX A :");

            println!("  fréquence   : {:.0} Hz", rx.tuner_params.rf_freq.rf_hz);

            println!("  bandwidth   : {} kHz", rx.tuner_params.bw_type);

            println!("  IF          : {} kHz", rx.tuner_params.if_type);

            println!("  LO mode     : {}", rx.tuner_params.lo_mode);

            println!("  Gain reduction : {} dB", rx.tuner_params.gain.gr_db);

            println!("  LNA state : {}", rx.tuner_params.gain.lna_state);

            println!("  Min gain reduction : {} dB", rx.tuner_params.gain.min_gr);

            println!(
                "  Gain values : curr={:.2} max={:.2} min={:.2}",
                rx.tuner_params.gain.gain_vals.curr,
                rx.tuner_params.gain.gain_vals.max,
                rx.tuner_params.gain.gain_vals.min
            );

            println!("  LNA state   : {}", rx.tuner_params.gain.lna_state);

            println!("  AGC         : {}", rx.ctrl_params.agc.enable);

            println!("  AGC setpoint: {} dBFS", rx.ctrl_params.agc.set_point_dbfs);

            println!("  DC offset   : {}", rx.ctrl_params.dc_offset.dc_enable);

            println!("  IQ balance  : {}", rx.ctrl_params.dc_offset.iq_enable);

            println!("  bias-T      : {}", rx.rsp1a_tuner_params.bias_t_enable);
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

        IQ_BLOCK_COUNT.store(0, Ordering::Relaxed);

        let context = self
            .callback_context
            .as_mut()
            .ok_or_else(|| anyhow!("Contexte callback IQ absent"))?;

        let context_ptr = context.as_mut() as *mut CallbackContext as *mut c_void;

        let mut callbacks = SdrplayCallbackFns {
            stream_a_cb_fn: Some(stream_a_callback),

            stream_b_cb_fn: None,

            event_cb_fn: Some(event_callback),
        };

        println!();
        println!("Initialisation du RSP1B...");

        let result = sdrplay_api_Init(dev, &mut callbacks, context_ptr);

        if result != 0 {
            return Err(anyhow!("sdrplay_api_Init() a échoué : {}", result));
        }

        self.initialized = true;

        let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();
        let verify_result = sdrplay_api_GetDeviceParams(dev, &mut verify_params);

        if verify_result == 0 && !verify_params.is_null() {
            if !(*verify_params).dev_params.is_null() {
                let dp = &*(*verify_params).dev_params;
                println!(
                    ">>> POST-INIT DEV : mode={} samplesPkt={} fs={:.0}",
                    dp.mode,
                    dp.samples_per_pkt,
                    dp.fs_freq.fs_hz
                );
            }

            if !(*verify_params).rx_channel_a.is_null() {
                let rx = &*(*verify_params).rx_channel_a;
                println!(
                    ">>> POST-INIT RX : freq={:.0} IQenable={} DCenable={} AGC={} DECenable={} DECfactor={} WBS={} ADSB={}",
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

        println!("RSP1B initialisé.");

        Ok(())
    }

    pub fn set_frequency(&mut self, frequency_hz: u64) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay non connecté"));
        }

        if !self.device_selected {
            return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B non initialisé"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() a échoué : {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() retourne NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA est NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            rx.tuner_params.rf_freq.rf_hz = frequency_hz as f64;

            const UPDATE_TUNER_FRF: c_int = 0x00020000;
            const UPDATE_TUNER_GR: c_int = 0x00008000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            // Un LNAstate qui n'existe pas dans la nouvelle bande est refusé
            // par le service (OutOfRange) : on le ramène au maximum de la
            // bande dans la même mise à jour.
            let new_band = Band::from_hz(frequency_hz);
            let mut reason = UPDATE_TUNER_FRF;

            if rx.tuner_params.gain.lna_state > new_band.max_lna_state() {
                println!(
                    ">>> LNAstate {} invalide en bande {:?} : ramené à {}",
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
                return Err(anyhow!("sdrplay_api_Update(FRF) a échoué : {}", result));
            }

            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            if sdrplay_api_GetDeviceParams(device.dev, &mut verify_params) == 0
                && !verify_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let verify_rx = &*(*verify_params).rx_channel_a;

                println!(
                    ">>> RF VERIFY : RF={:.0} Hz IF={} kHz BW={} kHz LO={} LNA={} gRdB={} curr={:.2} dB",
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

            println!("Fréquence RSP1B réglée à {} Hz", frequency_hz);
        }

        Ok(())
    }

    pub fn set_sample_rate(&mut self, sample_rate_hz: u32) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay non connecté"));
        }

        if !self.device_selected {
            return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B non initialisé"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() a échoué : {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() retourne NULL"));
            }

            if (*params).dev_params.is_null() {
                return Err(anyhow!("devParams est NULL"));
            }

            let dev_params = &mut *(*params).dev_params;

            dev_params.fs_freq.fs_hz = sample_rate_hz as f64;

            const UPDATE_DEV_FS: c_int = 0x00000001;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_DEV_FS, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(FS) a échoué : {}", result));
            }

            // Diagnostic : relire les paramètres après Update(FS)
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();
            let verify_result =
                sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

            if verify_result == 0
                && !verify_params.is_null()
                && !(*verify_params).dev_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let verify_dev = &*(*verify_params).dev_params;
                let verify_rx = &*(*verify_params).rx_channel_a;

                println!(
                    ">>> FS VERIFY : fsHz={} samplesPerPkt={} IF={} kHz BW={}",
                    verify_dev.fs_freq.fs_hz,
                    verify_dev.samples_per_pkt,
                    verify_rx.tuner_params.if_type,
                    verify_rx.tuner_params.bw_type
                );
            } else {
                println!(">>> FS VERIFY : GetDeviceParams() invalide après Update");
            }

            if let Some(context) = self.callback_context.as_ref() {
                context
                    .sample_rate
                    .store(sample_rate_hz as u64, Ordering::Relaxed);
            }

            println!("Sample rate RSP1B réglé à {} Hz", sample_rate_hz);
        }

        Ok(())
    }

    pub fn set_bandwidth(&mut self, bandwidth_hz: u32) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay non connecté"));
        }

        if !self.device_selected {
            return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B non initialisé"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

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
                return Err(anyhow!(
                    "Bande passante non supportée : {} Hz",
                    bandwidth_hz
                ));
            }
        };

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() a échoué : {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() retourne NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA est NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            // Les clients rtl_tcp renvoient la bande passante à chaque
            // connexion : inutile de reprogrammer le RSP si elle est déjà bonne.
            if rx.tuner_params.bw_type == bw_type {
                return Ok(());
            }

            rx.tuner_params.bw_type = bw_type;

            const UPDATE_TUNER_BW_TYPE: c_int = 0x00040000;

            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, UPDATE_TUNER_BW_TYPE, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update(BW_TYPE) a échoué : {}", result));
            }
let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

let verify_result =
    sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

if verify_result != 0 {
    return Err(anyhow!(
        "GetDeviceParams() après BW a échoué : {}",
        verify_result
    ));
}

if verify_params.is_null() {
    return Err(anyhow!(
        "GetDeviceParams() après BW retourne NULL"
    ));
}

if (*verify_params).rx_channel_a.is_null() {
    return Err(anyhow!(
        "rxChannelA après BW est NULL"
    ));
}

let verify_rx = &*(*verify_params).rx_channel_a;

println!(
    "Bandwidth après Update = {} kHz",
    verify_rx.tuner_params.bw_type
);
            println!("Bande passante RSP1B réglée à {} Hz", bandwidth_hz);
        }

        Ok(())
    }
    pub fn set_if_type(&mut self, if_type: IfType) -> Result<()> {
    if !self.connected {
        return Err(anyhow!("SDRplay non connecté"));
    }

    if !self.device_selected {
        return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
    }

    if !self.initialized {
        return Err(anyhow!("RSP1B non initialisé"));
    }

    let if_khz: c_int = match if_type {
        IfType::Zero => 0,
        IfType::KHz450 => 450,
        IfType::KHz1620 => 1620,
        IfType::KHz2048 => 2048,
    };

    let device = self
        .selected_device
        .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

    unsafe {
        let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

        if result != 0 {
            return Err(anyhow!("GetDeviceParams() a échoué : {}", result));
        }

        if params.is_null() {
            return Err(anyhow!("GetDeviceParams() retourne NULL"));
        }

        if (*params).rx_channel_a.is_null() {
            return Err(anyhow!("rxChannelA est NULL"));
        }

        let rx = &mut *(*params).rx_channel_a;

        rx.tuner_params.if_type = if_khz;

        const UPDATE_TUNER_IF_TYPE: c_int = 0x00080000;
        const TUNER_A: c_int = 1;
        const EXT1_NONE: c_int = 0;

        let result = sdrplay_api_Update(
            device.dev,
            TUNER_A,
            UPDATE_TUNER_IF_TYPE,
            EXT1_NONE,
        );

        if result != 0 {
            return Err(anyhow!(
                "sdrplay_api_Update(IF_TYPE) a échoué : {}",
                result
            ));
        }

        // Relecture des paramètres après Update
        let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let verify_result =
            sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

        if verify_result != 0 {
            return Err(anyhow!(
                "GetDeviceParams() après Update IF a échoué : {}",
                verify_result
            ));
        }

        if verify_params.is_null() {
            return Err(anyhow!(
                "GetDeviceParams() après Update IF retourne NULL"
            ));
        }

        if (*verify_params).rx_channel_a.is_null() {
            return Err(anyhow!(
                "rxChannelA après Update IF est NULL"
            ));
        }

        let verify_rx = &*(*verify_params).rx_channel_a;

        println!(
            "IF après Update = {} kHz",
            verify_rx.tuner_params.if_type
        );

        println!("IF RSP1B réglé à {} kHz", if_khz);
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
            return Err(anyhow!("SDRplay non connecté"));
        }

        if !self.device_selected {
            return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B non initialisé"));
        }

        let lo_mode: c_int = match lo_mode {
            1 | 2 | 3 | 4 => lo_mode,

            _ => {
                return Err(anyhow!(
                    "Mode LO non supporté : {}",
                    lo_mode
                ));
            }
        };

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!("GetDeviceParams() a échoué : {}", result));
            }

            if params.is_null() {
                return Err(anyhow!("GetDeviceParams() retourne NULL"));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!("rxChannelA est NULL"));
            }

            let rx = &mut *(*params).rx_channel_a;

            rx.tuner_params.lo_mode = lo_mode;

            const UPDATE_TUNER_LO_MODE: c_int = 0x00200000;
            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(
                device.dev,
                TUNER_A,
                UPDATE_TUNER_LO_MODE,
                EXT1_NONE,
            );

if result != 0 {
    return Err(anyhow!(
        "sdrplay_api_Update(LO_MODE) a échoué : {}",
        result
    ));
}

// Relecture des paramètres après Update
let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

let verify_result =
    sdrplay_api_GetDeviceParams(device.dev, &mut verify_params);

if verify_result != 0 {
    return Err(anyhow!(
        "GetDeviceParams() après Update LO a échoué : {}",
        verify_result
    ));
}

if verify_params.is_null() {
    return Err(anyhow!(
        "GetDeviceParams() après Update LO retourne NULL"
    ));
}

if (*verify_params).rx_channel_a.is_null() {
    return Err(anyhow!(
        "rxChannelA après Update LO est NULL"
    ));
}

let verify_rx = &*(*verify_params).rx_channel_a;

println!(
    "LO après Update = {}",
    verify_rx.tuner_params.lo_mode
);

println!("LO mode RSP1B réglé à {}", lo_mode);
        }

        Ok(())
    }
/// Gain « rtl_tcp » (commande 0x04, échelle R820T 0..49.6 dB) -> pas de gain.
pub fn set_gain(&mut self, gain_db: f32) -> Result<()> {
    if !gain_db.is_finite() {
        return Err(anyhow!("Gain invalide : {}", gain_db));
    }

    let tenths = (gain_db.max(0.0) * 10.0).round() as u32;

    self.set_gain_index(gain::index_from_tenths_db(tenths))
}

/// Applique un pas de gain (0..=28) avec la table de la bande courante :
/// LNAstate et gRdB sont envoyés ensemble dans une seule mise à jour.
pub fn set_gain_index(&mut self, index: usize) -> Result<()> {
    if !self.connected {
        return Err(anyhow!("SDRplay non connecté"));
    }

    if !self.device_selected {
        return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
    }

    if !self.initialized {
        return Err(anyhow!("SDRplay non initialisé"));
    }

    let index = index.min(gain::GAIN_STEPS - 1);
    let band = self.band;
    let (lna_state, gr_db) = gain::settings(band, index);

    let device = self.selected_device.ok_or_else(|| {
        anyhow!("Périphérique SDRplay sélectionné introuvable")
    })?;

    unsafe {
        let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

        if result != 0 {
            return Err(anyhow!("sdrplay_api_GetDeviceParams a échoué : {}", result));
        }

        if params.is_null() || (*params).rx_channel_a.is_null() {
            return Err(anyhow!("Paramètres du canal RX A indisponibles"));
        }

        let rx = &mut *(*params).rx_channel_a;

        rx.tuner_params.gain.lna_state = lna_state;
        rx.tuner_params.gain.gr_db = gr_db;

        const UPDATE_TUNER_GR: c_int = 0x00008000;
        const TUNER_A: c_int = 1;
        const EXT1_NONE: c_int = 0;

        let result =
            sdrplay_api_Update(device.dev, TUNER_A, UPDATE_TUNER_GR, EXT1_NONE);

        if result != 0 {
            return Err(anyhow!(
                "sdrplay_api_Update(GR) a échoué (bande {:?}, pas {}, LNA={}, gRdB={}) : {}",
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

            println!(
                ">>> GAIN : bande={:?} pas={}/{} -> LNA={} gRdB={} | réel LNA={} gRdB={} curr={:.2} dB (AGC={})",
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

    // Comme rsp_tcp : après un changement de LNAstate, on ré-applique
    // la configuration AGC (l'AGC reprend la main sur gRdB).
    if self.agc_on {
        self.set_agc(true)?;
    }

    Ok(())
}

/// Réglage d'un paramètre propre au RSP1A/RSP1B (bias-T, notch, PPM) :
/// modifie la structure de paramètres, puis envoie l'Update correspondant.
/// `apply` renvoie `true` si la valeur a réellement changé : sinon aucun
/// Update n'est envoyé (les clients rtl_tcp renvoient ces commandes à chaque
/// connexion, même à leur valeur par défaut).
fn update_rsp1_setting(
    &mut self,
    label: &str,
    reason: c_int,
    apply: impl FnOnce(&mut SdrplayDevParams, &mut SdrplayRxChannelParams) -> bool,
) -> Result<bool> {
    if !self.connected {
        return Err(anyhow!("SDRplay non connecté"));
    }

    if !self.device_selected {
        return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
    }

    let device = self.selected_device.ok_or_else(|| {
        anyhow!("Périphérique SDRplay sélectionné introuvable")
    })?;

    unsafe {
        let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let result = sdrplay_api_GetDeviceParams(device.dev, &mut params);

        if result != 0 {
            return Err(anyhow!("sdrplay_api_GetDeviceParams a échoué : {}", result));
        }

        if params.is_null()
            || (*params).dev_params.is_null()
            || (*params).rx_channel_a.is_null()
        {
            return Err(anyhow!("Paramètres du périphérique indisponibles"));
        }

        let dev = &mut *(*params).dev_params;
        let rx = &mut *(*params).rx_channel_a;

        if !apply(dev, rx) {
            return Ok(false);
        }

        // Avant sdrplay_api_Init, la structure sera lue à l'initialisation :
        // pas d'Update possible ni nécessaire.
        if self.initialized {
            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result = sdrplay_api_Update(device.dev, TUNER_A, reason, EXT1_NONE);

            if result != 0 {
                return Err(anyhow!("sdrplay_api_Update({}) a échoué : {}", label, result));
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
        println!(">>> Bias-T : {}", if enabled { "activé (alimentation antenne)" } else { "désactivé" });
    }

    Ok(())
}

pub fn set_rf_notch(&mut self, enabled: bool) -> Result<()> {
    // sdrplay_api_Update_Rsp1a_RfNotchControl : filtre réjecteur FM (88-108 MHz)
    let changed = self.update_rsp1_setting("RF notch (FM)", 0x0000_0020, |dev, _| {
        let value = enabled as u8;
        let changed = dev.rsp1a_params.rf_notch_enable != value;
        dev.rsp1a_params.rf_notch_enable = value;
        changed
    })?;

    if changed {
        println!(">>> RF notch (FM) : {}", if enabled { "activé" } else { "désactivé" });
    }

    Ok(())
}

pub fn set_dab_notch(&mut self, enabled: bool) -> Result<()> {
    // sdrplay_api_Update_Rsp1a_RfDabNotchControl : filtre réjecteur DAB (bande III)
    if enabled && self.band == Band::Band3 {
        println!(
            ">>> ATTENTION : le notch DAB atténue la bande III (174-240 MHz) : \
             la réception DAB sera dégradée"
        );
    }

    let changed = self.update_rsp1_setting("DAB notch", 0x0000_0040, |dev, _| {
        let value = enabled as u8;
        let changed = dev.rsp1a_params.rf_dab_notch_enable != value;
        dev.rsp1a_params.rf_dab_notch_enable = value;
        changed
    })?;

    if changed {
        println!(">>> DAB notch : {}", if enabled { "activé" } else { "désactivé" });
    }

    Ok(())
}

pub fn set_ppm(&mut self, ppm: f64) -> Result<()> {
    if !ppm.is_finite() || ppm.abs() > 1000.0 {
        return Err(anyhow!("Correction PPM invalide : {}", ppm));
    }

    // sdrplay_api_Update_Dev_Ppm
    let changed = self.update_rsp1_setting("PPM", 0x0000_0002, |dev, _| {
        let changed = (dev.ppm - ppm).abs() > 1e-9;
        dev.ppm = ppm;
        changed
    })?;

    if changed {
        println!(">>> Correction fréquence : {:.3} ppm", ppm);
    }

    Ok(())
}

/// À appeler régulièrement depuis la boucle principale (hors callbacks API) :
/// acquitte les messages de surcharge et signale les surcharges ADC.
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
                sdrplay_api_Update(
                    device.dev,
                    TUNER_A,
                    UPDATE_CTRL_OVERLOAD_ACK,
                    EXT1_NONE,
                )
            };

            if result != 0 {
                println!("Acquittement surcharge : Update a échoué ({})", result);
            }
        }
    }

    if let Some(context) = self.callback_context.as_ref() {
        let events = context.overload_events.load(Ordering::Relaxed);

        if events > 0
            && self.last_overload_log.elapsed() >= std::time::Duration::from_secs(2)
        {
            context.overload_events.store(0, Ordering::Relaxed);
            self.last_overload_log = Instant::now();

            println!(
                "!!! SURCHARGE ADC ({} fois) bande={:?} pas de gain={} : réduire le gain \
                 (LNAstate plus élevé) ou activer l'AGC",
                events, self.band, self.gain_index
            );
        }
    }
}
pub fn set_gr_db_test(&mut self, gr_db: c_int) -> Result<()> {
    if !self.connected {
        return Err(anyhow!("SDRplay non connecté"));
    }

    if !self.device_selected {
        return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
    }

    if !self.initialized {
        return Err(anyhow!("RSP1B non initialisé"));
    }

    let device = self
        .selected_device
        .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

    unsafe {
        let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

        let result =
            sdrplay_api_GetDeviceParams(device.dev, &mut params);

        if result != 0 {
            return Err(anyhow!(
                "GetDeviceParams() a échoué : {}",
                result
            ));
        }

        if params.is_null() {
            return Err(anyhow!(
                "GetDeviceParams() a retourné NULL"
            ));
        }

        if (*params).rx_channel_a.is_null() {
            return Err(anyhow!("rxChannelA est NULL"));
        }

        let rx = &mut *(*params).rx_channel_a;

        rx.tuner_params.gain.gr_db = gr_db;

        const UPDATE_TUNER_GR: c_int = 0x00008000;
        const TUNER_A: c_int = 1;
        const EXT1_NONE: c_int = 0;

        let result = sdrplay_api_Update(
            device.dev,
            TUNER_A,
            UPDATE_TUNER_GR,
            EXT1_NONE,
        );

        if result != 0 {
            return Err(anyhow!(
                "sdrplay_api_Update(GR) a échoué : {}",
                result
            ));
        }


        let mut stable_count = 0;
        let mut last_gr_db = -1;

        for _ in 0..100 {
            std::thread::sleep(
                std::time::Duration::from_millis(20)
            );

            let mut check_params: *mut SdrplayDeviceParams =
                std::ptr::null_mut();

            let check_result =
                sdrplay_api_GetDeviceParams(
                    device.dev,
                    &mut check_params
                );

            if check_result != 0
                || check_params.is_null()
                || (*check_params).rx_channel_a.is_null()
            {
                continue;
            }

            let check_rx = &*(*check_params).rx_channel_a;
            let current_gr_db =
                check_rx.tuner_params.gain.gr_db;

            if current_gr_db == last_gr_db {
                stable_count += 1;
            } else {
                stable_count = 0;
                last_gr_db = current_gr_db;
            }

            if stable_count >= 5 {
                println!(
                    "STABLE : gRdB={} après environ {} ms",
                    current_gr_db,
                    stable_count * 20
                );
                break;
            }
        }

        let mut params_after: *mut SdrplayDeviceParams =
            std::ptr::null_mut();

        let result =
            sdrplay_api_GetDeviceParams(device.dev, &mut params_after);

        if result != 0 {
            return Err(anyhow!(
                "GetDeviceParams() après Update a échoué : {}",
                result
            ));
        }

        if params_after.is_null()
            || (*params_after).rx_channel_a.is_null()
        {
            return Err(anyhow!(
                "rxChannelA après Update est NULL"
            ));
        }

        let rx_after = &*(*params_after).rx_channel_a;

        println!(
            "GR TEST : demandé={} réel_gRdB={} LNA={} curr={:.2} max={:.2} min={:.2}",
            gr_db,
            rx_after.tuner_params.gain.gr_db,
            rx_after.tuner_params.gain.lna_state,
            rx_after.tuner_params.gain.gain_vals.curr,
            rx_after.tuner_params.gain.gain_vals.max,
            rx_after.tuner_params.gain.gain_vals.min
        );
    }

    Ok(())
}
    pub fn set_lna_state(&mut self, lna_state: u8) -> Result<()> {
        if !self.connected {
            return Err(anyhow!("SDRplay non connecté"));
        }

        if !self.device_selected {
            return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B non initialisé"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result =
                sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() a échoué : {}",
                    result
                ));
            }

            if params.is_null() {
                return Err(anyhow!(
                    "GetDeviceParams() retourne NULL"
                ));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!(
                    "rxChannelA est NULL"
                ));
            }

            let rx = &mut *(*params).rx_channel_a;

            println!(
                "LNA AVANT : lna_state={} gr_db={} sync_update={}",
                rx.tuner_params.gain.lna_state,
                rx.tuner_params.gain.gr_db,

                rx.tuner_params.gain.sync_update
            );

            rx.tuner_params.gain.lna_state = lna_state;

            println!(
                "LNA DEMANDE : lna_state={} gr_db={}",
                rx.tuner_params.gain.lna_state,
                rx.tuner_params.gain.gr_db
            );

            const UPDATE_TUNER_GR: c_int = 0x00008000;
            const TUNER_A: c_int = 1;
            const EXT1_NONE: c_int = 0;

            let result =
                sdrplay_api_Update(
                    device.dev,
                    TUNER_A,
                    UPDATE_TUNER_GR,
                    EXT1_NONE,
                );

            println!("LNA UPDATE result={}", result);

            if result != 0 {
                return Err(anyhow!(
                    "sdrplay_api_Update(LNA) a échoué : {}",
                    result
                ));
            }

            /*
             * Relire les paramètres après Update pour vérifier
             * ce que l'API SDRplay a réellement conservé.
             */
            let mut params_after: *mut SdrplayDeviceParams =
                std::ptr::null_mut();

            let result =
                sdrplay_api_GetDeviceParams(
                    device.dev,
                    &mut params_after,
                );

            if result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() après Update a échoué : {}",
                    result
                ));
            }

            if params_after.is_null() {
                return Err(anyhow!(
                    "GetDeviceParams() après Update retourne NULL"
                ));
            }

            if (*params_after).rx_channel_a.is_null() {
                return Err(anyhow!(
                    "rxChannelA après Update est NULL"
                ));
            }

            let rx_after = &*(*params_after).rx_channel_a;

            println!(
                "LNA APRÈS : lna_state={} gr_db={} sync_update={} curr={} max={} min={}",
                rx_after.tuner_params.gain.lna_state,
                rx_after.tuner_params.gain.gr_db,
                rx_after.tuner_params.gain.sync_update,
                rx_after.tuner_params.gain.gain_vals.curr,
                rx_after.tuner_params.gain.gain_vals.max,
                rx_after.tuner_params.gain.gain_vals.min
            );

            println!(
                "État LNA RSP1B demandé : {}",
                lna_state
            );
        }

        Ok(())
    }

    pub fn set_agc(&mut self, enabled: bool) -> Result<()> {

        if !self.connected {
            return Err(anyhow!("SDRplay non connecté"));
        }

        if !self.device_selected {
            return Err(anyhow!("Aucun périphérique SDRplay sélectionné"));
        }

        if !self.initialized {
            return Err(anyhow!("RSP1B non initialisé"));
        }

        let device = self
            .selected_device
            .ok_or_else(|| anyhow!("Périphérique SDRplay absent"))?;

        unsafe {
            let mut params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            let result =
                sdrplay_api_GetDeviceParams(device.dev, &mut params);

            if result != 0 {
                return Err(anyhow!(
                    "GetDeviceParams() a échoué : {}",
                    result
                ));
            }

            if params.is_null() {
                return Err(anyhow!(
                    "GetDeviceParams() retourne NULL"
                ));
            }

            if (*params).rx_channel_a.is_null() {
                return Err(anyhow!(
                    "rxChannelA est NULL"
                ));
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
    // Même réglage que rsp_tcp (SDRplay) : schéma d'AGC « CTRL_EN » lent
    // (constantes de temps 500 ms), adapté à un flux large bande comme le DAB,
    // consigne -30 dBFS (plage valide -72..-20). L'ancien « 1 » = AGC_100HZ,
    // boucle rapide (100 Hz) avec la consigne par défaut -60 dBFS.
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

            let result = sdrplay_api_Update(
                device.dev,
                TUNER_A,
                UPDATE_CTRL_AGC,
                EXT1_NONE,
            );

            if result != 0 {
                return Err(anyhow!(
                    "sdrplay_api_Update(AGC) a échoué : {}",
                    result
                ));
            }

            // Vérification de la valeur AGC réellement conservée par SDRplay
            let mut verify_params: *mut SdrplayDeviceParams = std::ptr::null_mut();

            if sdrplay_api_GetDeviceParams(device.dev, &mut verify_params) == 0
                && !verify_params.is_null()
                && !(*verify_params).rx_channel_a.is_null()
            {
                let verify_rx = &*(*verify_params).rx_channel_a;

                println!(
                    ">>> AGC VERIFY : demandé={} agc.enable={}",
                    enabled,
                    verify_rx.ctrl_params.agc.enable
                );
            }

            println!(
                "AGC RSP1B {}",
                if enabled {
                    "activé"
                } else {
                    "désactivé"
                }
            );
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
                    eprintln!("sdrplay_api_Uninit() a échoué : {}", result);
                } else {
                    println!("RSP1B désinitialisé.");
                }
            }

            self.initialized = false;
        }

        if let Some(device) = self.selected_device.take() {
            let result = sdrplay_api_ReleaseDevice(device.dev);

            if result != 0 {
                eprintln!("ReleaseDevice() a échoué : {}", result);
            } else {
                println!("RSP1B libéré.");
            }
        }

        let result = sdrplay_api_Close();

        if result != 0 {
            eprintln!("sdrplay_api_Close() a échoué : {}", result);
        } else {
            println!("Session API SDRplay fermée.");
        }
    }

    self.connected = false;
    self.device_selected = false;
    self.initialized = false;
    self.serial = None;
    self.callback_context = None;
}

pub fn is_connected(&self) -> bool {
    self.connected
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
