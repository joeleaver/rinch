//! MediaStream — bundle of tracks (WebRTC-aligned).

#[cfg(any(feature = "audio", feature = "camera"))]
use crate::device::DeviceId;
#[cfg(any(feature = "audio", feature = "camera"))]
use crate::error::AvError;
#[cfg(any(feature = "audio", feature = "camera"))]
use crate::track::TrackId;
use crate::track::generate_track_id;

#[cfg(feature = "audio")]
use crate::track::AudioTrack;
#[cfg(feature = "camera")]
use crate::track::VideoTrack;

/// A bundle of audio and video tracks, mirroring the WebRTC MediaStream concept.
pub struct MediaStream {
    pub id: String,
    #[cfg(feature = "audio")]
    audio_tracks: Vec<AudioTrack>,
    #[cfg(feature = "camera")]
    video_tracks: Vec<VideoTrack>,
}

impl Default for MediaStream {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaStream {
    /// Create an empty media stream.
    pub fn new() -> Self {
        Self {
            id: generate_track_id(),
            #[cfg(feature = "audio")]
            audio_tracks: Vec::new(),
            #[cfg(feature = "camera")]
            video_tracks: Vec::new(),
        }
    }

    /// Add an audio track.
    #[cfg(feature = "audio")]
    pub fn add_audio_track(&mut self, track: AudioTrack) {
        self.audio_tracks.push(track);
    }

    /// Remove an audio track by ID, returning it if found.
    #[cfg(feature = "audio")]
    pub fn remove_audio_track(&mut self, id: &TrackId) -> Option<AudioTrack> {
        if let Some(pos) = self.audio_tracks.iter().position(|t| t.handle.id == *id) {
            Some(self.audio_tracks.remove(pos))
        } else {
            None
        }
    }

    /// Get all audio tracks.
    #[cfg(feature = "audio")]
    pub fn audio_tracks(&self) -> &[AudioTrack] {
        &self.audio_tracks
    }

    /// Add a video track.
    #[cfg(feature = "camera")]
    pub fn add_video_track(&mut self, track: VideoTrack) {
        self.video_tracks.push(track);
    }

    /// Remove a video track by ID, returning it if found.
    #[cfg(feature = "camera")]
    pub fn remove_video_track(&mut self, id: &TrackId) -> Option<VideoTrack> {
        if let Some(pos) = self.video_tracks.iter().position(|t| t.handle.id == *id) {
            Some(self.video_tracks.remove(pos))
        } else {
            None
        }
    }

    /// Get all video tracks.
    #[cfg(feature = "camera")]
    pub fn video_tracks(&self) -> &[VideoTrack] {
        &self.video_tracks
    }
}

/// Constraints for [`get_user_media`].
#[derive(Default)]
#[cfg(any(feature = "audio", feature = "camera"))]
pub struct MediaConstraints {
    /// Request an audio track.
    pub audio: bool,
    /// Request a video track.
    pub video: bool,
    /// Specific audio input device. `None` uses the default.
    pub audio_device: Option<DeviceId>,
    /// Specific camera device. `None` uses the default.
    pub video_device: Option<DeviceId>,
    /// Camera configuration.
    #[cfg(feature = "camera")]
    pub video_config: crate::camera::CameraConfig,
    /// Audio input configuration.
    #[cfg(feature = "audio")]
    pub audio_config: crate::audio::AudioInputConfig,
}

/// Open requested audio/video devices and return a [`MediaStream`] with the tracks.
///
/// This is the primary entry point for acquiring user media, similar to
/// the WebRTC `getUserMedia()` API.
#[cfg(any(feature = "audio", feature = "camera"))]
pub fn get_user_media(constraints: MediaConstraints) -> Result<MediaStream, AvError> {
    let mut stream = MediaStream::new();

    #[cfg(feature = "audio")]
    if constraints.audio {
        let track = if let Some(ref device) = constraints.audio_device {
            AudioTrack::from_microphone_on(device, constraints.audio_config)?
        } else {
            AudioTrack::from_microphone(constraints.audio_config)?
        };
        stream.add_audio_track(track);
    }

    #[cfg(feature = "camera")]
    if constraints.video {
        let track = if let Some(ref device) = constraints.video_device {
            VideoTrack::from_camera_on(device, constraints.video_config)?
        } else {
            VideoTrack::from_camera(constraints.video_config)?
        };
        stream.add_video_track(track);
    }

    Ok(stream)
}
