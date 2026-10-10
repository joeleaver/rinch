//! Where an out-of-flow box's containing block really is.
//!
//! Taffy resolves an out-of-flow box — `position: absolute` or `position:
//! fixed` — against its **direct parent**, always. CSS says otherwise: a fixed
//! box resolves against the viewport, and an absolute box against its nearest
//! *positioned* ancestor, falling back to the initial containing block when it
//! has none. So `inset: 0` on an absolute child of an unpositioned 300×200 div
//! gives a 300×200 box in Taffy where a browser gives the whole viewport
//! (issue #204) — which is also what `rinch-web` gives, so the desktop backend
//! diverges from the web one.
//!
//! This module answers the one question the correction needs — *which*
//! containing block, when it is not the one Taffy would use — and bakes the
//! resulting **size** into the Taffy style before layout. The matching
//! **position** correction is [`place_absolute`], called as each box is read
//! back, once more after the layout's late position writes and from a scroll:
//! it writes a parent-relative value, so `LayoutResult` stays parent-relative
//! and no coordinate consumer has to learn a new rule.
//!
//! ## What is corrected
//!
//! Three cases are answered here:
//!
//! - [`OutOfFlowKind::Fixed`] — the viewport. Its insets are resolved by the
//!   read-back; an axis with none keeps the static position, placed here
//!   ([`place_fixed_static`], issue #633).
//! - [`OutOfFlowKind::IcbAbsolute`] — an absolute box with no positioned
//!   ancestor at all, whose containing block is therefore the initial
//!   containing block. In rinch the `<html>` box *is* the viewport at (0, 0),
//!   so the ICB is known before layout runs.
//! - [`OutOfFlowKind::AncestorAbsolute`] — an absolute box whose containing
//!   block is an ancestor that is **not** the box Taffy lays it out in (issue
//!   #386): a `position: relative` grandparent, a transformed great-
//!   grandparent. Its containing block is that ancestor's **padding box**.
//!   The ancestor may have no box at all: a positioned **inline** span
//!   (issue #631), whose containing block is the box its fragments span on
//!   their lines — see *An inline span* below.
//!
//! ## An ancestor's size is not known before layout
//!
//! The first two have a containing block whose size exists before the compute.
//! An ancestor's does not: its used size is whatever the compute gives it. So
//! that case is a fixpoint, in the shape `calc_layout` already uses:
//!
//! 1. a style-application site bakes the size from the ancestor's **last**
//!    laid-out padding box, if it has one ([`bake_at_style_site`]) — right
//!    whenever the ancestor does not change size this pass, which is nearly
//!    always;
//! 2. after each root compute, [`RinchDocument::resolve_ancestor_absolutes`]
//!    compares every such box's Taffy style with what the ancestor's size
//!    *now* asks for and rewrites the ones that differ;
//! 3. `resolve_layout` re-runs the compute while that answers `true`. An
//!    out-of-flow box sizes nothing above it, so a containing block's size
//!    never depends on the boxes resolved against it and the loop converges
//!    in one extra compute — plus one per level when a containing block is
//!    itself an absolute resolved against a non-parent.
//!
//! A box whose size does not depend on its containing block (`left: 5px; top:
//! 5px; width: 10px`) is never rewritten and costs no compute: only its
//! position is patched.
//!
//! ## An inline span
//!
//! A `position: relative` (or `sticky`) non-atomic inline element owns no
//! box: Parley lays its text out as part of a line. CSS 2.1 §10.1 makes it a
//! containing block all the same — from the top-left of its first fragment's
//! padding box to the bottom-right of its last's. Three things follow from
//! where its geometry comes from, the lines of the inline formatting context
//! that flows it:
//!
//! - **it is measured per set of lines, not per box.**
//!   [`RinchDocument::measure_inline_containing_blocks`] walks a context's
//!   lines and entries **once** for every span in it that a box hangs from
//!   ([`measure_span_fragments`]) and keeps the answers beside the lines
//!   (`InlineLayout::span_fragments`); every look at a box after that is a
//!   lookup. Lines that are rebuilt come back with no answers and are
//!   measured again; lines that are not, are not. So a paragraph holding a
//!   hundred such boxes costs one linear walk when its text changes and a
//!   hundred lookups per pass otherwise (`abs_inline_measures`,
//!   `abs_inline_measure_steps`);
//! - **the lines are built after the read-back** (`build_ifc_layouts`), so at
//!   the size check of step 2 and at the placement of the read-back a span
//!   whose lines are being rebuilt has no box yet. Both are therefore done
//!   once more after the lines: `resolve_layout` measures the spans, calls
//!   [`RinchDocument::resolve_ancestor_absolutes`] again for these boxes
//!   alone, and goes round — compute, read-back, lines — when that rewrote a
//!   style; and the late re-placement ([`replace_all`]) always runs. Neither
//!   happens in a document whose last read-back met no such box
//!   (`NodeTree::abs_inline_cb_seen`), and a box whose size does not depend
//!   on the span costs no compute, only the second placement;
//! - **the chain ends at the block container holding the line**
//!   ([`inline_host`]), not at the span, which no coordinate walk can stop
//!   at. The span's box is measured from that element's border box, so its
//!   own scroll carries the absolute box with the line, and a scroller
//!   between the box and the span does not.
//!
//! A fragment starts wherever its text does, between pixels, so a position
//! hung from one is snapped to the pixel grid in the host's frame — the grid
//! every box Taffy places is on. (A host that is itself an atomic inline
//! stands between pixels on its line, and so does everything in it.)
//!
//! ## What each pass iterates
//!
//! None of the passes walks the slab, and none of them looks at a box Taffy
//! already resolves correctly — an absolute child of its own positioned
//! parent, the ordinary badge, is in no index here:
//!
//! - step 2 iterates [`NodeTree::ancestor_absolutes`], the boxes whose
//!   containing block was a non-parent ancestor when a style site last synced
//!   them. `read_layout_results` classifies every absolute box anyway, and a
//!   box it finds to be such a one that the set does not hold — an
//!   intermediate box started generating a box with no restyle of this one —
//!   is added there and the layout resolved again in the same pass
//!   ([`note_kind_at_read`]);
//! - the re-placement after the late position writes iterates
//!   `NodeTree::placed_absolutes`, the boxes the read-back placed this
//!   layout — and only in a layout where one of those writes moved something
//!   (`NodeTree::abs_late_moves`, [`replace_all`]);
//! - a scroll looks at one flag on the scrolled node (`Node::on_abs_chain`,
//!   set on every box between a placed box and its containing block as the
//!   read-back walks that chain) and iterates `placed_absolutes` only when it
//!   is set.
//!
//! ## The position
//!
//! [`place_absolute`] is the one answer for both absolute cases, and it is
//! written as a **parent-relative** value so `LayoutResult` keeps its meaning.
//! With `chain` the boxes strictly between the box and its containing block:
//!
//! ```text
//! layout = cb.border + inset + margin − Σ chain.layout + Σ chain.scroll   (an inset axis)
//! layout = Taffy's static position        + Σ chain.scroll                (an auto axis)
//! ```
//!
//! The scroll term is what keeps the box still when a scroller *between* it
//! and its containing block scrolls: it is the containing block's content, not
//! the scroller's (measured in Chrome 153 — a `top: 6px` box and a
//! static-position one both stay put at `scrollTop = 40`). The containing
//! block's **own** scroll is not in the sum, so that one does carry the box;
//! for the initial containing block the `<body>`'s scroll plays that part (it
//! is rinch's page scroll). Since a scroll runs no layout,
//! [`replace_after_scroll`] re-places the affected boxes from
//! `NodeTree::mark_scrolled`. That moves a box and drops no hit-test extent:
//! an absolute box is positioned, so it is in no ancestor's flow extent, and
//! the stacking sequences holding its offset are the ones a scroll drops.
//!
//! ## The static position
//!
//! An axis with both insets `auto` keeps the box where it would have been
//! in the flow (CSS 2.1 §10.3.7, §10.6.4). Taffy's answer — after the
//! preceding in-flow sibling, in the layout parent — is that place, except
//! among **inline content**, which Taffy lays out as one leaf: every box in
//! or beside a run of lines came out below the whole run (issue #632), and
//! a box that was inline-level before `position` made it a block has its
//! place *in* a line (issue #634). So the lines answer for those
//! ([`static_location`]):
//!
//! - the inline walk notes each out-of-flow box it passes
//!   (`IfcText::note_out_of_flow`; a direct child standing inside an
//!   anonymous box's run is not one of the run's members, and is noted from
//!   `Node::run_out_of_flow` — one before the run's first member too, when
//!   no block stands between), and once the lines are broken each note is
//!   turned into two positions (`ifc::resolve_out_of_flow_marks`, kept as
//!   `InlineLayout::out_of_flow`): in the line where the content before the
//!   box ends, at the line box's top — an inline-level box's place — and at
//!   the content edge below that line, a block-level one's (the first line's
//!   top when nothing precedes it);
//! - which of the two is the box's is its own style
//!   (`ComputedStyle::inline_level_before_blockify`, Stylo's
//!   `original_display`), read when the box is placed;
//! - `Node::static_ifc_root` names the root whose lines hold the box's
//!   place and the index of its note there, so a box with none — every box
//!   outside inline content — costs one flag read and keeps Taffy's answer,
//!   and one with a place costs one indexed look, not a search of its
//!   paragraph's notes (`abs_static_lookup_steps`);
//! - the lines are built after the read-back, so
//!   [`place_static_after_lines`] places each noted box again when they are
//!   (and when an anonymous box holding them is read back somewhere else):
//!   directly for an absolute box whose layout parent is its containing
//!   block, which is in no list here, and through [`replace_all`] for the
//!   rest.
//!
//! The position is measured **with nothing scrolled** (Chrome 153: a box
//! shown in a scroller already at `scrollTop = 40` sits where it would at
//! 0). For an absolute box that is the `+ Σ chain.scroll` above. A fixed
//! box's `layout` is a viewport position, so its static position is the sum
//! of the layouts above it with no scroll taken off
//! ([`unscrolled_parent_origin`]), written once per layout: no scroll moves
//! it afterwards, which is right — its containing block is the viewport. A
//! fixed box with a static axis is recorded with the placed absolute boxes
//! (`NodeTree::placed_absolutes`), because the late position writes move it
//! the same way; one with insets on both axes is in no list.
//!
//! ## The shrink-to-fit width
//!
//! An absolute box with an `auto` (or `fit-content`) width is shrunk to fit
//! (CSS 2.1 §10.3.7) in what its **containing block** leaves it: the block's
//! padding-box width less the insets and margins, and — with both inline
//! insets `auto` — less the distance from the block's padding edge to the
//! static position (issue #1404). Taffy measures it in the whole width of
//! the box it lays it out in. Three routes give it the room instead:
//!
//! - a box whose layout parent is its containing block is handed the
//!   `fit-content` keyword ([`fit_in_layout_parent`]), which Taffy measures
//!   at the block less the insets and margins — only when one of those takes
//!   room, so a plain box keeps `auto` and costs one measure;
//! - a box resolved against the ICB or a non-parent ancestor is baked
//!   `fit-content(<px>)` with the room ([`apply_out_of_flow_size_overrides`]);
//! - the static offset is known only once the lines are built, so it is
//!   measured then ([`RinchDocument::resolve_static_shrink_to_fit`]), kept on
//!   the node (`Node::abs_static_offset`) for every bake to read, and the
//!   layout goes round once when it moved. A grid container (whose keyword
//!   measure ignores insets) and a static offset in a box's own parent take
//!   a length from the parent's last layout, checked by the same pass.
//!
//! //! ## What is not corrected
//!
//! - The static position in a **flex** container is Taffy's: the main axis
//!   follows `justify-content`, the cross axis does not follow `align-items`
//!   (Chrome follows both; issue #1492).
//! - A static position follows rinch's **line boxes**, which are not always
//!   Chrome's: a line holding a 26px `inline-block` and text is 26px tall
//!   here and 30 there (issue #663), so a block-level box below it is 4px
//!   high.
//! - A fixed box with no insets and no size still **fills the viewport**
//!   (Chrome shrinks it to its content; issue #893), and a percentage margin on one
//!   placed by Taffy is of the layout parent's width.
//! - A **split** inline span — one holding a block-level child (#513), whose
//!   fragments lie in several anonymous boxes — is not measured: a box inside
//!   it keeps Taffy's answer, the block container (issue #1424). Nor is a
//!   span in a block that draws a `text-overflow: ellipsis` "…": those lines
//!   are rebuilt as flat text with no entry for the span.
//! - A relative span's own `left`/`top` moves neither its text nor a box
//!   resolved against it (issue #1425), and its horizontal padding takes no
//!   room on the line (issue #1426): the containing block is the span as
//!   rinch draws it. So is a raised one (`<sup>`, any positive
//!   `vertical-align`): its fragment follows its glyphs, which rinch draws
//!   higher than Chrome because the line does not grow for them (issue
//!   #1357). A span in a larger font with no text of its own sits 5px high
//!   for the same kind of reason: it gives rinch's line no strut (issue
//!   #1463).
//! - A wrapped right-to-left span's right edge is 4.5px off Chrome's (parley
//!   puts a right-to-left line's trailing space at the line's left end, where
//!   Chrome hangs it out of the line).
//! - **Paint does not clip a span-hung box by a static scroller around the
//!   span** (issue #1438, older than #631): the paint sequence descends the
//!   box tree, never meets the span, and ends the box's clip chain at the
//!   next positioned *box*. Layout, the scroll range and damage do see the
//!   span.
//! - An auto-width box shrunk to fit (see *The shrink-to-fit width*) whose
//!   content wraps is as wide as its widest line, not the room it has
//!   (issue #1276); and a box placed from its static position by a flex
//!   container's `justify-content` is given the room after the container's
//!   content edge, wherever it is then put.
//! - `position: fixed` takes none of the margin, padding or min/max rules
//!   below, and a transformed ancestor does not contain it (#1372).

use crate::RinchDocument;
use crate::computed_style::{
    ComputedStyle, DimensionValue, DisplayValue, LengthPercentageAutoValue, LengthPercentageValue,
    PositionValue,
};
use crate::node::{Node, NodeTree, RawNodeId};
use std::collections::HashMap;

/// The containing block an out-of-flow box resolves against, when it is not the
/// one Taffy would use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutOfFlowKind {
    /// `position: fixed` — the viewport.
    Fixed,
    /// `position: absolute` with no positioned ancestor — the initial
    /// containing block, which in rinch is the `<html>` box: the viewport
    /// at (0, 0).
    IcbAbsolute,
    /// `position: absolute` whose containing block is this ancestor, which is
    /// **not** the box Taffy lays it out in (issue #386): its padding box.
    AncestorAbsolute(RawNodeId),
}

impl OutOfFlowKind {
    /// Whether this is one of the two `position: absolute` cases, which share
    /// one position rule ([`place_absolute`]) and one set of margin, padding
    /// and min/max rules.
    pub(crate) fn is_absolute(self) -> bool {
        !matches!(self, Self::Fixed)
    }
}

/// Whether `node` generates the box its out-of-flow descendants are laid out
/// in — what Taffy calls their parent. A `display: contents` wrapper
/// generates none (its children are spliced into its own layout parent), and
/// a non-atomic `display: inline` element owns none either: an out-of-flow
/// box under it is hoisted into the block container that holds the line
/// (#591).
///
/// Read from the computed style and `display_mode` alone — not from the IFC
/// marks a structural pass leaves — so the answer is the same at a
/// style-application site, which runs before that pass, as after it.
fn generates_layout_box(node: &Node) -> bool {
    node.computed_style.display != DisplayValue::Contents && node.transform_applies()
}

/// Classify `node_id`'s containing block, or `None` when Taffy's
/// direct-parent answer already is the right one (or is a case this module
/// deliberately leaves alone).
///
/// The walk is only entered for a box that is actually out of flow, so the
/// overwhelmingly common node costs one enum compare.
pub(crate) fn out_of_flow_kind(tree: &NodeTree, node_id: RawNodeId) -> Option<OutOfFlowKind> {
    let node = tree.get(node_id)?;
    if matches!(node.computed_style.display, DisplayValue::None) {
        return None;
    }
    // `box_position`, not the computed `position`: a `display: contents`
    // element generates no box for `position` to apply to (#1038), so it has
    // no containing block to resolve against and nothing to place.
    match node.box_position() {
        PositionValue::Fixed => Some(OutOfFlowKind::Fixed),
        PositionValue::Absolute => {
            let mut current = node.parent?;
            // The nearest ancestor that generates a box: where Taffy lays
            // this one out.
            let mut layout_parent = None;
            loop {
                // `<html>`'s box is the viewport at the origin, so reaching it
                // — whatever its own `position` — means the containing block is
                // the ICB.
                if current == tree.html_id || current == tree.root_id {
                    return Some(OutOfFlowKind::IcbAbsolute);
                }
                let ancestor = tree.get(current)?;
                if ancestor.establishes_abs_containing_block() {
                    // The layout parent establishing it is Taffy's own answer,
                    // so there is nothing to correct. Anything else is an
                    // ancestor Taffy does not know about: a box further up
                    // (#386), or one with no box of its own at all — a
                    // positioned inline span, whose fragments are the
                    // containing block (#631) while Taffy lays the box out
                    // in the block that holds the span's line.
                    return (layout_parent.is_some() || !generates_layout_box(ancestor))
                        .then_some(OutOfFlowKind::AncestorAbsolute(current));
                }
                if layout_parent.is_none() && generates_layout_box(ancestor) {
                    layout_parent = Some(current);
                }
                current = ancestor.parent?;
            }
        }
        _ => None,
    }
}

/// Whether `child` is part of `parent`'s **scrollable overflow area**
/// (css-overflow-3 §3.1) — the question `paint::scrollbar::content_extents`
/// asks of every child box of a scroll container: a direct child, or a box
/// reached through `display: contents` children, for which `parent_id` is
/// still the scroll container (a wrapper generates no box, so it contains
/// nothing; issue #396).
///
/// A box swells its parent's scroll range only when that parent is its
/// containing block. The two out-of-flow positions therefore need the same walk
/// [`out_of_flow_kind`] does, and for the same reason: Taffy laid the box out
/// against its direct parent, which is not where CSS says it lives.
///
/// * **`position: fixed` never contributes.** It resolves against the viewport,
///   so nothing below the viewport scrolls to reach it. Measured in Chrome: an
///   `overflow: auto` div holding a viewport-filling fixed child reports
///   `scrollWidth == clientWidth` and grows no bar. rinch counted it, so a
///   closed `Drawer` — `position: fixed`, and since #751/#761 still rendered —
///   made an `overflow: auto` ancestor paint two scrollbars (issue #765).
/// * **`position: absolute` contributes only when its containing block is
///   `parent` or below it**: walking its DOM ancestors up to `parent`, one of
///   them is positioned or transformed
///   ([`Node::establishes_abs_containing_block`]), or `parent` *is* the
///   initial containing block, which in rinch is the `<html>` box. Below
///   `parent` that walk crosses only `display: contents` wrappers, which
///   establish nothing, and — for a box hoisted into its host out of a flowed
///   inline element (#591) — the inline elements between them, one of which
///   may be a `position: relative` span: its containing block, and itself
///   `parent`'s content (issue #1049). Those are exactly
///   [`out_of_flow_kind`]'s two stopping conditions, deliberately — an absolute
///   whose containing block is further up escapes this box's scroll range in
///   CSS (measured in Chrome: a static `overflow: auto` div inside a
///   `position: relative` wrapper reports no overflow for an absolute child),
///   and the two answers must not drift apart.
/// * **Everything else contributes**, `visibility: hidden` included: it still
///   generates a box, and Chrome still counts it (700×500 of
///   `scrollWidth`/`scrollHeight`). `display: none` generates none, and its
///   zeroed layout rect already contributes nothing without a special case.
///
/// Note this is *narrower* than "is the box in flow": a `relative` box is
/// out of flow for nobody and contributes normally, and so does a float.
///
/// The caller stops at the first box, so a box skipped here is measured by **nobody**:
/// the ancestor that really is its containing block never looks past its own
/// child boxes either. Chrome does give it to that ancestor (measured: a
/// 700x1500 absolute under a static `overflow: auto` div reaches
/// `documentElement.scrollHeight`), and closing that needs the recursive union
/// css-overflow-3 describes rather than a one-level max — issue #770. It is not
/// a regression: the box used to be measured by the *wrong* container, which is
/// what grew the phantom bar #765 was filed for.
pub(crate) fn contributes_to_scrollable_overflow(
    tree: &NodeTree,
    parent_id: RawNodeId,
    child: &Node,
) -> bool {
    match child.computed_style.position {
        PositionValue::Fixed => false,
        PositionValue::Absolute => {
            // Walk the box's DOM ancestors up to and including the container,
            // exactly as `out_of_flow_kind` walks them: the first one that
            // establishes a containing block is at or below the container, so
            // the box is in its containing-block chain. Below the container
            // the chain holds only boxes the walk does not stop at — `display:
            // contents` wrappers (which establish nothing) and, for a box
            // hoisted into its host (#591), the flowed inline elements it was
            // hoisted out of, any of which may be `position: relative`
            // (issue #1049).
            let mut current = child.parent;
            while let Some(id) = current {
                if id == parent_id {
                    return id == tree.html_id
                        || id == tree.root_id
                        || tree
                            .get(id)
                            .is_some_and(Node::establishes_abs_containing_block);
                }
                let Some(ancestor) = tree.get(id) else {
                    return false;
                };
                if ancestor.establishes_abs_containing_block() {
                    return true;
                }
                current = ancestor.parent;
            }
            // The container is not a DOM ancestor of the box; nothing the
            // walk can reach puts the box in its chain.
            false
        }
        _ => true,
    }
}

/// A containing block's padding box, and where it starts inside the border
/// box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContainingBox {
    pub width: f32,
    pub height: f32,
    pub border_left: f32,
    pub border_top: f32,
}

impl ContainingBox {
    fn viewport(tree: &NodeTree) -> Self {
        Self {
            width: tree.viewport.width,
            height: tree.viewport.height,
            border_left: 0.0,
            border_top: 0.0,
        }
    }

    /// `cb`'s padding box as the **last compute** left it. An atomic inline's
    /// is the one its own detached compute produced, which is the size the
    /// line was built around. An inline span's is its fragments' box in the
    /// lines **last built** for it ([`inline_fragments_box`]), measured from
    /// the border box of the block that holds those lines.
    fn of_ancestor(tree: &NodeTree, cb: RawNodeId) -> Option<Self> {
        let node = tree.get(cb)?;
        if !generates_layout_box(node) {
            return inline_fragments_box(tree, cb);
        }
        let layout = tree.taffy.layout(node.taffy_id?).ok()?;
        Some(Self {
            width: (layout.size.width - layout.border.left - layout.border.right).max(0.0),
            height: (layout.size.height - layout.border.top - layout.border.bottom).max(0.0),
            border_left: layout.border.left,
            border_top: layout.border.top,
        })
    }

    /// The box `kind` resolves against. `None` only for an ancestor Taffy
    /// holds no layout for.
    pub(crate) fn of(tree: &NodeTree, kind: OutOfFlowKind) -> Option<Self> {
        match kind {
            OutOfFlowKind::Fixed | OutOfFlowKind::IcbAbsolute => Some(Self::viewport(tree)),
            OutOfFlowKind::AncestorAbsolute(cb) => Self::of_ancestor(tree, cb),
        }
    }
}

/// Whether `kind`'s containing block is an element with no box of its own —
/// a positioned inline span (#631). Its geometry comes from the lines of the
/// inline formatting context that flows it, which are built **after** the
/// compute, so the passes that follow those lines run only when a box of
/// this kind was read back (`NodeTree::abs_inline_cb_seen`).
pub(crate) fn has_inline_containing_block(tree: &NodeTree, kind: Option<OutOfFlowKind>) -> bool {
    matches!(kind, Some(OutOfFlowKind::AncestorAbsolute(cb))
        if tree.get(cb).is_some_and(|n| !generates_layout_box(n)))
}

/// Where the chain of boxes from an absolute box to its containing block
/// `cb` ends: at `cb` itself, or — for an inline span, which has no box —
/// at the block container whose lines hold the span ([`inline_host`]). The
/// span's fragments are that block's content, so its own scroll carries the
/// box and every box below it must not.
fn chain_end(tree: &NodeTree, cb: RawNodeId) -> Option<RawNodeId> {
    let node = tree.get(cb)?;
    if generates_layout_box(node) {
        Some(cb)
    } else {
        Some(inline_host(tree, node.ifc_root?)?.0)
    }
}

/// The block container **element** holding the lines of the inline
/// formatting context rooted at `root_id`, and where that context's content
/// box starts inside the element's border box.
///
/// The root is the element itself, or — for text beside a block-level
/// sibling — an anonymous block box inside it (#566). An out-of-flow box
/// hoisted out of the line is held by the element either way
/// (`Node::hoisted_out_of_flow_to`), so the element is the one box every
/// chain from such a box passes; an anonymous box has no padding, border or
/// scroll of its own, only a place in the element.
fn inline_host(tree: &NodeTree, root_id: RawNodeId) -> Option<(RawNodeId, (f32, f32))> {
    let root = tree.get(root_id)?;
    if root.is_anonymous_block_box {
        Some((root.parent?, (root.layout.x, root.layout.y)))
    } else {
        Some((root_id, crate::paint::ifc_root_content_origin(root)))
    }
}

/// Where a positioned inline span's fragments lie in the lines of the inline
/// formatting context that flows it, before the span's own padding: the left
/// and top of its first fragment, the right and bottom of its last, in the
/// lines' own coordinates. Measured once per set of lines for every span a
/// box hangs from (`RinchDocument::measure_inline_containing_blocks`) and
/// kept beside the lines (`InlineLayout::span_fragments`), so that looking
/// at a box costs a lookup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpanFragments {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

/// The containing block a positioned **inline** element forms (CSS 2.1
/// §10.1): from the top-left of its first fragment's padding box to the
/// bottom-right of its last fragment's, in the lines last built for the
/// inline formatting context that flows it. `border_left`/`border_top` are
/// measured from the **border box** of the block container holding those
/// lines ([`inline_host`]), which is where the chain of an absolute box
/// inside the span ends ([`chain_end`]).
///
/// `None` when the span has not been measured in the lines its context holds
/// now: the lines were rebuilt since (the pass that follows them measures it
/// again), there are none yet, or the span has no fragment in them — see
/// [`measure_span_fragments`].
///
/// rinch gives an inline element's padding no room on its line and draws no
/// inline border, so the padding box here is the fragments' box grown by the
/// padding, as the element's background is painted.
fn inline_fragments_box(tree: &NodeTree, span_id: RawNodeId) -> Option<ContainingBox> {
    let span = tree.get(span_id)?;
    let root_id = span.ifc_root?;
    let inline = tree.get(root_id)?.text_layout.as_ref()?;
    let f = (*inline.span_fragments.get(&span_id)?)?;
    let cs = &span.computed_style;
    let left = f.left - cs.padding_left.to_px();
    let top = f.top - cs.padding_top.to_px();
    let right = f.right + cs.padding_right.to_px();
    let bottom = f.bottom + cs.padding_bottom.to_px();
    let (_, (ox, oy)) = inline_host(tree, root_id)?;
    Some(ContainingBox {
        width: (right - left).max(0.0),
        height: (bottom - top).max(0.0),
        border_left: ox + left,
        border_top: oy + top,
    })
}

/// One glyph cluster of an inline layout: its bytes, its visual extent and
/// its line.
struct ClusterBox {
    start: usize,
    x0: f32,
    x1: f32,
    line: usize,
    rtl: bool,
}

/// What a span holds in its inline formatting context, gathered in one walk
/// of the context's entries.
#[derive(Default)]
struct SpanContent {
    /// Whether the span's own entry was met (it is flowed by this context).
    seen: bool,
    /// Its text, as one range of the flat text.
    text: Option<(usize, usize)>,
    /// Its atomic inlines: `(line, left, right)`.
    boxes: Vec<(usize, f32, f32)>,
    /// What stood right before the span, for a span with no content.
    before: Before,
}

/// The last content before a span's own entry.
#[derive(Default, Clone, Copy)]
enum Before {
    /// Nothing: the span opens its context.
    #[default]
    Nothing,
    /// Text ending at this byte of the flat text.
    Text(usize),
    /// An atomic inline: its line and right edge.
    Box(usize, f32),
}

/// Measure, in the lines of the inline formatting context rooted at
/// `root_id`, the fragments of every span in `spans` — in **one** walk of the
/// lines and one of the context's entries, however many spans there are.
/// `font_box` answers a style's own font's ascent and descent; `steps` counts
/// the entries, ancestors, line items and clusters looked at.
///
/// Measured in Chrome 153:
///
/// - a fragment is the **span's own font's** rounded ascent and descent
///   around the line's baseline, whatever the line box's height and whatever
///   its children are set in — a 32px span in a 16px/20px line is 39px tall
///   and starts above the line, and a 16px span holding only 32px text is
///   20px tall;
/// - a span over several lines runs from its first fragment's left edge to
///   its last fragment's right edge, and when that is left of the first the
///   width is zero (`right: 0` then hangs the box from the first fragment's
///   left edge);
/// - a fragment's left and right are the visual extent of its content on
///   the line, so a span of right-to-left text is as wide as its text;
/// - an empty span is a zero-width fragment where it sits in the line;
/// - a span holding only an atomic inline is as wide as that box and as tall
///   as its font, not as the box;
/// - the span's **own** `vertical-align` shift moves the fragment with its
///   glyphs; a shifted child at its start or end does not.
///
/// A span answers `None` when it has no entry in these lines: a **split**
/// inline (one holding a block-level child, #513, whose fragments lie in
/// several anonymous boxes), and any span in lines rebuilt as flat text to
/// draw a `text-overflow: ellipsis` "…". A box inside such a span keeps
/// Taffy's answer, the block holding its line.
fn measure_span_fragments(
    tree: &NodeTree,
    root_id: RawNodeId,
    spans: &[RawNodeId],
    font_box: &mut dyn FnMut(&ComputedStyle) -> (f32, f32),
    steps: &mut u64,
) -> HashMap<RawNodeId, Option<SpanFragments>> {
    use parley::layout::PositionedLayoutItem;

    let mut out: HashMap<RawNodeId, Option<SpanFragments>> =
        spans.iter().map(|&s| (s, None)).collect();
    let Some(root) = tree.get(root_id) else {
        return out;
    };
    let Some(inline) = root.text_layout.as_ref() else {
        return out;
    };
    let layout = &inline.layout;

    // The lines: every cluster's extent, every atomic inline's, each line's
    // baseline and start.
    let mut clusters: Vec<ClusterBox> = Vec::new();
    let mut box_at: HashMap<usize, (usize, f32, f32)> = HashMap::new();
    let mut lines: Vec<(f32, f32)> = Vec::new();
    for (index, line) in layout.lines().enumerate() {
        let metrics = line.metrics();
        lines.push((metrics.baseline, metrics.offset));
        // A run is handed out once per style it holds; its clusters are
        // walked once, from where its first piece starts.
        let mut last_run = None;
        for item in line.items() {
            *steps += 1;
            match item {
                PositionedLayoutItem::GlyphRun(piece) => {
                    let run = piece.run();
                    if last_run == Some(run.index()) {
                        continue;
                    }
                    last_run = Some(run.index());
                    let mut x = piece.offset();
                    for cluster in run.visual_clusters() {
                        *steps += 1;
                        let advance = cluster.advance();
                        clusters.push(ClusterBox {
                            start: cluster.text_range().start,
                            x0: x,
                            x1: x + advance,
                            line: index,
                            rtl: cluster.is_rtl(),
                        });
                        x += advance;
                    }
                }
                PositionedLayoutItem::InlineBox(b) => {
                    last_run = None;
                    box_at.insert(b.id as usize, (index, b.x, b.x + b.width));
                }
            }
        }
    }
    clusters.sort_by_key(|c| c.start);

    // Each text node's bytes of the flat text.
    let mut text_of: HashMap<RawNodeId, (usize, usize)> = HashMap::new();
    for range in &inline.text_ranges {
        *steps += 1;
        let e = text_of
            .entry(range.node_id)
            .or_insert((range.flat_start, range.flat_end));
        *e = (e.0.min(range.flat_start), e.1.max(range.flat_end));
    }

    // The entries, in document order: each one's content goes to every
    // wanted span above it.
    let mut content: HashMap<RawNodeId, SpanContent> = spans
        .iter()
        .filter(|&&s| tree.get(s).is_some_and(|n| !n.is_split_inline()))
        .map(|&s| (s, SpanContent::default()))
        .collect();
    let stop = if root.is_anonymous_block_box {
        root.parent
    } else {
        None
    };
    let mut before = Before::Nothing;
    for &(id, _) in &inline.child_positions {
        *steps += 1;
        let Some(node) = tree.get(id) else { continue };
        if let Some(own) = content.get_mut(&id) {
            own.seen = true;
            own.before = before;
        }
        let text = text_of.get(&id).copied();
        let atomic = (node.is_element() && node.display_mode.is_atomic_inline())
            .then(|| box_at.get(&id).copied())
            .flatten();
        match (text, atomic) {
            (Some((_, end)), _) if text.is_some_and(|(s, e)| s < e) => before = Before::Text(end),
            (_, Some((line, _, right))) => before = Before::Box(line, right),
            _ => {}
        }
        if text.is_none() && atomic.is_none() {
            continue;
        }
        let mut above = node.parent;
        while let Some(p) = above {
            if p == root_id || Some(p) == stop {
                break;
            }
            *steps += 1;
            if let Some(span) = content.get_mut(&p) {
                if let Some((s, e)) = text {
                    let (a, b) = span.text.unwrap_or((s, e));
                    span.text = Some((a.min(s), b.max(e)));
                }
                if let Some(b) = atomic {
                    span.boxes.push(b);
                }
            }
            above = tree.get(p).and_then(|n| n.parent);
        }
    }

    for (&span_id, held) in &content {
        if !held.seen {
            continue;
        }
        let Some(span) = tree.get(span_id) else {
            continue;
        };
        let (ascent, descent) = font_box(&span.computed_style);
        // The span's **own** `vertical-align` shift: the one its direct
        // text is drawn with, which is the nearest shift among the span and
        // the inline elements around it in this context (shifts do not add
        // up; `InlineLayout::vertical_align_shift_at` takes the innermost
        // one too). Not the shift of its first or last byte, which is a
        // child's when the span starts or ends with a `<sub>`.
        let shift = {
            let mut shift = 0.0;
            let mut at = Some(span_id);
            while let Some(id) = at {
                if id == root_id || Some(id) == stop {
                    break;
                }
                *steps += 1;
                let Some(el) = tree.get(id) else { break };
                if !(el.is_element() && el.display_mode == crate::node::DisplayMode::Inline) {
                    break;
                }
                if el.computed_style.vertical_align
                    != crate::computed_style::VerticalAlignValue::Baseline
                {
                    let parent_size = el
                        .parent
                        .and_then(|p| tree.get(p))
                        .map_or(el.computed_style.font_size, |p| p.computed_style.font_size);
                    shift = RinchDocument::vertical_align_shift_px(&el.computed_style, parent_size);
                    if shift != 0.0 {
                        break;
                    }
                }
                at = el.parent;
            }
            shift
        };
        // (line, x) of the first fragment's left and the last one's right.
        let mut first: Option<(usize, f32)> = None;
        let mut last: Option<(usize, f32)> = None;
        let mut join_first = |line: usize, x: f32| {
            first = Some(match first {
                Some((l, v)) if l < line => (l, v),
                Some((l, v)) if l == line => (l, v.min(x)),
                _ => (line, x),
            });
        };
        let mut join_last = |line: usize, x: f32| {
            last = Some(match last {
                Some((l, v)) if l > line => (l, v),
                Some((l, v)) if l == line => (l, v.max(x)),
                _ => (line, x),
            });
        };
        if let Some((a, b)) = held.text.filter(|(a, b)| a < b) {
            let from = clusters.partition_point(|c| c.start < a);
            let to = clusters.partition_point(|c| c.start < b);
            if from < to {
                let line = clusters[from].line;
                for c in clusters[from..to].iter().take_while(|c| c.line == line) {
                    *steps += 1;
                    join_first(line, c.x0);
                }
                let line = clusters[to - 1].line;
                for c in clusters[from..to]
                    .iter()
                    .rev()
                    .take_while(|c| c.line == line)
                {
                    *steps += 1;
                    join_last(line, c.x1);
                }
            }
        }
        for &(line, left, right) in &held.boxes {
            *steps += 1;
            join_first(line, left);
            join_last(line, right);
        }
        let (first, last) = match (first, last) {
            (Some(f), Some(l)) => (f, l),
            // Nothing of the span is on any line: an empty span. It sits
            // where the content before it ends — at the start of what
            // follows, when anything does.
            _ => {
                let at = match held.before {
                    Before::Box(line, right) => Some((line, right)),
                    Before::Nothing | Before::Text(_) => {
                        let byte = match held.before {
                            Before::Text(end) => end,
                            _ => 0,
                        };
                        let next = clusters.partition_point(|c| c.start < byte);
                        match (clusters.get(next), clusters.last()) {
                            (Some(c), _) => Some((c.line, if c.rtl { c.x1 } else { c.x0 })),
                            (None, Some(c)) => Some((c.line, if c.rtl { c.x0 } else { c.x1 })),
                            (None, None) => lines.first().map(|&(_, offset)| (0, offset)),
                        }
                    }
                };
                match at {
                    Some(at) => (at, at),
                    None => continue,
                }
            }
        };
        let (Some(&(base_first, _)), Some(&(base_last, _))) =
            (lines.get(first.0), lines.get(last.0))
        else {
            continue;
        };
        out.insert(
            span_id,
            Some(SpanFragments {
                left: first.1,
                top: base_first + shift - ascent.round(),
                right: last.1,
                bottom: base_last + shift + descent.round(),
            }),
        );
    }
    out
}

/// The ascent and descent of `style`'s **primary font** at its size: the
/// face its stack resolves a Latin `x` to (as a text control's metrics are
/// read, `form_control::char_metrics`). `(0, 0)` when no face answers.
fn font_box(
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<peniko::Brush>,
    style: &ComputedStyle,
) -> (f32, f32) {
    const PROBE: &str = "x";
    let family = crate::fonts::parley_text_family(font_cx, &style.font_family, PROBE);
    let mut builder = layout_cx.ranged_builder(font_cx, PROBE, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(style.font_size));
    family.push_to(&mut builder);
    if (style.font_weight - 400.0).abs() > 1.0 {
        builder.push_default(parley::style::StyleProperty::FontWeight(
            parley::style::FontWeight::new(style.font_weight),
        ));
    }
    if style.font_style != crate::computed_style::FontStyleValue::Normal {
        builder.push_default(parley::style::StyleProperty::FontStyle(
            style.font_style.to_parley(),
        ));
    }
    let mut layout = builder.build(PROBE);
    layout.break_all_lines(None);
    layout
        .lines()
        .next()
        .and_then(|line| {
            line.items().find_map(|item| match item {
                parley::layout::PositionedLayoutItem::GlyphRun(run) => {
                    let m = run.run().metrics();
                    Some((m.ascent, m.descent))
                }
                _ => None,
            })
        })
        .unwrap_or((0.0, 0.0))
}

/// Bake `node_id`'s out-of-flow size into a Taffy style a site has just
/// rebuilt from `computed_style`, and keep [`NodeTree::ancestor_absolutes`]
/// and the node's `abs_ancestor_baked` flag telling the truth about it.
///
/// Call this from **every** site that rebuilds a Taffy style out of
/// `ComputedStyle` — `apply_stylo_styles_to_taffy`, `tick_transitions`,
/// `tick_animations` — or a restyle drops the override until the next layout
/// (an ancestor case, which [`RinchDocument::resolve_ancestor_absolutes`] puts
/// back at the price of a compute) or the next full cascade (the viewport
/// cases, which nothing else puts back).
///
/// An ancestor's size is the one its **last** layout gave it; one that has
/// never been laid out — a 0x0 box — bakes nothing here, and the pass after
/// the compute does it.
pub(crate) fn bake_at_style_site(
    tree: &mut NodeTree,
    node_id: RawNodeId,
    taffy_style: &mut taffy::Style,
) {
    let Some(node) = tree.nodes.get(node_id) else {
        return;
    };
    let mut ancestor = false;
    let mut baked_against_ancestor = false;
    if let Some(kind) = out_of_flow_kind(tree, node_id) {
        ancestor = matches!(kind, OutOfFlowKind::AncestorAbsolute(_));
        let known = ContainingBox::of(tree, kind)
            .filter(|cb| !ancestor || cb.width > 0.0 || cb.height > 0.0);
        if let Some(cb) = known {
            apply_out_of_flow_size_overrides(node, kind, (cb.width, cb.height), taffy_style);
            baked_against_ancestor = ancestor;
        }
    } else if node.box_position() == PositionValue::Absolute {
        fit_in_layout_parent(tree, node, taffy_style);
    }
    // The style handed in was built from the computed values, so a box that
    // is not ancestor-resolved now carries no ancestor's size. The set is
    // touched only when a box becomes ancestor-resolved; one that stops
    // being is dropped from it by the pass that iterates it. Every other
    // node — nearly all of them — pays a flag read.
    if ancestor && !node.abs_ancestor_recorded {
        tree.ancestor_absolutes.insert(node_id);
    }
    let node = &mut tree.nodes[node_id];
    node.abs_ancestor_recorded = ancestor;
    node.abs_ancestor_baked = baked_against_ancestor;
}

/// The #280 inset fast path has just written new insets to `node_id`'s
/// computed style and is about to write `taffy_style` — the node's current
/// Taffy style with the new inset — without going through a style site. For a
/// box resolved against a non-parent ancestor, a size baked from the **old**
/// insets is in it (`left: 0; right: 0` gives the width), so bake it again
/// from the new ones; the pass after the compute then finds nothing to
/// rewrite and the write costs one compute, as it does for any other box.
pub(crate) fn rebake_after_inset_write(
    tree: &NodeTree,
    node_id: RawNodeId,
    taffy_style: &mut taffy::Style,
) {
    let Some(node) = tree.nodes.get(node_id) else {
        return;
    };
    let kind = match out_of_flow_kind(tree, node_id) {
        Some(kind @ OutOfFlowKind::AncestorAbsolute(_)) => kind,
        Some(_) => return,
        // Taffy's own containing block: whether the width is shrunk to fit
        // by keyword depends on the insets ([`fit_in_layout_parent`]).
        None => {
            if node.box_position() == PositionValue::Absolute
                && matches!(node.computed_style.width, DimensionValue::Auto)
            {
                taffy_style.size.width = taffy::Dimension::auto();
                fit_in_layout_parent(tree, node, taffy_style);
            }
            return;
        }
    };
    let OutOfFlowKind::AncestorAbsolute(cb) = kind else {
        return;
    };
    // Only a style that carries a bake: one that carries none has a
    // containing block not yet laid out, which the pass after the compute
    // bakes for.
    if !node.abs_ancestor_baked {
        return;
    }
    let Some(cb) = ContainingBox::of_ancestor(tree, cb) else {
        return;
    };
    unbake(&node.computed_style, false, taffy_style);
    apply_out_of_flow_size_overrides(node, kind, (cb.width, cb.height), taffy_style);
}

/// `read_layout_results` has classified the absolute box `node_id` as `kind`.
/// Check that against what the style sites recorded: a box that is
/// ancestor-resolved and not in [`NodeTree::ancestor_absolutes`] became so
/// through a change to a box *between* it and its containing block that
/// restyled neither (the parent stopped being `display: contents`). Put it in
/// the set and ask `resolve_layout` to resolve again before it returns
/// (`NodeTree::abs_resolve_owed`).
///
/// The other direction needs nothing here: a box that stopped being
/// ancestor-resolved the same way is still in the set (every baked box is),
/// so the pass after the compute has already put its style back.
pub(crate) fn note_kind_at_read(
    tree: &mut NodeTree,
    node_id: RawNodeId,
    kind: Option<OutOfFlowKind>,
) {
    if matches!(kind, Some(OutOfFlowKind::AncestorAbsolute(_)))
        && !tree.nodes[node_id].abs_ancestor_recorded
    {
        tree.nodes[node_id].abs_ancestor_recorded = true;
        tree.ancestor_absolutes.insert(node_id);
        tree.abs_resolve_owed = true;
    }
}

/// Forget what the last read-back recorded: the placed boxes and the chain
/// flags. Called before `read_layout_results` walks the document.
pub(crate) fn begin_read(tree: &mut NodeTree) {
    tree.placed_absolutes.clear();
    tree.abs_static_fits.clear();
    tree.abs_late_moves = false;
    tree.abs_inline_cb_seen = false;
    let mut marked = std::mem::take(&mut tree.abs_chain_marked);
    for id in marked.drain(..) {
        if let Some(node) = tree.nodes.get_mut(id) {
            node.on_abs_chain = false;
        }
    }
    tree.abs_chain_marked = marked;
}

/// Bake an out-of-flow box's Taffy **size** from its real containing block,
/// `cb` — the width and height of the viewport or of an ancestor's padding
/// box.
///
/// Taffy would size the box against its direct parent, and the size has to be
/// right *before* `compute_layout` runs or the box's own subtree lays out
/// inside the wrong box — a percentage child, a `height: 100%` child, or text
/// wrapping would all be measured against the parent instead of the containing
/// block. That is why this is a pre-layout bake and not another post-layout
/// patch.
///
/// For the two absolute cases it also resolves what else Taffy would take
/// from the direct parent: the margins that come out of the space between two
/// insets, and a percentage `min-`/`max-` size, padding or margin (a
/// percentage padding or margin is of the containing block's **width** on
/// every side). A `Calc` in one of those is `calc_layout`'s, which resolves it
/// against the same box.
///
/// The result is a function of the computed style and `cb` alone **given a
/// `size` that is the computed style's own** (`ComputedStyle::to_taffy_style`'s
/// image of `width`/`height`): the branches below read which of `auto`,
/// `stretch` or a length the style carries.
pub(crate) fn apply_out_of_flow_size_overrides(
    node: &Node,
    kind: OutOfFlowKind,
    cb: (f32, f32),
    taffy_style: &mut taffy::Style,
) {
    let (cw, ch) = cb;
    let cs = &node.computed_style;
    let absolute = kind.is_absolute();
    // What a margin takes out of the space between two insets. `auto` takes
    // none of it here: with an auto size the box fills the space and an auto
    // margin is zero (CSS 2.1 §10.3.7).
    let margin = |m: LengthPercentageAutoValue| {
        if absolute {
            m.resolve(cw).unwrap_or(0.0)
        } else {
            0.0
        }
    };
    let (ml, mr) = (margin(cs.margin_left), margin(cs.margin_right));
    let (mt, mb) = (margin(cs.margin_top), margin(cs.margin_bottom));

    // `stretch` (#691) fills the containing block less the insets, which
    // Taffy would take from the direct parent. Unpaired, a missing inset
    // counts as zero — Taffy's `resolve_absolute_sizing_keywords` rule, and
    // Chrome 153's answer (`left: 10px` in an 800px viewport gives 790).
    if taffy_style.size.width.is_stretch() {
        let l = cs.left.resolve(cw).unwrap_or(0.0);
        let r = cs.right.resolve(cw).unwrap_or(0.0);
        taffy_style.size.width = taffy::Dimension::length((cw - l - r - ml - mr).max(0.0));
    } else if taffy_style.size.width == taffy::Dimension::auto()
        || (absolute && taffy_style.size.width == taffy::Dimension::fit_content())
    {
        let auto = taffy_style.size.width == taffy::Dimension::auto();
        match (cs.left.resolve(cw), cs.right.resolve(cw)) {
            (Some(l), Some(r)) if auto => {
                taffy_style.size.width = taffy::Dimension::length((cw - l - r - ml - mr).max(0.0));
            }
            // A fixed box with unpaired insets fills the viewport, as it has
            // since the fixed path was written.
            _ if !absolute => {
                taffy_style.size.width = taffy::Dimension::length(cw);
            }
            // An absolute one shrinks to fit (CSS 2.1 §10.3.7) — in what its
            // **containing block** leaves it: that block's width less the
            // insets and margins, and with both insets `auto`, less the
            // distance from the block's edge to the static position, which
            // is where the box starts (#1404). Taffy would measure it in the
            // whole width of the box it lays it out in; `fit-content(<px>)`
            // is its spelling of "measure at this available width".
            (l, r) => {
                let offset = if l.is_none() && r.is_none() {
                    node.abs_static_offset
                } else {
                    0.0
                };
                let available = cw - l.unwrap_or(0.0) - r.unwrap_or(0.0) - ml - mr - offset;
                taffy_style.size.width = taffy::Dimension::fit_content_px(available.max(0.0));
            }
        }
    } else if absolute {
        // Taffy would resolve these against the direct parent. A mixed
        // calc() takes the same arm as a plain percentage (#278/#496 review).
        match cs.width {
            DimensionValue::Percent(p) => {
                taffy_style.size.width = taffy::Dimension::length((cw * p).max(0.0));
            }
            DimensionValue::Calc { px, pct } => {
                taffy_style.size.width = taffy::Dimension::length((cw * pct + px).max(0.0));
            }
            _ => {}
        }
    }

    if taffy_style.size.height.is_stretch() {
        let t = cs.top.resolve(ch).unwrap_or(0.0);
        let b = cs.bottom.resolve(ch).unwrap_or(0.0);
        taffy_style.size.height = taffy::Dimension::length((ch - t - b - mt - mb).max(0.0));
    } else if taffy_style.size.height == taffy::Dimension::auto() {
        match (cs.top.resolve(ch), cs.bottom.resolve(ch)) {
            (Some(t), Some(b)) => {
                taffy_style.size.height = taffy::Dimension::length((ch - t - b - mt - mb).max(0.0));
            }
            _ if !absolute => {
                taffy_style.size.height = taffy::Dimension::length(ch);
            }
            _ => {}
        }
    } else if absolute {
        match cs.height {
            DimensionValue::Percent(p) => {
                taffy_style.size.height = taffy::Dimension::length((ch * p).max(0.0));
            }
            DimensionValue::Calc { px, pct } => {
                taffy_style.size.height = taffy::Dimension::length((ch * pct + px).max(0.0));
            }
            _ => {}
        }
    }

    if !absolute {
        return;
    }
    let limit = |slot: &mut taffy::LengthPercentageAuto, v: DimensionValue, basis: f32| {
        if let DimensionValue::Percent(p) = v {
            *slot = taffy::LengthPercentageAuto::length((basis * p).max(0.0));
        }
    };
    limit(&mut taffy_style.min_size.width, cs.min_width, cw);
    limit(&mut taffy_style.max_size.width, cs.max_width, cw);
    limit(&mut taffy_style.min_size.height, cs.min_height, ch);
    limit(&mut taffy_style.max_size.height, cs.max_height, ch);
    let pad = |slot: &mut taffy::LengthPercentage, v: LengthPercentageValue| {
        if let LengthPercentageValue::Percent(p) = v {
            *slot = taffy::LengthPercentage::length((cw * p).max(0.0));
        }
    };
    pad(&mut taffy_style.padding.left, cs.padding_left);
    pad(&mut taffy_style.padding.right, cs.padding_right);
    pad(&mut taffy_style.padding.top, cs.padding_top);
    pad(&mut taffy_style.padding.bottom, cs.padding_bottom);
    let edge = |slot: &mut taffy::LengthPercentageAuto, v: LengthPercentageAutoValue| {
        if let LengthPercentageAutoValue::Percent(p) = v {
            *slot = taffy::LengthPercentageAuto::length(cw * p);
        }
    };
    edge(&mut taffy_style.margin.left, cs.margin_left);
    edge(&mut taffy_style.margin.right, cs.margin_right);
    edge(&mut taffy_style.margin.top, cs.margin_top);
    edge(&mut taffy_style.margin.bottom, cs.margin_bottom);
}

/// Whether an inset or margin of `v` takes room from a containing block.
fn takes_room(v: LengthPercentageAutoValue) -> bool {
    use LengthPercentageAutoValue as V;
    match v {
        V::Auto => false,
        V::Length(px) => px != 0.0,
        V::Percent(p) => p != 0.0,
        V::Calc { .. } => true,
    }
}

/// The box Taffy lays `node` out in, if it is a **grid** container: Taffy's
/// grid measures an absolute item's sizing keyword in the item's whole grid
/// area, insets or not, so a box there needs a length.
fn layout_parent_box(tree: &NodeTree, node: &Node) -> Option<RawNodeId> {
    let mut current = node.parent;
    while let Some(id) = current {
        let ancestor = tree.get(id)?;
        if generates_layout_box(ancestor) {
            return Some(id);
        }
        current = ancestor.parent;
    }
    None
}

fn is_grid(tree: &NodeTree, id: RawNodeId) -> bool {
    tree.get(id).is_some_and(|n| {
        matches!(
            n.computed_style.display,
            DisplayValue::Grid | DisplayValue::InlineGrid
        )
    })
}

/// Shrink an auto-width absolute box **whose layout parent is its containing
/// block** to fit in what that block leaves it (CSS 2.1 §10.3.7, #1404): its
/// width less the insets and margins, and with both insets `auto` less the
/// distance from the block's padding edge to the static position, where the
/// box starts (`Node::abs_static_offset`).
///
/// Where it can, by handing Taffy the `fit-content` keyword: on an absolute
/// child of a block or flex container Taffy measures that at the containing
/// block's width less the insets and margins, where it measures `auto` at the
/// whole width. Only when that changes the answer — an inset or a margin that
/// takes room, and not both insets (the box then fills the space between
/// them); a box with neither keeps `auto`, which Taffy measures once.
///
/// The keyword knows nothing of a static position, and in a **grid**
/// container Taffy measures it in the item's whole grid area, insets or not.
/// Those take a length, `fit-content(<px>)`, from the containing block's
/// **last** laid-out padding box — as an ancestor-resolved box is baked
/// (`bake_at_style_site`). One with no layout yet is left to the keyword, and
/// `RinchDocument::resolve_static_shrink_to_fit` bakes it after the compute.
fn fit_in_layout_parent(tree: &NodeTree, node: &Node, taffy_style: &mut taffy::Style) {
    use LengthPercentageAutoValue as V;
    let width = taffy_style.size.width;
    if width != taffy::Dimension::auto() && width != taffy::Dimension::fit_content() {
        return;
    }
    let cs = &node.computed_style;
    if !matches!(cs.left, V::Auto) && !matches!(cs.right, V::Auto) {
        return;
    }
    let room = takes_room(cs.left)
        || takes_room(cs.right)
        || takes_room(cs.margin_left)
        || takes_room(cs.margin_right);
    let offset = if static_axes(cs).0 {
        node.abs_static_offset
    } else {
        0.0
    };
    let parent = layout_parent_box(tree, node);
    let grid = parent.is_some_and(|p| is_grid(tree, p));
    if !grid && offset == 0.0 {
        if room {
            taffy_style.size.width = taffy::Dimension::fit_content();
        }
        return;
    }
    if grid && !room && offset == 0.0 {
        return;
    }
    let known = parent
        .and_then(|p| ContainingBox::of_ancestor(tree, p))
        .filter(|cb| cb.width > 0.0 || cb.height > 0.0);
    let Some(cb) = known else {
        if !grid && room {
            taffy_style.size.width = taffy::Dimension::fit_content();
        }
        return;
    };
    let cw = cb.width;
    let take = |v: V| v.resolve(cw).unwrap_or(0.0);
    let available =
        cw - take(cs.left) - take(cs.right) - take(cs.margin_left) - take(cs.margin_right) - offset;
    taffy_style.size.width = taffy::Dimension::fit_content_px(available.max(0.0));
}

/// Whether the absolute box `node_id`, whose layout parent **is** its
/// containing block, is one [`fit_in_layout_parent`] may bake a length for —
/// and so one `RinchDocument::resolve_static_shrink_to_fit` looks at after
/// the lines: shrunk to fit from its static position, or in a grid container
/// with an inset or margin that takes room. Any other box costs a few reads.
pub(crate) fn fits_in_layout_parent_later(tree: &NodeTree, node_id: RawNodeId) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let cs = &node.computed_style;
    if node.box_position() != PositionValue::Absolute || !shrinks_to_fit(cs) {
        return false;
    }
    let (static_x, _) = static_axes(cs);
    if static_x {
        return true;
    }
    if !(matches!(cs.left, LengthPercentageAutoValue::Auto)
        || matches!(cs.right, LengthPercentageAutoValue::Auto))
    {
        return false;
    }
    (takes_room(cs.left)
        || takes_room(cs.right)
        || takes_room(cs.margin_left)
        || takes_room(cs.margin_right))
        && layout_parent_box(tree, node).is_some_and(|p| is_grid(tree, p))
}

fn shrinks_to_fit(cs: &ComputedStyle) -> bool {
    use crate::computed_style::IntrinsicSize;
    matches!(
        cs.width,
        DimensionValue::Auto | DimensionValue::Intrinsic(IntrinsicSize::FitContent)
    )
}

/// The inputs to hand `taffy::compute_leaf_layout` for a leaf with `style`.
///
/// Taffy's leaf algorithm takes the leaf's **own margins** off the available
/// width it was given; its block, flex and grid algorithms do not take a
/// container's own. An absolutely positioned box is measured at the room its
/// containing block leaves it, margins already out ([`fit_in_layout_parent`],
/// the bake in [`apply_out_of_flow_size_overrides`], Taffy's grid), so a
/// leaf — a box holding only inline content — had them taken off twice:
/// `right: 20px; margin-right: 20px` in 394px came out 334 wide, and the
/// same box around a block child 354 (Chrome: 354). The margins are put
/// back here, so one rule holds for both.
pub(crate) fn absolute_leaf_inputs(
    mut inputs: taffy::LayoutInput,
    style: &taffy::Style,
) -> taffy::LayoutInput {
    if style.position == taffy::Position::Absolute
        && inputs.known_dimensions.width.is_none()
        && let taffy::AvailableSpace::Definite(width) = inputs.available_space.width
    {
        use taffy::{CoreStyle, ResolveOrZero};
        let margin = style
            .margin()
            .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
        let margins = margin.left + margin.right;
        if margins != 0.0 {
            inputs.available_space.width = taffy::AvailableSpace::Definite(width + margins);
        }
    }
    inputs
}

/// Whether an absolute box with this style is **shrunk to fit from its
/// static position**: an `auto` or `fit-content` width and both inline insets
/// `auto`, so the room it has is what its containing block leaves after the
/// place the box starts at (#1404).
pub(crate) fn fits_from_static_position(cs: &ComputedStyle) -> bool {
    static_axes(cs).0 && shrinks_to_fit(cs)
}

/// Copy the fields [`apply_out_of_flow_size_overrides`] and [`unbake`] read
/// and write — and no other — from `from` into `to`.
fn copy_baked_fields(to: &mut taffy::Style, from: &taffy::Style) {
    to.size = from.size;
    to.min_size = from.min_size;
    to.max_size = from.max_size;
    to.padding = from.padding;
    to.margin = from.margin;
}

/// Whether `a` and `b` agree on every field [`copy_baked_fields`] copies.
fn same_baked_fields(a: &taffy::Style, b: &taffy::Style) -> bool {
    a.size == b.size
        && a.min_size == b.min_size
        && a.max_size == b.max_size
        && a.padding == b.padding
        && a.margin == b.margin
}

/// Put back what [`apply_out_of_flow_size_overrides`] may have written, as
/// `ComputedStyle::to_taffy_style` would have produced it: the `size`, and —
/// with `all` — each percentage it turned into a length. A `Calc` field is
/// left alone; `calc_layout` owns those.
fn unbake(cs: &ComputedStyle, all: bool, taffy_style: &mut taffy::Style) {
    taffy_style.size = taffy::Size {
        width: cs.width.to_taffy(),
        height: cs.height.to_taffy(),
    };
    if !all {
        return;
    }
    let limit = |slot: &mut taffy::LengthPercentageAuto, v: DimensionValue| {
        if matches!(v, DimensionValue::Percent(_)) {
            *slot = v.to_taffy_lpa();
        }
    };
    limit(&mut taffy_style.min_size.width, cs.min_width);
    limit(&mut taffy_style.max_size.width, cs.max_width);
    limit(&mut taffy_style.min_size.height, cs.min_height);
    limit(&mut taffy_style.max_size.height, cs.max_height);
    let pad = |slot: &mut taffy::LengthPercentage, v: LengthPercentageValue| {
        if matches!(v, LengthPercentageValue::Percent(_)) {
            *slot = v.to_taffy();
        }
    };
    pad(&mut taffy_style.padding.left, cs.padding_left);
    pad(&mut taffy_style.padding.right, cs.padding_right);
    pad(&mut taffy_style.padding.top, cs.padding_top);
    pad(&mut taffy_style.padding.bottom, cs.padding_bottom);
    let edge = |slot: &mut taffy::LengthPercentageAuto, v: LengthPercentageAutoValue| {
        if matches!(v, LengthPercentageAutoValue::Percent(_)) {
            *slot = v.to_taffy();
        }
    };
    edge(&mut taffy_style.margin.left, cs.margin_left);
    edge(&mut taffy_style.margin.right, cs.margin_right);
    edge(&mut taffy_style.margin.top, cs.margin_top);
    edge(&mut taffy_style.margin.bottom, cs.margin_bottom);
}

/// Where one axis of an absolutely positioned box starts, measured from its
/// containing block's padding edge — or `None` when both insets are `auto`
/// and the box keeps its static position (CSS 2.1 §10.3.7, §10.6.4).
///
/// Between two insets, an `auto` margin takes the space the box leaves:
/// both `auto` centre it, one `auto` takes all of it. Chrome 153: a 100x50
/// box with `inset: 0; margin: auto` in 400x300 sits at `(150, 125)`. When
/// the box is **larger** than the space, the block axis still centres (a
/// 350px-tall box sits at `-25`) and the inline axis starts at the inset
/// (`centre_overflow` is `false` there) — also measured.
fn axis_start(
    insets: (Option<f32>, Option<f32>),
    margins: (Option<f32>, Option<f32>),
    cb_len: f32,
    size: f32,
    centre_overflow: bool,
) -> Option<f32> {
    match insets {
        (Some(start), Some(end)) => {
            let free = cb_len - start - end - size;
            Some(match margins {
                (None, None) if free >= 0.0 || centre_overflow => start + free / 2.0,
                (None, None) => start,
                (None, Some(m_end)) => start + free - m_end,
                (Some(m_start), _) => start + m_start,
            })
        }
        (Some(start), None) => Some(start + margins.0.unwrap_or(0.0)),
        (None, Some(end)) => Some(cb_len - end - margins.1.unwrap_or(0.0) - size),
        (None, None) => None,
    }
}

/// The boxes strictly between `node_id` and its containing block, summed: the
/// offset of the box's layout parent from the containing block's border-box
/// origin, and the scroll offsets of every box on the way. `None` when the
/// box tree does not lead from the box to `cb` at all.
///
/// `cb` is `None` for the initial containing block. Its chain ends at the
/// `<body>`, where every coordinate walk in the codebase ends, and the body's
/// own scroll is left out of the sum: it is the page's scroll, which carries
/// an ICB box as an ancestor's own scroll carries its boxes.
///
/// With `mark`, every box whose scroll is in the sum is flagged
/// (`Node::on_abs_chain`): a scroll of one of those, and of no other box,
/// moves this one.
fn chain_to_containing_block(
    tree: &mut NodeTree,
    node_id: RawNodeId,
    cb: Option<RawNodeId>,
    mark: bool,
) -> Option<((f32, f32), (f32, f32))> {
    let (mut ox, mut oy) = (0.0_f32, 0.0_f32);
    let (mut sx, mut sy) = (0.0_f64, 0.0_f64);
    let mut current = RinchDocument::box_tree_parent(&tree.nodes, node_id);
    loop {
        if current == cb {
            break;
        }
        let Some(id) = current else {
            // Ran out of ancestors: the ICB walk of a detached subtree ends
            // here, as `compute_absolute_position`'s does. An ancestor that
            // was never reached is not on this box's chain.
            if cb.is_some() {
                return None;
            }
            break;
        };
        let node = tree.get(id)?;
        // Mirrors `paint`'s origin step: an atomic inline's `layout` is
        // relative to its IFC root's content box.
        let (dx, dy) = crate::paint::ifc_content_box_offset(tree, node);
        ox += node.layout.x + dx;
        oy += node.layout.y + dy;
        if cb.is_none() && id == tree.body_id {
            break;
        }
        sx += node.scroll_offset.0;
        sy += node.scroll_offset.1;
        if mark && !node.on_abs_chain {
            tree.nodes[id].on_abs_chain = true;
            tree.abs_chain_marked.push(id);
        }
        current = RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    Some(((ox, oy), (sx as f32, sy as f32)))
}

/// Where an absolutely positioned box belongs, as the **parent-relative**
/// `(x, y)` `LayoutResult` holds — see the module doc for the sum.
///
/// `taffy_location` is its static position on an axis with no inset — where
/// Taffy put it, or its place in a line ([`static_location`]) — and `size`
/// its used border-box size. `None` for
/// `position: fixed`, for an ancestor Taffy holds no layout for, and when the
/// box tree does not lead to the ancestor: the box then keeps Taffy's answer.
///
/// `mark` flags the boxes between it and its containing block for
/// [`replace_after_scroll`]; the read-back passes `true`.
pub(crate) fn place_absolute(
    tree: &mut NodeTree,
    node_id: RawNodeId,
    kind: OutOfFlowKind,
    taffy_location: (f32, f32),
    size: (f32, f32),
    mark: bool,
) -> Option<(f32, f32)> {
    // Where the chain ends, and whether the containing block is an inline
    // span — whose chain ends at another node, the block holding its line.
    let (end, on_text) = match kind {
        OutOfFlowKind::Fixed => return None,
        OutOfFlowKind::IcbAbsolute => (None, false),
        OutOfFlowKind::AncestorAbsolute(cb) => {
            let end = chain_end(tree, cb)?;
            (Some(end), end != cb)
        }
    };
    // The chain first: it is flagged for a scroll even in a layout where the
    // containing block cannot be measured yet (an inline span's lines are
    // built after the read-back; the re-placement that follows them does not
    // flag).
    let ((ox, oy), (sx, sy)) = chain_to_containing_block(tree, node_id, end, mark)?;
    let cb = ContainingBox::of(tree, kind)?;
    let style = &tree.get(node_id)?.computed_style;
    // Percentage margins resolve against the containing block's *width* on
    // both axes, per CSS.
    let x = axis_start(
        (style.left.resolve(cb.width), style.right.resolve(cb.width)),
        (
            style.margin_left.resolve(cb.width),
            style.margin_right.resolve(cb.width),
        ),
        cb.width,
        size.0,
        false,
    );
    let y = axis_start(
        (
            style.top.resolve(cb.height),
            style.bottom.resolve(cb.height),
        ),
        (
            style.margin_top.resolve(cb.width),
            style.margin_bottom.resolve(cb.width),
        ),
        cb.height,
        size.1,
        true,
    );
    // A span's fragment starts wherever its text does, between pixels; a
    // box hung from it goes on the pixel grid, where every box Taffy places
    // already is (and where a browser paints it: Chrome 153 lays the box out
    // at 47.64 and draws its edge at 48). Snapped in the frame of the block
    // holding the line — a box Taffy placed — not in the layout parent's,
    // which may itself be an atomic inline standing between pixels.
    let snap = |v: f32| if on_text { snap_to_pixel(v) } else { v };
    // With both insets `auto` the box keeps Taffy's static position — which
    // CSS *does* take from the flow position in the parent.
    Some((
        x.map_or(taffy_location.0, |x| snap(cb.border_left + x) - ox) + sx,
        y.map_or(taffy_location.1, |y| snap(cb.border_top + y) - oy) + sy,
    ))
}

/// `v` on the pixel grid. Out of line on purpose: `f32::round` is a library
/// call on the baseline x86-64 target, and inlined into [`place_absolute`]
/// the compiler computed it for every box and selected afterwards — about
/// 80 instructions per placed box that is hung from no span at all
/// (`rinch-bench`'s `abs_badge_relayout.icb_500`: 2,236,846 instructions
/// inlined, 2,195,839 like this).
#[inline(never)]
fn snap_to_pixel(v: f32) -> f32 {
    v.round()
}

/// Whether a box of this `size` is one layout produced, rather than the 0x0
/// a box inside a `display: none` subtree carries (CSS 2.1 §9.2.4) — which
/// must not be dragged onto its containing block. A box with any size is; a
/// real 0x0 box — an anchor with overflowing content — is told from a hidden
/// one by its ancestors, a walk only such a box pays.
pub(crate) fn is_laid_out(tree: &NodeTree, node_id: RawNodeId, size: (f32, f32)) -> bool {
    if size.0 > 0.0 || size.1 > 0.0 {
        return true;
    }
    let mut current = tree.get(node_id).and_then(|n| n.parent);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else {
            return false;
        };
        if node.computed_style.display == DisplayValue::None {
            return false;
        }
        if id == tree.root_id {
            return true;
        }
        current = node.parent;
    }
    // Not connected to the document.
    false
}

/// Whether an axis of `style` has both insets `auto`, so that the box keeps
/// its static position there: `(horizontal, vertical)`.
pub(crate) fn static_axes(style: &ComputedStyle) -> (bool, bool) {
    let auto = |v: LengthPercentageAutoValue| v.resolve(0.0).is_none();
    (
        auto(style.left) && auto(style.right),
        auto(style.top) && auto(style.bottom),
    )
}

/// The nearest ancestor of `node_id` in the box tree that generates a box:
/// the one its `layout` is relative to. (A `display: contents` wrapper is in
/// the chain and has none.)
fn layout_parent(tree: &NodeTree, node_id: RawNodeId) -> Option<RawNodeId> {
    let mut current = RinchDocument::box_tree_parent(&tree.nodes, node_id)?;
    loop {
        let node = tree.get(current)?;
        if node.computed_style.display != DisplayValue::Contents {
            return Some(current);
        }
        current = RinchDocument::box_tree_parent(&tree.nodes, current)?;
    }
}

/// The static position of the out-of-flow box `node_id` **in a line**, as an
/// offset from its layout parent's border box, margins not added — or `None`
/// when no inline formatting context holds one for it and Taffy's answer
/// (after its preceding in-flow sibling) is the static position.
///
/// The lines say where the box would have been (`InlineLayout::out_of_flow`,
/// `ifc::resolve_out_of_flow_marks`); the box's own style says which of the
/// two answers is its: in the line for a box that was inline-level before
/// `position` blockified it (#634), below it for a block-level one (#632).
///
/// `Node::static_ifc_root` is only a hint: the root must still hold lines,
/// with the box among their content. (A box moved elsewhere is not left in
/// them: every verb that moves a node drops the lines of the context it
/// left.) The lines' host is the box's layout parent — the block container
/// an out-of-flow box among inline content is laid out in ([`inline_host`]).
fn inline_static_position(tree: &NodeTree, node_id: RawNodeId) -> Option<(f32, f32)> {
    let node = tree.get(node_id)?;
    let (root_id, index) = node.static_ifc_root?;
    let inline = tree.get(root_id)?.text_layout.as_ref()?;
    // By index, not by search: a paragraph of N such boxes is looked at N
    // times per layout (`abs_static_lookup_steps`).
    tree.perf.bump(crate::perf::Counter::AbsStaticLookupSteps);
    let mark = inline.out_of_flow.get(index).filter(|m| m.id == node_id)?;
    let (_, (ox, oy)) = inline_host(tree, root_id)?;
    let (x, y) = if node.computed_style.inline_level_before_blockify {
        mark.inline
    } else {
        (0.0, mark.block_y)
    };
    // On the pixel grid, where Taffy puts every box (and Chrome paints this
    // one: laid out at 39.33, its edge drawn at 39).
    Some((snap_to_pixel(ox + x), snap_to_pixel(oy + y)))
}

/// Where the out-of-flow box `node_id` would have been in the flow, relative
/// to its layout parent's border box and with its own margins added — what
/// an axis with both insets `auto` keeps (CSS 2.1 §10.3.7, §10.6.4).
/// `taffy_location` is Taffy's answer, which stands unless the box sits among
/// inline content ([`inline_static_position`]).
///
/// One flag read for a box no line holds a place for, which is every box
/// outside inline content.
#[inline]
pub(crate) fn static_location(
    tree: &NodeTree,
    node_id: RawNodeId,
    kind: Option<OutOfFlowKind>,
    taffy_location: (f32, f32),
) -> (f32, f32) {
    if tree
        .nodes
        .get(node_id)
        .is_none_or(|n| n.static_ifc_root.is_none())
    {
        return taffy_location;
    }
    static_location_among_lines(tree, node_id, kind).unwrap_or(taffy_location)
}

/// [`static_location`] for a box some lines were built around. Out of line:
/// the caller is the read-back of every placed box, nearly all of which
/// stop at the flag.
#[inline(never)]
fn static_location_among_lines(
    tree: &NodeTree,
    node_id: RawNodeId,
    kind: Option<OutOfFlowKind>,
) -> Option<(f32, f32)> {
    // A box with an inset on each axis has no use for it.
    let (horizontal, vertical) = static_axes(&tree.nodes[node_id].computed_style);
    if !horizontal && !vertical {
        return None;
    }
    let (x, y) = inline_static_position(tree, node_id)?;
    // Percentage margins are of the containing block's width, on both axes.
    let basis = match kind {
        Some(kind) => ContainingBox::of(tree, kind),
        None => layout_parent(tree, node_id).and_then(|p| ContainingBox::of_ancestor(tree, p)),
    }
    .map_or(0.0, |cb| cb.width);
    let style = &tree.nodes[node_id].computed_style;
    Some((
        x + style.margin_left.resolve(basis).unwrap_or(0.0),
        y + style.margin_top.resolve(basis).unwrap_or(0.0),
    ))
}

/// Where the box holding `node_id` starts in the viewport **with nothing
/// scrolled**: the sum of the layouts above it. A `position: fixed` box's
/// static position is measured there (Chrome 153: shown inside a scroller
/// already scrolled by 40, it sits where it would with the scroller at 0),
/// and no scroll moves it afterwards.
///
/// The sum ends at a fixed ancestor, whose own `layout` is a viewport
/// position. An absolute ancestor placed against a non-parent containing
/// block carries, in its `layout`, the scroll of the boxes between the two
/// ([`place_absolute`]); that is taken off again.
fn unscrolled_parent_origin(tree: &mut NodeTree, node_id: RawNodeId) -> (f32, f32) {
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    let mut current = RinchDocument::box_tree_parent(&tree.nodes, node_id);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else { break };
        let (dx, dy) = crate::paint::ifc_content_box_offset(tree, node);
        x += node.layout.x + dx;
        y += node.layout.y + dy;
        match node.box_position() {
            PositionValue::Fixed => break,
            PositionValue::Absolute => {
                let end = match out_of_flow_kind(tree, id) {
                    Some(OutOfFlowKind::IcbAbsolute) => Some(None),
                    Some(OutOfFlowKind::AncestorAbsolute(cb)) => chain_end(tree, cb).map(Some),
                    _ => None,
                };
                if let Some(end) = end
                    && let Some((_, (sx, sy))) = chain_to_containing_block(tree, id, end, false)
                {
                    x -= sx;
                    y -= sy;
                }
            }
            _ => {}
        }
        current = RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    (x, y)
}

/// Where a `position: fixed` box goes on each axis that has both insets
/// `auto` — its static position, as a **viewport** coordinate, which is what
/// a fixed box's `layout` holds (issue #633). `None` on an axis with an
/// inset, which the read-back resolves against the viewport itself.
///
/// `taffy_location` is where Taffy put the box in its layout parent.
pub(crate) fn place_fixed_static(
    tree: &mut NodeTree,
    node_id: RawNodeId,
    taffy_location: (f32, f32),
) -> (Option<f32>, Option<f32>) {
    let Some(node) = tree.get(node_id) else {
        return (None, None);
    };
    let (horizontal, vertical) = static_axes(&node.computed_style);
    if !horizontal && !vertical {
        return (None, None);
    }
    let (sx, sy) = static_location(tree, node_id, Some(OutOfFlowKind::Fixed), taffy_location);
    let (ox, oy) = unscrolled_parent_origin(tree, node_id);
    (horizontal.then_some(ox + sx), vertical.then_some(oy + sy))
}

/// The lines holding (or no longer holding) `node_id`'s static position have
/// just been built, after the box was read back: put it where they say.
///
/// A box [`out_of_flow_kind`] classifies was recorded by the read-back and
/// is placed again by [`replace_all`], which this asks for. One it does not
/// — an absolute box whose layout parent is its containing block, Taffy's
/// own answer — is in no list, and is written here.
pub(crate) fn place_static_after_lines(tree: &mut NodeTree, node_id: RawNodeId) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    let (horizontal, vertical) = static_axes(&node.computed_style);
    if !horizontal && !vertical {
        return;
    }
    if out_of_flow_kind(tree, node_id).is_some() {
        tree.abs_late_moves = true;
        return;
    }
    if node.box_position() != PositionValue::Absolute
        || !is_laid_out(tree, node_id, (node.layout.width, node.layout.height))
    {
        return;
    }
    let Some(taffy) = node.taffy_id.and_then(|id| tree.taffy.layout(id).ok()) else {
        return;
    };
    let (sx, sy) = static_location(tree, node_id, None, (taffy.location.x, taffy.location.y));
    let node = &mut tree.nodes[node_id];
    let x = if horizontal { sx } else { node.layout.x };
    let y = if vertical { sy } else { node.layout.y };
    if node.layout.x == x && node.layout.y == y {
        return;
    }
    node.layout.x = x;
    node.layout.y = y;
    tree.paint_dirty_nodes.push(node_id);
    // A box placed against a containing block further up, from inside this
    // one, was placed from where this one stood.
    tree.abs_late_moves = true;
}

/// Write `node_id`'s placement as `kind` if it is not where it should be:
/// [`place_absolute`] (or [`place_fixed_static`]) for a box whose compute has
/// already been read back, from Taffy's last answer, the lines and the chain
/// as they stand. Returns whether it moved.
fn replace(tree: &mut NodeTree, node_id: RawNodeId, kind: OutOfFlowKind) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let size = (node.layout.width, node.layout.height);
    let at = (node.layout.x, node.layout.y);
    if !is_laid_out(tree, node_id, size) {
        return false;
    }
    let Some(taffy) = node.taffy_id.and_then(|id| tree.taffy.layout(id).ok()) else {
        return false;
    };
    let location = (taffy.location.x, taffy.location.y);
    let (x, y) = if kind == OutOfFlowKind::Fixed {
        let (x, y) = place_fixed_static(tree, node_id, location);
        (x.unwrap_or(at.0), y.unwrap_or(at.1))
    } else {
        let location = static_location(tree, node_id, Some(kind), location);
        match place_absolute(tree, node_id, kind, location, size, false) {
            Some(placed) => placed,
            None => return false,
        }
    };
    let node = &mut tree.nodes[node_id];
    if node.layout.x == x && node.layout.y == y {
        return false;
    }
    node.layout.x = x;
    node.layout.y = y;
    tree.paint_dirty_nodes.push(node_id);
    true
}

/// Place again every absolute box the read-back placed, now that the boxes
/// between each and its containing block are where they will be painted.
///
/// `read_layout_results` places a box as it reads it, from its ancestors'
/// layouts — but some things on its chain are written **later**: an
/// anonymous block box's own read-back, the position an inline formatting
/// context gives an atomic inline (`inline-block`), and a scroll offset the
/// post-layout clamp pulls in. Each of those three writers says when it
/// actually moved something (`NodeTree::abs_late_moves`); a layout in which
/// none did — nearly every relayout — has every box where the read-back put
/// it and looks at none here. Otherwise every placed box is compared, and one
/// whose chain did not move is not written. Run once, after all three, in
/// the order the boxes were read — parents first, so a box inside another
/// placed box sees it where it ends up.
///
/// `resolve_layout`'s text-only path (`!layout_dirty`) calls this as well:
/// it rebuilds dirty inline layouts, which is the second writer, with no
/// read-back (a `text-align` change moves an `inline-block` along its line
/// there). The flag is consumed here, so a later pass does not run on a
/// stale `true`.
pub(crate) fn replace_all(tree: &mut NodeTree) {
    if !std::mem::take(&mut tree.abs_late_moves) || tree.placed_absolutes.is_empty() {
        return;
    }
    let placed = std::mem::take(&mut tree.placed_absolutes);
    tree.perf
        .add(crate::perf::Counter::AbsBoxesVisited, placed.len() as u64);
    for &(id, kind) in &placed {
        replace(tree, id, kind);
    }
    tree.placed_absolutes = placed;
}

/// `scrolled` has a new scroll offset, and no layout will run for it: place
/// again every absolute box with `scrolled` strictly between it and its
/// containing block, so the box stays where it was — it is not that
/// scroller's content.
///
/// One flag read when `scrolled` is between no placed box and its containing
/// block, which is every scroll of a document whose absolute boxes are all
/// children of their own positioned parent. Otherwise the placed boxes are
/// asked one by one, each classified afresh: a change since the last layout
/// may have moved a box's containing block, and the layout that change owes
/// has not run.
pub(crate) fn replace_after_scroll(tree: &mut NodeTree, scrolled: RawNodeId) {
    if !tree.nodes.get(scrolled).is_some_and(|n| n.on_abs_chain) {
        return;
    }
    let placed = std::mem::take(&mut tree.placed_absolutes);
    tree.perf
        .add(crate::perf::Counter::AbsBoxesVisited, placed.len() as u64);
    for &(id, _) in &placed {
        let Some(kind) = out_of_flow_kind(tree, id) else {
            continue;
        };
        let between = match kind {
            OutOfFlowKind::IcbAbsolute => scrolled != tree.body_id,
            OutOfFlowKind::AncestorAbsolute(cb) => chain_end(tree, cb) != Some(scrolled),
            OutOfFlowKind::Fixed => false,
        };
        if between && is_box_ancestor(tree, scrolled, id) {
            replace(tree, id, kind);
        }
    }
    tree.placed_absolutes = placed;
}

/// Where the static position of the absolute box `node_id` starts on the
/// inline axis, measured from the padding edge of its containing block
/// (`kind`'s) — what an auto-width box with both inline insets `auto` gives
/// up of that block's width (#1404). Chrome 153: under a wrapper 42px into a
/// 394px containing block the box is shrunk to fit in 352.
///
/// Read from the layouts and lines **as they stand**, so the caller runs
/// after both are final. The box's own margin is not part of it.
///
/// It must not depend on the box's own width, or sizing the box from it
/// would feed back. So in a flex or grid container it is the edge Taffy
/// measures a start-aligned box from — the content edge of a flex container,
/// the padding edge of a grid — whatever `justify-content` or `align-items`
/// then does with the box (Chrome, for a box centred by its flex container,
/// takes twice the shorter distance from the centre to a containing-block
/// edge: `a11`/`a11b` in `tests/abs_shrink_to_fit_1404_tests.rs`).
///
/// `kind` is `None` for a box whose layout parent is its containing block.
fn static_inline_offset(
    tree: &mut NodeTree,
    node_id: RawNodeId,
    kind: Option<OutOfFlowKind>,
) -> Option<f32> {
    let parent_id = layout_parent(tree, node_id)?;
    let (end, cb) = match kind {
        Some(OutOfFlowKind::Fixed) => return None,
        Some(OutOfFlowKind::IcbAbsolute) => (None, ContainingBox::viewport(tree)),
        Some(kind @ OutOfFlowKind::AncestorAbsolute(cb)) => {
            (Some(chain_end(tree, cb)?), ContainingBox::of(tree, kind)?)
        }
        None => (
            Some(parent_id),
            ContainingBox::of_ancestor(tree, parent_id)?,
        ),
    };
    let ((ox, _), _) = chain_to_containing_block(tree, node_id, end, false)?;
    let parent = tree.get(parent_id)?;
    let edges = |parent: &Node| {
        let l = tree.taffy.layout(parent.taffy_id?).ok()?;
        Some((l.border.left, l.padding.left))
    };
    let x = match parent.computed_style.display {
        DisplayValue::Flex | DisplayValue::InlineFlex => {
            let (border, padding) = edges(parent)?;
            border + padding
        }
        DisplayValue::Grid | DisplayValue::InlineGrid => edges(parent)?.0,
        _ => {
            let node = tree.get(node_id)?;
            let taffy = tree.taffy.layout(node.taffy_id?).ok()?;
            let at = static_location(tree, node_id, kind, (taffy.location.x, taffy.location.y));
            at.0 - node
                .computed_style
                .margin_left
                .resolve(cb.width)
                .unwrap_or(0.0)
        }
    };
    Some(ox + x - cb.border_left)
}

/// Whether `ancestor` is on `node_id`'s box-tree parent chain.
fn is_box_ancestor(tree: &NodeTree, ancestor: RawNodeId, node_id: RawNodeId) -> bool {
    let mut current = RinchDocument::box_tree_parent(&tree.nodes, node_id);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    false
}

impl RinchDocument {
    /// Measure, in the lines as they stand, every inline span an absolute box
    /// hangs from (#631) — the spans of the boxes the read-back placed and of
    /// the ancestor-resolved ones — and keep the answers beside the lines
    /// (`InlineLayout::span_fragments`), where [`ContainingBox::of`] reads
    /// them.
    ///
    /// Lines that still hold an answer for each of their spans are left
    /// alone, so a layout that rebuilt no such line costs a lookup per box.
    /// Lines that were rebuilt (they come back with no answers), or that a
    /// box newly hangs from a span of, are measured **once**, for all their
    /// spans together ([`measure_span_fragments`]): the cost is the context's
    /// size plus the spans' content, not their product
    /// (`abs_inline_measure_steps`).
    ///
    /// Called where lines are rebuilt and boxes placed after them: by
    /// `resolve_layout` after `build_ifc_layouts`, on its text-only path too.
    pub(crate) fn measure_inline_containing_blocks(&mut self) {
        let tree = &self.tree;
        let mut wanted: Vec<(RawNodeId, RawNodeId)> = Vec::new();
        let mut want = |kind: Option<OutOfFlowKind>| {
            if let Some(OutOfFlowKind::AncestorAbsolute(cb)) = kind
                && let Some(span) = tree.get(cb)
                && !generates_layout_box(span)
                && let Some(root) = span.ifc_root
            {
                wanted.push((root, cb));
            }
        };
        for &(_, kind) in &tree.placed_absolutes {
            want(Some(kind));
        }
        // A box the read-back did not place (it is 0x0, or was never laid
        // out) is still asked about by the size check.
        for &id in &tree.ancestor_absolutes {
            want(out_of_flow_kind(tree, id));
        }
        wanted.sort_unstable();
        wanted.dedup();

        let mut styles: Vec<(u64, (f32, f32))> = Vec::new();
        let mut at = 0;
        while at < wanted.len() {
            let root_id = wanted[at].0;
            let end = at + wanted[at..].iter().take_while(|w| w.0 == root_id).count();
            let spans: Vec<RawNodeId> = wanted[at..end].iter().map(|w| w.1).collect();
            at = end;
            let tree = &self.tree;
            let Some(inline) = tree.get(root_id).and_then(|n| n.text_layout.as_ref()) else {
                continue;
            };
            if spans.iter().all(|s| inline.span_fragments.contains_key(s)) {
                continue;
            }
            let (font_cx, layout_cx) = (&mut self.font_cx, &mut self.layout_cx);
            let mut font = |style: &ComputedStyle| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                style.font_family.hash(&mut h);
                style.font_size.to_bits().hash(&mut h);
                style.font_weight.to_bits().hash(&mut h);
                (style.font_style as u8).hash(&mut h);
                let key = h.finish();
                if let Some(&(_, m)) = styles.iter().find(|(k, _)| *k == key) {
                    return m;
                }
                let m = font_box(font_cx, layout_cx, style);
                styles.push((key, m));
                m
            };
            let mut steps = 0;
            let measured = measure_span_fragments(tree, root_id, &spans, &mut font, &mut steps);
            tree.perf.bump(crate::perf::Counter::AbsInlineMeasures);
            tree.perf
                .add(crate::perf::Counter::AbsInlineMeasureSteps, steps);
            if let Some(inline) = self.tree.nodes[root_id].text_layout.as_mut() {
                inline.span_fragments = measured;
            }
        }
    }

    /// Bring every absolute box that is **shrunk to fit from its static
    /// position** (#1404; `NodeTree::abs_static_fits`, the ones the read-back
    /// placed) into agreement with where that position is now: the box's
    /// room is its containing block's width less the distance to it
    /// ([`static_inline_offset`]). Returns whether any Taffy style was
    /// rewritten; the caller then goes round — compute, read-back, lines.
    ///
    /// Called once the lines are built, because that is when the position is
    /// final: it may be a place in a line (#632, #634), and a box on the way
    /// to the containing block may be an atomic inline its own line has just
    /// placed. So the boxes the late writes moved are placed first
    /// ([`replace_all`]).
    ///
    /// The distance is kept on the node (`Node::abs_static_offset`), where
    /// every bake of the box reads it, so a layout in which it did not move
    /// rewrites nothing: one extra compute when such a box is first laid out
    /// away from its containing block's edge, and one when it is moved along
    /// the inline axis. A box's own width does not move its static position
    /// (an out-of-flow box sizes nothing above it), so one round settles it.
    ///
    /// One `is_empty` in a document with no such box.
    pub(crate) fn resolve_static_shrink_to_fit(&mut self) -> bool {
        if self.tree.abs_static_fits.is_empty() {
            return false;
        }
        replace_all(&mut self.tree);
        let fits = std::mem::take(&mut self.tree.abs_static_fits);
        self.tree
            .perf
            .add(crate::perf::Counter::AbsBoxesVisited, fits.len() as u64);
        let mut changed = false;
        for &(id, kind) in &fits {
            let statics = fits_from_static_position(&self.tree.nodes[id].computed_style);
            let offset = if statics {
                match static_inline_offset(&mut self.tree, id, kind) {
                    Some(offset) => offset,
                    None => continue,
                }
            } else {
                0.0
            };
            let node = &self.tree.nodes[id];
            let moved = (node.abs_static_offset - offset).abs() > 0.01;
            // A box resolved against a block Taffy does not know is kept in
            // step with that block's size by the bake sites and
            // `resolve_ancestor_absolutes`, which read the offset: only a
            // move is this pass's. One Taffy resolves itself carries a size
            // baked from its parent's last layout, which this pass alone
            // checks against the size the compute just gave the parent.
            if kind.is_some() && !moved {
                continue;
            }
            let Some(taffy_id) = node.taffy_id else {
                continue;
            };
            // Neither has a size built from its computed values.
            if node.estimated_height.is_some() || node.taffy_style_owned_by_contents_splice() {
                continue;
            }
            self.tree.nodes[id].abs_static_offset = offset;
            let node = &self.tree.nodes[id];
            let Ok(current) = self.tree.taffy.style(taffy_id) else {
                continue;
            };
            let mut next = current.clone();
            unbake(&node.computed_style, false, &mut next);
            match kind {
                Some(kind) => {
                    let Some(cb) = ContainingBox::of(&self.tree, kind) else {
                        continue;
                    };
                    apply_out_of_flow_size_overrides(node, kind, (cb.width, cb.height), &mut next);
                }
                None => fit_in_layout_parent(&self.tree, node, &mut next),
            }
            if next.size != current.size {
                let _ = self.tree.taffy.set_style(taffy_id, next);
                // The root compute does not reach a box inside an atomic
                // inline (#661).
                self.mark_atomic_inline_dirty(id);
                self.tree.perf.bump(crate::perf::Counter::TaffyStyleChanges);
                changed = true;
            }
        }
        self.tree.abs_static_fits = fits;
        changed
    }

    /// Bring every absolute box whose containing block is a non-parent
    /// ancestor (#386) into agreement with that ancestor's size **as the
    /// compute that just ran left it**. Returns whether any Taffy style was
    /// rewritten — the caller re-runs the compute until this answers `false`
    /// (see `resolve_layout` and the module doc).
    ///
    /// Each box is compared, not assumed: the fields a bake writes are
    /// rebuilt from the computed values and the ancestor's padding box and
    /// the style written only where one differs, so a box a style site
    /// already baked correctly, and one whose size does not depend on its
    /// containing block at all, cost a compare and no compute. A box that
    /// **was** baked against an ancestor and no longer has one — an
    /// intermediate box became `display: contents`, or the hoisting changed —
    /// is put back the same way.
    ///
    /// Iterates [`NodeTree::ancestor_absolutes`] and nothing else: a document
    /// whose absolute boxes are all Taffy's own answer pays one `is_empty`.
    ///
    /// With `inline_only`, only the boxes whose containing block is an
    /// inline span (#631) are looked at: the call `resolve_layout` makes
    /// after the lines are built, which is when such a block has the size
    /// this layout gives it. For those boxes, and only in that call, a block
    /// that cannot be measured (the span became a split inline) takes a bake
    /// back, so the box is Taffy's again.
    pub(crate) fn resolve_ancestor_absolutes(&mut self, inline_only: bool) -> bool {
        if self.tree.ancestor_absolutes.is_empty() {
            return false;
        }
        let mut ids: Vec<RawNodeId> = self.tree.ancestor_absolutes.iter().copied().collect();
        if inline_only {
            ids.retain(|&id| {
                has_inline_containing_block(&self.tree, out_of_flow_kind(&self.tree, id))
            });
        }
        self.tree
            .perf
            .add(crate::perf::Counter::AbsBoxesVisited, ids.len() as u64);
        let mut changed = false;
        // The fields a bake touches, rebuilt here per box; the rest of a
        // style is only cloned for a box that is actually rewritten.
        let mut next = taffy::Style::DEFAULT;
        for id in ids {
            let live = self
                .tree
                .nodes
                .get(id)
                .is_some_and(|n| n.box_position() == PositionValue::Absolute);
            if !live {
                // Freed, or no longer absolute: a superset entry to drop.
                self.tree.ancestor_absolutes.remove(&id);
                if let Some(node) = self.tree.nodes.get_mut(id) {
                    node.abs_ancestor_recorded = false;
                    node.abs_ancestor_baked = false;
                }
                continue;
            }
            let node = &self.tree.nodes[id];
            let Some(taffy_id) = node.taffy_id else {
                continue;
            };
            // Neither has a size built from its computed values to compare.
            if node.estimated_height.is_some() || node.taffy_style_owned_by_contents_splice() {
                continue;
            }
            let kind = out_of_flow_kind(&self.tree, id);
            let was_baked = node.abs_ancestor_baked;
            let Ok(current) = self.tree.taffy.style(taffy_id) else {
                continue;
            };
            copy_baked_fields(&mut next, current);
            // Whether the box stays in the set although it carries no bake.
            let mut keep = false;
            let baked = match kind {
                Some(kind @ OutOfFlowKind::AncestorAbsolute(cb)) => {
                    match ContainingBox::of_ancestor(&self.tree, cb) {
                        Some(cb) => {
                            unbake(&node.computed_style, false, &mut next);
                            apply_out_of_flow_size_overrides(
                                node,
                                kind,
                                (cb.width, cb.height),
                                &mut next,
                            );
                            true
                        }
                        // The lines are built and the span has no box in
                        // them: a size baked from the box it had is stale.
                        None if inline_only && was_baked => {
                            unbake(&node.computed_style, true, &mut next);
                            keep = true;
                            false
                        }
                        None => continue,
                    }
                }
                // Baked against an ancestor it no longer resolves against.
                _ if was_baked => {
                    unbake(&node.computed_style, true, &mut next);
                    match kind {
                        Some(kind) => {
                            let vp = self.tree.viewport;
                            apply_out_of_flow_size_overrides(
                                node,
                                kind,
                                (vp.width, vp.height),
                                &mut next,
                            );
                        }
                        // Taffy's own answer again, as a style site would
                        // have written it.
                        None => fit_in_layout_parent(&self.tree, node, &mut next),
                    }
                    false
                }
                // Neither ancestor-resolved nor carrying an ancestor's size:
                // nothing here is this pass's business any more.
                _ => {
                    self.tree.ancestor_absolutes.remove(&id);
                    self.tree.nodes[id].abs_ancestor_recorded = false;
                    continue;
                }
            };
            let rewritten = (!same_baked_fields(&next, current)).then(|| {
                let mut full = current.clone();
                copy_baked_fields(&mut full, &next);
                full
            });
            let node = &mut self.tree.nodes[id];
            node.abs_ancestor_baked = baked;
            if !baked && !keep {
                node.abs_ancestor_recorded = false;
                self.tree.ancestor_absolutes.remove(&id);
            }
            if let Some(full) = rewritten {
                let _ = self.tree.taffy.set_style(taffy_id, full);
                // The root compute does not reach a box inside an atomic
                // inline (#661).
                self.mark_atomic_inline_dirty(id);
                self.tree.perf.bump(crate::perf::Counter::TaffyStyleChanges);
                changed = true;
            }
        }
        changed
    }
}
