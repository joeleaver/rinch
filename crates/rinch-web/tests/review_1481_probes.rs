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

const HOST_MARKER: &str = "data-test-host-review-1481";

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

// ── review probes ────────────────────────────────────────────────────────────

fn capture() -> web_sys::HtmlTextAreaElement {
    document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .expect("capture textarea")
        .dyn_into()
        .unwrap()
}

fn composition(name: &str, data: &str) {
    let init = web_sys::CompositionEventInit::new();
    init.set_bubbles(true);
    init.set_data(data);
    let ev = web_sys::CompositionEvent::new_with_event_init_dict(name, &init).unwrap();
    capture().dispatch_event(&ev).unwrap();
}

fn preedits() -> u32 {
    let all = document()
        .query_selector_all("[data-pm-editor] [data-pm-preedit]")
        .unwrap();
    (0..all.length())
        .filter_map(|i| all.item(i))
        .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
        .filter(|e| {
            !e.get_attribute("style")
                .is_some_and(|s| s.contains("display: none"))
        })
        .count() as u32
}

/// What a browser does when a field that is composing loses focus: it ends
/// the composition (`compositionend`) inside the focus change, before the
/// field's `blur` listeners run. A synthetic composition is not ended by the
/// browser, so this stands in for it: a capture listener on the document,
/// which runs before the textarea's own `blur` listener.
fn end_composition_on_blur(
    data: &'static str,
) -> wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)> {
    let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |e: web_sys::Event| {
            let ta = capture();
            let is_ta = e
                .target()
                .is_some_and(|t| t.dyn_ref::<web_sys::Element>() == Some(ta.as_ref()));
            if is_ta {
                composition("compositionend", data);
            }
        },
    );
    document()
        .add_event_listener_with_callback_and_bool("blur", cb.as_ref().unchecked_ref(), true)
        .unwrap();
    cb
}

/// W1 control: focus leaves a composing editor natively (another field is
/// focused): the composition's end reaches the editor, the preedit goes.
#[wasm_bindgen_test]
fn w1_control_a_native_blur_mid_composition_ends_the_preedit() {
    let f = Fixture::mount();
    focused_a(&f);
    composition("compositionstart", "");
    composition("compositionupdate", "nihon");
    assert!(preedits() > 0, "control: a preedit is shown");
    let cb = end_composition_on_blur("NIHON");
    f.input.focus().unwrap();
    document()
        .remove_event_listener_with_callback_and_bool("blur", cb.as_ref().unchecked_ref(), true)
        .unwrap();
    assert!(f.input_has_focus());
    assert_eq!(preedits(), 0, "the preedit went with the composition");
    console_log!("W1 control: A = {:?}", f.first_line(0));
    f.a.focus();
    assert_eq!(preedits(), 0, "control: and focus brings none back");
    f.teardown();
}

/// W1: the same through `EditorHandle::blur`.
#[wasm_bindgen_test]
fn w1_blur_mid_composition_leaves_no_preedit() {
    let f = Fixture::mount();
    focused_a(&f);
    composition("compositionstart", "");
    composition("compositionupdate", "nihon");
    assert!(preedits() > 0, "control: a preedit is shown");
    let cb = end_composition_on_blur("NIHON");
    f.a.blur();
    document()
        .remove_event_listener_with_callback_and_bool("blur", cb.as_ref().unchecked_ref(), true)
        .unwrap();
    assert!(!f.capture_has_focus());
    let left = preedits();
    console_log!(
        "W1 preedit nodes after blur(): {left}; A = {:?}",
        f.first_line(0)
    );
    assert_eq!(left, 0, "a blurred editor shows no preedit");
    // And the next composition anywhere is not confused by a stale flag: A
    // focused again types.
    f.a.focus();
    assert_eq!(
        preedits(),
        0,
        "the composition ended with the blur: focus brings no preedit back"
    );
    f.teardown();
}

/// W4: after the root unmounted, blur is quiet.
#[wasm_bindgen_test]
fn w4_blur_after_unmount_is_quiet() {
    let f = Fixture::mount();
    focused_a(&f);
    let (a, b) = (f.a.clone(), f.b.clone());
    f.input.focus().unwrap();
    let input = f.input.clone();
    LIVE_ROOT.with(|r| r.take());
    f.root.unmount();
    a.blur();
    b.blur();
    assert!(
        document().active_element().as_deref() == Some(input.as_ref()),
        "the page's field keeps the keyboard"
    );
    f.host.remove();
}

/// W5: blur() of the focused editor from a `focusout`-free place while the
/// editor's textarea is focused, called twice and around a selection change.
#[wasm_bindgen_test]
fn w5_blur_twice_and_selection_change_draws_no_caret() {
    let f = Fixture::mount();
    focused_a(&f);
    f.a.blur();
    f.a.blur();
    f.a.set_selection(Selection::cursor(Pos(2)));
    assert_eq!(f.carets_visible(), 0);
    f.teardown();
}
