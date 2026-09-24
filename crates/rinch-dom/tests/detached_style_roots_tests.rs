//! #651 / #668 — a `style_roots` entry whose node is not in the document.
//!
//! `set_attribute`, `remove_attribute` and `set_style` all push the node onto
//! `tree.style_roots` and clear its cached Stylo data. The next
//! `resolve_styles` takes that list and, on the targeted path, cascades each
//! entry against `find_parent_computed_style`. For a node that is not
//! connected to the document that walk answers `None`, and Stylo happily
//! cascades against no parent at all: **every inherited property computes to
//! its initial value**, and every descendant selector fails to match because
//! there is no ancestor chain to match against.
//!
//! One mechanism, two consequences, and the fixtures below are split that way:
//!
//! - **Mount (#651).** A component's child is `set_attribute`d, then something
//!   else in the document triggers a resolve before the child is spliced in.
//!   The child is cascaded parentless — so a size that comes from a modifier
//!   class on an ancestor (`.rinch-checkbox--xs .rinch-checkbox__box`) misses,
//!   the base rule's size applies, and `has_been_styled` flips to `true`. The
//!   real resolution after the splice is then a **change** on an already-styled
//!   node, which is exactly what `transition` waits for. A browser never
//!   animates here: an element enters the document already carrying its final
//!   style, so it has no before-change style to transition from.
//!
//! - **Unmount (#668).** A removed subtree keeps its pending entry and is
//!   cascaded detached at the next layout, computing `font-family: serif` and
//!   `color: black`, which `build_ifc_layouts` then shapes its text from.
//!
//! The cure is in `resolve_styles`: skip an entry that is not connected to
//! `tree.root_id`. "Not in the document" is not a question CSS has an answer
//! to, and the answer it was inventing was wrong in both directions.
//!
//! # Mutants, and what kills each
//!
//! Attributions below are **measured**, not assigned — every mutant was applied
//! to a committed revision and the whole file run against it.
//!
//! | mutant | killed by |
//! |---|---|
//! | the skip removed (the base `resolve.rs`) | 5 fixtures |
//! | #651's option 2: cascade detached, withhold `has_been_styled` only | the 4 unmount fixtures; the mount one passes |
//! | a shallow `parent.is_some()` in place of the walk | `a_descendant_of_a_detached_node_is_not_recascaded_either`, `the_document_nodes_own_entry_is_resolved` |
//! | connectivity asked at the push sites instead of here | `a_node_attached_before_the_next_resolve_is_not_skipped`, alone |
//! | the attach route not re-resolving | `a_node_skipped_while_detached_is_styled_when_it_attaches`, alone |
//! | the walk anchored at `html_id` | `a_sibling_of_html_is_connected_because_the_anchor_is_the_document_node`, `the_document_nodes_own_entry_is_resolved` |
//! | the `root_id` self-case dropped | `the_document_nodes_own_entry_is_resolved`, alone |
//! | `has_been_styled` set on a skipped node | **nothing — it survives**, see below |
//!
//! The last two anchor fixtures reach shapes **no rinch code produces** — a node
//! parented to the document node, and an attribute set on the document node
//! itself. They are here because `depth_if_connected`'s choice of anchor and its
//! self-case are both claims its doc makes, and a claim with no witness is what a
//! later "simplification" deletes. Each says in its own doc how it is reached.
//!
//! Two results are worth recording because they are not what a reader would
//! guess.
//!
//! **Setting `has_been_styled` on a skipped node does not bring the animation
//! back**, so no fixture here kills that mutant and none claims to. The skip
//! withholds two things at once: the flag, and a `computed_style` populated
//! from the parentless cascade. A transition needs an old *value* to leave, and
//! a node that was skipped has `width: auto` — not the 20px the parentless
//! cascade used to give it — so `diff_animatable` finds nothing to animate
//! whatever the flag says. The two halves come from the one skip and no small
//! edit separates them.
//!
//! **Issue #651's other suggested cure — keep the parentless cascade and only
//! withhold `has_been_styled` — fixes the mount half and leaves the unmount
//! half exactly as it was.** Measured: the mount fixture passes under it and
//! all four unmount fixtures fail. That is why the skip is in `resolve_styles`
//! rather than at the `has_been_styled` assignment.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// A node's computed `width` in px, or `None` when it is not a length.
///
/// `DimensionValue` has no `PartialEq`, so the two fixtures that read a
/// cascaded width off a node outside the laid-out tree — where `layout.width`
/// is meaningless — go through this rather than a `matches!` per assertion.
fn width_px(doc: &RinchDocument, node: rinch_core::dom::NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

/// A document with `.root { font-family: monospace; color: rgb(0,128,0) }`
/// holding a `panel` with one text child, laid out once.
///
/// Returns `(doc, root, panel, text)`.
fn mounted_panel() -> (
    RinchDocument,
    rinch_core::dom::NodeId,
    rinch_core::dom::NodeId,
    rinch_core::dom::NodeId,
) {
    let mut doc = RinchDocument::new();
    // `line-height` and `font-size` are declared so nothing below is a pin on
    // this host's font set.
    doc.load_css(".root { font-family: monospace; color: rgb(0,128,0); font-size: 16px; line-height: 20px; }");
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "root");
    doc.append_child(body, root);

    let panel = doc.create_element("div");
    let t = doc.create_text("hello");
    doc.append_child(panel, t);
    doc.append_child(root, panel);
    doc.resolve_layout(800.0, 600.0);
    (doc, root, panel, t)
}

/// #668, the issue's own fixture. A removed subtree with a pending
/// `style_roots` entry used to cascade against no parent and land on the
/// initial value of every inherited property.
#[test]
fn a_detached_subtree_is_not_recascaded_to_initial_values() {
    let (mut doc, _root, panel, _t) = mounted_panel();

    let mounted_family = doc
        .tree
        .get(panel.0)
        .unwrap()
        .computed_style
        .font_family
        .clone();
    let mounted_color = doc.tree.get(panel.0).unwrap().computed_style.color;
    assert!(
        mounted_family.contains("monospace"),
        "precondition: the mounted panel inherits monospace, not {mounted_family:?}"
    );
    assert_eq!(
        mounted_color.map(|c| c.to_rgba8().to_u8_array()),
        Some([0, 128, 0, 255]),
        "precondition: the mounted panel inherits the green"
    );

    // Any reactive attribute or style effect leaves a pending entry behind.
    doc.set_attribute(panel, "data-x", "1");
    doc.remove_node(panel);
    doc.resolve_layout(800.0, 600.0);

    let after_family = doc
        .tree
        .get(panel.0)
        .unwrap()
        .computed_style
        .font_family
        .clone();
    let after_color = doc.tree.get(panel.0).unwrap().computed_style.color;
    assert_eq!(
        after_family, mounted_family,
        "a detached node must not be recascaded against no parent \
         (initial `font-family: serif`)"
    );
    assert_eq!(
        after_color, mounted_color,
        "a detached node must not be recascaded against no parent \
         (initial `color: black`)"
    );
}

/// "Connected" means *reachable from the document*, not *has a parent*. A row
/// **inside** a removed panel still has a parent, so a shallow
/// `parent.is_some()` test keeps its entry — and then
/// `find_parent_computed_style` walks to the panel, finds no cached style there
/// (the `set_attribute` on the panel cleared the whole subtree's through
/// `invalidate_descendant_styles`), walks off the top, and cascades the row
/// against `None` exactly as before.
#[test]
fn a_descendant_of_a_detached_node_is_not_recascaded_either() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".root { font-family: monospace; color: rgb(0,128,0); font-size: 16px; \
                 line-height: 20px; }",
    );
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "root");
    doc.append_child(body, root);

    let panel = doc.create_element("div");
    let row = doc.create_element("div");
    let t = doc.create_text("hello");
    doc.append_child(row, t);
    doc.append_child(panel, row);
    doc.append_child(root, panel);
    doc.resolve_layout(800.0, 600.0);

    let mounted = doc
        .tree
        .get(row.0)
        .unwrap()
        .computed_style
        .font_family
        .clone();
    assert!(
        mounted.contains("monospace"),
        "precondition: the mounted row inherits monospace, not {mounted:?}"
    );

    // The panel's own pending entry clears the whole subtree's cached styles;
    // the row then gets an entry of its own, and only then does the panel leave.
    doc.set_attribute(panel, "data-x", "1");
    doc.set_attribute(row, "data-y", "1");
    doc.remove_node(panel);
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(row.0).unwrap().computed_style.font_family,
        mounted,
        "a node whose parent chain does not reach the document is detached too, \
         however many parents it has"
    );
}

/// The shaping half of #668: `build_ifc_layouts` collects its roots from the
/// whole slab, so a detached subtree that was recascaded is **reshaped** from
/// values CSS never asked for.
///
/// **The detached root is still collected and still reshaped after this fix** —
/// that is #628 and this change does not touch it. Instrumented on this very
/// fixture, `build_ifc_layouts` reports `1 roots, 1 detached` on the pass after
/// the removal at both revisions. What changes is the *style* it reshapes from:
/// unchanged rather than invented, so the glyphs come out where they were. The
/// assertion is therefore on the width, not on the layout's survival.
#[test]
fn a_detached_subtree_keeps_its_text_layout() {
    let (mut doc, _root, panel, _t) = mounted_panel();
    assert!(
        doc.tree.get(panel.0).unwrap().text_layout.is_some(),
        "precondition: the mounted panel is the IFC root for its text"
    );
    let mounted_width = doc
        .tree
        .get(panel.0)
        .unwrap()
        .text_layout
        .as_ref()
        .unwrap()
        .layout
        .width();

    doc.set_attribute(panel, "data-x", "1");
    doc.remove_node(panel);
    doc.resolve_layout(800.0, 600.0);

    let after = doc
        .tree
        .get(panel.0)
        .unwrap()
        .text_layout
        .as_ref()
        .map(|l| l.layout.width());
    assert_eq!(
        after,
        Some(mounted_width),
        "a detached subtree must not be reshaped from an invented cascade"
    );
}

/// #651, in the shape every affected component has: a size that comes from a
/// **modifier class on an ancestor**, and a child that is `set_attribute`d
/// before it is spliced under that ancestor.
///
/// `.w--xs .box` is `Checkbox`'s `.rinch-checkbox--xs .rinch-checkbox__box`
/// with the names shortened; 20 → 16 is its real 1.25rem → 1rem.
fn checkbox_shaped_document() -> (RinchDocument, rinch_core::dom::NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".box { width: 20px; height: 20px; transition: width 150ms ease; } \
         .w--xs .box { width: 16px; }",
    );
    let body = doc.body();
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "class", "w--xs");
    doc.append_child(body, wrapper);
    // The first layout is what arms transitions — before it, `resolve_styles`
    // takes the full-walk branch and never reaches a detached node at all.
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.transitions_enabled,
        "precondition: the first layout arms transitions"
    );
    (doc, wrapper)
}

#[test]
fn a_child_styled_before_it_is_spliced_in_does_not_animate() {
    let (mut doc, wrapper) = checkbox_shaped_document();

    // The component builds its box and classes it while it is still parentless.
    let boxel = doc.create_element("div");
    doc.set_attribute(boxel, "class", "box");
    // Appending the check glyph into the box triggers a resolve — the box is
    // still parentless, and its pending entry is drained by that pass. This is
    // the real `Checkbox` order: the box gets its checkmark before the label
    // gets the box.
    let glyph = doc.create_element("span");
    doc.append_child(boxel, glyph);

    // ...and only now is it spliced in, where `.w--xs .box` can finally match.
    doc.append_child(wrapper, boxel);
    doc.resolve_layout(800.0, 600.0);

    assert!(
        doc.tree
            .active_transitions
            .get(&boxel.0)
            .is_none_or(|t| t.is_empty()),
        "a node's first *attached* style is its first style: entering the \
         document must not read as a 20 -> 16 change and animate"
    );
    assert_eq!(
        doc.tree.get(boxel.0).unwrap().layout.width,
        16.0,
        "the box must land on the descendant selector's size immediately"
    );
}

/// The dangerous mutant this fixture exists for: a skip that is never undone.
/// A node whose pending entry was dropped while it was detached must be
/// resolved when it is spliced in — otherwise it is never styled at all.
#[test]
fn a_node_skipped_while_detached_is_styled_when_it_attaches() {
    let mut doc = RinchDocument::new();
    doc.load_css(".late { width: 137px; height: 11px; }");
    let body = doc.body();
    let anchor = doc.create_element("div");
    doc.append_child(body, anchor);
    doc.resolve_layout(800.0, 600.0);

    let late = doc.create_element("div");
    doc.set_attribute(late, "class", "late");
    // Drain the pending entry while `late` is detached.
    let filler = doc.create_element("span");
    doc.append_child(late, filler);
    assert!(
        doc.tree.style_roots.is_empty(),
        "precondition: the append's own resolve drained the pending entry"
    );

    doc.append_child(body, late);
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(late.0).unwrap().layout.width,
        137.0,
        "a node skipped while detached must be styled when it attaches"
    );
}

/// Connectivity is a question about the tree at **resolve** time, not at push
/// time. A node whose entry was pushed while it was detached and which is
/// attached before the next `resolve_styles` must still be resolved by that
/// entry.
///
/// **Two things here are load-bearing and neither is decoration.**
///
/// `suppress_inline_restyle` is the only way to reach the case at all. An
/// ordinary `append_child` calls `recompute_node_styles_recursive` *and
/// resolves inside it*, so the node is styled before control returns and no
/// entry ever survives to a later pass. The flag is `append_child`'s own
/// documented branch for bulk DOM operations; nothing sets it today, which is
/// exactly why the resolve-time spelling matters — it is what keeps that branch
/// safe if anything ever does.
///
/// `neighbour` is a positive control that the pass ran. It used to guard
/// against `resolve_styles`' fallback to a **full tree walk** on an empty
/// `style_roots`, which would have made this fixture pass without the targeted
/// path running at all; that fallback is gone (a full walk now needs
/// `NodeTree::full_style_walk`), and an attribute write no longer records a
/// root at all — it records an invalidation snapshot, which becomes a root at
/// resolve time if some selector's answer changed.
///
/// The child's class is written while it is parentless and unstyled, so no
/// snapshot can be taken; what carries its style is the suppressed insertion,
/// which defers the subtree's cascade by recording it as a style root instead
/// of resolving on the spot.
#[test]
fn a_node_attached_before_the_next_resolve_is_not_skipped() {
    let mut doc = RinchDocument::new();
    doc.load_css(".w { width: 73px; height: 9px; } .n { width: 31px; height: 9px; }");
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    let neighbour = doc.create_element("div");
    doc.append_child(body, neighbour);
    doc.resolve_layout(800.0, 600.0);

    // Classed while parentless: the entry goes in with the node detached.
    let child = doc.create_element("div");
    doc.set_attribute(child, "class", "w");
    // Spliced in without the insertion's own restyle, so that entry is the only
    // thing that can carry the style.
    doc.tree.suppress_inline_restyle = true;
    doc.append_child(host, child);
    doc.tree.suppress_inline_restyle = false;
    // ...alongside a connected entry, so the list cannot empty out.
    doc.set_attribute(neighbour, "class", "n");
    assert!(
        doc.tree.style_roots.contains(&child.0) && doc.has_pending_style_snapshot(neighbour),
        "positive control: the deferred insertion is a root, the class write a snapshot"
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(neighbour.0).unwrap().layout.width,
        31.0,
        "positive control: the connected entry resolved, so the pass ran"
    );
    assert_eq!(
        doc.tree.get(child.0).unwrap().layout.width,
        73.0,
        "an entry pushed while detached must still resolve once the node is in"
    );
}

/// Every reader of a detached node's computed style keeps working. The skip
/// leaves the last resolved style in place rather than clearing it, so a
/// removed node reads back as it last did in the document instead of panicking
/// or answering nothing.
#[test]
fn a_detached_node_is_still_readable() {
    let (mut doc, _root, panel, t) = mounted_panel();
    let mounted_family = doc
        .tree
        .get(panel.0)
        .unwrap()
        .computed_style
        .font_family
        .clone();
    doc.set_attribute(panel, "data-x", "1");
    doc.remove_node(panel);
    doc.resolve_layout(800.0, 600.0);

    // The MCP / debug `get_node` path.
    let detail = rinch_dom::testing::get_node_detail(&doc.tree, panel.0)
        .expect("a detached node must still be reportable");
    assert!(
        detail["computed_styles"]["font_family"].is_string(),
        "a detached node's computed styles must still be reportable"
    );
    // The `query_node_layout` path, on the node and on its text child.
    assert!(doc.query_node_layout(panel.0 as u64).is_some());
    let _ = doc.query_node_layout(t.0 as u64);

    // **Scoping is what decides whether a debug tool reaches a detached node,
    // not the tool.** `query_selector` and an unscoped `dom_tree` both start at
    // `tree.body_id`, so neither finds one; `data-x` is on the panel and
    // nothing else, which is what makes the first assertion exact.
    assert!(
        rinch_dom::testing::query_selector(&doc.tree, "[data-x]").is_empty(),
        "query_selector walks from the body, so it cannot find a detached node"
    );
    let unscoped = rinch_dom::testing::serialize_tree_verbose(&doc.tree).to_string();
    assert!(
        !unscoped.contains("data-x"),
        "an unscoped dom_tree walks from the body too"
    );

    // But `dom_tree` takes a `root_id` and hands it straight to
    // `serialize_tree_full` (`app/debug_commands.rs`), so
    // `dom_tree(root_id: <a detached id>)` **does** reach one — and what it
    // reports is the style the node last resolved to *in* the document, which
    // is the whole of this change. Before it, the same call answered with the
    // parentless cascade's `serif`.
    let scoped = rinch_dom::testing::serialize_tree_full(&doc.tree, None, Some(panel.0), true);
    assert_eq!(
        scoped["attributes"]["data-x"], "1",
        "a root_id-scoped dom_tree does reach a detached node"
    );
    assert_eq!(
        scoped["computed_styles"]["font_family"].as_str(),
        Some(mounted_family.as_str()),
        "and reports the style it last had in the document, not an invented one"
    );
    assert!(
        mounted_family.contains("monospace"),
        "counter-oracle: this says nothing on a host where the document's own \
         family is already the initial `serif` ({mounted_family:?})"
    );
}

/// **The anchor is `tree.root_id`, the document node, not `tree.html_id`** —
/// and this is the shape that tells them apart: a node parented *directly* to
/// the document node, a sibling of `<html>`.
///
/// It is reachable only by handing `append_child` the document node's own id,
/// which nothing in rinch does. The fixture exists anyway, because the choice
/// of anchor is a claim `depth_if_connected`'s doc makes and an unpinned claim
/// is how a later edit "simplifies" one anchor into the other. The claim is
/// that this change **refuses nothing the targeted path used to resolve**:
/// before the connectivity test there was no test at all, so such a node was
/// cascaded, and under the `root_id` anchor it still is. An `html_id` anchor
/// would newly refuse it — silently, since the full-walk branch starts at
/// `html_id` and never reaches it either.
///
/// `anchor`'s write is the positive control that the pass ran. (It also used
/// to keep `style_roots` non-empty so `resolve_styles` could not fall back to a
/// full walk and mask the difference; that fallback is gone.)
#[test]
fn a_sibling_of_html_is_connected_because_the_anchor_is_the_document_node() {
    let mut doc = RinchDocument::new();
    doc.load_css(".x { width: 42px; height: 7px; } .t { width: 13px; height: 7px; }");
    let body = doc.body();
    let anchor = doc.create_element("div");
    doc.append_child(body, anchor);
    doc.resolve_layout(800.0, 600.0);

    let document_node = rinch_core::dom::NodeId(doc.tree.root_id);
    let extra = doc.create_element("div");
    doc.append_child(document_node, extra);
    doc.set_attribute(extra, "class", "x");
    doc.set_attribute(anchor, "class", "t");
    assert!(
        doc.has_pending_style_snapshot(anchor) && doc.has_pending_style_snapshot(extra),
        "positive control: both class writes are pending invalidations"
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        width_px(&doc, anchor),
        Some(13.0),
        "positive control: the connected entry resolved, so the pass ran"
    );
    assert_eq!(
        doc.tree.get(extra.0).unwrap().parent,
        Some(doc.tree.root_id),
        "precondition: the node really does hang off the document node"
    );
    assert_eq!(
        width_px(&doc, extra),
        Some(42.0),
        "a child of the document node reaches the anchor and is still cascaded"
    );
}

/// **The document node's own entry is resolved, not skipped**, which is what
/// `depth_if_connected`'s `node_id == root_id` self-case is for. Without it the
/// walk starts at `root_id`'s parent, finds `None`, and answers "detached" for
/// the one node that is the document.
///
/// No DOM verb records the document node as a style root any more — every
/// route that records one (an insertion, a restyle hint, an invalidation) is
/// element-only — so the fixture writes the entry itself. The self-case is
/// still load-bearing on another path: `detach_subtree_styles_if_moved_out`
/// asks `depth_if_connected` about a move's destination, which is the document
/// node for an `append_child` onto it.
///
/// The observable difference needs a cascade whose answer has *changed*, or
/// re-resolving and not re-resolving look identical. `load_css` merges a
/// stylesheet without invalidating any cached style, so the new `.kid` width
/// sits unused until something recascades; dropping the cached styles under
/// the document node is what asks for that recascade, and the walk from the
/// document node is what reaches it. Skipping the entry leaves `kid` at the old width.
#[test]
fn the_document_nodes_own_entry_is_resolved() {
    let mut doc = RinchDocument::new();
    doc.load_css(".kid { width: 20px; height: 7px; }");
    let body = doc.body();
    let kid = doc.create_element("div");
    doc.set_attribute(kid, "class", "kid");
    doc.append_child(body, kid);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width_px(&doc, kid),
        Some(20.0),
        "precondition: the first rule applied"
    );

    doc.load_css(".kid { width: 33px; height: 7px; }");
    // What `set_attribute` on the document node used to do: drop every
    // element's cached style under it, and record the document node itself.
    for id in [doc.tree.html_id, doc.tree.body_id, kid.0] {
        *doc.tree.nodes[id].stylo_element_data.borrow_mut() = None;
    }
    doc.tree.style_roots.push(doc.tree.root_id);
    doc.tree.styles_dirty = true;
    assert_eq!(
        doc.tree.style_roots.as_slice(),
        &[doc.tree.root_id],
        "positive control: the document node is the only entry, so the targeted \
         path decides the outcome on its own"
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        width_px(&doc, kid),
        Some(33.0),
        "the document node's entry must recascade the tree, not be skipped"
    );
}
