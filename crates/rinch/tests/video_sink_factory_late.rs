//! Issue #363 (review of PR #1015, F4): a player played before the frame-sink
//! factory exists gets its sink on a later `play()`.
//!
//! `register_active_player` asks the factory once per player and records that
//! it did. The record must follow a sink actually being made: set on the ask
//! alone, a player played while no factory is installed would never get one.
//! Its own binary, because the factory is a process-wide `OnceLock` that every
//! other video fixture installs.
#![cfg(all(feature = "desktop", feature = "video"))]

use std::cell::RefCell;

use rinch::render_surface::{create_video_frame_sink, registered_viewport_names};
use rinch::video::{VideoPlayer, VideoPlayerBackend};

type Sink = std::sync::Arc<dyn Fn(&[u8], u32, u32) + Send + Sync>;

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

#[test]
fn a_player_played_before_the_factory_exists_gets_a_sink_later() {
    let player = VideoPlayer::new(Box::new(SinkHoldingBackend::default()));
    let name = player.viewport_id();
    player.play();
    player.pause();
    assert!(
        !registered_viewport_names().contains(&name),
        "no factory yet, so no surface"
    );

    rinch::video::set_frame_sink_factory(create_video_frame_sink);
    player.play();
    assert!(
        registered_viewport_names().contains(&name),
        "played again once the factory exists, the player still has no surface"
    );
    player.cleanup();
}
