//! The `Tabs` CSS hooks in a real browser, issue #760.
//!
//! `rinch-components` ships **one** stylesheet to both backends, so a rule the
//! component never gives a hook to is dead on the web too. This is the web twin
//! of `rinch/src/app/css_hook_760_tests.rs`, and it answers the one question
//! the desktop half cannot: whether Chrome agrees that the sheet now styles the
//! active tab and hides the inactive panel, and whether the underline's declared
//! `transition` really runs on a switch.
//!
//! The underline is where the two backends had *different* reasons for the same
//! silence. On desktop the sheet's `::after` indicator never existed at all
//! (rinch materialises no pseudo-element for `content: ''`), so the component
//! drew a `<div>` with inline styles instead; in Chrome the `::after` did exist,
//! and was simply never given `data-active`, so it sat at `transparent` under an
//! opaque `<div>` that had no transition of its own. Either way the 150ms
//! animation the sheet declares could not run. It is an element on both
//! backends now, and `getAnimations()` below is Chrome saying so.
//!
//! **Mount through `rinch_web::mount_into`, not a hand-rolled `RenderScope`.**
//! `mount_tree` is what pushes the reactive owner; without it the switch effect
//! is disposed immediately and clicking a tab changes nothing — a silent failure
//! that reads as a CSS bug.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::{Tab, Tabs, TabsList, TabsPanel};
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-760";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A two-tab `Tabs` in a fresh host, with "a" active.
///
/// `setup_theme_css` (inside `mount_into`) carries `generate_component_css()`
/// with it, so the real tabs stylesheet is in the page — and with it the theme's
/// `--rinch-primary-color`, so the active colour is a real colour rather than an
/// invalid `var()` that computes to the same thing in both states.
fn mount_tabs() -> web_sys::Document {
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
    bdoc.body().unwrap().append_child(&host).unwrap();

    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let label_a = scope.create_text("A");
        let tab_a = Tab {
            value: "a".to_string(),
            ..Default::default()
        }
        .render(scope, &[label_a]);
        let label_b = scope.create_text("B");
        let tab_b = Tab {
            value: "b".to_string(),
            ..Default::default()
        }
        .render(scope, &[label_b]);
        let list = TabsList::default().render(scope, &[tab_a, tab_b]);
        let panel_a = TabsPanel {
            value: "a".to_string(),
        }
        .render(scope, &[]);
        let panel_b = TabsPanel {
            value: "b".to_string(),
        }
        .render(scope, &[]);
        Tabs {
            value: "a".to_string(),
            ..Default::default()
        }
        .render(scope, &[list, panel_a, panel_b])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    bdoc
}

fn all(doc: &web_sys::Document, selector: &str) -> Vec<web_sys::Element> {
    let list = doc.query_selector_all(selector).unwrap();
    (0..list.length())
        .filter_map(|i| {
            list.get(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        })
        .collect()
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

/// The browser's own list of running animations on an element.
/// Called through `Reflect` because `Element::get_animations` sits behind
/// web-sys' unstable APIs.
fn running_animations(el: &web_sys::Element) -> u32 {
    let f = js_sys::Reflect::get(el, &JsValue::from_str("getAnimations")).unwrap();
    let f: js_sys::Function = f.dyn_into().unwrap();
    let arr: js_sys::Array = f.call0(el).unwrap().dyn_into().unwrap();
    arr.length()
}

/// Whether `el` carries `class` as a whitespace-separated word. `Element` has no
/// `class_list` in this web-sys, so read the attribute the way the DOM does.
fn has_class(el: &web_sys::Element, class: &str) -> bool {
    el.get_attribute("class")
        .is_some_and(|c| c.split_whitespace().any(|one| one == class))
}

fn click(el: &web_sys::Element) {
    el.dyn_ref::<web_sys::HtmlElement>().unwrap().click();
}

async fn next_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Both hooks are set, and Chrome's own cascade acts on both of them.
#[wasm_bindgen_test]
async fn a_tab_switch_moves_the_hooks_and_the_browser_restyles() {
    let doc = mount_tabs();
    let tabs = all(&doc, ".rinch-tabs__tab");
    let panels = all(&doc, ".rinch-tabs__panel");
    assert_eq!(tabs.len(), 2);
    assert_eq!(panels.len(), 2);

    assert_eq!(
        tabs[0].get_attribute("data-active").as_deref(),
        Some("true")
    );
    assert_eq!(
        tabs[1].get_attribute("data-active").as_deref(),
        Some("false")
    );
    assert!(has_class(&tabs[0], "rinch-tabs__tab--active"));
    assert!(!has_class(&tabs[1], "rinch-tabs__tab--active"));

    // The inactive panel is hidden by the `hidden` attribute, present-form.
    assert_eq!(panels[0].get_attribute("hidden"), None);
    assert_eq!(panels[1].get_attribute("hidden").as_deref(), Some(""));
    assert_ne!(computed(&panels[0], "display"), "none");
    assert_eq!(
        computed(&panels[1], "display"),
        "none",
        "Chrome hides it — `.rinch-tabs__panel[hidden]`, and the UA sheet, agree"
    );

    // The active tab's colour comes from the cascade, and differs from the
    // inactive one's. Sampled as a difference rather than an absolute so the
    // fixture does not pin the theme's palette.
    let active_colour = computed(&tabs[0], "color");
    assert_ne!(
        active_colour,
        computed(&tabs[1], "color"),
        "the two tabs are different colours, so `[data-active=\"true\"]` is live"
    );

    click(&tabs[1]);
    next_frame().await;

    assert_eq!(
        tabs[0].get_attribute("data-active").as_deref(),
        Some("false")
    );
    assert_eq!(
        tabs[1].get_attribute("data-active").as_deref(),
        Some("true")
    );
    assert!(!has_class(&tabs[0], "rinch-tabs__tab--active"));
    assert!(has_class(&tabs[1], "rinch-tabs__tab--active"));
    assert_eq!(panels[0].get_attribute("hidden").as_deref(), Some(""));
    assert_eq!(panels[1].get_attribute("hidden"), None);
    assert_eq!(computed(&panels[0], "display"), "none");
    assert_ne!(computed(&panels[1], "display"), "none");
}

/// **The user-visible half.** Chrome runs the underline's 150ms
/// `background-color` transition on a switch — on the arriving tab and the
/// leaving one alike.
#[wasm_bindgen_test]
async fn the_underline_animates_in_chrome() {
    let doc = mount_tabs();
    let tabs = all(&doc, ".rinch-tabs__tab");
    let bars = all(&doc, ".rinch-tabs__tab-indicator");
    assert_eq!(
        bars.len(),
        2,
        "the `default` variant appends one underline element per tab"
    );

    assert!(
        computed(&bars[0], "transition-property").contains("background-color"),
        "precondition: the underline declares the transition, from the sheet — \
         got {:?}",
        computed(&bars[0], "transition-property")
    );
    assert_ne!(
        computed(&bars[0], "background-color"),
        computed(&bars[1], "background-color"),
        "precondition: the active underline is painted and the inactive one is \
         not, so a switch really changes the property. Equal values would put \
         this fixture on the fixed point where a dead hook passes"
    );
    assert_eq!(
        running_animations(&bars[0]),
        0,
        "precondition: nothing is animating before the switch"
    );

    click(&tabs[1]);
    next_frame().await;

    assert_eq!(
        running_animations(&bars[1]),
        1,
        "the arriving underline animates"
    );
    assert_eq!(
        running_animations(&bars[0]),
        1,
        "and so does the leaving one"
    );
}
