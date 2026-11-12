//! Edit, then lay out again, must equal a fresh layout of the final tree.
//!
//! Three incremental-layout defects the scoped-IFC oracle
//! (`scoped_ifc_oracle_tests.rs`) found in both its twins, the scoped and the
//! whole-document pass alike, so not the scoped pass's own: #896 (whitespace
//! set into an empty block kept a line box), #897 (an out-of-flow box in an
//! atomic inline positioned from the chip's previous position) and #899 (a
//! Taffy `set_children` panic around an absolute box hoisted out of an
//! inline). Each fixture builds a document, lays it out, edits it, lays it out
//! again, and compares every box with a second document built directly in
//! the final state and laid out once.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node::NodeKind;

const VP: (f32, f32) = (800.0, 600.0);

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; width: 360px; }
    p { margin: 0; }
    .chip { display: inline-block; padding: 2px; }
    .rel { position: relative; }
    .abs { position: absolute; left: 3px; top: 4px; width: 60px; }
";

fn new_doc() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    doc
}

fn r(v: f32) -> f32 {
    (v * 64.0).round() / 64.0
}

/// Every node's box below `<body>`, pre-order, with its absolute position
/// (summed over `layout` parents, which is what paint and hit testing use).
fn snapshot(doc: &RinchDocument) -> Vec<String> {
    let mut out = Vec::new();
    walk(doc, doc.tree.body_id, 0, &mut out);
    out
}

fn walk(doc: &RinchDocument, id: u64, depth: usize, out: &mut Vec<String>) {
    let Some(node) = doc.tree.get(id) else {
        return;
    };
    let pad = depth * 2;
    match &node.kind {
        NodeKind::Text(t) => out.push(format!("{:pad$}{:?}", "", t.content)),
        _ => {
            let l = &node.layout;
            let (ax, ay) = abs_pos(doc, id);
            out.push(format!(
                "{:pad$}<{}> @({}, {}) abs({}, {}) {}x{}",
                "",
                node.tag().unwrap_or("?"),
                r(l.x),
                r(l.y),
                r(ax),
                r(ay),
                r(l.width),
                r(l.height)
            ));
        }
    }
    for &b in &node.run_boxes {
        if let Some(bn) = doc.tree.get(b) {
            let l = &bn.layout;
            out.push(format!(
                "{:pad$}  [anon] @({}, {}) {}x{}",
                "",
                r(l.x),
                r(l.y),
                r(l.width),
                r(l.height)
            ));
        }
    }
    if let Some(tl) = &node.text_layout {
        out.push(format!(
            "{:pad$}  ifc {:?} {}x{} lines={}",
            "",
            tl.text_content,
            r(tl.layout.width()),
            r(tl.layout.height()),
            tl.layout.len()
        ));
    }
    for &c in &node.children {
        walk(doc, c, depth + 1, out);
    }
}

/// Where paint draws a box's border-box origin.
fn abs_pos(doc: &RinchDocument, id: u64) -> (f32, f32) {
    let b = rinch_dom::paint::painted_border_box(&doc.tree, id, 1.0);
    (b.x0 as f32, b.y0 as f32)
}

fn copy_node(src: &RinchDocument, id: u64, dst: &mut RinchDocument) -> Option<NodeId> {
    let node = src.tree.get(id)?;
    if node.is_pseudo_element || node.is_anonymous_block_box {
        return None;
    }
    Some(match &node.kind {
        NodeKind::Text(t) => dst.create_text(&t.content),
        NodeKind::Comment(c) => dst.create_comment(c),
        NodeKind::Element(e) => {
            let el = dst.create_element(&e.tag);
            let mut attrs: Vec<(&String, &String)> = node.attributes.iter().collect();
            attrs.sort();
            for (k, v) in attrs {
                dst.set_attribute(el, k, v);
            }
            for &c in &node.children {
                if let Some(cn) = copy_node(src, c, dst) {
                    dst.append_child(el, cn);
                }
            }
            el
        }
        NodeKind::Document => return None,
    })
}

/// A second document built directly in `src`'s final state, laid out once.
fn fresh(src: &RinchDocument) -> Vec<String> {
    let mut doc = new_doc();
    let body = doc.body();
    let kids = src.tree.get(src.tree.body_id).unwrap().children.clone();
    for c in kids {
        if let Some(n) = copy_node(src, c, &mut doc) {
            doc.append_child(body, n);
        }
    }
    doc.resolve_layout(VP.0, VP.1);
    snapshot(&doc)
}

fn assert_matches_fresh(doc: &mut RinchDocument) {
    doc.resolve_layout(VP.0, VP.1);
    let inc = snapshot(doc);
    let fr = fresh(doc);
    assert_eq!(
        inc.join("\n"),
        fr.join("\n"),
        "incremental (left) differs from a fresh layout (right)"
    );
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str) -> NodeId {
    let n = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(n, "class", class);
    }
    doc.append_child(parent, n);
    n
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

// ── #896 ────────────────────────────────────────────────────────────────────

/// `set_text_content("  ")` on an empty block: the whitespace collapses away,
/// so the block stays 0 tall and the paragraph below stays at y 0 (Chrome
/// agrees: a block whose only content is collapsible white space has no line
/// box).
#[test]
fn whitespace_set_into_an_empty_block_has_no_line_box_896() {
    let mut doc = new_doc();
    let body = doc.body();
    let d = el(&mut doc, body, "div", "");
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "below");
    doc.resolve_layout(VP.0, VP.1);
    doc.set_text_content(d, "  ");
    assert_matches_fresh(&mut doc);
    assert_eq!(doc.tree.get(d.0).unwrap().layout.height, 0.0);
    assert_eq!(doc.tree.get(p.0).unwrap().layout.y, 0.0);
}

/// The same through a text node appended (the path the random oracle took).
#[test]
fn whitespace_appended_into_an_empty_block_has_no_line_box_896() {
    let mut doc = new_doc();
    let body = doc.body();
    let d = el(&mut doc, body, "div", "");
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "below");
    doc.resolve_layout(VP.0, VP.1);
    text(&mut doc, d, " ");
    assert_matches_fresh(&mut doc);
    assert_eq!(doc.tree.get(p.0).unwrap().layout.y, 0.0);
}

/// And the other way: real text in a block replaced by white space only.
#[test]
fn text_replaced_by_whitespace_drops_the_line_box_896() {
    let mut doc = new_doc();
    let body = doc.body();
    let d = el(&mut doc, body, "div", "");
    let t = text(&mut doc, d, "words");
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "below");
    doc.resolve_layout(VP.0, VP.1);
    assert_eq!(doc.tree.get(p.0).unwrap().layout.y, 20.0);
    doc.set_text_content(t, "   ");
    assert_matches_fresh(&mut doc);
    assert_eq!(doc.tree.get(p.0).unwrap().layout.y, 0.0);
}

// ── #897 ────────────────────────────────────────────────────────────────────

/// An absolute box inside an `inline-block` chip, its containing block the
/// `.rel` div above the paragraph. Text inserted before the chip moves the
/// chip along its line (off the fixed point: 0 → a measured width); the
/// absolute box's offset from its containing block must not move, so its
/// position relative to the chip has to be recomputed from the chip's *new*
/// place.
#[test]
fn an_absolute_box_in_a_chip_follows_the_chip_897() {
    let mut doc = new_doc();
    let body = doc.body();
    let rel = el(&mut doc, body, "div", "rel");
    let p = el(&mut doc, rel, "p", "");
    let lead = text(&mut doc, p, "a");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, "chip");
    let abs = el(&mut doc, chip, "div", "abs");
    text(&mut doc, abs, "abs");
    doc.resolve_layout(VP.0, VP.1);
    let before = abs_pos(&doc, abs.0);
    doc.set_text_content(lead, "several words before the chip ");
    assert_matches_fresh(&mut doc);
    let after = abs_pos(&doc, abs.0);
    // `left: 3px; top: 4px` from `.rel`, wherever the chip went.
    assert_eq!(after, (3.0, 4.0));
    assert_eq!(before, after);
}

/// The chip moving to a second line: the abs box's static position is not
/// used (both offsets are set), so it stays put.
#[test]
fn an_absolute_box_in_a_chip_that_wraps_897() {
    let mut doc = new_doc();
    let body = doc.body();
    let rel = el(&mut doc, body, "div", "rel");
    let p = el(&mut doc, rel, "p", "");
    let lead = text(&mut doc, p, "x ");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, "chip");
    let abs = el(&mut doc, chip, "div", "abs");
    text(&mut doc, abs, "abs");
    doc.resolve_layout(VP.0, VP.1);
    doc.set_text_content(
        lead,
        "a long run of words before the chip that takes the whole first line and more ",
    );
    assert_matches_fresh(&mut doc);
    assert_eq!(abs_pos(&doc, abs.0), (3.0, 4.0));
}

fn styled(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let n = doc.create_element(tag);
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// The random oracle's most common stop (seed 50 of the CI set): an
/// `inline-flex` chip whose first item holds only white space. Emptying that
/// item leaves the chip's size alone but moves its baseline (it is taken from
/// the next item now), so the line around the chip has to be built again.
/// The re-measure of the chip compared only its size, and kept the line it had
/// (35px tall, the chip 15px down) where a fresh layout has 20px.
#[test]
fn a_chip_whose_baseline_moves_at_the_same_size_relines_896() {
    // `whole`: the same edit laid out by the whole-document structural pass
    // (first layouts, theme changes), which sizes every atomic inline and
    // decides by the IFC roots' content signatures which lines to rebuild.
    for (from, to, whole) in [
        (" ", "", false),
        ("", " ", false),
        (" ", "", true),
        ("", " ", true),
    ] {
        let mut doc = new_doc();
        let body = doc.body();
        text(&mut doc, body, "word");
        let chip = styled(&mut doc, body, "div", "display: inline-flex");
        let p = styled(&mut doc, chip, "p", "display: inline");
        if !from.is_empty() {
            text(&mut doc, p, from);
        }
        text(&mut doc, chip, "a");
        let em = styled(
            &mut doc,
            chip,
            "em",
            "display: flex; flex-direction: column",
        );
        el(&mut doc, em, "p", "");
        doc.resolve_layout(VP.0, VP.1);
        let size = |doc: &RinchDocument| {
            let l = &doc.tree.get(chip.0).unwrap().layout;
            (l.width, l.height)
        };
        let before = size(&doc);
        doc.set_inner_html(p, to);
        doc.tree.ifc_dirty = whole;
        assert_matches_fresh(&mut doc);
        // The fixture's premise: the chip is the size it was.
        assert_eq!(size(&doc), before, "{from:?} -> {to:?}, whole {whole}");
    }
}

/// The same through a text edit inside a nested chip (an `inline-block`'s
/// last line, its baseline, moves when its last block loses its text).
#[test]
fn an_inline_block_whose_last_line_empties_relines_896() {
    let mut doc = new_doc();
    let body = doc.body();
    text(&mut doc, body, "word ");
    let chip = styled(
        &mut doc,
        body,
        "span",
        "display: inline-block; height: 60px",
    );
    let a = el(&mut doc, chip, "div", "");
    text(&mut doc, a, "first");
    let b = el(&mut doc, chip, "div", "");
    let t = text(&mut doc, b, "second");
    text(&mut doc, body, " tail");
    doc.resolve_layout(VP.0, VP.1);
    doc.set_text_content(t, " ");
    assert_matches_fresh(&mut doc);
}

// ── list markers in an atomic inline ────────────────────────────────────────

/// An empty `<li>` gets the line its outside marker sits in
/// (`Node::marker_line_floor`), placed after the first layout and laid out
/// again. Inside an `inline-block` list the item's box is sized by the
/// list's own detached compute, which that second layout did not run, so the
/// first layout left the item 0 tall; an edit later gave it its 20px, and the
/// random oracle stopped there with the *fresh* side wrong. Chrome 157: the
/// list 40x20 / 150x20 / 112x22, each item 20 tall.
#[test]
fn an_empty_list_item_in_an_inline_block_list_has_its_marker_line() {
    for (style, width, height) in [
        ("display: inline-block; margin: 0", 40.0, 20.0),
        (
            "display: inline-block; padding: 1px; margin: 0",
            112.0,
            22.0,
        ),
    ] {
        let mut doc = new_doc();
        let body = doc.body();
        let d = el(&mut doc, body, "div", "");
        text(&mut doc, d, "x");
        let ul = styled(&mut doc, d, "ul", style);
        let li = el(&mut doc, ul, "li", "");
        if width > 100.0 {
            doc.set_attribute(li, "style", "width: 110px");
        }
        doc.resolve_layout(VP.0, VP.1);
        let l = &doc.tree.get(ul.0).unwrap().layout;
        assert_eq!((l.width, l.height), (width, height), "{style}");
        assert_eq!(doc.tree.get(li.0).unwrap().layout.height, 20.0, "{style}");
        // And an edit beside it changes nothing about it.
        text(&mut doc, d, " y");
        assert_matches_fresh(&mut doc);
    }
}

/// Random oracle seed 2311: text in a `width: 110px` item of an
/// `inline-block` goes from four lines to one. The chip's compute measured the
/// item's new text at its max-content width (35px) and kept the line
/// baselines recorded for the old text at 110px; the baseline lookup takes the
/// entry nearest the item's size, which was the old one (off by 60 in height
/// against 75 in width), so the chip was lined up 60px too low and its line
/// stood 81px tall where a fresh layout's is 38px.
#[test]
fn an_inline_block_whose_item_rewraps_forgets_the_old_line_baselines() {
    for structural in [false, true] {
        let mut doc = new_doc();
        let body = doc.body();
        let chip = styled(
            &mut doc,
            body,
            "section",
            "display: inline-block; padding: 1px",
        );
        let li = styled(&mut doc, chip, "li", "width: 110px");
        let t = text(
            &mut doc,
            li,
            "a rather longer run of text that wraps somewhere",
        );
        styled(&mut doc, chip, "div", "display: flex");
        doc.resolve_layout(VP.0, VP.1);
        assert_eq!(doc.tree.get(chip.0).unwrap().layout.height, 82.0);
        if structural {
            // In the same frame as a structural change in the chip.
            let ul = doc.create_element("ul");
            doc.set_attribute(ul, "style", "width: 110px");
            doc.insert_child(chip, ul, 2);
        }
        doc.set_text_content(t, "word");
        assert_matches_fresh(&mut doc);
        let body_ifc = doc.tree.get(body.0).unwrap().text_layout.as_ref().unwrap();
        assert_eq!(
            body_ifc.layout.height(),
            doc.tree.get(chip.0).unwrap().layout.height,
            "the chip's line is the chip's height (structural: {structural})"
        );
    }
}
