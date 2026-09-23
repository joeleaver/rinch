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
//! # What the assertions mean
//!
//! **Counters, not timings**, because counters are deterministic: the same
//! operation on the same document takes the same number of cascades, Parley
//! shapes, IFC passes and Taffy computes on every host and under any load.
//!
//! Two kinds of assertion, and a fix is expected to change the second kind:
//!
//! - `exact` pins behaviour that is **already right** — a colour-only hover
//!   runs no Taffy compute and no IFC setup pass. A regression fails it.
//! - `ceiling` records **today's** cost as an upper bound. It does not fail
//!   when a fix makes the path cheaper; the PR that makes it cheaper should
//!   lower the ceiling to the new number, so the win cannot silently regress
//!   afterwards. Every ceiling here was the measured value when it was
//!   written, not a round number.
//!
//! `cargo test -p rinch-dom --test perf_counter_baselines -- --nocapture`
//! prints every scenario's non-zero counters. The `#[ignore]`d
//! `perf_scenario_timings` prints wall-clock timings for the same scenarios;
//! run it in release:
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

fn report(name: &str, stats: &FrameStats) {
    let mut line = String::new();
    for (c, v) in stats.iter() {
        if v != 0 && !c.name().starts_with("time_") {
            line.push_str(&format!(" {}={}", c.name(), v));
        }
    }
    println!("{name:<28}{line}");
}

#[track_caller]
fn exact(stats: &FrameStats, counter: Counter, expected: u64, why: &str) {
    assert_eq!(stats.get(counter), expected, "{}: {why}", counter.name());
}

#[track_caller]
fn ceiling(stats: &FrameStats, counter: Counter, max: u64) {
    let v = stats.get(counter);
    assert!(
        v <= max,
        "{} = {v}, above its recorded baseline of {max}: this path got more \
         expensive. If that is intended, raise the ceiling and say why.",
        counter.name()
    );
}

fn hover(f: &mut Fixture, row: Option<NodeId>) {
    let mut changed = false;
    f.doc.update_hover(row.map(|r| r.0), &mut changed);
}

// ── Scenarios ──────────────────────────────────────────────────────────────

/// Positive control for every fixture below: the instrument fires. Without
/// this, an all-zero frame would read as "free" when it means "not counted".
#[test]
fn the_first_layout_counts_everything() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let p = doc.create_element("p");
    let t = doc.create_text("hello");
    doc.append_child(p, t);
    doc.append_child(body, p);
    doc.resolve_layout(VP.0, VP.1);
    let stats = doc.tree.perf.end_frame();
    report("first_layout", &stats);
    for c in [
        Counter::StyleResolves,
        Counter::ElementsCascaded,
        Counter::LayoutResolves,
        Counter::IfcSetupPasses,
        Counter::TaffyRootComputes,
        Counter::TaffyMeasureCalls,
        Counter::ShapeIfcBuild,
    ] {
        assert!(
            stats.get(c) > 0,
            "{} did not fire on a first layout",
            c.name()
        );
    }
    assert!(stats.get(Counter::TimeLayoutNs) > 0);
    assert_eq!(
        stats.get(Counter::TaffyRootComputes),
        doc.tree.taffy_computes,
        "taffy_root_computes mirrors NodeTree::taffy_computes"
    );
}

/// Nothing dirty: the layout early-returns. Already right, so exact.
#[test]
fn idle_frame() {
    let mut f = build(true);
    let s = frame(&mut f, |_| {});
    report("idle", &s);
    exact(&s, Counter::ElementsCascaded, 0, "nothing was invalidated");
    exact(&s, Counter::TaffyRootComputes, 0, "nothing is layout-dirty");
    exact(&s, Counter::IfcSetupPasses, 0, "nothing changed structure");
    exact(&s, Counter::ShapeIfcBuild, 0, "no text changed");
    exact(
        &s,
        Counter::LayoutSkippedPaintOnly,
        1,
        "the cheap early return",
    );
    // A full paint still walks every on-screen node (the rows below the fold
    // are culled): the shell's dirty region, not rinch-dom, is what makes an
    // idle frame cheap.
    ceiling(&s, Counter::PaintNodesVisited, 66);
}

/// `.row:hover { background-color }` on a row with no `display: contents`
/// inside: a paint-only restyle.
#[test]
fn hover_colour_only_without_wrapper() {
    let mut f = build(false);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| hover(f, Some(row)));
    report("hover (no wrapper)", &s);
    exact(
        &s,
        Counter::TaffyRootComputes,
        0,
        "a colour hover must not lay out",
    );
    exact(
        &s,
        Counter::IfcSetupPasses,
        0,
        "a colour hover changes no structure",
    );
    // The whole row subtree is re-cascaded, not just the row (audit F3):
    // row + span + chip today. The minimum is 1.
    ceiling(&s, Counter::ElementsCascaded, 3);
    // …and its text is re-shaped though no typography changed (audit F3 /
    // update-path F1.1): the row's IFC and the chip's. The minimum is 0.
    ceiling(&s, Counter::ShapeIfcBuild, 2);
    // One O(cache) retain scan per invalidated descendant (audit F10).
    ceiling(&s, Counter::IfcMeasureCacheRetains, 4);
}

/// The same hover, on rows that carry a `display: contents` wrapper — what
/// every `rsx!` reactive text produces.
///
/// **Expected to drop once the display:contents restyle fix (#875) lands**:
/// the wrapper's re-cascade currently rewrites its Taffy style
/// (`Display::Flex` from `to_taffy_style` against the spliced
/// `Display::None`), which sets `ifc_dirty` and `layout_dirty`, so one colour
/// hover pays a whole-document IFC setup pass, a root compute and a re-shape
/// of every row. After the fix these should read like the no-wrapper
/// scenario above: `ifc_setup_passes` 0, `taffy_root_computes` 0, and the
/// shape counts a handful. Lower the ceilings then.
#[test]
fn hover_colour_only_with_contents_wrapper() {
    let mut f = build(true);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| hover(f, Some(row)));
    report("hover (contents wrapper)", &s);
    ceiling(&s, Counter::IfcSetupPasses, 1);
    ceiling(&s, Counter::TaffyRootComputes, 1);
    ceiling(&s, Counter::IfcMeasureCacheClears, 1);
    ceiling(&s, Counter::ElementsCascaded, 4);
    // One per row (ROWS = 40): the measure cache was cleared wholesale, so
    // every row's IFC is re-shaped to be measured…
    ceiling(&s, Counter::ShapeMeasureIfc, 40);
    // …and every chip re-sized by its own standalone compute.
    ceiling(&s, Counter::InlineBlockComputes, 40);
    ceiling(&s, Counter::ShapeAtomicInline, 40);
    ceiling(&s, Counter::ShapeIfcBuild, 2);
}

/// A class toggle that changes only `background-color` (`.row` ↔ `.row.sel`).
#[test]
fn colour_only_class_toggle() {
    let mut f = build(false);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| f.doc.set_attribute(row, "class", "row sel"));
    report("class toggle (colour)", &s);
    exact(
        &s,
        Counter::TaffyRootComputes,
        0,
        "a colour change must not lay out",
    );
    exact(
        &s,
        Counter::IfcSetupPasses,
        0,
        "a colour change is not structural",
    );
    // As for the hover: an attribute write re-cascades the whole subtree and
    // re-shapes its text (audit F3).
    ceiling(&s, Counter::ElementsCascaded, 3);
    ceiling(&s, Counter::ShapeIfcBuild, 2);
    ceiling(&s, Counter::IfcMeasureCacheRetains, 6);
}

/// Append one row to the list. Today a structural change anywhere re-runs
/// the whole-document IFC setup pass and clears the whole measure cache, so
/// every row is re-shaped (audit layout F1): the shape counts scale with
/// `ROWS`, not with the one row added.
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
    report("append one row", &s);
    exact(&s, Counter::IfcSetupPasses, 1, "an insertion is structural");
    ceiling(&s, Counter::IfcMeasureCacheClears, 1);
    ceiling(&s, Counter::TaffyRootComputes, 1);
    // ROWS + 1: every row re-shaped to measure one new one.
    ceiling(&s, Counter::ShapeMeasureIfc, 41);
    ceiling(&s, Counter::ShapeIfcBuild, 1);
    // Every chip in the document re-sized (audit F11).
    ceiling(&s, Counter::InlineBlockComputes, 40);
    ceiling(&s, Counter::ShapeAtomicInline, 40);
    // Appending the text into the still-detached row restyles synchronously
    // with no style root recorded, which walks the *whole document* (287
    // nodes) to cascade nothing: `resolve_styles`' empty-roots fallback.
    ceiling(&s, Counter::FullStyleWalks, 1);
    ceiling(&s, Counter::StyleNodesVisited, 287);
}

/// Remove one row from the list: the same whole-document pass as an append.
#[test]
fn remove_one_row() {
    let mut f = build(true);
    let row = f.rows[ROWS / 2];
    let s = frame(&mut f, |f| {
        let list = f.list;
        f.doc.remove_child(list, row);
    });
    report("remove one row", &s);
    exact(&s, Counter::IfcSetupPasses, 1, "a removal is structural");
    ceiling(&s, Counter::TaffyRootComputes, 1);
    // Every remaining row re-shaped, and every chip — the removed row's
    // included — re-sized.
    ceiling(&s, Counter::ShapeMeasureIfc, 39);
    ceiling(&s, Counter::InlineBlockComputes, 40);
    ceiling(&s, Counter::ShapeAtomicInline, 40);
    ceiling(&s, Counter::ShapeIfcBuild, 0);
}

/// Change one text node's content: a text-only layout, no structural pass.
#[test]
fn set_text_on_one_row() {
    let mut f = build(true);
    let t = f.texts[ROWS / 2];
    let s = frame(&mut f, |f| f.doc.set_text_content(t, " (changed text)"));
    report("set_text_content", &s);
    exact(
        &s,
        Counter::IfcSetupPasses,
        0,
        "a text edit is not structural",
    );
    ceiling(&s, Counter::TaffyRootComputes, 1);
    ceiling(&s, Counter::ShapeIfcBuild, 1);
    ceiling(&s, Counter::IfcMeasureCacheRetains, 3);
    // Worth a second look (layout audit §7): the compute runs but calls the
    // measure function **zero** times, so the row's IFC root — whose text is
    // inside an inline wrapper, detached from the Taffy tree — is not
    // re-measured. Cheap, and possibly a missed height change rather than a
    // saving. Recorded, not endorsed.
    ceiling(&s, Counter::TaffyMeasureCalls, 0);
}

/// Resize the viewport by one pixel: a full-document restyle (the stylist
/// rebuilds its device) plus a re-measure of everything (audit layout F4).
#[test]
fn resize_by_1px() {
    let mut f = build(true);
    f.doc.resolve_layout(VP.0 + 1.0, VP.1);
    let s = f.doc.tree.perf.end_frame();
    report("resize 1px", &s);
    exact(
        &s,
        Counter::FullRestyleViewport,
        1,
        "a viewport change restyles all",
    );
    exact(&s, Counter::FullRestyles, 1, "…and nothing else did");
    // Every element in the document, each with two pseudo-element passes.
    ceiling(&s, Counter::ElementsCascaded, 163);
    ceiling(&s, Counter::TaffyStyleSyncs, 163);
    // One per row: the wrapper's spliced `Display::None` against the
    // re-cascaded `Display::Flex` (the #875 mechanism). Expected to drop to 0
    // with that fix, which also takes `ifc_setup_passes` here to 0.
    ceiling(&s, Counter::TaffyStyleChanges, 40);
    ceiling(&s, Counter::IfcSetupPasses, 1);
    ceiling(&s, Counter::TaffyRootComputes, 1);
    ceiling(&s, Counter::ShapeMeasureIfc, 40);
    ceiling(&s, Counter::ShapeIfcBuild, 40);
    ceiling(&s, Counter::ShapeAtomicInline, 40);
}

/// One tick of a `transform` animation on one chip: transform moves no box,
/// so nothing should lay out.
#[test]
fn transform_animation_tick() {
    let mut f = build(true);
    let chip = f.chips[ROWS / 2];
    f.doc.set_attribute(chip, "class", "chip spin");
    f.doc.resolve_layout(VP.0, VP.1);
    paint(&mut f);
    f.doc.tree.perf.reset();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let s = frame(&mut f, |f| {
        f.doc.tick_animations();
    });
    report("transform animation tick", &s);
    exact(
        &s,
        Counter::IfcSetupPasses,
        0,
        "a transform tick is not structural",
    );
    exact(
        &s,
        Counter::ElementsCascaded,
        0,
        "a tick writes computed style directly",
    );
    // A transform moves no box, yet the tick costs a root Taffy compute and
    // re-shapes the chip's text to measure it. The tick re-syncs the node's
    // Taffy style on a `DirtyFlags::LAYOUT` that is never cleared (layout
    // audit F9). The minimum is 0 for both.
    ceiling(&s, Counter::TaffyRootComputes, 1);
    ceiling(&s, Counter::ShapeMeasureIfc, 1);
    // The spinning chip is a stacking context of its own: body + chip.
    ceiling(&s, Counter::StackingOrderBuilds, 2);
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
