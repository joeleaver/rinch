//! Review of #1652: the title tooltip in an embedded context.
//!     cargo test -p rinch --features embed --test embed_title_tooltip_review_1652
#![cfg(all(feature = "desktop", any(feature = "gpu", feature = "embed")))]

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{AppAction, Instant, PlatformEvent};
use rinch::prelude::*;

#[test]
fn a_resting_pointer_is_a_wake_and_the_scene_draws_the_tooltip() {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "position: absolute; left: 20px; top: 20px; width: 200px; height: 100px",
            );
            root.set_attribute("title", "Hello");
            root
        },
    );
    ctx.update(&[]);
    let _ = ctx.scene();
    assert_eq!(ctx.next_wake(), None, "control: nothing scheduled");
    ctx.update(&[PlatformEvent::MouseMove { x: 77.0, y: 61.0 }]);
    let paths_before = ctx.scene().encoding().n_paths;
    let wake = ctx
        .next_wake()
        .expect("a resting pointer over a title is a wake");
    assert!(!ctx.needs_update(), "not due yet");
    let now = Instant::now();
    if wake > now {
        std::thread::sleep(wake - now);
    }
    assert!(ctx.needs_update(), "due");
    let actions = ctx.update(&[]);
    assert!(actions.contains(&AppAction::RequestRedraw));
    assert_eq!(ctx.app().title_tooltip_text(), Some("Hello"));
    assert_eq!(ctx.next_wake(), None, "no wake once shown");
    let paths_after = ctx.scene().encoding().n_paths;
    assert!(
        paths_after > paths_before,
        "the scene draws it: {paths_before} -> {paths_after}"
    );
}

/// `pointer_left` takes a shown tooltip down and owes a frame (#1646 review).
#[test]
fn pointer_left_hides_it() {
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 800,
            height: 600,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute(
                "style",
                "position: absolute; left: 20px; top: 20px; width: 200px; height: 100px",
            );
            root.set_attribute("title", "Hello");
            root
        },
    );
    ctx.update(&[]);
    assert!(!ctx.pointer_left(), "control: nothing to hide");
    ctx.update(&[PlatformEvent::MouseMove { x: 77.0, y: 61.0 }]);
    let wake = ctx.next_wake().expect("armed");
    let now = Instant::now();
    if wake > now {
        std::thread::sleep(wake - now);
    }
    ctx.update(&[]);
    assert_eq!(ctx.app().title_tooltip_text(), Some("Hello"));
    assert!(ctx.pointer_left());
    assert_eq!(ctx.app().title_tooltip_text(), None);
    assert_eq!(ctx.next_wake(), None);
}
