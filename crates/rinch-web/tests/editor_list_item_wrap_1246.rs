//! A list item whose text wraps keeps its marker beside the text's first line
//! on the web too (#1246).
//!
//! The shared editor stylesheet makes a task item a wrapping flex row of its
//! checkbox `::before` and its paragraph. The paragraph kept `flex-basis:
//! auto`, so Chrome sized it at its max-content width and `flex-wrap` moved a
//! long one onto a flex line of its own: the checkbox alone on a line, the text
//! below it. A `ul`/`ol` item is `display: list-item` on the web (rinch-web's
//! override, native `::marker`), so it never had the bug; it is pinned here as
//! the control that the shared rules leave it alone.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test editor_list_item_wrap_1246
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::{Pos, Selection};
use rinch_web::{RootHandle, create_editor};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

const HOST_MARKER: &str = "data-test-host-list-wrap";

const LONG: &str = "the edges of the world are not void or grid: they thin into pencil \
     sketch, then paper, then handwriting, the description of the place before she \
     imagined it, which is the signature look of her mind";

fn clear() {
    if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
        for i in 0..stale.length() {
            if let Some(node) = stale.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
            {
                el.remove();
            }
        }
    }
}

/// One editor 400px wide over `html`, then `command` (if any).
fn mount(html: &str, command: Option<&str>) -> (RootHandle, web_sys::Element) {
    clear();
    let host = document().create_element("div").unwrap();
    host.set_attribute(HOST_MARKER, "").unwrap();
    host.set_attribute(
        "style",
        "font-family: sans-serif; font-size: 16px; width: 400px;",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    assert!(handle.load_html(html));
    if let Some(c) = command {
        handle.set_selection(Selection::cursor(Pos(1)));
        assert!(handle.command(c), "{c} runs");
    }
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| mounted.mount(scope),
    );
    (root, host)
}

fn el(sel: &str) -> web_sys::Element {
    document()
        .query_selector(sel)
        .unwrap()
        .unwrap_or_else(|| panic!("{sel}"))
}

fn computed(e: &web_sys::Element, prop: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(e)
        .unwrap()
        .unwrap()
        .get_property_value(prop)
        .unwrap()
}

/// The rects of the first and last characters of `p`'s first text node.
fn first_and_last_char(p: &web_sys::Element) -> (web_sys::DomRect, web_sys::DomRect) {
    let text = p.first_child().expect("a text node");
    let n = text.text_content().unwrap().chars().count() as u32;
    let rect = |i: u32| {
        let range = document().create_range().unwrap();
        range.set_start(&text, i).unwrap();
        range.set_end(&text, i + 1).unwrap();
        range.get_bounding_client_rect()
    };
    (rect(0), rect(n - 1))
}

#[wasm_bindgen_test]
fn a_wrapping_task_item_keeps_its_checkbox_beside_the_first_line() {
    let (root, host) = mount(&format!("<p>{LONG}</p>"), Some("toggleTaskList"));
    let item = el("[data-pm-editor] [data-pm-type=\"task_item\"]");
    assert_eq!(computed(&item, "display"), "flex", "positive control");
    let p = el("[data-pm-editor] [data-pm-type=\"task_item\"] > p");
    let (ir, pr) = (item.get_bounding_client_rect(), p.get_bounding_client_rect());
    let (first, last) = first_and_last_char(&p);
    // The checkbox `::before` is the item's first flex item; it has no rect of
    // its own to read, so its line is the item's top and its width is the gap
    // between the item's left edge and the paragraph's.
    assert!(
        (pr.top() - ir.top()).abs() < 0.5,
        "the paragraph starts on the checkbox's line (item top {}, p top {})",
        ir.top(),
        pr.top()
    );
    assert!(
        pr.left() > ir.left() + 8.0,
        "beside the checkbox (item left {}, p left {})",
        ir.left(),
        pr.left()
    );
    assert!(
        last.top() > first.top() + first.height(),
        "and wraps onto more than one line"
    );
    assert!(
        pr.right() <= ir.right() + 0.5,
        "inside the item (item right {}, p right {})",
        ir.right(),
        pr.right()
    );
    root.unmount();
    host.remove();
}

#[wasm_bindgen_test]
fn a_wrapping_bullet_item_is_a_native_list_item_with_its_text_on_the_marker_line() {
    let (root, host) = mount(&format!("<ul><li><p>{LONG}</p></li></ul>"), None);
    let li = el("[data-pm-editor] li");
    assert_eq!(computed(&li, "display"), "list-item", "rinch-web's override");
    let p = el("[data-pm-editor] li > p");
    let (lr, pr) = (li.get_bounding_client_rect(), p.get_bounding_client_rect());
    let (first, last) = first_and_last_char(&p);
    assert!((pr.top() - lr.top()).abs() < 0.5, "text starts on the li's first line");
    assert!(last.top() > first.top() + first.height(), "and wraps");
    root.unmount();
    host.remove();
}
