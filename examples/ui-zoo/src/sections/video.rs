//! Video Player section — showcases rinch-video components.

use rinch::prelude::*;
use rinch_video::{
    PlaybackState, VideoControls, VideoViewport, use_video_player, use_video_player_paused,
};

#[component]
pub fn video_section() -> NodeHandle {
    let player = use_video_player("https://media.w3.org/2010/05/sintel/trailer.mp4");

    rsx! {
        Fragment {
            Stack { gap: "xs",
                Title { order: 1, "Video Player" }
                Text { size: "lg", color: "dimmed",
                    "Video playback powered by libmpv. VideoViewport renders decoded frames as a compositor layer beneath the UI on the GPU backend, and inline during paint — at the viewport's own z-order, so overlays cover it — on the software one. VideoControls provides play/pause, seeking, volume, and timestamp."
                }
            }
            Space { h: "xl" }

            // ── Video player ──────────────────────────────────────────
            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden;",

                div { style: "display: flex; flex-direction: column;",
                    // Viewport
                    div { style: "width: 100%; height: 400px; background: transparent;",
                        VideoViewport { player: player.clone() }
                    }

                    // Controls bar
                    VideoControls { player: player.clone() }
                }
            }

            Space { h: "lg" }

            // ── Player state display ──────────────────────────────────
            Paper { p: "md", radius: "md", with_border: true,
                Stack { gap: "sm",
                    Title { order: 3, "Player State" }
                    Text { size: "sm",
                        {|| {
                            let state = player.state.get();
                            match state {
                                PlaybackState::Idle => "State: Idle".to_string(),
                                PlaybackState::Loading => "State: Loading".to_string(),
                                PlaybackState::Playing => "State: Playing".to_string(),
                                PlaybackState::Paused => "State: Paused".to_string(),
                                PlaybackState::Ended => "State: Ended".to_string(),
                                PlaybackState::Error(e) => format!("State: Error — {e}"),
                            }
                        }}
                    }
                    Text { size: "sm",
                        {|| format!("Position: {:.1}s / {:.1}s", player.position.get(), player.duration.get())}
                    }
                    Text { size: "sm",
                        {|| format!(
                            "Volume: {:.0}%{}",
                            player.volume.get() * 100.0,
                            if player.muted.get() { " (muted)" } else { "" }
                        )}
                    }
                }
            }

            Space { h: "lg" }

            // ── API demo buttons ──────────────────────────────────────
            Paper { p: "md", radius: "md", with_border: true,
                Stack { gap: "sm",
                    Title { order: 3, "API Demo" }
                    Text { size: "sm", color: "dimmed",
                        "Control the player programmatically via the VideoPlayer handle."
                    }
                    Group { gap: "sm",
                        Button {
                            onclick: { let p = player.clone(); move || p.play() },
                            "Play"
                        }
                        Button {
                            onclick: { let p = player.clone(); move || p.pause() },
                            "Pause"
                        }
                        Button {
                            variant: "light",
                            onclick: { let p = player.clone(); move || p.seek(0.0) },
                            "Restart"
                        }
                        Button {
                            variant: "light",
                            onclick: { let p = player.clone(); move || p.seek(p.position.get() + 10.0) },
                            "+10s"
                        }
                        Button {
                            variant: "light",
                            onclick: { let p = player.clone(); move || p.seek((p.position.get() - 10.0).max(0.0)) },
                            "-10s"
                        }
                    }
                    Group { gap: "sm",
                        Button {
                            variant: "subtle",
                            onclick: { let p = player.clone(); move || p.set_volume(1.0) },
                            "Vol 100%"
                        }
                        Button {
                            variant: "subtle",
                            onclick: { let p = player.clone(); move || p.set_volume(0.5) },
                            "Vol 50%"
                        }
                        Button {
                            variant: "subtle",
                            onclick: { let p = player.clone(); move || p.set_muted(!p.muted.get()) },
                            "Toggle Mute"
                        }
                    }
                }
            }

            Space { h: "xl" }

            // ── Network streaming demo ──────────────────────────────────
            Stack { gap: "xs",
                Title { order: 2, "Network Streaming" }
                Text { size: "lg", color: "dimmed",
                    "Streaming from a remote URL. mpv handles HTTP buffering natively. The seek bar shows buffered data as a lighter region."
                }
            }
            Space { h: "md" }

            {network_video_player(__scope)}
        }
    }
}

#[component]
fn network_video_player() -> NodeHandle {
    let net_player = use_video_player_paused("https://media.w3.org/2010/05/sintel/trailer.mp4");

    rsx! {
        Fragment {
            Paper { p: "0", radius: "md", with_border: true,
                style: "overflow: hidden;",

                div { style: "display: flex; flex-direction: column;",
                    div { style: "width: 100%; height: 400px; background: transparent;",
                        VideoViewport { player: net_player.clone() }
                    }
                    VideoControls { player: net_player.clone() }
                }
            }

            Space { h: "lg" }

            Paper { p: "md", radius: "md", with_border: true,
                Stack { gap: "sm",
                    Title { order: 3, "Stream State" }
                    Text { size: "sm",
                        {|| {
                            let state = net_player.state.get();
                            match state {
                                PlaybackState::Idle => "State: Idle".to_string(),
                                PlaybackState::Loading => "State: Loading / Buffering...".to_string(),
                                PlaybackState::Playing => "State: Playing".to_string(),
                                PlaybackState::Paused => "State: Paused".to_string(),
                                PlaybackState::Ended => "State: Ended".to_string(),
                                PlaybackState::Error(e) => format!("State: Error — {e}"),
                            }
                        }}
                    }
                    Text { size: "sm",
                        {|| format!("Position: {:.1}s / {:.1}s", net_player.position.get(), net_player.duration.get())}
                    }
                    Text { size: "sm",
                        {|| format!("Buffered: {:.1}s ahead", net_player.buffered.get())}
                    }
                }
            }
        }
    }
}
