//! How far a subtree reaches, for the two questions that need to know.
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
//! An element with `opacity < 1` is composited through a group layer, and every
//! `push_layer` in this crate is handed a *bounds* shape along with the opacity.
//! The two painters read that shape very differently.
//! [`TinySkiaPainter::push_layer`](super::skia_painter::TinySkiaPainter) names
//! the parameter `_bounds` and never looks at it — its layer is a pixmap the
//! size of the whole surface, composited back whole — while
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
//! [`compose_node_transform`], the same order of early returns. That is
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
//!   above. **Not handled**: this walk still narrows an absolute at every
//!   clipping ancestor, so a layer holding one can come back too small in the
//!   same way. It is pre-existing rather than new, it needs a *partial* escape
//!   that `Escapes` cannot express, and it is filed as **#550** — named here so
//!   the gap is visible instead of latent.
//! - **`position: sticky`** takes the **full** chain and is correctly not one of
//!   them; its `Unknown` is about coordinates, not clipping, and narrowing it is
//!   right.
//!
//! #545, #547, #550.

use peniko::kurbo::{Affine, Rect, Vec2};

use super::{compose_node_transform, painter::PaintShape};
use crate::computed_style::{DisplayValue, PositionValue};
use crate::node::{DisplayMode, Node, NodeKind, NodeTree, RawNodeId};

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
/// clips the box: a clip the fixed box's owning stacking context was itself
/// hoisted past does still reach it (#549), so `Escapes` can be more generous
/// than the truth. That is the direction this module errs in on purpose.
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
    let Some(node) = tree.get(node_id) else {
        return UNBOUNDED;
    };
    // The walk re-derives every node's position as `offset + layout.x * scale`,
    // including the root's, so hand it the offset that reproduces the `x`/`y`
    // paint already computed. Going the other way — trusting the walk to
    // recompute the root's position — would lose the sticky adjustment paint
    // applied above the call site.
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;

    let mut walk = Walk {
        tree,
        scale,
        budget: MAX_VISITS,
        skip_root_clip: false,
    };
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
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

/// Would `node_id`'s clip bracket cut anything that is actually drawn?
///
/// `false` means "it might, or I could not tell", and is the answer this
/// returns for every kind of not-knowing — which is what makes it safe to elide
/// a clip on a `true`: a wrong `true` is a rendering bug, a wrong `false` costs
/// one clip layer that was already being pushed. Card K43.
///
/// `rect` is the clip's own rect in the same space [`opacity_layer_bounds`]
/// answers in — physical pixels, before the node's CSS transform, i.e. exactly
/// the `Rect` `clip_shape` handed the caller.
///
/// # Why this is a walk and not four lines of arithmetic
///
/// "Does anything inside reach past the box" is the question this module
/// already answers, and answering it again separately is how the two drift.
/// The first draft of this optimisation did re-derive it, and got the one thing
/// wrong that a re-derivation always gets wrong: it compared an IFC root's text
/// against the **border-box** origin, while `paint_node` draws that text at the
/// **content** origin (`ifc_root_content_origin`, padding + border). Measured,
/// on a `width: 40px; padding-left: 100px; overflow: hidden` box holding a 75px
/// run: the clip was elided as having nothing to cut, and 268 ink pixels landed
/// outside it. The walk gets that right at both places it matters — the root's
/// own text and an inline-block an IFC positions — because it mirrors paint
/// rather than paraphrasing it, and it also brings the visit budget,
/// `MAX_DEPTH`, and the [`Extent::Escapes`] discipline for free.
///
/// # The trap, which is that this looks right and is vacuous without it
///
/// [`Walk::node`] narrows a clipping node's children by that node's own clip.
/// Asking [`opacity_layer_bounds`] about a clipping box therefore returns that
/// box — every time, whatever is inside it — so the obvious spelling,
/// `opacity_layer_bounds(...) ⊆ rect`, is **true for every clipping box in the
/// document** and elides all of them. `skip_root_clip` is what suspends that
/// one intersection for the root, and only for the root. If this function ever
/// starts eliding everything, that flag is the first thing to check.
///
/// A sticky descendant's [`Extent::Unknown`] rides on the same point: it is
/// unnarrowed here on purpose, so it reaches the top as a not-knowing and
/// answers `false`, rather than being narrowed to the clip that is under
/// question and answering `true`.
pub(super) fn clip_cuts_nothing(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
    rect: Rect,
) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let offset_x = x - node.layout.x as f64 * scale;
    let offset_y = y - node.layout.y as f64 * scale;

    let mut walk = Walk {
        tree,
        scale,
        budget: MAX_VISITS,
        skip_root_clip: true,
    };
    match walk.node(node_id, offset_x, offset_y, Affine::IDENTITY, true, 0) {
        // Half a device pixel of slack, matching the tolerance every other
        // geometric comparison in paint carries: a box whose right edge is its
        // container's right edge must read as fitting, and layout arithmetic
        // does not land on the exact same float twice.
        Extent::Within(r) => {
            r.x0 >= rect.x0 - 0.5
                && r.y0 >= rect.y0 - 0.5
                && r.x1 <= rect.x1 + 0.5
                && r.y1 <= rect.y1 + 0.5
        }
        // Paint draws nothing at all in here, so there is nothing for the clip
        // to cut. Note this is *not* the same as `Within` of a zero-area rect,
        // which is why the enum has both.
        Extent::Nothing => true,
        // `Unknown` (a sticky descendant) and `Escapes` (a fixed one, the visit
        // budget, `MAX_DEPTH`) are both "I could not place it", and a clip is
        // never elided on a not-knowing.
        Extent::Unknown | Extent::Escapes => false,
    }
}

/// Convenience for the call sites, which all want a [`PaintShape`].
pub(super) fn opacity_layer_shape(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    x: f64,
    y: f64,
) -> PaintShape {
    opacity_layer_bounds(tree, node_id, scale, x, y).into()
}

struct Walk<'a> {
    tree: &'a NodeTree,
    scale: f64,
    /// Nodes left to look at before the walk gives up. Shared across the whole
    /// walk, not per level, so the cost of one call is bounded whatever shape
    /// the subtree has.
    budget: u32,
    /// Do not narrow the **root's** children by the root's own clip.
    ///
    /// Only [`clip_cuts_nothing`] sets this, and it is the difference between
    /// that function working and being vacuous. See its doc comment.
    /// Descendants' clips always apply, in both callers.
    skip_root_clip: bool,
}

impl Walk<'_> {
    /// The extent of everything `paint_node` would draw for this node and its
    /// subtree, in the layer's coordinate space.
    ///
    /// `is_root` marks the node the layer belongs to. It changes two things:
    /// the node's own CSS transform is *not* applied (the painter applies it to
    /// the bounds shape itself), and the escape hatches for boxes that paint
    /// somewhere other than where this walk would put them do not fire — the
    /// root is by definition being painted right here.
    fn node(
        &mut self,
        node_id: RawNodeId,
        offset_x: f64,
        offset_y: f64,
        parent_transform: Affine,
        is_root: bool,
        depth: u32,
    ) -> Extent {
        // Giving up answers [`Extent::Escapes`], not `Unknown`, and the
        // distinction is the whole of #547 F7: "I did not look" includes "there
        // may be a box in here that escapes your clip". An `Unknown` is narrowed
        // by the first clipping ancestor on the way back up, so a fixed box past
        // the budget — or deeper than `MAX_DEPTH` — inside a clipper produced a
        // layer smaller than what paint draws, which is the F2 defect reached by
        // a second route.
        //
        // This restores what [`MAX_VISITS`]'s own documentation already promises
        // ("past it the answer is `UNBOUNDED` … so the fallback costs a
        // full-target clip and never a wrong picture") — that was simply untrue
        // whenever a clipping ancestor sat between here and the layer root.
        //
        // Worth keeping the shape of that in mind, because it generalises: the
        // promise was not deleted as over-claiming, it was **made true**, and
        // the history stayed here at the guard that fixed it rather than in the
        // promise. A doc comment that says something false is usually saying
        // what the code was meant to do, and the cheaper repair — softening the
        // claim — throws away the specification and leaves the defect. Check
        // which of the two is wrong before assuming it is the prose.
        //
        // It is also what lets [`Extent::is_final`] be the simple predicate it
        // is: this guard runs *before* every other arm, so once the budget is
        // gone every remaining sibling answers `Escapes` and the loop stops on
        // the next one.
        if self.budget == 0 || depth > MAX_DEPTH {
            return Extent::Escapes;
        }
        self.budget -= 1;

        let Some(node) = self.tree.get(node_id) else {
            return Extent::Nothing;
        };

        // The same refusals `paint_node` makes, in the same order. Anything it
        // declines to draw contributes nothing to the layer's extent — and the
        // `opacity <= 0.0` one is worth more than it looks, because it is the
        // always-mounted scrim from card K24: a full-screen element that is not
        // there, whose subtree this walk would otherwise measure every frame.
        //
        // **Why `Nothing` is safe here and `Escapes` is not needed — and why two
        // of these four are safe only by accident.** `Nothing` is the `union`
        // identity, so a subtree that answers it vanishes silently rather than
        // being narrowed: it is the *worse* failure of the two if it is ever
        // wrong. Each refusal has to mean "paint draws nothing here, hoisted
        // descendants included", and that is a stronger claim than "paint skips
        // this node" — `stacking::Collector::collect_hoisted` reaches a hoisted
        // entry directly, not through its parent, so a parent paint refuses does
        // not refuse what was hoisted out of it.
        //
        // - `display: none` and `opacity <= 0.0` are safe **by construction**.
        //   A `display: none` node is not in layout at all, so nothing under it
        //   is hoisted anywhere. `opacity < 1` creates a stacking context, so a
        //   fixed descendant is hoisted no further than this node (#545) and
        //   paint refuses the node itself — nothing escapes. Note what that
        //   rests on: if `Node::creates_stacking_context` ever stopped answering
        //   to `opacity`, this exit would start losing boxes silently.
        // - The tag list and `estimated_height` are the same asymmetry as F7 and
        //   are **latent rather than safe**: neither has a matching guard in
        //   `collect_hoisted`, so a hoisted descendant of one would be painted
        //   while this walk answered `Nothing`. Neither is reachable today — the
        //   tags get `display: none` from the UA stylesheet and would be caught
        //   by the exit below anyway, and a `position: fixed` box inside a
        //   virtualized editor block is not something the editor model can
        //   produce. If either ever becomes reachable, the answer is `Escapes`,
        //   for exactly the reason the give-up guard above returns it.
        if let NodeKind::Element(ref el) = node.kind
            && matches!(
                el.tag.as_str(),
                "style" | "script" | "head" | "meta" | "link"
            )
        {
            return Extent::Nothing;
        }
        if node.estimated_height.is_some() {
            return Extent::Nothing;
        }
        let cs = &node.computed_style;
        if cs.opacity <= 0.0 && cs.display != DisplayValue::Contents {
            return Extent::Nothing;
        }
        if cs.display == DisplayValue::None {
            return Extent::Nothing;
        }

        if !is_root {
            // A `position: fixed` box is painted *inside* this layer, so it does
            // widen it — but at coordinates this walk cannot produce. Since #545
            // a fixed box is hoisted only to its nearest ancestor stacking
            // context, and every layer root is one (`opacity < 1` and a
            // transform both create one), so a fixed descendant is an entry of
            // this layer's own sequence or of one nested inside it. Its
            // `layout.x`/`layout.y` are viewport coordinates, though, and this
            // walk accumulates offsets from the layer root — so the subtree
            // alone does not contain the answer and `Unknown` is the honest one,
            // exactly as for `sticky` below.
            //
            // It used to answer `Nothing` for a non-body root, on the grounds
            // that the body reached past every intervening context and painted
            // the box outside this layer. That stopped being true with #545.
            //
            // [`Extent::Escapes`] and not [`Extent::Unknown`], and the
            // difference is not cosmetic: an `Unknown` is narrowed by the first
            // clipping ancestor on the way back up, and a clipping ancestor
            // *inside* this layer does not clip a fixed box — its entry carries
            // an empty clip chain and paint lifts the collecting root's own
            // bracket around it. Collapsing to `Unknown` therefore returned a
            // layer smaller than what it paints, which tiny-skia ignores (it
            // never reads layer bounds) and Vello enforces (it clips
            // `push_layer` to them). That is a **backend divergence**, the
            // single failure this module exists to end, and no pixel assertion
            // against the software painter can see it — so
            // `a_fixed_descendant_is_not_narrowed_by_a_clipper_it_escapes`
            // asserts on the bounds themselves.
            if cs.position == PositionValue::Fixed {
                return Extent::Escapes;
            }
            // `position: sticky` is painted at a position `paint_node` derives
            // by walking *up* to the nearest scroll ancestor — which may well be
            // above the element this layer belongs to. The subtree alone does not
            // contain the answer, and duplicating that ancestor walk here would
            // create a second implementation of it to drift out of step with the
            // first. That drift is precisely the class of bug this module exists
            // to close, so a sticky descendant answers `Unknown`.
            if cs.position == PositionValue::Sticky {
                return Extent::Unknown;
            }
        }

        let layout = &node.layout;
        let scroll = Vec2::new(
            node.scroll_offset.0 * self.scale,
            node.scroll_offset.1 * self.scale,
        );

        if layout.width == 0.0 || layout.height == 0.0 {
            let x = offset_x + layout.x as f64 * self.scale;
            let y = offset_y + layout.y as f64 * self.scale;

            // `display: contents` has no box at all: its children are laid out
            // in the grandparent's space, so paint recurses with the offsets and
            // transform it was given, unchanged.
            if cs.display == DisplayValue::Contents {
                return self.children(node, offset_x, offset_y, parent_transform, depth);
            }

            // A box collapsed to zero in one dimension still keeps its origin
            // and its transform, and paint still walks into it (#142). A box
            // collapsed in *both* is one paint returns from — but the stacking
            // walk descends *through* it to hoist positioned descendants out,
            // and those are painted. Recursing into it costs a level and can
            // only enlarge the result, which is the right way to be wrong.
            //
            // Read that "only enlarge" narrowly: it is measured against *not*
            // recursing, not against what this branch used to return. When the
            // collapsed box is the layer root, `paint_node` passed `UNBOUNDED`
            // here, so any extent computed below — however carefully — is a
            // smaller shape than the one this branch shipped before, and it is
            // the only branch of which that is true. The module doc's safety
            // property is stated with this exception; see "The one rule".
            let transform = self.own_transform(node, x, y, parent_transform, is_root);
            return self.children(node, x - scroll.x, y - scroll.y, transform, depth);
        }

        let x = offset_x + layout.x as f64 * self.scale;
        let y = offset_y + layout.y as f64 * self.scale;
        let w = layout.width as f64 * self.scale;
        let h = layout.height as f64 * self.scale;
        let transform = self.own_transform(node, x, y, parent_transform, is_root);
        let rect = Rect::new(x, y, x + w, y + h);

        // What this node draws for itself, in its own untransformed space,
        // starting from the border box every arm of `paint_node` fills.
        let mut own = rect;

        // An outset `box-shadow` reaches `offset ± (blur + spread)` from the
        // border box. `paint_box_shadow` actually stops at `blur * 0.5 + spread`
        // — the empirical match to Chrome's visible extent — so the whole blur
        // radius is a deliberate half-blur of slack, cheap insurance against
        // that approximation being retuned outward later. Inset shadows are
        // painted inside the box, and `paint_box_shadow` skips them anyway.
        for shadow in &cs.box_shadow {
            if shadow.inset {
                continue;
            }
            let reach = (shadow.blur_radius.abs() + shadow.spread_radius.abs()) as f64 * self.scale;
            let dx = shadow.offset_x as f64 * self.scale;
            let dy = shadow.offset_y as f64 * self.scale;
            own = own.union(Rect::new(
                x + dx - reach,
                y + dy - reach,
                x + w + dx + reach,
                y + h + dy + reach,
            ));
        }

        // An outline is stroked outside the border box, centred on
        // `outline-offset` out, so it reaches `offset + width` — and a negative
        // `outline-offset` only pulls it inward, which the `max` keeps from
        // shrinking the rect below the border box.
        if cs.outline_width > 0.0 {
            let reach = (cs.outline_width + cs.outline_offset.max(0.0)) as f64 * self.scale;
            own = own.inset(reach);
        }

        // Inline content: the text this node lays out as an inline formatting
        // context root, drawn from its content-box origin. `InlineLayout::layout`
        // is built in CSS px and `render_text` re-applies `scale`, so its width
        // and height scale here too. A line that overflows its box — a long
        // unbreakable word — is measured by Parley and so is caught by this.
        //
        // What is *not* caught is glyph ink that reaches past its own line box:
        // an italic's overhang, a tall diacritic, a glyph whose ink exceeds its
        // advance. Parley will not hand that back cheaply — the only route is
        // per-glyph outline bounds, which is a per-frame cost this walk cannot
        // take on — so the layout box is what it uses. That is a slice of ink
        // the border-box bounds this function replaces was already cutting, and
        // the enclosing element boxes are in the union, so it is a pre-existing
        // approximation left where it was rather than a new one introduced here.
        let mut extent = Extent::Within(transform.transform_rect_bbox(own));
        if let Some(inline) = &node.text_layout {
            let (off_x, off_y) = super::ifc_root_content_origin(node);
            let content_x = x + off_x as f64 * self.scale - scroll.x;
            let content_y = y + off_y as f64 * self.scale - scroll.y;
            let text = Rect::new(
                content_x,
                content_y,
                content_x + inline.layout.width() as f64 * self.scale,
                content_y + inline.layout.height() as f64 * self.scale,
            );
            extent = extent.union(Extent::Within(
                transform.transform_rect_bbox(text_shadow_reach(text, node, self.scale)),
            ));
        }

        // A text node paints its cached Parley layout at its own origin. The
        // layout can be a little wider than the box Taffy sized for it (see the
        // on-demand fallback in `paint_node`'s text arm and #127), which is
        // exactly the sort of overhang the border box would have hidden.
        if let (NodeKind::Text(_), Some(cached)) = (&node.kind, &node.cached_text_parley) {
            let text = Rect::new(
                x,
                y,
                x + cached.width() as f64 * self.scale,
                y + cached.height() as f64 * self.scale,
            );
            // Text shadows on a text node come from its parent's style.
            let styled = node.parent.and_then(|p| self.tree.get(p)).unwrap_or(node);
            extent = extent.union(Extent::Within(
                transform.transform_rect_bbox(text_shadow_reach(text, styled, self.scale)),
            ));
        }

        let mut children = self.children(node, x - scroll.x, y - scroll.y, transform, depth);

        // Where the subtree is genuinely clipped, the bounds shrink. The
        // predicate has to be the one `paint_node` opens its clip bracket with
        // and not an approximation of it, so it is literally that function —
        // `Node::clips_overflow`, shared since #324, where this walk used to
        // carry its own copy of paint's `overflow_y`-only spelling and a
        // paragraph explaining that it was deliberately mirroring a deviation.
        //
        // The rect is the border box, radii and all: a rounded clip is inside
        // its own square, so intersecting with the square is the conservative
        // answer and the one an *extent* wants.
        //
        // Paint also sometimes decides not to push a clip it is entitled to
        // (card K43: a clip that covers the render target, or one nothing
        // reaches past). Intersecting anyway stays correct in both of those
        // cases — the first only drops content that is off-window, the second
        // drops nothing at all.
        //
        // `skip_root_clip` is the one exception, and it applies to the **root
        // only**: [`clip_cuts_nothing`] asks what the root's clip would have to
        // cut, so narrowing by that very clip first would answer "nothing" for
        // every clipping box in the document.
        if node.clips_overflow() && !(is_root && self.skip_root_clip) {
            children = children.clipped_to(transform.transform_rect_bbox(rect));
        }

        extent.union(children)
    }

    /// Every child `paint_node` would descend into, at the offsets it would use.
    fn children(
        &mut self,
        node: &Node,
        offset_x: f64,
        offset_y: f64,
        transform: Affine,
        depth: u32,
    ) -> Extent {
        let mut acc = Extent::Nothing;

        // An inline formatting context root paints its inline-block children
        // from the inline layout, at the *content*-box origin, and skips them in
        // its ordinary child walk (`already_drawn_inline`). Reproducing that
        // split matters for more than a padding's worth of offset: an
        // inline-block nested inside a `<span>` is positioned by the IFC that
        // owns it, not by the span, and the ordinary walk never reaches it at
        // all — the span itself is skipped as already drawn.
        let ifc_root = node.text_layout.is_some();

        // The box tree, not the element tree (#566).
        for &child_id in crate::RinchDocument::box_tree_children(&self.tree.nodes, node.id).iter() {
            let Some(child) = self.tree.get(child_id) else {
                continue;
            };
            if ifc_root && child.ifc_root == Some(node.id) && !child.creates_stacking_context() {
                continue;
            }
            acc = acc.union(self.node(child_id, offset_x, offset_y, transform, false, depth + 1));
            // Only `Escapes` may stop the loop — see [`Extent::is_final`].
            if acc.is_final() {
                return acc;
            }
        }

        // The inline boxes, if this IFC has any — and most have none.
        //
        // The gate is worth its own paragraph, because walking a Parley layout
        // to find out is not cheap: `lines()` materialises a `Line` per line,
        // and on a 181-node subtree where 80 of the nodes were a div holding a
        // two-line label, doing it unconditionally was four fifths of the whole
        // walk (16.6us against 3.9us) to find no inline boxes at all. So ask
        // the cheap question first. `ifc.rs` pushes an inline box in exactly
        // one place, for a child whose `display_mode` is `InlineBlock`, and it
        // records every inline child it was told about in `child_positions` —
        // text runs and `<span>`s included. Scanning that list for an
        // inline-block is a handful of slab lookups, and it is `false` for
        // every IFC that is only text.
        let has_inline_boxes = |inline: &crate::node::InlineLayout| {
            inline.child_positions.iter().any(|(id, _)| {
                self.tree
                    .get(*id)
                    .is_none_or(|n| n.display_mode == DisplayMode::InlineBlock)
            })
        };
        if let Some(inline) = node.text_layout.as_ref().filter(|l| has_inline_boxes(l)) {
            let (off_x, off_y) = super::ifc_root_content_origin(node);
            let content_x = offset_x + off_x as f64 * self.scale;
            let content_y = offset_y + off_y as f64 * self.scale;
            for line in inline.layout.lines() {
                // Lines are charged to the same budget as nodes: a very long
                // article inside a translucent element is a walk like any other.
                //
                // `Escapes` for the same reason as the guard at the top of
                // `node` — this is the same give-up, and the lines not looked at
                // may hold an inline-block whose subtree holds a fixed box. (Not
                // a fixed box *directly*: Stylo blockifies an out-of-flow box,
                // so one is never an inline item. The route is one level down,
                // and `an_unvisited_inline_line_may_hide_a_fixed_box` walks it.)
                if self.budget == 0 {
                    return Extent::Escapes;
                }
                self.budget -= 1;
                for item in line.items() {
                    let parley::layout::PositionedLayoutItem::InlineBox(positioned) = item else {
                        continue;
                    };
                    acc = acc.union(self.node(
                        positioned.id as RawNodeId,
                        content_x,
                        content_y,
                        transform,
                        false,
                        depth + 1,
                    ));
                    if acc.is_final() {
                        return acc;
                    }
                }
            }
        }

        acc
    }

    fn own_transform(
        &self,
        node: &Node,
        x: f64,
        y: f64,
        parent_transform: Affine,
        is_root: bool,
    ) -> Affine {
        if is_root {
            // The painter applies the layer's transform to the bounds shape, and
            // that transform already carries this node's own. Composing it again
            // here would rotate the rect twice.
            parent_transform
        } else {
            compose_node_transform(node, x, y, self.scale, parent_transform)
        }
    }
}

/// `rect` grown by however far `node`'s `text-shadow` carries its glyphs.
///
/// `render_text_with_shadow` offsets the shadow pass in *unscaled* layout units
/// while the main text is drawn scaled, so the reach is taken at whichever of
/// the two is larger; a shadow drawn nearer than the rect allows is not a
/// problem, one drawn further is. The blur radius is included even though this
/// painter draws shadow text without blurring it, because the day it does the
/// glyphs will spread by it.
fn text_shadow_reach(rect: Rect, node: &Node, scale: f64) -> Rect {
    let shadows = &node.computed_style.text_shadow;
    if shadows.is_empty() {
        return rect;
    }
    let unit = scale.max(1.0);
    let mut grown = rect;
    for shadow in shadows {
        let blur = shadow.blur_radius.abs() as f64 * unit;
        let dx = shadow.offset_x as f64 * unit;
        let dy = shadow.offset_y as f64 * unit;
        grown = grown.union(Rect::new(
            rect.x0 + dx - blur,
            rect.y0 + dy - blur,
            rect.x1 + dx + blur,
            rect.y1 + dy + blur,
        ));
    }
    grown
}
