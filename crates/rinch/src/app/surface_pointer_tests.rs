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
                    // The wheel turned towards the user (a negative winit
                    // `delta_y`) zooms out, as a browser's positive `deltaY`.
                    format!("pinch 50,50 {}", (-1.0f32).exp()),
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

// ── Ending a capture without its release, chords, zoom sign (review of #1502) ─

/// A press on the surface whose release is swallowed (alt-tab, a WM grab)
/// does not leave the surface holding the pointer for the rest of the
/// session (#189/#381's shape): a window blur ends it as it ends every other
/// press gesture (`heal_missed_release`), and the surface hears it end — a
/// `MouseUp` where the cursor is, without pointer events. Then the pointer is
/// a hover like any other: leaving the surface is a `MouseLeave`, not a move.
#[test]
fn a_blur_mid_press_ends_the_surfaces_capture() {
    let (mut app, _surface, seen) = app_with_surface();
    down(&mut app, (100.0, 100.0));
    seen.borrow_mut().clear();
    ev(&mut app, PlatformEvent::WindowFocus(false));
    assert_eq!(names(&seen), ["up 50,50"]);
    seen.borrow_mut().clear();
    // Back in the window with no button down, hovering elsewhere.
    moved(&mut app, (400.0, 300.0));
    assert_eq!(names(&seen), ["leave"]);
}

/// A left press elsewhere after a swallowed release proves the release went
/// missing: the surface hears its press end at that press, and the new
/// press's drag and release are not the surface's. (The move that brings the
/// pointer to the new press is still the surface's: until the press, nothing
/// tells it from a drag.)
#[test]
fn a_later_press_elsewhere_ends_the_capture_and_keeps_its_own_release() {
    let (mut app, _surface, seen) = app_with_surface();
    down(&mut app, (100.0, 100.0));
    // The release is swallowed. The user clicks elsewhere in the window.
    seen.borrow_mut().clear();
    down(&mut app, (600.0, 500.0));
    assert_eq!(names(&seen), ["move 550,450", "up 550,450"]);
    seen.borrow_mut().clear();
    moved(&mut app, (610.0, 500.0));
    up(&mut app, (610.0, 500.0));
    assert_eq!(names(&seen), ["leave"]);
}

/// A right press and release while the left button holds the surface is a
/// chord (#1087): the surface hears both, in its own coordinates, and the
/// left press keeps the pointer until the left release.
#[test]
fn a_chord_release_does_not_end_the_left_press_capture() {
    let (mut app, _surface, seen) = app_with_surface();
    down(&mut app, (100.0, 100.0));
    seen.borrow_mut().clear();
    let right = MouseButton::Right;
    ev(
        &mut app,
        PlatformEvent::MouseDown {
            x: 100.0,
            y: 100.0,
            button: right,
        },
    );
    moved(&mut app, (500.0, 100.0));
    ev(
        &mut app,
        PlatformEvent::MouseUp {
            x: 500.0,
            y: 100.0,
            button: right,
        },
    );
    moved(&mut app, (400.0, 300.0));
    let left = MouseButton::Left;
    ev(
        &mut app,
        PlatformEvent::MouseUp {
            x: 400.0,
            y: 300.0,
            button: left,
        },
    );
    assert_eq!(
        names(&seen),
        [
            "down 50,50",
            "move 450,50",
            "up 450,50",
            "move 350,250",
            "up 350,250",
            "leave",
        ]
    );
}

/// A platform `PointerCancel` (the Android touch-scroll takeover, a
/// `PointerCancel` from any shell) ends the surface's press: the surface hears
/// it (MouseUp without pointer events, PointerCancel with), and the pointer is
/// free again.
#[test]
fn a_platform_pointer_cancel_ends_the_surfaces_press() {
    let (mut app, surface, seen) = app_with_surface();
    surface.set_pointer_events(true);
    down(&mut app, (100.0, 100.0));
    seen.borrow_mut().clear();
    ev(&mut app, PlatformEvent::PointerCancel);
    moved(&mut app, (400.0, 300.0));
    let got = names(&seen);
    assert!(
        got.first().is_some_and(|e| e.starts_with("pcancel")),
        "no cancel reached the surface: {got:?}"
    );
    assert!(!got.iter().any(|e| e.starts_with("pmove")), "{got:?}");
}

/// Ctrl+wheel zoom direction. On desktop `delta_y` is winit's sign: a
/// positive delta scrolls a container UP (`old_y - delta_y`), i.e. the wheel
/// turned away from the user, which zooms IN in every browser (where it is a
/// negative `deltaY`). So a negative desktop `delta_y` (wheel towards the
/// user, scrolls down) must zoom OUT: scale < 1.
#[test]
fn ctrl_wheel_towards_the_user_zooms_out_on_desktop_as_in_the_browser() {
    let (mut app, surface, seen) = app_with_surface();
    surface.set_pointer_events(true);
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
    let scale = seen.borrow().iter().find_map(|e| match e {
        SurfaceEvent::Pinch { scale, .. } => Some(*scale),
        _ => None,
    });
    assert!(scale.is_some_and(|s| s < 1.0), "scale {scale:?}");
}

/// A press of one finger on the surface while another holds it is no proof
/// the first one's release went missing: both keep their capture, and the
/// pointer leaves the surface only once the last of them is released.
#[test]
fn a_second_fingers_press_and_release_leave_the_first_ones_capture() {
    let (mut app, surface, seen) = app_with_surface();
    surface.set_pointer_events(true);
    set_current_pointer(Some(finger(10, true)));
    down(&mut app, (100.0, 100.0));
    set_current_pointer(Some(finger(11, true)));
    down(&mut app, (150.0, 100.0));
    // Finger 11 lifts off the surface while finger 10 still holds it.
    set_current_pointer(Some(finger(11, false)));
    up(&mut app, (400.0, 300.0));
    assert!(
        !names(&seen)
            .iter()
            .any(|n| n == "leave" || n.starts_with("pcancel")),
        "{:?}",
        names(&seen)
    );
    seen.borrow_mut().clear();
    set_current_pointer(Some(finger(10, true)));
    moved(&mut app, (450.0, 300.0));
    assert_eq!(names(&seen), ["pmove 400,250 Touch 0.5"]);
    set_current_pointer(Some(finger(10, false)));
    up(&mut app, (450.0, 300.0));
    set_current_pointer(None);
    assert_eq!(names(&seen).last().map(String::as_str), Some("leave"));
}

/// A captured move with no pointer named (an embedded host, the debug
/// server, a test: nothing calls `set_current_pointer`) is the mouse with its
/// button down, so it reports the Pointer Events default `0.5`.
#[test]
fn a_captured_move_of_an_unnamed_pointer_presses_at_half() {
    let (mut app, surface, seen) = app_with_surface();
    surface.set_pointer_events(true);
    set_current_pointer(None);
    down(&mut app, (100.0, 100.0));
    seen.borrow_mut().clear();
    moved(&mut app, (400.0, 300.0));
    assert_eq!(names(&seen), ["pmove 350,250 Mouse 0.5"]);
}

/// A surface unregistered (unmounted) while it holds a press holds nothing:
/// another surface hears its enter, moves and leave again.
#[test]
fn an_unregistered_surface_holds_no_press() {
    let (mut app, surface, _seen) = app_with_surface();
    down(&mut app, (100.0, 100.0));
    assert_eq!(app.surface_captures.len(), 1, "control: captured");
    crate::render_surface::unregister_render_surface(surface.id());
    moved(&mut app, (400.0, 300.0));
    assert!(app.surface_captures.is_empty());
}

// ── REVIEW-1502 round 2 fixtures ─────────────────────────────────────────────

/// N1: two fingers pinch, a third taps (down, a jitter move, up) on the same
/// surface: the tap makes no `Pinch`, ends neither pinching finger's capture
/// (its left press heals only its own pointer), and the next move of a
/// pinching finger scales against the spread before the tap.
#[test]
fn n1_a_third_finger_tapping_during_a_pinch_changes_nothing() {
    let (mut app, surface, seen) = app_with_surface();
    surface.set_pointer_events(true);
    set_current_pointer(Some(finger(10, true)));
    down(&mut app, (100.0, 100.0));
    set_current_pointer(Some(finger(11, true)));
    down(&mut app, (150.0, 100.0));
    seen.borrow_mut().clear();
    set_current_pointer(Some(finger(12, true)));
    down(&mut app, (200.0, 130.0));
    moved(&mut app, (203.0, 131.0));
    set_current_pointer(Some(finger(12, false)));
    up(&mut app, (203.0, 131.0));
    let got = names(&seen);
    assert!(
        !got.iter()
            .any(|n| n.starts_with("pinch") || n.starts_with("pcancel")),
        "{got:?}"
    );
    assert_eq!(
        app.surface_captures.len(),
        2,
        "both pinching fingers keep the surface"
    );
    seen.borrow_mut().clear();
    set_current_pointer(Some(finger(11, true)));
    moved(&mut app, (200.0, 100.0));
    let pinches: Vec<String> = names(&seen)
        .into_iter()
        .filter(|n| n.starts_with("pinch"))
        .collect();
    assert_eq!(pinches, ["pinch 100,50 2"]);
    set_current_pointer(None);
}

/// N1b: the tracker alone — a third touch moving never reports; then the
/// first lifts and the third becomes one of the first two.
#[test]
fn n1b_the_tracker_ignores_a_third_touch_until_it_is_one_of_the_first_two() {
    let mut t = crate::render_surface::PinchTracker::default();
    t.down(1, 0.0, 0.0);
    t.down(2, 10.0, 0.0);
    t.down(3, 50.0, 50.0);
    for i in 0..5 {
        assert_eq!(
            t.moved(3, 50.0 + i as f32 * 7.3, 50.0 - i as f32 * 1.1),
            None
        );
    }
    assert_eq!(t.moved(2, 20.0, 0.0), Some((10.0, 0.0, 2.0)));
    t.up(1);
    // Now 2 (20,0) and 3 (79.2,45.6) are the pair.
    assert!(t.moved(3, 20.0, 10.0).is_some());
}

/// N2 (policy, #381 rule): a capture armed by the RIGHT button is ended by a
/// left chord press — the left-press proof does not ask which button armed the
/// press, as it already does not for a `Drag`. Pinned so the choice is
/// visible; flip the assertion if the policy changes.
#[test]
fn n2_a_left_chord_press_cancels_a_right_button_capture() {
    let (mut app, _surface, seen) = app_with_surface();
    moved(&mut app, (100.0, 100.0));
    let right = MouseButton::Right;
    ev(
        &mut app,
        PlatformEvent::MouseDown {
            x: 100.0,
            y: 100.0,
            button: right,
        },
    );
    seen.borrow_mut().clear();
    let left = MouseButton::Left;
    ev(
        &mut app,
        PlatformEvent::MouseDown {
            x: 100.0,
            y: 100.0,
            button: left,
        },
    );
    assert_eq!(names(&seen), ["up 50,50", "down 50,50"]);
}
