//! The rich-text editor's keyboard focus and the overlay focus rules (issue
//! #1126), in a real browser.
//!
//! On the web an editor holds the keyboard through one hidden capture
//! `<textarea data-pm-capture>` appended to `<body>`, so `document.activeElement`
//! is never inside the editor's container. Every question rinch asks of the
//! focus — "is it inside this overlay" (`restore_focus`), "who holds it"
//! (`NodeHandle::active_element`, which key entries and the opener memory read),
//! "is it inside the part a removal takes" (`release_focus_within`) — has to
//! read the capture field as the editor's container. (Tab containment never
//! sees a Tab from an editor: its keymap consumes Tab.) Before the fix it read
//! as "focus is outside": closing a
//! `trap_focus` `Modal` left the keyboard on the hidden editor and never gave
//! it back to the opener.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test overlay_editor_focus_1126
//! ```
//!
//! The desktop twin is `rinch/src/app/overlay_focus_tests.rs`'s `editor_*`
//! fixtures.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::components::Modal;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal, show_dom};
use rinch_web::{EditorHandle, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST: &str = "data-test-host-1126";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn capture_active() -> bool {
    document()
        .active_element()
        .is_some_and(|el| el.has_attribute("data-pm-capture"))
}

fn active_desc() -> String {
    document()
        .active_element()
        .map(|el| {
            if el.has_attribute("data-pm-capture") {
                "<capture>".to_string()
            } else if !el.id().is_empty() {
                el.id()
            } else {
                format!("<{}>", el.tag_name().to_lowercase())
            }
        })
        .unwrap_or_else(|| "<none>".into())
}

fn mouse(name: &str, x: f64, y: f64) {
    let target = document()
        .element_from_point(x as f32, y as f32)
        .expect("hit");
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

/// A press into the editor's first paragraph, and the positive control that
/// it reached rinch: the capture field holds the keyboard.
fn press_editor(host: &str) {
    let p = document()
        .query_selector(&format!("#{host} [data-pm-editor] p"))
        .unwrap()
        .expect("editor paragraph");
    let r = p.get_bounding_client_rect();
    mouse("mousedown", r.x() + 2.0, r.y() + 12.0);
    mouse("mouseup", r.x() + 2.0, r.y() + 12.0);
    assert!(
        capture_active(),
        "positive control: the press focused the editor, active = {}",
        active_desc()
    );
}

/// A Tab keydown where the browser would send it: at the focused element.
fn tab(shift: bool) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Tab");
    init.set_code("Tab");
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    document()
        .active_element()
        .expect("something focused")
        .dispatch_event(&ev)
        .unwrap();
}

/// Whether the editor draws its caret — it does only while it is the
/// focused editor.
fn caret_visible(host: &str) -> bool {
    let Some(c) = document()
        .query_selector(&format!("#{host} [data-pm-editor] [data-pm-caret]"))
        .unwrap()
    else {
        return false;
    };
    let st = web_sys::window()
        .unwrap()
        .get_computed_style(&c)
        .unwrap()
        .unwrap();
    st.get_property_value("display").unwrap() != "none"
        && st.get_property_value("visibility").unwrap() == "visible"
        && c.get_client_rects().length() > 0
}

async fn microtask() {
    let p = js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL);
    wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
}

fn purge() {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
    if let Some(a) = document()
        .active_element()
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
    {
        a.blur().ok();
    }
}

fn host(id: &str) -> web_sys::Element {
    purge();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST, "").unwrap();
    host.set_id(id);
    host.set_attribute(
        "style",
        "font-family: monospace; font-size: 16px; line-height: 24px; padding: 20px; width: 400px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

fn editor(html: &str) -> EditorHandle {
    let handle = create_editor();
    assert!(handle.load_html(html));
    handle
}

fn button(s: &mut RenderScope, id: &str) -> NodeHandle {
    let b = s.create_element("button");
    b.set_attribute("id", id);
    b.set_attribute("style", "display:block;width:100px;height:20px");
    b
}

fn input(s: &mut RenderScope, id: &str) -> NodeHandle {
    let i = s.create_element("input");
    i.set_attribute("id", id);
    i.set_attribute("style", "display:block;width:120px;height:28px");
    i
}

fn modal(s: &mut RenderScope, open: Signal<bool>, children: &[NodeHandle]) -> NodeHandle {
    Modal {
        opened_fn: Some(Rc::new(move || open.get())),
        trap_focus: true,
        with_close_button: false,
        ..Default::default()
    }
    .render(s, children)
}

fn focus_opener() {
    document()
        .get_element_by_id("opener")
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .focus()
        .unwrap();
    assert_eq!(active_desc(), "opener");
}

/// The issue's case: the opener holds the keyboard, a `trap_focus` modal
/// holding an editor opens, a press puts the keyboard in the editor, the modal
/// closes. The keyboard goes back to the opener and the editor lets it go.
#[wasm_bindgen_test]
async fn closing_the_modal_hands_the_keyboard_from_its_editor_to_the_opener() {
    let h = host("h1");
    let open = Signal::new(false);
    let m = editor("<p>modal</p>");
    let root: RootHandle = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            page.append_child(&button(s, "opener"));
            let ed = m.mount(s);
            let dialog = modal(s, open, &[ed]);
            page.append_child(&dialog);
            page
        },
    );
    focus_opener();
    open.set(true);
    press_editor("h1");
    assert!(caret_visible("h1"));
    open.set(false);
    microtask().await;
    assert_eq!(active_desc(), "opener", "the opener got the keyboard back");
    assert!(
        !caret_visible("h1"),
        "the closed editor let the keyboard go"
    );
    root.unmount();
    purge();
}

/// The same, but the open modal is unmounted by a branch rather than closed.
#[wasm_bindgen_test]
async fn unmounting_the_open_modal_hands_the_keyboard_from_its_editor_to_the_opener() {
    let h = host("h2");
    let open = Signal::new(false);
    let shown = Signal::new(true);
    let m = editor("<p>modal</p>");
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            page.append_child(&button(s, "opener"));
            let m = m.clone();
            show_dom(
                s,
                &page,
                move || shown.get(),
                move |s: &mut RenderScope| {
                    let ed = m.mount(s);
                    modal(s, open, &[ed])
                },
                None::<fn(&mut RenderScope) -> NodeHandle>,
            );
            page
        },
    );
    focus_opener();
    open.set(true);
    press_editor("h2");
    shown.set(false);
    microtask().await;
    assert_eq!(active_desc(), "opener", "the opener got the keyboard back");
    root.unmount();
    purge();
}

/// Tab inside a trap stays the editor's: its keymap binds Tab (to
/// `sinkListItem`) and consumes it even where that does nothing, so the trap's
/// own Tab handling never sees a Tab from a focused editor and needs no reading
/// of the capture field. This pins that premise; if the editor ever lets Tab
/// go, the trap must learn to count the capture field as the editor too.
#[wasm_bindgen_test]
async fn tab_in_an_editor_inside_a_trap_stays_with_the_editor() {
    let h = host("h3");
    let open = Signal::new(false);
    let m = editor("<p>modal</p>");
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let before = button(s, "before");
            let ed = m.mount(s);
            let after = input(s, "after");
            let dialog = modal(s, open, &[before, ed, after]);
            page.append_child(&dialog);
            page
        },
    );
    open.set(true);
    press_editor("h3");
    tab(false);
    assert!(capture_active(), "Tab moved to {}", active_desc());
    tab(true);
    assert!(capture_active(), "Shift+Tab moved to {}", active_desc());
    open.set(false);
    root.unmount();
    purge();
}

/// `NodeHandle::active_element` (what an overlay remembers as its opener, and
/// what key entries ask) names the focused editor's container.
#[wasm_bindgen_test]
async fn active_element_names_the_focused_editors_container() {
    let h = host("h4");
    let m = editor("<p>one</p>");
    let held: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::default();
    let keep = held.clone();
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let ed = m.mount(s);
            page.append_child(&ed);
            *keep.borrow_mut() = Some((page.clone(), ed));
            page
        },
    );
    press_editor("h4");
    let (page, ed) = held.borrow().clone().unwrap();
    assert_eq!(
        page.active_element().map(|n| n.node_id()),
        Some(ed.node_id()),
        "the capture field reads as the editor"
    );
    root.unmount();
    purge();
}

/// A modal opened while an editor held the keyboard gives the keyboard back
/// to that editor when it closes, as a browser restores a dialog's opener.
#[wasm_bindgen_test]
async fn closing_a_modal_opened_from_an_editor_gives_the_editor_the_keyboard() {
    let h = host("h5");
    let open = Signal::new(false);
    let m = editor("<p>page editor</p>");
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let ed = m.mount(s);
            page.append_child(&ed);
            let field = input(s, "in-modal");
            let dialog = modal(s, open, &[field]);
            page.append_child(&dialog);
            page
        },
    );
    press_editor("h5");
    open.set(true);
    assert_eq!(active_desc(), "in-modal", "the modal took the keyboard");
    open.set(false);
    microtask().await;
    assert!(
        capture_active(),
        "the editor got the keyboard back, active = {}",
        active_desc()
    );
    assert!(caret_visible("h5"));
    root.unmount();
    purge();
}

/// Taking the part of the page that holds the focused editor out of the
/// document releases the editor, as a browser blurs a focused element it
/// removes.
#[wasm_bindgen_test]
async fn removing_the_part_holding_the_focused_editor_releases_it() {
    let h = host("h6");
    let m = editor("<p>one</p>");
    let held: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let keep = held.clone();
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let wrap = s.create_element("div");
            wrap.append_child(&m.mount(s));
            page.append_child(&wrap);
            *keep.borrow_mut() = Some(wrap);
            page
        },
    );
    press_editor("h6");
    let wrap = held.borrow().clone().unwrap();
    wrap.remove();
    assert!(
        !capture_active(),
        "the removed editor still holds the keyboard"
    );
    root.unmount();
    purge();
}
