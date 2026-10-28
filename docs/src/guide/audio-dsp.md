# Audio DSP on the Audio Thread

`rinch_av::audio::open_dsp_output` plays an audio processor that runs **on the audio thread**, with
one processor written once for both platforms:

- **natively** the processor is an ordinary Rust value moved into the output device's callback
  (cpal);
- **on the web** the same processor is compiled into a small, separate wasm module and runs inside an
  `AudioWorklet`.

The app talks to it in byte messages and hears back in byte replies. A busy frame, a layout or a
garbage collection on the page's main thread cannot glitch the sound, because nothing on the main
thread is in the audio path.

It needs `rinch-av` with the `audio` feature, and the dependency-free crate `rinch-dsp` for the
processor itself.

## Writing a processor

Implement `rinch_dsp::DspProcessor`:

```rust
use rinch_dsp::{DspProcessor, channels_mut};

pub struct Tone {
    phase: f32,
    step: f32,
    gain: f32,
    played: u32,
}

impl Tone {
    pub fn new(sample_rate: u32, _channels: u16) -> Self {
        Self { phase: 0.0, step: 440.0 / sample_rate as f32, gain: 0.0, played: 0 }
    }
}

impl DspProcessor for Tone {
    /// Messages from the app, in order, applied before the next render.
    fn message(&mut self, bytes: &[u8]) {
        if let [gain] = bytes {
            self.gain = f32::from(*gain) / 255.0;
        }
    }

    /// `out` is planar: channel `c` is `out[c * frames..(c + 1) * frames]`.
    fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        for ch in channels_mut(out, channels) {
            let mut phase = self.phase;
            for s in ch.iter_mut() {
                *s = (phase * std::f32::consts::TAU).sin() * self.gain;
                phase = (phase + self.step).fract();
            }
        }
        self.phase = (self.phase + self.step * frames as f32).fract();
        self.played += frames as u32;
    }

    /// Called after each render until it returns `false`.
    fn reply(&mut self, out: &mut Vec<u8>) -> bool {
        if self.played < 48_000 {
            return false;
        }
        self.played = 0;
        out.push(1); // "another second played"
        true
    }
}
```

The rules the host keeps, so that a processor can be allocation-free on the audio thread:

- **`render`** gets one flat planar buffer the host owns and reuses. The host **zeroes it before every
  call**: a sample the processor leaves untouched is silence, never what the last render wrote.
- **`reply`** writes into one byte buffer the host owns, cleared before each call. Pushing into it
  allocates only past its capacity. Natively the host reserves the reply queue's size up front. In
  the web module the buffer keeps whatever capacity earlier replies grew it to. An empty reply
  (return `true` with nothing written) is still a reply.
- Neither the messages nor the replies allocate on the audio thread natively (see
  [Queues](#queues-and-polling)). The processor's own code is the processor's business: pre-size what
  it needs in its constructor.

## Opening an output

```rust
use rinch_av::audio::{DspOutputConfig, open_dsp_output};

let out = open_dsp_output(
    DspOutputConfig::default(),        // 48 kHz, 2 channels, 512 frames, 64 KiB queues
    |rate, channels| Tone::new(rate, channels), // natively
    include_bytes!("../dsp/tone.wasm"),         // on the web: the same Tone, exported
)?;

out.send(&[128]);                // `false` if it could not be queued
for reply in out.replies() {     // poll, e.g. once a frame
    // ...
}
if let Some(why) = out.error() { // the output stopped
    // ...
}
```

- `make` runs on the calling thread, before `open_dsp_output` returns, once the device's real rate
  and channel count are known. Only the processor it returns moves to the audio thread, so only the
  processor type must be `Send + 'static`.
- On the web `make` is never called and its processor type has no bounds, so one call site serves
  both platforms. A web-only caller can pass `|_, _| ()`.
- `out.config()` reports what really runs: the device's sample rate and channel count. Its
  `buffer_size` is 128 on the web (the worklet's render quantum). Natively it is the size you asked
  for, which the device was opened with as a fixed size; `0` asks for the backend's default and
  stays `0`, because cpal does not report the default it chose.
- Dropping the `DspOutput` stops the sound.

## Building the web module

The worklet instantiates the module with **an empty import object**. A worklet's global scope has no
`TextDecoder`, `fetch` or DOM, so no wasm-bindgen glue can run there. The module must therefore be
a plain `cdylib` with no wasm-bindgen anywhere in its dependency graph. `rinch-dsp` has no
dependencies for this reason.

A crate for it (often a small crate beside the one that defines the processor):

```toml
# dsp/Cargo.toml
[package]
name = "tone-dsp"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
rinch-dsp = "…"          # the same version rinch-av uses
my-synth = { path = "../synth" }  # the processor; it must not pull in wasm-bindgen either

[profile.release]
opt-level = "s"
panic = "abort"
```

```rust
// dsp/src/lib.rs
rinch_dsp::export_dsp!(|rate: u32, channels: u16| my_synth::Tone::new(rate, channels));
```

```sh
cargo build --release --target wasm32-unknown-unknown --manifest-path dsp/Cargo.toml
cp dsp/target/wasm32-unknown-unknown/release/tone_dsp.wasm assets/
wasm-tools print assets/tone_dsp.wasm | grep '(import'   # must print nothing
```

If that crate sits inside your workspace directory but should not be a member, give it its own
`[workspace]` table, as rinch's test fixture `crates/rinch-av/tests/fixtures/dsp-echo` does.

Before it creates the worklet node, the page checks that the module imports nothing and has every
export listed below. A module that fails the check, or that the browser will not compile, becomes
the output's `error()`.

### The module's C ABI

`export_dsp!` generates these exports. They are `unsafe extern "C"`: they share one `static` without
locking, which is sound because a worklet calls them on one thread, one at a time.

| export | signature | |
|---|---|---|
| `rinch_dsp_abi` | `() -> u32` | the ABI version (`rinch_dsp::ABI_VERSION`, currently `1`) |
| `rinch_dsp_init` | `(sample_rate: f32, channels: u32)` | build the processor |
| `rinch_dsp_alloc` | `(len: u32) -> ptr` | a buffer the host writes one message into |
| `rinch_dsp_message` | `(len: u32)` | hand the processor that message |
| `rinch_dsp_render` | `(frames: u32) -> ptr` | render; `channels × frames` planar `f32` at `ptr` |
| `rinch_dsp_reply` | `() -> i32` | take the next reply: its length, or `-1` for none |
| `rinch_dsp_reply_ptr` | `() -> ptr` | where that reply is |
| `memory` | | the module's memory |

The worklet checks `rinch_dsp_abi` first, and refuses a module that reports another version or has
no such export. The error names both versions. **Rebuild the module whenever you update rinch**: a
module built against an older `rinch-dsp` fails with that error rather than misbehaving.

## Threading

| | natively | web |
|---|---|---|
| `make` / module compile | the calling thread, inside `open_dsp_output` | the page, asynchronously after `open_dsp_output` returns |
| `message`, `render`, `reply` | the audio device's callback thread | the `AudioWorkletGlobalScope`'s thread |
| `send`, `replies`, `error` | the thread that owns the `DspOutput` | the page's main thread |
| render size | the device's buffer (`config().buffer_size` when fixed) | 128-frame quanta |

Within one output the processor's three methods are never called at the same time. A message is
applied before the next render, in the order it was sent. On the web the module loads
asynchronously: messages sent before it is running are kept (within the message budget) and
delivered first, in order.

## Queues and polling

Both directions are bounded, by `DspOutputConfig::message_capacity` and `reply_capacity` (bytes; 64
KiB each by default; every message or reply also costs 4 bytes of framing):

- **Messages.** `send` returns `false` when the message does not fit: the audio side is behind (on
  the web, also while the context is suspended and so renders nothing), the message is longer than
  the capacity allows, or the output has failed. Natively the queue is a preallocated
  single-producer ring (`rtrb`). The audio thread reads each message into one buffer of the ring's
  size, so a message neither allocates nor frees there. On the web the budget covers everything not
  yet applied: messages kept before the worklet started, messages crossing the port, and messages
  waiting in the worklet. The worklet reports what it has applied, which frees that part of the
  budget.
- **Replies.** Poll `replies()` regularly; once a frame is plenty. Replies that do not fit are
  dropped (the newest ones) and counted by `dropped_replies()`. The processor's `reply` is still
  called until it returns `false`, so its own queue drains either way. Natively the reply queue is
  a preallocated ring as well.

## Failures and panics

`error()` returns why the output stopped, or `None` while it runs (on the web, also while the module
is still loading). After an error the output renders silence and `send` returns `false`.

- **Web, start-up**: the browser would not compile the module, the module imports something or
  lacks an export, `addModule` failed (see [CSP](#content-security-policy)), or the ABI version is
  not this rinch's.
- **Web, a processor that traps.** A Rust panic in the module is an `unreachable` trap. The worklet
  catches it in `process`, reports it once (`RuntimeError: …`), and stops rendering. A
  `processorerror` from the node is reported the same way. A trap cannot be recovered: open a new
  output.
- **Native, a processor that panics.** The callback catches the panic (it must not unwind into the
  audio backend), records it as `the DSP processor panicked: …`, and outputs silence from then on.
  This needs the default `panic = "unwind"`. With `panic = "abort"` the process aborts.
- **Native, the device.** A cpal stream error (for example, the device was unplugged) is logged and
  does not reach `error()`.

## Web specifics

- **One `AudioContext` per output.** Each `open_dsp_output` creates its own context, at the requested
  rate where the browser allows it; `config().sample_rate` reports the rate it really runs. Closing
  happens on drop. Dropping an output while its worklet is still loading is quiet: the context
  closes, and the load gives up without reporting an error.
- **Autoplay.** Browsers keep a new `AudioContext` suspended until a user gesture. Call
  `out.resume()` from a click or key handler. Until then nothing renders, so messages accumulate
  against the message budget.
- **Content Security Policy.** The worklet script is loaded from a `blob:` URL, and the module is
  compiled with `WebAssembly.compile`. A page with a CSP therefore needs `script-src blob:`, and
  `'wasm-unsafe-eval'` (or `'unsafe-eval'`) for the compile. Without them `error()` reports the
  refusal.
- **No cross-origin isolation.** Messages and replies cross the worklet's `MessagePort`; there is no
  `SharedArrayBuffer`.
- **What runs in the worklet each quantum.** It allocates nothing of its own apart from the replies
  it posts. The views of the module's memory are cached, and re-made only when the memory grows or
  the render buffer moves.
