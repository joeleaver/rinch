//! Review of #1595 in Chrome: claim/unclaim churn — by an attr change, and by a
//! spec registered and dropped after mount — leaks no registry entry and does not
//! panic (the host swap requested the overlay refresh with the core borrowed, and
//! the web's refresher runs at once); End and a press past the end of a line whose
//! last atom is a claimed image.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{AttrValue, Pos, SetNodeAttrStep};
use rinch_web::web_document::__node_registry_len;
use rinch_web::{EditorHandle, NodeViewSpec, RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-review-1595";

thread_local! {
    static LIVE_ROOT: std::cell::Cell<Option<RootHandle>> = const { std::cell::Cell::new(None) };
}

fn mount(doc: &str, claim: bool) -> EditorHandle {
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
    assert!(handle.load_html(doc));
    if claim {
        handle
            .register_node_view(
                NodeViewSpec::new("image", |s: &mut RenderScope, _ctx| {
                    let chip = s.create_element("b");
                    chip.set_attribute("class", "chip");
                    chip.set_attribute("style", "display: inline-block; width: 60px");
                    let t = s.create_text("chip");
                    chip.append_child(&t);
                    chip
                })
                .matches(|a| a.get_str("data-badge").is_some()),
            )
            .keep();
    }
    let h = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| h.mount(scope),
    );
    LIVE_ROOT.with(|r| r.set(Some(root)));
    handle
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

fn key(name: &str) {
    let el = document().active_element().unwrap();
    for kind in ["keydown", "keyup"] {
        let init = web_sys::KeyboardEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_key(name);
        let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init).unwrap();
        el.dispatch_event(&ev).unwrap();
    }
}

fn toggle(handle: &EditorHandle, on: bool) {
    assert!(handle.update(|st| {
        let mut tr = st.tr();
        let v = if on { Some(AttrValue::from("n")) } else { None };
        let step = match v {
            Some(v) => SetNodeAttrStep::new(3, "data-badge", v),
            None => SetNodeAttrStep {
                pos: 3,
                attr: "data-badge".into(),
                value: None,
            },
        };
        tr.step(Box::new(step)).ok()?;
        Some(tr)
    }));
}

#[wasm_bindgen_test]
fn claim_unclaim_churn_grows_no_registry() {
    let handle = mount(r#"<p>ab<img src="x.png" data-badge="n">cd</p>"#, true);
    for _ in 0..3 {
        toggle(&handle, false);
        toggle(&handle, true);
    }
    let base = __node_registry_len() as isize;
    for _ in 0..50 {
        toggle(&handle, false);
        toggle(&handle, true);
    }
    let g = __node_registry_len() as isize - base;
    assert_eq!(g, 0, "registry grew by {g} over 50 claim/unclaim cycles");
    // The <img> came back each time and the span went: one image host now.
    let n = document()
        .query_selector_all(&format!("[{HOST_MARKER}] [data-pm-type='image']"))
        .unwrap()
        .length();
    assert_eq!(n, 1);
}

fn end_and_press_past(claim: bool) -> (Pos, Pos) {
    let handle = mount(
        r#"<p>ab<img src="x.png" data-badge="n" style="width: 20px; height: 20px"></p><p>gh</p>"#,
        claim,
    );
    let c = handle.caret_rect(Pos(2)).unwrap();
    press(c.x as f64 + 1.0, (c.y + c.height / 2.0) as f64);
    assert_eq!(handle.selection().head(), Pos(2));
    key("End");
    let after_end = handle.selection().head();
    let img = document()
        .query_selector(&format!("[{HOST_MARKER}] [data-pm-type='image']"))
        .unwrap()
        .unwrap()
        .get_bounding_client_rect();
    press(img.right() + 20.0, img.top() + img.height() / 2.0);
    (after_end, handle.selection().head())
}

#[wasm_bindgen_test]
fn end_and_press_past_a_line_final_claimed_image() {
    let unclaimed = end_and_press_past(false);
    let claimed = end_and_press_past(true);
    wasm_bindgen_test::console_log!("REVIEW-1595 unclaimed {unclaimed:?} claimed {claimed:?}");
    assert_eq!(claimed, (Pos(4), Pos(4)));
}

/// A spec registered after mount claims an image that is already shown: the
/// `<img>` → `<span>` swap, on the web, with the overlay refresher live.
#[wasm_bindgen_test]
fn a_spec_registered_after_mount_claims_without_a_panic() {
    let handle = mount(r#"<p>ab<img src="x.png" data-badge="n">cd</p>"#, false);
    let reg = handle.register_node_view(
        NodeViewSpec::new("image", |s: &mut RenderScope, _ctx| s.create_element("b"))
            .matches(|a| a.get_str("data-badge").is_some()),
    );
    assert!(
        document()
            .query_selector(&format!(
                "[{HOST_MARKER}] span[data-pm-type='image'][data-pm-node-view]"
            ))
            .unwrap()
            .is_some()
    );
    drop(reg);
    assert!(
        document()
            .query_selector(&format!("[{HOST_MARKER}] img[data-pm-type='image']"))
            .unwrap()
            .is_some()
    );
}

/// A claiming spec registered and dropped after mount, 50 times: each swap of the
/// `<img>` for the view's `<span>` and back runs the web's overlay refresher at
/// once, which re-borrowed the core the swap still held ("RefCell already
/// borrowed", which aborts the app), and no cycle leaves a node in the registry.
#[wasm_bindgen_test]
fn registering_and_dropping_a_claiming_spec_after_mount_grows_no_registry() {
    let handle = mount(r#"<p>ab<img src="x.png" data-badge="n">cd</p>"#, false);
    let claimed = || {
        document()
            .query_selector(&format!(
                "[{HOST_MARKER}] span[data-pm-type='image'][data-pm-node-view]"
            ))
            .unwrap()
            .is_some()
    };
    let img = || {
        document()
            .query_selector(&format!("[{HOST_MARKER}] img[data-pm-type='image']"))
            .unwrap()
            .is_some()
    };
    let cycle = || {
        let reg = handle.register_node_view(
            NodeViewSpec::new("image", |s: &mut RenderScope, _ctx| {
                let b = s.create_element("b");
                let t = s.create_text("chip");
                b.append_child(&t);
                b
            })
            .matches(|a| a.get_str("data-badge").is_some()),
        );
        assert!(claimed() && !img(), "the spec claims the shown image");
        drop(reg);
        assert!(img() && !claimed(), "dropping it gives the <img> back");
    };
    for _ in 0..3 {
        cycle();
    }
    let base = __node_registry_len() as isize;
    for _ in 0..50 {
        cycle();
    }
    let g = __node_registry_len() as isize - base;
    assert_eq!(g, 0, "registry grew by {g} over 50 register/drop cycles");
}
