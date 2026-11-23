//! A picture that fails to load fires its `<img>`'s `onerror` on the desktop
//! (issues #2202, #2203), and the components built on it fall back.
//!
//! The browser fires `error` at an `<img>` whose picture failed. The desktop
//! had no such event: rinch-dom knew (it shows the `alt` placeholder for a
//! failed picture, #2309), but nothing told the app, so `Image::fallback_src`
//! (a JavaScript `onerror="this.src=…"` string) did nothing here and `Avatar`
//! kept a broken image. rinch-dom now queues each failed `<img>`
//! (`drain_img_errors`) and the shell fires its `data-onerror` after layout.
//!
//! Two ways to fail are covered, because they reach the queue by different
//! roads: a malformed `data:` URL fails in place as its `src` is written, and
//! a loader's miss arrives later, through the pending-image drain inside
//! layout.

use super::*;

use rinch_components::avatar::Avatar;
use rinch_components::image::Image;
use rinch_core::{Callback, Component};
use std::cell::Cell;
use std::time::{Duration, Instant};

const VIEWPORT: (f32, f32) = (800.0, 600.0);

/// Not a PNG: decoding it in place fails.
const BAD_DATA_URI: &str = "data:image/png;base64,bm90IGEgcGljdHVyZQ==";
/// A 3x2 PNG.
const GOOD_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAMAAAACCAYAAACddGYaAAAAEUlEQVR42mP4z8DwH4YZkDkAm34L9Q9fJiIAAAAASUVORK5CYII=";

/// A scheme whose loader answers every source with a miss, asynchronously.
fn failing_scheme() -> &'static str {
    const SCHEME: &str = "app-2202-missing";
    crate::image::register_image_scheme(SCHEME, |src: &str| {
        rinch_core::image::ImageLoadResult::Failed(format!("{src} is not here"))
    });
    SCHEME
}

/// An `<img src>` carrying an `onerror` that counts its calls.
fn img_with_onerror(src: &'static str, fired: Rc<Cell<u32>>) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let img = scope.create_element("img");
        let fired = fired.clone();
        let id = scope.register_handler(move || fired.set(fired.get() + 1));
        img.set_attribute("data-onerror", &id.0.to_string());
        img.set_attribute("src", src);
        root.append_child(&img);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app
}

/// Frames until `done` holds (a loader answers on another thread), at most 10 s.
fn frames_until(app: &mut RinchApp, done: impl Fn(&RinchApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) && Instant::now() < deadline {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn has_class(app: &RinchApp, class: &str) -> bool {
    let doc = app.doc.as_ref().expect("mounted");
    let d = doc.borrow();
    // Connected only: the placeholder is built (detached) at render time.
    d.tree.nodes.iter().any(|(id, n)| {
        n.attributes
            .get("class")
            .is_some_and(|c| c.split_whitespace().any(|one| one == class))
            && rinch_core::dom::DomDocument::is_connected(&*d, rinch_core::dom::NodeId(id))
    })
}

fn img_srcs(app: &RinchApp) -> Vec<String> {
    let doc = app.doc.as_ref().expect("mounted");
    let d = doc.borrow();
    d.tree
        .nodes
        .iter()
        .filter(|(id, n)| {
            n.tag() == Some("img")
                && rinch_core::dom::DomDocument::is_connected(&*d, rinch_core::dom::NodeId(*id))
        })
        .filter_map(|(_, n)| n.attributes.get("src").cloned())
        .collect()
}

/// A malformed `data:` URL fails as its `src` is written: `onerror` runs in
/// the first frame, once, however many frames follow.
#[test]
fn a_picture_that_fails_in_place_fires_onerror_once() {
    let fired = Rc::new(Cell::new(0));
    let mut app = img_with_onerror(BAD_DATA_URI, fired.clone());
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_eq!(
        fired.get(),
        1,
        "the failure is announced in the first frame"
    );
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(fired.get(), 1, "and only once");
}

/// A loader's miss arrives on another thread and is drained inside layout:
/// `onerror` runs after the layout that took it in.
#[test]
fn a_picture_a_loader_misses_fires_onerror() {
    let scheme = failing_scheme();
    let src: &'static str = Box::leak(format!("{scheme}:portrait").into_boxed_str());
    let fired = Rc::new(Cell::new(0));
    let mut app = img_with_onerror(src, fired.clone());
    let f = fired.clone();
    frames_until(&mut app, move |_| f.get() > 0);
    assert_eq!(fired.get(), 1);
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(fired.get(), 1, "once per failure");
}

/// The control: the same harness with a picture that decodes fires nothing.
#[test]
fn a_picture_that_loads_fires_no_onerror() {
    let fired = Rc::new(Cell::new(0));
    let mut app = img_with_onerror(GOOD_DATA_URI, fired.clone());
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(fired.get(), 0);
}

/// `Avatar` (#2202): its image fails, `on_error` runs, and the initials
/// take its place.
#[test]
fn an_avatar_whose_image_fails_shows_its_initials() {
    let scheme = failing_scheme();
    let fired = Rc::new(Cell::new(0));
    let f = fired.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let f = f.clone();
        Avatar {
            src: format!("{scheme}:avatar"),
            name: "Ada Lovelace".into(),
            on_error: Some(Callback::new(move || f.set(f.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[])
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    assert!(has_class(&app, "rinch-avatar__image"), "precondition");
    frames_until(&mut app, |app| has_class(app, "rinch-avatar__placeholder"));
    assert_eq!(fired.get(), 1);
    assert!(img_srcs(&app).is_empty(), "the image gave way");
    let doc = app.doc.as_ref().expect("mounted");
    let text = {
        let d = doc.borrow();
        let id = d
            .tree
            .nodes
            .iter()
            .find(|(id, n)| {
                n.attributes
                    .get("class")
                    .is_some_and(|c| c.contains("rinch-avatar__placeholder"))
                    && rinch_core::dom::DomDocument::is_connected(&*d, rinch_core::dom::NodeId(*id))
            })
            .map(|(id, _)| id)
            .expect("placeholder");
        rinch_core::dom::DomDocument::text_content(&*d, rinch_core::dom::NodeId(id))
    };
    assert_eq!(text.as_deref(), Some("AL"));
}

/// `Image::fallback_src` (#2203): the failed picture is replaced by the
/// fallback, which loads, and nothing more fires.
#[test]
fn an_image_whose_picture_fails_shows_its_fallback_src() {
    let fired = Rc::new(Cell::new(0));
    let f = fired.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let f = f.clone();
        Image {
            src: BAD_DATA_URI.into(),
            fallback_src: GOOD_DATA_URI.into(),
            on_error: Some(Callback::new(move || f.set(f.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[])
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(img_srcs(&app), vec![GOOD_DATA_URI.to_string()]);
    assert_eq!(fired.get(), 1, "the fallback loaded: one failure");
}

/// `src=""` fails, as in Chrome (which fires `error` for it); a missing `src`
/// is no load at all and fires nothing.
#[test]
fn an_empty_src_fires_onerror_and_a_missing_one_does_not() {
    let fired = Rc::new(Cell::new(0));
    let mut app = img_with_onerror("", fired.clone());
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(fired.get(), 1, "src=\"\" fires once");

    let missing = Rc::new(Cell::new(0));
    let m = missing.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let img = scope.create_element("img");
        let m = m.clone();
        let id = scope.register_handler(move || m.set(m.get() + 1));
        img.set_attribute("data-onerror", &id.0.to_string());
        img.set_attribute("alt", "no source");
        root.append_child(&img);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(missing.get(), 0, "no src, no error");
}

/// An unknown `register_rgba` name fails in place and is listed among the
/// newly failed too, so it reaches the queue twice in one frame: the queue
/// keeps one, and `onerror` runs once.
#[test]
fn a_failure_queued_twice_in_a_frame_fires_once() {
    let src: &'static str =
        Box::leak(rinch_core::image::rgba_src("no-such-picture-2202").into_boxed_str());
    let fired = Rc::new(Cell::new(0));
    let mut app = img_with_onerror(src, fired.clone());
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(fired.get(), 1);
}

/// A `src` rewritten to a good picture before the frame that would announce
/// the failure: the element no longer names a failed picture, so nothing
/// fires.
#[test]
fn a_failure_whose_src_was_replaced_by_a_good_one_fires_nothing() {
    let fired = Rc::new(Cell::new(0));
    let f = fired.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let img = scope.create_element("img");
        let f = f.clone();
        let id = scope.register_handler(move || f.set(f.get() + 1));
        img.set_attribute("data-onerror", &id.0.to_string());
        img.set_attribute("src", BAD_DATA_URI);
        img.set_attribute("src", GOOD_DATA_URI);
        root.append_child(&img);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    for _ in 0..3 {
        app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    }
    assert_eq!(fired.get(), 0);
}

/// `Anchor { onclick }` with no `href` (#2206 review): Tab reaches it and
/// Enter runs the handler. A placeholder `<a>` is not focusable, so without
/// its `tabindex` the keyboard could not get there. (It is given a box:
/// desktop Tab skips inline elements altogether, #2963.)
#[test]
fn an_anchor_with_onclick_and_no_href_is_reached_by_tab_and_enter() {
    use rinch_components::anchor::Anchor;
    let clicks = Rc::new(Cell::new(0));
    let c = clicks.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let c = c.clone();
        let root = scope.create_element("div");
        let text = scope.create_text("Settings");
        let anchor = Anchor {
            onclick: Some(Callback::new(move || c.set(c.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[text]);
        // A box of its own: desktop Tab skips an inline element, whose
        // Taffy box is 0x0, even an `<a href>` (#2963, not this PR's).
        anchor.set_attribute("style", "display: inline-block");
        root.append_child(&anchor);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    let key = |app: &mut RinchApp, key: KeyCode| {
        app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: None,
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
            (800, 600),
            1.0,
        );
    };
    key(&mut app, KeyCode::Tab);
    assert!(
        matches!(app.focus_target, FocusTarget::Node(_)),
        "Tab reached the anchor, got {:?}",
        app.focus_target
    );
    key(&mut app, KeyCode::Enter);
    assert_eq!(clicks.get(), 1, "Enter ran its onclick");
}
