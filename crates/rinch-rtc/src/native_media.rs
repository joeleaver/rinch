//! Native media backend: not yet. The web backend lands first (#1916, PR B1);
//! the native one (openh264, Opus, rinch-av capture, cpal playback, the
//! named video sink) is the next PR. Until then every native media call
//! reports `NotSupported`, and `MediaView` shows black.

use rinch_core::dom::{NodeHandle, RenderScope};

use crate::RtcError;
use crate::media::{
    AudioSourceOptions, AudioSourceWriter, CameraOptions, DeviceInfo, Direction, MediaError,
    MediaKind, MediaTrack, MicrophoneOptions, VideoSourceOptions, VideoSourceWriter,
};

fn not_yet() -> MediaError {
    MediaError::NotSupported("native media lands in the next rinch-rtc PR (#1916)".into())
}

fn rtc_not_yet() -> RtcError {
    RtcError::InvalidState(not_yet().to_string())
}

pub(crate) async fn open_camera(_key: u64, _o: CameraOptions) -> Result<(), MediaError> {
    Err(not_yet())
}

pub(crate) async fn open_microphone(_key: u64, _o: MicrophoneOptions) -> Result<(), MediaError> {
    Err(not_yet())
}

pub(crate) fn open_video_source(
    _key: u64,
    _o: VideoSourceOptions,
    _w: VideoSourceWriter,
) -> Result<(), MediaError> {
    Err(not_yet())
}

pub(crate) fn open_audio_source(
    _key: u64,
    _o: AudioSourceOptions,
    _w: AudioSourceWriter,
) -> Result<(), MediaError> {
    Err(not_yet())
}

pub(crate) fn waiting_for_gesture(_key: u64) -> bool {
    false
}

pub(crate) fn set_local_enabled(_key: u64, _enabled: bool) {}

pub(crate) fn stop_local(_key: u64) {}

pub(crate) fn add_track(_p: u64, _l: u64, _s: u64, _t: u64) -> Result<(), RtcError> {
    Err(rtc_not_yet())
}

pub(crate) fn add_transceiver(
    _p: u64,
    _k: MediaKind,
    _d: Direction,
    _s: u64,
    _t: u64,
) -> Result<(), RtcError> {
    Err(rtc_not_yet())
}

pub(crate) fn transceivers(_p: u64) -> Vec<(u64, u64, MediaKind)> {
    Vec::new()
}

pub(crate) async fn replace_track(_p: u64, _s: u64, _l: Option<u64>) -> Result<(), RtcError> {
    Err(rtc_not_yet())
}

pub(crate) fn set_max_bitrate(_p: u64, _s: u64, _bps: Option<u32>) {}

pub(crate) fn transceiver_mid(_p: u64, _t: u64) -> Option<String> {
    None
}

pub(crate) fn transceiver_direction(_p: u64, _t: u64) -> Direction {
    Direction::Inactive
}

pub(crate) fn set_transceiver_direction(_p: u64, _t: u64, _d: Direction) {}

pub(crate) fn transceiver_current_direction(_p: u64, _t: u64) -> Option<Direction> {
    None
}

pub(crate) fn stop_transceiver(_p: u64, _t: u64) {}

pub(crate) fn transceiver_sender(_p: u64, _t: u64) -> Option<u64> {
    None
}

pub(crate) fn set_remote_volume(_p: u64, _k: u64, _v: f32) {}

pub(crate) fn set_remote_playing(_p: u64, _k: u64, _playing: bool) {}

pub(crate) fn on_remote_frame(_p: u64, _k: u64, _cb: crate::media::FrameCallback) {}

pub(crate) fn create_view(scope: &mut RenderScope, view_id: &str, _t: &MediaTrack) -> NodeHandle {
    let el = scope.create_element("div");
    el.set_attribute("data-rtc-view", view_id);
    el
}

pub(crate) async fn devices(_kind: MediaKind) -> Vec<DeviceInfo> {
    Vec::new()
}
