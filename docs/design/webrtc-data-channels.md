# WebRTC data channels (`rinch-rtc`)

**Status:** Decided (issue #1896). Joe accepted D1–D11 as recommended,
2026-10-25. Playweft confirmed D11 suits them: they keep a WebSocket route
for UDP-blocked players, advertise `a=max-message-size`, and chunk large
snapshots themselves. Implementation follows the PR sequence in section 8.
**Scope:** a cross-platform WebRTC **data channel** client in a new crate,
`rinch-rtc`, with the shape of `rinch-ws`: callbacks on the UI thread, bytes
in and out, and one API on web and desktop. Signalling transport, media
tracks, and STUN/TURN servers are out of scope, as the issue says.
**First user:** Playweft, moving its multiplayer link from a WebSocket to data
channels. Its hosted game server is a tokio str0m peer (ICE-lite, one fixed
UDP port, host candidates only). Players can also host each other across
the internet, and a browser can host too. Its channel layout per peer is one
reliable control channel and one unordered tick channel, both
pre-negotiated by id.

**Evidence:** a spike on branch `spike-1896` (`spikes/1896/`, its own cargo
workspace, not for merge) ran the three native exchanges this record relies
on. See [section 7](#7-the-spike).

## 0. What is already in the tree

The workspace already has **`crates/rinch-webrtc`**. It is a media-call crate
(`VideoCall`, `PeerConnection` with `add_track`/`on_track`) on **str0m 0.7.0**
for native (feature `native`, which also pulls `opus`) and web-sys for the
browser (feature `web`), and it is used by `examples/video-call` and the
ui-zoo WebRTC section, together with `crates/rinch-signaling`. It does not
fit this issue, and patching it would not get there:

- **No data channels** on either backend.
- **Callbacks are `Send` and are not on the UI thread.** Native callbacks run
  when the app calls `poll()` by hand; web callbacks run straight from the JS
  closure. Neither has rinch-ws's owner check, document frame, or batch.
- **Native binds one UDP socket and advertises `127.0.0.1`** as its only
  candidate. There is no gathering, STUN, or TURN, so it cannot reach
  another machine.
- **Without its features, the API compiles to stubs** that return "not
  enabled" errors at run time, and the web side has a second, `_async`-suffixed
  API.
- str0m 0.7.0 is seventeen minor versions behind (current: 0.24.1). The
  crypto, DTLS and SCTP layers have all been replaced since then (see
  [section 3](#3-native-backend)).

`rinch-rtc` is therefore a new crate, as the issue asks. What happens to
`rinch-webrtc` is D1.

## Summary

- **One `Peer` per remote party, any number per app.** Signalling is
  plain values the app carries itself: `Description { kind, sdp }` and
  `IceCandidate { candidate, sdp_mid, sdp_m_line_index }`.
  `create_offer`, `create_answer` and `set_remote_description` are `async`
  and run on the UI thread under `spawn_local` (D4). `add_ice_candidate`
  never fails for arriving early: rinch holds such candidates until the
  remote description is set.
- **Channels** are opened by the offerer with
  `create_data_channel(label, options)` and heard on the other side through
  `on_data_channel`, or pre-negotiated by id on both sides. Reliability is
  `Reliable` (ordered, reliable) or `Unreliable { max_retransmits }`
  (unordered).
- **Callbacks follow rinch-ws exactly.** Each channel has `on_open`,
  `on_message(Vec<u8>)`, `on_close` and `on_error`, and the peer has
  `on_ice_candidate(Option<IceCandidate>)` (where `None` means end of
  candidates), `on_connection_state` and `on_data_channel`. All of them run
  on the UI thread, each in a batch, and are pruned when the component that
  registered them unmounts.
- **Lifetime:** a peer created during a render closes when that component
  unmounts. Creating it under `unowned(..)` (or outside any render) opts
  out (D5).
- **Web:** `RTCPeerConnection` and `RTCDataChannel` through web-sys, with
  `binaryType = "arraybuffer"`.
- **Native: webrtc-rs 0.21** (`webrtc` crate, a thin async layer over its
  sans-IO `rtc` core), with `ring` crypto. All peers share one worker thread
  that runs a current-thread tokio runtime, and events reach the UI through
  `rinch_core::run_on_main_thread`, as rinch-ws's do (D2, D3). It does STUN
  and TURN gathering, trickle, end-of-candidates, ICE restart and
  pre-negotiated channels. str0m does none of the gathering, and TURN is a
  hard requirement. The spike connected a webrtc-rs client to a str0m
  ICE-lite peer shaped like Playweft's server.
- **Back-pressure:** each channel has a send-queue limit (1 MiB by default)
  that rinch enforces itself on both backends. Past the limit, `send`
  returns `Err(SendError::QueueFull)`, nothing is queued, and the channel
  stays open. Oversized messages are refused against the negotiated
  `max_message_size`, which is 64 KiB against Playweft's server (D7).
- **Reconnection:** the app sees `Disconnected` (transient), `Failed` and
  `Closed`. rinch never reconnects by itself. `restart_ice()` makes the next
  offer an ICE restart (D9).

## 1. API

### 1.1 Types

```rust
pub struct RtcConfig {
    pub ice_servers: Vec<IceServer>,
    /// `All` (default) or `Relay` (TURN only: `iceTransportPolicy: "relay"`).
    pub ice_transport_policy: IceTransportPolicy,
    /// `Trickle` (default): `create_offer`/`create_answer` return at once and
    /// candidates arrive through `on_ice_candidate`. `Complete`: they return
    /// once gathering has finished, with every candidate inside the SDP, and
    /// `on_ice_candidate` is not called.
    pub gathering: Gathering,
}

pub struct IceServer {
    pub urls: Vec<String>,          // "stun:host:3478", "turn:host:3478?transport=udp"
    pub username: Option<String>,
    pub credential: Option<String>,
}
impl IceServer {
    pub fn stun(url: impl Into<String>) -> Self;
    pub fn turn(url: impl Into<String>, username: impl Into<String>, credential: impl Into<String>) -> Self;
}

pub enum SdpKind { Offer, Answer }
pub struct Description { pub kind: SdpKind, pub sdp: String }

/// The browser's `RTCIceCandidateInit`, which is what every signalling
/// channel carries.
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_m_line_index: Option<u16>,
}

/// `RTCPeerConnectionState`.
pub enum PeerState { New, Connecting, Connected, Disconnected, Failed, Closed }
/// `RTCDataChannelState`.
pub enum ChannelState { Connecting, Open, Closing, Closed }

pub struct ChannelOptions {
    pub reliability: Reliability,
    /// `Some(id)`: pre-negotiated. Declared with the same id and options on
    /// both sides, with no in-band (DCEP) announcement and no `on_data_channel`.
    pub negotiated: Option<u16>,
    /// Bytes `send` may have queued before it refuses (section 5). Default 1 MiB.
    pub send_buffer_limit: usize,
}
impl ChannelOptions {
    /// In-band (announced over DCEP), default limit.
    pub fn new(reliability: Reliability) -> Self;
    /// Pre-negotiated by `id`, default limit.
    pub fn negotiated(id: u16, reliability: Reliability) -> Self;
}

#[non_exhaustive]
pub enum Reliability {
    /// Ordered and reliable: the TCP-like default.
    Reliable,
    /// Unordered, and each message is retransmitted at most
    /// `max_retransmits` times (0 means it is sent once).
    Unreliable { max_retransmits: u16 },
}
```

`Description` and `IceCandidate` derive `serde` behind a `serde` feature, so
an app can put them on its WebSocket without a mirror type. This is my small
call.

### 1.2 `Peer`

```rust
/// One RTCPeerConnection. `!Send`: lives on the UI thread with its callbacks.
/// Cheap to clone (an `Rc`); see 1.5 for when it closes.
#[derive(Clone)]
pub struct Peer { /* id + backend */ }

impl Peer {
    pub fn new(config: RtcConfig) -> Result<Peer, RtcError>;

    // Signalling. Each call is queued behind the previous one, in call
    // order, on both backends (the browser's operations chain; the native
    // command queue).
    /// createOffer + setLocalDescription. Returns the SDP to send.
    pub async fn create_offer(&self) -> Result<Description, RtcError>;
    /// createAnswer + setLocalDescription, after a remote offer is set.
    pub async fn create_answer(&self) -> Result<Description, RtcError>;
    pub async fn set_remote_description(&self, d: Description) -> Result<(), RtcError>;
    /// A remote candidate, or `None` for the remote end-of-candidates. Held
    /// by rinch until a remote description is set (a browser rejects one that
    /// arrives early). A malformed candidate is reported through `on_error`.
    pub fn add_ice_candidate(&self, c: Option<IceCandidate>);
    /// The next `create_offer` restarts ICE (new ufrag/pwd, regathers).
    pub fn restart_ice(&self);

    pub fn create_data_channel(&self, label: &str, options: ChannelOptions)
        -> Result<Channel, RtcError>;
    pub fn state(&self) -> PeerState;
    pub fn close(&self);

    // Callbacks: chainable, and each replaces any previous one, as in rinch-ws.
    pub fn on_ice_candidate(&self, cb: impl FnMut(Option<IceCandidate>) + 'static) -> &Self;
    pub fn on_connection_state(&self, cb: impl FnMut(PeerState) + 'static) -> &Self;
    pub fn on_data_channel(&self, cb: impl FnMut(Channel) + 'static) -> &Self;
    /// ICE candidate errors (a TURN server refusing credentials, an
    /// unreachable STUN server) and late add_ice_candidate failures. These
    /// are not fatal on their own: the state callback says whether the peer
    /// still connects.
    pub fn on_error(&self, cb: impl FnMut(RtcError) + 'static) -> &Self;
}
```

The calls are `async` on both backends (D4). On the web they wrap the
browser's promises (`wasm-bindgen-futures`). On native they post a command to
the worker and resolve when its reply comes back through
`run_on_main_thread`, a main-thread table of pending replies keyed by
request id, like `park_main_callback`'s. The futures are `!Send` and are
polled with `rinch_core::spawn_local`, which exists on every host (#1767):

```rust
// The host side of a peer: answering an offer that arrived over the app's WebSocket.
let peer = Peer::new(RtcConfig { ice_servers, ..Default::default() })?;
let control = peer.create_data_channel("control", ChannelOptions::negotiated(0, Reliability::Reliable))?;
let tick = peer.create_data_channel("tick",
    ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 }))?;
{
    let ws = ws.clone();
    peer.on_ice_candidate(move |c| ws.send_text(signal::candidate(c)));
}
peer.on_connection_state(move |s| status.set(s));
tick.on_message(move |bytes| world.update(|w| w.apply_tick(&bytes)));

ws.on_message(move |m| match signal::parse(m) {
    Signal::Offer(sdp) => {
        let (peer, ws) = (peer.clone(), ws.clone());
        spawn_local(async move {
            peer.set_remote_description(sdp).await?;
            ws.send_text(signal::answer(peer.create_answer().await?));
            Ok::<_, RtcError>(())
        });
    }
    Signal::Candidate(c) => peer.add_ice_candidate(c),
});
```

With `Gathering::Complete`, the same code sends a single answer that holds
every candidate, and `on_ice_candidate` never fires. Playweft starts with
trickle. Against an ICE-lite server, the client's candidates are not needed
at all: the spike never forwarded them, and the server learned the client's
address from its connectivity checks.

### 1.3 `Channel`

```rust
#[derive(Clone)]
pub struct Channel { /* peer id + channel key */ }

impl Channel {
    pub fn label(&self) -> String;
    /// The SCTP stream id. For an in-band channel, `None` until the DTLS
    /// role has been decided (as `RTCDataChannel.id` is `null` until then).
    pub fn id(&self) -> Option<u16>;
    pub fn state(&self) -> ChannelState;
    pub fn send(&self, bytes: impl Into<Vec<u8>>) -> Result<(), SendError>;
    pub fn buffered_amount(&self) -> usize;
    pub fn set_buffered_amount_low_threshold(&self, bytes: usize);
    /// The peer's advertised SCTP max-message-size (RFC 8841; 64 KiB if
    /// the remote SDP does not say).
    pub fn max_message_size(&self) -> usize;
    pub fn close(&self);

    pub fn on_open(&self, cb: impl FnMut() + 'static) -> &Self;
    pub fn on_message(&self, cb: impl FnMut(Vec<u8>) + 'static) -> &Self;
    pub fn on_close(&self, cb: impl FnMut() + 'static) -> &Self;
    pub fn on_error(&self, cb: impl FnMut(RtcError) + 'static) -> &Self;
    pub fn on_buffered_amount_low(&self, cb: impl FnMut() + 'static) -> &Self;
}

pub enum SendError {
    NotOpen,
    TooLarge { len: usize, max: usize },
    QueueFull { buffered: usize, limit: usize },
}
```

- **Bytes only** (D8). A text message from a browser peer arrives as its
  UTF-8 bytes, and there is no `send_text`.
- **Dropping a `Channel` handle does not close the channel.** A channel lives
  as long as its peer or until `close()`, so a handle that an
  `on_data_channel` callback did not keep does not silently tear the channel
  down. rinch-ws closes on drop, but a WebSocket has no owner other than its
  handle, and a channel does: its peer.
- **Pre-negotiated ids:** declare negotiated channels before creating any
  in-band one, and keep their ids low. In-band ids are picked by the stacks
  (even or odd by DTLS role). The spike's `control` 0 and `tick` 1 next to an
  in-band `chat` worked against str0m.
- **A channel declared before `create_offer`**, in-band or negotiated, is
  what gives the offer its `m=application` line. That a negotiated-only
  offer gets one on both backends is for a test to pin: every spike offer
  also had an in-band channel.

### 1.4 Threading and dispatch

This is the same contract as rinch-ws (`crates/rinch-ws/src/lib.rs`, module
docs and `invoke`), and its registry code is the model:

- `Peer::new` must be called on the UI thread. Callbacks live in a
  main-thread-local registry keyed by peer id and channel key, so they may be
  `!Send` and capture `Signal`s, `Rc`s, or an editor handle.
- Each callback **slot** records the ambient owner and the `CallbackFrame`
  (document and context root) when it is registered. At dispatch, a slot
  whose owner has gone is dropped and not put back, and the connection's
  other dead slots are swept (#183, #374). A live slot runs under its owner,
  in `rinch_core::batch`, with its frame entered. An ownerless one runs
  under `unowned`.
- Each callback is taken out of the registry while it runs, so a callback
  may close or drop the peer, or re-register, without a double borrow.
- On **web** the JS event closures call `dispatch` directly. On **native**
  the worker calls `run_on_main_thread(move || dispatch(id, event))`. Only
  ids and `Send` event payloads cross the thread boundary.

A stream form (`Peer::into_stream`, `Channel::into_stream`, like
`WsHandle::into_stream`, #1767) can follow later. It is not in this record.

### 1.5 Lifetime: cancel on unmount, with an opt-out

The standing rule (timers, `park_main_callback`, `fetch`, rinch-ws) is that
a resource that can outlive its component is cancelled when that component
unmounts, and an explicit opt-out keeps it. rinch-ws applies the rule to
**callbacks**: a slot is pruned when its owner dies, and the socket closes
only when its handle drops. A peer needs the stronger form (D5). A peer whose
callbacks have all been pruned still holds UDP sockets, a TURN allocation
(which a relay server bills or rate-limits), and an SCTP association that
keeps the remote side "connected" to nothing. Nothing would ever say so.

- `Peer::new` called **during a render** registers one `on_cleanup` on the
  ambient owner, and that cleanup closes the peer. Peers are created once
  per connection, not once per keystroke, so the per-call cleanup growth the
  rule warns about (registrations that pile up) does not arise. Callback
  slots are pruned as in rinch-ws.
- **Opt-out:** create it under `rinch_core::reactive::unowned(|| Peer::new(cfg))`,
  or outside any render (in `main`, in a store's constructor, in a detached
  callback). That peer has app lifetime and ends with `close()` or when its
  last clone drops. A game's lobby peer kept in a store across route changes
  is the case this exists for, and the guide's examples will show it.
- **Closing the peer** closes every channel (`on_close` fires on each live
  one), releases TURN allocations (webrtc-rs sends Refresh with lifetime 0),
  and reports `PeerState::Closed`.
- A callback that captures a clone of its own `Peer` keeps an unowned peer
  alive (a cycle through the registry). The docs say so, and `Peer::downgrade()`
  gives a `WeakPeer` for that case, as rinch-ws's docs use `Rc::downgrade`.

### 1.6 Deliberately left out

These are left out for now but may come later: media tracks, the stream
form, `getStats`, choosing the native bind address or port range,
`max_packet_life_time` and the other reliability combinations (D6),
certificates the app supplies, and peer-reflexive or mDNS controls.
Signalling transport and STUN/TURN servers are never in scope.

## 2. Web backend

`RTCPeerConnection` through web-sys. The structure is that of rinch-ws's
`wasm.rs`: the JS closures are owned by the backend, detached
(`set_on*(None)`) before they drop, and they call `crate::dispatch` directly.

**web-sys features** (all present in the 0.3.103 the lockfile has):
`RtcPeerConnection`, `RtcConfiguration`, `RtcIceServer`,
`RtcIceTransportPolicy`, `RtcSessionDescription`, `RtcSessionDescriptionInit`,
`RtcSdpType`, `RtcOfferOptions` (for `iceRestart`), `RtcIceCandidate`,
`RtcIceCandidateInit`, `RtcPeerConnectionIceEvent`,
`RtcPeerConnectionIceErrorEvent`, `RtcPeerConnectionState`,
`RtcIceGatheringState`, `RtcDataChannel`, `RtcDataChannelInit`,
`RtcDataChannelEvent`, `RtcDataChannelState`, `RtcDataChannelType`,
`MessageEvent`, `Event`. Also `js-sys` and `wasm-bindgen-futures`
(`JsFuture` around the promises).

web-sys 0.3.103 has no binding for `restartIce()`, the
`onicecandidateerror` setter, or `RTCPeerConnection.sctp`
(`maxMessageSize`). There are workarounds for each. ICE restart uses
`createOffer({ iceRestart: true })`, which the binding exposes. The error
listener goes on through `add_event_listener_with_callback("icecandidateerror", …)`.
`max_message_size` is read with `js_sys::Reflect` on `pc.sctp`, and
falls back to 64 KiB.

**Binary messages.** `binaryType` is set to `"arraybuffer"` when each channel
is created or received. An incoming `MessageEvent` whose data is an
`ArrayBuffer` becomes `Uint8Array::new(&data).to_vec()`, and a string becomes
its UTF-8 bytes. `send` calls `send_with_u8_array` after rinch's own checks
(section 5).

**`bufferedAmount` back-pressure.** `buffered_amount()` reads
`RTCDataChannel.bufferedAmount`, and the threshold and callback map to
`bufferedAmountLowThreshold` and `onbufferedamountlow`. Before calling the
browser's `send`, rinch refuses with `QueueFull` when
`bufferedAmount + len > send_buffer_limit`, and with `TooLarge` when
`len > max_message_size`. The browser's own overflow behaviour is never
reached, so rinch does not depend on it. The spec makes an over-full send
throw `OperationError`. What Chromium and Firefox actually do at their
limits is for the implementation PR's browser test to state per engine
(`tests/support/engine.rs`), not for this record to assume.

**Trickle and complete.** `onicecandidate` events with a candidate go to
`on_ice_candidate(Some(..))`. The null-candidate event, or the gathering
state reaching `complete`, gives a single `on_ice_candidate(None)`. In
`Complete` mode, `create_offer` awaits `icegatheringstatechange` reaching
`complete` and returns `localDescription.sdp`.

**Many peers** are independent `RTCPeerConnection`s. A browser can host,
answering N offers with N peers.

## 3. Native backend

### 3.1 The two stacks, as of 2026-10-25

| | **str0m** | **webrtc-rs** (`webrtc` + `rtc`) |
|---|---|---|
| Current version | 0.24.1 (2026-10-03); 0.21 → 0.24.1 since 2026-06-27, six releases | `webrtc` 0.21.1 / `rtc` 0.21.1 (2026-10-11); 0.21.0 2026-09-19 |
| Shape | sans-IO, sync, `&mut self`; no threads, no runtime | sans-IO core (`rtc`) plus an async driver (`webrtc`) that owns sockets, timers and gathering; runtime trait with tokio and smol built in |
| API | its own (`sdp_api`, `direct_api`, `poll_output`); the README's table says "Peer Connection API ✗" | W3C-shaped (`create_offer`, `set_local_description`, `on_ice_candidate`, `create_data_channel`), which is what the web backend wraps |
| ICE gathering | **none**: host candidates, STUN (srflx) and NIC enumeration are the user's job (README, "NIC enumeration and TURN") | host (from bind addresses), srflx (`stun_gatherer.rs`), relay (`turn_relayer.rs`), mDNS, ICE-TCP |
| TURN client | **none**: README feature table "Turn ✗"; the feature request (#723) is open | **UDP only**: `turn_relayer.rs` (1,201 lines) skips `turns:` and non-UDP TURN URLs with a warning |
| ICE-lite | yes (`set_ice_lite`), as server | as a full agent against a lite peer: yes (spike) |
| DTLS | its own `dimpl` (0.7.x, 0.8 in unreleased), crypto providers: aws-lc-rs (default), rust-crypto, openssl, apple, wincrypto | `rtc-dtls`, crypto providers ring (default) or aws-lc-rs, chosen per connection |
| SCTP / data channels | `sctp-proto` 0.10; in-band and negotiated; `buffered_amount` + low event; SCTP send buffer and receive message size configurable (0.24.1, unreleased #1083) | `rtc-sctp`; in-band and negotiated; low **and** high water events; per-channel send-buffer limit (`with_data_channel_send_buffer_limit`, default unbounded) |
| Trickle / end-of-candidates / ICE restart | trickle yes (you add candidates); restart `sdp_api().ice_restart` | yes / yes / `restart_ice()` |
| Maintenance | very active (repo pushed 2026-10-25; about 28 unreleased changelog entries since 0.24.1) | very active (pushed 2026-10-25); 0.20.0 (2026-07-31, alphas from March) was a full rewrite onto `rtc`, and 0.21 is "working towards a stable public API", pre-1.0; the old async line was 0.17.x |
| Normal dependencies (spike crate) | 103 (49 not already in rinch's `Cargo.lock`) | 170 (51 not already in rinch's `Cargo.lock`), tokio already present via `gpu` |
| C code | **aws-lc-sys, even with `rust-crypto`**: `str0m-rust-crypto` turns on dimpl's `rcgen` feature, which turns on dimpl's `aws-lc-rs` (cargo tree, below) | `ring` only (already in the tree for rustls) |
| Stripped release binary of the spike | 6.05 MB | 6.30 MB (includes tokio) |

The str0m C build is a packaging fact worth knowing whatever is decided.
`cargo tree -e features -i aws-lc-rs` on the spike's str0m crate shows
`str0m feature "rust-crypto"` → `str0m-rust-crypto` → `dimpl feature "rcgen"`
→ `dimpl feature "aws-lc-rs"` → `aws-lc-sys`. A str0m build therefore needs
a C toolchain on every target, Android included. It is worth an upstream
issue.

### 3.2 What TURN means for str0m

str0m has no TURN client, so choosing str0m means rinch writes the
gathering half of an ICE agent: everything webrtc-rs's
`stun_gatherer.rs` + `turn_relayer.rs` do, around a str0m `Rtc`:

1. **Local candidates:** enumerate interfaces (an `if-addrs`-style crate),
   bind a UDP socket per usable address, add host candidates, and
   handle IPv4 and IPv6.
2. **srflx:** send STUN Binding requests to each `stun:` server from each
   socket, and turn the mapped address into a server-reflexive candidate.
   str0m does not gather these either.
3. **relay:** a TURN client per (socket, `turn:` server). That covers the
   Allocate exchange with long-term-credential auth (401, then retry with
   realm and nonce, then 438 stale-nonce), and refreshing before the
   allocation's lifetime runs out (600 s by default). Each remote candidate
   needs a CreatePermission, renewed every 5 minutes, and ChannelBind for
   throughput. str0m's `Transmit`s whose source is the relayed address must
   be wrapped in Send indications or ChannelData. Data indications and
   ChannelData must be unwrapped back into `Input::Receive` with the relayed
   address as the destination. Allocations are released with a Refresh of
   lifetime 0 on close.
4. **Gathering state:** emit candidates as they arrive, end-of-candidates
   when every server has answered or timed out, and regather on ICE restart.
5. **TCP and TLS TURN** (`turn:…?transport=tcp`, `turns:`) for networks that
   block UDP: browsers do this, and webrtc-rs does not.

Usable sans-IO TURN clients exist: `turn-client-proto` 0.7.3 (2026-10-06,
ystreet; UDP, plus TCP/TLS through `turn-client-rustls` or
`turn-client-dimpl`) and `rtc-turn` 0.21.1 (webrtc-rs's, UDP). Either would
spare us the TURN message format and auth. Neither spares us items 1–4 or
the relay plumbing in 3. My estimate is 1,500–2,500 lines plus a TURN server
in the test suite, against a backend of roughly 600–900 lines over
webrtc-rs. That code would be ours to keep correct across NAT types we
cannot reproduce in CI.

### 3.3 Recommendation: webrtc-rs 0.21, one worker thread (D2, D3)

TURN is the hard requirement for players hosting each other across the
internet, and webrtc-rs is the only one of the two that has it, along with
the rest of gathering. The case for str0m was interop with Playweft's
server. That does not need str0m on the client: the spike's webrtc-rs client
reached a str0m ICE-lite peer on its first run. The remaining arguments are
fewer dependencies and a sync loop. The dependency count is a wash: 51 new
crates against 49, and str0m's 49 do not yet include a TURN client or NIC
enumeration. The sync loop is real but small, because webrtc-rs runs fine
on a current-thread tokio runtime on one thread (spike).

Costs of the recommendation, stated plainly:

- **No TURN over TCP or TLS on native** (D11). A player on a network that
  blocks all UDP cannot relay from a desktop client, though the same player
  in a browser can. Fixing it means an upstream contribution to
  `turn_relayer.rs` or a later rinch-side relayer.
- **A young API.** 0.20.0 (2026-07-31) was a full rewrite, and minor
  versions are allowed to break. rinch pins `=0.21.x` and keeps webrtc-rs
  behind its own types, so a bump touches only `native.rs`.
- **Slower to connect on loopback in the spike:** about 0.4–0.6 s to an
  open channel, against 64 ms for str0m↔str0m. The spike did not establish
  why (ICE check pacing and gathering timers are the obvious candidates).
  A game connects once, but the implementation PR should look.
- **tokio** with `rt`, `net`, `time` and `sync`. It is already in every
  `gpu` build and every notification and hotkey build. The `runtime-tokio`
  feature also compiles `rt-multi-thread`, which rinch-rtc does not use.

**Architecture.**

- `rinch-rtc` depends on `webrtc = { version = "=0.21.1", default-features = false, features = ["runtime-tokio", "crypto-ring"] }`
  and tokio for native, and on web-sys for wasm. These are target `cfg`s,
  as in rinch-ws, with no `native`/`web` features and no stubs. An app
  that does not depend on `rinch-rtc` pays nothing.
- **One worker thread per app** (`rinch-rtc`, started by the first
  `Peer::new`). It runs a current-thread tokio runtime that owns every
  peer. The UI thread sends it commands over a tokio unbounded channel:
  `NewPeer`, `CreateOffer{req}`, `SetRemote{req}`, `AddCandidate`,
  `RestartIce`, `CreateChannel`, `Send`, `SetLowThreshold`, `CloseChannel`
  and `Close`. Commands are FIFO, which gives the call-order guarantee
  in 1.2. Each peer is a webrtc-rs `PeerConnection` with a
  `PeerConnectionEventHandler` that forwards to the UI. Each channel has a
  task that polls `DataChannel::poll()` and forwards its events. This is
  exactly the spike's `dc-webrtc`. rinch-ws's one thread per socket would
  give a host of eight players eight threads, plus eight runtimes.
- **Bind addresses:** `0.0.0.0:0` (and `[::]:0` where available). Each peer
  gets its own sockets, which matches what a browser does.
- **No process-global crypto install.** webrtc-rs chooses its provider per
  connection, so the rustls `CryptoProvider` trap that rinch-ws documents
  does not apply. The `ring` feature is on by name anyway.
- **Android** is unverified. `ring` and `quinn-udp` build for Android
  generally, and `check-android` will say so in the first PR.

## 4. The interop target and its tests

The peer to interoperate with is Playweft's: str0m, ICE-lite, one fixed UDP
port, a host candidate only, answering. The spike's `dc-interop` built
exactly that: a webrtc-rs client offering with trickle, and a str0m peer
that is never sent the client's candidates. Both pre-negotiated channels
and an in-band one opened and echoed in about 0.65 s. A 60,000-byte message
round-tripped. A 200,000-byte send was refused by the client: str0m's
answer has no `a=max-message-size`, so RFC 8841's 64 KiB default applies to
everything sent to Playweft's server unless the server advertises more.

**Native test (the `test` shard).** `crates/rinch-rtc/tests/interop_lite.rs`
uses a test-support module `tests/support/lite_peer.rs`, which runs a str0m
ICE-lite echo peer on a thread over `UdpSocket::bind("127.0.0.1:0")`.
str0m is a **dev-dependency** only. Signalling is an in-process channel. The
test thread registers itself as the main thread and pumps the dispatcher
queue, as `crates/rinch-ws/tests/common/mod.rs` does, with one test file per
process. The cases:

1. Negotiated `control` (0, reliable) and `tick` (1, unreliable, 0 retransmits):
   both open, and the bytes echo. The **positive control** is the server's
   own count of bytes received, which the test reads. The client's
   callbacks alone could be satisfied by a test that never connected.
2. An in-band channel from the client is heard by the server, and an
   in-band channel from the server reaches the client's `on_data_channel`.
3. `Gathering::Trickle` and `Gathering::Complete` both connect.
   End-of-candidates is seen exactly once.
4. `send` of `max_message_size() + 1` bytes gives `Err(TooLarge)`. Filling
   the queue gives `Err(QueueFull)`, then `on_buffered_amount_low` fires
   once the server drains, and the channel is still open.
5. The server closes the channel, and the client sees `on_close`. The
   server process stops, and the client sees `Disconnected` and then
   `Failed`. This uses a test-only hook to shorten webrtc-rs's ICE timers
   through its `SettingEngine`, so the case takes seconds and not half a
   minute.
6. Unmounting the owning scope closes the peer, and the server sees the
   association end. A peer created under `unowned` survives the same
   unmount.
7. Many peers: four clients against four lite peers on one worker thread,
   all at once.

A TURN case also belongs here, with `ice_transport_policy: Relay` against a
TURN server. In CI that is either coturn installed in the job, or a Rust
TURN server from `turn-server-proto`/`rtc-turn` run in-process. The
implementation PR picks whichever builds in the shard without a new system
dependency.

**Web test (Chrome, `test-wasm`), feasible as follows.** A wasm test cannot
open a UDP socket, so the str0m peer runs beside the browser.
`crates/rinch-rtc/examples/lite_echo.rs` (dev-only) serves `POST /offer` →
answer (with CORS headers) over a minimal HTTP listener and runs the
ICE-lite echo peer, the shape of str0m's own `chat` example. The CI job
starts it in the background before
`cargo test -p rinch-rtc --target wasm32-unknown-unknown`, and passes its
URL at build time as `RINCH_RTC_TEST_URL`, the convention of rinch-ws's
`RINCH_WS_TEST_URL`. The wasm test connects and repeats cases 1, 2 and 4.
It mounts nothing: rinch-rtc dispatches its callbacks itself and needs no
`rinch_web` mount, and its positive control is bytes that came back
through the server.
Two traps:

- **A skip is not a pass.** rinch-ws's live web test logs and passes when
  its URL is unset. Here, CI sets `RINCH_RTC_REQUIRE_LIVE=1` and the test
  **fails** without a URL. Every silent zero needs a positive control.
- **Loopback.** Chrome by default offers no loopback host candidates, so the echo
  peer advertises the runner's primary non-loopback IPv4, which it finds by
  "connecting" a UDP socket to a public address and reading its local
  address. That needs no Chrome flag. If a runner has no such interface,
  the fallback is Chrome's `--allow-loopback-in-peer-connection` in the
  `webdriver.json` that `ci.yml` already writes for another step. Firefox's
  equivalent is a pref (`media.peerconnection.ice.loopback`, to be
  confirmed in the PR). The record
  proposes Chrome first and Firefox in the same PR if the pref can be set
  through the runner, as D10.

That Chrome itself connects to a str0m ICE-lite peer is what this test
establishes. The spike did not run a browser.

## 5. Back-pressure and buffering

- **Per-channel limit, enforced by rinch on both backends.**
  `ChannelOptions::send_buffer_limit` defaults to 1 MiB.
  `send(bytes)` checks the state first (`NotOpen`), then
  `len > max_message_size()` (`TooLarge`), then
  `buffered_amount() + len > limit` (`QueueFull`). Any refusal queues
  nothing and leaves the channel open. On an unreliable tick channel the
  right response is usually to drop the tick and send the next fresh one.
  On a reliable channel, wait for `on_buffered_amount_low`.
- **Why 1 MiB and not the platforms' numbers.** Chromium has historically
  capped its send queue at 16 MiB (webrtc-rs's docs cite
  `MaxSendQueueSize`). webrtc-rs is unbounded by default, and so are Safari
  and Firefox by that same doc's account. With a cap set by rinch, the same
  app does the same thing everywhere, and 1 MiB is about one SCTP receive
  window. A game's tick channel never gets near it, and a bulk sender pacing
  on the low event never reaches it. The number is D7.
- **`buffered_amount()` on native** is a mirror kept on the UI thread. It is
  incremented by `send` before the command is posted, and corrected
  whenever the worker reports the SCTP buffered amount (after each write it
  performs, and on every low event). It can therefore overstate the
  amount, never understate it, so the limit check is conservative.
  webrtc-rs's own `with_data_channel_send_buffer_limit` is set to the same
  limit as a backstop. Its `send().await` then waits for capacity on the
  worker rather than growing without bound if the mirror were ever wrong.
- **Low-water:** `set_buffered_amount_low_threshold(n)` maps to the
  browser's attribute and to webrtc-rs's
  `set_buffered_amount_low_threshold`. `on_buffered_amount_low` fires on
  the falling edge.
- **Receive side:** messages are delivered to `on_message` as they arrive.
  There is no receive-side throttle in v1. A UI thread that falls behind
  builds up callbacks in the main-thread queue, the same exposure rinch-ws
  has. the `rtc` core's `poll_read` back-pressure (its README) is a later option
  if this shows up.

## 6. Errors and reconnection

- **Peer states** are the W3C ones. `Disconnected` is transient: consent
  checks are failing (a Wi-Fi handoff, a blip) and the stack keeps trying,
  so it may return to `Connected` by itself. `Failed` means ICE gave up, and
  only an ICE restart or a new peer recovers. `Closed` is final. Each
  backend keeps its own stack's timers for these transitions. rinch does
  not normalise them. The native test shortens them only through a
  test-only hook.
- **What is surfaced:**
  - Signalling-call failures (a malformed SDP, a description in the wrong
    state) are the `Err` of the async call.
  - A malformed remote candidate goes to `Peer::on_error`.
  - ICE candidate errors also go to `Peer::on_error`: a TURN 401 after
    retry, or an unreachable STUN server. This is `icecandidateerror` on
    web and `on_ice_candidate_error` in webrtc-rs. They are informational,
    and the state callback says whether the peer still connects. Without
    them, a wrong TURN credential looks like a NAT that cannot be
    traversed.
  - A channel-level failure goes to `Channel::on_error` and is followed by
    `on_close`, as rinch-ws reports a failed socket.
  - Worker-thread loss (a panic in the stack) closes every native peer with
    `on_error(RtcError::Internal)` then `Closed`, so the app is not left
    with a silent hang. rinch-ws's notes on its rustls panic say why this
    matters.
- **Reconnection is the app's.** Signalling is the app's, so rinch cannot
  reconnect on its own. `restart_ice()` (D9) marks the next `create_offer`
  as an ICE restart (`createOffer({iceRestart: true})` on web,
  `PeerConnection::restart_ice` on native). The app sends that offer over
  its signalling channel, and existing channels survive if the restart
  succeeds. The guide's recipe is: on `Disconnected`, wait; on `Failed`,
  have the offerer `restart_ice()` and re-offer; if that fails too, build a
  new `Peer`. Against an ICE-lite server the client is always the
  controlling agent and offers the restart. Whether Playweft's server
  accepts restarts or prefers a fresh peer is its call. str0m supports
  restarts (`sdp_api().ice_restart`).

## 7. The spike

Branch `spike-1896`, commit `d9f71d03`, directory `spikes/1896` (its own
cargo workspace; not for merge). Built release, stripped, with
`CARGO_INCREMENTAL=0`, on the shared host.

| Crate | Result | Stripped size | Normal deps (new to rinch's lock) |
|---|---|---|---|
| `dc-webrtc`: two webrtc-rs 0.21.1 peers on one worker thread (current-thread tokio), trickle through a "main thread" over std channels, an in-band and a negotiated unordered channel | OK, channels open at 0.41 s / 0.61 s, end-of-candidates seen on both | 6,304,040 B | 170 (51) |
| `dc-str0m`: two str0m 0.24.1 peers (`rust-crypto`), same exchange, sync | OK, 64 ms | 6,054,504 B | 103 (49) |
| `dc-interop`: webrtc-rs client vs str0m ICE-lite peer, one port, host candidate only, client candidates not forwarded | OK, 0.65–0.70 s; 60 kB round trip; 200 kB refused (64 KiB default) | 12,177,864 B (both stacks) | 226 |

The sizes are whole binaries, std and the spike's own code included. The
difference between the first two rows is the comparison that matters. The
str0m row has no TURN client, no STUN client, and no NIC enumeration. The
webrtc-rs row has all three.

## 8. Implementation sequence

1. `rinch-rtc` crate: the types, the main-thread registry (lifted from
   rinch-ws's `Slot`/`invoke`/`sweep_dead` together with their tests), the
   web backend, and the browser tests that need no server (construction,
   negotiated channels between two in-page peers, back-pressure refusals).
2. The native backend on webrtc-rs, and the native interop tests (section 4,
   cases 1–7), plus the TURN case.
3. The `lite_echo` example and the `test-wasm` CI step: the live
   Chrome test against str0m ICE-lite, failing without its server in CI.
4. User guide: `docs/src/guide/rtc.md` (signalling recipes for trickle and
   complete, the host-many-peers pattern, lifetime and `unowned`,
   back-pressure, reconnection), and an internals page `docs/internals/rtc.md`
   with its row in `CLAUDE.md`'s index (the one CLAUDE.md edit allowed).

## 9. Implementation notes and corrections

The implementation is #1909 (native), #1915 (web) and the docs PR. What it
taught, and what this record got wrong:

- **str0m's answer does advertise `a=max-message-size:262144`.** Sections 4
  and 7 say it carries none. The spike's 200 kB refusal was webrtc-rs's own
  ceiling: it sends at most what it advertises itself, 64 KiB by default.
  The native backend now advertises 256 KiB. The registry applies the
  remote SDP's value the same way on both backends (absent: 64 KiB, `0`: no
  limit), because Chrome reports 256 KiB to a peer whose SDP says nothing.
- **str0m's SCTP send buffer defaults to 128 KiB**, and str0m never accepts a
  single write larger than its free buffer. So by default a str0m server
  cannot send any message over 128 KiB.
- **str0m 0.24.1 does not deliver a close on a stream that has carried no
  data yet.**
- **Back-pressure** (section 5): `on_buffered_amount_low` follows the
  stack's falling edge ungated. `with_data_channel_send_buffer_limit` is set
  to a 16 MiB backstop rather than the per-channel limit, because it is a
  per-connection setting. Per-channel limits clamp to 16 MiB.
- **Lifetime** (section 1.5) gained process-exit handling: an `atexit` close,
  and `rinch_rtc::shutdown()`. Windows' `std::process::exit` may skip the
  former.
- **Events reach the UI through `dispatch_main_callback`**, so a peer made
  before `App::run` queues its events. That also defines headless use: drain
  the queue yourself.
- **Tests:** the str0m dev-dependency uses `openssl`, not `rust-crypto`,
  which would have put aws-lc into the rcgen that webrtc-rs uses.

Mechanism detail is on `docs/internals/rtc.md`.

## Decisions

**Decided by Joe, 2026-10-25: D1–D11 as recommended (option (a) in each).**

**D1. What becomes of `rinch-webrtc`.**
(a) New `rinch-rtc` crate; `rinch-webrtc` and `rinch-signaling` stay as they
are, and a follow-up issue decides whether to retire them or rebuild the
media call on `rinch-rtc`. (b) The same, and remove `rinch-webrtc`,
`rinch-signaling`, `examples/video-call` and the ui-zoo WebRTC section now.
(c) Add data channels to `rinch-webrtc` instead.
**Recommend (a).** The issue asks for `rinch-rtc`, and the old crate's
problems (section 0) go beyond missing data channels. Removing a media demo
is a separate call from adding data channels. Until D1's follow-up, a
workspace with both builds str0m 0.7 (only under `rinch-webrtc/native`) as
well as webrtc-rs.

**D2. The native stack.**
(a) webrtc-rs 0.21 (`webrtc` over `rtc`), `ring` crypto.
(b) str0m 0.24 with rinch's own gathering, STUN and TURN (section 3.2),
using `turn-client-proto`.
(c) webrtc-rs's sans-IO `rtc` driven by rinch's own sync loop. This drops
tokio, but it means rewriting the gathering and TURN relayer that live in
the async `webrtc` crate, so it is (b)'s work on a different core.
**Recommend (a).** It is the only one with TURN and gathering, it is
W3C-shaped like the web backend, and the spike showed it interoperating
with Playweft's str0m ICE-lite server. Choose (b) if a smaller dependency
on a sans-IO stack shared with the server outweighs owning roughly
2,000 lines of ICE gathering and TURN, plus TURN-over-TCP/TLS later.

**D3. The native runtime (if D2 is (a)).**
(a) A current-thread tokio runtime on one `rinch-rtc` worker thread for all
peers. (b) smol, which is not otherwise in the tree. (c) A custom
`webrtc::runtime::Runtime` on std threads.
**Recommend (a).** tokio is already in `gpu`, `notifications` and
`global-hotkeys` builds, and the spike ran on exactly this setup.

**D4. The signalling calls' shape.**
(a) `async fn create_offer/create_answer/set_remote_description`, polled
with `spawn_local`, with `add_ice_candidate` fire-and-forget (buffered
until the remote description is set; errors go to `on_error`).
(b) Callbacks throughout, as sketched in the issue: `create_offer()` returns
nothing and the SDP arrives in `on_local_description`, with errors in
`on_error`.
**Recommend (a).** Each call's result and error belong to that call, the
order is explicit in the code, and `spawn_local` exists on every host. (b)
keeps everything in callback style, like rinch-ws, but turns "set the
remote description, then answer" into a state machine the app writes by
hand.

**D5. The peer's lifetime.**
(a) A peer created during a render closes when its component unmounts.
`unowned(..)` or creation outside a render opts out. Callbacks are pruned
per slot, as in rinch-ws.
(b) rinch-ws parity: only the callbacks are pruned, and the peer closes when
its last handle drops or on `close()`.
**Recommend (a)**, the standing cancel-on-unmount rule. Under (b), a peer
left in a store after its component has gone keeps its sockets, its TURN
allocation and the remote side's connection, with nothing left to hear
them.

**D6. Reliability options in v1.**
(a) `Reliable` and `Unreliable { max_retransmits }` only, with the enum
`#[non_exhaustive]`.
(b) Also `max_packet_life_time` (the issue lists it), and unordered-reliable
and ordered-unreliable.
**Recommend (a).** It is Playweft's layout and the brief's, and adding
variants later does not break anything. (b) costs little if Joe wants the
issue's full list now: both stacks and the browser support every
combination.

**D7. The send-queue limit and overflow.**
(a) A per-channel limit, 1 MiB by default and set in `ChannelOptions`,
enforced by rinch on both backends. Overflow is `Err(QueueFull)`, nothing is
queued, and the channel stays open.
(b) The same with a 16 MiB default (Chromium's number).
(c) No rinch limit: each platform's own behaviour (unbounded on native,
browser-specific on web).
**Recommend (a).** It gives the same behaviour everywhere, and an error
rather than memory growth or a browser closing the channel. 1 MiB covers
any game traffic, and bulk senders pace on `on_buffered_amount_low`.

**D8. Text messages.**
(a) Bytes only: incoming text arrives as UTF-8 bytes, and there is no
`send_text`.
(b) A `RtcMessage::{Text, Binary}` enum like `WsMessage`.
**Recommend (a)**, per the brief, since every user so far sends binary.
(b) is cheap to add later as a separate callback, without breaking (a).

**D9. ICE restart in v1.**
(a) `Peer::restart_ice()` (the next offer restarts), plus the guide's
Disconnected/Failed recipe.
(b) None: recover by building a new `Peer`.
**Recommend (a).** Both stacks and the browser support it, the API surface
is one method, and a restart keeps the channels and their state.

**D10. Interop tests in CI.**
(a) The native str0m-lite test (str0m as a dev-dependency) in the `test`
shard, and the live Chrome test in `test-wasm` against the `lite_echo`
companion. The Chrome test fails rather than skips when CI sets
`RINCH_RTC_REQUIRE_LIVE`. Firefox is added in the same PR if its loopback
pref can be set from the runner, and otherwise in a follow-up.
(b) Native only, with the browser covered by a manual check.
**Recommend (a).** Interop with Playweft's server shape is the requirement,
and the browser is half the clients. The cost is one background process in
`test-wasm`.

**D11. TURN over TCP and TLS on native.**
(a) Ship v1 with UDP TURN only (webrtc-rs's current limit), document it, and
file a follow-up issue (an upstream contribution to `turn_relayer.rs`, or a
rinch-side relayer).
(b) Require TCP/TLS TURN before v1 ships.
**Recommend (a).** UDP TURN covers symmetric NATs, which is the common
reason a direct path fails. A network that blocks all UDP is rarer, and the
browser client still covers that player. If Playweft's audience is known to
be behind such networks, choose (b), and (b) moves D2 back towards (b).
