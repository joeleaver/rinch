#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// #758: the fold added to `set_attribute`/`remove_attribute`
/// for match-decision purposes must NOT reach SVG's case-significant attribute
/// names at the DOM. `viewBox` must survive storage with its original casing.
#[wasm_bindgen_test]
fn viewbox_survives_the_fold_case_significant() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let build = move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let svg = scope.create_element("svg");
        svg.set_attribute("id", "fold-svg");
        svg.set_attribute("viewBox", "0 0 10 10");
        root.append_child(&svg);
        root
    };
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    let el = document().get_element_by_id("fold-svg").unwrap();
    let outer = el.outer_html();
    assert!(
        outer.contains("viewBox"),
        "viewBox must survive with its original casing: outerHTML was {outer:?}"
    );
    assert!(
        !outer.contains("viewbox=\""),
        "viewBox must not be folded to lowercase: outerHTML was {outer:?}"
    );
    root.unmount();
    host.remove();
}
