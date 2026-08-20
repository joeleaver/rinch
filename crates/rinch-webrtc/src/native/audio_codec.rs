//! Opus audio codec integration for WebRTC.
//!
//! Provides encoder/decoder threads that bridge between rinch-av AudioTrack
//! and str0m's media pipeline.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use opus::{Application, Channels, Decoder, Encoder};
use tracing::{debug, trace, warn};

/// Opus encoder configuration.
pub struct OpusEncoderConfig {
    /// Sample rate in Hz (must be 48000 for WebRTC).
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Target bitrate in bits per second.
    pub bitrate: i32,
    /// Frame duration in milliseconds (must be 2.5, 5, 10, 20, 40, or 60).
    pub frame_duration_ms: u32,
}

impl Default for OpusEncoderConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            bitrate: 64000,
            frame_duration_ms: 20,
        }
    }
}

/// Handle to a running Opus encoder thread. Dropping stops encoding.
pub struct OpusEncoderHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for OpusEncoderHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Callback type for encoded Opus packets.
pub type EncodedPacketSink = Box<dyn Fn(&[u8], u64) + Send + 'static>;

/// Start an Opus encoder thread that reads from an AudioTrackReader
/// and calls `sink` with each encoded packet.
///
/// The `sink` receives (encoded_data, timestamp_in_48khz_samples).
pub fn start_opus_encoder(
    reader: rinch_av::AudioTrackReader,
    config: OpusEncoderConfig,
    sink: EncodedPacketSink,
) -> OpusEncoderHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    let thread = std::thread::Builder::new()
        .name("rinch-opus-enc".into())
        .spawn(move || {
            run_encoder(reader, &config, &sink, &stop_clone);
        })
        .expect("failed to spawn opus encoder thread");

    OpusEncoderHandle {
        stop,
        thread: Some(thread),
    }
}

fn run_encoder(
    reader: rinch_av::AudioTrackReader,
    config: &OpusEncoderConfig,
    sink: &EncodedPacketSink,
    stop: &AtomicBool,
) {
    let channels = match config.channels {
        1 => Channels::Mono,
        2 => Channels::Stereo,
        _ => {
            warn!("unsupported channel count {}, using mono", config.channels);
            Channels::Mono
        }
    };

    let mut encoder = match Encoder::new(config.sample_rate, channels, Application::Voip) {
        Ok(e) => e,
        Err(e) => {
            warn!("failed to create Opus encoder: {e}");
            return;
        }
    };

    if let Err(e) = encoder.set_bitrate(opus::Bitrate::Bits(config.bitrate)) {
        warn!("failed to set bitrate: {e}");
    }

    // Frame size in samples per channel.
    let frame_samples = (config.sample_rate as usize * config.frame_duration_ms as usize) / 1000;
    let frame_total_samples = frame_samples * config.channels as usize;

    // Accumulate samples until we have a full frame.
    let mut pending: Vec<f32> = Vec::with_capacity(frame_total_samples * 2);
    let mut output_buf = vec![0u8; 4000]; // Max Opus packet size
    let mut rtp_timestamp: u64 = 0;

    debug!(
        "Opus encoder started: {}Hz, {} ch, {}bps, {}ms frames ({} samples/frame)",
        config.sample_rate,
        config.channels,
        config.bitrate,
        config.frame_duration_ms,
        frame_samples
    );

    while !stop.load(Ordering::Relaxed) {
        // Try to get audio data (with a short timeout via try_recv + sleep).
        match reader.try_recv() {
            Some(chunk) => {
                // Resample if needed (simple case: just use the samples if rate matches).
                if chunk.sample_rate == config.sample_rate && chunk.channels == config.channels {
                    pending.extend_from_slice(&chunk.samples);
                } else if chunk.channels != config.channels
                    && chunk.channels == 2
                    && config.channels == 1
                {
                    // Stereo to mono downmix.
                    for pair in chunk.samples.as_chunks::<2>().0 {
                        pending.push((pair[0] + pair[1]) * 0.5);
                    }
                } else if chunk.channels != config.channels
                    && chunk.channels == 1
                    && config.channels == 2
                {
                    // Mono to stereo upmix.
                    for &s in &chunk.samples {
                        pending.push(s);
                        pending.push(s);
                    }
                } else {
                    // Same channels, different rate — just use as-is (lossy but functional).
                    pending.extend_from_slice(&chunk.samples);
                }

                // Encode complete frames.
                while pending.len() >= frame_total_samples {
                    let frame = &pending[..frame_total_samples];
                    match encoder.encode_float(frame, &mut output_buf) {
                        Ok(n) => {
                            sink(&output_buf[..n], rtp_timestamp);
                            rtp_timestamp += frame_samples as u64;
                        }
                        Err(e) => {
                            trace!("opus encode error: {e}");
                        }
                    }
                    pending.drain(..frame_total_samples);
                }
            }
            None => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    debug!("Opus encoder stopped");
}

/// Handle to a running Opus decoder thread. Dropping stops decoding.
pub struct OpusDecoderHandle {
    /// Send encoded packets to the decoder.
    pub tx: std::sync::mpsc::Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for OpusDecoderHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Decoded audio callback: receives (f32 samples, sample_rate, channels).
pub type DecodedAudioSink = Box<dyn Fn(&[f32], u32, u16) + Send + 'static>;

/// Start an Opus decoder thread that receives encoded packets
/// and calls `sink` with decoded PCM samples.
pub fn start_opus_decoder(
    sample_rate: u32,
    channels: u16,
    sink: DecodedAudioSink,
) -> OpusDecoderHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();

    let thread = std::thread::Builder::new()
        .name("rinch-opus-dec".into())
        .spawn(move || {
            run_decoder(rx, sample_rate, channels, &sink, &stop_clone);
        })
        .expect("failed to spawn opus decoder thread");

    OpusDecoderHandle {
        tx,
        stop,
        thread: Some(thread),
    }
}

fn run_decoder(
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    sample_rate: u32,
    channels: u16,
    sink: &DecodedAudioSink,
    stop: &AtomicBool,
) {
    let ch = match channels {
        1 => Channels::Mono,
        2 => Channels::Stereo,
        _ => {
            warn!("unsupported channel count {}, using mono", channels);
            Channels::Mono
        }
    };

    let mut decoder = match Decoder::new(sample_rate, ch) {
        Ok(d) => d,
        Err(e) => {
            warn!("failed to create Opus decoder: {e}");
            return;
        }
    };

    // Max frame: 120ms at 48kHz stereo = 11520 samples.
    let mut output_buf = vec![0f32; 11520];

    debug!("Opus decoder started: {}Hz, {} ch", sample_rate, channels);

    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(std::time::Duration::from_millis(5)) {
            Ok(packet) => match decoder.decode_float(&packet, &mut output_buf, false) {
                Ok(decoded_samples) => {
                    let total = decoded_samples * channels as usize;
                    sink(&output_buf[..total], sample_rate, channels);
                }
                Err(e) => {
                    trace!("opus decode error: {e}");
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    debug!("Opus decoder stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let config = OpusEncoderConfig::default();
        let sample_rate = config.sample_rate;
        let _channels = config.channels;
        let frame_samples = (sample_rate as usize * config.frame_duration_ms as usize) / 1000;

        // Create encoder.
        let mut encoder = Encoder::new(sample_rate, Channels::Mono, Application::Voip).unwrap();
        encoder
            .set_bitrate(opus::Bitrate::Bits(config.bitrate))
            .unwrap();

        // Create decoder.
        let mut decoder = Decoder::new(sample_rate, Channels::Mono).unwrap();

        // Generate a 440Hz sine wave frame.
        let input: Vec<f32> = (0..frame_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin())
            .collect();

        // Encode.
        let mut encoded = vec![0u8; 4000];
        let n = encoder.encode_float(&input, &mut encoded).unwrap();
        let encoded = &encoded[..n];
        assert!(n > 0, "encoded packet should not be empty");

        // Decode.
        let mut decoded = vec![0f32; frame_samples * 2];
        let decoded_samples = decoder.decode_float(encoded, &mut decoded, false).unwrap();
        assert_eq!(decoded_samples, frame_samples);

        // Verify decoded audio is roughly similar (Opus is lossy).
        // Check that the decoded signal has non-trivial energy.
        let energy: f32 = decoded[..decoded_samples].iter().map(|s| s * s).sum();
        assert!(
            energy > 0.01,
            "decoded audio should have non-trivial energy, got {energy}"
        );
    }
}
