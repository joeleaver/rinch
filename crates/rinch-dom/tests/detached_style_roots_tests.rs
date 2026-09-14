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
//! # What each fixture kills
//!
//! | fixture | mutant |
//! |---|---|
//! | `a_detached_subtree_is_not_recascaded_to_initial_values` | the connectivity skip removed |
//! | `a_detached_subtree_keeps_its_text_layout` | the skip removed (the shaping half of #668) |
//! | `a_child_styled_before_it_is_spliced_in_does_not_animate` | the skip removed; `has_been_styled` set on a skipped node |
//! | `a_node_skipped_while_detached_is_styled_when_it_attaches` | the skip present, attach not re-resolving |
//! | `a_node_attached_before_the_next_resolve_is_not_skipped` | connectivity tested at push time instead of resolve time |
//! | `a_detached_node_is_still_readable` | a skip that clears the cached style instead of leaving it |

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

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

/// The shaping half of #668: `build_ifc_layouts` collects its roots from the
/// whole slab, so a detached subtree that was recascaded is also **reshaped**,
/// every layout, from values CSS never asked for. With the cascade skipped
/// there is no new style to reshape from, so the old layout survives.
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
/// entry — a filter applied where `set_attribute` pushes would drop it.
#[test]
fn a_node_attached_before_the_next_resolve_is_not_skipped() {
    let mut doc = RinchDocument::new();
    doc.load_css(".w { width: 73px; height: 9px; }");
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.resolve_layout(800.0, 600.0);

    // Build detached, class it detached, splice it in with the low-level
    // `append_child`, then resolve — the entry pushed while it was parentless
    // is the one that must carry the style.
    let child = doc.create_element("div");
    doc.set_attribute(child, "class", "w");
    doc.append_child(host, child);
    doc.resolve_layout(800.0, 600.0);

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
    // The whole-slab serializers walk every node, detached ones included.
    let _ = rinch_dom::testing::serialize_tree_verbose(&doc.tree);
}
