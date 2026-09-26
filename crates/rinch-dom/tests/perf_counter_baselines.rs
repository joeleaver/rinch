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
//! A viewport change of more than half a pixel re-runs layout, and restyles
//! whatever a media query or a viewport unit ties to the size — a second route
//! to most of these numbers; only the `resize_by_1px*` scenarios take it on
//! purpose.

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
    build_with(ROWS, contents_wrapper, "")
}

fn build_n(n_rows: usize, contents_wrapper: bool) -> Fixture {
    build_with(n_rows, contents_wrapper, "")
}

/// [`build`] with `n_rows` rows and `extra_css` loaded after the fixture's own
/// sheet.
fn build_with(n_rows: usize, contents_wrapper: bool, extra_css: &str) -> Fixture {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    if !extra_css.is_empty() {
        doc.load_css(extra_css);
    }
    let body = doc.body();
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::new();
    let mut texts = Vec::new();
    let mut chips = Vec::new();
    for i in 0..n_rows {
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
        .vw { width: 10vw; height: 4px; }
        .gen::before { content: \"*\"; }
        .inset { width: 20px; height: 20px; box-shadow: inset 0 0 4px rgb(0, 0, 0); }
        .hang { width: 1px; white-space: pre-wrap; }
        @media (min-width: 801px) { .mq { color: rgb(0, 0, 255); } }
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
    // A viewport-unit user, generated content, and a media-query dependant.
    let vw = doc.create_element("div");
    doc.set_attribute(vw, "class", "vw gen mq");
    doc.append_child(body, vw);
    // A blurred inset shadow (painted as a mask).
    let inset = doc.create_element("div");
    doc.set_attribute(inset, "class", "inset");
    doc.append_child(body, inset);
    // Preserved spaces at a soft wrap: at 1px every word overflows, the first
    // space after it hangs, and the second must be hung on the same line.
    let hang = doc.create_element("div");
    doc.set_attribute(hang, "class", "hang");
    let ht = doc.create_text("a  b");
    doc.append_child(hang, ht);
    doc.append_child(body, hang);

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
    // 800 -> 801 flips the `min-width: 801px` query: a full restyle.
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    // 801 -> 802 flips nothing: only the `vw` user restyles.
    doc.resolve_layout(VP.0 + 2.0, VP.1);
    // A structural change (scoped), then a bare whole-document request.
    let extra = doc.create_element("div");
    doc.append_child(flex, extra);
    doc.resolve_layout(VP.0 + 2.0, VP.1);
    doc.tree.ifc_dirty = true;
    doc.tree.layout_dirty = true;
    doc.resolve_layout(VP.0 + 2.0, VP.1);
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
        StyleInvalidations,
        ViewportUnitRestyles,
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
        IfcHangPasses,
        IfcHangLines,
        IfcPhantomRebreaks,
        LayoutResolves,
        LayoutSkippedPaintOnly,
        LayoutSkippedTextOnly,
        IfcSetupPasses,
        IfcFullPasses,
        IfcFullInitial,
        IfcFullTheme,
        IfcFullUnattributed,
        IfcScopedPasses,
        IfcScopeContainers,
        IfcScopeNodes,
        TaffyRootComputes,
        TaffyMeasureCalls,
        InlineBlockComputes,
        CalcFixpointPasses,
        PaintNodesVisited,
        StackingOrderBuilds,
        InsetShadowMaskPx,
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
/// on-screen node (the rows below the fold are dismissed by their parent's
/// loop without a visit, #910): the shell's dirty region, not rinch-dom, is
/// what makes an idle frame cheap.
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
            (PaintNodesVisited, 50),
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
/// **One element is cascaded** — the row — which is the minimum. Stylo's
/// invalidator (`style_invalidations`, one per snapshotted element) finds that
/// `.row:hover` reaches only the row itself; its new style changes only a reset
/// property, so its children are visited to check for an explicit `inherit`
/// (`style_nodes_visited`) and none is re-cascaded. The whole row subtree used
/// to be re-cascaded (3 elements).
///
/// No `::before` / `::after` pass: no stylesheet here has a rule for either,
/// so no element is matched against them (`pseudo_element_passes` was two per
/// cascaded element).
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
            (ElementsCascaded, 1),
            (StyleNodesVisited, 3),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The same hover, on rows that carry a `display: contents` wrapper — what
/// every `rsx!` reactive text produces. One element cascaded (it was 4: the
/// row, its span, its chip and the wrapper). Since #875 the wrapper's re-cascade no
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
            (ElementsCascaded, 1),
            (StyleNodesVisited, 4),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A class toggle that changes only `background-color` (`.row` ↔ `.row.sel`).
/// As for the hover, one element is cascaded: `.row.sel` depends on the class
/// only in its rightmost compound, so Stylo's invalidator restyles the row and
/// nothing under it (the subtree used to be re-cascaded, audit F3).
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
            (ElementsCascaded, 1),
            (StyleNodesVisited, 3),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// An attribute no selector reads (`data-foo` on a row): Stylo's invalidator
/// finds no dependency and **nothing is cascaded**. The node is still marked
/// dirty for paint (an attribute can reach paint without reaching style), so
/// the frame is otherwise a paint-only one. It re-cascaded the row's subtree.
#[test]
fn unselected_attribute_write() {
    let mut f = build(false);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| f.doc.set_attribute(row, "data-foo", "1"));
    expect(
        "unselected attribute",
        &s,
        &[
            (StyleResolves, 1),
            (StyleInvalidations, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A class on the list container that a descendant rule depends on
/// (`.list.dense .row`): every row is restyled — through Stylo's descendant
/// invalidation, not a subtree re-cascade — and the rows' own children are
/// visited but not re-cascaded, since only the rows' padding (a reset
/// property) moved.
#[test]
fn container_class_a_descendant_rule_depends_on() {
    let mut f = build_with(ROWS, false, ".list.dense .row { padding: 3px; }");
    let list = f.list;
    let s = frame(&mut f, |f| f.doc.set_attribute(list, "class", "list dense"));
    expect(
        "container class (descendant rule)",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 40),
            (StyleNodesVisited, 121),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 40),
            (TaffyStyleChanges, 40),
            (ShapeMeasureIfc, 40),
            (ShapeIfcBuild, 40),
            (IfcMeasureCacheHits, 120),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 160),
            (PaintNodesVisited, 48),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Append one row to the list. The structural pass is **scoped**
/// (`rinch_dom::ifc_scope`): it sets up the list and the new row
/// (`ifc_scope_containers` = 2, `ifc_scope_nodes` = the list, its 41 rows as
/// stop nodes, and the row's text) and nothing else. So:
///
/// - no other row's `display: contents` wrapper is re-spliced, so no row is
///   dirtied, and Taffy asks each old row **once** (`taffy_measure_calls` =
///   ROWS + the new row's 4; it was four per row, 164) — the same as
///   `append_one_row_without_wrappers`, which it now equals counter for
///   counter;
/// - no chip is re-sized (`inline_block_computes` 0, was ROWS = 40, and
///   `shape_atomic_inline` 0, was 40): the new row has none, and every other
///   chip's IFC is outside the scope;
/// - the new row is shaped **once** (`shape_measure_ifc` 1): its four measures
///   in the compute share one shape.
///
/// Still more than the minimum: the ROWS measure calls are Taffy's — the
/// column grew, so the available height every row's final layout is keyed
/// under changed, and a leaf's cache misses on it even when both its own
/// dimensions are known — each answered from rinch's cache.
///
/// The style side is now the minimum: the row is cascaded once and the walk
/// visits the row alone (`style_nodes_visited` 1; a text node carries no
/// style and is not visited). It was 287 — the
/// whole document — because the synchronous insertion restyle consumed its own
/// style root and left `styles_dirty` set, and `resolve_styles` read an empty
/// root list as "walk everything". It now walks everything only when a
/// whole-document restyle asked it to (`NodeTree::full_style_walk`).
/// `taffy_style_syncs` is 2, not 1: the new row was childless when its text
/// went in, so its Taffy style is re-synced to drop the empty-block line floor
/// (`note_first_child`).
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
            (StyleNodesVisited, 1),
            (TaffyStyleSyncs, 2),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 43),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 2),
            (IfcScopeNodes, 43),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 44),
            (PaintNodesVisited, 50),
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
/// No chip is sized at all (`inline_block_computes` 0, was 40 computes that
/// each hit Taffy's cache): the scoped pass sizes only the atomic inlines it
/// placed.
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
            (StyleNodesVisited, 1),
            (TaffyStyleSyncs, 2),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 43),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 2),
            (IfcScopeNodes, 43),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 44),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Remove one row: a scoped pass over the list alone (`ifc_scope_containers`
/// 1, `ifc_scope_nodes` = the list and its 39 remaining rows). Nothing is
/// shaped, no row's wrapper is re-spliced, so Taffy asks each row once
/// (`taffy_measure_calls` 39, was 156 = four per row), and no chip is sized
/// (`inline_block_computes` 0, was ROWS = 40 — the old scan walked the whole
/// slab, the removed row's detached chip included). The removed subtree is not
/// set up at all.
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
            (IfcMeasureCacheHits, 39),
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 1),
            (IfcScopeNodes, 40),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 39),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Change one text node's content: a text-only layout, no structural pass.
///
/// The row's IFC is re-measured — four calls for its four available spaces,
/// **one** shape (`shape_measure_ifc` 1, the rest `ifc_measure_cache_hits`):
/// the measure function used to bypass the cache for a dirty root on every
/// call, and now drops the root's old sizes once, at the start of the compute,
/// and reuses what it shapes itself. The calls were **0** before #878,
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
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 3),
            (IfcMeasureInvalidations, 3),
            (TaffyMeasureCalls, 4),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (PaintNodesVisited, 50),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// Resize the viewport by one pixel. No media query in the sheet changes its
/// answer and no rule uses a viewport unit, so **nothing is restyled**: the
/// cascade data is not rebuilt and no element is re-cascaded. Only layout
/// runs, and it re-measures every row at the new width.
///
/// It used to be a full-document restyle on every resize event (163 elements
/// cascaded, the stylist's cascade data rebuilt).
#[test]
fn resize_by_1px() {
    let mut f = build(true);
    f.doc.resolve_layout(VP.0 + 1.0, VP.1);
    let s = f.doc.tree.perf.end_frame();
    expect(
        "resize 1px",
        &s,
        &[
            (ShapeMeasureIfc, 40),
            (ShapeIfcBuild, 40),
            (IfcMeasureCacheHits, 120),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 160),
        ],
    );
}

/// The same resize with the chips sized in `vw`: exactly the 40 chips restyle
/// (`viewport_unit_restyles`, one cascade each), and nothing else does. Their new widths re-size them
/// (`inline_block_computes`).
#[test]
fn resize_by_1px_with_viewport_units() {
    let mut f = build_with(ROWS, true, ".chip { width: 5vw; }");
    f.doc.resolve_layout(VP.0 + 1.0, VP.1);
    let s = f.doc.tree.perf.end_frame();
    expect(
        "resize 1px (vw chips)",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 40),
            (StyleNodesVisited, 40),
            (ViewportUnitRestyles, 40),
            (TaffyStyleSyncs, 40),
            (TaffyStyleChanges, 40),
            (ShapeMeasureIfc, 40),
            (ShapeIfcBuild, 40),
            (ShapeAtomicInline, 40),
            (IfcMeasureCacheHits, 120),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 160),
            (InlineBlockComputes, 40),
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
            (PaintNodesVisited, 50),
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

/// Release wall-clock for the structural paths at scale — one append, one
/// removal and one `if`-style toggle (a row's chip removed, then put back) on
/// 500 and 2000 rows, with and without a `display: contents` wrapper per row —
/// with the counters that say why. Best of `REPS`; nothing asserted.
///
/// ```text
/// cargo test --release -p rinch-dom --test perf_counter_baselines structural_timings_at_scale -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn structural_timings_at_scale() {
    const REPS: usize = 15;
    type Op = fn(&mut Fixture, usize);
    let ops: [(&str, Op); 3] = [
        ("append row", |f, _| {
            let row = f.doc.create_element("div");
            f.doc.set_attribute(row, "class", "row");
            let t = f.doc.create_text("new row");
            f.doc.append_child(row, t);
            let list = f.list;
            f.doc.append_child(list, row);
        }),
        ("remove row", |f, r| {
            let row = f.rows[f.rows.len() / 2 + r];
            let list = f.list;
            f.doc.remove_child(list, row);
        }),
        ("toggle chip", |f, r| {
            let row = f.rows[f.rows.len() / 2];
            let chip = f.chips[f.rows.len() / 2];
            if r % 2 == 0 {
                f.doc.remove_child(row, chip);
            } else {
                f.doc.append_child(row, chip);
            }
        }),
    ];
    println!(
        "{:<12} {:>6} {:>8} {:>10} {:>10} {:>7} {:>7} {:>7} {:>7} {:>8} {:>7} {:>6}",
        "op",
        "rows",
        "wrapper",
        "layout ms",
        "total ms",
        "ifc ms",
        "taffy",
        "build",
        "ib_cmp",
        "measure",
        "shapes",
        "scope"
    );
    for rows in [500usize, 2000] {
        for wrapper in [false, true] {
            for (name, op) in ops {
                let mut f = build_n(rows, wrapper);
                let mut best: Option<(f64, FrameStats)> = None;
                for r in 0..REPS {
                    let t = std::time::Instant::now();
                    op(&mut f, r);
                    f.doc.resolve_layout(VP.0, VP.1);
                    let total = t.elapsed().as_secs_f64() * 1000.0;
                    let stats = f.doc.tree.perf.end_frame();
                    if best.as_ref().is_none_or(|(b, _)| total < *b) {
                        best = Some((total, stats));
                    }
                }
                let (total, s) = best.unwrap();
                println!(
                    "{name:<12} {rows:>6} {wrapper:>8} {:>10.3} {total:>10.3} {:>7.3} {:>7.3} {:>7.3} {:>7} {:>8} {:>7} {:>6}",
                    s.get(Counter::TimeLayoutNs) as f64 / 1e6,
                    s.get(Counter::TimeIfcSetupNs) as f64 / 1e6,
                    s.get(Counter::TimeTaffyComputeNs) as f64 / 1e6,
                    s.get(Counter::TimeBuildIfcNs) as f64 / 1e6,
                    s.get(Counter::InlineBlockComputes),
                    s.get(Counter::TaffyMeasureCalls),
                    s.get(Counter::ShapeMeasureIfc) + s.get(Counter::ShapeAtomicInline),
                    s.get(Counter::IfcScopeNodes),
                );
            }
        }
    }
}
