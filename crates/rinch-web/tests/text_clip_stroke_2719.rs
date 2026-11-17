//! #2719 (`background-clip: text`, `-webkit-text-fill-color`,
//! `-webkit-text-stroke`, `paint-order`) on the web backend: the browser
//! paints them itself, so all rinch-web has to do is hand the declarations
//! through. This pins that it does, through a `style` attribute and through
//! `NodeHandle::set_style` (`style.setProperty`), by reading the browser's
//! computed style back. The desktop half is
//! `crates/rinch-dom/tests/text_clip_stroke_2719_tests.rs`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

struct Mounted {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    dom: web_sys::Element,
    node: NodeHandle,
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount(style: &str) -> Mounted {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let style = style.to_string();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let el = scope.create_element("div");
            el.set_attribute("style", &style);
            el.set_attribute("data-t2719", "");
            *slot_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let dom = host.query_selector("[data-t2719]").unwrap().unwrap();
    let node = slot.borrow_mut().take().unwrap();
    Mounted {
        root,
        host,
        dom,
        node,
    }
}

impl Mounted {
    fn computed(&self, property: &str) -> String {
        web_sys::window()
            .unwrap()
            .get_computed_style(&self.dom)
            .unwrap()
            .unwrap()
            .get_property_value(property)
            .unwrap()
    }

    fn done(self) {
        self.root.unmount();
        self.host.remove();
    }
}

#[wasm_bindgen_test]
fn text_stroke_fill_and_clip_reach_the_browser() {
    let m = mount(
        "font-size: 30px; color: rgb(255, 0, 0); background-image: linear-gradient(red, blue); \
         background-clip: text; -webkit-text-fill-color: transparent; \
         -webkit-text-stroke: 3px rgb(0, 0, 255); paint-order: stroke",
    );
    assert_eq!(m.computed("background-clip"), "text");
    assert_eq!(m.computed("-webkit-text-fill-color"), "rgba(0, 0, 0, 0)");
    assert_eq!(m.computed("-webkit-text-stroke-width"), "3px");
    assert_eq!(m.computed("-webkit-text-stroke-color"), "rgb(0, 0, 255)");
    assert_eq!(m.computed("paint-order"), "stroke");
    m.node.set_style("-webkit-text-stroke-width", "5px");
    m.node
        .set_style("-webkit-text-stroke-color", "rgb(0, 128, 0)");
    m.node.set_style("-webkit-text-fill-color", "rgb(1, 2, 3)");
    m.node.set_style("paint-order", "normal");
    m.node.set_style("-webkit-background-clip", "padding-box");
    assert_eq!(m.computed("-webkit-text-stroke-width"), "5px");
    assert_eq!(m.computed("-webkit-text-stroke-color"), "rgb(0, 128, 0)");
    assert_eq!(m.computed("-webkit-text-fill-color"), "rgb(1, 2, 3)");
    assert_eq!(m.computed("paint-order"), "normal");
    assert_eq!(m.computed("background-clip"), "padding-box");
    m.done();
}
