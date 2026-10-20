//! Node views in a browser (PR B): an app's component drawn in an editor
//! document, through real DOM events.
//!
//! - a press on an atom's frame node-selects it — any selectable leaf atom, by
//!   the schema (an `embed` with no view included: review of #1565, F1);
//! - a press inside the component is the component's: not prevented, and the
//!   editor lets go of the keyboard;
//! - a key in the component reaches the view first; an Escape it leaves gives
//!   the keyboard back to the editor, caret after the node;
//! - a component that grows moves the caret below it (a `ResizeObserver` owes
//!   the editor an overlay pass).
//!
//! The desktop twin is `rinch/src/app/editor_node_view_tests.rs`; the view's
//! lifetime and identity rules are pinned in `rinch-editor-view/tests/node_views.rs`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_node_views
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::Signal;
use rinch_editor_core::{NodeSelection, Pos, Selection};
use rinch_web::{CaretFrom, EditorHandle, NodeViewSpec, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-node-views";

thread_local! {
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

/// `ab` 0..4, the viewed embed 4..5, `cd` 5..9, a plain embed 9..10, `ef` 10..14.
const DOC: &str = r#"<p>ab</p><div data-pm-type="embed" data-sheet="s1"></div><p>cd</p><div data-pm-type="embed" data-plain="1"></div><p>ef</p>"#;

#[derive(Default)]
struct View {
    renders: std::cell::Cell<usize>,
    input: RefCell<Option<NodeHandle>>,
    grow: RefCell<Option<Signal<u32>>>,
    keys: RefCell<Vec<String>>,
    entered: RefCell<Vec<CaretFrom>>,
}

struct Fixture {
    handle: EditorHandle,
    view: Rc<View>,
}

fn mount() -> Fixture {
    if let Some(stale) = LIVE_ROOT.with(|r| r.take()) {
        stale.unmount();
    }
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(el) = stale
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
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
    host.set_attribute(
        "style",
        "position: fixed; top: 0; left: 0; width: 500px; font-family: monospace; \
         font-size: 16px; line-height: 24px; padding: 20px; background: white; z-index: 10;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html(DOC));
    let view = Rc::new(View::default());
    let (v, k, e) = (view.clone(), view.clone(), view.clone());
    handle
        .register_node_view(
            NodeViewSpec::new("embed", move |s: &mut RenderScope, _ctx| {
                v.renders.set(v.renders.get() + 1);
                let grow = Signal::new(20u32);
                *v.grow.borrow_mut() = Some(grow);
                let root = s.create_element("div");
                let input = s.create_element("input");
                input.set_attribute("style", "width: 200px; height: 30px; font-size: 16px");
                root.append_child(&input);
                let filler = s.create_element("div");
                s.create_effect({
                    let filler = filler.clone();
                    move || filler.set_style("height", &format!("{}px", grow.get()))
                });
                root.append_child(&filler);
                *v.input.borrow_mut() = Some(input);
                root
            })
            .matches(|attrs| attrs.get_str("data-sheet").is_some())
            .on_key(move |key| {
                k.keys.borrow_mut().push(key.key.to_string());
                false
            })
            .on_caret_enter(move |from| {
                e.entered.borrow_mut().push(from);
                false
            }),
        )
        .keep();
    let h = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| h.mount(scope),
    );
    LIVE_ROOT.with(|r| r.set(Some(root)));
    Fixture { handle, view }
}

fn mouse_on(target: &web_sys::Element, name: &str, x: f32, y: f32) -> web_sys::MouseEvent {
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
    ev
}

/// A press at `(x, y)` on whatever the browser hit-tests there: answers whether
/// the `mousedown` was prevented.
fn press(x: f32, y: f32) -> bool {
    let target = document()
        .element_from_point(x, y)
        .unwrap_or_else(|| panic!("nothing under ({x}, {y})"));
    let down = mouse_on(&target, "mousedown", x, y);
    mouse_on(&target, "mouseup", x, y);
    down.default_prevented()
}

fn keydown(el: &web_sys::Element, key: &str, code: &str) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn embeds() -> Vec<web_sys::Element> {
    let list = document()
        .query_selector_all(&format!("[{HOST_MARKER}] [data-pm-type='embed']"))
        .unwrap();
    (0..list.length())
        .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
        .collect()
}

fn rect(el: &web_sys::Element) -> web_sys::DomRect {
    el.get_bounding_client_rect()
}

fn capture() -> web_sys::Element {
    document()
        .query_selector("textarea[data-pm-capture]")
        .unwrap()
        .expect("the capture textarea")
}

fn capture_has_focus() -> bool {
    document().active_element().as_ref() == Some(&capture())
}

fn node_sel(anchor: usize) -> Selection {
    Selection::Node(NodeSelection {
        anchor: Pos(anchor),
        head: Pos(anchor + 1),
    })
}

/// A press placing the caret in the middle of the `i`-th paragraph's text.
fn press_text(i: usize) {
    let list = document()
        .query_selector_all(&format!("[{HOST_MARKER}] p"))
        .unwrap();
    let p: web_sys::Element = list.item(i as u32).unwrap().dyn_into().unwrap();
    let r = rect(&p);
    // Monospace 16px: one character is about 9.6px; land after the first.
    press(r.left() as f32 + 12.0, (r.top() + r.height() / 2.0) as f32);
}

fn caret_top() -> f64 {
    let caret = document()
        .query_selector(&format!("[{HOST_MARKER}] [data-pm-caret]"))
        .unwrap()
        .expect("a caret overlay");
    rect(&caret).top()
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Review of #1565, F1: a press on an `embed` with no view node-selects it, by
/// the schema's flags. It used to place nothing useful.
#[wasm_bindgen_test]
fn a_press_on_an_embed_node_selects_it() {
    let f = mount();
    let plain = embeds()[1].clone();
    let r = rect(&plain);
    assert!(press(
        (r.left() + r.width() / 2.0) as f32,
        (r.top() + r.height() / 2.0) as f32
    ));
    assert_eq!(f.handle.selection(), node_sel(9));
}

#[wasm_bindgen_test]
fn a_press_on_the_frame_selects_and_a_press_on_the_component_is_the_components() {
    let f = mount();
    assert_eq!(f.view.renders.get(), 1, "only the matched embed has a view");
    let viewed = embeds()[0].clone();
    assert!(viewed.has_attribute(rinch_web::NODE_VIEW_ATTR));
    let r = rect(&viewed);
    assert!(
        press(r.left() as f32 + 2.0, r.top() as f32 + 2.0),
        "the frame"
    );
    assert_eq!(f.handle.selection(), node_sel(4));

    press_text(0);
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(2)));
    assert!(capture_has_focus());
    let input = f.view.input.borrow().clone().unwrap();
    let input_el: web_sys::Element = document()
        .query_selector(&format!("[{HOST_MARKER}] [data-pm-node-view] input"))
        .unwrap()
        .unwrap();
    let ir = rect(&input_el);
    // Beside the input, on the component.
    let prevented = press((ir.right() + 20.0) as f32, (ir.top() + 5.0) as f32);
    assert!(!prevented, "the component's press is left to the browser");
    assert_eq!(
        f.handle.selection(),
        Selection::cursor(Pos(2)),
        "no editor caret moved"
    );
    // On the input: not prevented (the browser focuses it), and the editor lets go.
    let prevented = press((ir.left() + 5.0) as f32, (ir.top() + 5.0) as f32);
    assert!(!prevented);
    assert!(!capture_has_focus(), "the editor let go of the keyboard");
    let _ = input;
}

#[wasm_bindgen_test]
fn keys_in_the_component_reach_the_view_and_escape_returns_the_caret_after_it() {
    let f = mount();
    press_text(0);
    let input: web_sys::HtmlElement = document()
        .query_selector(&format!("[{HOST_MARKER}] [data-pm-node-view] input"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let r = input.get_bounding_client_rect();
    press((r.left() + 5.0) as f32, (r.top() + 5.0) as f32);
    // A synthetic press has no default action: focus the input as the browser
    // would have.
    input.focus().unwrap();
    let ev = keydown(&input, "x", "KeyX");
    assert!(
        !ev.default_prevented(),
        "an unclaimed key goes on to the input"
    );
    assert_eq!(*f.view.keys.borrow(), ["x"]);
    assert_eq!(f.handle.doc().child(0).child(0).text(), Some("ab"));
    let ev = keydown(&input, "Escape", "Escape");
    assert!(ev.default_prevented(), "Escape was taken");
    assert_eq!(*f.view.keys.borrow(), ["x", "Escape"]);
    assert!(capture_has_focus(), "the editor has the keyboard again");
    assert_eq!(
        f.handle.selection(),
        Selection::cursor(Pos(6)),
        "after the node"
    );
    // The editor takes the next key.
    keydown(&capture(), "z", "KeyZ");
    assert_eq!(f.handle.doc().child(2).child(0).text(), Some("zcd"));
    assert_eq!(f.view.renders.get(), 1, "typing elsewhere kept the mount");
}

#[wasm_bindgen_test]
fn down_onto_the_view_node_selects_it_and_offers_the_caret() {
    let f = mount();
    press_text(0);
    keydown(&capture(), "ArrowDown", "ArrowDown");
    assert_eq!(f.handle.selection(), node_sel(4));
    assert_eq!(*f.view.entered.borrow(), [CaretFrom::Above]);
}

#[wasm_bindgen_test]
async fn a_component_that_grows_moves_the_caret_below_it() {
    let f = mount();
    press_text(1);
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(7)));
    sleep(30).await;
    let before = caret_top();
    f.view.grow.borrow().unwrap().set(120);
    // The ResizeObserver runs before the next paint; the pass it owes runs in a
    // microtask after it.
    sleep(100).await;
    let after = caret_top();
    assert!(
        (after - before - 100.0).abs() < 1.0,
        "the caret followed the growth: {before} -> {after}"
    );
    assert_eq!(f.view.renders.get(), 1);
}

// ── Review of #1575 ──────────────────────────────────────────────────────────

fn keydown_mod(el: &web_sys::Element, key: &str, code: &str, ctrl: bool) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_code(code);
    init.set_ctrl_key(ctrl);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn view_input() -> web_sys::HtmlElement {
    document()
        .query_selector(&format!("[{HOST_MARKER}] [data-pm-node-view] input"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap()
}

/// Backspace, Delete, Ctrl+Z, Tab and a letter in the view's input: none edits
/// the document, none is prevented (the input's own default runs).
#[wasm_bindgen_test]
fn r1575_keys_in_the_views_input_never_edit_the_document() {
    let f = mount();
    press_text(0);
    keydown(&capture(), "q", "KeyQ");
    let before = rinch_editor_core::serialize::html::node_to_html(&f.handle.doc());
    let input = view_input();
    let r = input.get_bounding_client_rect();
    press((r.left() + 5.0) as f32, (r.top() + 5.0) as f32);
    input.focus().unwrap();
    for (k, c, ctrl) in [
        ("Backspace", "Backspace", false),
        ("Delete", "Delete", false),
        ("z", "KeyZ", true),
        ("y", "KeyY", true),
        ("Tab", "Tab", false),
        ("a", "KeyA", true),
        ("Enter", "Enter", false),
    ] {
        let ev = keydown_mod(&input, k, c, ctrl);
        assert!(!ev.default_prevented(), "{k} (ctrl {ctrl}) was prevented");
    }
    // The browser's own undo in the input.
    let init = web_sys::InputEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_input_type("historyUndo");
    let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
    input.dispatch_event(&ev).unwrap();
    assert!(!ev.default_prevented());
    assert_eq!(
        rinch_editor_core::serialize::html::node_to_html(&f.handle.doc()),
        before,
        "the document is untouched"
    );
    assert_eq!(f.view.renders.get(), 1);
}

/// A selection drag that starts on the component selects no editor text.
#[wasm_bindgen_test]
fn r1575_a_drag_from_inside_the_view_selects_no_editor_text() {
    let f = mount();
    press_text(0);
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(2)));
    let ir = view_input().get_bounding_client_rect();
    let (sx, sy) = ((ir.right() + 20.0) as f32, (ir.top() + 5.0) as f32);
    let start = document().element_from_point(sx, sy).unwrap();
    mouse_on(&start, "mousedown", sx, sy);
    let list = document()
        .query_selector_all(&format!("[{HOST_MARKER}] p"))
        .unwrap();
    let p: web_sys::Element = list.item(1).unwrap().dyn_into().unwrap();
    let pr = rect(&p);
    let (ex, ey) = (
        (pr.left() + 14.0) as f32,
        (pr.top() + pr.height() / 2.0) as f32,
    );
    for i in 1..=5 {
        let t = i as f32 / 5.0;
        let (x, y) = (sx + (ex - sx) * t, sy + (ey - sy) * t);
        let el = document().element_from_point(x, y).unwrap();
        mouse_on(&el, "mousemove", x, y);
    }
    let end = document().element_from_point(ex, ey).unwrap();
    mouse_on(&end, "mouseup", ex, ey);
    assert_eq!(f.handle.selection(), Selection::cursor(Pos(2)));
    // Positive control: the same drag from the first paragraph's text selects.
    press_text(0);
    let p0: web_sys::Element = list.item(0).unwrap().dyn_into().unwrap();
    let r0 = rect(&p0);
    let (x0, y0) = (
        (r0.left() + 12.0) as f32,
        (r0.top() + r0.height() / 2.0) as f32,
    );
    let el = document().element_from_point(x0, y0).unwrap();
    mouse_on(&el, "mousedown", x0, y0);
    for i in 1..=5 {
        let t = i as f32 / 5.0;
        let (x, y) = (x0 + (ex - x0) * t, y0 + (ey - y0) * t);
        let el = document().element_from_point(x, y).unwrap();
        mouse_on(&el, "mousemove", x, y);
    }
    mouse_on(&end, "mouseup", ex, ey);
    assert!(
        !matches!(f.handle.selection(), Selection::Text(ref t) if t.anchor == t.head),
        "control: a drag from text selects: {:?}",
        f.handle.selection()
    );
}

/// Mounting and unmounting an editor with a view leaks no registered node.
#[wasm_bindgen_test]
fn r1575_mount_unmount_leaks_no_node() {
    let _ = mount();
    let base = rinch_web::web_document::__node_registry_len() as isize;
    for _ in 0..10 {
        let _ = mount();
    }
    let after = rinch_web::web_document::__node_registry_len() as isize;
    assert_eq!(after - base, 0, "registered nodes grew by {}", after - base);
}
