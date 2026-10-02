//! #521 — style resolution's pseudo-element removal detaches a pseudo by its
//! OWN Taffy id only (`style_resolution/resolve.rs`, the `children_to_remove`
//! loop). For a `display: contents` pseudo this is the same defect #515/#517
//! fix at every other mutation site: `sync_display_contents` polyfills
//! `display: contents` by hiding the pseudo's own Taffy node and splicing its
//! generated content's Taffy id directly into the *originating element's*
//! Taffy child list. After the first layout pass the pseudo's own id is not
//! in that list — detaching it is a silent no-op — and the spliced content's
//! Taffy id stays behind, still a real, attached, laid-out Taffy child with
//! no DOM node behind it any more.
//!
//! ## Why the reproduction needs a flex (or grid) originator
//!
//! A `::before`'s generated content is always plain text (one `<span>`
//! wrapping one text node — `style_resolution/pseudo.rs`). Whether that
//! text's `taffy_id` survives the splice as a genuine standing Taffy child
//! depends on what kind of box the originator is:
//!
//! - **Block** (the default): the text is absorbed into the originator's own
//!   inline formatting context. `setup_inline_formatting_contexts` detaches
//!   the text's `taffy_id` from whatever Taffy parent `sync_display_contents`
//!   just gave it, in the very same layout pass that created it — so by the
//!   time any later removal runs, there is nothing left attached for the
//!   buggy own-id-only detach to fail at. Probed directly: a block
//!   originator's Taffy child count is `0` both immediately after creation
//!   and after removal, on fixed AND unfixed code alike — this shape is
//!   *not* evidence of the bug, and is not used as a fixture here.
//! ```text
//! (probed, not asserted — block originators never carry a live spliced
//! child to lose)
//! ```
//! - **Flex/grid**: a flex or grid container's direct children are genuine
//!   Taffy items, not IFC members, and nothing detaches a direct text
//!   child's `taffy_id` from the container the way IFC setup does for a
//!   block. The spliced text stays a real, standing Taffy child — this is
//!   the shape that actually lets the own-id-only detach leave a ghost
//!   behind.
//!
//! Measured against unfixed `main` (reverting the `taffy_detach_contribution`
//! swap back to the own-id-only `taffy_remove_child_safe` call): the ghost
//! Taffy child survives the removal — `taffy_child_count` stays `1` instead
//! of dropping to `0` — and **`RINCH_TREE_CHECK=1` does not catch it**. The
//! ghost is a Taffy node still correctly parented by a live originator, still
//! claimed by exactly one parent, with no live DOM node behind it at all —
//! `taffy_tree_violations`' own walk is keyed on *live DOM nodes'* `taffy_id`s
//! (see `ifc.rs::taffy_tree_violations`, "a node outside `parents` has no DOM
//! identity and is never asked about"), so a leaked node with no DOM identity
//! at all is outside what that invariant can ever see. This is the probe that
//! established that; the assertion below is a direct Taffy-tree read instead.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

const BEFORE_CONTENTS: &str = ".w::before { content: \"x\"; display: contents }";

fn taffy_child_count(doc: &RinchDocument, node: NodeId) -> usize {
    let taffy_id = doc.tree.get(node.0).unwrap().taffy_id.unwrap();
    doc.tree.taffy.children(taffy_id).unwrap().len()
}

fn has_pseudo_child(doc: &RinchDocument, owner: NodeId) -> bool {
    doc.tree
        .get(owner.0)
        .unwrap()
        .children
        .iter()
        .any(|&c| doc.tree.get(c).is_some_and(|n| n.is_pseudo_element))
}

/// A flex originator's `::before { display: contents }` generates text that
/// stays a genuine, standing Taffy child (not absorbed by an inline
/// formatting context the way it would be under a block originator).
/// Removing the rule's match entirely (`had_pseudo && !has_pseudo` — the
/// generated content goes away and does not come back, the case the call
/// site's own comment names) must free that child along with the
/// pseudo-element's own DOM node.
///
/// Kills: reverting the `taffy_detach_contribution` swap back to the
/// own-id-only `taffy_remove_child_safe` call — that mutant is exactly
/// unfixed `main`, and is the mutant this test is written against (verified
/// directly: it leaves `taffy_child_count` at `1`, not `0`).
#[test]
fn removing_a_display_contents_before_rule_frees_its_spliced_taffy_child() {
    let mut doc = RinchDocument::new();
    doc.load_css(BEFORE_CONTENTS);
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    doc.set_attribute(w, "style", "display: flex;");
    doc.append_child(body, w);

    doc.resolve_layout(VW, VH);
    assert!(
        has_pseudo_child(&doc, w),
        "sanity: the ::before rule matched and generated a pseudo child"
    );
    assert_eq!(
        taffy_child_count(&doc, w),
        1,
        "sanity: display:contents splices the generated text directly into \
         the originator's own Taffy child list, and a flex originator keeps \
         it as a genuine standing item"
    );

    // Change the class so `.w::before` no longer matches at all — the
    // generated content goes away and does not come back.
    doc.set_attribute(w, "class", "off");
    doc.resolve_layout(VW, VH);

    assert!(
        !has_pseudo_child(&doc, w),
        "sanity: the pseudo-element's own DOM node was freed"
    );
    assert_eq!(
        taffy_child_count(&doc, w),
        0,
        "the spliced generated-content leaf must be freed along with the \
         pseudo-element's own node — on unfixed main this stays 1: a ghost \
         Taffy child with no DOM node behind it, laid out and painted \
         forever"
    );
}

/// The same toggle, but the pseudo is REPLACED rather than removed outright
/// (a second `::before` rule, also `display: contents`, matches instead of
/// the first). `had_pseudo && has_pseudo` is both true here — the new pseudo
/// is itself a live `display: contents` descendant, so the very next
/// structural pass's `sync_display_contents` rebuilds the originator's Taffy
/// child list from the DOM from scratch and silently heals the stale detach
/// on its own. Kept as the negative control: it passes on fixed AND unfixed
/// code alike, which is exactly why the removal-with-nothing-replacing-it
/// shape above — where nothing is left to trigger that self-heal — is the
/// one that actually needs the fix.
#[test]
fn replacing_a_display_contents_before_rule_self_heals_via_the_next_structural_pass() {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "{BEFORE_CONTENTS}\n.w.two::before {{ content: \"yy\"; display: contents }}"
    ));
    let body = doc.body();
    let w = doc.create_element("div");
    doc.set_attribute(w, "class", "w");
    doc.set_attribute(w, "style", "display: flex;");
    doc.append_child(body, w);

    doc.resolve_layout(VW, VH);
    assert_eq!(taffy_child_count(&doc, w), 1, "sanity: one spliced leaf");

    doc.set_attribute(w, "class", "w two");
    doc.resolve_layout(VW, VH);

    assert!(has_pseudo_child(&doc, w), "sanity: the new pseudo generated");
    assert_eq!(
        taffy_child_count(&doc, w),
        1,
        "exactly one spliced leaf survives — the new pseudo's — because the \
         new pseudo is itself a live display:contents descendant and the \
         next structural pass rebuilds the originator's Taffy child list \
         from scratch; this passes on both fixed and unfixed main and is not \
         evidence either way on its own"
    );
}
