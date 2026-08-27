//! Empirical pinning tests for issue #139: a pointer-capture drag belongs to the
//! document that armed it.
//!
//! `ACTIVE_DRAG` (`rinch-core/src/events/drag.rs`) is one thread-local slot, but
//! a thread can pump *two* documents' pointer streams through it. With no
//! scoping:
//!
//! * a drag armed in document A is driven by pointer moves fed to document B,
//!   with B-relative coordinates — the slider jumps; and
//! * a release fed to B **commits** it (`on_end` is the commit callback),
//!   which is the worse half and is invisible to a move-only test.
//!
//! The reachable shape is **one host pumping several documents from a single
//! event stream**: two embedded `RinchContext`s (what these tests build), or a
//! drag left live past a missed `MouseUp`. It is *not* reachable by simply
//! dragging a mouse from one desktop window into another: while a button is
//! held the pointer is grabbed to the pressing window on every platform rinch
//! targets (X11/Wayland implicit grab, AppKit's mouseDown routing, and winit's
//! `SetCapture` on Win32), `handle_click` — where a `Drag` is armed — runs from
//! the `MouseDown` arm (`app/event_dispatch.rs`), and `finish_drag` is
//! unconditional in the `MouseUp` arm. So a plain mouse drag both begins and
//! ends inside one window's event stream.
//!
//! Each test therefore pins one direction, and the third pins the
//! counterfactual: "scope by document" is trivially satisfiable by never firing
//! at all, so the owning document must still drive and still commit.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test input_routing

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::platform::{MouseButton, PlatformEvent};
use rinch::prelude::*;

// ── harness ──────────────────────────────────────────────────────────────────

type Job = Box<dyn FnOnce() + Send>;

/// Run `f` on the single shared "UI" worker thread and propagate its panic (if
/// any) back to the calling test thread.
///
/// `RinchContext::new` calls `rinch_core::register_main_thread()`, a
/// process-wide `OnceLock`: the first test thread to create a context becomes
/// "the main thread" forever. libtest gives each `#[test]` its own thread, so
/// all bodies are marshalled onto one long-lived worker (which also serializes
/// them). Same reasoning — and same helper — as `tests/multi_context.rs`.
fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });

    let (result_tx, result_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = result_tx.send(result);
    });
    sender
        .lock()
        .expect("ui sender lock")
        .send(job)
        .expect("ui worker alive");
    match result_rx.recv().expect("ui worker responded") {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: WIDTH,
        height: HEIGHT,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

fn press(x: f32, y: f32) -> PlatformEvent {
    PlatformEvent::MouseDown {
        x,
        y,
        button: MouseButton::Left,
    }
}

fn release(x: f32, y: f32) -> PlatformEvent {
    PlatformEvent::MouseUp {
        x,
        y,
        button: MouseButton::Left,
    }
}

// ── the drag under test ──────────────────────────────────────────────────────

/// Everything the armed drag's callbacks saw.
#[derive(Default)]
struct DragLog {
    moves: RefCell<Vec<(f32, f32)>>,
    end: Cell<Option<(f32, f32)>>,
}

/// A context whose full-window `div` arms a `Drag::absolute()` from its
/// `onclick`, recording every `on_move` and the single `on_end` into `log`.
///
/// `Drag::absolute()` from a click handler is the documented pointer-capture
/// pattern (CLAUDE.md, "Pointer Capture Drag") — sliders, panel dragging and
/// resize handles all arm this way.
fn armed_context(log: Rc<DragLog>) -> RinchContext {
    RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style: "width: 800px; height: 600px;",
                onclick: move || {
                    let m = log.clone();
                    let e = log.clone();
                    Drag::absolute()
                        .on_move(move |x, y| m.moves.borrow_mut().push((x, y)))
                        .on_end(move |x, y| e.end.set(Some((x, y))))
                        .start();
                },
                "drag me"
            }
        }
    })
}

/// A second, unrelated document on the same thread — the DevTools panel, or a
/// second embedded context. It arms nothing; it only pumps its own events.
fn bystander_context() -> RinchContext {
    RinchContext::new(cfg(), |__scope: &mut RenderScope| {
        rsx! { div { style: "width: 800px; height: 600px;", "other window" } }
    })
}

/// Arm the drag in `a` and assert it took, so every test below starts from a
/// live drag rather than passing vacuously.
fn arm(a: &mut RinchContext) {
    a.update(&[press(400.0, 300.0)]);
    assert!(
        Drag::is_active(),
        "the click handler must have armed a drag — the rest of this test is \
         vacuous otherwise"
    );
}

// ── 1. another document's motion must not drive the drag ─────────────────────

/// Document B's `MouseMove` must not be delivered to a drag armed by document A.
///
/// B's coordinates are in B's window space, so feeding them through is not a
/// near-miss: the slider (or panel, or resize handle) snaps to a position taken
/// from a completely different surface.
#[test]
fn b_mousemove_does_not_drive_as_drag() {
    let moves = on_ui_thread(|| {
        let log = Rc::new(DragLog::default());
        let mut a = armed_context(log.clone());
        let mut b = bystander_context();
        arm(&mut a);

        b.update(&[PlatformEvent::MouseMove { x: 999.0, y: 999.0 }]);

        let moves = log.moves.borrow().clone();
        Drag::cancel();
        drop(b);
        drop(a);
        moves
    });

    assert!(
        moves.is_empty(),
        "a drag armed in document A must not be driven by document B's pointer \
         motion, but on_move saw {moves:?}"
    );
}

// ── 2. …and another document's release must not commit it ────────────────────

/// Document B's `MouseUp` must neither fire A's `on_end` nor take A's drag.
///
/// This is the half that is worse than a wrong coordinate: `on_end` is the
/// *commit* callback, so a release over the second window writes the committed
/// value — and the gesture the user is still performing in window A is over.
/// Test 1 cannot see this: it never sends a release.
#[test]
fn b_mouseup_does_not_commit_as_drag() {
    let (end, still_active) = on_ui_thread(|| {
        let log = Rc::new(DragLog::default());
        let mut a = armed_context(log.clone());
        let mut b = bystander_context();
        arm(&mut a);

        b.update(&[release(999.0, 999.0)]);

        let observed = (log.end.get(), Drag::is_active());
        Drag::cancel();
        drop(b);
        drop(a);
        observed
    });

    assert_eq!(
        end, None,
        "document B's mouseup must not commit document A's drag"
    );
    assert!(
        still_active,
        "document B's mouseup must not take document A's drag either — the \
         gesture in A is still in progress"
    );
}

// ── 3. the counterfactual: A still drives and still commits ──────────────────

/// The owning document must still drive the drag and still commit it, with its
/// own coordinates.
///
/// Without this, "scope the drag by document" is satisfiable by never
/// dispatching at all — and `Drag::is_active()` gates hover, surface events and
/// text selection, so an over-broad skip would wedge all three in the very
/// window holding the drag.
#[test]
fn a_drag_still_ends_in_its_own_context() {
    let (moves, end, active_after) = on_ui_thread(|| {
        let log = Rc::new(DragLog::default());
        let mut a = armed_context(log.clone());
        let b = bystander_context();
        arm(&mut a);

        a.update(&[PlatformEvent::MouseMove { x: 410.0, y: 320.0 }]);
        a.update(&[release(420.0, 330.0)]);

        let observed = (log.moves.borrow().clone(), log.end.get(), Drag::is_active());
        Drag::cancel();
        drop(b);
        drop(a);
        observed
    });

    assert_eq!(
        moves,
        vec![(410.0, 320.0)],
        "the arming document's own motion must still reach on_move"
    );
    assert_eq!(
        end,
        Some((420.0, 330.0)),
        "and its own mouseup must still commit, at its own coordinates"
    );
    assert!(!active_after, "the drag is over once its owner released");
}
