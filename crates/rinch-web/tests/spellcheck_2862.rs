//! Spellcheck on the web (#2862): the browser checks the page's fields, and
//! rinch only passes the `spellcheck` state through to the real DOM.
//!
//! `App::spellcheck(false)` puts `spellcheck="false"` on `<body>`; a field's
//! `spellcheck` prop is written on the field. What is asserted is the
//! browser's own answer, `HTMLElement.spellcheck`, which applies HTML's
//! inheritance: so the test shows the state reaching the browser, not only
//! an attribute being written.
//!
//! Its own file (its own page): `run` mounts under `#rinch-body` for the
//! page's lifetime.
#![cfg(target_arch = "wasm32")]

use rinch::App;
use rinch::components::{TextInput, Textarea};
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_web::AppExt as _;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn app(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("id", "spellcheck-2862");
    let plain = Textarea::default().render(scope, &[]);
    plain.set_attribute("id", "plain");
    root.append_child(&plain);
    let on = TextInput {
        spellcheck: Some(true),
        ..Default::default()
    }
    .render(scope, &[]);
    on.set_attribute("id", "on");
    root.append_child(&on);
    root
}

/// The browser's resolved `spellcheck` state of the field inside `#id`.
fn resolved(id: &str) -> bool {
    document()
        .query_selector(&format!("#{id} textarea, #{id} input"))
        .unwrap()
        .expect("the field is on the page")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .spellcheck()
}

#[wasm_bindgen_test]
fn the_app_switch_and_the_prop_reach_the_browser() {
    // Positive control: before the app is run, a page textarea is checked by
    // the browser — the property is a live answer, not a constant.
    let probe = document().create_element("textarea").unwrap();
    document().body().unwrap().append_child(&probe).unwrap();
    let probe: web_sys::HtmlElement = probe.dyn_into().unwrap();
    assert!(probe.spellcheck(), "a textarea is checked by default");

    App::new(app).spellcheck(false).run();
    assert!(document().get_element_by_id("spellcheck-2862").is_some());

    assert_eq!(
        document()
            .body()
            .unwrap()
            .get_attribute("spellcheck")
            .as_deref(),
        Some("false")
    );
    assert!(!probe.spellcheck(), "<body>'s state is not inherited");
    assert!(
        !resolved("plain"),
        "a field with no prop inherits the app's off"
    );
    assert!(
        resolved("on"),
        "`spellcheck: Some(true)` lost to the app's off"
    );

    // Turned back on while the app runs: the page is the browser's again.
    rinch::spellcheck::set_enabled(true);
    assert!(
        document()
            .body()
            .unwrap()
            .get_attribute("spellcheck")
            .is_none()
    );
    assert!(resolved("plain"));
}
