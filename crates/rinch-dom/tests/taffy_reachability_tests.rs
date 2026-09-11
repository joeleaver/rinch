//! #589 — `taffy_tree_violations` asks a *global* question now, and says what
//! its exemptions cost.
//!
//! Invariant `C` asked *"does this node have a Taffy parent?"*. Every node in a
//! subtree that hangs off nothing answers yes, so a whole branch could be laid
//! out by no compute pass with every validator in the crate reporting `[]` — a
//! flex column measuring `0` and its content drawn nowhere, measured in #589
//! against #585's mutant. `D` replaces the local test with membership of the
//! set reachable from a root layout actually computes from; `C` survives as the
//! trivial case (no Taffy parent at all).
//!
//! `E` is the other half. `C` and `D` exempt `display: none` and
//! `display: contents` *because those elements generate no box* — a claim with
//! a consequence, so `E` checks the consequence. #543 shipped as exactly that:
//! a closed `DropdownMenu` that kept a `160x168` box, stayed painted and stayed
//! clickable, while every validator here answered `[]` because the node was
//! exempt from the only rule that looked at it.
//!
//! # Checker fixtures are labelled as such
//!
//! Five of the seven fixtures below corrupt the tree **by hand** — they move a
//! Taffy edge or write a `layout` field — because the states they pin are ones
//! the producer no longer reaches: #589's needs #585's mutant and #543's needs
//! #594 reverted.
//! They test the *validator*, not the engine, and each says so in its own doc so
//! a later mutation campaign cannot mistake one for a producer pin. The producer
//! witnesses are named in each: the walk-removal mutant on
//! `anon_box_contents_flatten_tests`, and the fifteen live `D detached` lines
//! #513 prints across this suite.
//!
//! Two fixtures are **not** checkers and are the more important half:
//! `an_inline_block_measured_as_its_own_root_is_not_reported` and
//! `inline_content_keeps_a_real_box_and_is_not_a_ghost` are ordinary documents
//! that the rules must stay quiet about, and they are what fails if either
//! refinement is dropped.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn taffy_id(doc: &RinchDocument, id: NodeId) -> taffy::NodeId {
    doc.tree
        .get(id.0)
        .and_then(|n| n.taffy_id)
        .unwrap_or_else(|| panic!("node {id:?} has no Taffy node"))
}

/// The violations, one per line, for an assertion message.
fn lines(doc: &RinchDocument) -> String {
    doc.taffy_tree_violations().join("\n  ")
}

/// Move `child`'s Taffy edge under `new_parent`, leaving everything below it
/// intact — the shape #589 is about, built by hand.
fn reparent_taffy(doc: &mut RinchDocument, child: NodeId, new_parent: NodeId) {
    let c = taffy_id(doc, child);
    let p = taffy_id(doc, new_parent);
    if let Some(old) = doc.tree.taffy.parent(c) {
        doc.tree.taffy.remove_child(old, c).unwrap();
    }
    doc.tree.taffy.add_child(p, c).unwrap();
}

// ── D: the chain of valid edges that hangs off nothing ──────────────────────

/// **Checker.** A block whose Taffy parent is a `display: contents` node that is
/// itself in nobody's child list — #589's exact structure, reached here by
/// moving one Taffy edge rather than by breaking the engine.
///
/// The two assertions are a pair and the second is the one that matters: the
/// subtree is reported (`D`), and it is reported **by `D` and not by `C`**,
/// because `C`'s question is still `Some(parent)` at every link. Dropping the
/// second would let a `C`-only implementation pass.
///
/// The producer witness for this rule is elsewhere and is a mutant, not a
/// fixture: `taffy_child_list_owners` reduced to `vec![node_id]` (#585) puts a
/// real document into this state, and
/// `anon_box_contents_flatten_tests::a_container_restyled_to_contents_*` fail on
/// it through `assert_consistent` — which they could not do before this rule.
#[test]
fn a_chain_hanging_off_a_contents_node_is_reported_as_detached() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let wrap = el(&mut doc, col, "div", "display: contents");
    let blk = el(&mut doc, wrap, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);
    assert!(
        doc.taffy_tree_violations().is_empty(),
        "precondition: a flattened contents wrapper is clean:\n  {}",
        lines(&doc)
    );

    // The wrapper's own Taffy node is detached by `sync_display_contents` and
    // its child is hoisted. Put the child back under it: every edge is valid,
    // the wrapper is exempt from C, and nothing computes the branch.
    reparent_taffy(&mut doc, blk, wrap);

    let v = doc.taffy_tree_violations();
    assert!(
        v.iter().any(|l| l.starts_with("D detached")),
        "the stranded block must be reported:\n  {}",
        v.join("\n  ")
    );
    assert!(
        !v.iter().any(|l| l.starts_with("C orphan")),
        "and C cannot see it — every link in the chain has a Taffy parent, \
         which is the whole of #589:\n  {}",
        v.join("\n  ")
    );
    // The other validator is blind by construction: the DOM tree is untouched.
    // Asserted rather than assumed, because "every check in the crate answered
    // `[]`" is the claim #589 rests on.
    assert!(
        doc.dom_tree_violations().is_empty(),
        "the DOM tree is intact — only the Taffy tree is broken:\n  {}",
        doc.dom_tree_violations().join("\n  ")
    );
}

/// **Checker.** `C` is unchanged for the trivial case, and is still spelled
/// `C orphan` — #584 refers to that string.
///
/// Here the node has no Taffy parent at all, which is also "unreachable", so a
/// D-only implementation would report it under the wrong name and silently
/// invalidate #584's triage.
#[test]
fn a_node_with_no_taffy_parent_is_still_reported_as_an_orphan() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let blk = el(&mut doc, col, "div", "height: 30px");
    doc.resolve_layout(VW, VH);
    assert!(doc.taffy_tree_violations().is_empty(), "precondition");

    let t = taffy_id(&doc, blk);
    let p = doc.tree.taffy.parent(t).expect("attached");
    doc.tree.taffy.remove_child(p, t).unwrap();

    let v = doc.taffy_tree_violations();
    assert!(
        v.iter().any(|l| l.starts_with("C orphan")),
        "an orphan keeps its own message:\n  {}",
        v.join("\n  ")
    );
}

/// **Checker.** One defect strands a branch; the report names the top of it once.
///
/// Without suppression this prints a line for the block *and* one for its child.
///
/// **This fixture is currently suppression's only witness, and that is why it is
/// written by hand.** On `1a60722` the whole-suite count is 15 either way: every
/// live `D detached` line is a #513 shape whose stranded nodes are siblings, so
/// there is no ancestor to suppress from. The suite last showed the difference
/// on `db9c64f` (36 lines against 26, and 104 against 66 with the seed set
/// wrong). Losing the fixture would leave the refinement with no test at all.
/// The cost is stated in the validator's doc: a second, independent detachment
/// inside an already-reported subtree is not separately named.
#[test]
fn only_the_topmost_detached_node_is_named() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = el(&mut doc, body, "div", "width: 400px");
    let wrap = el(&mut doc, col, "div", "display: contents");
    let blk = el(&mut doc, wrap, "div", "height: 30px");
    let inner = el(&mut doc, blk, "div", "height: 10px");
    doc.resolve_layout(VW, VH);
    assert!(doc.taffy_tree_violations().is_empty(), "precondition");
    let inner_t = taffy_id(&doc, inner);

    reparent_taffy(&mut doc, blk, wrap);

    let v = doc.taffy_tree_violations();
    assert_eq!(
        v.len(),
        1,
        "the branch is one defect and gets one line:\n  {}",
        v.join("\n  ")
    );
    assert!(
        !v[0].contains(&format!("dom {}", inner.0)),
        "and the line names the top of the branch, not a node inside it: {}",
        v[0]
    );
    // The child really is unreachable — so the single line is suppression, not
    // an accident of it being fine.
    assert!(
        doc.tree.taffy.parent(inner_t).is_some(),
        "precondition for the claim above: the child kept its Taffy parent"
    );
}

// ── D: the second legitimate root, which is why naive reachability over-reports

/// **Producer, and the one that fails if the seed set is wrong.**
///
/// An inline-block inside an IFC is detached from its parent's Taffy tree on
/// purpose — Parley places it — and `measure_inline_blocks` computes it as a
/// Taffy **root** of its own. Its subtree is therefore laid out correctly while
/// being unreachable from the document root, which is a third legitimate
/// category of unreachable node beside `display: none` and `display: contents`.
///
/// Seeding reachability only from the document root reports this document.
/// Across `-p rinch-dom -p rinch` on `1a60722` it takes the sweep from 15 lines
/// to 77, every one of them this shape. On `db9c64f` it did something worse as
/// well, which is the reason the two refinements are ordered rather than
/// independent: it *masked* 10 real `C orphan`s behind D lines their ancestors
/// had only earned because the seeding was wrong. Nothing is corrupted here; it
/// is an ordinary paragraph with a button-shaped thing in it.
#[test]
fn an_inline_block_measured_as_its_own_root_is_not_reported() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = el(
        &mut doc,
        body,
        "p",
        "width: 400px; font-size: 16px; line-height: 20px",
    );
    txt(&mut doc, p, "before ");
    let ib = el(&mut doc, p, "span", "display: inline-block; padding: 4px");
    let inner = el(&mut doc, ib, "div", "width: 30px; height: 30px");
    txt(&mut doc, p, " after");
    doc.resolve_layout(VW, VH);

    // The precondition that makes the assertion mean anything: the inner block
    // genuinely is unreachable from the document root's Taffy node.
    let root_t = taffy_id(&doc, NodeId(doc.tree.root_id));
    let mut cur = Some(taffy_id(&doc, inner));
    let mut reaches_root = false;
    while let Some(t) = cur {
        if t == root_t {
            reaches_root = true;
            break;
        }
        cur = doc.tree.taffy.parent(t);
    }
    assert!(
        !reaches_root,
        "precondition: the inline-block's subtree hangs off the inline-block, \
         not off the document root — otherwise this fixture proves nothing"
    );

    assert!(
        doc.taffy_tree_violations().is_empty(),
        "an inline-block is computed as a root of its own, so its subtree is \
         laid out by a real pass and must not be reported:\n  {}",
        lines(&doc)
    );
}

// ── E: an element that generates no box must not be carrying one ────────────

/// **Checker.** #543's ghost, restated as an invariant.
///
/// The producer is gone — #594 zeroes a hidden subtree in `read_layout_results`
/// — so the state is written by hand here. The mutant that reproduces it for
/// real is that zeroing removed, measured while landing this rule.
///
/// It matters that this is `display: none`, not merely "some node with a stale
/// box": `C` and `D` skip the node *because it generates no box*, and `E` is the
/// consequence of that exemption rather than a new claim about the tree.
#[test]
fn a_hidden_element_that_kept_its_box_is_reported_as_a_ghost() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = el(&mut doc, body, "div", "display: none; width: 160px");
    doc.resolve_layout(VW, VH);
    assert!(
        doc.taffy_tree_violations().is_empty(),
        "precondition: a hidden element carries no box today (#543 / #594):\n  {}",
        lines(&doc)
    );

    doc.tree.nodes[panel.0].layout.width = 160.0;
    doc.tree.nodes[panel.0].layout.height = 168.0;

    let v = doc.taffy_tree_violations();
    assert!(
        v.iter().any(|l| l.starts_with("E ghost box")),
        "a `display: none` element holding a 160x168 box is the shipped bug:\n  {}",
        v.join("\n  ")
    );
}

/// **Checker.** The same for the other boxless value.
///
/// `read_layout_results`' `Contents` branch zeroes these for a different reason
/// (a detached wrapper's stale `location` would be double-counted onto every
/// descendant), so the two exemptions are guarded by two separate pieces of
/// production code and `E` is what notices if either stops holding.
#[test]
fn a_contents_element_that_kept_its_box_is_reported_as_a_ghost() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrap = el(&mut doc, body, "div", "display: contents");
    el(&mut doc, wrap, "div", "height: 30px");
    doc.resolve_layout(VW, VH);
    assert!(doc.taffy_tree_violations().is_empty(), "precondition");

    doc.tree.nodes[wrap.0].layout.y = 12.0;

    let v = doc.taffy_tree_violations();
    assert!(
        v.iter().any(|l| l.starts_with("E ghost box")),
        "a boxless wrapper with an origin is a box:\n  {}",
        v.join("\n  ")
    );
}

/// **Producer, and the reason `E` stops where it does.**
///
/// `C` and `D` have a third exemption — `ifc_root.is_some()` — and it is *not*
/// "generates no box". Inline content carries a real box that the IFC assigns
/// (`write_inline_positions`), not Taffy. So widening `E` to the whole exempt
/// set would fire on every inline-block in the library, and this fixture is what
/// says so.
///
/// The consequence is a real blind spot rather than a tidy boundary, and it is
/// named in the validator's doc: a node whose `ifc_root` is **stale** is exempt
/// from `C` and `D` and invisible to `E`. That is #597's intermediate state, and
/// a clean sweep is no evidence against it.
#[test]
fn inline_content_keeps_a_real_box_and_is_not_a_ghost() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = el(
        &mut doc,
        body,
        "p",
        "width: 400px; font-size: 16px; line-height: 20px",
    );
    txt(&mut doc, p, "before ");
    let ib = el(
        &mut doc,
        p,
        "span",
        "display: inline-block; width: 40px; height: 24px",
    );
    doc.resolve_layout(VW, VH);

    let r = doc.tree.get(ib.0).unwrap().layout;
    assert!(
        r.width > 0.0 && r.height > 0.0,
        "precondition: IFC-owned content really does carry a box, got {r:?}"
    );
    assert!(
        doc.tree.get(ib.0).unwrap().ifc_root.is_some(),
        "precondition: and it is exempt from C/D for the `ifc_root` reason"
    );
    assert!(
        doc.taffy_tree_violations().is_empty(),
        "so E must not be widened to the whole exempt set:\n  {}",
        lines(&doc)
    );
}
