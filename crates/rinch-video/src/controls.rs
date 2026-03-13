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
    // Outer container
    let container = scope.create_element("div");
    container.set_attribute("class", "rinch-video-controls");
    container.set_attribute(
        "style",
        "display: flex; flex-direction: column; background: #1a1a2e; padding: 4px 8px; gap: 4px; user-select: none;",
    );

    // --- Seek bar row ---
    let seek_row = scope.create_element("div");
    seek_row.set_attribute(
        "style",
        "display: flex; align-items: center; gap: 8px; height: 20px;",
    );

    let seek_bar = scope.create_element("div");
    seek_bar.set_attribute(
        "style",
        "flex: 1; height: 20px; background: transparent; cursor: pointer; position: relative; display: flex; align-items: center;",
    );

    // Seek bar click handler — seeks to the clicked position
    {
        let p = player.clone();
        let handler_id = scope.register_handler(move || {
            let ctx = get_click_context();
            let pct = ctx.percent_x();
            let dur = p.duration.get();
            if dur > 0.0 {
                let seek_to = (pct as f64) * dur;
                p.seek(seek_to);
            }
        });
        seek_bar.set_attribute("data-rid", &handler_id.0.to_string());
    }

    // Visual track (thin bar inside the click target)
    let seek_track = scope.create_element("div");
    seek_track.set_attribute(
        "style",
        "flex: 1; height: 4px; background: #333; border-radius: 2px; position: relative; pointer-events: none; overflow: hidden;",
    );

    // Buffer indicator (lighter bar showing buffered range)
    let seek_buffer = scope.create_element("div");
    {
        let p = player.clone();
        let buf = seek_buffer.clone();
        scope.create_effect(move || {
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

    // Playback progress fill (red bar)
    let seek_fill = scope.create_element("div");
    {
        let p = player.clone();
        let fill = seek_fill.clone();
        scope.create_effect(move || {
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

    seek_track.append_child(&seek_buffer);
    seek_track.append_child(&seek_fill);
    seek_bar.append_child(&seek_track);
    seek_row.append_child(&seek_bar);
    container.append_child(&seek_row);

    // --- Controls row ---
    let controls_row = scope.create_element("div");
    controls_row.set_attribute(
        "style",
        "display: flex; align-items: center; gap: 8px; height: 32px;",
    );

    // Play/Pause button — pre-render all icon variants, toggle visibility reactively
    let play_btn = scope.create_element("button");
    play_btn.set_attribute(
        "style",
        "background: none; border: none; color: white; cursor: pointer; padding: 4px 8px; display: flex; align-items: center; justify-content: center;",
    );
    {
        let p = player.clone();
        let handler_id = scope.register_handler(move || p.toggle());
        play_btn.set_attribute("data-rid", &handler_id.0.to_string());
    }
    let icon_play = icon(scope, TablerIcon::PlayerPlay, 20);
    let icon_pause = icon(scope, TablerIcon::PlayerPause, 20);
    let icon_restart = icon(scope, TablerIcon::Refresh, 20);
    play_btn.append_child(&icon_play);
    play_btn.append_child(&icon_pause);
    play_btn.append_child(&icon_restart);
    {
        let p = player.clone();
        let i_play = icon_play.clone();
        let i_pause = icon_pause.clone();
        let i_restart = icon_restart.clone();
        scope.create_effect(move || {
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
    controls_row.append_child(&play_btn);

    // Timestamp
    if show_timestamp {
        let ts = scope.create_element("span");
        ts.set_attribute(
            "style",
            "color: #aaa; font-size: 12px; font-family: monospace; min-width: 80px;",
        );
        {
            let p = player.clone();
            let ts_clone = ts.clone();
            scope.create_effect(move || {
                let pos = p.position.get();
                let dur = p.duration.get();
                let text = format!("{} / {}", format_time(pos), format_time(dur));
                ts_clone.set_text(&text);
            });
        }
        controls_row.append_child(&ts);
    }

    // Spacer
    let spacer = scope.create_element("div");
    spacer.set_attribute("style", "flex: 1;");
    controls_row.append_child(&spacer);

    // Volume: mute button + slider (YouTube-style)
    if show_volume {
        let vol_group = scope.create_element("div");
        vol_group.set_attribute("style", "display: flex; align-items: center; gap: 4px;");

        // Mute/unmute button
        let vol_btn = scope.create_element("button");
        vol_btn.set_attribute(
            "style",
            "background: none; border: none; color: white; cursor: pointer; padding: 4px 8px; display: flex; align-items: center; justify-content: center;",
        );
        {
            let p = player.clone();
            let handler_id = scope.register_handler(move || p.set_muted(!p.muted.get()));
            vol_btn.set_attribute("data-rid", &handler_id.0.to_string());
        }
        let icon_vol_off = icon(scope, TablerIcon::VolumeOff, 18);
        let icon_vol_low = icon(scope, TablerIcon::Volume2, 18);
        let icon_vol_high = icon(scope, TablerIcon::Volume, 18);
        vol_btn.append_child(&icon_vol_off);
        vol_btn.append_child(&icon_vol_low);
        vol_btn.append_child(&icon_vol_high);
        {
            let p = player.clone();
            let i_off = icon_vol_off.clone();
            let i_low = icon_vol_low.clone();
            let i_high = icon_vol_high.clone();
            scope.create_effect(move || {
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
        vol_group.append_child(&vol_btn);

        // Volume slider (click target area)
        let vol_slider = scope.create_element("div");
        vol_slider.set_attribute(
            "style",
            "width: 80px; height: 20px; background: transparent; cursor: pointer; display: flex; align-items: center;",
        );
        {
            let p = player.clone();
            let handler_id = scope.register_handler(move || {
                let ctx = get_click_context();
                let vol = ctx.percent_x();
                p.set_muted(false);
                p.set_volume(vol);
            });
            vol_slider.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Volume slider track
        let vol_track = scope.create_element("div");
        vol_track.set_attribute(
            "style",
            "flex: 1; height: 4px; background: #555; border-radius: 2px; position: relative; pointer-events: none;",
        );

        // Volume slider fill
        let vol_fill = scope.create_element("div");
        {
            let p = player.clone();
            let fill = vol_fill.clone();
            scope.create_effect(move || {
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

        vol_track.append_child(&vol_fill);
        vol_slider.append_child(&vol_track);
        vol_group.append_child(&vol_slider);
        controls_row.append_child(&vol_group);
    }

    container.append_child(&controls_row);
    container
}
