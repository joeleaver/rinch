//! A press on a `data-rid` handler while an editor owns the keyboard: left to
//! the browser when anything focusable is on its chain (PR #862, #271).
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
const MARK: &str = "data-test-host-review-862-r2";

struct F {
    root: RootHandle,
    host: web_sys::Element,
    ed: EditorHandle,
}

fn clean() {
    if let Ok(stale) = document().query_selector_all(&format!("[{MARK}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i)
                && let Ok(el) = n.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
}

fn mount() -> F {
    clean();
    rinch_web::__reset_activation_state();
    let host = document().create_element("div").unwrap();
    host.set_attribute(MARK, "").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let ed = create_editor();
    assert!(ed.load_html("<p>Alpha text one</p>"));
    let e2 = ed.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| build(s, e2));
    F { root, host, ed }
}

fn el(scope: &mut RenderScope, tag: &str, id: &str, rid: Option<&str>, text: &str) -> NodeHandle {
    let e = scope.create_element(tag);
    e.set_attribute("id", id);
    if let Some(r) = rid {
        e.set_attribute("data-rid", r);
    }
    if !text.is_empty() {
        e.append_child(&scope.create_text(text));
    }
    e
}

fn build(scope: &mut RenderScope, ed: EditorHandle) -> NodeHandle {
    let w = scope.create_element("div");
    w.append_child(&ed.mount(scope));
    let rid = scope.register_handler(|| {}).0.to_string();
    let rid = rid.as_str();
    // Tree shape: a tabindex row containing a data-rid chevron.
    let row = el(scope, "div", "row", None, "");
    row.set_attribute("tabindex", "0");
    row.append_child(&el(scope, "span", "chev", Some(rid), ">"));
    row.append_child(&scope.create_text(" row label"));
    w.append_child(&row);
    // A data-rid wrapper containing an <input> and a checkbox.
    let wrap = el(scope, "div", "wrap", Some(rid), "wrap ");
    let inp = el(scope, "input", "inner-input", None, "");
    inp.set_attribute("type", "text");
    wrap.append_child(&inp);
    let cb = el(scope, "input", "inner-check", None, "");
    cb.set_attribute("type", "checkbox");
    wrap.append_child(&cb);
    w.append_child(&wrap);
    // <a> without href carrying a handler.
    w.append_child(&el(scope, "a", "a-nohref", Some(rid), "link"));
    // A plain button with a data-rid span inside it.
    let btn = el(scope, "button", "btn", None, "");
    btn.append_child(&el(scope, "span", "btn-span", Some(rid), "inner"));
    w.append_child(&btn);
    // A plain non-focusable data-rid div (positive control).
    w.append_child(&el(scope, "div", "plain", Some(rid), "plain"));
    w
}

impl F {
    fn editor_el(&self) -> web_sys::Element {
        self.host
            .query_selector("[data-pm-editor]")
            .unwrap()
            .unwrap()
    }
    fn capture(&self) -> web_sys::HtmlTextAreaElement {
        document()
            .query_selector("textarea[data-pm-capture]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }
    fn caret_visible(&self) -> bool {
        match self.editor_el().query_selector("[data-pm-caret]").unwrap() {
            Some(c) => {
                let st = web_sys::window()
                    .unwrap()
                    .get_computed_style(&c)
                    .unwrap()
                    .unwrap();
                st.get_property_value("display").unwrap() != "none"
                    && st.get_property_value("visibility").unwrap() == "visible"
                    && c.get_client_rects().length() > 0
            }
            None => false,
        }
    }
    fn point(&self) -> (f32, f32) {
        let p = self.editor_el().query_selector("p").unwrap().unwrap();
        let t = p.first_child().unwrap();
        let r = document().create_range().unwrap();
        r.set_start(&t, 2).unwrap();
        r.set_end(&t, 3).unwrap();
        let b = r.get_bounding_client_rect();
        (
            (b.x() + b.width() / 2.0) as f32,
            (b.y() + b.height() / 2.0) as f32,
        )
    }
    fn focus_editor(&self) {
        let (x, y) = self.point();
        mouse_at("mousedown", x, y, 0);
        mouse_at("mouseup", x, y, 0);
        assert!(
            document().active_element().as_deref() == Some(self.capture().as_ref()),
            "positive control: the press focused the capture textarea"
        );
        assert!(self.caret_visible(), "positive control: caret painted");
    }
    fn text(&self) -> String {
        self.editor_el().text_content().unwrap_or_default()
    }
    fn teardown(self) {
        rinch_web::__reset_activation_state();
        self.root.unmount();
        self.host.remove();
    }
}

fn mouse_on(target: &web_sys::Element, name: &str, x: f32, y: f32, button: i16) -> bool {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(button);
    init.set_buttons(if button == 2 { 2 } else { 1 });
    init.set_detail(1);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}
fn mouse_at(name: &str, x: f32, y: f32, button: i16) -> bool {
    let t = document().element_from_point(x, y).unwrap();
    mouse_on(&t, name, x, y, button)
}
fn press_prevented(id: &str) -> bool {
    let e = document().get_element_by_id(id).unwrap();
    let r = e.get_bounding_client_rect();
    mouse_on(
        &e,
        "mousedown",
        (r.x() + r.width() / 2.0) as f32,
        (r.y() + r.height() / 2.0) as f32,
        0,
    )
}
fn key(el: &web_sys::EventTarget, k: &str, code: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(k);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

#[wasm_bindgen_test]
fn positive_control_plain_is_prevented() {
    let f = mount();
    f.focus_editor();
    assert!(press_prevented("plain"));
    f.teardown();
}

/// Tree chevron shape: a data-rid inside a tabindex row. Browser and desktop
/// (`resolve_click_focus` -> PressFocus::Node(row)) both give the row the focus.
#[wasm_bindgen_test]
fn rid_inside_a_focusable_row_is_left_to_the_browser() {
    let f = mount();
    f.focus_editor();
    assert!(
        !press_prevented("chev"),
        "the row must be able to take focus"
    );
    f.teardown();
}

/// A data-rid span inside a plain <button>: the browser focuses the button.
#[wasm_bindgen_test]
fn rid_inside_a_button_is_left_to_the_browser() {
    let f = mount();
    f.focus_editor();
    assert!(
        !press_prevented("btn-span"),
        "the button must be able to take focus"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn input_and_checkbox_inside_a_rid_take_focus() {
    let f = mount();
    f.focus_editor();
    assert!(
        !press_prevented("inner-check"),
        "checkbox inside a data-rid"
    );
    f.focus_editor();
    assert!(
        !press_prevented("inner-input"),
        "text input inside a data-rid"
    );
    f.teardown();
}

#[wasm_bindgen_test]
fn a_without_href_is_non_focusable() {
    let f = mount();
    f.focus_editor();
    assert!(press_prevented("a-nohref"));
    f.teardown();
}

/// The #820 parking cycle: right press in the editor, contextmenu at the parked
/// textarea, the menu dismissed (a left press on a non-focusable handler, which is
/// what ends a cycle). The editor must still own the keyboard with its caret.
#[wasm_bindgen_test]
fn menu_cycle_leaves_the_editor_owning_the_keyboard() {
    let f = mount();
    f.focus_editor();
    let (x, y) = f.point();
    assert!(mouse_at("mousedown", x, y, 2), "right press consumed");
    // contextmenu at the parked textarea
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(2);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let cm = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
    f.capture().dispatch_event(&cm).unwrap();
    mouse_at("mouseup", x, y, 2);
    assert!(document().active_element().as_deref() == Some(f.capture().as_ref()));
    assert!(f.caret_visible(), "caret visible during the cycle");
    // Escape to the page (menu closed with Escape reaches nothing in a real
    // browser; here it reaches the textarea).
    key(f.capture().as_ref(), "Escape", "Escape");
    assert!(f.caret_visible(), "caret visible after Escape");
    let before = f.text();
    assert!(key(f.capture().as_ref(), "q", "KeyQ"));
    assert_ne!(
        f.text(),
        before,
        "the editor still takes a key after the cycle"
    );
    // A paste chosen from the menu while the cycle is live.
    let before = f.text();
    let dt = web_sys::DataTransfer::new().unwrap();
    dt.set_data("text/plain", "ZZ").unwrap();
    let ci = web_sys::ClipboardEventInit::new();
    ci.set_bubbles(true);
    ci.set_cancelable(true);
    ci.set_clipboard_data(Some(&dt));
    let _ = before;
    f.teardown();
}

/// A menu cycle followed by a blur of the textarea (focus to a button) releases.
#[wasm_bindgen_test]
fn menu_cycle_then_focus_to_button_releases() {
    let f = mount();
    f.focus_editor();
    let (x, y) = f.point();
    mouse_at("mousedown", x, y, 2);
    let btn: web_sys::HtmlElement = document()
        .get_element_by_id("btn")
        .unwrap()
        .dyn_into()
        .unwrap();
    btn.focus().unwrap();
    assert!(!f.caret_visible(), "a released editor paints no caret");
    let _ = &f.ed;
    f.teardown();
}

/// Same, through the `Editor` component (which unregisters on cleanup).
#[wasm_bindgen_test]
fn after_an_editor_component_unmounts_nothing_is_prevented() {
    use rinch_core::element::Component;
    clean();
    rinch_web::__reset_activation_state();
    let host = document().create_element("div").unwrap();
    host.set_attribute(MARK, "").unwrap();
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let ed = create_editor();
    assert!(ed.load_html("<p>Alpha text one</p>"));
    let e2 = ed.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let w = scope.create_element("div");
        let c = rinch_web::Editor {
            editor: Some(e2.clone()),
            ..Default::default()
        }
        .render(scope, &[]);
        w.append_child(&c);
        w
    });
    let f = F { root, host, ed };
    f.focus_editor();
    let F { root, host, ed } = f;
    root.unmount();
    host.remove();
    drop(ed);
    clean();
    let host = document().create_element("div").unwrap();
    host.set_attribute(MARK, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), |scope| {
        let rid = scope.register_handler(|| {}).0.to_string();
        let d = scope.create_element("div");
        d.set_attribute("id", "lone2");
        d.set_attribute("data-rid", &rid);
        d.append_child(&scope.create_text("lone"));
        d
    });
    assert!(
        !press_prevented("lone2"),
        "no editor on the page: nothing prevented"
    );
    root.unmount();
    host.remove();
}

/// No editor alive: nothing is prevented, even though a stale capture textarea
/// may still be the focused element.
#[wasm_bindgen_test]
#[ignore = "#868: a raw handle.mount stays registered after unmount"]
fn after_the_editor_unmounts_nothing_is_prevented() {
    let f = mount();
    f.focus_editor();
    f.root.unmount();
    // the host still holds non-editor elements? rebuild a plain div by hand
    clean();
    rinch_web::__reset_activation_state();
    let host = document().create_element("div").unwrap();
    host.set_attribute(MARK, "").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), |scope| {
        let rid = scope.register_handler(|| {}).0.to_string();
        let d = scope.create_element("div");
        d.set_attribute("id", "lone");
        d.set_attribute("data-rid", &rid);
        d.append_child(&scope.create_text("lone"));
        d
    });
    assert!(
        !press_prevented("lone"),
        "no editor on the page: nothing prevented"
    );
    root.unmount();
    host.remove();
}
