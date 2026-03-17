//! VideoControls component.
//!
//! A full-featured control bar for video playback. Uses the
//! [`VideoPlayer`] handle's reactive signals for automatic UI updates.
//!
//! The controls are built from plain HTML elements with inline styles,
//! so they work without rinch-components.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events::get_click_context;
use rinch_tabler_icons::{
    TablerIcon, TablerIconOptions, TablerIconStyle, render_tabler_icon_with_options,
};

use crate::VideoPlayer;

/// Render a Tabler icon at a specific pixel size.
fn icon(scope: &mut RenderScope, which: TablerIcon, size: u32) -> NodeHandle {
    render_tabler_icon_with_options(
        scope,
        which,
        TablerIconOptions {
            style: TablerIconStyle::Filled,
            size: Some(size),
            ..Default::default()
        },
    )
}

/// Video playback controls.
///
/// Provides play/pause, seek bar, volume control, and timestamp display.
#[derive(Debug)]
pub struct VideoControls {
    /// The video player handle.
    pub player: VideoPlayer,
    /// Whether to show volume controls (default: true).
    pub show_volume: bool,
    /// Whether to show the timestamp (default: true).
    pub show_timestamp: bool,
}

impl Default for VideoControls {
    fn default() -> Self {
        Self {
            player: VideoPlayer::new(Box::new(crate::viewport::NoopBackend)),
            show_volume: true,
            show_timestamp: true,
        }
    }
}

impl Component for VideoControls {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let player = self.player.clone();
        let show_volume = self.show_volume;
        let show_timestamp = self.show_timestamp;

        video_controls_render(scope, player, show_volume, show_timestamp)
    }
}

/// Format seconds as M:SS or H:MM:SS.
fn format_time(seconds: f64) -> String {
    if seconds.is_nan() || seconds.is_infinite() || seconds < 0.0 {
        return "0:00".to_string();
    }
    let total = seconds as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn video_controls_render(
    scope: &mut RenderScope,
    player: VideoPlayer,
    show_volume: bool,
    show_timestamp: bool,
) -> NodeHandle {
    let __scope = scope;

    // --- Seek bar ---
    // Buffer indicator (reactive style)
    let seek_buffer = rinch_macros::rsx! { div {} };
    {
        let p = player.clone();
        let buf = seek_buffer.clone();
        __scope.create_effect(move || {
            let pos = p.position.get();
            let dur = p.duration.get();
            let buffered = p.buffered.get();
            let buf_pct = if dur > 0.0 {
                ((pos + buffered) / dur * 100.0).min(100.0)
            } else {
                0.0
            };
            buf.set_attribute(
                "style",
                &format!(
                    "position: absolute; top: 0; left: 0; height: 100%; background: rgba(255,255,255,0.2); width: {buf_pct:.1}%; pointer-events: none;"
                ),
            );
        });
    }

    // Playback progress fill (reactive style)
    let seek_fill = rinch_macros::rsx! { div {} };
    {
        let p = player.clone();
        let fill = seek_fill.clone();
        __scope.create_effect(move || {
            let pos = p.position.get();
            let dur = p.duration.get();
            let pct = if dur > 0.0 {
                (pos / dur * 100.0).min(100.0)
            } else {
                0.0
            };
            fill.set_attribute(
                "style",
                &format!(
                    "position: absolute; top: 0; left: 0; height: 100%; background: #e94560; border-radius: 2px; width: {pct:.1}%; pointer-events: none;"
                ),
            );
        });
    }

    // Seek bar click handler
    let p_seek = player.clone();

    let seek_row = rinch_macros::rsx! {
        div {
            style: "display: flex; align-items: center; gap: 8px; height: 20px;",
            div {
                style: "flex: 1; height: 20px; background: transparent; cursor: pointer; position: relative; display: flex; align-items: center;",
                onclick: move || {
                    let ctx = get_click_context();
                    let pct = ctx.percent_x();
                    let dur = p_seek.duration.get();
                    if dur > 0.0 {
                        let seek_to = (pct as f64) * dur;
                        p_seek.seek(seek_to);
                    }
                },
                div {
                    style: "flex: 1; height: 4px; background: #333; border-radius: 2px; position: relative; pointer-events: none; overflow: hidden;",
                    {seek_buffer}
                    {seek_fill}
                }
            }
        }
    };

    // --- Play/Pause button ---
    let icon_play = icon(__scope, TablerIcon::PlayerPlay, 20);
    let icon_pause = icon(__scope, TablerIcon::PlayerPause, 20);
    let icon_restart = icon(__scope, TablerIcon::Refresh, 20);

    let p_toggle = player.clone();
    let play_btn = rinch_macros::rsx! {
        button {
            style: "background: none; border: none; color: white; cursor: pointer; padding: 4px 8px; display: flex; align-items: center; justify-content: center;",
            onclick: move || p_toggle.toggle(),
            {icon_play.clone()}
            {icon_pause.clone()}
            {icon_restart.clone()}
        }
    };

    // Toggle icon visibility reactively
    {
        let p = player.clone();
        let i_play = icon_play;
        let i_pause = icon_pause;
        let i_restart = icon_restart;
        __scope.create_effect(move || {
            let playing = p.playing.get();
            let ended = p.state.get() == crate::PlaybackState::Ended;
            let (show_play, show_pause, show_restart) = if playing {
                ("none", "inline", "none")
            } else if ended {
                ("none", "none", "inline")
            } else {
                ("inline", "none", "none")
            };
            i_play.set_style("display", show_play);
            i_pause.set_style("display", show_pause);
            i_restart.set_style("display", show_restart);
        });
    }

    // --- Timestamp ---
    let timestamp = if show_timestamp {
        let ts = rinch_macros::rsx! {
            span {
                style: "color: #aaa; font-size: 12px; font-family: monospace; min-width: 80px;",
            }
        };
        let p = player.clone();
        let ts_clone = ts.clone();
        __scope.create_effect(move || {
            let pos = p.position.get();
            let dur = p.duration.get();
            let text = format!("{} / {}", format_time(pos), format_time(dur));
            ts_clone.set_text(&text);
        });
        Some(ts)
    } else {
        None
    };

    // --- Volume controls ---
    let volume_group = if show_volume {
        let icon_vol_off = icon(__scope, TablerIcon::VolumeOff, 18);
        let icon_vol_low = icon(__scope, TablerIcon::Volume2, 18);
        let icon_vol_high = icon(__scope, TablerIcon::Volume, 18);

        let p_mute = player.clone();
        let vol_btn = rinch_macros::rsx! {
            button {
                style: "background: none; border: none; color: white; cursor: pointer; padding: 4px 8px; display: flex; align-items: center; justify-content: center;",
                onclick: move || p_mute.set_muted(!p_mute.muted.get()),
                {icon_vol_off.clone()}
                {icon_vol_low.clone()}
                {icon_vol_high.clone()}
            }
        };

        // Toggle volume icon visibility reactively
        {
            let p = player.clone();
            let i_off = icon_vol_off;
            let i_low = icon_vol_low;
            let i_high = icon_vol_high;
            __scope.create_effect(move || {
                let muted = p.muted.get();
                let vol = p.volume.get();
                let (show_off, show_low, show_high) = if muted || vol == 0.0 {
                    ("inline", "none", "none")
                } else if vol < 0.5 {
                    ("none", "inline", "none")
                } else {
                    ("none", "none", "inline")
                };
                i_off.set_style("display", show_off);
                i_low.set_style("display", show_low);
                i_high.set_style("display", show_high);
            });
        }

        // Volume slider fill (reactive style)
        let vol_fill = rinch_macros::rsx! { div {} };
        {
            let p = player.clone();
            let fill = vol_fill.clone();
            __scope.create_effect(move || {
                let muted = p.muted.get();
                let vol = p.volume.get();
                let pct = if muted { 0.0 } else { (vol * 100.0).min(100.0) };
                fill.set_attribute(
                    "style",
                    &format!(
                        "height: 100%; background: white; border-radius: 2px; width: {pct:.1}%; pointer-events: none;"
                    ),
                );
            });
        }

        let p_vol = player.clone();
        let vol_group = rinch_macros::rsx! {
            div {
                style: "display: flex; align-items: center; gap: 4px;",
                {vol_btn}
                div {
                    style: "width: 80px; height: 20px; background: transparent; cursor: pointer; display: flex; align-items: center;",
                    onclick: move || {
                        let ctx = get_click_context();
                        let vol = ctx.percent_x();
                        p_vol.set_muted(false);
                        p_vol.set_volume(vol);
                    },
                    div {
                        style: "flex: 1; height: 4px; background: #555; border-radius: 2px; position: relative; pointer-events: none;",
                        {vol_fill}
                    }
                }
            }
        };
        Some(vol_group)
    } else {
        None
    };

    // --- Assemble controls row ---
    let controls_row = rinch_macros::rsx! {
        div {
            style: "display: flex; align-items: center; gap: 8px; height: 32px;",
            {play_btn}
        }
    };

    if let Some(ts) = timestamp {
        controls_row.append_child(&ts);
    }

    let spacer = rinch_macros::rsx! { div { style: "flex: 1;" } };
    controls_row.append_child(&spacer);

    if let Some(vg) = volume_group {
        controls_row.append_child(&vg);
    }

    // --- Outer container ---
    rinch_macros::rsx! {
        div {
            class: "rinch-video-controls",
            style: "display: flex; flex-direction: column; background: #1a1a2e; padding: 4px 8px; gap: 4px; user-select: none;",
            {seek_row}
            {controls_row}
        }
    }
}
