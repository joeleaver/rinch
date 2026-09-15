//! #763, at the shell — a paused `@keyframes` animation lets the app go idle.
//!
//! The rinch-dom half is `crates/rinch-dom/tests/paused_animation_frame_tests.rs`,
//! which asserts on what `tick_animations` answers and what it leaves dirty.
//! This file asserts the two things those decide, because the `AboutToWait`
//! frame clock (`app/event_dispatch.rs`) reads the animation state **twice**,
//! and a fix that satisfied one reader would still spin on the other:
//!
//! - **the redraw request** — `tick_animations`' answer, plus any node the tick
//!   left dirty, which `resolve_and_repaint` turns into a redraw of its own;
//! - **K23's `had_running`** — "was there anything to tick", read *before* the
//!   tick so that the tick which *finishes* an animation still reaches the
//!   glass. It sets `scene_dirty`, which is exactly what the Android loop reads
//!   as `Frame::needs_paint`. It used to be `!active_animations.is_empty()`, so
//!   a paused entry kept every Android frame presented and the loop paced at
//!   the panel's rate with nothing moving.
//!
//! A paused animation has nothing to advance, so it is neither.
//!
//! # Mutants, and what kills each
//!
//! | mutant | killed by |
//! |---|---|
//! | `main`: a paused entry counts as running in `tick_animations` | `a_paused_animation_lets_the_app_go_idle`, `a_paused_animation_owes_the_android_loop_no_frame`, `a_loader_in_a_closed_drawer_idles_once_the_app_pauses_it` |
//! | `had_running` still reads `!active_animations.is_empty()` | `a_paused_animation_owes_the_android_loop_no_frame`, **alone** in this file — the desktop redraw request never reads it |
//! | the tick re-applies a paused sample and marks the node dirty | `a_paused_animation_lets_the_app_go_idle` and `a_paused_animation_owes_the_android_loop_no_frame` |
//! | `had_running` / the tick answer `false` for everything | `resuming_a_paused_animation_restarts_the_frame_clock` and `a_running_animation_beside_a_paused_one_keeps_the_clock_running` |

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::*;
use crate as rinch;
use crate::shell::android_frame;
use rinch_components::{Drawer, Loader};
use rinch_core::{Component, Signal};
use rinch_macros::rsx;

const VIEWPORT: (f32, f32) = (800.0, 600.0);
const PHYSICAL: (u32, u32) = (800, 600);

/// `width` affects layout and `font-size` affects text measurement — the two
/// ways a tick can dirty the tree without the tick itself answering `true`.
const CSS: &str = "
    @keyframes k763-grow { from { width: 40px; } to { width: 100px; } }
    @keyframes k763-type { from { font-size: 10px; } to { font-size: 30px; } }
    .box  { width: 10px; height: 10px; font-size: 16px; line-height: 20px; }
    .grow { animation: k763-grow 1000s linear infinite; }
    .type { animation: k763-type 1000s linear infinite; }
    .held { animation-play-state: paused; }
";

/// What the boxes' animations are doing.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// No animation declared at all.
    Still,
    /// Both animations declared, both paused.
    Paused,
    /// Both declared, both running.
    Running,
    /// The `width` one paused, the `font-size` one running.
    Mixed,
}

struct Boxes {
    app: RinchApp,
    phase: Signal<Phase>,
}

/// Two boxes — one animating `width`, one `font-size` — whose classes follow a
/// signal.
///
/// The animations are declared by a class change *after* mount rather than
/// from the first frame, so nothing here depends on #762 (an animation present
/// in the very first frame never starts).
fn mount() -> Boxes {
    let phase = Signal::new(Phase::Still);
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style { {CSS} }
                div {
                    class: {move || match phase.get() {
                        Phase::Still => "box grow-box",
                        Phase::Paused | Phase::Mixed => "box grow-box grow held",
                        Phase::Running => "box grow-box grow",
                    }},
                    "w"
                }
                div {
                    class: {move || match phase.get() {
                        Phase::Still => "box type-box",
                        Phase::Paused => "box type-box type held",
                        Phase::Running | Phase::Mixed => "box type-box type",
                    }},
                    "t"
                }
            }
        }
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    Boxes { app, phase }
}

fn go(boxes: &mut Boxes, phase: Phase) {
    boxes.phase.set(phase);
    boxes.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
}

/// How many animations are registered, and how many of them are paused.
fn registered(app: &RinchApp) -> (usize, usize) {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let all: Vec<_> = d.tree.active_animations.values().flatten().collect();
    let paused = all
        .iter()
        .filter(|a| a.play_state == rinch_dom::animation::AnimationPlayState::Paused)
        .count();
    (all.len(), paused)
}

/// Pump the frame clock `n` times the way the desktop event loop does when
/// nothing else is happening, and count the frames that asked to be redrawn.
fn idle_frames_requesting_redraw(app: &mut RinchApp, n: usize) -> usize {
    let mut asked = 0;
    for _ in 0..n {
        let actions = app.handle_event(PlatformEvent::AboutToWait, PHYSICAL, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            asked += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    asked
}

/// The positive control every zero in this file rests on — and the first half
/// of it passes on `main` too.
#[test]
fn a_running_animation_keeps_the_clock_running() {
    let mut boxes = mount();
    go(&mut boxes, Phase::Running);
    assert_eq!(registered(&boxes.app), (2, 0), "precondition: both running");
    assert_eq!(
        idle_frames_requesting_redraw(&mut boxes.app, 4),
        4,
        "a moving animation is repainted every frame"
    );

    boxes.app.scene_dirty = false;
    let frame = android_frame::pump_frame(&mut boxes.app, PHYSICAL, 1.0);
    assert!(
        frame.needs_paint,
        "and the Android loop is told the frame has to be presented"
    );
}

/// The issue: a paused animation lets a desktop app sleep.
#[test]
fn a_paused_animation_lets_the_app_go_idle() {
    let mut boxes = mount();
    go(&mut boxes, Phase::Paused);
    assert_eq!(
        registered(&boxes.app),
        (2, 2),
        "precondition: both animations are registered, and both are paused — \
         the entries stay, it is only the clock that stops"
    );

    assert_eq!(
        idle_frames_requesting_redraw(&mut boxes.app, 6),
        0,
        "nothing on screen is moving, so no frame may ask to be redrawn"
    );
    assert_eq!(
        registered(&boxes.app),
        (2, 2),
        "and six ticks later both are still there, still paused"
    );
}

/// The other reader: K23's `had_running`, which reaches the Android loop as
/// `Frame::needs_paint` through `scene_dirty`. The desktop redraw request never
/// reads it, so the fixture above cannot see it.
#[test]
fn a_paused_animation_owes_the_android_loop_no_frame() {
    let mut boxes = mount();
    go(&mut boxes, Phase::Paused);
    assert_eq!(registered(&boxes.app), (2, 2), "precondition: both paused");
    // Settle whatever the phase change itself left for the clock to batch.
    let _ = android_frame::pump_frame(&mut boxes.app, PHYSICAL, 1.0);

    for round in 0..3 {
        boxes.app.scene_dirty = false;
        let frame = android_frame::pump_frame(&mut boxes.app, PHYSICAL, 1.0);
        assert!(
            !frame.needs_paint,
            "round {round}: a paused animation made the frame clock mark the \
             scene dirty, so the Android loop presents — and paces at the \
             panel's rate — with nothing moving"
        );
        assert!(
            !frame.pending_layout,
            "round {round}: a paused `width` or `font-size` animation left \
             layout pending, which the Android loop also presents for"
        );
        assert!(
            !frame.actions.contains(&AppAction::RequestRedraw),
            "round {round}: nor may it request a redraw"
        );
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
}

/// Pausing one animation does not silence another.
#[test]
fn a_running_animation_beside_a_paused_one_keeps_the_clock_running() {
    let mut boxes = mount();
    go(&mut boxes, Phase::Mixed);
    assert_eq!(
        registered(&boxes.app),
        (2, 1),
        "precondition: one paused, one running"
    );
    assert_eq!(
        idle_frames_requesting_redraw(&mut boxes.app, 4),
        4,
        "the running one is still moving"
    );
}

/// Resuming turns the clock back on — both readers of it.
#[test]
fn resuming_a_paused_animation_restarts_the_frame_clock() {
    let mut boxes = mount();
    go(&mut boxes, Phase::Paused);
    assert_eq!(
        idle_frames_requesting_redraw(&mut boxes.app, 2),
        0,
        "precondition: paused and idle"
    );

    go(&mut boxes, Phase::Running);
    assert_eq!(registered(&boxes.app), (2, 0), "the same two, now running");
    assert_eq!(
        idle_frames_requesting_redraw(&mut boxes.app, 4),
        4,
        "a resumed animation is repainted every frame again"
    );

    boxes.app.scene_dirty = false;
    let frame = android_frame::pump_frame(&mut boxes.app, PHYSICAL, 1.0);
    assert!(frame.needs_paint, "on Android too");
}

// ── What this unblocks: a `Loader` in a closed `Drawer` ──────────────────────

/// The size `settle` re-resolves at. Differs from [`VIEWPORT`] by more than half
/// a pixel, for the reason `hidden_animation_frames_tests::settle` gives.
const SETTLED: (f32, f32) = (804.0, 600.0);
const SETTLED_PHYSICAL: (u32, u32) = (804, 600);

/// An app-level rule, not a component change: the closed `Drawer`'s root is
/// `visibility: hidden` (#751), which is rendered, so a `Loader` inside it keeps
/// animating. Pausing it is what makes the closed drawer cheap, and it only does
/// anything now that a paused animation stops asking for frames.
const PAUSE_WHEN_CLOSED: &str =
    ".rinch-drawer__root--hidden .rinch-loader__oval { animation-play-state: paused; }";

/// A `Loader` inside a `Drawer`, with the app rule above, settled the way
/// `hidden_animation_frames_tests::settle` settles (and for its reasons — the
/// component stylesheet has to be installed, and #762 means the viewport bump
/// is what actually starts the animations).
fn mount_loader_in_drawer() -> (RinchApp, Signal<bool>) {
    let opened = Signal::new(false);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let loader = Loader::default().render(scope, &[]);
        let drawer = Drawer {
            opened_fn: Some(std::rc::Rc::new(move || opened.get())),
            position: "left".to_string(),
            ..Default::default()
        }
        .render(scope, &[loader]);
        root.append_child(&drawer);
        root
    });
    app.mount_component(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&format!(
            "{}\n{PAUSE_WHEN_CLOSED}",
            rinch_components::generate_component_css()
        ));
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    {
        let doc = app.doc.as_ref().unwrap();
        doc.borrow_mut().resolve_layout(SETTLED.0, SETTLED.1);
    }
    (app, opened)
}

fn drawer_idle_frames(app: &mut RinchApp, n: usize) -> usize {
    let mut asked = 0;
    for _ in 0..n {
        let actions = app.handle_event(PlatformEvent::AboutToWait, SETTLED_PHYSICAL, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            asked += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    asked
}

/// `hidden_animation_frames_tests::a_loader_in_a_closed_drawer_keeps_animating_
/// because_the_drawer_is_rendered` asserts the cost of a closed drawer holding
/// a `Loader`. This is the cure that fixture's doc names, applied by the app:
/// pause the spinner while the drawer is closed, and the app idles — and opening
/// the drawer resumes it rather than dropping or restarting it.
#[test]
fn a_loader_in_a_closed_drawer_idles_once_the_app_pauses_it() {
    let (mut app, opened) = mount_loader_in_drawer();
    assert_eq!(
        registered(&app),
        (1, 1),
        "precondition: the closed drawer's `Loader` is registered, and paused \
         by the app's rule"
    );
    assert_eq!(
        drawer_idle_frames(&mut app, 4),
        0,
        "a closed drawer holding a paused `Loader` lets the app sleep"
    );

    opened.set(true);
    app.resolve_and_repaint(SETTLED.0, SETTLED.1);
    assert_eq!(
        registered(&app),
        (1, 0),
        "opening the drawer resumes the same spinner"
    );
    assert_eq!(
        drawer_idle_frames(&mut app, 4),
        4,
        "and the clock runs while it is on screen"
    );
}
