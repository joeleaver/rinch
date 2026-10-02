//! `set_style` against a later declaration that covers the written property,
//! on the web backend (#470) — the Chrome half of the twin whose desktop half
//! is `crates/rinch-dom/tests/set_style_shorthand_overlap_tests.rs`.
//!
//! `WebDocument::set_style` is `style.setProperty`, so the browser's CSSOM
//! decides, and these pass before and after #470: they are the measurement
//! the desktop fixtures assert, taken through the same `NodeHandle::set_style`
//! call on the same markup. Positions are the child's border box relative to
//! its parent's, which is what desktop's `LayoutResult` reports.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const PARENT: &str = "position: relative; width: 300px; height: 200px; \
                      border-left: 5px solid black; border-top: 7px solid black";

struct Mounted {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    parent: web_sys::Element,
    child_dom: web_sys::HtmlElement,
    child: NodeHandle,
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount(child_style: &'static str) -> Mounted {
    use wasm_bindgen::JsCast;
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let parent = scope.create_element("div");
            parent.set_attribute("style", PARENT);
            parent.set_attribute("data-t470", "parent");
            let child = scope.create_element("div");
            child.set_attribute("style", child_style);
            child.set_attribute("data-t470", "child");
            parent.append_child(&child);
            *slot_in.borrow_mut() = Some(child);
            parent
        },
    );
    let parent = host.query_selector("[data-t470=parent]").unwrap().unwrap();
    let child_dom = host
        .query_selector("[data-t470=child]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let child = slot.borrow_mut().take().unwrap();
    Mounted {
        root,
        host,
        parent,
        child_dom,
        child,
    }
}

impl Mounted {
    fn xy(&self) -> (f64, f64) {
        let p = self.parent.get_bounding_client_rect();
        let c = self.child_dom.get_bounding_client_rect();
        (c.left() - p.left(), c.top() - p.top())
    }

    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

#[wasm_bindgen_test]
fn a_longhand_declared_before_a_covering_shorthand_wins_when_written() {
    let m = mount("position: absolute; left: 5px; inset: 0; width: 10px; height: 10px");
    assert_eq!(m.xy(), (5.0, 7.0), "baseline: `inset: 0` wins the parse");
    m.child.set_style("left", "25px");
    assert_eq!(
        m.child_dom.style().get_property_value("left").unwrap(),
        "25px"
    );
    assert_eq!(m.xy(), (30.0, 7.0));
    m.done();
}

#[wasm_bindgen_test]
fn a_shorthand_written_over_a_later_longhand_covers_it() {
    let m = mount("position: absolute; inset: 0; left: 15px; width: 10px; height: 10px");
    assert_eq!(m.xy(), (20.0, 7.0), "baseline: the later `left` wins");
    m.child.set_style("inset", "3px");
    assert_eq!(m.xy(), (8.0, 10.0));
    m.done();
}

#[wasm_bindgen_test]
fn a_physical_longhand_written_before_its_logical_twin_wins() {
    let m = mount(
        "position: absolute; left: 5px; inset-inline-start: 0; top: 0; \
         width: 10px; height: 10px",
    );
    assert_eq!(m.xy(), (5.0, 7.0), "baseline: the later logical twin wins");
    m.child.set_style("left", "25px");
    assert_eq!(m.xy(), (30.0, 7.0));
    m.child.set_style("color", "red");
    assert_eq!(m.xy(), (30.0, 7.0));
    m.done();
}

#[wasm_bindgen_test]
fn a_later_declaration_that_covers_nothing_leaves_the_write_in_place() {
    let m = mount("position: absolute; left: 5px; top: 3px; width: 10px; height: 10px");
    m.child.set_style("left", "25px");
    assert_eq!(m.xy(), (30.0, 10.0));
    m.done();
}

#[wasm_bindgen_test]
fn a_margin_longhand_declared_before_the_margin_shorthand_wins_when_written() {
    let m = mount("margin-left: 5px; margin: 0; width: 10px; height: 10px");
    assert_eq!(m.xy(), (5.0, 7.0), "baseline: `margin: 0` wins the parse");
    m.child.set_style("margin-left", "25px");
    assert_eq!(m.xy(), (30.0, 7.0));
    m.done();
}

#[wasm_bindgen_test]
fn a_batch_moves_only_the_overlapped_write() {
    let m = mount("position: absolute; width: 10px; left: 5px; inset: 0; height: 10px");
    m.child.set_style("left", "25px");
    m.child.set_style("width", "20px");
    assert_eq!(m.xy(), (30.0, 7.0));
    assert_eq!(m.child_dom.get_bounding_client_rect().width(), 20.0);
    m.done();
}

/// The measurement behind desktop's pinned divergence (#1298): `setProperty`
/// replaces the `left` longhand of an `!important` `inset` outright.
#[wasm_bindgen_test]
fn a_later_important_shorthand_does_not_beat_set_property() {
    let m = mount("position: absolute; left: 5px; inset: 0 !important; width: 10px; height: 10px");
    assert_eq!(
        m.xy(),
        (5.0, 7.0),
        "baseline: the important `inset` wins the parse"
    );
    m.child.set_style("left", "25px");
    assert_eq!(m.xy(), (30.0, 7.0));
    m.done();
}
