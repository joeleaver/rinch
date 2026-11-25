//! How far a subtree reaches, for the three questions that need to know.
//!
//! [`opacity_layer_bounds`] is the original one — how large a layer has to be
//! so that it does not cut off what it composites — and everything below is
//! written in its terms. [`clip_cuts_nothing`] is the second (card K43): a clip
//! bracket that provably removes no drawn pixel is not worth pushing, and
//! deciding that is the same walk asked to stop one intersection short. It is
//! here rather than in `paint/mod.rs` for the reason **Mirroring, not
//! re-deriving** gives below, and because the version that lived there got the
//! IFC content origin wrong — see that function's own doc.
//!
//! [`subtree_is_entirely_outside`] is the third (#562): the off-window cull
//! wants to skip a stacking context, and a stacking context's own box is not
//! evidence about a `position: fixed` descendant painted in viewport space. It
//! is the same walk again, asked whether the answer can be acted on at all —
//! so it returns a *decision* rather than a rect, and every not-knowing below
//! answers "no". **It is also the one caller for which an under-measure is not
//! a GPU-only cost**: the other two hand their answer to `push_layer`, where
//! too small loses content on the Vello path; this one hands it to a `return`,
//! where too small loses content everywhere. That is why it is the only caller
//! that refuses #550's hole — see the end of **The mirror has a hole**.
//!
//! An element with `opacity < 1` is composited through a group layer, and every
//! `push_layer` in this crate is handed a *bounds* shape along with the opacity.
//! The two painters read that shape very differently.
//! [`TinySkiaPainter::push_layer`](super::skia_painter::TinySkiaPainter) names
//! the parameter `_bounds` and never looks at it — its layer is a
//! surface-sized pixmap, composited back over whatever area the painter itself
//! saw drawn into it (its own bookkeeping, not this module's bounds) — while
//! [`VelloPainter::push_layer`](super::vello_painter::VelloPainter) passes it
//! straight to `vello::Scene::push_layer`, which *clips* every command inside
//! the layer to it.
//!
//! Paint used to hand both of them the element's border box. That is a
//! reasonable thing to give a compositor as a hint and a wrong thing to give it
//! as a clip: CSS is explicit that a stacking context does not clip its
//! descendants, so a `box-shadow`, an overflowing absolutely-positioned child,
//! or a `transform` that carries a box outside its parent is drawn by the
//! software painter and silently thrown away by the Vello one. The two
//! renderers disagreed about the same document, and that disagreement is what
//! kept the GPU path from becoming the default. See card K36.
//!
//! The cheap fix — pass a rect so large it clips nothing, the way the
//! zero-area path in `paint/mod.rs` already does — makes the two agree by
//! giving Vello no information at all. What this module does instead is keep
//! the bounds *meaningful*: [`opacity_layer_bounds`] walks the subtree and
//! returns the union of what it will actually paint, so Vello's clip becomes an
//! optimisation hint rather than a lie, and the software painter — which
//! ignores it — is unaffected either way. "Rather than a lie" is the honest
//! strength of that claim: it holds absolutely for every layer root with a box,
//! and with one stated exception for a root collapsed to zero area. Both are
//! spelled out under "The one rule" below, and the exception is the first thing
//! to read if something in this module ever appears to cut content off.
//!
//! # The one rule
//!
//! **A rect that is too large costs a little GPU fill. A rect that is too small
//! is the bug this module exists to fix.** Every judgement below is therefore
//! made in the direction of a larger rect, and never a guess: an answer is
//! either mirrored from what `paint_node` does, or it says which kind of
//! not-knowing it is.
//!
//! **That last distinction is the rule, and getting it wrong is what this
//! module's bugs are made of.** "Cannot answer with certainty" is not one state
//! but two, and they surface differently:
//!
//! - [`Extent::Unknown`] — *the walk looked and could not place the box.* It is
//!   still bounded by an ancestor that clips it, so [`Extent::clipped_to`]
//!   narrows it to that clip. Exactly one thing produces it: `position: sticky`.
//! - [`Extent::Escapes`] — *no clip inside this layer may be **assumed** to
//!   bound it.* Nothing narrows it, and it surfaces as [`UNBOUNDED`], the same
//!   ±1e7 rect the zero-area path has always used. Produced by a `position:
//!   fixed` descendant, and by **either give-up guard** — the visit budget and
//!   `MAX_DEPTH` — because a walk that stopped early cannot claim anything about
//!   what it did not visit.
//!
//! This paragraph used to say that anything uncertain answers `Unknown` and that
//! `Unknown` surfaces as `UNBOUNDED`. Both halves were false once a box could
//! escape an intervening clip, and **the second was the belief that produced
//! three separate bugs in one review** (#547 F2, F5, F7): each was a place where
//! a value that *can* be narrowed was returned for a box that must not be, and
//! each was fixed while this summary went on asserting the thing that made them
//! look correct. The specification is the first place to fix and the last place
//! anyone looks — so if a future change adds a third kind of not-knowing, add it
//! here first.
//!
//! The direction of the error still matters more than its presence: too large
//! costs fill, too small loses content on the GPU path only, where no
//! software-rasterized test can see it.
//!
//! A useful safety property falls out of that, and it is worth stating with its
//! exception rather than without, because the exception is where the next bug
//! will be. For a layer root with a **non-degenerate box**, the walk starts
//! from that box and only ever unions onto it — the intersection in
//! [`Walk::node`] applies to *descendants* of a clipping box, never to the
//! subtree root — so the result contains the border-box rect `paint_node` used
//! to pass, and for those roots this change cannot make a layer smaller than it
//! was.
//!
//! The **zero-area root is not covered by that**, and it is the one place this
//! change can clip something that was not clipped before. `paint_node`'s
//! `(width == 0) != (height == 0)` branch paints only children, so the walk's
//! matching branch returns the children's extent with no root box unioned onto
//! it — and that branch used to pass [`UNBOUNDED`] unconditionally, precisely
//! because a bounds that is wrong there blanks the subtree. A computed answer
//! is therefore a real trade, not a free tightening: everything paint draws
//! outside a descendant's layout box that this walk does not model is now
//! clipped under such a root. The known items are an on-demand text layout
//! wider than the box Taffy sized (the fallback arm in `paint_node`'s text arm,
//! #127), `paint_input_value`, which pushes no clip of its own at all, an
//! inline span's `background` padding reaching above its line box, and the
//! glyph ink noted below.
//!
//! It is kept rather than reverted because the alternative — answer
//! [`UNBOUNDED`] for every zero-area root and keep the absolute invariant —
//! throws away the branch's whole benefit to protect a case no one has been
//! able to construct an actual lost pixel for, and because the branch is a
//! container collapsed on one axis, which is rare and rarely holds an
//! unmodelled overhang. **If you are here because something inside a collapsed
//! container is being cut off on the GPU path, this paragraph is the reason and
//! returning [`UNBOUNDED`] at that branch is the fix.**
//!
//! # Mirroring, not re-deriving
//!
//! The walk's arithmetic is copied from `paint_node`'s, node for node: the same
//! `offset + layout.x * scale`, the same scroll subtraction, the same
//! [`compose_node_transform`](super::compose_node_transform), the same order of early returns. That is
//! deliberate. The question this module answers is not "where does CSS say this
//! box is" but "where will *this painter* put it", and the only way to be sure
//! of the second is to do the same sums. Where paint's placement is unusual —
//! an inline-block positioned by an inline formatting context — the walk
//! reproduces the unusual thing rather than the tidy one, and where it cannot
//! (a `position: fixed` descendant, whose coordinates are the viewport's and not
//! this layer's) it answers [`Extent::Escapes`] rather than a tidy wrong number.
//!
//! # The mirror has a hole, and this is its shape
//!
//! "Mirror what `paint_node` does" is the contract above, and for **hoisted**
//! boxes this walk cannot honour it. Paint places those from the **stacking
//! sequence** — [`crate::stacking::PaintEntry`], with a clip chain computed for
//! each entry — while this is a **tree** walk that descends straight through the
//! very boxes the chain leaves out. Three bugs came out of that hole in one
//! review, and they are one under-specified invariant rather than three
//! mistakes, so it is written here:
//!
//! > **For any node whose paint position or clipping comes from a `PaintEntry`
//! > rather than from this walk's own descent, answer with something no ancestor
//! > clip can narrow — and keep looking until every such node in the subtree has
//! > been found.**
//!
//! Both halves earn their place. The first is [`Extent::Escapes`] surviving
//! [`Extent::clipped_to`]: a fixed box hoisted to a stacking-context ancestor
//! takes an empty chain, so a clipping box *between* it and this layer root does
//! not clip it, and answering `Unknown` would let the first such clipper narrow
//! the layer to less than it paints — invisible to tiny-skia, enforced by Vello.
//! The second is [`Extent::is_final`]'s `exhausted` condition: stopping the
//! sibling loop early on some *other* child's `Unknown` skips the fixed box
//! entirely, and the same narrowing follows with nothing in its own subtree
//! wrong.
//!
//! **`stacking::Collector::span` is the authority on which nodes those
//! are**, and it names exactly two — do not guess from the tree:
//!
//! - **`position: fixed`** — chain truncated to nothing. Handled, by `Escapes`.
//! - **`position: absolute`** — chain truncated at its containing block, so it
//!   escapes any clipper *below* that block while remaining clipped by the ones
//!   above. **Not handled for the two bounds callers**: they still narrow an
//!   absolute at every clipping ancestor, so a layer holding one can come back
//!   too small in the same way. It is pre-existing rather than new, it needs a
//!   *partial* escape that `Escapes` cannot express, and it is filed as
//!   **#550** — named here so the gap is visible instead of latent.
//!
//!   [`subtree_is_entirely_outside`] sets [`Walk::absolutes_escape_clips`] and
//!   does not accept it, because for a cull the same under-measure deletes a
//!   box rather than clipping a layer. It does not *close* #550 either: it
//!   answers `Escapes` for an absolute whenever [`Walk::clipper_below_cb`] says
//!   there is a clip in scope it would escape, which is a whole escape where a
//!   partial one would be exact. That costs an unpruned subtree and never a
//!   lost box, which is the direction **The one rule** asks for. Turning the
//!   same flag on for the bounds callers would close #550 for them too, at the
//!   price of [`UNBOUNDED`] on every layer holding such an absolute; that is a
//!   GPU trade nobody has measured, so it stays off there.
//! - **`position: sticky`** takes the **full** chain and is correctly not one of
//!   them; its `Unknown` is about coordinates, not clipping, and narrowing it is
//!   right.
//!
//! #545, #547, #550.

use peniko::kurbo::{Affine, Rect};

use crate::node::{NodeTree, RawNodeId};

mod queries;
mod walk;

pub(super) use queries::{
    clip_cuts_nothing, opacity_layer_shape, subtree_is_entirely_outside, subtree_paint_extent,
};
use walk::Walk;

/// A rect no clip can cut anything out of, at any scale a real window reaches.
///
/// This is the value `paint/mod.rs` has been passing for the zero-area layer
/// case since long before this module existed, and it is what any not-knowing
/// that reaches the top surfaces as. Note *reaches the top*: an
/// `Extent::Unknown` can be narrowed to a clip on the way up and never get
/// here, which is correct for the one thing that produces it and was the bug
/// for everything else (see **The one rule**). tiny-skia ignores layer bounds
/// entirely; Vello clips to them, and clipping to this is clipping to nothing.
pub const UNBOUNDED: Rect = Rect::new(-1e7, -1e7, 1e7, 1e7);

/// How many nodes the walk will look at before it gives up and says
/// [`Extent::Escapes`].
///
/// This runs once per frame per translucent element, on a phone, in a frame
/// budget of 8.3ms — cards K42 and K43 spent a lot of effort getting this app
/// to 120fps on a moto g stylus 5G and this must not be where it goes back.
///
/// Since #562 it also runs once per off-window stacking context the cull is
/// deciding about. **That is not free, and it is not always a saving either.**
/// On a definite answer the walk replaces the paint of a whole subtree, which
/// is the case it is for. On a *not-knowing* it is paid on top: the walk runs,
/// answers [`Extent::Unknown`] or [`Extent::Escapes`], and the subtree is
/// painted in full anyway. Measured on the adverse shape — 60 off-window
/// `opacity: 0.99` stacking contexts, each 200 plain children with a
/// `position: sticky` box **last**, so the sibling loop measures all of them —
/// that is 12,000 extra visits a frame, and it lands below the noise floor
/// either way: 1.165 → 0.943ms against a no-op painter, 555.5 → 558.3ms with
/// `TinySkiaPainter`. The cap below is what keeps it there.
///
/// Measured on the developer laptop, in release, the walk costs about 20ns a
/// node and allocates nothing: a 181-node subtree of rows, cells and labels —
/// the shape of this app's library screen — comes to 3.4us. The cap is
/// therefore not there to make the common case fast; it is there so that no
/// document can make it slow. At 512 the worst case is roughly 10us of walk on
/// a laptop and well under a tenth of a millisecond on the phone, and past it
/// the answer is `UNBOUNDED` — which is exactly the bounds every one of these
/// layers effectively had before this module existed, so the fallback costs a
/// full-target clip and never a wrong picture.
///
/// There is deliberately no cache. A cached bounds would have to be invalidated
/// on every layout change, every style change, and every scroll of every
/// ancestor inside the subtree — and a bounds that is stale by one frame is a
/// clip that cuts off content, which is the bug this module was written to fix.
/// A correct cache is not cheap to prove, and 20ns a node does not need one.
const MAX_VISITS: u32 = 512;

/// How deep the walk will go. A subtree deeper than this is not worth
/// descending for a bounds hint; the same "give up conservatively" rule applies.
const MAX_DEPTH: u32 = 32;

/// What a subtree paints, as far as this walk can tell.
///
/// The four cases are distinct on purpose. `Nothing` is not `Within` a
/// zero-area rect: unioning a rect with a degenerate rect at the origin would
/// drag the result all the way to (0, 0), which is how a "conservative" bounds
/// function quietly becomes a full-screen one. And `Unknown` is not `Within`
/// [`UNBOUNDED`] either, because a `Unknown` subtree under an `overflow: hidden`
/// ancestor is still bounded by that ancestor's clip — see [`Extent::clipped_to`].
///
/// `Escapes` is `Unknown` **plus** the one thing that reasoning does not hold
/// for: no clip inside this layer may be *assumed* to bound it. Two producers,
/// and the wording has to cover both — a `position: fixed` descendant, where the
/// walk knows no clip bounds it, and either **give-up** guard (the budget or
/// `MAX_DEPTH`, in `Walk::node` and in the inline-lines loop), where the walk
/// did not look and so cannot claim anything about what is in there. "May not be
/// assumed to bound it" is what both can honestly say, and it is the same
/// operationally: neither may be narrowed. It exists because collapsing either
/// case to
/// `Unknown` is a silent GPU-only bug, not a conservative approximation —
/// `clipped_to` would narrow it to a clip paint does not apply, and the layer
/// would come back *smaller* than what it paints. tiny-skia ignores layer
/// bounds and would draw the box anyway; Vello clips to them and would throw it
/// away. See the note on `Fixed` in [`Walk::node`].
///
/// It is a claim about what this walk can *bound*, not a proof that nothing
/// clips the box: the clips above a transformed, filtered or masked ancestor,
/// and a transformed or filtered one's own, do still reach a fixed box (#549),
/// so `Escapes` can be more
/// generous than the truth. That is the direction this module errs in on
/// purpose.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Extent {
    /// Provably nothing is drawn.
    Nothing,
    /// Everything drawn lies inside this rect.
    Within(Rect),
    /// Not known. Treat as covering the plane.
    Unknown,
    /// Not known, **and** not bounded by any clip inside this layer.
    Escapes,
}

impl Extent {
    fn union(self, other: Extent) -> Extent {
        match (self, other) {
            // `Escapes` outranks `Unknown`: a union containing something no clip
            // in this layer bounds is itself unbounded by those clips, and
            // widening is always the safe direction here.
            (Extent::Escapes, _) | (_, Extent::Escapes) => Extent::Escapes,
            (Extent::Unknown, _) | (_, Extent::Unknown) => Extent::Unknown,
            (Extent::Nothing, e) | (e, Extent::Nothing) => e,
            (Extent::Within(a), Extent::Within(b)) => Extent::Within(a.union(b)),
        }
    }

    /// Whether the sibling loop may stop here, because nothing still to be
    /// visited can change the answer.
    ///
    /// **`Escapes` stops; nothing else does.** The proof is short and it rests
    /// on the give-up guard at the top of [`Walk::node`] answering `Escapes`:
    ///
    /// - `Escapes` is the top of the lattice — `union` lets it dominate and
    ///   `clipped_to` cannot narrow it — so no later sibling can change it.
    /// - `Unknown` must **not** stop, because a later sibling may be a
    ///   `position: fixed` box and answer `Escapes`, which `clipped_to` must not
    ///   narrow. Returning early on an earlier sibling's `Unknown` loses it and
    ///   the first clipping ancestor then narrows the lot — the F2 defect with
    ///   nothing in the fixed box's own subtree wrong (#547 F5). Since the
    ///   give-up guard now answers `Escapes`, the only remaining producer of
    ///   `Unknown` is `position: sticky`, which says nothing about later
    ///   siblings at all.
    /// - `Nothing` and `Within` are obviously not final.
    ///
    /// **The cost is bounded without a special case, which is why there is not
    /// one.** An earlier revision stopped on `Unknown` when the budget was
    /// exhausted, to keep a wide subtree from iterating its whole child list
    /// past the budget — [`MAX_VISITS`] bounds the nodes *measured*, not the
    /// loop *iterations*. That is no longer needed **and would now be wrong**:
    /// once the budget is gone every remaining sibling answers `Escapes` at
    /// `node`'s first line, so the loop stops on the very next one, and stopping
    /// on an earlier `Unknown` instead would skip exactly those `Escapes`
    /// answers.
    ///
    /// So the total work is bounded by `MAX_VISITS` measured nodes plus at most
    /// one extra iteration per open sibling loop, i.e. `MAX_VISITS + MAX_DEPTH +
    /// 1` calls into `node` — independent of how wide any node is. That bound is
    /// the load-bearing half; the numbers that motivated it (a 20,000-child
    /// subtree at 59.7µs when nothing stopped the loop, against a flat ~11.5µs
    /// at both 5,000 and 20,000 children now) only cover the shapes someone
    /// happened to build.
    fn is_final(self) -> bool {
        matches!(self, Extent::Escapes)
    }

    /// What is left of this extent once it is clipped to `clip`.
    ///
    /// The `Unknown` arm is the interesting one and the reason this type has
    /// more than two cases: content whose extent could not be worked out is
    /// still bounded by an ancestor that clips it, so an `overflow: hidden` box
    /// containing something unanalysable contributes the box, not the plane.
    ///
    /// **That is only sound because `position: sticky` is now `Unknown`'s sole
    /// producer** (one site; `Escapes` has three). A sticky box really is
    /// clipped by its scroll ancestor, so narrowing it is right. Every other
    /// not-knowing this module had — a fixed descendant, the visit budget,
    /// `MAX_DEPTH` — was moved to `Escapes` precisely because this arm narrowed
    /// it and should not have.
    ///
    /// So: **if you are about to return `Unknown` from a new site, the question
    /// to answer first is whether an ancestor clip really does bound what you
    /// could not measure.** If the honest answer is "I did not look", it is
    /// `Escapes`. That single question is the generalisation of #547's three
    /// findings, and asking it is what stops a fourth.
    fn clipped_to(self, clip: Rect) -> Extent {
        match self {
            Extent::Nothing => Extent::Nothing,
            // The whole point of the fourth case: this clip is one paint does
            // not apply to what is inside, so narrowing to it would return a
            // layer smaller than its own content.
            Extent::Escapes => Extent::Escapes,
            Extent::Unknown => Extent::Within(clip),
            Extent::Within(r) => {
                let hit = r.intersect(clip);
                // kurbo's `intersect` returns an inverted rect when the two do
                // not overlap; that is emptiness, not a rect to union with.
                if hit.width() > 0.0 && hit.height() > 0.0 {
                    Extent::Within(hit)
                } else {
                    Extent::Nothing
                }
            }
        }
    }
}

/// The bounds to hand `push_layer` for the layer opened around `node_id`.
///
/// `x`/`y` are the node's painted origin in physical pixels — the same `x`/`y`
/// `paint_node` computed for its own border-box rect, *after* any `position:
/// sticky` adjustment and *before* the node's own CSS transform, because the
/// transform is passed to the painter separately and applies to this shape too.
/// The returned rect is in that same space, so it is a drop-in replacement for
/// the `Rect::new(x, y, x + w, y + h)` that used to be passed.
///
/// The result contains the node's own border box in every case, and contains
/// every descendant this walk placed. Where it could not place one it returns
/// [`UNBOUNDED`] — what every one of these layers effectively had before, a
/// bounds that clips nothing — *unless* a clipping ancestor genuinely bounds
/// what could not be placed, which is the single case `Extent::Unknown`
/// exists for. **The one rule** above is the distinction, and is the thing to
/// read before adding a return to `Walk::node`.
pub fn opacity_layer_bounds(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> Rect {
    match opacity_layer_extent(tree, node_id, scale, x, y) {
        // A zero-area answer is not worth trusting even when it is arrived at
        // honestly: the layer is being pushed because paint is about to draw
        // something into it, and a degenerate clip would blank whatever that
        // is. The zero-area branch in `paint_node` reaches this with an element
        // that has no box of its own and no children, and `UNBOUNDED` is
        // exactly what that branch passed before this function existed.
        Extent::Within(r) if r.width() > 0.0 && r.height() > 0.0 => r,
        _ => UNBOUNDED,
    }
}

/// Whether the subtree at `node_id` provably paints nothing at all, whatever
/// clips are open around it: the walk [`opacity_layer_bounds`] uses answers
/// [`Extent::Nothing`], or that all it draws lies within a rect of no area
/// (anything that draws a pixel, a border, a stroke or a glyph, gives its
/// rect area). `x`/`y` are the node's painted origin, as there.
pub(super) fn paints_nothing_at_all(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> bool {
    match opacity_layer_extent(tree, node_id, scale, x, y) {
        Extent::Nothing => true,
        Extent::Within(r) => !(r.width() > 0.0 && r.height() > 0.0),
        Extent::Unknown | Extent::Escapes => false,
    }
}

/// [`opacity_layer_bounds`] with the root's own `overflow` clip left out:
/// what the subtree paints when the root does not clip it. For a damage
/// computation on a box whose clipping changed since it was painted (#2222):
/// what it painted unclipped is cleared, though it clips now.
pub(super) fn unclipped_root_bounds(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> Rect {
    match layer_extent(tree, node_id, scale, x, y, true) {
        Extent::Within(r) if r.width() > 0.0 && r.height() > 0.0 => r,
        _ => UNBOUNDED,
    }
}

/// What a **moved** box's subtree paints, for its damage (#2222):
/// [`opacity_layer_bounds`] with viewport-`fixed` descendants left out, since
/// they do not move with it.
pub(super) fn moved_subtree_bounds(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> Rect {
    match damage_walk(tree, node_id, scale, x, y, false, true) {
        Extent::Within(r) if r.width() > 0.0 && r.height() > 0.0 => r,
        _ => UNBOUNDED,
    }
}

/// [`opacity_layer_bounds`] for a question asked once rather than once a
/// frame, so with no visit budget: what a drag ghost's snapshot paints,
/// measured when the drag starts (#3065). [`MAX_DEPTH`] still guards the
/// recursion. `None` where the walk cannot bound it all the same (a
/// `position: fixed` or `sticky` descendant, a subtree deeper than
/// [`MAX_DEPTH`]) and where it paints nothing.
pub(super) fn unbudgeted_subtree_bounds(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> Option<Rect> {
    let node = tree.get(node_id)?;
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;
    let mut walk = Walk::new(tree, scale, false, false);
    walk.budget = u32::MAX;
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
        Extent::Within(r) if r.width() > 0.0 && r.height() > 0.0 => Some(r),
        _ => None,
    }
}

/// What `node_id`'s subtree paints as [`moved_subtree_bounds`] measures it,
/// at scale 1 and relative to its border-box origin, for
/// `PaintedState::subtree_reach` (#2222): a moved box whose subtree has not
/// changed since is damaged from it without a walk. Descendants with a
/// recorded reach are not entered, so measuring every box of a freshly
/// painted tree deepest first is linear in the tree. `None` for a box with
/// no area, which the damage never asks about.
///
/// The reach is read back at whatever scale the next paint has and wherever
/// the box has moved to, so no box in it is snapped to a device grid
/// (#2368): it is measured unsnapped, with the snap gain
/// ([`crate::node::SubtreeReach::Within`]) that bounds what snapping adds at
/// any scale and position. [`placed_subtree_reach`] applies both.
pub(crate) fn measure_subtree_reach(
    tree: &NodeTree,
    node_id: RawNodeId,
) -> Option<crate::node::SubtreeReach> {
    use crate::node::SubtreeReach;
    let node = tree.get(node_id)?;
    let (w, h) = (node.layout.width, node.layout.height);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let mut walk = Walk {
        measuring: true,
        ..Walk::new(tree, 1.0, false, true)
    };
    let extent = walk.node(
        node_id,
        -(node.layout.x as f64),
        -(node.layout.y as f64),
        Affine::IDENTITY,
        true,
        0,
    );
    let snap_gain = walk.snap_gain.map(|g| g as f32);
    Some(match extent {
        Extent::Within(r) => SubtreeReach::Within {
            rect: [
                (r.x0 as f32).min(0.0),
                (r.y0 as f32).min(0.0),
                (r.x1 as f32).max(w),
                (r.y1 as f32).max(h),
            ],
            snap_gain,
        },
        // Paints nothing (refused at the root): its box is a bound.
        Extent::Nothing => SubtreeReach::Within {
            rect: [0.0, 0.0, w, h],
            snap_gain,
        },
        Extent::Unknown => SubtreeReach::Unknown,
        Extent::Escapes => SubtreeReach::Escapes,
    })
}

/// Where a subtree whose recorded reach is `rect` / `snap_gain`
/// ([`crate::node::SubtreeReach::Within`]) paints at `scale` with its border
/// box at `(x, y)` (physical px), before its own transform: the rect scaled
/// and placed, with what snapping its boxes to the device grid adds (#2368).
/// The boxes no transform inside moves are snapped on this grid, and
/// rounding is monotonic, so the rect's own snapped self bounds theirs
/// ([`super::pixel_snap::reach_of`]); a transformed one is snapped on its
/// own grid, half a device pixel per edge at most, which its transform
/// carries out by `snap_gain` times that.
pub(crate) fn placed_subtree_reach(
    rect: [f32; 4],
    snap_gain: [f32; 2],
    scale: f64,
    x: f64,
    y: f64,
) -> Rect {
    let [x0, y0, x1, y1] = rect.map(|v| v as f64 * scale);
    let slack = super::pixel_snap::SNAP_SLACK;
    super::pixel_snap::reach_of(Rect::new(x + x0, y + y0, x + x1, y + y1))
        .inflate(slack * snap_gain[0] as f64, slack * snap_gain[1] as f64)
}

/// The subtree's extent as the walk finds it (see [`opacity_layer_bounds`]).
fn opacity_layer_extent(tree: &NodeTree, node_id: RawNodeId, scale: f64, x: f64, y: f64) -> Extent {
    layer_extent(tree, node_id, scale, x, y, false)
}

/// [`opacity_layer_extent`], with the root's own clip left out when
/// `skip_root_clip`.
fn layer_extent(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
    skip_root_clip: bool,
) -> Extent {
    damage_walk(tree, node_id, scale, x, y, skip_root_clip, false)
}

/// The walk behind [`layer_extent`] and [`moved_subtree_bounds`], with each
/// flag of [`Walk`] they set.
fn damage_walk(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
    skip_root_clip: bool,
    skip_fixed: bool,
) -> Extent {
    let Some(node) = tree.get(node_id) else {
        return Extent::Unknown;
    };
    // The walk re-derives every node's position as `offset + layout.x * scale`,
    // including the root's, so hand it the offset that reproduces the `x`/`y`
    // paint already computed. Going the other way — trusting the walk to
    // recompute the root's position — would lose the sticky adjustment paint
    // applied above the call site.
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;

    let mut walk = Walk::new(tree, scale, skip_root_clip, skip_fixed);
    walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0)
}
