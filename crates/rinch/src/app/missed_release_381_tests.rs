//! A desktop pointer-capture drag whose release was swallowed ends through
//! `on_cancel`, not `on_end` (issue #381).
//!
//! Desktop cannot see the button state on a move (`PlatformEvent::MouseMove`
//! carries no mask, and neither does winit's `PointerMoved`), so #189's heal —
//! which keys off a move that reports the button up — never fires here. What
//! desktop *can* see are two events that prove the release was missed:
//!
//! - **a left press while the drag is still live.** A mouse cannot be pressed
//!   twice without being released in between, so the release went somewhere
//!   else. Before #381 nothing looked, and the *next* unrelated click's
//!   release ran `finish_drag` and committed `on_end` at that click's
//!   position — a slider jumping to wherever the user clicked next.
//! - **the window losing focus.** A window that lost the keyboard to a modal,
//!   a WM grab or another application will not be sent the release.
//!
//! Both end the drag the #189 way: `on_cancel` with the last coordinates
//! `on_move` was given, before the press is dispatched. And both apply to the
//! editor's drag-select (`registry::DRAG`), which has the same shape (#294).
//!
//! Every coordinate below is off every fixed point the code has: the press, the
//! last move, the stray click and its release are four different points, so a
//! callback handed the wrong one reads a different number.

use super::*;

const WINDOW: (u32, u32) = (800, 600);

/// The button whose press arms the drag: (0,0)-(120,40).
const ARM: (f32, f32) = (60.0, 20.0);
/// The last place `on_move` saw the pointer before the release went missing.
const LAST_MOVE: (f32, f32) = (313.0, 171.0);
/// A second button, (0,100)-(120,140): its click records whether a drag was
/// still live while it ran — the ordering half of the fix.
const PROBE: (f32, f32) = (60.0, 120.0);
/// Empty page, hit by nothing with a handler.
const ELSEWHERE: (f32, f32) = (611.0, 457.0);

#[derive(Default)]
struct Log {
    moves: RefCell<Vec<(f32, f32)>>,
    ends: RefCell<Vec<(f32, f32)>>,
    cancels: RefCell<Vec<(f32, f32)>>,
    /// `Drag::is_active()` as the probe's click handler saw it.
    probe_saw_drag: RefCell<Vec<bool>>,
}

fn mount() -> (RinchApp, Rc<Log>) {
    let log = Rc::new(Log::default());
    let log_in = log.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");

        let arm = scope.create_element("div");
        arm.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 120px; height: 40px",
        );
        let rid = scope.register_handler({
            let log = log_in.clone();
            move || {
                let (m, e, c) = (log.clone(), log.clone(), log.clone());
                rinch_core::Drag::absolute()
                    .on_move(move |x, y| m.moves.borrow_mut().push((x, y)))
                    .on_end(move |x, y| e.ends.borrow_mut().push((x, y)))
                    .on_cancel(move |x, y| c.cancels.borrow_mut().push((x, y)))
                    .start();
            }
        });
        arm.set_attribute("data-rid", &rid.0.to_string());
        root.append_child(&arm);

        let probe = scope.create_element("div");
        probe.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 100px; width: 120px; height: 40px",
        );
        let rid = scope.register_handler({
            let log = log_in.clone();
            move || {
                log.probe_saw_drag
                    .borrow_mut()
                    .push(rinch_core::Drag::is_active())
            }
        });
        probe.set_attribute("data-rid", &rid.0.to_string());
        root.append_child(&probe);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    (app, log)
}

fn send(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, WINDOW, 1.0);
}

fn press(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) {
    send(app, PlatformEvent::MouseDown { x, y, button });
}

fn release(app: &mut RinchApp, (x, y): (f32, f32), button: MouseButton) {
    send(app, PlatformEvent::MouseUp { x, y, button });
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    send(app, PlatformEvent::MouseMove { x, y });
}

/// Arm the drag and move it once, then "lose" the release: the state a
/// native context menu or a WM grab leaves behind.
fn arm_and_lose_the_release(app: &mut RinchApp, log: &Log) {
    press(app, ARM, MouseButton::Left);
    move_to(app, LAST_MOVE);
    assert_eq!(
        *log.moves.borrow(),
        vec![LAST_MOVE],
        "precondition: the press armed a drag and the move drove it"
    );
}

/// Positive control: an ordinary gesture commits exactly once, and a second
/// ordinary gesture after it does too. The heal must not touch a drag whose
/// release arrives — in particular, not the one the press itself just armed.
#[test]
fn an_ordinary_drag_still_commits_through_on_end() {
    let (mut app, log) = mount();
    for round in 1..=2 {
        press(&mut app, ARM, MouseButton::Left);
        move_to(&mut app, LAST_MOVE);
        release(&mut app, ELSEWHERE, MouseButton::Left);
        assert_eq!(log.ends.borrow().len(), round, "round {round} commits");
        assert_eq!(*log.ends.borrow().last().unwrap(), ELSEWHERE);
        assert!(log.cancels.borrow().is_empty(), "nothing was cancelled");
    }
}

/// The bug as the user meets it: the next click anywhere commits the stranded
/// drag at *that click's* position. Now the press proves the release was
/// missed and the drag is torn down through `on_cancel`, at the last position
/// it was actually driven to.
#[test]
fn a_left_press_after_a_swallowed_release_cancels_rather_than_commits() {
    let (mut app, log) = mount();
    arm_and_lose_the_release(&mut app, &log);

    press(&mut app, ELSEWHERE, MouseButton::Left);
    assert_eq!(
        *log.cancels.borrow(),
        vec![LAST_MOVE],
        "the press ends the stranded drag through on_cancel, at its last move"
    );
    release(&mut app, ELSEWHERE, MouseButton::Left);
    assert!(
        log.ends.borrow().is_empty(),
        "no on_end: a release nobody saw has no commit position, and this \
         release belongs to the new press ({:?})",
        log.ends.borrow()
    );
    assert!(!rinch_core::Drag::is_active());
}

/// The drag is gone **before** the press is dispatched, so the press's own
/// handlers see no drag — not a stale one that follows the pointer while they
/// run.
#[test]
fn the_stranded_drag_is_gone_before_the_press_is_dispatched() {
    let (mut app, log) = mount();
    arm_and_lose_the_release(&mut app, &log);

    press(&mut app, PROBE, MouseButton::Left);
    assert_eq!(
        *log.probe_saw_drag.borrow(),
        vec![false],
        "the probe's click ran with the stranded drag already cancelled"
    );
    assert_eq!(*log.cancels.borrow(), vec![LAST_MOVE]);
    release(&mut app, PROBE, MouseButton::Left);
    assert!(log.ends.borrow().is_empty());
}

/// A press on the drag's own arming button supersedes the old drag through
/// `on_cancel` (#293) and arms a new one, which then commits normally. One
/// cancel, not two: the heal and the supersede must not both fire.
#[test]
fn a_press_that_arms_a_new_drag_cancels_the_old_one_once_and_keeps_the_new() {
    let (mut app, log) = mount();
    arm_and_lose_the_release(&mut app, &log);

    press(&mut app, ARM, MouseButton::Left);
    assert_eq!(*log.cancels.borrow(), vec![LAST_MOVE]);
    assert!(rinch_core::Drag::is_active(), "the new drag is live");
    release(&mut app, ELSEWHERE, MouseButton::Left);
    assert_eq!(*log.ends.borrow(), vec![ELSEWHERE], "the new drag commits");
    assert_eq!(log.cancels.borrow().len(), 1);
}

/// Only a **left** press is proof. A right or middle press during a live left
/// drag is a chord the user can really make, button held.
#[test]
fn a_right_press_during_a_live_drag_does_not_cancel_it() {
    let (mut app, log) = mount();
    arm_and_lose_the_release(&mut app, &log);

    press(&mut app, ELSEWHERE, MouseButton::Right);
    assert!(
        log.cancels.borrow().is_empty(),
        "a right press is not proof the left release was missed"
    );
    assert!(rinch_core::Drag::is_active());
    press(&mut app, ELSEWHERE, MouseButton::Middle);
    assert!(log.cancels.borrow().is_empty(), "nor is a middle press");
    assert!(rinch_core::Drag::is_active());
    rinch_core::Drag::cancel();
}

/// A window that lost focus will not be sent the release, so a blur ends the
/// drag the same way — and the release that follows the refocus commits
/// nothing.
#[test]
fn a_window_blur_cancels_a_live_drag() {
    let (mut app, log) = mount();
    arm_and_lose_the_release(&mut app, &log);

    send(&mut app, PlatformEvent::WindowFocus(false));
    assert_eq!(
        *log.cancels.borrow(),
        vec![LAST_MOVE],
        "the blur ends the drag through on_cancel"
    );
    assert!(!rinch_core::Drag::is_active());

    send(&mut app, PlatformEvent::WindowFocus(true));
    release(&mut app, ELSEWHERE, MouseButton::Left);
    assert!(log.ends.borrow().is_empty(), "nothing commits afterwards");
    assert_eq!(log.cancels.borrow().len(), 1);
}

/// A blur with no drag live is a no-op for this machinery, and a refocus is
/// never proof of anything.
#[test]
fn a_refocus_does_not_cancel_a_live_drag() {
    let (mut app, log) = mount();
    send(&mut app, PlatformEvent::WindowFocus(false));
    arm_and_lose_the_release(&mut app, &log);
    send(&mut app, PlatformEvent::WindowFocus(true));
    assert!(log.cancels.borrow().is_empty());
    assert!(rinch_core::Drag::is_active());
    rinch_core::Drag::cancel();
}

/// Another document's press or blur says nothing about this document's
/// pointer (#139): a second `RinchApp` on the thread — a DevTools window, a
/// second embedded context — must not tear this drag down.
#[test]
fn another_documents_press_or_blur_leaves_this_documents_drag_alone() {
    let (mut app, log) = mount();
    let (mut other, _other_log) = mount();
    arm_and_lose_the_release(&mut app, &log);

    press(&mut other, ELSEWHERE, MouseButton::Left);
    release(&mut other, ELSEWHERE, MouseButton::Left);
    send(&mut other, PlatformEvent::WindowFocus(false));
    assert!(
        log.cancels.borrow().is_empty(),
        "another document's press or blur cancelled this drag"
    );

    // Still this document's to finish: its own release commits it.
    release(&mut app, ELSEWHERE, MouseButton::Left);
    assert_eq!(*log.ends.borrow(), vec![ELSEWHERE]);
}

/// The editor's drag-select has the same shape (#294): armed by a press in an
/// editor, cleared only by the release. A stale one kept extending the
/// selection under a pointer with no button held. The same two events end it.
#[cfg(feature = "desktop")]
mod editor_drag_select {
    use super::*;

    const CONTAINER: usize = 987_654;

    /// The key `RinchApp::input_doc` hands the editor registry.
    fn doc(app: &RinchApp) -> Option<u64> {
        rinch_core::doc_identity(app.doc_key())
    }

    fn stale_drag_select(app: &RinchApp) {
        crate::editor::begin_drag(doc(app), CONTAINER, 3);
        assert_eq!(
            crate::editor::drag_anchor(doc(app)),
            Some((CONTAINER, 3)),
            "precondition: a drag-select is in progress"
        );
    }

    #[test]
    fn a_left_press_ends_a_stale_drag_select() {
        let (mut app, _log) = mount();
        stale_drag_select(&app);
        press(&mut app, ELSEWHERE, MouseButton::Left);
        assert_eq!(crate::editor::drag_anchor(doc(&app)), None);
        release(&mut app, ELSEWHERE, MouseButton::Left);
    }

    #[test]
    fn a_right_press_leaves_a_drag_select_alone() {
        let (mut app, _log) = mount();
        stale_drag_select(&app);
        press(&mut app, ELSEWHERE, MouseButton::Right);
        assert_eq!(
            crate::editor::drag_anchor(doc(&app)),
            Some((CONTAINER, 3))
        );
        crate::editor::end_drag(doc(&app));
    }

    #[test]
    fn a_window_blur_ends_a_drag_select() {
        let (mut app, _log) = mount();
        stale_drag_select(&app);
        send(&mut app, PlatformEvent::WindowFocus(false));
        assert_eq!(crate::editor::drag_anchor(doc(&app)), None);
    }

    #[test]
    fn another_documents_press_or_blur_leaves_a_drag_select_alone() {
        let (app, _log) = mount();
        let (mut other, _other_log) = mount();
        stale_drag_select(&app);
        press(&mut other, ELSEWHERE, MouseButton::Left);
        release(&mut other, ELSEWHERE, MouseButton::Left);
        send(&mut other, PlatformEvent::WindowFocus(false));
        assert_eq!(
            crate::editor::drag_anchor(doc(&app)),
            Some((CONTAINER, 3))
        );
        crate::editor::end_drag(doc(&app));
    }
}
