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
//! **position** correction lives in `layout_engine::read_layout_results`, which
//! writes the parent-relative delta so `LayoutResult` stays parent-relative and
//! no coordinate consumer has to learn a new rule.
//!
//! ## What is corrected, and what is not
//!
//! Only two cases are answered here, both of which have a containing block
//! whose size is known *before* layout runs:
//!
//! - [`OutOfFlowKind::Fixed`] — the viewport. Unchanged behaviour; this module
//!   only gives the existing correction a single home so all three
//!   style-application sites get it.
//! - [`OutOfFlowKind::IcbAbsolute`] — an absolute box with no positioned
//!   ancestor at all, whose containing block is therefore the initial
//!   containing block. In rinch the `<html>` box *is* the viewport at (0, 0),
//!   so the ICB is known up front the same way the viewport is.
//!
//! An absolute box whose containing block is a **positioned ancestor that is
//! not the direct parent** is deliberately left alone: that ancestor's used
//! size is not known until a first compute pass has run, so correcting it needs
//! the two-pass shape `ifc::resolve_percentage_inline_blocks` already uses.
//! That is the other half of #204, tracked as #386. Percentage
//! `padding`/`margin` on the box itself, percentage `min-`/`max-` sizes, and
//! the shrink-to-fit available width of an auto-sized absolute are likewise
//! still resolved against the Taffy parent.

use crate::computed_style::{DimensionValue, DisplayValue, PositionValue};
use crate::layout::Viewport;
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
    match node.computed_style.position {
        PositionValue::Fixed => Some(OutOfFlowKind::Fixed),
        PositionValue::Absolute => {
            let mut current = node.parent?;
            loop {
                // `<html>`'s box is the viewport at the origin, so reaching it
                // — whatever its own `position` — means the containing block is
                // the ICB.
                if current == tree.html_id || current == tree.root_id {
                    return Some(OutOfFlowKind::IcbAbsolute);
                }
                let ancestor = tree.get(current)?;
                if ancestor.establishes_abs_containing_block() {
                    // The direct parent establishing it is Taffy's own answer,
                    // so there is nothing to correct; a further ancestor
                    // establishing it is the half of #204 left for the
                    // follow-up. Either way, hands off.
                    return None;
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
/// * **`position: absolute` contributes only when `parent` is its containing
///   block**: `parent` is positioned or transformed
///   ([`Node::establishes_abs_containing_block`]), or *is* the initial
///   containing block, which in rinch is the `<html>` box. Those are exactly
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
            parent_id == tree.html_id
                || parent_id == tree.root_id
                || tree
                    .get(parent_id)
                    .is_some_and(Node::establishes_abs_containing_block)
        }
        _ => true,
    }
}

/// Bake an out-of-flow box's Taffy **size** from its real containing block.
///
/// Taffy would size the box against its direct parent, and the size has to be
/// right *before* `compute_layout` runs or the box's own subtree lays out
/// inside the wrong box — a percentage child, a `height: 100%` child, or text
/// wrapping would all be measured against the parent instead of the containing
/// block. That is why this is a pre-layout bake and not another post-layout
/// patch.
///
/// Call this from every site that rebuilds a Taffy style out of `ComputedStyle`
/// — `apply_stylo_styles_to_taffy`, `tick_transitions`, `tick_animations` —
/// or a restyle drops the override until the next full cascade.
pub(crate) fn apply_out_of_flow_size_overrides(
    node: &Node,
    kind: OutOfFlowKind,
    viewport: Viewport,
    taffy_style: &mut taffy::Style,
) {
    let vw = viewport.width;
    let vh = viewport.height;
    let cs = &node.computed_style;

    if taffy_style.size.width == taffy::Dimension::auto() {
        match (cs.left.resolve(vw), cs.right.resolve(vw)) {
            (Some(l), Some(r)) => {
                taffy_style.size.width = taffy::Dimension::length((vw - l - r).max(0.0));
            }
            // A fixed box with unpaired insets fills the viewport, as it has
            // since the fixed path was written. An absolute one must not: an
            // auto-width absolute still shrinks to fit, so leave it auto and
            // let Taffy measure it.
            _ if kind == OutOfFlowKind::Fixed => {
                taffy_style.size.width = taffy::Dimension::length(vw);
            }
            _ => {}
        }
    } else if kind == OutOfFlowKind::IcbAbsolute {
        // Taffy would resolve these against the direct parent; the containing
        // block is the ICB, i.e. the viewport. A mixed calc() takes the same
        // arm as a plain percentage (#278/#496 review).
        match cs.width {
            DimensionValue::Percent(p) => {
                taffy_style.size.width = taffy::Dimension::length((vw * p).max(0.0));
            }
            DimensionValue::Calc { px, pct } => {
                taffy_style.size.width = taffy::Dimension::length((vw * pct + px).max(0.0));
            }
            _ => {}
        }
    }

    if taffy_style.size.height == taffy::Dimension::auto() {
        match (cs.top.resolve(vh), cs.bottom.resolve(vh)) {
            (Some(t), Some(b)) => {
                taffy_style.size.height = taffy::Dimension::length((vh - t - b).max(0.0));
            }
            _ if kind == OutOfFlowKind::Fixed => {
                taffy_style.size.height = taffy::Dimension::length(vh);
            }
            _ => {}
        }
    } else if kind == OutOfFlowKind::IcbAbsolute {
        match cs.height {
            DimensionValue::Percent(p) => {
                taffy_style.size.height = taffy::Dimension::length((vh * p).max(0.0));
            }
            DimensionValue::Calc { px, pct } => {
                taffy_style.size.height = taffy::Dimension::length((vh * pct + px).max(0.0));
            }
            _ => {}
        }
    }
}
