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
    A channel the remote side opens is announced by the handler (`Incoming`).
    The handler then hands it to the peer task (`Adopt`), which starts its
    pump: the handler must not await, or it stalls webrtc-rs's driver.
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
  it open but deaf. In-band ids are known only on the web, after open.
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

## Known gaps

- On native, TURN is UDP only (D11; #1917).
- On native, an in-band channel's stream id is unknown (webrtc-rs 0.21 does
  not expose it).
- rtc-sctp's full-window escape may let an endless fragmented message grow
  without bound (#1913, upstream).
