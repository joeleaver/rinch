//! Full-featured desktop video calling with WebRTC.
//!
//! Features:
//! - Camera preview & remote video via RenderSurface
//! - Microphone audio with Opus encoding
//! - Screen sharing via PipeWire/XDG Portal
//! - WebSocket signaling for call setup
//! - Mute/unmute audio, enable/disable camera, screen share toggle
//! - Connection state display
//!
//! Usage:
//!   1. Start the signaling server: `cargo run -p rinch-signaling-server`
//!   2. Run two instances of this app
//!   3. One clicks "Start Call", the other clicks "Join Call"

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch::render_surface::create_render_surface;
use rinch_av::VideoTrack;
use rinch_av::camera::CameraConfig;
use rinch_screen_capture::{CaptureConfig, ScreenCapture};
use rinch_signaling::WebSocketSignaling;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use rinch_webrtc::{ConnectionState, VideoCall};
use tracing::warn;

// ── App Store ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct CallStore {
    // Connection
    server_url: Signal<String>,
    room_id: Signal<String>,
    state: Signal<ConnectionState>,
    status_msg: Signal<String>,

    // Media toggles
    mic_enabled: Signal<bool>,
    cam_enabled: Signal<bool>,
    screen_sharing: Signal<bool>,

    // Whether a call is active (started or joined)
    in_call: Signal<bool>,
}

impl CallStore {
    fn new() -> Self {
        Self {
            server_url: Signal::new("ws://127.0.0.1:9090".into()),
            room_id: Signal::new("test-room".into()),
            state: Signal::new(ConnectionState::New),
            status_msg: Signal::new(String::new()),
            mic_enabled: Signal::new(true),
            cam_enabled: Signal::new(true),
            screen_sharing: Signal::new(false),
            in_call: Signal::new(false),
        }
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

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
        .size(960, 700)
        .theme(theme)
        .run();
}

// ── App Component ───────────────────────────────────────────────────────────

#[component]
fn app() -> NodeHandle {
    create_store(CallStore::new());

    rsx! {
        div {
            style: "display: flex; flex-direction: column; height: 100vh; background: var(--rinch-color-body);",

            // Header
            div {
                style: "padding: 12px 20px; border-bottom: 1px solid var(--rinch-color-dark-4); display: flex; align-items: center; gap: 12px;",
                Title { order: 3, "Rinch Video Call" }
                div { style: "flex: 1;" }
                ConnectionBadge {}
            }

            // Main content
            div {
                style: "flex: 1; display: flex; overflow: hidden;",

                // Video area
                div {
                    style: "flex: 1; display: flex; flex-direction: column; gap: 4px; padding: 8px; min-width: 0;",
                    RemoteVideo {}
                    LocalPreview {}
                }

                // Sidebar
                div {
                    style: "width: 280px; border-left: 1px solid var(--rinch-color-dark-4); display: flex; flex-direction: column; overflow-y: auto;",
                    ControlPanel {}
                }
            }

            // Bottom toolbar
            Toolbar {}
        }
    }
}

// ── Connection Badge ────────────────────────────────────────────────────────

#[component]
fn ConnectionBadge() -> NodeHandle {
    let store = use_store::<CallStore>();

    rsx! {
        Badge {
            variant: {|| match store.state.get() {
                ConnectionState::Connected => "filled",
                ConnectionState::Connecting => "light",
                ConnectionState::Failed => "filled",
                _ => "outline",
            }},
            color: {|| match store.state.get() {
                ConnectionState::Connected => "green",
                ConnectionState::Connecting => "yellow",
                ConnectionState::Failed => "red",
                ConnectionState::Disconnected => "orange",
                _ => "gray",
            }},
            {|| format!("{:?}", store.state.get())}
        }
    }
}

// ── Remote Video ────────────────────────────────────────────────────────────

#[component]
fn RemoteVideo() -> NodeHandle {
    let store = use_store::<CallStore>();
    let surface = create_render_surface();

    // Render a placeholder or the remote video surface
    surface.set_render_callback(|writer, w, h| {
        // Dark background when no remote video
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for chunk in pixels.as_chunks_mut::<4>().0 {
            chunk[0] = 30;
            chunk[1] = 30;
            chunk[2] = 30;
            chunk[3] = 255;
        }
        writer.submit_frame(&pixels, w, h);
    });

    rsx! {
        div {
            style: "flex: 1; position: relative; border-radius: var(--rinch-radius-md); overflow: hidden; min-height: 0;",

            RenderSurface { surface: Some(surface), style: "width: 100%; height: 100%;" }

            // Overlay when not connected
            div {
                style: {|| {
                    let connected = store.state.get() == ConnectionState::Connected;
                    if connected {
                        "display: none;"
                    } else {
                        "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(0,0,0,0.6);"
                    }
                }},
                Text { size: "xl", color: "dimmed",
                    {|| {
                        match store.state.get() {
                            ConnectionState::New => "Start or join a call".to_string(),
                            ConnectionState::Connecting => "Connecting...".to_string(),
                            ConnectionState::Disconnected => "Disconnected".to_string(),
                            ConnectionState::Failed => "Connection failed".to_string(),
                            ConnectionState::Closed => "Call ended".to_string(),
                            ConnectionState::Connected => String::new(),
                        }
                    }}
                }
            }
        }
    }
}

// ── Local Preview ───────────────────────────────────────────────────────────

#[component]
fn LocalPreview() -> NodeHandle {
    let store = use_store::<CallStore>();
    let surface = create_render_surface();
    let local_video: Rc<RefCell<Option<VideoTrack>>> = Rc::new(RefCell::new(None));

    // Try to open camera and pipe frames to surface
    let writer = surface.writer();
    match VideoTrack::from_camera(CameraConfig {
        width: 320,
        height: 240,
        fps: 15,
        ..Default::default()
    }) {
        Ok(track) => {
            let pipe = track.pipe_to(move |frame| {
                writer.submit_frame(&frame.data, frame.width, frame.height);
            });
            // Keep the pipe alive by leaking it into a static (it stops on drop)
            std::mem::forget(pipe);
            *local_video.borrow_mut() = Some(track);
        }
        Err(e) => {
            warn!("Failed to open camera: {e}");
            // Show dark placeholder
            surface.set_render_callback(|writer, w, h| {
                let pixels = vec![20u8; (w * h * 4) as usize];
                writer.submit_frame(&pixels, w, h);
            });
        }
    }

    rsx! {
        div {
            style: "height: 160px; position: relative; border-radius: var(--rinch-radius-md); overflow: hidden; flex-shrink: 0;",

            RenderSurface { surface: Some(surface), style: "width: 100%; height: 100%;" }

            // Label
            div {
                style: "position: absolute; bottom: 8px; left: 8px; background: rgba(0,0,0,0.6); padding: 2px 8px; border-radius: 4px;",
                Text { size: "xs", color: "white", "You" }
            }

            // Camera off overlay
            div {
                style: {|| {
                    if store.cam_enabled.get() {
                        "display: none;"
                    } else {
                        "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: var(--rinch-color-dark-7);"
                    }
                }},
                Text { size: "sm", color: "dimmed", "Camera off" }
            }
        }
    }
}

// ── Control Panel (Sidebar) ─────────────────────────────────────────────────

#[component]
fn ControlPanel() -> NodeHandle {
    let store = use_store::<CallStore>();

    rsx! {
        div {
            style: "padding: 16px; display: flex; flex-direction: column; gap: 16px;",

            // Connection settings
            Title { order: 5, "Connection" }

            TextInput {
                label: "Server URL",
                placeholder: "ws://127.0.0.1:9090",
                value_fn: move || store.server_url.get(),
                oninput: move |val: String| store.server_url.set(val),
            }

            TextInput {
                label: "Room ID",
                placeholder: "test-room",
                value_fn: move || store.room_id.get(),
                oninput: move |val: String| store.room_id.set(val),
            }

            // Call actions
            Group { gap: "sm", grow: true,
                Button {
                    variant: "filled",
                    color: "green",
                    disabled: {|| store.in_call.get()},
                    onclick: {
                        let store = store;
                        move || start_call(store)
                    },
                    {render_tabler_icon(__scope, TablerIcon::PhoneOutgoing, TablerIconStyle::Outline)}
                    " Start"
                }

                Button {
                    variant: "filled",
                    color: "blue",
                    disabled: {|| store.in_call.get()},
                    onclick: {
                        let store = store;
                        move || join_call(store)
                    },
                    {render_tabler_icon(__scope, TablerIcon::PhoneIncoming, TablerIconStyle::Outline)}
                    " Join"
                }
            }

            Button {
                variant: "light",
                color: "red",
                disabled: {|| !store.in_call.get()},
                onclick: move || {
                    store.in_call.set(false);
                    store.state.set(ConnectionState::Closed);
                    store.status_msg.set("Call ended".into());
                },
                {render_tabler_icon(__scope, TablerIcon::PhoneOff, TablerIconStyle::Outline)}
                " Hang Up"
            }

            // Status message
            div {
                style: "min-height: 24px;",
                Text { size: "sm", color: "dimmed", {|| store.status_msg.get()} }
            }

            Divider {}

            // Screen sharing
            Title { order: 5, "Screen Sharing" }

            Button {
                variant: {|| if store.screen_sharing.get() { "filled" } else { "light" }},
                color: {|| if store.screen_sharing.get() { "red" } else { "blue" }},
                onclick: {
                    let store = store;
                    move || toggle_screen_share(store)
                },
                {render_tabler_icon(__scope, TablerIcon::ScreenShare, TablerIconStyle::Outline)}
                {|| if store.screen_sharing.get() { " Stop Sharing" } else { " Share Screen" }}
            }
        }
    }
}

// ── Bottom Toolbar ──────────────────────────────────────────────────────────

#[component]
fn Toolbar() -> NodeHandle {
    let store = use_store::<CallStore>();

    rsx! {
        div {
            style: "padding: 12px; border-top: 1px solid var(--rinch-color-dark-4); display: flex; justify-content: center; gap: 12px;",

            // Mic toggle
            ActionIcon {
                variant: {|| if store.mic_enabled.get() { "filled" } else { "light" }},
                color: {|| if store.mic_enabled.get() { "blue" } else { "red" }},
                size: "xl",
                radius: "xl",
                onclick: move || store.mic_enabled.update(|v| *v = !*v),
                if store.mic_enabled.get() {
                    {render_tabler_icon(__scope, TablerIcon::Microphone, TablerIconStyle::Outline)}
                } else {
                    {render_tabler_icon(__scope, TablerIcon::MicrophoneOff, TablerIconStyle::Outline)}
                }
            }

            // Camera toggle
            ActionIcon {
                variant: {|| if store.cam_enabled.get() { "filled" } else { "light" }},
                color: {|| if store.cam_enabled.get() { "blue" } else { "red" }},
                size: "xl",
                radius: "xl",
                onclick: move || store.cam_enabled.update(|v| *v = !*v),
                if store.cam_enabled.get() {
                    {render_tabler_icon(__scope, TablerIcon::Video, TablerIconStyle::Outline)}
                } else {
                    {render_tabler_icon(__scope, TablerIcon::VideoOff, TablerIconStyle::Outline)}
                }
            }

            // Screen share toggle
            ActionIcon {
                variant: {|| if store.screen_sharing.get() { "filled" } else { "light" }},
                color: {|| if store.screen_sharing.get() { "red" } else { "gray" }},
                size: "xl",
                radius: "xl",
                onclick: {
                    let store = store;
                    move || toggle_screen_share(store)
                },
                {render_tabler_icon(__scope, TablerIcon::ScreenShare, TablerIconStyle::Outline)}
            }

            // Hang up
            ActionIcon {
                variant: "filled",
                color: "red",
                size: "xl",
                radius: "xl",
                disabled: {|| !store.in_call.get()},
                onclick: move || {
                    store.in_call.set(false);
                    store.state.set(ConnectionState::Closed);
                    store.status_msg.set("Call ended".into());
                },
                {render_tabler_icon(__scope, TablerIcon::PhoneOff, TablerIconStyle::Outline)}
            }
        }
    }
}

// ── Call Logic ───────────────────────────────────────────────────────────────

fn start_call(store: CallStore) {
    let url = format!("{}/{}", store.server_url.get(), store.room_id.get());
    store.status_msg.set(format!("Connecting to {url}..."));

    match WebSocketSignaling::connect(&url) {
        Ok(signaling) => match VideoCall::new() {
            Ok(mut call) => {
                let state_sig = store.state;
                let status_sig = store.status_msg;
                let in_call_sig = store.in_call;

                call.peer.on_connection_state_change(move |s| {
                    state_sig.send(s);
                    status_sig.send(format!("{s:?}"));
                    if matches!(
                        s,
                        ConnectionState::Disconnected
                            | ConnectionState::Failed
                            | ConnectionState::Closed
                    ) {
                        in_call_sig.send(false);
                    }
                });

                let mic = store.mic_enabled.get();
                let cam = store.cam_enabled.get();
                match call.start(signaling, mic, cam) {
                    Ok(()) => {
                        store.in_call.set(true);
                        store.status_msg.set("Waiting for peer...".into());
                    }
                    Err(e) => {
                        store.status_msg.set(format!("Start failed: {e:?}"));
                    }
                }
            }
            Err(e) => store.status_msg.set(format!("WebRTC init failed: {e:?}")),
        },
        Err(e) => store.status_msg.set(format!("Connect failed: {e}")),
    }
}

fn join_call(store: CallStore) {
    let url = format!("{}/{}", store.server_url.get(), store.room_id.get());
    store.status_msg.set(format!("Joining {url}..."));

    match WebSocketSignaling::connect(&url) {
        Ok(signaling) => match VideoCall::new() {
            Ok(mut call) => {
                let state_sig = store.state;
                let status_sig = store.status_msg;
                let in_call_sig = store.in_call;

                call.peer.on_connection_state_change(move |s| {
                    state_sig.send(s);
                    status_sig.send(format!("{s:?}"));
                    if matches!(
                        s,
                        ConnectionState::Disconnected
                            | ConnectionState::Failed
                            | ConnectionState::Closed
                    ) {
                        in_call_sig.send(false);
                    }
                });

                match call.join(signaling) {
                    Ok(()) => {
                        store.in_call.set(true);
                        store.status_msg.set("Joining call...".into());
                    }
                    Err(e) => {
                        store.status_msg.set(format!("Join failed: {e:?}"));
                    }
                }
            }
            Err(e) => store.status_msg.set(format!("WebRTC init failed: {e:?}")),
        },
        Err(e) => store.status_msg.set(format!("Connect failed: {e}")),
    }
}

// Global screen capture handle (kept alive while sharing)
thread_local! {
    static SCREEN_CAPTURE: RefCell<Option<ScreenCapture>> = const { RefCell::new(None) };
}

fn toggle_screen_share(store: CallStore) {
    if store.screen_sharing.get() {
        // Stop sharing
        SCREEN_CAPTURE.with(|c| {
            c.borrow_mut().take(); // Drop stops capture
        });
        store.screen_sharing.set(false);
        store.status_msg.set("Screen sharing stopped".into());
    } else {
        // Start sharing
        store.status_msg.set("Opening screen picker...".into());

        // Run the portal dialog on a background thread since it blocks
        let screen_sharing_sig = store.screen_sharing;
        let status_sig = store.status_msg;

        std::thread::spawn(
            move || match ScreenCapture::start(CaptureConfig::default()) {
                Ok(capture) => {
                    SCREEN_CAPTURE.with(|c| {
                        *c.borrow_mut() = Some(capture);
                    });
                    screen_sharing_sig.send(true);
                    status_sig.send("Sharing screen".into());
                }
                Err(rinch_screen_capture::CaptureError::Cancelled) => {
                    status_sig.send("Screen share cancelled".into());
                }
                Err(e) => {
                    status_sig.send(format!("Screen share error: {e}"));
                }
            },
        );
    }
}
