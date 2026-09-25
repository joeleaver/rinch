//! Issue #363: a video frame sink dropped on another thread still gives its
//! surface back.
//!
//! The sink is `Send + Sync`, but the surface registry it holds a place in is
//! thread-local, so a drop on the wrong thread cannot reach it directly: it is
//! queued for the main thread instead. Its own binary, because the main-thread
//! queue is process-global — a parallel test draining it would run the release
//! on *its* thread, whose registry does not hold the surface.
#![cfg(feature = "desktop")]

use rinch::render_surface::{create_video_frame_sink, registered_viewport_names};

fn surfaces_named(name: &str) -> usize {
    registered_viewport_names()
        .iter()
        .filter(|n| n.as_str() == name)
        .count()
}

#[test]
fn a_sink_dropped_off_thread_releases_its_surface_on_the_next_drain() {
    let name = "video-363-offthread";
    let sink = create_video_frame_sink(name);
    assert_eq!(surfaces_named(name), 1);

    std::thread::spawn(move || drop(sink)).join().unwrap();
    // Positive control: the drop did not reach this thread's registry itself.
    assert_eq!(
        surfaces_named(name),
        1,
        "released before the main thread ran anything"
    );

    rinch_core::drain_main_callbacks();
    assert_eq!(
        surfaces_named(name),
        0,
        "a sink dropped off-thread leaked its surface: {:?}",
        registered_viewport_names()
    );
}
