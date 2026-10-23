//! Issue #1764 in a real browser: a `DropdownMenu` or `Popover` opened near
//! the viewport's bottom-right corner flips above its trigger and shifts left
//! to stay 8px inside the viewport — placed in the same task as the open
//! (`getBoundingClientRect` lays out on demand), so no frame is painted with
//! the dropdown past the edge. Twin of
//! `rinch/src/app/dropdown_viewport_1764_tests.rs`.
//!
//! Every fixture checks its own premise: the dropdown's unplaced box would run
//! past the viewport (else a fixture that passes says nothing).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test dropdown_viewport_1764
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::{
    DropdownMenu, DropdownMenuDropdown, DropdownMenuTarget, Popover, PopoverDropdown, PopoverTarget,
};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-1764";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn bdoc() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn mount(
    build: impl Fn(&mut rinch_core::dom::RenderScope) -> rinch_core::dom::NodeHandle + 'static,
) {
    let doc = bdoc();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = doc.query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = doc.create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    doc.body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

fn one(selector: &str) -> web_sys::Element {
    bdoc()
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("nothing matches `{selector}`"))
}

fn viewport() -> (f64, f64) {
    let el = bdoc().document_element().unwrap();
    (el.client_width() as f64, el.client_height() as f64)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Menu,
    Popover,
}

/// `kind` with ten 200x30 rows, its 40x20 trigger in a fixed box at
/// `wrapper_style`, opened by `open`.
fn build(kind: Kind, wrapper_style: &'static str, open: Signal<bool>) {
    mount(move |scope| {
        let trigger = scope.create_element("div");
        trigger.set_attribute("style", "width: 40px; height: 20px;");
        let rows: Vec<_> = (0..10)
            .map(|_| {
                let row = scope.create_element("div");
                row.set_attribute("style", "width: 200px; height: 30px;");
                row
            })
            .collect();
        let opened_fn: Rc<dyn Fn() -> bool> = Rc::new(move || open.get());
        let comp = match kind {
            Kind::Menu => {
                let t = DropdownMenuTarget.render(scope, &[trigger]);
                let d = DropdownMenuDropdown.render(scope, &rows);
                DropdownMenu {
                    opened_fn: Some(opened_fn),
                    ..Default::default()
                }
                .render(scope, &[t, d])
            }
            Kind::Popover => {
                let t = PopoverTarget.render(scope, &[trigger]);
                let d = PopoverDropdown.render(scope, &rows);
                Popover {
                    opened_fn: Some(opened_fn),
                    ..Default::default()
                }
                .render(scope, &[t, d])
            }
        };
        comp.set_attribute("data-probe", "root");
        let wrapper = scope.create_element("div");
        wrapper.set_attribute("style", wrapper_style);
        wrapper.append_child(&comp);
        wrapper
    });
}

fn panel(kind: Kind) -> web_sys::Element {
    one(match kind {
        Kind::Menu => "[data-probe=root] .rinch-dropdown-menu__dropdown",
        Kind::Popover => "[data-probe=root] .rinch-popover__dropdown",
    })
}

/// Opened in the corner: above the trigger, inside the viewport, at once.
fn corner(kind: Kind, offset: f64) {
    let open = Signal::new(false);
    build(
        kind,
        "position: fixed; right: 60px; bottom: 30px; line-height: 0;",
        open,
    );
    let root = one("[data-probe=root]").get_bounding_client_rect();
    let (vw, vh) = viewport();
    open.set(true);
    // Same task: no frame has been painted since the open.
    let p = panel(kind).get_bounding_client_rect();
    // Premise: below the trigger and from its natural x it would not fit.
    assert!(
        root.bottom() + offset + p.height() > vh && root.x() + p.width() > vw - 8.0,
        "premise: the unplaced dropdown overflows (root {:?}, panel {}x{}, viewport {vw}x{vh})",
        (root.x(), root.y()),
        p.width(),
        p.height()
    );
    assert!(
        (p.bottom() - (root.y() - offset)).abs() < 0.6,
        "flipped above its trigger: panel bottom {}, trigger top {}",
        p.bottom(),
        root.y()
    );
    assert!(
        (p.right() - (vw - 8.0)).abs() < 0.6,
        "shifted to end 8px from the right edge: panel right {}, viewport {vw}",
        p.right()
    );
    assert!(
        p.y() >= 8.0 - 0.5 && p.x() >= 8.0 - 0.5,
        "inside: {:?}",
        (p.x(), p.y())
    );
}

#[wasm_bindgen_test]
fn a_menu_in_the_corner_flips_up_and_shifts_left_in_the_same_task() {
    corner(Kind::Menu, 4.0);
}

#[wasm_bindgen_test]
fn a_popover_in_the_corner_flips_up_and_shifts_left_in_the_same_task() {
    corner(Kind::Popover, 8.0);
}

/// One that fits is where its classes put it: below, start-aligned.
#[wasm_bindgen_test]
fn a_menu_that_fits_is_not_moved() {
    let open = Signal::new(false);
    build(
        Kind::Menu,
        "position: fixed; left: 30px; top: 20px; line-height: 0;",
        open,
    );
    open.set(true);
    let root = one("[data-probe=root]").get_bounding_client_rect();
    let p = panel(Kind::Menu).get_bounding_client_rect();
    assert!(
        (p.x() - root.x()).abs() < 0.6 && (p.y() - (root.bottom() + 4.0)).abs() < 0.6,
        "below its trigger: panel {:?}, trigger {:?}",
        (p.x(), p.y()),
        (root.x(), root.bottom())
    );
    assert!(
        panel(Kind::Menu)
            .get_attribute("style")
            .is_none_or(|s| !s.contains("max-height")),
        "not capped"
    );
}
