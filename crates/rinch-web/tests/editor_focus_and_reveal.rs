//! `EditorHandle::focus` and `EditorHandle::scroll_into_view` in a browser,
//! with no press: what an app does when it opens a note at a deep link —
//! select the quoted words, bring them on screen, give the editor the keyboard.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_focus_and_reveal
//! ```
//!
//! No fixture here presses anything before the call under test: the capture
//! textarea is not even created until something focuses an editor, so the
//! first assertion of every focus test is that `focus()` made it and gave it
//! the browser's focus. The handle's own rules are pinned natively in
//! `rinch-editor-view`'s `handle::tests::reveal`.
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
fn window() -> web_sys::Window {
    web_sys::window().unwrap()
}

const HOST_MARKER: &str = "data-test-host-focus-reveal";

thread_local! {
    /// The root a fixture mounted, so the next fixture can unmount one a failed
    /// test left behind (a failed assertion never reaches its own `teardown`).
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

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
    /// A text field outside the editor.
    input: web_sys::HtmlInputElement,
}

impl Fixture {
    /// 40 paragraphs in a 120px scroller, with a text field above it. Nothing
    /// is focused.
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
        window().scroll_to_with_x_and_y(0.0, 0.0);
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        let input: web_sys::HtmlInputElement = document()
            .create_element("input")
            .unwrap()
            .dyn_into()
            .unwrap();
        host.append_child(&input).unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        let html: String = (0..40).map(|i| format!("<p>line {i:03}</p>")).collect();
        assert!(handle.load_html(&html));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let scroller = scope.create_element("div");
                scroller.set_attribute("data-reveal-scroller", "");
                scroller.set_attribute("style", "height: 120px; width: 300px; overflow-y: auto");
                scroller.append_child(&mounted.mount(scope));
                scroller
            },
        );
        LIVE_ROOT.with(|r| r.set(Some(root)));
        Self {
            root,
            host,
            handle,
            input,
        }
    }

    fn scroller(&self) -> web_sys::Element {
        document()
            .query_selector("[data-reveal-scroller]")
            .unwrap()
            .expect("the scroller")
    }

    fn capture(&self) -> Option<web_sys::Element> {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
    }

    fn capture_has_focus(&self) -> bool {
        let active = document().active_element();
        self.capture()
            .is_some_and(|ta| active.as_ref() == Some(&ta))
    }

    fn para_text(&self, i: u32) -> String {
        document()
            .query_selector_all("[data-pm-editor] p")
            .unwrap()
            .item(i)
            .unwrap()
            .text_content()
            .unwrap_or_default()
    }

    /// The caret line at `pos`, `(top, bottom)` in client px.
    fn line_at(&self, pos: Pos) -> (f64, f64) {
        let r = self
            .handle
            .caret_rect(pos)
            .expect("the position has a caret");
        (r.y as f64, (r.y + r.height) as f64)
    }

    fn on_screen(&self, pos: Pos) -> bool {
        let s = self.scroller().get_bounding_client_rect();
        let (a, b) = self.line_at(pos);
        a >= s.top() - 0.5 && b <= s.bottom() + 0.5
    }

    fn caret_visible(&self) -> bool {
        document()
            .query_selector("[data-pm-editor] [data-pm-caret]")
            .unwrap()
            .and_then(|c| c.get_attribute("style"))
            .is_some_and(|s| s.contains("visibility: visible"))
    }

    fn teardown(self) {
        LIVE_ROOT.with(|r| r.take());
        self.root.unmount();
        self.host.remove();
        window().scroll_to_with_x_and_y(0.0, 0.0);
    }
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

// ── focus ────────────────────────────────────────────────────────────────

#[wasm_bindgen_test]
fn focus_gives_the_editor_the_keyboard_without_a_press() {
    let f = Fixture::mount();
    f.handle.set_selection(Selection::cursor(end_of(0)));
    type_key("x", "KeyX");
    assert_eq!(
        f.para_text(0),
        "line 000",
        "control: no editor had the keyboard"
    );

    f.handle.focus();
    assert!(
        f.capture_has_focus(),
        "the capture textarea has the browser's focus"
    );
    assert!(
        f.caret_visible(),
        "the caret is drawn where the selection is"
    );
    type_key("z", "KeyZ");
    assert_eq!(f.para_text(0), "line 000z", "the key went to the editor");
    f.teardown();
}

#[wasm_bindgen_test]
fn focus_takes_the_keyboard_from_a_text_field_and_moves_nothing() {
    let f = Fixture::mount();
    f.input.focus().unwrap();
    assert!(
        document().active_element().as_deref() == Some(f.input.as_ref()),
        "control: the field has focus"
    );
    let range = Selection::text(start_of(30), end_of(30));
    f.handle.set_selection(range.clone());
    f.handle.focus();
    assert!(f.capture_has_focus());
    assert_eq!(f.handle.selection(), range, "the selection is where it was");
    assert_eq!(f.scroller().scroll_top(), 0, "focus scrolls nothing");
    assert_eq!(window().scroll_y().unwrap(), 0.0, "nor the page");
    type_key("z", "KeyZ");
    assert_eq!(f.para_text(30), "z", "typing replaces the selected words");
    f.teardown();
}

// ── scroll_into_view ─────────────────────────────────────────────────────

#[wasm_bindgen_test]
fn a_far_range_comes_on_screen_without_focus_or_a_press() {
    let f = Fixture::mount();
    let (from, to) = (start_of(30), end_of(31));
    f.handle.set_selection(Selection::text(from, to));
    assert_eq!(
        f.scroller().scroll_top(),
        0,
        "control: selecting scrolls nothing"
    );
    assert!(!f.on_screen(from), "control: the range starts off screen");

    f.handle.scroll_into_view(from, to);
    assert!(
        f.scroller().scroll_top() > 0,
        "it scrolled, with no input event"
    );
    assert!(f.on_screen(from), "the start is on screen");
    assert!(f.on_screen(to), "and the end, since both fit");
    let bottom = f.scroller().get_bounding_client_rect().bottom();
    let (_, end_bottom) = f.line_at(to);
    assert!(
        bottom - end_bottom >= 15.0,
        "with the margin below it (got {})",
        bottom - end_bottom
    );
    assert!(!f.capture_has_focus(), "nothing was focused");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_caret_position_comes_on_screen() {
    let f = Fixture::mount();
    f.handle.scroll_into_view(start_of(35), start_of(35));
    assert!(f.scroller().scroll_top() > 0);
    assert!(f.on_screen(start_of(35)));
    f.teardown();
}

#[wasm_bindgen_test]
fn a_range_above_the_view_comes_in_at_the_top_with_the_margin() {
    let f = Fixture::mount();
    f.scroller().set_scroll_top(700);
    assert!(!f.on_screen(start_of(3)), "control: scrolled past it");
    f.handle.scroll_into_view(start_of(3), end_of(3));
    let top = f.scroller().get_bounding_client_rect().top();
    let (line_top, _) = f.line_at(start_of(3));
    assert!(f.on_screen(start_of(3)));
    assert!(
        (line_top - top - 16.0).abs() <= 1.5,
        "the start sits 16px under the top edge (got {})",
        line_top - top
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_range_taller_than_the_view_shows_its_start_from_either_side() {
    let f = Fixture::mount();
    let (from, to) = (start_of(20), end_of(29));
    f.handle.scroll_into_view(from, to);
    assert!(f.on_screen(from), "from above: the start is shown");
    assert!(!f.on_screen(to), "control: the range does not fit");

    f.scroller().set_scroll_top(5000);
    assert!(!f.on_screen(from), "control: scrolled past the range");
    f.handle.scroll_into_view(from, to);
    assert!(f.on_screen(from), "from below: the start is shown");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_range_already_in_view_moves_nothing() {
    let f = Fixture::mount();
    // Paragraph 0, margin included, is well inside the 120px scroller.
    assert!(f.on_screen(end_of(0)), "control");
    f.handle.scroll_into_view(start_of(0), end_of(0));
    assert_eq!(f.scroller().scroll_top(), 0);
    assert_eq!(window().scroll_y().unwrap(), 0.0);
    f.teardown();
}

/// The whole deep-link sequence: select the words, reveal them, focus, type.
#[wasm_bindgen_test]
fn select_reveal_focus_then_typing_replaces_the_words() {
    let f = Fixture::mount();
    let (from, to) = (Pos(start_of(33).0 + 5), end_of(33));
    f.handle.set_selection(Selection::text(from, to));
    f.handle.scroll_into_view(from, to);
    f.handle.focus();
    assert!(f.on_screen(from));
    assert!(f.capture_has_focus());
    type_key("z", "KeyZ");
    assert_eq!(f.para_text(33), "line z");
    f.teardown();
}
