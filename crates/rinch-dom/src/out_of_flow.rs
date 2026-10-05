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
//! [`NodeTree::absolute_registry`] is what makes step 2 cheap: every
//! `position: absolute` node a style site has seen, filtered at use. A
//! document with none pays one `is_empty`.
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
//! `NodeTree::mark_scrolled`.
//!
//! ## What is not corrected
//!
//! - A containing block that generates no box — a `position: relative`
//!   **inline** span — is left to Taffy, which resolves against the span's
//!   block container (issue #631).
//! - The shrink-to-fit *available* width of an auto-width absolute is still
//!   the Taffy parent's (Chrome: a box of four 130px inline-blocks under a
//!   200px parent in a 400px containing block is 400 wide; rinch: 260).
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
    match node.computed_style.position {
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
                let has_box = generates_layout_box(ancestor);
                if layout_parent.is_none() && has_box {
                    layout_parent = Some(current);
                }
                if ancestor.establishes_abs_containing_block() {
                    // The layout parent establishing it is Taffy's own answer,
                    // so there is nothing to correct. So is — for now — a
                    // containing block with no box of its own, a `position:
                    // relative` inline span: Taffy resolves against the span's
                    // block container (#631). Anything else is an ancestor
                    // Taffy does not know about (#386).
                    return (has_box && layout_parent != Some(current))
                        .then_some(OutOfFlowKind::AncestorAbsolute(current));
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
    /// line was built around.
    fn of_ancestor(tree: &NodeTree, cb: RawNodeId) -> Option<Self> {
        let layout = tree.taffy.layout(tree.get(cb)?.taffy_id?).ok()?;
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

/// Bake `node_id`'s out-of-flow size into a Taffy style a site has just
/// rebuilt from `computed_style`, and keep [`NodeTree::absolute_registry`] and
/// [`NodeTree::ancestor_baked`] telling the truth about it.
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
    let absolute = node.computed_style.position == PositionValue::Absolute;
    let mut baked_against_ancestor = false;
    if let Some(kind) = out_of_flow_kind(tree, node_id) {
        let known = ContainingBox::of(tree, kind).filter(|cb| {
            !matches!(kind, OutOfFlowKind::AncestorAbsolute(_)) || cb.width > 0.0 || cb.height > 0.0
        });
        if let Some(cb) = known {
            apply_out_of_flow_size_overrides(node, kind, (cb.width, cb.height), taffy_style);
            baked_against_ancestor = matches!(kind, OutOfFlowKind::AncestorAbsolute(_));
        }
    }
    if absolute {
        tree.absolute_registry.insert(node_id);
    }
    if baked_against_ancestor {
        tree.ancestor_baked.insert(node_id);
    } else if !tree.ancestor_baked.is_empty() {
        tree.ancestor_baked.remove(&node_id);
    }
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
fn chain_to_containing_block(
    tree: &NodeTree,
    node_id: RawNodeId,
    cb: Option<RawNodeId>,
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
pub(crate) fn place_absolute(
    tree: &NodeTree,
    node_id: RawNodeId,
    kind: OutOfFlowKind,
    taffy_location: (f32, f32),
    size: (f32, f32),
) -> Option<(f32, f32)> {
    let cb_id = match kind {
        OutOfFlowKind::Fixed => return None,
        OutOfFlowKind::IcbAbsolute => None,
        OutOfFlowKind::AncestorAbsolute(cb) => Some(cb),
    };
    let cb = ContainingBox::of(tree, kind)?;
    let ((ox, oy), (sx, sy)) = chain_to_containing_block(tree, node_id, cb_id)?;
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
    // With both insets `auto` the box keeps Taffy's static position — which
    // CSS *does* take from the flow position in the parent.
    Some((
        x.map_or(taffy_location.0, |x| cb.border_left + x - ox) + sx,
        y.map_or(taffy_location.1, |y| cb.border_top + y - oy) + sy,
    ))
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

/// [`place_absolute`] for a box whose compute has already been read back:
/// the position it should have now, from Taffy's last answer and the chain as
/// it stands. `None` when the box is not one this module places.
fn current_placement(tree: &NodeTree, node_id: RawNodeId) -> Option<(f32, f32)> {
    let node = tree.get(node_id)?;
    if node.computed_style.position != PositionValue::Absolute {
        return None;
    }
    let kind = out_of_flow_kind(tree, node_id)?;
    if !is_laid_out(tree, node_id, (node.layout.width, node.layout.height)) {
        return None;
    }
    let taffy = tree.taffy.layout(node.taffy_id?).ok()?;
    place_absolute(
        tree,
        node_id,
        kind,
        (taffy.location.x, taffy.location.y),
        (node.layout.width, node.layout.height),
    )
}

/// Write `node_id`'s placement if it is not where it should be. Returns
/// whether it moved.
fn replace(tree: &mut NodeTree, node_id: RawNodeId) -> bool {
    let Some((x, y)) = current_placement(tree, node_id) else {
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

/// Place every absolute box again, now that the boxes between each and its
/// containing block are where they will be painted.
///
/// `read_layout_results` places a box as it reads it, from its ancestors'
/// layouts — but two things on its chain are written **later**: the position
/// an inline formatting context gives an atomic inline (`inline-block`), and
/// a scroll offset the post-layout clamp pulls in. A box whose chain holds
/// neither is already right and is not written. Run once, after both.
pub(crate) fn replace_all(tree: &mut NodeTree) {
    if tree.absolute_registry.is_empty() {
        return;
    }
    let ids: Vec<RawNodeId> = tree.absolute_registry.iter().copied().collect();
    for id in ids {
        replace(tree, id);
    }
}

/// `scrolled` has a new scroll offset, and no layout will run for it: place
/// again every absolute box with `scrolled` strictly between it and its
/// containing block, so the box stays where it was — it is not that
/// scroller's content. Returns whether any box was moved (the caller's hit
/// cache then holds a stale box).
pub(crate) fn replace_after_scroll(tree: &mut NodeTree, scrolled: RawNodeId) -> bool {
    if tree.absolute_registry.is_empty() {
        return false;
    }
    let ids: Vec<RawNodeId> = tree.absolute_registry.iter().copied().collect();
    let mut moved = false;
    for id in ids {
        let between = match out_of_flow_kind(tree, id) {
            Some(OutOfFlowKind::IcbAbsolute) => scrolled != tree.body_id,
            Some(OutOfFlowKind::AncestorAbsolute(cb)) => scrolled != cb,
            _ => false,
        };
        if between && is_box_ancestor(tree, scrolled, id) {
            moved |= replace(tree, id);
        }
    }
    moved
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
    /// Each box is compared, not assumed: its style is rebuilt from the
    /// computed values and the ancestor's padding box and written only where
    /// it differs, so a box a style site already baked correctly, and one
    /// whose size does not depend on its containing block at all, cost a
    /// compare and no compute. A box that **was** baked against an ancestor
    /// and no longer has one — an intermediate box became `display: contents`,
    /// or the hoisting changed — is put back the same way.
    pub(crate) fn resolve_ancestor_absolutes(&mut self) -> bool {
        if self.tree.absolute_registry.is_empty() {
            return false;
        }
        let ids: Vec<RawNodeId> = self.tree.absolute_registry.iter().copied().collect();
        let mut changed = false;
        for id in ids {
            let live = self
                .tree
                .nodes
                .get(id)
                .is_some_and(|n| n.computed_style.position == PositionValue::Absolute);
            if !live {
                // Freed, or no longer absolute: a superset entry to drop.
                self.tree.absolute_registry.remove(&id);
                self.tree.ancestor_baked.remove(&id);
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
            let was_baked = self.tree.ancestor_baked.contains(&id);
            let Ok(current) = self.tree.taffy.style(taffy_id) else {
                continue;
            };
            let (next, baked) = match kind {
                Some(kind @ OutOfFlowKind::AncestorAbsolute(cb)) => {
                    let Some(cb) = ContainingBox::of_ancestor(&self.tree, cb) else {
                        continue;
                    };
                    let mut next = current.clone();
                    unbake(&node.computed_style, false, &mut next);
                    apply_out_of_flow_size_overrides(node, kind, (cb.width, cb.height), &mut next);
                    (next, true)
                }
                // Baked against an ancestor it no longer resolves against.
                _ if was_baked => {
                    let mut next = current.clone();
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
                    (next, false)
                }
                _ => continue,
            };
            let differs = next != *current;
            if baked {
                self.tree.ancestor_baked.insert(id);
            } else {
                self.tree.ancestor_baked.remove(&id);
            }
            if differs {
                let _ = self.tree.taffy.set_style(taffy_id, next);
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
