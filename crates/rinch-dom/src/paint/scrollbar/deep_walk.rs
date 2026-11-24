//! The walk of a scroll container's descendants below its own children
//! (#2054): what overflows them, and, for a kept walk, what it read of every
//! box it met ([`WalkSnap`]) and what it found below it ([`Rec`]).

use super::deep_memo::DeepEntry;
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
/// the container's frame. With `keep`, the walk is recorded for a caller
/// that keeps it ([`DeepOverflow`](super::DeepOverflow)): each transformed box is listed instead
/// of measured, every box met is noted with what the walk read of it, and a
/// box inside its top box's border box is not counted ([`Keep::top`]).
/// Returns everything found below `node`, counted or not.
pub(super) fn descend_into(
    tree: &NodeTree,
    container_id: RawNodeId,
    node: &crate::node::Node,
    origin: (f64, f64),
    reach: &mut Reach,
    keep: &mut Option<Keep>,
) -> Found {
    let mut found = Found::NONE;
    if WalkSnap::stops(node) {
        return found;
    }
    let at = box_origin(tree, node, origin);
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, node.id).iter() {
        if let Some(child) = tree.get_live(child_id) {
            found.join(descendant_reach(tree, container_id, child, at, reach, keep));
        }
    }
    found
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
/// and is not entered; with `keep` it is listed there to be measured by the
/// caller. Returns what it found, counted or not.
pub(super) fn descendant_reach(
    tree: &NodeTree,
    container_id: RawNodeId,
    node: &crate::node::Node,
    origin: (f64, f64),
    reach: &mut Reach,
    keep: &mut Option<Keep>,
) -> Found {
    if node.is_text() {
        return Found::NONE;
    }
    let out = walk_out(tree, container_id, node);
    if let Some(keep) = keep {
        keep.snaps
            .insert(node.id, Rec::new(WalkSnap::of(tree, node, out), origin));
    }
    if node.computed_style.display == crate::computed_style::DisplayValue::None || out {
        return Found::NONE;
    }
    if node.taffy_style_owned_by_contents_splice() {
        let found = descend_into(tree, container_id, node, origin, reach, keep);
        Keep::note(keep, node.id, origin, found.span, found.counted);
        return found;
    }
    let (x, y) = box_origin(tree, node, origin);
    let (w, h) = (node.layout.width as f64, node.layout.height as f64);
    if node.has_applied_transform() {
        if let Some(keep) = keep {
            keep.live.push(node.id);
            return Found::NONE;
        }
        let drawn = crate::paint::compose_node_transform(node, x, y, 1.0, kurbo::Affine::IDENTITY)
            .transform_rect_bbox(kurbo::Rect::new(x, y, x + w, y + h));
        if drawn.width() > 0.0 || drawn.height() > 0.0 {
            reach.grow(drawn.x0, drawn.x1, drawn.y1);
        }
        return Found::NONE;
    }
    // The first box below the container, through `display: contents`
    // wrappers: what is inside it is not counted ([`Keep::top`]).
    let top = keep.as_mut().filter(|k| k.top.is_none()).map(|k| {
        k.top = Some(containing_rect(node, x, y, w, h));
    });
    let mut found = Found::NONE;
    if w > 0.0 || h > 0.0 {
        found.count(reach, keep, x, x + w, y + h);
    }
    let mut below = Found::NONE;
    if !WalkSnap::stops(node) {
        if let Some((x0, x1, y1)) = lines_span(node, x, y) {
            below.count(reach, keep, x0, x1, y1);
        }
        below.join(descend_into(tree, container_id, node, origin, reach, keep));
    }
    found.join(below);
    Keep::note(keep, node.id, (x, y), below.span, found.counted);
    if top.is_some()
        && let Some(keep) = keep
    {
        keep.top = None;
    }
    found
}

/// The rect a top box holds its descendants in, for a kept walk
/// ([`Keep::top`]): its border box, at `(x, y)` in the container's frame —
/// or `None`, holding nothing, where the container's own measure may not
/// count that box (an empty one, an atomic inline, whose children are
/// placed from its IFC root's content box, or a transformed one).
pub(super) fn containing_rect(
    node: &crate::node::Node,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Option<kurbo::Rect> {
    (w > 0.0 && h > 0.0 && !node.display_mode.is_atomic_inline() && !node.has_applied_transform())
        .then(|| kurbo::Rect::new(x, y, x + w, y + h))
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

    /// Whether a [`Reach`] grown by it is grown by nothing past `rect`'s:
    /// its left, right and bottom edges are inside them.
    pub(super) fn inside(&self, rect: &kurbo::Rect) -> bool {
        *self == Span::EMPTY
            || (self.left >= rect.x0 && self.right <= rect.x1 && self.bottom <= rect.y1)
    }
}

/// What a walk found below one box ([`descendant_reach`]): the extent of
/// every contribution, counted or not, and whether any was counted.
#[derive(Debug, Clone, Copy)]
pub(super) struct Found {
    span: Span,
    counted: bool,
}

impl Found {
    const NONE: Found = Found {
        span: Span::EMPTY,
        counted: false,
    };

    fn join(&mut self, other: Found) {
        self.span.join(other.span);
        self.counted |= other.counted;
    }

    /// One contribution: counted in `reach` unless a kept walk's top box
    /// holds it ([`Keep::top`]).
    fn count(&mut self, reach: &mut Reach, keep: &Option<Keep>, x0: f64, x1: f64, y1: f64) {
        self.span.grow(x0, x1, y1);
        if let Some(Keep {
            top: Some(Some(rect)),
            ..
        }) = keep
            && (Span {
                left: x0,
                right: x1,
                bottom: y1,
            })
            .inside(rect)
        {
            return;
        }
        self.counted = true;
        reach.grow(x0, x1, y1);
    }
}

/// What a kept walk recorded beside its reach ([`walk_deep_overflow`]).
#[derive(Debug, Default)]
pub(super) struct Keep {
    /// The transformed boxes met, not entered.
    live: Vec<RawNodeId>,
    /// Every box met, with what the walk read of it.
    snaps: Snaps,
    /// The border box of the first box below the container the walk is in
    /// (through `display: contents` wrappers), when it holds what is inside
    /// it ([`containing_rect`]); `None` above it. A contribution inside it is
    /// not counted: the container's own measure counts that box at least as
    /// far ([`extend_with_child`](super::measure::extend_with_child): its margin box, or its border box where it
    /// is drawn, with the end padding the deep reach takes off), so it adds
    /// nothing to the range — and a change inside it, which is most of what
    /// a layout changes, leaves the kept walk standing ([`DeepOverflow`](super::DeepOverflow)).
    top: Option<Option<kurbo::Rect>>,
}

impl Keep {
    /// Record what the walk found below box `id`, whose children are placed
    /// from `at`.
    fn note(keep: &mut Option<Keep>, id: RawNodeId, at: (f64, f64), below: Span, counted: bool) {
        if let Some(rec) = keep.as_mut().and_then(|k| k.snaps.get_mut(&id)) {
            rec.at = at;
            rec.below = below;
            rec.counted = counted;
        }
    }
}

/// What a kept walk read of every box it met, by id. A walk inserts one
/// record per box of the container's subtree, so the map is keyed with
/// FxHash: with the std `SipHash` the inserts were most of a walk's cost
/// (a walk of 4,000 boxes, `row_shift_damage` in `rinch-bench`). Node ids
/// come from the tree's slab, not from outside, so there is nothing to
/// guard against collisions for.
pub(super) type Snaps = rustc_hash::FxHashMap<RawNodeId, Rec>;

/// One box a kept walk met ([`DeepEntry::snaps`]): what it read of it, and
/// what it found below it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Rec {
    pub(super) snap: WalkSnap,
    /// Where its children are placed from, in the container's frame, when
    /// `below` was found (moved with it since, [`DeepOverflow`](super::DeepOverflow)).
    pub(super) at: (f64, f64),
    /// Everything below it — its lines and its descendants' contributions,
    /// counted or not — in the container's frame as it was at `at`.
    pub(super) below: Span,
    /// Whether anything of it or below it was counted in the reach.
    pub(super) counted: bool,
}

impl Rec {
    pub(super) fn new(snap: WalkSnap, at: (f64, f64)) -> Self {
        Self {
            snap,
            at,
            below: Span::EMPTY,
            counted: false,
        }
    }
}

/// Everything [`descendant_reach`] reads of one box: where it is and how big,
/// where what it lays out from its content box starts, its lines' extent,
/// and each property that decides whether the walk counts it and goes into
/// it — or whether it counts a box below it ([`walk_out`]). A kept walk is
/// stale exactly when one of these differs for a box it met, or a box
/// appears under one it entered ([`DeepOverflow`](super::DeepOverflow)).
///
/// A box's own `layout` does not say where its content is: its lines, and
/// the atomic inlines its inline formatting context places (whose `layout`
/// is relative to its content box), start one padding+border in
/// ([`crate::paint::ifc_root_content_origin`]). Padding or a border changes that
/// without moving or resizing the box, and without moving the boxes placed
/// from it in their own `layout`, so the snapshot carries it
/// (`content_origin`), and each box carries the offset the walk added to
/// its own origin for it (`inline_offset`, [`box_origin`]).
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
    /// holds them as a top box ([`containing_rect`]).
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

    /// Whether nothing below this box can reach the walk.
    pub(super) fn is_stop(&self) -> bool {
        self.out
            || self.clips
            || self.transformed
            || self.collapsed
            || self.display == crate::computed_style::DisplayValue::None
    }
}

/// A scroll container's descendants' scrollable overflow below its own
/// children (#2054), in its border-box frame: what [`descendant_reach`]
/// finds under each child that contributes, the transformed boxes measured
/// where they are drawn now.
///
/// css-overflow-3 §2.2: a box's scrollable overflow holds its descendants'
/// unless a box between clips. Chrome 158 scrolls a `200x100` scroller to a
/// `300px` box inside a `height: 50px` child; [`extend_with_child`](super::measure::extend_with_child) stops at
/// the child, so the range was 0.
///
/// What is inside the border box of the first box below the container is
/// not counted ([`Keep::top`]): the container's own measure counts that box
/// at least as far.
///
/// **Cost.** The walk is the container's whole subtree down to the boxes
/// that clip, so it is kept, per container, until something it read under
/// the container changes where it could move the reach ([`DeepOverflow`](super::DeepOverflow)): a read in between (each paint
/// of its bar, each wheel event, each pointer move over it) is a lookup, a
/// check of the boxes pushed paint-dirty since the last read, and a measure
/// of the transformed boxes it found, whose transform may change with no
/// layout.
pub(super) fn deep_overflow(tree: &NodeTree, container_id: RawNodeId) -> Reach {
    tree.deep_overflow.reach(tree, container_id)
}

/// The walk [`deep_overflow`] keeps: the static part, the transformed boxes
/// found (measured on each read), and what it read of every box it met,
/// recorded into `snaps` (empty: the map of an entry dropped before, kept
/// for its capacity, so a walk of a subtree as big as last time's grows no
/// table).
pub(super) fn walk_deep_overflow(
    tree: &NodeTree,
    container_id: RawNodeId,
    snaps: Snaps,
) -> DeepEntry {
    debug_assert!(snaps.is_empty());
    let mut reach = Reach::NONE;
    let mut keep = Some(Keep {
        snaps,
        ..Keep::default()
    });
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, container_id).iter() {
        let Some(child) = tree.get_live(child_id) else {
            continue;
        };
        let enters = child.taffy_style_owned_by_contents_splice()
            || crate::out_of_flow::contributes_to_scrollable_overflow(tree, container_id, child);
        if let Some(keep) = &mut keep
            && !child.is_text()
        {
            keep.snaps.insert(
                child_id,
                Rec::new(WalkSnap::of(tree, child, !enters), (0.0, 0.0)),
            );
        }
        if enters {
            // A box child is its descendants' top box; a `display: contents`
            // one's children are theirs ([`descendant_reach`]).
            let at = box_origin(tree, child, (0.0, 0.0));
            if let Some(keep) = &mut keep
                && !child.taffy_style_owned_by_contents_splice()
            {
                let (w, h) = (child.layout.width as f64, child.layout.height as f64);
                keep.top = Some(containing_rect(child, at.0, at.1, w, h));
            }
            let found = descend_into(tree, container_id, child, (0.0, 0.0), &mut reach, &mut keep);
            Keep::note(&mut keep, child_id, at, found.span, found.counted);
            if let Some(keep) = &mut keep {
                keep.top = None;
            }
        }
    }
    let keep = keep.unwrap_or_default();
    DeepEntry {
        reach,
        live: keep.live,
        snaps: keep.snaps,
    }
}
