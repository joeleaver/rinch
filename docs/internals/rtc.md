# rinch-rtc (WebRTC data channels)

Read this page when your task touches `crates/rinch-rtc`: peers, data
channels, the native webrtc-rs worker, the web `RTCPeerConnection` backend,
or the interop tests against str0m. The design and the decisions (D1–D11,
decided 2026-10-25) are in `docs/design/webrtc-data-channels.md`. The user
guide is `docs/src/guide/rtc.md`.

## Shape

- `src/lib.rs`: the public types, `Peer` and `Channel`. Every method works
  on the main-thread registry, then posts to the backend.
- `src/registry.rs`: the main-thread half both backends share. It holds each
  peer's and channel's state, the callback slots, and the replies the async
  signalling calls wait for. A backend reports an `Event` for a peer id, and
  `dispatch` applies it.
  - The slots are rinch-ws's (`crates/rinch-ws/src/lib.rs`): each records its
    owner and `CallbackFrame`, is taken out while it runs, and is pruned (with
    its dead siblings swept) once its owner has gone.
- `src/native.rs`: webrtc-rs 0.21.
- `src/wasm.rs`: web-sys.

There are no cargo features for the backends: the target picks one.

## Native: one worker thread

- **The worker.** The first `Peer::new` starts a `rinch-rtc` thread running a
  current-thread tokio runtime for the life of the process.
  - A router task owns a map from peer id to a command sender.
  - Each peer is one task that owns its webrtc-rs `PeerConnection` and works
    its commands **in order**. So a peer's calls keep the app's call order,
    and a slow one (an offer waiting for `Gathering::Complete`) holds up only
    that peer.
  - Each channel has a pump task that forwards `DataChannel::poll()` events.
    For a channel the remote side opens, the handler first hands it to the
    peer task (`Adopt`, which starts its pump), and only then announces it
    to the main thread (`Incoming`). So a call the app makes from inside
    `on_data_channel`, such as `close()`, finds the channel already there.
    The handler must not await, or it stalls webrtc-rs's driver.
- **Events go out through `dispatch_main_callback`, not
  `run_on_main_thread`.** The latter panics on a thread with no dispatcher,
  so a peer created before `App::run` would kill the worker. The former
  queues until a host drains. The same bug in rinch-ws is #1912.
- **A panic in the stack** ends only that peer: a watcher task turns the
  join error into `Error(Internal)` followed by `Closed`. A send to a dead
  worker does the same through a deferred dispatch.
- **Settings:**
  - `with_sctp_max_message_size(256 KiB)`. webrtc-rs sends at most what it
    advertises itself, and its default of 64 KiB capped every send below a
    remote's 256 KiB.
  - `with_data_channel_send_buffer_limit(16 MiB)`, the backstop behind the
    main thread's own per-channel limit.
  - `rtc` is a direct dependency only to name `SctpMaxMessageSize`, which
    `webrtc` does not re-export.
- **End-of-candidates** (`on_ice_candidate(None)`) is emitted once per
  gathering round. webrtc-rs can report `Complete` more than once per round,
  and does not always report `Gathering` for an ICE restart, so the restart's
  offer starts a new round itself.
- **Process exit.**
  - `atexit(shut_down_at_exit)` is registered when the worker starts. It
    sends `Shutdown` and waits up to about 600 ms. The router closes every
    peer still in its map, **and** waits for the closes already forwarded.
    Every forwarded `Close` carries a completion signal. Without that,
    `close(); exit()` died before the close was sent.
  - `rinch_rtc::shutdown()` does the same on demand, and then removes every
    registry entry, so each peer reads `Closed`.
  - The `atexit` handler touches only the worker channel, never the
    (thread-local) registry.
  - On Windows, `std::process::exit` ends the process through `ExitProcess`,
    which may skip the exe's `atexit` table. The docs tell apps to call
    `shutdown()` first.
- **Test hooks** (`rinch_rtc::testing`, `#[doc(hidden)]`):
  - `set_bind_addrs`: tests bind `127.0.0.1:0`, because webrtc-rs leaves
    loopback out of what it gathers from a wildcard bind.
  - `set_ice_timeouts`: the Disconnected → Failed test.
  - `channel_settings`: the channel's ordered, `max_retransmits` and
    negotiated values as webrtc-rs holds them, asked of the worker directly.

## Main-thread rules that are easy to break

- **Remote candidates wait** in `early` until a remote description has
  been **applied**, not merely requested. Each `set_remote_description` is
  tracked separately (`remote_pending`, one entry per call). Its successful
  reply sets `has_remote`, records the SDP's `a=max-message-size`, and
  flushes `early`. It does this in `dispatch`, so it happens whether or not
  the app awaits the future.
- **`max_message_size()` is `min(stack's figure, the remote SDP's
  a=max-message-size)`.** An absent attribute means 64 KiB (RFC 8841), and
  `0` means no limit. It is applied in the registry so both backends agree:
  Chrome reports 256 KiB to a peer whose SDP says nothing.
- **`buffered_amount()` is a mirror**: `reported + pending`. `send` adds the
  message to `pending`, and the backend's `Buffered` event moves it into
  `reported`. It can overstate, never understate. On the web, `reported` is
  the browser's own `bufferedAmount`, so the mirror is exact.
- **`on_buffered_amount_low` fires on the stack's falling edge, ungated.**
  An earlier version gated it on the mirror, so a send made in between
  swallowed it and an app that sends only from `on_low` stalled.
- **A negotiated id already used** by an open channel of the same peer is
  refused. webrtc-rs's registry overwrote the first channel's entry and left
  it open but deaf. In-band ids are known only on the web: the backend
  reads the id the browser gives a channel at creation (once the DTLS role
  is known), so an unopened in-band channel's id is checked too.
- **After `close()`,** `state()` answers `Closed` at once, and only the final
  `Closed` event is reported. Late states from the stack are ignored.
  `Event::Closed` closes each channel (`on_close` once), then reports the
  peer's state, then removes the entry.

## Web

- **Signalling ops go through a per-peer queue worked by one local task.**
  The browser's operations chain orders only the calls it has been *given*.
  `create_offer`'s `setLocalDescription` is given only after its
  `createOffer` resolves, so a `set_remote_description` made meanwhile would
  otherwise overtake it.
- **No web-sys bindings** for `restartIce()` (`createOffer({iceRestart})` is
  used), the `onicecandidateerror` setter (an event listener is used), or
  `pc.sctp.maxMessageSize` (read through `Reflect`).
- **`close()` detaches every closure** (`set_on*(None)`,
  `remove_event_listener`) before they drop. The browser then fires nothing,
  so the deferred `Event::Closed` closes the channels in the registry.
- **`createDataChannel` goes through a catching `Function::call2`.** The
  web-sys binding doesn't catch, so a throw (a label over 65535 bytes, id
  65535) unwound out of wasm. A refused channel is reported as native
  reports it: `ChannelError`, then `ChannelClosed`.
- **An incoming channel is wired into the peer's map before `Incoming` is
  dispatched,** for the same reason as native's `Adopt` ordering.
- **A channel's closures are freed when it closes:** detached and dropped
  after its own `close` callback returns. `close()` also settles a pending
  `Gathering::Complete` wait: it removes the listener and resolves the
  promise, so the peer's task ends.
- **Test hooks** (`testing`, wasm): `channel_settings` (the browser
  channel's ordered, maxRetransmits and negotiated values),
  `browser_low_threshold`, `pending_gathering_waits` and `live_channels`.
- **An announced in-band channel may already be open** when `ondatachannel`
  fires. The backend dispatches `Open` there too. The registry opens a
  channel once, so the later `open` event is a no-op.

## Tests

- **Native** (`tests/`): each file is its own process, because
  `register_main_thread` is process-global. The cases are listed in each
  file's docs.
  - `interop_lite`: against `tests/common/lite.rs`, a str0m 0.24.1 ICE-lite
    peer shaped like Playweft's server (one UDP port, a host candidate, never
    told the client's candidates). It counts what it receives (the positive
    control) and records each channel's settings, which for in-band channels
    arrive over DCEP. Two harness facts:
    - its SCTP send buffer is raised to 1 MiB: str0m's 128 KiB default can
      never send a message over 128 KiB;
    - str0m does not deliver a close on a stream that has carried nothing
      yet, so the test closes a used one.
  - `two_peers`, `lifetime`.
  - `before_app_run`: no dispatcher, headless drains.
  - `exit_closes`: re-runs its own binary as a child that connects to a lite
    peer in the parent and exits four ways: open, `close()` then exit, drop
    then exit, and `shutdown()` then `_exit`.
- **str0m is a dev-dependency with `openssl`, not `rust-crypto`.**
  `str0m-rust-crypto` turns on dimpl's `rcgen` feature, which turns on
  rcgen's `aws_lc_rs`. Cargo unifies that into the rcgen webrtc-rs uses, so
  the tests would have made certificates with aws-lc instead of the ring
  that ships, and every workspace build would compile aws-lc-sys.
  `openssl-sys` is already built for rinch-webrtc.
- **Web** (`tests/web_*.rs`, Chrome and Firefox in `test-wasm`):
  - `web_in_page` needs no server.
  - `web_live_lite` talks to `examples/lite_echo.rs`, which CI starts. It
    advertises the primary IPv4, because Chrome offers no loopback
    candidates. The URL is baked in at build time as `RINCH_RTC_TEST_URL`.
    With `RINCH_RTC_REQUIRE_LIVE=1`, the test fails rather than skips when
    the URL is missing.
  - Locally, run Firefox through Playwright's bundled build against
    `wasm-bindgen-test-runner`'s interactive page (`NO_HEADLESS=1`).

## Media (the `media` feature, #1916)

Design record: `docs/design/rtc-media.md`. The web backend is here; native
media is the next PR (`native_media.rs` returns `NotSupported` until then).

- **Layout.** `media/mod.rs` holds the API and the main-thread state:
  - local tracks in their own thread-local `LOCALS` (they belong to no
    peer);
  - remote tracks and the `on_track` / `on_negotiation_needed` slots in the
    peer's registry entry (`PeerEntry::media`), so they follow the
    data-channel rules: a batch, owner pruning in `sweep_dead`, `on_ended`
    for every remote track when the peer closes (`media::peer_closed`).

  A backend reports `Event::Media(MediaEvent)` through
  `registry::dispatch`. `media/view.rs` is `MediaView`.
- **Web (`wasm_media.rs`).**
  - `attach` / `detach` are called from `wasm.rs`'s `new_peer` / `close`.
    They own `ontrack`, `onnegotiationneeded` and every remote track's
    closures. Detached closures are dropped after the current call, which
    may be one of them.
  - Everything a peer sends goes in one `MediaStream`, so the far side's
    `stream_ids` group one person's audio and video.
  - `addTrack`, `addTransceiver`, `setParameters` and
    `transceiver.stop` go through a catching `Function::apply`.
  - A transceiver gets its keys when first seen (ours, or one the far side's
    offer made). A sender is keyed separately, because `addTrack` may reuse
    an existing transceiver.
  - `video_source` is a detached canvas's `captureStream(fps)`, painted by a
    timer from the writer's latest frame.
  - `audio_source` is an `AudioContext` into a
    `MediaStreamAudioDestinationNode`. A `ScriptProcessorNode` feeds it at
    once; an `AudioWorklet` (module from a Blob URL, fed by a 10 ms pump
    through its port) replaces it once loaded.
  - A remote audio track gets a hidden `<audio data-rinch-rtc-audio>` in
    the body, removed on `ended` or close.
  - `MediaView` looks its `<video>` up by `data-rtc-view` on animation
    frames, for up to ~5 s, then sets `srcObject`, and `muted` as a
    property as well as the attribute.
  - `on_frame` uses `requestVideoFrameCallback`. A live stream's
    `currentTime` advances with the clock, frames or not, so rAF plus
    `currentTime` re-read the frozen last frame.
- **Trap: `MediaStreamTrack` and `MediaStream` have an inherent `clone()`**
  (JS `clone()`, which makes a *new* track), and it outranks `Clone::clone`.
  `track.clone()` handed the sender a copy that `set_enabled` never
  reached: the first test run sent the box while "disabled". Write
  `Clone::clone(&track)`. `testing::sender_track_id` pins it.
- **Tests:**
  - `tests/web_media.rs` runs two peers in the page with synthetic sources.
    It covers:
    - a moving box through `video_source`: at least 80% of frames show it,
      checked by its bright bounding box;
    - disabled sends black;
    - `replace_track` with no negotiation;
    - `set_max_bitrate`;
    - Inactive mutes the far side after renegotiating;
    - close ends the remote track;
    - a 440 Hz tone through `audio_source`, analysed on the far side;
    - `MediaView` mounted through rinch-web and read back;
    - recvonly and the refusals.
  - The crate's `webdriver.json` allows autoplay.
  - Native unit tests (`src/tests.rs`, `media_events`) dispatch media
    events by hand.
- **Review of #1933, as fixed:**
  - Remote tracks are deduplicated on the browser track: `track` fires again
    when a transceiver comes back from inactive.
  - `on_frame`'s loop is a `Loop` the `RemoteWeb` can stop. A
    requestVideoFrameCallback closure holds itself to re-register, and a
    video that presents no more frames never calls it again, so close and
    `ended` break it from outside.
  - `audio_source` keeps the AudioContext's `statechange` handler and a
    window gesture listener while the context is held (`LocalStatus`).
  - `getUserMedia` runs in its own task (`request`, with a one-shot
    channel), so a dropped future leaves no device on: `keep_local` stops a
    track whose handle is gone.
  - `MediaView` clears `srcObject` from a scope cleanup.
  - Signalling state is tracked in the registry per request kind
    (`SignalingOp`). An offer in flight reads as `HaveLocalOffer`.
  - Test hooks: `video_frames_taken`, `audio_samples_taken`,
    `sender_track_id`, `remote_audio_elements`, `live_local_tracks`.
  - CI runs `web_media` in Firefox after a null PulseAudio sink is started.
    It passed without one too, but rinch-av's capture needs it. A Chrome
    step without the autoplay flag pins `WaitingForGesture`
    (`RINCH_RTC_NO_AUTOPLAY`).
- **Known gaps:**
  - Remote audio routed to a chosen output device (`setSinkId`) is not
    exposed.
  - The worklet-or-processor choice is not reported.
  - The gesture that unblocks autoplay is the app's job.

## Known gaps

- On native, TURN is UDP only (D11; #1917).
- On native, an in-band channel's stream id is unknown (webrtc-rs 0.21 does
  not expose it).
- rtc-sctp's full-window escape may let an endless fragmented message grow
  without bound (#1913, upstream).
