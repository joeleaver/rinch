//! A selection change made by app code — `EditorHandle::set_selection` or
//! `command("selectAll")` from a timer, an effect, a toolbar button — draws the
//! caret and the selection highlight in a browser, with no editor input event
//! to refresh them (#1001). Desktop's twin is
//! `rinch/src/app/editor_programmatic_selection_tests.rs`.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_programmatic_selection
//! ```
//!
//! Each fixture waits one macrotask after the call, which is where a change
//! made from a timer is next observable to anyone.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-programmatic-selection";

/// Paragraph `i` is `line NNN`: 8 characters, spanning `1 + 10i` to `9 + 10i`.
fn start_of(i: usize) -> Pos {
    Pos(1 + 10 * i)
}
fn end_of(i: usize) -> Pos {
    Pos(9 + 10 * i)
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// Six paragraphs, the editor focused through `focus()` (no press).
    fn mount() -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        rinch_web::__reset_activation_state();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        let html: String = (0..6).map(|i| format!("<p>line {i:03}</p>")).collect();
        assert!(handle.load_html(&html));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrap = scope.create_element("div");
                wrap.set_attribute("style", "width: 300px");
                wrap.append_child(&mounted.mount(scope));
                wrap
            },
        );
        handle.focus();
        Self { root, host, handle }
    }

    /// The drawn selection-highlight rects, as `(top, height)` in client px.
    fn highlights(&self) -> Vec<(f64, f64)> {
        let list = document()
            .query_selector_all("[data-pm-editor] [data-pm-selection]")
            .unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
            .map(|el| el.get_bounding_client_rect())
            .filter(|r| r.width() > 0.0 && r.height() > 0.0)
            .map(|r| (r.top(), r.height()))
            .collect()
    }

    /// The caret overlay's top-left in client px, if it is shown.
    fn caret(&self) -> Option<(f64, f64)> {
        let el = document()
            .query_selector("[data-pm-editor] [data-pm-caret]")
            .unwrap()?;
        let style = el.get_attribute("style").unwrap_or_default();
        if !style.contains("visibility: visible") {
            return None;
        }
        let r = el.get_bounding_client_rect();
        Some((r.left(), r.top()))
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

async fn next_task() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), 0)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
async fn a_range_set_from_app_code_draws_its_highlight() {
    let f = Fixture::mount();
    next_task().await;
    assert!(f.highlights().is_empty(), "control: no range yet");

    f.handle
        .set_selection(Selection::text(start_of(2), end_of(2)));
    next_task().await;

    let rects = f.highlights();
    assert_eq!(rects.len(), 1, "one line selected: {rects:?}");
    let line = f.handle.caret_rect(start_of(2)).expect("a caret line");
    assert!(
        (rects[0].0 - line.y as f64).abs() < 3.0,
        "the highlight sits on the third line: {rects:?} vs {line:?}"
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn a_caret_set_from_app_code_is_drawn_where_it_now_is() {
    let f = Fixture::mount();
    next_task().await;
    let target = Pos(start_of(4).0 + 3);
    f.handle.set_selection(Selection::cursor(target));
    next_task().await;

    let want = f.handle.caret_rect(target).expect("a caret line");
    let (x, y) = f.caret().expect("the caret is shown");
    assert!(
        (x - want.x as f64).abs() < 3.0 && (y - want.y as f64).abs() < 3.0,
        "the caret is drawn at ({x}, {y}), the selection puts it at {want:?}"
    );
    f.teardown();
}

#[wasm_bindgen_test]
async fn select_all_from_app_code_highlights_every_line() {
    let f = Fixture::mount();
    next_task().await;
    assert!(f.handle.command("selectAll"), "positive control");
    next_task().await;
    assert_eq!(f.highlights().len(), 6, "{:?}", f.highlights());
    f.teardown();
}
