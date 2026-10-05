//! A box whose place in the paint order changes repaints everything its
//! subtree paints, not only its own rect (#1384).
//!
//! When a box starts or stops being a stacking context, a `z-index`
//! descendant of it is trapped in, or released from, its context, and so
//! changes order against boxes **outside** the restyled box. The same happens
//! to its in-flow content when the box itself becomes a positioned layer, and
//! to everything under it when its own `z-index` changes. The damage used to
//! be the box's own rect, so a descendant that overflows it and overlaps a
//! sibling was reordered where nothing was repainted.
//!
//! Every fixture is a local pixel oracle: the incremental frame against a
//! from-scratch frame of the same tree, with a positive control that the flip
//! does change pixels (`changed > 0`) and that the frame was a partial
//! repaint — a full one would pass vacuously.

use super::*;
use rinch_dom::perf::{Counter, FrameStats};
use rinch_dom::transition::TransitionProperty;

const SIZE: (u32, u32) = (600, 400);
/// Where the overflowing descendant and the sibling overlap.
const OVERLAP: (i32, i32, i32, i32) = (150, 0, 250, 100);
const ALL: (i32, i32, i32, i32) = (0, 0, 600, 400);

fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn incremental_frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(1.0, SIZE, false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}

fn diff_in(a: &[u8], b: &[u8], rect: (i32, i32, i32, i32)) -> usize {
    let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
    let mut n = 0;
    for y in rect.1.max(0)..rect.3.min(h) {
        for x in rect.0.max(0)..rect.2.min(w) {
            let i = ((y * w + x) * 4) as usize;
            if (0..4).any(|k| (a[i + k] as i32 - b[i + k] as i32).abs() > 2) {
                n += 1;
            }
        }
    }
    n
}

fn resolve(app: &mut RinchApp) {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
}

fn el(scope: &mut RenderScope, parent: &NodeHandle, style: &str) -> NodeHandle {
    let e = scope.create_element("div");
    e.set_attribute("style", style);
    parent.append_child(&e);
    e
}

/// Which descendant of the flipping box `p` reaches the sibling `s`.
#[derive(Clone, Copy)]
enum Scene {
    /// `p > k(relative, z-index: 5)` beside `s(absolute, z-index: 1)`: `k` is
    /// hoisted past `p` unless `p` is a stacking context.
    ZChild,
    /// `s(absolute, z auto)` then `p > q(static)`: `q` is in-flow content
    /// under `s` unless `p` is itself a positioned layer after `s`.
    FlowChild,
    /// `s(absolute, z-index: 2)` then `p > q(static)`: `q` follows `p`'s own
    /// `z-index`.
    FlowChildZ,
    /// [`Scene::ZChild`] with `k` inside a `z` 0 stacking context of its own
    /// (`opacity`), which seals it: `k` sorts inside that context whether or
    /// not `p` is one.
    SealedZChild,
    /// [`Scene::ZChild`] with `p` in a 100px `overflow: hidden` box and `k`
    /// absolute against `c`, so `k` escapes the clip `p` is under.
    EscapingZChild,
}

const P: &str = "width: 100px; height: 100px;";

/// Mount `scene` with `p` styled `P` + `from`; returns the app, `p`, and the
/// document's `<style>` text (`css`) is installed first.
fn mount(scene: Scene, css: &'static str, from: &'static str) -> (RinchApp, NodeHandle) {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute("style", "width: 600px; height: 400px");
        if !css.is_empty() {
            let style = scope.create_element("style");
            style.append_child(&scope.create_text(css));
            outer.append_child(&style);
        }
        let c = el(
            scope,
            &outer,
            "position: relative; width: 300px; height: 100px",
        );
        let blue = "left: 150px; top: 0; width: 100px; height: 100px; background: rgb(0, 0, 200)";
        let red = "width: 100px; height: 100px; background: rgb(200, 0, 0)";
        let p = match scene {
            Scene::ZChild => {
                let p = el(scope, &c, &format!("{P} {from}"));
                el(
                    scope,
                    &p,
                    &format!("position: relative; left: 150px; z-index: 5; {red}"),
                );
                el(
                    scope,
                    &c,
                    &format!("position: absolute; z-index: 1; {blue}"),
                );
                p
            }
            Scene::SealedZChild => {
                let p = el(scope, &c, &format!("{P} {from}"));
                let seal = el(scope, &p, "position: relative; opacity: 0.99");
                el(
                    scope,
                    &seal,
                    &format!("position: relative; left: 150px; z-index: 5; {red}"),
                );
                el(
                    scope,
                    &c,
                    &format!("position: absolute; z-index: 1; {blue}"),
                );
                p
            }
            Scene::EscapingZChild => {
                let clipper = el(scope, &c, "width: 100px; height: 100px; overflow: hidden");
                let p = el(scope, &clipper, &format!("{P} {from}"));
                el(
                    scope,
                    &p,
                    &format!("position: absolute; left: 150px; top: 0; z-index: 5; {red}"),
                );
                el(
                    scope,
                    &c,
                    &format!("position: absolute; z-index: 1; {blue}"),
                );
                p
            }
            Scene::FlowChild | Scene::FlowChildZ => {
                let z = if matches!(scene, Scene::FlowChildZ) {
                    "z-index: 2;"
                } else {
                    ""
                };
                el(scope, &c, &format!("position: absolute; {z} {blue}"));
                let p = el(scope, &c, &format!("{P} {from}"));
                el(scope, &p, &format!("margin-left: 150px; {red}"));
                p
            }
        };
        p.set_attribute("class", "p");
        *slot_in.borrow_mut() = Some(p);
        outer
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    resolve(&mut app);
    let p = slot.borrow().clone().unwrap();
    (app, p)
}

/// After `step`, the incremental frame is the full frame, the frame was a
/// partial repaint, and the overlap changed (so there was something to miss).
fn assert_flip_repaints(app: &mut RinchApp, what: &str, step: impl FnOnce(&mut RinchApp)) {
    let before = full_frame(app);
    step(app);
    let (inc, stats) = incremental_frame(app);
    let full = full_frame(app);
    assert!(
        diff_in(&before, &full, OVERLAP) > 9000,
        "[{what}] positive control: the flip reorders the overlap ({} px)",
        diff_in(&before, &full, OVERLAP)
    );
    assert_eq!(stats.get(Counter::RepaintFull), 0, "[{what}] {stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "[{what}] {stats:?}");
    assert_eq!(
        diff_in(&inc, &full, ALL),
        0,
        "[{what}] stale pixels after the incremental frame"
    );
}

fn restyle(scene: Scene, from: &'static str, to: &'static str) {
    let (mut app, p) = mount(scene, "", from);
    assert_flip_repaints(&mut app, &format!("{from:?} -> {to:?}"), |app| {
        p.set_attribute("style", &format!("{P} {to}"));
        resolve(app);
    });
}

/// Every creator that can be toggled without moving the box, both ways.
const CREATORS: [&str; 5] = [
    "opacity: 0.99",
    "position: relative; z-index: 0",
    "transform: translateX(1px)",
    "filter: brightness(0.99)",
    "position: sticky",
];

#[test]
fn a_restyle_that_starts_a_stacking_context_repaints_the_trapped_descendant() {
    for on in CREATORS {
        restyle(Scene::ZChild, "", on);
    }
}

#[test]
fn a_restyle_that_ends_a_stacking_context_repaints_the_released_descendant() {
    for on in CREATORS {
        restyle(Scene::ZChild, on, "");
    }
}

/// `position: fixed` with no insets stays at its static position in this
/// scene's first row, so only the order changes.
#[test]
fn a_restyle_to_and_from_fixed_repaints_the_descendant() {
    restyle(Scene::ZChild, "", "position: fixed");
    restyle(Scene::ZChild, "position: fixed", "");
}

/// Not a stacking context either way: the box becomes a positioned layer
/// (`z-index: auto`), which moves its in-flow content from the block step to
/// the positioned step.
#[test]
fn a_restyle_to_and_from_a_positioned_layer_repaints_its_in_flow_content() {
    restyle(Scene::FlowChild, "", "position: relative");
    restyle(Scene::FlowChild, "position: relative", "");
}

/// A stacking context either way: its `z-index` crosses a sibling's.
#[test]
fn a_z_index_change_repaints_everything_the_context_holds() {
    restyle(
        Scene::FlowChildZ,
        "position: relative; z-index: 1",
        "position: relative; z-index: 3",
    );
    restyle(
        Scene::FlowChildZ,
        "position: relative; z-index: 3",
        "position: relative; z-index: 1",
    );
}

/// A positioned `z-index: auto` box that becomes a stacking context at `z` 0
/// keeps its own place in the order; what it traps is the descendant that
/// sorts away from 0.
#[test]
fn a_positioned_box_that_becomes_a_context_repaints_its_z_ordered_descendant() {
    for on in ["z-index: 0", "opacity: 0.99"] {
        let (on, off): (&'static str, &'static str) = (
            Box::leak(format!("position: relative; {on}").into_boxed_str()),
            "position: relative",
        );
        restyle(Scene::ZChild, off, on);
        restyle(Scene::ZChild, on, off);
    }
}

/// A transform that starts or stops applying on such a box also moves what
/// is under it, hoisted or not.
#[test]
fn a_transform_starting_on_a_positioned_box_repaints_the_content_it_moves() {
    let (off, on) = (
        "position: relative",
        "position: relative; transform: translateY(20px)",
    );
    for (from, to) in [(off, on), (on, off)] {
        let (mut app, p) = mount(Scene::FlowChild, "", from);
        let before = full_frame(&mut app);
        p.set_attribute("style", &format!("{P} {to}"));
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        let full = full_frame(&mut app);
        assert!(diff_in(&before, &full, (150, 0, 250, 120)) > 3000);
        assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
        assert_eq!(diff_in(&inc, &full, ALL), 0, "[{from:?} -> {to:?}] stale");
    }
}

/// The descendant escapes a clip the flipping box is under (it is absolute
/// against a box above the clipper), so the box's own clip chain must not
/// cut the damage.
#[test]
fn a_descendant_that_escapes_the_boxs_clip_is_repainted() {
    restyle(Scene::EscapingZChild, "", "opacity: 0.99");
    restyle(Scene::EscapingZChild, "opacity: 0.99", "");
}

// ── Ticks ─────────────────────────────────────────────────────────────────

fn transition_start(app: &RinchApp, p: &NodeHandle, prop: TransitionProperty) -> f64 {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.active_transitions[&p.node_id().0][&prop].start_time_ms
}

fn tick_to(app: &mut RinchApp, ms: f64) {
    {
        let doc = app.doc.as_ref().unwrap().clone();
        let mut d = doc.borrow_mut();
        rinch_dom::transition::tick_transitions(&mut d.tree, ms);
    }
    // What `AboutToWait` does after a tick that moved something.
    app.scene_dirty = true;
    resolve(app);
}

fn creates_context(app: &RinchApp, p: &NodeHandle) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes[p.node_id().0].creates_stacking_context()
}

/// A transition that makes `p` a stacking context: the restyle that starts
/// it and its first ticks, in one frame. Whichever of the two flips the
/// answer (an opacity run starts at exactly `1`; a transform run's first
/// value is already a transform), the frame has to repaint the overlap.
fn transition_in(css: &'static str, prop: TransitionProperty) {
    let (mut app, p) = mount(Scene::ZChild, css, "");
    assert!(!creates_context(&app, &p), "[{css}] before");
    assert_flip_repaints(&mut app, css, |app| {
        p.set_attribute("class", "p on");
        resolve(app);
        let start = transition_start(app, &p, prop);
        tick_to(app, start + 400.0);
    });
    assert!(creates_context(&app, &p), "[{css}] mid-run");
}

/// The first frame of such a run painted on its own (the value still at its
/// start), then a tick: the flip is the tick's alone.
fn transition_in_by_tick(css: &'static str, prop: TransitionProperty) {
    let (mut app, p) = mount(Scene::ZChild, css, "");
    p.set_attribute("class", "p on");
    resolve(&mut app);
    let start = transition_start(&app, &p, prop);
    tick_to(&mut app, start);
    let _ = incremental_frame(&mut app);
    assert!(!creates_context(&app, &p), "[{css}] at its first frame");
    assert_flip_repaints(&mut app, css, |app| tick_to(app, start + 400.0));
    assert!(creates_context(&app, &p), "[{css}] mid-run");
}

/// A transition that ends with `p` no longer a stacking context: painted
/// mid-run, then ticked past its end. The flip is the tick's alone.
fn transition_out(css: &'static str, prop: TransitionProperty) {
    let (mut app, p) = mount(Scene::ZChild, css, "");
    p.set_attribute("class", "p on");
    resolve(&mut app);
    let start = transition_start(&app, &p, prop);
    tick_to(&mut app, start + 400.0);
    let _ = incremental_frame(&mut app);
    assert!(creates_context(&app, &p), "[{css}] mid-run");
    assert_flip_repaints(&mut app, css, |app| tick_to(app, start + 5000.0));
    assert!(!creates_context(&app, &p), "[{css}] finished");
}

#[test]
fn an_opacity_transition_leaving_one_repaints_the_trapped_descendant() {
    let css = ".p { transition: opacity 1000ms linear; } .p.on { opacity: 0.5; }";
    transition_in(css, TransitionProperty::Opacity);
    transition_in_by_tick(css, TransitionProperty::Opacity);
}

#[test]
fn an_opacity_transition_arriving_at_one_repaints_the_released_descendant() {
    transition_out(
        ".p { transition: opacity 1000ms linear; opacity: 0.5; } .p.on { opacity: 1; }",
        TransitionProperty::Opacity,
    );
}

#[test]
fn a_transform_transition_leaving_none_repaints_the_trapped_descendant() {
    transition_in(
        ".p { transition: transform 1000ms linear; } .p.on { transform: translateX(2px); }",
        TransitionProperty::Transform,
    );
}

/// An animation that fills forwards at `opacity: 1`: a context while it runs,
/// none once its finishing tick writes the fill. (One with no fill keeps its
/// last sample when it ends — #783's neighbourhood — so nothing flips there.)
#[test]
fn an_animation_that_starts_and_one_that_fills_at_one_repaint_the_descendant() {
    let (mut app, p) = mount(
        Scene::ZChild,
        "@keyframes k { from { opacity: 0.5; } to { opacity: 1; } } \
         .p.on { animation: k 1000ms linear forwards; }",
        "",
    );
    // Starting it is a restyle that makes `p` a context.
    assert_flip_repaints(&mut app, "animation start", |app| {
        p.set_attribute("class", "p on");
        resolve(app);
    });
    assert!(creates_context(&app, &p), "running: a context");
    assert_flip_repaints(&mut app, "animation end", |app| {
        {
            let doc = app.doc.as_ref().unwrap().clone();
            let mut d = doc.borrow_mut();
            let start = d.tree.active_animations[&p.node_id().0][0].start_time_ms;
            rinch_dom::animation::tick_animations(&mut d.tree, start + 5000.0);
        }
        app.scene_dirty = true;
        resolve(app);
    });
    assert!(!creates_context(&app, &p), "finished: not a context");
}

// ── What must NOT grow ────────────────────────────────────────────────────

/// A restyle that changes how the box paints and not where in the order — a
/// colour, an opacity that stays below one, a transform that stays a
/// transform — damages the box, not the overflowing descendant 150px away.
/// Nor does a positioned box that becomes a stacking context with nothing
/// under it to trap: no `z-index` descendant, or one sealed in a context of
/// its own.
#[test]
fn a_restyle_that_reorders_nothing_damages_only_the_box() {
    // The box at the window's corner and the 4px anti-aliasing margin.
    let own = 104 * 104;
    let rel = "position: relative";
    let rel_ctx = "position: relative; opacity: 0.99";
    for (scene, from, to) in [
        (Scene::ZChild, "", "background: rgb(0, 200, 0)"),
        (Scene::ZChild, "opacity: 0.5", "opacity: 0.4"),
        (
            Scene::ZChild,
            "position: relative; z-index: 0",
            "position: relative; z-index: 0; background: rgb(0, 200, 0)",
        ),
        // A `z-index` on a box it does not apply to orders nothing.
        (Scene::FlowChild, "z-index: 1", "z-index: 3"),
        (Scene::FlowChild, rel, rel_ctx),
        (Scene::FlowChild, rel_ctx, rel),
        (Scene::SealedZChild, rel, rel_ctx),
        (Scene::SealedZChild, rel_ctx, rel),
    ] {
        let (mut app, p) = mount(scene, "", from);
        let _ = full_frame(&mut app);
        p.set_attribute("style", &format!("{P} {to}"));
        resolve(&mut app);
        let (inc, stats) = incremental_frame(&mut app);
        let full = full_frame(&mut app);
        assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
        assert_eq!(stats.get(Counter::RepaintedPx), own, "[{from:?} -> {to:?}]");
        assert_eq!(diff_in(&inc, &full, ALL), 0, "[{from:?} -> {to:?}] stale");
    }
}
