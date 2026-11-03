//! #2098 (`image-rendering`) and #2099 (`mask-image: url(…)`) on the web
//! backend: the browser draws both itself, so all rinch-web has to do is hand
//! the declarations through. These pin that it does, through a `style`
//! attribute and through `NodeHandle::set_style` (`style.setProperty`),
//! prefixed and not, by reading the browser's computed style back. The
//! desktop half is `crates/rinch-dom/tests/image_rendering_mask_2098_2099_tests.rs`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// The 4x4 half-opaque mask of the desktop tests, as a `data:` PNG.
const HALF: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAYAAACp8Z5+AAAAGUlEQVR4nGNgYGD4D8UMDAwMDEwMaICwAAB8lAIFSJaEeAAAAABJRU5ErkJggg==";

struct Mounted {
    root: rinch_web::RootHandle,
    host: web_sys::Element,
    dom: web_sys::Element,
    node: NodeHandle,
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount(style: String) -> Mounted {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let el = scope.create_element("div");
            el.set_attribute("style", &style);
            el.set_attribute("data-t2099", "");
            *slot_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let dom = host.query_selector("[data-t2099]").unwrap().unwrap();
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

/// The url a computed `mask-image` names, whichever quoting the engine
/// serializes it with (`url("…")`).
fn mask_url(computed: &str) -> Option<&str> {
    let inner = computed.strip_prefix("url(")?.strip_suffix(')')?;
    Some(inner.trim_matches('"'))
}

#[wasm_bindgen_test]
fn image_rendering_reaches_the_browser() {
    let m = mount("width: 10px; height: 10px; image-rendering: pixelated".into());
    assert_eq!(m.computed("image-rendering"), "pixelated");
    m.node.set_style("image-rendering", "crisp-edges");
    assert_eq!(m.computed("image-rendering"), "crisp-edges");
    m.node.set_style("image-rendering", "auto");
    assert_eq!(m.computed("image-rendering"), "auto");
    // A positive control that the element is the one being styled.
    assert!(m.dom.dyn_ref::<web_sys::HtmlElement>().is_some());
    m.done();
}

#[wasm_bindgen_test]
fn a_url_mask_reaches_the_browser() {
    let m = mount(format!(
        "width: 40px; height: 40px; background: red; mask-image: url({HALF}); \
         mask-size: 100% 100%; mask-repeat: no-repeat; mask-position: 1px 2px"
    ));
    assert_eq!(mask_url(&m.computed("mask-image")), Some(HALF));
    assert_eq!(m.computed("mask-size"), "100% 100%");
    assert_eq!(m.computed("mask-repeat"), "no-repeat");
    assert_eq!(m.computed("mask-position"), "1px 2px");
    m.node.set_style("mask-image", "none");
    assert_eq!(m.computed("mask-image"), "none");
    m.done();
}

/// The `-webkit-` spelling, as an attribute and through `setProperty`: both
/// engines map it to `mask-image`.
#[wasm_bindgen_test]
fn a_prefixed_url_mask_reaches_the_browser() {
    let m = mount(format!(
        "width: 40px; height: 40px; background: red; -webkit-mask-image: url({HALF})"
    ));
    assert_eq!(mask_url(&m.computed("mask-image")), Some(HALF));
    m.node.set_style("-webkit-mask-image", "none");
    assert_eq!(m.computed("mask-image"), "none");
    m.node
        .set_style("-webkit-mask-image", &format!("url({HALF})"));
    assert_eq!(mask_url(&m.computed("mask-image")), Some(HALF));
    m.done();
}
