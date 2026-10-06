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
//! - [`OutOfFlowKind::Fixed`] — the viewport. Unchanged behaviour; this module
//!   only gives the existing correction a single home so every
//!   style-application site gets it.
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
//! padding box to the bottom-right of its last's — and
//! [`inline_fragments_box`] reads that out of the lines of the inline
//! formatting context that flows it. Two things follow from where those
//! lines come from:
//!
//! - **they are built after the read-back** (`build_ifc_layouts`), so at the
//!   size check of step 2 and at the placement of the read-back the span's
//!   box is the one the *last* layout's lines gave it, or none. Both are
//!   therefore done once more after the lines: `resolve_layout` calls
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
//! every box Taffy places is already on.
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
//! ## What is not corrected
//!
//! - A **split** inline span — one holding a block-level child (#513), whose
//!   fragments lie in several anonymous boxes — is not measured: a box inside
//!   it keeps Taffy's answer, the block container (issue #1424).
//! - A relative span's own `left`/`top` moves neither its text nor a box
//!   resolved against it (issue #1425), and its horizontal padding
//!   takes no room on the line (issue #1426): the containing block is the
//!   span as rinch draws it.
//! - The shrink-to-fit *available* width of an auto-width absolute is still
//!   the Taffy parent's (Chrome: a box of four 130px inline-blocks under a
//!   200px parent in a 400px containing block is 400x40; rinch: 130x80 —
//!   issue #1404).
//! - `position: fixed` takes none of the margin, padding or min/max rules
//!   below, and a transformed ancestor does not contain it (#1372).

use crate::RinchDocument;
use crate::computed_style::{
    ComputedStyle, DimensionValue, DisplayValue, LengthPercentageAutoValue, LengthPercentageValue,
    PositionValue,
};
use crate::node::{Node, NodeTree, RawNodeId};

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

/// The top and bottom of an inline fragment set in `run`'s font: the font's
/// rounded ascent and descent around the baseline, which is the box Chrome
/// gives an inline element (its `getClientRects` are 20px tall for Inter at
/// 16px, ascent 15.5 and descent 3.9, on any line height).
fn run_extent(run: &parley::layout::GlyphRun<'_, peniko::Brush>) -> (f32, f32) {
    let m = run.run().metrics();
    (
        run.baseline() - m.ascent.round(),
        run.baseline() + m.descent.round(),
    )
}

/// The same for a fragment with no run of its own on `line`: the line's
/// first run stands in for its font, and a line with no text at all (only
/// atomic inlines) gives its own box.
fn line_extent(line: &parley::layout::Line<'_, peniko::Brush>) -> (f32, f32) {
    line.items()
        .find_map(|item| match item {
            parley::layout::PositionedLayoutItem::GlyphRun(run) => Some(run_extent(&run)),
            _ => None,
        })
        .unwrap_or_else(|| {
            let m = line.metrics();
            (m.block_min_coord, m.block_max_coord)
        })
}

/// The containing block a positioned **inline** element forms (CSS 2.1
/// §10.1): from the top-left of its first fragment's padding box to the
/// bottom-right of its last fragment's, in the lines last built for the
/// inline formatting context that flows it. `border_left`/`border_top` are
/// measured from the **border box** of the block container holding those
/// lines ([`inline_host`]), which is where the chain of an absolute box
/// inside the span ends ([`chain_end`]).
///
/// Measured in Chrome 153:
///
/// - a fragment is the element's **own font's** rounded ascent and descent
///   around the line's baseline, whatever the line box's height — a 32px span
///   in a 16px/20px line is 39px tall and starts above the line;
/// - a span over several lines runs from its first fragment's left edge to
///   its last fragment's right edge, and when that is left of the first the
///   width is zero (`right: 0` then hangs the box from the first fragment's
///   left edge);
/// - an empty span is a zero-width fragment where it sits in the line;
/// - a span holding only an atomic inline is as wide as that box and as tall
///   as its font, not as the box.
///
/// `None` when the span has no lines to be measured in: its context has no
/// inline layout yet (the first layout, before its lines are built; a
/// collapsed or `display: none` subtree), or it is a **split** inline (one
/// holding a block-level child, #513), whose fragments lie in several
/// anonymous boxes — that one keeps Taffy's answer, the block holding the
/// absolute box's own line.
///
/// rinch gives an inline element's padding no room on its line and draws no
/// inline border, so the padding box here is the glyph box grown by the
/// padding, as the element's background is painted.
fn inline_fragments_box(tree: &NodeTree, span_id: RawNodeId) -> Option<ContainingBox> {
    use parley::layout::PositionedLayoutItem;

    let span = tree.get(span_id)?;
    if span.is_split_inline() {
        return None;
    }
    let root_id = span.ifc_root?;
    let root = tree.get(root_id)?;
    let inline = root.text_layout.as_ref()?;
    let layout = &inline.layout;

    // The span's content in this context: the entries that follow its own
    // in the walk's order, for as long as they are its descendants.
    let at = inline
        .child_positions
        .iter()
        .position(|(id, _)| *id == span_id)?;
    let inside = |mut id: RawNodeId| {
        while let Some(parent) = tree.get(id).and_then(|n| n.parent) {
            if parent == span_id {
                return true;
            }
            id = parent;
        }
        false
    };
    let members: Vec<RawNodeId> = inline.child_positions[at + 1..]
        .iter()
        .map(|(id, _)| *id)
        .take_while(|id| inside(*id))
        .collect();
    // Its text, as one range of the flat text (the members are contiguous).
    let mut text: Option<(usize, usize)> = None;
    for range in &inline.text_ranges {
        if members.contains(&range.node_id) {
            let (a, b) = text.unwrap_or((range.flat_start, range.flat_end));
            text = Some((a.min(range.flat_start), b.max(range.flat_end)));
        }
    }
    let is_member_box = |id: u64| {
        members.contains(&(id as usize))
            && tree
                .get(id as usize)
                .is_some_and(|n| n.display_mode.is_atomic_inline())
    };

    // (left, top) of the first fragment, (right, bottom) of the last.
    let mut first: Option<(f32, f32)> = None;
    let mut last: Option<(f32, f32)> = None;
    for line in layout.lines() {
        let range = line.text_range();
        let (mut left, mut right) = (f32::INFINITY, f32::NEG_INFINITY);
        if let Some((a, b)) = text {
            let (start, end) = (a.max(range.start), b.min(range.end));
            if start < end {
                (left, right) = crate::text_query::line_range_x(layout, &line, start, end, false);
            }
        }
        // The span's own runs on this line: the first one's ascent and the
        // last one's descent are the fragment's.
        let mut above: Option<f32> = None;
        let mut below: Option<f32> = None;
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(run) => {
                    let Some((a, b)) = text else { continue };
                    let r = run.run().text_range();
                    if r.start < b && a < r.end {
                        let (top, bottom) = run_extent(&run);
                        above.get_or_insert(top);
                        below = Some(bottom);
                    }
                }
                PositionedLayoutItem::InlineBox(b) if is_member_box(b.id) => {
                    left = left.min(b.x);
                    right = right.max(b.x + b.width);
                }
                PositionedLayoutItem::InlineBox(_) => {}
            }
        }
        if left > right {
            continue;
        }
        // A span with no text of its own on the line (it holds only an
        // atomic inline there) stands in the line's font.
        let (top, bottom) = line_extent(&line);
        if first.is_none() {
            first = Some((left, above.unwrap_or(top)));
        }
        last = Some((right, below.unwrap_or(bottom)));
    }

    let (mut left, mut top, mut right, mut bottom) = match (first, last) {
        (Some((l, t)), Some((r, b))) => (l, t, r, b),
        // Nothing of the span is on any line: an empty span. It sits where
        // the content before it ends.
        _ => {
            let mut x = None;
            let mut byte = text.map(|(a, _)| a);
            if byte.is_none() {
                for &(id, _) in inline.child_positions[..at].iter().rev() {
                    if let Some(end) = inline
                        .text_ranges
                        .iter()
                        .filter(|r| r.node_id == id)
                        .map(|r| r.flat_end)
                        .max()
                    {
                        byte = Some(end);
                        break;
                    }
                    let atomic = tree.get(id).is_some_and(|n| n.is_element())
                        && tree
                            .get(id)
                            .is_some_and(|n| n.display_mode.is_atomic_inline());
                    if atomic {
                        x = layout.lines().find_map(|line| {
                            line.items().find_map(|item| match item {
                                PositionedLayoutItem::InlineBox(b) if b.id as usize == id => {
                                    Some((b.x + b.width, line_extent(&line)))
                                }
                                _ => None,
                            })
                        });
                        break;
                    }
                }
            }
            let (x, (top, bottom)) = match x {
                Some(found) => found,
                None => {
                    let caret = parley::Cursor::from_byte_index(
                        layout,
                        byte.unwrap_or(0),
                        parley::layout::Affinity::Downstream,
                    )
                    .geometry(layout, 0.0);
                    let mid = ((caret.y0 + caret.y1) / 2.0) as f32;
                    let line = layout
                        .lines()
                        .find(|l| {
                            let m = l.metrics();
                            (m.block_min_coord..m.block_max_coord).contains(&mid)
                        })
                        .or_else(|| layout.lines().next())?;
                    (caret.x0 as f32, line_extent(&line))
                }
            };
            (x, top, x, bottom)
        }
    };
    let cs = &span.computed_style;
    left -= cs.padding_left.to_px();
    top -= cs.padding_top.to_px();
    right += cs.padding_right.to_px();
    bottom += cs.padding_bottom.to_px();
    let (_, (ox, oy)) = inline_host(tree, root_id)?;
    Some(ContainingBox {
        width: (right - left).max(0.0),
        height: (bottom - top).max(0.0),
        border_left: ox + left,
        border_top: oy + top,
    })
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
    let Some(kind @ OutOfFlowKind::AncestorAbsolute(cb)) = out_of_flow_kind(tree, node_id) else {
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
    } else if taffy_style.size.width == taffy::Dimension::auto() {
        match (cs.left.resolve(cw), cs.right.resolve(cw)) {
            (Some(l), Some(r)) => {
                taffy_style.size.width = taffy::Dimension::length((cw - l - r - ml - mr).max(0.0));
            }
            // A fixed box with unpaired insets fills the viewport, as it has
            // since the fixed path was written. An absolute one must not: an
            // auto-width absolute still shrinks to fit, so leave it auto and
            // let Taffy measure it.
            _ if !absolute => {
                taffy_style.size.width = taffy::Dimension::length(cw);
            }
            _ => {}
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
/// `taffy_location` is where Taffy put it (its static position on an axis
/// with no inset) and `size` its used border-box size. `None` for
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

/// Write `node_id`'s placement as `kind` if it is not where it should be:
/// [`place_absolute`] for a box whose compute has already been read back,
/// from Taffy's last answer and the chain as it stands. Returns whether it
/// moved.
fn replace(tree: &mut NodeTree, node_id: RawNodeId, kind: OutOfFlowKind) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let size = (node.layout.width, node.layout.height);
    if !is_laid_out(tree, node_id, size) {
        return false;
    }
    let Some(taffy) = node.taffy_id.and_then(|id| tree.taffy.layout(id).ok()) else {
        return false;
    };
    let location = (taffy.location.x, taffy.location.y);
    let Some((x, y)) = place_absolute(tree, node_id, kind, location, size, false) else {
        return false;
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
                    if let Some(kind) = kind {
                        let vp = self.tree.viewport;
                        apply_out_of_flow_size_overrides(
                            node,
                            kind,
                            (vp.width, vp.height),
                            &mut next,
                        );
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
