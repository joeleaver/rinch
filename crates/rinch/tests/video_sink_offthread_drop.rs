//! Issue #363: a video frame sink whose release cannot reach the surface
//! registry at the moment it drops still gives its surface back.
//!
//! Two such moments: a drop on another thread (the sink is `Send + Sync`, the
//! registry thread-local), and a drop from inside a walk of the registry (a
//! render callback letting go of a player). Both queue the release for the main
//! thread. Their own binary, holding one lock between them, because the
//! main-thread queue is process-global — a parallel test draining it would run
//! the release on *its* thread, whose registry does not hold the surface.
#![cfg(feature = "desktop")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use rinch::render_surface::{
    create_render_surface, create_video_frame_sink, invoke_render_callbacks, mount_render_surface,
    registered_viewport_names, update_layout_size_by_id,
};

static MAIN_QUEUE: Mutex<()> = Mutex::new(());

fn surfaces_named(name: &str) -> usize {
    registered_viewport_names()
        .iter()
        .filter(|n| n.as_str() == name)
        .count()
}

#[test]
fn a_sink_dropped_off_thread_releases_its_surface_on_the_next_drain() {
    let _queue = MAIN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
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

/// A render callback runs while the registry is borrowed; a sink it drops
/// there must not try to take the registry mutably (a `BorrowMutError` panic
/// inside `Drop`), and still releases its surface.
#[test]
fn a_sink_dropped_inside_a_render_callback_releases_its_surface() {
    let _queue = MAIN_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let name = "video-363-in-callback";
    let held = Rc::new(RefCell::new(Some(create_video_frame_sink(name))));

    let host = create_render_surface();
    mount_render_surface(&host);
    update_layout_size_by_id(host.id(), 10, 10); // a callback runs only once measured
    let held_in_cb = held.clone();
    host.set_render_callback(move |_, _, _| {
        held_in_cb.borrow_mut().take();
    });

    invoke_render_callbacks();
    assert!(
        held.borrow().is_none(),
        "the callback ran and dropped the sink"
    );
    rinch_core::drain_main_callbacks();
    assert_eq!(
        surfaces_named(name),
        0,
        "a sink dropped inside a render callback leaked its surface: {:?}",
        registered_viewport_names()
    );
}
