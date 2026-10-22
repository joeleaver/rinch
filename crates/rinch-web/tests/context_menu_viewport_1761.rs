//! A `ContextMenu` stays inside the browser's viewport (#1761), as on the
//! desktop (`crates/rinch/src/app/context_menu_viewport_1761_tests.rs`).
//!
//! The menu is shifted back by its overflow, `EDGE_MARGIN` (4px) from the
//! edge, and a menu taller than the viewport scrolls. The placement is CSS
//! (spacers in a flex frame), so the browser lays it out with no measurement
//! on rinch's side. Every row height is declared, and every expectation is
//! derived from the viewport the test page happens to have.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use rinch_web::RootHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-ctxmenu-1761-host";
const EDGE: f64 = 4.0;

fn window() -> web_sys::Window {
    web_sys::window().unwrap()
}

fn document() -> web_sys::Document {
    window().document().unwrap()
}

fn viewport() -> (f64, f64) {
    let root = document().document_element().unwrap();
    (root.client_width() as f64, root.client_height() as f64)
}

fn purge() {
    for selector in [
        &format!("[{HOST_MARKER}]") as &str,
        ".rinch-context-menu__portal",
    ] {
        if let Ok(stale) = document().query_selector_all(selector) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
    }
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    opened: Signal<bool>,
}

impl Fixture {
    /// A viewport-filling target and a menu of `rows` rows, each `row_w` x 30px.
    fn mount(rows: usize, row_w: f64) -> Self {
        purge();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let opened: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
        let opened_in = opened.clone();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| -> NodeHandle {
                let target = scope.create_element("div");
                target.set_attribute("id", "ctx-1761-target");
                target.set_attribute(
                    "style",
                    "position: fixed; top: 0; left: 0; width: 100vw; height: 100vh",
                );
                let list = scope.create_element("div");
                for i in 0..rows {
                    let row = scope.create_element("div");
                    row.set_attribute("class", &format!("row-{i}"));
                    row.set_attribute("style", &format!("width: {row_w}px; height: 30px"));
                    list.append_child(&row);
                }
                let cm = rinch::components::ContextMenu::default();
                opened_in.set(Some(cm.opened));
                cm.render(scope, &[target, list])
            },
        );
        rinch_web::set_suppress_native_context_menu(false);
        Self {
            root,
            host,
            opened: opened.get().unwrap(),
        }
    }

    fn right_click(&self, x: f64, y: f64) {
        let target = document().get_element_by_id("ctx-1761-target").unwrap();
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_button(2);
        init.set_buttons(2);
        init.set_client_x(x as i32);
        init.set_client_y(y as i32);
        let event =
            web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
        target.dispatch_event(&event).unwrap();
        assert!(
            self.opened.get(),
            "positive control: the right-click reached rinch and opened the menu"
        );
    }

    fn rect(&self, selector: &str) -> web_sys::DomRect {
        document()
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"))
            .get_bounding_client_rect()
    }

    fn menu(&self) -> web_sys::DomRect {
        self.rect(".rinch-context-menu__dropdown")
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
        purge();
    }
}

/// The most rows whose menu (rows plus `chrome` px of padding and border)
/// still fits the viewport less its two margins. Hung from y = 70 such a
/// menu overruns the bottom by more than 30px whatever the viewport.
fn rows_fitting(vh: f64, chrome: f64) -> usize {
    ((vh - 2.0 * EDGE - 1.0 - chrome) / 30.0).floor() as usize
}

#[wasm_bindgen_test]
fn a_menu_running_off_the_bottom_is_shifted_up_by_its_overflow() {
    let (_vw, vh) = viewport();
    // Measure the dropdown's padding and border with one row first.
    let probe = Fixture::mount(1, 120.0);
    probe.right_click(53.0, 70.0);
    let chrome = probe.menu().height() - 30.0;
    probe.teardown();

    let y = 70.0;
    let rows = rows_fitting(vh, chrome);
    let f = Fixture::mount(rows, 120.0);
    f.right_click(53.0, y);
    let m = f.menu();
    let height = rows as f64 * 30.0 + chrome;
    assert!(
        height < vh - 2.0 * EDGE && y + height > vh - EDGE + 10.0,
        "fixture: the menu fits the viewport but not below the pointer ({height} in {vh})"
    );
    assert_eq!(m.height(), height);
    assert_eq!(m.bottom(), vh - EDGE, "ends EDGE_MARGIN above the bottom");
    assert_eq!(m.top(), vh - EDGE - height, "shifted up by its overflow");
    assert_eq!(m.left(), 53.0, "and not moved sideways");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_menu_running_off_the_right_is_shifted_left_by_its_overflow() {
    let (vw, _vh) = viewport();
    let x = vw - 200.0;
    let f = Fixture::mount(3, 300.0);
    f.right_click(x, 70.0);
    let m = f.menu();
    assert!(
        x + m.width() > vw,
        "fixture: the menu overruns the right edge"
    );
    assert_eq!(m.right(), vw - EDGE, "ends EDGE_MARGIN left of the edge");
    assert_eq!(m.top(), 70.0, "and not moved vertically");
    f.teardown();
}

#[wasm_bindgen_test]
fn a_menu_that_fits_hangs_from_the_pointer() {
    let f = Fixture::mount(3, 120.0);
    f.right_click(53.0, 70.0);
    let m = f.menu();
    assert_eq!((m.left(), m.top()), (53.0, 70.0));
    f.teardown();
}

#[wasm_bindgen_test]
fn a_menu_taller_than_the_viewport_starts_at_the_margin_and_scrolls() {
    let (_vw, vh) = viewport();
    let rows = (vh / 30.0).ceil() as usize + 3;
    let f = Fixture::mount(rows, 120.0);
    f.right_click(53.0, 70.0);
    assert_eq!(f.menu().top(), EDGE, "starts EDGE_MARGIN below the top");
    let last = f.rect(&format!(".row-{}", rows - 1));
    assert!(
        last.bottom() > vh,
        "positive control: the last row is off-screen"
    );

    let frame = document()
        .query_selector(".rinch-context-menu__frame")
        .unwrap()
        .unwrap();
    assert!(
        frame.scroll_height() > frame.client_height(),
        "the frame scrolls"
    );
    frame.set_scroll_top(frame.scroll_height());
    let last = f.rect(&format!(".row-{}", rows - 1));
    assert!(
        last.bottom() <= vh && last.top() >= 0.0,
        "scrolled to its end, the last row is in view: {}..{}",
        last.top(),
        last.bottom()
    );
    f.teardown();
}

/// Puts `<html dir="rtl">` back to its default when the test ends, passed or
/// not, since every wasm test shares one page.
struct RtlPage;

impl RtlPage {
    fn set() -> Self {
        document()
            .document_element()
            .unwrap()
            .set_attribute("dir", "rtl")
            .unwrap();
        RtlPage
    }
}

impl Drop for RtlPage {
    fn drop(&mut self) {
        let _ = document()
            .document_element()
            .unwrap()
            .remove_attribute("dir");
    }
}

/// On a right-to-left page the spacers' row must not be mirrored: a press at
/// x = 150 still opens the menu starting at 150 (mirrored, it ends at
/// `vw - 150`), while the menu's own content stays right-to-left.
#[wasm_bindgen_test]
fn an_rtl_page_does_not_mirror_the_placement() {
    let _rtl = RtlPage::set();
    let (vw, _vh) = viewport();
    let x = 150.0;
    assert!(
        vw - x - 200.0 > x + 50.0,
        "fixture: a point well left of the centre, where the mirror would put it"
    );
    let f = Fixture::mount(3, 120.0);
    f.right_click(x, 70.0);
    let m = f.menu();
    assert_eq!((m.left(), m.top()), (x, 70.0), "starts at the pointer");
    let dropdown = document()
        .query_selector(".rinch-context-menu__dropdown")
        .unwrap()
        .unwrap();
    let direction = window()
        .get_computed_style(&dropdown)
        .unwrap()
        .unwrap()
        .get_property_value("direction")
        .unwrap();
    assert_eq!(
        direction, "rtl",
        "the menu's own content stays right-to-left"
    );
    f.teardown();
}
