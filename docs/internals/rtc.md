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
  `openssl-sys` builds against the system OpenSSL (libssl-dev), for these
  tests only (rinch-webrtc, whose str0m 0.7 built a vendored one, is gone).
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

Design record: `docs/design/rtc-media.md`. Features: `media` (openh264 from
source) or `media-cisco-openh264` (Cisco's binary, loaded at run time); the
code's gate is `media-core`, which both turn on. `media-vp8` (VP8 on libvpx)
is `["media-core", "dep:vpx-sys"]` and is added to either: alone it has no
H.264 backend, and `lib.rs` stops it with a `compile_error!` naming the two
valid combinations (without that, `media-core` alone fails with E0599 on
`OpenH264API::from_source`).

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
- **Desktop (`native_media.rs`).** The module comment lists the threads.
  Things that are not obvious:
  - **One webrtc-rs track per sender, for life.** webrtc-rs 0.21's
    `RtpSender::replace_track` swaps the track in its core but never binds
    the new handle, so its writes fail ("bind failed"). `replace_track` here
    re-points the sender's forwarder from one source to another, and the
    SSRC and sequence numbers run on, as a browser's replaceTrack does.
    `add_transceiver` makes its transceiver from a track of ours
    (`add_transceiver_from_track`) for the same reason. A transceiver that
    the far side's offer made still goes through webrtc-rs's
    `replace_track`.
  - **Senders are `TrackLocalStaticRTP`.** The worker packetizes each
    sender's frames itself (`forward`, one packetizer per sender), because
    the codec, and so the payloader, is known only after negotiation.
  - **The codec and payload type** come from `RtpSender::get_parameters`,
    polled on a 250 ms tick until known. They are kept per sender: the
    parameters of a replaced track report no codecs. The sender's codec goes
    into an `AtomicU8` the pump reads (`wanted_codecs`), so the pump encodes
    once per codec in use and nothing before any is known.
  - **Codec order.** `media_engine` registers VP8 (only with `media-vp8`)
    then H.264 (when `h264_available()`: built from source, or Cisco's
    binary loaded), no RTX. Without `media-vp8` the `VideoCodec::Vp8`
    variant, the VP8 payloader/assembler and `native_codec/vp8.rs` are
    cfg'd out, and `VideoCodec::from_mime("video/VP8")` is `None`, so a
    stream in VP8 is never decoded. Test hooks: `testing::prefer_h264` puts
    H.264 first, `testing::h264_only` drops VP8 (`media_echo --h264-only`);
    both are read when the worker builds the peer's connection, and both
    change nothing in a build without VP8, which is H.264-only already.
    `tests/media_codecs.rs` checks both builds: with VP8, the order and a
    VP8→H.264 renegotiation; without, that a VP8-first offer (a browser's,
    munged in) is answered with H.264 alone and flows both ways.
  - **An offer with no video codec in common is reported.** After a remote
    *offer* is applied, `PeerWorker::check_remote_offer` runs
    `unanswerable_video` over it: a live (port not 0) video m-line listing
    codecs, none of them in `video_codecs()`, gets one
    `on_error(RtcError::Media(..))` naming both sides' codecs and the cure
    (`media-vp8` for VP8, `load_openh264` for H.264 not yet loaded). The
    answer still rejects that m-line (port 0) and the call goes on; webrtc-rs
    itself says nothing, so before this a VP8-only offer to a build without
    `media-vp8` lost its video silently (#1975's review;
    `tests/media_vp8_only_offer.rs`).
  - **Trap: a sender track's codec label.** rtc uses it twice.
    `codec_preferences_from_encodings` makes it the transceiver's only
    codec preference (a labelled track offers that codec alone), and
    `interceptor_local_streams_op` binds the sender to the interceptors
    (NACK responder, sender reports) only if the label is among the
    negotiated codecs, so an unlabelled track is never bound and nothing
    is retransmitted. `video_label` picks the label (the remote offer's
    first supported codec, else ours) and the worker resets the
    transceiver's preferences to all codecs, the label first. rtc binds a
    sender once, in `start_rtp_senders`, when the description that
    completes its first negotiation is applied, so a label the
    negotiation leaves out would send unbound (no retransmission, no SRs).
    The worker relabels such a sender just before that binding (#1954):
    after a remote offer is applied (the binding comes with our answer),
    to the first codec in the sender's `get_parameters`; before a remote
    answer is applied, to the first codec of that sender's m-section
    (found by mid) that we have. `relabel` passes a fresh
    `TrackLocalStaticRTP` (same track id, stream and SSRC) to the
    sender's `replace_track`, which in webrtc-rs swaps only rtc's copy of
    the track: the webrtc-rs track the forwarder writes to stays bound
    and RTCP still reaches it by track id. Once a negotiation completes,
    `check_labels` marks each newly negotiated video sender settled and
    logs one `tracing` warning for one still unmatched; settled senders
    are never relabelled. `tests/media_unlabelled_codec.rs` counts SRs in
    a UDP relay for both shapes (answerer's track before an offer without
    VP8, offerer's VP8 label left out of the answer).
  - **No RTX.** rtc's NACK generator records arrivals by SSRC before RTX
    is resolved, so an RTX repair never counted and the same packet was
    NACKed every 100 ms. Without RTX registered the responder resends the
    original packet. `tests/media_wire_loss.rs` drops packets in a UDP
    relay between the peers, which is the only place loss is visible to
    the interceptors.
  - **Receive by payload type.** `read_remote` maps payload types to codecs
    from the receiver's `get_parameters` (refreshed on an unknown one) and
    rebuilds the decoder and `Assembler` when a packet's codec differs
    from the current one.
  - **Keyframes.** On request, plus a 10 s backstop. The default
    interceptors deliver no PLI/FIR to the sender
    (`TrackLocalEvent::OnRtcpPacket` fires only for packets an interceptor
    marks), so `KeyframeRequests`, last in the chain, marks them
    `Attribute::DeliverToApplication`; the worker's `poll_rtcp` sets the
    pump's keyframe flag. Also a keyframe on a new sender, on the codec
    becoming known, and on enable (which also re-reads the source's slot,
    so a still picture comes back).
  - **Receive.** `Assembler` is a `SampleBuilder` per codec (512 packets,
    `with_max_time_delay(500 ms)`: without it one lost packet stalled every
    later frame). A sequence gap still open after `REPAIR` (200 ms: time for a
    retransmission) sets `KeyGate::awaiting`: a PLI goes at once (and
    every 500 ms while it stays set), and frames are held back from the
    decoder until `VideoCodec::is_keyframe`; it starts set, a codec change
    sets it, and so does a jump of over 256 (`gate_tests`). The decoder thread sends a PLI on a decode error (at
    most once a second). The sender honours at most one keyframe request
    a second.
  - **Rate.** `PumpEncoder` per codec: a new cap goes to the running
    encoder (`VideoEncoder::set_bitrate`), and a send budget gates every
    frame, keyframes included (the request flag is cleared only when a
    keyframe goes out). libvpx gets `VP8E_SET_MAX_INTRA_BITRATE_PCT`, a
    fixed one-period duration per frame and a synthetic pts (frame count
    × period): given the real gap or the wall clock, its rate control read
    the budget's skipped frames as a slower stream and made larger frames
    (180 KB, or 4 fps on camera-like content at 300 kbps). Keyframe
    requests are per encoder (`PumpEncoder::key_pending`). `forward`
    paces a frame's packets at `Encoded::pace_bps` (2.5 × the bitrate).
  - **RTP timestamps** step by the time since the codec's last frame
    (`PumpEncoder::last_sent`), not by 1/fps, before the frame:
    `forward` calls `skip_samples` and then `packetize(…, 0)`.
  - **Sizes.** The pump scales a picture under 16 pixels a side up by a
    whole factor (openh264's minimum) and drops an odd last row or column.
  - **Mute.** webrtc-rs reports no mute when negotiation stops a track's
    media (inactive, sendonly on our side). `PeerWorker::refresh`, which
    runs after every offer, answer and remote description, mutes the
    remote tracks of transceivers that no longer receive. The reader
    unmutes on the next packet.
  - **Remote tracks are keyed on the stack's track id.** `on_track` again
    for the same id reuses the key and starts a new reader; no second
    `Track` event.
  - **Rollback.** `apply_remote` rolls our pending offer back
    (`SetLocal(rollback)`) before an incoming offer, because webrtc-rs has
    no implicit rollback. An explicit `Description::rollback()` goes to
    SetLocal or SetRemote by which offer is pending
    (`pending_local_description`). `testing::pending_descriptions` reads the
    native state back, so a test checks that a rollback happened, not just
    that it returned `Ok`.
  - **Mirror.** The main thread answers `transceivers()`, `mid()` and the
    directions from `MIRRORS`. `add_track` and `add_transceiver` push an
    entry at once; `refresh` fills in mids and negotiated directions. The
    direction *asked for* is the main thread's own and `refresh` leaves it
    alone (a SetDirection still on its way would otherwise be overwritten).
  - **Audio.**
    - Sources are resampled linearly to 48 kHz (`Resampler`), not with
      rubato as the record said: one small resampler shared with playback,
      no new dependency. Fine for voice; a music app may want better.
    - A source rate that isn't a multiple of 50 Hz (11025, 22050) carries
      its remainder from one 20 ms tick to the next.
    - Remote Opus goes through `OpusJitter` on the worker: ordered by
      extended sequence number, duplicates and late packets dropped, one
      packet held (`OPUS_HOLD`; more outlasts the mixer's 60 ms), a loss
      rebuilt from the next packet's in-band FEC or concealed by PLC. The
      encoder has in-band FEC on, expecting 10% loss. libopus writes FEC
      only for frames its voice detector calls active speech: a steady tone
      (the loopback tests' source) gets none, which is why `jitter_tests`
      encodes a gliding, syllabic buzz.
    - `OpusJitter` resyncs on a jump: more than `MAX_GAP` (15) packets
      ahead skips instead of concealing, more than `MAX_LATE` (50) behind
      starts again; concealment is one packet's duration (`frame`, the
      last decoded packet's; FEC uses the next packet's).
    - The send side trims a backlog past 60 ms down to one tick
      (`audio_pump`), so a stall leaves no lasting latency.
    - The per-track buffer: 60 ms before it starts, 300 ms at most, and
      trimmed to 80 ms past 160 ms (`DRIFT_HIGH` / `DRIFT_TARGET`: a sender
      whose clock runs fast).
    - The output's rate is published only once the stream is open (0 until
      then), so the resampler never runs at the wrong rate.
    - One mixer feeds one rinch-av output, opened on the main thread on the
      first remote audio. With no device (a CI runner) that is reported
      once as `RtcError::Media`, and decoding carries on.
    - `testing::capture_audio_output` replaces the device with a 20 ms
      pull into a buffer.
  - **Video sink (M6).** `rinch_core::video_sink` holds the factory the
    shell installs (`rinch::render_surface::create_video_frame_sink`) from
    `RinchRuntime::new`; `video_sink::installs_on_this_thread` lets a test
    see it. rinch-video's `set_frame_sink_factory` forwards to it, and so
    now takes a `Send + Sync` factory (an API break).
    `MediaView`'s sink is created on the main thread (the surface registry
    is the main thread's) and dropped by the view's scope cleanup.
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
    CI also runs `web_media_native` (a desktop peer, `media_echo`, against
    the browser) in Chrome over VP8 and over H.264, and in Firefox over VP8
    (CI's Firefox has no OpenH264 plugin). The VP8 echo (47812) is built
    with `media,media-vp8`; the H.264 echo (47813) is a plain `media` build
    copied to `media_echo_h264`, the build an app gets by default.
    It passed without one too, but rinch-av's capture needs it. A Chrome
    step without the autoplay flag pins `WaitingForGesture`
    (`RINCH_RTC_NO_AUTOPLAY`).
- **Known gaps:**
  - Remote audio routed to a chosen output device (`setSinkId`) is not
    exposed.
  - The worklet-or-processor choice is not reported.
  - The gesture that unblocks autoplay is the app's job.

## The call demos, and what they replaced (#1916, step 4)

`rinch-webrtc` (a str0m 0.7 media crate that never carried media),
`rinch-signaling` and `rinch-signaling-server` were removed in the same PR
that ported their two users onto rinch-rtc, so the tree never had two call
stacks or none (M12). Their workspace members went with them; str0m 0.7,
its vendored OpenSSL build (`openssl-src`), `audiopus_sys` and
`sctp-proto` 0.3 left `Cargo.lock`. str0m 0.24 stays, as rinch-rtc's
test-only interop peer, and with it `sctp-proto` 0.10.

- **`examples/video-call`** (two machines). JSON over a rinch-ws socket to
  `examples/rtc-room-relay`, which forwards text between the two peers of
  a room and adds `{"type":"peer-joined"}` (to the one already waiting)
  and `{"type":"bye"}` (to the one left behind). The waiting side offers
  on `peer-joined`; the newcomer is the polite one. Both senders exist
  before the first offer (video: `add_track` of a generated pattern;
  audio: `add_transceiver(Audio, SendRecv)` with no track), so "Start
  camera", "send a test pattern instead", mute and video off are all
  `replace_track` / `set_enabled`, never a renegotiation. The relay is a
  library with a thin binary (`rtc_room_relay::run`), so video-call's
  `src/tests.rs` starts one on an ephemeral port and runs two calls
  through it with no window: who offers, the pattern arriving both ways,
  "bye".
- **ui-zoo's WebRTC section** (`sections/webrtc_call.rs` for the logic,
  `webrtc.rs` for the rsx). Two peers in the window, signalling by direct
  hand-off. A sends a pattern and a 440 Hz tone, B a pattern back;
  `negotiate` offers A→B, then B→A only if B's transceiver found no m-line
  in A's offer (so it does not depend on whether the backend reuses B's
  `add_track` transceiver for A's m-line). The sources are fed by
  `set_interval` on the UI thread, which works the same on desktop and
  web. The call is made during the section's render: peers, timers and
  callbacks are the section's, and `on_cleanup` closes it.
  `tests/webrtc_in_page.rs` runs that logic with no window (the test
  thread as main thread, as rinch-rtc's own tests do).
- **Devices open only from a click.** video-call's "Start camera", the
  WebRTC section's "Use camera", and the av section's "Start camera" /
  "Start microphone" are the only paths to a camera or a microphone; the
  av section used to open both while rendering. Other buttons reach other
  devices: the av section's "Share screen" (screen capture) and the mic
  test's "Play" (an audio output). `rinch_av::testing` (behind rinch-av's
  `test-util` feature, off by default; ui-zoo's dev-dependency turns it
  on) counts every device open in rinch-av and can deny them before they
  reach hardware;
  `examples/ui-zoo/tests/av_opens_no_device.rs` renders the av section
  with devices denied and asserts zero opens, then clicks both buttons as
  its positive control.
- **A track held at thread exit.** A `LocalTrack` kept in a signal (or any
  thread-local registered before rinch-rtc's tables) is dropped during
  thread-local teardown, after `LOCALS` and `CAPTURES` are gone. Its drop
  uses `try_with` for them: with `with` it panicked there, which aborts
  the process (`tests/media_drop_at_exit.rs`).
- **A peer held at thread exit.** `PeerInner::drop` reaches the registry
  (`registry::remove_peer`), `native_media::SENDING` and, on the web,
  `wasm::PEERS` and `wasm_media::PEERS`, all with `try_with` (#1916, #1952).
  `tests/review_peer_drop_at_exit.rs` is the order-dependent shape (a peer in
  an app's thread-local); `tests/peer_drop_in_registry_teardown_1952.rs` the
  forced one, a peer owned by another peer's callback and so dropped by the
  registry as it is destroyed.
- **CI.** With ui-zoo and video-call in the workspace, every `--workspace`
  build compiles `media` (openh264 and libopus, both from source). Nothing
  in the workspace turns on `media-vp8` (VP8, on the system libvpx), so
  `libvpx-dev` is installed only where it is named: the `feature-gated`
  test shard (`cargo test -p rinch-rtc --features media,media-vp8`), the
  `clippy` job's `media,media-vp8` and `media-cisco-openh264,media-vp8`
  lines, and `test-wasm` (the VP8
  `media_echo`). The other shards, the docs build and the visual tests
  need no libvpx (until 2026-10-28 VP8 was part of `media` and they all
  installed it).

## Known gaps

- On native, TURN is UDP only (D11; #1917).
- On native, an in-band channel's stream id is unknown (webrtc-rs 0.21 does
  not expose it).
- rtc-sctp's full-window escape may let an endless fragmented message grow
  without bound (#1913, upstream).
