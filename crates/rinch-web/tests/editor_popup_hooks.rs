//! Browser-driven tests for what an app needs to drive an autocomplete popup
//! from the rich-text editor: `EditorHandle::on_key`, `on_selection_change`,
//! `on_caret_moved` and `caret_rect`.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_popup_hooks
//! ```
//!
//! The rules are the handle's and are pinned natively in `rinch-editor-view`.
//! What only a browser can show is the web glue's half: the editor's
//! document-capture `keydown` listener used to act on the arrows and Enter
//! before any app code saw them; now every key is offered first, in the
//! event's own spelling, and a consumed one is `preventDefault`ed and stopped
//! and leaves the editor untouched. Keys an input method owns are not offered.
//! `caret_rect` is where the caret overlay is painted, in client pixels.
//!
//! Every consumed key has an unconsumed control beside it. Every fixture
//! mounts a real editor through `rinch_web::mount_into` and focuses it with a
//! genuine `mousedown`, proving the capture textarea holds focus first.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-popup-hooks";

thread_local! {
    /// The root a fixture mounted, so the next fixture can unmount one a failed
    /// test left behind (a failed assertion never reaches its own `teardown`),
    /// editor registration and all: one failure then stays one failure.
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

/// "Hello world one" is 1..16, "Second paragraph here" is 18..39, the empty
/// paragraph's caret is 41.
const CONTENT: &str = "<p>Hello world one</p><p>Second paragraph here</p><p></p>";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// One editor over [`CONTENT`], focused by a real press on the "l" at
    /// character 3 of the first paragraph.
    fn focused() -> Self {
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
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(CONTENT));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        LIVE_ROOT.with(|r| r.set(Some(root)));
        let f = Self { root, host, handle };
        let (x, y) = f.point(0, 3);
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(f.capture().as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
        f
    }

    fn editor_el(&self) -> web_sys::Element {
        document()
            .query_selector("[data-pm-editor]")
            .unwrap()
            .expect("the editor is mounted")
    }

    /// The viewport centre of character `ch` of paragraph `p`.
    fn point(&self, p: u32, ch: u32) -> (f32, f32) {
        let para = self
            .editor_el()
            .query_selector_all("p")
            .unwrap()
            .item(p)
            .unwrap_or_else(|| panic!("no paragraph {p}"));
        let text = para.first_child().expect("the paragraph has a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, ch).unwrap();
        range.set_end(&text, ch + 1).unwrap();
        let r = range.get_bounding_client_rect();
        (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        )
    }

    /// The editor's text as the page shows it, block by block.
    fn text(&self) -> String {
        let ps = self.editor_el().query_selector_all("p").unwrap();
        (0..ps.length())
            .map(|i| ps.item(i).unwrap().text_content().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|")
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// The painted caret overlay's client rect.
    fn painted_caret(&self) -> web_sys::DomRect {
        self.editor_el()
            .query_selector("[data-pm-caret]")
            .unwrap()
            .expect("a caret overlay")
            .get_bounding_client_rect()
    }

    fn teardown(self) {
        LIVE_ROOT.with(|r| r.take());
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

/// Modifiers for [`keydown`].
#[derive(Default, Clone, Copy)]
struct Mods {
    ctrl: bool,
    shift: bool,
    meta: bool,
}

/// A `keydown` on the capture textarea; answers whether it was
/// `preventDefault`ed (consumed by the editor or the app).
fn keydown(f: &Fixture, key: &str, code: &str, mods: Mods) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    init.set_ctrl_key(mods.ctrl);
    init.set_shift_key(mods.shift);
    init.set_meta_key(mods.meta);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

fn composition(f: &Fixture, name: &str, data: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_data(data);
    let ev = web_sys::CompositionEvent::new_with_event_init_dict(name, &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
}

/// Every key offered, as `(key, primary, shift)`, answering `consume(key)`.
type Offered = Rc<RefCell<Vec<(String, bool, bool)>>>;

fn offer_keys(f: &Fixture, consume: impl Fn(&str) -> bool + 'static) -> Offered {
    let seen: Offered = Rc::default();
    f.handle.on_key({
        let seen = seen.clone();
        move |k| {
            seen.borrow_mut()
                .push((k.key.to_string(), k.primary, k.shift));
            consume(k.key)
        }
    });
    seen
}

/// Count the `keydown`s that reach a bubble-phase listener on the host: a
/// consumed key is stopped, as one the editor handles itself.
fn count_bubbled_keys(f: &Fixture) -> Rc<RefCell<u32>> {
    let n: Rc<RefCell<u32>> = Rc::default();
    let n_in = n.clone();
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| *n_in.borrow_mut() += 1);
    document()
        .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();
    let _ = f;
    n
}

#[wasm_bindgen_test]
fn a_consumed_arrow_down_and_enter_leave_the_editor_alone() {
    let f = Fixture::focused();
    let seen = offer_keys(&f, |k| matches!(k, "ArrowDown" | "Enter"));
    let caret = f.handle.selection();
    let bubbled = count_bubbled_keys(&f);

    assert!(keydown(&f, "ArrowDown", "ArrowDown", Mods::default()));
    assert!(keydown(&f, "Enter", "Enter", Mods::default()));

    assert_eq!(
        f.handle.selection(),
        caret,
        "ArrowDown did not move the caret"
    );
    assert_eq!(
        f.text(),
        "Hello world one|Second paragraph here|",
        "Enter split nothing"
    );
    assert_eq!(*bubbled.borrow(), 0, "a consumed key is stopped");
    let keys: Vec<String> = seen.borrow().iter().map(|(k, ..)| k.clone()).collect();
    assert_eq!(keys, ["ArrowDown", "Enter"]);
    f.teardown();
}

#[wasm_bindgen_test]
fn a_declined_key_is_the_editors_as_before() {
    let f = Fixture::focused();
    let seen = offer_keys(&f, |_| false);

    assert!(keydown(&f, "ArrowDown", "ArrowDown", Mods::default()));
    let head = f.handle.selection().head().0;
    assert!(
        (18..=39).contains(&head),
        "ArrowDown moved the caret into the second paragraph: {head}"
    );
    assert!(keydown(&f, "Enter", "Enter", Mods::default()));
    assert_eq!(
        f.editor_el().query_selector_all("p").unwrap().length(),
        4,
        "Enter split the paragraph"
    );
    assert_eq!(seen.borrow().len(), 2, "both were offered first");
    f.teardown();
}

/// Keys are offered in the event's own spelling with their modifiers; a
/// consumed printable key types nothing, a declined one types.
#[wasm_bindgen_test]
fn keys_are_offered_with_their_modifiers() {
    let f = Fixture::focused();
    let seen = offer_keys(&f, |k| k == "[");
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };

    assert!(keydown(&f, "[", "BracketLeft", Mods::default()));
    assert_eq!(f.text(), "Hello world one|Second paragraph here|");
    keydown(&f, "A", "KeyA", shift);
    keydown(&f, " ", "Space", Mods::default());
    keydown(&f, "b", "KeyB", ctrl);
    let meta = Mods {
        meta: true,
        ..Mods::default()
    };
    keydown(&f, "k", "KeyK", meta);
    keydown(&f, "Escape", "Escape", Mods::default());

    // The primary accelerator is Ctrl off macOS, where this suite runs: Meta
    // is not it.
    assert_eq!(
        *seen.borrow(),
        [
            ("[".to_string(), false, false),
            ("A".to_string(), false, true),
            (" ".to_string(), false, false),
            ("b".to_string(), true, false),
            ("k".to_string(), false, false),
            ("Escape".to_string(), false, false),
        ]
    );
    assert_eq!(f.text(), "HelA lo world one|Second paragraph here|");
    f.teardown();
}

/// A key pressed while an input method composes is the input method's, and a
/// `Process` / `Unidentified` key (an IME's or a soft keyboard's) is never
/// offered.
#[wasm_bindgen_test]
fn keys_an_input_method_owns_are_not_offered() {
    let f = Fixture::focused();
    let seen = offer_keys(&f, |_| true);

    composition(&f, "compositionstart", "");
    keydown(&f, "ArrowDown", "ArrowDown", Mods::default());
    composition(&f, "compositionend", "");
    keydown(&f, "Process", "KeyK", Mods::default());
    keydown(&f, "Unidentified", "", Mods::default());
    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());

    assert!(keydown(&f, "ArrowDown", "ArrowDown", Mods::default()));
    assert_eq!(
        seen.borrow().len(),
        1,
        "control: offered once composing ends"
    );
    f.teardown();
}

/// The link picker's shape: Enter replaces what was typed with a link from
/// inside the key callback. Nothing may be borrowed while it runs.
#[wasm_bindgen_test]
fn the_key_callback_may_edit_the_editor_it_came_from() {
    let f = Fixture::focused();
    let h = f.handle.clone();
    f.handle.on_key(move |k| {
        if k.key != "Enter" {
            return false;
        }
        assert!(h.caret_rect(h.selection().head()).is_some());
        h.set_selection(Selection::text(Pos(1), Pos(6)));
        assert!(h.toggle_link("pimble:a/b"));
        h.set_selection(Selection::cursor(Pos(6)));
        true
    });
    assert!(keydown(&f, "Enter", "Enter", Mods::default()));
    assert_eq!(f.text(), "Hello world one|Second paragraph here|");
    let a = f.editor_el().query_selector("a").unwrap().expect("a link");
    assert_eq!(a.text_content().as_deref(), Some("Hello"));
    // The caret was refreshed from the callback's selection.
    let r = f.handle.caret_rect(Pos(6)).unwrap();
    assert!((f.painted_caret().x() as f32 - r.x).abs() <= 1.0);
    f.teardown();
}

/// Real keys and clicks report the selection once per change; a consumed key
/// reports nothing.
#[wasm_bindgen_test]
fn the_selection_callback_hears_real_keys_and_clicks_once_each() {
    let f = Fixture::focused();
    let start = f.handle.selection().head().0;
    let seen: Rc<RefCell<Vec<Selection>>> = Rc::default();
    f.handle.on_selection_change({
        let seen = seen.clone();
        move |sel| seen.borrow_mut().push(sel.clone())
    });

    keydown(&f, "x", "KeyX", Mods::default());
    assert_eq!(
        *seen.borrow(),
        vec![Selection::cursor(Pos(start + 1))],
        "typing"
    );
    keydown(&f, "ArrowRight", "ArrowRight", Mods::default());
    assert_eq!(seen.borrow().len(), 2, "an arrow");

    let (x, y) = f.point(1, 2);
    mouse("mousedown", x, y);
    mouse("mouseup", x, y);
    assert_eq!(seen.borrow().len(), 3, "a click");

    let _keys = offer_keys(&f, |k| k == "ArrowDown");
    keydown(&f, "ArrowDown", "ArrowDown", Mods::default());
    assert_eq!(seen.borrow().len(), 3, "a consumed key");
    f.teardown();
}

/// `caret_rect` is where the caret overlay is painted, for a caret in text and
/// on a blank line, and answers for a position the caret is not at.
#[wasm_bindgen_test]
fn caret_rect_is_where_the_caret_is_painted() {
    let f = Fixture::focused();
    for pos in [4, 22, 41] {
        f.handle.set_selection(Selection::cursor(Pos(pos)));
        // The caret pass an input event would run.
        f.handle.update_caret();
        let painted = f.painted_caret();
        let r = f.handle.caret_rect(Pos(pos)).expect("a laid-out caret");
        assert!(
            (r.x - painted.x() as f32).abs() <= 1.0
                && (r.y - painted.y() as f32).abs() <= 1.0
                && (r.height - painted.height() as f32).abs() <= 1.0,
            "pos {pos}: caret_rect {r:?} vs painted ({}, {}, h {})",
            painted.x(),
            painted.y(),
            painted.height()
        );
    }
    let start = f.handle.caret_rect(Pos(18)).unwrap();
    let later = f.handle.caret_rect(Pos(22)).unwrap();
    assert!(start.x < later.x && (start.y - later.y).abs() < 0.5);
    assert_eq!(f.handle.caret_rect(Pos(0)), None, "between blocks");

    let unmounted = create_editor();
    assert!(unmounted.load_html(CONTENT));
    assert_eq!(unmounted.caret_rect(Pos(4)), None, "not mounted");
    f.teardown();
}

/// `on_caret_moved` comes once the caret is placed, with geometry equal to the
/// painted caret.
#[wasm_bindgen_test]
fn caret_moved_reports_the_painted_geometry() {
    let f = Fixture::focused();
    let h = f.handle.clone();
    let moved: Rc<RefCell<Vec<Option<rinch_core::ElementBounds>>>> = Rc::default();
    f.handle.on_caret_moved({
        let moved = moved.clone();
        move || moved.borrow_mut().push(h.caret_rect(h.selection().head()))
    });
    keydown(&f, "[", "BracketLeft", Mods::default());
    keydown(&f, "[", "BracketLeft", Mods::default());
    let last = moved
        .borrow()
        .last()
        .copied()
        .flatten()
        .expect("a call with geometry");
    let painted = f.painted_caret();
    assert!(
        (last.x - painted.x() as f32).abs() <= 1.0 && (last.y - painted.y() as f32).abs() <= 1.0,
        "{last:?} vs the painted caret"
    );
    let calls = moved.borrow().len();
    f.handle.update_caret();
    assert_eq!(
        moved.borrow().len(),
        calls,
        "a caret pass that moves nothing"
    );
    f.teardown();
}

/// A dismiss-stack entry standing in for a `Modal`'s `close_on_escape`. The
/// web backend marks no document, so any key reaches it.
fn modal_entry() -> (Rc<RefCell<u32>>, rinch_core::DismissHandle) {
    let dismissed: Rc<RefCell<u32>> = Rc::default();
    let handle = rinch_core::push_dismiss_handler(0, {
        let dismissed = dismissed.clone();
        move || {
            *dismissed.borrow_mut() += 1;
            true
        }
    });
    (dismissed, handle)
}

/// Twin of desktop's `on_key_sees_escape_before_the_dismiss_stack`: an
/// autocomplete popup in an editor inside a `Modal` takes Escape, and the
/// modal stays open.
#[wasm_bindgen_test]
fn on_key_sees_escape_before_the_dismiss_stack() {
    let f = Fixture::focused();
    let (dismissed, entry) = modal_entry();
    let seen = offer_keys(&f, |k| k == "Escape");
    assert!(keydown(&f, "Escape", "Escape", Mods::default()));
    assert_eq!((seen.borrow().len(), *dismissed.borrow()), (1, 0));
    drop(entry);
    f.teardown();
}

/// Twin of desktop's `an_escape_on_key_leaves_still_closes_the_modal`.
#[wasm_bindgen_test]
fn an_escape_on_key_leaves_still_closes_the_modal() {
    let f = Fixture::focused();
    let (dismissed, entry) = modal_entry();
    let seen = offer_keys(&f, |_| false);
    keydown(&f, "Escape", "Escape", Mods::default());
    assert_eq!((seen.borrow().len(), *dismissed.borrow()), (1, 1));
    drop(entry);
    f.teardown();
}
