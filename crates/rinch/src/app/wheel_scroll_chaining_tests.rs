//! Which container the desktop wheel scrolls (review of #2084): the nearest
//! one, from the element under the pointer up, that can still move toward the
//! wheel on an axis it overflows on — and the whole wheel goes to it.
//!
//! - **Chaining.** A list at its end lets the scroller around it take the
//!   wheel, as a browser does for a new gesture. It used to stop at the
//!   nearest container that overflowed, at its end or not, and move nothing.
//! - **One container per wheel.** A diagonal wheel over a strip that scrolls
//!   only sideways moves the strip and drops the vertical part, as Chrome and
//!   Firefox do (measured with Playwright's builds of both, a `(40, 6)` wheel:
//!   strip `(40, 0)`, page `(0, 0)`). It used to split by axis, so the
//!   vertical part scrolled the page.
//! - **The same rule as `NodeHandle::can_scroll_toward`**, which is how a
//!   wheel handler that declines (the node-graph canvas) knows what the
//!   default scroll will do: `rinch_dom::paint::scrollbar::can_scroll_toward`,
//!   on the device pixel grid (#1543) and under `lock_scroll` (#474).

use super::*;
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

fn mount_at(scale: f64, build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> RinchApp {
    let build = RefCell::new(Some(build));
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        (build.borrow_mut().take().expect("built once"))(scope)
    });
    app.set_device_pixel_ratio(scale);
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
}

fn find(app: &RinchApp, class: &str) -> NodeHandle {
    let doc = app.doc.as_ref().expect("mounted");
    let id = doc
        .borrow()
        .tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("class").is_some_and(|c| c == class))
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!(".{class} is in the document"));
    let weak: std::rc::Weak<RefCell<dyn rinch_core::dom::DomDocument>> =
        Rc::downgrade(&(doc.clone() as Rc<RefCell<dyn rinch_core::dom::DomDocument>>));
    NodeHandle::new(rinch_core::dom::NodeId(id), weak)
}

fn offset(app: &RinchApp, h: &NodeHandle) -> (f64, f64) {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree.get(h.node_id().0).unwrap().scroll_offset
}

fn center(app: &RinchApp, h: &NodeHandle) -> (f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = super::hit_testing::painted_element_box(&d.tree, h.node_id().0);
    (x + w / 2.0, y + h / 2.0)
}

/// A wheel at `at`, DOM sign (positive `dy` scrolls down), sent with winit's.
fn wheel(app: &mut RinchApp, scale: f64, at: (f32, f32), dx: f64, dy: f64) {
    let (x, y) = at;
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, scale);
    app.handle_event(
        PlatformEvent::MouseWheel {
            x,
            y,
            delta_x: -dx,
            delta_y: -dy,
        },
        VP,
        scale,
    );
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

/// A page scroller (500px, 1300px of content) holding a 100px list (400px of
/// rows) and a 120px strip that scrolls only sideways (400px wide).
fn page_with_list_and_strip() -> RinchApp {
    mount_at(1.0, |__scope| {
        rsx! {
            div { class: "page", style: "width: 600px; height: 500px; overflow-y: auto",
                div { class: "list", style: "width: 200px; height: 100px; overflow-y: auto",
                    div { style: "height: 400px" }
                }
                div { class: "strip", style: "width: 120px; height: 40px; overflow-x: auto",
                    div { style: "width: 400px; height: 20px" }
                }
                div { style: "height: 760px" }
            }
        }
    })
}

/// A list at its end hands a new wheel to the page around it; the control is
/// the same wheel while the list can still move.
#[test]
fn a_list_at_its_end_chains_to_the_page() {
    let mut app = page_with_list_and_strip();
    let (page, list) = (find(&app, "page"), find(&app, "list"));
    let at = center(&app, &list);
    wheel(&mut app, 1.0, at, 0.0, 40.0);
    assert_eq!(offset(&app, &list).1, 40.0, "control: the list moves first");
    assert_eq!(offset(&app, &page).1, 0.0);

    list.set_scroll_top(300.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    wheel(&mut app, 1.0, at, 0.0, 40.0);
    assert_eq!(offset(&app, &list).1, 300.0, "the list is at its end");
    assert_eq!(offset(&app, &page).1, 40.0, "the page took the wheel");
    // And up again, the list (which can move up) takes it back.
    let at = center(&app, &list);
    wheel(&mut app, 1.0, at, 0.0, -40.0);
    assert_eq!(offset(&app, &list).1, 260.0);
    assert_eq!(offset(&app, &page).1, 40.0);
}

/// A diagonal wheel goes to one container, whole: the strip takes `(40, 6)`
/// as `(40, 0)` and the page does not move.
#[test]
fn a_diagonal_wheel_goes_to_one_container() {
    let mut app = page_with_list_and_strip();
    let (page, strip) = (find(&app, "page"), find(&app, "strip"));
    let at = center(&app, &strip);
    wheel(&mut app, 1.0, at, 40.0, 6.0);
    assert_eq!(offset(&app, &strip), (40.0, 0.0));
    assert_eq!(
        offset(&app, &page),
        (0.0, 0.0),
        "the vertical part is dropped"
    );

    // A plain vertical wheel over the strip, which cannot move that way,
    // scrolls the page: the strip is skipped, not a wall.
    wheel(&mut app, 1.0, at, 0.0, 6.0);
    assert_eq!(offset(&app, &page), (0.0, 6.0));
}

/// At 125% with sizes off the grid, the end is the range's end on the device
/// grid (#1543): a 100px list of 570.3px stops at 469.6, not 470.3. There it
/// answers that it cannot move down, and the wheel chains to the page.
#[test]
fn at_a_fractional_scale_the_end_is_on_the_device_grid() {
    const SCALE: f64 = 1.25;
    let mut app = mount_at(SCALE, |__scope| {
        rsx! {
            div { class: "page", style: "width: 600px; height: 500px; overflow-y: auto",
                div { class: "list", style: "width: 200px; height: 100px; overflow-y: auto",
                    div { style: "height: 570.3px" }
                }
                div { style: "height: 900px" }
            }
        }
    });
    let (page, list) = (find(&app, "page"), find(&app, "list"));
    let at = center(&app, &list);
    wheel(&mut app, SCALE, at, 0.0, 1000.0);
    assert_eq!(offset(&app, &list).1, 469.6, "the wheel stops on the grid");
    assert_eq!(offset(&app, &page).1, 0.0);
    assert!(!list.can_scroll_toward(0.0, 1.0), "at its end on the grid");
    assert!(list.can_scroll_toward(0.0, -1.0));

    wheel(&mut app, SCALE, at, 0.0, 40.0);
    assert_eq!(offset(&app, &list).1, 469.6);
    assert_eq!(offset(&app, &page).1, 40.0, "the page took the wheel");
}

/// Under a `lock_scroll` overlay a list outside it cannot move (the wheel
/// does not move it, and `can_scroll_toward` says so), and nothing behind
/// takes the wheel instead; a list inside the overlay still scrolls.
#[test]
fn a_list_outside_a_lock_scroll_overlay_cannot_move() {
    let mut app = mount_at(1.0, |__scope| {
        rsx! {
            div { class: "page", style: "width: 600px; height: 500px; overflow-y: auto",
                div { class: "list", style: "width: 200px; height: 100px; overflow-y: auto",
                    div { style: "height: 400px" }
                }
                div { class: "overlay", style: "width: 200px",
                    div { class: "inside", style: "width: 200px; height: 100px; overflow-y: auto",
                        div { style: "height: 400px" }
                    }
                }
                div { style: "height: 900px" }
            }
        }
    });
    let (page, list, overlay, inside) = (
        find(&app, "page"),
        find(&app, "list"),
        find(&app, "overlay"),
        find(&app, "inside"),
    );
    assert!(
        list.can_scroll_toward(0.0, 1.0),
        "control: unlocked, it can"
    );
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_scroll_locked(true, overlay.node_id());

    assert!(!list.can_scroll_toward(0.0, 1.0), "locked out");
    assert!(!page.can_scroll_toward(0.0, 1.0), "locked out");
    assert!(inside.can_scroll_toward(0.0, 1.0), "inside the overlay");

    let at = center(&app, &list);
    wheel(&mut app, 1.0, at, 0.0, 40.0);
    assert_eq!(offset(&app, &list), (0.0, 0.0));
    assert_eq!(offset(&app, &page), (0.0, 0.0));
    let at = center(&app, &inside);
    wheel(&mut app, 1.0, at, 0.0, 40.0);
    assert_eq!(offset(&app, &inside), (0.0, 40.0));
}
