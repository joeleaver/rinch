//! A press on a child of a `draggable="true"` element clicks what was
//! pressed, not the draggable (#2133).
//!
//! A draggable's click waits for the release, so that a drag can still start
//! from the press. When no drag starts, the release dispatches the click. It
//! used to walk up from the *draggable*, so a `data-rid` on a descendant was
//! never reached: a draggable `Tree` row's chevron selected the row, and a
//! card's `onclick` inside a `SortableItem` never ran. Desktop dispatches at
//! the press point (`PendingDrag::mousedown_pos` → `handle_click`), so the
//! nearest live handler of the pressed element runs (only that one: rinch's
//! click does not bubble to an outer `data-rid` on either backend).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test draggable_child_click_2133
//! ```
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::dom::NodeHandle;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2133";

thread_local! {
    static PREVIOUS: RefCell<Option<rinch_web::RootHandle>> = const { RefCell::new(None) };
}

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `build` into a fresh host, unmounting the previous test's.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) {
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
    host.set_attribute(
        "style",
        "position: fixed; left: 0px; top: 0px; width: 400px",
    )
    .unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

fn el(id: &str) -> web_sys::HtmlElement {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no element #{id}"))
        .dyn_into()
        .unwrap()
}

fn centre(el: &web_sys::Element) -> (i32, i32) {
    let r = el.get_bounding_client_rect();
    (
        (r.x() + r.width() / 2.0) as i32,
        (r.y() + r.height() / 2.0) as i32,
    )
}

fn pointer(el: &web_sys::Element, name: &str, (x, y): (i32, i32), kind: &str) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type(kind);
    init.set_button(if name == "pointermove" { -1 } else { 0 });
    init.set_buttons(if name == "pointerup" { 0 } else { 1 });
    init.set_client_x(x);
    init.set_client_y(y);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A mouse press and release at the centre of `el`, dispatched on it.
fn tap(el: &web_sys::Element, kind: &str) {
    let p = centre(el);
    pointer(el, "pointerdown", p, kind);
    pointer(el, "pointerup", p, kind);
}

/// The browser's trailing `click` (trusted under `__force_trusted_clicks`).
fn click(el: &web_sys::Element, detail: i32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(detail);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

fn counter(scope: &mut RenderScope, el: &NodeHandle, attr: &str) -> Rc<Cell<u32>> {
    let n = Rc::new(Cell::new(0u32));
    let c = n.clone();
    let id = scope.register_handler(move || c.set(c.get() + 1));
    el.set_attribute(attr, &id.0.to_string());
    n
}

/// Counters of the fixture below, in the order the fields are listed.
#[derive(Clone, Default)]
struct Counts {
    row: Rc<Cell<u32>>,
    child: Rc<Cell<u32>>,
    other: Rc<Cell<u32>>,
    fresh: Rc<Cell<u32>>,
    dragstart: Rc<Cell<u32>>,
}

impl Counts {
    fn get(&self) -> (u32, u32, u32, u32) {
        (
            self.row.get(),
            self.child.get(),
            self.other.get(),
            self.fresh.get(),
        )
    }
}

/// A draggable row (`#row`, its own click and `data-ondragstart`) holding
/// side by side: `#child` (a button with its own click), `#other` (another
/// clickable), `#plain` (no handler) and a hidden `#fresh` (a clickable that
/// a test swaps in for `#child`, as a re-render would).
fn draggable_row() -> Counts {
    let counts = Rc::new(RefCell::new(Counts::default()));
    let out = counts.clone();
    mount(move |scope| {
        let row = scope.create_element("div");
        row.set_attribute("id", "row");
        row.set_attribute("draggable", "true");
        row.set_attribute(
            "style",
            "display: flex; width: 360px; height: 40px; padding: 4px",
        );
        let mut c = counts.borrow_mut();
        c.row = counter(scope, &row, "data-rid");
        c.dragstart = counter(scope, &row, "data-ondragstart");
        let cell = |scope: &mut RenderScope, tag: &str, id: &str| {
            let e = scope.create_element(tag);
            e.set_attribute("id", id);
            e.set_attribute("style", "width: 60px; height: 30px; margin: 0; padding: 0");
            e.append_child(&scope.create_text(id));
            row.append_child(&e);
            e
        };
        let child = cell(scope, "button", "child");
        c.child = counter(scope, &child, "data-rid");
        let other = cell(scope, "span", "other");
        c.other = counter(scope, &other, "data-rid");
        cell(scope, "span", "plain");
        let fresh = cell(scope, "button", "fresh");
        fresh.set_attribute("style", "display: none");
        c.fresh = counter(scope, &fresh, "data-rid");
        row
    });
    out.borrow().clone()
}

#[wasm_bindgen_test]
fn a_click_on_a_child_of_a_draggable_runs_the_childs_handler() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    let child = el("child");
    let p = centre(&child);
    pointer(&child, "pointerdown", p, "mouse");
    assert_eq!(c.get(), (0, 0, 0, 0), "the click waits for the release");
    pointer(&child, "pointerup", p, "mouse");
    click(&child, 1);
    assert_eq!(
        c.get(),
        (0, 1, 0, 0),
        "(row, child, other, fresh): the pressed child's handler, once, and not the row's"
    );
    assert_eq!(c.dragstart.get(), 0, "no drag started");
}

#[wasm_bindgen_test]
fn a_click_on_the_draggable_or_a_plain_child_runs_the_draggables_handler() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    tap(&el("plain"), "mouse");
    click(&el("plain"), 1);
    assert_eq!(c.get(), (1, 0, 0, 0), "a child with no handler: the row's");
    let row = el("row");
    let r = row.get_bounding_client_rect();
    // The row's own padding, right of its children.
    let p = ((r.right() - 6.0) as i32, (r.y() + r.height() / 2.0) as i32);
    pointer(&row, "pointerdown", p, "mouse");
    pointer(&row, "pointerup", p, "mouse");
    click(&row, 1);
    assert_eq!(c.get(), (2, 0, 0, 0), "the draggable itself");
}

#[wasm_bindgen_test]
fn a_drag_from_a_child_starts_and_clicks_nothing() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    let child = el("child");
    let p = centre(&child);
    pointer(&child, "pointerdown", p, "mouse");
    pointer(&child, "pointermove", (p.0 + 30, p.1), "mouse");
    assert_eq!(c.dragstart.get(), 1, "positive control: the drag started");
    pointer(&child, "pointerup", (p.0 + 30, p.1), "mouse");
    click(&row_el(), 1);
    assert_eq!(c.get(), (0, 0, 0, 0), "a drag clicks nothing");
}

fn row_el() -> web_sys::Element {
    el("row").into()
}

/// Released on a different child, short of the drag threshold: the click is
/// the pressed one's, as desktop's (it clicks at the press point).
#[wasm_bindgen_test]
fn a_press_on_one_child_and_a_release_on_another_clicks_the_pressed_one() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    let child = el("child");
    let other = el("other");
    let cr = child.get_bounding_client_rect();
    let or = other.get_bounding_client_rect();
    let y = (cr.y() + cr.height() / 2.0) as i32;
    let down = (cr.right() as i32 - 1, y);
    let up = (or.x() as i32 + 1, y);
    assert!(up.0 - down.0 < 5, "the release is short of the threshold");
    pointer(&child, "pointerdown", down, "mouse");
    pointer(&other, "pointermove", up, "mouse");
    pointer(&other, "pointerup", up, "mouse");
    click(&row_el(), 1);
    assert_eq!(c.dragstart.get(), 0, "no drag");
    assert_eq!(c.get(), (0, 1, 0, 0), "(row, child, other, fresh)");
}

/// The pressed child is replaced before the release (a re-render): the click
/// goes to what is now at the press point, as desktop's hit test at
/// `mousedown_pos` does, not to the detached child.
#[wasm_bindgen_test]
fn a_child_replaced_before_the_release_clicks_what_is_at_the_press_point() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    let child = el("child");
    let fresh = el("fresh");
    let p = centre(&child);
    pointer(&child, "pointerdown", p, "mouse");
    fresh
        .set_attribute("style", "width: 60px; height: 30px; margin: 0; padding: 0")
        .unwrap();
    child.replace_with_with_node_1(&fresh).unwrap();
    assert!(
        document()
            .element_from_point(p.0 as f32, p.1 as f32)
            .is_some_and(|e| e.is_same_node(Some(&fresh))),
        "control: the replacement is at the press point"
    );
    pointer(&fresh, "pointerup", p, "mouse");
    assert_eq!(c.get(), (0, 0, 0, 1), "(row, child, other, fresh)");
}

/// A touch tap (long-press activation) on a child clicks the child.
#[wasm_bindgen_test]
fn a_touch_tap_on_a_child_of_a_draggable_runs_the_childs_handler() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    tap(&el("child"), "touch");
    click(&el("child"), 1);
    assert_eq!(c.get(), (0, 1, 0, 0), "(row, child, other, fresh)");
}

/// Enter on a focused button inside a draggable: the browser's own click,
/// no pointer gesture. Unaffected by the pointer path.
#[wasm_bindgen_test]
fn a_keyboard_click_on_a_button_inside_a_draggable_runs_the_button_once() {
    let c = draggable_row();
    rinch_web::__force_trusted_clicks(true);
    let child = el("child");
    child.focus().unwrap();
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Enter");
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    child.dispatch_event(&ev).unwrap();
    click(&child, 0);
    assert_eq!(c.get(), (0, 1, 0, 0), "(row, child, other, fresh)");
}

// ── Tree ──────────────────────────────────────────────────────────────────

fn tree() -> UseTreeReturn {
    let state = UseTreeReturn::new(UseTreeOptions::default());
    mount(move |scope| {
        Tree {
            data: vec![
                TreeNodeData::new("a", "Apples").with_child(TreeNodeData::new("a1", "Avocado")),
                TreeNodeData::new("b", "Beets"),
            ],
            tree: Some(state),
            select_on_click: true,
            draggable: true,
            ..Default::default()
        }
        .render(scope, &[])
    });
    state
}

fn tree_part(value: &str, part: &str) -> web_sys::Element {
    document()
        .query_selector(&format!(
            "[{HOST_ATTR}] li[data-value='{value}'] > .rinch-tree__node-content {part}"
        ))
        .unwrap()
        .unwrap_or_else(|| panic!("no {part} in row {value}"))
}

/// A draggable `Tree` row's chevron expands the row and does not select it,
/// as on desktop (`tree_drop_tests`); a press on the label selects.
#[wasm_bindgen_test]
fn a_draggable_tree_rows_chevron_expands_without_selecting() {
    let state = tree();
    let content = document()
        .query_selector(&format!(
            "[{HOST_ATTR}] li[data-value='a'] > .rinch-tree__node-content"
        ))
        .unwrap()
        .unwrap();
    assert_eq!(
        content.get_attribute("draggable").as_deref(),
        Some("true"),
        "control: the row is draggable"
    );
    tap(&tree_part("a", ".rinch-tree__chevron"), "mouse");
    assert!(
        state.expanded.get().contains("a"),
        "the chevron expands: {:?}",
        state.expanded.get()
    );
    assert!(
        state.selected.get().is_empty(),
        "the chevron selects nothing: {:?}",
        state.selected.get()
    );
    tap(&tree_part("b", ".rinch-tree__label"), "mouse");
    assert!(
        state.selected.get().contains("b"),
        "positive control: the label selects"
    );
}

// ── SortableItem ──────────────────────────────────────────────────────────

/// A card inside a `SortableItem` (no handle: the item is the draggable)
/// runs its own `onclick`.
#[wasm_bindgen_test]
fn a_card_inside_a_sortable_item_runs_its_onclick() {
    let clicks = Rc::new(Cell::new(0u32));
    let c = clicks.clone();
    mount(move |__scope| {
        let c = c.clone();
        rsx! {
            SortableList {
                group: "cards", id: "list",
                for key in vec!["x".to_string(), "y".to_string()] {
                    SortableItem {
                        key: key.clone(), value: key.clone(),
                        div {
                            class: "card-2133",
                            style: "height: 30px",
                            onclick: {
                                let c = c.clone();
                                move || c.set(c.get() + 1)
                            },
                            {key.clone()}
                        }
                    }
                }
            }
        }
    });
    let card = document()
        .query_selector(&format!("[{HOST_ATTR}] .card-2133"))
        .unwrap()
        .unwrap();
    assert!(
        card.closest("[draggable=\"true\"]").unwrap().is_some(),
        "control: the card is inside a draggable"
    );
    tap(&card, "mouse");
    assert_eq!(clicks.get(), 1, "the card's onclick ran");
}
