//! A gesture ends on the release of the button that started it (issue #1087).
//!
//! The desktop `MouseUp` arm used to ignore which button came up. Every
//! gesture below is armed by a **left** press, and a right or middle release
//! while the left button was still held ended it anyway: an element drag was
//! **dropped** (`data-ondrop`) wherever the pointer was, a pending one was
//! clicked, a pointer-capture `Drag` committed through `on_end`, a scrollbar
//! thumb or text-selection drag stopped following the pointer, the editor's
//! drag-select ended, and `:active` was cleared under a held button. #381 and
//! #1028 had already made a right press leave these gestures alone; the right
//! press's own release undid that.
//!
//! A browser ends a gesture on the release of the button that started it (and
//! a chorded press or release is a `pointermove` there, not a `pointerup`).
//!
//! Every fixture does the chord, proves the gesture is still live — a move
//! after the stray release still drives it, from a point no earlier event
//! used — and then shows the left release still ends it, the positive control.

use super::missed_release_1028_tests::{
    EMPTY, ON_TARGET_LIST, OVER_TARGET, SOURCE, focus_offset, mount_dnd, mount_scroller,
    mount_text, move_to, press, release, scroll_top, send, strand_a_dom_drag,
    strand_a_scrollbar_drag, strand_a_text_selection,
};
use super::*;

fn chord(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) {
    send(app, PlatformEvent::MouseDown { x, y, button });
    send(app, PlatformEvent::MouseUp { x, y, button });
}

// ── Element-to-element drag ────────────────────────────────────────────────

#[test]
fn a_right_or_middle_release_does_not_drop_a_live_dom_drag() {
    let (mut app, log) = mount_dnd();
    strand_a_dom_drag(&mut app);

    chord(&mut app, OVER_TARGET, MouseButton::Right);
    assert_eq!(log.drops.get(), 0, "a right release is not the drop");
    assert_eq!(log.ends.get(), 0);
    assert!(app.active_dnd.is_some(), "the drag is still live");

    chord(&mut app, OVER_TARGET, MouseButton::Middle);
    assert_eq!(log.drops.get(), 0, "nor is a middle release");
    assert!(app.active_dnd.is_some());

    // Positive control: the button that started it still drops it, once.
    move_to(&mut app, ON_TARGET_LIST);
    release(&mut app, ON_TARGET_LIST);
    assert_eq!(log.drops.get(), 1);
    assert_eq!(log.ends.get(), 1);
    assert!(app.active_dnd.is_none());
}

#[test]
fn a_right_release_does_not_click_a_pending_drag() {
    let (mut app, log) = mount_dnd();
    press(&mut app, SOURCE);
    assert!(app.pending_drag.is_some(), "precondition: a pending drag");

    chord(&mut app, EMPTY, MouseButton::Right);
    assert_eq!(
        log.source_clicks.get(),
        0,
        "the right release clicked the source"
    );
    assert!(app.pending_drag.is_some(), "still pending");

    release(&mut app, SOURCE);
    assert_eq!(log.source_clicks.get(), 1, "the left release clicks it");
    assert!(app.pending_drag.is_none());
}

// ── Scrollbar-thumb drag ───────────────────────────────────────────────────

#[test]
fn a_right_release_does_not_end_a_scrollbar_drag() {
    let (mut app, id) = mount_scroller();
    let at = strand_a_scrollbar_drag(&mut app, id);

    chord(&mut app, EMPTY, MouseButton::Right);
    assert!(app.scrollbar_drag.is_some(), "the thumb drag is still live");
    move_to(&mut app, (195.0, 281.0));
    let moved = scroll_top(&app, id);
    assert!(moved > at, "a move still scrolls ({at} -> {moved})");

    release(&mut app, (195.0, 281.0));
    assert!(app.scrollbar_drag.is_none());
    move_to(&mut app, (195.0, 331.0));
    assert_eq!(scroll_top(&app, id), moved, "ended by the left release");
}

// ── Read-only text selection ───────────────────────────────────────────────

#[test]
fn a_right_press_and_release_leave_a_text_selection_drag_alone() {
    let mut app = mount_text();
    let focus = strand_a_text_selection(&mut app);
    let anchor = app.text_selection.as_ref().unwrap().anchor_offset;

    // Over the text, further along: the right press used to restart the
    // selection there (and arm it as a right-button selection).
    chord(&mut app, (301.0, 9.0), MouseButton::Right);
    assert_eq!(
        app.text_selection.as_ref().map(|s| s.anchor_offset),
        Some(anchor),
        "the right press moved the selection's anchor"
    );
    assert!(app.text_selecting, "the right release ended the drag");

    move_to(&mut app, (371.0, 11.0));
    assert!(focus_offset(&app) > focus, "a move still extends it");

    release(&mut app, (371.0, 11.0));
    assert!(!app.text_selecting, "the left release ends it");
}

/// A middle press, like a right one, leaves the selection being dragged alone.
#[test]
fn a_middle_press_leaves_a_text_selection_drag_alone() {
    let mut app = mount_text();
    let focus = strand_a_text_selection(&mut app);
    let anchor = app.text_selection.as_ref().unwrap().anchor_offset;
    chord(&mut app, (301.0, 9.0), MouseButton::Middle);
    assert_eq!(
        app.text_selection.as_ref().map(|s| s.anchor_offset),
        Some(anchor)
    );
    assert!(app.text_selecting);
    move_to(&mut app, (371.0, 11.0));
    assert!(focus_offset(&app) > focus);
    release(&mut app, (371.0, 11.0));
}

/// A right press away from any text used to clear the selection under a held
/// left button.
#[test]
fn a_right_press_off_the_text_keeps_the_selection() {
    let mut app = mount_text();
    let focus = strand_a_text_selection(&mut app);
    chord(&mut app, EMPTY, MouseButton::Right);
    assert_eq!(focus_offset(&app), focus);
    assert!(app.text_selecting);
    release(&mut app, EMPTY);
}

// ── Pointer-capture `Drag` ─────────────────────────────────────────────────

mod pointer_capture {
    use super::super::missed_release_381_tests::{
        ELSEWHERE, LAST_MOVE, PROBE, arm_and_lose_the_release, mount,
    };
    use super::*;

    #[test]
    fn a_right_release_does_not_commit_a_left_drag() {
        let (mut app, log) = mount();
        arm_and_lose_the_release(&mut app, &log);

        chord(&mut app, ELSEWHERE, MouseButton::Right);
        assert!(
            log.ends.borrow().is_empty(),
            "the right release committed it"
        );
        assert!(log.cancels.borrow().is_empty());
        assert!(rinch_core::Drag::is_active());

        move_to(&mut app, (419.0, 283.0));
        assert_eq!(
            *log.moves.borrow(),
            vec![LAST_MOVE, (419.0, 283.0)],
            "a move still drives it"
        );
        release(&mut app, (421.0, 287.0));
        assert_eq!(*log.ends.borrow(), vec![(421.0, 287.0)]);
        assert!(!rinch_core::Drag::is_active());
    }

    /// A drag armed by a right press is the right button's gesture: its own
    /// release ends it. It is armed from `data-onmousedown`; a right press
    /// used to click the `data-rid` too, with a click context that said `Left`
    /// whatever the button, and since #1093 it clicks nothing.
    #[test]
    fn a_drag_armed_by_a_right_press_ends_on_the_right_release() {
        for attribute in ["data-onmousedown"] {
            let ends = Rc::new(RefCell::new(Vec::<(f32, f32)>::new()));
            let e = ends.clone();
            let mut app = RinchApp::new(move |scope: &mut RenderScope| {
                let root = scope.create_element("div");
                root.set_attribute("style", "width: 800px; height: 600px");
                let arm = scope.create_element("div");
                arm.set_attribute(
                    "style",
                    "position: absolute; left: 0px; top: 0px; width: 120px; height: 40px",
                );
                let rid = scope.register_handler({
                    let e = e.clone();
                    move || {
                        let e = e.clone();
                        rinch_core::Drag::absolute()
                            .on_end(move |x, y| e.borrow_mut().push((x, y)))
                            .start();
                    }
                });
                arm.set_attribute(attribute, &rid.0.to_string());
                root.append_child(&arm);
                root
            });
            app.mount_component(800.0, 600.0);
            app.resolve_and_repaint(800.0, 600.0);

            send(
                &mut app,
                PlatformEvent::MouseDown {
                    x: 37.0,
                    y: 23.0,
                    button: MouseButton::Right,
                },
            );
            assert!(rinch_core::Drag::is_active(), "{attribute}: armed");
            assert_eq!(
                rinch_core::get_click_context().button,
                rinch_core::events::MouseButton::Right,
                "{attribute}: the handler's click context names the right button"
            );
            move_to(&mut app, (233.0, 149.0));
            send(
                &mut app,
                PlatformEvent::MouseUp {
                    x: 239.0,
                    y: 151.0,
                    button: MouseButton::Right,
                },
            );
            assert_eq!(*ends.borrow(), vec![(239.0, 151.0)], "{attribute}");
            assert!(!rinch_core::Drag::is_active(), "{attribute}");
        }
    }

    /// A right chord on the probe's `data-onmousedown` rewrites the click
    /// context to `Right`; the
    /// drag is still judged by the button that armed it, not by the context
    /// current at the release.
    #[test]
    fn a_right_chord_on_a_handler_does_not_commit_a_left_drag() {
        let (mut app, log) = mount();
        arm_and_lose_the_release(&mut app, &log);
        chord(&mut app, PROBE, MouseButton::Right);
        assert_eq!(
            rinch_core::get_click_context().button,
            rinch_core::events::MouseButton::Right,
            "precondition: the chord's press reached a handler"
        );
        assert!(
            log.ends.borrow().is_empty(),
            "committed by the right release"
        );
        assert!(rinch_core::Drag::is_active());
        move_to(&mut app, (419.0, 283.0));
        assert_eq!(*log.moves.borrow(), vec![LAST_MOVE, (419.0, 283.0)]);
        release(&mut app, (421.0, 287.0));
        assert_eq!(*log.ends.borrow(), vec![(421.0, 287.0)]);
    }

    /// A drag armed outside any press (a timer) belongs to the button most
    /// recently pressed — every press counts, not only one that reached a
    /// handler. Here a right press on the probe's `data-onmousedown` is
    /// followed by two left
    /// presses on empty page, which set no click context: the drag is the left
    /// button's, and a left release ends it.
    #[test]
    fn a_drag_armed_outside_a_press_belongs_to_the_last_press() {
        let (mut app, log) = mount();
        chord(&mut app, PROBE, MouseButton::Right);
        chord(&mut app, ELSEWHERE, MouseButton::Left);
        chord(&mut app, (617.0, 463.0), MouseButton::Left);
        assert_eq!(
            rinch_core::get_click_context().button,
            rinch_core::events::MouseButton::Right,
            "precondition: the left presses reached no handler"
        );
        let (e, c) = (log.clone(), log.clone());
        rinch_core::Drag::absolute()
            .on_end(move |x, y| e.ends.borrow_mut().push((x, y)))
            .on_cancel(move |x, y| c.cancels.borrow_mut().push((x, y)))
            .start();

        send(
            &mut app,
            PlatformEvent::MouseUp {
                x: 107.0,
                y: 311.0,
                button: MouseButton::Right,
            },
        );
        assert!(rinch_core::Drag::is_active(), "a right release ended it");
        release(&mut app, (103.0, 307.0));
        assert_eq!(*log.ends.borrow(), vec![(103.0, 307.0)]);
        assert!(log.cancels.borrow().is_empty());

        // And the other way round: after a right press it is the right's.
        chord(&mut app, (613.0, 459.0), MouseButton::Right);
        let e = log.clone();
        rinch_core::Drag::absolute()
            .on_end(move |x, y| e.ends.borrow_mut().push((x, y)))
            .start();
        release(&mut app, (109.0, 313.0));
        assert!(rinch_core::Drag::is_active(), "a left release ended it");
        send(
            &mut app,
            PlatformEvent::MouseUp {
                x: 113.0,
                y: 317.0,
                button: MouseButton::Right,
            },
        );
        assert_eq!(*log.ends.borrow(), vec![(103.0, 307.0), (113.0, 317.0)]);
    }

    /// The editor's drag-select is armed only by a left press in an editor.
    #[cfg(feature = "desktop")]
    #[test]
    fn a_right_release_does_not_end_an_editor_drag_select() {
        const CONTAINER: usize = 876_543;
        let (mut app, _log) = mount();
        let doc = rinch_core::doc_identity(app.doc_key());
        crate::editor::begin_drag(doc, CONTAINER, 5);

        chord(&mut app, ELSEWHERE, MouseButton::Right);
        assert_eq!(
            crate::editor::drag_anchor(doc),
            Some((CONTAINER, 5)),
            "the right release ended the drag-select"
        );
        release(&mut app, ELSEWHERE);
        assert_eq!(crate::editor::drag_anchor(doc), None);
    }

    /// `:active` holds while the left button is down.
    #[test]
    fn a_right_release_does_not_clear_active() {
        let (mut app, log) = mount();
        press(&mut app, (97.0, 29.0));
        let active = app.doc.as_ref().unwrap().borrow().tree.active_node;
        assert!(active.is_some(), "precondition: the press set :active");

        chord(&mut app, ELSEWHERE, MouseButton::Right);
        assert_eq!(app.doc.as_ref().unwrap().borrow().tree.active_node, active);

        release(&mut app, (97.0, 29.0));
        assert_eq!(app.doc.as_ref().unwrap().borrow().tree.active_node, None);
        assert_eq!(log.ends.borrow().len(), 1);
    }
}
