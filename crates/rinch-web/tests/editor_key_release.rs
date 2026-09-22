//! Browser-driven tests for the web editor *releasing* the keyboard (issue #271,
//! from the review of PR #862).
//!
//! The editor owns a key only while its capture textarea holds focus. When focus
//! leaves that textarea for somewhere else in the page — a `<button>`, or `<body>`
//! after a press on blank page — the editor lets go of the keyboard **and** stops
//! painting its caret, which is what desktop's focus arbiter does
//! (`focus.rs`: a blurred `FocusTarget::Editor` calls `hide_overlays`). A caret
//! blinking in an editor that ignores typing is the one state neither backend
//! may show. Two editors and a chord pin that the rule does not reach the
//! editor that does hold focus.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const MARK: &str = "data-test-host-review-862";

struct F {
    root: RootHandle,
    host: web_sys::Element,
    a: EditorHandle,
    b: EditorHandle,
}

fn mount() -> F {
    if let Ok(stale) = document().query_selector_all(&format!("[{MARK}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i)
                && let Ok(el) = n.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
    rinch_web::__reset_activation_state();
    let host = document().create_element("div").unwrap();
    host.set_attribute(MARK, "").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let a = create_editor();
    assert!(a.load_html("<p>Alpha text one</p>"));
    let b = create_editor();
    assert!(b.load_html("<p>Beta text two</p>"));
    let (ca, cb) = (a.clone(), b.clone());
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| build(scope, ca, cb),
    );
    F { root, host, a, b }
}

fn build(scope: &mut RenderScope, a: EditorHandle, b: EditorHandle) -> NodeHandle {
    let w = scope.create_element("div");
    for (h, id) in [(&a, "ed-a"), (&b, "ed-b")] {
        let wrap = scope.create_element("div");
        wrap.set_attribute("id", id);
        wrap.append_child(&h.mount(scope));
        w.append_child(&wrap);
    }
    let btn = scope.create_element("button");
    btn.set_attribute("id", "plain-btn");
    btn.append_child(&scope.create_text("plain"));
    w.append_child(&btn);
    w
}

impl F {
    fn editor_el(&self, id: &str) -> web_sys::Element {
        document()
            .query_selector(&format!("#{id} [data-pm-editor]"))
            .unwrap()
            .unwrap()
    }
    fn text(&self, id: &str) -> String {
        self.editor_el(id).text_content().unwrap_or_default()
    }
    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }
    fn caret_visible(&self, id: &str) -> bool {
        match self
            .editor_el(id)
            .query_selector("[data-pm-caret]")
            .unwrap()
        {
            Some(c) => {
                let st = web_sys::window()
                    .unwrap()
                    .get_computed_style(&c)
                    .unwrap()
                    .unwrap();
                st.get_property_value("display").unwrap() != "none"
                    && c.get_client_rects().length() > 0
            }
            None => false,
        }
    }
    fn press(&self, id: &str) {
        let p = self.editor_el(id).query_selector("p").unwrap().unwrap();
        let t = p.first_child().unwrap();
        let r = document().create_range().unwrap();
        r.set_start(&t, 2).unwrap();
        r.set_end(&t, 3).unwrap();
        let b = r.get_bounding_client_rect();
        let (x, y) = (
            (b.x() + b.width() / 2.0) as f32,
            (b.y() + b.height() / 2.0) as f32,
        );
        mouse("mousedown", x, y);
        mouse("mouseup", x, y);
        assert!(
            document().active_element().as_deref() == Some(self.capture().as_ref()),
            "positive control: the press focused the capture textarea"
        );
    }
    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse(name: &str, x: f32, y: f32) {
    let target = document().element_from_point(x, y).unwrap();
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

fn key(el: &web_sys::EventTarget, k: &str, code: &str, ctrl: bool) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(k);
    init.set_code(code);
    init.set_ctrl_key(ctrl);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

/// Positive: the second editor pressed owns the keys; the first is untouched.
#[wasm_bindgen_test]
fn two_editors_the_last_pressed_owns_the_key() {
    let f = mount();
    f.press("ed-a");
    f.press("ed-b");
    let (ta, tb) = (f.text("ed-a"), f.text("ed-b"));
    assert!(key(f.capture().as_ref(), "q", "KeyQ", false));
    assert_eq!(f.text("ed-a"), ta);
    assert_ne!(f.text("ed-b"), tb);
    f.teardown();
}

/// Positive: a chord on the focused capture textarea still reaches the keymap.
#[wasm_bindgen_test]
fn ctrl_b_on_the_focused_editor_toggles_bold() {
    let f = mount();
    f.press("ed-a");
    assert!(!f.a.is_mark_active("bold"));
    assert!(key(f.capture().as_ref(), "b", "KeyB", true));
    assert!(
        f.a.is_mark_active("bold"),
        "Ctrl+B toggled the stored bold mark"
    );
    let _ = &f.b;
    f.teardown();
}

/// Desktop hides a blurred editor's caret (`focus.rs`,
/// `FocusTarget::Editor(prev) => handle.hide_overlays()`). On the web, once focus
/// moves to a button the editor no longer owns the keyboard, so its caret goes.
#[wasm_bindgen_test]
fn focus_moving_to_a_button_hides_the_editor_caret() {
    let f = mount();
    f.press("ed-a");
    assert!(
        f.caret_visible("ed-a"),
        "positive control: caret painted while focused"
    );
    let btn: web_sys::HtmlElement = document()
        .get_element_by_id("plain-btn")
        .unwrap()
        .dyn_into()
        .unwrap();
    btn.focus().unwrap();
    assert!(
        !f.caret_visible("ed-a"),
        "the editor no longer owns the keyboard but still paints its caret"
    );
    f.teardown();
}

/// A press on blank page moves focus to `<body>` natively; synthetic mousedowns
/// do not, so blur the textarea as the browser would. The editor then neither
/// takes the key nor paints a caret — desktop's blur. (A press on a
/// *non-focusable* `data-rid` never gets here: it keeps the textarea focused,
/// pinned in `editor_key_ownership.rs`.)
#[wasm_bindgen_test]
fn after_focus_leaves_to_body_the_editor_neither_types_nor_paints_a_caret() {
    let f = mount();
    f.press("ed-a");
    let before = f.text("ed-a");
    f.capture().blur().unwrap();
    let body = document().body().unwrap();
    assert!(document().active_element().as_deref() == Some(body.as_ref()));
    key(body.as_ref(), "q", "KeyQ", false);
    let typed = f.text("ed-a") != before;
    let caret = f.caret_visible("ed-a");
    assert!(!typed, "a key on <body> is not the editor's");
    assert!(
        !caret,
        "an editor that does not own the keyboard paints no caret"
    );
    f.teardown();
}

/// Window blur retains (desktop #226: `WindowFocus(false)` notifies but keeps the
/// claim). Alt-tabbing away blurs the capture textarea while it stays the
/// document's focused element, and alt-tabbing back returns focus to it — so the
/// editor must not let go on such a blur. Modelled by a `blur` delivered to the
/// textarea while it is still `document.activeElement`, which is the state an
/// alt-tab leaves (a real window switch cannot be driven from a test).
#[wasm_bindgen_test]
fn a_blur_that_leaves_the_textarea_focused_keeps_the_editor() {
    let f = mount();
    f.press("ed-a");
    let before = f.text("ed-a");
    let ev = web_sys::Event::new("blur").unwrap();
    f.capture().dispatch_event(&ev).unwrap();
    assert!(
        document().active_element().as_deref() == Some(f.capture().as_ref()),
        "the textarea is still the focused element"
    );
    assert!(f.caret_visible("ed-a"), "the caret is still painted");
    assert!(key(f.capture().as_ref(), "q", "KeyQ", false));
    assert_ne!(f.text("ed-a"), before, "and the editor still takes the key");
    f.teardown();
}
