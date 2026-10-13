//! Memory layout of the hand-written declarations of the SDRplay API structures.
//!
//! The API is a C library and its structures are read and written in place, so
//! a wrong size or offset would silently corrupt the receiver's settings. The
//! expected values below were checked against the real `sdrplay_api*.h`
//! headers by the SDroxide project, which pins them in its own FFI module; this
//! test keeps ours identical.

use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn device_structure_matches_the_api() {
    assert_eq!(size_of::<SdrplayDevice>(), 96);
    assert_eq!(offset_of!(SdrplayDevice, hw_ver), 64);
    assert_eq!(offset_of!(SdrplayDevice, tuner), 68);
    assert_eq!(offset_of!(SdrplayDevice, rsp_duo_mode), 72);
    assert_eq!(offset_of!(SdrplayDevice, valid), 76);
    assert_eq!(offset_of!(SdrplayDevice, rsp_duo_sample_freq), 80);
    assert_eq!(offset_of!(SdrplayDevice, dev), 88);
}

#[test]
fn device_parameters_match_the_api() {
    assert_eq!(size_of::<SdrplayDeviceParams>(), 24);

    assert_eq!(size_of::<SdrplayDevParams>(), 64);
    assert_eq!(offset_of!(SdrplayDevParams, fs_freq), 8);
    assert_eq!(offset_of!(SdrplayDevParams, mode), 36);
    assert_eq!(offset_of!(SdrplayDevParams, rsp1a_params), 44);
    assert_eq!(offset_of!(SdrplayDevParams, rsp_duo_params), 48);
    assert_eq!(offset_of!(SdrplayDevParams, rsp_dx_params), 52);
}

#[test]
fn channel_parameters_match_the_api() {
    assert_eq!(size_of::<SdrplayRxChannelParams>(), 144);
    assert_eq!(offset_of!(SdrplayRxChannelParams, ctrl_params), 72);
    assert_eq!(offset_of!(SdrplayRxChannelParams, rsp1a_tuner_params), 104);
    assert_eq!(offset_of!(SdrplayRxChannelParams, rsp2_tuner_params), 108);
    assert_eq!(
        offset_of!(SdrplayRxChannelParams, rsp_duo_tuner_params),
        124
    );
    assert_eq!(offset_of!(SdrplayRxChannelParams, rsp_dx_tuner_params), 140);

    assert_eq!(size_of::<SdrplayTunerParams>(), 72);
    assert_eq!(offset_of!(SdrplayTunerParams, gain), 12);
    assert_eq!(offset_of!(SdrplayTunerParams, rf_freq), 40);
    assert_eq!(offset_of!(SdrplayTunerParams, dc_offset_tuner), 56);

    assert_eq!(size_of::<SdrplayGain>(), 24);
    assert_eq!(offset_of!(SdrplayGain, min_gr), 8);
    assert_eq!(offset_of!(SdrplayGain, gain_vals), 12);

    assert_eq!(size_of::<SdrplayControlParams>(), 32);
    assert_eq!(offset_of!(SdrplayControlParams, decimation), 2);
    assert_eq!(offset_of!(SdrplayControlParams, agc), 8);
    assert_eq!(offset_of!(SdrplayControlParams, adsb_mode), 28);

    assert_eq!(size_of::<SdrplayAgc>(), 20);
    assert_eq!(offset_of!(SdrplayAgc, attack_ms), 8);
    assert_eq!(offset_of!(SdrplayAgc, sync_update), 16);
}

#[test]
fn model_specific_parameters_match_the_api() {
    assert_eq!(size_of::<SdrplayRsp2TunerParams>(), 16);
    assert_eq!(size_of::<SdrplayRspDuoTunerParams>(), 16);
    assert_eq!(offset_of!(SdrplayRspDuoTunerParams, reset_slave_flags), 11);
    assert_eq!(size_of::<SdrplayRspDxParams>(), 12);
    assert_eq!(offset_of!(SdrplayRspDxParams, antenna_sel), 4);
    assert_eq!(size_of::<SdrplayRspDxTunerParams>(), 4);
}

#[test]
fn callback_structures_match_the_api() {
    assert_eq!(size_of::<SdrplayStreamCbParams>(), 20);
    assert_eq!(size_of::<SdrplayEventParams>(), 16);
    assert_eq!(size_of::<SdrplayGainCbParam>(), 16);
    assert_eq!(size_of::<SdrplayCallbackFns>(), 24);
}
