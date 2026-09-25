//! Every way an element-to-element drag can end leaves the same state behind
//! (issue #333).
//!
//! A DOM drag (`draggable="true"` + the `data-ondrag*` suite) ends by one of
//! three routes: a `MouseUp` (a drop, or a release over nothing), an Escape
//! press, and a `PointerCancel` (a touch scroll takeover, a system gesture).
//! Each used to spell its own teardown, and the Escape arm spelled less than
//! the other two: it never reset `suppress_drag_ghost()`'s flag, never told a
//! render surface the drag was over it had left, and fired `data-ondragend`
//! under whatever `ClickContext` an earlier event left behind. So a drag whose
//! source hid the built-in ghost (it draws its own), cancelled with Escape,
//! left the *next* drag with no ghost at all.
//!
//! Each fixture below drives the real event path and asks the same question of
//! every route, because a test of one route passes throughout while another is
//! wrong — which is how the Escape arm went unnoticed next to `MouseUp`'s.

use super::*;
use crate::render_surface::{RenderSurface, SurfaceEvent, create_render_surface};
use rinch_core::Component;
use std::cell::Cell;

const WINDOW: (u32, u32) = (800, 600);

/// Where the drag is when it ends — deliberately not the press point, not the
/// target's origin and not a round number, so a `ClickContext` left over from
/// any earlier event cannot read the same.
const DRAG_END: (f32, f32) = (437.0, 243.0);

struct Fixture {
    app: RinchApp,
    /// `data-ondragend` fired, and the cursor its `ClickContext` carried.
    dragend: Rc<RefCell<Vec<(f32, f32)>>>,
    dragleave: Rc<Cell<u32>>,
}

/// A draggable source at (40,40)-(140,80) whose `data-ondragstart` hides the
/// built-in ghost — the documented way for a source that renders its own —
/// and a drop target at (400,200)-(500,300).
fn fixture() -> Fixture {
    fixture_with(true)
}

fn fixture_with(suppress_on_start: bool) -> Fixture {
    let dragend: Rc<RefCell<Vec<(f32, f32)>>> = Rc::new(RefCell::new(Vec::new()));
    let dragleave = Rc::new(Cell::new(0u32));
    let (end_sink, leave_sink) = (dragend.clone(), dragleave.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");

        let source = scope.create_element("div");
        source.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 40px; width: 100px; height: 40px",
        );
        source.set_attribute("draggable", "true");
        if suppress_on_start {
            let start = events::register_handler(Rc::new(events::suppress_drag_ghost));
            source.set_attribute("data-ondragstart", &start.0.to_string());
        }
        let sink = end_sink.clone();
        let end = events::register_handler(Rc::new(move || {
            let ctx = events::get_click_context();
            sink.borrow_mut().push((ctx.mouse_x, ctx.mouse_y));
        }));
        source.set_attribute("data-ondragend", &end.0.to_string());
        root.append_child(&source);

        let target = scope.create_element("div");
        target.set_attribute(
            "style",
            "position: absolute; left: 400px; top: 200px; width: 100px; height: 100px",
        );
        let drop = events::register_handler(Rc::new(|| {}));
        target.set_attribute("data-ondrop", &drop.0.to_string());
        let sink = leave_sink.clone();
        let leave = events::register_handler(Rc::new(move || sink.set(sink.get() + 1)));
        target.set_attribute("data-ondragleave", &leave.0.to_string());
        root.append_child(&target);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    Fixture {
        app,
        dragend,
        dragleave,
    }
}

fn send(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, WINDOW, 1.0)
}

/// Press the source, cross the threshold, and move over the target to
/// [`DRAG_END`]. Asserts the drag really is live with its ghost hidden, so a
/// teardown fixture cannot pass on a drag that never started.
fn start_drag_over_target(f: &mut Fixture) {
    events::reset_drag_ghost_visibility();
    send(
        &mut f.app,
        PlatformEvent::MouseDown {
            x: 60.0,
            y: 55.0,
            button: MouseButton::Left,
        },
    );
    send(&mut f.app, PlatformEvent::MouseMove { x: 90.0, y: 70.0 });
    send(
        &mut f.app,
        PlatformEvent::MouseMove {
            x: DRAG_END.0,
            y: DRAG_END.1,
        },
    );
    let drag = f.app.active_dnd.as_ref().expect("the drag went live");
    assert!(
        drag.over_target.is_some(),
        "the drag is over the drop target"
    );
    assert!(
        !events::is_drag_ghost_visible(),
        "positive control: the source's ondragstart hid the ghost"
    );
}

fn escape(app: &mut RinchApp) -> Vec<AppAction> {
    send(
        app,
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
    )
}

fn release(app: &mut RinchApp) -> Vec<AppAction> {
    send(
        app,
        PlatformEvent::MouseUp {
            x: DRAG_END.0,
            y: DRAG_END.1,
            button: MouseButton::Left,
        },
    )
}

/// What every route must leave behind: no drag, the ghost visible again, and
/// `data-ondragend` fired once at the drag's own last position.
fn assert_torn_down(f: &Fixture, route: &str) {
    assert!(f.app.active_dnd.is_none(), "{route}: the drag ended");
    assert!(
        events::is_drag_ghost_visible(),
        "{route}: a drag's ghost suppression ends with the drag"
    );
    assert_eq!(
        *f.dragend.borrow(),
        vec![DRAG_END],
        "{route}: ondragend fired once, reading the drag's last cursor"
    );
}

/// The issue itself.
#[test]
fn escape_restores_the_ghost_a_drag_suppressed() {
    let mut f = fixture();
    start_drag_over_target(&mut f);
    escape(&mut f.app);
    assert_torn_down(&f, "Escape");
    assert_eq!(
        f.dragleave.get(),
        1,
        "Escape: the target heard the drag leave"
    );
}

#[test]
fn a_release_restores_the_ghost_a_drag_suppressed() {
    let mut f = fixture();
    start_drag_over_target(&mut f);
    release(&mut f.app);
    assert_torn_down(&f, "MouseUp");
}

#[test]
fn a_pointer_cancel_restores_the_ghost_a_drag_suppressed() {
    let mut f = fixture();
    start_drag_over_target(&mut f);
    send(&mut f.app, PlatformEvent::PointerCancel);
    assert_torn_down(&f, "PointerCancel");
    assert_eq!(
        f.dragleave.get(),
        1,
        "PointerCancel: the target heard the drag leave"
    );
}

/// The user-visible consequence, and a second line of defence: a drag starts
/// with the ghost visible whatever the last one left behind. Here nothing
/// suppresses on the second drag's start, so the flag seen after it is exactly
/// what the start left. Driven by setting the flag directly rather than by an
/// Escape, so the fixture keeps discriminating once every teardown resets it.
#[test]
fn a_new_drag_starts_with_the_ghost_visible() {
    // This drag's start must not hide the ghost itself.
    let mut f = fixture_with(false);
    events::suppress_drag_ghost();
    send(
        &mut f.app,
        PlatformEvent::MouseDown {
            x: 60.0,
            y: 55.0,
            button: MouseButton::Left,
        },
    );
    send(&mut f.app, PlatformEvent::MouseMove { x: 90.0, y: 70.0 });
    assert!(f.app.active_dnd.is_some(), "the drag went live");
    assert!(
        events::is_drag_ghost_visible(),
        "a suppression left over from before this drag does not hide its ghost"
    );
    escape(&mut f.app);
}

/// Escape over a render surface tells the surface the drag left, as a
/// `PointerCancel` always has; it used to leave `drag_over_surface` set and the
/// surface believing a drag was still over it.
#[test]
fn escape_over_a_surface_tells_the_surface_the_drag_left() {
    let surface = create_render_surface();
    let seen: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    surface.set_event_handler(move |event| {
        let name = match event {
            SurfaceEvent::DragEnter { .. } => "enter",
            SurfaceEvent::DragLeave => "leave",
            SurfaceEvent::Drop { .. } => "drop",
            _ => return,
        };
        sink.borrow_mut().push(name);
    });
    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let source = scope.create_element("div");
        source.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 40px; width: 100px; height: 40px",
        );
        source.set_attribute("draggable", "true");
        root.append_child(&source);
        let holder = scope.create_element("div");
        holder.set_attribute(
            "style",
            "position: absolute; left: 400px; top: 200px; width: 200px; height: 150px; display: flex",
        );
        let child = RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        holder.append_child(&child);
        root.append_child(&holder);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);

    send(
        &mut app,
        PlatformEvent::MouseDown {
            x: 60.0,
            y: 55.0,
            button: MouseButton::Left,
        },
    );
    send(&mut app, PlatformEvent::MouseMove { x: 90.0, y: 70.0 });
    send(
        &mut app,
        PlatformEvent::MouseMove {
            x: DRAG_END.0,
            y: DRAG_END.1,
        },
    );
    assert_eq!(
        *seen.borrow(),
        vec!["enter"],
        "positive control: the drag is over the surface"
    );
    escape(&mut app);
    assert_eq!(*seen.borrow(), vec!["enter", "leave"]);
    assert!(
        app.drag_over_surface.is_none(),
        "no surface is under a drag"
    );
}
