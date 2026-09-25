//! Issue #363: a video's render surface lives exactly as long as its player
//! needs it.
//!
//! The shell's frame-sink factory used to `mem::forget` the surface handle, and
//! `register_active_player` called the factory on **every** `play()`. So each
//! play/resume added one more surface to the thread's registry, none was ever
//! removed, and each kept the last decoded frame it was handed — for the rest
//! of the process, after the video's component had long unmounted.
//!
//! The registry is thread-local and every test runs on its own thread, so each
//! test sees only the surfaces it made.

use std::cell::RefCell;
use std::sync::Once;

use rinch_video::{VideoPlayer, VideoPlayerBackend};

use super::{create_video_frame_sink, registered_viewport_names};

type Sink = std::sync::Arc<dyn Fn(&[u8], u32, u32) + Send + Sync>;

/// What the shell does at startup. The factory is a process-wide `OnceLock`,
/// so every test installs the same one, once.
fn install_shell_factory() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| rinch_video::set_frame_sink_factory(create_video_frame_sink));
}

/// A backend that holds its sink the way `MpvPlayer` does, and releases it on
/// `cleanup`, as a backend's cleanup is documented to.
#[derive(Default)]
struct SinkHoldingBackend {
    sink: RefCell<Option<Sink>>,
}

impl VideoPlayerBackend for SinkHoldingBackend {
    fn play(&self) {}
    fn pause(&self) {}
    fn seek(&self, _seconds: f64) {}
    fn set_volume(&self, _vol: f32) {}
    fn set_muted(&self, _muted: bool) {}
    fn set_source(&self, _src: &str) {}
    fn cleanup(&self) {
        self.sink.borrow_mut().take();
    }
    fn set_frame_sink(&self, sink: Sink) {
        *self.sink.borrow_mut() = Some(sink);
    }
}

fn player() -> VideoPlayer {
    install_shell_factory();
    VideoPlayer::new(Box::new(SinkHoldingBackend::default()))
}

fn surfaces_named(name: &str) -> usize {
    registered_viewport_names()
        .iter()
        .filter(|n| n.as_str() == name)
        .count()
}

/// The issue's own shape: once the player is cleaned up — which
/// `use_video_player` does when the component showing it unmounts — the
/// surface is gone from the registry, and its last frame with it.
#[test]
fn cleanup_releases_the_players_surface() {
    let player = player();
    let name = player.viewport_id();
    player.play();
    assert_eq!(surfaces_named(&name), 1, "play() registers the surface");

    player.cleanup();
    assert_eq!(
        surfaces_named(&name),
        0,
        "a cleaned-up player's surface is still registered: {:?}",
        registered_viewport_names()
    );
}

/// Pausing and resuming is one surface, not one per `play()`.
#[test]
fn resuming_playback_reuses_the_surface() {
    let player = player();
    let name = player.viewport_id();
    for _ in 0..3 {
        player.play();
        player.pause();
    }
    player.play();
    assert_eq!(
        surfaces_named(&name),
        1,
        "every play() minted another surface: {:?}",
        registered_viewport_names()
    );
    player.cleanup();
    assert_eq!(surfaces_named(&name), 0);
}

/// A player dropped without `cleanup` — built outside a render and let go —
/// still gives its surface back, because the surface belongs to the sink.
#[test]
fn dropping_the_player_releases_its_surface() {
    let player = player();
    let name = player.viewport_id();
    player.play();
    player.pause(); // out of the active list, so this is the last clone
    assert_eq!(surfaces_named(&name), 1);

    drop(player);
    assert_eq!(
        surfaces_named(&name),
        0,
        "a dropped player's surface is still registered: {:?}",
        registered_viewport_names()
    );
}

/// The sink owns the registration: it lasts while any clone of the sink does,
/// and ends with the last one.
#[test]
fn the_last_clone_of_a_sink_releases_the_surface() {
    let name = "video-363-sink";
    let sink = create_video_frame_sink(name);
    let other = sink.clone();
    sink(&[0, 0, 0, 255], 1, 1);
    drop(sink);
    assert_eq!(surfaces_named(name), 1, "a live clone keeps the surface");
    drop(other);
    assert_eq!(surfaces_named(name), 0, "the last clone releases it");
}
