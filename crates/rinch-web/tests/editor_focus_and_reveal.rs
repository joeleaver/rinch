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
use rinch_web::{EditorHandle, RootHandle, ScrollAlign, create_editor};
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
        Self::mount_with(true)
    }

    /// The same, with `in_scroller: false` putting the editor straight in the
    /// page (the page is what scrolls).
    fn mount_with(in_scroller: bool) -> Self {
        Self::mount_styled(in_scroller, "")
    }

    /// [`Self::mount_with`], with `extra` appended to the scroller's style.
    fn mount_styled(in_scroller: bool, extra: &'static str) -> Self {
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
                if in_scroller {
                    scroller.set_attribute("data-reveal-scroller", "");
                    scroller.set_attribute(
                        "style",
                        &format!("height: 120px; width: 300px; overflow-y: auto; {extra}"),
                    );
                }
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

// ── scroll_into_view_aligned ─────────────────────────────────────────────

const THIRD: ScrollAlign = ScrollAlign::Fraction(1.0 / 3.0);

impl Fixture {
    /// How far below the scroller's top edge the caret line at `pos` sits.
    fn offset_in_view(&self, pos: Pos) -> f64 {
        let top = self.scroller().get_bounding_client_rect().top();
        self.line_at(pos).0 - top
    }

    fn max_scroll(&self) -> i32 {
        let s = self.scroller();
        s.scroll_height() - s.client_height()
    }
}

#[wasm_bindgen_test]
fn a_far_range_lands_a_third_of_the_way_down() {
    let f = Fixture::mount();
    let (from, to) = (start_of(30), end_of(31));
    f.handle.set_selection(Selection::text(from, to));
    assert!(!f.on_screen(from), "control: the range starts off screen");

    f.handle.scroll_into_view_aligned(from, to, THIRD);
    let at = f.offset_in_view(from);
    assert!(
        (at - 40.0).abs() <= 2.0,
        "the start sits a third of the way down the 120px view (got {at})"
    );
    assert!(f.on_screen(to), "and the end, which fits below it");
    assert_eq!(window().scroll_y().unwrap(), 0.0, "the page did not move");
    assert!(!f.capture_has_focus(), "nothing was focused");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_placed_range_above_the_view_lands_at_the_same_place() {
    let f = Fixture::mount();
    f.scroller().set_scroll_top(f.max_scroll());
    assert!(!f.on_screen(start_of(20)), "control: scrolled past it");
    f.handle
        .scroll_into_view_aligned(start_of(20), end_of(20), THIRD);
    let at = f.offset_in_view(start_of(20));
    assert!((at - 40.0).abs() <= 2.0, "got {at}");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_range_near_the_top_of_the_document_stays_near_the_top() {
    let f = Fixture::mount();
    f.scroller().set_scroll_top(700);
    f.handle
        .scroll_into_view_aligned(start_of(0), end_of(0), THIRD);
    assert_eq!(f.scroller().scroll_top(), 0, "clamped to the top");
    assert!(f.on_screen(start_of(0)));
    f.teardown();
}

#[wasm_bindgen_test]
fn a_range_near_the_end_of_the_document_stops_at_the_bottom() {
    let f = Fixture::mount();
    f.handle
        .scroll_into_view_aligned(start_of(39), end_of(39), THIRD);
    assert!(
        (f.scroller().scroll_top() - f.max_scroll()).abs() <= 1,
        "clamped to the end: {} of {}",
        f.scroller().scroll_top(),
        f.max_scroll()
    );
    assert!(f.on_screen(start_of(39)));
    assert!(f.offset_in_view(start_of(39)) > 40.0);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_placed_range_moves_even_when_it_is_in_view() {
    let f = Fixture::mount();
    let pos = start_of(2);
    assert!(f.on_screen(pos), "control");
    assert!(f.offset_in_view(pos) > 20.0, "control: below the margin");
    f.handle
        .scroll_into_view_aligned(pos, end_of(2), ScrollAlign::Fraction(0.0));
    let at = f.offset_in_view(pos);
    assert!(
        (at - 16.0).abs() <= 1.5,
        "Fraction(0.0) is the top edge plus the margin (got {at})"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_range_taller_than_the_space_below_still_puts_its_start_there() {
    let f = Fixture::mount();
    let (from, to) = (start_of(20), end_of(29));
    f.handle.scroll_into_view_aligned(from, to, THIRD);
    let at = f.offset_in_view(from);
    assert!((at - 40.0).abs() <= 2.0, "got {at}");
    assert!(!f.on_screen(to), "control: the rest runs off the bottom");
    f.teardown();
}

#[wasm_bindgen_test]
fn nearest_is_scroll_into_view() {
    let f = Fixture::mount();
    f.handle
        .scroll_into_view_aligned(start_of(0), end_of(0), ScrollAlign::Nearest);
    assert_eq!(
        f.scroller().scroll_top(),
        0,
        "a range in view moves nothing"
    );

    f.handle
        .scroll_into_view_aligned(start_of(30), end_of(31), ScrollAlign::Nearest);
    let bottom = f.scroller().get_bounding_client_rect().bottom();
    let (_, end_bottom) = f.line_at(end_of(31));
    assert!(
        (bottom - end_bottom - 16.0).abs() <= 1.5,
        "the end comes in at the bottom with the margin (got {})",
        bottom - end_bottom
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn with_no_scroller_the_page_is_placed() {
    let f = Fixture::mount_with(false);
    // Room below the document, so the page can scroll paragraph 35 up to a
    // third of the way down whatever the window's height.
    let room = document().create_element("div").unwrap();
    room.set_attribute("style", "height: 3000px").unwrap();
    f.host.append_child(&room).unwrap();
    let viewport = window().inner_height().unwrap().as_f64().unwrap();
    let pos = start_of(35);
    assert!(
        f.line_at(pos).0 > viewport,
        "control: paragraph 35 is below the fold of a {viewport}px window"
    );
    f.handle.scroll_into_view_aligned(pos, end_of(35), THIRD);
    let at = f.line_at(pos).0;
    assert!(
        (at - viewport / 3.0).abs() <= 2.0,
        "a third of the way down the window (got {at} of {viewport})"
    );
    assert!(window().scroll_y().unwrap() > 0.0);
    f.teardown();
}

/// The twin of desktop's `a_fraction_is_of_a_bordered_padded_scrollers_padding_box`:
/// `Fraction(f)` is `f` of the padding box (`clientHeight`), from the inside of
/// the top border (`clientTop`).
#[wasm_bindgen_test]
fn a_fraction_is_of_a_bordered_padded_scrollers_padding_box() {
    let f = Fixture::mount_styled(true, "border-top: 10px solid black; padding: 30px 0");
    let s = f.scroller();
    let padding_box = f64::from(s.client_height());
    f.handle
        .scroll_into_view_aligned(start_of(20), end_of(20), ScrollAlign::Fraction(0.5));
    let at = f.offset_in_view(start_of(20));
    let want = 10.0 + 0.5 * padding_box;
    assert!(
        (at - want).abs() <= 1.5,
        "half way down the padding box, below the border: want {want}, got {at}"
    );
    f.teardown();
}

/// A scroller below the page's fold: the scroller is placed, then the page
/// brought round to it the `nearest` way — which must not undo the placement
/// (#842: the web's `scrollIntoView` moves every scrollable ancestor).
#[wasm_bindgen_test]
fn a_placed_scroller_below_the_fold_is_placed_and_brought_on_screen() {
    let f = Fixture::mount();
    let spacer = document().create_element("div").unwrap();
    spacer.set_attribute("style", "height: 3000px").unwrap();
    f.host.prepend_with_node_1(&spacer).unwrap();
    let after = document().create_element("div").unwrap();
    after.set_attribute("style", "height: 3000px").unwrap();
    f.host.append_child(&after).unwrap();
    let viewport = window().inner_height().unwrap().as_f64().unwrap();
    assert!(
        f.scroller().get_bounding_client_rect().top() > viewport,
        "control: the scroller is below the fold"
    );
    f.handle
        .scroll_into_view_aligned(start_of(30), end_of(30), THIRD);
    let at = f.offset_in_view(start_of(30));
    assert!(
        (at - 40.0).abs() <= 2.0,
        "placed in the scroller (got {at})"
    );
    assert!(window().scroll_y().unwrap() > 0.0, "the page moved");
    let (top, bottom) = f.line_at(start_of(30));
    assert!(
        top >= 0.0 && bottom <= viewport,
        "and the line is in the window ({top}..{bottom} of {viewport})"
    );
    f.teardown();
}
