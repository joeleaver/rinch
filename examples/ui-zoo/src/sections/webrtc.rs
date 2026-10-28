//! WebRTC section: a call between two rinch-rtc peers in this window.
//!
//! No server and no second instance: the two peers hand each other their
//! offer, answer and candidates directly. Peer A sends a generated test
//! pattern and tone; "Use camera" switches it to the camera and microphone
//! with `replace_track`, and nothing opens a device until then. The logic
//! is in `webrtc_call.rs`; `examples/video-call` is the two-machine version.

use rinch::prelude::*;
use rinch_rtc::PeerState;
use rinch_rtc::media::MediaView;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use super::webrtc_call::{InPageCall, Source};

fn state_color(s: PeerState) -> &'static str {
    match s {
        PeerState::Connected => "var(--rinch-color-green-6)",
        PeerState::Connecting => "var(--rinch-color-yellow-6)",
        PeerState::Failed => "var(--rinch-color-red-6)",
        PeerState::Disconnected => "var(--rinch-color-orange-6)",
        _ => "var(--rinch-color-gray-5)",
    }
}

const VIEW_STYLE: &str = "width: 100%; aspect-ratio: 4 / 3; border-radius: 8px;";

pub fn webrtc_section(__scope: &mut RenderScope) -> NodeHandle {
    let call = match InPageCall::start() {
        Ok(call) => call,
        Err(e) => {
            return rsx! {
                Fragment {
                    Title { order: 2, "WebRTC" }
                    Alert { color: "red", title: "The call could not start", {e} }
                }
            };
        }
    };
    // The call was made while this section renders, so its peers, timers
    // and callbacks are the section's. Hold it, and close it (releasing a
    // camera in use) when the section goes away.
    {
        let call = call.clone();
        __scope.on_cleanup(move || call.close());
    }
    let view = call.view();

    let toggle_source = {
        let call = call.clone();
        Callback::new(move || {
            if view.source.get() == Source::Camera {
                call.use_pattern()
            } else {
                call.use_camera()
            }
        })
    };
    let toggle_mute = {
        let call = call.clone();
        Callback::new(move || call.set_muted(!view.muted.get()))
    };
    let toggle_video = {
        let call = call.clone();
        Callback::new(move || call.set_video_off(!view.video_off.get()))
    };
    let toggle_hear = {
        let call = call.clone();
        Callback::new(move || call.set_hear(!view.hear.get()))
    };

    rsx! {
        Fragment {
            Title { order: 2, "WebRTC" }
            Space { h: "md" }
            Text { color: "dimmed",
                "Two rinch-rtc peers in this window, connected over real WebRTC on this machine. "
                "Peer A sends a generated test pattern and a 440 Hz tone; peer B sends a pattern back. "
                "Nothing opens your camera or microphone until you press Use camera."
            }
            Space { h: "lg" }

            Group { gap: "md",
                {state_badge(__scope, "Peer A", view.state_a)}
                {state_badge(__scope, "Peer B", view.state_b)}
            }
            Space { h: "md" }

            SimpleGrid { cols: Some(3), spacing: "md",
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "xs",
                        Text { weight: "600", "A sends" }
                        for shown in view.local.get() {
                            MediaView { track: shown.0.clone(), style: VIEW_STYLE }
                        }
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "xs",
                        Text { weight: "600", "B receives from A" }
                        for shown in view.at_b.get() {
                            MediaView { track: shown.0.clone(), style: VIEW_STYLE }
                        }
                    }
                }
                Paper { p: "md", radius: "md", with_border: true,
                    Stack { gap: "xs",
                        Text { weight: "600", "A receives from B" }
                        for shown in view.at_a.get() {
                            MediaView { track: shown.0.clone(), style: VIEW_STYLE }
                        }
                    }
                }
            }
            Space { h: "md" }

            Group { gap: "sm",
                Button {
                    variant: "filled",
                    disabled: {move || view.source.get() == Source::Requesting},
                    onclick: toggle_source.clone(),
                    {render_tabler_icon(__scope, TablerIcon::Video, TablerIconStyle::Outline)}
                    {move || match view.source.get() {
                        Source::Pattern => " Use camera",
                        Source::Requesting => " Asking for the camera…",
                        Source::Camera => " Send the test pattern instead",
                    }}
                }
                Button {
                    variant: {move || if view.muted.get() { "filled" } else { "light" }},
                    color: {move || if view.muted.get() { "red" } else { "blue" }},
                    onclick: toggle_mute.clone(),
                    {move || if view.muted.get() { "Unmute" } else { "Mute" }}
                }
                Button {
                    variant: {move || if view.video_off.get() { "filled" } else { "light" }},
                    color: {move || if view.video_off.get() { "red" } else { "blue" }},
                    onclick: toggle_video.clone(),
                    {move || if view.video_off.get() { "Video on" } else { "Video off" }}
                }
                Button {
                    variant: "light",
                    onclick: toggle_hear.clone(),
                    {move || if view.hear.get() { "Stop playing B's audio" } else { "Play B's audio" }}
                }
            }
            if !view.error.get().is_empty() {
                Space { h: "sm" }
                Alert { color: "red", title: "Error", {move || view.error.get()} }
            }

            Space { h: "xl" }
            Paper { p: "lg", radius: "md", shadow: "sm",
                Stack { gap: "sm",
                    Title { order: 4, "The API" }
                    Code { block: true,
                        "let (pattern, writer) = LocalTrack::video_source(VideoSourceOptions::default())?;\n"
                        "let sender = peer_a.add_track(&pattern)?;\n"
                        "peer_b.on_track(move |remote| shown.set(Some(remote)));\n"
                        "// ...offer, answer and candidates, over your own signalling...\n\n"
                        "// Later, from a click: the camera, without renegotiating.\n"
                        "let camera = LocalTrack::camera(CameraOptions::default()).await?;\n"
                        "sender.replace_track(Some(&camera)).await?;\n\n"
                        "rsx! { MediaView { track: remote.clone(), style: \"width: 320px;\" } }"
                    }
                    Text { size: "sm", color: "dimmed",
                        "Desktop: webrtc-rs with VP8, H.264 and Opus. Web: the browser's RTCPeerConnection. "
                        "See the WebRTC guide, and examples/video-call for a call between two machines."
                    }
                }
            }
        }
    }
}

fn state_badge(
    __scope: &mut RenderScope,
    label: &'static str,
    state: Signal<PeerState>,
) -> NodeHandle {
    rsx! {
        Group { gap: "xs",
            div {
                style: {move || format!(
                    "width: 12px; height: 12px; border-radius: 50%; background: {}",
                    state_color(state.get())
                )}
            }
            Text { weight: "600", {move || format!("{label}: {:?}", state.get())} }
        }
    }
}
