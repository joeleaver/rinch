//! The walk of a box's descendants below its own children (#2054, #2223):
//! what overflows them. `scroll_extents` runs it whole for a box that does
//! not scroll; a scroll container keeps it ([`deep_overflow`], recorded by
//! `deep_record` and kept by `deep_memo`). The pieces both share are here:
//! where a box places its children ([`box_origin`]), whether a box is the
//! container's overflow at all ([`walk_out`]), its lines' extent
//! ([`lines_span`]), the [`Span`] a contribution is measured in, and what a
//! kept walk reads of each box ([`WalkSnap`]).

use super::extents::Reach;
use super::measure::{is_empty_box, lines_left};
use crate::NodeTree;
use crate::computed_style::PositionValue;
use crate::node::RawNodeId;
use peniko::kurbo;

/// Grow `reach` (in `container_id`'s border-box frame) by every box below
/// `node` that `node`'s scrollable overflow holds and so passes up to the
/// container's: unless `node` clips, or is transformed (its own transformed
/// box, counted where it is, stands for it), or is out of flow
/// ([`descendant_reach`]). `origin` is `node`'s parent's border-box origin in
/// the container's frame.
pub(super) fn descend_into(
    tree: &NodeTree,
    container_id: RawNodeId,
    node: &crate::node::Node,
    origin: (f64, f64),
    reach: &mut Reach,
) {
    if WalkSnap::stops(node) {
        return;
    }
    let at = box_origin(tree, node, origin);
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, node.id).iter() {
        if let Some(child) = tree.get_live(child_id) {
            descendant_reach(tree, container_id, child, at, reach);
        }
    }
}

/// Where `node`'s children are placed from, in the frame `origin` (its
/// parent's) is in: its border-box origin, or `origin` itself for a
/// `display: contents` wrapper, which has no box and lays its children out
/// in its parent's frame.
pub(super) fn box_origin(
    tree: &NodeTree,
    node: &crate::node::Node,
    origin: (f64, f64),
) -> (f64, f64) {
    if node.taffy_style_owned_by_contents_splice() {
        return origin;
    }
    let (dx, dy) = crate::paint::ifc_content_box_offset(tree, node);
    (
        origin.0 + (node.layout.x + dx) as f64,
        origin.1 + (node.layout.y + dy) as f64,
    )
}

/// Whether [`descendant_reach`] counts nothing of `node` and does not enter
/// it as the scroll container's overflow: a fixed box, an absolute one
/// contained above the container, and an empty absolute one.
pub(super) fn walk_out(tree: &NodeTree, container_id: RawNodeId, node: &crate::node::Node) -> bool {
    !node.taffy_style_owned_by_contents_splice()
        && (!crate::out_of_flow::contributes_to_scrollable_overflow(tree, container_id, node)
            || (node.box_position() == PositionValue::Absolute && is_empty_box(node)))
}

/// The span of `node`'s lines, from its content origin, for a box whose
/// border-box origin is `(x, y)`: a `nowrap` line wider than the box
/// overflows it.
pub(super) fn lines_span(node: &crate::node::Node, x: f64, y: f64) -> Option<(f64, f64, f64)> {
    let inline = node.text_layout.as_ref()?;
    let (ox, oy) = crate::paint::ifc_root_content_origin(node);
    let (lx, ly) = (x + ox as f64, y + oy as f64);
    Some((
        lx + lines_left(&inline.layout).min(0.0),
        lx + inline.layout.width() as f64,
        ly + inline.layout.height() as f64,
    ))
}

/// Grow `reach` by `node`'s border box, at `origin` (its parent's border-box
/// origin in `container_id`'s frame), and by what overflows it: the lines
/// of its inline layout, and its own descendants ([`descend_into`]). Its
/// border box alone, not its margins: Chrome counts no margin of a box
/// below a container's own children. A fixed box, and an absolute one
/// whose containing block is above the container
/// ([`crate::out_of_flow::contributes_to_scrollable_overflow`]), are not the
/// container's overflow; an empty absolute one counts nothing; nor does a
/// box under `display: none`. A transformed box counts where it is drawn,
/// and is not entered.
pub(super) fn descendant_reach(
    tree: &NodeTree,
    container_id: RawNodeId,
    node: &crate::node::Node,
    origin: (f64, f64),
    reach: &mut Reach,
) {
    if node.is_text()
        || node.computed_style.display == crate::computed_style::DisplayValue::None
        || walk_out(tree, container_id, node)
    {
        return;
    }
    if node.taffy_style_owned_by_contents_splice() {
        descend_into(tree, container_id, node, origin, reach);
        return;
    }
    let (x, y) = box_origin(tree, node, origin);
    let (w, h) = (node.layout.width as f64, node.layout.height as f64);
    if node.has_applied_transform() {
        let drawn = crate::paint::compose_node_transform(node, x, y, 1.0, kurbo::Affine::IDENTITY)
            .transform_rect_bbox(kurbo::Rect::new(x, y, x + w, y + h));
        if drawn.width() > 0.0 || drawn.height() > 0.0 {
            reach.grow(drawn.x0, drawn.x1, drawn.y1);
        }
        return;
    }
    if w > 0.0 || h > 0.0 {
        reach.grow(x, x + w, y + h);
    }
    if !WalkSnap::stops(node) {
        if let Some((x0, x1, y1)) = lines_span(node, x, y) {
            reach.grow(x0, x1, y1);
        }
        descend_into(tree, container_id, node, origin, reach);
    }
}

/// A box's contributions' extent on the edges a [`Reach`] keeps: its
/// furthest left and right, and its furthest bottom. Empty until grown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Span {
    pub(super) left: f64,
    pub(super) right: f64,
    pub(super) bottom: f64,
}

impl Span {
    pub(super) const EMPTY: Span = Span {
        left: f64::INFINITY,
        right: f64::NEG_INFINITY,
        bottom: f64::NEG_INFINITY,
    };

    pub(super) fn grow(&mut self, x0: f64, x1: f64, y1: f64) {
        self.left = self.left.min(x0);
        self.right = self.right.max(x1);
        self.bottom = self.bottom.max(y1);
    }

    pub(super) fn join(&mut self, other: Span) {
        if other != Span::EMPTY {
            self.grow(other.left, other.right, other.bottom);
        }
    }

    /// Moved by `(dx, dy)`; exactly itself when that is `(0, 0)`.
    pub(super) fn moved(self, (dx, dy): (f64, f64)) -> Span {
        if self == Span::EMPTY || (dx == 0.0 && dy == 0.0) {
            return self;
        }
        Span {
            left: self.left + dx,
            right: self.right + dx,
            bottom: self.bottom + dy,
        }
    }

    /// Whether `sum`, a join of spans with `self` among them, may lose an
    /// edge when `self` is replaced by `new`: `self` reaches one of its
    /// edges, and `new` does not reach as far there. Where it does not, the
    /// new sum is `sum` joined with `new`.
    pub(super) fn holds_an_edge_of(&self, sum: &Span, new: &Span) -> bool {
        (self.left <= sum.left && new.left > self.left)
            || (self.right >= sum.right && new.right < self.right)
            || (self.bottom >= sum.bottom && new.bottom < self.bottom)
    }

    /// Whether a [`Reach`] grown by it is grown by nothing past `rect`'s:
    /// its left, right and bottom edges are inside them.
    pub(super) fn inside(&self, rect: &kurbo::Rect) -> bool {
        *self == Span::EMPTY
            || (self.left >= rect.x0 && self.right <= rect.x1 && self.bottom <= rect.y1)
    }
}

/// Everything the walk reads of one box: where it is and how big, where
/// what it lays out from its content box starts, its lines' extent, and
/// each property that decides whether the walk counts it and goes into it
/// — or whether it counts a box below it ([`walk_out`]). A kept walk
/// records one per box it met (`deep_record::Rec`).
///
/// A box's own `layout` does not say where its content is: its lines, and
/// the atomic inlines its inline formatting context places (whose `layout`
/// is relative to its content box), start one padding+border in
/// ([`crate::paint::ifc_root_content_origin`]). Padding or a border changes that
/// without moving or resizing the box, and without moving the boxes placed
/// from it in their own `layout`, so the snapshot carries it
/// (`content_origin`), and each box carries the offset the walk added to
/// its own origin for it (`inline_offset`, [`box_origin`]).
///
/// Its fields fall in two parts ([`Self::same_kind`]): where the box is,
/// how big, and its lines — what a layout moves, and what a kept walk
/// re-measures in place — and everything else, which decides what the walk
/// does with the box and below it, and whose change walks the container
/// again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WalkSnap {
    pub(super) layout: crate::node::LayoutResult,
    /// What [`box_origin`] adds to its layout origin: its IFC root's content
    /// origin, for an atomic inline ([`crate::paint::ifc_content_box_offset`]).
    pub(super) inline_offset: (f32, f32),
    /// Its own content origin, from its border box: what its lines and the
    /// atomic inlines of its inline formatting context are placed from.
    pub(super) content_origin: (f32, f32),
    /// Its lines' `(width, height, leftmost start)`, for an IFC root.
    pub(super) lines: Option<(f32, f32, f32)>,
    /// How it takes part in its formatting context: whether it is an atomic
    /// inline decides where its children are placed from and whether it
    /// holds them as a top box ([`WalkSnap::containing_rect`]).
    pub(super) mode: crate::node::DisplayMode,
    pub(super) clips: bool,
    pub(super) transformed: bool,
    pub(super) position: PositionValue,
    pub(super) display: crate::computed_style::DisplayValue,
    pub(super) contents: bool,
    pub(super) collapsed: bool,
    /// Whether it is the containing block of an absolute, and of a fixed,
    /// box below it: what decides whether such a box is the container's
    /// overflow ([`crate::out_of_flow::contributes_to_scrollable_overflow`]).
    /// A `filter` makes one with no change to anything else recorded here.
    pub(super) holds_abs: bool,
    pub(super) holds_fixed: bool,
    /// A fixed box contained by a box below the viewport, which is judged as
    /// an absolute one.
    pub(super) fixed_contained: bool,
    /// Not the container's overflow (fixed, contained above, empty absolute).
    pub(super) out: bool,
}

impl WalkSnap {
    pub(super) fn of(tree: &NodeTree, node: &crate::node::Node, out: bool) -> Self {
        Self {
            layout: node.layout,
            inline_offset: crate::paint::ifc_content_box_offset(tree, node),
            content_origin: crate::paint::ifc_root_content_origin(node),
            lines: node.text_layout.as_ref().map(|t| {
                (
                    t.layout.width(),
                    t.layout.height(),
                    lines_left(&t.layout) as f32,
                )
            }),
            mode: node.display_mode,
            clips: node.clips_overflow(),
            transformed: node.has_applied_transform(),
            position: node.box_position(),
            display: node.computed_style.display,
            contents: node.taffy_style_owned_by_contents_splice(),
            collapsed: node.estimated_height.is_some(),
            holds_abs: node.establishes_abs_containing_block(),
            holds_fixed: node.establishes_fixed_containing_block(),
            fixed_contained: node.fixed_contained,
            out,
        }
    }

    /// Whether `self` and `other` differ at most in where the box is, how
    /// big, its lines, and whether it is out (which the walk judges from its
    /// ancestors and its size): everything that decides what the walk does
    /// with the box and below it is the same.
    ///
    /// Every field is named (no `..`): a field added to the snap is a compile
    /// error here until it is classed as one or the other.
    pub(super) fn same_kind(&self, other: &WalkSnap) -> bool {
        let WalkSnap {
            layout: _,
            inline_offset: _,
            lines: _,
            out: _,
            content_origin,
            mode,
            clips,
            transformed,
            position,
            display,
            contents,
            collapsed,
            holds_abs,
            holds_fixed,
            fixed_contained,
        } = *self;
        clips == other.clips
            && transformed == other.transformed
            && contents == other.contents
            && collapsed == other.collapsed
            && holds_abs == other.holds_abs
            && holds_fixed == other.holds_fixed
            && fixed_contained == other.fixed_contained
            && position == other.position
            && display == other.display
            && mode == other.mode
            && content_origin == other.content_origin
    }

    /// Whether the walk stops at `node`: it clips, is transformed, or is a
    /// block virtualization collapsed to an estimated height (the estimate
    /// stands for its content, whose boxes are not laid out to it).
    pub(super) fn stops(node: &crate::node::Node) -> bool {
        node.is_text()
            || node.clips_overflow()
            || node.has_applied_transform()
            || node.estimated_height.is_some()
            || node.computed_style.display == crate::computed_style::DisplayValue::None
    }

    /// Whether the walk goes into this box: it is the container's overflow,
    /// displayed, and neither clips, is transformed nor is collapsed.
    pub(super) fn entered(&self) -> bool {
        !(self.out
            || self.clips
            || self.transformed
            || self.collapsed
            || self.display == crate::computed_style::DisplayValue::None)
    }

    /// Where its children are placed from, in its parent's children frame
    /// ([`box_origin`]): nowhere else for a `display: contents` wrapper.
    pub(super) fn offset(&self) -> (f64, f64) {
        if self.contents {
            (0.0, 0.0)
        } else {
            (
                (self.layout.x + self.inline_offset.0) as f64,
                (self.layout.y + self.inline_offset.1) as f64,
            )
        }
    }

    /// Its lines' span from its own border-box origin ([`lines_span`] at
    /// `(0, 0)`), or empty.
    pub(super) fn lines_span(&self) -> Span {
        let mut span = Span::EMPTY;
        if let Some((w, h, left)) = self.lines {
            let (ox, oy) = (self.content_origin.0 as f64, self.content_origin.1 as f64);
            span.grow(ox + (left as f64).min(0.0), ox + w as f64, oy + h as f64);
        }
        span
    }

    /// The rect it holds its descendants in as a top box, for a kept walk:
    /// its border box, at `(x, y)` in the container's frame — or `None`,
    /// holding nothing, where the container's own measure may not count that
    /// box (an empty one, an atomic inline, whose children are placed from
    /// its IFC root's content box, or a transformed one).
    ///
    /// What a top box holds adds nothing to the range: the container's own
    /// measure counts that box at least as far
    /// ([`extend_with_child`](super::measure::extend_with_child): its margin
    /// box, or its border box where it is drawn, with the end padding the
    /// deep reach takes off).
    pub(super) fn containing_rect(&self, (x, y): (f64, f64)) -> Option<kurbo::Rect> {
        let (w, h) = (self.layout.width as f64, self.layout.height as f64);
        (w > 0.0 && h > 0.0 && !self.mode.is_atomic_inline() && !self.transformed)
            .then(|| kurbo::Rect::new(x, y, x + w, y + h))
    }
}

/// A scroll container's descendants' scrollable overflow below its own
/// children (#2054), in its border-box frame, as kept by the memo
/// ([`DeepOverflow`](super::DeepOverflow)): for each top box — the first box
/// below the container, through `display: contents` wrappers — everything
/// below it, unless all of that is inside the top box's border box
/// ([`WalkSnap::containing_rect`]); and each transformed box below a top box, measured
/// where it is drawn now.
///
/// css-overflow-3 §2.2: a box's scrollable overflow holds its descendants'
/// unless a box between clips. Chrome 158 scrolls a `200x100` scroller to a
/// `300px` box inside a `height: 50px` child; [`extend_with_child`](super::measure::extend_with_child) stops at
/// the child, so the range was 0.
///
/// **Cost.** A walk of the container's whole subtree down to the boxes that
/// clip, the first time; then it is kept, with a record of every box met,
/// and each change re-measures only the boxes it reached and the boxes above
/// them ([`DeepOverflow`](super::DeepOverflow)): a read in between (each paint
/// of its bar, each wheel event, each pointer move over it) is a lookup, a
/// check of the boxes pushed paint-dirty since the last read, and a measure
/// of the transformed boxes it found, whose transform may change with no
/// layout.
pub(super) fn deep_overflow(tree: &NodeTree, container_id: RawNodeId) -> Reach {
    tree.deep_overflow.reach(tree, container_id)
}
