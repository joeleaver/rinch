//! Performance regression scenarios at the shell: idle, keyed `for`, scrolling,
//! dragging, theme and scale changes, and one scenario per full-repaint reason.
//!
//! Each drives a real `RinchApp` on the software painter the way the desktop
//! runtime does (`perf_expect`: `AboutToWait`, then the paint the loop asked
//! for) and asserts the **whole** frame — every non-timing counter exact,
//! anything unlisted `0` (#877's contract). A scenario that does unexpected
//! work is pinned as it is and says so in its doc: the pin is a finding, not an
//! endorsement, and the fix that removes the work updates the number.
//!
//! Each scenario also carries a **positive control**: the counter that proves
//! it did the thing it names (a repaint for a repaint, a moved row for a move).
//! Without one, an idle scenario that accidentally painted nothing because it
//! never mounted would pass as a perfect idle.
//!
//! The editor's scenarios are in `perf_regression_editor_tests.rs`; the
//! document-level ones are `crates/rinch-dom/tests/perf_regression_scenarios.rs`.
//! A failure prints the frame it saw, ready to paste; `PERF_BASELINE_PRINT=1`
//! prints every frame:
//!
//! ```text
//! PERF_BASELINE_PRINT=1 cargo test -p rinch --lib perf_regression -- --nocapture --test-threads=1
//! ```
//!
//! No number here moves with the host's font set, and that takes two things.
//! Line boxes are declared (`line-height`, fixed heights), which pins the
//! vertical axis. Every app is built with `perf_expect::new_app`, whose
//! `sans-serif` is the bundled Inter, and every run of text is set in
//! `sans-serif`, which pins the horizontal one. A declared line box alone does
//! **not** make a width, a caret step or a repainted area font-independent:
//! the editor's ArrowRight repainted 608 px on one host and 640 on CI before
//! the font was bundled.

use super::perf_expect::*;
use super::*;
use crate as rinch;
use rinch_components::{Drawer, Loader};
use rinch_core::{Component, Signal};
use rinch_dom::perf::{Counter::*, FrameStats};
use rinch_macros::rsx;

const ROW_CSS: &str = "
    body, input, button { font-family: sans-serif; }
    body { font-size: 14px; line-height: 20px; }
    .row { height: 20px; }
    .row:hover { background-color: rgb(200, 0, 0); }
    .field { position: absolute; left: 300px; top: 300px; width: 200px; height: 24px; }
    .scroller { width: 300px; height: 400px; overflow-y: auto; }
    .big { width: 800px; height: 600px; }
    .big.on { background-color: rgb(0, 0, 200); }
    .side { position: absolute; left: 600px; top: 400px; width: 180px; height: 180px; overflow-y: auto; }
    .side-row { height: 20px; }
";

/// Ten static rows, a text `<input>`, and a 200-row scroller off to the side.
fn static_page() -> RinchApp {
    mount_settled(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(ROW_CSS);
        style.append_child(&css);
        root.append_child(&style);
        for i in 0..10 {
            let row = scope.create_element("div");
            row.set_attribute("class", "row");
            let t = scope.create_text(&format!("row {i}"));
            row.append_child(&t);
            root.append_child(&row);
        }
        let input = scope.create_element("input");
        input.set_attribute("class", "field");
        input.set_attribute("value", "typed");
        // A text field is one the text engine owns: it carries an `oninput`.
        let on_input = rinch_core::events::register_input_handler(
            rinch_core::events::InputCallback::new(|_| {}),
        );
        input.set_attribute("data-oninput", &on_input.0.to_string());
        root.append_child(&input);
        // A clipping scroller well away from anything the scenarios on this
        // page damage. A partial repaint must prune it on its box, not walk
        // into its 200 rows: that is what `paint_nodes_visited` pins in the
        // hover and "type one character" frames.
        let side = scope.create_element("div");
        side.set_attribute("class", "side");
        for i in 0..200 {
            let row = scope.create_element("div");
            row.set_attribute("class", "side-row");
            let t = scope.create_text(&format!("side {i}"));
            row.append_child(&t);
            side.append_child(&row);
        }
        root.append_child(&side);
        root
    })
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, SIZE, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, SIZE, 1.0);
}

fn key(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        SIZE,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        SIZE,
        1.0,
    );
}

// ── Idle ───────────────────────────────────────────────────────────────────

const IDLE_TURNS: usize = 20;

/// An app with nothing happening asks for no frame and does no work, however
/// many times the loop turns. Positive control: a hover on the same app does
/// repaint, so the zero is the loop's, not a dead app's.
#[test]
fn an_idle_app_redraws_nothing() {
    let mut app = static_page();
    let (redraws, total) = idle_turns(&mut app, IDLE_TURNS);
    assert_eq!(redraws, 0, "no idle turn may ask for a frame");
    expect_frame("idle", &total, &[]);

    let hover = interaction(&mut app, |app| {
        app.handle_event(PlatformEvent::MouseMove { x: 20.0, y: 30.0 }, SIZE, 1.0);
    });
    assert_eq!(hover.get(RepaintPartial), 1, "positive control: {hover:?}");
    expect_frame(
        "hover a row",
        &hover,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 22400),
            (SurfacePx, 480000),
            (PaintNodesVisited, 5),
            (StackingOrderBuilds, 2),
            (GlyphCacheHits, 15),
            (ClipMasks, 1),
            (ClipMaskPx, 25600),
            (HitTests, 1),
            (HitTestNodesVisited, 3),
            (HitExtentsComputed, 14),
        ],
    );
}

/// A focused `<input>` costs nothing while idle either. Its caret does **not**
/// blink — only the rich-text editor's does (`caret_blink_tick` is the
/// runtime's one timed wake) — so there is no per-blink cost to pin. The
/// positive control is the focus itself: typing a character into the field
/// repaints it.
#[test]
fn a_focused_input_idles_for_free() {
    let mut app = static_page();
    let focus = interaction(&mut app, |app| click(app, 310.0, 310.0));
    assert!(
        matches!(app.focus_target, FocusTarget::Input(_)),
        "precondition: the input holds the keyboard"
    );
    assert!(focus.get(PaintFrames) == 1, "{focus:?}");
    settle(&mut app);

    let (redraws, total) = idle_turns(&mut app, IDLE_TURNS);
    assert_eq!(redraws, 0);
    expect_frame("idle, input focused", &total, &[]);

    let typed = interaction(&mut app, |app| key(app, KeyCode::KeyX, Some("x")));
    expect_frame(
        "type one character into an input",
        &typed,
        &[
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 6656),
            (SurfacePx, 480000),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 5),
            (GlyphCacheMisses, 1),
            (ClipMasks, 1),
            (ClipMaskPx, 7632),
        ],
    );
}

/// A paused `@keyframes` animation asks for no frame (#763).
#[test]
fn a_paused_animation_idles_for_free() {
    let mut app = mount_settled(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(
            "@keyframes k { from { transform: translateX(0px); } to { transform: translateX(50px); } }
             .spin { width: 20px; height: 20px; background: rgb(0, 0, 200);
                     animation: k 1s linear infinite; animation-play-state: paused; }",
        );
        style.append_child(&css);
        root.append_child(&style);
        let b = scope.create_element("div");
        b.set_attribute("class", "spin");
        root.append_child(&b);
        root
    });
    assert_eq!(
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .active_animations
            .len(),
        1,
        "positive control: the animation is registered (and paused)"
    );
    let (redraws, total) = idle_turns(&mut app, IDLE_TURNS);
    assert_eq!(redraws, 0);
    expect_frame("idle, paused animation", &total, &[]);
}

/// A `Loader` with the shipped component stylesheet installed, and nothing
/// else — bare, or inside a closed `Drawer`.
fn mount_loader(in_closed_drawer: bool) -> RinchApp {
    let mut app = new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = rinch_components::generate_component_css();
        let text = scope.create_text(&css);
        style.append_child(&text);
        root.append_child(&style);
        let loader = Loader::default().render(scope, &[]);
        if in_closed_drawer {
            let drawer = Drawer {
                opened_fn: Some(std::rc::Rc::new(|| false)),
                position: "left".to_string(),
                ..Default::default()
            }
            .render(scope, &[loader]);
            root.append_child(&drawer);
        } else {
            root.append_child(&loader);
        }
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    settle(&mut app);
    app
}

/// Idle turns with time passing, so a running animation has something to
/// advance. Returns each turn's frame.
fn ticking_turns(app: &mut RinchApp, n: usize) -> Vec<(bool, FrameStats)> {
    (0..n)
        .map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(3));
            let asked = about_to_wait(app);
            let s = if asked {
                paint(app)
            } else {
                app.end_perf_frame().unwrap()
            };
            (asked, s)
        })
        .collect()
}

/// A running `Loader` redraws every frame — it is moving — and each frame's
/// work is bounded and identical: the same counters, turn after turn. The
/// oval is a `transform` animation, so it must stay on the paint-only path (no
/// cascade, no Taffy compute, no shaping) and repaint only its own box.
///
/// Two counters are measured in a range instead of exactly, and only these
/// two: the repainted area and the clip-mask area are the **rotated** oval's
/// bounding box, which depends on the angle the clock landed on. The `md`
/// oval is 32px square; rotated, its box is 32 × (|cos θ| + |sin θ|) wide,
/// from 32px at 0° to 45.3px at 45°, and the damage is that box grown by the
/// dirty margin. The floors are derived, not sampled: the damage always
/// contains the oval's own 32px box, and the clip mask contains the damage,
/// so neither can be under 32². Sampled floors were flaky: measured values
/// reach down to 37² repainted and 39² of mask, and the review's 38²/42²
/// failed about one run in eight. The ceilings are the 45° box (45.3px) plus
/// 8px of margin, 54², and the mask 4px wider, 58², so a frame that repaints
/// more than the oval's neighbourhood still fails. Everything else is exact.
#[test]
fn a_running_loader_costs_the_same_bounded_frame_every_turn() {
    let mut app = mount_loader(false);
    let turns = ticking_turns(&mut app, 5);
    assert!(
        turns.iter().all(|(asked, _)| *asked),
        "positive control: a spinning loader asks for every frame"
    );
    for (i, (_, s)) in turns.iter().enumerate().skip(1) {
        for (c, lo, hi) in [
            (RepaintedPx, 32 * 32, 54 * 54),
            (ClipMaskPx, 32 * 32, 58 * 58),
        ] {
            assert!(
                (lo..=hi).contains(&s.get(c)),
                "turn {i}: {} = {} is outside the rotated oval's range {lo}..={hi}",
                c.name(),
                s.get(c)
            );
        }
        let angle_free = {
            let p = rinch_dom::perf::PerfCounters::default();
            p.add(RepaintedPx, s.get(RepaintedPx));
            p.add(ClipMaskPx, s.get(ClipMaskPx));
            s.since(&p.frame())
        };
        expect_frame(
            &format!("running Loader, turn {i}"),
            &angle_free,
            &[
                (PaintFrames, 1),
                (RepaintPartial, 1),
                (DamageRects, 1),
                (SurfacePx, 480000),
                (PaintNodesVisited, 4),
                (StackingOrderBuilds, 2),
                (ClipMasks, 1),
            ],
        );
    }
}

/// A `Loader` inside a **closed** `Drawer` lets the app sleep: no turn asks for
/// a frame, and the turns that ask for none do no work at all.
///
/// A closed drawer is `visibility: hidden` (#751), which is *rendered*, so an
/// animation under it runs unless something pauses it — a browser agrees. Until
/// #912 nothing did, and this scenario was pinned as a finding: every turn asked
/// for a redraw that painted nothing (`repaint_none` 1), forever. The drawer's
/// closed rule now declares `animation-play-state: paused` over its subtree, and
/// a paused animation asks for no frame (#763). The stylesheet here is the
/// shipped component CSS and nothing else.
#[test]
fn a_loader_in_a_closed_drawer_idles() {
    let mut app = mount_loader(true);
    {
        let d = app.doc.as_ref().unwrap().borrow();
        let all: Vec<_> = d.tree.active_animations.values().flatten().collect();
        assert_eq!(
            all.len(),
            1,
            "positive control: the oval's animation is registered, so the zero \
             below is a pause and not an animation that never started"
        );
        assert_eq!(
            all[0].play_state,
            rinch_dom::animation::AnimationPlayState::Paused,
            "and it is paused by the drawer's own closed rule"
        );
    }
    let turns = ticking_turns(&mut app, IDLE_TURNS);
    assert!(
        turns.iter().all(|(asked, _)| !*asked),
        "no turn asks for a frame"
    );
    let mut total = FrameStats::default();
    for (_, s) in &turns {
        total.accumulate(s);
    }
    expect_frame("Loader in a closed Drawer, idle", &total, &[]);
}

// ── Keyed `for`, 200 rows ──────────────────────────────────────────────────

const LIST_ROWS: usize = 200;

#[derive(Clone, PartialEq, Debug)]
struct Item {
    id: usize,
    label: String,
}

fn items(range: std::ops::Range<usize>) -> Vec<Item> {
    range
        .map(|id| Item {
            id,
            label: format!("item {id}"),
        })
        .collect()
}

/// A 200-row keyed list in a 300x400 scroller — narrow enough that damage
/// confined to it stays a partial repaint (the full-repaint threshold is half
/// the window), so the repainted area is part of what is pinned.
fn mount_list() -> (RinchApp, Signal<Vec<Item>>) {
    let list = Signal::new(items(0..LIST_ROWS));
    let app = mount_settled(move |__scope: &mut RenderScope| {
        rsx! {
            div {
                style { {ROW_CSS} }
                div { class: "scroller",
                    for item in list.get() {
                        div { key: item.id, class: "row", {item.label.clone()} }
                    }
                }
            }
        }
    });
    (app, list)
}

/// Connected nodes carrying `class="row"`.
fn row_count(app: &RinchApp) -> usize {
    use rinch_core::dom::{DomDocument, NodeId};
    let d = app.doc.as_ref().unwrap().borrow();
    let ids: Vec<usize> = d.tree.nodes.iter().map(|(id, _)| id).collect();
    ids.into_iter()
        .filter(|&id| d.get_attribute(NodeId(id), "class").as_deref() == Some("row"))
        .filter(|&id| {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if c == d.tree.root_id {
                    return true;
                }
                cur = d.tree.nodes[c].parent;
            }
            false
        })
        .count()
}

/// Move one row (the 150th to position 10): the reconcile repositions one
/// node and re-renders none (`effect_runs` 1: the `for` itself).
///
/// **Findings shared by all four `for` scenarios, pinned as they are; a fix must LOWER this number, and its PR updates the pin:**
///
/// - **#910, fixed: paint visits the rows the scroller shows** (25: html,
///   body, the chain down to the scroller, and the rows its clip and ink
///   margin let through), where it visited all 205. The clip the painter has
///   open is a cull, and a row it cuts away is dismissed by the scroller's
///   loop without a visit.
/// - **#909, fixed: the damage is clipped by the scroller.** `repainted_px` is
///   122 816 = 304 x 404 — the scroller's box and its 4px margin — where it
///   was 182 400 = 304 x 600, down to the window's bottom edge: rows the
///   change moved below the scroller's viewport, invisible, named damage.
///   Each rect is now intersected with its node's clip chain
///   (`paint::clip_chain_bounds`). The glyphs of the rows in the band that
///   no longer repaints went with it (`glyph_cache_hits` 201 → 138 over both
///   fixes).
/// - **Two clip masks cover 367 044 px** for that 122 816 px repaint: the
///   damage's own clip and the scroller's, each filled over its bounds.
/// - **#914, fixed:** the moved row is neither re-cascaded nor re-shaped
///   (`elements_cascaded` 0, `shape_*` 0): a move within one parent keeps its
///   style (`keeps_style_across_move`) and its own IFC layout
///   (`invalidate_ifc_left_by`). The one `ifc_measure_invalidations` left is
///   the parent's (`invalidate_parent_ifc`).
#[test]
fn a_keyed_for_moves_one_row() {
    let (mut app, list) = mount_list();
    let s = interaction(&mut app, |_| {
        list.update(|v| {
            let r = v.remove(150);
            v.insert(10, r);
        })
    });
    assert_eq!(row_count(&app), LIST_ROWS, "positive control");
    expect_frame(
        "for: move one row",
        &s,
        &[
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 2),
            (IfcScopeNodes, 203),
            (TaffyRootComputes, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 122816),
            (SurfacePx, 480000),
            (PaintNodesVisited, 25),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 138),
            (ClipMasks, 2),
            (ClipMaskPx, 367044),
            (PaintSurfaceAllocs, 1),
            (EffectRuns, 1),
            (SignalNotifies, 1),
        ],
    );
}

/// Insert one row in the middle.
#[test]
fn a_keyed_for_inserts_one_row_in_the_middle() {
    let (mut app, list) = mount_list();
    let s = interaction(&mut app, |_| {
        list.update(|v| {
            v.insert(
                100,
                Item {
                    id: 10_000,
                    label: "new".into(),
                },
            )
        })
    });
    assert_eq!(row_count(&app), LIST_ROWS + 1, "positive control");
    expect_frame(
        "for: insert one row mid-list",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (TaffyStyleSyncs, 2),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 2),
            (IfcScopeNodes, 204),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 122816),
            (SurfacePx, 480000),
            (PaintNodesVisited, 25),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 137),
            (ClipMasks, 2),
            (ClipMaskPx, 367044),
            (PaintSurfaceAllocs, 1),
            (EffectRuns, 1),
            (SignalNotifies, 1),
        ],
    );
}

/// Remove one row from the middle.
#[test]
fn a_keyed_for_removes_one_row_from_the_middle() {
    let (mut app, list) = mount_list();
    let s = interaction(&mut app, |_| {
        list.update(|v| {
            v.remove(100);
        })
    });
    assert_eq!(row_count(&app), LIST_ROWS - 1, "positive control");
    expect_frame(
        "for: remove one row mid-list",
        &s,
        &[
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 1),
            (IfcScopeNodes, 201),
            (TaffyRootComputes, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 122816),
            (SurfacePx, 480000),
            (PaintNodesVisited, 25),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 137),
            (ClipMasks, 2),
            (ClipMaskPx, 367044),
            (PaintSurfaceAllocs, 1),
            (EffectRuns, 1),
            (SignalNotifies, 1),
        ],
    );
}

/// Replace every row with new keys: 200 removals and 200 renders.
#[test]
fn a_keyed_for_replaces_every_row() {
    let (mut app, list) = mount_list();
    let s = interaction(&mut app, |_| list.set(items(LIST_ROWS..2 * LIST_ROWS)));
    assert_eq!(row_count(&app), LIST_ROWS, "positive control");
    expect_frame(
        "for: replace all rows",
        &s,
        &[
            (StyleResolves, 201),
            (ElementsCascaded, 200),
            (StyleNodesVisited, 200),
            (TaffyStyleSyncs, 400),
            (TaffyStyleChanges, 200),
            (ShapeMeasureIfc, 200),
            (ShapeIfcBuild, 200),
            (IfcMeasureInvalidations, 600),
            (IfcSignatureChanges, 200),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 201),
            (IfcScopeNodes, 402),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 200),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 122816),
            (SurfacePx, 480000),
            (PaintNodesVisited, 25),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 168),
            (ClipMasks, 2),
            (ClipMaskPx, 367044),
            (PaintSurfaceAllocs, 1),
            (EffectRuns, 1),
            (SignalNotifies, 1),
        ],
    );
}

// ── Scroll ─────────────────────────────────────────────────────────────────

const SCROLL_ROWS: usize = 500;

/// A 300x400 scroller of 500 text rows (narrow for the reason `mount_list` gives).
fn mount_scroller() -> (RinchApp, NodeHandle) {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let app = mount_settled(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(ROW_CSS);
        style.append_child(&css);
        root.append_child(&style);
        let scroller = scope.create_element("div");
        scroller.set_attribute("class", "scroller");
        for i in 0..SCROLL_ROWS {
            let row = scope.create_element("div");
            row.set_attribute("class", "row");
            let t = scope.create_text(&format!("row {i}"));
            row.append_child(&t);
            scroller.append_child(&row);
        }
        root.append_child(&scroller);
        *out2.borrow_mut() = Some(scroller.clone());
        root
    });
    let scroller = out.borrow().clone().unwrap();
    (app, scroller)
}

/// One wheel notch over a 500-row scroller: the scroller's box repaints, no
/// element is restyled, nothing is laid out, nothing is shaped.
///
/// **Finding, pinned as it is — #911 (hit tests); a fix must LOWER this
/// number, and its PR updates the pin:** the notch runs **two** hit tests and
/// recomputes **498** subtree extents — the scroll invalidates the hit cache
/// (it has to: the rows moved), and the next test rebuilds every row's extent
/// rather than the handful under the pointer. Paint visits **24** nodes for the
/// ~20 rows on screen; it visited all 504 until #910.
#[test]
fn a_wheel_scroll_repaints_the_scroller_and_restyles_nothing() {
    let (mut app, scroller) = mount_scroller();
    let s = interaction(&mut app, |app| {
        app.handle_event(
            PlatformEvent::MouseWheel {
                x: 50.0,
                y: 100.0,
                delta_x: 0.0,
                delta_y: -100.0,
            },
            SIZE,
            1.0,
        );
    });
    assert!(scroller.scroll_top() > 0.0, "positive control: it scrolled");
    expect_frame(
        "wheel scroll, 500 rows",
        &s,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 1),
            (RepaintedPx, 122816),
            (SurfacePx, 480000),
            (PaintNodesVisited, 24),
            (StackingOrderBuilds, 2),
            (GlyphCacheHits, 121),
            (ClipMasks, 2),
            (ClipMaskPx, 367044),
            (PaintSurfaceAllocs, 1),
            (HitTests, 2),
            (HitTestNodesVisited, 8),
            (HitExtentsComputed, 498),
        ],
    );
}

// ── Drag ───────────────────────────────────────────────────────────────────

/// A panel dragged by `Drag::absolute` over ten moves queued before one frame:
/// one layout for the ten, and a repaint of the panel's old and new rects.
#[test]
fn ten_queued_drag_moves_lay_out_once() {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let mut app = mount_settled(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let panel = scope.create_element("div");
        panel.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 50px; height: 50px; \
             background: rgb(0, 0, 200)",
        );
        root.append_child(&panel);
        *out2.borrow_mut() = Some(panel.clone());
        root
    });
    let panel = out.borrow().clone().unwrap();
    let x = Signal::new(0.0f32);
    let _e = rinch_core::reactive::Effect::new(move || {
        panel.set_style("left", &format!("{}px", x.get()));
    });
    settle(&mut app);
    rinch_core::Drag::absolute()
        .on_move(move |px, _| x.set(px))
        .start();
    let s = interaction(&mut app, |app| {
        for i in 0..10 {
            app.handle_event(
                PlatformEvent::MouseMove {
                    x: 100.0 + i as f32 * 10.0,
                    y: 25.0,
                },
                SIZE,
                1.0,
            );
        }
    });
    rinch_core::Drag::cancel();
    assert_eq!(
        x.get(),
        190.0,
        "positive control: every move reached on_move"
    );
    expect_frame(
        "drag, 10 queued moves",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintFrames, 1),
            (RepaintPartial, 1),
            (DamageRects, 2),
            (RepaintedPx, 6048),
            (SurfacePx, 480000),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
            (ClipMasks, 1),
            (ClipMaskPx, 13776),
            (PaintSurfaceAllocs, 1),
            (EffectRuns, 10),
            (SignalNotifies, 10),
        ],
    );
}

// ── Theme and scale ────────────────────────────────────────────────────────

/// A dark-mode toggle: the document restyles in full and the frame repaints in
/// full, for the theme reason.
///
/// **A finding, pinned as it is — #913; a fix must LOWER this number, and its PR updates the pin.** Every paragraph's paint layout is rebuilt
/// (`shape_ifc_build` 211: every row and side-scroller row) — a full restyle drops them — though the one
/// variable the toggle changes is used by no text here.
#[cfg(feature = "theme")]
#[test]
fn a_theme_toggle_restyles_and_repaints_in_full_for_the_theme() {
    let mut app = static_page();
    app.set_owned_theme_css(Some(":root { --rinch-primary-color: #4dabf7; }".into()));
    settle(&mut app);
    app.set_owned_theme_css(Some(":root { --rinch-primary-color: #1864ab; }".into()));
    let s = interaction(&mut app, |app| {
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    });
    assert_eq!(s.get(RepaintFullTheme), 1, "positive control: {s:?}");
    expect_frame(
        "theme toggle",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 216),
            (StyleNodesVisited, 427),
            (FullRestyles, 1),
            (FullRestyleTheme, 1),
            (FullStyleWalks, 1),
            (TaffyStyleSyncs, 216),
            (ShapeIfcBuild, 211),
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullTheme, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullTheme, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 136),
            (ClipMasks, 1),
            (ClipMaskPx, 33856),
        ],
    );
}

/// A scale-factor change (the window dragged to a 2x display): the document
/// restyles for the device pixel ratio, and the surface — twice the pixels —
/// repaints in full. The frame's reason is `resize` (the surface resize is
/// decided first), not `restyle`: both apply, and one full repaint is counted.
#[test]
fn a_scale_factor_change_restyles_and_repaints_in_full() {
    let mut app = static_page();
    app.handle_event(PlatformEvent::ScaleFactorChanged(2.0), SIZE, 2.0);
    about_to_wait(&mut app);
    let s = paint_at(&mut app, 2.0);
    assert_eq!(s.get(FullRestyleDpr), 1, "positive control: {s:?}");
    expect_frame(
        "scale factor 1 -> 2",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 216),
            (StyleNodesVisited, 427),
            (FullRestyles, 1),
            (FullRestyleDpr, 1),
            (FullStyleWalks, 1),
            (TaffyStyleSyncs, 216),
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullResize, 1),
            (RepaintedPx, 1920000),
            (SurfacePx, 1920000),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 115),
            (GlyphCacheMisses, 21),
            (ClipMasks, 1),
            (ClipMaskPx, 132496),
            (PaintSurfaceAllocs, 1),
        ],
    );
}

// ── Every full-repaint reason (#879) ───────────────────────────────────────
//
// `first_frame`, `resize` and `invalidated` are pinned in `perf_stats_tests`,
// `restyle` and `unattributed` in `named_damage_tests`, each by its own
// counter alone. These assert the whole frame for every reason, so relabelling
// one reason as another fails here.

/// The first frame: no previous pixels.
#[test]
fn full_repaint_first_frame() {
    let mut app = new_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "font-family: sans-serif; font-size: 14px; line-height: 20px",
        );
        let t = scope.create_text("hello");
        root.append_child(&t);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.reset_perf();
    let s = paint(&mut app);
    expect_frame(
        "full repaint: first frame",
        &s,
        &[
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullFirstFrame, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 1),
            (GlyphCacheMisses, 4),
        ],
    );
}

/// The surface was resized.
#[test]
fn full_repaint_resize() {
    let mut app = static_page();
    app.resize_layout(SIZE.0 + 1, SIZE.1);
    let _ = app.build_pixels(1.0, (SIZE.0 + 1, SIZE.1), false);
    let s = app.end_perf_frame().unwrap();
    expect_frame(
        "full repaint: resize",
        &s,
        &[
            (StyleResolves, 1),
            (TaffyStyleSyncs, 2),
            (ShapeMeasureIfc, 10),
            (ShapeIfcBuild, 10),
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 11),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullResize, 1),
            (RepaintedPx, 480600),
            (SurfacePx, 480600),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 136),
            (ClipMasks, 1),
            (ClipMaskPx, 33856),
            (PaintSurfaceAllocs, 1),
        ],
    );
}

/// Something marked the scene dirty without naming what changed, and nothing
/// else was damaged.
#[test]
fn full_repaint_unattributed() {
    let mut app = static_page();
    app.mark_scene_dirty();
    let s = paint(&mut app);
    expect_frame(
        "full repaint: unattributed",
        &s,
        &[
            (ShapePaint, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullUnattributed, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 136),
            (ClipMasks, 1),
            (ClipMaskPx, 33856),
        ],
    );
}

/// The damage covers half the surface or more.
#[test]
fn full_repaint_region_too_large() {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let mut app = mount_settled(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        let css = scope.create_text(ROW_CSS);
        style.append_child(&css);
        root.append_child(&style);
        let big = scope.create_element("div");
        big.set_attribute("class", "big");
        root.append_child(&big);
        *out2.borrow_mut() = Some(big.clone());
        root
    });
    let big = out.borrow().clone().unwrap();
    let s = interaction(&mut app, |_| big.set_attribute("class", "big on"));
    expect_frame(
        "full repaint: region too large",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullRegionTooLarge, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A `<style>` element was added: a whole-document restyle, which can change
/// any node's paint without naming one.
#[test]
fn full_repaint_restyle() {
    let mut app = static_page();
    let s = interaction(&mut app, |app| {
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        let body = d.body();
        let style = d.create_element("style");
        let css = d.create_text(".late { color: rgb(0, 0, 200); }");
        d.append_child(style, css);
        d.append_child(body, style);
    });
    expect_frame(
        "full repaint: restyle",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 217),
            (StyleNodesVisited, 428),
            (FullRestyles, 1),
            (FullRestyleStylesheet, 1),
            (FullStyleWalks, 1),
            (TaffyStyleSyncs, 218),
            (ShapeIfcBuild, 1),
            (ShapePaint, 1),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 2),
            (IfcScopeNodes, 4),
            (TaffyRootComputes, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullRestyle, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 136),
            (ClipMasks, 1),
            (ClipMaskPx, 33856),
        ],
    );
}

/// The previous frame was thrown away with no reason recorded.
#[test]
fn full_repaint_invalidated() {
    let mut app = static_page();
    app.has_previous_frame = false;
    app.request_repaint();
    let s = paint(&mut app);
    expect_frame(
        "full repaint: invalidated",
        &s,
        &[
            (ShapePaint, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullInvalidated, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
            (GlyphCacheHits, 136),
            (ClipMasks, 1),
            (ClipMaskPx, 33856),
        ],
    );
}

/// A repaint was requested and named no damage: nothing is painted at all.
#[test]
fn an_empty_damage_repaints_nothing() {
    let mut app = static_page();
    app.request_repaint();
    let s = paint(&mut app);
    expect_frame("repaint none", &s, &[(RepaintNone, 1)]);
}

/// The GPU (and embed) path always re-encodes the whole scene.
#[cfg(any(feature = "gpu", feature = "embed"))]
#[test]
fn full_repaint_gpu() {
    let mut app = static_page();
    app.request_repaint();
    let _ = app.build_scene(1.0, SIZE);
    let s = app.end_perf_frame().unwrap();
    expect_frame(
        "full repaint: gpu",
        &s,
        &[
            (ShapePaint, 1),
            (PaintFrames, 1),
            (RepaintFull, 1),
            (RepaintFullGpu, 1),
            (RepaintedPx, 480000),
            (SurfacePx, 480000),
            (PaintNodesVisited, 27),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── Counters with no exact scenario (#879, item 3) ─────────────────────────

/// `rerender_events_queued` is folded into the frame by `end_perf_frame` from
/// a process-wide atomic, which the runtime's dispatcher bumps
/// (`rinch_runtime::send_native_event_counts_one_rerender_per_drain` pins the
/// bump). This pins the fold: three events queued between two frames reach the
/// frame, exactly. The atomic is process-wide, so the only two tests that
/// write it — this one and `send_native_event_counts_one_rerender_per_drain` —
/// both hold `RERENDER_EVENTS_TEST_LOCK`.
///
/// `time_present_ns` has no such check. It is added only by the desktop and
/// Android runtimes' present, after a real window's `present_pixels` or GPU
/// submit, and no test here has a window.
#[test]
fn queued_rerender_events_are_folded_into_the_frame() {
    let mut app = static_page();
    let _lock = RERENDER_EVENTS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _ = app.end_perf_frame();
    RERENDER_EVENTS_QUEUED.fetch_add(3, std::sync::atomic::Ordering::Relaxed);
    let s = app.end_perf_frame().unwrap();
    assert_eq!(s.get(RerenderEventsQueued), 3, "{s:?}");
}
