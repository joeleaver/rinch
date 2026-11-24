//! A box's scrollable range: [`content_extents`] and [`scroll_extents`], the
//! in-flow ranges layout's clamp asks first, and the per-container memo of
//! the exact in-flow measure ([`ExactFlow`]).

use super::deep_walk::{deep_overflow, descend_into};
use super::measure::measure_content;
use super::visible_extents;
use crate::NodeTree;
use crate::node::RawNodeId;

/// The content extent `(width, height)` along both axes, relative to the
/// container's content box, from its child **boxes'** layout rects — the
/// container's box-tree children, which besides its element children are the
/// anonymous block boxes laying out runs of its inline content beside a block
/// child (#995) and the out-of-flow boxes hoisted out of its inline elements
/// (#591), and the children of any `display: contents` child, which has no box
/// of its own to measure (issue #396; see `extend_with_children`).
///
/// Only boxes the container is the **containing block** of are measured —
/// `crate::out_of_flow::contributes_to_scrollable_overflow` is the rule, and
/// its doc is where the CSS and the Chrome measurements live. A `position:
/// fixed` child resolves against the viewport and a `position: absolute` one
/// may resolve against an ancestor, and neither swells the scroll range of a
/// box that merely happens to be its Taffy parent (issue #765).
///
/// **And every absolute box below the first box whose range this is**
/// (issue #770): one laid out in a box that is no scroll container, or whose
/// containing block is above the box it is laid out in, is part of the
/// scrollable overflow of the nearest scroll container at or above its
/// containing block (`out_of_flow::overflow_scroller`; the page's is the
/// `<body>`'s). The layout read-back lists them
/// (`NodeTree::overflow_absolutes`), and `extend_with_overflow_absolutes`
/// measures this container's through every box between them, once per
/// geometry (`OverflowAbsolutes`).
///
/// Taffy's `child.layout.{x,y}` are relative to the **parent's border box**, so
/// they include the leading padding and border; subtracting that offset is what
/// keeps a padded container from deciding it overflows when it does not.
/// rinch works in the container's *content*-box frame where Chrome's
/// `scrollWidth`/`clientWidth` work in its *padding*-box frame (issue #480).
///
/// **How each box contributes (issues #776 and #775),** css-overflow-3 §2.2 as
/// Chrome 156 measures it (`tests/scroll_range_margin_transform_776_tests.rs`
/// has the rows):
///
/// * **In flow** (anything but `absolute`): its **margin box** at its *static*
///   position — the layout rect less a `relative` offset, grown by the end
///   margins as declared (a percentage against the container's content width,
///   `auto` as 0, a negative one never shrinking the border box; an
///   over-constrained block's *used* `margin-right` is negative, and Chrome
///   counts the declared one). Chrome extends this in-flow area by the
///   container's end padding, and the pair of paddings on each side of the
///   frame difference cancel, so it is measured as is. Two exceptions: a
///   non-replaced `display: inline` box counts its border box alone (its
///   block-axis margins do nothing, its inline-axis ones are in its line),
///   and a grid item counts its border box alone, since Chrome counts the
///   grid's tracks, which layout does not keep (see `extend_with_child`).
/// * **Where it is drawn:** an `absolute` box's border box, an in-flow box's
///   border box at its `relative` offset, and either one's **transformed**
///   bounding box — no margins, and *not* extended by the end padding, so
///   each is measured less the container's end padding. An `absolute` box
///   counts only here: one translated back inside the padding box overflows
///   nothing, whatever its untransformed size.
///
/// Before #776/#775 every box counted its untransformed border box extended by
/// the end padding: end margins and transforms added nothing, an `absolute;
/// inset: 0` child of a padded scroller painted two bars, and a `relative`
/// box offset toward the start lost the range its static box needs.
///
/// * **Below the children** (#2054): every box under a child that overflows
///   its own parent, as Chrome counts it — its border box (no margins) where
///   it is drawn, not entered past a box that clips or is transformed; a
///   fixed box, and an absolute one contained above the container, count
///   nothing (`deep_overflow`, kept until something it read under the container
///   changes). A 300px child of a `height: 50px` child scrolls 200, as in
///   Chrome; it scrolled 0 before.
///
/// Not counted yet: a margin that collapses through a child's end (a
/// child of `height: auto` whose last child has `margin-bottom: 80px` scrolls
/// 30 in Chrome and 0 here), and a transformed child's own overflow (its
/// border box is counted, transformed).
///
/// In logical pixels — this is layout's own unit, and every caller either wants
/// it that way or scales the result itself.
///
/// The padding+border sum below is deliberately *not*
/// [`crate::paint::ifc_root_content_origin`]: that helper defines where an IFC draws
/// its inline content, and its doc contract is about anonymous roots — while
/// this one measures a scroll container's own scrollable overflow, and a
/// scroll container is never an anonymous box (an anonymous box's style
/// carries no `overflow`).
pub fn content_extents(tree: &NodeTree, node_id: RawNodeId) -> (f64, f64) {
    content_extents_with(tree, node_id, true)
}

/// `element.scrollWidth` / `scrollHeight` on rinch-dom
/// (`DomDocument::scroll_width` / `scroll_height`), in [`content_extents`]'
/// frame (from the content box's inline-start and top edges).
///
/// A scroll container answers [`content_extents`], the range its bar and the
/// wheel use, so the three cannot disagree. A box that does not scroll has
/// no range to agree with, and answers its scrollable overflow as Chrome
/// measures it there: [`content_extents`] joined with every descendant box
/// that overflows its own parent (#2223). css-overflow-3 §2.2: a box's
/// scrollable overflow holds its descendants' unless a box between clips —
/// a `260px` box holding a `260px` child holding a `460px` grandchild has a
/// `scrollWidth` of 460 in Chrome 157, where [`content_extents`], which stops
/// at the first box, answers 260. A scroll container's range counts them
/// too since #2054, kept between layouts (`DeepOverflow`).
///
/// **Cost:** the whole subtree, walked on every call (nothing is cached), for
/// a box that does not scroll; a scroll container pays [`content_extents`].
pub fn scroll_extents(tree: &NodeTree, node_id: RawNodeId) -> (f64, f64) {
    let (w, h) = content_extents(tree, node_id);
    let Some(node) = tree.get(node_id) else {
        return (w, h);
    };
    if node.scrolls_x() || node.scrolls_y() {
        return (w, h);
    }
    let cs = &node.computed_style;
    let content_left = (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64;
    let content_top = (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64;
    let mut reach = Reach::NONE;
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, node_id).iter() {
        // A direct child is [`content_extents`]' own: only what is below it.
        // An out-of-flow child contained above the box (a fixed overlay, a
        // modal's absolute layer) is not its overflow, nor is its content.
        if let Some(child) = tree.get_live(child_id)
            && crate::out_of_flow::contributes_to_scrollable_overflow(tree, node_id, child)
        {
            descend_into(tree, node_id, child, (0.0, 0.0), &mut reach, &mut None);
        }
    }
    if reach == Reach::NONE {
        return (w, h);
    }
    let deep_w = if node.computed_style.direction_rtl {
        content_left + visible_extents(tree, node_id).0 - reach.left
    } else {
        reach.right - content_left
    };
    (w.max(deep_w), h.max(reach.bottom - content_top))
}

/// [`content_extents`] without the absolute boxes listed for `node_id` from
/// below its children (#770, `NodeTree::overflow_absolutes`). They only add
/// to it, so a scroll offset inside this range is inside that one: the
/// layout's clamp asks this first, and measures them only for an offset past
/// it.
pub(crate) fn content_extents_in_flow(tree: &NodeTree, node_id: RawNodeId) -> (f64, f64) {
    content_extents_with(tree, node_id, false)
}

pub(super) fn content_extents_with(
    tree: &NodeTree,
    node_id: RawNodeId,
    listed: bool,
) -> (f64, f64) {
    let mut rounded = measure_content(tree, node_id, listed, None);
    // The descendants' overflow below the children (#2054), of a scroll
    // container: like the listed absolutes, it only adds to the range, so
    // layout's clamp asks the in-flow range first and this only past it.
    if listed
        && let Some(node) = tree.get(node_id)
        && (node.scrolls_x() || node.scrolls_y())
    {
        let deep = deep_overflow(tree, node_id);
        if deep != Reach::NONE {
            let cs = &node.computed_style;
            // Measured where it is drawn: no end padding (Chrome 158), as
            // `ContainerFrame::grow_drawn` takes it off.
            rounded.reach.grow(
                deep.left + cs.padding_left.to_px() as f64,
                deep.right - cs.padding_right.to_px() as f64,
                deep.bottom - cs.padding_bottom.to_px() as f64,
            );
        }
    }
    if let Some(node) = tree.get(node_id)
        && (node.scrolls_x() || node.scrolls_y())
        && let Some(extents) = scroller_extents(tree, node_id, node, &rounded)
    {
        return extents;
    }
    rounded.content(rounded.flow)
}

/// [`content_extents_in_flow`] from the rounded rects alone: within
/// [`ROUNDED_RANGE_SLACK`] of the whole-pixel range on either side, and
/// without the unrounded measure's walk. For a caller that only needs to know
/// an offset is well inside the range — layout's clamp, on every layout.
pub(crate) fn content_extents_in_flow_rounded(tree: &NodeTree, node_id: RawNodeId) -> (f64, f64) {
    let rounded = measure_content(tree, node_id, false, None);
    rounded.content(rounded.flow)
}

/// How far a scroll range measured from the rounded rects
/// ([`content_extents_in_flow_rounded`]) can sit from the whole-pixel range
/// [`scroller_extents`] answers: an in-flow box's rounded end is within 1px
/// of its unrounded one (1.5px before its location was rounded as an
/// absolute edge, #2437), the content box within 1px, and the range rounds
/// by half a pixel at each end.
pub(crate) const ROUNDED_RANGE_SLACK: f64 = 4.0;

/// A scroll container's content extents, with its range on each axis it
/// scrolls on **Chrome's whole-pixel range** (#2116): `scrollHeight` is the
/// scrollable overflow and `clientHeight` the padding box, each rounded to a
/// whole CSS pixel from layout units (1/64px) — wherever the box is and at any
/// device scale — and the range is their difference (measured in Chrome 156,
/// `tests/subpixel_overflow_2116_tests.rs`). So 0.4px of overflow is none and
/// 0.5px is a pixel's range.
///
/// Both are measured from Taffy's **unrounded** layout. The rounded rects
/// cannot answer it: Taffy rounds a box's edges where they are on the page,
/// not where they are in the container, so a child's rounded end can sit up to
/// a pixel from where its unrounded one is against the container's rounded
/// box (more than a pixel before #2437 rounded the location that way too) — the
/// menu bar's two-item dropdown, whose content fits exactly, measured a pixel
/// past its content box at some positions and painted a bar over it.
///
/// An axis with no overflow keeps its measure (a scroller emptier than its box
/// reports what it holds); an overflowing one is the content box plus the
/// range, so every reader of the range — the bar, the wheel
/// ([`can_scroll_toward`](super::can_scroll_toward), [`overflows`](super::overflows)), layout's clamp, `scrollHeight` —
/// answers the same. An axis that does not scroll keeps the unrounded measure.
/// `None` when the box's rounded size is not a rounding of its unrounded one
/// (a size written after layout); the rounded measure then stands.
///
/// **Cost.** Only the in-flow boxes' margin boxes are read unrounded, and that
/// walk is memoized per container for the current geometry ([`ExactFlow`]):
/// every other contribution — the container's lines, its absolute, offset and
/// transformed boxes where they are drawn — is the rounded walk's
/// (`rounded`), which the caller has made anyway. So a scroller asked again
/// with nothing moved (each paint, each pointer move) pays the rounded walk
/// and a lookup, as before #2116, and the unrounded walk once per layout.
pub(super) fn scroller_extents(
    tree: &NodeTree,
    node_id: RawNodeId,
    node: &crate::node::Node,
    rounded: &Measured,
) -> Option<(f64, f64)> {
    let taffy_id = node.taffy_id?;
    let u = tree.taffy.unrounded_layout(taffy_id);
    if (u.size.width - node.layout.width).abs() >= 1.0
        || (u.size.height - node.layout.height).abs() >= 1.0
    {
        return None;
    }
    let cs = &node.computed_style;
    let pad_h = (cs.padding_left.to_px() + cs.padding_right.to_px()) as f64;
    let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64;
    let border_h = (cs.border_left_width.to_px() + cs.border_right_width.to_px()) as f64;
    let border_v = (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64;
    let client_w = (u.size.width as f64 - border_h).max(pad_h);
    let client_h = (u.size.height as f64 - border_v).max(pad_v);
    let flow = tree.exact_flow.get(tree, node_id, || {
        measure_content(tree, node_id, false, Some(taffy_id)).flow
    });
    let (content_w, content_h) = rounded.content(flow);
    // To layout units first, as Chrome holds them, so float noise in a sum
    // that lands on a half pixel rounds the way the exact value does.
    let whole = |px: f64| ((px * 64.0).round() / 64.0).round();
    let range_x = (whole(content_w + pad_h) - whole(client_w)).max(0.0);
    let range_y = (whole(content_h + pad_v) - whole(client_h)).max(0.0);
    let (visible_w, visible_h) = visible_extents(tree, node_id);
    let settle = |scrolls: bool, measured: f64, visible: f64, range: f64| {
        if !scrolls {
            measured
        } else if range > 0.0 {
            visible + range
        } else {
            measured.min(visible)
        }
    };
    Some((
        settle(node.scrolls_x(), content_w, visible_w, range_x),
        settle(node.scrolls_y(), content_h, visible_h, range_y),
    ))
}

/// What [`measure_content`] measured, in the container's border-box frame:
/// the in-flow boxes' margin boxes (`flow`) apart from everything else
/// (`right`, `bottom`), so [`scroller_extents`] can put the unrounded `flow`
/// beside the rounded rest.
pub(crate) struct Measured {
    pub(super) reach: Reach,
    pub(super) flow: Reach,
    pub(super) content_origin: (f64, f64),
    /// The content box's right edge, in the border-box frame: where an
    /// rtl container's content is measured from ([`scrolls_from_right`](super::scrolls_from_right)).
    pub(super) content_right: f64,
    /// Whether the container's inline axis runs right to left, so its
    /// scrollable overflow is measured leftward from `content_right` (#2546).
    pub(super) rtl: bool,
}

impl Measured {
    /// The content extents with `flow` as the in-flow boxes' reach. The
    /// width is measured from the content box's inline-start edge: its left
    /// edge rightward, or — for an rtl container — its right edge leftward
    /// (#2546), so overflow past the inline-start edge is never scrollable,
    /// as in Chrome.
    fn content(&self, flow: Reach) -> (f64, f64) {
        let width = if self.rtl {
            self.content_right - self.reach.left.min(flow.left)
        } else {
            self.reach.right.max(flow.right) - self.content_origin.0
        };
        (
            width.max(0.0),
            (self.reach.bottom.max(flow.bottom) - self.content_origin.1).max(0.0),
        )
    }
}

/// How far a scroll container's content reaches, in its border-box frame:
/// its furthest right and bottom edges, and its furthest left edge, which
/// only an rtl container reads ([`Measured::content`], #2546).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Reach {
    pub(super) left: f64,
    pub(super) right: f64,
    pub(super) bottom: f64,
}

impl Reach {
    /// Nothing measured yet.
    pub(super) const NONE: Reach = Reach {
        left: f64::INFINITY,
        right: 0.0,
        bottom: 0.0,
    };

    /// Grow by a horizontal span `x0..x1` and a bottom edge `y1`.
    #[inline(always)]
    pub(super) fn grow(&mut self, x0: f64, x1: f64, y1: f64) {
        self.left = self.left.min(x0);
        self.right = self.right.max(x1);
        self.bottom = self.bottom.max(y1);
    }
}

/// Each scroll container's in-flow reach measured unrounded
/// ([`scroller_extents`], #2116), for the geometry it was measured in —
/// **keyed on `NodeTree::layout_epoch`**: the in-flow margin boxes are the
/// layout's rects and the styles that layout read (no scroll, no transform,
/// nothing a paint-only restyle changes), and every layout pass moves it. Not
/// on `HitCache::shape_generation`, which every paint-dirty push moves too:
/// a hover frame would walk every row unrounded (+3.8% on
/// `shell::hover_frame`).
#[derive(Debug, Default)]
pub(crate) struct ExactFlow {
    state: std::cell::RefCell<ExactFlowState>,
}

#[derive(Debug, Default)]
pub(super) struct ExactFlowState {
    /// The `layout_epoch` the entries were measured in.
    epoch: Option<u64>,
    /// `(scroll container, its in-flow reach)`, in the order first asked.
    entries: Vec<(RawNodeId, Reach)>,
}

impl ExactFlow {
    fn get(&self, tree: &NodeTree, id: RawNodeId, measure: impl FnOnce() -> Reach) -> Reach {
        let generation = tree.layout_epoch;
        {
            let mut state = self.state.borrow_mut();
            if state.epoch != Some(generation) {
                state.epoch = Some(generation);
                state.entries.clear();
            } else if let Some(&(_, flow)) = state.entries.iter().find(|e| e.0 == id) {
                return flow;
            }
        }
        let flow = measure();
        self.state.borrow_mut().entries.push((id, flow));
        flow
    }
}
