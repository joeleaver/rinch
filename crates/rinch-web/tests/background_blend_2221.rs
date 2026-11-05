//! #2221 (`background-blend-mode`) on the web backend: the browser blends
//! the layers itself, so all rinch-web has to do is hand the declaration
//! through. This pins that it does, through a `style` attribute and through
//! `NodeHandle::set_style` (`style.setProperty`), a list included, by reading
//! the browser's computed style back. The desktop half is
//! `crates/rinch-dom/tests/background_blend_2221_tests.rs`.
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
            el.set_attribute("data-t2221", "");
            *slot_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let dom = host.query_selector("[data-t2221]").unwrap().unwrap();
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
fn background_blend_mode_reaches_the_browser() {
    let m = mount(
        "width: 40px; height: 40px; background-color: rgb(255, 128, 64); \
         background-image: linear-gradient(red, blue); background-blend-mode: multiply",
    );
    assert_eq!(m.computed("background-blend-mode"), "multiply");
    m.node.set_style("background-blend-mode", "luminosity");
    assert_eq!(m.computed("background-blend-mode"), "luminosity");
    m.node.set_style("background-blend-mode", "normal");
    assert_eq!(m.computed("background-blend-mode"), "normal");
    // One entry per image layer, as a list.
    m.node.set_style(
        "background-image",
        "linear-gradient(red, blue), linear-gradient(green, white)",
    );
    m.node
        .set_style("background-blend-mode", "screen, color-dodge");
    assert_eq!(m.computed("background-blend-mode"), "screen, color-dodge");
    m.done();
}
