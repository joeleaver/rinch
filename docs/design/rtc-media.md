# Media tracks on rinch-rtc (#1916)

**Status:** Decided (issue #1916). Joe accepted M1–M12 as recommended,
2026-10-26. For M3 that is option (c): openh264 built from source by
default, an opt-in to load Cisco's binary at run time, and docs that say
plainly what each covers. Implemented in the four steps of section 9; the
last (the ports and the removal) is recorded in 5.4.
**Context:** Joe decided #1916: rebuild the media call on `rinch-rtc`, then
remove `rinch-webrtc`. `rinch-rtc` (data channels) is the design record
`docs/design/webrtc-data-channels.md`, and is implemented on main
(#1909, #1915).
**Scope:** audio and video tracks on a `rinch_rtc::Peer`, on web and on
desktop: sending, receiving, capture, codecs, and showing and playing what
arrives. Then porting the two demos, and removing `rinch-webrtc`,
`rinch-signaling` and `rinch-signaling-server`.
**Evidence:** a spike on branch `spike-1916` (`spikes/1916/`, not for
merge). Section 8 gives its numbers. The spike, and every test designed
here, uses **synthetic sources only**: a generated test pattern and a
generated tone. Nothing in it opens a camera, a microphone or a screen.

## 0. What is in the tree today

**`rinch-webrtc` never carries media.** It negotiates audio and video
m-lines on str0m 0.7, but:

- `PeerConnection::add_track` always returns "not implemented", and
  `on_track` has an empty body (`crates/rinch-webrtc/src/peer.rs`).
- The Opus encoder and decoder in `native/audio_codec.rs` are called from
  nowhere.
- There is no video codec at all.
- Received payloads (`IncomingMedia`) are still encoded and are never
  depacketized.
- On native, only a `127.0.0.1` candidate is advertised.
- In `VideoCall`, the caller never applies the answer (`call.rs`), `join`
  blocks the UI thread, and on the web `clone_box` is `unimplemented!()`
  (`rinch-signaling`).

**Neither demo can complete a call:**

- **`examples/video-call`.** It points at a URL the relay rejects (no
  `/room/` segment). The `VideoCall` is dropped at the end of the function
  that made it. "Remote video" is a grey placeholder.
- **The ui-zoo WebRTC section.** Its default port (9090) doesn't match the
  relay's (8080). It drops the call at the end of the click handler and
  never polls it.

There is little to keep from the old crate beyond the shape of its two
demos.

**What is reusable:**

- **`rinch-av`** (`crates/rinch-av`): device lists, a camera through
  nokhwa 0.10 (frames are RGBA8 after MJPEG decode, delivered on a capture
  thread), and microphone and speaker through cpal 0.15 (interleaved
  `f32`, on cpal's thread). It has `VideoTrack` / `AudioTrack` fan-out
  helpers. Known defects:
  - The microphone's rate and channel count are the *requested* ones, not
    what the device negotiated.
  - Nothing resamples.
  - On the web, the microphone's state is dropped as soon as setup finishes.
  - On the web, the camera's `stop()` never stops its tracks.
  - The web camera reads pixels back through a canvas, which a WebRTC
    sender does not need.
- **`rinch-screen-capture`:** PipeWire (Linux) and `getDisplayMedia` (web),
  producing RGBA `VideoFrame`s.
- **The named video sink from #1768 / #1865**
  (`rinch::render_surface::create_video_frame_sink(viewport_id) ->
  VideoFrameSink`):
  - It is `Arc<dyn Fn(&[u8], u32, u32) + Send + Sync>`, callable from any
    thread, RGBA8.
  - It shows frames in the element that carries `data-viewport = id`.
  - Only nodes with a new frame are damaged.
  - On the GPU window the frame takes the split path (one `write_texture`
    per frame, about 1.6 ms of composition at 1080p, per the split-layer
    record). Otherwise it is drawn in-scene; the software renderer draws it
    in paint order.
  - Frames are `contain`-fitted on a black backdrop.
  - The shell hands this factory to `rinch-video` at startup
    (`set_frame_sink_factory`). `rinch-rtc` cannot call it directly: it
    depends on `rinch-core`, not on `rinch`.

On the web, nothing in rinch attaches a `MediaStream` to a visible
`<video>`, and rinch-web has no public way to set `srcObject` on an element.
`rinch-video`'s web player finds its own `<video>` by a data attribute and
drives it through web-sys, and the same trick works here.

## Summary

- **The API follows the browser's model, simplified.**
  - `LocalTrack` is something you send: a camera, a microphone, or a source
    the app feeds itself, such as a test pattern, a game view or a tone.
  - `peer.add_track(&track)` returns a `Sender`, which has
    `replace_track(Option<&LocalTrack>)`.
  - `peer.add_transceiver(kind, direction)` returns a `Transceiver`, with a
    direction you can change.
  - `peer.on_track(|RemoteTrack|)` fires for each received track.
  - Muting is `track.set_enabled(false)`, as in the browser: black frames or
    silence. The far side sees `on_mute` / `on_unmute`.
  - A `MediaView { track }` component shows a local or remote video track.
    A remote audio track plays by default, with `set_volume` and `stop`.
- **Web:** the browser does everything. That means `getUserMedia`
  (directly, not through rinch-av), `addTrack` / `replaceTrack` / the
  transceivers, and a `<video srcObject>` for `MediaView`. No frame passes
  through wasm unless the app asks for one, which only the app-fed sources
  do (`canvas.captureStream` and a WebAudio graph).
- **Desktop:** webrtc-rs 0.21 packetizes, depacketizes and negotiates, but
  encodes nothing. The codecs come from elsewhere:
  - **Video: H.264 through `openh264`** (built from source with `cc`, BSD,
    no system library), **and VP8 through the system libvpx behind the
    `media-vp8` feature**. Where VP8 is built it is offered first (M2,
    decided 2026-10-27; VP8 made opt-in 2026-10-28). The spike sent a test pattern through
    webrtc-rs, and 89 of 90 H.264 frames decoded correctly. The H.264
    patent position is M3.
  - **Audio: Opus through `opus` 0.4**, with bundled libopus. On this host it
    costs 0.14 ms to encode and 0.04 ms to decode a 20 ms frame.
  - Capture is rinch-av, with its defects fixed.
  - Playback is cpal plus a jitter buffer and a resampler.
  - Received video is converted I420→RGBA on the CPU (0.35 ms at 720p) and
    handed to the named video sink. The split path draws it.
  - There is no echo cancellation in v1 (M5).
- **Packaging.** Media sits behind a `media` feature of `rinch-rtc`, off by
  default (M1), so a data-channel-only app (Playweft) pays nothing. On web
  the feature adds only web-sys bindings.
- **The ports.**
  - `examples/video-call` is rewritten on rinch-rtc, with `rinch-ws` for
    signalling and a tiny room relay as an example.
  - The ui-zoo WebRTC section becomes an in-page call between two peers,
    with a test pattern until the user asks for the camera.
  - Then `rinch-webrtc`, `rinch-signaling` and `rinch-signaling-server` are
    removed.

## 1. API

All of this sits under `rinch_rtc::media`, behind the `media` feature. The
threading and lifetime rules are the data-channel ones: everything is called
on the UI thread, callbacks run there in a batch, and a track or peer made
during a render is cancelled when that component unmounts.

### 1.1 Local tracks

```rust
pub enum MediaKind { Audio, Video }

/// Something this app sends. Cheap to clone (`Rc`), `!Send`.
#[derive(Clone)]
pub struct LocalTrack { /* id + backend */ }

impl LocalTrack {
    /// The camera. Asks permission on the web. On desktop it is rinch-av's
    /// camera. `None` picks the default device.
    pub fn camera(options: CameraOptions) -> impl Future<Output = Result<LocalTrack, MediaError>>;
    /// The microphone, likewise.
    pub fn microphone(options: MicrophoneOptions) -> impl Future<Output = Result<LocalTrack, MediaError>>;

    /// A video track the app feeds: a game view, a test pattern, a
    /// processed camera. Frames are RGBA8, any thread. Web:
    /// canvas.captureStream. Desktop: straight into the encoder.
    pub fn video_source(options: VideoSourceOptions) -> (LocalTrack, VideoSourceWriter);
    /// An audio track the app feeds: f32 interleaved, any rate (resampled).
    pub fn audio_source(options: AudioSourceOptions) -> (LocalTrack, AudioSourceWriter);

    pub fn kind(&self) -> MediaKind;
    pub fn id(&self) -> String;
    /// `false`: video sends black frames and audio silence, and the far side
    /// sees `on_mute`. The browser's `track.enabled`. The device stays open.
    pub fn set_enabled(&self, enabled: bool);
    pub fn enabled(&self) -> bool;
    /// End it: the device is released, and every sender carrying it stops.
    pub fn stop(&self);
    pub fn on_ended(&self, cb: impl FnMut() + 'static) -> &Self;
}

pub struct CameraOptions { pub device: Option<String>, pub width: u32, pub height: u32, pub fps: u32 }
pub struct MicrophoneOptions { pub device: Option<String> }
pub struct VideoSourceOptions { pub width: u32, pub height: u32, pub fps: u32 }
pub struct AudioSourceOptions { pub sample_rate: u32, pub channels: u16 }

/// `Send + Sync + Clone`; keeps only the latest frame (as the video sink does).
impl VideoSourceWriter { pub fn push_rgba(&self, rgba: &[u8], width: u32, height: u32); }
impl AudioSourceWriter { pub fn push(&self, interleaved: &[f32]); }

/// Device lists, for a picker (rinch-av on desktop, enumerateDevices on web).
pub fn video_inputs() -> impl Future<Output = Vec<DeviceInfo>>;
pub fn audio_inputs() -> impl Future<Output = Vec<DeviceInfo>>;
```

A local video track can also be shown with `MediaView` (section 1.4) for a
self-view. That costs nothing extra on the web, and on desktop it is one
more sink beside the encoder.

### 1.2 Sending: senders and transceivers

```rust
impl Peer {
    /// Send `track`: a sendrecv transceiver of its kind. Renegotiate after
    /// (see on_negotiation_needed) unless it was added before the offer.
    pub fn add_track(&self, track: &LocalTrack) -> Result<Sender, RtcError>;
    /// A transceiver with nothing to send yet, e.g. RecvOnly to receive
    /// video without a camera.
    pub fn add_transceiver(&self, kind: MediaKind, direction: Direction) -> Result<Transceiver, RtcError>;
    pub fn transceivers(&self) -> Vec<Transceiver>;
    /// Something changed that needs a new offer/answer: a track added,
    /// a direction changed. As the browser's `negotiationneeded`.
    pub fn on_negotiation_needed(&self, cb: impl FnMut() + 'static) -> &Self;
    pub fn on_track(&self, cb: impl FnMut(RemoteTrack) + 'static) -> &Self;
}

pub enum Direction { SendRecv, SendOnly, RecvOnly, Inactive }

#[derive(Clone)] pub struct Sender { .. }
impl Sender {
    /// Swap what is sent without renegotiating (same kind), or `None` to
    /// send nothing. `RTCRtpSender.replaceTrack`.
    pub fn replace_track(&self, track: Option<&LocalTrack>) -> impl Future<Output = Result<(), RtcError>>;
    pub fn track(&self) -> Option<LocalTrack>;
    /// The target bitrate cap (`maxBitrate`), video and audio.
    pub fn set_max_bitrate(&self, bps: Option<u32>);
}

#[derive(Clone)] pub struct Transceiver { .. }
impl Transceiver {
    pub fn kind(&self) -> MediaKind;
    pub fn mid(&self) -> Option<String>;
    pub fn sender(&self) -> Sender;
    pub fn direction(&self) -> Direction;
    /// Takes effect at the next negotiation (fires on_negotiation_needed).
    pub fn set_direction(&self, d: Direction);
    /// What was last negotiated.
    pub fn current_direction(&self) -> Option<Direction>;
    pub fn stop(&self);
}
```

`add_track` adds a `sendrecv` transceiver, as in the browser. A peer that
only receives adds `add_transceiver(Video, RecvOnly)`. Muting comes in two
strengths:

- `LocalTrack::set_enabled(false)` keeps the stream flowing with black
  frames or silence. It is cheap to undo, and the far side sees `on_mute`.
- `Sender::replace_track(None)` stops sending.

Both work without renegotiation on both backends: webrtc-rs 0.21 has
`RtpSender::replace_track` and `RtpTransceiver::set_direction`.

### 1.3 Receiving

```rust
#[derive(Clone)] pub struct RemoteTrack { .. }   // `!Send`, UI thread
impl RemoteTrack {
    pub fn kind(&self) -> MediaKind;
    pub fn id(&self) -> String;
    /// The stream ids the sender grouped it in (audio + video of one person).
    pub fn stream_ids(&self) -> Vec<String>;
    pub fn transceiver(&self) -> Transceiver;
    pub fn muted(&self) -> bool;
    pub fn on_mute(&self, cb: impl FnMut() + 'static) -> &Self;
    pub fn on_unmute(&self, cb: impl FnMut() + 'static) -> &Self;
    pub fn on_ended(&self, cb: impl FnMut() + 'static) -> &Self;

    /// Audio: playing to the default output starts as soon as it arrives.
    pub fn set_volume(&self, v: f32);   // 0.0 = silent
    pub fn set_playing(&self, playing: bool);

    /// Video: each decoded frame, RGBA8, on a worker thread (desktop) or on
    /// the UI thread after a canvas read-back (web). For analysis, recording or
    /// tests; showing a track needs MediaView, which does not go through this.
    pub fn on_frame(&self, cb: impl FnMut(&[u8], u32, u32) + Send + 'static) -> &Self;
}
```

### 1.4 Showing video: `MediaView`

```rust
rsx! { MediaView { track: remote_video.clone(), style: "width: 640px; height: 360px;" } }
rsx! { MediaView { track: local_camera.clone(), mirrored: true } }   // self-view
```

- **Web:** a `<video autoplay playsinline>` whose `srcObject` is a
  `MediaStream` holding the track. It is muted for local tracks and for
  remote video (remote audio plays through its own element; see 2.3). The
  browser decodes and paints it. The backend finds its element by a data
  attribute, as `rinch-video`'s web player does, because rinch-web has no
  `NodeHandle`-to-element API.
- **Desktop:** a div with `data-viewport = "rtc-<track id>"`, fed by the
  named video sink. The decoder thread converts each frame to RGBA and calls
  the sink. Readiness is `data-viewport-ready`, as for `VideoViewport`, so
  the element shows black until the first frame. How rinch-rtc reaches the
  sink is M6.
- **Fit:** both use `contain`. The web gets `object-fit: contain` from CSS.
  On desktop the video arm hard-codes `contain` (#1908).

### 1.5 Errors

```rust
pub enum MediaError {
    PermissionDenied,           // the user, or the browser's policy
    DeviceNotFound(String),
    DeviceInUse,
    NotSupported(String),       // no camera API, codec missing, `media` off
    Backend(String),
}
```

A codec failure on desktop (an encoder that cannot start, a decoder error)
is reported through the track's `on_ended` and through `Peer::on_error`.

### 1.6 As built (PR B1, #1933)

Where the code differs from the sketches above, it is the code that holds:

- `video_source` and `audio_source` return `Result<(LocalTrack, Writer),
  MediaError>`: the web can fail to make the canvas or the AudioContext.
- The writers have `is_ended()`, so a producer can stop when its track ends
  (stopped or dropped).
- `LocalTrack` also has `ended()` and `status()`. `status()` returns
  `LocalStatus::{Live, WaitingForGesture, Ended}`.
  - An `audio_source` made outside a user gesture is `WaitingForGesture`
    while the browser holds its AudioContext.
  - The page's next click, tap or key press resumes it, as rinch-av's
    capture does (#1928).
- `RemoteTrack::transceiver()` returns `Option<Transceiver>`, which is
  `None` once the peer is gone. `RemoteTrack` also has `ended()`.
- **A remote track is keyed on the browser track.** A transceiver that goes
  inactive and comes back fires `track` again for the same track. That stays
  one `RemoteTrack`, with one `<audio>`, and it unmutes. B2 applies the same
  rule natively.
- `MediaView` takes a `MediaTrack` (`None`, `Local`, `Remote`, with `From`
  for both tracks), plus `style` and `class`. Unmounting it clears the
  `<video>`'s `srcObject`.
- `RtcError::Media(String)` carries what a peer reports about media: remote
  audio that the autoplay policy refused.
- `LocalTrack::camera` and `microphone` make the `getUserMedia` request in
  a task of their own. A future dropped while the prompt is open (its
  component unmounted) leaves no device on: the track the browser grants
  later is stopped.
- The native `data-viewport` naming (1.4) is settled in B2.

### 1.7 Glare and rollback

`Peer::signaling_state()` returns `SignalingState::{Stable, HaveLocalOffer,
HaveRemoteOffer, Closed}`.

- An offer still being made counts as `HaveLocalOffer`. Perfect
  negotiation's collision test is therefore just "an offer arrived and the
  state is not `Stable`". The app needs no "making an offer" flag of its
  own, because rinch-rtc's per-peer queue orders each signalling call after
  the ones before it.
- The polite peer simply sets the incoming offer. Browsers roll its own
  offer back implicitly.
- `SdpKind` is `#[non_exhaustive]` and has `Rollback`
  (`Description::rollback()`), which undoes whichever offer is pending. The
  web backend does this through `setLocalDescription` or
  `setRemoteDescription` by the signalling state. Desktop returns an error
  until B2.

**webrtc-rs 0.21 does not roll back implicitly.** Its signalling state
machine (`rtc` 0.21.1, `peer_connection/state/signaling_state.rs`,
`check_next_signaling_state`) has no `have-local-offer → SetRemote(offer)`
transition, so the polite peer's `set_remote_description(offer)` fails
there. It does support an explicit rollback through
`set_local_description(rollback)` and `set_remote_description(rollback)`
(RFC 9429 5.7, with tests in `peer_connection/mod.rs`). B2's native backend
therefore does the implicit rollback itself: a remote offer arriving in
`have-local-offer` is applied as `SetLocal(rollback)` and then
`SetRemote(offer)`. That keeps the polite side the same on both backends.

## 2. Web backend

### 2.1 Capture and sending

- `LocalTrack::camera` and `microphone` call
  `navigator.mediaDevices.getUserMedia({video|audio: constraints})` and keep
  the `MediaStreamTrack`. rinch-av's web camera, which reads pixels back
  through a canvas, is **not** used: a sender takes the track itself.
- `video_source` creates a hidden `<canvas>` and calls
  `canvas.captureStream(fps)`. `push_rgba` does `putImageData`, which has to
  happen on the UI thread, so a push from another thread is queued, latest
  frame only.
- `audio_source` uses an `AudioContext` with an `AudioWorklet`. A
  `ScriptProcessorNode` fallback feeds a `MediaStreamAudioDestinationNode`,
  and that node's `stream` supplies the track.
- `add_track` maps to `pc.addTrack(track, stream)`, `replace_track` to
  `sender.replaceTrack`, `add_transceiver` to `pc.addTransceiver(kind,
  {direction})`, `set_direction` to `transceiver.direction = ..`, and
  `set_max_bitrate` to `sender.setParameters({encodings:[{maxBitrate}]})`.
- `set_enabled` maps to `track.enabled`, and `stop` to `track.stop()`.
- **New web-sys features:** `MediaStreamTrack`, `MediaStream`,
  `MediaDevices`, `MediaStreamConstraints`, `MediaTrackConstraints`,
  `RtcRtpSender`, `RtcRtpReceiver`, `RtcRtpTransceiver`,
  `RtcRtpTransceiverInit`, `RtcRtpTransceiverDirection`, `RtcTrackEvent`,
  `HtmlVideoElement`, `HtmlAudioElement`, `HtmlCanvasElement`,
  `CanvasRenderingContext2d`, `ImageData`, `AudioContext`,
  `MediaStreamAudioDestinationNode`, `AudioWorkletNode`. Some of these
  (`RtcRtpSender.setParameters`, `canvas.captureStream`) have patchy web-sys
  coverage, and are reached through `js_sys::Reflect`, as `sctp` was.

### 2.2 Receiving

- `ontrack` becomes a `RemoteTrack` holding the `MediaStreamTrack`, and the
  event's streams give `stream_ids`.
- The track's `onmute`, `onunmute` and `onended` drive the callbacks.
- `on_frame` (web) draws the playing `<video>` into a canvas on
  requestAnimationFrame and reads it back. That is costly, and is meant for
  tests and analysis only.

### 2.3 Playing audio

A remote audio track gets a hidden `<audio autoplay>` with `srcObject`,
appended to the document body and removed when the track ends.
`set_volume` and `set_playing` drive it.

Autoplay policy: Chrome and Firefox allow audio after a user gesture on the
page, and a call is normally started from a click. If `play()` is rejected,
the error is reported through `Peer::on_error`
(`MediaError::NotSupported("autoplay")`), and a later `set_playing(true)`
from a click retries.

## 3. Desktop backend

### 3.1 What webrtc-rs does and doesn't do

webrtc-rs 0.21 does:

- negotiate codecs (`MediaEngine`), with default registrations for Opus,
  G.722, PCMU/PCMA, VP8, VP9, H.264, AV1 and H.265;
- packetize: `TrackLocalStaticSample::sample_writer(..).write_sample` takes
  one encoded frame;
- give each received track's RTP packets through `TrackRemote::poll()`,
  with `OnMute` / `OnUnmute` / `OnEnded` events;
- depacketize: `rtc::media::io::sample_builder::SampleBuilder` with the
  per-codec depacketizers (`H264Packet`, VP8, VP9, AV1, Opus);
- run the RTCP interceptors (`register_default_interceptors`): NACK,
  reports, TWCC;
- `replace_track`, `set_direction`, `add_transceiver_from_kind`.

It does **no encoding or decoding** and has no capture. Those come from the
crates below, on rinch-rtc's worker thread or threads of its own.

### 3.2 Codecs

| Codec | Crate | Build | Licence | Measured (this host, section 8) |
|---|---|---|---|---|
| H.264 (constrained baseline) | `openh264` 0.9.9 | Cisco's C source built by `cc` (`source` feature, the default); no system library; nasm optional | BSD-2; patents: see M3 | 640×480: encode 10.9 ms, decode 3.0 ms per frame; 1280×720: 22.3 / 5.6 ms (C only: this host has no nasm) |
| VP8 | libvpx via `env-libvpx-sys` 5.1.3 (+ `vpx-encode` 0.6.2) | **system libvpx** through pkg-config. Its bindings stop below 1.16, so the `generate` feature (bindgen, so libclang at build time) is needed for this host's 1.16. Last release 2023 | MPL-2 (bindings), BSD (libvpx); royalty-free | encode 7.9 ms (640×480), 17.6 ms (1280×720); decode not measured |
| Opus | `opus` 0.4.0 (`opusic-sys`, bundled libopus via cmake) | cmake at build time | BSD-3 | 48 kHz stereo, 20 ms: encode 0.14 ms, decode 0.04 ms |
| Opus, pure Rust | `opus-rs` 0.1.40 | none | BSD-3 | not measured; young (0.1, 2026) |

What the table means:

- **A 720p30 H.264 stream costs most of one core** to encode without
  assembly (22 ms of a 33 ms budget). 640×480 costs about a third of one.
  So the default camera size is 640×480 at 30 fps, with 1280×720 opt-in.
  Whether openh264's assembly (its nasm build path) changes this was not
  measured.
- **Decode is cheap.** Even 720p takes a sixth of a core, so one desktop
  can show several remote streams.
- **Browser interop:**
  - Chrome, Edge and Safari decode and encode H.264 constrained baseline
    natively.
  - **Firefox** does H.264 through Cisco's OpenH264 plugin, which it
    downloads itself. The plugin is normally present, but can be missing on
    some Linux builds; VP8 is built into every browser.
  - The native↔Firefox test (section 6) found the gap: CI's Firefox, and
    Playwright's, have no plugin, still list H.264 in their offers, and send
    nothing. That decided M2 for VP8 (3.7).
- **VP9 and AV1:** not offered. Their encoders cost several times H.264's
  in software, and every browser does H.264 or VP8.

### 3.3 The send path (per local video track)

1. **Source.** A camera (rinch-av, RGBA on its capture thread) or a
   `VideoSourceWriter` (RGBA from any thread). Either way, latest frame
   only.
2. **Encoder thread** (`rinch-rtc-enc-<n>`). Paced to the track's fps, it
   converts RGBA to I420 (openh264's `YUVBuffer::from_rgba8_source`), then
   encodes:
   - a keyframe on a PLI/FIR from the far side;
   - bitrate from `set_max_bitrate`, capped by the congestion estimate where
     webrtc-rs's TWCC gives one.
3. **Hand-off.** The encoded frame goes to the worker thread, which calls
   `write_sample` on the sender's `TrackLocalStaticSample`.
4. **Disabled track:** a black I420 frame at 1 fps, or a frame skip with a
   keyframe on re-enable (implementation detail).

Encoding runs off the worker thread so that a slow frame never stalls the
peer's signalling or data channels. There is one encoder thread per sending
video track: a host sending to N peers encodes N times unless the same
`LocalTrack` feeds them all. With the same `LocalTrack`, one encoder feeds
every sender of that track: simulcast-free fan-out, at the cost of one
bitrate for all.

**Audio** goes from the microphone or an `AudioSourceWriter` to a resampler
(`rubato`, pure Rust), then to 20 ms frames, then Opus encoding (on the
capture or encoder thread; it is cheap), then `write_sample`.

### 3.4 The receive path (per remote track)

- **Video.** Each track has a decode task. RTP from `TrackRemote::poll()`
  goes into a `SampleBuilder` with the codec's depacketizer, which assembles
  frames. They are decoded with libvpx or openh264, on a decoder
  thread (`rinch-rtc-dec-<n>`), converted I420→RGBA (openh264's SIMD
  `write_rgba8`, 0.35 ms at 720p), and then:
  - sent to every `MediaView` showing the track, through the named video
    sink;
  - passed to `on_frame` callbacks.
  - A decoder error or a gap requests a keyframe (PLI through
    `write_rtcp`).
- **Audio.**
  - Opus decoding feeds a small jitter buffer: 60 ms target, and 20 ms of
    packet-loss concealment through Opus's own PLC.
  - Then a mixer for all remote audio tracks, then the resampler to the
    output device's rate, then one cpal output stream for the process,
    opened on the first remote audio track.
  - `set_volume` scales that track before mixing.

### 3.5 Capture on desktop

rinch-av provides both, with fixes made first, in a PR of their own:

- **Microphone:** report the negotiated rate and channels, not the requested
  ones. Bump cpal 0.15 → 0.18. Resample in rinch-rtc rather than assume
  48 kHz.
- **Camera:** nokhwa 0.10 as today. Stop decoding MJPEG to RGBA only to
  convert it to I420 again; that is a later optimisation, not v1.

Screen sharing (`LocalTrack::screen()` on rinch-screen-capture; Linux
PipeWire on desktop, `getDisplayMedia` on the web) fits the same send path
with openh264's `ScreenContentRealTime` mode. It is left for later (M10).

### 3.6 Audio processing

Browsers apply echo cancellation, noise suppression and gain control to
`getUserMedia` audio by default. Desktop gets none of that from cpal. Two
desktop peers on speakers will echo, and a desktop peer on speakers talking
to a browser makes the browser user hear themselves.
`webrtc-audio-processing` 2.1 wraps Google's module, but it is a C++ build.
M5 is whether v1 ships without it (and documents headphones) or with it.

### 3.7 As built (PR B2)

Where the native backend differs from 3.1–3.6, the code holds:

- **Video codecs (M2, decided 2026-10-27: VP8 and H.264; amended
  2026-10-28: VP8 behind `media-vp8`).**
  - VP8 is libvpx through `env-libvpx-sys` 5.1.3 with its `generate`
    feature (bindgen at build time), on the system's libvpx (pkg-config),
    only with the `media-vp8` feature (`["media-core", "dep:vpx-sys"]`,
    added to `media` or to `media-cisco-openh264`; alone a
    `compile_error!`). No
    `vpx-encode`: `native_codec/vp8.rs` calls the encoder and decoder
    directly (CBR, `kf_mode` disabled so keyframes come only on request,
    `cpu-used` 8, error-resilient). Without the feature the
    `VideoCodec::Vp8` variant and every VP8 path are cfg'd out.
  - **Order.** With `media-vp8`, a desktop peer registers VP8 first, then
    H.264 (no RTX; see the retransmission bullet). An answer follows the
    offer's order, so a call between two such desktops or with any browser
    uses VP8; H.264 is used where the offer puts it first (Safari). VP8
    first because every browser encodes it, and a Firefox without the
    OpenH264 plugin lists H.264 and then sends nothing (3.2). A browser's
    own offer usually lists VP8 first too. Without `media-vp8` a peer
    registers H.264 alone and answers a VP8-first offer with H.264, so
    desktop↔Chrome and desktop↔desktop calls are H.264, and a Firefox
    without the plugin sends that desktop no video.
  - **A sender's track is labelled, and its preferences reset.** webrtc-rs
    0.21 uses a track's codec twice. It becomes the transceiver's only
    codec preference: a VP8-labelled track offered VP8 alone and answered
    an H.264-only offer it then could not send (#1947's second review). And
    a sender is bound to the interceptors (the NACK responder, sender
    reports) only if that codec is among the negotiated ones: the
    unlabelled tracks the second review's fix used were never bound, so
    nothing was retransmitted (third review). So a video track is labelled
    with the codec negotiation will most likely pick (the far side's first
    supported one when its offer is already here, our first otherwise),
    and its transceiver's preferences are set again to every codec, that
    one first (`video_label`). Every m-line offers both codecs, and an
    answer lists only what the offer did. A negotiation can still leave
    the label out (a track added before an H.264-only offer arrives; an
    offerer's VP8 label answered by a peer without VP8), and such a
    sender used to send unbound: no retransmission or sender reports.
    rtc gives a sender's track no codec at negotiation, as it does a
    receiver's, so the worker relabels the sender itself just before rtc
    binds it (#1954): after a remote offer is applied, to the codec our
    answer will use; before a remote answer is applied, to the answer's
    first codec. A sender left unmatched anyway gets one warning.
  - **The sender encodes whatever was negotiated.** Each sender reads its
    codec from webrtc-rs's `get_parameters` (every 250 ms until known,
    then every 2 s, so a renegotiation that changes it is followed), and
    the pump encodes once per codec its senders want, not once per sender.
  - **The receiver picks its decoder per packet,** by payload type, from
    the receiver's negotiated parameters. A packet of another codec (a
    renegotiation from VP8 to H.264) replaces the decoder and the
    depacketizer, and waits for a keyframe.
  - Tests can reorder (`testing::prefer_h264`) or drop VP8
    (`testing::h264_only`). Both are read when a peer's connection is
    built, on the worker, shortly after `Peer::new`.
    `tests/media_codecs.rs` pins the order, an answer to an H.264-only and
    (with VP8) an H.264-first offer (both directions in H.264), and with
    VP8 a renegotiated change of codec; without VP8, the answer to a
    VP8-first offer (H.264 alone, both directions).
- **Features (M3).**
  - `media` builds openh264 from source.
  - `media-cisco-openh264` builds no H.264 code and loads Cisco's binary
    at run time (`media::load_openh264`). rinch-rtc makes the path
    absolute, reads the file once, checks its SHA-256 against Cisco's
    OpenH264 2.6.0 build for the platform, and loads that same file; a
    second call with another path is an error. Until it has loaded, peers
    offer and accept no H.264; a build with `media-vp8` as well has VP8
    meanwhile, one without it no video codec. (Before 2026-10-28 VP8 was
    in `media-core` and so always covered that wait.) A window remains
    between the hash and the
    loader opening the file again by path (it cannot load from memory);
    the guide says to keep the file where only the app's user can write.
  - Both turn on `media-core`, the code's gate. VP8 is in neither:
    `media-vp8` (`["media-core", "dep:vpx-sys"]`) adds it to either one,
    so a Cisco-binary app has VP8 without building H.264 from source
    (#1975's review; an earlier draft built it on `media`). On its own it
    is a `compile_error!`: `media-core` has no H.264 backend.
  - A remote offer whose live video m-line has no codec in common (a
    VP8-only offer to a build without `media-vp8`) is answered with that
    m-line rejected, as before, and reported through `on_error` naming the
    missing codec and its feature; webrtc-rs reports nothing.
- **A sender keeps one webrtc-rs track for life,** a
  `TrackLocalStaticRTP` that the worker packetizes into itself (one
  packetizer per sender, built once the codec is known), not a
  `TrackLocalStaticSample`.
  - webrtc-rs 0.21's `RtpSender::replace_track` does not bind the new
    track, so its writes fail. `replace_track` re-points the sender's
    forwarder to the new source instead; the SSRC and sequence numbers run
    on, as a browser's does.
  - `add_transceiver` makes its transceiver from a track of ours for the
    same reason.
- **Keyframes on request,** with a 10 s backstop. webrtc-rs's default
  interceptors stop PLI and FIR before the sending app, so a last
  interceptor (`KeyframeRequests`) passes them on, and the worker sets the
  sending pump's keyframe flag. A keyframe also goes out on a new sender,
  on enable, and once the codec is known.
  - At most one keyframe a second per encoder: a receiver repeats its PLI
    until a keyframe comes, and one still being paced out must not be
    followed by another. A request inside that second waits, it is not
    dropped, and it waits for each codec's encoder separately (one codec's
    keyframe does not answer for another's).
  - The receiver's `SampleBuilder` (512 packets) gives up a frame missing a
    packet after 500 ms (`with_max_time_delay`); without that a lost packet
    held every later frame.
  - **Retransmission.** The receiver's interceptors NACK a missing
    packet and the sender's resend it from their buffer, as itself (same
    SSRC and sequence number): RTX is not registered, because webrtc-rs's
    NACK generator tracks arrivals by SSRC before RTX is resolved, so a
    repair sent as RTX never counted as received (79 NACKs and 261 RTX
    packets for 5 losses in 10 s). A browser offering RTX is answered
    without it and resends the same way. Measured with a UDP relay
    dropping video RTP on the wire (320×240 box, 15 s): before, 0.5% loss
    froze the picture 12% of the time, 1% about 30%, 3% about 60%; after,
    0% at 0.5, 1 and 3%, in VP8 and H.264, one NACK per loss, no keyframe
    requested (`tests/media_wire_loss.rs`, 3%).
  - **A gap that is not repaired is acted on:** the assembler holds the
    frame while a retransmission may come; a gap still open after 200 ms
    (`KeyGate`, `REPAIR`) gets a PLI at once (again every 500 ms until a
    keyframe), and frames are held back from the decoder until the
    keyframe, so no corrupt picture is shown. The same holds at the start,
    after a codec change, and when the assembler reports a frame lost. A
    decode error also sends a PLI (at most one a second). After a 300 ms
    outage (`tests/media_loss.rs`, which drops packets after the
    interceptors, so nothing is retransmitted) the request reaches the
    sender within 400 ms and a clean picture is back within 1.5 s.
- **Bitrate.** Both encoders run at `set_max_bitrate` (or a default from
  the size), and a new cap reaches the running encoder (libvpx
  `vpx_codec_enc_config_set`, openh264 `ENCODER_OPTION_BITRATE`): no new
  encoder, no keyframe.
  - **Neither encoder holds a cap alone on a hard picture.** openh264 in
    its Bitrate mode with frame skipping ran at 2.6× the cap on noise, and
    has nothing to limit a keyframe's size: no max-intra setting;
    `ENCODER_OPTION_MAX_BITRATE` and `BITS_VARY_PERCENTAGE` changed nothing
    measurable; a QP floor is the only lever (a noise keyframe at 640×480
    is 172 KB at the default range, 66 KB at QP ≥ 42, 12.5 KB only at 51),
    and it would cost every frame. libvpx (CBR, frame dropping,
    `VP8E_SET_MAX_INTRA_BITRATE_PCT` at 30 × fps %, WebRTC's
    `MaxIntraTarget`) keeps a noise keyframe to 36 KB, which a keyframe a
    second (a lossy link's PLIs) still makes three times a 100 kbps cap.
  - **So the pump keeps a send budget per encoder:** the bitrate since the
    last frame, with half a second saved at most. While it is spent no
    frame is encoded, a requested keyframe included (the request waits; it
    is consumed only when a keyframe goes out), and an overshoot is paid
    back in skipped frames, up to 10 s. On noise at 100 kbps a keyframe is
    three (VP8) to seven (H.264) seconds of it. A new size makes a new
    encoder (a keyframe) but keeps the budget and its debt.
  - **libvpx runs on a synthetic clock:** each frame lasts one frame
    period and its pts is the frame count times that, also after frames
    the budget skipped. Given the real gap as duration it spent it all on
    the next frame (180 KB of noise at 300 kbps); given the wall clock as
    pts (cf3e999e), it read the skipped frames as a falling frame rate and
    made each frame larger, which the budget skipped more of. On
    camera-like content (a 640×480 gradient, a textured square moving, a
    drifting blob, sensor noise ±3) that was 4–12 fps at 300 kbps, and the
    rate stayed at 8–12 fps after the cap was lifted (#1947's third
    review). Now: 24–28 fps at 300 kbps, 30 at 1 Mbps and uncapped,
    settling within a second of a change; H.264, 30 throughout
    (`tests/media_frame_rate.rs`). The budget does bind on such content at
    300 kbps; it skips a frame in five or six.
  - **Pacing.** A frame's packets go out at 2.5 × the bitrate (WebRTC's
    pacing factor), not in one burst: 150-packet frames lost packets on
    the far side's socket even on loopback, and the receiver then asked
    for keyframes it also lost.
  - `tests/media_bitrate.rs` sets the cap mid-call on full-frame noise, in
    VP8 and in H.264 (checked to be what the call negotiated), at 300 and
    100 kbps, and checks the bytes produced over 6 s against the cap plus
    half a second and the largest frame.
- **RTP timestamps** advance by the time since the codec's last frame, not
  by 1/fps: the pump takes the latest picture at each tick, so that is
  capture time to within a tick, through skipped frames and the
  half-second repeats of a still source. The advance comes before the
  frame (`skip_samples`, then `packetize` with 0): `packetize`'s own
  `samples` is added after the packets, which put each frame's
  timestamp one gap late (third review; `tests/media_rtp_time.rs`).
- **Small and odd sizes.** openh264 takes nothing under 16 pixels a side,
  so a smaller picture is scaled up by a whole factor; an odd last row or
  column is dropped (`tests/media_sizes.rs`, both codecs: a 1×1 picture
  arrives, a 641×481 one is not sheared).
- **Mute is synthesised.** webrtc-rs reports no mute when negotiation stops
  a track's media. After each offer, answer and remote description, the
  worker mutes remote tracks whose transceiver no longer receives, and the
  next packet unmutes them.
- **Rollback (1.7):**
  - an incoming offer while ours is pending is applied as
    `SetLocal(rollback)` then `SetRemote(offer)`;
  - `Description::rollback()` rolls back whichever offer is pending.
- **Resampling is linear,** not rubato (3.3). It serves sources at any rate
  into 48 kHz Opus, and 48 kHz into the output device. It is good enough
  for voice and adds no dependency.
- **Opus loss handling.** Each remote audio track has an `OpusJitter`:
  packets are ordered by (extended) sequence number, duplicates and late
  ones are dropped, and one packet is held for a late arrival (more would
  outlast the mixer's 60 ms). A lost packet is rebuilt from the next one's
  in-band FEC when it has it, and concealed by Opus PLC otherwise, as long
  as the packets are (10 to 60 ms: the next packet's duration for FEC,
  the last one's for PLC).
  - **A jump is a resync.** A gap of more than 15 packets is not
    concealed but skipped, and a packet more than 50 behind starts the
    sequence again rather than being dropped as late. Two packets from any
    remote 0x7fff apart used to make 655 s of concealment in one call on
    the worker every peer shares; a jump of 0x8000 or backwards silenced
    the track for minutes.
  libopus puts FEC only in frames its voice detector calls speech, so
  a steady tone or music gets PLC (`jitter_tests` pins both).
  - The mixer's buffer is per track: 60 ms before playback starts, 300 ms
    at most. A buffer past 160 ms (the sender's clock faster than ours) is
    trimmed to 80 ms.
  - A source at a rate that isn't a multiple of 50 Hz (11.025 kHz) carries
    the remainder from one 20 ms frame to the next.
  - **Send latency cannot ratchet.** After a stall (the pump late, the
    process paused) a source is ahead of the 20 ms ticks; past 60 ms of
    backlog the oldest audio is dropped, down to one tick. A 300 ms pause
    used to stay 290 ms of latency for the rest of the call.
- **Memory.** #1947's review measured about 1 MB of RSS growth a call. It
  was the tests' audio capture (`testing::capture_audio_output` keeps every
  sample). With it cleared, RSS is flat over 60 calls under
  `MALLOC_ARENA_MAX=1`, and levels off with glibc's default arenas
  (`tests/media_lifetime.rs`). On a loaded host the allocator sometimes
  keeps one step (14 MB once in about fifteen runs, flat before and after),
  so the test fails on the smaller of two ten-call growths, which a leak
  raises and a step does not.
- **No audio device** (a CI runner) is reported once through `on_error`
  as `RtcError::Media`; decoding goes on. Tests read the mix through
  `testing::capture_audio_output`.
- **`MediaView` on desktop** is a div with both `data-rtc-view` (the
  identity it shares with the web's `<video>`; reserved) and
  `data-viewport` (the compositor's key, already reserved). The sink comes
  from `rinch_core::video_sink` (M6); rinch-video's factory now forwards
  there.
- **Codec cost on the CI runner** (ubuntu-24.04, release, codec_bench from
  the spike):

  | | C only | with nasm |
  |---|---|---|
  | H.264 encode 640×480 | 8.4 ms (6.8 ms on another run) | 2.3 ms |
  | H.264 encode 1280×720 | 19.8 ms (16.3) | 5.4 ms |
  | H.264 decode 640×480 / 1280×720 | 2.0 / 4.4 ms | 0.7 / 1.6 ms |
  | Opus 20 ms stereo enc / dec | 0.13 / 0.03 ms | same |

  With nasm a 720p30 call costs about a sixth of a core instead of
  two-thirds. CI installs nasm, and the guide tells apps to build with it.
- **Build cost on the CI runner** (debug, cold):
  - openh264-sys2's build script takes 28–31 s, with or without nasm
    (nasm's assembly builds in parallel).
  - opusic-sys's (cmake) takes about 20 s.
  - `cargo build -p rinch-rtc --features media --tests` over the same build
    without `media` adds 52–57 s.
  - Installing nasm is 8 s uncached, and it is cached with the other apt
    packages.
  - **VP8** (scratch PR #1949, 2026-10-26) adds `bindgen` 0.68.1 (3.7 s)
    and `env-libvpx-sys` (1 s, its bindgen run included); `clang-sys` was
    already built for nokhwa's v4l2-sys. Both run while openh264-sys2's
    build script (26–33 s) does, so the cold build's wall clock does not
    move measurably (82.7 s with VP8, 100.6 s for B2 without it: runner
    noise). `libvpx-dev` (1.14) installs in 2.5 s uncached.
  - **CI time.** The native media tests took 144 s on the runner in the
    feature-gated shard, about 150 s with the last round of test changes
    (`media_lifetime` 26 s, `media_loss` 25 s, `media_audio_rates` 24 s,
    `media_longrun` 20 s, `media_bitrate` 16 s).
    test-wasm builds `media_echo` (64 s cold) and `web_media_native`
    (30 s), which runs for 10 s in Chrome and 5 s in Firefox.

## 4. Rendering received video: the split path

The received-video path ends at the named video sink:

```
decoder thread: I420 -> RGBA (0.35 ms at 720p)
    -> VideoFrameSink(rgba, w, h)            // Send + Sync, any thread
    -> latest frame kept, redraw requested
    -> paint: node `data-viewport = rtc-<id>` damaged
    -> GPU window: split path, one write_texture per new frame
       (in-scene fallback per the split-layer record's rules)
    -> software renderer: drawn in paint order
```

- **Nothing new is needed in the renderer.** A remote video is drawn
  exactly as `rinch-video`'s mpv frames are, including the split-path
  costs, the fall-backs (opacity, rotation, overlap, `GameViewport` in the
  window) and the `contain` fit.
- **CPU conversion vs a GPU YUV shader (M7).** Converting on the CPU costs
  0.35 ms per 720p frame and keeps the sink RGBA. An I420 upload with a
  shader on the split path would save that conversion and 60% of the upload
  bytes, but it is renderer work in the path #1865 just stabilised, and the
  software and in-scene paths would still need RGBA.
- **Several remote videos** (a gallery) are several sinks. Each takes the
  split path only while the regions don't overlap (D7 of the split-layer
  record). Otherwise they draw in-scene, at the in-scene cost (about 3–7 ms
  per frame for a 640×360 surface on the reference GPU).
- On the web none of this applies: `<video srcObject>` is the browser's own
  pipeline.

## 5. Porting, then removing

### 5.1 `examples/video-call`

Rewritten on `rinch-rtc` (with `media`) and `rinch-ws`:

- Signalling is JSON over a `rinch-ws` socket to a room relay.
- The relay is `examples/rtc-room-relay`, about 150 lines on tungstenite
  (as `rinch-signaling-server` is today). It is an example, not a crate,
  because signalling is the app's own (M9).
- **UI:**
  - a local self-view and a remote `MediaView`;
  - mute and camera-off toggles (`set_enabled`);
  - a "send a test pattern instead" switch (`replace_track` to a
    `video_source`);
  - connection state.
- **The camera and microphone are requested only when the user clicks
  "Start camera".** The example opens neither on start, so it can be
  launched in CI and by agents without touching a device.

### 5.2 ui-zoo

- **The WebRTC section** becomes an in-page call between two peers in one
  window. It shows the API with no server.
  - By default it sends a generated test pattern and tone (`video_source`,
    `audio_source`). A "Use camera" button switches with `replace_track`.
  - It works the same in ui-zoo-desktop and ui-zoo-web.
  - Today's demo, which needs a relay, a second instance and a URL, goes
    away. `video-call` keeps the two-machine case.
- **The av section** (rinch-av devices, mic level, camera preview, screen
  share) does not use rinch-webrtc, and stays.
  - Today it opens the camera and the microphone as soon as the page
    renders (`sections/av.rs`).
  - The port moves both behind buttons, so that opening ui-zoo on that page
    touches no device. Listing devices starts no capture, and stays as it is.
  - Its camera preview can become a `MediaView` on a `LocalTrack` later.

### 5.3 Removal

- **Removed:**
  - `crates/rinch-webrtc`, `crates/rinch-signaling` and
    `crates/rinch-signaling-server`, with their workspace entries;
  - `examples/ui-zoo`'s dependencies on them, and ui-zoo-desktop's
    `rinch-webrtc/native`;
  - mentions in `README.md`, `AGENTS.md` and `docs/`.
  - D1 of the data-channel record is updated to point here.
- **Dependencies that leave:**
  - **str0m 0.7** and its non-optional OpenSSL. `openssl-sys` stays in the
    graph as rinch-rtc's test-only dependency (str0m 0.24 `openssl`, the
    interop tests' peer).
  - rinch-webrtc's unconditional `rinch-av` with `audio,camera`: cpal and
    nokhwa then come only through apps and ui-zoo.
- **CI:** nothing names these crates except the workspace-wide builds,
  which simply get smaller. No job changes beyond the new media tests in
  section 6.
- **Sequencing:** the port and the removal land in one PR after the media
  PRs, so the tree never has two call implementations side by side, and
  never has none (M12).

### 5.4 As built (step 4)

Where the ports and the removal differ from 5.1–5.3, the code holds:

- **The relay** (`examples/rtc-room-relay`) is about 290 lines, not 150:
  it keeps the old server's hardening (one thread per connection owning
  its socket, bounded per-peer queues, a full queue drops the peer, the
  state lock read through poisoning) and its eight tests, ported. Its
  only protocol is two notices of its own, `{"type":"peer-joined"}` to
  the peer already waiting and `{"type":"bye"}` to the one left behind;
  everything else is forwarded unread.
- **video-call** negotiates once. Both senders exist before the first
  offer: a generated pattern on the video sender, and an audio
  transceiver (`SendRecv`) with no track until the microphone. The side
  that was waiting offers on `peer-joined`. Screen sharing, which the old
  demo had through rinch-screen-capture, is not carried over (M10).
- **The ui-zoo section** sends a pattern back from peer B as well, so both
  peers receive. B's received audio is not played until the user asks
  (`set_playing`), so the page does not start a tone by itself. The call
  starts when the section renders (synthetic sources only); "Use camera"
  is the one device path.
- **The av section** opened the camera and the microphone while
  rendering, and never released them. Both now open from buttons and
  close on stop and on unmount. `rinch_av::testing` (a counted, deniable
  gate on every device open, behind rinch-av's `test-util` feature) is
  what its test asserts on.
- **CI does change** (5.3 said it would not): ui-zoo and video-call turn
  `media` on under `--workspace`, so every `test` shard installs nasm, and
  the native media tests also run in the integration-a-m shard now (H.264
  only). VP8 (M2) first made `media` need the system libvpx, so every
  shard, the docs build and the visual tests installed `libvpx-dev`; since
  M2's amendment (2026-10-28) only the jobs that build `media-vp8` do: the
  `feature-gated` shard (`--features media,media-vp8`), the `clippy` job's
  `media-vp8` lines (with `media` and with `media-cisco-openh264`), and
  `test-wasm`'s VP8 `media_echo`.
- **Dependencies that left `Cargo.lock`:** str0m 0.7, `openssl-src` (str0m
  0.7 built a vendored OpenSSL; `openssl-sys` now builds against the
  system's, for rinch-rtc's tests only), `audiopus_sys` and `sctp-proto`
  0.3 (0.10 stays, under str0m 0.24).
  cpal and nokhwa stay, through rinch-av in ui-zoo and in rinch-rtc's
  `media`.
- **Found on the way:** a `LocalTrack` still held when its thread exits
  (in a signal, at an app's exit) aborted the process: its drop reached
  rinch-rtc's thread-local tables after they were destroyed. It now
  tolerates that (`tests/media_drop_at_exit.rs`).

## 6. Tests

None of these tests touches a real device; every source is synthetic.

- **Native, peer to peer** (`tests/media_loopback.rs`, the `test` shard).
  Two rinch-rtc peers:
  - `video_source` sends the spike's moving box. The far side's `on_frame`
    must find the box where the sender drew it in at least 80% of frames,
    and nowhere else (the spike: 89 of 90).
  - `audio_source` sends a 440 Hz tone, and the far side's decoded audio
    must peak at 440 Hz (Goertzel). This needs a test hook to read the mixed
    output instead of opening cpal, so no audio device is needed in CI.
  - `set_enabled(false)` gives black frames and `on_mute`.
  - `replace_track` to a second pattern changes what arrives with no new
    offer.
  - `set_direction(Inactive)` with a renegotiation stops frames.
- **Web, in page** (`tests/web_media.rs`, Chrome and Firefox): the same with
  `canvas.captureStream` and an `OscillatorNode`.
  - The far side's `MediaView` `<video>` is read back through a canvas for
    the box check.
  - An `AnalyserNode` checks the tone.
  - No `getUserMedia`, so no permission prompt and no fake-device flags.
- **Browser ↔ native**, the codec interop.
  - `examples/media_echo` is a native rinch-rtc peer that decodes whatever
    it receives and sends it back, re-encoded. It sits behind the same
    `POST /offer` signalling as `lite_echo`, which CI already starts.
  - Chrome and Firefox send the box pattern and check the echoed video.
  - This is the test that pins openh264 ↔ browser H.264 in both directions,
    and Firefox's OpenH264 plugin in CI.
- **Unchanged:**
  - The data-channel suites.
  - The str0m ICE-lite interop tests stay data-only. Playweft's server has
    no media.
- **CI build cost:**
  - openh264's C build is about 1 minute of `cc` on this host, which most
    of the spike's 1 m 20 s release build went to.
  - libopus via cmake is shorter.
  - Both happen only where `rinch-rtc/media` is on: the media tests, the
    ported example, and ui-zoo.
  - libopus needs `cmake`, which the GitHub runner images ship.
  - openh264-sys2 has an optional nasm build path for its assembly. This
    host has no nasm, so the spike measured the C-only build, and I haven't
    checked the runners for nasm. Installing it is an option, not a need.

## 7. Dependency weight

- **Native, with `media`:**
  - On top of rinch-rtc today, five crates that rinch's lockfile doesn't
    already have: `openh264`, `openh264-sys2`, `opusic-sys`, `safe_arch`,
    `wide`. Plus `rubato` for resampling (not in the spike).
  - The C builds are openh264 (cc) and libopus (cmake).
  - Size: the spike's H.264 loopback binary is 7.77 MB stripped, against
    6.30 MB for the data-channel spike (the same webrtc-rs). That is about
    1.5 MB for openh264, the media engine and the interceptors.
- **VP8 (M2, `media-vp8` only since 2026-10-28)** adds the system libvpx
  (`libvpx-dev`), `env-libvpx-sys`, and **bindgen with libclang at build
  time**: the crate's pregenerated bindings stop below the libvpx in
  current distributions (1.14 on ubuntu-24.04, 1.16 on this host).
  `media` alone adds none of these.
- **Capture and playback** (cpal, nokhwa) come through rinch-av and are
  already in the tree.
- **Web:** web-sys feature flags only.
- **Without `media`:** nothing, which is the point of M1.

## 8. The spike

Branch `spike-1916`, commit `4a29efd4`, `spikes/1916/` (its own workspace,
not for merge). The host is an AMD Ryzen AI 9 HX 370, shared with other
agents' builds, so timings are indicative rather than benchmarks.

| What | Result |
|---|---|
| `codec_bench`: openh264 0.9.9, C only (no nasm), test pattern | 640×480@800 kbps: enc 10.91 / dec 2.96 ms per frame; 1280×720@1.5 Mbps: enc 22.31 / dec 5.62 ms; I420→RGBA 0.13 / 0.35 ms; rate control on target (799 / 1498 kbps) |
| `codec_bench`: Opus 0.4 (bundled libopus) | 48 kHz stereo 20 ms: enc 0.143 ms, dec 0.038 ms, ~100 kbps |
| `codec_bench`: VP8, libvpx 1.16 (system) | enc 7.86 ms (640×480), 17.57 ms (1280×720); `env-libvpx-sys` failed to build until its `generate` (bindgen) feature was on |
| `h264_loopback`: webrtc-rs 0.21, two peers, `TrackLocalStaticSample` → `on_track` → `SampleBuilder<H264Packet>` → openh264 | connected in 0.23 s; 90 frames sent, 89 decoded, all 89 showing the box where the sender drew it |
| Size | `h264_loopback` 7.77 MB stripped (data-channel spike: 6.30 MB) |

## 9. Implementation sequence

1. **rinch-av fixes** (its own PR): the negotiated microphone format, the
   cpal bump, the web microphone and camera lifetime bugs.
2. **The rinch-rtc `media` API and the web backend:** `LocalTrack`,
   `Sender`, `Transceiver`, `RemoteTrack`, `MediaView`, and the in-page web
   tests.
3. **The native backend:** codecs, encode and decode threads, audio
   playback, the sink hook (M6), the native loopback tests, and
   `media_echo` with its browser interop step in CI.
4. **The ports and the removal** (5.1–5.3), with the user guide page and an
   internals section. Done; as built in 5.4.

## Decisions

**Decided by Joe, 2026-10-26: M1–M12 as recommended (M3: option (c)).**

**M1. Where media lives.**
(a) In `rinch-rtc`, behind a `media` feature that is off by default.
(b) A separate crate, `rinch-rtc-media`, on top of `rinch-rtc`.
(c) Always on in `rinch-rtc`.
**Recommend (a).** One `Peer` type and one crate. A data-channel-only app
(Playweft) pays nothing, and on the web the feature costs only bindings.
(b) would need `Peer` to expose a media hook to a second crate, a public
seam with little benefit.

**M2. The video codec on desktop. Decided by Joe, 2026-10-27: VP8 is
added, both codecs are negotiated, VP8 first (3.7).** The Firefox interop
test showed the gap the recommendation below left open.
**Amended by Joe, 2026-10-28: (c).** VP8 moves behind an opt-in
`media-vp8` feature (`["media-core", "dep:vpx-sys"]`, added to `media` or
to `media-cisco-openh264`); `media` is H.264 and
Opus alone, so that it needs no system package beyond a C compiler and
cmake. With VP8 in `media`, the cost in the second bullet below (libvpx,
and libclang for bindgen) reached every build of ui-zoo and video-call and
every CI job that builds the workspace. Where VP8 is built it is still
offered first. CI builds it
for the Firefox interop step, whose Firefox has no OpenH264 plugin, and
tests the default build against Chrome in H.264 (3.7).
(a) H.264 only, through `openh264` built from source.
(b) VP8 only, through the system libvpx.
(c) H.264 by default, plus VP8 behind a feature.
**Recommend (a), with (c) as a follow-up if the Firefox interop test shows
a gap.**
- H.264 builds anywhere with a C compiler, Android included, with no system
  library, and Chrome, Edge and Safari support it natively.
- VP8 would make every desktop build need libvpx installed, plus libclang
  for bindgen with today's libvpx. Its Rust bindings were last released in
  2023.

**M3. The H.264 patent position (a legal call).**
(a) Ship openh264 built from source, and say in the docs that H.264 patent
licensing is the app's own concern.
(b) Use `openh264`'s `libloading` feature to load Cisco's prebuilt binary at
run time, downloaded separately. Cisco's binary licence covers the MPEG LA
royalties when its conditions are met (a separate download, user control);
a source build is not covered by that arrangement.
(c) Both: the source build by default, and the Cisco-binary mode as a
feature.
**Recommend (c), with the docs stating the difference plainly.** This is
Joe's to decide, and possibly counsel's. VP8 (M2b) sidesteps the question.

**M4. Opus on desktop.**
(a) `opus` 0.4: bundled libopus, built with cmake.
(b) `opus-rs`, pure Rust: young, and unmeasured here.
**Recommend (a).** It is the reference implementation, the spike measured it
at 0.14 ms per frame, and cmake is on every runner. (b) is worth revisiting
for an all-Rust build.

**M5. Echo cancellation on desktop.**
(a) None in v1: document that desktop calls need headphones, and file a
follow-up.
(b) `webrtc-audio-processing` 2.1 (Google's module, a C++ build) in v1.
**Recommend (a).** It keeps v1's build to C codecs. A browser peer still
cancels its own echo, and Playweft's first use is game voice and video,
where headsets are usual. Choose (b) if speaker-to-speaker desktop calls
matter at launch.

**M6. How rinch-rtc reaches the video sink on desktop.**
(a) Move the frame-sink factory from `rinch-video` into `rinch-core`, as a
generic "named video sink" hook the shell installs once. rinch-video and
rinch-rtc both use it.
(b) rinch-rtc depends on `rinch` and calls
`render_surface::create_video_frame_sink` directly.
(c) No `MediaView` on desktop: the app wires `RemoteTrack::on_frame` to a
`RenderSurface` itself.
**Recommend (a).** It is a small move of an existing mechanism, keeps
rinch-rtc on `rinch-core` only, and gives `MediaView` the same behaviour on
both backends. (b) inverts the crate layering, and (c) makes every app
redo the wiring.

**M7. Colour conversion for received video.**
(a) On the CPU (I420→RGBA, 0.35 ms per 720p frame) into the existing RGBA
sink.
(b) An I420 upload plus a YUV shader on the split path.
**Recommend (a) for v1,** and (b) as a follow-up if many-stream galleries
show the cost. (b) is renderer work in #1865's path, and the software and
in-scene paths still need RGBA.

**M8. Capture on desktop.**
(a) rinch-av (cpal and nokhwa), fixed first in its own PR: the negotiated
format, cpal 0.18, and the web lifetime bugs.
(b) New capture code inside rinch-rtc.
**Recommend (a).** One device layer for the av section, the call and apps.
On the web, rinch-rtc uses `getUserMedia` directly instead (2.1): a sender
needs the track, not pixels.

**M9. Signalling for the ported call.**
(a) `rinch-ws` in the app, with a small room relay as an example
(`examples/rtc-room-relay`). Remove `rinch-signaling` and
`rinch-signaling-server`.
(b) Keep `rinch-signaling` as a crate, ported to `rinch-ws` and
`rinch-rtc`'s types.
**Recommend (a).** The data-channel record's rule is that signalling is the
app's own, and `rinch-ws` already carries it.

**M10. Scope of v1.**
(a) Camera, microphone, and app-fed video and audio sources; no screen
share.
(b) The same plus `LocalTrack::screen()` (rinch-screen-capture; Linux
PipeWire and web `getDisplayMedia`).
**Recommend (a), with (b) next.** Screen share fits the same send path, but
brings openh264's screen mode and PipeWire into the test matrix. It can be
added without changing the API.

**M11. CI tests.**
(a) All three tiers of section 6, with synthetic sources only: native peer
to peer, in-page in Chrome and Firefox, and browser ↔ native through
`media_echo`.
(b) Native and in-page only, with browser↔native checked by hand.
**Recommend (a).** Codec interop between openh264 and the browsers is the
riskiest part of this design, and only (a) tests it. It reuses the
`lite_echo` wiring that test-wasm already has.

**M12. When rinch-webrtc goes.**
(a) The port and the removal land together in one PR, after the media PRs
(section 9, step 4).
(b) Remove rinch-webrtc now. The demos break until the port lands.
**Recommend (a).** The tree never has two call stacks side by side, nor
none.
