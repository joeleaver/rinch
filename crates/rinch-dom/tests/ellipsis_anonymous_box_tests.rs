//! Text beside a block child takes `text-overflow: ellipsis` from its block
//! container (#1071).
//!
//! In `<div class=clip>long text<div>child</div>more text</div>` each text run
//! is laid out by an **anonymous block box**, an IFC root owned by no element.
//! Its `computed_style` holds only the inherited properties
//! (`ComputedStyle::for_anonymous_box`), and `text-overflow` and `overflow` are
//! not inherited — so the ellipsis decision, which read the root's own style,
//! never drew a "…" there. Chrome 153 does, on both runs (measured, headless
//! screenshot of the same markup): the anonymous box's lines are the block
//! container's lines, clipped by its `overflow`.
//!
//! The same holds when the run sits in a `<span>` split around the block child
//! (#513). The span holds the text, but the boxes are its block container's,
//! and a restyle of that container reaches no text child of its own — so this
//! shape is the one that pins the container's `run_boxes` invalidation.
//!
//! Controls, also measured in Chrome 153: a **grid** container's text is an
//! anonymous grid item, which does not clip, so no "…"; and a container with
//! `overflow: visible` draws none.
//!
//! Only font-independent facts are asserted: every run is far wider than 90px
//! in any face and the line box is declared.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const RUN1: &str = "a first run much too long for ninety pixels in any font";
const RUN2: &str = "a second run much too long for ninety pixels in any font";
const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .clip { width: 90px; overflow: hidden; white-space: nowrap;
            text-overflow: ellipsis; }
    .plain { width: 90px; overflow: hidden; white-space: nowrap; }
    .vis { overflow: visible; }
    .grid { display: grid; }
";

struct Doc {
    doc: RinchDocument,
    owner: NodeId,
    texts: [NodeId; 2],
}

/// `<div class={class}>RUN1<div>child</div>RUN2</div>`, with the two runs and
/// the block child inside a `<span>` when `span` is set.
fn build(class: &str, span: bool) -> Doc {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let owner = doc.create_element("div");
    doc.set_attribute(owner, "class", class);
    doc.append_child(body, owner);
    let holder = if span {
        let s = doc.create_element("span");
        doc.append_child(owner, s);
        s
    } else {
        owner
    };
    let t1 = doc.create_text(RUN1);
    doc.append_child(holder, t1);
    let child = doc.create_element("div");
    doc.append_child(holder, child);
    let ct = doc.create_text("child");
    doc.append_child(child, ct);
    let t2 = doc.create_text(RUN2);
    doc.append_child(holder, t2);
    doc.resolve_layout(400.0, 300.0);
    Doc {
        doc,
        owner,
        texts: [t1, t2],
    }
}

/// For each run: whether the layout that draws it ends in "…". A run drawn by
/// no inline layout (a grid container's text leaf) answers from the leaf's
/// cached layout, which never carries one — `false`.
fn ellipses(d: &Doc) -> [bool; 2] {
    d.texts.map(|t| {
        let node = d.doc.tree.get(t.0).unwrap();
        match node.ifc_root {
            Some(root) => d
                .doc
                .tree
                .get(root)
                .unwrap()
                .text_layout
                .as_ref()
                .is_some_and(|l| l.text_content.ends_with('\u{2026}')),
            None => false,
        }
    })
}

/// The runs must be laid out by anonymous boxes for the fixture to be about
/// #1071 at all.
fn assert_anonymous_roots(d: &Doc, what: &str) {
    for t in d.texts {
        let root = d.doc.tree.get(t.0).unwrap().ifc_root;
        let root = root.unwrap_or_else(|| panic!("{what}: a run has no IFC root"));
        assert!(
            d.doc.tree.get(root).unwrap().is_anonymous_block_box,
            "{what}: a run's IFC root is not an anonymous block box"
        );
    }
}

#[test]
fn text_beside_a_block_child_takes_the_containers_ellipsis() {
    let d = build("clip", false);
    assert_anonymous_roots(&d, "block owner");
    assert_eq!(ellipses(&d), [true, true], "block owner");
}

#[test]
fn text_in_a_split_span_takes_the_block_containers_ellipsis() {
    let d = build("clip", true);
    assert_anonymous_roots(&d, "split span");
    assert_eq!(ellipses(&d), [true, true], "split span");
}

#[test]
fn controls_draw_no_ellipsis() {
    // Measured in Chrome 153: none of these draws a "…".
    for (class, span) in [
        ("clip vis", false),
        ("clip vis", true),
        ("plain", false),
        ("clip grid", false),
    ] {
        let d = build(class, span);
        assert_eq!(ellipses(&d), [false, false], "{class} span={span}");
    }
}

/// A restyle of the **owner** is what changes the answer, and the anonymous
/// boxes hold none of the properties it changes — so the boxes must be
/// re-shaped anyway. Each shape is laid out in its first class, restyled, laid
/// out at another viewport, and compared with a fresh build in the final class.
#[test]
fn a_restyle_of_the_owner_re_decides_the_anonymous_boxes_ellipsis() {
    let shapes: &[(&str, &str, bool, bool)] = &[
        ("plain", "clip", false, true),
        ("clip", "plain", false, false),
        ("clip vis", "clip", false, true),
        ("clip", "clip vis", false, false),
        ("clip", "clip grid", false, false),
        ("clip grid", "clip", false, true),
        ("plain", "clip", true, true),
        ("clip", "plain", true, false),
    ];
    let mut failures = Vec::new();
    for &(from, to, span, want) in shapes {
        let mut d = build(from, span);
        d.doc.set_attribute(d.owner, "class", to);
        d.doc.resolve_layout(401.0, 300.0);
        let got = ellipses(&d);
        let oracle = ellipses(&build(to, span));
        if oracle != [want, want] {
            failures.push(format!(
                "{from} -> {to} span={span}: fresh oracle {oracle:?}, want {want}"
            ));
        }
        if got != oracle {
            failures.push(format!(
                "{from} -> {to} span={span}: {got:?} after the restyle, {oracle:?} fresh"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The block -> grid -> block round trip #1046/#1067 made rebuild: the anonymous
/// boxes are minted again on the way back, and must draw the "…" again.
#[test]
fn a_block_grid_block_round_trip_draws_the_ellipsis_again() {
    let mut d = build("clip", false);
    d.doc.set_attribute(d.owner, "class", "clip grid");
    d.doc.resolve_layout(401.0, 300.0);
    assert_eq!(ellipses(&d), [false, false], "as a grid");
    d.doc.set_attribute(d.owner, "class", "clip");
    d.doc.resolve_layout(402.0, 300.0);
    assert_anonymous_roots(&d, "back to block");
    assert_eq!(ellipses(&d), [true, true], "back to block");
}
