//! A `RinchApp` that publishes its window (the desktop and Android shells'
//! main window) feeds `rinch::window` (issue #1770): its mount size and scale,
//! then every resize, scale change and focus change it handles. One that does
//! not (the DevTools panel, an embedded context) feeds nothing.

use super::*;
use crate::window::{scale_factor, window_focused, window_size};
use std::cell::Cell;

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
    assert_eq!(seen.get(), Some(true), "the reader ran before the entry was told");
}
