//! `ColorInput`'s outside-click dismissal, in a real browser (issue #465).
//!
//! The dismissal is three CSS rules and a click handler. `rinch-components`
//! ships **one** stylesheet to both backends, so everything the desktop twin
//! (`rinch/src/app/color_input_dismiss_465_tests.rs`) measures against rinch's
//! own stacking code is a question Chrome answers independently — and the two
//! engines are the only oracle each other has. What is checked here:
//!
//! - the backdrop is revealed by the `--opened` class and hidden without it;
//! - **the ordering**, read as `elementFromPoint` rather than as a `z-index`
//!   string: the field and the picker's own panel are above the backdrop, and
//!   the page beside them is not. A `z-index` assertion would pass against a
//!   backdrop that had become a stacking context of its own, or against a panel
//!   whose `position` was dropped, because `z-index` on a static box does
//!   nothing and reads back unchanged.
//! - a click on the backdrop closes the picker through the real `data-rid`
//!   delegation.
//!
//! The reveal rule is spelled with child combinators
//! (`.rinch-color-input--opened > .…__wrapper > .…__backdrop`), which is safe
//! only because `ColorInput::render` appends both nodes itself — the #774 trap
//! is a `>` in front of a **caller's** child, which `rsx!` can put a
//! `display: contents` wrapper before. There is no caller's child here:
//! `ColorInput` ignores `children` entirely.
//!
//! **Mount through `rinch_web::mount_into`, not a hand-rolled `RenderScope`** —
//! without the owner `mount_tree` pushes, component effects are disposed at
//! once and the `--opened` class is never written.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test color_input_dismiss
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::ColorInput;
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-465";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount a `ColorInput` in a fresh host, unmounting the previous fixture's
/// tree. The host is given a width so the field has a box, and is pushed clear
/// of the viewport's top-left corner so "beside the field" is a real point.
fn mount(close_on_click_outside: bool) -> web_sys::Document {
    let bdoc = browser_document();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = bdoc.query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = bdoc.create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute("style", "width: 240px; margin: 40px")
        .unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        ColorInput {
            value: "#000000".to_string(),
            close_on_click_outside,
            ..Default::default()
        }
        .render(scope, &[])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    bdoc
}

fn one(doc: &web_sys::Document, selector: &str) -> web_sys::Element {
    doc.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("nothing matches `{selector}`"))
}

fn computed(el: &web_sys::Element, prop: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value(prop)
        .unwrap()
}

fn click(el: &web_sys::Element) {
    el.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
}

fn is_open(doc: &web_sys::Document) -> bool {
    one(doc, ".rinch-color-input")
        .class_list()
        .contains("rinch-color-input--opened")
}

/// What the browser says is on top at a point, as the element itself.
fn hit(doc: &web_sys::Document, x: f64, y: f64) -> web_sys::Element {
    doc.element_from_point(x as f32, y as f32)
        .unwrap_or_else(|| panic!("nothing is under ({x}, {y})"))
}

fn centre(el: &web_sys::Element) -> (f64, f64) {
    let r = el.get_bounding_client_rect();
    assert!(
        r.width() > 0.0 && r.height() > 0.0,
        "precondition: the element has a box"
    );
    (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0)
}

/// Describe what was hit, for a failure message worth reading.
fn describe(el: &web_sys::Element) -> String {
    format!("<{} class={:?}>", el.tag_name(), el.get_attribute("class"))
}

/// The reveal: hidden while closed, shown by the `--opened` class alone.
#[wasm_bindgen_test]
fn the_backdrop_is_revealed_by_the_opened_class_in_chrome() {
    let doc = mount(true);
    let backdrop = one(&doc, ".rinch-color-input__backdrop");
    let panel = one(&doc, ".rinch-color-input__dropdown");

    assert_eq!(
        (computed(&backdrop, "display"), computed(&panel, "display")),
        ("none".to_string(), "none".to_string()),
        "a closed ColorInput shows neither box"
    );

    click(&one(&doc, ".rinch-color-input__input-group"));
    assert!(is_open(&doc), "precondition: the field opened it");
    assert_eq!(
        (computed(&backdrop, "display"), computed(&panel, "display")),
        ("block".to_string(), "block".to_string()),
        "the one class reveals both"
    );
}

/// The ordering, measured as hit testing: beside the field the backdrop is on
/// top, on the field and inside the panel it is not.
///
/// All three points come from one open picker, because the three are one claim
/// — `999 < 1000 < 1001` — and splitting them would let a fixture pass while the
/// level it did not sample was wrong.
#[wasm_bindgen_test]
fn the_backdrop_covers_the_page_but_not_the_field_or_the_panel_in_chrome() {
    let doc = mount(true);
    click(&one(&doc, ".rinch-color-input__input-group"));
    assert!(is_open(&doc), "precondition: the picker is open");

    let backdrop = one(&doc, ".rinch-color-input__backdrop");

    // 1. Beside the field, well clear of the panel: the backdrop answers.
    let panel = one(&doc, ".rinch-color-input__dropdown");
    let pr = panel.get_bounding_client_rect();
    let beside = (pr.right() + 60.0, pr.top() + 20.0);
    let got = hit(&doc, beside.0, beside.1);
    assert_eq!(
        got,
        backdrop,
        "beside the picker the backdrop must be what a click hits, or there is \
         no dismiss region — got {}",
        describe(&got)
    );

    // 2. On the text field: the field answers, because it is lifted above the
    //    backdrop. This is the half `DropdownMenu` does not need and
    //    `ColorInput` does — the trigger is a text input you click into.
    let input = one(&doc, ".rinch-color-input__input");
    let (x, y) = centre(&input);
    let got = hit(&doc, x, y);
    assert!(
        got == input || input.contains(Some(got.unchecked_ref())),
        "a click on the text field must reach the field, not the backdrop — \
         got {}",
        describe(&got)
    );

    // 3. Inside the panel: the picker answers, because the backdrop is one
    //    level under it.
    let sat = one(&doc, ".rinch-color-picker__saturation");
    let (x, y) = centre(&sat);
    let got = hit(&doc, x, y);
    assert!(
        got == sat || sat.contains(Some(got.unchecked_ref())),
        "a click inside the dropdown must reach the picker, not the backdrop — \
         got {}",
        describe(&got)
    );
}

/// The gesture end to end: a click on the backdrop runs its `data-rid` handler
/// and closes the picker, and it only ever closes.
#[wasm_bindgen_test]
fn a_click_on_the_backdrop_closes_the_picker_in_chrome() {
    let doc = mount(true);
    let backdrop = one(&doc, ".rinch-color-input__backdrop");

    click(&one(&doc, ".rinch-color-input__input-group"));
    assert!(is_open(&doc), "precondition: the picker is open");

    click(&backdrop);
    assert!(!is_open(&doc), "the backdrop closed it");

    // `element.click()` reaches a `display: none` element, which a pointer
    // cannot — which is exactly what makes this the cheapest statement of
    // "closes, never toggles" available in a browser.
    click(&backdrop);
    assert!(!is_open(&doc), "and a second one does not re-open it");
}

/// Off mounts no backdrop, in the browser as on desktop.
#[wasm_bindgen_test]
fn close_on_click_outside_false_mounts_no_backdrop_in_chrome() {
    let doc = mount(false);
    assert!(
        doc.query_selector(".rinch-color-input__backdrop")
            .unwrap()
            .is_none(),
        "close_on_click_outside: false must mount no backdrop"
    );

    click(&one(&doc, ".rinch-color-input__input-group"));
    assert!(is_open(&doc), "the field still opens it");
    click(&one(&doc, ".rinch-color-input__input-group"));
    assert!(!is_open(&doc), "and still closes it");
}
