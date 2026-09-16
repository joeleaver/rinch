//! Browser-driven tests for the editor's right-click menu on the web (issue #814).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The editor's surface is deliberately not `contenteditable`, so the browser's
//! own `contextmenu` hit test used to find a plain element there and build the
//! menu for one: no Paste, no Cut. rinch draws no menu of its own on the web
//! (a page's paste needs `navigator.clipboard.readText()`, which prompts), so
//! the fix is CodeMirror 5's: at the right-button press the hidden capture
//! `<textarea>` is **parked under the pointer**, the browser's hit test finds an
//! editable, and the menu it builds is the editing one. Its Paste / Cut / Copy
//! then fire the ordinary clipboard events on the (still focused) textarea,
//! which the editor already answers from the model.
//!
//! **What can and cannot be measured here.** Chrome's menu is not in the DOM,
//! so no fixture sees it. What a fixture can see is everything the menu is
//! built from: `document.elementFromPoint` at the press point at `contextmenu`
//! time (Chrome hit-tests there to choose the menu), the textarea's own
//! selection (Copy and Cut are offered only while the field has one), and the
//! clipboard / `beforeinput` events the menu's items fire, which are dispatched
//! here synthetically at the element the real ones target. A real right press
//! through chromedriver's Actions API was measured separately (see the PR)
//! to confirm that Chrome's own `contextmenu` dispatch targets the parked
//! textarea.
//!
//! Every fixture mounts a real editor through `rinch_web::mount_into`, focuses
//! it with a genuine `mousedown`, and checks that the capture textarea exists
//! and holds focus before asserting anything — a fixture that dispatched events
//! into an unmounted page would exercise no rinch listener at all.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-test-host-814";

/// The two-paragraph document every fixture edits, and its positions:
/// paragraph 1 opens at 0 and its text runs 1..=16, paragraph 2 opens at 17
/// and its text starts at 18. "world" is 7..12, "Second" is 18..24.
const CONTENT: &str = "<p>Hello world one</p><p>Second paragraph here</p>";
const WORLD: (usize, usize) = (7, 12);

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
    /// Dispatches of the `data-oncontextmenu` handler the app-menu fixture wraps
    /// the editor in (0 for every other fixture).
    app_menu: Rc<Cell<u32>>,
}

impl Fixture {
    fn mount() -> Self {
        Self::mount_with(false)
    }

    /// `app_menu`: wrap the editor in a `<div>` carrying a live
    /// `data-oncontextmenu` handler.
    fn mount_with(app_menu: bool) -> Self {
        rinch_web::set_suppress_native_context_menu(false);
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
        // A fixed font so the char geometry the fixtures click on is stable.
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(CONTENT));
        let count = Rc::new(Cell::new(0u32));
        let counter = count.clone();
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let editor = mounted.mount(scope);
                if app_menu {
                    let wrapper = scope.create_element("div");
                    wrapper.set_attribute("id", "app-menu");
                    let id = scope.register_handler(move || counter.set(counter.get() + 1));
                    wrapper.set_attribute("data-oncontextmenu", &id.0.to_string());
                    wrapper.append_child(&editor);
                    wrapper
                } else {
                    editor
                }
            },
        );
        Self {
            root,
            host,
            handle,
            app_menu: count,
        }
    }

    /// The editor's paragraphs.
    fn paragraph(&self, i: u32) -> web_sys::Element {
        document()
            .query_selector_all("[data-pm-editor] p")
            .unwrap()
            .item(i)
            .unwrap_or_else(|| panic!("no paragraph {i}"))
            .dyn_into()
            .unwrap()
    }

    /// The viewport centre of character `ch` of paragraph `p`.
    fn point(&self, p: u32, ch: u32) -> (f32, f32) {
        let text = self
            .paragraph(p)
            .first_child()
            .expect("the paragraph's text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, ch).unwrap();
        range.set_end(&text, ch + 1).unwrap();
        let r = range.get_bounding_client_rect();
        (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        )
    }

    /// The editor's text, block by block — the view projects the model, so
    /// this is the document.
    fn text(&self) -> String {
        let ps = document().query_selector_all("[data-pm-editor] p").unwrap();
        (0..ps.length())
            .map(|i| ps.item(i).unwrap().text_content().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|")
    }

    /// The hidden capture textarea (created on the first editor focus).
    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// Focus the editor with a genuine left press at `(x, y)` and prove the
    /// capture target exists and holds focus — the positive control every
    /// fixture rests on.
    fn focus_at(&self, (x, y): (f32, f32)) {
        mouse("mousedown", x, y, 0);
        mouse("mouseup", x, y, 0);
        let ta = self.capture();
        assert!(
            document().active_element().as_deref() == Some(ta.as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
    }

    /// The browser's right-press sequence at `(x, y)`: `mousedown` (button 2),
    /// then the `contextmenu` it fires at whatever is under the pointer **by
    /// then** — the hit test Chrome's own dispatch performs after the press
    /// handlers ran — then `mouseup`. Returns the `contextmenu` event, for its
    /// `defaultPrevented`, and the tag it targeted.
    fn right_press(&self, (x, y): (f32, f32)) -> (web_sys::MouseEvent, String) {
        mouse("mousedown", x, y, 2);
        let target = under(x, y);
        let tag = target.tag_name();
        let ev = mouse_on(&target, "contextmenu", x, y, 2);
        mouse("mouseup", x, y, 2);
        (ev, tag)
    }

    fn teardown(self) {
        rinch_web::set_suppress_native_context_menu(false);
        self.root.unmount();
        self.host.remove();
    }
}

/// `document.elementFromPoint` — the browser's own answer to "what is under the
/// pointer", which is what its `contextmenu` hit test asks.
fn under(x: f32, y: f32) -> web_sys::Element {
    document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"))
}

fn is_capture(el: &web_sys::Element) -> bool {
    el.has_attribute("data-pm-capture")
}

/// Dispatch a mouse event at whatever is under `(x, y)`, as the browser would.
fn mouse(name: &str, x: f32, y: f32, button: i16) -> web_sys::MouseEvent {
    let target = under(x, y);
    mouse_on(&target, name, x, y, button)
}

fn mouse_on(
    target: &web_sys::Element,
    name: &str,
    x: f32,
    y: f32,
    button: i16,
) -> web_sys::MouseEvent {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(button);
    init.set_buttons(if button == 2 { 2 } else { 1 });
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
    ev
}

/// A `copy` / `cut` / `paste` on `el` carrying a fresh `DataTransfer`, as the
/// menu's items fire it on the focused editable.
fn clipboard(el: &web_sys::Element, name: &str, plain: Option<&str>) -> web_sys::DataTransfer {
    let dt = web_sys::DataTransfer::new().unwrap();
    if let Some(text) = plain {
        dt.set_data("text/plain", text).unwrap();
    }
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(&dt));
    let ev = web_sys::ClipboardEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    dt
}

fn keydown(el: &web_sys::Element, key: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(key);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

fn composition(el: &web_sys::Element, name: &str, data: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_data(data);
    let ev = web_sys::CompositionEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// Resolve after `ms` — the timers the menu cycle runs on are real timers.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn selection_span(ta: &web_sys::HtmlTextAreaElement) -> (u32, u32) {
    (
        ta.selection_start().unwrap().unwrap(),
        ta.selection_end().unwrap().unwrap(),
    )
}

// ── Positioning ─────────────────────────────────────────────────────────────

/// At the right press the capture textarea goes under the pointer, so the
/// browser's `contextmenu` hit test — `elementFromPoint` at the press point —
/// finds an editable. Kills: a parked style that keeps `pointer-events: none`
/// (the textarea is under the point and still not hit), or one that does not
/// move it at all.
#[wasm_bindgen_test]
fn a_right_press_parks_the_capture_textarea_under_the_pointer() {
    let f = Fixture::mount();
    let at = f.point(0, 8);
    f.focus_at(at);
    assert!(
        !is_capture(&under(at.0, at.1)),
        "positive control: before the right press the text is under the pointer"
    );

    mouse("mousedown", at.0, at.1, 2);

    let hit = under(at.0, at.1);
    assert!(
        is_capture(&hit),
        "the browser's hit test at the press point must find the capture textarea, found <{}>",
        hit.tag_name()
    );
    let r = f.capture().get_bounding_client_rect();
    assert!(
        r.x() <= at.0 as f64
            && at.0 as f64 <= r.x() + r.width()
            && r.y() <= at.1 as f64
            && at.1 as f64 <= r.y() + r.height(),
        "the textarea's box must contain the press point"
    );
    assert!(
        document().active_element().as_deref() == Some(f.capture().as_ref()),
        "the parked textarea still holds focus — the menu's items fire on the focused editable"
    );
    f.teardown();
}

/// Once the browser has dispatched `contextmenu` at the parked textarea its
/// hit test is done, and the textarea goes back off-screen so the next left
/// press finds the editor's text again. Kills: never scheduling the unpark.
#[wasm_bindgen_test]
async fn the_textarea_goes_back_off_screen_after_the_contextmenu_dispatch() {
    let f = Fixture::mount();
    let at = f.point(1, 4);
    f.focus_at(at);
    let (ev, tag) = f.right_press(at);
    assert_eq!(
        tag, "TEXTAREA",
        "the contextmenu dispatch targets the parked textarea"
    );
    assert!(
        !ev.default_prevented(),
        "with the flag off nothing rinch does may suppress the browser's editing menu"
    );

    sleep(150).await;

    let hit = under(at.0, at.1);
    assert!(
        !is_capture(&hit),
        "after the dispatch the press point must resolve to the editor again, found <{}>",
        hit.tag_name()
    );
    let r = f.capture().get_bounding_client_rect();
    assert!(
        r.width() <= 1.0 && r.height() <= 1.0 && r.x() < 0.5 && r.y() < 0.5,
        "the textarea is back in its off-screen 1px box, got {}x{} at ({}, {})",
        r.width(),
        r.height(),
        r.x(),
        r.y()
    );
    f.teardown();
}

/// On Linux and macOS the menu opens at the press, and the release usually
/// goes to the menu window rather than the page — so the `contextmenu`
/// dispatch alone, with no `mouseup` ever seen, must be enough to send the
/// textarea back. Kills: unparking only from the right-button release.
#[wasm_bindgen_test]
async fn a_held_right_press_still_unparks_after_the_contextmenu_dispatch() {
    let f = Fixture::mount();
    let at = f.point(0, 5);
    f.focus_at(at);
    mouse("mousedown", at.0, at.1, 2);
    let target = under(at.0, at.1);
    assert!(is_capture(&target), "positive control: parked at the press");
    mouse_on(&target, "contextmenu", at.0, at.1, 2);
    // No mouseup: the menu has the pointer.

    sleep(150).await;

    let hit = under(at.0, at.1);
    assert!(
        !is_capture(&hit),
        "the contextmenu dispatch alone unparks the textarea, found <{}>",
        hit.tag_name()
    );
    f.teardown();
}

/// A left press that lands on the still-parked textarea (a fast second click
/// inside the unpark window) is the editor's click: it places the caret where
/// the pointer is rather than blurring the editor for "another text field".
/// Kills: treating a press on the capture textarea like a press on any textarea.
#[wasm_bindgen_test]
fn a_left_press_on_the_parked_textarea_still_places_the_caret() {
    let f = Fixture::mount();
    let first = f.point(0, 2);
    f.focus_at(first);
    mouse("mousedown", first.0, first.1, 2);
    assert!(is_capture(&under(first.0, first.1)), "parked");

    // The textarea is 30px square around the press; a point inside that box
    // but on a different character.
    let second = f.point(0, 3);
    let hit = under(second.0, second.1);
    assert!(
        is_capture(&hit),
        "positive control: the second press lands on the parked textarea"
    );
    mouse_on(&hit, "mousedown", second.0, second.1, 0);

    assert!(
        document().active_element().as_deref() == Some(f.capture().as_ref()),
        "the editor keeps focus"
    );
    let sel = f.handle.selection();
    assert!(
        sel.is_empty(),
        "a plain left press places a caret, got {sel:?}"
    );
    // The centre of character 3 resolves to the boundary either side of it.
    assert!(
        (4..=5).contains(&sel.head().0),
        "the caret lands at the second press's character, got {sel:?}"
    );
    assert!(
        !is_capture(&under(second.0, second.1)),
        "and the textarea is unparked"
    );
    f.teardown();
}

// ── The right-press caret rule ──────────────────────────────────────────────

/// A right press inside the selection keeps it (that is what the menu's Cut and
/// Copy act on); outside it moves the caret to the press point. Kills: a
/// right press that always places a caret (what `main` did) or never does.
#[wasm_bindgen_test]
fn a_right_press_keeps_a_selection_it_lands_in_and_moves_the_caret_otherwise() {
    let f = Fixture::mount();
    f.focus_at(f.point(0, 1));
    f.handle
        .set_selection(Selection::text(Pos(WORLD.0), Pos(WORLD.1)));
    assert_eq!(
        f.handle.selection_clipboard().map(|(_, t)| t).as_deref(),
        Some("world"),
        "positive control: the selection is the word"
    );

    let inside = f.point(0, 8); // the 'r' of "world"
    f.right_press(inside);
    assert_eq!(
        f.handle.selection(),
        Selection::text(Pos(WORLD.0), Pos(WORLD.1)),
        "a right press inside the selection keeps it"
    );

    let outside = f.point(1, 3); // inside "Second"
    f.right_press(outside);
    let sel = f.handle.selection();
    assert!(
        sel.is_empty(),
        "a right press outside the selection collapses it, got {sel:?}"
    );
    assert!(
        (21..=22).contains(&sel.head().0),
        "and moves the caret to the press point (either side of character 3), got {sel:?}"
    );
    f.teardown();
}

// ── Cut / Copy ──────────────────────────────────────────────────────────────

/// Copy and Cut are offered only while the field under the menu has a
/// selection, and the mirror holds one textblock — so a selection spanning
/// two blocks used to be mirrored collapsed and the menu greyed both out.
/// Parked, the textarea carries a selection whenever the editor does, and the
/// `copy` / `cut` the menu fires are answered from the editor's selection,
/// both blocks of it. Kills: parking with a collapsed selection; answering
/// `copy` from the textarea's own contents.
#[wasm_bindgen_test]
fn cut_and_copy_from_the_menu_act_on_the_editors_selection_across_blocks() {
    let f = Fixture::mount();
    f.focus_at(f.point(0, 1));
    // "world one" + "Second"
    f.handle
        .set_selection(Selection::text(Pos(WORLD.0), Pos(24)));
    let expected = "world one\nSecond";
    assert_eq!(
        f.handle.selection_clipboard().map(|(_, t)| t).as_deref(),
        Some(expected),
        "positive control: the model selection crosses the block boundary"
    );

    let at = f.point(0, 8);
    f.right_press(at);
    let ta = f.capture();
    let (start, end) = selection_span(&ta);
    assert!(
        end > start,
        "the field under the menu must carry a selection or Copy/Cut are greyed out, got {start}..{end}"
    );

    let dt = clipboard(&ta, "copy", None);
    assert_eq!(
        dt.get_data("text/plain").unwrap(),
        expected,
        "Copy puts the editor's selection on the clipboard, not the textarea's"
    );
    assert_eq!(
        f.text(),
        "Hello world one|Second paragraph here",
        "Copy edits nothing"
    );

    let dt = clipboard(&ta, "cut", None);
    assert_eq!(dt.get_data("text/plain").unwrap(), expected);
    assert_eq!(
        f.text(),
        "Hello  paragraph here",
        "Cut deletes the editor's selection"
    );
    assert!(f.handle.selection().is_empty());
    f.teardown();
}

// ── Paste ───────────────────────────────────────────────────────────────────

/// The menu's Paste fires `paste` on the focused textarea and lands at the
/// editor's selection, replacing it; afterwards the textarea mirrors the
/// caret's block again rather than the menu-cycle sentinel. Kills: not
/// ending the cycle on `paste` (the mirror stays stale).
#[wasm_bindgen_test]
async fn paste_from_the_menu_replaces_the_editors_selection() {
    let f = Fixture::mount();
    f.focus_at(f.point(0, 1));
    f.handle
        .set_selection(Selection::text(Pos(WORLD.0), Pos(WORLD.1)));

    let at = f.point(0, 8);
    f.right_press(at);
    assert!(
        is_capture(&under(at.0, at.1)),
        "positive control: the menu was built for the textarea"
    );

    let ta = f.capture();
    clipboard(&ta, "paste", Some("planet"));
    assert_eq!(f.text(), "Hello planet one|Second paragraph here");

    sleep(150).await;
    assert_eq!(
        ta.value(),
        "Hello planet one",
        "after the cycle the textarea mirrors the caret's block again"
    );
    let (start, end) = selection_span(&ta);
    assert_eq!(
        (start, end),
        (12, 12),
        "with the caret mirrored after the pasted word"
    );
    f.teardown();
}

// ── Select All ──────────────────────────────────────────────────────────────

/// The browser's Select All selects the textarea's contents, which is one
/// block at most. Parked, the field carries a sentinel the editor's own writes
/// never select from offset 0, so a selection that spans the whole field
/// afterwards can only be the menu's Select All — and it becomes the editor's.
/// Kills: no sentinel (a same-block selection from the block start reads as
/// Select All), or reading the span at the wrong end.
#[wasm_bindgen_test]
async fn select_all_from_the_menu_selects_the_whole_document() {
    let f = Fixture::mount();
    f.focus_at(f.point(1, 3));
    let at = f.point(1, 3);
    f.right_press(at);
    let ta = f.capture();
    assert!(
        document().active_element().as_deref() == Some(ta.as_ref()),
        "positive control: the menu's Select All acts on the focused textarea"
    );

    // What the menu's item does: select the focused field's whole contents.
    ta.select();

    sleep(600).await;
    let sel = f.handle.selection();
    assert_eq!(
        (sel.from(), sel.to()),
        (Pos(1), Pos(39)),
        "the editor's selection spans the whole document, got {sel:?}"
    );
    f.teardown();
}

/// A menu dismissed with nothing chosen leaves the editor as it was: the
/// selection unchanged, and the textarea back to mirroring the caret's block
/// with no sentinel in it. Kills: translating Select All without checking the
/// span; not restoring the mirror when the cycle ends.
#[wasm_bindgen_test]
fn a_dismissed_menu_changes_nothing() {
    let f = Fixture::mount();
    f.focus_at(f.point(0, 1));
    f.handle
        .set_selection(Selection::text(Pos(WORLD.0), Pos(WORLD.1)));
    let at = f.point(0, 9);
    f.right_press(at);
    let ta = f.capture();
    assert!(
        ta.value().starts_with('\u{200b}'),
        "positive control: the sentinel is in the field"
    );

    // Escape closes the menu; the next key reaches the page.
    keydown(&ta, "Escape");

    assert_eq!(
        f.handle.selection(),
        Selection::text(Pos(WORLD.0), Pos(WORLD.1))
    );
    assert_eq!(
        ta.value(),
        "Hello world one",
        "the mirror is back, sentinel gone"
    );
    assert_eq!(selection_span(&ta), (6, 11), "with the selection mirrored");
    assert!(!is_capture(&under(at.0, at.1)), "and the textarea unparked");
    f.teardown();
}

// ── The mirror survives ─────────────────────────────────────────────────────

/// A soft keyboard and an IME edit the textarea and are reconciled by diffing
/// it against what the editor last wrote there. After a menu cycle that
/// mirror must be the caret's block again, or a composition commit would be
/// diffed against the sentinel. Kills: ending the cycle without re-syncing.
#[wasm_bindgen_test]
fn an_ime_composition_after_a_menu_cycle_still_lands_in_the_document() {
    let f = Fixture::mount();
    let at = f.point(0, 7);
    f.focus_at(at);
    f.right_press(at);
    // The caret after "Hello", set while the cycle is still live: ending it
    // must mirror where the caret IS, not where the press put it.
    f.handle.set_selection(Selection::cursor(Pos(6)));
    keydown(&f.capture(), "Escape");

    let ta = f.capture();
    assert_eq!(
        ta.value(),
        "Hello world one",
        "positive control: the mirror is the caret's block"
    );
    let caret = selection_span(&ta).0;
    assert_eq!(
        caret, 5,
        "positive control: the caret is mirrored after \"Hello\""
    );
    // The IME's commit: it rewrites the field, then `compositionend` says what it composed.
    composition(&ta, "compositionstart", "");
    let mut value = ta.value();
    value.insert_str(caret as usize, "ö");
    ta.set_value(&value);
    composition(&ta, "compositionend", "ö");

    assert_eq!(f.text(), "Helloö world one|Second paragraph here");
    f.teardown();
}

/// A `beforeinput` from a soft keyboard after a menu cycle inserts at the
/// editor's caret, and the mirror follows.
#[wasm_bindgen_test]
fn soft_keyboard_typing_after_a_menu_cycle_still_lands_in_the_document() {
    let f = Fixture::mount();
    let at = f.point(1, 6); // after "Second"
    f.focus_at(at);
    f.right_press(at);
    keydown(&f.capture(), "Escape");

    let ta = f.capture();
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type("insertText");
    init.set_data(Some("!"));
    let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    ta.dispatch_event(&ev).unwrap();

    assert_eq!(f.text(), "Hello world one|Second! paragraph here");
    assert_eq!(
        ta.value(),
        "Second! paragraph here",
        "the mirror follows the edit"
    );
    f.teardown();
}

// ── An app's own menu wins ──────────────────────────────────────────────────

/// An editor inside an element carrying a live `data-oncontextmenu` gets the
/// app's menu, as every other element under that handler does: the textarea
/// stays where it is, the `contextmenu` reaches the handler and is prevented.
/// Kills: parking regardless of an app handler.
#[wasm_bindgen_test]
fn an_apps_context_menu_handler_wins_over_the_editing_menu() {
    let f = Fixture::mount_with(true);
    let at = f.point(0, 8);
    f.focus_at(at);
    let (ev, tag) = f.right_press(at);

    assert_ne!(
        tag, "TEXTAREA",
        "the textarea is not parked under an app handler"
    );
    assert!(
        ev.default_prevented(),
        "the app's handler suppresses the browser's menu"
    );
    assert_eq!(f.app_menu.get(), 1, "and is dispatched");
    f.teardown();
}

// ── Touch ───────────────────────────────────────────────────────────────────

/// The right-press path must not disturb the touch tap-to-focus path: a touch
/// contact that goes down and comes up still focuses the editor.
#[wasm_bindgen_test]
fn a_touch_tap_still_focuses_the_editor() {
    let f = Fixture::mount();
    let at = f.point(1, 2);
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_type("touch");
    init.set_pointer_id(7);
    init.set_is_primary(true);
    init.set_client_x(at.0 as i32);
    init.set_client_y(at.1 as i32);
    let target = under(at.0, at.1);
    let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    target.dispatch_event(&down).unwrap();
    let up = web_sys::PointerEvent::new_with_event_init_dict("pointerup", &init).unwrap();
    target.dispatch_event(&up).unwrap();

    let ta = f.capture();
    assert!(
        document().active_element().as_deref() == Some(ta.as_ref()),
        "a tap focuses the capture textarea"
    );
    let head = f.handle.selection().head().0;
    assert!(
        (20..=21).contains(&head),
        "the caret lands at the tap, got {head}"
    );
    f.teardown();
}
