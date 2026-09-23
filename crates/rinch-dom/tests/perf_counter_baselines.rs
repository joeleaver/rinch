//! Counter baselines for the update paths the 2026-09 performance audit
//! measured (`rinch_dom::perf`, `docs/src/guide/performance.md`).
//!
//! Each scenario builds the same list document the audit's probe used —
//! `body > div.list (flex column) > ROWS × div.row`, every row holding
//! `span > text`, optionally a `display: contents` wrapper around a second
//! text (what `rsx!` emits for every `{|| …}` and every `if`/`for`/`match`),
//! and an `inline-block` chip — lays it out twice, paints it once, zeroes the
//! counters, performs **one** operation, re-lays out, paints again in full,
//! and reads the frame.
//!
//! # Every counter is pinned exactly
//!
//! Counters, not timings, because counters are deterministic: the same
//! operation on the same document takes the same number of cascades, Parley
//! shapes, IFC passes and Taffy computes on every run and under any load.
//! So each scenario asserts **the whole frame** — every non-`time_*` counter,
//! with any counter the baseline does not list expected to be `0`. That
//! catches both directions:
//!
//! - a path that got **more expensive** (a counter rose), and
//! - an instrument that **stopped counting** (a counter fell to 0). A bare
//!   upper bound cannot see the second: a deleted increment reads as a win.
//!
//! **A performance fix is expected to change these numbers.** Editing the
//! baseline to the new values *is* the proof the fix worked; say in the PR
//! which counters moved and why. A counter that moved when nothing should
//! have moved it is a regression or a broken increment — find out which.
//!
//! The values do depend on layout — which rows fall inside the 800x600
//! window decides `paint_nodes_visited` — and every line box is therefore
//! declared (`line-height: 20px`, fixed paddings) rather than derived from a
//! font metric, so the numbers do not move with the host's font set.
//!
//! `PERF_BASELINE_PRINT=1 cargo test -p rinch-dom --test perf_counter_baselines
//! -- --nocapture` prints each scenario's frame as a ready-to-paste baseline.
//! The `#[ignore]`d `perf_scenario_timings` prints wall-clock timings for the
//! same scenarios; run it in release:
//!
//! ```text
//! cargo test --release -p rinch-dom --test perf_counter_baselines -- --ignored --nocapture
//! ```
//!
//! # Every document is measured at one viewport
//!
//! A viewport change of more than half a pixel restyles the whole document,
//! which is a second route to most of these numbers; only `resize_by_1px`
//! takes it on purpose.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::{Counter, FrameStats};

use Counter::*;

const ROWS: usize = 40;
const VP: (f32, f32) = (800.0, 600.0);

const CSS: &str = "
    body { font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .list { display: flex; flex-direction: column; }
    .row { display: block; padding: 2px; }
    .row:hover { background-color: rgb(200, 0, 0); }
    .row.sel { background-color: rgb(0, 0, 200); }
    .chip { display: inline-block; padding: 2px; }
    @keyframes perf-spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
    .spin { animation: perf-spin 1000s linear infinite; }
";

struct Fixture {
    doc: RinchDocument,
    list: NodeId,
    rows: Vec<NodeId>,
    texts: Vec<NodeId>,
    chips: Vec<NodeId>,
    painter: TinySkiaPainter,
}

fn build(contents_wrapper: bool) -> Fixture {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::new();
    let mut texts = Vec::new();
    let mut chips = Vec::new();
    for i in 0..ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let span = doc.create_element("span");
        let t = doc.create_text(&format!("Row number {i} with some text"));
        doc.append_child(span, t);
        doc.append_child(row, span);
        if contents_wrapper {
            let w = doc.create_element("span");
            doc.set_attribute(w, "style", "display: contents");
            let t2 = doc.create_text(&format!(" ({i})"));
            doc.append_child(w, t2);
            doc.append_child(row, w);
            texts.push(t2);
        } else {
            texts.push(t);
        }
        let chip = doc.create_element("span");
        doc.set_attribute(chip, "class", "chip");
        let t3 = doc.create_text("chip");
        doc.append_child(chip, t3);
        doc.append_child(row, chip);
        doc.append_child(list, row);
        rows.push(row);
        chips.push(chip);
    }
    doc.append_child(body, list);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let mut f = Fixture {
        doc,
        list,
        rows,
        texts,
        chips,
        painter: TinySkiaPainter::new(VP.0 as u32, VP.1 as u32),
    };
    paint(&mut f);
    f.doc.tree.perf.reset();
    f
}

fn paint(f: &mut Fixture) {
    let doc = &mut f.doc;
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut f.painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
}

/// One operation, then the frame the shell would paint for it.
fn frame(f: &mut Fixture, op: impl FnOnce(&mut Fixture)) -> FrameStats {
    op(f);
    f.doc.resolve_layout(VP.0, VP.1);
    paint(f);
    f.doc.tree.perf.end_frame()
}

/// Assert the whole frame: every counter in `baseline` has exactly that
/// value, and every other non-timing counter is `0`.
#[track_caller]
fn expect(name: &str, stats: &FrameStats, baseline: &[(Counter, u64)]) {
    if std::env::var_os("PERF_BASELINE_PRINT").is_some() {
        println!("{name}:");
        for (c, v) in stats.iter() {
            if v != 0 && !c.name().starts_with("time_") {
                println!("            ({c:?}, {v}),");
            }
        }
    }
    let mut diffs = Vec::new();
    for (c, v) in stats.iter() {
        if c.name().starts_with("time_") {
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
    for (i, (c, _)) in baseline.iter().enumerate() {
        assert!(
            !baseline[..i].iter().any(|(d, _)| d == c),
            "{name}: {} listed twice in its baseline",
            c.name()
        );
    }
    assert!(
        diffs.is_empty(),
        "{name}: the frame differs from its recorded baseline.\n  {}\n\
         A rise is a path that got more expensive; a fall to 0 may be an \
         increment that stopped counting. If a fix moved these on purpose, \
         update the baseline (PERF_BASELINE_PRINT=1 prints it) and say why.",
        diffs.join("\n  ")
    );
}

fn hover(f: &mut Fixture, row: Option<NodeId>) {
    let mut changed = false;
    f.doc.update_hover(row.map(|r| r.0), &mut changed);
}

// ── Positive control ───────────────────────────────────────────────────────

/// Every counter rinch-dom owns fires on *some* path. Without this, a
/// counter that no scenario reaches could lose its increment and every
/// baseline would still pass with it at 0.
///
/// The shell-only counters (paint frames, repaint reasons, repainted and
/// surface pixels, hit tests, the reactive deltas, present time) are pinned
/// by `crates/rinch/src/app/perf_stats_tests.rs` instead.
#[test]
fn every_rinch_dom_counter_fires_somewhere() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "
        body { font-size: 16px; line-height: 20px; }
        .flex { display: flex; }
        .chip { display: inline-block; padding: 2px; }
        .clip { width: 60px; overflow: hidden; white-space: nowrap;
                text-overflow: ellipsis; }
        .calc { width: calc(50% - 10px); height: 10px; }
        .z { position: relative; z-index: 1; }
        ",
    );
    let body = doc.body();
    // An IFC root holding an atomic inline.
    let p = doc.create_element("p");
    let t = doc.create_text("hello ");
    doc.append_child(p, t);
    let chip = doc.create_element("span");
    doc.set_attribute(chip, "class", "chip");
    let ct = doc.create_text("chip");
    doc.append_child(chip, ct);
    doc.append_child(p, chip);
    doc.append_child(body, p);
    // A text leaf that is a flex item (measured through `NodeContext::Text`).
    let flex = doc.create_element("div");
    doc.set_attribute(flex, "class", "flex");
    let ft = doc.create_text("flex item text");
    doc.append_child(flex, ft);
    doc.append_child(body, flex);
    // Ellipsis truncation.
    let clip = doc.create_element("div");
    doc.set_attribute(clip, "class", "clip");
    let clt = doc.create_text("a line much too long for sixty pixels");
    doc.append_child(clip, clt);
    doc.append_child(body, clip);
    // A mixed calc() (the fixpoint), an input (painted text) and a stacking
    // context of its own.
    let calc = doc.create_element("div");
    doc.set_attribute(calc, "class", "calc z");
    doc.append_child(body, calc);
    let input = doc.create_element("input");
    doc.set_attribute(input, "value", "typed");
    doc.append_child(body, input);

    doc.resolve_layout(VP.0, VP.1);
    // A text edit: cache retains, and hits on the second measure.
    doc.set_text_content(t, "hello again ");
    doc.resolve_layout(VP.0, VP.1);
    // A colour-only restyle: the text-only skip, and a paint-only one after.
    doc.set_style(p, "color", "rgb(255, 0, 0)");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    // Whole-document restyles by each reason rinch-dom can be asked for.
    doc.recompute_all_styles_full();
    doc.set_device_pixel_ratio(2.0);
    let style = doc.create_element("style");
    let css = doc.create_text(".late { color: blue; }");
    doc.append_child(style, css);
    doc.append_child(body, style);
    doc.load_css("html { font-size: 20px; }");
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    let mut painter = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
    let total = doc.tree.perf.end_frame();

    let rinch_dom_counters = [
        StyleResolves,
        ElementsCascaded,
        StyleNodesVisited,
        PseudoElementPasses,
        FullRestyles,
        FullRestyleViewport,
        FullRestyleTheme,
        FullRestyleStylesheet,
        FullRestyleDpr,
        FullRestyleRootFontSize,
        FullStyleWalks,
        TaffyStyleSyncs,
        TaffyStyleChanges,
        ShapeMeasureIfc,
        ShapeMeasureText,
        ShapeIfcBuild,
        ShapeAtomicInline,
        EllipsisBuilds,
        ShapePaint,
        IfcMeasureCacheHits,
        IfcMeasureInvalidations,
        IfcSignatureChanges,
        LayoutResolves,
        LayoutSkippedPaintOnly,
        LayoutSkippedTextOnly,
        IfcSetupPasses,
        TaffyRootComputes,
        TaffyMeasureCalls,
        InlineBlockComputes,
        CalcFixpointPasses,
        PaintNodesVisited,
        StackingOrderBuilds,
        TimeStyleNs,
        TimeLayoutNs,
        TimeIfcSetupNs,
        TimeTaffyComputeNs,
        TimeBuildIfcNs,
    ];
    let silent: Vec<&str> = rinch_dom_counters
        .iter()
        .filter(|&&c| total.get(c) == 0)
        .map(|c| c.name())
        .collect();
    assert!(
        silent.is_empty(),
        "these rinch-dom counters never fired: {silent:?}\n{total:?}"
    );
    assert_eq!(
        total.get(TaffyRootComputes),
        doc.tree.taffy_computes,
        "taffy_root_computes mirrors NodeTree::taffy_computes"
    );
    assert_eq!(
        total.get(IfcSetupPasses),
        doc.tree.ifc_setup_passes,
        "ifc_setup_passes mirrors NodeTree::ifc_setup_passes"
    );
}

// ── Scenarios ──────────────────────────────────────────────────────────────

/// Nothing dirty: the layout early-returns. The full paint still walks every
/// on-screen node (the rows below the fold are culled): the shell's dirty
/// region, not rinch-dom, is what makes an idle frame cheap.
#[test]
fn idle_frame() {
    let mut f = build(true);
    let s = frame(&mut f, |_| {});
    expect(
        "idle",
        &s,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 66),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// `.row:hover { background-color }` on a row with no `display: contents`
/// inside: a paint-only restyle, no Taffy compute, no IFC setup pass.
///
/// No text is re-shaped: the cascade compares each re-cascaded node's text
/// inputs and finds none changed, so every IFC keeps its Parley layout and
/// the frame takes the paint-only path (`shape_ifc_build` was 2 and the path
/// was the text-only one while every restyle dropped every layout under the
/// restyled node).
///
/// Still more than the minimum (audit F3 / update-path F1.1): the whole row
/// subtree is re-cascaded (row + span + chip; the minimum is 1).
#[test]
fn hover_colour_only_without_wrapper() {
    let mut f = build(false);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| hover(f, Some(row)));
    expect(
        "hover (no wrapper)",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 5),
            (PseudoElementPasses, 6),
            (TaffyStyleSyncs, 3),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 66),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The same hover, on rows that carry a `display: contents` wrapper — what
/// every `rsx!` reactive text produces. Since #875 the wrapper's re-cascade no
/// longer rewrites its Taffy style, so this reads like the no-wrapper hover
/// plus the wrapper's own cascade: no IFC setup pass, no compute, and — as
/// for the no-wrapper hover — no text re-shaped.
#[test]
fn hover_colour_only_with_contents_wrapper() {
    let mut f = build(true);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| hover(f, Some(row)));
    expect(
        "hover (contents wrapper)",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 7),
            (PseudoElementPasses, 8),
            (TaffyStyleSyncs, 4),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 66),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A class toggle that changes only `background-color` (`.row` ↔ `.row.sel`).
/// As for the hover, an attribute write re-cascades the whole subtree (audit
/// F3) but re-shapes no text, since no text input changed.
#[test]
fn colour_only_class_toggle() {
    let mut f = build(false);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| f.doc.set_attribute(row, "class", "row sel"));
    expect(
        "class toggle (colour)",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 5),
            (PseudoElementPasses, 6),
            (TaffyStyleSyncs, 3),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 66),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Append one row to the list. A structural change anywhere still re-runs the
/// whole-document IFC setup pass, but the pass no longer clears the measure
/// cache: it compares every root's content signature and drops only the roots
/// whose content moved — here just the new row (`ifc_signature_changes`), so
/// **one** root is shaped (`shape_measure_ifc` was ROWS + 1 = 41) and every
/// other measure is a cache hit.
///
/// Still more than the minimum:
///
/// - Taffy still *asks* for every row (`taffy_measure_calls`, four available
///   spaces per row): `sync_display_contents` re-splices every `display:
///   contents` wrapper in the document on a structural pass, and the
///   `set_children` that does it dirties each row's Taffy node. The rows'
///   cached sizes answer (`ifc_measure_cache_hits`), so it costs a lookup,
///   not a shape. `append_one_row_without_wrappers` is the same append with
///   no wrapper to re-splice.
/// - every chip is still re-sized by its own compute (audit layout F11).
/// - appending the text into the still-detached row also restyles
///   synchronously with no style root recorded, which walks the whole
///   document (`style_nodes_visited`) to cascade one element:
///   `resolve_styles`' empty-roots fallback.
#[test]
fn append_one_row() {
    let mut f = build(true);
    let s = frame(&mut f, |f| {
        let row = f.doc.create_element("div");
        f.doc.set_attribute(row, "class", "row");
        let t = f.doc.create_text("new row");
        f.doc.append_child(row, t);
        let list = f.list;
        f.doc.append_child(list, row);
    });
    expect(
        "append one row",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 287),
            (PseudoElementPasses, 2),
            (FullStyleWalks, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (ShapeAtomicInline, 40),
            (IfcMeasureCacheHits, 163),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 164),
            (InlineBlockComputes, 40),
            (PaintNodesVisited, 67),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The same append on rows with no `display: contents` wrapper, so nothing
/// re-splices the rows' Taffy nodes. Taffy asks each old row **once**
/// (`taffy_measure_calls` = ROWS + the new row's 4, against
/// `append_one_row`'s four per row), and rinch's cache answers each
/// (`ifc_measure_cache_hits`): the column grew by a row, so the available
/// height every row's final layout is keyed under changed, and Taffy's leaf
/// cache — which keys a final layout on its available space even when both
/// of the leaf's own dimensions are already known — misses and calls the
/// measure function for the content size. That one is Taffy's, not rinch's.
/// And no chip is re-shaped (`shape_atomic_inline` 0 against 40): their
/// detached computes hit Taffy's own cache, since nothing re-spliced them.
#[test]
fn append_one_row_without_wrappers() {
    let mut f = build(false);
    let s = frame(&mut f, |f| {
        let row = f.doc.create_element("div");
        f.doc.set_attribute(row, "class", "row");
        let t = f.doc.create_text("new row");
        f.doc.append_child(row, t);
        let list = f.list;
        f.doc.append_child(list, row);
    });
    expect(
        "append one row (no wrapper)",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 207),
            (PseudoElementPasses, 2),
            (FullStyleWalks, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 43),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 44),
            (InlineBlockComputes, 40),
            (PaintNodesVisited, 67),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Remove one row: the same whole-document pass as an append. No root's
/// content changed — the list is not an IFC — so nothing is shaped at all
/// (`shape_measure_ifc` was ROWS - 1 = 39) and every measure is a hit. The
/// removed row's chip is detached, yet `inline_block_computes` stays at ROWS:
/// the atomic-inline scan walks the whole slab, detached subtrees included.
#[test]
fn remove_one_row() {
    let mut f = build(true);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| {
        let list = f.list;
        f.doc.remove_child(list, row);
    });
    expect(
        "remove one row",
        &s,
        &[
            (ShapeAtomicInline, 40),
            (IfcMeasureCacheHits, 156),
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 156),
            (InlineBlockComputes, 40),
            (PaintNodesVisited, 65),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Change one text node's content: a text-only layout, no structural pass.
///
/// The row's IFC is re-measured — four available spaces, each shaped once
/// (`taffy_measure_calls` = `shape_measure_ifc` = 4). They were **0** before,
/// which was bug #878: the text sits in a wrapper inside the row, the
/// invalidation dropped the row's cached measure but marked no Taffy node the
/// compute would reach (the wrapper is detached from Taffy), so the compute
/// served the row's old size and a text that grew a line left its box a line
/// short. `invalidate_ifc_root` now marks the root.
#[test]
fn set_text_on_one_row() {
    let mut f = build(true);
    let t = f.texts[ROWS / 2];
    let s = frame(&mut f, |f| f.doc.set_text_content(t, " (changed text)"));
    expect(
        "set_text_content",
        &s,
        &[
            (ShapeMeasureIfc, 4),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (TaffyMeasureCalls, 4),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (PaintNodesVisited, 66),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Resize the viewport by one pixel: a full-document restyle (the stylist
/// rebuilds its device and every element is re-cascaded, two pseudo-element
/// passes each) and a re-measure of every row (audit layout F4). Since #875
/// no wrapper rewrites its Taffy style, so there is no IFC setup pass.
#[test]
fn resize_by_1px() {
    let mut f = build(true);
    f.doc.resolve_layout(VP.0 + 1.0, VP.1);
    let s = f.doc.tree.perf.end_frame();
    expect(
        "resize 1px",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 163),
            (StyleNodesVisited, 283),
            (PseudoElementPasses, 326),
            (FullRestyles, 1),
            (FullRestyleViewport, 1),
            (FullStyleWalks, 1),
            (TaffyStyleSyncs, 163),
            (ShapeMeasureIfc, 40),
            (ShapeIfcBuild, 40),
            (IfcMeasureCacheHits, 120),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 160),
        ],
    );
}

/// One tick of a `transform` animation on one chip, on rows **without**
/// wrappers: a transform moves no box, so the frame must take the paint-only
/// path — no cascade, no compute, no shaping. The spinning chip is a stacking
/// context of its own, so paint builds two sequences (body + chip).
#[test]
fn transform_animation_tick() {
    let mut f = build(false);
    let chip = f.chips[ROWS / 2];
    f.doc.set_attribute(chip, "class", "chip spin");
    f.doc.resolve_layout(VP.0, VP.1);
    paint(&mut f);
    f.doc.tree.perf.reset();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let s = frame(&mut f, |f| {
        f.doc.tick_animations();
    });
    expect(
        "transform animation tick",
        &s,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 66),
            (StackingOrderBuilds, 2),
        ],
    );
}

// ── Timings (release, ignored) ─────────────────────────────────────────────

/// Wall-clock per scenario, best of `REPS`, with the counters beside it.
/// Informational only; nothing is asserted.
#[test]
#[ignore]
fn perf_scenario_timings() {
    const REPS: usize = 20;
    type Op = fn(&mut Fixture, usize);
    let scenarios: [(&str, bool, Op); 7] = [
        ("idle", true, |_, _| {}),
        ("hover (no wrapper)", false, |f, r| {
            let row = f.rows[ROWS / 2];
            hover(f, (r % 2 == 0).then_some(row));
        }),
        ("hover (contents wrapper)", true, |f, r| {
            let row = f.rows[ROWS / 2];
            hover(f, (r % 2 == 0).then_some(row));
        }),
        ("class toggle (colour)", false, |f, r| {
            let row = f.rows[ROWS / 2];
            f.doc
                .set_attribute(row, "class", if r % 2 == 0 { "row sel" } else { "row" });
        }),
        ("append one row", true, |f, _| {
            let row = f.doc.create_element("div");
            let t = f.doc.create_text("new row");
            f.doc.append_child(row, t);
            let list = f.list;
            f.doc.append_child(list, row);
        }),
        ("set_text_content", true, |f, r| {
            let t = f.texts[ROWS / 2];
            f.doc
                .set_text_content(t, if r % 2 == 0 { " (changed)" } else { " (again)" });
        }),
        ("resize 1px", true, |f, r| {
            f.doc.resolve_layout(VP.0 + (r % 2) as f32 + 1.0, VP.1);
        }),
    ];
    println!(
        "{:<28} {:>10} {:>10} {:>10} {:>10}",
        "scenario", "style ms", "layout ms", "paint ms", "total ms"
    );
    for (name, contents, op) in scenarios {
        let mut f = build(contents);
        let mut best: Option<(f64, FrameStats)> = None;
        for r in 0..REPS {
            let t = std::time::Instant::now();
            op(&mut f, r);
            f.doc.resolve_layout(VP.0, VP.1);
            let pt = std::time::Instant::now();
            paint(&mut f);
            f.doc
                .tree
                .perf
                .add(Counter::TimePaintNs, pt.elapsed().as_nanos() as u64);
            let total = t.elapsed().as_secs_f64() * 1000.0;
            let stats = f.doc.tree.perf.end_frame();
            if best.as_ref().is_none_or(|(b, _)| total < *b) {
                best = Some((total, stats));
            }
        }
        let (total, s) = best.unwrap();
        let ms = |c| s.get(c) as f64 / 1e6;
        println!(
            "{name:<28} {:>10.3} {:>10.3} {:>10.3} {:>10.3}",
            ms(Counter::TimeStyleNs),
            ms(Counter::TimeLayoutNs),
            ms(Counter::TimePaintNs),
            total
        );
    }
}
