//! `EditorHandle::blur` in a browser: an app with several editors takes the
//! keyboard away from one (a pane that stops holding a document) and leaves
//! nothing focused, without touching a keyboard owner that is not that editor.
//!
//! Run with a chromedriver matching the installed Chrome (and geckodriver for
//! Firefox, `docs/src/guide/wasm.md`):
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_blur
//! ```
//!
//! The desktop twin is `rinch/src/app/editor_blur_tests.rs`; the handle's part
//! is pinned in `rinch-editor-view`'s `handle::tests::reveal`.
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

const HOST_MARKER: &str = "data-test-host-editor-blur";

thread_local! {
    /// The root a fixture mounted, so the next fixture can unmount one a failed
    /// test left behind (a failed assertion never reaches its own `teardown`).
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

/// Paragraph `i` is `line i`: 6 characters, so it ends at `7 + 8i`.
fn end_of(i: usize) -> Pos {
    Pos(7 + 8 * i)
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    a: EditorHandle,
    b: EditorHandle,
    /// A text field outside the editors.
    input: web_sys::HtmlInputElement,
}

impl Fixture {
    /// A text field, then editors A and B. Nothing is focused, and each
    /// editor's caret is at the end of its first line.
    fn mount() -> Self {
        if let Some(stale) = LIVE_ROOT.with(|r| r.take()) {
            stale.unmount();
        }
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
        if let Some(active) = document()
            .active_element()
            .and_then(|a| a.dyn_into::<web_sys::HtmlElement>().ok())
        {
            active.blur().ok();
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        // Pinned to the top of the viewport: a fixture below the fold is not
        // laid out where a headless browser's small window can see it.
        host.set_attribute(
            "style",
            "position: fixed; top: 0; left: 0; width: 500px; font-family: monospace; \
             font-size: 16px; line-height: 24px; padding: 20px; background: white; z-index: 10;",
        )
        .unwrap();
        let input: web_sys::HtmlInputElement = document()
            .create_element("input")
            .unwrap()
            .dyn_into()
            .unwrap();
        host.append_child(&input).unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let a = editor();
        let b = editor();
        let (ma, mb) = (a.clone(), b.clone());
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let both = scope.create_element("div");
                both.append_child(&ma.mount(scope));
                both.append_child(&mb.mount(scope));
                both
            },
        );
        LIVE_ROOT.with(|r| r.set(Some(root)));
        a.set_selection(Selection::cursor(end_of(0)));
        b.set_selection(Selection::cursor(end_of(0)));
        Self {
            root,
            host,
            a,
            b,
            input,
        }
    }

    fn capture_has_focus(&self) -> bool {
        let active = document().active_element();
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .is_some_and(|ta| active.as_ref() == Some(&ta))
    }

    fn input_has_focus(&self) -> bool {
        document().active_element().as_deref() == Some(self.input.as_ref())
    }

    /// The first line of editor `which` (0 is A, 1 is B), as the page shows it.
    fn first_line(&self, which: u32) -> String {
        let editor = document()
            .query_selector_all("[data-pm-editor]")
            .unwrap()
            .item(which)
            .expect("the editor")
            .dyn_into::<web_sys::Element>()
            .unwrap();
        editor
            .query_selector("p")
            .unwrap()
            .expect("its first line")
            .text_content()
            .unwrap_or_default()
    }

    /// How many editors draw a caret.
    fn carets_visible(&self) -> usize {
        let carets = document()
            .query_selector_all("[data-pm-editor] [data-pm-caret]")
            .unwrap();
        (0..carets.length())
            .filter_map(|i| carets.item(i))
            .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
            .filter(|c| {
                c.get_attribute("style")
                    .is_some_and(|s| s.contains("visibility: visible"))
            })
            .count()
    }

    fn teardown(self) {
        LIVE_ROOT.with(|r| r.take());
        self.root.unmount();
        self.host.remove();
    }
}

fn editor() -> EditorHandle {
    let handle = create_editor();
    let html: String = (0..3).map(|i| format!("<p>line {i}</p>")).collect();
    assert!(handle.load_html(&html));
    handle
}

/// A printable `keydown` where the browser would send it: the focused element.
fn type_key(key: &str, code: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let target: web_sys::EventTarget = match document().active_element() {
        Some(el) => el.into(),
        None => document().body().unwrap().into(),
    };
    target.dispatch_event(&ev).unwrap();
}

fn focused_a(f: &Fixture) {
    f.a.focus();
    assert!(f.capture_has_focus(), "control: A has the keyboard");
    assert_eq!(f.carets_visible(), 1, "control: A's caret is drawn");
}

#[wasm_bindgen_test]
fn blur_leaves_nothing_focused_and_keys_reach_no_editor() {
    let f = Fixture::mount();
    focused_a(&f);

    f.a.blur();
    assert!(!f.capture_has_focus(), "the capture textarea let go");
    assert!(!f.input_has_focus(), "and nothing else took it");
    assert_eq!(f.carets_visible(), 0, "no caret is drawn");
    type_key("x", "KeyX");
    assert_eq!(f.first_line(0), "line 0", "the key did not reach A");
    assert_eq!(f.first_line(1), "line 0", "nor B");
    f.teardown();
}

#[wasm_bindgen_test]
fn blur_of_an_editor_without_the_keyboard_leaves_another_editor_focused() {
    let f = Fixture::mount();
    f.b.focus();
    assert!(f.capture_has_focus(), "control: B has the keyboard");

    f.a.blur();
    assert!(f.capture_has_focus(), "B keeps the keyboard");
    assert_eq!(f.carets_visible(), 1, "B's caret is still drawn");
    type_key("z", "KeyZ");
    assert_eq!(f.first_line(1), "line 0z", "the key went to B");
    assert_eq!(f.first_line(0), "line 0");
    f.teardown();
}

#[wasm_bindgen_test]
fn blur_of_an_editor_without_the_keyboard_leaves_a_text_field_focused() {
    let f = Fixture::mount();
    focused_a(&f);
    f.input.focus().unwrap();
    assert!(f.input_has_focus(), "control: the field took the keyboard");

    f.a.blur();
    assert!(f.input_has_focus(), "the field keeps it");
    f.teardown();
}

#[wasm_bindgen_test]
fn the_later_of_focus_and_blur_wins() {
    let f = Fixture::mount();
    f.b.focus();
    f.a.focus();
    f.a.blur();
    assert!(
        !f.capture_has_focus(),
        "focus() then blur(): nothing holds it"
    );
    assert_eq!(f.carets_visible(), 0);

    f.a.blur();
    f.a.focus();
    assert!(f.capture_has_focus(), "blur() then focus(): A holds it");
    type_key("z", "KeyZ");
    assert_eq!(f.first_line(0), "line 0z");
    f.teardown();
}

#[wasm_bindgen_test]
fn the_selection_survives_blur_and_focus_brings_it_back() {
    let f = Fixture::mount();
    let range = Selection::text(Pos(1), end_of(0));
    f.a.set_selection(range.clone());
    f.a.focus();
    assert!(f.capture_has_focus(), "control");

    f.a.blur();
    assert!(!f.capture_has_focus());
    assert_eq!(f.a.selection(), range, "blur keeps the selection");

    f.a.focus();
    assert_eq!(f.a.selection(), range, "and focus brings it back as it was");
    type_key("w", "KeyW");
    assert_eq!(f.first_line(0), "w", "typing replaces the kept selection");
    f.teardown();
}
