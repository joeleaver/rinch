//! `NodeHandle::bounding_rect()` and `rsx!`'s `ref:` in the browser (issue
//! #1779), the twin of `rinch-dom/tests/bounding_rect_1779_tests.rs` and
//! `rinch-macros/tests/rsx_node_ref_1779.rs`.
//!
//! The target is offset inside a `scale(2)` box, so a parent-relative answer
//! (`get_layout_bounds`) and an untransformed one both differ from the painted
//! box. The handler fixture reads the ref and the rect from inside a real
//! click rinch dispatched, and compares them with that click's
//! `ClickContext` — the frame the rect promises.
#![cfg(target_arch = "wasm32")]

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::get_click_context;
use rinch_core::reactive::ElementBounds;
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_MARKER: &str = "data-bounding-rect-1779-host";

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    fn el(&self, id: &str) -> web_sys::Element {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

fn pointer_event(el: &web_sys::Element, name: &str) {
    let r = el.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(0);
    init.set_buttons(if name == "pointerdown" { 1 } else { 0 });
    init.set_client_x((r.x() + r.width() / 2.0) as i32);
    init.set_client_y((r.y() + r.height() / 2.0) as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

#[component]
fn scene(target: Signal<Option<NodeHandle>>, seen: Seen) -> NodeHandle {
    rsx! {
        div {
            div {
                style: "position: fixed; left: 30px; top: 40px; width: 200px; height: 200px; \
                        transform: scale(2); transform-origin: 0 0; margin: 0; padding: 0",
                div {
                    id: "rect-1779-target",
                    ref: target,
                    style: "margin: 5px 0 0 10px; width: 50px; height: 20px",
                    onclick: {
                        let seen = seen.clone();
                        move || {
                            let ctx = get_click_context();
                            let rect = target.get().and_then(|h| h.bounding_rect());
                            seen.borrow_mut().push((
                                rect,
                                ElementBounds {
                                    x: ctx.element_x,
                                    y: ctx.element_y,
                                    width: ctx.element_width,
                                    height: ctx.element_height,
                                },
                            ));
                        }
                    },
                }
            }
        }
    }
}

type NodeRef = Signal<Option<NodeHandle>>;
type RefAndOpen = (NodeRef, Signal<bool>);
type Seen = Rc<RefCell<Vec<(Option<ElementBounds>, ElementBounds)>>>;

fn mount_scene() -> (Fixture, Signal<Option<NodeHandle>>, Seen) {
    let cell: Rc<Cell<Option<Signal<Option<NodeHandle>>>>> = Rc::new(Cell::new(None));
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let (cell_in, seen_in) = (cell.clone(), seen.clone());
    let f = Fixture::mount(move |scope| {
        let target = Signal::new(None);
        cell_in.set(Some(target));
        scene(scope, target, seen_in)
    });
    (f, cell.get().unwrap(), seen)
}

/// The ref is the element the markup names, and its rect is the browser's
/// painted box — not `get_layout_bounds`' parent-relative one.
#[wasm_bindgen_test]
fn the_ref_holds_the_element_and_its_rect_is_the_painted_box() {
    let (f, target, _seen) = mount_scene();
    let handle = target.get().expect("the ref was set by the render");
    let el = f.el("rect-1779-target");
    assert_eq!(
        handle.get_attribute("id").as_deref(),
        Some("rect-1779-target")
    );
    let r = el.get_bounding_client_rect();
    let b = handle
        .bounding_rect()
        .expect("a connected element has a rect");
    assert_eq!(
        (b.x, b.y, b.width, b.height),
        (
            r.x() as f32,
            r.y() as f32,
            r.width() as f32,
            r.height() as f32
        )
    );
    assert_eq!((b.x, b.y, b.width, b.height), (50.0, 50.0, 100.0, 40.0));
    assert_ne!(
        handle.get_layout_bounds().map(|(x, y, _, _)| (x, y)),
        Some((b.x, b.y)),
        "the parent-relative answer is a different one"
    );
    f.teardown();
}

/// From inside a click rinch dispatched, the ref and the rect answer, in the
/// frame of that click's `ClickContext`.
#[wasm_bindgen_test]
fn a_handler_reads_the_rect_in_the_click_context_frame() {
    let (f, _target, seen) = mount_scene();
    let el = f.el("rect-1779-target");
    pointer_event(&el, "pointerdown");
    pointer_event(&el, "pointerup");
    let seen = seen.borrow();
    assert_eq!(
        seen.len(),
        1,
        "positive control: the click reached rinch's handler"
    );
    let (rect, ctx) = seen[0];
    assert_eq!(rect, Some(ctx));
    drop(seen);
    f.teardown();
}

/// A handle out of the document answers `None`, and answers again once it is
/// put back.
#[wasm_bindgen_test]
fn a_detached_element_answers_none() {
    let (f, target, _seen) = mount_scene();
    let handle = target.get().unwrap();
    let parent = handle.parent_node().unwrap();
    handle.remove();
    assert_eq!(handle.bounding_rect(), None);
    parent.append_child(&handle);
    assert!(handle.bounding_rect().is_some());
    f.teardown();
}

#[component]
fn hidden_ref(r: Signal<Option<NodeHandle>>, open: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            style: "position: fixed; left: 300px; top: 200px",
            if open.get() {
                div { style: "display: none", span { ref: r } }
            }
        }
    }
}

#[wasm_bindgen_test]
fn a_box_less_ref_answers_zero_and_hiding_its_branch_clears_it() {
    let cell: Rc<Cell<Option<RefAndOpen>>> = Rc::new(Cell::new(None));
    let cell_in = cell.clone();
    let f = Fixture::mount(move |scope| {
        let r = Signal::new(None);
        let open = Signal::new(true);
        cell_in.set(Some((r, open)));
        hidden_ref(scope, r, open)
    });
    let (r, open) = cell.get().unwrap();
    let h = r.get().expect("ref set");
    assert_eq!(h.bounding_rect(), Some(ElementBounds::default()));
    open.set(false);
    assert!(r.get().is_none(), "the hidden branch cleared its ref");
    assert_eq!(
        h.bounding_rect(),
        None,
        "a handle out of the document answers None"
    );
    f.teardown();
}
