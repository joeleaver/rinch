//! A video call between two machines, on rinch-rtc's audio and video tracks
//! (design record `docs/design/rtc-media.md` 5.1).
//!
//! 1. Start the relay somewhere both machines reach:
//!    `cargo run -p rtc-room-relay` (port 8080).
//! 2. Run this on each machine, enter the relay's URL and the same room, and
//!    press Join. The first to join waits; when the second arrives the relay
//!    tells the first, which makes the offer.
//!
//! Signalling is JSON over a rinch-ws socket ([`Wire`]); the relay forwards it
//! unread. Each side sends a generated test pattern until "Start camera" asks
//! for the camera and the microphone. **Nothing opens a device on start**, so
//! the example can be launched anywhere (CI, an agent, a machine without a
//! camera). The camera then replaces the pattern with `replace_track`, and the
//! "test pattern instead" switch swaps them back and forth, without
//! renegotiating: both senders exist before the first offer.
//!
//! Desktop calls have no echo cancellation (M5): use headphones.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use rinch::prelude::*;
use rinch_rtc::media::{
    CameraOptions, Direction, LocalTrack, MediaKind, MediaTrack, MediaView, MicrophoneOptions,
    Sender, VideoSourceOptions, VideoSourceWriter,
};
use rinch_rtc::{Description, IceCandidate, Peer, PeerState, RtcConfig, SdpKind, SignalingState};
use rinch_ws::{WsHandle, WsMessage};
use serde::{Deserialize, Serialize};

// ── Signalling ──────────────────────────────────────────────────────────────

/// What travels over the relay. `peer-joined` and `bye` come from the relay
/// itself; the rest from the other side.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum Wire {
    PeerJoined,
    Bye,
    Description { description: Description },
    Candidate { candidate: Option<IceCandidate> },
}

// ── State ───────────────────────────────────────────────────────────────────

/// One video to show, keyed by its track's id (a `MediaView`'s track is
/// fixed for life, so a new track needs a new view).
#[derive(Clone)]
struct Shown(MediaTrack);

impl Shown {
    fn id(&self) -> String {
        match &self.0 {
            MediaTrack::None => String::new(),
            MediaTrack::Local(t) => t.id(),
            MediaTrack::Remote(t) => t.id(),
        }
    }
}

impl PartialEq for Shown {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl std::fmt::Debug for Shown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id())
    }
}

#[derive(Clone, Copy)]
struct Ui {
    server_url: Signal<String>,
    room: Signal<String>,
    joined: Signal<bool>,
    status: Signal<String>,
    state: Signal<PeerState>,
    camera_on: Signal<bool>,
    asking: Signal<bool>,
    use_pattern: Signal<bool>,
    muted: Signal<bool>,
    video_off: Signal<bool>,
    remote_muted: Signal<bool>,
    local: Signal<Vec<Shown>>,
    remote: Signal<Vec<Shown>>,
}

/// The call: the socket, the current peer and its two senders, and the
/// tracks they can carry. Lives as long as the app (the root component).
struct Call {
    ui: Ui,
    ws: RefCell<Option<WsHandle>>,
    peer: RefCell<Option<Peer>>,
    video: RefCell<Option<Sender>>,
    audio: RefCell<Option<Sender>>,
    /// Polite in the perfect-negotiation sense: the side that joined second.
    polite: Cell<bool>,
    pattern: LocalTrack,
    camera: RefCell<Option<LocalTrack>>,
    mic: RefCell<Option<LocalTrack>>,
}

impl Call {
    fn send(&self, msg: &Wire) {
        if let (Some(ws), Ok(text)) = (self.ws.borrow().as_ref(), serde_json::to_string(msg)) {
            ws.send_text(text);
        }
    }

    /// What the video sender should carry now.
    fn video_track(&self) -> LocalTrack {
        match self.camera.borrow().as_ref() {
            Some(camera) if !self.ui.use_pattern.get() => camera.clone(),
            _ => self.pattern.clone(),
        }
    }

    /// Put the right tracks on the senders (no renegotiation), and show what
    /// is sent.
    fn update_senders(self: &Rc<Self>) {
        let video = self.video_track();
        video.set_enabled(!self.ui.video_off.get());
        if let Some(mic) = self.mic.borrow().as_ref() {
            mic.set_enabled(!self.ui.muted.get());
        }
        self.ui.local.set(vec![Shown(video.clone().into())]);
        let (vs, aus) = (self.video.borrow().clone(), self.audio.borrow().clone());
        let mic = self.mic.borrow().clone();
        let ui = self.ui;
        spawn_local(async move {
            if let Some(s) = vs
                && let Err(e) = s.replace_track(Some(&video)).await
            {
                ui.status.set(format!("replace_track: {e}"));
            }
            if let Some(s) = aus
                && let Err(e) = s.replace_track(mic.as_ref()).await
            {
                ui.status.set(format!("replace_track: {e}"));
            }
        });
    }

    /// A fresh peer, with both senders in place before any offer, so later
    /// changes are `replace_track`s.
    fn new_peer(self: &Rc<Self>) {
        if let Some(old) = self.peer.borrow_mut().take() {
            old.close();
        }
        self.ui.remote.set(Vec::new());
        let peer = match Peer::new(RtcConfig::default()) {
            Ok(p) => p,
            Err(e) => {
                self.ui.status.set(format!("peer: {e}"));
                return;
            }
        };
        let ui = self.ui;
        peer.on_connection_state(move |s| {
            ui.state.set(s);
            if s == PeerState::Connected {
                ui.status.set("In a call".into());
            }
        });
        peer.on_error(move |e| ui.status.set(format!("{e}")));
        let me = Rc::downgrade(self);
        peer.on_ice_candidate(move |candidate| {
            if let Some(call) = me.upgrade() {
                call.send(&Wire::Candidate { candidate });
            }
        });
        peer.on_track(move |remote| {
            if remote.kind() == MediaKind::Video {
                remote.on_mute(move || ui.remote_muted.set(true));
                remote.on_unmute(move || ui.remote_muted.set(false));
                ui.remote.set(vec![Shown(remote.into())]);
            }
            // Remote audio plays by itself.
        });
        let video = peer.add_track(&self.video_track());
        let audio = peer.add_transceiver(MediaKind::Audio, Direction::SendRecv);
        match (video, audio) {
            (Ok(v), Ok(a)) => {
                *self.video.borrow_mut() = Some(v);
                *self.audio.borrow_mut() = Some(a.sender());
            }
            (Err(e), _) | (_, Err(e)) => self.ui.status.set(format!("tracks: {e}")),
        }
        *self.peer.borrow_mut() = Some(peer);
        self.ui.state.set(PeerState::New);
        self.update_senders();
    }

    fn on_wire(self: &Rc<Self>, msg: Wire) {
        let Some(peer) = self.peer.borrow().clone() else {
            return;
        };
        match msg {
            Wire::PeerJoined => {
                // We were here first: we offer, and are the impolite side.
                self.polite.set(false);
                self.ui.status.set("The other side joined; calling…".into());
                let me = self.clone();
                spawn_local(async move {
                    match peer.create_offer().await {
                        Ok(offer) => me.send(&Wire::Description { description: offer }),
                        Err(e) => me.ui.status.set(format!("offer: {e}")),
                    }
                });
            }
            Wire::Bye => {
                self.ui
                    .status
                    .set("The other side left; waiting for them…".into());
                self.polite.set(true);
                self.new_peer();
            }
            Wire::Description { description } => {
                let me = self.clone();
                spawn_local(async move { me.on_description(peer, description).await });
            }
            Wire::Candidate { candidate } => peer.add_ice_candidate(candidate),
        }
    }

    async fn on_description(self: Rc<Self>, peer: Peer, description: Description) {
        let offer = description.kind == SdpKind::Offer;
        // Perfect negotiation (see the rinch-rtc guide). Only the first to
        // join offers, so a collision should not happen; this is the rule if
        // it does.
        let collision = offer && peer.signaling_state() != SignalingState::Stable;
        if collision && !self.polite.get() {
            return;
        }
        if let Err(e) = peer.set_remote_description(description).await {
            self.ui.status.set(format!("remote description: {e}"));
            return;
        }
        if offer {
            match peer.create_answer().await {
                Ok(answer) => self.send(&Wire::Description {
                    description: answer,
                }),
                Err(e) => self.ui.status.set(format!("answer: {e}")),
            }
        }
    }

    fn join(self: &Rc<Self>) {
        let url = format!(
            "{}/room/{}",
            self.ui.server_url.get().trim_end_matches('/'),
            self.ui.room.get()
        );
        let ws = match rinch_ws::connect(url.clone()) {
            Ok(ws) => ws,
            Err(e) => {
                self.ui.status.set(format!("{url}: {e}"));
                return;
            }
        };
        let ui = self.ui;
        let me: Weak<Call> = Rc::downgrade(self);
        ws.on_open(move || ui.status.set("Waiting for the other side…".into()));
        ws.on_message(move |m: WsMessage| {
            let (Some(call), Some(text)) = (me.upgrade(), m.as_text()) else {
                return;
            };
            match serde_json::from_str::<Wire>(text) {
                Ok(msg) => call.on_wire(msg),
                Err(e) => ui.status.set(format!("bad message: {e}")),
            }
        });
        ws.on_error(move |e| ui.status.set(format!("relay: {e}")));
        ws.on_close(move |_| {
            ui.status.set("The relay closed the connection".into());
            ui.joined.set(false);
        });
        *self.ws.borrow_mut() = Some(ws);
        // Until told otherwise, we are the second to arrive.
        self.polite.set(true);
        self.ui.joined.set(true);
        self.ui.status.set(format!("Connecting to {url}…"));
        self.new_peer();
    }

    fn leave(&self) {
        if let Some(ws) = self.ws.borrow_mut().take() {
            ws.close();
        }
        if let Some(peer) = self.peer.borrow_mut().take() {
            peer.close();
        }
        self.video.borrow_mut().take();
        self.audio.borrow_mut().take();
        self.ui.joined.set(false);
        self.ui.state.set(PeerState::Closed);
        self.ui.remote.set(Vec::new());
        self.ui.status.set("Left the call".into());
    }

    /// Ask for the camera and the microphone (only from the button).
    fn start_camera(self: &Rc<Self>) {
        if self.ui.asking.get() || self.ui.camera_on.get() {
            return;
        }
        self.ui.asking.set(true);
        let me = self.clone();
        spawn_local(async move {
            let ui = me.ui;
            match LocalTrack::camera(CameraOptions::default()).await {
                Ok(camera) => *me.camera.borrow_mut() = Some(camera),
                Err(e) => ui.status.set(format!("camera: {e}")),
            }
            match LocalTrack::microphone(MicrophoneOptions::default()).await {
                Ok(mic) => *me.mic.borrow_mut() = Some(mic),
                Err(e) => ui.status.set(format!("microphone: {e}")),
            }
            ui.asking.set(false);
            let any = me.camera.borrow().is_some() || me.mic.borrow().is_some();
            ui.camera_on.set(any);
            if any {
                ui.use_pattern.set(false);
            }
            me.update_senders();
        });
    }

    /// Release the camera and microphone; the pattern goes back on.
    fn stop_camera(self: &Rc<Self>) {
        for t in [
            self.camera.borrow_mut().take(),
            self.mic.borrow_mut().take(),
        ]
        .into_iter()
        .flatten()
        {
            t.stop();
        }
        self.ui.camera_on.set(false);
        self.update_senders();
    }
}

// ── The test pattern ────────────────────────────────────────────────────────

const PW: u32 = 640;
const PH: u32 = 360;

/// A white box moving over a dark blue field: something that obviously moves.
fn feed_pattern(writer: VideoSourceWriter) {
    let mut k = 0u32;
    set_interval(66, move || {
        k = k.wrapping_add(1);
        let side = 96;
        let (x, y) = ((k * 6) % (PW - side), (k * 4) % (PH - side));
        let mut f = Vec::with_capacity((PW * PH * 4) as usize);
        for _ in 0..PW * PH {
            f.extend_from_slice(&[20, 30, 70, 255]);
        }
        for row in y..y + side {
            let start = ((row * PW + x) * 4) as usize;
            for px in f[start..start + (side * 4) as usize].as_chunks_mut::<4>().0 {
                px.copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        writer.push_rgba(&f, PW, PH);
    });
}

// ── UI ──────────────────────────────────────────────────────────────────────

#[component]
fn app() -> NodeHandle {
    let ui = Ui {
        server_url: Signal::new("ws://127.0.0.1:8080".into()),
        room: Signal::new("test-room".into()),
        joined: Signal::new(false),
        status: Signal::new("Not in a call".into()),
        state: Signal::new(PeerState::New),
        camera_on: Signal::new(false),
        asking: Signal::new(false),
        use_pattern: Signal::new(true),
        muted: Signal::new(false),
        video_off: Signal::new(false),
        remote_muted: Signal::new(false),
        local: Signal::new(Vec::new()),
        remote: Signal::new(Vec::new()),
    };
    let (pattern, writer) = LocalTrack::video_source(VideoSourceOptions {
        width: PW,
        height: PH,
        fps: 15,
    })
    .expect("a generated video track");
    feed_pattern(writer);
    ui.local.set(vec![Shown(pattern.clone().into())]);
    let call = Rc::new(Call {
        ui,
        ws: RefCell::new(None),
        peer: RefCell::new(None),
        video: RefCell::new(None),
        audio: RefCell::new(None),
        polite: Cell::new(true),
        pattern,
        camera: RefCell::new(None),
        mic: RefCell::new(None),
    });

    let cb = |f: fn(&Rc<Call>)| {
        let call = call.clone();
        Callback::new(move || f(&call))
    };
    let join = cb(|c| {
        if c.ui.joined.get() {
            c.leave()
        } else {
            c.join()
        }
    });
    let camera = cb(|c| {
        if c.ui.camera_on.get() {
            c.stop_camera()
        } else {
            c.start_camera()
        }
    });
    let pattern_switch = cb(|c| {
        c.ui.use_pattern.update(|p| *p = !*p);
        c.update_senders();
    });
    let mute = cb(|c| {
        c.ui.muted.update(|m| *m = !*m);
        c.update_senders();
    });
    let video_off = cb(|c| {
        c.ui.video_off.update(|v| *v = !*v);
        c.update_senders();
    });

    rsx! {
        div { style: "display: flex; height: 100vh; background: var(--rinch-color-body);",
            // The videos
            div { style: "flex: 1; position: relative; background: #000; min-width: 0;",
                for shown in ui.remote.get() {
                    MediaView { track: shown.0.clone(), style: "width: 100%; height: 100%;" }
                }
                if ui.remote.get().is_empty() {
                    div { style: "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;",
                        Text { size: "xl", color: "dimmed", "No one here yet" }
                    }
                }
                if ui.remote_muted.get() {
                    div { style: "position: absolute; top: 12px; left: 12px; background: rgba(0,0,0,0.6); padding: 4px 10px; border-radius: 6px;",
                        Text { size: "xs", style: "color: white;", "Their video is paused" }
                    }
                }
                div { style: "position: absolute; right: 16px; bottom: 16px; width: 240px; height: 135px; border-radius: 8px; overflow: hidden; border: 2px solid rgba(255,255,255,0.3);",
                    for shown in ui.local.get() {
                        MediaView { track: shown.0.clone(), mirrored: true, style: "width: 100%; height: 100%;" }
                    }
                }
            }

            // The controls
            div { style: "width: 300px; padding: 16px; display: flex; flex-direction: column; gap: 12px; border-left: 1px solid var(--rinch-color-border);",
                Title { order: 3, "Rinch Video Call" }
                Group { gap: "xs",
                    div {
                        style: {move || format!(
                            "width: 10px; height: 10px; border-radius: 50%; background: {};",
                            match ui.state.get() {
                                PeerState::Connected => "var(--rinch-color-green-6)",
                                PeerState::Connecting => "var(--rinch-color-yellow-6)",
                                PeerState::Failed => "var(--rinch-color-red-6)",
                                PeerState::Disconnected => "var(--rinch-color-orange-6)",
                                _ => "var(--rinch-color-gray-5)",
                            }
                        )}
                    }
                    Text { size: "sm", {move || format!("{:?}", ui.state.get())} }
                }
                Text { size: "sm", color: "dimmed", {move || ui.status.get()} }

                TextInput {
                    label: "Relay",
                    value_fn: move || ui.server_url.get(),
                    oninput: move |v: String| ui.server_url.set(v),
                }
                TextInput {
                    label: "Room",
                    value_fn: move || ui.room.get(),
                    oninput: move |v: String| ui.room.set(v),
                }
                Button {
                    variant: "filled",
                    color: {move || if ui.joined.get() { "red" } else { "green" }},
                    onclick: join.clone(),
                    {move || if ui.joined.get() { "Leave" } else { "Join" }}
                }

                Divider {}
                Button {
                    variant: {move || if ui.camera_on.get() { "light" } else { "filled" }},
                    disabled: {move || ui.asking.get()},
                    onclick: camera.clone(),
                    {move || if ui.asking.get() {
                        "Asking for the camera…"
                    } else if ui.camera_on.get() {
                        "Stop camera"
                    } else {
                        "Start camera"
                    }}
                }
                Button {
                    variant: "light",
                    disabled: {move || !ui.camera_on.get()},
                    onclick: pattern_switch.clone(),
                    {move || if ui.use_pattern.get() { "Send the camera" } else { "Send a test pattern instead" }}
                }
                Group { gap: "sm",
                    Button {
                        variant: {move || if ui.muted.get() { "filled" } else { "light" }},
                        color: {move || if ui.muted.get() { "red" } else { "blue" }},
                        onclick: mute.clone(),
                        {move || if ui.muted.get() { "Unmute" } else { "Mute" }}
                    }
                    Button {
                        variant: {move || if ui.video_off.get() { "filled" } else { "light" }},
                        color: {move || if ui.video_off.get() { "red" } else { "blue" }},
                        onclick: video_off.clone(),
                        {move || if ui.video_off.get() { "Video on" } else { "Video off" }}
                    }
                }
                Text { size: "xs", color: "dimmed",
                    "Desktop calls have no echo cancellation: use headphones."
                }
            }
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "video_call=debug,rinch=warn".parse().unwrap()),
        )
        .init();

    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        dark_mode: true,
        ..Default::default()
    };
    App::new(app)
        .title("Rinch Video Call")
        .size(1100, 680)
        .theme(theme)
        .run();
}
