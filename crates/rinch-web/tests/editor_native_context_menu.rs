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

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
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

/// A link and an image, for the link / image rule. Paragraph 1's text runs
/// 1..21 — "Hello " 1..7, the link's "link text" 7..16, " here" 16..21 — and
/// paragraph 2 opens at 22: "img " is 23..27, the image is the node at 27, " end"
/// follows from 28.
const LINK_IMAGE: &str = "<p>Hello <a href=\"https://example.com/x\">link text</a> here</p>\
    <p>img <img src=\"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAH0lEQVR4nGM4ISdHFcQwatCoQaMGjRo0atCoQQNvEADGtUkfvb3KfwAAAABJRU5ErkJggg==\"> end</p>";
const LINK: (usize, usize) = (7, 16);
const IMAGE_AT: usize = 27;

/// A horizontal rule between two paragraphs — a leaf with no caret position of
/// its own. The rule is the node at 17; paragraph 2 opens at 18.
const RULE: &str = "<p>Hello world one</p><hr><p>Second paragraph here</p>";
const RULE_AT: usize = 17;

/// The zero-width space the parked field starts and ends with.
const SENTINEL: char = '\u{200b}';

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
        Self::mount_content(CONTENT, app_menu)
    }

    /// Mount an editor holding `content`, optionally under an app menu handler.
    fn mount_content(content: &str, app_menu: bool) -> Self {
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
        assert!(handle.load_html(content));
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

    /// The viewport centre of character `ch` of paragraph `p`, counted across the
    /// paragraph's text nodes (a link's text is a node of its own).
    fn point(&self, p: u32, ch: u32) -> (f32, f32) {
        let (text, off) = text_at(&self.paragraph(p), ch);
        let range = document().create_range().unwrap();
        range.set_start(&text, off).unwrap();
        range.set_end(&text, off + 1).unwrap();
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
    mouse_with_ctrl(target, name, x, y, button, false)
}

fn mouse_with_ctrl(
    target: &web_sys::Element,
    name: &str,
    x: f32,
    y: f32,
    button: i16,
    ctrl: bool,
) -> web_sys::MouseEvent {
    let init = web_sys::MouseEventInit::new();
    init.set_ctrl_key(ctrl);
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

/// A `keydown` / `keyup` of `key` on `el`, with Shift when `shift`.
fn key_event(el: &web_sys::Element, name: &str, key: &str, shift: bool) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(key);
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

/// The text node holding character `ch` of `el`'s text, and `ch`'s offset in it.
fn text_at(el: &web_sys::Element, ch: u32) -> (web_sys::Node, u32) {
    fn walk(node: &web_sys::Node, ch: u32, seen: &mut u32) -> Option<(web_sys::Node, u32)> {
        let kids = node.child_nodes();
        for i in 0..kids.length() {
            let kid = kids.item(i)?;
            if kid.node_type() == web_sys::Node::TEXT_NODE {
                let len = kid
                    .text_content()
                    .unwrap_or_default()
                    .encode_utf16()
                    .count() as u32;
                if ch < *seen + len {
                    return Some((kid, ch - *seen));
                }
                *seen += len;
            } else if let Some(found) = walk(&kid, ch, seen) {
                return Some(found);
            }
        }
        None
    }
    walk(el, ch, &mut 0).unwrap_or_else(|| panic!("no character {ch} in <{}>", el.tag_name()))
}

/// The viewport x of the caret boundary before character `ch` of `el`'s text.
fn caret_x(el: &web_sys::Element, ch: u32) -> f32 {
    let (text, off) = text_at(el, ch);
    let range = document().create_range().unwrap();
    range.set_start(&text, off).unwrap();
    range.set_end(&text, off).unwrap();
    range.get_bounding_client_rect().x() as f32
}

fn centre(el: &web_sys::Element) -> (f32, f32) {
    let r = el.get_bounding_client_rect();
    (
        (r.x() + r.width() / 2.0) as f32,
        (r.y() + r.height() / 2.0) as f32,
    )
}

/// The editor's `<img>`, once it has decoded and has a box to press on.
async fn loaded_image() -> web_sys::Element {
    for _ in 0..100 {
        if let Some(img) = document().query_selector("[data-pm-editor] img").unwrap()
            && img.get_bounding_client_rect().width() > 0.0
        {
            return img;
        }
        sleep(20).await;
    }
    panic!("the editor's image never got a box");
}

/// The editor's horizontal rule.
fn rule() -> web_sys::Element {
    document()
        .query_selector("[data-pm-editor] hr")
        .unwrap()
        .expect("the editor's horizontal rule")
}

/// Make `navigator.platform` report `value` (`None` restores the real one), for the
/// fixtures that stand in for macOS.
fn fake_platform(value: Option<&str>) {
    let nav: js_sys::Object =
        js_sys::Reflect::get(&web_sys::window().unwrap(), &"navigator".into())
            .unwrap()
            .unchecked_into();
    match value {
        Some(v) => {
            let desc = js_sys::Object::new();
            js_sys::Reflect::set(&desc, &"value".into(), &v.into()).unwrap();
            js_sys::Reflect::set(&desc, &"configurable".into(), &true.into()).unwrap();
            js_sys::Object::define_property(&nav, &"platform".into(), &desc);
        }
        None => {
            js_sys::Reflect::delete_property(&nav, &"platform".into()).unwrap();
        }
    }
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
    assert!(is_capture(&target), "precondition: parked at the press");
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
        "precondition: the second press lands on the parked textarea"
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

/// However a press ends — even when no menu ever answers it (a key an app
/// cancelled, a platform whose keys open none, a release that never arrives) —
/// the parked textarea is hittable only for the input event the browser builds
/// its menu from, not until something else happens: content under the box a
/// moment later is what a click there reaches. Kills: a park that waits for a
/// `contextmenu`, a release or a timeout of seconds before it goes back.
#[wasm_bindgen_test]
async fn a_park_no_menu_answers_never_intercepts_a_click_on_page_content() {
    let f = Fixture::mount();
    f.focus_at(f.point(0, 1));
    f.handle.set_selection(Selection::cursor(Pos(21)));
    let (_, cy) = f.point(1, 3);
    let cx = caret_x(&f.paragraph(1), 3);
    // Page content where the menu key's box will sit: a button over the text.
    let button = document().create_element("button").unwrap();
    button.set_attribute(HOST_MARKER, "").unwrap();
    button
        .set_attribute(
            "style",
            &format!(
                "position: fixed; left: {}px; top: {}px; width: 20px; height: 10px; \
                 margin: 0; padding: 0; border: 0; z-index: 10;",
                cx + 2.0,
                cy - 5.0
            ),
        )
        .unwrap();
    document().body().unwrap().append_child(&button).unwrap();
    let probe = (cx + 12.0, cy);
    assert!(
        under(probe.0, probe.1).is_same_node(Some(&button)),
        "positive control: the button is what a click at the probe point reaches"
    );

    // The menu key, with no `contextmenu` after it.
    key_event(&f.capture(), "keydown", "ContextMenu", false);
    assert!(
        is_capture(&under(probe.0, probe.1)),
        "precondition: the key parks the textarea over the button"
    );
    sleep(250).await;
    let hit = under(probe.0, probe.1);
    assert!(
        hit.is_same_node(Some(&button)),
        "a moment later a click there reaches the button again, found <{}>",
        hit.tag_name()
    );

    // A right press no menu and no release follow.
    let at = f.point(0, 8);
    mouse("mousedown", at.0, at.1, 2);
    assert!(
        is_capture(&under(at.0, at.1)),
        "precondition: the press parks the textarea"
    );
    sleep(250).await;
    let hit = under(at.0, at.1);
    assert!(
        !is_capture(&hit),
        "a moment later the press point is the editor's again, found <{}>",
        hit.tag_name()
    );
    button.remove();
    f.teardown();
}

/// Where the browser fires `contextmenu` at the **release** — Windows, for the
/// right button and for the menu key (by reading Chromium; not measured here) —
/// a press held longer than the park must still give the editing menu: the
/// release parks the textarea again when no `contextmenu` has come for the press
/// yet. Kills: no re-park at a right-button release; none at the menu key's.
#[wasm_bindgen_test]
async fn a_menu_that_follows_the_release_still_finds_the_textarea() {
    let f = Fixture::mount();
    let at = f.point(0, 8);
    f.focus_at(at);
    mouse("mousedown", at.0, at.1, 2);
    sleep(250).await;
    assert!(
        !is_capture(&under(at.0, at.1)),
        "a held press leaves nothing parked"
    );
    // Released a little way from where it went down.
    let up = (at.0 + 3.0, at.1);
    mouse("mouseup", up.0, up.1, 2);
    let target = under(up.0, up.1);
    assert!(
        is_capture(&target),
        "the release parks the textarea again, found <{}>",
        target.tag_name()
    );
    let ev = mouse_on(&target, "contextmenu", up.0, up.1, 2);
    assert!(!ev.default_prevented());
    sleep(150).await;
    assert!(
        !is_capture(&under(up.0, up.1)),
        "and it goes back after the menu"
    );

    // The menu key, whose `contextmenu` follows its release.
    f.handle.set_selection(Selection::cursor(Pos(21)));
    let (_, cy) = f.point(1, 3);
    let probe = (caret_x(&f.paragraph(1), 3) + 6.0, cy);
    let ta = f.capture();
    key_event(&ta, "keydown", "ContextMenu", false);
    sleep(250).await;
    assert!(
        !is_capture(&under(probe.0, probe.1)),
        "a held key leaves nothing parked"
    );
    key_event(&ta, "keyup", "ContextMenu", false);
    let target = under(probe.0, probe.1);
    assert!(
        is_capture(&target),
        "the key's release parks the textarea at the caret again, found <{}>",
        target.tag_name()
    );
    mouse_on(&target, "contextmenu", probe.0, probe.1, 0);
    sleep(150).await;
    assert!(!is_capture(&under(probe.0, probe.1)));
    f.teardown();
}

/// Focus moving to another element ends the menu cycle there and then — the
/// field gives up the sentinel — rather than on a timer. Kills: nothing ending a
/// cycle when focus leaves the textarea.
#[wasm_bindgen_test]
fn focus_moving_elsewhere_ends_the_menu_cycle() {
    let f = Fixture::mount();
    let at = f.point(0, 3);
    f.focus_at(at);
    f.right_press(at);
    let ta = f.capture();
    assert!(
        ta.value().starts_with(SENTINEL),
        "precondition: the cycle holds the field"
    );
    let other: web_sys::HtmlInputElement = document()
        .create_element("input")
        .unwrap()
        .dyn_into()
        .unwrap();
    other.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&other).unwrap();

    other.focus().unwrap();

    assert!(
        !ta.value().starts_with(SENTINEL),
        "the cycle ended with focus leaving, got {:?}",
        ta.value()
    );
    other.remove();
    f.teardown();
}

// ── The keyboard path ───────────────────────────────────────────────────────

/// The menu key and Shift+F10 make the browser fire `contextmenu` at the
/// focused element — the capture textarea — and open the menu at its box,
/// which used to be the viewport's top-left corner. On that key the textarea
/// is parked with its left edge at the caret — Chrome opens a keyboard menu at
/// the focused box's left edge (measured) — so the menu opens at the caret; the
/// key itself is left to the browser. Kills: not parking on the key; consuming
/// the key; a box centred on the caret (the menu opened 15px left of it).
#[wasm_bindgen_test]
async fn the_menu_key_parks_the_textarea_at_the_caret() {
    let f = Fixture::mount();
    f.focus_at(f.point(0, 1));
    f.handle.set_selection(Selection::cursor(Pos(21)));
    // Where the caret is: paragraph 2, before character 3.
    let (_, cy) = f.point(1, 3);
    let cx = caret_x(&f.paragraph(1), 3);
    let (right_of_caret, left_of_caret) = ((cx + 6.0, cy), (cx - 6.0, cy));
    let ta = f.capture();
    assert!(
        !is_capture(&under(right_of_caret.0, right_of_caret.1)),
        "positive control: nothing parked before the key"
    );

    for key in ["ContextMenu", "F10"] {
        let ev = key_event(&ta, "keydown", key, key == "F10");

        assert!(
            !ev.default_prevented(),
            "{key}: the key must reach the browser, which is what opens the menu"
        );
        let hit = under(right_of_caret.0, right_of_caret.1);
        assert!(
            is_capture(&hit),
            "{key}: the textarea is parked at the caret, found <{}>",
            hit.tag_name()
        );
        let left = ta.get_bounding_client_rect().x() as f32;
        assert!(
            (left - cx).abs() <= 1.0,
            "{key}: the box starts at the caret (x {cx}), not around it (x {left})"
        );
        assert!(
            !is_capture(&under(left_of_caret.0, left_of_caret.1)),
            "{key}: nothing left of the caret is covered"
        );
        // The browser's `contextmenu` for the key, at the textarea.
        mouse_on(&hit, "contextmenu", right_of_caret.0, right_of_caret.1, 0);
        sleep(150).await;
        assert!(
            !is_capture(&under(right_of_caret.0, right_of_caret.1)),
            "{key}: and back off-screen after the dispatch"
        );
    }
    f.teardown();
}

/// The menu key under an app's `data-oncontextmenu` leaves the textarea where it
/// is, as a right-click there does: the app has claimed the editor's context
/// menu. Kills: the key parking regardless of an app handler.
#[wasm_bindgen_test]
fn the_menu_key_does_not_park_under_an_apps_context_menu_handler() {
    let f = Fixture::mount_with(true);
    f.focus_at(f.point(0, 1));
    f.handle.set_selection(Selection::cursor(Pos(21)));
    let (_, cy) = f.point(1, 3);
    let probe = (caret_x(&f.paragraph(1), 3) + 6.0, cy);
    let ta = f.capture();

    let ev = key_event(&ta, "keydown", "ContextMenu", false);

    assert!(!ev.default_prevented(), "the key still reaches the browser");
    assert!(
        !is_capture(&under(probe.0, probe.1)),
        "nothing is parked at the caret under an app handler"
    );
    assert!(
        !ta.value().starts_with(SENTINEL),
        "and no menu cycle took the field, got {:?}",
        ta.value()
    );
    f.teardown();
}

/// A selected horizontal rule has no caret to park at. The menu key parks at the
/// rule's selection box instead, with the field carrying the selection, so the
/// menu's Copy acts on the rule — rather than opening at the page's corner over an
/// empty field with Copy greyed out. Kills: parking nothing without a caret rect.
#[wasm_bindgen_test]
fn the_menu_key_parks_at_a_selected_rule_that_has_no_caret() {
    let f = Fixture::mount_content(RULE, false);
    f.focus_at(f.point(0, 2));
    let hr = rule();
    let at = centre(&hr);
    mouse("mousedown", at.0, at.1, 0);
    mouse("mouseup", at.0, at.1, 0);
    let sel = f.handle.selection();
    assert_eq!(
        (sel.from(), sel.to()),
        (Pos(RULE_AT), Pos(RULE_AT + 1)),
        "precondition: a press on the rule node-selects it, got {sel:?}"
    );
    let outline = document()
        .query_selector("[data-pm-editor] [data-pm-selected]")
        .unwrap()
        .expect("precondition: the node-selection outline");
    let r = outline.get_bounding_client_rect();
    let probe = ((r.x() + 6.0) as f32, (r.y() + r.height() / 2.0) as f32);
    let ta = f.capture();

    let ev = key_event(&ta, "keydown", "ContextMenu", false);

    assert!(!ev.default_prevented());
    let hit = under(probe.0, probe.1);
    assert!(
        is_capture(&hit),
        "parked at the rule's box, found <{}>",
        hit.tag_name()
    );
    let (start, end) = selection_span(&ta);
    assert!(
        end > start,
        "the field carries the node selection for Copy, got {start}..{end}"
    );
    f.teardown();
}

// ── Undo from the menu ──────────────────────────────────────────────────────

/// The menu's Undo fires `beforeinput` with `historyUndo` on the textarea. It
/// is the editor's undo, not the textarea's: the document reverts and the
/// field is left alone. Kills: mapping `historyUndo` to nothing.
#[wasm_bindgen_test]
fn undo_from_the_menu_is_the_editors_undo() {
    let f = Fixture::mount();
    f.focus_at(f.point(1, 6));
    f.handle.set_selection(Selection::cursor(Pos(24)));
    assert!(f.handle.insert_text("!!"));
    assert_eq!(
        f.text(),
        "Hello world one|Second!! paragraph here",
        "positive control: the edit to undo"
    );

    f.right_press(f.point(1, 2));
    let ta = f.capture();
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type("historyUndo");
    let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    ta.dispatch_event(&ev).unwrap();

    assert!(
        ev.default_prevented(),
        "the textarea's own undo is cancelled"
    );
    assert_eq!(f.text(), "Hello world one|Second paragraph here");
    assert_eq!(
        ta.value(),
        "Second paragraph here",
        "the mirror follows the undo"
    );
    f.teardown();
}

/// The menu's Undo runs the page's undo stack, and Chrome (153, measured with a
/// trusted Undo command) dispatches `historyUndo` at the field that owns the
/// newest step — here an `<input>` typed into before the editor was focused —
/// not at the focused capture textarea. With the editor focused it is the
/// editor's undo. With that input itself focused, its own undo is left alone.
/// Kills: no guard for a history event aimed at another field; a guard that
/// ignores where focus is.
#[wasm_bindgen_test]
fn undo_from_the_menu_aimed_at_another_field_is_the_editors_undo() {
    let f = Fixture::mount();
    let other: web_sys::HtmlInputElement = document()
        .create_element("input")
        .unwrap()
        .dyn_into()
        .unwrap();
    other.set_attribute(HOST_MARKER, "").unwrap();
    document().body().unwrap().append_child(&other).unwrap();
    let history = |target: &web_sys::Element| {
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type("historyUndo");
        let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        target.dispatch_event(&ev).unwrap();
        ev
    };

    f.focus_at(f.point(1, 6));
    f.handle.set_selection(Selection::cursor(Pos(24)));
    assert!(f.handle.insert_text("!!"));
    f.right_press(f.point(1, 2));
    let ev = history(other.as_ref());
    assert!(
        ev.default_prevented(),
        "the other field's undo is cancelled"
    );
    assert_eq!(
        f.text(),
        "Hello world one|Second paragraph here",
        "and the editor's edit is undone"
    );
    assert!(
        document().active_element().as_deref() == Some(f.capture().as_ref()),
        "focus stays in the editor"
    );
    // Focus moved to the input without a press (Tab, a script): the editor stays
    // the focused editor as far as rinch knows, and the input's own undo is its own.
    other.focus().unwrap();
    let ev = history(other.as_ref());
    assert!(
        !ev.default_prevented(),
        "a focused input keeps its own undo"
    );
    assert_eq!(
        f.text(),
        "Hello world one|Second paragraph here",
        "and the editor is not undone again"
    );
    other.remove();
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

/// On macOS a Control-click is the context click: a *left* press carrying
/// `ctrlKey`, which the browser answers with `contextmenu`. It gets the same
/// caret rule and park as a right press there, and only there — elsewhere a
/// Control-click is a click. `navigator.platform` stands in for a Mac here; the
/// real thing is unverified. Kills: a Control-click never parking; one parking
/// on every platform.
#[wasm_bindgen_test]
fn on_macos_a_control_click_opens_the_editing_menu_too() {
    let f = Fixture::mount();
    f.focus_at(f.point(1, 2));
    let at = f.point(0, 8);

    fake_platform(Some("MacIntel"));
    mouse_with_ctrl(&under(at.0, at.1), "mousedown", at.0, at.1, 0, true);
    let parked = under(at.0, at.1);
    let head = f.handle.selection();
    fake_platform(None);

    assert!(
        is_capture(&parked),
        "a Mac Control-click parks the textarea, found <{}>",
        parked.tag_name()
    );
    assert!(
        head.is_empty() && (9..=10).contains(&head.head().0),
        "with the right-press caret rule: the caret moves to the press, got {head:?}"
    );
    mouse_on(&parked, "contextmenu", at.0, at.1, 0);
    keydown(&f.capture(), "Escape");

    // Off the Mac it is a click: the caret moves and nothing is parked.
    let at = f.point(1, 5);
    mouse_with_ctrl(&under(at.0, at.1), "mousedown", at.0, at.1, 0, true);
    let hit = under(at.0, at.1);
    mouse_with_ctrl(&hit, "mouseup", at.0, at.1, 0, true);
    assert!(
        !is_capture(&hit),
        "a Control-click on Linux is a click, found <{}>",
        hit.tag_name()
    );
    assert!(
        !f.capture().value().starts_with(SENTINEL),
        "and starts no menu cycle"
    );
    f.teardown();
}

/// A right press on a horizontal rule node-selects it before parking — the rule
/// has no text for a caret — so the menu's Copy and Cut act on the rule: the
/// field carries a selection. Kills: a right press on a leaf that does not
/// node-select it.
#[wasm_bindgen_test]
fn a_right_press_on_a_horizontal_rule_selects_it_for_the_menu() {
    let f = Fixture::mount_content(RULE, false);
    f.focus_at(f.point(0, 2));
    let hr = rule();
    let at = centre(&hr);
    assert!(
        under(at.0, at.1).is_same_node(Some(&hr)),
        "positive control: the press lands on the rule"
    );

    let (_, tag) = f.right_press(at);

    assert_eq!(tag, "TEXTAREA");
    let sel = f.handle.selection();
    assert_eq!(
        (sel.from(), sel.to()),
        (Pos(RULE_AT), Pos(RULE_AT + 1)),
        "the rule is node-selected, got {sel:?}"
    );
    let (start, end) = selection_span(&f.capture());
    assert!(
        end > start,
        "and the field has a selection for Copy / Cut, got {start}..{end}"
    );
    f.teardown();
}

// ── Links and images ────────────────────────────────────────────────────────

/// A right press on a link with nothing selected there keeps the browser's own
/// link menu (Open link, Copy link address): nothing is parked and the
/// `contextmenu` goes to the `<a>`. The caret moves to the press point first, as
/// a native `contenteditable` moves it (measured in Chrome 153). A selection
/// somewhere else changes nothing. Kills: parking over a link regardless.
#[wasm_bindgen_test]
fn a_right_press_on_a_link_with_nothing_selected_there_keeps_the_link_menu() {
    let f = Fixture::mount_content(LINK_IMAGE, false);
    f.focus_at(f.point(0, 2));
    let on_link = f.point(0, 8); // the 'n' of "link"

    for elsewhere in [Selection::cursor(Pos(3)), Selection::text(Pos(17), Pos(20))] {
        f.handle.set_selection(elsewhere.clone());
        let (ev, tag) = f.right_press(on_link);
        assert_eq!(
            tag, "A",
            "{elsewhere:?}: the contextmenu targets the link, not a parked textarea"
        );
        assert!(
            !ev.default_prevented(),
            "{elsewhere:?}: nothing suppresses the browser's link menu"
        );
        let sel = f.handle.selection();
        assert!(
            sel.is_empty() && (LINK.0..=LINK.1).contains(&sel.head().0),
            "{elsewhere:?}: the caret moves onto the link, as in a native editor, got {sel:?}"
        );
        assert!(
            !f.capture().value().starts_with(SENTINEL),
            "{elsewhere:?}: and no menu cycle took the field"
        );
    }
    f.teardown();
}

/// A right press on an image with nothing selected there keeps the browser's
/// image menu (Save image, Copy image): nothing is parked, the `contextmenu`
/// goes to the `<img>`, and the selection stays where it was — a native
/// `contenteditable` does not select the image either (measured). Kills:
/// parking over an image regardless.
#[wasm_bindgen_test]
async fn a_right_press_on_an_image_with_nothing_selected_there_keeps_the_image_menu() {
    let f = Fixture::mount_content(LINK_IMAGE, false);
    f.focus_at(f.point(0, 2));
    let at = centre(&loaded_image().await);

    for elsewhere in [Selection::cursor(Pos(3)), Selection::text(Pos(17), Pos(20))] {
        f.handle.set_selection(elsewhere.clone());
        let (ev, tag) = f.right_press(at);
        assert_eq!(
            tag, "IMG",
            "{elsewhere:?}: the contextmenu targets the image, not a parked textarea"
        );
        assert!(!ev.default_prevented());
        assert_eq!(
            f.handle.selection(),
            elsewhere,
            "the selection is left where it was"
        );
    }
    f.teardown();
}

/// A right press inside a non-empty selection parks as it does anywhere else,
/// even over a link or an image, so the menu's Cut / Copy / Paste act on that
/// selection — kept whole, not collapsed onto the link or narrowed to the
/// image. An image that is itself the selection counts too. Kills: never parking
/// over a link or an image; node-selecting an image the selection already holds.
#[wasm_bindgen_test]
async fn a_right_press_inside_a_selection_parks_over_a_link_or_an_image() {
    let f = Fixture::mount_content(LINK_IMAGE, false);
    f.focus_at(f.point(0, 2));
    let image = centre(&loaded_image().await);
    // From "llo" in paragraph 1, over the link and the image, to " e" after it.
    let spanning = Selection::text(Pos(3), Pos(30));

    f.handle.set_selection(spanning.clone());
    let (_, tag) = f.right_press(f.point(0, 8));
    assert_eq!(tag, "TEXTAREA", "parked over the link inside the selection");
    assert_eq!(f.handle.selection(), spanning, "the selection is kept");

    keydown(&f.capture(), "Escape");
    let (_, tag) = f.right_press(image);
    assert_eq!(
        tag, "TEXTAREA",
        "parked over the image inside the selection"
    );
    assert_eq!(
        f.handle.selection(),
        spanning,
        "the image is not node-selected out of the selection"
    );

    keydown(&f.capture(), "Escape");
    let image_only = Selection::node_at(&f.handle.doc(), Pos(IMAGE_AT)).expect("the image node");
    f.handle.set_selection(image_only.clone());
    let (_, tag) = f.right_press(image);
    assert_eq!(tag, "TEXTAREA", "parked over the selected image");
    assert_eq!(f.handle.selection(), image_only);
    let (start, end) = selection_span(&f.capture());
    assert!(end > start, "and the field has a selection for Copy / Cut");
    f.teardown();
}

/// An app's `data-oncontextmenu` wins over links and images as it does over the
/// rest of the editor: it is dispatched and suppresses the browser's menu, and
/// nothing is parked — outside the selection and inside it. Kills: parking over
/// a link inside a selection without asking the app.
#[wasm_bindgen_test]
async fn an_apps_context_menu_handler_wins_over_links_and_images_too() {
    let f = Fixture::mount_content(LINK_IMAGE, true);
    f.focus_at(f.point(0, 2));
    let image = centre(&loaded_image().await);
    let link = f.point(0, 8);
    let cases = [
        (
            "a link, caret elsewhere",
            Selection::cursor(Pos(3)),
            link,
            "A",
        ),
        (
            "an image, caret elsewhere",
            Selection::cursor(Pos(3)),
            image,
            "IMG",
        ),
        (
            "a link inside a selection",
            Selection::text(Pos(3), Pos(30)),
            link,
            "A",
        ),
    ];
    for (i, (label, selection, at, want)) in cases.into_iter().enumerate() {
        f.handle.set_selection(selection);
        let (ev, tag) = f.right_press(at);
        assert_eq!(tag, want, "{label}: nothing parked");
        assert!(
            ev.default_prevented(),
            "{label}: the app's handler suppresses the browser's menu"
        );
        assert_eq!(f.app_menu.get(), i as u32 + 1, "{label}: and is dispatched");
    }
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
        "precondition: the menu was built for the textarea"
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
        "precondition: the sentinel is in the field"
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

/// The page hears of the menu's Select All only through the field's own `select`
/// / `selectionchange`, and a menu stays open as long as the user likes, so
/// nothing but the next real interaction may take the sentinel away. A Select
/// All chosen 5.6 s after the press — past the 5 s poll this used to run on —
/// still selects the whole document. Kills: any timer that ends the cycle; no
/// `select` / `selectionchange` listener.
#[wasm_bindgen_test]
async fn select_all_chosen_long_after_the_menu_opened_still_selects_the_whole_document() {
    let f = Fixture::mount();
    let at = f.point(1, 3);
    f.focus_at(at);
    f.right_press(at);
    let ta = f.capture();

    sleep(5_600).await;
    assert!(
        ta.value().starts_with(SENTINEL),
        "the menu cycle still holds the field, got {:?}",
        ta.value()
    );
    ta.select();
    sleep(100).await;

    let sel = f.handle.selection();
    assert_eq!(
        (sel.from(), sel.to()),
        (Pos(1), Pos(39)),
        "a late Select All selects the whole document, got {sel:?}"
    );
    f.teardown();
}

/// A right-click on macOS also selects the word under the pointer — inside the
/// parked field, since that is what is under the pointer. With a caret and no
/// selection the field used to hold the sentinel alone, and a word selection of
/// that (measured in Chrome by double-clicking such a field, the same word
/// granularity: 0..1) is the whole field, which reads as Select All. The field
/// now ends with a sentinel too, so a word — one character at either end here —
/// never spans it. Unverified on a Mac. Kills: a sentinel at one end only.
#[wasm_bindgen_test]
async fn a_word_selection_in_the_parked_field_is_never_select_all() {
    let f = Fixture::mount();
    let at = f.point(1, 3);
    f.focus_at(at);
    f.right_press(at);
    let ta = f.capture();
    let caret = f.handle.selection();
    assert!(caret.is_empty(), "precondition: a caret, no selection");
    let len = ta.value().encode_utf16().count() as u32;

    for (start, end) in [(0, 1), (len - 1, len)] {
        ta.set_selection_range(start, end).unwrap();
        // The `select` / `selectionchange` the browser queues for it.
        sleep(50).await;
        assert_eq!(
            f.handle.selection(),
            caret,
            "a selection of {start}..{end} in a field of {len} is not Select All"
        );
    }
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
        "precondition: the caret is mirrored after \"Hello\""
    );
    // The IME's commit: it rewrites the field, then `compositionend` says what it composed.
    composition(&ta, "compositionstart", "");
    let mut value = ta.value();
    value.insert(caret as usize, 'ö');
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

// ── The page-wide suppression flag ──────────────────────────────────────────

/// With `set_suppress_native_context_menu(true)` the page takes the browser's
/// menu away everywhere — except in an editing context (issue #812), which the
/// parked textarea is. So the editor keeps its editing menu under the flag,
/// while the same right press on the editor's surface without the textarea
/// under it would be suppressed. Kills: a carve-out keyed on anything but the
/// event's target; parking after the delegation has already answered.
#[wasm_bindgen_test]
fn with_the_flag_on_the_parked_textarea_keeps_the_browsers_menu() {
    let f = Fixture::mount();
    let at = f.point(0, 8);
    f.focus_at(at);
    rinch_web::set_suppress_native_context_menu(true);

    // Positive control: with the flag on, a contextmenu at the editor's surface
    // itself (nothing parked) is suppressed.
    let surface = under(at.0, at.1);
    assert!(!is_capture(&surface));
    let ev = mouse_on(&surface, "contextmenu", at.0, at.1, 2);
    assert!(
        ev.default_prevented(),
        "the flag suppresses the menu on the plain surface"
    );

    let (ev, tag) = f.right_press(at);
    assert_eq!(tag, "TEXTAREA");
    assert!(
        !ev.default_prevented(),
        "the parked textarea is an editing context: the browser's menu stays"
    );
    f.teardown();
}
