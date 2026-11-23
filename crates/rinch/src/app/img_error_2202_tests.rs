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
