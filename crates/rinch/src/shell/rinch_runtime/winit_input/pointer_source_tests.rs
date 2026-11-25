use super::*;
use winit::event::{
    ButtonSource, FingerId, Force, PointerSource, TabletToolButton, TabletToolData, TabletToolKind,
};

fn tool(kind: TabletToolKind, force: Option<f64>) -> PointerSource {
    PointerSource::TabletTool {
        kind,
        data: TabletToolData {
            force: force.map(Force::Normalized),
            ..Default::default()
        },
    }
}

/// A chord's release leaves the held button down; a window blur forgets
/// every press, since the release of a button held across it may go to
/// another window (review of #1502, N3).
#[test]
fn pressed_buttons_track_chords_and_a_blur_forgets_them() {
    let mut p = PressedButtons::default();
    p.note(1, Some(MouseButton::Left), true);
    p.note(1, Some(MouseButton::Right), true);
    p.note(1, Some(MouseButton::Right), false);
    assert!(p.is_down(1), "the left button is still held");
    p.note(1, Some(MouseButton::Left), false);
    assert!(!p.is_down(1));
    p.note(1, Some(MouseButton::Left), true);
    p.note(TOUCH_POINTER_BASE, None, true);
    p.clear();
    assert!(!p.is_down(1) && !p.is_down(TOUCH_POINTER_BASE));
    p.note(TOUCH_POINTER_BASE, None, true);
    assert!(p.lift(TOUCH_POINTER_BASE) && !p.lift(TOUCH_POINTER_BASE));
}

#[test]
fn a_pen_reports_its_force_and_an_eraser_says_so() {
    let p = surface_pointer_of(&tool(TabletToolKind::Pen, Some(0.3)), true, true);
    assert_eq!(p.kind, SurfacePointerKind::Pen);
    assert!((p.pressure - 0.3).abs() < 1e-6);
    assert_eq!(p.id, TABLET_POINTER_ID);
    let e = surface_pointer_of(&tool(TabletToolKind::Eraser, Some(1.5)), true, true);
    assert_eq!(e.kind, SurfacePointerKind::Eraser);
    assert_eq!(e.pressure, 1.0, "clamped");
    // A tool without force sensing reports the Pointer Events default.
    assert_eq!(
        surface_pointer_of(&tool(TabletToolKind::Pen, None), true, true).pressure,
        0.5
    );
    assert_eq!(
        surface_pointer_of(&tool(TabletToolKind::Pen, None), true, false).pressure,
        0.0
    );
}

#[test]
fn each_finger_is_its_own_pointer_and_never_the_mouse() {
    let finger = |raw: usize| PointerSource::Touch {
        finger_id: FingerId::from_raw(raw),
        force: None,
    };
    let a = surface_pointer_of(&finger(0), true, true);
    let b = surface_pointer_of(&finger(1), false, true);
    assert_eq!(a.kind, SurfacePointerKind::Touch);
    assert_ne!(a.id, b.id);
    assert!(a.id != SurfacePointer::mouse(true).id && a.id != TABLET_POINTER_ID);
    // No finger's id is the mouse's or the tablet's, the first two
    // included (a touch base of 0 gives finger 1 the mouse's id).
    for raw in 0..4 {
        let id = surface_pointer_of(&finger(raw), false, true).id;
        assert!(id != SurfacePointer::mouse(true).id && id != TABLET_POINTER_ID);
    }
    assert_eq!(a.id, pointer_id_of(&finger(0)));
    assert!(a.primary && !b.primary);
    assert_eq!(a.pressure, 0.5, "no force measured, contact down");
}

#[test]
fn the_mouse_is_pointer_one_and_a_release_has_no_pressure() {
    let m = surface_pointer_of(&PointerSource::Mouse, true, true);
    assert_eq!(m, SurfacePointer::mouse(true));
    let pressed = surface_pointer_of_button(
        &ButtonSource::TabletTool {
            kind: TabletToolKind::Pen,
            button: TabletToolButton::Contact,
            data: TabletToolData {
                force: Some(Force::Normalized(0.8)),
                ..Default::default()
            },
        },
        true,
        true,
    );
    assert!((pressed.pressure - 0.8).abs() < 1e-6);
    let released = surface_pointer_of_button(&ButtonSource::Mouse(MouseButton::Left), true, false);
    assert_eq!(released.pressure, 0.0);
    assert_eq!(released.id, 1);
}
