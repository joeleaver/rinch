//! The CSS paint sequence for a stacking context's children.
//!
//! Paint and hit testing are the same walk run in opposite directions: paint
//! draws a stacking context's children front-to-back, hit testing probes them
//! back-to-front and returns the first that answers. They used to compute that
//! order twice, from two different sets of rules, and the two disagreed — which
//! is a class of bug where a box is drawn somewhere the taps do not go. This
//! module is the single answer both of them ask.
//!
//! # The sequence
//!
//! For a stacking-context root, [`stacking_paint_order`] returns its children in
//! CSS 2.1 Appendix E order, back to front:
//!
//! 1. **Negative-`z-index` stacking contexts**, ascending `z`, then tree order.
//! 2. **In-flow, non-positioned, non-stacking-context children**, in tree order.
//! 3. **The `z == 0` level**, in tree order: positioned descendants with
//!    `z-index: auto` (Appendix E step 8 — the step Rinch had no phase for at
//!    all), interleaved with stacking contexts whose `z-index` is `auto` or an
//!    explicit `0`.
//! 4. **Positive-`z-index` stacking contexts**, ascending `z`, then tree order.
//!
//! Steps 3 and 4 are one sort, not two passes: every hoisted entry carries a
//! `z_index` (`auto` counting as `0`) and a tree-order index, and the sort key is
//! the pair. A positioned `z-index: auto` box is entered at `z == 0`, so a FAB
//! written after a scroller in the markup lands above it, and one written before
//! it lands below — which is what tree order means at a single z level.
//!
//! # Hoisting
//!
//! A child is *hoisted* when it does not paint in its parent's own tree-order
//! run but at the nearest stacking-context ancestor's ordered sequence. Both
//! stacking contexts and positioned `z-index: auto` boxes hoist —
//! [`paints_at_stacking_root`] is the predicate, and a parent walking its
//! children in tree order must skip everything it answers `true` for, or the
//! subtree is painted twice.
//!
//! The collection walk keeps descending *through* a hoisted positioned
//! `z-index: auto` box, so its own positioned and stacking-context descendants
//! surface in this same sequence rather than inside it. That is Appendix E step
//! 8's second half — the box is treated as if it created a stacking context,
//! "but any positioned descendants and descendants which actually create a new
//! stacking context should be considered part of the parent stacking context".
//! It stops at a real stacking context, whose descendants are that context's
//! business.
//!
//! # Clip chains
//!
//! Hoisting a box out of its parent's run moves it out of every clip bracket
//! between it and the collecting root — and `overflow` does not create a
//! stacking context, so those brackets are ordinary boxes the walk passes
//! straight through. Each entry therefore records the **chain of clipping
//! ancestors** it was hoisted past, as a [`ClipSpan`] into [`PaintOrder::clips`],
//! and its consumer re-applies them on entry: paint pushes them, hit testing
//! rejects a probe point outside them. That is what lets `creates_stacking_context`
//! match the CSS list (#324) instead of forming a context for every scroller so
//! that the bracket would happen to enclose the right set of boxes.
//!
//! A clip is recorded at the clipping node's own **pre-scroll** painted origin.
//! A container's box does not move when its content scrolls, and taking the
//! chain rect from the walk's accumulated (scrolled) offset is invisible at
//! scroll offset 0. Pinned by
//! `clip_chain_tests::a_scrolled_container_clips_a_hoisted_box_at_its_own_box`,
//! which probes both sides of the difference — the same shape
//! `clip_predicate_tests::a_scrolled_container_clips_at_its_own_box` pins for
//! the bracket.
//!
//! The **collecting root's own** clip is deliberately not in any chain. Paint
//! opens that bracket before it walks the sequence and hit testing gates the
//! whole walk on it, so putting it in the chain would apply it twice; the walk
//! starts at the root's children for exactly that reason.
//!
//! ## `position: absolute` truncates its chain
//!
//! CSS does not clip an absolutely positioned box by an `overflow` ancestor
//! that is not in its **containing-block** chain: the box is positioned against
//! its containing block, and a scroller it merely sits inside in the markup has
//! nothing to say about it. So the walk tracks how many clips were on the stack
//! at the nearest [`Node::establishes_abs_containing_block`] ancestor, and an
//! absolute entry's chain is truncated there. `position: fixed` resolves against
//! the viewport and takes an **empty** chain.
//!
//! This is also what keeps #204's initial-containing-block correction correct:
//! an absolute that `out_of_flow.rs` resolved against the viewport must not then
//! be clipped by the unpositioned scroller it was written inside.
//!
//! **Known gap.** The root's own bracket is applied unconditionally, so an
//! absolute hoisted to a root that clips but establishes no containing block —
//! `opacity: 0.9; overflow: hidden` on a static box, and nothing positioned
//! between — is clipped by it where CSS would not. Layout places such a box
//! against its real containing block (#386); this is the paint half of that
//! shape, and needs the clip moved off the bracket to fix.
//!
//! # Clip groups
//!
//! Entries under one chain share one push, but the sort by `z-index` can
//! alternate two unrelated panes' rows, `[A], [B], [A], [B], …`, and each switch
//! closes one pane's clip and opens the other's. Vello allocates a clip's tiles
//! over its whole bounds (#1645). [`PaintOrder::grouped_by_clip`] gives paint the
//! same entries with such a run brought together.
//!
//! **Moving an entry is proved safe.** An entry is only moved past entries it
//! is proved apart from. It is painted inside its chain's clips on the painter's
//! own stack, so nothing it draws, a fixed descendant or a shadow included,
//! reaches outside them (a fixed box inside it is lifted out of its own root's
//! bracket and no further, #549). Two entries whose chains share no device pixel
//! touch no pixel in common, so neither one's move changes the other. That proof
//! reads only the clip rects. Every pair that may share a pixel keeps its order,
//! and hit testing keeps walking the true sequence.
//!
//! **Merging is not free of pixels.** Gathering also puts entries of one chain
//! into **one clip layer**, and on Vello a clip is an isolated layer on every
//! tile its edge crosses, so what is inside that layer decides the result there.
//! Pixels at clip edges can change on the GPU. Each of these is an accepted cost
//! (#2074):
//!
//! - **Rounded edges.** Entries in separate layers each take the edge's coverage,
//!   so an earlier entry's colour fringes a later one's; in one layer the edge
//!   applies once.
//! - **`filter: grayscale`.** Its `Saturation` layer blends against what is in
//!   the isolated layer. Alone in its clip it sees nothing, so a tile-wide ring
//!   inside the clip edge is left unfiltered; with the entries below it in the
//!   same layer, the ring goes.
//! - **An in-scene `GameViewport` clear**, the same way: it erases what is in
//!   the layer, so with the entries below it there, it clears the ring as well.
//!
//! The software painter has no such layer and its pixels do not change. Which
//! picture Vello gives already depended on how entries interleaved, and it still
//! does: an unrelated pane moving can start or end a grouping, and with it change
//! those edge pixels.
//!
//! # Transforms
//!
//! The accumulated offsets never cross a CSS transform, so an entry always lands
//! in the collecting root's own space — the space the painter's `node_transform`
//! and hit testing's probe point are both already in. A transformed box creates a
//! stacking context, so the walk stops at it; and a positioned `z-index: auto`
//! box that the walk descends *through* has, by the same rule, no transform of
//! its own. The clip chain inherits that: every clip it records is in the
//! collecting root's untransformed space, so a consumer pushes all of them under
//! the root's own transform with no per-clip composition.
//!
//! # `position: fixed`
//!
//! A fixed box is viewport-**positioned**, not viewport-**stacked**. Its
//! `layout.x`/`layout.y` are already viewport coordinates, so its entry takes
//! zeroed offsets and (per the rule above) an empty clip chain — which is **not**
//! the same as escaping every clip, and the difference is #549; the third
//! consequence below is the whole of it, so read that before relying on this
//! sentence. It is hoisted only as far as its **nearest ancestor stacking
//! context**, exactly like any other box that creates one, because that is what
//! CSS 2.1 Appendix E says and what a browser does.
//!
//! It used to be pulled out to the body's sequence whatever lay between (#545).
//! That compared `z-index` values across two stacking contexts — the same fault
//! `overflow` caused before stage B — so a `z-index: 99` dismiss backdrop
//! escaped a wrapper its `z-index: 100` panel could not, and covered it. There
//! is no `is_body` special case in this module any more; the body is simply the
//! outermost stacking context, and a fixed box with no nearer one lands there
//! by the ordinary walk.
//!
//! Three consequences its consumers must honour, because a fixed entry can now
//! appear under a root that clips or transforms:
//!
//! - **The collecting root's own clip does not apply to it.** The root is not
//!   its containing block. Paint lifts its bracket around such an entry and hit
//!   testing exempts it from the root's bounds gate; without that, a fixed modal
//!   inside an `overflow: hidden; z-index: 1` panel would be clipped away, which
//!   [is not what a browser does](https://drafts.csswg.org/css-position/#fixed-pos).
//!   That is #324 stage B's documented "Known gap", which this made live.
//! - **Clips *above* the collecting root still do apply — #549.** This is the
//!   limit of the bullet above and the thing not to misread: "a fixed entry
//!   takes an empty clip chain" is **not** "a fixed box escapes every clip". Its
//!   own chain is empty, but the chain on the *root's* entry, in whatever
//!   sequence hoisted the root, is pushed around the root's whole subtree — the
//!   fixed box with it. So under `plain clipper > stacking context > fixed` the
//!   box is clipped away, where a browser paints it and where rinch itself did
//!   before #545. Paint and hit testing agree on it, so it is a consistent
//!   deviation rather than drift, and the honest fix is architectural: the
//!   collecting root's clip has to move off the paint-time bracket and into
//!   every entry's chain. Pinned by
//!   `stacking_tests::a_fixed_box_does_not_escape_clips_above_the_context_that_owns_it`.
//! - **The collecting root's own transform does not apply to it either.** A
//!   fixed entry is painted under the *body's* transform, which is where its
//!   coordinates live and what `paint::compute_absolute_position_and_transform`
//!   reports for it. (A transformed ancestor should really *contain* a fixed box
//!   — position, clip and transform together — and rinch models none of that;
//!   `out_of_flow.rs` answers "the viewport" for every fixed box. Keeping the
//!   body's transform here leaves that case exactly as wrong as it already was,
//!   rather than letting paint and hit testing disagree about it.)
//!
//! A fourth consumer has to know the same thing from the other side.
//! `paint::layer_bounds` measures a translucent layer by walking the
//! **tree**, not this sequence, so it sees clipping ancestors that a fixed
//! descendant's entry escapes; narrowing to one of those returns a layer smaller
//! than its own content, which tiny-skia ignores and Vello enforces. Its
//! `Extent::Escapes` case exists for exactly that — and, since #547, for either
//! of its give-up guards too, on the same principle: a walk that stopped early
//! cannot claim a clip bounds what it did not visit. The two modules are one
//! question — *which clips actually apply to a hoisted fixed box* — answered in
//! three places, and it is worth checking all three before assuming a fix is
//! local.

use std::ops::Deref;

use peniko::kurbo::{Affine, Rect};

use crate::paint::radii::BorderRadii;

use crate::computed_style::{DisplayValue, PositionValue};
use crate::node::{Node, NodeTree, RawNodeId};
use crate::paint::clip_shape;

/// How a consumer descends into a [`PaintEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintKind {
    /// A child painted in its parent's tree-order run: an in-flow,
    /// non-positioned box that forms no stacking context. Descend into it as an
    /// ordinary child — it is not a stacking-context root.
    InFlow,
    /// A descendant stacking context, entered as a stacking-context root.
    StackingContext,
    /// A positioned descendant with `z-index: auto`. Entered as an ordinary node
    /// — it is *not* a stacking-context root, because its own positioned and
    /// stacking-context descendants are entries of this same sequence.
    PositionedAuto,
}

/// One clipping ancestor a hoisted entry was lifted past, in the collecting
/// root's own space and in the caller's units.
///
/// Exactly what [`crate::paint::clip_shape`] hands back, which is what makes
/// the chain the same shape as the bracket `paint_node` would have opened —
/// the two cannot drift apart because there is one derivation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipRect {
    /// The clipping box's **padding** box (#536) — CSS clips to the padding
    /// edge, not the border box; the two coincide with no border.
    pub rect: Rect,
    /// Its (padding-box, i.e. border-inset) `border-radius`, already resolved
    /// and scaled. All-zero for a square clip.
    pub radii: BorderRadii,
}

impl ClipRect {
    /// Whether `(x, y)` is inside the clip, **ignoring the radii**.
    ///
    /// Rect-only on purpose: hit testing's own `check_children` gate tests a
    /// clipping box's plain (padding-box, #536) rect with no radii either, so
    /// a rounded chain link that cut its corners here would make a hoisted box
    /// *less* reachable than the unhoisted box beside it. Inclusive on both
    /// edges, like that gate.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.rect.x0 && x <= self.rect.x1 && y >= self.rect.y0 && y <= self.rect.y1
    }
}

/// A range of [`PaintOrder::clips`] — one entry's chain of clipping ancestors.
///
/// A range rather than a `Vec` on the entry so that [`PaintEntry`] stays `Copy`:
/// both consumers copy entries out of the sorted sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClipSpan {
    /// First index into [`PaintOrder::clips`].
    pub start: u32,
    /// One past the last.
    pub end: u32,
}

impl ClipSpan {
    /// No clipping ancestors: the entry paints unclipped by anything between it
    /// and the collecting root.
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    /// Whether this chain is empty — the overwhelmingly common case, and the
    /// one a consumer should short-circuit on.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// How many clipping ancestors are in the chain.
    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }
}

/// How many clips, from the outside in, two chains have in common: the ones a
/// painter can leave open going from one entry to the next.
pub fn shared_clip_prefix(a: &[ClipRect], b: &[ClipRect]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// How many of the newest groups [`PaintOrder::grouped_by_clip`] looks through
/// for an entry's chain. It bounds the pass at this many comparisons an entry
/// when every entry has a chain of its own: a list of cards that each clip one
/// positioned child. An entry it does not reach simply opens a group of its own,
/// which is the order it had.
const CLIP_GROUP_LOOKBACK: usize = 8;

/// The run of a sequence's entries that paint under one chain, while
/// [`PaintOrder::grouped_by_clip`] builds it. A list threaded through a side
/// table, so that gathering a run allocates nothing per group.
struct ClipGroup {
    chain: ClipSpan,
    /// The device pixels the chain lets anything reach.
    pixels: Rect,
    first: u32,
    last: u32,
}

/// Whether two device-pixel rects share no pixel. Spelled as a proof of
/// separation so that a NaN, which proves nothing, answers `false`.
fn pixels_apart(a: Rect, b: Rect) -> bool {
    a.x1 <= b.x0 || b.x1 <= a.x0 || a.y1 <= b.y0 || b.y1 <= a.y0
}

/// One child of a stacking-context root, in paint order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintEntry {
    /// The node to descend into.
    pub node_id: RawNodeId,
    /// How to descend into it.
    pub kind: PaintKind,
    /// Accumulated X offset from the collecting root to this node's *parent*,
    /// in the same units as the `offset_x` handed to [`stacking_paint_order`].
    pub offset_x: f64,
    /// Accumulated Y offset, likewise.
    pub offset_y: f64,
    /// `z-index`, with `auto` counting as `0`. Always `0` for [`PaintKind::InFlow`].
    pub z_index: i32,
    /// The clipping ancestors this entry was hoisted past, as a range of
    /// [`PaintOrder::clips`] — see the module docs. Read it through
    /// [`PaintOrder::clips_for`].
    pub clips: ClipSpan,
}

/// A scroll container whose overlay scrollbars this sequence draws (#1717).
///
/// A scroller that is no stacking context draws its bars at the end of its
/// own `paint_node`, after its in-flow content. Its positioned descendants are
/// not drawn there: they are hoisted to this sequence and drawn after the
/// scroller, so they used to cover its thumb. Chrome 155 draws a scroller's
/// bars right after the last entry of the enclosing stacking context's
/// sequence the scroller *contains* — one whose clip chain runs through it,
/// i.e. whose containing-block chain does — and so below anything after that
/// entry: a later sibling, a box at a higher `z-index`, an absolute that
/// escapes the scroller. That entry is [`Self::after`].
///
/// Only entries at `z >= 0` count: a negative-`z` one is drawn before the
/// scroller itself, so a scroller whose contained entries are all negative is
/// not deferred and draws its bars itself.
///
/// Two scrollers share an `after` only when one contains the other; the inner
/// one comes first in [`PaintOrder::bars`], so the outer one's bars are drawn
/// over it, as Chrome 155 draws them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeferredBars {
    /// The scroll container.
    pub node_id: RawNodeId,
    /// Draw the bars right after the entry at this index of
    /// [`PaintOrder::entries`].
    pub after: usize,
    /// The container's border-box origin in the sequence's own space (the
    /// space of the entries' offsets); a box does not move with its own scroll.
    pub origin_x: f64,
    /// Likewise, the Y origin.
    pub origin_y: f64,
    /// The clips the container itself is drawn under, which its bars are
    /// drawn under too, as a range of [`PaintOrder::clips`] (read it through
    /// [`PaintOrder::clips_for_span`]).
    pub clips: ClipSpan,
}

/// A stacking-context root's paint sequence, plus the clip chains its entries
/// index into.
///
/// Derefs to the entries, so `order.iter()`, `order[i]` and `order.len()` read
/// as they did when this was a plain `Vec<PaintEntry>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaintOrder {
    /// The children, back to front.
    pub entries: Vec<PaintEntry>,
    /// Every chain any entry references, concatenated. Append-only during
    /// collection and never reordered afterwards, so a [`ClipSpan`] survives the
    /// sort that orders `entries`.
    pub clips: Vec<ClipRect>,
    /// Scroll containers in this sequence whose overlay scrollbars are drawn
    /// here, after an entry, rather than by the container's own `paint_node`
    /// (#1717). Sorted by [`DeferredBars::after`].
    pub bars: Vec<DeferredBars>,
}

impl PaintOrder {
    /// The clipping ancestors `entry` was hoisted past, outermost first.
    pub fn clips_for(&self, entry: &PaintEntry) -> &[ClipRect] {
        self.clips_for_span(entry.clips)
    }

    /// The clip chain a [`ClipSpan`] names, outermost first.
    pub fn clips_for_span(&self, span: ClipSpan) -> &[ClipRect] {
        &self.clips[span.start as usize..span.end as usize]
    }

    /// Every device pixel the clips of `span` let anything reach: their
    /// intersection under `to_device`, rounded out to whole pixels because a
    /// clip's edge antialiases the pixel it crosses, and two clips meeting
    /// inside one pixel both draw into it. An empty chain reaches everywhere.
    fn clip_pixels(&self, span: ClipSpan, to_device: Affine) -> Rect {
        let Some((first, rest)) = self.clips_for_span(span).split_first() else {
            return Rect::new(
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::INFINITY,
            );
        };
        let r = rest.iter().fold(first.rect, |r, c| r.intersect(c.rect));
        let d = to_device.transform_rect_bbox(r);
        Rect::new(d.x0.floor(), d.y0.floor(), d.x1.ceil(), d.y1.ceil())
    }

    /// The order to paint [`Self::entries`] in, as indices into them, when it
    /// differs from theirs: entries under one chain brought together across
    /// entries whose chains share no device pixel with it (see **Clip groups**
    /// in the module docs, which says what merging them into one clip layer
    /// does to pixels at clip edges on Vello). `to_device` is the transform
    /// paint pushes the chains under, and it must be the final one: "whole
    /// pixel" means nothing in a space a fractional transform is applied to
    /// afterwards, which is why a drag ghost's paint does not ask.
    /// `None` means paint them as they stand, the usual answer, and it takes
    /// one pass over the entries and allocates nothing.
    ///
    /// An entry joins the newest group of its chain if every group after that
    /// one is pixels apart from the chain, and opens a new group otherwise. So
    /// it never passes anything it may overlap, and it stays behind everything
    /// in the group it joins. Grouping can only remove clip switches: an entry
    /// leaves the neighbours it had for one under its own chain.
    ///
    /// The entries up to each [`DeferredBars::after`] keep their place ahead
    /// of it, so that a bar is drawn at the position `after` names, with
    /// exactly the entries before it that it always had. A bar is drawn outside
    /// its scroller's clip, so no chain says where it reaches.
    pub fn grouped_by_clip(&self, to_device: Affine) -> Option<Vec<u32>> {
        // An entry can only move back to its chain over at least one other
        // group, and returning is a second change of chain.
        let changes = self
            .entries
            .windows(2)
            .filter(|w| {
                let (a, b) = (w[0].clips, w[1].clips);
                a != b
                    && !a.is_empty()
                    && !b.is_empty()
                    && self.clips_for_span(a) != self.clips_for_span(b)
            })
            .count();
        if changes < 2 {
            return None;
        }

        let mut groups: Vec<ClipGroup> = Vec::new();
        let mut next = vec![u32::MAX; self.entries.len()];
        // The first group the next entry may join.
        let mut segment = 0;
        let mut bars = self.bars.iter().map(|b| b.after).peekable();
        let mut last_chain = (
            ClipSpan::EMPTY,
            self.clip_pixels(ClipSpan::EMPTY, to_device),
        );
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.clips != last_chain.0 {
                last_chain = (entry.clips, self.clip_pixels(entry.clips, to_device));
            }
            let (chain, pixels) = last_chain;
            let mut home = None;
            for group in groups[segment..].iter_mut().rev().take(CLIP_GROUP_LOOKBACK) {
                if self.clips_for_span(group.chain) == self.clips_for_span(chain) {
                    home = Some(group);
                    break;
                }
                if !pixels_apart(group.pixels, pixels) {
                    break;
                }
            }
            match home {
                Some(group) => {
                    next[group.last as usize] = i as u32;
                    group.last = i as u32;
                }
                None => groups.push(ClipGroup {
                    chain,
                    pixels,
                    first: i as u32,
                    last: i as u32,
                }),
            }
            while bars.next_if_eq(&i).is_some() {
                segment = groups.len();
            }
        }

        let mut order = Vec::with_capacity(self.entries.len());
        for group in &groups {
            let mut at = group.first;
            while at != u32::MAX {
                order.push(at);
                at = next[at as usize];
            }
        }
        Some(order)
    }
}

impl Deref for PaintOrder {
    type Target = [PaintEntry];

    fn deref(&self) -> &[PaintEntry] {
        &self.entries
    }
}

/// Whether `node` is positioned with `z-index: auto` — CSS 2.1 Appendix E step
/// 8's "positioned descendants with 'z-index: auto'".
///
/// A stacking-order question, not the containing-block one: which box an
/// absolute resolves against is [`Node::establishes_abs_containing_block`]'s,
/// and this deliberately does not ask it.
///
/// A `display: contents` element answers `false` whatever its `position`: it
/// has no box to layer (#1038). A positioned one used to be hoisted to step 8,
/// so its in-flow children painted over later siblings they sit under in
/// Chrome 153.
fn is_positioned_z_auto(node: &Node) -> bool {
    node.box_position() != PositionValue::Static && node.computed_style.z_index.is_none()
}

/// Whether `node` paints at the nearest stacking-context ancestor's ordered
/// sequence rather than in its own parent's tree-order run.
///
/// A parent walking its children in tree order — paint's non-stacking-context
/// branch, hit testing's — must skip every child this answers `true` for: they
/// are already entries of an ancestor's [`stacking_paint_order`], and painting
/// them here as well would draw the subtree twice, at the wrong depth.
pub fn paints_at_stacking_root(node: &Node) -> bool {
    node.creates_stacking_context() || is_positioned_z_auto(node)
}

/// Where a box sits in the paint order, as far as its own style decides it —
/// what [`crate::node::PaintedState`] remembers so that the damage can tell a
/// restyle or a tick that **reorders** the box's subtree against the boxes
/// around it from one that only changes how it looks (#1384).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintOrderKey {
    /// [`paints_at_stacking_root`]: an entry of an ancestor's sequence, not
    /// part of its parent's tree-order run.
    pub layered: bool,
    /// The `z-index` that entry sorts by: the box's own when it is a
    /// stacking context, `0` otherwise (a `z-index` on anything else does
    /// nothing).
    pub z: i32,
    /// [`Node::creates_stacking_context`]: whether its descendants' entries
    /// are collected here rather than hoisted past it. A change of this
    /// alone — a positioned `z-index: auto` box that becomes a context at
    /// `z` 0 — leaves the box's own entry where it was and still reaches its
    /// whole subtree: descendants that sort away from 0 are trapped or
    /// released, group opacity starts or stops compositing everything under
    /// it, and the clip chain pushed around a context's entry starts or stops
    /// applying to descendants that otherwise escape it (#549).
    pub context: bool,
}

impl PaintOrderKey {
    pub fn of(node: &Node) -> Self {
        let context = node.creates_stacking_context();
        Self {
            layered: context || is_positioned_z_auto(node),
            z: if context {
                node.computed_style.z_index.unwrap_or(0)
            } else {
                0
            },
            context,
        }
    }
}

/// The children of the stacking-context root `node_id`, back to front.
///
/// `offset_x`/`offset_y` is the root's own scroll-adjusted content origin — the
/// offset its direct children are laid out against — and every entry's offset is
/// accumulated from there through the non-hoisted boxes in between, so an entry
/// can be descended into directly with no walk of the nodes it skipped.
/// `scale` converts `layout` units to the caller's: paint passes the DPI scale
/// and works in physical pixels, hit testing passes `1.0` and works in layout
/// pixels.
///
/// Every stacking-context root is collected the same way. There is deliberately
/// no "is this the body" flag: a `position: fixed` box is hoisted to its nearest
/// ancestor stacking context like any other, and reaches the body only when
/// there is no nearer one (#545).
pub fn stacking_paint_order(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
) -> PaintOrder {
    build_order::<true>(tree, node_id, scale, offset_x, offset_y)
}

/// [`stacking_paint_order`] without [`PaintOrder::bars`], which only paint
/// reads: hit testing finds a scrollbar under the pointer before it resolves a
/// click (`find_scrollbar_hit`), not through this sequence, and the cold hit
/// test is paid on every pointer move after a layout.
pub fn stacking_hit_order(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
) -> PaintOrder {
    build_order::<false>(tree, node_id, scale, offset_x, offset_y)
}

/// The sequence, with [`PaintOrder::bars`] when `BARS`. A const parameter, so
/// the hit-test copy of the walk carries none of the bar bookkeeping.
fn build_order<const BARS: bool>(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
) -> PaintOrder {
    tree.perf.bump(crate::perf::Counter::StackingOrderBuilds);
    let Some(_node) = tree.get(node_id) else {
        return PaintOrder::default();
    };

    // Steps 1, 3 and 4: everything hoisted to this root, gathered in tree order
    // and then sorted by (z, tree order). Collected first so `order` counts
    // every node the walk passes, direct children included.
    let mut collector = Collector::<BARS> {
        tree,
        scale,
        hoisted: Vec::new(),
        clips: Vec::new(),
        live: Vec::new(),
        cb_depth: 0,
        order: 0,
        bar_owners: Vec::new(),
        open_bars: Vec::new(),
    };
    // The box tree, not the element tree (#566).
    let box_children = crate::RinchDocument::box_tree_children(&tree.nodes, node_id);
    collector.collect_hoisted(&box_children, offset_x, offset_y);

    let Collector {
        mut hoisted,
        mut clips,
        bar_owners,
        ..
    } = collector;
    hoisted.sort_by_key(|(dom_order, e)| (e.z_index, *dom_order));

    let split = hoisted.partition_point(|(_, e)| e.z_index < 0);
    let mut entries: Vec<PaintEntry> = Vec::with_capacity(hoisted.len() + box_children.len());
    entries.extend(hoisted[..split].iter().map(|(_, e)| *e));

    // Step 2: the root's own in-flow, non-positioned children, in tree order.
    // Nothing was hoisted past anything to reach here, so the chain is empty by
    // construction — the root's own bracket is all that applies.
    entries.extend(box_children.iter().filter_map(|&child_id| {
        let child = tree.get_live(child_id)?;
        (!paints_at_stacking_root(child)).then_some(PaintEntry {
            node_id: child_id,
            kind: PaintKind::InFlow,
            offset_x,
            offset_y,
            z_index: 0,
            clips: ClipSpan::EMPTY,
        })
    }));

    let first_hoisted = entries.len();
    entries.extend(hoisted[split..].iter().map(|(_, e)| *e));

    // The scrollers whose bars wait for their last contained entry (#1717).
    // Their chains go after every entry's, so the entries' tail reuse in
    // `Collector::span` is untouched.
    let mut bars: Vec<DeferredBars> = bar_owners
        .into_iter()
        // Innermost first among scrollers sharing an `after`: an outer
        // scroller's bars are drawn over an inner one's (Chrome 155).
        .rev()
        .filter_map(|owner| {
            let last = owner.last?;
            let at = hoisted[split..].partition_point(|(d, e)| (e.z_index, *d) <= last);
            debug_assert!(at > 0, "a contained entry is in the sequence");
            let span = if owner.chain.is_empty() {
                ClipSpan::EMPTY
            } else {
                let start = clips.len();
                clips.extend_from_slice(&owner.chain);
                ClipSpan {
                    start: start as u32,
                    end: clips.len() as u32,
                }
            };
            Some(DeferredBars {
                node_id: owner.node_id,
                after: first_hoisted + at - 1,
                origin_x: owner.origin_x,
                origin_y: owner.origin_y,
                clips: span,
            })
        })
        .collect();
    bars.sort_by_key(|b| b.after);
    PaintOrder {
        entries,
        clips,
        bars,
    }
}

/// The hoisting walk's state: the entries found so far, and the chain of
/// clipping ancestors currently open above the cursor.
struct Collector<'a, const BARS: bool> {
    tree: &'a NodeTree,
    scale: f64,
    /// Hoisted entries, paired with the tree-order index that breaks ties
    /// within a z level.
    hoisted: Vec<(usize, PaintEntry)>,
    /// The output side table, append-only.
    clips: Vec<ClipRect>,
    /// The clipping ancestors between the collecting root and the cursor,
    /// outermost first. A stack: pushed on entering a clipping box, popped on
    /// leaving it.
    live: Vec<ClipRect>,
    /// `live.len()` as of the nearest ancestor that establishes a containing
    /// block for absolutely positioned boxes — the point an absolute entry's
    /// chain is truncated at. Counted *after* that ancestor's own clip is
    /// pushed, because a `position: relative; overflow: hidden` box does clip
    /// its own absolute children.
    cb_depth: usize,
    order: usize,
    /// Scroll containers met on the way down, whose bars may be deferred to
    /// this sequence (#1717), in the order they were met.
    bar_owners: Vec<BarOwner>,
    /// Indices into [`Self::bar_owners`] of the scroll containers the cursor
    /// is inside, outermost first.
    open_bars: Vec<usize>,
}

/// A scroll container the collector descended into, and the last entry it
/// contains so far — see [`DeferredBars`].
struct BarOwner {
    node_id: RawNodeId,
    /// The index of the container's own clip in [`Collector::live`]: an entry
    /// whose chain takes more links than this is clipped, so contained, by it.
    own: usize,
    /// How many links of `live` the container itself is drawn under.
    chain_len: usize,
    origin_x: f64,
    origin_y: f64,
    /// The greatest `(z_index, tree order)` sort key of a contained entry at
    /// `z >= 0`.
    last: Option<(i32, usize)>,
    /// `live[..chain_len]`, copied when the first contained entry is met: by
    /// the time the sequence is sorted `live` has moved on.
    chain: Vec<ClipRect>,
}

impl<const BARS: bool> Collector<'_, BARS> {
    /// How many links of [`Self::live`] apply to `node` — what [`Self::span`]
    /// materialises.
    fn chain_len(&self, node: &Node) -> usize {
        match node.computed_style.position {
            // Viewport-relative: no ancestor between here and the root is in
            // its containing-block chain.
            PositionValue::Fixed => 0,
            PositionValue::Absolute => self.cb_depth,
            _ => self.live.len(),
        }
    }

    /// An entry at `key` with a chain of `chain_len` links was hoisted: every
    /// open scroll container whose clip is in that chain contains it.
    fn note_contained(&mut self, chain_len: usize, key: (i32, usize)) {
        for &b in &self.open_bars {
            let owner = &mut self.bar_owners[b];
            if owner.own >= chain_len {
                continue;
            }
            if owner.last.is_none() {
                owner.chain = self.live[..owner.chain_len].to_vec();
            }
            owner.last = owner.last.max(Some(key));
        }
    }

    /// Materialise the chain that applies to `node`, as a range of
    /// [`Self::clips`].
    fn span(&mut self, node: &Node) -> ClipSpan {
        let n = self.chain_len(node);
        if n == 0 {
            return ClipSpan::EMPTY;
        }

        // Siblings under one scroller all want the same chain, and a scroll
        // region with fifty positioned rows would otherwise copy it fifty
        // times. Reuse the tail when it is already exactly this chain — a
        // memoisation, so correctness never rests on the compare.
        let tail = self.clips.len().saturating_sub(n);
        if self.clips.len() >= n && self.clips[tail..] == self.live[..n] {
            return ClipSpan {
                start: tail as u32,
                end: self.clips.len() as u32,
            };
        }

        let start = self.clips.len();
        self.clips.extend_from_slice(&self.live[..n]);
        ClipSpan {
            start: start as u32,
            end: self.clips.len() as u32,
        }
    }

    /// Gather every hoisted descendant of `children` in tree order.
    fn collect_hoisted(
        &mut self,
        children: &[RawNodeId],
        parent_offset_x: f64,
        parent_offset_y: f64,
    ) {
        for &child_id in children {
            let Some(child) = self.tree.get_live(child_id) else {
                continue;
            };
            // Nothing under `display: none` renders, positioned and fixed
            // descendants included. Descending into such a subtree hoisted
            // every positioned box or stacking context in it (a dimmed toolbar
            // button is one: `opacity < 1`) into this sequence, where paint
            // entered each and, finding no area, pushed an opacity layer over
            // the whole plane: Pimble's hidden toolbars and closed menus put 64
            // such layers in every frame, 2.04M of the 2.81M tiles its 4K window
            // asked Vello for, past Vello's fixed buffer.
            if matches!(child.computed_style.display, DisplayValue::None) {
                continue;
            }
            // Nor does anything under a `visibility: hidden` box with nothing
            // visible below it (#2137) — a closed `Drawer`, `Popover` or menu.
            // `visibility` inherits and a descendant may set it back to
            // `visible`, so this is not the first hidden node but one whose
            // whole box subtree is hidden (`visibility::shows_below`, the hit
            // tester's memo): paint draws nothing for any of it, and hit
            // testing hits none of it, so its positioned boxes and stacking
            // contexts are entries neither walk does anything with. Hoisting
            // them made paint enter each, and push an empty group layer for a
            // translucent one, every frame the overlay stayed closed.
            //
            // Not under an open scroll container that draws bars (#1717): its
            // bars are drawn after its last contained entry, and a hidden one
            // can be that entry, so dropping it would move the bars ahead of
            // whatever sorts between — over which they used to be drawn. Paint
            // still skips each such entry on arrival (`paints_nothing_within`).
            if !(BARS && !self.open_bars.is_empty())
                && crate::visibility::subtree_shows_nothing(self.tree, child)
            {
                continue;
            }
            let dom_order = self.order;
            self.order += 1;

            let is_fixed = child.computed_style.position == PositionValue::Fixed;
            let is_sc = child.creates_stacking_context();

            if !is_sc && !is_positioned_z_auto(child) {
                // Not hoisted: descend through it, accumulating its offset (and
                // its clip, if it has one), to reach the hoisted boxes below.
                self.descend(child, parent_offset_x, parent_offset_y);
                continue;
            }

            // Fixed boxes are viewport-relative: `layout.x`/`layout.y` are already
            // absolute, so the accumulated offset must not be added — to the entry,
            // or to anything hoisted out from under it.
            let (base_x, base_y) = if is_fixed {
                (0.0, 0.0)
            } else {
                (parent_offset_x, parent_offset_y)
            };

            // The chain the *ancestors* impose. Not this box's own clip: paint
            // opens that bracket itself when it enters the entry, and hit
            // testing gates on it there.
            let clips = self.span(child);
            let z_index = child.computed_style.z_index.unwrap_or(0);
            if BARS && z_index >= 0 && !self.open_bars.is_empty() {
                self.note_contained(clips.len(), (z_index, dom_order));
            }

            // No `ifc_content_box_offset` on the entry itself, deliberately, though
            // the obvious symmetry with `descend` below says there should be — and
            // NOT because the correction would be dead code. This sequence is
            // shared with hit testing, whose own `descend`
            // (`crates/rinch/src/app/hit_testing.rs`) adds the IFC offset itself
            // when it enters a node, so the entry's offset must stay the plain
            // border-box chain or every tap on a hoisted inline-block lands one
            // padding+border off (the offset double-added).
            // `a_hoisted_inline_block_is_tapped_where_its_ifc_paints_it` in that
            // file pins it. Paint, for its part, never positions such a box
            // through this entry: with a live IFC it is skipped by
            // `drawn_by_its_ifc`, and with a virtualized one (`estimated_height`)
            // it is not painted at all.
            self.hoisted.push((
                dom_order,
                PaintEntry {
                    node_id: child_id,
                    kind: if is_sc {
                        PaintKind::StackingContext
                    } else {
                        PaintKind::PositionedAuto
                    },
                    offset_x: base_x,
                    offset_y: base_y,
                    z_index,
                    clips,
                },
            ));

            if !is_sc {
                // A positioned `z-index: auto` box is entered as an ordinary node,
                // so its own hoisted descendants are this sequence's, not its.
                // A stacking context, by contrast, owns its descendants outright
                // — its fixed ones included, since #545.
                self.descend(child, base_x, base_y);
            }
        }
    }

    /// Recurse into `child`'s children with `child`'s own layout offset, scroll
    /// and clip folded into the walk's state.
    fn descend(&mut self, child: &Node, parent_offset_x: f64, parent_offset_y: f64) {
        // Unlike the entry push in `collect_hoisted` (which must NOT add this —
        // see the comment there), the offset IS added here: this walk is entering
        // `child`'s own coordinate space to place its hoisted descendants, and an
        // IFC-positioned box's `layout.{x,y}` is content-box-relative, so
        // descending through one without the correction puts every hoisted
        // descendant a padding+border out (#407).
        let (ifc_dx, ifc_dy) = crate::paint::ifc_content_box_offset(self.tree, child);

        // `child`'s own painted origin, *before* its scroll offset: the box does
        // not move when its content scrolls, so this is where its clip is. Its
        // children, which do move, are placed at the scrolled origin below.
        let cx = parent_offset_x + (child.layout.x + ifc_dx) as f64 * self.scale;
        let cy = parent_offset_y + (child.layout.y + ifc_dy) as f64 * self.scale;
        let x = cx - child.scroll_offset.0 * self.scale;
        let y = cy - child.scroll_offset.1 * self.scale;

        let pushed = match clip_shape(child, self.scale, cx, cy) {
            Some((rect, radii)) => {
                self.live.push(ClipRect { rect, radii });
                true
            }
            None => false,
        };
        // A scroll container that may draw bars: its contained entries are
        // noted as they are hoisted (#1717). Only one that clips — its clip is
        // what an entry's chain is checked against.
        let bar_owner = BARS && pushed && (child.scrolls_y() || child.scrolls_x());
        if bar_owner {
            // The links `child` itself is drawn under: `chain_len` as it was
            // before its own clip was pushed.
            let own = self.live.len() - 1;
            let chain_len = match child.computed_style.position {
                PositionValue::Fixed => 0,
                PositionValue::Absolute => self.cb_depth,
                _ => own,
            };
            self.open_bars.push(self.bar_owners.len());
            self.bar_owners.push(BarOwner {
                node_id: child.id,
                own,
                chain_len,
                origin_x: cx,
                origin_y: cy,
                last: None,
                chain: Vec::new(),
            });
        }
        let outer_cb = self.cb_depth;
        if child.establishes_abs_containing_block() {
            self.cb_depth = self.live.len();
        }

        self.collect_hoisted(
            &crate::RinchDocument::box_tree_children(&self.tree.nodes, child.id),
            x,
            y,
        );

        self.cb_depth = outer_cb;
        if bar_owner {
            self.open_bars.pop();
        }
        if pushed {
            self.live.pop();
        }
    }
}

#[cfg(test)]
mod clip_group_tests {
    use super::{
        ClipRect, ClipSpan, DeferredBars, PaintEntry, PaintKind, PaintOrder, RawNodeId,
        pixels_apart,
    };
    use crate::paint::radii::BorderRadii;
    use peniko::kurbo::{Affine, Rect};

    fn clip(x0: f64, x1: f64) -> ClipRect {
        ClipRect {
            rect: Rect::new(x0, 7.0, x1, 311.0),
            radii: BorderRadii::ZERO,
        }
    }

    /// A sequence of one entry per chain named, each chain a list of clips.
    fn order(chains: &[&[ClipRect]]) -> PaintOrder {
        let mut o = PaintOrder::default();
        for chain in chains {
            let start = o.clips.len() as u32;
            o.clips.extend_from_slice(chain);
            o.entries.push(PaintEntry {
                node_id: RawNodeId::default(),
                kind: PaintKind::PositionedAuto,
                offset_x: 0.0,
                offset_y: 0.0,
                z_index: 0,
                clips: ClipSpan {
                    start,
                    end: o.clips.len() as u32,
                },
            });
        }
        o
    }

    fn bar_after(o: &mut PaintOrder, after: usize) {
        o.bars.push(DeferredBars {
            node_id: RawNodeId::default(),
            after,
            origin_x: 0.0,
            origin_y: 0.0,
            clips: ClipSpan::EMPTY,
        });
    }

    fn grouped(o: &PaintOrder) -> Vec<u32> {
        o.grouped_by_clip(Affine::IDENTITY)
            .unwrap_or_else(|| (0..o.len() as u32).collect())
    }

    #[test]
    fn alternating_entries_under_clips_apart_are_gathered() {
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        let o = order(&[&[a], &[b], &[a], &[b], &[a]]);
        assert_eq!(grouped(&o), [0, 2, 4, 1, 3]);
    }

    /// Clips that meet on a whole pixel share none.
    #[test]
    fn clips_meeting_on_a_pixel_edge_are_apart() {
        let (a, b) = (clip(13.0, 101.0), clip(101.0, 211.0));
        assert_eq!(grouped(&order(&[&[a], &[b], &[a]])), [0, 2, 1]);
    }

    #[test]
    fn overlapping_clips_keep_their_order() {
        let (a, b) = (clip(13.0, 101.0), clip(100.0, 211.0));
        assert_eq!(grouped(&order(&[&[a], &[b], &[a], &[b]])), [0, 1, 2, 3]);
    }

    /// Both clips antialias the pixel they meet inside, so it is shared.
    #[test]
    fn clips_meeting_inside_a_pixel_keep_their_order() {
        let (a, b) = (clip(13.0, 101.5), clip(101.5, 211.0));
        assert_eq!(grouped(&order(&[&[a], &[b], &[a]])), [0, 1, 2]);
    }

    /// Whole pixels in the root's space are not whole pixels on the device.
    #[test]
    fn apartness_is_judged_on_the_device() {
        let (a, b) = (clip(13.0, 101.0), clip(101.0, 211.0));
        let o = order(&[&[a], &[b], &[a]]);
        assert_eq!(
            o.grouped_by_clip(Affine::translate((0.25, 0.0))),
            Some(vec![0, 1, 2])
        );
        assert_eq!(
            o.grouped_by_clip(Affine::translate((3.0, 0.0))),
            Some(vec![0, 2, 1])
        );
        assert_eq!(
            o.grouped_by_clip(Affine::scale(1.5)),
            Some(vec![0, 1, 2]),
            "101 lands on 151.5"
        );
    }

    /// A transform can swing two rects apart in the root into one another's
    /// device bounds: a chain is only as apart as its bounding box.
    #[test]
    fn a_rotation_is_judged_by_device_bounds() {
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        let o = order(&[&[a], &[b], &[a]]);
        assert_eq!(o.grouped_by_clip(Affine::rotate(0.3)), Some(vec![0, 1, 2]));
    }

    /// The chain reaches only where all its clips do: the outer clip alone
    /// overlaps everything here.
    #[test]
    fn a_chain_reaches_the_intersection_of_its_clips() {
        let outer = clip(0.0, 400.0);
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        let o = order(&[&[outer, a], &[outer, b], &[outer, a]]);
        assert_eq!(grouped(&o), [0, 2, 1]);
    }

    /// An inner clip can reach past its outer one (a chain is nested in the
    /// tree, not in its rects): no one link of it says where it reaches.
    #[test]
    fn neither_end_of_a_chain_bounds_it_alone() {
        let (narrow, wide) = (clip(13.0, 101.0), clip(50.0, 400.0));
        let other = clip(103.0, 211.0);
        let o = order(&[&[narrow, wide], &[other], &[narrow, wide]]);
        assert_eq!(grouped(&o), [0, 2, 1]);
    }

    /// An entry with no chain can draw anywhere under the root.
    #[test]
    fn an_entry_with_no_clips_is_passed_by_nothing() {
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        let o = order(&[&[a], &[b], &[], &[a], &[b], &[a]]);
        assert_eq!(grouped(&o), [0, 1, 2, 3, 5, 4]);
    }

    /// `[A] [B] [C] [A]` with C over A: the last entry stops at C, though B is
    /// clear of it and the group it would join is further back.
    #[test]
    fn an_entry_does_not_pass_a_group_it_overlaps_on_the_way_to_its_own() {
        let (a, b, c) = (clip(13.0, 101.0), clip(103.0, 211.0), clip(90.0, 102.0));
        let o = order(&[&[a], &[b], &[c], &[a]]);
        assert_eq!(grouped(&o), [0, 1, 2, 3]);
    }

    /// One chain written twice in the side table is one chain.
    #[test]
    fn chains_are_compared_by_their_clips_not_their_spans() {
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        let o = order(&[&[a], &[b], &[a]]);
        assert_ne!(o[0].clips, o[2].clips);
        assert_eq!(grouped(&o), [0, 2, 1]);
    }

    /// Nothing crosses a position a bar is drawn at.
    #[test]
    fn entries_stay_on_their_side_of_a_bar() {
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        let mut o = order(&[&[a], &[b], &[a], &[b], &[a], &[b]]);
        bar_after(&mut o, 2);
        assert_eq!(grouped(&o), [0, 2, 1, 3, 5, 4]);
    }

    /// The lookback is a bound on the work: an entry past it opens a group.
    #[test]
    fn an_entry_past_the_lookback_opens_a_group() {
        let clips: Vec<ClipRect> = (0..10)
            .map(|i| clip(i as f64 * 10.0, i as f64 * 10.0 + 9.0))
            .collect();
        let then = |late: usize| {
            let chains: Vec<&[ClipRect]> = clips
                .iter()
                .chain([&clips[late]])
                .map(std::slice::from_ref)
                .collect();
            grouped(&order(&chains))
        };
        // Clip 2's group is the eighth back, clip 1's the ninth.
        assert_eq!(then(2), [0, 1, 2, 10, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(then(1), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    /// The common shapes take the path that allocates nothing.
    #[test]
    fn a_sequence_with_nothing_to_gather_has_no_grouping() {
        let (a, b) = (clip(13.0, 101.0), clip(103.0, 211.0));
        for chains in [
            &[][..],
            &[&[a][..], &[a], &[a]],
            &[&[a], &[a], &[b], &[b]],
            &[&[a], &[], &[b]],
        ] {
            assert_eq!(order(chains).grouped_by_clip(Affine::IDENTITY), None);
        }
    }

    #[test]
    fn a_nan_is_not_a_proof_of_separation() {
        let nan = Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN);
        assert!(!pixels_apart(nan, Rect::new(0.0, 0.0, 9.0, 9.0)));
    }
}

#[cfg(test)]
mod shared_clip_prefix_tests {
    use super::{ClipRect, shared_clip_prefix};
    use crate::paint::radii::BorderRadii;
    use peniko::kurbo::Rect;

    fn clip(x0: f64, y0: f64, x1: f64, y1: f64) -> ClipRect {
        ClipRect {
            rect: Rect::new(x0, y0, x1, y1),
            radii: BorderRadii::ZERO,
        }
    }

    #[test]
    fn the_outer_clips_two_chains_share_stay_open() {
        let window = clip(0.0, 0.0, 1200.0, 800.0);
        let content = clip(0.0, 36.0, 1200.0, 776.0);
        let tree = clip(0.0, 79.0, 260.0, 776.0);
        let pane = clip(261.0, 79.0, 1200.0, 776.0);
        assert_eq!(
            shared_clip_prefix(&[window, content, tree], &[window, content, pane]),
            2
        );
        assert_eq!(
            shared_clip_prefix(&[window, content], &[window, content, pane]),
            2
        );
        assert_eq!(shared_clip_prefix(&[window, content, pane], &[window]), 1);
        assert_eq!(shared_clip_prefix(&[tree], &[pane]), 0);
        assert_eq!(shared_clip_prefix(&[], &[window]), 0);
        // A chain that diverges and then agrees again shares only the prefix:
        // clips are nested, so nothing after the first difference can stay.
        assert_eq!(
            shared_clip_prefix(&[window, tree, content], &[window, pane, content]),
            1
        );
    }
}
