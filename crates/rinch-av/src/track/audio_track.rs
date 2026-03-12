//! AudioTrack — pub/sub channel for audio samples with multiple consumer support.

use super::{TrackHandle, TrackId, TrackKind, TrackState, generate_track_id};
use rinch_core::Signal;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// A chunk of interleaved audio samples.
#[derive(Clone, Debug)]
pub struct AudioChunk {
    /// Interleaved f32 samples in [-1.0, 1.0].
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u16,
    /// Microseconds since track creation.
    pub timestamp_us: u64,
}

type Subscribers = Arc<Mutex<Vec<mpsc::Sender<AudioChunk>>>>;

/// A broadcast channel for audio samples.
///
/// Subscribers receive cloned `AudioChunk`s. Dead subscribers (disconnected
/// receivers) are pruned automatically on the next `push`.
pub struct AudioTrack {
    pub handle: TrackHandle,
    subscribers: Subscribers,
    // Keep the input stream alive for microphone tracks.
    _input_stream: Option<crate::audio::AudioInputStream>,
}

/// A receiver for audio chunks from an [`AudioTrack`].
pub struct AudioTrackReader {
    rx: mpsc::Receiver<AudioChunk>,
}

impl AudioTrack {
    /// Create a new audio track with the given label.
    pub fn new(label: &str) -> Self {
        Self {
            handle: TrackHandle {
                id: TrackId(generate_track_id()),
                kind: TrackKind::Audio,
                label: label.to_string(),
                enabled: Signal::new(true),
                state: Signal::new(TrackState::Live),
            },
            subscribers: Arc::new(Mutex::new(Vec::new())),
            _input_stream: None,
        }
    }

    /// Create an audio track from the default microphone.
    pub fn from_microphone(config: crate::audio::AudioInputConfig) -> Result<Self, crate::AvError> {
        let mut track = Self::new("microphone");
        let subscribers = track.subscribers.clone();
        let sample_rate = config.sample_rate;
        let channels = config.channels;
        let start = std::time::Instant::now();

        let stream = crate::audio::open_audio_input(config, move |samples| {
            push_to_subscribers(
                &subscribers,
                samples,
                sample_rate,
                channels,
                start.elapsed().as_micros() as u64,
            );
        })?;

        track._input_stream = Some(stream);
        Ok(track)
    }

    /// Create an audio track from a specific microphone device.
    pub fn from_microphone_on(
        device: &crate::DeviceId,
        config: crate::audio::AudioInputConfig,
    ) -> Result<Self, crate::AvError> {
        let mut track = Self::new("microphone");
        let subscribers = track.subscribers.clone();
        let sample_rate = config.sample_rate;
        let channels = config.channels;
        let start = std::time::Instant::now();

        let stream = crate::audio::open_audio_input_on(device, config, move |samples| {
            push_to_subscribers(
                &subscribers,
                samples,
                sample_rate,
                channels,
                start.elapsed().as_micros() as u64,
            );
        })?;

        track._input_stream = Some(stream);
        Ok(track)
    }

    /// Subscribe to receive audio chunks from this track.
    pub fn subscribe(&self) -> AudioTrackReader {
        let (tx, rx) = mpsc::channel();
        self.subscribers.lock().unwrap().push(tx);
        AudioTrackReader { rx }
    }

    /// Pipe audio to the default output device (speakers).
    ///
    /// Returns an `AudioOutputStream` that keeps playback alive until dropped.
    pub fn play_to_output(
        &self,
        config: crate::audio::AudioOutputConfig,
    ) -> Result<crate::audio::AudioOutputStream, crate::AvError> {
        let reader = self.subscribe();
        let mut pending: Vec<f32> = Vec::new();

        crate::audio::open_audio_output(config, move |buf| {
            // Drain any available chunks into our pending buffer.
            while let Some(chunk) = reader.try_recv() {
                pending.extend_from_slice(&chunk.samples);
            }

            // Fill the output buffer from pending samples.
            let to_copy = buf.len().min(pending.len());
            buf[..to_copy].copy_from_slice(&pending[..to_copy]);
            pending.drain(..to_copy);

            // Zero-fill any remaining buffer space.
            for s in &mut buf[to_copy..] {
                *s = 0.0;
            }
        })
    }

    /// Push samples into the track manually (for non-device sources).
    pub fn push(&self, samples: &[f32], sample_rate: u32, channels: u16, timestamp_us: u64) {
        push_to_subscribers(
            &self.subscribers,
            samples,
            sample_rate,
            channels,
            timestamp_us,
        );
    }

    /// Get the track handle.
    pub fn track(&self) -> &TrackHandle {
        &self.handle
    }
}

impl AudioTrackReader {
    /// Blocking receive of the next audio chunk.
    pub fn recv(&self) -> Option<AudioChunk> {
        self.rx.recv().ok()
    }

    /// Non-blocking receive of the next audio chunk.
    pub fn try_recv(&self) -> Option<AudioChunk> {
        self.rx.try_recv().ok()
    }
}

/// Push samples to all active subscribers, pruning dead ones.
fn push_to_subscribers(
    subscribers: &Subscribers,
    samples: &[f32],
    sample_rate: u32,
    channels: u16,
    timestamp_us: u64,
) {
    let chunk = AudioChunk {
        samples: samples.to_vec(),
        sample_rate,
        channels,
        timestamp_us,
    };

    let mut subs = subscribers.lock().unwrap();
    subs.retain(|tx| tx.send(chunk.clone()).is_ok());
}
