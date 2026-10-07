//! Review fixtures for PR #1434 (issue #631: a positioned inline span is the
//! containing block of an absolute box inside it).
//!
//! Every rect is Chrome 153's (`--headless=new`, standards mode, `* {
//! box-sizing: border-box }`, 800x600, the bundled Inter through
//! `@font-face`), relative to the container's border box, in the scaffold of
//! `abs_inline_containing_block_tests.rs`.
//!
//! The tests that pass at the PR head pin something no fixture of the PR
//! does (each names the mutant it kills). The `#[ignore]`d ones are findings:
//! they fail at the PR head, and say what Chrome gives.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";
const REL: &str = "position: relative";

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    doc
}

fn one(doc: &RinchDocument, m: &str) -> usize {
    let found = query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "[data-m={m}] names exactly one node");
    found[0]
}

fn build(c_extra: &str, inner: &str) -> RinchDocument {
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(
        wrap,
        &format!(r#"<div data-m="c" style="{C}{c_extra}">{inner}</div>"#),
    );
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
    doc
}

/// `m`'s box relative to the container's border box.
fn rect(doc: &RinchDocument, m: &str) -> [f32; 4] {
    let (id, c) = (one(doc, m), one(doc, "c"));
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
    let (cx, cy) = rinch_dom::paint::compute_absolute_position(&doc.tree, c, 1.0);
    let l = doc.tree.get(id).unwrap().layout;
    [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
}

#[track_caller]
fn assert_rect(got: [f32; 4], want: [f32; 4], what: &str) {
    let close = got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.55);
    assert!(close, "{what}: got {got:?}, Chrome 153 gives {want:?}");
}

fn abs(style: &str) -> String {
    format!(r#"<div data-m="abs" style="position: absolute; {style}"></div>"#)
}

/// One relayout's `(abs_boxes_visited, taffy_root_computes)`.
fn frame(doc: &mut RinchDocument, height: f32) -> (u64, u64) {
    doc.tree.perf.end_frame();
    doc.resolve_layout(800.0, height);
    let f = doc.tree.perf.frame();
    (
        f.get(Counter::AbsBoxesVisited),
        f.get(Counter::TaffyRootComputes),
    )
}

// ── Pins the PR lacks (pass at the PR head) ─────────────────────────────────

/// An empty span, and a span holding only an atomic inline, stand in the
/// **font's** box, not the line box. Every such fixture of the PR has a 20px
/// line, which is Inter's 16px ascent + descent: the line box and the font
/// box coincide there. At `line-height: 30px` they do not (font box 12..32).
///
/// Kills: `line_extent` answering the line's `block_min_coord..block_max_coord`
/// always (survives the PR's suite).
#[test]
fn an_empty_span_is_as_tall_as_the_font_not_the_line() {
    let doc = build(
        "line-height: 30px;",
        &format!(r#"lead <span style="{REL}">{}</span>tail"#, abs("inset: 0")),
    );
    assert_rect(rect(&doc, "abs"), [47.64, 12.0, 0.0, 20.0], "empty span");

    let doc = build(
        "line-height: 30px;",
        &format!(
            r#"lead <span style="{REL}"><span style="display: inline-block; width: 30px; height: 8px"></span>{}</span>tail"#,
            abs("inset: 0")
        ),
    );
    assert_rect(
        rect(&doc, "abs"),
        [47.64, 12.0, 30.0, 20.0],
        "span of one inline-block",
    );
}

/// `NodeTree::abs_inline_cb_seen` goes back to `false` with the last
/// span-hung box, and the document then pays what one that never held such a
/// box pays.
///
/// Kills: `begin_read` no longer clearing the flag (survives the PR's suite:
/// the second placement then runs over every placed box in every layout, for
/// good).
#[test]
fn a_document_that_lost_its_last_span_hung_box_pays_what_one_without_pays() {
    let mk = |span_style: &str| {
        build(
            "",
            &format!(
                r#"<div style="position: relative"><div><div style="position: absolute; left: 0; top: 0; width: 4px; height: 4px"></div></div></div>lead <span data-m="s" style="{span_style}">text{}tail</span>"#,
                abs("left: 0; top: 0; width: 4px; height: 4px")
            ),
        )
    };
    let mut never = mk("");
    let baseline = frame(&mut never, 640.0);

    let mut had = mk(REL);
    let with = frame(&mut had, 640.0);
    assert!(
        with.0 > baseline.0,
        "positive control: a span-hung box costs visits ({with:?} vs {baseline:?})"
    );
    let s = NodeId(one(&had, "s"));
    had.set_attribute(s, "style", "");
    had.resolve_layout(800.0, 600.0);
    assert_eq!(frame(&mut had, 680.0), baseline, "span static again");

    let mut removed = mk(REL);
    let a = NodeId(one(&removed, "abs"));
    removed.remove_node(a);
    removed.resolve_layout(800.0, 600.0);
    // One ancestor-resolved box is left, as in `never` less its host-resolved one.
    assert_eq!(frame(&mut removed, 680.0), baseline, "box removed");
}

/// A text-only pass (`text-align`) in a document with an ancestor-resolved
/// box and **no** span-hung one places nothing again.
///
/// Kills: the text-only path setting `abs_late_moves` unconditionally
/// (survives the PR's suite).
#[test]
fn a_text_only_pass_with_no_span_hung_box_places_nothing() {
    let mut doc = build(
        "",
        r#"<div style="position: relative"><div><div style="position: absolute; left: 0; top: 0; width: 4px; height: 4px"></div></div></div><div data-m="l">lead text</div>"#,
    );
    let l = NodeId(one(&doc, "l"));
    doc.tree.perf.end_frame();
    doc.set_attribute(l, "style", "text-align: right");
    doc.resolve_layout(800.0, 600.0);
    let f = doc.tree.perf.frame();
    assert_eq!(
        (
            f.get(Counter::TaffyRootComputes),
            f.get(Counter::AbsBoxesVisited)
        ),
        (0, 0),
        "the text-only path, and no box placed again"
    );
}

/// A box **sized** from a wrapped span follows a `text-align` change in the
/// same layout: right-aligned, the last fragment ends right of the first's
/// start and the block is 27.67 wide, where left-aligned it was 0.
#[test]
fn a_text_align_change_resizes_a_box_sized_from_a_wrapped_span() {
    let mut doc = build(
        "width: 120px;",
        &format!(
            r#"<div data-m="l">lead <span style="{REL}">one two three four five six{}</span> end</div>"#,
            abs("inset: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [47.64, 7.0, 0.0, 60.0], "left");
    let l = NodeId(one(&doc, "l"));
    doc.set_attribute(l, "style", "text-align: right");
    doc.resolve_layout(800.0, 600.0);
    // Chrome: `[59.25, 7, 27.67, 60]`. rinch right-aligns a line in a child
    // block one pixel further right than Chrome with or without the box
    // (plain text there starts at 302.98 where Chrome's starts at 301.98), so
    // only the size and the row are asserted to the half pixel.
    let got = rect(&doc, "abs");
    assert!((got[0] - 59.25).abs() < 1.6, "x: {got:?}");
    assert_rect(
        [59.25, got[1], got[2], got[3]],
        [59.25, 7.0, 27.67, 60.0],
        "right",
    );
}

// ── Findings (fail at the PR head) ──────────────────────────────────────────

/// F2. The fragment is the **span's own** font box. `inline_fragments_box`
/// takes the ascent of the first glyph run inside the span and the descent
/// of the last, which are a *descendant's* runs when the span's content
/// starts or ends with a child set in another size.
///
/// rinch at the PR head: `[48, -3, 47, 39]` (the 32px child's box).
#[test]
#[ignore = "review of #1434, F2: a span's fragment takes its children's font"]
fn the_fragment_is_the_spans_font_box_whatever_its_children_are_set_in() {
    let doc = build(
        "",
        &format!(
            r#"lead <span style="{REL}"><b style="font-size: 32px; font-weight: normal">big</b>{}</span> tail"#,
            abs("inset: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [47.64, 12.0, 46.97, 20.0], "32px child");

    // A 10px span around 16px text: Chrome 12px tall at 13; rinch [48, 7, 49, 20].
    let doc = build(
        "",
        &format!(
            r#"lead <span style="{REL}; font-size: 10px"><b style="font-size: 16px; font-weight: normal">bigger</b>{}</span> tail"#,
            abs("inset: 0")
        ),
    );
    assert_rect(
        rect(&doc, "abs"),
        [47.64, 13.0, 48.66, 12.0],
        "10px span, 16px child",
    );
}

/// F4. In a block that draws a `text-overflow: ellipsis` "…", the lines are
/// rebuilt as flat text with no member entries, the span cannot be found in
/// them, and the box falls back to the block container (the pre-#631
/// answer). rinch at the PR head: `[0, 0, 40, 30]`.
#[test]
#[ignore = "review of #1434, F4: a span in an ellipsized block is not measured"]
fn a_span_in_an_ellipsized_line_is_still_the_containing_block() {
    let doc = build(
        "width: 120px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        &format!(
            r#"lead <span style="{REL}">text{}tail and more and more and more</span>"#,
            abs("width: 40px; height: 30px; top: 0; left: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [47.64, 7.0, 40.0, 30.0], "ellipsized");
}

/// F5. `vertical-align` shifts a span's glyphs at paint (#724) and its
/// fragment box does not follow: the box hangs from where the text would be
/// unshifted. Chrome: a `<sub>`'s fragment starts at 14.19; rinch at the PR
/// head: 10.
#[test]
#[ignore = "review of #1434, F5: the fragment ignores the vertical-align shift"]
fn a_sub_spans_fragment_is_where_its_text_is_drawn() {
    let doc = build(
        "",
        &format!(
            r#"lead <sub style="{REL}">text{}tail</sub>"#,
            abs("inset: 0")
        ),
    );
    assert_rect(rect(&doc, "abs"), [47.64, 14.19, 41.98, 16.0], "sub");
}

/// F3 (not introduced by #1434; at `51cb7c71` the same box is drawn at
/// x = 11). A box hung from a span is clipped by the clipping boxes between
/// the span and the next positioned ancestor — they are in its
/// containing-block chain. The paint sequence does not see the span
/// (`stacking::Collector` descends the box tree, where the hoisted box is the
/// host's child), truncates the box's clip chain at the container, and draws
/// the box outside the scroller. Chrome: `elementFromPoint(60, 110)` is
/// `BODY`, the box is not drawn.
#[test]
#[ignore = "review of #1434, F3: a span-hung box escapes a static scroller's clip"]
fn a_span_hung_box_is_clipped_by_a_static_scroller_around_the_span() {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    let mut doc = build(
        "margin: 0;",
        &format!(
            r#"<div style="overflow: auto; height: 60px"><div style="height: 100px"></div><div>lead <span style="{REL}">text{}tail</span></div><div style="height: 300px"></div></div>"#,
            abs("left: 0; top: 0; width: 40px; height: 10px; background: rgb(255, 0, 0)")
        ),
    );
    // The scroller is y 7..67; the span's line is at 107, scrolled out.
    assert_rect(rect(&doc, "abs"), [47.64, 107.0, 40.0, 10.0], "layout");
    let mut painter = TinySkiaPainter::new(400, 200);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let i = ((110 * painter.width() + 60) * 4) as usize;
    assert_eq!(
        &painter.pixels()[i..i + 4],
        &[0, 0, 0, 0],
        "nothing is drawn below the scroller"
    );
}

/// F1b. A span of right-to-left text (parley reorders it whatever
/// `direction` says). Its range starts at the visual **right**, so
/// `line_range_x` answers `left > right`, `inline_fragments_box` skips the
/// line as if the span were not on it, and the span is treated as empty: a
/// zero-width block at the caret. rinch at the PR head: `[126, 7, 0, 19]`
/// (the word is drawn across 47.64..126). Chrome 153: `[47.64, 7, 60.5, 20]`
/// in its fallback face; the left edge follows `lead ` (Inter) in any face,
/// and the width is the word's.
#[test]
#[ignore = "review of #1434, F1b: a span of right-to-left text is measured as empty"]
fn a_span_of_right_to_left_text_is_not_empty() {
    let doc = build(
        "",
        &format!(
            r#"lead <span style="{REL}">שלום עולם{}</span> tail"#,
            abs("inset: 0")
        ),
    );
    let got = rect(&doc, "abs");
    assert!(
        (got[0] - 47.64).abs() <= 0.55 && got[2] > 30.0,
        "the block covers the word: got {got:?}, Chrome 153 gives [47.64, 7, 60.5, 20]"
    );
}
