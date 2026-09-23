//! The shell half of the exact-count performance contract: drive a `RinchApp`
//! the way the desktop runtime does, and assert whole frames.
//!
//! Shared by `perf_regression_tests.rs` and `perf_regression_editor_tests.rs`.
//! `perf_stats_tests.rs` keeps its own helpers (it predates this module, and
//! other PRs edit it); the rinch-dom twin is
//! `crates/rinch-dom/tests/support/perf_expect.rs`.
//!
//! # The contract (#877)
//!
//! [`expect_frame`] compares **every** non-timing counter exactly; a counter the
//! baseline does not list must be `0`. A rise is a path that got more
//! expensive. A fall is either a fix — update the baseline and say in the PR
//! which counters moved and why — or an increment that stopped counting.
//!
//! Two counters are left out, and each for a reason that is not "it is
//! inconvenient":
//!
//! - the `time_*` counters, which are wall-clock;
//! - `rerender_events_queued`, which is folded in from a **process-wide**
//!   atomic (`RERENDER_EVENTS_QUEUED`), so a runtime test on another thread of
//!   the same test binary can move it inside this test's frame. Nothing here
//!   queues a `ReRender` (only the winit dispatcher does), so it is asserted to
//!   be `0` by nothing, rather than asserted flakily.
//!
//! # Updating a baseline
//!
//! A failing assertion prints the frame it saw as a paste-ready baseline.
//! `PERF_BASELINE_PRINT=1` prints every scenario's frame, passing or not:
//!
//! ```text
//! PERF_BASELINE_PRINT=1 cargo test -p rinch --lib perf_regression -- --nocapture --test-threads=1
//! ```

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

/// The window every scenario runs in, logical px at scale 1.
pub(crate) const SIZE: (u32, u32) = (800, 600);

/// Whether a counter is outside the exact contract (see the module docs).
fn excluded(c: Counter) -> bool {
    c.name().starts_with("time_") || c == Counter::RerenderEventsQueued
}

/// The frame as a paste-ready baseline.
pub(crate) fn baseline_text(stats: &FrameStats) -> String {
    let mut s = String::new();
    for (c, v) in stats.iter() {
        if v != 0 && !excluded(c) {
            s.push_str(&format!("            ({c:?}, {v}),\n"));
        }
    }
    s
}

/// Assert the whole frame: every counter in `baseline` has exactly that value,
/// every other counter the contract covers is `0`.
#[track_caller]
pub(crate) fn expect_frame(name: &str, stats: &FrameStats, baseline: &[(Counter, u64)]) {
    if std::env::var_os("PERF_BASELINE_PRINT").is_some() {
        println!("{name}:\n{}", baseline_text(stats));
    }
    for (i, (c, _)) in baseline.iter().enumerate() {
        assert!(
            !baseline[..i].iter().any(|(d, _)| d == c),
            "{name}: {} listed twice in its baseline",
            c.name()
        );
        assert!(
            !excluded(*c),
            "{name}: {} is outside the exact contract and cannot be baselined",
            c.name()
        );
    }
    let mut diffs = Vec::new();
    for (c, v) in stats.iter() {
        if excluded(c) {
            continue;
        }
        let want = baseline
            .iter()
            .find(|(b, _)| *b == c)
            .map_or(0, |(_, w)| *w);
        if v != want {
            diffs.push(format!("{}: {v} (baseline {want})", c.name()));
        }
    }
    assert!(
        diffs.is_empty(),
        "{name}: the frame differs from its recorded baseline.\n  {}\n\
         A rise is a path that got more expensive; a fall to 0 may be an \
         increment that stopped counting. If a change moved these on purpose, \
         paste the frame below over the baseline and say why in the PR.\n\
         Actual frame:\n{}",
        diffs.join("\n  "),
        baseline_text(stats)
    );
}

/// One `AboutToWait` turn of the event loop. Returns whether it asked for a
/// redraw.
pub(crate) fn about_to_wait(app: &mut RinchApp) -> bool {
    app.handle_event(PlatformEvent::AboutToWait, SIZE, 1.0)
        .contains(&AppAction::RequestRedraw)
}

/// One redraw, as `RinchRuntime::paint_software` performs it: resolve what is
/// pending, paint, end the performance frame.
pub(crate) fn paint(app: &mut RinchApp) -> FrameStats {
    paint_at(app, 1.0)
}

/// [`paint`] at a display scale.
pub(crate) fn paint_at(app: &mut RinchApp, scale: f64) -> FrameStats {
    if app.has_pending_layout() {
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    }
    let _ = app.build_pixels(scale, SIZE, false);
    app.end_perf_frame().expect("a mounted app has a document")
}

/// The frame an interaction produces: `events` are delivered, the loop turns
/// once (`AboutToWait`), and the redraw they asked for is painted.
pub(crate) fn interaction(app: &mut RinchApp, events: impl FnOnce(&mut RinchApp)) -> FrameStats {
    events(app);
    about_to_wait(app);
    paint(app)
}

/// The bundled face every scenario's text is set in.
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// A `RinchApp` whose `sans-serif` is the bundled Inter, ahead of anything the
/// host has in that slot (`AppFont::sans_serif` prepends its claim).
///
/// **Every scenario's text must be set in `sans-serif`**, and every scenario
/// must be built through this, not `RinchApp::new`. A declared `line-height`
/// pins only the vertical axis. Horizontal extents — a caret's step over one
/// glyph, a text box's width, and so the damage rect around either — come from
/// the font's advances. A host font there once made `repainted_px` 608 on one
/// machine and 640 on CI (Noto Sans against DejaVu Sans, one glyph of the
/// editor's ArrowRight).
pub(crate) fn new_app(
    component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
) -> RinchApp {
    let mut app = RinchApp::new(component);
    app.register_app_font(AppFont::sans_serif(INTER));
    app
}

/// Mount a component, paint until nothing is left over from mounting (the
/// first frame, the first layout's one-off damage), and zero the counters.
pub(crate) fn mount_settled(
    component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
) -> RinchApp {
    let mut app = new_app(component);
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    settle(&mut app);
    app
}

/// Paint, turn the loop and paint again, three times over, then zero the
/// counters.
pub(crate) fn settle(app: &mut RinchApp) {
    for _ in 0..3 {
        paint(app);
        about_to_wait(app);
    }
    paint(app);
    app.reset_perf();
}

/// `n` idle turns of the desktop loop with nothing happening: each turn is an
/// `AboutToWait`, and a redraw only when that turn asked for one. Returns how
/// many turns asked, and every counter over all of them.
pub(crate) fn idle_turns(app: &mut RinchApp, n: usize) -> (usize, FrameStats) {
    let mut total = FrameStats::default();
    let mut redraws = 0;
    for turn in 0..n {
        let asked = about_to_wait(app);
        // The desktop loop reads the redraw request; the Android loop presents
        // on `scene_dirty` (#763). An idle turn must leave neither set.
        assert!(
            asked || !app.scene_dirty,
            "idle turn {turn} marked the scene dirty without asking for a redraw \
             — the Android loop would present a frame for it"
        );
        if asked {
            redraws += 1;
            total.accumulate(&paint(app));
        } else {
            total.accumulate(&app.end_perf_frame().expect("mounted"));
        }
    }
    (redraws, total)
}
