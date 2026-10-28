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
//! # The module's exports (what [`export_dsp!`] generates)
//!
//! | export | signature | |
//! |---|---|---|
//! | `rinch_dsp_init` | `(sample_rate: f32, channels: u32)` | build the processor |
//! | `rinch_dsp_alloc` | `(len: u32) -> ptr` | a buffer the host writes one message into |
//! | `rinch_dsp_message` | `(len: u32)` | hand the processor that message |
//! | `rinch_dsp_render` | `(frames: u32) -> ptr` | render; `channels × frames` planar `f32` at `ptr` |
//! | `rinch_dsp_reply` | `() -> len` | take the next reply into the reply buffer (0: none) |
//! | `rinch_dsp_reply_ptr` | `() -> ptr` | where that reply is |
//!
//! plus the module's own `memory`. Everything happens on one thread (the worklet's), between calls.

/// An audio processor. One instance runs on the audio thread for the output's life.
pub trait DspProcessor {
    /// A message the app sent, in the order it sent them; applied before the next render.
    fn message(&mut self, bytes: &[u8]);

    /// Fill each channel of `out` (all the same length) with the next samples.
    fn render(&mut self, out: &mut [&mut [f32]]);

    /// Something to tell the app, if there is anything. Called after each render until `None`.
    fn reply(&mut self) -> Option<Vec<u8>> {
        None
    }
}

/// The state behind [`export_dsp!`]'s exports. Public for the macro only.
#[doc(hidden)]
pub mod export {
    use core::cell::UnsafeCell;

    use super::DspProcessor;

    /// Where the processor and its buffers live inside the module.
    pub struct Host {
        pub processor: Option<Box<dyn DspProcessor>>,
        pub channels: usize,
        pub inbox: Vec<u8>,
        pub out: Vec<f32>,
        pub reply: Vec<u8>,
    }

    /// The module's one [`Host`]. A worklet runs a module on exactly one thread and calls into it one
    /// export at a time, so a plain cell is all the synchronisation there is to have.
    pub struct Slot(pub UnsafeCell<Option<Host>>);

    // SAFETY: an AudioWorklet module is single-threaded and its exports are never re-entered (each
    // call returns before the next begins), so no two references to the host ever coexist.
    unsafe impl Sync for Slot {}

    impl Slot {
        #[allow(clippy::mut_from_ref)]
        pub fn host(&self) -> &mut Option<Host> {
            // SAFETY: see the `Sync` impl — one thread, one call at a time.
            unsafe { &mut *self.0.get() }
        }
    }

    pub fn init(slot: &Slot, processor: Box<dyn DspProcessor>, channels: u32) {
        *slot.host() = Some(Host {
            processor: Some(processor),
            channels: channels.max(1) as usize,
            inbox: Vec::new(),
            out: Vec::new(),
            reply: Vec::new(),
        });
    }

    pub fn alloc(slot: &Slot, len: u32) -> *mut u8 {
        let Some(h) = slot.host() else {
            return core::ptr::null_mut();
        };
        h.inbox.clear();
        h.inbox.resize(len as usize, 0);
        h.inbox.as_mut_ptr()
    }

    pub fn message(slot: &Slot, len: u32) {
        if let Some(h) = slot.host()
            && let Some(p) = h.processor.as_mut()
        {
            let n = (len as usize).min(h.inbox.len());
            p.message(&h.inbox[..n]);
        }
    }

    pub fn render(slot: &Slot, frames: u32) -> *const f32 {
        let Some(h) = slot.host() else {
            return core::ptr::null();
        };
        let frames = frames as usize;
        h.out.clear();
        h.out.resize(frames * h.channels, 0.0);
        if let Some(p) = h.processor.as_mut() {
            let mut chans: Vec<&mut [f32]> = h.out.chunks_mut(frames.max(1)).collect();
            p.render(&mut chans);
        }
        h.out.as_ptr()
    }

    pub fn reply(slot: &Slot) -> u32 {
        let Some(h) = slot.host() else { return 0 };
        match h.processor.as_mut().and_then(|p| p.reply()) {
            Some(r) => {
                h.reply = r;
                h.reply.len() as u32
            }
            None => 0,
        }
    }

    pub fn reply_ptr(slot: &Slot) -> *const u8 {
        slot.host()
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
/// Use it in a `cdylib` built for `wasm32-unknown-unknown`, with no wasm-bindgen in its graph.
#[macro_export]
macro_rules! export_dsp {
    ($make:expr) => {
        static __RINCH_DSP: $crate::export::Slot =
            $crate::export::Slot(::core::cell::UnsafeCell::new(None));

        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_init(sample_rate: f32, channels: u32) {
            let make = $make;
            let p = make(sample_rate as u32, channels as u16);
            $crate::export::init(&__RINCH_DSP, ::std::boxed::Box::new(p), channels);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_alloc(len: u32) -> *mut u8 {
            $crate::export::alloc(&__RINCH_DSP, len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_message(len: u32) {
            $crate::export::message(&__RINCH_DSP, len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_render(frames: u32) -> *const f32 {
            $crate::export::render(&__RINCH_DSP, frames)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_reply() -> u32 {
            $crate::export::reply(&__RINCH_DSP)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn rinch_dsp_reply_ptr() -> *const u8 {
            $crate::export::reply_ptr(&__RINCH_DSP)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::DspProcessor;

    /// Remembers the last message's first byte as a level, renders it on every channel, and replies
    /// once per message with how many frames it rendered.
    struct Level {
        level: f32,
        replies: Vec<Vec<u8>>,
    }

    impl DspProcessor for Level {
        fn message(&mut self, bytes: &[u8]) {
            self.level = f32::from(bytes[0]) / 100.0;
            self.replies.push(vec![bytes[0]]);
        }
        fn render(&mut self, out: &mut [&mut [f32]]) {
            for ch in out.iter_mut() {
                ch.fill(self.level);
            }
        }
        fn reply(&mut self) -> Option<Vec<u8>> {
            self.replies.pop()
        }
    }

    crate::export_dsp!(|_rate: u32, _channels: u16| Level {
        level: 0.0,
        replies: Vec::new()
    });

    #[test]
    fn the_exports_carry_a_message_in_samples_and_a_reply_out() {
        rinch_dsp_init(48_000.0, 2);
        let p = rinch_dsp_alloc(1);
        // SAFETY: `alloc` handed out a buffer of exactly one byte.
        unsafe { *p = 50 };
        rinch_dsp_message(1);
        let out = rinch_dsp_render(4);
        // SAFETY: `render` handed out `channels × frames` = 8 samples.
        let samples = unsafe { core::slice::from_raw_parts(out, 8) };
        assert!(samples.iter().all(|&s| s == 0.5));
        assert_eq!(rinch_dsp_reply(), 1);
        // SAFETY: the reply is one byte long.
        assert_eq!(unsafe { *rinch_dsp_reply_ptr() }, 50);
        assert_eq!(rinch_dsp_reply(), 0);
    }
}
