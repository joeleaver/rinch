//! #520 — `sync_display_contents` must rebuild parents whose
//! contents-descendant status changed in EITHER direction.
//!
//! The splice is a one-way street on unfixed main: a wrapper toggled from
//! `display: contents` to a box display leaves its children spliced into the
//! grandparent's Taffy child list while its own box is never re-attached —
//! the tree is wrong from that moment, before any node is removed. The fix
//! tracks the splice per node (`Node::contents_spliced`), rebuilds departed
//! wrappers and their flattening ancestors, and gates
//! `taffy_detach_contribution` on the flag as well as computed display (a
//! wrapper restyled away from `contents` and detached before the next layout
//! pass computes a box display while its children still sit spliced).
//!
//! Fixture discipline (per the #515/#517 reviews):
//! - wrappers carry TWO children or are NESTED — a one-child wrapper is the
//!   identity value where several plausible mutants agree;
//! - every toggle test has a PLAIN-node twin — a wrapper that was never
//!   `contents` must be unaffected by the departed handling, otherwise an
//!   over-aggressive mutant is invisible to CI;
//! - intermediate states are asserted, not just the final one — a
//!   toggle-and-toggle-back fixture ends where broken code also ends.
//!
//! Every "Kills:" line below names a mutant that was ACTUALLY APPLIED to the
//! fixed source, with this test observed failing, then reverted. 9 of the 12
//! tests also fail on unfixed origin/main (1d505d8); the three that pass
//! there are the designed guards (`block_wrapper_toggled_to_contents_splices`
//! and the two plain twins) and say so in their own docs.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn height_of(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.height
}

fn flex_column(doc: &mut RinchDocument, height: f32) -> NodeId {
    let body = doc.body();
    child_of(
        doc,
        body,
        &format!("display: flex; flex-direction: column; height: {height}px;"),
    )
}

/// Replace the node's whole inline style. `set_attribute` (not `set_style`)
/// so the toggle is a clean replacement, not a shorthand/longhand merge.
fn restyle(doc: &mut RinchDocument, node: NodeId, style: &str) {
    doc.set_attribute(node, "style", style);
}

// ---------------------------------------------------------------------------
// The toggle itself (#520's core defect)
// ---------------------------------------------------------------------------

/// A spliced wrapper toggled `contents → flex` must get its box back and its
/// children must stop contributing to the grandparent.
///
/// Kills: departed detection removed from the sync scan; the departed
/// wrapper's own child-list rebuild dropped; the departed ancestor walk
/// dropped (this fixture has no OTHER contents node keeping the column in
/// the affected set — the nested fixture below does, and misses that
/// mutant); `truncate(1)` on the rebuilt child list.
#[test]
fn toggle_to_box_gives_box_back_and_unsplices() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, a1), 100.0, "sanity: three-way split");
    assert_eq!(height_of(&doc, b), 100.0, "sanity: three-way split");

    restyle(
        &mut doc,
        wrapper,
        "display: flex; flex-direction: column; flex: 1;",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, b),
        150.0,
        "the wrapper's box must be back in the column (100 = children still spliced)"
    );
    assert_eq!(
        height_of(&doc, wrapper),
        150.0,
        "the wrapper must own a real box again"
    );
    assert_eq!(
        height_of(&doc, a1),
        75.0,
        "children must lay out inside the wrapper again (100 = still spliced, 0 = lost)"
    );
    assert_eq!(height_of(&doc, a2), 75.0, "second child likewise");
}

/// The issue's measurement: toggle a spliced wrapper to a box display, let a
/// layout pass run, then `remove_child` it — the survivor must get the full
/// column. On unfixed main the intermediate pass never heals the splice, so
/// the removal detaches an id that is not in the parent's list and the
/// phantoms keep 200 of the 300px.
///
/// Kills: the `contents_spliced` flag never being set; the helper's own-id
/// removal dropped. Deliberately NOT claimed against the sync half alone:
/// with sync broken but the flag-gated helper intact, the detach itself
/// heals — the two layers are redundant on this path, and that redundancy
/// is the design.
#[test]
fn toggle_to_box_then_remove_frees_slots() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "sanity: three-way split");

    restyle(
        &mut doc,
        wrapper,
        "display: flex; flex-direction: column; flex: 1;",
    );
    doc.resolve_layout(VW, VH);

    doc.remove_child(col, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        300.0,
        "survivor must claim the whole column (100 = both phantoms left, 150 = one)"
    );
}

/// Toggle, then detach BEFORE any layout pass — with the restyle flushed
/// eagerly by an unrelated `append_child` (its
/// `recompute_node_styles_recursive` resolves every pending style root), so
/// the wrapper computes a box display while its children still sit spliced.
/// Computed display alone would say "nothing was spliced" — the exact hole in
/// the #517 gate premise that #520 names.
///
/// Kills: the `contents_spliced` flag never being set (with the flag gone,
/// BOTH healing layers vanish and the phantom is permanent). Either single
/// layer alone — the flag-gated detach or the departed sync rebuild on the
/// slab-surviving wrapper — passes this test; the `set_inner_html` variant
/// below is the one only the detach layer can save.
#[test]
fn toggle_then_remove_before_layout_frees_slots() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "sanity: three-way split");

    restyle(&mut doc, wrapper, "display: block;");
    // Eagerly flush the pending restyle: the wrapper now COMPUTES block while
    // its children are still spliced into the column's Taffy list.
    let body = doc.body();
    let _unrelated = child_of(&mut doc, body, "height: 10px;");

    doc.remove_child(col, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        300.0,
        "survivor must claim the whole column (100 = both phantoms left)"
    );
}

/// Same window, but the detach path frees the slab (`set_inner_html`'s
/// child-clearing calls `remove_subtree`), so no later sync pass can heal —
/// the detach itself must remove the spliced ids or they leak forever.
///
/// Kills: the helper's flag gate reverted to display-only; the flag never
/// being set; the helper's own-id removal dropped.
#[test]
fn toggle_then_set_inner_html_clear_frees_slots() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let _b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);

    restyle(&mut doc, wrapper, "display: block;");
    // Eager flush as above: wrapper computes block, splice still in place.
    let body = doc.body();
    let _unrelated = child_of(&mut doc, body, "height: 10px;");

    doc.set_inner_html(col, "");
    let c1 = child_of(&mut doc, col, "flex: 1;");
    let c2 = child_of(&mut doc, col, "flex: 1;");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, c1),
        150.0,
        "fresh children must split the column two ways (75 = both phantoms left)"
    );
    assert_eq!(height_of(&doc, c2), 150.0, "second fresh child likewise");
}

// ---------------------------------------------------------------------------
// The reverse toggle (box → contents)
// ---------------------------------------------------------------------------

/// A FLEX wrapper toggled to `contents` must splice. `Contents` maps to
/// `taffy::Display::Flex`, so the Taffy-style comparison in
/// `apply_stylo_styles_to_taffy` sees no change for this toggle — without the
/// computed-display crossing check nothing sets `ifc_dirty` and the toggle is
/// silently ignored.
///
/// Kills: the crossing check removed from `apply_stylo_styles_to_taffy`.
#[test]
fn flex_wrapper_toggled_to_contents_splices() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(
        &mut doc,
        col,
        "display: flex; flex-direction: column; flex: 1;",
    );
    let a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, a1), 75.0, "sanity: children inside wrapper");
    assert_eq!(height_of(&doc, b), 150.0, "sanity: two-way split");

    restyle(&mut doc, wrapper, "display: contents");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, a1),
        100.0,
        "children must join the column directly (75 = toggle silently ignored)"
    );
    assert_eq!(height_of(&doc, a2), 100.0, "second child likewise");
    assert_eq!(
        height_of(&doc, b),
        100.0,
        "sibling must share three ways (150 = toggle silently ignored)"
    );
}

/// A BLOCK wrapper toggled to `contents` splices (Block → Flex differs in
/// Taffy, so this direction worked before #520). Regression guard for the
/// sync restructure: entering the affected set must keep working.
#[test]
fn block_wrapper_toggled_to_contents_splices() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: block; height: 30px;");
    let a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        270.0,
        "sanity: block wrapper takes 30px"
    );

    restyle(&mut doc, wrapper, "display: contents");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, a1), 100.0, "children join the column");
    assert_eq!(height_of(&doc, a2), 100.0, "children join the column");
    assert_eq!(height_of(&doc, b), 100.0, "three-way split after splice");
}

/// `contents → none` must hide the children with the wrapper. Both displays
/// stamp `taffy::Display::None` on the wrapper's own Taffy node, so the
/// Taffy-style comparison cannot flag the crossing, and the spliced children
/// are not the wrapper's Taffy children — hiding the wrapper's box does not
/// hide them. Only the crossing check + departed rebuild reach this state.
///
/// Kills: the crossing check removed; departed detection removed from the
/// sync scan; the flag never being set.
#[test]
fn contents_toggled_to_none_hides_spliced_children() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "sanity: three-way split");

    restyle(&mut doc, wrapper, "display: none;");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, b),
        300.0,
        "a display:none wrapper hides its children (100 = phantoms still laid out)"
    );
}

// ---------------------------------------------------------------------------
// Nested wrappers
// ---------------------------------------------------------------------------

/// Toggling only the INNER of two nested contents wrappers: the departed
/// node's flattening ancestor lies BEHIND the still-contents outer wrapper,
/// so the ancestor walk must skip through it, and the grandparent's rebuild
/// must flatten the outer wrapper around the inner one's restored box.
///
/// Kills: departed detection removed; the departed wrapper's own child-list
/// rebuild dropped; the flag never being set; `truncate(1)` on the rebuilt
/// list. (The dropped-ancestor-walk mutant survives HERE — the
/// still-contents outer wrapper keeps the column in the affected set — which
/// is why `toggle_to_box_gives_box_back_and_unsplices` exists un-nested.)
#[test]
fn nested_wrappers_inner_toggle_heals_through_outer() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let outer = child_of(&mut doc, col, "display: contents");
    let inner = child_of(&mut doc, outer, "display: contents");
    let d1 = child_of(&mut doc, inner, "flex: 1;");
    let d2 = child_of(&mut doc, inner, "flex: 1;");
    let a3 = child_of(&mut doc, outer, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    for (n, name) in [(d1, "d1"), (d2, "d2"), (a3, "a3"), (b, "b")] {
        assert_eq!(height_of(&doc, n), 75.0, "sanity: four-way split ({name})");
    }

    restyle(
        &mut doc,
        inner,
        "display: flex; flex-direction: column; flex: 1;",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, inner), 100.0, "inner box joins the column");
    assert_eq!(height_of(&doc, a3), 100.0, "outer's other child re-splices");
    assert_eq!(height_of(&doc, b), 100.0, "sibling shares three ways");
    assert_eq!(height_of(&doc, d1), 50.0, "grandchildren back inside inner");
    assert_eq!(height_of(&doc, d2), 50.0, "grandchildren back inside inner");
}

/// BOTH nested wrappers toggled away from `contents` in one flush, then the
/// parent cleared via `set_inner_html` (slab-freeing). The detach must
/// flatten through the inner wrapper by its SPLICED state, not its current
/// display — the current effective children of the outer wrapper are
/// `[inner, a3]`, but the ids sitting in the column's list are the
/// grandchildren's.
///
/// Kills: `collect_taffy_detach_candidates` swapped for the narrow
/// `collect_effective_taffy_children` (this is the ONLY test that catches
/// that mutant); the helper's flag gate reverted to display-only; the flag
/// never being set; the own-id removal dropped.
#[test]
fn nested_wrappers_both_toggled_then_clear_frees_all_slots() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let outer = child_of(&mut doc, col, "display: contents");
    let inner = child_of(&mut doc, outer, "display: contents");
    let _d1 = child_of(&mut doc, inner, "flex: 1;");
    let _d2 = child_of(&mut doc, inner, "flex: 1;");
    let _a3 = child_of(&mut doc, outer, "flex: 1;");
    let _b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);

    restyle(&mut doc, outer, "display: block;");
    restyle(&mut doc, inner, "display: block;");
    // Eager flush: both wrappers compute block, splice still in place.
    let body = doc.body();
    let _unrelated = child_of(&mut doc, body, "height: 10px;");

    doc.set_inner_html(col, "");
    let c1 = child_of(&mut doc, col, "flex: 1;");
    let c2 = child_of(&mut doc, col, "flex: 1;");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, c1),
        150.0,
        "fresh children split two ways (60 = all three phantoms left, 75 = two)"
    );
    assert_eq!(height_of(&doc, c2), 150.0, "second fresh child likewise");
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

/// contents → box → contents across three layout passes. The middle state is
/// asserted — broken code agrees with correct code on the final state (the
/// re-splice rebuilds the same list), so without the middle assert this
/// fixture would pass vacuously.
///
/// Kills (via the middle asserts): departed detection removed; the own
/// child-list rebuild dropped; the departed ancestor walk dropped;
/// `truncate(1)`. Kills (via the re-splice leg, which is flex → contents):
/// the crossing check removed.
#[test]
fn toggle_round_trip_re_splices() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "sanity: spliced");

    restyle(
        &mut doc,
        wrapper,
        "display: flex; flex-direction: column; flex: 1;",
    );
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 150.0, "middle state: wrapper boxed");
    assert_eq!(height_of(&doc, a1), 75.0, "middle state: children inside");

    restyle(&mut doc, wrapper, "display: contents");
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, a1), 100.0, "re-spliced");
    assert_eq!(height_of(&doc, a2), 100.0, "re-spliced");
    assert_eq!(height_of(&doc, b), 100.0, "re-spliced");
}

// ---------------------------------------------------------------------------
// Plain-node twins — a wrapper that was NEVER `contents` must be unaffected
// ---------------------------------------------------------------------------

/// Twin of the toggle+remove tests: an ordinary display toggle on a plain
/// wrapper, then removal. Sibling geometry must be exactly what a plain
/// removal gives.
///
/// Kills: the helper's own-id removal dropped — the #519 blind-spot mutant,
/// re-verified against the reshaped helper.
#[test]
fn plain_wrapper_display_toggle_then_remove_twin() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(
        &mut doc,
        col,
        "display: flex; flex-direction: column; flex: 1;",
    );
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 150.0, "sanity: two-way split");

    restyle(&mut doc, wrapper, "display: block; flex: 1;");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        150.0,
        "toggle alone changes nothing here"
    );

    doc.remove_child(col, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        300.0,
        "plain removal frees exactly the wrapper's own slot"
    );
}

/// Twin of the `set_inner_html` test: clearing a column whose wrapper was
/// never `contents` (restyle flushed eagerly the same way) must leave fresh
/// children a clean two-way split.
///
/// Kills: the helper's own-id removal dropped.
#[test]
fn plain_wrapper_clear_via_set_inner_html_twin() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(
        &mut doc,
        col,
        "display: flex; flex-direction: column; flex: 1;",
    );
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let _b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);

    restyle(&mut doc, wrapper, "display: block; flex: 1;");
    let body = doc.body();
    let _unrelated = child_of(&mut doc, body, "height: 10px;");

    doc.set_inner_html(col, "");
    let c1 = child_of(&mut doc, col, "flex: 1;");
    let c2 = child_of(&mut doc, col, "flex: 1;");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c1), 150.0, "clean two-way split");
    assert_eq!(height_of(&doc, c2), 150.0, "clean two-way split");
}
