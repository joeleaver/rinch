//! The Taffy insertion index is derived from the parent's **attached** Taffy
//! child list, not from a count of DOM siblings that merely hold a `taffy_id`
//! (#477).
//!
//! The premise `compute_taffy_child_index` used to rest on — *a node's Taffy
//! child list ≡ its DOM children that have a `taffy_id`, in DOM order* — is
//! false in three independent ways:
//!
//! 1. the IFC pass detaches inline (and `display: none`) children of an IFC
//!    root, leaving their `taffy_id` set (`ifc.rs`, `mark_inline_descendants`);
//! 2. `sync_display_contents` splices a `display: contents` wrapper's
//!    grandchildren into the parent's list in place of the wrapper — so the
//!    wrapper is **one** DOM sibling contributing **zero or N** Taffy slots;
//! 3. #466's IFC measure leaf is a Taffy child with no DOM identity at all.
//!
//! Every assertion here is made **immediately after the DOM mutation, before
//! `resolve_layout`**. That is deliberate and it is the whole point: five
//! separate whole-list rebuild passes (`sync_display_contents` ×2, the
//! anonymous-box pair, the #466 canonicalization) heal a bad attachment on the
//! next `ifc_dirty` pass, and every mutation entry point sets `ifc_dirty`. The
//! defect is therefore latent rather than live, and the seam between the
//! mutation and the next layout pass is the only place it is observable.

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

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
    t
}

fn taffy_of(doc: &RinchDocument, node: NodeId) -> taffy::NodeId {
    doc.tree.nodes[node.0]
        .taffy_id
        .unwrap_or_else(|| panic!("node {} has no taffy id", node.0))
}

/// The parent's live Taffy child list.
fn attached(doc: &RinchDocument, parent: NodeId) -> Vec<taffy::NodeId> {
    doc.tree.taffy.children(taffy_of(doc, parent)).unwrap()
}

/// Where `child` sits in `parent`'s Taffy child list, or `None` if it is not
/// attached to it at all.
fn taffy_index_of(doc: &RinchDocument, parent: NodeId, child: NodeId) -> Option<usize> {
    let c = taffy_of(doc, child);
    attached(doc, parent).iter().position(|&x| x == c)
}

/// A replica of the **old** rule, kept as the counterfactual: how many
/// preceding DOM siblings merely hold a `taffy_id`. Every test below states
/// this number so the buggy index is on the record beside the correct one.
fn dom_sibling_count(doc: &RinchDocument, parent: NodeId, dom_index: usize) -> usize {
    let children = &doc.tree.nodes[parent.0].children;
    (0..dom_index)
        .filter(|&i| i < children.len() && doc.tree.nodes[children[i]].taffy_id.is_some())
        .count()
}

/// The index must be *right*, not merely *survivable*. `attach_taffy_child_at`
/// clamps an out-of-range index into the list and counts a fault, so a child
/// can end up attached even from a wrong index — this separates "the
/// derivation is correct" from "the safety net caught it".
fn assert_no_attach_faults(doc: &RinchDocument) {
    assert_eq!(
        doc.tree.taffy_attach_faults, 0,
        "the insertion index must be derived from the attached list, not \
         clamped into range after the fact"
    );
}

// ---------------------------------------------------------------------------
// Gap source 1: the IFC detach
// ---------------------------------------------------------------------------

/// `insert_before` into an all-inline container. Both text children are
/// detached into the IFC, so the container's Taffy list is **empty** while two
/// DOM children hold `taffy_id`s.
///
/// Buggy index: **1** (one preceding DOM sibling with a `taffy_id`) into a
/// zero-length list — out of range, Taffy errors, the error was swallowed and
/// the block was attached nowhere. Correct index: **0**.
///
/// Kills: counting DOM siblings instead of attached ones; and swallowing the
/// resulting `insert_child_at_index` error instead of falling back.
#[test]
fn a_block_inserted_before_the_second_of_two_detached_texts_is_attached() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "font-size: 16px; width: 400px");
    text_in(&mut doc, container, "one ");
    let two = text_in(&mut doc, container, "two");
    doc.resolve_layout(VW, VH);

    assert!(
        attached(&doc, container).is_empty(),
        "precondition: the IFC pass detached both text children"
    );
    assert_eq!(
        dom_sibling_count(&doc, container, 1),
        1,
        "precondition: the old rule would compute index 1 into an empty list"
    );

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "height: 40px");
    doc.insert_before(container, block, two);

    assert_eq!(
        taffy_index_of(&doc, container, block),
        Some(0),
        "the block must be attached at index 0 of the container's (previously \
         empty) Taffy child list; None means the out-of-range insert failed \
         and the error went nowhere"
    );
    assert_no_attach_faults(&doc);
}

/// The **append** leg of `insert_child`, which has no `add_child` fallback at
/// all: an index past the end of the DOM child list still routes through
/// `insert_child_at_index`, so even a plain append failed out of range.
///
/// Buggy index: **2** (both texts hold `taffy_id`s) into a zero-length list.
/// Correct index: **0**.
///
/// Kills: leaving `insert_child`'s append leg on the DOM count, and any fix
/// that only repairs `insert_before`.
#[test]
fn insert_child_appending_past_the_end_of_an_all_inline_container_attaches() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "font-size: 16px; width: 400px");
    text_in(&mut doc, container, "one ");
    text_in(&mut doc, container, "two");
    doc.resolve_layout(VW, VH);

    assert!(
        attached(&doc, container).is_empty(),
        "precondition: the IFC pass detached both text children"
    );
    assert_eq!(
        dom_sibling_count(&doc, container, 2),
        2,
        "precondition: the old rule would compute index 2 into an empty list"
    );

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "height: 40px");
    doc.insert_child(container, block, 99);

    assert_eq!(
        taffy_index_of(&doc, container, block),
        Some(0),
        "an append into an all-inline container must attach the block; None \
         means `insert_child`'s only Taffy path failed out of range with no \
         fallback"
    );
    assert_no_attach_faults(&doc);
}

/// `replace_node` in a three-text IFC root. The middle text is replaced by a
/// block; the two surviving texts are detached, so the list is empty.
///
/// Buggy index: **1** (one preceding text holds a `taffy_id`). Correct: **0**.
///
/// Kills: fixing only the two `insert_*` call sites and leaving
/// `replace_node`'s on the DOM count.
#[test]
fn replace_node_in_an_all_inline_container_attaches_the_replacement() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "font-size: 16px; width: 400px");
    text_in(&mut doc, container, "aa ");
    let mid = text_in(&mut doc, container, "bb ");
    text_in(&mut doc, container, "cc");
    doc.resolve_layout(VW, VH);

    assert!(
        attached(&doc, container).is_empty(),
        "precondition: the IFC pass detached all three text children"
    );
    assert_eq!(
        dom_sibling_count(&doc, container, 1),
        1,
        "precondition: the old rule would compute index 1 into an empty list"
    );

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "height: 40px");
    doc.replace_node(mid, block);

    assert_eq!(
        taffy_index_of(&doc, container, block),
        Some(0),
        "the replacement must be attached; None means the out-of-range insert \
         failed silently"
    );
    assert_no_attach_faults(&doc);
}

/// The same gap with an **out-of-flow** child, which is the shape #477
/// forecast would stop being papered over. It still is papered over (#466's
/// PR2 canonicalization added a fifth rebuild covering exactly this markup),
/// so the observable is again the raw attachment before layout.
///
/// Buggy index: **1**. Correct: **0**.
///
/// Kills: a fix that special-cases in-flow blocks and leaves out-of-flow
/// inserts on the DOM count.
#[test]
fn an_absolute_child_inserted_between_detached_texts_is_attached() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; width: 400px",
    );
    text_in(&mut doc, container, "one ");
    let two = text_in(&mut doc, container, "two");
    doc.resolve_layout(VW, VH);

    assert!(
        attached(&doc, container).is_empty(),
        "precondition: the IFC pass detached both text children"
    );

    let abs = doc.create_element("div");
    doc.set_attribute(
        abs,
        "style",
        "position: absolute; left: 10px; top: 10px; width: 20px; height: 6px",
    );
    doc.insert_before(container, abs, two);

    assert_eq!(
        taffy_index_of(&doc, container, abs),
        Some(0),
        "the absolute child must be attached at index 0"
    );
    assert_no_attach_faults(&doc);
}

// ---------------------------------------------------------------------------
// Gap source 2: `display: contents` flattening
// ---------------------------------------------------------------------------

/// A `display: contents` wrapper is **one** DOM sibling but contributes **N**
/// Taffy slots. The old count therefore undershoots, and the correct index is
/// genuinely non-zero — this is the shape that a "clamp it into range" fix
/// would still get wrong, because the buggy index is *in range* and merely
/// puts the node in the wrong place.
///
/// Container (flex, so no anonymous-box pass interferes) holds
/// `[wrapper{g1, g2}, tail]`; the wrapper is spliced, so the attached list is
/// `[g1, g2, tail]`. Inserting before `tail` (DOM index 1):
///
/// - buggy index **1** — one preceding DOM sibling with a `taffy_id` — which
///   lands the new box *between* `g1` and `g2`;
/// - correct index **2**, after the wrapper's whole contribution.
///
/// Kills: counting a contents wrapper as one slot; and any fix that only
/// clamps an out-of-range index rather than deriving it from the attached
/// list.
#[test]
fn an_insert_after_a_contents_wrapper_lands_after_all_of_its_flattened_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column",
    );
    let wrapper = child_of(&mut doc, container, "div", "display: contents");
    let g1 = child_of(&mut doc, wrapper, "div", "height: 10px");
    let g2 = child_of(&mut doc, wrapper, "div", "height: 10px");
    let tail = child_of(&mut doc, container, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        attached(&doc, container),
        vec![taffy_of(&doc, g1), taffy_of(&doc, g2), taffy_of(&doc, tail)],
        "precondition: the wrapper's two children are spliced into the \
         container's Taffy list in its place"
    );
    assert_eq!(
        dom_sibling_count(&doc, container, 1),
        1,
        "precondition: the old rule counts the wrapper as one slot"
    );

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "height: 10px");
    doc.insert_before(container, block, tail);

    assert_eq!(
        taffy_index_of(&doc, container, block),
        Some(2),
        "the new box follows every one of the wrapper's flattened children; \
         index 1 is the old DOM count, which drops it between g1 and g2"
    );
    assert_no_attach_faults(&doc);
}

/// Two **empty** `display: contents` wrappers contribute *nothing*, while the
/// DOM count sees two preceding siblings holding `taffy_id`s. This is the
/// mirror of the previous test: there the wrapper contributed more than one
/// slot, here it contributes fewer.
///
/// `[wrapper, wrapper, blk]` where only `blk` is attached; inserting before
/// `blk` (DOM index 2):
///
/// - buggy index **2**, into a one-element list — out of range, so the child
///   was attached nowhere;
/// - correct index **0**.
///
/// Two wrappers rather than one because one is the arity fixed point: with a
/// single wrapper the buggy index is 1, which is `len` and therefore still in
/// range, so the failure would not show at all.
///
/// Kills: a fix that handles only the "wrapper contributes many" direction.
#[test]
fn an_insert_before_a_block_past_two_empty_contents_wrappers_is_attached() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column",
    );
    let w1 = child_of(&mut doc, container, "div", "display: contents");
    let w2 = child_of(&mut doc, container, "div", "display: contents");
    let blk = child_of(&mut doc, container, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        attached(&doc, container),
        vec![taffy_of(&doc, blk)],
        "precondition: the two empty wrappers contribute nothing"
    );
    assert_eq!(
        dom_sibling_count(&doc, container, 2),
        2,
        "precondition: the old rule counts both wrappers, giving index 2 into \
         a one-element list"
    );
    let _ = (w1, w2);

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "height: 10px");
    doc.insert_before(container, block, blk);

    assert_eq!(
        taffy_index_of(&doc, container, block),
        Some(0),
        "the new box precedes the only attached child; None means the \
         out-of-range insert failed silently"
    );
    assert_no_attach_faults(&doc);
}

/// A wrapper restyled **away** from `display: contents` between layout passes
/// still has its grandchildren spliced into the parent's Taffy list — the
/// splice is undone by the next `sync_display_contents`, not by the restyle.
/// So its computed display says "I am one ordinary box" while its real
/// occupancy is still its children's slots, and `Node::contents_spliced` is the
/// only record of that (#520).
///
/// `[wrapper{g1, g2}, tail]` laid out, then the wrapper restyled to
/// `display: block`, then a box inserted before `tail`:
///
/// - a contribution rule gated on computed `display == Contents` alone answers
///   with the wrapper's own Taffy id, which the splice detached — so *nothing*
///   of the wrapper is found attached, the search runs off the front of the
///   sibling list and returns **0**, putting the new box ahead of `g1`;
/// - reading `contents_spliced` too finds `g2` at slot 1 and answers **2**.
///
/// Kills: dropping the `contents_spliced` half of the contribution gate. The
/// other contents fixtures here cannot — their wrappers all still compute
/// `Contents`, so the first half of the gate carries them on its own.
#[test]
fn an_insert_after_a_wrapper_restyled_off_contents_still_clears_its_slots() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column",
    );
    let wrapper = child_of(&mut doc, container, "div", "display: contents");
    let g1 = child_of(&mut doc, wrapper, "div", "height: 10px");
    let g2 = child_of(&mut doc, wrapper, "div", "height: 10px");
    let tail = child_of(&mut doc, container, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        attached(&doc, container),
        vec![taffy_of(&doc, g1), taffy_of(&doc, g2), taffy_of(&doc, tail)],
        "precondition: the wrapper's children are spliced in"
    );

    // Between frames: the wrapper stops being `contents`, but nothing has
    // re-synced the Taffy list yet.
    doc.set_attribute(wrapper, "style", "display: block");
    // A style attribute alone only queues a style root; the *next* DOM
    // mutation is what flushes it (`append_child` →
    // `recompute_node_styles_recursive` → `resolve_styles`, which resolves
    // every pending root, not just its own). Any unrelated insertion does it —
    // this one is elsewhere in the document on purpose, so the fixture's own
    // shape is untouched.
    let elsewhere = doc.create_element("div");
    doc.append_child(body, elsewhere);
    assert!(
        doc.tree.nodes[wrapper.0].contents_spliced,
        "precondition: the splice is still recorded"
    );
    assert_ne!(
        format!("{:?}", doc.tree.nodes[wrapper.0].computed_style.display),
        "Contents",
        "precondition: the restyle is already computed, so the first half of \
         the contribution gate no longer carries this wrapper — without this \
         the fixture sits on a fixed point where both halves agree"
    );
    assert_eq!(
        attached(&doc, container),
        vec![taffy_of(&doc, g1), taffy_of(&doc, g2), taffy_of(&doc, tail)],
        "precondition: and the grandchildren are still the wrapper's slots"
    );

    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "height: 10px");
    doc.insert_before(container, block, tail);

    assert_eq!(
        taffy_index_of(&doc, container, block),
        Some(2),
        "the new box follows the wrapper's still-spliced children; index 0 is \
         what a rule blind to `contents_spliced` answers"
    );
    assert_no_attach_faults(&doc);
}

// ---------------------------------------------------------------------------
// Gap source 3: the #466 measure leaf, a Taffy child with no DOM identity
// ---------------------------------------------------------------------------

/// A `text + absolute` container carries a #466 measure leaf: a Taffy child at
/// index 0 with **no DOM node behind it**. This is the third gap source, and
/// the one that is not about detachment at all — the parent's Taffy list is
/// *longer* than its DOM-with-`taffy_id` count, and shifted.
///
/// Container `[text, text, abs1, abs2]`; the two texts are detached into the
/// IFC and the leaf takes slot 0, so the list is `[leaf, abs1, abs2]`.
/// Inserting a third absolute before `abs2` (DOM index 3):
///
/// - buggy index **3** — both texts and `abs1` hold `taffy_id`s — which is
///   `len`, in range, and **appends** the new box *after* `abs2`;
/// - correct index **2**, immediately after `abs1` and before `abs2`.
///
/// Note this shape errs in range, so it fails as a misordering rather than as
/// a dropped child: clamping alone would not have caught it, and neither would
/// an assertion that only asks whether the box is attached.
///
/// Kills: counting the measure leaf as a DOM sibling; ignoring it and so
/// mis-sizing the list; and any repair that only rescues out-of-range indices.
#[test]
fn an_insert_into_a_container_with_a_measure_leaf_lands_between_its_neighbours() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; width: 400px",
    );
    text_in(&mut doc, container, "aa ");
    text_in(&mut doc, container, "bb ");
    let abs1 = child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 0; top: 0; width: 20px; height: 6px",
    );
    let abs2 = child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 40px; top: 0; width: 20px; height: 6px",
    );
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "precondition: the measure leaf exists"
    );
    let before = attached(&doc, container);
    assert_eq!(
        before.len(),
        3,
        "precondition: leaf + two absolutes are attached, got {before:?}"
    );
    assert_eq!(
        dom_sibling_count(&doc, container, 3),
        3,
        "precondition: the old rule counts two texts and abs1, giving index 3 \
         — past abs2, where the new box belongs before it"
    );

    let abs3 = doc.create_element("div");
    doc.set_attribute(
        abs3,
        "style",
        "position: absolute; left: 80px; top: 0; width: 20px; height: 6px",
    );
    doc.insert_before(container, abs3, abs2);

    let i1 = taffy_index_of(&doc, container, abs1).expect("abs1 stays attached");
    let i2 = taffy_index_of(&doc, container, abs2).expect("abs2 stays attached");
    let i3 = taffy_index_of(&doc, container, abs3).expect("abs3 must be attached");
    assert!(
        i1 < i3 && i3 < i2,
        "abs3 was inserted before abs2 and after abs1, so its Taffy slot must \
         be between theirs — got abs1={i1}, abs3={i3}, abs2={i2}"
    );
    assert_no_attach_faults(&doc);
}
