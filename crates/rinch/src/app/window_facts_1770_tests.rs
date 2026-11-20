//! A `RinchApp` that publishes its window (the desktop and Android shells'
//! main window) feeds `rinch::window` (issue #1770): its mount size and scale,
//! then every resize, scale change and focus change it handles. One that does
//! not (the DevTools panel, an embedded context) feeds nothing.

use super::*;
use crate::window::{scale_factor, window_focused, window_size};
use std::cell::{Cell, RefCell};

fn app(publishes: bool) -> RinchApp {
    let mut app = RinchApp::new(|scope: &mut RenderScope| scope.create_element("div"));
    app.set_publishes_window_state(publishes);
    app.set_device_pixel_ratio(1.25);
    app.mount_component(643.0, 481.0);
    app
}

#[test]
fn the_main_window_publishes_its_size_scale_and_focus() {
    let mut app = app(true);
    assert_eq!(window_size(), (643.0, 481.0));
    assert_eq!(scale_factor(), 1.25);
    assert!(!window_focused(), "not focused until the system says so");

    let runs = Rc::new(Cell::new(0));
    let _reader = {
        let runs = runs.clone();
        rinch_core::Effect::new(move || {
            let _ = (window_size(), scale_factor(), window_focused());
            runs.set(runs.get() + 1);
        })
    };
    assert_eq!(runs.get(), 1);

    app.handle_event(PlatformEvent::WindowFocus(true), (643, 481), 1.25);
    assert!(window_focused());
    assert_eq!(runs.get(), 2);

    app.handle_event(
        PlatformEvent::Resized {
            width: 517,
            height: 389,
        },
        (517, 389),
        1.25,
    );
    assert_eq!(window_size(), (517.0, 389.0));
    assert_eq!(runs.get(), 3);

    app.handle_event(PlatformEvent::ScaleFactorChanged(1.75), (517, 389), 1.75);
    assert_eq!(scale_factor(), 1.75);
    assert_eq!(runs.get(), 4);

    app.handle_event(PlatformEvent::WindowFocus(false), (517, 389), 1.75);
    assert!(!window_focused());
    assert_eq!(runs.get(), 5);

    // The same focus again changes nothing.
    app.handle_event(PlatformEvent::WindowFocus(false), (517, 389), 1.75);
    assert_eq!(runs.get(), 5);
}

#[test]
fn an_app_that_does_not_publish_feeds_nothing() {
    let main = app(true);
    let mut panel = app(false);
    let _ = main;
    // `app(false)` mounted at the same size; tell them apart by what follows.
    panel.handle_event(
        PlatformEvent::Resized {
            width: 311,
            height: 199,
        },
        (311, 199),
        1.25,
    );
    panel.handle_event(PlatformEvent::ScaleFactorChanged(2.5), (311, 199), 2.5);
    panel.handle_event(PlatformEvent::WindowFocus(true), (311, 199), 2.5);
    assert_eq!(window_size(), (643.0, 481.0));
    assert_eq!(scale_factor(), 1.25);
    assert!(!window_focused());

    // Nor does its mount.
    let mut other = RinchApp::new(|scope: &mut RenderScope| scope.create_element("div"));
    other.set_device_pixel_ratio(3.0);
    other.mount_component(101.0, 103.0);
    assert_eq!(window_size(), (643.0, 481.0));
    assert_eq!(scale_factor(), 1.25);
}

/// A new scale reaches `scale_factor` by whatever route it takes — the
/// desktop's re-created window and Android's `InitWindow` call
/// `set_device_pixel_ratio` without a `ScaleFactorChanged`.
#[test]
fn every_new_scale_is_published() {
    let mut app = app(true);
    app.set_device_pixel_ratio(2.75);
    assert_eq!(scale_factor(), 2.75);

    let mut panel = app_unmounted();
    panel.set_device_pixel_ratio(4.5);
    assert_eq!(scale_factor(), 2.75, "an app that does not publish");
}

fn app_unmounted() -> RinchApp {
    RinchApp::new(|scope: &mut RenderScope| scope.create_element("div"))
}

/// Turned on after the mount (`RinchContext` mounts in its constructor), the
/// flag publishes the scale and size the app already has.
#[test]
fn turning_publishing_on_after_the_mount_publishes_what_the_app_has() {
    let mut app = app(false);
    app.handle_event(
        PlatformEvent::Resized {
            width: 707,
            height: 353,
        },
        (707, 353),
        1.25,
    );
    assert_eq!(window_size(), (0.0, 0.0));
    app.set_publishes_window_state(true);
    assert_eq!(window_size(), (707.0, 353.0));
    assert_eq!(scale_factor(), 1.25);
}

/// `window_focused` turns false only after the app's own blur handling: a
/// reader sees the key entries already told the window lost focus.
#[test]
fn a_blur_reader_runs_after_the_apps_own_blur_handling() {
    let told = Rc::new(Cell::new(false));
    let entry: Rc<RefCell<Option<rinch_core::DismissHandle>>> = Rc::default();
    let mut app = {
        let (told, entry) = (told.clone(), entry.clone());
        RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            let told = told.clone();
            *entry.borrow_mut() = Some(rinch_core::push_key_handler(
                &root,
                |_| false,
                move || told.set(true),
            ));
            root
        })
    };
    app.set_publishes_window_state(true);
    app.mount_component(643.0, 481.0);
    app.handle_event(PlatformEvent::WindowFocus(true), (643, 481), 1.0);
    told.set(false);

    let seen = Rc::new(Cell::new(None));
    let _reader = {
        let (told, seen) = (told.clone(), seen.clone());
        rinch_core::Effect::new(move || {
            if !window_focused() {
                seen.set(Some(told.get()));
            }
        })
    };
    app.handle_event(PlatformEvent::WindowFocus(false), (643, 481), 1.0);
    assert!(told.get(), "positive control: the key entry was told");
    assert_eq!(
        seen.get(),
        Some(true),
        "the reader ran before the entry was told"
    );
}

/// A new scale goes out with the surface size the shell set before it, in
/// one batch (`show_window`'s re-created window, Android's re-`InitWindow`):
/// a reader of both runs once, and sees both new.
#[test]
fn a_new_scale_and_its_surface_are_one_publish() {
    let mut app = app(true);
    let seen = Rc::new(Cell::new(((0, 0), 0.0)));
    let runs = Rc::new(Cell::new(0));
    let _reader = {
        let (seen, runs) = (seen.clone(), runs.clone());
        rinch_core::Effect::new(move || {
            seen.set((crate::window::window_physical_size(), scale_factor()));
            runs.set(runs.get() + 1);
        })
    };
    app.set_surface_size((1286, 962));
    app.set_device_pixel_ratio(2.0);
    assert_eq!(runs.get(), 2, "one run for both");
    assert_eq!(seen.get(), ((1286, 962), 2.0));
}

/// One read of a window's facts: its logical size, its physical size and its
/// scale.
#[cfg(feature = "windowing")]
type Read = ((f64, f64), (u32, u32), f64);

#[cfg(feature = "windowing")]
thread_local! {
    /// What the effect made in [`window_of_its_own`]'s mount read, each run.
    static OWN_READS: RefCell<Vec<Read>> = const { RefCell::new(Vec::new()) };
}

/// A component whose mount makes an effect that reads its window's facts.
#[cfg(feature = "windowing")]
fn window_of_its_own(scope: &mut RenderScope) -> NodeHandle {
    rinch_core::Effect::new(|| {
        let read = (
            window_size(),
            crate::window::window_physical_size(),
            scale_factor(),
        );
        OWN_READS.with(|reads| reads.borrow_mut().push(read));
    });
    scope.create_element("div")
}

/// #2789: a window with facts of its own (a `Window {}`, `open_window`,
/// DevTools) publishes them under its new document's key before its
/// component is built, so an effect its mount makes reads the window's own
/// facts from its first run and follows them, never the thread's.
#[cfg(feature = "windowing")]
#[test]
fn an_effect_made_in_a_windows_mount_reads_and_follows_its_own_facts() {
    OWN_READS.with(|reads| reads.borrow_mut().clear());
    // The thread's facts (the main window's), which it must not read.
    crate::window::publish(crate::window::WindowFacts {
        scale: Some(1.0),
        size: Some((1111.0, 777.0)),
        physical_size: Some((1111, 777)),
        focused: Some(true),
    });
    let mut app = RinchApp::new(window_of_its_own);
    app.set_publishes_own_window_state();
    app.set_device_pixel_ratio(1.5);
    app.set_surface_size((961, 721));
    app.mount_component(641.0, 481.0);
    let reads = || OWN_READS.with(|reads| reads.borrow().clone());
    assert_eq!(
        reads(),
        [((641.0, 481.0), (961, 721), 1.5)],
        "its own, from the first run"
    );

    app.set_surface_size((1201, 901));
    app.handle_event(
        PlatformEvent::Resized {
            width: 801,
            height: 601,
        },
        (1201, 901),
        1.5,
    );
    assert_eq!(
        reads().last(),
        Some(&((801.0, 601.0), (1201, 901), 1.5)),
        "a resize runs it again with the new values"
    );
    assert_eq!(reads().len(), 2, "once for the resize");

    let before = reads().len();
    crate::window::publish(crate::window::WindowFacts {
        size: Some((333.0, 222.0)),
        ..Default::default()
    });
    assert_eq!(
        reads().len(),
        before,
        "the thread's facts are not what it follows"
    );
}
