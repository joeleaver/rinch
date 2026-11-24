//! #2993 on the web: Enter, Shift+Enter and Mod+Enter in a code block, as
//! `keydown`s through the editor's keymap path and as the `beforeinput`s a
//! soft keyboard sends (`insertParagraph`, `insertLineBreak`). Enter and
//! Shift+Enter type a `"\n"` into the block, which the page shows in the
//! `<pre>`; Mod+Enter leaves the block for a paragraph after it. Before the
//! fix Enter split the block in two and Shift+Enter did nothing.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_code_block_newline_2993
//! ```
//!
//! Every fixture mounts through `rinch_web::mount_into`, focuses with a genuine
//! press and checks the capture textarea holds focus before asserting anything.
//! Positions: `<p>x</p>` 0..3, the code block at 3, its text `abcd` at 4..8,
//! its end at 9.
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

const HOST_MARKER: &str = "data-test-host-code-newline-2993";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    handle: EditorHandle,
}

impl Fixture {
    /// One editor over `content`, focused by a real press on the first
    /// paragraph's first character.
    fn focused(content: &str) -> Self {
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
        host.set_attribute(
            "style",
            "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
        )
        .unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let handle = create_editor();
        assert!(handle.load_html(content));
        let mounted = handle.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| mounted.mount(scope),
        );
        let f = Self { root, host, handle };
        let para = f
            .editor_el()
            .query_selector("p")
            .unwrap()
            .expect("a paragraph");
        let text = para.first_child().expect("the paragraph has a text node");
        let range = document().create_range().unwrap();
        range.set_start(&text, 0).unwrap();
        range.set_end(&text, 1).unwrap();
        let r = range.get_bounding_client_rect();
        let (x, y) = (
            (r.x() + r.width() / 2.0) as f32,
            (r.y() + r.height() / 2.0) as f32,
        );
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
            .expect("an editor")
    }

    /// The editor's blocks as the page shows them: each block's tag and text.
    fn blocks(&self) -> Vec<(String, String)> {
        let children = self.editor_el().query_selector_all("p, pre").unwrap();
        (0..children.length())
            .map(|i| {
                let el: web_sys::Element = children.item(i).unwrap().dyn_into().unwrap();
                (
                    el.tag_name().to_ascii_lowercase(),
                    el.text_content().unwrap_or_default(),
                )
            })
            .collect()
    }

    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .expect("the capture textarea exists once an editor was focused")
            .dyn_into()
            .unwrap()
    }

    /// A `beforeinput` of `input_type` on the capture textarea; answers whether
    /// the editor took it (`preventDefault`ed).
    fn before_input(&self, input_type: &str) -> bool {
        let init = web_sys::InputEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_input_type(input_type);
        let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
    }

    /// An Enter `keydown` with Shift and the platform's primary modifier
    /// (Cmd on a Mac, else Ctrl) as given; answers whether the editor
    /// consumed it.
    fn enter(&self, shift: bool, primary: bool) -> bool {
        self.key("Enter", "Enter", shift, primary)
    }

    /// A `keydown` of `key` / `code` with Shift and the platform's primary
    /// modifier as given, on the capture textarea; answers whether the editor
    /// consumed it (`preventDefault`ed).
    fn key(&self, key: &str, code: &str, shift: bool, primary: bool) -> bool {
        // The platform test `rinch-web` makes (`navigator.platform`).
        let mac = js_sys::Reflect::get(&web_sys::window().unwrap(), &"navigator".into())
            .and_then(|n| js_sys::Reflect::get(&n, &"platform".into()))
            .ok()
            .and_then(|p| p.as_string())
            .is_some_and(|p| p.starts_with("Mac"));
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(key);
        init.set_code(code);
        init.set_shift_key(shift);
        init.set_ctrl_key(primary && !mac);
        init.set_meta_key(primary && mac);
        let ev =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        self.capture().dispatch_event(&ev).unwrap();
        ev.default_prevented()
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

const DOC: &str = "<p>x</p><pre>abcd</pre><p>y</p>";

fn b(tag: &str, text: &str) -> (String, String) {
    (tag.to_string(), text.to_string())
}

/// Enter and Shift+Enter `keydown`s in `ab|cd` type a newline into the code
/// block; the page's `<pre>` shows it and the caret is after it.
#[wasm_bindgen_test]
fn enter_and_shift_enter_keys_type_a_newline_in_a_code_block() {
    for shift in [false, true] {
        let f = Fixture::focused(DOC);
        assert_eq!(
            f.blocks(),
            [b("p", "x"), b("pre", "abcd"), b("p", "y")],
            "positive control: the page shows the document"
        );
        f.handle.set_selection(Selection::cursor(Pos(6)));
        assert!(f.enter(shift, false), "shift {shift}: consumed");
        assert_eq!(
            f.blocks(),
            [b("p", "x"), b("pre", "ab\ncd"), b("p", "y")],
            "shift {shift}"
        );
        assert_eq!(
            f.handle.selection(),
            Selection::cursor(Pos(7)),
            "shift {shift}"
        );
        f.teardown();
    }
}

/// A soft keyboard's Enter and Shift+Enter arrive as `beforeinput`s, which
/// run the same commands.
#[wasm_bindgen_test]
fn a_soft_keyboards_enter_types_a_newline_in_a_code_block() {
    for input_type in ["insertParagraph", "insertLineBreak"] {
        let f = Fixture::focused(DOC);
        f.handle.set_selection(Selection::cursor(Pos(6)));
        assert!(f.before_input(input_type), "{input_type}: taken");
        assert_eq!(
            f.blocks(),
            [b("p", "x"), b("pre", "ab\ncd"), b("p", "y")],
            "{input_type}"
        );
        f.teardown();
    }
}

/// Mod+Enter leaves the code block for an empty paragraph after it.
#[wasm_bindgen_test]
fn mod_enter_leaves_a_code_block_for_a_paragraph() {
    let f = Fixture::focused(DOC);
    f.handle.set_selection(Selection::cursor(Pos(6)));
    assert!(f.enter(false, true), "consumed");
    assert_eq!(
        f.blocks(),
        [b("p", "x"), b("pre", "abcd"), b("p", ""), b("p", "y")]
    );
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(10)));
    f.teardown();
}

/// Outside a code block Enter still splits the paragraph (the control).
#[wasm_bindgen_test]
fn enter_in_a_paragraph_still_splits_it() {
    let f = Fixture::focused(DOC);
    f.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(f.enter(false, false));
    assert_eq!(
        f.blocks(),
        [b("p", "x"), b("p", ""), b("pre", "abcd"), b("p", "y")]
    );
    f.teardown();
}

/// #2993's review: Ctrl/Cmd+Enter in a paragraph, where `exitCode` does not
/// apply, is not the editor's — as in ProseMirror, an inapplicable binding
/// leaves its key alone. It is not `preventDefault`ed, it changes nothing, and
/// it reaches the app's keyboard interceptor and a page `keydown` listener
/// (an app's "send" chord). At 5f10b1475 the matched binding was consumed: the
/// capture listener stopped it before either heard it.
#[wasm_bindgen_test]
fn mod_enter_outside_a_code_block_reaches_the_page() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;

    let f = Fixture::focused(DOC);
    let page_heard = Rc::new(RefCell::new(Vec::<(bool, bool)>::new()));
    let ph = page_heard.clone();
    let listener =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Enter" {
                ph.borrow_mut()
                    .push((ev.ctrl_key() || ev.meta_key(), ev.default_prevented()));
            }
        });
    document()
        .add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
        .unwrap();
    let intercepted = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = intercepted.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        seen.borrow_mut().push(k.key.clone());
        false
    });

    // In the paragraph: falls through.
    f.handle.set_selection(Selection::cursor(Pos(2)));
    let consumed = f.enter(false, true);
    let blocks = f.blocks();
    let heard = page_heard.borrow().clone();
    let interceptor_heard = intercepted.borrow().clone();

    // The control: in the code block the binding applies and is consumed, so
    // neither hears it (and a positive control that the listener is live).
    page_heard.borrow_mut().clear();
    intercepted.borrow_mut().clear();
    f.handle.set_selection(Selection::cursor(Pos(6)));
    let consumed_in_code = f.enter(false, true);
    let heard_in_code = page_heard.borrow().clone();
    let interceptor_heard_in_code = intercepted.borrow().clone();

    rinch_core::clear_keyboard_interceptor();
    document()
        .remove_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
        .unwrap();
    f.teardown();

    assert!(
        !consumed,
        "Mod+Enter in a paragraph is not preventDefaulted"
    );
    assert_eq!(
        blocks,
        [b("p", "x"), b("pre", "abcd"), b("p", "y")],
        "and changes nothing"
    );
    assert_eq!(heard, [(true, false)], "a page keydown listener hears it");
    assert_eq!(
        interceptor_heard,
        ["Enter"],
        "the app's interceptor hears it"
    );
    assert!(consumed_in_code, "control: consumed in the code block");
    assert!(heard_in_code.is_empty(), "{heard_in_code:?}");
    assert!(
        interceptor_heard_in_code.is_empty(),
        "{interceptor_heard_in_code:?}"
    );
}

/// #2993's second review: Ctrl/Cmd+Shift+R on a paragraph that is already
/// right-aligned (`setTextAlignRight` answers `false`) is still the editor's
/// key — left to Chrome it hard-reloads the page and the unsaved edits are
/// lost. It is `preventDefault`ed and a page `keydown` listener never hears
/// it. At b7d5350ad (every binding but a declared few fell through where
/// inapplicable) the second press was left to the browser.
#[wasm_bindgen_test]
fn ctrl_shift_r_on_a_right_aligned_paragraph_is_the_editors() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::closure::Closure;

    let f = Fixture::focused(DOC);
    let page_heard = Rc::new(RefCell::new(0usize));
    let ph = page_heard.clone();
    let listener =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            if ev.code() == "KeyR" {
                *ph.borrow_mut() += 1;
            }
        });
    document()
        .add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
        .unwrap();
    f.handle.set_selection(Selection::cursor(Pos(1)));
    let before = f.handle.doc();
    // The control: the first press applies (the paragraph becomes right-aligned).
    let first = f.key("R", "KeyR", true, true);
    let aligned = !f.handle.doc().same_ref(&before);
    let after_first = f.handle.doc();
    // Now the command has nothing to do.
    let second = f.key("R", "KeyR", true, true);
    let unchanged = f.handle.doc().same_ref(&after_first);
    let heard = *page_heard.borrow();
    document()
        .remove_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref())
        .unwrap();
    let blocks = f.blocks();
    f.teardown();

    assert!(first && aligned, "control: the first press aligns right");
    assert!(
        second,
        "the second press is consumed although it does nothing"
    );
    assert!(unchanged, "and changes nothing");
    assert_eq!(heard, 0, "the page hears neither press");
    assert_eq!(blocks, [b("p", "x"), b("pre", "abcd"), b("p", "y")]);
}

/// Mod+E where `toggleCode` refuses — a read-only editor — is still the
/// editor's key; left to Chrome, Ctrl+E focuses the address bar. At
/// b7d5350ad it was not `preventDefault`ed. In a code block it is consumed
/// too (there `toggleCode` applies: rinch's mark toggles do not check the
/// parent's allowed marks).
#[wasm_bindgen_test]
fn mod_e_in_a_read_only_editor_or_a_code_block_is_the_editors() {
    let f = Fixture::focused(DOC);
    f.handle.set_selection(Selection::text(Pos(4), Pos(6)));
    let in_code = f.key("e", "KeyE", false, true);
    f.handle.set_read_only(true);
    let before = f.handle.doc();
    f.handle.set_selection(Selection::text(Pos(1), Pos(2)));
    let read_only = f.key("e", "KeyE", false, true);
    let unchanged = f.handle.doc().same_ref(&before);
    // The control: editable again, the same chord applies in the paragraph.
    f.handle.set_read_only(false);
    let editable = f.key("e", "KeyE", false, true);
    let toggled = !f.handle.doc().same_ref(&before);
    f.teardown();
    assert!(in_code, "Mod+E in a code block is preventDefaulted");
    assert!(read_only, "Mod+E in a read-only editor is preventDefaulted");
    assert!(unchanged, "and changes nothing");
    assert!(editable && toggled, "control: applies when editable");
}
