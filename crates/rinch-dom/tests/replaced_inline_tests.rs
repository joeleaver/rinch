//! A replaced element or form control at `display: inline` is still an
//! **atomic** inline-level box (#1089).
//!
//! The UA sheet makes `img, input, button, select, textarea, svg`
//! `inline-block`, so this only bites when author CSS writes
//! `display: inline`. Before the fix rinch classified such an element as a
//! *flowed* inline element — IFC content with no box of its own — so it
//! measured `0x0` and painted nothing.
//!
//! **The oracle is Chrome 153**, measured with
//! `div(font: 16px/20px; width: 300px) > "ab" + <tag style="display: X;
//! width: 20px; height: 20px; padding: 0; border: 0; margin: 0;
//! box-sizing: border-box; transform: translateX(30px)">` for X in
//! `inline` and `inline-block`: for every one of the six tags the two give
//! the **identical** box (`getBoundingClientRect` offset, 20x20, and the
//! container's height). So the font-independent statement of the rule is
//! "`display: inline` lays out exactly as `display: inline-block`", which is
//! what each fixture asserts — plus the 20x20 size, which is what the broken
//! code got wrong (it reported `0x0`).

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::node::DisplayMode;

/// The six the UA sheet makes `inline-block`, and five more Chrome 153 also
/// renders as a 20x20 atomic box at `display: inline`.
const TAGS: [&str; 11] = [
    "img", "input", "button", "svg", "select", "textarea", "video", "canvas", "iframe", "meter",
    "progress",
];

const BOX: &str = "width: 20px; height: 20px; padding: 0; border: 0; margin: 0; \
                   box-sizing: border-box; transform: translateX(30px); background: red";

/// Build `div > "ab" + <tag style="display: {display}; …">` and return the
/// document and the element.
fn build(tag: &str, display: &str) -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(
        d,
        "style",
        "font-size: 16px; line-height: 20px; width: 300px",
    );
    doc.append_child(body, d);
    let t = doc.create_text("ab");
    doc.append_child(d, t);
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", &format!("display: {display}; {BOX}"));
    doc.append_child(d, e);
    doc.resolve_layout(800.0, 600.0);
    (doc, e.0)
}

/// The on-screen box paint places the element at: `(x, y, w, h)`, transform
/// included.
fn painted_box(doc: &RinchDocument, id: usize) -> (f64, f64, f32, f32) {
    let (x, y, tf) = rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, id, 1.0);
    let c = tf.as_coeffs();
    let n = doc.tree.get(id).unwrap();
    (x + c[4], y + c[5], n.layout.width, n.layout.height)
}

#[test]
fn a_replaced_element_at_display_inline_lays_out_as_an_inline_block() {
    for tag in TAGS {
        let (inl, i) = build(tag, "inline");
        let (blk, b) = build(tag, "inline-block");
        let ni = inl.tree.get(i).unwrap();
        assert!(
            ni.display_mode.is_atomic_inline(),
            "{tag}: display: inline must still be an atomic inline, got {:?}",
            ni.display_mode
        );
        assert!(!ni.is_flowed_inline_element(), "{tag}: must own a box");
        let got = painted_box(&inl, i);
        let want = painted_box(&blk, b);
        assert_eq!((got.2, got.3), (20.0, 20.0), "{tag}: box size");
        assert_eq!(
            got, want,
            "{tag}: inline vs inline-block (Chrome: identical)"
        );
        // The text before it still takes its space: the box sits past "ab"
        // plus the 30px translation, off the x = 30 fixed point.
        assert!(got.0 > 30.0, "{tag}: box x {} must be past the text", got.0);
    }
}

/// `overflow` and the clip predicate: an atomic inline clips, a non-atomic
/// inline box does not (#591). A replaced element at `display: inline` is
/// the former.
#[test]
fn a_replaced_element_at_display_inline_clips_its_overflow() {
    for tag in TAGS {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let d = doc.create_element("div");
        doc.append_child(body, d);
        let e = doc.create_element(tag);
        doc.set_attribute(
            e,
            "style",
            &format!("display: inline; overflow: hidden; {BOX}"),
        );
        doc.append_child(d, e);
        doc.resolve_layout(800.0, 600.0);
        assert!(
            doc.tree.get(e.0).unwrap().clips_overflow(),
            "{tag}: an atomic inline with overflow: hidden clips"
        );
    }
}

/// The same element flipped from its UA `inline-block` to `display: inline`
/// by a restyle keeps its box — and a `<span>` flipped the same way does not
/// (the positive control that the flip really reached the cascade).
#[test]
fn flipping_to_display_inline_keeps_the_box() {
    for tag in TAGS.into_iter().chain(["span"]) {
        let (mut doc, e) = build(tag, "inline-block");
        let before = painted_box(&doc, e);
        assert_eq!((before.2, before.3), (20.0, 20.0), "{tag}: before");
        doc.set_attribute(
            rinch_core::dom::NodeId(e),
            "style",
            &format!("display: inline; {BOX}"),
        );
        doc.resolve_layout(801.0, 600.0);
        let n = doc.tree.get(e).unwrap();
        if tag == "span" {
            assert_eq!(n.display_mode, DisplayMode::Inline, "control");
            assert!(n.is_flowed_inline_element(), "control: a span is flowed");
        } else {
            let after = painted_box(&doc, e);
            assert_eq!(after, before, "{tag}: flip to inline changed the box");
        }
    }
}

/// Paint `doc` into a 300x120 pixmap with the software painter.
fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = rinch_dom::paint::skia_painter::TinySkiaPainter::new(300, 120);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (300.0, 120.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

fn html_doc(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(
        d,
        "style",
        "font-size: 16px; line-height: 20px; width: 280px; padding-top: 30px",
    );
    doc.append_child(body, d);
    doc.set_inner_html(d, html);
    doc.resolve_layout(300.0, 120.0);
    doc
}

fn first_with_tag(doc: &RinchDocument, tag: &str) -> usize {
    doc.tree
        .nodes
        .iter()
        .find(|(_, n)| n.tag() == Some(tag))
        .map(|(i, _)| i)
        .unwrap()
}

/// An `<svg>` **with a child** at `display: inline` paints and classifies
/// exactly as its `inline-block` twin. The childless fixtures above cannot
/// tell `InlineBlock` from `InlineFlex` (the inside formatting context never
/// shows); this one kills the mutant mapping the new arm to `InlineFlex`
/// (review of #1095, M3).
#[test]
fn an_svg_with_a_child_at_display_inline_matches_its_inline_block_twin() {
    let mk = |d: &str| {
        html_doc(&format!(
            "ab<svg style=\"display: {d}; width: 20px; height: 20px\" viewBox=\"0 0 20 20\">\
             <rect x=\"0\" y=\"0\" width=\"20\" height=\"20\" fill=\"red\"></rect></svg>cd"
        ))
    };
    let mut inl = mk("inline");
    let mut blk = mk("inline-block");
    let v = inl.taffy_tree_violations();
    assert!(v.is_empty(), "{v:?}");
    let pi = rasterize(&mut inl);
    let pb = rasterize(&mut blk);
    let red = |p: &[u8]| {
        p.chunks(4)
            .filter(|c| c[0] > 200 && c[1] < 60 && c[2] < 60 && c[3] > 200)
            .count()
    };
    assert!(red(&pb) > 300, "the twin paints the rect: {}", red(&pb));
    assert!(pi == pb, "pixels differ: {} vs {} red", red(&pi), red(&pb));
    let ri = inl.tree.get(first_with_tag(&inl, "rect")).unwrap();
    let rb = blk.tree.get(first_with_tag(&blk, "rect")).unwrap();
    assert_eq!(ri.display_mode, rb.display_mode);
    assert_eq!(ri.ifc_root.is_some(), rb.ifc_root.is_some());
    assert_eq!(ri.layout, rb.layout);
}

/// A transform on a replaced element at `display: inline` applies: it moves
/// the painted box by 37px, as in Chrome (`Node::transform_applies`, #1080).
#[test]
fn a_transform_moves_a_replaced_element_at_display_inline() {
    for tag in ["img", "button", "svg"] {
        let pos = |tf: &str| {
            let doc = html_doc(&format!(
                "ab<{tag} style=\"display: inline; width: 20px; height: 20px; {tf}\"></{tag}>"
            ));
            let id = first_with_tag(&doc, tag);
            let (x, _, t) =
                rinch_dom::paint::compute_absolute_position_and_transform(&doc.tree, id, 1.0);
            (
                doc.tree.get(id).unwrap().transform_applies(),
                x + t.as_coeffs()[4],
            )
        };
        let (applies, moved) = pos("transform: translateX(37px)");
        assert!(applies, "{tag}: transform must apply");
        let moved = moved - pos("").1;
        assert!((moved - 37.0).abs() < 0.01, "{tag}: moved {moved}");
    }
}
