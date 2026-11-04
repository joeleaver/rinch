//! #2146 and #2140 — an atomic inline's own percentage sizes, resolved
//! against its containing block.
//!
//! - **#2146**: a `width: calc(50% + 10px)` atomic inline was laid out at the
//!   calc's length part (10px): `resolve_percentage_inline_blocks` asked
//!   only whether a size was a plain `Percent`, and the box's detached
//!   compute has no Taffy parent for `resolve_layout_calcs` to take a basis
//!   from, so the percentage part was never resolved.
//! - **#2140**: `max-width: 100%` on an atomic inline whose content cannot
//!   get narrower than the containing block (a 400px box, or `nowrap` text)
//!   left the box at that content's width, where the cap wins: `max-width`
//!   overrides the shrink-to-fit width (CSS 2.1 §10.4), and the content
//!   overflows the capped box.
//!
//! Expected values are Chrome 156's (from the issues) and follow from the
//! spec: a percentage on an atomic inline's `width`/`min-width`/`max-width`
//! resolves against the containing block's inner width, which is a declared
//! length in every fixture here. No assertion is a glyph measurement: every
//! expected width is a declared length or a percentage of one; the text
//! fixtures only need their text to be wider than the block, which 60 words
//! at 16px are on any font set.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (800.0, 600.0);

const CSS: &str = "body { margin: 0; font-size: 16px; line-height: 20px }";

fn build(html_body: impl FnOnce(&mut RinchDocument, NodeId)) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    html_body(&mut doc, body);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
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

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn width(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.nodes[id.0].layout.width
}

#[track_caller]
fn assert_width(doc: &RinchDocument, id: NodeId, want: f32, what: &str) {
    let got = width(doc, id);
    assert!((got - want).abs() < 0.5, "{what}: width {got}, want {want}");
}

/// An inline-block sized `style` between two words in a `cb_width` block.
fn atomic_in_block(cb_width: &str, style: &str) -> (RinchDocument, NodeId) {
    let mut d = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", &format!("width:{cb_width}"));
        text(doc, cb, "before ");
        d = Some(el(doc, cb, "span", style));
        text(doc, cb, " after");
    });
    (doc, d.unwrap())
}

// ---- #2146 -------------------------------------------------------------

/// The issue's case: 50% of 360 + 10 = 190 (Chrome 156: 190).
#[test]
fn calc_width_resolves_its_percentage_2146() {
    let (doc, d) = atomic_in_block(
        "360px",
        "display:inline-block;width:calc(50% + 10px);height:5px",
    );
    assert_width(&doc, d, 190.0, "width: calc(50% + 10px) in 360px");
}

/// A negative length part, off the 50% fixed point: 100% of 360 − 20 = 340.
#[test]
fn calc_width_with_negative_length_2146() {
    let (doc, d) = atomic_in_block(
        "360px",
        "display:inline-block;width:calc(100% - 20px);height:5px",
    );
    assert_width(&doc, d, 340.0, "width: calc(100% - 20px) in 360px");
}

/// `inline-flex` takes the same path: 25% of 360 + 7 = 97.
#[test]
fn calc_width_on_inline_flex_2146() {
    let (doc, d) = atomic_in_block(
        "360px",
        "display:inline-flex;width:calc(25% + 7px);height:5px",
    );
    assert_width(&doc, d, 97.0, "inline-flex width: calc(25% + 7px)");
}

/// A `content-box` box with padding: the calc is its content width, so its
/// border box is 190 + 2·15 = 220.
#[test]
fn calc_width_content_box_with_padding_2146() {
    let (doc, d) = atomic_in_block(
        "360px",
        "display:inline-block;width:calc(50% + 10px);padding:0 15px;height:5px",
    );
    assert_width(&doc, d, 220.0, "content-box calc width + padding");
}

/// A calc `min-width` raises an empty box to 30% of 360 + 4 = 112.
#[test]
fn calc_min_width_2146() {
    let (doc, d) = atomic_in_block(
        "360px",
        "display:inline-block;min-width:calc(30% + 4px);height:5px",
    );
    assert_width(&doc, d, 112.0, "min-width: calc(30% + 4px)");
}

/// A calc `max-width` caps a 400px child's box at 50% of 360 − 6 = 174.
#[test]
fn calc_max_width_2146() {
    let mut d = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:360px");
        let a = el(
            doc,
            cb,
            "span",
            "display:inline-block;max-width:calc(50% - 6px)",
        );
        el(
            doc,
            a,
            "span",
            "display:inline-block;width:400px;height:10px",
        );
        d = Some(a);
    });
    assert_width(&doc, d.unwrap(), 174.0, "max-width: calc(50% - 6px)");
}

/// The containing block resized: the calc follows (50% of 500 + 10 = 260).
#[test]
fn calc_width_follows_a_resized_block_2146() {
    let mut ids = None;
    let mut doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:360px");
        text(doc, cb, "before ");
        let d = el(
            doc,
            cb,
            "span",
            "display:inline-block;width:calc(50% + 10px);height:5px",
        );
        ids = Some((cb, d));
    });
    let (cb, d) = ids.unwrap();
    assert_width(&doc, d, 190.0, "before the resize");
    doc.set_style(cb, "width", "500px");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    assert_width(&doc, d, 260.0, "after the resize to 500px");
}

// ---- #2140 -------------------------------------------------------------

/// The issue's three boxes in a 300px block (Chrome 156: 300, 300, 300).
#[test]
fn max_width_caps_below_min_content_2140() {
    let mut ids = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:300px");
        let a = el(doc, cb, "span", "display:inline-flex;max-width:100%");
        el(
            doc,
            a,
            "span",
            "display:inline-block;width:400px;height:10px",
        );
        text(doc, cb, " ");
        let b = el(
            doc,
            cb,
            "span",
            "display:inline-flex;max-width:100%;white-space:nowrap",
        );
        text(doc, b, &"word ".repeat(60));
        text(doc, cb, " ");
        let c = el(doc, cb, "span", "display:inline-block;max-width:100%");
        el(
            doc,
            c,
            "span",
            "display:inline-block;width:400px;height:10px",
        );
        ids = Some((a, b, c));
    });
    let (a, b, c) = ids.unwrap();
    assert_width(&doc, a, 300.0, "inline-flex around a 400px box");
    assert_width(&doc, b, 300.0, "inline-flex with nowrap text");
    assert_width(&doc, c, 300.0, "inline-block around a 400px box");
}

/// Off the 100% fixed point: `max-width: 60%` of 300 = 180.
#[test]
fn max_width_percent_below_min_content_2140() {
    let mut d = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:300px");
        let a = el(doc, cb, "span", "display:inline-block;max-width:60%");
        el(
            doc,
            a,
            "span",
            "display:inline-block;width:400px;height:10px",
        );
        d = Some(a);
    });
    assert_width(&doc, d.unwrap(), 180.0, "max-width: 60% around a 400px box");
}

/// The issue's one-word pill in a 10px block: capped at 10.
#[test]
fn one_word_pill_in_a_narrow_block_2140() {
    let mut d = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:10px");
        let p = el(
            doc,
            cb,
            "span",
            "display:inline-flex;max-width:100%;white-space:nowrap;overflow:hidden",
        );
        text(doc, p, "pill");
        d = Some(p);
    });
    assert_width(&doc, d.unwrap(), 10.0, "one-word pill in 10px");
}

/// The capped box keeps its content at the content's own size: the 400px
/// child overflows, it is not squeezed (Chrome: 400).
#[test]
fn capped_box_content_overflows_2140() {
    let mut ids = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:300px");
        let a = el(doc, cb, "span", "display:inline-block;max-width:100%");
        let inner = el(
            doc,
            a,
            "span",
            "display:inline-block;width:400px;height:10px",
        );
        ids = Some((a, inner));
    });
    let (a, inner) = ids.unwrap();
    assert_width(&doc, a, 300.0, "the capped box");
    assert_width(&doc, inner, 400.0, "the 400px child");
}

/// A `calc()` width inside an `auto` inline-block is cyclic, so it counts as
/// `auto` toward the outer box's width (css-sizing-3 §5.2.1), as a plain
/// percentage does. The outer box takes its 100px content, and the inner box
/// is then 50% of 100 + 10 = 60. If the inner box counted as its `10px`
/// length part, the outer box would come out 10 wide.
#[test]
fn calc_width_inside_an_auto_box_contributes_as_auto_2146() {
    let mut ids = None;
    let doc = build(|doc, body| {
        let cb = el(doc, body, "div", "width:360px");
        let chip = el(doc, cb, "span", "display:inline-block");
        let inner = el(
            doc,
            chip,
            "span",
            "display:inline-block;width:calc(50% + 10px)",
        );
        el(doc, inner, "div", "width:100px;height:10px");
        ids = Some((chip, inner));
    });
    let (chip, inner) = ids.unwrap();
    assert_width(&doc, chip, 100.0, "the auto chip");
    assert_width(&doc, inner, 60.0, "calc(50% + 10px) of the chip");
}
