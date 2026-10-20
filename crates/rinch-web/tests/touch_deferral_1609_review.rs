//! The review of PR #1609 (#1582, #1588): where the `touch-action: none` that
//! keeps a touch click on the press may sit (the real `Slider` carries it on its
//! container, not on the element its handler is on), the edges of "the page
//! scrolls", which deferred presses are prevented, and that a `Modal` gives its
//! scroll lock back. Synthetic touches; the reviewer's probes p1–p7, plus three
//! for the mutants they left (a plain row's press, a page that scrolls only
//! sideways, a `pan-y` scroller).
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-test-host-review-1609";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn clear() {
    rinch_web::__reset_activation_state();
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(n) = stale.item(i) {
                n.dyn_into::<web_sys::Element>().unwrap().remove();
            }
        }
    }
    let _ = document()
        .document_element()
        .unwrap()
        .remove_attribute("style");
    let _ = document().body().unwrap().remove_attribute("style");
}

fn host(style: &str) -> web_sys::Element {
    clear();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute("style", style).unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

fn pointer(el: &web_sys::Element, kind: &str) -> web_sys::PointerEvent {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(9);
    init.set_is_primary(true);
    init.set_pointer_type("touch");
    init.set_button(0);
    init.set_buttons(if kind == "pointerdown" { 1 } else { 0 });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn page_overflow() -> (i32, i32) {
    let se = document().scrolling_element().unwrap();
    (se.scroll_height(), se.client_height())
}

type Log = Rc<RefCell<Vec<String>>>;

/// Build: `outer(style) > inner(style) > leaf#leaf` (leaf has the handler),
/// then a 3000px spacer so the page scrolls.
fn nested(
    outer_style: &str,
    inner_style: &str,
    spacer: bool,
) -> (rinch_web::RootHandle, web_sys::Element, Log) {
    let h = host("width: 300px");
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let (os, is) = (outer_style.to_string(), inner_style.to_string());
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let outer = s.create_element("div");
            outer.set_attribute("style", &os);
            let inner = s.create_element("div");
            inner.set_attribute("style", &is);
            let leaf = s.create_element("div");
            leaf.set_attribute("id", "leaf");
            leaf.set_attribute("style", "height: 30px; width: 100px; background: #ddd");
            let sink = sink.clone();
            let rid = s.register_handler(move || {
                sink.borrow_mut().push("leaf".into());
                rinch_core::Drag::percent().on_move(|_, _| {}).start();
            });
            leaf.set_attribute("data-rid", &rid.0.to_string());
            inner.append_child(&leaf);
            let filler = s.create_element("div");
            filler.set_attribute("style", "height: 600px");
            inner.append_child(&filler);
            outer.append_child(&inner);
            page.append_child(&outer);
            if spacer {
                let sp = s.create_element("div");
                sp.set_attribute("style", "height: 3000px");
                page.append_child(&sp);
            }
            page
        },
    );
    (root, h, log)
}

fn done(root: rinch_web::RootHandle, h: web_sys::Element) {
    rinch_core::Drag::cancel();
    root.unmount();
    h.remove();
    clear();
}

/// P1 — Slider's shape: `touch-action: none` on the PARENT of the rid element,
/// on a scrolling page. Must be eager (the real Slider's rid is on its overlay,
/// the touch-action on the container).
#[wasm_bindgen_test]
fn p1_touch_action_none_on_an_ancestor_is_eager() {
    let (root, h, log) = nested("touch-action: none", "", true);
    let (sh, ch) = page_overflow();
    assert!(sh > ch, "positive control: page scrolls");
    pointer(
        &document().get_element_by_id("leaf").unwrap(),
        "pointerdown",
    );
    assert_eq!(*log.borrow(), ["leaf"], "eager");
    assert!(rinch_core::Drag::is_active());
    pointer(&document().get_element_by_id("leaf").unwrap(), "pointerup");
    done(root, h);
}

/// P2 — the scroller itself carries `touch-action: none` (inclusive ancestor).
#[wasm_bindgen_test]
fn p2_touch_action_none_on_the_scroller_itself_is_eager() {
    let (root, h, log) = nested(
        "",
        "height: 100px; overflow: auto; touch-action: none",
        false,
    );
    pointer(
        &document().get_element_by_id("leaf").unwrap(),
        "pointerdown",
    );
    let r = log.borrow().clone();
    pointer(&document().get_element_by_id("leaf").unwrap(), "pointerup");
    done(root, h);
    assert_eq!(r, ["leaf"], "eager");
}

/// P3 — `touch-action: none` ABOVE the scroller is not read: deferred.
#[wasm_bindgen_test]
fn p3_touch_action_none_above_the_scroller_is_deferred() {
    let (root, h, log) = nested("touch-action: none", "height: 100px; overflow: auto", false);
    pointer(
        &document().get_element_by_id("leaf").unwrap(),
        "pointerdown",
    );
    let r = log.borrow().clone();
    pointer(&document().get_element_by_id("leaf").unwrap(), "pointerup");
    done(root, h);
    assert!(r.is_empty(), "deferred: {r:?}");
}

/// P4 — a page that is exactly the viewport's size does not scroll: eager.
#[wasm_bindgen_test]
fn p4_a_page_exactly_the_viewport_size_is_eager() {
    let h = host("");
    // Hide everything else the harness put in <body>.
    let body = document().body().unwrap();
    let kids = document().query_selector_all("body > *").unwrap();
    let mut hidden = Vec::new();
    for i in 0..kids.length() {
        let k: web_sys::Element = kids.item(i).unwrap().dyn_into().unwrap();
        if !k.is_same_node(Some(&h)) {
            let old = k.get_attribute("style");
            k.set_attribute("style", "display: none").unwrap();
            hidden.push((k, old));
        }
    }
    body.set_attribute("style", "margin: 0").unwrap();
    let vh = web_sys::window()
        .unwrap()
        .inner_height()
        .unwrap()
        .as_f64()
        .unwrap();
    h.set_attribute("style", &format!("height: {vh}px; overflow: hidden"))
        .unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let b = s.create_element("button");
            b.set_attribute("id", "b4");
            b.append_child(&s.create_text("x"));
            let sink = sink.clone();
            let rid = s.register_handler(move || sink.borrow_mut().push("b".into()));
            b.set_attribute("data-rid", &rid.0.to_string());
            b
        },
    );
    let (sh, ch) = page_overflow();
    pointer(&document().get_element_by_id("b4").unwrap(), "pointerdown");
    let r = log.borrow().clone();
    pointer(&document().get_element_by_id("b4").unwrap(), "pointerup");
    root.unmount();
    h.remove();
    for (k, old) in hidden {
        match old {
            Some(v) => k.set_attribute("style", &v).unwrap(),
            None => k.remove_attribute("style").unwrap(),
        }
    }
    clear();
    assert_eq!(
        sh, ch,
        "positive control: page exactly viewport-sized ({sh} vs {ch})"
    );
    assert_eq!(r, ["b"], "eager on a page that cannot scroll");
}

/// P5 — the real `Slider` component on a scrolling page arms its Drag on contact.
#[wasm_bindgen_test]
fn p5_real_slider_on_a_scrolling_page_arms_on_contact() {
    let h = host("width: 300px");
    let v = Signal::new(0.0f64);
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let page = s.create_element("div");
            let slider = rinch::components::Slider {
                min: Some(0.0),
                max: Some(100.0),
                onchange: Some(rinch_core::ValueCallback::new(move |x: f64| v.set(x))),
                ..Default::default()
            }
            .render(s, &[]);
            page.append_child(&slider);
            let sp = s.create_element("div");
            sp.set_attribute("style", "height: 3000px");
            page.append_child(&sp);
            page
        },
    );
    let overlay = document()
        .query_selector(".rinch-slider__overlay")
        .unwrap()
        .unwrap();
    let ta = web_sys::window()
        .unwrap()
        .get_computed_style(&document().query_selector(".rinch-slider").unwrap().unwrap())
        .unwrap()
        .unwrap()
        .get_property_value("touch-action")
        .unwrap();
    pointer(&overlay, "pointerdown");
    let active = rinch_core::Drag::is_active();
    pointer(&overlay, "pointerup");
    done(root, h);
    assert_eq!(ta, "none", "positive control: component CSS loaded");
    assert!(active, "the slider's drag is armed on contact");
}

/// P6 — Modal lock released after close+unmount (does the code leak a lock?).
#[wasm_bindgen_test]
fn p6_modal_close_then_unmount_releases_the_lock() {
    let h = host("");
    let open = Signal::new(false);
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let input = s.create_element("input");
            rinch::components::Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                with_close_button: false,
                ..Default::default()
            }
            .render(s, &[input])
        },
    );
    open.set(true);
    let depth_open = rinch_web::__scroll_lock_depth();
    open.set(false);
    root.unmount();
    h.remove();
    let depth = rinch_web::__scroll_lock_depth();
    let style = document()
        .document_element()
        .unwrap()
        .get_attribute("style");
    clear();
    assert_eq!(depth_open, 1, "positive control: the open modal locks");
    assert_eq!(depth, 0);
    assert!(
        style.unwrap_or_default().is_empty(),
        "html style left behind"
    );
}

/// P7 — unmounted while open.
#[wasm_bindgen_test]
fn p7_modal_unmounted_while_open_releases_the_lock() {
    let h = host("");
    let open = Signal::new(false);
    let root = rinch_web::mount_into(
        &h,
        ThemeProviderProps::default(),
        move |s: &mut RenderScope| {
            let input = s.create_element("input");
            rinch::components::Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                with_close_button: false,
                ..Default::default()
            }
            .render(s, &[input])
        },
    );
    open.set(true);
    root.unmount();
    h.remove();
    let depth = rinch_web::__scroll_lock_depth();
    clear();
    assert_eq!(depth, 0);
}

/// P8 — a deferred press on a plain clickable row (a `div[data-rid]`, nothing
/// focusable) is prevented too, so no compatibility `mousedown` follows the
/// tap (#1588 for a list row). Kills preventing only presses that focus.
#[wasm_bindgen_test]
fn p8_a_deferred_press_on_a_plain_row_is_prevented() {
    let (root, h, log) = nested("", "height: 100px; overflow: auto", false);
    let leaf = document().get_element_by_id("leaf").unwrap();
    let ev = pointer(&leaf, "pointerdown");
    let deferred = log.borrow().is_empty();
    pointer(&leaf, "pointerup");
    let r = log.borrow().clone();
    done(root, h);
    assert!(deferred, "positive control: the press was deferred");
    assert_eq!(r, ["leaf"], "positive control: the tap clicked");
    assert!(
        ev.default_prevented(),
        "the plain row's deferred press is prevented"
    );
}

/// P9 — a page that scrolls only sideways can pan too: the press waits.
/// Kills dropping the x axis of the page check.
#[wasm_bindgen_test]
fn p9_a_page_that_scrolls_only_sideways_defers() {
    let (root, h, log) = nested("width: 5000px", "height: 40px; overflow: hidden", false);
    let se = document().scrolling_element().unwrap();
    let wide = se.scroll_width() > se.client_width();
    let tall = se.scroll_height() > se.client_height();
    let leaf = document().get_element_by_id("leaf").unwrap();
    pointer(&leaf, "pointerdown");
    let r = log.borrow().clone();
    pointer(&leaf, "pointerup");
    done(root, h);
    assert!(wide && !tall, "positive control: wide {wide}, tall {tall}");
    assert!(r.is_empty(), "clicked on contact: {r:?}");
}

/// P10 — `touch-action: pan-y` lets a finger pan, so a press in a `pan-y`
/// scroller waits for the release. Kills reading any value but `auto` as
/// `none`.
#[wasm_bindgen_test]
fn p10_a_pan_y_scroller_defers() {
    let (root, h, log) = nested(
        "",
        "height: 100px; overflow: auto; touch-action: pan-y",
        false,
    );
    let leaf = document().get_element_by_id("leaf").unwrap();
    pointer(&leaf, "pointerdown");
    let r = log.borrow().clone();
    pointer(&leaf, "pointerup");
    done(root, h);
    assert!(r.is_empty(), "clicked on contact: {r:?}");
}
