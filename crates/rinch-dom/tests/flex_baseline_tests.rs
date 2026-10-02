//! `align-items: baseline` lines items up by their **first baselines**, in a flex
//! row and in a grid row alike (css-flexbox-1 §8.3, css-align-3 §9) — #1013.
//!
//! Taffy computes the alignment; it can only use a baseline the leaf reports.
//! Until #1013 every rinch measure returned `compute_leaf_layout`'s output, whose
//! `baselines` are `Baselines::NONE`, so every text leaf's and IFC root's
//! baseline was synthesized from its bottom border edge and `baseline` meant
//! "align bottom edges". A marker beside a three-line paragraph sat on the
//! paragraph's last line; two one-line items of different font sizes lined up
//! their bottoms, so a small item with a tall line box rode high.
//!
//! Every expected number is Chrome 153's, measured with the same markup and the
//! bundled Inter (registered under an override name so the host's fonts cannot
//! answer) at declared font sizes and line heights. Chrome rounds a font's
//! ascent and descent to whole pixels and rinch does not, so an item placed by
//! a baseline can land half a pixel apart before Taffy rounds the box; the
//! fixtures allow one pixel of that, which is far below every bottom-edge
//! answer they rule out (each differs from Chrome by 4px or more).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 600.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const BASE: &str = "font-family: ProbeFace; font-size: 16px; line-height: 25px";

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn y(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.y
}

fn container(d: &mut RinchDocument, style: &str) -> NodeId {
    let body = d.body();
    el(d, body, "div", &format!("{BASE}; width: 300px; {style}"))
}

#[track_caller]
fn near(got: f32, chrome: f32, what: &str) {
    assert!(
        (got - chrome).abs() <= 1.0,
        "{what}: y = {got}, Chrome 153 = {chrome}"
    );
}

/// The issue's case. Chrome: marker `0,0 13.5x25`, paragraph `19.5,0
/// 180.5x75`. Bottom-edge alignment put the marker at `y = 50`.
#[test]
fn a_marker_sits_on_a_multi_line_paragraphs_first_line() {
    let mut d = doc();
    let c = container(
        &mut d,
        "display: flex; align-items: baseline; gap: 6px; width: 200px; white-space: pre-wrap",
    );
    let marker = el(&mut d, c, "span", "");
    text(&mut d, marker, "\u{2022} ");
    let p = el(&mut d, c, "p", "margin: 0; flex: 1 1 0");
    text(
        &mut d,
        p,
        "a long list item that wraps onto more than one line here",
    );
    d.resolve_layout(VW, VH);
    assert_eq!(d.tree.get(p.0).unwrap().layout.height, 75.0, "three lines");
    near(y(&d, p), 0.0, "paragraph");
    near(y(&d, marker), 0.0, "marker");
}

/// Two one-line IFC roots of different sizes: a 32px face in a 40px line and a
/// 12px face in a 30px line. Chrome puts the small one at `y = 12`; bottom
/// edges would put it at 10. (Its line box is taller than its glyphs, so
/// "bottom" and "baseline" are not the same place even on one line.)
#[test]
fn one_line_items_of_different_sizes_align_their_baselines() {
    let mut d = doc();
    let c = container(&mut d, "display: flex; align-items: baseline");
    let a = el(&mut d, c, "span", "font-size: 32px; line-height: 40px");
    text(&mut d, a, "Big");
    let b = el(&mut d, c, "span", "font-size: 12px; line-height: 30px");
    text(&mut d, b, "small");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "big");
    near(y(&d, b), 12.0, "small");
}

/// The same in a grid row. Chrome: `0` and `12`.
#[test]
fn a_grid_row_aligns_first_baselines_too() {
    let mut d = doc();
    let c = container(
        &mut d,
        "display: grid; grid-template-columns: auto auto; align-items: baseline; \
         justify-content: start",
    );
    let a = el(&mut d, c, "span", "font-size: 32px; line-height: 40px");
    text(&mut d, a, "Big");
    let b = el(&mut d, c, "span", "font-size: 12px; line-height: 30px");
    text(&mut d, b, "small");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "big");
    near(y(&d, b), 12.0, "small");
}

/// A flex container's own text is an anonymous item — in rinch a text leaf
/// measured through `NodeContext::Text`, not an IFC. Chrome draws its glyphs
/// (a 20px content area) at `y = 15`, so its 25px line box starts at 12.5;
/// bottom edges would put it at 15.
#[test]
fn a_text_leaf_reports_its_first_baseline() {
    let mut d = doc();
    let c = container(&mut d, "display: flex; align-items: baseline");
    let t = text(&mut d, c, "Text");
    let b = el(&mut d, c, "span", "font-size: 32px; line-height: 40px");
    text(&mut d, b, "Big");
    d.resolve_layout(VW, VH);
    near(y(&d, b), 0.0, "big");
    near(y(&d, t), 12.5, "text leaf");
}

/// A baseline is measured from the border-box top, so an item's top padding
/// and border are part of it. Chrome: `Pad` (10px padding, 3px border) at 0,
/// 38 tall; `Mid` (24px in a 30px line) at 8 — which bottom edges also give,
/// so the case is repeated below with a bottom padding that moves the bottom
/// edge and not the baseline.
#[test]
fn the_baseline_includes_the_items_top_padding_and_border() {
    let mut d = doc();
    let c = container(&mut d, "display: flex; align-items: baseline");
    let a = el(
        &mut d,
        c,
        "span",
        "padding-top: 10px; border-top: 3px solid red",
    );
    text(&mut d, a, "Pad");
    let b = el(&mut d, c, "span", "font-size: 24px; line-height: 30px");
    text(&mut d, b, "Mid");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "padded");
    near(y(&d, b), 8.0, "mid");

    // Off the coincidence: a bottom padding moves the bottom edge and not the
    // baseline. Chrome: `Pad` at 0, `Mid` still at 8 (the padded item is now
    // 58 tall).
    let mut d = doc();
    let c = container(&mut d, "display: flex; align-items: baseline");
    let a = el(
        &mut d,
        c,
        "span",
        "padding-top: 10px; padding-bottom: 20px; border-top: 3px solid red",
    );
    text(&mut d, a, "Pad");
    let b = el(&mut d, c, "span", "font-size: 24px; line-height: 30px");
    text(&mut d, b, "Mid");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "padded, bottom padding");
    near(y(&d, b), 8.0, "mid, beside bottom padding");
}

/// A block item has the first baseline of its first in-flow child (Taffy's
/// block algorithm propagates it, offset by where the child sits). Chrome: the
/// `div` at 0 holding a 20px/30px paragraph 7px down (and 20px of bottom
/// margin, so the block's bottom edge is nowhere near its baseline); the 16px
/// `x` beside it at 11. Bottom edges put `x` at 32.
#[test]
fn a_block_item_takes_its_first_childs_baseline() {
    let mut d = doc();
    let c = container(&mut d, "display: flex; align-items: baseline");
    let a = el(&mut d, c, "div", "");
    let ap = el(
        &mut d,
        a,
        "p",
        "margin: 7px 0 20px; font-size: 20px; line-height: 30px",
    );
    text(&mut d, ap, "Block");
    let b = el(&mut d, c, "span", "");
    text(&mut d, b, "x");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "block item");
    near(y(&d, b), 11.0, "x");
}

/// A box with no line has no baseline and synthesizes one from its bottom
/// border edge, as before. Chrome: the 30px-tall empty `div` beside `Big`
/// (32px in a 40px line) at `y = 1`.
#[test]
fn an_item_with_no_line_still_aligns_its_bottom_edge() {
    let mut d = doc();
    let c = container(&mut d, "display: flex; align-items: baseline");
    let a = el(&mut d, c, "span", "font-size: 32px; line-height: 40px");
    text(&mut d, a, "Big");
    let b = el(&mut d, c, "div", "width: 20px; height: 30px");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "big");
    near(y(&d, b), 1.0, "empty box");
}

/// An `inline-flex` is sized by its own detached compute
/// (`compute_atomic_inline_root`), not the root one, and its measure reports
/// the baseline the same way. Chrome: `Big` at 0, `small` (12px in a 30px line,
/// 9px bottom padding) at 12; bottom edges put `small` at 1.
#[test]
fn an_inline_flex_aligns_its_own_items_by_baseline() {
    let mut d = doc();
    let c = container(&mut d, "");
    let f = el(
        &mut d,
        c,
        "span",
        "display: inline-flex; align-items: baseline",
    );
    let a = el(&mut d, f, "span", "font-size: 32px; line-height: 40px");
    text(&mut d, a, "Big");
    let b = el(
        &mut d,
        f,
        "span",
        "font-size: 12px; line-height: 30px; padding-bottom: 9px",
    );
    text(&mut d, b, "small");
    d.resolve_layout(VW, VH);
    near(y(&d, a), 0.0, "big");
    near(y(&d, b), 12.0, "small");
}
