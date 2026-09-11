//! The anonymous-box passes rebuild a Taffy child list from the **same**
//! authority `sync_display_contents` does (#476).
//!
//! `sync_display_contents` builds a parent's Taffy child list from
//! `collect_effective_taffy_children`, which replaces every `display: contents`
//! child with the boxes it flattens. `cleanup_anonymous_block_boxes` and
//! `create_anonymous_block_boxes` ran immediately afterwards in the same
//! `ifc_dirty` block and rebuilt the same list from **raw
//! `nodes[parent].children`** — a rule that cannot see the flattening. They
//! therefore re-added the wrapper's own boxless Taffy node and dropped the
//! grandchildren it stands for; and because a flattened grandchild is not a DOM
//! child of the parent, nothing re-added it anywhere. It was left **orphaned**
//! in Taffy: laid out `0x0`, painted not at all, on that pass and on every pass
//! after it.
//!
//! What was spared is exactly what `create_anonymous_block_boxes` skips —
//! `DisplayMode::Flex` — and the whole shipped `display: contents` suite is
//! written over flex containers, which is why a defect that erases content
//! lived behind a green board. That is narrower than "only plain blocks":
//! `display: grid` maps to `DisplayMode::Block` in rinch, so grid containers
//! were affected too, and there is a fixture for one below. The flex twin is
//! kept as the control that names the culprit.
//!
//! **Fixed points avoided on purpose** (this project's recurring test failure):
//! one block in the wrapper cannot tell "orphan the set" from "orphan the
//! first", so there is a two-block fixture; a block at the end cannot tell an
//! off-by-one from a drop, so there is a block-first fixture; `y == 0` is where
//! a laid-out box and an orphan agree, so the text sibling above gives every
//! other fixture a non-zero expected offset; and a contents chain of **length
//! one** is where the owners *loop* and a single step up agree, so the fixtures
//! that exercise the walk nest two deep. That last one was missed on the first
//! pass and found by mutation — the file already claimed to have stepped off
//! the fixed points while sitting on this one.
//!
//! **Which fixtures those are moved with #568.** The two nested-wrapper
//! fixtures below were written for it and no longer reach the walk at all: a
//! `display: contents` element is skipped as a container now, so nothing hands
//! `taffy_child_list_owners` a `Contents` node from that shape. The "owners
//! walk, after #568" section near the end is where the walk — loop included —
//! is witnessed today, and it is the one place in this file whose fixtures fail
//! against a walk mutant (#585).

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

fn rect(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// Whether a node's box has a Taffy parent. An orphan — a node with a
/// `taffy_id` that no parent's child list holds — answers `false`, which is the
/// #476 symptom stated structurally rather than through the geometry it
/// produces.
///
/// **Local, and deliberately named for what it asks.** It said "reachable from
/// the Taffy root" until #589, which is a different and stronger property: a
/// chain of valid edges hanging off nothing answers `true` here at every link.
/// The stronger question belongs to `taffy_tree_violations`' `D` rule, which
/// [`assert_consistent`] below asks of the whole document, so a fixture wanting
/// it should call that rather than reach for this.
fn attached_to_taffy(doc: &RinchDocument, id: NodeId) -> bool {
    doc.tree
        .get(id.0)
        .and_then(|n| n.taffy_id)
        .and_then(|t| doc.tree.taffy.parent(t))
        .is_some()
}

/// No node claimed by two parents, no `parent()`/`children()` disagreement, no
/// orphan, **no subtree laid out by nobody** —
/// `RinchDocument::taffy_tree_violations`.
///
/// Called from every fixture below rather than pinned once, because the
/// property is cross-cutting: on `main` it catches the raw-DOM rebuild in four
/// separate ways (the whole-list rebuild in either pass, the ancestor walk, and
/// a non-recursive `collect_effective_taffy_children`), and it caught `main`
/// reaching the right *height* through a tree where one Taffy node sat in two
/// parents' child lists — which no geometry assertion can see.
///
/// The three `*_restyled_to_contents_*` fixtures at the bottom of this file
/// used to need a private `reachable_from_taffy_root` helper beside it, because
/// the validator could not make their claim: a subtree detached at a
/// `display: contents` node keeps every edge inside it, so every member reports
/// a Taffy parent and invariant `C` skips the one node that does not. The
/// helper is gone — #589 put root reachability into the validator as `D`, so
/// this call now *is* the assertion those fixtures were making by hand, and it
/// makes it of every node in the document rather than of the one the fixture
/// thought to name. Verified fail-first against #585's mutant
/// (`taffy_child_list_owners` reduced to `vec![node_id]`): both witnesses fail
/// here, through this function, with the helper deleted.
fn assert_consistent(doc: &RinchDocument, what: &str) {
    let v = doc.taffy_tree_violations();
    assert!(
        v.is_empty(),
        "{what}: Taffy tree is inconsistent:\n  {}",
        v.join("\n  ")
    );
}

/// `<div>label<div style="display: contents"><div h=40/></div></div>` — the
/// issue's own markup — plus the identical markup with the wrapper removed.
/// Returns `(doc, container, block)` for each.
fn wrapped_and_plain() -> (
    (RinchDocument, NodeId, NodeId),
    (RinchDocument, NodeId, NodeId),
) {
    let wrapped = {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = child_of(&mut doc, body, "div", "font-size: 16px");
        text_in(&mut doc, outer, "label");
        let wrapper = child_of(&mut doc, outer, "div", "display: contents");
        let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
        doc.resolve_layout(VW, VH);
        (doc, outer, blk)
    };
    let plain = {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = child_of(&mut doc, body, "div", "font-size: 16px");
        text_in(&mut doc, outer, "label");
        let blk = child_of(&mut doc, outer, "div", "height: 40px; background: red");
        doc.resolve_layout(VW, VH);
        (doc, outer, blk)
    };
    (wrapped, plain)
}

// ── The reported bug ────────────────────────────────────────────────────────

/// A `display: contents` wrapper generates no box, so wrapping a block in one
/// must not move it, resize it, or remove it. The oracle is the identical
/// markup with the wrapper deleted, measured in the same test — no hard-coded
/// line height to drift.
///
/// Kills: rebuilding the parent's Taffy children from raw
/// `nodes[parent].children` in `create_anonymous_block_boxes`. With that mutant
/// the block is orphaned — `(0, 0, 0x0)`, no Taffy parent — and the container
/// keeps only the text's height.
#[test]
fn a_wrapped_block_beside_text_is_laid_out_exactly_where_an_unwrapped_one_is() {
    let ((wdoc, wouter, wblk), (pdoc, pouter, pblk)) = wrapped_and_plain();

    assert!(
        attached_to_taffy(&wdoc, wblk),
        "the wrapped block must be reachable from the Taffy root; an orphan is \
         laid out by nothing and painted by nothing"
    );
    assert_eq!(
        rect(&wdoc, wblk),
        rect(&pdoc, pblk),
        "a boxless wrapper must not change its child's box"
    );
    assert_eq!(
        height_of(&wdoc, wouter),
        height_of(&pdoc, pouter),
        "nor the container's height"
    );

    // Off the fixed point: the text line above gives the block a non-zero
    // offset, so "laid out" and "collapsed at the origin" cannot agree.
    let (_, y, w, h) = rect(&wdoc, wblk);
    assert!(
        y > 0.0 && w > 0.0 && h > 0.0,
        "the fixture must not sit at the origin, or an orphan would pass: got \
         y={y}, {w}x{h}"
    );
    assert_consistent(&wdoc, "wrapped");
    assert_consistent(&pdoc, "unwrapped control");
}

/// The same wrapper with the block **first** and the text after it. A drop and
/// an off-by-one insert are the same picture when the wrapped box is last, so
/// this is the half of the pair that tells them apart: here the block belongs
/// at `y == 0` and the text below it.
///
/// Kills: the same raw-DOM rebuild, and any "append the flattened boxes at the
/// end" repair of it.
#[test]
fn a_wrapped_block_before_the_text_is_laid_out_at_the_top() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    text_in(&mut doc, outer, "label");
    doc.resolve_layout(VW, VH);

    let (x, y, w, h) = rect(&doc, blk);
    assert_eq!(
        (x, y, w, h),
        (0.0, 0.0, VW, 40.0),
        "the wrapped block comes first in document order, so it takes the top"
    );
    assert!(
        height_of(&doc, outer) > 40.0,
        "and the text line still follows it: container height {} should exceed \
         the block's 40",
        height_of(&doc, outer)
    );
    assert_consistent(&doc, "block first");
}

/// **Two** blocks under one wrapper. One is the arity fixed point — "orphan
/// every flattened box" and "orphan the first" produce the same picture — so
/// the single-block fixtures above cannot distinguish a rebuild that drops the
/// whole flattened run from one that drops all but its head.
///
/// Kills: a rebuild that flattens only the wrapper's first child.
#[test]
fn both_blocks_under_one_wrapper_are_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let first = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    let second = child_of(&mut doc, wrapper, "div", "height: 25px; background: red");
    doc.resolve_layout(VW, VH);

    assert!(attached_to_taffy(&doc, first), "first block orphaned");
    assert!(attached_to_taffy(&doc, second), "second block orphaned");

    let (_, y1, _, h1) = rect(&doc, first);
    let (_, y2, _, h2) = rect(&doc, second);
    assert_eq!((h1, h2), (40.0, 25.0), "both keep their declared heights");
    assert!(y2 > y1, "and stack in document order: y1={y1}, y2={y2}");
    assert_eq!(
        height_of(&doc, outer),
        y1 + h1 + h2,
        "the container's height counts both"
    );
    assert_consistent(&doc, "two blocks");
}

/// The orphaning was **stable**: `sync_display_contents` re-spliced the block
/// into the container on every pass and the anonymous-box rebuild orphaned it
/// again, so it never self-healed. A repeated-pass fixture is what says the fix
/// is a fix and not a first-pass coincidence.
///
/// Kills: any repair applied only to the first layout pass.
#[test]
fn the_wrapped_block_stays_laid_out_across_repeated_passes() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");

    let mut seen = Vec::new();
    for _ in 0..4 {
        doc.resolve_layout(VW, VH);
        seen.push(rect(&doc, blk));
    }

    assert!(
        seen.iter().all(|r| *r == seen[0]),
        "the geometry must be stable across passes: {seen:?}"
    );
    let (_, y, w, h) = seen[0];
    assert!(
        y > 0.0 && (w, h) == (VW, 40.0),
        "and it must be the laid-out geometry, not a stable zero: {:?}",
        seen[0]
    );
    assert_consistent(&doc, "after four passes");
}

/// Wrappers nested to any depth flatten the same way — the authority recurses,
/// so the rebuild that consumes it must inherit that for free.
#[test]
fn nested_wrappers_flatten_the_same_way() {
    let ((_, _, _), (pdoc, pouter, pblk)) = wrapped_and_plain();

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    text_in(&mut doc, outer, "label");
    let w1 = child_of(&mut doc, outer, "div", "display: contents");
    let w2 = child_of(&mut doc, w1, "div", "display: contents");
    let blk = child_of(&mut doc, w2, "div", "height: 40px; background: red");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        rect(&doc, blk),
        rect(&pdoc, pblk),
        "two boxless wrappers are still no box"
    );
    assert_eq!(height_of(&doc, outer), height_of(&pdoc, pouter));
    assert_consistent(&doc, "nested wrappers");
}

/// The control that names the culprit. `create_anonymous_block_boxes` skips
/// `DisplayMode::Flex`, so a flex parent never reached the broken rebuild and
/// laid the wrapped block out correctly the whole time. It must still.
///
/// Without this, "the contents flattening is broken" would be an equally good
/// explanation of the block fixtures, and a fix aimed at `sync_display_contents`
/// would look right.
#[test]
fn a_flex_parent_was_never_affected_and_still_is_not() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; display: flex; flex-direction: column",
    );
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    doc.resolve_layout(VW, VH);

    let (_, y, w, h) = rect(&doc, blk);
    assert!(
        y > 0.0 && (w, h) == (VW, 40.0),
        "the flex control must be laid out below the text: {:?}",
        rect(&doc, blk)
    );
    assert_consistent(&doc, "flex control");
}

/// A **grid** container is affected, and this is what stops "only plain blocks
/// were broken" being written down again. `display: grid` maps to
/// `DisplayMode::Block` in rinch (`style_resolution`), and
/// `create_anonymous_block_boxes` skips only `Flex` — so a grid container with
/// mixed content mints anonymous boxes and hit the same orphaning.
#[test]
fn a_grid_parent_was_affected_too() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px; display: grid");
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the wrapped block is orphaned under a grid container"
    );
    assert_eq!(rect(&doc, blk).3, 40.0, "and it keeps its height");
    assert!(
        height_of(&doc, outer) > 40.0,
        "the container measures the text as well as the block, got {}",
        height_of(&doc, outer)
    );
    assert_consistent(&doc, "grid parent");
}

// ── The same defect reached through the other flattened roles ───────────────

/// A **transparent** wrapper (one holding only an out-of-flow box, #518) inside
/// a container that is mixed for an independent reason. The container's
/// anonymous-box rebuild flattens it exactly like an opaque one, so its
/// absolute has to survive too — it was orphaned before, `0x0`, never painted.
///
/// This is the shape `ifc_contents_transparency_tests`'
/// `a_transparent_wrapper_does_not_split_a_run_in_an_independently_mixed_container`
/// already builds and only asserts the run grouping of.
///
/// Kills: flattening only wrappers the run loop judged opaque.
#[test]
fn an_absolute_under_a_transparent_wrapper_in_a_mixed_container_is_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");
    text_in(&mut doc, c, "before ");
    let wrapper = child_of(&mut doc, c, "span", "display: contents");
    let abs = child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 0; top: 0; width: 10px; height: 10px",
    );
    text_in(&mut doc, c, " after");
    // The independent reason the container is mixed.
    child_of(&mut doc, c, "div", "width: 40px; height: 20px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        rect(&doc, abs),
        (0.0, 0.0, 10.0, 10.0),
        "the absolute must be laid out; (0, 0, 0, 0) is the orphan"
    );
    assert!(attached_to_taffy(&doc, abs));
    assert_consistent(&doc, "transparent wrapper holding an absolute");
}

/// A transparent wrapper holding **inline** content, in the same
/// independently-mixed container. The wrapper's `<span>` was orphaned and its
/// text vanished; it must be reachable.
///
/// Deliberately weak about *where*, and about geometry — and **#568 is why
/// that was the right call**. When this was written the run grouping declined
/// to put a transparent wrapper in the inline run, so the span was laid out by
/// Taffy as its own box on its own second line and the container was 20px
/// taller than a browser makes it; the prediction recorded here was that #568
/// would pull that content into the anonymous box's run, at which point the
/// span and its text become IFC-owned and their `layout` collapses to
/// `(0, 0, 0x0)`, which is what an IFC-owned box looks like in this engine.
/// That is what happened, and this fixture needed no edit: reachability holds
/// under both regimes, a `w > 0 && h > 0` on the text would not have.
/// `anon_box_flattened_classification_tests` owns the geometry now.
///
/// Honest about which half is evidence: the assertion on `span` failed on
/// `main` before #476 (it was the orphan), and so did the consistency check.
/// The one on `inner` passed there — the text is a child of the *orphaned*
/// span, so it has a Taffy parent — and is a guard against a future drop, not
/// fail-first evidence.
///
/// Kills: a fix that flattens only the roles the run loop happens to look at.
#[test]
fn a_transparent_wrappers_inline_content_stays_reachable() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");
    text_in(&mut doc, c, "before ");
    let wrapper = child_of(&mut doc, c, "span", "display: contents");
    let span = child_of(&mut doc, wrapper, "span", "");
    let inner = text_in(&mut doc, span, "MIDDLE");
    text_in(&mut doc, c, " after");
    child_of(&mut doc, c, "div", "width: 40px; height: 20px");
    doc.resolve_layout(VW, VH);

    let laid_out_by_somebody =
        |id: NodeId| attached_to_taffy(&doc, id) || doc.tree.get(id.0).unwrap().ifc_root.is_some();
    assert!(
        laid_out_by_somebody(span),
        "the wrapped inline content must be laid out by somebody — Taffy or an \
         IFC. Neither is the orphan."
    );
    assert!(
        laid_out_by_somebody(inner),
        "and so must its text: an orphaned text node is drawn by nothing"
    );
    assert_consistent(&doc, "transparent wrapper holding inline content");
}

// ── The cleanup half of the pair, isolated ─────────────────────────────────

/// `cleanup_anonymous_block_boxes` rebuilds the same list, and on a pass where
/// the container stops being mixed it is the **only** rebuild that runs —
/// `create_anonymous_block_boxes` returns before its own.
///
/// The fixture reaches that: pass 1 is `span + wrapper > block`, which is mixed
/// and mints an anonymous box; pass 2 hides the span, so `has_inline` is false,
/// creation is skipped, and the cleanup rebuild alone decides where the
/// wrapped block goes. With cleanup on raw DOM order the block is orphaned on
/// pass 2 — laid out and painted on pass 1, gone on pass 2, from a change that
/// did not touch it.
///
/// Kills: fixing `create_anonymous_block_boxes` and leaving
/// `cleanup_anonymous_block_boxes` on raw `nodes[parent].children`.
#[test]
fn the_cleanup_rebuild_uses_the_same_authority() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    let label = child_of(&mut doc, outer, "span", "");
    text_in(&mut doc, label, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");

    doc.resolve_layout(VW, VH);
    assert!(
        attached_to_taffy(&doc, blk),
        "precondition: the creation pass attaches it"
    );
    let before = rect(&doc, blk);

    // Hiding the inline sibling makes the container un-mixed, so this pass
    // runs the cleanup rebuild and skips the creation one.
    doc.set_attribute(label, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the block was attached on pass 1 and orphaned on pass 2 — the cleanup \
         rebuild dropped it"
    );
    let (_, y, w, h) = rect(&doc, blk);
    assert_eq!(
        (w, h),
        (VW, 40.0),
        "and it keeps its box: was {before:?}, now {:?}",
        rect(&doc, blk)
    );
    assert_eq!(
        y, 0.0,
        "with the text hidden it moves to the top, not to nowhere"
    );
    assert_eq!(height_of(&doc, outer), 40.0);
    assert_consistent(&doc, "after the cleanup-only pass");
}

// ── Whose list, not just what is in it ─────────────────────────────────────

/// An anonymous box can be **stored inside a boxless element**. Since #568 the
/// *container* classifies its flattened children and mints the box, and the box
/// takes the slot its run's head vacated — which is inside the wrapper whenever
/// the run starts there. (Before #568 the wrapper itself was classified as
/// mixed content and minted the box; the box ended up in the same list either
/// way, which is why this fixture reads the same.) The net structure is still
/// right, because the flattening lifts that anonymous box into the ancestor's
/// list — but only if the ancestor's list is rebuilt after the box exists.
/// `sync_display_contents` built it before, and nothing re-runs it.
///
/// Killed, until #568: rebuilding only the anonymous box's own recorded
/// parent. That parent was the wrapper while the wrapper was the node
/// classified as mixed content, and Taffy's `set_children` steals each adopted
/// child from its previous parent, so rebuilding just the wrapper emptied the
/// container's list and collapsed it to `h = 0`. Since #568 the box's parent
/// **is** the container, so "rebuild only the box's own parent" is no longer a
/// mutant at all — it is the correct code, and this fixture reads the same
/// either way (#585). What it still asserts is the flattening result itself:
/// the box ends up in the container's list and the container measures it.
#[test]
fn an_anonymous_box_minted_inside_a_wrapper_does_not_strand_the_ancestor() {
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
    let blk = child_of(&mut doc, wrapper, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the wrapped block is orphaned"
    );
    let (_, y, _, h) = rect(&doc, blk);
    assert_eq!(h, 30.0);
    assert!(
        y > 0.0,
        "the block sits below the wrapped text's line, got y={y}"
    );
    assert_eq!(
        height_of(&doc, container),
        y + h,
        "the container measures the text line plus the block, not 0"
    );
    assert_consistent(&doc, "anonymous box inside one wrapper");
}

/// **Two** contents levels between the mixed container and the box-generating
/// one. One is the arity fixed point of the owners **loop**: with a chain of
/// length 1, "walk up while the node is `contents`" and "take a single step up"
/// are the same walk — and every other fixture in this file, the single-wrapper
/// twin above included, has a chain of length 1.
///
/// Found by mutation, not by inspection. Truncating `taffy_child_list_owners`
/// to one step passed this entire suite, and its symptom is `(0, 0, 0x0)` —
/// #476 itself, restored, on a green board. This is exactly the fixed-point
/// trap the file header claims to have stepped off, surviving in the one
/// parameter nothing varied.
///
/// **That is history now, and this fixture no longer kills it (#585).** It
/// killed the loop's removal because the *wrapper* was the node classified as
/// mixed content, so the box's recorded parent was a `display: contents` node
/// and the owners walk ran. #568 skips a `display: contents` element as a
/// container, so the box is minted by `container` and the walk is never
/// entered from this shape — measured with a probe inside the loop, which
/// fires zero times across this suite. Both walk mutants survive it. What it
/// still asserts is that a box minted for a run two contents levels down lands
/// in the container's list; the walk's own witness is the "owners walk, after
/// #568" section.
#[test]
fn an_anonymous_box_two_contents_levels_down_does_not_strand_the_ancestor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let w1 = child_of(&mut doc, container, "span", "display: contents");
    let w2 = child_of(&mut doc, w1, "span", "display: contents");
    text_in(&mut doc, w2, "VISIBLE");
    let blk = child_of(&mut doc, w2, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the wrapped block is orphaned"
    );
    let (_, y, _, h) = rect(&doc, blk);
    assert_eq!(h, 30.0, "the block keeps its height");
    assert!(y > 0.0, "it sits below the wrapped text's line, got y={y}");
    assert_eq!(
        height_of(&doc, container),
        y + h,
        "the container measures the text line plus the block, not 0"
    );
    assert_consistent(&doc, "two contents levels");
}

/// Three levels, plus a plain block sibling so the container is independently
/// mixed as well — the mutant has to survive a container being rebuilt for its
/// *own* reasons, not only as somebody's ancestor.
///
/// This one **failed on pre-#476 `main`** as well as against the truncated
/// walk, so it was fail-first evidence for that fix and a mutation guard at the
/// same time. The mutation-guard half went with #568, for the reason the
/// two-level fixture above gives: nothing here reaches the owners walk any more
/// (#585). The fail-first half stands.
#[test]
fn three_contents_levels_down_with_an_independently_mixed_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "top");
    let w1 = child_of(&mut doc, container, "span", "display: contents");
    let w2 = child_of(&mut doc, w1, "span", "display: contents");
    let w3 = child_of(&mut doc, w2, "span", "display: contents");
    text_in(&mut doc, w3, "INNER");
    let blk = child_of(&mut doc, w3, "div", "height: 30px");
    let trailing = child_of(&mut doc, container, "div", "height: 7px");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the wrapped block is orphaned"
    );
    assert!(
        attached_to_taffy(&doc, trailing),
        "the trailing sibling is orphaned"
    );
    assert_eq!(rect(&doc, blk).3, 30.0);
    let container_height = height_of(&doc, container);
    assert!(
        container_height >= 37.0,
        "the container must measure the wrapped block (30) and the trailing \
         sibling (7), got {container_height}"
    );
    assert_consistent(&doc, "three contents levels");
}

// ── The owners walk, after #568 ────────────────────────────────────────────

/// `<div style="display: flex; flex-direction: column">` around a plain block
/// holding `text + block`, then that block restyled to `display: contents`.
///
/// **This is the route into `taffy_child_list_owners`' contents walk that #568
/// leaves open, and #585 is that nothing was taking it.** Every fixture above
/// hands that function a node whose computed display is not `Contents`, so the
/// walk does not run — measured directly with a probe inside the loop, which
/// fires zero times across `cargo test -p rinch-dom -p rinch` (41 binaries) on
/// `4fe65ed`. Both halves of the walk survived that same suite as mutants:
/// `owners = vec![node_id]` (no walk at all) and a loop-free single step. So it
/// is not only the loop that had lost its witnesses; it was the whole walk.
///
/// The shape #567 built it for is the one #568 removed: a wrapper classified as
/// mixed content in its own right, minting a box inside a boxless element.
/// Phase 1 of `create_anonymous_block_boxes` now skips a `display: contents`
/// element, and `ComputedStyle::for_anonymous_box` copies `Contents` only from
/// a `Contents` parent — which a container reaching phase 2 never is — so a
/// minted box is always `Block` (measured, not read off the guard). Neither
/// creation call site can reach the walk any more.
///
/// The **cleanup** site can. `cleanup_anonymous_block_boxes` takes each
/// affected parent from `anon.parent`, recorded on the pass that *minted* the
/// box; #568's guard applies at classification time, on this pass. A container
/// that was `display: block` when it minted a box and has since been restyled
/// to `display: contents` therefore arrives as a `Contents` node and the walk
/// runs. An inline `style` write and a class change through a stylesheet both
/// reach it — the cascade route is measured, not assumed.
///
/// That is the state the walk needs, not the only way of entering it, and the
/// distinction is worth keeping: what has to be true is that `anon.parent`
/// names a node whose computed display is `Contents` **now**. Restyling that
/// container is the way found; an ancestor's restyle and a reparenting do not
/// change the container's own display and so are not routes, and a slab index
/// recycled from a removed container into a fresh `display: contents` element
/// would be one but was not reproduced. If another appears it belongs here.
///
/// The ancestor must be one phase 1 skips — `DisplayMode::Flex`,
/// `Inline` or `InlineBlock` — and a flex column is the everyday one, since the
/// component library lays out with `Stack` and `Group`. Against a **block**
/// ancestor the walk is redundant: flattening a mixed container into a block
/// makes that block mixed in its own right, so phase 2 rebuilds its list for
/// its own reasons. The control below is exactly that case and it discriminates
/// nothing, which is why these two fixtures are built on flex.
///
/// Kills: `taffy_child_list_owners` reduced to `vec![node_id]`. The cleanup
/// rebuild then stops at the restyled container, whose own Taffy node generates
/// no box, and `set_children` steals the content out of the flex column's list
/// on the way — the column collapses to `h = 0` with its whole subtree laid out
/// nowhere.
#[test]
fn a_container_restyled_to_contents_rebuilds_the_ancestor_that_now_holds_its_boxes() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = child_of(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column; width: 400px",
    );
    let inner = child_of(&mut doc, col, "div", "font-size: 16px; line-height: 20px");
    text_in(&mut doc, inner, "label");
    let blk = child_of(&mut doc, inner, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, col),
        50.0,
        "precondition: the text line (20) plus the block (30), with the \
         anonymous box minted by `inner` while it is still a block"
    );

    // `inner` keeps its mixed content and stops generating a box. Its
    // anonymous box's recorded parent is now a `display: contents` node.
    doc.set_attribute(
        inner,
        "style",
        "display: contents; font-size: 16px; line-height: 20px",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(rect(&doc, blk).3, 30.0, "the block keeps its height");
    assert_eq!(
        height_of(&doc, col),
        50.0,
        "the flex column still measures the text line plus the block, not 0"
    );
    assert_consistent(&doc, "container restyled to contents, flex ancestor");
}

/// The same, with **two** contents levels between the restyled container and
/// the flex column that ends up holding its boxes.
///
/// A chain of length one is the arity fixed point of the *loop*: there, "walk
/// up while the node is `contents`" and "take a single step up" are the same
/// walk, so the fixture above cannot tell them apart. This is the same fixed
/// point #567 sat on until mutation found it, restated for the route that
/// still reaches the walk.
///
/// Kills: the loop replaced by a single step (as well as the walk's total
/// removal). The single step stops on `w1`, which generates no box, so the flex
/// column's list is never rebuilt after the anonymous box is dropped and it
/// collapses to `h = 0`.
#[test]
fn a_container_restyled_to_contents_two_levels_down_walks_the_whole_chain() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let col = child_of(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column; width: 400px",
    );
    let w1 = child_of(&mut doc, col, "div", "display: contents");
    let inner = child_of(&mut doc, w1, "div", "font-size: 16px; line-height: 20px");
    text_in(&mut doc, inner, "label");
    let blk = child_of(&mut doc, inner, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, col), 50.0, "precondition");

    doc.set_attribute(
        inner,
        "style",
        "display: contents; font-size: 16px; line-height: 20px",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(rect(&doc, blk).3, 30.0, "the block keeps its height");
    assert_eq!(
        height_of(&doc, col),
        50.0,
        "the flex column two levels up still measures the text line plus the \
         block, not 0"
    );
    assert_consistent(&doc, "two contents levels, flex ancestor");
}

/// The control that names the culprit: the identical restyle under a **block**
/// ancestor, which repairs itself.
///
/// It must stay green, and it must stay green against both walk mutants —
/// measured, and stated here so nobody reads it as a second witness. Flattening
/// `text + block` into a block ancestor makes that ancestor mixed content in
/// its own right, so `create_anonymous_block_boxes` mints a box for it and
/// rebuilds its Taffy list at the end of the same pass, whatever the cleanup
/// walk did or did not do. That is why the two fixtures above need a container
/// display phase 1 skips.
#[test]
fn the_same_restyle_under_a_block_ancestor_repairs_itself() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "width: 400px");
    let inner = child_of(&mut doc, outer, "div", "font-size: 16px; line-height: 20px");
    text_in(&mut doc, inner, "label");
    let blk = child_of(&mut doc, inner, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, outer), 50.0, "precondition");

    doc.set_attribute(
        inner,
        "style",
        "display: contents; font-size: 16px; line-height: 20px",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(rect(&doc, blk).3, 30.0);
    assert_eq!(height_of(&doc, outer), 50.0);
    assert_consistent(&doc, "block ancestor control");
}

// ── The local pixel oracle ─────────────────────────────────────────────────

/// Layout numbers say the box exists; only pixels say it is drawn. The
/// fixtures below fill exactly one region in pure red and nothing else is red,
/// so the correct count is provably `width * height` and the broken one is
/// provably **zero** — the local oracle a whole-screen comparison cannot give
/// (a defect this size is a fraction of a percent of the frame).
#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Count of pixels that are exactly opaque `rgb(255, 0, 0)`.
    fn red_pixels(doc: &mut RinchDocument) -> usize {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut layout_cx,
        );
        painter
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| **p == [255, 0, 0, 255])
            .count()
    }

    /// The wrapped block paints the same 800x40 = 32000 red pixels the
    /// unwrapped one does. Broken: **0** — not mispositioned, absent.
    #[test]
    fn a_wrapped_block_paints_the_same_pixels_as_an_unwrapped_one() {
        let ((mut wdoc, _, _), (mut pdoc, _, _)) = wrapped_and_plain();

        let plain = red_pixels(&mut pdoc);
        assert_eq!(
            plain,
            (VW as usize) * 40,
            "control: the unwrapped block fills its whole box"
        );
        assert_eq!(
            red_pixels(&mut wdoc),
            plain,
            "the wrapped block must paint exactly the same pixels; 0 is the \
             orphan, which is drawn by nothing"
        );
    }

    /// Two wrapped blocks paint both boxes — 800x40 + 800x25. One block cannot
    /// tell "paints none of them" from "paints all but the first".
    #[test]
    fn two_wrapped_blocks_both_paint() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = child_of(&mut doc, body, "div", "font-size: 16px");
        text_in(&mut doc, outer, "label");
        let wrapper = child_of(&mut doc, outer, "div", "display: contents");
        child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
        child_of(&mut doc, wrapper, "div", "height: 25px; background: red");
        doc.resolve_layout(VW, VH);

        assert_eq!(
            red_pixels(&mut doc),
            (VW as usize) * 65,
            "both wrapped blocks must be drawn"
        );
    }
}
