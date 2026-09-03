//! The IFC leaf invariant (#466).
//!
//! Taffy 0.12 consults a node's measure function only when that node has zero
//! Taffy children (`taffy_tree.rs:303-327`, the `(_, false)` arm of the
//! `match (display_mode, has_children)` dispatch). So an IFC root gets its
//! height from its inline content **only while its `InlineRoot`-carrying Taffy
//! node is a leaf** — a non-leaf carrier's measure is not skipped but
//! structurally unreachable, and an auto-height root collapses to `h = 0`.
//!
//! Until now that leafness was supplied by accident at every site (inline
//! detachment happening to empty the child list; `has_block` counting an
//! out-of-flow child as block content so an anonymous box gets minted), and
//! nothing asserted it. These tests pin the invariant over the three
//! leaf-supplying shapes, pin the stale-context sweep that keeps dead
//! `InlineRoot` contexts off non-leaves, pin the comment rule that closed the
//! fourth shape (a `show_dom` marker beside a block child used to mint a
//! fresh non-leaf carrier every pass), and prove the validator can actually
//! fire. Every assertion here calls the production predicate
//! ([`RinchDocument::ifc_leaf_invariant_violations`]) or observes production
//! state directly — nothing re-derives the check.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::{NodeContext, RinchDocument};

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
    t
}

fn height_of(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.height
}

fn taffy_id_of(doc: &RinchDocument, node: NodeId) -> taffy::NodeId {
    doc.tree.get(node.0).unwrap().taffy_id.unwrap()
}

/// Whether `node`'s Taffy node carries `NodeContext::InlineRoot`.
fn carries_inline_root(doc: &RinchDocument, node: NodeId) -> bool {
    matches!(
        doc.tree.taffy.get_node_context(taffy_id_of(doc, node)),
        Some(NodeContext::InlineRoot(_))
    )
}

fn taffy_child_count(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .taffy
        .children(taffy_id_of(doc, node))
        .unwrap()
        .len()
}

// ── The three leaf-supplying shapes ─────────────────────────────────────────
//
// Each test asserts both halves of the invariant positively: the discovered
// root's context sits on a childless Taffy node (not merely "no violations",
// which a mutant that never sets `InlineRoot` at all would satisfy), and the
// measured height is nonzero (proof the measure actually fired through the
// leaf path).

/// All-inline root: `mark_inline_descendants` detaches every inline child, so
/// the root's own Taffy node is the childless carrier.
///
/// Kills: a mutant that stops setting `InlineRoot` (context assertion + the
/// height collapses), and one that stops detaching inline children (the child
/// count is nonzero — and in debug builds the in-setup validator panics
/// before the assertion is even reached).
#[test]
fn all_inline_root_carries_the_context_on_a_childless_taffy_node() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    doc.resolve_layout(VW, VH);

    assert!(
        carries_inline_root(&doc, container),
        "an all-inline block container must carry NodeContext::InlineRoot"
    );
    assert_eq!(
        taffy_child_count(&doc, container),
        0,
        "inline detachment must leave the IFC root's Taffy node childless — \
         Taffy never consults the measure of a node with children"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 20.0).abs() < 2.0,
        "the leaf measure gives the root its line height, got {h}"
    );
}

/// Mixed content: the anonymous block box is the childless carrier, and the
/// container — no longer an IFC root at all — carries no `InlineRoot`.
///
/// Kills: a mutant that leaves the anonymous box's inline children attached
/// (child count / validator), and one that marks the *container* a root
/// despite the anonymous box (the container-context assertion).
#[test]
fn mixed_content_hands_the_context_to_a_childless_anonymous_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    child_of(&mut doc, container, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(doc.tree.anonymous_block_boxes.len(), 1);
    let anon_raw = doc.tree.anonymous_block_boxes[0];
    let anon = NodeId(anon_raw);
    assert!(
        carries_inline_root(&doc, anon),
        "the anonymous block box is the IFC root for the inline run"
    );
    assert_eq!(
        taffy_child_count(&doc, anon),
        0,
        "the anonymous box must be a Taffy leaf so its measure fires"
    );
    assert!(
        !carries_inline_root(&doc, container),
        "the container is not an IFC root — the anonymous box took the run"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 50.0).abs() < 2.0,
        "text line (20) + block child (30) expected, got {h}"
    );
}

/// Contents-only-inline (#61): the flattened grandchild is detached through
/// the transparent wrapper, so the container is the childless carrier.
///
/// Kills: a mutant that stops recursing through `display: contents` wrappers
/// in `mark_inline_descendants` — the reparented grandchild's Taffy node
/// stays attached, the carrier is a non-leaf, and both the child-count
/// assertion and (in debug) the in-setup validator go red.
#[test]
fn contents_only_inline_root_is_a_childless_carrier() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let wrapper = child_of(&mut doc, container, "div", "display: contents");
    text_in(&mut doc, wrapper, "hello world");
    doc.resolve_layout(VW, VH);

    assert!(
        carries_inline_root(&doc, container),
        "the block container establishes the IFC over contents-wrapped text"
    );
    assert_eq!(
        taffy_child_count(&doc, container),
        0,
        "the flattened inline grandchild must be detached from the carrier"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 20.0).abs() < 2.0,
        "the leaf measure gives the root its line height, got {h}"
    );
}

// ── The stale-context sweep ─────────────────────────────────────────────────

/// A root that stops being one loses its stale context. `InlineRoot` has one
/// writer and no clearer: before the sweep, a container that was all-inline
/// and then gained a block child kept the dead context on a now-non-leaf
/// Taffy node — harmless to layout (the block arm never consults it), but a
/// standing violation of the invariant the validator asserts.
///
/// Kills: deleting the sweep — in debug builds the in-setup validator panics
/// on the second `resolve_layout`, and were the validator deleted too, the
/// explicit context assertion here fails. The height assertion additionally
/// pins that the sweep clears only *dead* data: the text still renders, via
/// the anonymous box.
#[test]
fn a_container_that_stops_being_a_root_loses_its_stale_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    doc.resolve_layout(VW, VH);
    assert!(carries_inline_root(&doc, container), "precondition");

    // The container gains a block child between frames: an anonymous box now
    // takes the inline run and the container stops being an IFC root.
    child_of(&mut doc, container, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert!(
        !carries_inline_root(&doc, container),
        "the sweep must clear the stale InlineRoot off the now-non-leaf container"
    );
    assert!(taffy_child_count(&doc, container) > 0);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 50.0).abs() < 2.0,
        "text line (20) + block child (30) expected, got {h}"
    );
}

/// The sweep's `children > 0` guard is load-bearing: a *childless* stale
/// carrier keeps its context, because a childless node's measure IS reachable
/// — clearing it would change measure behaviour, and the sweep must be
/// observationally neutral.
///
/// Kills: widening the sweep to clear every stale context regardless of
/// children.
#[test]
fn the_sweep_leaves_a_childless_stale_carrier_alone() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let text = text_in(&mut doc, container, "hello world");
    doc.resolve_layout(VW, VH);
    assert!(carries_inline_root(&doc, container), "precondition");

    // Remove the text: the container is childless, no longer discovered as a
    // root — and its Taffy node still carries the old context.
    doc.remove_child(container, text);
    doc.resolve_layout(VW, VH);

    assert_eq!(taffy_child_count(&doc, container), 0);
    assert!(
        carries_inline_root(&doc, container),
        "a childless stale carrier is left alone — its measure is reachable, \
         so clearing it would not be neutral"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

// ── The validator discriminates ─────────────────────────────────────────────

/// The production predicate reports a context-carrying non-leaf. A validator
/// nothing can trip is decoration; this doctors the exact accident — a Taffy
/// child under an `InlineRoot` carrier — and confirms the predicate names the
/// node.
///
/// Kills: inverting or dropping either conjunct of the predicate (a version
/// that ignores the child count, or one that always answers empty).
#[test]
fn the_predicate_reports_a_context_carrying_non_leaf() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    doc.resolve_layout(VW, VH);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());

    // Doctor the accident: attach a bare Taffy child to the carrier.
    let container_taffy = taffy_id_of(&doc, container);
    let leaf = doc.tree.taffy.new_leaf(Default::default()).unwrap();
    doc.tree.taffy.add_child(container_taffy, leaf).unwrap();

    assert_eq!(
        doc.ifc_leaf_invariant_violations(),
        vec![container.0],
        "a Taffy node carrying InlineRoot with children is exactly one violation"
    );
}

/// The validator is wired into setup, and the sweep does not paper over a
/// marking-path violation: a node (re)marked a root this pass is exempt from
/// the sweep, so a root that keeps a Taffy child trips the assert.
///
/// Kills: unwiring the validator from `setup_inline_formatting_contexts`
/// (nothing panics), and deleting the sweep's `roots_this_pass` exemption
/// (the sweep would clear the doctored context before the validator looks —
/// nothing panics, and the accident-catcher is silently disarmed).
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "IFC leaf invariant violated")]
fn the_validator_fires_inside_setup_on_a_marked_root_that_kept_a_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    doc.resolve_layout(VW, VH);

    // Doctor the accident, then force the IFC pass to re-run. The container is
    // re-discovered as a root (it still has its inline DOM child), the sweep
    // exempts it, the DOM-less doctored child survives inline detachment — and
    // the validator must catch the non-leaf carrier.
    let container_taffy = taffy_id_of(&doc, container);
    let leaf = doc.tree.taffy.new_leaf(Default::default()).unwrap();
    doc.tree.taffy.add_child(container_taffy, leaf).unwrap();
    doc.tree.ifc_dirty = true;
    doc.tree.layout_dirty = true;
    doc.resolve_layout(VW, VH);
}

// ── Comments and roothood (#466's fourth shape) ─────────────────────────────

/// A comment is not inline *content*: a block container whose only inline
/// children are comments, beside a block child, must not be marked an IFC
/// root — the block child stays attached, so the mark would mint a fresh
/// non-leaf `InlineRoot` every pass, through the marking path the sweep
/// deliberately exempts. This is exactly what `show_dom`'s marker comment
/// plus its branch root produce under any `display: block` parent, and it
/// violated the invariant on unmodified main.
///
/// Kills: reverting the discovery amendment (comments counting toward
/// roothood again) — in debug builds the in-setup validator panics inside
/// `resolve_layout`, and the context assertion fails without it. The height
/// assertion pins that withholding the mark changed nothing the block
/// algorithm produced.
#[test]
fn a_comment_beside_a_block_child_does_not_make_the_container_a_root() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let marker = doc.create_comment("show");
    doc.append_child(container, marker);
    child_of(&mut doc, container, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert!(
        !carries_inline_root(&doc, container),
        "a comment beside an attached block child must not establish an IFC"
    );
    assert_eq!(
        taffy_child_count(&doc, container),
        1,
        "the block child stays attached — which is why the mark is withheld"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 30.0).abs() < 0.5,
        "the block algorithm's answer is unchanged: the child's 30px, got {h}"
    );
}

/// The comment rule withholds roothood only when a non-inline child would
/// stay attached: a container holding nothing but comments — a collapsed
/// `show_dom` branch — keeps the marked-leaf path (and its measured height,
/// zero) it has always had.
///
/// Kills: widening the amendment to drop comment-only containers too.
#[test]
fn a_comment_only_container_is_still_a_marked_leaf() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let marker = doc.create_comment("show");
    doc.append_child(container, marker);
    doc.resolve_layout(VW, VH);

    assert!(
        carries_inline_root(&doc, container),
        "comment-only keeps the root+measure path"
    );
    assert_eq!(taffy_child_count(&doc, container), 0);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert_eq!(
        height_of(&doc, container),
        0.0,
        "the measure fires over empty inline content and answers zero"
    );
}

/// The ink-bearing corner of the comment rule (#490): comment +
/// contents-wrapped text + block sibling — exactly `div { if x { "text" }
/// Block{} }` in rsx. On main the comment made the container a root, so
/// `mark_inline_descendants` detached the wrapper-flattened text into a root
/// whose measure could never run (the block child stayed attached): the line
/// contributed nothing to the height and painted at y = 0 over the block,
/// while the identical markup *without* the marker comment rendered
/// correctly. Withholding the mark leaves the text in Taffy as an ordinary
/// text leaf.
///
/// The control agreement is the load-bearing half: PRs 2 and 3 rework
/// exactly this region, and without it this corner could regress while the
/// no-ink comment-beside-block test stays green.
///
/// Kills: reverting the comment amendment (h collapses to the block's 30 and
/// the control disagrees), and any future change that makes a marker comment
/// alter its siblings' layout.
#[test]
fn contents_wrapped_text_beside_a_comment_matches_its_comment_free_twin() {
    fn build(with_comment: bool) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = child_of(
            &mut doc,
            body,
            "div",
            "font-size: 16px; line-height: 20px; width: 400px",
        );
        if with_comment {
            let marker = doc.create_comment("show");
            doc.append_child(container, marker);
        }
        let wrapper = child_of(&mut doc, container, "span", "display: contents");
        text_in(&mut doc, wrapper, "hello world");
        let block = child_of(&mut doc, container, "div", "height: 30px");
        doc.resolve_layout(VW, VH);
        (doc, container, block)
    }

    let (doc, container, block) = build(true);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 50.0).abs() < 2.0,
        "text line (20) + block child (30) expected — on main the comment \
         made this 30, the text line contributing nothing, got {h}"
    );
    let block_y = doc.tree.get(block.0).unwrap().layout.y;
    assert!(
        (block_y - 20.0).abs() < 2.0,
        "the block sits below the text line, not under it at y=0, got {block_y}"
    );

    // Control: the same markup without the marker comment. A `show_dom`
    // marker must not change its siblings' layout.
    let (doc2, container2, block2) = build(false);
    assert_eq!(
        height_of(&doc, container),
        height_of(&doc2, container2),
        "comment and comment-free twins must agree on the container height"
    );
    assert_eq!(
        doc.tree.get(block.0).unwrap().layout.y,
        doc2.tree.get(block2.0).unwrap().layout.y,
        "comment and comment-free twins must agree on the block position"
    );
}

/// An empty container is not a marked root — it takes the empty-block line
/// floor path instead, and its height comes from that floor, not from a
/// measure over nothing (which answers 0).
///
/// Kills: mutating `all_children_are_comments`'s initializer from
/// `!children.is_empty()` to `true`, which would mark every empty container
/// — the natural-identity value where mutants hide. Near-equivalent today
/// only because the line floor is applied redundantly at two other sites;
/// this pins the discovery path itself.
#[test]
fn an_empty_container_is_not_a_marked_root_and_keeps_the_line_floor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    doc.resolve_layout(VW, VH);

    assert!(
        !carries_inline_root(&doc, container),
        "an empty div establishes no IFC"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 20.0).abs() < 2.0,
        "the empty-block line floor gives one line box, got {h}"
    );
}

// ── display:none children of an IFC root (#466's fifth shape) ───────────────

/// A `display: none` child generates no box, and `scan_contents_children`
/// already says so when it classifies a contents wrapper as transparent — but
/// `mark_inline_descendants` used to leave the hidden child's Taffy node
/// attached, so the root was a non-leaf, its measure structurally
/// unreachable, and the container collapsed to `h = 0` with its visible text
/// laid out but never given a box. This is the one deliberate geometry change
/// in this commit, confined to shapes that were collapsed on main.
///
/// Kills: deleting the `display: none` detach arm — the validator panics in
/// debug builds, and the height collapses back to 0 without it.
#[test]
fn a_display_none_child_no_longer_collapses_the_ifc_root() {
    // The hidden child inside the contents wrapper (recursion depth 1).
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    text_in(&mut doc, wrapper, "VISIBLE");
    child_of(&mut doc, wrapper, "div", "display: none");
    doc.resolve_layout(VW, VH);

    assert!(carries_inline_root(&doc, container));
    assert_eq!(
        taffy_child_count(&doc, container),
        0,
        "the hidden child must be detached — one attached Taffy child makes \
         the measure unreachable"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 20.0).abs() < 2.0,
        "the measure fires and the text gets its line box (was 0 on main), got {h}"
    );

    // The hidden child as a direct sibling of the wrapper (depth 0).
    let mut doc2 = RinchDocument::new();
    let body2 = doc2.body();
    let container2 = child_of(
        &mut doc2,
        body2,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let wrapper2 = child_of(&mut doc2, container2, "span", "display: contents");
    text_in(&mut doc2, wrapper2, "VISIBLE");
    child_of(&mut doc2, container2, "div", "display: none");
    doc2.resolve_layout(VW, VH);

    assert!(carries_inline_root(&doc2, container2));
    assert_eq!(taffy_child_count(&doc2, container2), 0);
    assert_eq!(doc2.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h2 = height_of(&doc2, container2);
    assert!((h2 - 20.0).abs() < 2.0, "got {h2}");
}

/// The detach does not strand the hidden node: turning it visible re-attaches
/// it (the display change sets `ifc_dirty` and `sync_display_contents`
/// rebuilds from DOM order), and hiding it again detaches it again.
///
/// Kills: an implementation that *removes* the hidden child's Taffy node
/// instead of detaching it, or one that forgets the round trip.
#[test]
fn a_detached_hidden_child_comes_back_when_it_turns_visible() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    text_in(&mut doc, wrapper, "VISIBLE");
    let hidden = child_of(&mut doc, wrapper, "div", "display: none");
    doc.resolve_layout(VW, VH);
    assert!(
        (height_of(&doc, container) - 20.0).abs() < 2.0,
        "precondition"
    );

    // Visible: the wrapper now wraps a block, so the container stops being an
    // IFC root; the child must be attached and laid out.
    doc.set_attribute(hidden, "style", "display: block; height: 30px");
    doc.resolve_layout(VW, VH);
    assert!(
        !carries_inline_root(&doc, container),
        "a contents wrapper holding a block box is not IFC-transparent"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!(
        (h - 50.0).abs() < 2.0,
        "text (20) + the re-attached child (30) expected, got {h}"
    );

    // Hidden again: back to the measured line box.
    doc.set_attribute(hidden, "style", "display: none");
    doc.resolve_layout(VW, VH);
    assert!(carries_inline_root(&doc, container));
    assert_eq!(taffy_child_count(&doc, container), 0);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!((h - 20.0).abs() < 2.0, "got {h}");
}
