//! Track abstraction — WebRTC-aligned pub/sub for audio and video.

#[cfg(feature = "audio")]
mod audio_track;
#[cfg(feature = "camera")]
mod video_track;

#[cfg(feature = "audio")]
pub use audio_track::*;
#[cfg(feature = "camera")]
pub use video_track::*;

use rinch_core::Signal;

/// Unique track identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

/// The kind of media track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Video,
}

/// Track lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackState {
    /// Track is active and producing data.
    Live,
    /// Track has ended (device disconnected, stream closed).
    Ended,
}

/// Reactive track state exposed to UI.
pub struct TrackHandle {
    pub id: TrackId,
    pub kind: TrackKind,
    pub label: String,
    pub enabled: Signal<bool>,
    pub state: Signal<TrackState>,
}

/// Simple pseudo-UUID (random hex string, no external dep).
pub(crate) fn generate_track_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    );
    let a = h.finish();
    let mut h2 = s.build_hasher();
    h2.write_u64(a.wrapping_mul(6364136223846793005));
    let b = h2.finish();
    format!("{:016x}-{:016x}", a, b)
}
