//! #653 — a text node folded into an inline formatting context must not keep
//! a stale Taffy-leaf box left over from before the fold.
//!
//! # The mechanism
//!
//! Every text node gets its own real Taffy leaf at creation (`create_text`,
//! `new_leaf_with_context`), before anything knows whether it will ever join
//! an IFC. `mark_inline_descendants` folds inline-level content into its
//! root's Parley layout by detaching each member's Taffy node from the
//! **outermost root's** Taffy children (`root_taffy_children.contains(&child_taffy)`)
//! — a check against the root passed down through the whole recursion. A
//! direct inline child of the root (a `<span>`) passes that check and is
//! correctly detached. A **text node nested one level deeper**, inside that
//! `<span>`, never does: its own `taffy_id` is attached to the *span's* Taffy
//! node, not the root's, so `root_taffy_children.contains(&text_taffy)` is
//! always false and the detach branch never runs for it. It still gets
//! `ifc_root = Some(root)`, but its Taffy attachment is untouched — it is
//! only *effectively* orphaned because the span above it was detached one
//! level up.
//!
//! That is harmless as long as the text leaf is never computed while still
//! reachable (the common case: on a document's first-ever layout, marking
//! always runs before the first Taffy compute). It stops being harmless the
//! moment something re-attaches the span to its parent's Taffy children
//! without re-running marking (the one-way-detach hazard `ifc_reattach_tests`
//! documents for elements, #597) and a plain Taffy compute reaches the
//! still-attached text leaf, measuring it as an ordinary block child with a
//! real, non-zero box — a box `read_layout_results`' generic
//! `taffy.layout(taffy_id)` fallback, which has no equivalent of the
//! `is_flowed_inline_element`/split-inline/`display:contents` zero-guards for
//! a **text** node, happily copies into `node.layout` on every later pass,
//! stale or not, with no corrector: `write_inline_positions` (the only thing
//! that ever gives a direct-child IFC member a box) only runs when
//! `build_ifc_layouts` decides the root needs reshaping, which it does not
//! once the root's own content and width are unchanged — exactly the #653
//! report's "two text nodes keep or drop a stale box depending on timing".
//!
//! # The fixture
//!
//! A real re-attach trigger could not be pinned down from static reading
//! alone (the issue author didn't find one either). So this fixture reaches
//! the hazardous intermediate state directly, through the same `doc.tree`
//! access `ifc_leaf_invariant_tests.rs` and `ifc_reattach_tests.rs` already
//! use: lay out once (establishing the correct zero box), re-attach the span
//! to its root's Taffy children by hand (standing in for whatever real event
//! undoes the one-way detach), then force a **second** layout pass that runs
//! a real Taffy compute but — by clearing the pending IFC seeds — runs **no**
//! structural setup at all, matching `resolve_layout_inner`'s own "IFC
//! structure unchanged" branch. That is a real, reachable branch of
//! production code, not a fabrication: it is what the "else" arm of the
//! `ifc_dirty` / `!ifc_seeds.is_empty()` match in `resolve_layout_inner`
//! takes on any ordinary pass, and the only thing this fixture adds is making
//! the re-attached subtree still reachable when Taffy's compute runs that
//! pass over it.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::{LayoutResult, RinchDocument};

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn taffy_id_of(doc: &RinchDocument, node: NodeId) -> taffy::NodeId {
    doc.tree.get(node.0).unwrap().taffy_id.unwrap()
}

#[test]
fn a_text_node_already_folded_into_an_ifc_does_not_regain_a_stale_leaf_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    doc.append_child(body, container);

    let span = doc.create_element("span");
    doc.append_child(container, span);
    let text = doc.create_text("Extra small");
    doc.append_child(span, text);

    // First layout: the document's first-ever layout is always a
    // whole-document pass, so marking runs before any compute ever reaches
    // `span`/`text` — the correctly-folded baseline.
    doc.resolve_layout(VW, VH);
    assert_eq!(
        doc.tree.get(text.0).unwrap().ifc_root,
        Some(container.0),
        "the text node must already be a member of its container's IFC"
    );
    assert_eq!(
        doc.tree.get(text.0).unwrap().layout,
        LayoutResult::default(),
        "folded on the very first pass, before any compute could reach it — must be the zero box"
    );

    // Hand-simulate the hazard: re-attach `span` to `container`'s Taffy
    // children, undoing `mark_inline_descendants`'s one-way detach, with
    // `text` still attached to `span` exactly as it always has been (its own
    // detach never happened — the bug this fixture pins). Mark both dirty so
    // Taffy's cache does not just replay the old (zero) answer.
    let container_taffy = taffy_id_of(&doc, container);
    let span_taffy = taffy_id_of(&doc, span);
    let text_taffy = taffy_id_of(&doc, text);
    doc.tree
        .taffy
        .add_child(container_taffy, span_taffy)
        .unwrap();
    let _ = doc.tree.taffy.mark_dirty(span_taffy);
    let _ = doc.tree.taffy.mark_dirty(text_taffy);

    // Force a second pass that runs a real Taffy compute (`layout_dirty`)
    // but no structural setup at all (no seeds, not `ifc_dirty`) — the
    // "IFC structure unchanged" branch `resolve_layout_inner` takes on any
    // ordinary pass whose seeds this one stands in for being dropped or
    // never recorded.
    doc.tree.ifc_seeds.clear();
    doc.tree.ifc_dirty = false;
    doc.tree.layout_dirty = true;
    doc.resolve_layout(VW, VH);

    // `span` is reachable from the root again, so Taffy's block algorithm
    // measured it — and, through it, `text`'s own leaf — as ordinary
    // attached content. Without the fix, `read_layout_results`' generic
    // fallback copies that real, non-zero measurement straight into
    // `text`'s `.layout`, even though `text.ifc_root` still names it a
    // member with no box of its own.
    assert_eq!(
        doc.tree.get(text.0).unwrap().ifc_root,
        Some(container.0),
        "the text node is still, and was always, a member of the IFC"
    );
    assert_eq!(
        doc.tree.get(text.0).unwrap().layout,
        LayoutResult::default(),
        "an IFC member text node must not regain a box from an unmarked compute of its \
         (accidentally reachable) stale Taffy leaf — it has none of its own"
    );
}
