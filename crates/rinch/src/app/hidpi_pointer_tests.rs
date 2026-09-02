//! Pointer coordinates are logical (CSS) pixels at every scale factor (#299).
//!
//! `RinchApp::handle_event` takes `window_size` in **physical** pixels and the
//! scale factor beside it, and derives the logical layout viewport from the two.
//! The pointer coordinates on the event are the other way round: they are
//! already logical, because `hit_test` probes the raw Taffy layout tree and
//! since #246 the document is laid out at the window's *logical* size.
//!
//! Nothing in the workspace had ever passed `handle_event` a scale factor other
//! than `1.0`, where every space coincides — which is exactly why the desktop
//! shell could hand it winit's physical pointer position for as long as it did.
//! These tests run the seam at 2x.

use super::*;
use rinch_core::element::WindowProps;
use std::cell::Cell;

/// Physical 1600x1200 at 2x — a logical 800x600 viewport, the size every other
/// test in this crate mounts at.
const PHYSICAL: (u32, u32) = (1600, 1200);
const SCALE: f64 = 2.0;

/// One `data-rid` box pinned viewport-independently at (400,200)-(500,240), so
/// the only variable in these tests is the scale factor. The handler records
/// the `ClickContext` it was given.
fn app_with_pinned_target(seen: Rc<Cell<Option<events::ClickContext>>>) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let target = scope.create_element("div");
        target.set_attribute(
            "style",
            "position: absolute; left: 400px; top: 200px; width: 100px; height: 40px",
        );
        let seen = seen.clone();
        let handler_id = events::register_handler(Rc::new(move || {
            seen.set(Some(events::get_click_context()));
        }));
        target.set_attribute("data-rid", &handler_id.0.to_string());
        root.append_child(&target);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

fn click_at(app: &mut RinchApp, x: f32, y: f32) -> Vec<AppAction> {
    let mut actions = app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PHYSICAL,
        SCALE,
    );
    actions.extend(app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
        PHYSICAL,
        SCALE,
    ));
    actions
}

/// The contract, stated as the shell must satisfy it: at 2x a click lands on
/// the box when it is given the box's **logical** centre, and lands nowhere
/// near it when given the *painted* (physical) centre the desktop shell used to
/// forward. This is the issue's own measured table, run at the seam.
#[test]
fn a_click_is_hit_tested_in_logical_pixels_at_2x() {
    let seen: Rc<Cell<Option<events::ClickContext>>> = Rc::new(Cell::new(None));
    let mut app = app_with_pinned_target(seen.clone());

    click_at(&mut app, 450.0, 220.0);
    let ctx = seen
        .take()
        .expect("the logical centre of the pinned box must take the click");
    assert_eq!(
        (ctx.element_x, ctx.element_y, ctx.element_width),
        (400.0, 200.0, 100.0),
        "the element box a handler reads is logical too"
    );

    // The same box painted: (900, 440). That is what winit reports and what the
    // shell forwarded before #299 — it is off the target and, at this fixture's
    // size, off the document's right edge entirely.
    click_at(&mut app, 900.0, 440.0);
    assert!(
        seen.take().is_none(),
        "a physical coordinate must not reach the target — if it does, \
         something downstream is still compensating for the old shell"
    );
}

/// `Drag::percent` divides the raw pointer position by `ClickContext`'s element
/// bounds, which come from `painted_element_box` at scale `1.0` (#203 PR1).
/// The two only agree if the pointer is logical as well — this is the
/// "fixed for free" consumer, pinned so it stays fixed.
#[test]
fn click_context_percentages_are_correct_at_2x() {
    let seen: Rc<Cell<Option<events::ClickContext>>> = Rc::new(Cell::new(None));
    let mut app = app_with_pinned_target(seen.clone());

    // 25% along the box: 400 + 0.25 * 100 = 425.
    click_at(&mut app, 425.0, 220.0);
    let ctx = seen.take().expect("the box must take the click");
    assert!(
        (ctx.percent_x() - 0.25).abs() < 1e-4,
        "percent_x was {}, not 0.25",
        ctx.percent_x()
    );
    assert_eq!(
        (ctx.mouse_x, ctx.mouse_y),
        (425.0, 220.0),
        "the pointer is reported in the same space as the element box"
    );
}

/// Mount the pinned-target fixture as a borderless resizable window with a
/// 12px resize inset.
fn app_with_resize_inset(seen: Rc<Cell<Option<events::ClickContext>>>) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        // Fills the viewport so a click near an edge still hits a handler when
        // it is *not* claimed as a resize.
        root.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 800px; height: 600px",
        );
        let seen = seen.clone();
        let handler_id = events::register_handler(Rc::new(move || {
            seen.set(Some(events::get_click_context()));
        }));
        root.set_attribute("data-rid", &handler_id.0.to_string());
        root
    });
    app.set_window_props(WindowProps {
        borderless: true,
        resizable: true,
        resize_inset: Some(12.0),
        ..Default::default()
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

fn resize_direction(actions: &[AppAction]) -> Option<rinch_platform::ResizeDirection> {
    actions.iter().find_map(|a| match a {
        AppAction::DragResizeWindow(d) => Some(*d),
        _ => None,
    })
}

fn hover_cursor(app: &mut RinchApp, x: f32, y: f32) -> Option<rinch_platform::CursorStyle> {
    app.handle_event(PlatformEvent::MouseMove { x, y }, PHYSICAL, SCALE)
        .iter()
        .find_map(|a| match a {
            AppAction::SetCursor(c) => Some(*c),
            _ => None,
        })
}

/// `WindowProps::resize_inset` is documented as a CSS-pixel quantity ("should
/// match the CSS padding/margin used for shadow effects"), and the pointer is
/// logical, so the whole comparison is logical. `detect_resize_edge` used to be
/// handed the **physical** `window_size` and `inset * scale_factor` — a
/// deliberate compensation for the old physical pointer, and the reason this
/// test fails before #299 in *both* directions at once.
#[test]
fn the_resize_grab_zone_is_measured_in_logical_pixels_at_2x() {
    let seen: Rc<Cell<Option<events::ClickContext>>> = Rc::new(Cell::new(None));
    let mut app = app_with_resize_inset(seen.clone());

    // 5 logical px from the right edge of an 800px-wide viewport: inside the
    // 12px zone, so an East resize. The old form compared 795 against
    // `1600 - 24` and saw an ordinary click.
    let actions = click_at(&mut app, 795.0, 300.0);
    assert_eq!(
        resize_direction(&actions),
        Some(rinch_platform::ResizeDirection::East),
        "a press 5 logical px inside the right edge must start an East resize"
    );

    // `MouseMove` runs the same test for the *cursor* rather than the drag, and
    // is a separate call site — so it gets its own assertion rather than
    // riding on the press's.
    assert_eq!(
        hover_cursor(&mut app, 795.0, 300.0),
        Some(rinch_platform::CursorStyle::EResize),
        "hovering the same point must show the East resize cursor"
    );
    // Not `None`: an ordinary hover still sets a cursor (`Auto` here, from the
    // hovered node's computed style). What must not happen is a resize cursor.
    assert_eq!(
        hover_cursor(&mut app, 20.0, 300.0),
        Some(rinch_platform::CursorStyle::Auto),
        "…and 20 logical px in is an ordinary hover, not a West resize"
    );

    // 20 logical px from the left edge: outside the 12px zone, so an ordinary
    // click. The old form compared 20 against a doubled inset of 24 and
    // swallowed it as a West resize.
    seen.set(None);
    let actions = click_at(&mut app, 20.0, 300.0);
    assert_eq!(
        resize_direction(&actions),
        None,
        "a press 20 logical px in is past the 12px inset — not a resize"
    );
    assert!(
        seen.take().is_some(),
        "…and therefore reaches the click handler"
    );
}

/// The drag ghost is the one thing in the pointer path that stays in device
/// pixels: `paint_subtree` rasterises the dragged node at `scale_factor`, and
/// the blit/`Affine::translate` that keeps the grabbed point under the cursor
/// lands in a physical-pixel framebuffer. So the anchor is captured in
/// **logical** space (it is a pointer position) and `cursor - anchor` is scaled
/// up at paint time. Before #299 the anchor was captured at `scale_factor`
/// against an already-physical pointer.
#[test]
fn the_drag_anchor_is_logical_and_the_ghost_translate_is_not() {
    let seen: Rc<Cell<Option<events::ClickContext>>> = Rc::new(Cell::new(None));
    let mut app = app_with_pinned_target(seen);
    let target = {
        let doc = app.doc().expect("mounted");
        let d = doc.borrow();
        // body → the fixture's root div → the pinned box.
        let root = *d
            .tree
            .get(d.tree.body_id)
            .expect("body")
            .children
            .first()
            .expect("the fixture root");
        *d.tree
            .get(root)
            .expect("root")
            .children
            .first()
            .expect("the pinned target")
    };

    // Press 30 logical px into the box, then drag 100 logical px right.
    app.activate_drag(target, (430.0, 210.0), (530.0, 210.0), SCALE);
    let drag = app.active_dnd.as_ref().expect("a drag was activated");
    assert_eq!(
        drag.anchor,
        (30.0, 10.0),
        "the anchor is where in the node the press landed, in logical pixels"
    );

    // What the two paint paths compute — asked of the *production* helper both
    // of them call, not recomputed here: an assertion that re-derives the
    // expression passes just as happily with the `* scale` deleted from
    // `build_scene`/`build_pixels`, which is the whole thing this test is named
    // for. The ghost must sit at the node's painted origin plus the drag
    // distance, in device pixels: (400 + 100) * 2 = 1000 across, 200 * 2 = 400
    // down.
    assert_eq!(
        drag.ghost_translate(SCALE),
        (1000.0, 400.0),
        "the ghost translate is device pixels, so it carries the scale the \
         logical anchor does not"
    );
    assert_eq!(
        drag.ghost_translate(1.0),
        (500.0, 200.0),
        "…and at 1x it degenerates to the logical offset, which is why \
         deleting the scale is invisible on a 1x machine"
    );
}
