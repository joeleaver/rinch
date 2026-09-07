//! A `display: contents` wrapper counts as block content only when it is
//! **opaque** (#518).
//!
//! CSS 2.1 §9.2.1.1 generates an anonymous block box for a block container
//! holding both inline and block-level in-flow children. A `display: contents`
//! wrapper has no box of its own, so what it contributes is whatever it wraps:
//! a wrapper around a real block is block content, and a wrapper around inline
//! content, nothing, or only out-of-flow boxes is not.
//!
//! `create_anonymous_block_boxes` used to count every contents wrapper,
//! transparent or not, while the IFC scan next door classified the same node
//! by what it actually wraps (#289/#502). Two sites, one node, two answers —
//! the defect family the whole #466 sequence was about. The visible cost was
//! an absolute dropped entirely: the minted anonymous box left the wrapper's
//! own `display: none` Taffy node attached, and the absolute underneath it was
//! laid out 0x0, never positioned, never painted.
//!
//! **Both directions are tested here on purpose.** A fix that simply stopped
//! counting contents wrappers would pass the transparent cases and silently
//! stop generating anonymous boxes for the opaque ones — so every transparent
//! fixture has an opaque twin that differs only in what the wrapper holds.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
}

/// Whether `node` ended up as inline content of an *anonymous* IFC root —
/// i.e. whether the container was classified as mixed content.
fn is_in_anonymous_root(doc: &RinchDocument, node: NodeId) -> bool {
    doc.tree
        .get(node.0)
        .and_then(|n| n.ifc_root)
        .and_then(|root| doc.tree.get(root))
        .map(|root| root.is_anonymous_block_box)
        .unwrap_or(false)
}

// ── The reported bug ────────────────────────────────────────────────────────

/// The issue's own markup: direct text beside a `display: contents` wrapper
/// whose only child is absolutely positioned.
///
/// The wrapper wraps no in-flow block, so the container holds inline content
/// only and no anonymous box is generated. The absolute is then reachable and
/// resolves against the viewport (it has no positioned ancestor, #204), which
/// is `left: 10px; top: 10px; width: 40px; height: 20px`.
///
/// Kills: counting a transparent contents wrapper in `has_block`. With that
/// mutant the absolute is laid out `(0, 0, 0, 0)` — the reported symptom.
#[test]
fn an_absolute_under_a_transparent_wrapper_beside_text_is_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "");
    text_in(&mut doc, container, "text");
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    let abs = child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px",
    );

    doc.resolve_layout(VW, VH);

    let l = doc.tree.get(abs.0).unwrap().layout;
    assert_eq!(
        (l.x, l.y, l.width, l.height),
        (10.0, 10.0, 40.0, 20.0),
        "the absolute must be laid out and positioned; (0, 0, 0, 0) means it \
         is still trapped under the wrapper's detached Taffy node"
    );
}

/// The opaque twin, which is what stops the fix from being "never count a
/// contents wrapper". The same markup with a **block** in the wrapper instead
/// of an absolute: that is real in-flow block content, the container *is*
/// mixed, and the text must go into an anonymous block box.
///
/// Kills: dropping the `Contents` arm from `has_block` altogether.
#[test]
fn a_block_under_an_opaque_wrapper_beside_text_still_mints_an_anonymous_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "");
    let t = doc.create_text("text");
    doc.append_child(container, t);
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    child_of(&mut doc, wrapper, "div", "width: 40px; height: 20px");

    doc.resolve_layout(VW, VH);

    assert!(
        is_in_anonymous_root(&doc, t),
        "an opaque wrapper holds real block content, so the text beside it \
         must be wrapped in an anonymous block box"
    );
}

/// And the transparent case must *not* mint one — the other half of the same
/// classification, asserted directly rather than only through the absolute's
/// geometry.
#[test]
fn a_transparent_wrapper_beside_text_mints_no_anonymous_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "");
    let t = doc.create_text("text");
    doc.append_child(container, t);
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px",
    );

    doc.resolve_layout(VW, VH);

    assert!(
        !is_in_anonymous_root(&doc, t),
        "a wrapper holding only an out-of-flow box contributes no block-level \
         content, so the container is not mixed"
    );
}

// ── Run grouping has to agree with the same classification ──────────────────

/// Text on both sides of a transparent wrapper is **one** inline run.
///
/// `has_block` and the run-grouping loop must answer identically about the
/// same node: if the container is classified as mixed but the wrapper does not
/// end a run — or the reverse — the text either splits across two anonymous
/// boxes and two lines, or is grouped as one and then boxed as if it were two.
/// Browsers render this as a single line.
///
/// Kills: ending the run on a transparent wrapper. The two text nodes land in
/// separate runs, and the second is laid out on a second line.
#[test]
fn a_transparent_wrapper_does_not_split_the_text_around_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");
    let before = doc.create_text("before ");
    doc.append_child(container, before);
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 0; top: 0; width: 10px; height: 10px",
    );
    let after = doc.create_text(" after");
    doc.append_child(container, after);

    doc.resolve_layout(VW, VH);

    let root_before = doc.tree.get(before.0).unwrap().ifc_root;
    let root_after = doc.tree.get(after.0).unwrap().ifc_root;
    assert_eq!(
        root_before, root_after,
        "text on both sides of a transparent wrapper belongs to one inline \
         run; two different IFC roots mean the wrapper ended the run and the \
         text was split across two anonymous boxes"
    );
}

/// The opaque twin again: a real block between two runs of text *does* split
/// them, and onto different lines. Without this, "never end a run" would pass
/// the test above.
#[test]
fn an_opaque_wrapper_does_split_the_text_around_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");
    let before = doc.create_text("before ");
    doc.append_child(container, before);
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    child_of(&mut doc, wrapper, "div", "width: 40px; height: 20px");
    let after = doc.create_text(" after");
    doc.append_child(container, after);

    doc.resolve_layout(VW, VH);

    let root_before = doc
        .tree
        .get(before.0)
        .unwrap()
        .ifc_root
        .expect("inline content");
    let root_after = doc
        .tree
        .get(after.0)
        .unwrap()
        .ifc_root
        .expect("inline content");
    assert_ne!(
        root_before, root_after,
        "a block inside the wrapper is real block content and must end the \
         run, putting the text on either side in separate anonymous boxes"
    );

    // And those boxes are on different lines, which is what the split is for.
    let y_before = doc.tree.get(root_before).unwrap().layout.y;
    let y_after = doc.tree.get(root_after).unwrap().layout.y;
    assert!(
        y_after > y_before,
        "the second anonymous box must sit below the first: \
         before.y={y_before}, after.y={y_after}"
    );
}

/// A nested transparent chain is still transparent — the scan recurses, so a
/// wrapper inside a wrapper around an absolute must behave like one wrapper.
#[test]
fn nested_transparent_wrappers_stay_transparent() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "");
    text_in(&mut doc, container, "text");
    let outer = child_of(&mut doc, container, "span", "display: contents");
    let inner = child_of(&mut doc, outer, "span", "display: contents");
    let abs = child_of(
        &mut doc,
        inner,
        "div",
        "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px",
    );

    doc.resolve_layout(VW, VH);

    let l = doc.tree.get(abs.0).unwrap().layout;
    assert_eq!(
        (l.x, l.y, l.width, l.height),
        (10.0, 10.0, 40.0, 20.0),
        "the transparency scan must recurse through nested wrappers"
    );
}

/// A transparent wrapper inside a container that is mixed **for an
/// independent reason** must still not end the inline run.
///
/// This is the case the two tests above structurally cannot reach, and it is
/// the one that makes the run-grouping half of the fix testable at all. When
/// the wrapper is the container's only non-inline child, a transparent one
/// makes `has_block` false and `create_anonymous_block_boxes` returns before
/// the run loop ever executes — so the loop's treatment of that wrapper is
/// unobservable, and ending the run on *every* contents wrapper passes the
/// whole suite.
///
/// Adding a real block sibling makes the container mixed regardless, the loop
/// runs, and the transparent wrapper's contribution becomes visible: the text
/// on either side of it is one run, because a wrapper holding only an
/// out-of-flow box contributes no block-level box to break on. The trailing
/// block is what legitimately ends it.
///
/// Kills: ending the run on any `Contents` child regardless of transparency —
/// which is `has_block` and the run loop disagreeing about one node again,
/// the exact shape #518 was.
#[test]
fn a_transparent_wrapper_does_not_split_a_run_in_an_independently_mixed_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");

    let before = doc.create_text("before ");
    doc.append_child(container, before);

    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 0; top: 0; width: 10px; height: 10px",
    );

    let after = doc.create_text(" after");
    doc.append_child(container, after);

    // The independent reason the container is mixed. Without it, the guard in
    // `create_anonymous_block_boxes` returns before the run loop.
    child_of(&mut doc, container, "div", "width: 40px; height: 20px");

    doc.resolve_layout(VW, VH);

    let root_before = doc.tree.get(before.0).unwrap().ifc_root;
    let root_after = doc.tree.get(after.0).unwrap().ifc_root;
    assert!(
        root_before.is_some(),
        "the text is inline content of some IFC"
    );
    assert_eq!(
        root_before, root_after,
        "the transparent wrapper contributes no block-level box, so the text \
         on either side of it stays one run — two roots mean the run loop \
         broke on it while `has_block` did not count it"
    );
}
