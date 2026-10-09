//! Review of PR #1486: the path a browser WITHOUT `Node.moveBefore` takes
//! (Safari today). Chrome 154 and Firefox 157 both have the method, so the
//! `else` arm of every `has_move_before()` branch in `move_keeps_focus_1483.rs`
//! and `focused_node_displaced_1478.rs` runs in no CI job. This file deletes
//! the method before rinch-web first looks it up (it is its own wasm binary
//! and page), so the blur-first `insertBefore` path is exercised for real.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn strip() {
    js_sys::eval(
        "delete Element.prototype.moveBefore; delete Node.prototype.moveBefore; \
         delete Document.prototype.moveBefore; delete DocumentFragment.prototype.moveBefore;",
    )
    .unwrap();
    let has = js_sys::eval("typeof Element.prototype.moveBefore")
        .unwrap()
        .as_string()
        .unwrap();
    assert_eq!(has, "undefined", "positive control: the method is gone");
}

fn active() -> String {
    document()
        .active_element()
        .map(|el| {
            if el.id().is_empty() {
                el.tag_name()
            } else {
                el.id()
            }
        })
        .unwrap_or_default()
}

fn by_id(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap()
        .dyn_into()
        .unwrap()
}

#[wasm_bindgen_test]
fn without_the_method_a_moved_owner_is_blurred_first_and_its_entry_told_once() {
    strip();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let left = Rc::new(Cell::new(0u32));
    type Slots = (
        NodeHandle,
        NodeHandle,
        NodeHandle,
        rinch_core::DismissHandle,
    );
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let (l, sl) = (left.clone(), slots.clone());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        wrap.set_attribute("id", "nm-wrap");
        let trigger = s.create_element("button");
        trigger.set_attribute("id", "nm-trigger");
        wrap.append_child(&trigger);
        page.append_child(&wrap);
        let other = s.create_element("button");
        other.set_attribute("id", "nm-other");
        page.append_child(&other);
        let l = l.clone();
        let keep = rinch_core::push_key_handler(&trigger, |_| false, move || l.set(l.get() + 1));
        *sl.borrow_mut() = Some((page.clone(), wrap, other, keep));
        page
    });
    let (page, wrap, other, _keep) = slots.borrow_mut().take().unwrap();
    for (i, verb) in ["append", "before", "after"].into_iter().enumerate() {
        by_id("nm-trigger").focus().unwrap();
        assert_eq!(active(), "nm-trigger", "positive control: focused");
        match verb {
            "append" => page.append_child(&wrap),
            "before" => page.insert_before(&wrap, &other),
            _ => other.insert_after(&wrap),
        }
        assert_ne!(
            active(),
            "nm-trigger",
            "{verb}: no moveBefore, the focus went"
        );
        assert_eq!(
            left.get(),
            i as u32 + 1,
            "{verb}: and the entry heard it once"
        );
        // The document is free (no listener ran under the borrow).
        other.set_attribute("data-after", verb);
        assert_eq!(
            by_id("nm-other").get_attribute("data-after").as_deref(),
            Some(verb)
        );
    }
    root.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn without_the_method_a_keyed_reorder_still_works() {
    strip();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let rows = Signal::new(vec![1u32, 2, 3]);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                ul { id: "nm-list",
                    for n in rows.get() {
                        li { key: n, input { id: {format!("nm-in-{n}")} } }
                    }
                }
            }
        },
    );
    let mut lost = 0;
    for k in 1..=3u32 {
        rows.set(vec![1, 2, 3]);
        by_id(&format!("nm-in-{k}")).focus().unwrap();
        rows.set(vec![3, 2, 1]);
        if active() != format!("nm-in-{k}") {
            lost += 1;
        }
    }
    assert!(lost >= 2, "the moved rows lost the focus (lost {lost})");
    let ids =
        js_sys::eval("[...document.querySelectorAll('#nm-list input')].map(e => e.id).join(',')")
            .unwrap()
            .as_string()
            .unwrap();
    assert_eq!(ids, "nm-in-3,nm-in-2,nm-in-1");
    root.unmount();
    host.remove();
}

/// #1478's "a verb the browser refuses keeps the focus", in a browser without
/// the method. Where the method exists the refusal checks in the `NodeHandle`
/// verbs are shadowed by `moves_keeping_focus` (mutant M15 of the review
/// survives the whole #1478 file there), so this is their only pin.
#[wasm_bindgen_test]
fn without_the_method_a_refused_insertion_still_keeps_the_focus() {
    strip();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    type Slots = (NodeHandle, NodeHandle, NodeHandle, NodeHandle);
    let slots: Rc<RefCell<Option<Slots>>> = Default::default();
    let sl = slots.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |s| {
        let page = s.create_element("div");
        let wrap = s.create_element("div");
        let trigger = s.create_element("button");
        trigger.set_attribute("id", "nr-trigger");
        let inner = s.create_element("span");
        wrap.append_child(&trigger);
        wrap.append_child(&inner);
        page.append_child(&wrap);
        let other = s.create_element("p");
        page.append_child(&other);
        *sl.borrow_mut() = Some((page.clone(), wrap, inner, other));
        page
    });
    let (page, wrap, inner, _other) = slots.borrow_mut().take().unwrap();
    by_id("nr-trigger").focus().unwrap();
    // `inner` is not a child of `page`: NotFoundError, nothing moves.
    page.insert_before(&wrap, &inner);
    assert_eq!(active(), "nr-trigger", "reference not a child: focus kept");
    // Into its own subtree: HierarchyRequestError, nothing moves.
    inner.append_child(&wrap);
    assert_eq!(active(), "nr-trigger", "into itself: focus kept");
    root.unmount();
    host.remove();
}
