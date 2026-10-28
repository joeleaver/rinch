//! **An audio processor that runs on the audio thread**, written once for both platforms.
//!
//! Implement [`DspProcessor`] — take byte messages from the app, render samples, hand byte replies
//! back — and open it with `rinch_av::audio::open_dsp_output`:
//!
//! - **natively** the processor is an ordinary Rust value moved into the output device's callback;
//! - **on the web** the same processor is compiled into a separate, raw `cdylib` (no wasm-bindgen) and
//!   instantiated inside an `AudioWorklet`, where [`export_dsp!`] gives it the small C ABI below. The
//!   page compiles the module and hands it to the worklet; messages and replies cross the worklet's
//!   `MessagePort` as bytes. No `SharedArrayBuffer`, so no cross-origin isolation is needed.
//!
//! The DSP therefore runs off the main thread on both platforms: a busy frame, a layout or a garbage
//! collection on the page never glitches the sound.
//!
//! # Real-time rules
//!
//! The host calls the processor without allocating: [`render`](DspProcessor::render) gets one flat
//! planar buffer the host keeps, and [`reply`](DspProcessor::reply) writes into one byte buffer the
//! host keeps and clears between calls. A processor that also does not allocate (pre-size what it
//! needs in its constructor) never touches the allocator on the audio thread.
//!
//! # The module's exports (what [`export_dsp!`] generates)
//!
//! | export | signature | |
//! |---|---|---|
//! | `rinch_dsp_abi` | `() -> u32` | the ABI version, [`ABI_VERSION`]; the host refuses any other |
//! | `rinch_dsp_init` | `(sample_rate: f32, channels: u32)` | build the processor |
//! | `rinch_dsp_alloc` | `(len: u32) -> ptr` | a buffer the host writes one message into |
//! | `rinch_dsp_message` | `(len: u32)` | hand the processor that message |
//! | `rinch_dsp_render` | `(frames: u32) -> ptr` | render; `channels × frames` planar `f32` at `ptr` |
//! | `rinch_dsp_reply` | `() -> i32` | take the next reply into the reply buffer: its length, or `-1` for none |
//! | `rinch_dsp_reply_ptr` | `() -> ptr` | where that reply is |
//!
//! plus the module's own `memory`. Everything happens on one thread (the worklet's), one call at a
//! time; the exports are `unsafe` for exactly that reason (see [`export_dsp!`]).

/// The version of the C ABI [`export_dsp!`] generates, exported as `rinch_dsp_abi`. A host refuses a
/// module that reports another version (or none): a module built against a different `rinch-dsp`
/// fails with an error instead of misbehaving.
pub const ABI_VERSION: u32 = 1;

/// An audio processor. One instance runs on the audio thread for the output's life.
pub trait DspProcessor {
    /// A message the app sent, in the order it sent them; applied before the next render.
    fn message(&mut self, bytes: &[u8]);

    /// Render the next `out.len() / channels` frames into `out`, **planar**: channel `c` is
    /// `out[c * frames..(c + 1) * frames]` ([`channels_mut`] splits it). The host zeroes `out` before
    /// every call, so a sample left untouched is silence.
    fn render(&mut self, out: &mut [f32], channels: usize);

    /// Something to tell the app: write it into `out` (empty on entry; the host reuses it, so pushing
    /// into it allocates only past its capacity) and return `true`, or return `false` for nothing.
    /// Called after each render until it returns `false`.
    fn reply(&mut self, out: &mut Vec<u8>) -> bool {
        let _ = out;
        false
    }
}

/// `out` (as handed to [`DspProcessor::render`]) as one mutable slice per channel.
pub fn channels_mut(out: &mut [f32], channels: usize) -> core::slice::ChunksExactMut<'_, f32> {
    let frames = out.len() / channels.max(1);
    // `chunks_exact_mut(0)` panics; zero frames is zero channels' worth of nothing.
    out.chunks_exact_mut(frames.max(1))
}

/// The state behind [`export_dsp!`]'s exports. Public for the macro only.
#[doc(hidden)]
pub mod export {
    use core::cell::UnsafeCell;

    use super::DspProcessor;

    /// Where the processor and its buffers live inside the module.
    pub struct Host {
        processor: Box<dyn DspProcessor>,
        channels: usize,
        inbox: Vec<u8>,
        out: Vec<f32>,
        reply: Vec<u8>,
    }

    /// The module's one [`Host`]. Nothing in it is reachable without `unsafe`: every accessor is an
    /// `unsafe fn` whose contract is "one thread, one call at a time".
    pub struct Slot(UnsafeCell<Option<Host>>);

    // SAFETY: the cell is reachable only through the `unsafe fn`s below, whose callers promise that
    // no two calls on one slot overlap (an AudioWorklet runs a module on one thread and its exports
    // return before the next begins). Without that promise nothing safe can touch the contents.
    unsafe impl Sync for Slot {}

    impl Slot {
        #[allow(clippy::new_without_default)]
        pub const fn new() -> Self {
            Self(UnsafeCell::new(None))
        }

        /// # Safety
        /// No other reference into this slot may be alive (no overlapping call on any thread).
        #[allow(clippy::mut_from_ref)]
        unsafe fn host(&self) -> &mut Option<Host> {
            // SAFETY: the caller's promise.
            unsafe { &mut *self.0.get() }
        }
    }

    /// # Safety
    /// No other call on `slot` may overlap this one.
    pub unsafe fn init(slot: &Slot, processor: Box<dyn DspProcessor>, channels: u32) {
        // SAFETY: the caller's promise.
        *unsafe { slot.host() } = Some(Host {
            processor,
            channels: channels.max(1) as usize,
            inbox: Vec::new(),
            out: Vec::new(),
            reply: Vec::new(),
        });
    }

    /// # Safety
    /// No other call on `slot` may overlap this one.
    pub unsafe fn alloc(slot: &Slot, len: u32) -> *mut u8 {
        // SAFETY: the caller's promise.
        let Some(h) = (unsafe { slot.host() }) else {
            return core::ptr::null_mut();
        };
        h.inbox.clear();
        h.inbox.resize(len as usize, 0);
        h.inbox.as_mut_ptr()
    }

    /// # Safety
    /// No other call on `slot` may overlap this one.
    pub unsafe fn message(slot: &Slot, len: u32) {
        // SAFETY: the caller's promise.
        if let Some(h) = unsafe { slot.host() } {
            let n = (len as usize).min(h.inbox.len());
            h.processor.message(&h.inbox[..n]);
        }
    }

    /// # Safety
    /// No other call on `slot` may overlap this one.
    pub unsafe fn render(slot: &Slot, frames: u32) -> *const f32 {
        // SAFETY: the caller's promise.
        let Some(h) = (unsafe { slot.host() }) else {
            return core::ptr::null();
        };
        let len = frames as usize * h.channels;
        // Grows only when the quantum does (never, in a worklet); otherwise reuses the buffer.
        h.out.resize(len, 0.0);
        // Untouched samples are silence, whatever the last render left.
        h.out.fill(0.0);
        h.processor.render(&mut h.out, h.channels);
        h.out.as_ptr()
    }

    /// # Safety
    /// No other call on `slot` may overlap this one.
    pub unsafe fn reply(slot: &Slot) -> i32 {
        // SAFETY: the caller's promise.
        let Some(h) = (unsafe { slot.host() }) else {
            return -1;
        };
        h.reply.clear();
        if h.processor.reply(&mut h.reply) {
            i32::try_from(h.reply.len()).unwrap_or(i32::MAX)
        } else {
            -1
        }
    }

    /// # Safety
    /// No other call on `slot` may overlap this one.
    pub unsafe fn reply_ptr(slot: &Slot) -> *const u8 {
        // SAFETY: the caller's promise.
        unsafe { slot.host() }
            .as_ref()
            .map_or(core::ptr::null(), |h| h.reply.as_ptr())
    }
}

/// Give a raw wasm module the exports an AudioWorklet host calls (see the crate docs).
///
/// `$make` is called once, with the output's sample rate and channel count, to build the processor:
///
/// ```ignore
/// rinch_dsp::export_dsp!(|rate: u32, channels: u16| MySynth::new(rate as f32, channels));
/// ```
///
/// Use it in a `cdylib` built for `wasm32-unknown-unknown`, with no wasm-bindgen in its graph (the
/// guide's "Audio DSP" page has the recipe). The exports are `unsafe extern "C"`: they share one
/// `static` without locking, which is sound only while one thread calls them one at a time — what an
/// AudioWorklet does, and what a native caller must promise.
#[macro_export]
macro_rules! export_dsp {
    ($make:expr) => {
        static __RINCH_DSP: $crate::export::Slot = $crate::export::Slot::new();

        /// The ABI version this module speaks (touches no state, so safe).
        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_abi() -> u32 {
            $crate::ABI_VERSION
        }

        /// # Safety
        /// One thread, one call at a time, across all of this module's `rinch_dsp_*` exports.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rinch_dsp_init(sample_rate: f32, channels: u32) {
            let make = $make;
            let p = make(sample_rate as u32, channels as u16);
            // SAFETY: the caller's promise.
            unsafe { $crate::export::init(&__RINCH_DSP, ::std::boxed::Box::new(p), channels) }
        }

        /// # Safety
        /// One thread, one call at a time, across all of this module's `rinch_dsp_*` exports.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rinch_dsp_alloc(len: u32) -> *mut u8 {
            // SAFETY: the caller's promise.
            unsafe { $crate::export::alloc(&__RINCH_DSP, len) }
        }

        /// # Safety
        /// One thread, one call at a time, across all of this module's `rinch_dsp_*` exports.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rinch_dsp_message(len: u32) {
            // SAFETY: the caller's promise.
            unsafe { $crate::export::message(&__RINCH_DSP, len) }
        }

        /// # Safety
        /// One thread, one call at a time, across all of this module's `rinch_dsp_*` exports.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rinch_dsp_render(frames: u32) -> *const f32 {
            // SAFETY: the caller's promise.
            unsafe { $crate::export::render(&__RINCH_DSP, frames) }
        }

        /// # Safety
        /// One thread, one call at a time, across all of this module's `rinch_dsp_*` exports.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rinch_dsp_reply() -> i32 {
            // SAFETY: the caller's promise.
            unsafe { $crate::export::reply(&__RINCH_DSP) }
        }

        /// # Safety
        /// One thread, one call at a time, across all of this module's `rinch_dsp_*` exports.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rinch_dsp_reply_ptr() -> *const u8 {
            // SAFETY: the caller's promise.
            unsafe { $crate::export::reply_ptr(&__RINCH_DSP) }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::{DspProcessor, channels_mut, export};

    /// Remembers the last message's first byte as a level, renders it on every channel, and replies
    /// once per message with that byte.
    struct Level {
        level: f32,
        replies: Vec<u8>,
    }

    impl DspProcessor for Level {
        fn message(&mut self, bytes: &[u8]) {
            self.level = f32::from(bytes[0]) / 100.0;
            self.replies.push(bytes[0]);
        }
        fn render(&mut self, out: &mut [f32], _channels: usize) {
            out.fill(self.level);
        }
        fn reply(&mut self, out: &mut Vec<u8>) -> bool {
            match self.replies.pop() {
                Some(b) => {
                    out.push(b);
                    true
                }
                None => false,
            }
        }
    }

    // The macro must expand and compile; the static it declares is called by one test only.
    crate::export_dsp!(|_rate: u32, _channels: u16| Level {
        level: 0.0,
        replies: Vec::new()
    });

    #[test]
    fn the_exports_carry_a_message_in_samples_and_a_reply_out() {
        assert_eq!(rinch_dsp_abi(), crate::ABI_VERSION);
        // SAFETY: this is the only test that calls the macro's exports, so no call overlaps.
        unsafe {
            rinch_dsp_init(48_000.0, 2);
            let p = rinch_dsp_alloc(1);
            // `alloc` handed out a buffer of exactly one byte.
            *p = 50;
            rinch_dsp_message(1);
            let out = rinch_dsp_render(4);
            // `render` handed out `channels × frames` = 8 samples.
            let samples = core::slice::from_raw_parts(out, 8);
            assert!(samples.iter().all(|&s| s == 0.5));
            assert_eq!(rinch_dsp_reply(), 1);
            // The reply is one byte long.
            assert_eq!(*rinch_dsp_reply_ptr(), 50);
            assert_eq!(rinch_dsp_reply(), -1);
        }
    }

    /// Off the fixed point: every channel and frame distinct, so a wrong planar stride or a
    /// one-channel render shows. (The test above fills every channel with one level.)
    #[test]
    fn render_hands_the_processor_one_slice_per_channel_planar() {
        struct Ramp;
        impl DspProcessor for Ramp {
            fn message(&mut self, _: &[u8]) {}
            fn render(&mut self, out: &mut [f32], channels: usize) {
                assert_eq!(channels, 3);
                assert_eq!(out.len(), 12, "channels × frames");
                let mut n = 0;
                for (c, ch) in channels_mut(out, channels).enumerate() {
                    assert_eq!(ch.len(), 4, "one sample per frame");
                    for (i, s) in ch.iter_mut().enumerate() {
                        *s = (c * 100 + i) as f32;
                    }
                    n += 1;
                }
                assert_eq!(n, 3, "one slice per channel");
            }
        }
        let slot = export::Slot::new();
        // SAFETY: `slot` is local to this test; no call overlaps.
        unsafe {
            export::init(&slot, Box::new(Ramp), 3);
            let out = export::render(&slot, 4);
            // 3 channels × 4 frames.
            let s = core::slice::from_raw_parts(out, 12);
            assert_eq!(
                s,
                &[
                    0., 1., 2., 3., 100., 101., 102., 103., 200., 201., 202., 203.
                ]
            );
            // A message longer than the buffer `alloc` handed out is cut to it.
            let p = export::alloc(&slot, 2);
            assert!(!p.is_null());
            export::message(&slot, 99);
        }
    }

    /// The zero-fill rule: a sample the processor leaves untouched is silence, even where the last
    /// render wrote something (the buffer is reused between renders).
    #[test]
    fn samples_left_untouched_are_silence() {
        /// Writes 0.75 everywhere on its first render, then only channel 0's first frame.
        struct Once(bool);
        impl DspProcessor for Once {
            fn message(&mut self, _: &[u8]) {}
            fn render(&mut self, out: &mut [f32], _channels: usize) {
                if self.0 {
                    out[0] = 0.25;
                } else {
                    out.fill(0.75);
                    self.0 = true;
                }
            }
        }
        let slot = export::Slot::new();
        // SAFETY: `slot` is local to this test; no call overlaps.
        unsafe {
            export::init(&slot, Box::new(Once(false)), 2);
            let first = core::slice::from_raw_parts(export::render(&slot, 3), 6).to_vec();
            assert_eq!(first, vec![0.75; 6]);
            let second = core::slice::from_raw_parts(export::render(&slot, 3), 6);
            assert_eq!(second, &[0.25, 0., 0., 0., 0., 0.]);
        }
    }

    /// An empty reply is still a reply (length 0), distinct from "none" (-1).
    #[test]
    fn an_empty_reply_is_not_no_reply() {
        struct Empty(u8);
        impl DspProcessor for Empty {
            fn message(&mut self, _: &[u8]) {}
            fn render(&mut self, _: &mut [f32], _: usize) {}
            fn reply(&mut self, _out: &mut Vec<u8>) -> bool {
                self.0 = self.0.saturating_sub(1);
                self.0 > 0
            }
        }
        let slot = export::Slot::new();
        // SAFETY: `slot` is local to this test; no call overlaps.
        unsafe {
            export::init(&slot, Box::new(Empty(3)), 1);
            assert_eq!(export::reply(&slot), 0);
            assert_eq!(export::reply(&slot), 0);
            assert_eq!(export::reply(&slot), -1);
        }
    }
}
