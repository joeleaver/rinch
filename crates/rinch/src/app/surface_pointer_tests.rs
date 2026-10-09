//! A `RenderSurface`'s pointer input on desktop: capture for the length of a
//! press, and the opt-in pointer events (`set_pointer_events`) that say which
//! device a press came from, how hard it presses, and turn two touches,
//! a trackpad pinch or Ctrl+wheel into `Pinch`.
//!
//! The runtime's half (winit's pointer source → `SurfacePointer`) is pinned in
//! `shell/rinch_runtime.rs`; the pinch arithmetic in `render_surface.rs`. What
//! is pinned here is the app's: which surface hears an event, in which
//! coordinates, as which variant.

use super::*;
use crate::render_surface::{
    RenderSurfaceHandle, SurfaceEvent, SurfacePointer, SurfacePointerKind, create_render_surface,
    set_current_pointer,
};
use rinch_core::Component;

const VP: (u32, u32) = (800, 600);

/// A 200×100 surface whose top-left corner is at (50, 50).
fn app_with_surface() -> (
    RinchApp,
    RenderSurfaceHandle,
    Rc<RefCell<Vec<SurfaceEvent>>>,
) {
    set_current_pointer(None);
    let surface = create_render_surface();
    let mounted = surface.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; position: relative");
        let frame = scope.create_element("div");
        frame.set_attribute(
            "style",
            "position: absolute; left: 50px; top: 50px; width: 200px; height: 100px",
        );
        let child = crate::render_surface::RenderSurface {
            surface: Some(mounted.clone()),
        }
        .render(scope, &[]);
        frame.append_child(&child);
        root.append_child(&frame);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let seen: Rc<RefCell<Vec<SurfaceEvent>>> = Rc::default();
    let seen_in = seen.clone();
    surface.set_event_handler(move |e| {
        // Focus comes and goes with presses; it is not what these pin.
        if !matches!(e, SurfaceEvent::FocusGained | SurfaceEvent::FocusLost) {
            seen_in.borrow_mut().push(e);
        }
    });
    (app, surface, seen)
}

fn ev(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, VP, 1.0);
}

fn down(app: &mut RinchApp, (x, y): (f32, f32)) {
    ev(app, PlatformEvent::MouseMove { x, y });
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
}

fn up(app: &mut RinchApp, (x, y): (f32, f32)) {
    ev(app, PlatformEvent::MouseMove { x, y });
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

fn moved(app: &mut RinchApp, (x, y): (f32, f32)) {
    ev(app, PlatformEvent::MouseMove { x, y });
}

/// The events as short strings, for whole-sequence assertions.
fn names(seen: &RefCell<Vec<SurfaceEvent>>) -> Vec<String> {
    seen.borrow()
        .iter()
        .map(|e| match e {
            SurfaceEvent::MouseDown { x, y, .. } => format!("down {x},{y}"),
            SurfaceEvent::MouseMove { x, y } => format!("move {x},{y}"),
            SurfaceEvent::MouseUp { x, y, .. } => format!("up {x},{y}"),
            SurfaceEvent::MouseEnter { .. } => "enter".into(),
            SurfaceEvent::MouseLeave => "leave".into(),
            SurfaceEvent::PointerDown { x, y, pointer, .. } => {
                format!("pdown {x},{y} {:?} {}", pointer.kind, pointer.pressure)
            }
            SurfaceEvent::PointerMove { x, y, pointer } => {
                format!("pmove {x},{y} {:?} {}", pointer.kind, pointer.pressure)
            }
            SurfaceEvent::PointerUp { x, y, pointer, .. } => {
                format!("pup {x},{y} {:?} {}", pointer.kind, pointer.pressure)
            }
            SurfaceEvent::PointerCancel { pointer } => format!("pcancel {}", pointer.id),
            SurfaceEvent::Pinch { x, y, scale } => format!("pinch {x},{y} {scale}"),
            SurfaceEvent::MouseWheel { delta_y, .. } => format!("wheel {delta_y}"),
            other => format!("{other:?}"),
        })
        .collect()
}

/// A press on the surface keeps the pointer until its release: the moves
/// outside it and the release outside it are the surface's, in its own
/// coordinates (negative or past its size), and the pointer leaves it only
/// once released. Before, a move off the surface during a press was nobody's
/// and the release came in window coordinates.
#[test]
fn a_press_keeps_the_pointer_until_its_release() {
    let (mut app, _surface, seen) = app_with_surface();
    moved(&mut app, (100.0, 100.0));
    down(&mut app, (100.0, 100.0));
    moved(&mut app, (400.0, 300.0));
    moved(&mut app, (20.0, 10.0));
    up(&mut app, (20.0, 10.0));
    moved(&mut app, (25.0, 10.0));
    assert_eq!(
        names(&seen),
        [
            "enter",
            "move 50,50",
            "move 50,50",
            "down 50,50",
            "move 350,250",
            "move -30,-40",
            "move -30,-40",
            "up -30,-40",
            "leave",
        ]
    );
}

/// A release over the surface after a press that stayed on it leaves the
/// pointer there: no `MouseLeave`.
#[test]
fn a_release_over_the_surface_does_not_leave_it() {
    let (mut app, _surface, seen) = app_with_surface();
    down(&mut app, (100.0, 100.0));
    up(&mut app, (110.0, 100.0));
    assert_eq!(names(&seen).last().map(String::as_str), Some("up 60,50"));
    assert!(!names(&seen).contains(&"leave".to_string()));
}

/// With pointer events on, a pen's press, moves and release say it is a pen
/// and how hard it presses; without, the same input is the mouse events it
/// always was.
#[test]
fn pointer_events_say_which_device_and_how_hard() {
    let pen = |pressure: f32| SurfacePointer {
        id: 2,
        kind: SurfacePointerKind::Pen,
        pressure,
        primary: true,
    };
    for opted_in in [false, true] {
        let (mut app, surface, seen) = app_with_surface();
        surface.set_pointer_events(opted_in);
        assert_eq!(surface.pointer_events(), opted_in);
        set_current_pointer(Some(pen(0.25)));
        down(&mut app, (100.0, 100.0));
        set_current_pointer(Some(pen(0.75)));
        moved(&mut app, (300.0, 100.0));
        set_current_pointer(Some(pen(0.0)));
        up(&mut app, (300.0, 100.0));
        set_current_pointer(None);
        let got = names(&seen);
        if opted_in {
            assert_eq!(
                got,
                [
                    "enter",
                    "pmove 50,50 Pen 0.25",
                    "pdown 50,50 Pen 0.25",
                    "pmove 250,50 Pen 0.75",
                    "pmove 250,50 Pen 0",
                    "pup 250,50 Pen 0",
                    "leave",
                ]
            );
        } else {
            assert_eq!(
                got,
                [
                    "enter",
                    "move 50,50",
                    "down 50,50",
                    "move 250,50",
                    "move 250,50",
                    "up 250,50",
                    "leave",
                ]
            );
        }
    }
}

fn finger(id: u64, down: bool) -> SurfacePointer {
    SurfacePointer {
        id,
        kind: SurfacePointerKind::Touch,
        pressure: if down { 0.5 } else { 0.0 },
        primary: id == 10,
    }
}

/// Two fingers on the surface: each is its own pointer, captured on its own,
/// and spreading them is a `Pinch` about their midpoint whose scale is the
/// new distance over the old.
#[test]
fn two_fingers_spreading_apart_pinch() {
    let (mut app, surface, seen) = app_with_surface();
    surface.set_pointer_events(true);
    set_current_pointer(Some(finger(10, true)));
    down(&mut app, (100.0, 100.0));
    set_current_pointer(Some(finger(11, true)));
    down(&mut app, (150.0, 100.0));
    seen.borrow_mut().clear();
    // The second finger moves out to 100 px from the first: twice as far.
    moved(&mut app, (200.0, 100.0));
    let pinches: Vec<String> = names(&seen)
        .into_iter()
        .filter(|n| n.starts_with("pinch"))
        .collect();
    assert_eq!(pinches, ["pinch 100,50 2"]);
    // And back in to 25 px: half the last distance... a quarter of it.
    seen.borrow_mut().clear();
    moved(&mut app, (125.0, 100.0));
    assert!(
        names(&seen).contains(&"pinch 62.5,50 0.25".to_string()),
        "{:?}",
        names(&seen)
    );
    // One finger lifts: no more pinches from the other one moving.
    set_current_pointer(Some(finger(11, false)));
    up(&mut app, (125.0, 100.0));
    seen.borrow_mut().clear();
    set_current_pointer(Some(finger(10, true)));
    moved(&mut app, (90.0, 100.0));
    assert!(!names(&seen).iter().any(|n| n.starts_with("pinch")));
    set_current_pointer(None);
}

/// The platform taking a touch away mid-press reaches the surface that held
/// it: `PointerCancel` with pointer events, a `MouseUp` without.
#[test]
fn a_cancelled_touch_ends_its_press() {
    for opted_in in [false, true] {
        let (mut app, surface, seen) = app_with_surface();
        surface.set_pointer_events(opted_in);
        set_current_pointer(Some(finger(10, true)));
        down(&mut app, (100.0, 100.0));
        moved(&mut app, (400.0, 400.0));
        seen.borrow_mut().clear();
        app.cancel_surface_pointer(finger(10, false));
        set_current_pointer(None);
        let expected = if opted_in { "pcancel 10" } else { "up 350,350" };
        assert_eq!(names(&seen).first().map(String::as_str), Some(expected));
        // The press is over: a move elsewhere is not the surface's.
        seen.borrow_mut().clear();
        moved(&mut app, (500.0, 500.0));
        assert!(
            !names(&seen).iter().any(|n| n.contains("move")),
            "{:?}",
            names(&seen)
        );
    }
}

/// Ctrl+wheel (what a browser makes of a trackpad pinch, and a mouse's zoom
/// chord) and a trackpad's pinch gesture are a `Pinch` on a surface with
/// pointer events, and a wheel event as before on one without.
#[test]
fn ctrl_wheel_and_a_trackpad_pinch_zoom_a_surface_with_pointer_events() {
    for opted_in in [false, true] {
        let (mut app, surface, seen) = app_with_surface();
        surface.set_pointer_events(opted_in);
        moved(&mut app, (100.0, 100.0));
        seen.borrow_mut().clear();
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        ev(&mut app, PlatformEvent::ModifiersChanged(ctrl));
        ev(
            &mut app,
            PlatformEvent::MouseWheel {
                x: 100.0,
                y: 100.0,
                delta_x: 0.0,
                delta_y: -100.0,
            },
        );
        ev(
            &mut app,
            PlatformEvent::ModifiersChanged(Modifiers::default()),
        );
        app.surface_pinch_gesture(0.5);
        let got = names(&seen);
        if opted_in {
            assert_eq!(
                got,
                [
                    format!("pinch 50,50 {}", std::f32::consts::E),
                    "pinch 50,50 1.5".to_string()
                ]
            );
        } else {
            assert_eq!(got, ["wheel -100"]);
        }
    }
}

#[test]
fn the_pinch_tracker_scales_by_the_spread_of_the_first_two_touches() {
    use crate::render_surface::PinchTracker;
    let mut t = PinchTracker::default();
    t.down(1, 0.0, 0.0);
    assert_eq!(t.moved(1, 10.0, 0.0), None, "one touch is no pinch");
    t.down(2, 30.0, 0.0);
    assert_eq!(t.moved(2, 50.0, 0.0), Some((30.0, 0.0, 2.0)));
    assert_eq!(t.moved(2, 50.0, 0.0), None, "no change, no pinch");
    t.down(3, 500.0, 500.0);
    assert_eq!(t.moved(3, 600.0, 600.0), None, "a third touch is not in it");
    assert_eq!(t.moved(1, 30.0, 0.0), Some((40.0, 0.0, 0.5)));
    t.up(1);
    // Touches 2 and 3 are the first two now; their spread is the new base.
    assert_eq!(
        t.moved(3, 500.0, 500.0),
        Some((275.0, 250.0, {
            let before = ((600.0f32 - 50.0).powi(2) + 600.0f32.powi(2)).sqrt();
            let after = ((500.0f32 - 50.0).powi(2) + 500.0f32.powi(2)).sqrt();
            after / before
        }))
    );
}
