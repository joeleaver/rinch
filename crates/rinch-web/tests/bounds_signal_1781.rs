//! `NodeHandle::bounds_signal()` in the browser (issue #1781): rinch-web
//! refreshes it from `getBoundingClientRect()` whenever something may have
//! moved the element — its own resize, a DOM write, a scroll — one frame later,
//! and repeated calls from one owner share one signal.
//!
//! Every check compares the signal with the browser's own
//! `getBoundingClientRect()` and first asserts the browser's answer is not the
//! zero rect the signal starts at, so a signal nobody refreshes fails.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{Effect, ElementBounds};
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-bounds-signal-1781-host";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

async fn frames(n: usize) {
    for _ in 0..n {
        next_frame().await;
    }
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn browser_rect(el: &web_sys::Element) -> ElementBounds {
    let r = el.get_bounding_client_rect();
    ElementBounds {
        x: r.x() as f32,
        y: r.y() as f32,
        width: r.width() as f32,
        height: r.height() as f32,
    }
}

fn assert_tracks(sig: Signal<ElementBounds>, el: &web_sys::Element, what: &str) {
    let want = browser_rect(el);
    assert_ne!(want, ElementBounds::default(), "{what}: the browser's rect is not the zero rect");
    assert_eq!(sig.get(), want, "{what}: the signal holds the browser's rect");
}

type Target = Signal<Option<NodeHandle>>;

#[component]
fn scene(target: Target, filler: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            style: "position: fixed; left: 30px; top: 40px; width: 400px; margin: 0; padding: 0",
            if filler.get() {
                div { id: "filler-1781", style: "height: 37px" }
            }
            div {
                id: "scroller-1781",
                style: "height: 200px; overflow: auto",
                div { style: "height: 25px" }
                div {
                    id: "target-1781",
                    ref: target,
                    style: "width: 120px; height: 30px; margin-left: 7px",
                }
                div { style: "height: 1000px" }
            }
        }
    }
}

fn mount_scene() -> (Fixture, NodeHandle, Signal<bool>) {
    let cell: Rc<Cell<Option<(Target, Signal<bool>)>>> = Rc::new(Cell::new(None));
    let cell_in = cell.clone();
    let f = Fixture::mount(move |scope| {
        let target = Signal::new(None);
        let filler = Signal::new(false);
        cell_in.set(Some((target, filler)));
        scene(scope, target, filler)
    });
    let (target, filler) = cell.get().unwrap();
    (f, target.get().expect("ref filled at mount"), filler)
}

/// The first rect arrives a frame after the call, and each later move does:
/// a resize the page writes itself (no rinch write), a sibling inserted above
/// by rinch (moves it, same size), and a scroll of the box it is in.
#[wasm_bindgen_test]
async fn the_signal_follows_the_element() {
    let (f, handle, filler) = mount_scene();
    let el = f.el("target-1781");
    let sig = handle.bounds_signal();
    frames(2).await;
    assert_tracks(sig, &el, "first rect");
    assert_eq!(sig.get().width, 120.0);

    el.style().set_property("width", "155px").unwrap();
    frames(2).await;
    assert_tracks(sig, &el, "after a resize");
    assert_eq!(sig.get().width, 155.0);

    let y = sig.get().y;
    filler.set(true);
    frames(2).await;
    assert_tracks(sig, &el, "after a sibling went in above");
    assert_eq!(sig.get().y, y + 37.0, "moved down, same size");

    let y = sig.get().y;
    f.el("scroller-1781").set_scroll_top(11);
    frames(3).await;
    assert_tracks(sig, &el, "after a scroll");
    assert_eq!(sig.get().y, y - 11.0);
    f.teardown();
}

/// A size that changes with no DOM write at all — a running CSS animation —
/// reaches the signal: the element's own resize is observed.
#[wasm_bindgen_test]
async fn an_animated_size_reaches_the_signal() {
    let style = document().create_element("style").unwrap();
    style.set_text_content(Some(
        "@keyframes grow-1781 { from { width: 10px } to { width: 1010px } }",
    ));
    document().head().unwrap().append_child(&style).unwrap();
    let (f, handle, _) = mount_scene();
    let el = f.el("target-1781");
    let sig = handle.bounds_signal();
    frames(2).await;
    el.style()
        .set_property("animation", "grow-1781 20s linear forwards")
        .unwrap();
    frames(2).await;
    let first = sig.get().width;
    frames(8).await;
    let later = sig.get().width;
    assert!(
        later > first && later < 1010.0,
        "the width grew with the animation: {first} -> {later}"
    );
    style.remove();
    f.teardown();
}

/// An effect reading the signal runs again when the rect moves.
#[wasm_bindgen_test]
async fn an_effect_on_the_signal_reruns() {
    let (f, handle, _) = mount_scene();
    let el = f.el("target-1781");
    let sig = handle.bounds_signal();
    let widths = Rc::new(RefCell::new(Vec::new()));
    let w = widths.clone();
    let _e = Effect::new(move || w.borrow_mut().push(sig.get().width));
    frames(2).await;
    el.style().set_property("width", "64px").unwrap();
    frames(2).await;
    assert_eq!(*widths.borrow(), vec![0.0, 120.0, 64.0]);
    f.teardown();
}

/// Calls from one owner share one signal, so a caller asking per event does
/// not grow anything; the watch is dropped once no signal is left.
#[wasm_bindgen_test]
async fn calls_share_one_signal_and_the_watch_ends_with_it() {
    let (f, handle, _) = mount_scene();
    let scope = rinch_core::reactive::Scope::new();
    let (a, b) = scope.run(|| (handle.bounds_signal(), handle.bounds_signal()));
    assert_eq!(a.identity(), b.identity());
    frames(2).await;
    assert!(rinch_web::__bounds_watch_count() >= 1, "the element is watched");
    scope.dispose();
    // A DOM write wakes a refresh, which finds the signal gone.
    f.el("target-1781").style().set_property("width", "50px").unwrap();
    frames(2).await;
    assert_eq!(rinch_web::__bounds_watch_count(), 0, "nothing is watched any more");
    assert!(!rinch_web::__bounds_watch_installed(), "the observers are gone");
    f.teardown();
}
