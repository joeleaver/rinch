//! A subtree that leaves the document leaves its inline-formatting state behind
//! (#1073), and nothing outside the document is shaped (#1069).
//!
//! `set_text_content` on an element with children orphans them — `parent =
//! None`, not freed, so a handle the app still holds stays alive. Every other
//! verb that takes a subtree out (`remove_child`, `remove_node`, the moves)
//! runs `clear_ifc_root_recursive` over it; this one did not, so the orphans
//! kept their `ifc_root` marks, their IFC root stayed a root, and the next
//! layout shaped it and sized its atomic inlines though nothing paints them.
//! #1070 filtered the atomic-inline sizers; the marks are what fed all of them.
//!
//! The re-attach differential is the other half: whatever is left behind — or
//! dropped — while a subtree is out, attaching it again must lay it out as a
//! fresh document of the same final state does.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(el, "style", style);
    }
    doc.append_child(parent, el);
    el
}

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
    t
}

/// Every node of `node`'s subtree that still carries an `ifc_root` mark.
fn marked_in(doc: &RinchDocument, node: NodeId) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![node.0];
    while let Some(id) = stack.pop() {
        let n = doc.tree.get(id).unwrap();
        if n.ifc_root.is_some() {
            out.push(id);
        }
        stack.extend(n.children.iter().copied());
    }
    out
}

const P_STYLE: &str = "font-size: 16px; line-height: 20px; width: 200px";

#[derive(Clone, Copy, Debug)]
enum Detach {
    SetTextContent,
    RemoveChild,
}

/// `body > outer > p > ["alpha beta ", span > "gamma"]`, laid out.
fn orphan_doc() -> (RinchDocument, NodeId, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "");
    let p = child_of(&mut doc, outer, "p", P_STYLE);
    text_in(&mut doc, p, "alpha beta ");
    let span = child_of(&mut doc, p, "span", "");
    text_in(&mut doc, span, "gamma");
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    (doc, body, outer, p, span)
}

fn detach(doc: &mut RinchDocument, how: Detach, outer: NodeId, p: NodeId) {
    match how {
        Detach::SetTextContent => doc.set_text_content(outer, "new"),
        Detach::RemoveChild => doc.remove_child(outer, p),
    }
}

/// The orphans lose their marks at the detach, as a removed subtree's do.
///
/// Kills: dropping the `clear_ifc_root_recursive` from `set_text_content`'s
/// orphan loop (the `SetTextContent` case fails; `RemoveChild` is the twin
/// that says what the answer is).
#[test]
fn set_text_content_clears_its_orphans_ifc_marks_as_remove_child_does() {
    for how in [Detach::RemoveChild, Detach::SetTextContent] {
        let (mut doc, _body, outer, p, _span) = orphan_doc();
        let before = marked_in(&doc, p);
        assert!(
            before.len() >= 3,
            "{how:?}: control, attached the text, the span and its text are \
             IFC content of the `p` ({before:?})"
        );
        detach(&mut doc, how, outer, p);
        assert!(
            doc.tree.get(p.0).unwrap().parent.is_none(),
            "{how:?}: precondition, the `p` is out of the document"
        );
        assert_eq!(
            marked_in(&doc, p),
            Vec::<usize>::new(),
            "{how:?}: a subtree out of the document carries no IFC marks"
        );
        // And the next layout leaves it that way: the scoped pass the detach
        // asks for does not reach a disconnected subtree.
        doc.resolve_layout(VW - 7.0, VH);
        assert_eq!(
            marked_in(&doc, p),
            Vec::<usize>::new(),
            "{how:?}: …after a layout too"
        );
    }
}

/// What a laid-out IFC root says about itself, for the differential.
fn ifc_summary(doc: &RinchDocument, root: NodeId) -> (String, usize, f32, f32) {
    let n = doc.tree.get(root.0).unwrap();
    let tl = n
        .text_layout
        .as_ref()
        .expect("an attached IFC root has a paint layout");
    (
        tl.text_content.clone(),
        tl.layout.lines().count(),
        n.layout.width,
        n.layout.height,
    )
}

/// An orphaned IFC root attached again lays out as a fresh one does — over
/// both detach routes, with its content changed before the detach (no layout
/// in between), optionally changed again while out, a whole-document pass
/// optionally run while it is out (the pass that still marks detached
/// subtrees, whose root `build_ifc_layouts` now declines to shape), and a
/// scoped or whole-document pass on the re-attach.
#[test]
fn a_reattached_orphan_ifc_root_lays_out_as_a_fresh_one() {
    for how in [Detach::SetTextContent, Detach::RemoveChild] {
        for whole_while_out in [false, true] {
            for change_while_out in [false, true] {
                for whole_on_attach in [false, true] {
                    let case = format!(
                        "{how:?}, whole-document pass while out {whole_while_out}, \
                         changed while out {change_while_out}, whole-document \
                         pass on attach {whole_on_attach}"
                    );
                    let (mut doc, body, outer, p, span) = orphan_doc();
                    // Changed before the detach, with no layout between: the
                    // root leaves dirty.
                    text_in(&mut doc, span, " delta epsilon");
                    detach(&mut doc, how, outer, p);
                    doc.resolve_layout(VW, VH);
                    if whole_while_out {
                        doc.recompute_all_styles_full();
                        doc.resolve_layout(VW, VH);
                    }
                    if change_while_out {
                        text_in(&mut doc, span, " zeta eta theta");
                    }
                    doc.append_child(body, p);
                    if whole_on_attach {
                        doc.recompute_all_styles_full();
                    }
                    doc.resolve_layout(VW, VH);
                    let got = ifc_summary(&doc, p);

                    let mut fresh = RinchDocument::new();
                    let fbody = fresh.body();
                    let fp = child_of(&mut fresh, fbody, "p", P_STYLE);
                    text_in(&mut fresh, fp, "alpha beta ");
                    let fspan = child_of(&mut fresh, fp, "span", "");
                    text_in(&mut fresh, fspan, "gamma");
                    text_in(&mut fresh, fspan, " delta epsilon");
                    if change_while_out {
                        text_in(&mut fresh, fspan, " zeta eta theta");
                    }
                    fresh.resolve_layout(VW, VH);
                    fresh.resolve_layout(VW, VH);
                    let want = ifc_summary(&fresh, fp);

                    assert!(
                        want.1 >= 2 && want.0.contains("delta"),
                        "{case}: positive control, the fresh root wraps and holds \
                         the pre-detach change ({want:?})"
                    );
                    assert_eq!(got, want, "{case}: re-attached root vs a fresh layout");
                }
            }
        }
    }
}

/// `body > wrap > outer > p > ["alpha beta ", span > "gamma"]`, laid out.
fn nested_doc() -> (RinchDocument, NodeId, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = child_of(&mut doc, body, "div", "");
    let outer = child_of(&mut doc, wrap, "div", "");
    let p = child_of(&mut doc, outer, "p", P_STYLE);
    text_in(&mut doc, p, "alpha beta ");
    let span = child_of(&mut doc, p, "span", "");
    text_in(&mut doc, span, "gamma");
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    (doc, wrap, outer, p, span)
}

/// The root is not a direct orphan: an **ancestor** of it was removed, so the
/// root's own `parent` is still `Some`. Every fixture above detaches the root
/// itself, so a turn-away that only asked `parent.is_none()` passed them all
/// (found by #1102's review). Re-attached through the ancestor, the root lays
/// out as a fresh document does.
///
/// Kills: `build_ifc_layouts`' connectivity test weakened to
/// `parent.is_none()`.
#[test]
fn a_root_under_a_detached_ancestor_is_turned_away_and_comes_back() {
    for whole_on_attach in [false, true] {
        let (mut doc, wrap, outer, p, span) = nested_doc();
        text_in(&mut doc, span, " delta epsilon");
        doc.remove_child(wrap, outer);
        doc.resolve_layout(VW, VH);
        let shapes0 = doc.tree.perf.get(Counter::ShapeIfcBuild);
        doc.recompute_all_styles_full();
        doc.resolve_layout(VW, VH);
        assert!(
            !marked_in(&doc, p).is_empty(),
            "control: the whole-document pass marked the detached subtree again"
        );
        assert_eq!(
            doc.tree.perf.get(Counter::ShapeIfcBuild) - shapes0,
            0,
            "a root under a detached ancestor is not shaped"
        );
        assert!(doc.tree.get(p.0).unwrap().text_layout.is_none());
        text_in(&mut doc, span, " zeta eta theta");
        doc.append_child(wrap, outer);
        if whole_on_attach {
            doc.recompute_all_styles_full();
        }
        doc.resolve_layout(VW, VH);
        let got = ifc_summary(&doc, p);

        let (mut fresh, _fw, _fo, fp, fspan) = nested_doc();
        text_in(&mut fresh, fspan, " delta epsilon");
        text_in(&mut fresh, fspan, " zeta eta theta");
        fresh.resolve_layout(VW, VH);
        fresh.resolve_layout(VW, VH);
        let want = ifc_summary(&fresh, fp);
        assert!(want.1 >= 2, "control: the fresh root wraps ({want:?})");
        assert_eq!(got, want, "whole-document pass on attach {whole_on_attach}");
    }
}
