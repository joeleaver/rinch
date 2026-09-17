//! Browser-driven tests for a read-only rich-text editor on the web
//! (`EditorHandle::set_read_only`).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_read_only
//! # the collaboration fixture as well:
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --features collaboration \
//!   --test editor_read_only
//! ```
//!
//! The guarantee — every local edit refused, remote ones still applied — is the
//! handle's, shared with desktop and pinned natively in `rinch-editor-view`.
//! What only a browser can show is this crate's half:
//!
//! * every channel the web glue has for an edit — `keydown`, `beforeinput`, the
//!   mirror-and-diff `input`, `paste`, `cut`, a composition — ends at that gate,
//!   each shown to **edit** an editable editor first;
//! * the hidden capture `<textarea>` is `readonly` exactly while the focused
//!   editor is, which is what keeps a soft keyboard down, an IME from composing
//!   and the browser's own menu (#814) from offering Paste and Cut — and it
//!   follows the switch at runtime, and from one editor to another;
//! * the reader's side still works: a press places the caret, arrows move it,
//!   Shift extends, Mod+A selects, `copy` fills the clipboard;
//! * a remote collaboration delta still reaches the page.
//!
//! Every fixture mounts a real editor through `rinch_web::mount_into`, focuses it
//! with a genuine `mousedown`, and checks that the capture textarea exists and
//! holds focus before asserting anything.
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

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-test-host-read-only";

/// Paragraph 1 opens at 0 and its text runs 1..=16; "world" is 7..12.
const CONTENT: &str = "<p>Hello world one</p><p>Second paragraph here</p>";
const WORLD: (usize, usize) = (7, 12);
const TEXT: &str = "Hello world one|Second paragraph here";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// One editor over [`CONTENT`], focused by a real press, then switched.
    fn focused(read_only: bool) -> Self {
        let f = Self::mount(&[CONTENT]);
        f.focus_at(f.point(0, 0, 3));
        f.handle.set_read_only(read_only);
        f
    }

    /// Mount one editor per entry of `contents`, stacked; `handle` is the first.
    fn mount(contents: &[&str]) -> Self {
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
        let handles: Vec<EditorHandle> = contents
            .iter()
            .map(|content| {
                let handle = create_editor();
                assert!(handle.load_html(content));
                handle
            })
            .collect();
        let mounted = handles.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| {
                let wrapper = scope.create_element("div");
                for handle in &mounted {
                    wrapper.append_child(&handle.mount(scope));
                }
                wrapper
            },
        );
        Self {
            root,
            host,
            handle: handles[0].clone(),
        }
    }

    fn editor_el(&self, editor: u32) -> web_sys::Element {
        document()
            .query_selector_all("[data-pm-editor]")
            .unwrap()
            .item(editor)
            .unwrap_or_else(|| panic!("no editor {editor}"))
            .dyn_into()
            .unwrap()
    }

    /// The viewport centre of character `ch` of paragraph `p` of editor `editor`.
    fn point(&self, editor: u32, p: u32, ch: u32) -> (f32, f32) {
        let para = self
            .editor_el(editor)
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

    /// The first editor's text **as the page shows it**, block by block.
    fn text(&self) -> String {
        let ps = self.editor_el(0).query_selector_all("p").unwrap();
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

    /// Focus an editor with a genuine left press at `(x, y)` and prove the
    /// capture target exists and holds focus — the positive control every
    /// fixture rests on.
    fn focus_at(&self, (x, y): (f32, f32)) {
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        let ta = self.capture();
        assert!(
            document().active_element().as_deref() == Some(ta.as_ref()),
            "positive control: a left press must focus the capture textarea"
        );
    }

    fn select_world(&self) {
        self.handle
            .set_selection(Selection::text(Pos(WORLD.0), Pos(WORLD.1)));
    }

    fn teardown(self) {
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

/// A `keydown` on the capture textarea; answers whether the editor consumed it
/// (`preventDefault`ed), which is what keeps a key from the page and the field.
fn keydown(f: &Fixture, key: &str, code: &str, ctrl: bool, shift: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    init.set_ctrl_key(ctrl);
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

/// A `beforeinput` of `input_type` on the capture textarea — a soft keyboard's
/// only channel.
fn before_input(f: &Fixture, input_type: &str, data: Option<&str>) {
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type(input_type);
    init.set_data(data);
    let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
}

/// A `copy` / `cut` / `paste` on the capture textarea carrying a fresh
/// `DataTransfer`, as the browser fires it on the focused editable.
fn clipboard(f: &Fixture, name: &str, plain: Option<&str>) -> web_sys::DataTransfer {
    let dt = web_sys::DataTransfer::new().unwrap();
    if let Some(text) = plain {
        dt.set_data("text/plain", text).unwrap();
    }
    let init = web_sys::ClipboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_clipboard_data(Some(&dt));
    let ev = web_sys::ClipboardEvent::new_with_event_init_dict(name, &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
    dt
}

/// What an autocorrect / a keyboard with no `inputType` we know does: rewrite
/// the mirrored field and fire `input`, leaving the edit to the diff.
fn rewrite_mirror(f: &Fixture, value: &str) {
    let ta = f.capture();
    ta.set_value(value);
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    let ev = web_sys::InputEvent::new_with_event_init_dict("input", &init).unwrap();
    ta.dispatch_event(&ev).unwrap();
}

fn composition(f: &Fixture, name: &str, data: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_data(data);
    let ev = web_sys::CompositionEvent::new_with_event_init_dict(name, &init).unwrap();
    f.capture().dispatch_event(&ev).unwrap();
}

/// One way an edit reaches the editor from the browser, by name.
type EditChannel = (&'static str, fn(&Fixture));

/// Every channel the web glue has for an edit. Each starts from "world" selected
/// (or, where it sets one, a caret) in a focused editor.
fn edit_channels() -> Vec<EditChannel> {
    vec![
        ("a typed letter (keydown)", |f| {
            keydown(f, "x", "KeyX", false, false);
        }),
        ("Backspace (keydown → keymap)", |f| {
            keydown(f, "Backspace", "Backspace", false, false);
        }),
        ("Enter (keydown → keymap)", |f| {
            keydown(f, "Enter", "Enter", false, false);
        }),
        ("Mod+B (keydown → keymap)", |f| {
            keydown(f, "b", "KeyB", true, false);
        }),
        ("soft-keyboard text (beforeinput insertText)", |f| {
            before_input(f, "insertText", Some("x"));
        }),
        ("soft-keyboard delete (beforeinput)", |f| {
            before_input(f, "deleteContentBackward", None);
        }),
        ("a word delete (beforeinput)", |f| {
            f.handle.set_selection(Selection::cursor(Pos(12)));
            before_input(f, "deleteWordBackward", None);
        }),
        ("a soft Enter (beforeinput insertParagraph)", |f| {
            before_input(f, "insertParagraph", None);
        }),
        ("an autocorrect rewrite (input → mirror diff)", |f| {
            rewrite_mirror(f, "Hello there one");
        }),
        ("paste", |f| {
            clipboard(f, "paste", Some("PASTED"));
        }),
        ("cut", |f| {
            clipboard(f, "cut", None);
        }),
        ("an IME commit (composition → mirror diff)", |f| {
            f.handle.set_selection(Selection::cursor(Pos(12)));
            composition(f, "compositionstart", "");
            composition(f, "compositionupdate", "ね");
            f.capture().set_value("Hello worldね one");
            composition(f, "compositionend", "ね");
        }),
    ]
}

#[wasm_bindgen_test]
fn no_input_channel_edits_a_read_only_editor() {
    for (what, edit) in edit_channels() {
        let control = Fixture::focused(false);
        control.select_world();
        let before = control.handle.doc();
        edit(&control);
        assert!(
            !control.handle.doc().same_ref(&before),
            "control: {what} edits an editable editor"
        );
        control.teardown();

        let f = Fixture::focused(true);
        f.select_world();
        let doc = f.handle.doc();
        edit(&f);
        assert!(
            f.handle.doc().same_ref(&doc),
            "{what} must leave a read-only document alone"
        );
        assert_eq!(f.text(), TEXT, "{what}: and the page with it");
        f.teardown();
    }
}

/// Undo and redo reach the editor as `beforeinput` history gestures too.
#[wasm_bindgen_test]
fn undo_is_refused_while_read_only_and_works_again_after() {
    let f = Fixture::focused(false);
    f.handle.set_selection(Selection::cursor(Pos(6)));
    keydown(&f, "!", "Digit1", false, true);
    assert_eq!(f.text(), "Hello! world one|Second paragraph here");

    f.handle.set_read_only(true);
    keydown(&f, "z", "KeyZ", true, false);
    before_input(&f, "historyUndo", None);
    assert_eq!(f.text(), "Hello! world one|Second paragraph here");

    f.handle.set_read_only(false);
    keydown(&f, "z", "KeyZ", true, false);
    assert_eq!(f.text(), TEXT, "writable again: the same chord undoes");
    f.teardown();
}

/// The capture field is `readonly` exactly while the focused editor is: from the
/// press that focuses it, across a runtime flip, and from one editor to the next.
#[wasm_bindgen_test]
fn the_capture_field_is_readonly_exactly_while_the_focused_editor_is() {
    let f = Fixture::mount(&[CONTENT, "<p>an editable neighbour</p>"]);
    f.handle.set_read_only(true);
    // What the field's `readonly` was **at each `focus()` call** on it. A writable
    // field taking focus is what raises a soft keyboard, so being `readonly` a
    // moment later is too late — and no assertion after the press can tell the
    // two apart. (Wrapping `focus` rather than listening for the event: a headless
    // page without system focus fires none.)
    js_sys::eval(
        "(() => { const proto = HTMLTextAreaElement.prototype, focus = proto.focus; \
           window.__roAtFocus = []; \
           proto.focus = function (...a) { \
             if (this.hasAttribute('data-pm-capture')) window.__roAtFocus.push(this.readOnly); \
             return focus.apply(this, a); }; \
           window.__roRestoreFocus = () => { proto.focus = focus; }; })()",
    )
    .unwrap();
    let at_focus = || {
        let seen = js_sys::eval("JSON.stringify(window.__roAtFocus)").unwrap();
        seen.as_string().unwrap()
    };

    // Locked *before* the press, so the field is already `readonly` when it is
    // focused.
    f.focus_at(f.point(0, 0, 3));
    assert!(f.capture().read_only(), "focused a read-only editor");
    assert_eq!(
        at_focus(),
        "[true]",
        "and was `readonly` before it took focus"
    );
    assert_eq!(
        f.editor_el(0).get_attribute("data-pm-readonly").as_deref(),
        Some("true")
    );

    // The one capture field serves every editor: the neighbour is writable.
    f.focus_at(f.point(1, 0, 3));
    assert!(!f.capture().read_only(), "focused the editable neighbour");
    f.focus_at(f.point(0, 0, 3));
    assert!(f.capture().read_only(), "and back");
    assert_eq!(
        at_focus(),
        "[true,false,true]",
        "each time decided before the focus, not after"
    );
    js_sys::eval("window.__roRestoreFocus()").unwrap();

    // A runtime flip (a role changed) reaches the field with no input event.
    f.handle.set_read_only(false);
    assert!(!f.capture().read_only(), "unlocked while focused");
    assert_eq!(f.editor_el(0).get_attribute("data-pm-readonly"), None);
    keydown(&f, "x", "KeyX", false, false);
    assert_ne!(f.text(), TEXT, "and it types again");
    f.handle.set_read_only(true);
    assert!(f.capture().read_only(), "locked while focused");
    f.teardown();
}

/// What a reader does: place the caret, move it, extend, select all, copy. And
/// the keys it cannot use are still the editor's — consumed, so a typed letter
/// does not go on to the page's shortcuts and Space does not scroll it.
#[wasm_bindgen_test]
fn a_read_only_editor_still_selects_and_copies_and_owns_its_keys() {
    let f = Fixture::focused(true);
    let doc = f.handle.doc();

    let at = f.point(0, 0, 6);
    mouse("mousedown", at.0, at.1);
    mouse("mouseup", at.0, at.1);
    let caret = f.handle.selection();
    assert!(caret.is_empty(), "a press places a caret");

    assert!(keydown(&f, "ArrowRight", "ArrowRight", false, false));
    assert_eq!(f.handle.selection().head().0, caret.head().0 + 1);
    assert!(keydown(&f, "ArrowRight", "ArrowRight", false, true));
    assert!(!f.handle.selection().is_empty(), "Shift+Arrow extends");

    f.select_world();
    let dt = clipboard(&f, "copy", None);
    assert_eq!(dt.get_data("text/plain").unwrap(), "world");
    assert!(dt.get_data("text/html").unwrap().contains("world"));

    // Cut is not a quiet Copy: the clipboard is left as it was.
    let dt = clipboard(&f, "cut", None);
    assert_eq!(dt.get_data("text/plain").unwrap(), "");

    assert!(keydown(&f, "a", "KeyA", true, false), "Mod+A is consumed");
    let dt = clipboard(&f, "copy", None);
    assert_eq!(
        dt.get_data("text/plain").unwrap(),
        "Hello world one\nSecond paragraph here"
    );

    assert!(
        keydown(&f, "x", "KeyX", false, false),
        "a letter is consumed"
    );
    assert!(keydown(&f, " ", "Space", false, false), "Space is consumed");
    assert!(
        keydown(&f, "Enter", "Enter", false, false),
        "Enter is consumed"
    );
    assert!(f.handle.doc().same_ref(&doc), "and none of it was an edit");
    f.teardown();
}

/// No composition is shown over a read-only editor, even from a browser that
/// composes into a `readonly` field anyway.
#[wasm_bindgen_test]
fn a_read_only_editor_shows_no_composition() {
    let preedit_shown = || {
        document()
            .query_selector("[data-pm-preedit]")
            .unwrap()
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
            .is_some_and(|el| el.style().get_property_value("display").unwrap() != "none")
    };

    let control = Fixture::focused(false);
    composition(&control, "compositionstart", "");
    composition(&control, "compositionupdate", "ne");
    assert!(
        preedit_shown(),
        "control: an editable editor shows the preedit"
    );
    composition(&control, "compositionend", "");
    control.teardown();

    let f = Fixture::focused(true);
    composition(&f, "compositionstart", "");
    composition(&f, "compositionupdate", "ne");
    assert!(!preedit_shown(), "no preedit over a read-only editor");
    composition(&f, "compositionend", "");
    assert_eq!(f.text(), TEXT);
    f.teardown();
}

/// The other half of read-only: what a peer writes still arrives and is shown.
/// Needs `--features collaboration`, which no CI job turns on for this crate.
#[cfg(feature = "collaboration")]
#[wasm_bindgen_test]
fn a_read_only_editor_still_shows_remote_edits_and_sends_none() {
    use std::cell::Cell;
    use std::rc::Rc;

    let f = Fixture::focused(true);
    // An unmounted peer hosts; the mounted, read-only editor joins it.
    let host = create_editor();
    assert!(host.load_html(CONTENT));
    let guest_in = f.handle.clone();
    let snapshot = host
        .start_collaboration_host(move |delta| {
            guest_in.collab_receive(&delta);
        })
        .expect("the host projects its document");
    let sent = Rc::new(Cell::new(0u32));
    let counter = sent.clone();
    f.handle
        .start_collaboration_guest(&snapshot, move |_| counter.set(counter.get() + 1))
        .expect("a read-only guest joins");
    assert_eq!(f.text(), TEXT);

    host.set_selection(Selection::cursor(Pos(6)));
    assert!(host.insert_text(","));
    assert_eq!(
        f.text(),
        "Hello, world one|Second paragraph here",
        "the peer's edit reached the page"
    );

    // Local input, through the browser's channels, still goes nowhere.
    f.select_world();
    keydown(&f, "x", "KeyX", false, false);
    before_input(&f, "insertText", Some("x"));
    clipboard(&f, "paste", Some("PASTED"));
    assert_eq!(f.text(), "Hello, world one|Second paragraph here");
    assert_eq!(sent.get(), 0, "and nothing was sent to the peer");

    f.handle.stop_collaboration();
    f.teardown();
}
