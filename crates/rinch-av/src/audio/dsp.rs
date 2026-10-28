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
//! out.send(&note_on(60));
//! ```
//!
//! On the web the module loads asynchronously: messages sent before it is running are kept and
//! delivered in order once it is. Browsers start audio only after the user's first click or key, so
//! call [`DspOutput::resume`] from that gesture.
//!
//! **Queues are bounded** ([`DspOutputConfig::message_capacity`], [`reply_capacity`]
//! (DspOutputConfig::reply_capacity)): natively they are preallocated single-producer rings, so the
//! audio thread never allocates or frees for a message or a reply. A full message queue makes
//! [`DspOutput::send`] return `false`; a full reply queue drops the newest replies and counts them
//! ([`DspOutput::dropped_replies`]). Poll [`DspOutput::replies`] regularly (once a frame is plenty).
//!
//! **Failures are reported, not just logged**: [`DspOutput::error`] says why an output stopped (a
//! module the browser would not load, a module built for another `rinch-dsp` ABI, a processor that
//! panicked). After an error the output is silent and `send` returns `false`.

pub use rinch_dsp::DspProcessor;

use crate::error::AvError;

/// What to open.
#[derive(Debug, Clone)]
pub struct DspOutputConfig {
    /// Requested sample rate; [`DspOutput::config`] says what the device really runs.
    pub sample_rate: u32,
    pub channels: u16,
    /// Requested buffer in frames. Natively a non-zero size is asked of the device as a fixed size
    /// (and [`DspOutput::config`] repeats it, since the stream opened with it); `0` lets the backend
    /// choose, and stays `0` because cpal does not report the backend's choice. The web always
    /// renders in 128-frame quanta, and reports `128`.
    pub buffer_size: u32,
    /// Bytes of messages that may wait for the audio thread at once; each message costs its length
    /// plus 4. A message that does not fit is refused ([`DspOutput::send`] returns `false`), so this
    /// is also the largest message (less 4). On the web it bounds both the messages kept before the
    /// worklet runs and those waiting inside the worklet (a suspended context renders nothing).
    pub message_capacity: usize,
    /// Bytes of replies that may wait for [`DspOutput::replies`]; each costs its length plus 4.
    /// Replies past it are dropped (newest first) and counted in [`DspOutput::dropped_replies`].
    pub reply_capacity: usize,
}

impl Default for DspOutputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            buffer_size: 512,
            message_capacity: 64 * 1024,
            reply_capacity: 64 * 1024,
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
    /// What the output really runs: the device's sample rate and channel count, and the buffer size
    /// as [`DspOutputConfig::buffer_size`] describes.
    pub fn config(&self) -> &DspOutputConfig {
        &self.config
    }

    /// Send the processor one message. Messages arrive in order, before the next render.
    ///
    /// `false` when it was not queued: the message queue is full (the audio thread is behind, or on
    /// the web the context is suspended), the message is longer than
    /// [`DspOutputConfig::message_capacity`] allows, or the output has failed ([`error`](Self::error)).
    pub fn send(&self, bytes: &[u8]) -> bool {
        self.inner.send(bytes)
    }

    /// Everything the processor replied since the last call, in order.
    pub fn replies(&self) -> Vec<Vec<u8>> {
        self.inner.replies()
    }

    /// How many replies were dropped because the reply queue was full (nobody polled
    /// [`replies`](Self::replies) for a while).
    pub fn dropped_replies(&self) -> u64 {
        self.inner.dropped_replies()
    }

    /// Why the output stopped, if it did: the module failed to load or start (web), its ABI version
    /// is not this `rinch-dsp`'s, or the processor panicked (a trap, on the web). `None` while it
    /// runs, and on the web also while the module is still loading.
    pub fn error(&self) -> Option<String> {
        self.inner.error()
    }

    /// Let the sound start (the web's autoplay rule): call it from a click or key handler. A no-op
    /// natively and once running.
    pub fn resume(&self) {
        #[cfg(target_arch = "wasm32")]
        self.inner.resume();
    }
}

// One definition for both targets, differing only in `P`'s bounds (attributes on where-clause
// predicates are not stable).
macro_rules! open_dsp_output_fn {
    ($($bounds:tt)*) => {
        /// Open the default output device playing a processor: `make(sample_rate, channels)`
        /// natively, `web_module` (the processor as a raw wasm module, see [`rinch_dsp`]) on the
        /// web.
        ///
        /// `make` runs on the calling thread, before this returns, once the device's real rate is
        /// known; only the processor it returns crosses to the audio thread, so only `P` must be
        /// `Send`. On the web `make` is never called and `P` has no bounds: the app's call site is
        /// the same on both platforms (its processor type implements [`DspProcessor`] anyway, since
        /// that is what its module exports), and a web-only caller can pass `|_, _| ()`.
        #[allow(unused_variables)]
        pub fn open_dsp_output<P, F>(
            config: DspOutputConfig,
            make: F,
            web_module: &[u8],
        ) -> Result<DspOutput, AvError>
        where
            F: FnOnce(u32, u16) -> P,
            $($bounds)*
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
    };
}

#[cfg(not(target_arch = "wasm32"))]
open_dsp_output_fn!(P: DspProcessor + Send + 'static,);
#[cfg(target_arch = "wasm32")]
open_dsp_output_fn!();

/// [`open_dsp_output`] into a graph the caller owns: an `OfflineAudioContext` in tests.
#[cfg(all(target_arch = "wasm32", test))]
pub(crate) fn open_dsp_output_in(
    ctx: &web_sys::BaseAudioContext,
    dest: &web_sys::AudioNode,
    config: DspOutputConfig,
    web_module: &[u8],
) -> DspOutput {
    let (inner, config) = super::dsp_web::open_in(None, ctx, dest, config, web_module);
    DspOutput { inner, config }
}

#[cfg(all(target_arch = "wasm32", test))]
impl DspOutput {
    /// The worklet node exists (the module loaded and the node was created).
    pub(crate) fn started(&self) -> bool {
        self.inner.started()
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
mod native {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    use rtrb::{Consumer, Producer, RingBuffer};

    use super::{DspOutputConfig, DspProcessor};
    use crate::error::AvError;

    /// Frames the planar buffer is sized for when the backend chooses the buffer size. A device
    /// that hands a longer buffer grows it once, in that callback.
    const DEFAULT_MAX_FRAMES: usize = 4096;

    /// What the audio thread tells the app without a queue.
    #[derive(Default)]
    pub(super) struct Status {
        pub(super) error: OnceLock<String>,
        pub(super) failed: AtomicBool,
        pub(super) dropped_replies: AtomicU64,
    }

    pub(super) struct Native {
        _stream: cpal::Stream,
        to_audio: Mutex<Producer<u8>>,
        from_audio: Mutex<Consumer<u8>>,
        status: Arc<Status>,
    }

    impl Native {
        pub(super) fn send(&self, bytes: &[u8]) -> bool {
            if self.status.failed.load(Ordering::Acquire) {
                return false;
            }
            push_frame(&mut self.to_audio.lock().unwrap(), bytes)
        }

        pub(super) fn replies(&self) -> Vec<Vec<u8>> {
            let mut ring = self.from_audio.lock().unwrap();
            let mut out = Vec::new();
            let mut r = Vec::new();
            while pop_frame(&mut ring, &mut r) {
                out.push(std::mem::take(&mut r));
            }
            out
        }

        pub(super) fn dropped_replies(&self) -> u64 {
            self.status.dropped_replies.load(Ordering::Relaxed)
        }

        pub(super) fn error(&self) -> Option<String> {
            self.status.error.get().cloned()
        }
    }

    pub(super) fn open<P, F>(
        config: DspOutputConfig,
        make: F,
    ) -> Result<(Native, DspOutputConfig), AvError>
    where
        P: DspProcessor + Send + 'static,
        F: FnOnce(u32, u16) -> P,
    {
        let (to_audio, inbox) = RingBuffer::<u8>::new(config.message_capacity.max(4));
        let (outbox, from_audio) = RingBuffer::<u8>::new(config.reply_capacity.max(4));
        let status = Arc::new(Status::default());
        let device = crate::native::cpal_audio::default_output_device()?;
        let max_frames = match config.buffer_size {
            0 => DEFAULT_MAX_FRAMES,
            n => n as usize,
        };
        let (stream, n) = crate::native::cpal_audio::open_output_stream(
            &device,
            config.sample_rate,
            config.channels,
            config.buffer_size,
            {
                let status = status.clone();
                move |n| {
                    // Built here, on the calling thread, at the rate the device really runs, before
                    // the stream (and so the first callback) exists.
                    let processor = make(n.sample_rate, n.channels);
                    callback(
                        processor,
                        usize::from(n.channels.max(1)),
                        max_frames,
                        inbox,
                        outbox,
                        status,
                    )
                }
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
                to_audio: Mutex::new(to_audio),
                from_audio: Mutex::new(from_audio),
                status,
            },
            config,
        ))
    }

    /// Queue `bytes` as one frame (a little-endian `u32` length, then the bytes), or `false` when it
    /// does not fit. Frames go in whole: the one consumer never sees a header without its bytes.
    pub(super) fn push_frame(ring: &mut Producer<u8>, bytes: &[u8]) -> bool {
        let Ok(len) = u32::try_from(bytes.len()) else {
            return false;
        };
        if ring.slots() < 4 + bytes.len() {
            return false;
        }
        // Both fit (checked above, and only this producer adds), so neither can fail.
        ring.push_entire_slice(&len.to_le_bytes()).is_ok() && ring.push_entire_slice(bytes).is_ok()
    }

    /// Take the next whole frame into `into` (cleared first), or `false` when there is none. Does
    /// not allocate while the frame fits `into`'s capacity.
    pub(super) fn pop_frame(ring: &mut Consumer<u8>, into: &mut Vec<u8>) -> bool {
        let len = {
            let Ok(head) = ring.read_chunk(4) else {
                return false;
            };
            let (a, b) = head.as_slices();
            let mut h = [0u8; 4];
            h[..a.len()].copy_from_slice(a);
            h[a.len()..].copy_from_slice(b);
            // Not committed: the header stays until the whole frame is read.
            u32::from_le_bytes(h) as usize
        };
        let Ok(frame) = ring.read_chunk(4 + len) else {
            return false;
        };
        let (a, b) = frame.as_slices();
        into.clear();
        let skip_a = a.len().min(4);
        into.extend_from_slice(&a[skip_a..]);
        into.extend_from_slice(&b[4 - skip_a..]);
        frame.commit_all();
        true
    }

    /// The output callback's body: apply queued messages, render planar, interleave into `buf`,
    /// forward replies. Separate from the device so it can be tested without one.
    ///
    /// Allocation-free once running: messages are read into one buffer sized to the message ring,
    /// replies are written into one buffer the processor reuses, the planar buffer is sized for
    /// `max_frames`. A panic in the processor is caught here (it must not unwind into the audio
    /// backend): the output records it in `status` and is silent from then on.
    pub(super) fn callback<P: DspProcessor>(
        mut processor: P,
        channels: usize,
        max_frames: usize,
        mut inbox: Consumer<u8>,
        mut outbox: Producer<u8>,
        status: Arc<Status>,
    ) -> impl FnMut(&mut [f32]) {
        let mut planar: Vec<f32> = vec![0.0; max_frames * channels];
        let mut message: Vec<u8> = Vec::with_capacity(inbox.buffer().capacity());
        let mut reply: Vec<u8> = Vec::with_capacity(outbox.buffer().capacity());
        let mut dead = false;
        move |buf: &mut [f32]| {
            if dead {
                buf.fill(0.0);
                return;
            }
            let ran = catch_unwind(AssertUnwindSafe(|| {
                while pop_frame(&mut inbox, &mut message) {
                    processor.message(&message);
                }
                let frames = buf.len() / channels;
                if planar.len() < frames * channels {
                    // A longer buffer than the device was asked for: grow once.
                    planar.resize(frames * channels, 0.0);
                }
                let planar = &mut planar[..frames * channels];
                // Untouched samples are silence, whatever the last render left.
                planar.fill(0.0);
                processor.render(planar, channels);
                for (i, frame) in buf.chunks_exact_mut(channels).enumerate() {
                    for (c, s) in frame.iter_mut().enumerate() {
                        *s = planar[c * frames + i];
                    }
                }
                buf[frames * channels..].fill(0.0);
                loop {
                    reply.clear();
                    if !processor.reply(&mut reply) {
                        break;
                    }
                    if !push_frame(&mut outbox, &reply) {
                        status.dropped_replies.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
            if let Err(panic) = ran {
                dead = true;
                buf.fill(0.0);
                let why = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "a panic".into());
                let _ = status
                    .error
                    .set(format!("the DSP processor panicked: {why}"));
                status.failed.store(true, Ordering::Release);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        use rtrb::RingBuffer;

        use super::{DspProcessor, Status, callback, pop_frame, push_frame};

        /// Renders `channel * 100 + frame + offset`, where a message's first byte is the offset;
        /// replies with each message and the number of frames of each render.
        struct Ramp {
            offset: f32,
            replies: Vec<Vec<u8>>,
        }
        impl DspProcessor for Ramp {
            fn message(&mut self, b: &[u8]) {
                if b == [0xee] {
                    panic!("asked to");
                }
                self.offset = f32::from(b[0]);
                self.replies.push(b.to_vec());
            }
            fn render(&mut self, out: &mut [f32], channels: usize) {
                let frames = out.len() / channels;
                for (c, ch) in rinch_dsp::channels_mut(out, channels).enumerate() {
                    for (i, s) in ch.iter_mut().enumerate() {
                        *s = (c * 100 + i) as f32 + self.offset;
                    }
                }
                self.replies.push(vec![0xff, frames as u8]);
            }
            fn reply(&mut self, out: &mut Vec<u8>) -> bool {
                if self.replies.is_empty() {
                    return false;
                }
                out.extend_from_slice(&self.replies.remove(0));
                true
            }
        }

        fn ramp() -> Ramp {
            Ramp {
                offset: 0.0,
                replies: vec![],
            }
        }

        fn drain(c: &mut rtrb::Consumer<u8>) -> Vec<Vec<u8>> {
            let mut all = Vec::new();
            let mut r = Vec::new();
            while pop_frame(c, &mut r) {
                all.push(r.clone());
            }
            all
        }

        #[test]
        fn messages_apply_before_render_and_planar_is_interleaved_per_frame() {
            let (mut to_audio, inbox) = RingBuffer::new(64);
            let (outbox, mut from_audio) = RingBuffer::new(64);
            let mut cb = callback(ramp(), 3, 4, inbox, outbox, Arc::default());
            assert!(push_frame(&mut to_audio, &[7]));
            let mut buf = vec![-1.0; 3 * 4];
            cb(&mut buf);
            assert_eq!(
                buf,
                vec![
                    7., 107., 207., 8., 108., 208., 9., 109., 209., 10., 110., 210.
                ]
            );
            assert_eq!(drain(&mut from_audio), vec![vec![7], vec![0xff, 4]]);
            // A smaller device buffer reuses the planar store, sliced to this callback.
            let mut buf = vec![-1.0; 3 * 2];
            cb(&mut buf);
            assert_eq!(buf, vec![7., 107., 207., 8., 108., 208.]);
            // A longer one than it was sized for grows it.
            let mut buf = vec![-1.0; 3 * 6];
            cb(&mut buf);
            assert_eq!(&buf[15..], &[12., 112., 212.]);
        }

        /// The zero-fill rule natively: what the processor leaves untouched is silence, even where
        /// the previous callback's render wrote (the planar buffer is reused).
        #[test]
        fn samples_left_untouched_are_silence() {
            struct Once(bool);
            impl DspProcessor for Once {
                fn message(&mut self, _: &[u8]) {}
                fn render(&mut self, out: &mut [f32], _: usize) {
                    if !self.0 {
                        out.fill(0.75);
                        self.0 = true;
                    }
                }
            }
            let (_, inbox) = RingBuffer::new(16);
            let (outbox, _) = RingBuffer::new(16);
            let mut cb = callback(Once(false), 2, 3, inbox, outbox, Arc::default());
            let mut buf = vec![-1.0; 6];
            cb(&mut buf);
            assert_eq!(buf, vec![0.75; 6]);
            let mut buf = vec![-1.0; 6];
            cb(&mut buf);
            assert_eq!(buf, vec![0.0; 6]);
        }

        /// Replies nobody polls stay within the reply ring (the review's
        /// `finding_unpolled_replies_grow_without_bound` saw 10 000 queued at the PR head): the
        /// audio side drops what does not fit, counts it, and keeps draining the processor.
        #[test]
        fn unpolled_replies_stay_bounded_and_are_counted() {
            let (_to_audio, inbox) = RingBuffer::new(16);
            // Each reply is [0xff, 128]: 2 bytes + a 4-byte header.
            let (outbox, mut from_audio) = RingBuffer::new(60);
            let status = Arc::new(Status::default());
            let mut cb = callback(ramp(), 2, 128, inbox, outbox, status.clone());
            let mut buf = vec![0.0; 2 * 128];
            for _ in 0..10_000 {
                cb(&mut buf);
            }
            let queued = drain(&mut from_audio);
            assert_eq!(queued.len(), 10, "60 bytes hold ten 6-byte replies");
            assert_eq!(
                status.dropped_replies.load(Ordering::Relaxed),
                10_000 - 10,
                "every reply that did not fit is counted (the processor was still drained)"
            );
            // Polled, the ring takes replies again.
            cb(&mut buf);
            assert_eq!(drain(&mut from_audio), vec![vec![0xff, 128]]);
        }

        #[test]
        fn a_full_message_ring_refuses_and_frames_survive_the_wrap() {
            let (mut p, mut c) = RingBuffer::new(10);
            assert!(push_frame(&mut p, &[1, 2, 3]), "7 of 10 bytes");
            assert!(!push_frame(&mut p, &[4]), "5 more do not fit");
            assert!(!push_frame(&mut p, &[0; 7]), "longer than the ring");
            let mut r = Vec::new();
            assert!(pop_frame(&mut c, &mut r));
            assert_eq!(r, [1, 2, 3]);
            // This one wraps round the end of the ring, header and bytes.
            assert!(push_frame(&mut p, &[9, 8, 7, 6]));
            assert!(pop_frame(&mut c, &mut r));
            assert_eq!(r, [9, 8, 7, 6]);
            assert!(push_frame(&mut p, &[]), "an empty message is a message");
            assert!(pop_frame(&mut c, &mut r));
            assert!(r.is_empty());
            assert!(!pop_frame(&mut c, &mut r));
        }

        /// A panicking processor does not unwind into the backend: the output goes silent and says
        /// why, and `send` would refuse from then on (`Status::failed`).
        #[test]
        fn a_panicking_processor_is_reported_and_silenced() {
            let (mut to_audio, inbox) = RingBuffer::new(64);
            let (outbox, _from_audio) = RingBuffer::new(64);
            let status = Arc::new(Status::default());
            let mut cb = callback(ramp(), 1, 4, inbox, outbox, status.clone());
            let mut buf = vec![-1.0; 4];
            cb(&mut buf);
            assert_eq!(buf, vec![0., 1., 2., 3.]);
            assert!(status.error.get().is_none());
            assert!(push_frame(&mut to_audio, &[0xee]));
            cb(&mut buf);
            assert_eq!(buf, vec![0.0; 4]);
            let err = status.error.get().expect("the panic is reported");
            assert!(err.contains("asked to"), "{err}");
            assert!(status.failed.load(Ordering::Acquire));
            buf.fill(-1.0);
            cb(&mut buf);
            assert_eq!(buf, vec![0.0; 4], "silent from then on");
        }
    }
}
