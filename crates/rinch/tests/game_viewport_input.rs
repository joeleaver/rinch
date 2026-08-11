//! Empirical pinning tests for issue #207: a `GameViewport` hole must be
//! hittable, because hittability is what routes the mouse.
//!
//! `RinchContext::wants_mouse` hit-tests the DOM and walks up from the hit node
//! looking for `data-viewport`. A hole with `pointer-events: none` is skipped by
//! `hit_test`, the hit falls through to `body`, no `data-viewport` ancestor is
//! found, and `wants_mouse` answers `true` **everywhere** — the UI silently
//! claims the mouse over the whole window and the game gets nothing.
//!
//! Every test therefore asserts a *pair*: a point inside the hole is game
//! territory, and a point on the surrounding chrome is UI territory. The pair
//! matters because `wants_mouse` also returns `false` when nothing is hit at
//! all, so "false inside the hole" alone would be satisfiable by an empty
//! document.
//!
//! Requires the `embed` (or `gpu`) feature:
//!     cargo test -p rinch --features embed --test game_viewport_input

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use rinch::embed::{GameViewport, RinchContext, RinchContextConfig};
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
/// Height of the chrome strip above the viewport in every layout below.
const TOOLBAR_H: f32 = 40.0;

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: WIDTH,
        height: HEIGHT,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

/// Centre of the named viewport's rect, after asserting the rect is a real
/// region (a zero-area hole would make the routing assertions vacuous).
fn viewport_center(ctx: &RinchContext, name: &str) -> (f32, f32) {
    let rect = ctx
        .viewport_rect(name)
        .unwrap_or_else(|| panic!("no viewport named {name:?} in the document"));
    assert!(
        rect.width > 1.0 && rect.height > 1.0,
        "viewport {name:?} must have a real area to route pointers into, got {rect:?}"
    );
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

// ── 1. the bare form ─────────────────────────────────────────────────────────

/// `GameViewport { name: "main" }` with **no** props but the name — the form
/// that #207 reported as silently starving the game of mouse input. The grid
/// wrapper stretches the hole without putting a `style:` on it, so nothing here
/// can accidentally clobber the component's own defaults.
#[test]
fn bare_game_viewport_gives_the_mouse_to_the_game() {
    on_ui_thread(|| {
        let mut ctx = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            rsx! {
                div { style: "display: flex; flex-direction: column; width: 800px; height: 600px;",
                    div { style: "height: 40px;", "toolbar" }
                    div { style: "display: grid; flex: 1;",
                        GameViewport { name: "main" }
                    }
                }
            }
        });
        ctx.update(&[]);

        let (cx, cy) = viewport_center(&ctx, "main");
        assert!(
            !ctx.wants_mouse(cx, cy),
            "a pointer inside the bare viewport hole belongs to the game, but the UI \
             claimed it — the hole is not hittable (#207)"
        );
        assert!(
            ctx.wants_mouse(400.0, TOOLBAR_H / 2.0),
            "a pointer on the toolbar above the hole belongs to the UI"
        );
    });
}

// ── 2. the documented styled form ────────────────────────────────────────────

/// The `style: "flex: 1;"` form from the guide. A component's universal
/// `style:` prop *replaces* the style attribute, so this form must route
/// identically to the bare one — before #207 it worked only because that
/// replacement happened to wipe the baked-in `pointer-events: none`.
#[test]
fn styled_game_viewport_gives_the_mouse_to_the_game() {
    on_ui_thread(|| {
        let mut ctx = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            rsx! {
                div { style: "display: flex; flex-direction: column; width: 800px; height: 600px;",
                    div { style: "height: 40px;", "toolbar" }
                    GameViewport { name: "main", style: "flex: 1;" }
                }
            }
        });
        ctx.update(&[]);

        let (cx, cy) = viewport_center(&ctx, "main");
        assert!(
            !ctx.wants_mouse(cx, cy),
            "a pointer inside a `style:`-sized viewport hole belongs to the game"
        );
        assert!(
            ctx.wants_mouse(400.0, TOOLBAR_H / 2.0),
            "a pointer on the toolbar above the hole belongs to the UI"
        );
    });
}

// ── 3. the author override still wins ────────────────────────────────────────

/// #207 changed the *default*, not the mechanism: the hole's hittability is a
/// UA-stylesheet declaration, so an app that really wants the UI to claim a
/// region can still say so on the viewport itself.
#[test]
fn author_pointer_events_none_hands_the_hole_back_to_the_ui() {
    on_ui_thread(|| {
        let mut ctx = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            rsx! {
                div { style: "display: flex; flex-direction: column; width: 800px; height: 600px;",
                    div { style: "height: 40px;", "toolbar" }
                    GameViewport { name: "main", style: "flex: 1; pointer-events: none;" }
                }
            }
        });
        ctx.update(&[]);

        let (cx, cy) = viewport_center(&ctx, "main");
        assert!(
            ctx.wants_mouse(cx, cy),
            "an author `pointer-events: none` on the viewport must still beat the UA \
             default and hand the region back to the UI"
        );
    });
}

// ── 4. the inherited-`none` HUD root (#195) ──────────────────────────────────

/// The sibling trap from #195: a click-through HUD root sets
/// `pointer-events: none` and every descendant inherits it. A UA declaration on
/// `[data-viewport]` beats inheritance, so the hole stays hittable and the game
/// keeps its input even under such a root.
#[test]
fn hole_survives_pointer_events_none_inherited_from_a_hud_root() {
    on_ui_thread(|| {
        let mut ctx = RinchContext::new(cfg(), |__scope: &mut RenderScope| {
            rsx! {
                div { style: "display: flex; flex-direction: column; width: 800px; \
                              height: 600px; pointer-events: none;",
                    div { style: "height: 40px;", "click-through chrome" }
                    GameViewport { name: "main", style: "flex: 1;" }
                }
            }
        });
        ctx.update(&[]);

        let (cx, cy) = viewport_center(&ctx, "main");
        assert!(
            !ctx.wants_mouse(cx, cy),
            "the viewport hole must not inherit `pointer-events: none` from a \
             click-through HUD root (#195)"
        );
    });
}
