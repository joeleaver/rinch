//! **An output whose DSP runs on the audio thread**, natively and on the web (rinch#1963).
//!
//! [`open_dsp_output`] plays a [`DspProcessor`]: natively it is moved into the output device's
//! callback; on the web the same processor, compiled to a raw wasm module with
//! [`rinch_dsp::export_dsp!`], runs inside an `AudioWorklet`. Either way the app talks to it in byte
//! messages ([`DspOutput::send`]) and hears back in byte replies ([`DspOutput::replies`]), and
//! nothing the page does on its main thread can glitch the sound.
//!
//! ```ignore
//! let out = open_dsp_output(
//!     DspOutputConfig::default(),
//!     |rate, channels| MySynth::new(rate, channels), // natively
//!     include_bytes!("my_synth.wasm"),               // on the web: the same synth, exported
//! )?;
//! out.send(note_on(60));
//! ```
//!
//! On the web the module loads asynchronously: messages sent before it is running are kept and
//! delivered in order once it is. Browsers start audio only after the user's first click or key, so
//! call [`DspOutput::resume`] from that gesture.

pub use rinch_dsp::DspProcessor;

use crate::error::AvError;

/// What to open.
#[derive(Debug, Clone)]
pub struct DspOutputConfig {
    /// Requested sample rate; [`DspOutput::config`] says what the device really runs.
    pub sample_rate: u32,
    pub channels: u16,
    /// Requested buffer in frames (native only; the web renders in 128-frame quanta).
    pub buffer_size: u32,
}

impl Default for DspOutputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            buffer_size: 512,
        }
    }
}

/// A running output. Dropping it stops the sound.
pub struct DspOutput {
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    inner: native::Native,
    #[cfg(target_arch = "wasm32")]
    inner: super::dsp_web::Web,
    config: DspOutputConfig,
}

impl DspOutput {
    /// The rate and channel count the output really runs at.
    pub fn config(&self) -> &DspOutputConfig {
        &self.config
    }

    /// Send the processor one message. Messages arrive in order, before the next render.
    pub fn send(&self, bytes: Vec<u8>) {
        self.inner.send(bytes);
    }

    /// Everything the processor replied since the last call, in order.
    pub fn replies(&self) -> Vec<Vec<u8>> {
        self.inner.replies()
    }

    /// Let the sound start (the web's autoplay rule): call it from a click or key handler. A no-op
    /// natively and once running.
    pub fn resume(&self) {
        #[cfg(target_arch = "wasm32")]
        self.inner.resume();
    }
}

/// Open the default output device playing a processor: `make(sample_rate, channels)` natively,
/// `web_module` (the processor as a raw wasm module, see [`rinch_dsp`]) on the web.
#[allow(unused_variables)]
pub fn open_dsp_output<P, F>(
    config: DspOutputConfig,
    make: F,
    web_module: &[u8],
) -> Result<DspOutput, AvError>
where
    P: DspProcessor + Send + 'static,
    F: FnOnce(u32, u16) -> P + Send + 'static,
{
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    {
        let (inner, config) = native::open(config, make)?;
        Ok(DspOutput { inner, config })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (inner, config) = super::dsp_web::open(config, web_module)?;
        Ok(DspOutput { inner, config })
    }
    #[cfg(all(not(target_arch = "wasm32"), not(feature = "audio")))]
    {
        Err(AvError::Backend("audio feature not enabled".into()))
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
mod native {
    use std::sync::mpsc::{Receiver, Sender, channel};

    use super::{DspOutputConfig, DspProcessor};
    use crate::error::AvError;

    pub(super) struct Native {
        _stream: cpal::Stream,
        to_audio: Sender<Vec<u8>>,
        from_audio: Receiver<Vec<u8>>,
    }

    impl Native {
        pub(super) fn send(&self, bytes: Vec<u8>) {
            let _ = self.to_audio.send(bytes);
        }

        pub(super) fn replies(&self) -> Vec<Vec<u8>> {
            self.from_audio.try_iter().collect()
        }
    }

    pub(super) fn open<P, F>(
        config: DspOutputConfig,
        make: F,
    ) -> Result<(Native, DspOutputConfig), AvError>
    where
        P: DspProcessor + Send + 'static,
        F: FnOnce(u32, u16) -> P + Send + 'static,
    {
        let (to_audio, inbox) = channel::<Vec<u8>>();
        let (outbox, from_audio) = channel::<Vec<u8>>();
        let device = crate::native::cpal_audio::default_output_device()?;
        let (stream, n) = crate::native::cpal_audio::open_output_stream(
            &device,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            move |n| {
                // Built at the rate the device really runs, before the first callback.
                let processor = make(n.sample_rate, n.channels);
                callback(processor, usize::from(n.channels.max(1)), inbox, outbox)
            },
        )?;
        let config = DspOutputConfig {
            sample_rate: n.sample_rate,
            channels: n.channels,
            ..config
        };
        Ok((
            Native {
                _stream: stream,
                to_audio,
                from_audio,
            },
            config,
        ))
    }

    /// The output callback's body: apply queued messages, render planar, interleave into `buf`,
    /// forward replies. Separate from the device so it can be tested without one.
    pub(super) fn callback<P: DspProcessor>(
        mut processor: P,
        channels: usize,
        inbox: Receiver<Vec<u8>>,
        outbox: Sender<Vec<u8>>,
    ) -> impl FnMut(&mut [f32]) {
        let mut planar: Vec<f32> = Vec::new();
        move |buf: &mut [f32]| {
            while let Ok(m) = inbox.try_recv() {
                processor.message(&m);
            }
            let frames = buf.len() / channels;
            if planar.len() < frames * channels {
                // Grows to the device's buffer in the first callbacks, then never again.
                planar.resize(frames * channels, 0.0);
            }
            let planar = &mut planar[..frames * channels];
            planar.fill(0.0);
            {
                let mut chans: Vec<&mut [f32]> = planar.chunks_mut(frames.max(1)).collect();
                processor.render(&mut chans);
            }
            for (i, frame) in buf.chunks_mut(channels).enumerate() {
                for (c, s) in frame.iter_mut().enumerate() {
                    *s = planar[c * frames + i];
                }
            }
            while let Some(r) = processor.reply() {
                let _ = outbox.send(r);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::mpsc::channel;

        use super::{DspProcessor, callback};

        /// Renders `channel * 100 + frame + offset`, where a message's first byte is the offset;
        /// replies with each message and the number of frames of each render.
        struct Ramp {
            offset: f32,
            replies: Vec<Vec<u8>>,
        }
        impl DspProcessor for Ramp {
            fn message(&mut self, b: &[u8]) {
                self.offset = f32::from(b[0]);
                self.replies.push(b.to_vec());
            }
            fn render(&mut self, out: &mut [&mut [f32]]) {
                for (c, ch) in out.iter_mut().enumerate() {
                    for (i, s) in ch.iter_mut().enumerate() {
                        *s = (c * 100 + i) as f32 + self.offset;
                    }
                }
                self.replies.push(vec![0xff, out[0].len() as u8]);
            }
            fn reply(&mut self) -> Option<Vec<u8>> {
                (!self.replies.is_empty()).then(|| self.replies.remove(0))
            }
        }

        #[test]
        fn messages_apply_before_render_and_planar_is_interleaved_per_frame() {
            let (to_audio, inbox) = channel();
            let (outbox, from_audio) = channel();
            let mut cb = callback(
                Ramp {
                    offset: 0.0,
                    replies: vec![],
                },
                3,
                inbox,
                outbox,
            );
            to_audio.send(vec![7]).unwrap();
            let mut buf = vec![-1.0; 3 * 4];
            cb(&mut buf);
            assert_eq!(
                buf,
                vec![
                    7., 107., 207., 8., 108., 208., 9., 109., 209., 10., 110., 210.
                ]
            );
            let replies: Vec<_> = from_audio.try_iter().collect();
            assert_eq!(replies, vec![vec![7], vec![0xff, 4]]);
            // A smaller device buffer reuses the grown planar store, sliced to this callback.
            let mut buf = vec![-1.0; 3 * 2];
            cb(&mut buf);
            assert_eq!(buf, vec![7., 107., 207., 8., 108., 208.]);
        }

        /// FINDING (review #1964): replies nobody polls pile up without bound in the unbounded
        /// mpsc channel (and in the web's `VecDeque`); each is an allocation on the audio thread.
        #[test]
        fn finding_unpolled_replies_grow_without_bound() {
            let (_to_audio, inbox) = channel();
            let (outbox, from_audio) = channel();
            let mut cb = callback(
                Ramp {
                    offset: 0.0,
                    replies: vec![],
                },
                2,
                inbox,
                outbox,
            );
            let mut buf = vec![0.0; 2 * 128];
            for _ in 0..10_000 {
                cb(&mut buf);
            }
            assert_eq!(from_audio.try_iter().count(), 10_000);
        }
    }
}
