//! Every box that draws a `text-overflow: ellipsis` draws it as an **IFC
//! root** (#982).
//!
//! There used to be a second ellipsis site, a rebuild in
//! `copy_cached_text_layouts` for a text node measured as a Taffy text *leaf*.
//! #969 took a flex or grid container's own text out of it (that text is an
//! anonymous item, which does not clip, and Chrome 153 draws no "…"), and #998
//! took out the last route markup had to it: a `span` flex item behind a
//! `display: contents` wrapper, which rinch left `display: inline` and measured
//! as a leaf. With both gone the site was deleted.
//!
//! Each shape below is a box that clips its own text with an ellipsis, reached
//! a different way — blockified as a flex or grid item (directly, behind one or
//! two `display: contents` wrappers, in a column), out of flow (absolute, fixed,
//! floated), atomic inline, a plain block, a form control. For every one the
//! fixture asserts the text belongs to an inline formatting context rooted at
//! the clipping box, and that the root's line ends in "…" inside the box. A
//! shape that fell back to being measured as a leaf fails on `ifc_root`, and
//! would now draw its line clipped with no "…" at all.
//!
//! Only the assertions that cannot move with the host's fonts are made: the
//! string is far wider than 60px in any face, the line box is declared, and
//! the width is bounded by the box, not pinned.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (400.0, 300.0);
const LONG: &str = "a line much too long for sixty pixels in any font at all";
const BASE_CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .clip { width: 60px; overflow: hidden; white-space: nowrap;
            text-overflow: ellipsis; }
    .flex { display: flex; }
    .col { display: flex; flex-direction: column; }
    .grid { display: grid; }
    .iflex { display: inline-flex; }
    .c { display: contents; }
    .rel { position: relative; }
    .abs { position: absolute; }
    .fixed { position: fixed; }
    .float { float: left; }
    .ib { display: inline-block; }
    .inline { display: inline; }
";

/// Build `wrappers` (outermost first) under `<body>`, then the clipping
/// `(tag, class)` holding [`LONG`], lay out twice (the second at another
/// viewport, so the tree is not left on the early-return path), and return the
/// clipping element and its text.
fn build(wrappers: &[(&str, &str)], clip: (&str, &str)) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(BASE_CSS);
    let mut parent = doc.body();
    for &(tag, class) in wrappers {
        let e = doc.create_element(tag);
        doc.set_attribute(e, "class", class);
        doc.append_child(parent, e);
        parent = e;
    }
    let x = doc.create_element(clip.0);
    doc.set_attribute(x, "class", clip.1);
    doc.append_child(parent, x);
    let t = doc.create_text(LONG);
    doc.append_child(x, t);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0 + 1.0, VP.1);
    (doc, x, t)
}

/// `(name, wrappers outermost first, the clipping (tag, class))`.
type Shape<'a> = (&'a str, &'a [(&'a str, &'a str)], (&'a str, &'a str));

#[test]
fn every_clipping_box_draws_its_ellipsis_as_an_ifc_root() {
    let shapes: &[Shape] = &[
        ("block div", &[], ("div", "clip")),
        (
            "inline span, blockified by flex",
            &[("div", "flex")],
            ("span", "clip inline"),
        ),
        ("span in a flex column", &[("div", "col")], ("span", "clip")),
        ("span in a grid", &[("div", "grid")], ("span", "clip")),
        (
            "span behind display: contents in flex (#998)",
            &[("div", "flex"), ("div", "c")],
            ("span", "clip"),
        ),
        (
            "span behind two display: contents in grid",
            &[("div", "grid"), ("div", "c"), ("div", "c")],
            ("span", "clip"),
        ),
        (
            "span behind display: contents in inline-flex",
            &[("div", "iflex"), ("div", "c")],
            ("span", "clip"),
        ),
        ("absolute span", &[("div", "rel")], ("span", "clip abs")),
        (
            "absolute span in flex",
            &[("div", "flex rel")],
            ("span", "clip abs"),
        ),
        ("fixed span", &[], ("span", "clip fixed")),
        ("floated span", &[("div", "")], ("span", "clip float")),
        ("inline-block span", &[("div", "")], ("span", "clip ib")),
        ("pre in flex", &[("div", "flex")], ("pre", "clip")),
        ("button", &[("div", "")], ("button", "clip")),
    ];
    let mut failures = Vec::new();
    for &(name, wrappers, clip) in shapes {
        let (doc, x, t) = build(wrappers, clip);
        let root = doc.tree.get(t.0).unwrap().ifc_root;
        if root != Some(x.0) {
            failures.push(format!(
                "{name}: text's ifc_root is {root:?}, not the clip {}",
                x.0
            ));
            continue;
        }
        let Some(layout) = doc.tree.get(x.0).unwrap().text_layout.as_ref() else {
            failures.push(format!("{name}: the clip has no inline layout"));
            continue;
        };
        if !layout.text_content.ends_with('\u{2026}') {
            failures.push(format!("{name}: no ellipsis: {:?}", layout.text_content));
            continue;
        }
        let w = layout.layout.width();
        if w > 60.0 {
            failures.push(format!(
                "{name}: the line is {w}px, wider than its 60px box"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
