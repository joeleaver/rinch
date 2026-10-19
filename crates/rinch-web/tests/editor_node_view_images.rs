//! Node views C in a browser: a claimed inline image is an inline-block `<span>`
//! holding the app's component, one atomic inline of its line; the caret map
//! steps over the component's own text. Also the keys of an `<input>` in a
//! view: `on_key` hears every press (no release) first, and an unclaimed one is
//! the input's (the browser's default: Tab moves to the next input).
//!
//! The desktop twin is `rinch/src/app/editor_node_view_image_tests.rs`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_node_view_images
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{EditorHandle, NodeViewSpec, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-node-view-images";

thread_local! {
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

/// `ab` 1..3, the badged image at 3, `cd` 4..6; then an embed with two inputs.
const DOC: &str = r#"<p>ab<img src="x.png" data-badge="n">cd</p><div data-pm-type="embed" data-x="1"></div><p>ef</p>"#;
const IMAGE: usize = 3;

#[derive(Default)]
struct Keys(RefCell<Vec<String>>);

fn mount() -> (EditorHandle, Rc<Keys>) {
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
    handle
        .register_node_view(
            NodeViewSpec::new("image", |s: &mut RenderScope, _ctx| {
                let chip = s.create_element("b");
                chip.set_attribute("class", "chip");
                chip.set_attribute("style", "display: inline-block; width: 60px");
                // Text of the component's own: none of it is the editor's.
                let t = s.create_text("chip");
                chip.append_child(&t);
                chip
            })
            .matches(|a| a.get_str("data-badge").is_some()),
        )
        .keep();
    let keys = Rc::new(Keys::default());
    let k = keys.clone();
    handle
        .register_node_view(
            NodeViewSpec::new("embed", |s: &mut RenderScope, _ctx| {
                let root = s.create_element("div");
                for _ in 0..2 {
                    let input = s.create_element("input");
                    root.append_child(&input);
                }
                root
            })
            .on_key(move |key| {
                k.0.borrow_mut().push(key.key.to_string());
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
    (handle, keys)
}

fn q(sel: &str) -> web_sys::Element {
    document()
        .query_selector(&format!("[{HOST_MARKER}] {sel}"))
        .unwrap()
        .unwrap_or_else(|| {
            let html = document()
                .query_selector(&format!("[{HOST_MARKER}]"))
                .unwrap()
                .map(|h| h.inner_html())
                .unwrap_or_default();
            panic!("no {sel} in {html}")
        })
}

fn press(x: f64, y: f64) {
    let target = document().element_from_point(x as f32, y as f32).unwrap();
    for name in ["mousedown", "mouseup"] {
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
}

fn key_event(el: &web_sys::Element, kind: &str, key: &str, shift: bool) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

#[wasm_bindgen_test]
fn a_claimed_image_is_an_inline_block_span_and_its_text_is_not_the_editors() {
    let (handle, _) = mount();
    let span = q("span[data-pm-type='image'][data-pm-node-view]");
    let style = web_sys::window()
        .unwrap()
        .get_computed_style(&span)
        .unwrap()
        .unwrap();
    assert_eq!(
        style.get_property_value("display").unwrap(),
        "inline-block"
    );
    assert!(span.query_selector("b.chip").unwrap().is_some());
    let r = span.get_bounding_client_rect();
    let at = |pos| handle.caret_rect(Pos(pos)).expect("a caret");
    let (before, after, c) = (at(IMAGE), at(IMAGE + 1), at(IMAGE + 2));
    assert!(
        (before.x as f64 - r.left()).abs() < 1.5,
        "before: {} vs {}",
        before.x,
        r.left()
    );
    assert!(
        (after.x as f64 - r.right()).abs() < 1.5,
        "after: {} vs {}",
        after.x,
        r.right()
    );
    // After "c": one monospace char past the span, not inside the chip's text.
    assert!(
        c.x as f64 > r.right() + 5.0 && (c.x as f64) < r.right() + 15.0,
        "after c: {} (span right {})",
        c.x,
        r.right()
    );
    // A press on "cd"'s middle lands between them; a press on the chip moves nothing.
    press(c.x as f64 + 1.0, (c.y + c.height / 2.0) as f64);
    assert_eq!(handle.selection(), Selection::cursor(Pos(IMAGE + 2)));
    let chip = q("b.chip").get_bounding_client_rect();
    press(
        chip.left() + chip.width() / 2.0,
        chip.top() + chip.height() / 2.0,
    );
    assert_eq!(handle.selection(), Selection::cursor(Pos(IMAGE + 2)));
}

#[wasm_bindgen_test]
fn keys_in_a_views_input_reach_on_key_presses_only_and_stay_the_inputs() {
    let (handle, keys) = mount();
    let list = document()
        .query_selector_all(&format!("[{HOST_MARKER}] [data-pm-node-view] input"))
        .unwrap();
    let a: web_sys::HtmlElement = list.item(0).unwrap().dyn_into().unwrap();
    a.focus().unwrap();
    let doc = handle.doc();
    for (k, shift) in [
        ("Enter", false),
        ("Tab", false),
        ("Tab", true),
        ("ArrowLeft", false),
        ("ArrowRight", false),
        ("ArrowUp", false),
        ("ArrowDown", false),
    ] {
        let down = key_event(&a, "keydown", k, shift);
        assert!(!down.default_prevented(), "{k} is the input's");
        key_event(&a, "keyup", k, shift);
    }
    assert_eq!(handle.doc(), doc, "the document is untouched");
    assert_eq!(
        *keys.0.borrow(),
        [
            "Enter",
            "Tab",
            "Tab",
            "ArrowLeft",
            "ArrowRight",
            "ArrowUp",
            "ArrowDown"
        ],
        "every press, no release"
    );
    let esc = key_event(&a, "keydown", "Escape", false);
    assert!(esc.default_prevented(), "an unclaimed Escape returns the keyboard");
}
