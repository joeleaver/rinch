//! review-1015: a video sink dropped during thread-local teardown must not
//! abort the process. `SurfaceLease::drop` reaches `SURFACE_REGISTRY.with`,
//! which panics once that TLS is destroyed; a panic in a TLS destructor is
//! `fatal runtime error: thread local panicked on drop` (SIGABRT). The main
//! thread's TLS destructors run at `exit()` on Linux (measured, both returning
//! from `main` and `std::process::exit`), and the desktop loop touches
//! rinch-video's `ACTIVE_PLAYERS` (`is_video_active()`, every AboutToWait)
//! independently of the surface registry. Its own binary: failure is an abort.
#![cfg(all(feature = "desktop", feature = "video"))]

use std::cell::RefCell;

use rinch::render_surface::create_video_frame_sink;
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
fn a_player_still_playing_at_thread_exit_does_not_abort() {
    rinch::video::set_frame_sink_factory(create_video_frame_sink);
    let r = std::thread::spawn(|| {
        // The event loop's AboutToWait poll: ACTIVE_PLAYERS' destructor is
        // registered before SURFACE_REGISTRY's, so it runs after it.
        let _ = rinch::video::is_video_active();
        let player = VideoPlayer::new(Box::new(SinkHoldingBackend::default()));
        player.play(); // the only other clone lives in ACTIVE_PLAYERS
        assert!(
            rinch::render_surface::registered_viewport_names().contains(&player.viewport_id()),
            "positive control: the factory ran and registered the surface"
        );
        drop(player);
    })
    .join();
    assert!(r.is_ok(), "thread teardown panicked");
}
