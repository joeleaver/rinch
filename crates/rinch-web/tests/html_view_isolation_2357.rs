//! #2357: the page's style sheets stop at an `HtmlView`'s edge, on the web
//! as on desktop. The view keeps its document in a shadow root
//! (`isolate_styles`), so a theme's `* { box-sizing: border-box }` and
//! heading rules do not match it, while the host's inherited properties
//! still reach it. `app_styles: true` puts the document in the light tree,
//! where the page's sheets match it again.
//!
//! The numbers are Chrome's for the issue's repro, the same the desktop
//! test (`crates/rinch/src/app/html_view_isolation_2357_tests.rs`) asserts.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test html_view_isolation_2357
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::Component;
use rinch_core::dom::NodeHandle;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2357";

/// The leaking rules of a theme's base styles, as the page's own sheet.
const PAGE_CSS: &str = "*, *::before, *::after { box-sizing: border-box } \
h5 { line-height: 1.5 } a { color: rgb(1, 2, 3); text-decoration: none }";

const REPRO: &str = r#"<table style="border-collapse:collapse"><tr><td id="c" style="width:200pt; padding:0 6pt; height:10px"></td></tr></table><div style="font-size:14px; line-height:1"><h5 id="h" style="margin:0; font-size:14px">Heading</h5></div><p id="p" style="margin:0">plain</p><a id="a" href="https://example.com/">link</a>"#;

thread_local! {
    static PREVIOUS: RefCell<Option<rinch_web::RootHandle>> = const { RefCell::new(None) };
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mounts the repro in an `HtmlView` under a host carrying inherited
/// properties, on a page with [`PAGE_CSS`].
fn mount(app_styles: bool) {
    mount_with(app_styles, false)
}

/// [`mount`], with `paper` too.
fn mount_with(app_styles: bool, paper: bool) {
    rinch_web::__reset_activation_state();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let sheet = document().create_element("style").unwrap();
    sheet.set_attribute(HOST_ATTR, "sheet").unwrap();
    sheet.set_text_content(Some(PAGE_CSS));
    document().head().unwrap().append_child(&sheet).unwrap();

    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 600px; \
         font-family: monospace; font-size: 13px; line-height: 2; color: rgb(255, 0, 0)",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        HtmlView {
            html: REPRO.into(),
            app_styles,
            paper,
            ..Default::default()
        }
        .render(scope, &[])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

/// The document's element `#id`: in the view's shadow root when there is
/// one, else in the page.
fn el(id: &str) -> web_sys::Element {
    let view = document()
        .query_selector(&format!("[{HOST_ATTR}] .rinch-html-view"))
        .unwrap()
        .expect("the view");
    let found = match view.shadow_root() {
        Some(shadow) => shadow.query_selector(&format!("#{id}")),
        None => view.query_selector(&format!("#{id}")),
    };
    found.unwrap().unwrap_or_else(|| panic!("no #{id}"))
}

fn style(id: &str, property: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(&el(id))
        .unwrap()
        .unwrap()
        .get_property_value(property)
        .unwrap()
}

#[wasm_bindgen_test]
fn the_pages_sheets_do_not_reach_the_document() {
    mount(false);
    let width = el("c").get_bounding_client_rect().width();
    assert!(
        (width - 282.67).abs() < 1.0,
        "#c is {width} wide; Chrome: 282.66"
    );
    let height = el("h").get_bounding_client_rect().height();
    assert!((height - 14.0).abs() < 0.5, "#h is {height} high; 14");
    assert_eq!(style("c", "box-sizing"), "content-box");
    // The browser's link style, not the page's `a` rule.
    assert_eq!(style("a", "color"), "rgb(0, 0, 238)");
    // The host's inherited properties still reach it.
    assert_eq!(style("p", "font-size"), "13px");
    assert_eq!(style("p", "line-height"), "26px");
    assert_eq!(style("p", "font-family"), "monospace");
    // `color` too: the view sets none of its own.
    assert_eq!(style("p", "color"), "rgb(255, 0, 0)");
}

#[wasm_bindgen_test]
fn paper_draws_the_document_black_on_white() {
    mount_with(false, true);
    assert_eq!(style("p", "color"), "rgb(0, 0, 0)");
    let bg = web_sys::window()
        .unwrap()
        .get_computed_style(&view_element())
        .unwrap()
        .unwrap()
        .get_property_value("background-color")
        .unwrap();
    assert_eq!(bg, "rgb(255, 255, 255)");
}

#[wasm_bindgen_test]
fn app_styles_lets_the_pages_sheets_match_inside_the_view() {
    mount(true);
    let width = el("c").get_bounding_client_rect().width();
    assert!(
        (width - 266.67).abs() < 1.0,
        "#c is {width} wide; themed: 266.67"
    );
    let height = el("h").get_bounding_client_rect().height();
    assert!(
        (height - 21.0).abs() < 0.5,
        "#h is {height} high; themed: 21"
    );
    assert_eq!(style("a", "color"), "rgb(1, 2, 3)");
}

/// An `HtmlView` holding one link, after a button, in a focus trap (as a
/// `Modal` makes one); returns the view's handle.
fn mount_trap() -> NodeHandle {
    rinch_web::__reset_activation_state();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = document().query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let held: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let h = held.clone();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let trap = scope.create_element("div");
        trap.set_attribute("data-trap-focus", "");
        let button = scope.create_element("button");
        button.set_attribute("id", "before");
        button.append_child(&scope.create_text("before"));
        trap.append_child(&button);
        let view = HtmlView {
            html: r#"<p>Some <a id="a" href="https://example.com/">mail</a></p>"#.into(),
            ..Default::default()
        }
        .render(scope, &[]);
        trap.append_child(&view);
        *h.borrow_mut() = Some(view);
        trap
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    held.take().unwrap()
}

/// Tab, as a keydown at the focused element (composed, as the browser's).
fn tab() -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_composed(true);
    init.set_key("Tab");
    init.set_code("Tab");
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let mut target = document().active_element().expect("something has focus");
    while let Some(inner) = target.shadow_root().and_then(|s| s.active_element()) {
        target = inner;
    }
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

#[wasm_bindgen_test]
fn a_link_in_the_view_is_a_tab_stop_inside_a_focus_trap_and_the_active_element() {
    let view = mount_trap();
    let button: web_sys::HtmlElement = document()
        .query_selector(&format!("[{HOST_ATTR}] #before"))
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    button.focus().unwrap();
    assert!(tab(), "the trap moves the focus itself");
    let link = el("a");
    let shadow_active = view_element().shadow_root().unwrap().active_element();
    assert!(
        shadow_active.is_some_and(|a| a.is_same_node(Some(&link))),
        "Tab from the button reaches the link in the view's shadow root"
    );
    // `NodeHandle::active_element` names the link, as on desktop, not the
    // view `document.activeElement` answers.
    let active = view.active_element().expect("the focused node");
    assert_eq!(
        active.get_attribute("href").as_deref(),
        Some("https://example.com/")
    );
}

fn view_element() -> web_sys::Element {
    document()
        .query_selector(&format!("[{HOST_ATTR}] .rinch-html-view"))
        .unwrap()
        .expect("the view")
}

#[wasm_bindgen_test]
fn text_content_and_children_of_the_view_include_the_shadow_root() {
    let view = mount_trap();
    let text = view.text_content().unwrap_or_default();
    assert!(text.ends_with("Some mail"), "{text:?}");
    // The view's `<style>` (light tree), then its body (shadow root), as
    // desktop lists them.
    let children = view.children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].tag_name().as_deref(), Some("style"));
    assert_eq!(
        children[1].get_attribute("class").as_deref(),
        Some("rinch-html-body")
    );
}

/// Clearing the element around the view forgets the view's subtree through
/// the view itself (no handle in it is discarded on its own), so the walk
/// has to go into the view's shadow root to reach the document's nodes.
#[wasm_bindgen_test]
fn clearing_around_the_view_forgets_the_nodes_in_the_shadow_root() {
    let view = mount_trap();
    let link = el("a");
    let nid = js_sys::Reflect::get(&link, &"__nid".into())
        .unwrap()
        .as_f64()
        .expect("the link is registered") as u64;
    assert!(rinch_web::web_document::__node_registry_contains(nid));
    let trap = view.parent_node().expect("the trap");
    trap.set_inner_html("");
    assert!(
        !rinch_web::web_document::__node_registry_contains(nid),
        "a node in the view's shadow root outlived its view"
    );
}
