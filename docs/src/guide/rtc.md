# WebRTC: Data Channels, Audio and Video

`rinch-rtc` gives an app peer-to-peer (or client-to-server) links over UDP:
WebRTC **data channels**, with the same shape as `rinch-ws`. Callbacks run on
the UI thread, bytes go in and out, and the API is the same on the desktop
and in the browser.

- **Desktop:** webrtc-rs 0.21. All peers share one worker thread, and STUN
  and TURN (UDP) gathering are built in.
- **Web:** the browser's `RTCPeerConnection`.

Use it when a WebSocket is the wrong tool:

- **NAT traversal.** ICE finds a direct path between two players behind home
  routers (STUN), or relays through a TURN server.
- **Unordered, unreliable delivery.** A lost packet doesn't stall the newer
  ones queued behind it: game ticks, cursors, voice-like data.

Audio and video tracks are behind the `media` feature (see
[Audio and video](#audio-and-video)).

```toml
[dependencies]
rinch-rtc = { workspace = true }
# or, for Serialize/Deserialize on the signalling types:
rinch-rtc = { workspace = true, features = ["serde"] }
```

There is no `native` or `web` feature: the backend follows the target. An
app that doesn't depend on `rinch-rtc` gets none of its dependencies (tokio
and webrtc-rs on desktop).

## Signalling is yours

Two peers have to swap a few strings before they can talk: an SDP offer, an
SDP answer, and ICE candidates. `rinch-rtc` hands those out and takes them in
as plain values. You carry them over whatever channel you already have,
such as a `rinch-ws` socket or HTTP.

| Type | What it is |
|---|---|
| `Description { kind: SdpKind::Offer \| Answer, sdp: String }` | an offer or an answer |
| `IceCandidate { candidate, sdp_mid, sdp_m_line_index }` | one candidate (the browser's `RTCIceCandidateInit`) |

```rust
use rinch::prelude::*;
use rinch_rtc::*;

let peer = Peer::new(RtcConfig {
    ice_servers: vec![
        IceServer::stun("stun:stun.example.org:3478"),
        IceServer::turn("turn:turn.example.org:3478", user, password),
    ],
    ..Default::default()
})?;

// Pre-negotiated channels: declared with the same id and options on both
// sides, before the offer.
let control = peer.create_data_channel(
    "control",
    ChannelOptions::negotiated(0, Reliability::Reliable),
)?;
let tick = peer.create_data_channel(
    "tick",
    ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 }),
)?;

tick.on_message(move |bytes| world.update(|w| w.apply_tick(&bytes)));
control.on_open(move || status.set("connected".into()));

// Trickle: each local candidate goes to the other side as it is found;
// `None` is "no more".
let ws2 = ws.clone();
peer.on_ice_candidate(move |c| ws2.send_text(encode_candidate(c)));

// Offer, and send it.
let (p, ws2) = (peer.clone(), ws.clone());
spawn_local(async move {
    if let Ok(offer) = p.create_offer().await {
        ws2.send_text(encode_offer(offer));
    }
});

// When the answer and the other side's candidates arrive:
ws.on_message(move |m| match decode(m) {
    Msg::Answer(answer) => {
        let p = peer.clone();
        spawn_local(async move { let _ = p.set_remote_description(answer).await; });
    }
    Msg::Candidate(c) => peer.add_ice_candidate(c),
});
```

The answering side mirrors this: `set_remote_description(offer)`, then
`create_answer()`, and it sends the answer back.

- **`create_offer`, `create_answer` and `set_remote_description` are `async`.**
  Await them inside `spawn_local`. Each call is queued *when you make it*, in
  call order, on both backends. The future only waits for its result.
  `create_offer` and `create_answer` also set the local description.
- **`add_ice_candidate` is fire-and-forget.** A candidate that arrives before
  a remote description has been applied is held, and added once one is. A
  candidate the stack refuses is reported through `Peer::on_error`.
- **Trickle or not.** `RtcConfig::gathering` is `Gathering::Trickle` (the
  default) or `Gathering::Complete`. With `Complete`, `create_offer` and
  `create_answer` wait for gathering to finish and return an SDP that
  carries every candidate, and `on_ice_candidate` is never called. Send one
  message instead of many.
- **A server with a public address and ICE-lite** doesn't need the client's
  candidates at all: it learns the client's address from its connectivity
  checks. rinch's tests connect to a str0m ICE-lite peer exactly that way.
- **TURN only.** `ice_transport_policy: IceTransportPolicy::Relay` uses relay
  candidates only. On desktop, TURN is UDP only: `turns:` and
  `?transport=tcp` URLs are skipped (#1896, D11). Browsers support both.

## Peers

`Peer::new(config)` makes one peer connection. An app can have any number:
a host opens one per player.

| `Peer` | |
|---|---|
| `create_offer()`, `create_answer()`, `set_remote_description(d)` | signalling (`async`) |
| `add_ice_candidate(Option<IceCandidate>)` | a remote candidate; `None` is the remote end-of-candidates |
| `restart_ice()` | the next `create_offer` restarts ICE |
| `create_data_channel(label, options)` | a channel (see below) |
| `state()` | `PeerState`: `New`, `Connecting`, `Connected`, `Disconnected`, `Failed`, `Closed` |
| `close()` | close the peer and every channel |
| `downgrade()` | a `WeakPeer`, for callbacks that need their own peer |
| `on_ice_candidate`, `on_connection_state`, `on_data_channel`, `on_error` | callbacks |

`Peer` is cheap to clone (an `Rc`) and `!Send`. Create and use it on the UI
thread. **Keep a handle:** a peer closes when its last clone drops. A local
that goes out of scope at the end of the component function, or an `async`
task that finishes, takes the peer with it. Keep it in a store, or in
something that lives as long as the component:

```rust
let keep = peer.clone();
rinch_core::reactive::on_cleanup(move || drop(keep));
```

## Channels

```rust
// In-band: announced to the other side, which hears it in on_data_channel.
let chat = peer.create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))?;

// On the other side:
peer.on_data_channel(move |ch| {
    ch.on_message(move |bytes| log.update(|l| l.push(bytes)));
    channels.update(|c| c.push(ch)); // keep it; dropping the handle doesn't close it
});
```

- **Reliability:** `Reliability::Reliable` (ordered, reliable) or
  `Reliability::Unreliable { max_retransmits }` (unordered; each message is
  retransmitted at most that many times, and `0` means sent once).
- **Pre-negotiated** (`ChannelOptions::negotiated(id, reliability)`): both
  sides declare the same id and options. There is no announcement and no
  `on_data_channel`. Declare negotiated channels before any in-band one, and
  keep their ids low. An id already used by an open channel of the same peer
  is refused with `RtcError::InvalidState`.
- **Declare at least one channel before `create_offer`**, so that the offer
  carries the data section.
- **Callbacks:** `on_open`, `on_message(Vec<u8>)`, `on_close` (once),
  `on_error` (followed by `on_close`), `on_buffered_amount_low`.
- **Bytes only.** A text message from a browser peer arrives as its UTF-8
  bytes.
- **`id()`** is the SCTP stream id. A negotiated channel reports its id. On
  the web, an in-band channel reports the id the browser gives it, which is
  at creation once the connection's DTLS role is known and otherwise when
  the channel opens. On the desktop an in-band channel never reports one.

### Sending, back-pressure and message size

`send(bytes)` either queues the whole message or refuses it, and queues
nothing when it refuses:

| `SendError` | When |
|---|---|
| `NotOpen` | the channel is still connecting, or is closing or closed |
| `TooLarge { len, max }` | the message is larger than `max_message_size()` |
| `QueueFull { buffered, limit }` | it would take `buffered_amount()` past the channel's `send_buffer_limit` |

- **`max_message_size()`** is the other side's `a=max-message-size`, applied
  the same way on both backends. If the SDP doesn't say, it is 64 KiB
  (RFC 8841); `0` means no limit. The desktop accepts, and so can send, up to
  256 KiB, which is what Chrome advertises. Keep messages under the smallest
  limit your peers advertise, and chunk anything larger yourself.
- **`send_buffer_limit`** is 1 MiB per channel by default, and at most
  16 MiB (`ChannelOptions::with_send_buffer_limit`). A message onto an empty
  queue is always admitted, even one larger than the limit.
- **Back-pressure.** On a reliable channel, stop at `QueueFull` and resume in
  `on_buffered_amount_low`. Set the level with
  `set_buffered_amount_low_threshold` (default 0: fires when the queue
  drains). On an unreliable channel, it is usually better to drop the tick
  and send the next fresh one.

```rust
bulk.set_buffered_amount_low_threshold(256 * 1024);
let pump = move || {
    while let Some(chunk) = next_chunk() {
        if let Err(SendError::QueueFull { .. }) = bulk.send(chunk.clone()) {
            put_back(chunk);
            return; // on_buffered_amount_low calls pump again
        }
    }
};
```

## States, errors and reconnecting

- **`Disconnected`** is transient: connectivity checks are failing (a Wi-Fi
  handoff, a blip), and the stack keeps trying. It may come back to
  `Connected` by itself.
- **`Failed`** means ICE gave up. Recover with an ICE restart: the offerer
  calls `restart_ice()`, then `create_offer()`, and sends that offer as
  usual. Channels survive a restart that succeeds. If the restart fails
  too, build a new `Peer`.
- **`Closed`** is final.
- **`Peer::on_error`** reports ICE candidate errors (a TURN server refusing
  the credentials, an unreachable STUN server) and refused remote
  candidates. These are informational: the state says whether the peer
  still connects. A failed signalling call is the `Err` of its future.

Each backend keeps its own stack's timers for when `Disconnected` and
`Failed` happen. rinch does not reconnect on its own, because signalling is
yours.

## Lifetime

A peer created **while a component renders** closes when that component
unmounts, silently: its callbacks belonged to the component. That releases
its sockets and any TURN allocation, and the other side sees the channels
close. Each callback is also dropped when the component that registered it
unmounts, as in `rinch-ws`.

To keep a peer past its component (a lobby link kept in a store across
route changes), create it outside any render or under `unowned`:

```rust
let peer = rinch_core::reactive::unowned(|| Peer::new(config))?;
```

Such a peer lives until `close()`, or until its last clone drops. A callback
that captures a clone of its own peer keeps it alive; capture
`peer.downgrade()` instead.

**At process exit** (desktop), peers that are still open, and closes already
under way, are finished on the way out, so the other side sees them go at
once rather than when its ICE checks time out. This takes at most about half
a second.

- On Linux, macOS and Android, both returning from `main` and
  `std::process::exit` do it.
- On Windows, only returning from `main` does it: call `rinch_rtc::shutdown()`
  before `std::process::exit`.
- A process killed outright closes nothing.

`shutdown()` closes every peer on demand, on both backends.

## Without `App::run`

Events reach the UI thread through rinch-core's main-callback queue, and a
rinch app's event loop drains it. A program without one, such as a CLI tool,
drives it itself on the thread that made the peers:

```rust
rinch_core::register_main_thread();
let peer = Peer::new(config)?;
loop {
    rinch_core::drain_main_callbacks(); // runs callbacks, resolves the futures
    // poll your futures, sleep a little
}
```

Until something drains it, the queue only grows, payloads included. A peer
created in `main` before `App::run` is fine: its events wait for the event
loop.

## Audio and video

Turn on the `media` feature:

```toml
rinch-rtc = { workspace = true, features = ["media"] }
```

- **On the web** it uses the browser's own capture, codecs and playback.
- **On desktop** it uses:
  - video in **H.264** (openh264, built from Cisco's C source), and Opus
    audio (libopus, built from source);
  - with **`media-vp8`** as well, video in **VP8** (the system's libvpx);
  - capture through rinch-av (cpal and nokhwa);
  - playback through one rinch-av output stream for the process.

  **Build requirements:** a C compiler and cmake (libopus). **Install
  nasm too** (`apt install nasm`, `brew install nasm`): openh264 then uses
  its assembly and encodes about 3× faster, so a 1280×720 call costs a
  sixth of a core instead of two-thirds. Without nasm it still builds, C
  only.

  **VP8 (`media-vp8`).** Turn it on to call a Firefox that has no OpenH264
  plugin (see [Firefox and H.264](#firefox-and-h264)):

  ```toml
  rinch-rtc = { workspace = true, features = ["media", "media-vp8"] }
  ```

  Add it to `media` or to `media-cisco-openh264` (on its own it is a
  compile error naming the two). With `media-cisco-openh264` it builds no
  H.264 code in, and gives video before `load_openh264` has run. It
  needs libvpx with its headers
  (`apt install libvpx-dev`, `brew install libvpx`; found by pkg-config)
  and libclang, since the VP8 bindings are generated at build time. On the
  web it changes nothing.

  **Which codec a call uses.** An answer follows the offer's order, and
  every browser offers VP8 first.
  - Without `media-vp8` a desktop peer offers and accepts H.264 alone, so
    every call it makes or answers negotiates H.264: desktop↔desktop,
    desktop↔Chrome and desktop↔Safari. A browser's VP8-first offer is
    answered with H.264, the codec both have. An offer whose video lists
    VP8 alone has that video rejected: the call connects without it, and
    the peer's `on_error` gets an `RtcError::Media` naming VP8 and
    `media-vp8`.
  - With `media-vp8` a desktop peer offers VP8 first, then H.264. So
    desktop↔Chrome, desktop↔Firefox and desktop↔desktop calls use VP8, and
    H.264 is used where the other side offers it first, as Safari does.
    VP8 comes first because every browser can encode it, while a Firefox
    without Cisco's OpenH264 plugin lists H.264 in its offers and then
    sends nothing.

  A desktop answers an offer with only the codecs it lists (an H.264-only
  offer gets an H.264 answer, and is sent H.264), and follows a
  renegotiation that changes the codec.

  **Bitrate on hard pictures.** `set_max_bitrate` holds in both codecs.
  A keyframe of a very busy picture can be larger than a second's worth
  of a low cap, though (openh264's several times larger than VP8's), and
  the frames after it are skipped to pay it back, so such video pauses
  after each keyframe. Ordinary camera pictures are not that hard: a
  busy 640×480 picture with sensor noise keeps 24–28 fps in VP8 at
  300 kbps and 30 fps in H.264.

  **Packet loss** is repaired by retransmission (NACK): on a link losing
  3% of video packets the picture does not freeze. A lost packet that
  cannot be recovered within about 200 ms gets a keyframe request, and
  nothing corrupt is shown meanwhile.
  Pictures under 16 pixels a side are scaled up.

### H.264 and its patents: `media` or `media-cisco-openh264`

H.264 is patented. How a desktop build gets its encoder decides who
covers the licensing:

- **`media`** builds openh264 from Cisco's C source into your app. That
  build is *not* covered by Cisco's MPEG LA licence: H.264 patent
  licensing is then your app's own concern.
- **`media-cisco-openh264`** builds no H.264 code in. Your app downloads
  Cisco's prebuilt OpenH264 library itself, from
  `ciscobinary.openh264.org`, separately from your app, as Cisco's terms
  require (<https://www.openh264.org/faq.html>).
  - Cisco ships it bzip2'd (`libopenh264-2.6.0-linux64.8.so.bz2` and
    so on). Decompress it.
  - Hand its path to `rinch_rtc::media::load_openh264(path)`.
  - Only **OpenH264 2.6.0** is accepted: the file's SHA-256 must be
    Cisco's 2.6.0 build for this platform.
  - The path is made absolute before the file is read. A second call with
    another file is an error.
  - The file is hashed, then the system loader opens it again by path, so
    whoever can replace it in that moment could have other code loaded.
    Keep it where only your app's user can write.
  - Until it has loaded, desktop peers offer and accept no H.264. With
    `media-vp8` as well they have VP8 meanwhile
    (`features = ["media-cisco-openh264", "media-vp8"]`); without it they
    have no video codec until then. Peers made after the load offer H.264.

  Cisco's binary licence covers the royalties when its conditions are
  met.

Pick exactly one of the two; `media-vp8` goes with either. VP8 and Opus
are royalty-free either way. This is not
legal advice; ask your counsel which fits your distribution.

The model is the browser's, simplified.

- A **`LocalTrack`** is something this app sends:
  - `LocalTrack::camera(..)` and `LocalTrack::microphone(..)` are futures that
    ask the user's permission (`getUserMedia`).
  - `LocalTrack::video_source(..)` and `LocalTrack::audio_source(..)` are
    tracks the app feeds itself: a game view, a test pattern, generated
    sound. Each returns a writer you push into from any thread:
    `push_rgba(rgba, w, h)` for RGBA8 frames (only the latest is kept), and
    `push(&[f32])` for interleaved samples (at most a second is queued).
- **`peer.add_track(&track)`** sends a track and returns a `Sender`.
  `sender.replace_track(Some(&other))` swaps what is sent without
  renegotiating, and `set_max_bitrate` caps it.
- **`peer.add_transceiver(kind, Direction::RecvOnly)`** asks to receive with
  nothing to send. `peer.transceivers()` lists them all.
  `transceiver.set_direction(..)` takes effect at the next negotiation.
- **`peer.on_negotiation_needed(..)`** fires when something changed that
  needs a new offer and answer: a track added, a direction changed. Run your
  offer/answer exchange again there.
- **`peer.on_track(..)`** hands over each `RemoteTrack`.
  - Remote audio plays by itself; `set_volume` and `set_playing` control it.
  - Remote video is shown with `MediaView`.
  - `on_mute`, `on_unmute` and `on_ended` report the track's state.
    `on_frame` hands over decoded RGBA frames, for analysis and tests only:
    on the web it reads the video back through a canvas, which is costly.
- **Mute** is `track.set_enabled(false)`: video sends black frames and audio
  sends silence. The device stays open. `stop()` releases it, and so does
  dropping the last clone of the track.

```rust
use rinch_rtc::media::{LocalTrack, MediaView, CameraOptions};

// From a click handler, so the browser allows playback and capture.
let camera = LocalTrack::camera(CameraOptions::default()).await?;
peer.add_track(&camera)?;
peer.on_track(move |remote| remote_video.set(Some(remote)));

rsx! {
    MediaView { track: camera.clone(), mirrored: true, style: "width: 160px;" }
    // ...and the remote one, keyed on the track.
}
```

`MediaView` shows the track with `object-fit: contain` on black, and its
track is fixed for the component's life: key the component on the track to
show another. Under the hood:

- **On the web** it is a `<video>` playing the track, muted (remote audio
  has its own element).
- **On desktop** it is a `div` the window's compositor draws decoded frames
  into. Remote frames are decoded on a thread of their own, with RGBA
  handed to the renderer. It is black until the first frame.

Both carry `data-rtc-view` with the same view id. On desktop the div also
carries `data-viewport`, the compositor's key for a video surface, and
`data-viewport-ready="true"` once a frame has arrived.

**Desktop audio has no echo cancellation** (M5). Browsers cancel echo for
microphone audio, but desktop capture does not. Two desktop peers on
speakers will hear themselves, so desktop calls want headphones.

The usual signalling, callback and lifetime rules apply. Media callbacks run
on the UI thread and are dropped when the component that registered them
unmounts. Closing the peer ends its remote tracks.

**Autoplay.** Browsers let audio play and an `AudioContext` run only after
the user has interacted with the page. Start a call from a click.

- If remote audio is refused, the peer's `on_error` reports
  `RtcError::Media`, and a later `remote.set_playing(true)` from a click
  retries.
- An `audio_source` made before any interaction reports
  `track.status() == LocalStatus::WaitingForGesture` and sends silence until
  the page's next click, tap or key press.

**Other things worth knowing.**

- `sender.track()` is what the sender carries now: what `add_track` gave
  it, or the last `replace_track`.
- A source's writer reports `is_ended()` once its track is stopped or
  dropped. Stop producing frames then.
- A camera or microphone request whose future is dropped while the
  permission prompt is still open (the component that asked unmounted)
  leaves no device on.
- Unmounting a `MediaView` lets go of its track.
- A remote track that pauses (its sender went inactive) and comes back is
  the same `RemoteTrack`: it mutes, then unmutes. `on_track` does not fire
  again.

### A whole call: the examples

Two examples show the API end to end, with synthetic sources until you ask
for a device:

- **ui-zoo's WebRTC page** (`examples/ui-zoo/src/sections/webrtc_call.rs`):
  two peers in one window, desktop and web, with no server. One sends a
  generated pattern (`video_source`) and a tone (`audio_source`), the other
  a pattern back, and "Use camera" swaps the camera and microphone in with
  `replace_track`.
- **`examples/video-call`**: a call between two machines. Run
  `cargo run -p rtc-room-relay` where both can reach it, then
  `cargo run -p video-call` on each, and join the same room. Signalling is
  JSON over a `rinch-ws` socket; `examples/rtc-room-relay` forwards it and
  says when the other side arrives (`{"type":"peer-joined"}`, and the
  waiting side then offers) or leaves (`{"type":"bye"}`).

Both add every sender before the first offer: the video sender with a
generated pattern, and in video-call an audio transceiver
(`add_transceiver(MediaKind::Audio, Direction::SendRecv)`) that gets the
microphone later. Every change after that (camera on, back to the pattern,
mute, video off) is `replace_track` or `set_enabled`, and needs no new
offer.

**Ask for devices from a click.** Neither example opens a camera or a
microphone until the user presses a button, and an app should do the same:
the permission prompt then answers something the user did, and a page that
turns the camera on because it was opened surprises people.

### Glare: both sides offer at once

When both sides change something at the same moment (each adds a track,
say), both run `on_negotiation_needed` and both send an offer. The usual
answer is *perfect negotiation*:

- One side is **polite**; pick by anything both agree on, such as the
  lower peer id.
- On an incoming offer, a collision is `peer.signaling_state() !=
  SignalingState::Stable`. An offer still being made already counts, so you
  need no flag of your own.
- The **impolite** side ignores a colliding offer. Its own offer wins.
- The **polite** side just sets the incoming offer, which rolls its own
  back, then answers. Its change still needs sending, so it offers again
  afterwards.

```rust
// On an offer from the signalling channel:
let collision = peer.signaling_state() != SignalingState::Stable;
if collision && !polite {
    return; // ours wins; theirs is dropped
}
peer.set_remote_description(offer).await?; // polite: rolls ours back
let answer = peer.create_answer().await?;
send(answer);
// polite and collided: our own change still needs an offer
if collision {
    let offer = peer.create_offer().await?;
    send(offer);
}
```

`set_remote_description(Description::rollback())` undoes a pending offer
explicitly.

### Firefox and H.264

Firefox does H.264 through Cisco's OpenH264 plugin, which it downloads by
itself into the profile shortly after first start. Until it has the
plugin, Firefox still *lists* H.264 in its offers but encodes nothing.
A desktop app built with `media-vp8` prefers VP8, so its desktop↔Firefox
video works either way. Without `media-vp8` the call negotiates H.264, and
a Firefox without the plugin sends no video in it. A peer that only offers
H.264 hits the same.
