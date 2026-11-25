//! Measuring a container's content from its children: each child's box where
//! it is laid out or drawn ([`extend_with_child`]), its listed absolute
//! boxes, and the frame the measure runs in ([`ContainerFrame`]).

use super::extents::{Measured, Reach};
use super::visible_extents;
use crate::NodeTree;
use crate::computed_style::PositionValue;
use crate::node::DisplayMode;
use crate::node::RawNodeId;
use peniko::kurbo;

/// [`content_extents_with`](super::extents::content_extents_with)'s walk. With `unrounded` (the container's Taffy
/// node), every box Taffy laid out directly in the container is read at its
/// unrounded rect ([`child_rect`]); every other box at its rounded one.
pub(super) fn measure_content(
    tree: &NodeTree,
    node_id: RawNodeId,
    listed: bool,
    unrounded: Option<taffy::NodeId>,
) -> Measured {
    let Some(node) = tree.get(node_id) else {
        return Measured {
            reach: Reach::NONE,
            flow: Reach::NONE,
            content_origin: (0.0, 0.0),
            content_right: 0.0,
            rtl: false,
        };
    };
    let cs = &node.computed_style;
    let content_left = (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64;
    let content_top = (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64;
    let rtl = node.computed_style.direction_rtl;
    let mut reach = Reach::NONE;
    // The container's own inline content, when it is an IFC root: every line
    // its inline layout holds, wherever in the DOM the text sits — directly in
    // it, in a `display: contents` wrapper (every reactive `{|| text}` is one),
    // or in an inline `<span>` — since none of those owns a box that measures
    // it. The IFC draws at the content origin (`ifc_root_content_origin`), so
    // the layout's own size is already in the content frame. Read from the
    // inline layout rather than from its text children's boxes: a text child
    // under a wrapper or a span is given no box at all, and a direct one is
    // given the root's *border*-box width at the border-box origin, which
    // over-reported a padded container's width by its horizontal padding and
    // its height by its top padding.
    let inline_measured = if let Some(inline) = &node.text_layout {
        reach.right = content_left + inline.layout.width() as f64;
        reach.bottom = content_top + inline.layout.height() as f64;
        if rtl {
            reach.left = content_left + lines_left(&inline.layout);
        }
        true
    } else {
        false
    };
    // A `<textarea>`'s own text (#827): its value is an attribute, so no box
    // and no inline layout holds it. Its height as last shaped for its
    // scroll, at the content origin like an IFC's lines; it wraps at the
    // column, so it adds no width.
    if let Some(text_h) = crate::form_control::textarea_scroll_extent(node) {
        reach.bottom = reach.bottom.max(content_top + text_h as f64);
    }
    let frame = ContainerFrame {
        end_padding: (
            cs.padding_right.to_px() as f64,
            cs.padding_bottom.to_px() as f64,
        ),
        padding_left: cs.padding_left.to_px() as f64,
        rtl,
        content_box: visible_extents(tree, node_id),
        inline_measured,
        unrounded: unrounded.map(|id| (id, taffy_origin(&tree.taffy, id))),
        flow: std::cell::Cell::new(Reach::NONE),
        grid: matches!(
            cs.display,
            crate::computed_style::DisplayValue::Grid
                | crate::computed_style::DisplayValue::InlineGrid
        ),
    };
    // Two monomorphs: the rounded walk every call makes compiles as it did
    // before #2116, with no unrounded branch per box.
    if frame.unrounded.is_some() {
        extend_with_children::<true>(tree, node_id, node_id, &frame, &mut reach);
    } else {
        extend_with_children::<false>(tree, node_id, node_id, &frame, &mut reach);
    }
    if listed {
        extend_with_overflow_absolutes(tree, node_id, &frame, &mut reach);
    }
    Measured {
        reach,
        flow: frame.flow.get(),
        content_origin: (content_left, content_top),
        content_right: content_left + frame.content_box.0,
        rtl,
    }
}

/// Grow `right`/`bottom` (in the scroll container's border-box frame) by every
/// box among `parent_id`'s children that is `container_id`'s scrollable
/// content, walking **through** `display: contents` children (issue #396).
///
/// The children are the **box tree's**
/// ([`crate::RinchDocument::box_tree_children`]), not the element tree's
/// (issue #995). A container holding text beside a block child lays each run
/// of its inline content out in an anonymous block box, and a split inline
/// (#513) lays each fragment out in one; neither box is in any `children`
/// list, and the text nodes they lay out carry no box, so a walk of
/// `children` stopped at the last element box and every line after it was out
/// of reach of the wheel and the bar. The same list holds an out-of-flow box
/// hoisted out of a flowed inline element into its host (#591), which the
/// element walk never reached either: it saw only the inline's `0x0` box.
///
/// A `display: contents` element generates no box. Its children are laid out
/// in its parent's formatting context — `sync_display_contents` splices them
/// into the parent's Taffy child list — so their `layout` rects are already in
/// the container's frame, exactly as a direct child's are, and they are the
/// container's content as if the wrapper were not there (measured in Chrome:
/// `tests/contents_scroll_extent_tests.rs`). The wrapper's own `0x0` rect is
/// not a box and is not measured. Since `rsx!` emits one such wrapper per
/// reactive text, `if`, `match`, `for` and embedded `Vec<NodeHandle>`, reading
/// that rect instead made every `for`-built scroll region unscrollable.
///
/// The containing-block rule is asked of **the scroll container**, for every
/// box reached through a wrapper too: a box that is not generated contains
/// nothing, so a wrapper never stands between a flattened absolute and the
/// container.
///
/// With `frame.inline_measured`, the container's own inline layout has already been
/// measured, so a text node it lays out is skipped rather than read through
/// the proxy box `write_inline_positions` gives a direct one.
pub(super) fn extend_with_children<const UNROUNDED: bool>(
    tree: &NodeTree,
    container_id: RawNodeId,
    parent_id: RawNodeId,
    frame: &ContainerFrame,
    reach: &mut Reach,
) {
    let Some(parent) = tree.get_live(parent_id) else {
        return;
    };
    // **The common case walks `children` once.** Where `parent` holds no
    // anonymous box and hosts no hoisted box, its box-tree children are its
    // `children` unless one of them is a split inline or a hoisted box — the
    // per-child test `box_tree_children` makes before it answers. Asking it
    // here instead folds that test into this loop rather than paying a second
    // pass over every row of a long list per call (measured +37% per call on
    // 500 rows with the second pass). On the first child that needs the box
    // tree, the walk takes the box tree whole: every child measured so far is
    // in it too, and a max over the same rect twice is the same max.
    if parent.run_boxes.is_empty()
        && !parent.hosts_hoisted_out_of_flow
        && !parent.is_anonymous_block_box
    {
        for &child_id in &parent.children {
            let Some(child) = tree.get_live(child_id) else {
                continue;
            };
            if child.is_split_inline()
                || child.hosts_hoisted_out_of_flow
                || child.hoisted_out_of_flow_to.is_some()
            {
                extend_with_box_tree_children::<UNROUNDED>(
                    tree,
                    container_id,
                    parent_id,
                    frame,
                    reach,
                );
                return;
            }
            extend_with_child::<UNROUNDED>(tree, container_id, child_id, child, frame, reach);
        }
        return;
    }
    extend_with_box_tree_children::<UNROUNDED>(tree, container_id, parent_id, frame, reach);
}

/// [`extend_with_children`] over `parent_id`'s box-tree children.
pub(super) fn extend_with_box_tree_children<const UNROUNDED: bool>(
    tree: &NodeTree,
    container_id: RawNodeId,
    parent_id: RawNodeId,
    frame: &ContainerFrame,
    reach: &mut Reach,
) {
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, parent_id).iter() {
        let Some(child) = tree.get_live(child_id) else {
            continue;
        };
        extend_with_child::<UNROUNDED>(tree, container_id, child_id, child, frame, reach);
    }
}

/// Grow `right`/`bottom` by every absolute box whose scrollable overflow is
/// `container_id`'s but which is laid out in another box (issue #770): one
/// under a box between it and the container, or one whose containing block
/// is an ancestor of the box it is laid out in.
///
/// Each counts as an `absolute` child does in [`extend_with_child`]: its
/// border box where it is drawn, transformed, less the container's end
/// padding — here in the container's border-box frame through every box
/// between them.
///
/// The layout read-back lists the absolute boxes
/// (`NodeTree::overflow_absolutes`), and [`OverflowAbsolutes`](super::OverflowAbsolutes) measures them
/// once per geometry: so this is one binary search, plus a walk for each box
/// with a transform on its way, and a document with none pays an `is_empty`.
pub(super) fn extend_with_overflow_absolutes(
    tree: &NodeTree,
    container_id: RawNodeId,
    frame: &ContainerFrame,
    reach: &mut Reach,
) {
    if tree.overflow_absolutes.listed.is_empty() {
        return;
    }
    let state = tree.overflow_absolutes.current(tree);
    let measured = &state.measured;
    if let Ok(i) = measured.binary_search_by_key(&container_id, |m| m.0) {
        let (_, x0, x1, y1) = measured[i];
        frame.grow_drawn(reach, x0, x1, y1);
    }
    let live = &state.live;
    if live.is_empty() {
        return;
    }
    let first = live.partition_point(|&(scroller, _)| scroller < container_id);
    for &(scroller, abs_id) in &live[first..] {
        if scroller != container_id {
            break;
        }
        // Freed, or no longer an absolute box with an area, since the layout
        // that listed it (`extend_with_child` skips an empty one too).
        if !tree
            .get(abs_id)
            .is_some_and(|n| n.box_position() == PositionValue::Absolute && !is_empty_box(n))
        {
            continue;
        }
        let Some(drawn) = border_box_in(tree, abs_id, container_id) else {
            continue;
        };
        frame.grow_drawn(reach, drawn.x0, drawn.x1, drawn.y1);
    }
}

/// Whether `node`'s border box has no area — no width or no height.
pub(super) fn is_empty_box(node: &crate::node::Node) -> bool {
    node.layout.width <= 0.0 || node.layout.height <= 0.0
}

/// `node_id`'s border box where it is drawn, in the border-box frame of its
/// box-tree ancestor `ancestor_id` with that ancestor unscrolled: its layout
/// rect moved by the layout offset of every box between them and by the
/// scroll of each, and transformed by each transform on the way (the box's
/// own included), composed as paint composes them
/// ([`crate::paint::compose_node_transform`]). `None` when `ancestor_id` is not a
/// box-tree ancestor of the node.
///
/// The scrolls are taken off because a box whose containing block is above a
/// scroller does not move with it: `out_of_flow::replace_after_scroll` writes
/// that scroll into the box's `layout`, so the sum less the scrolls is where
/// it is drawn, whatever any scroller between them is scrolled to.
pub(super) fn border_box_in(
    tree: &NodeTree,
    node_id: RawNodeId,
    ancestor_id: RawNodeId,
) -> Option<kurbo::Rect> {
    let node = tree.get(node_id)?;
    // The common case: nothing on the way is transformed, so the offsets add.
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    let mut transformed = false;
    let mut current = Some(node_id);
    loop {
        let id = current?;
        if id == ancestor_id {
            break;
        }
        let n = tree.get_live(id)?;
        transformed |= n.has_applied_transform();
        let (dx, dy) = crate::paint::ifc_content_box_offset(tree, n);
        x += (n.layout.x + dx) as f64;
        y += (n.layout.y + dy) as f64;
        if id != node_id {
            x -= n.scroll_offset.0;
            y -= n.scroll_offset.1;
        }
        current = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    let (w, h) = (node.layout.width as f64, node.layout.height as f64);
    if !transformed {
        return Some(kurbo::Rect::new(x, y, x + w, y + h));
    }
    // A transform on the way: compose top-down, as paint does.
    let mut chain = Vec::new();
    let mut current = Some(node_id);
    while let Some(id) = current {
        if id == ancestor_id {
            break;
        }
        chain.push(id);
        current = crate::RinchDocument::box_tree_parent(&tree.nodes, id);
    }
    let (mut off_x, mut off_y) = (0.0_f64, 0.0_f64);
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    let mut transform = kurbo::Affine::IDENTITY;
    for &id in chain.iter().rev() {
        let n = tree.get_live(id)?;
        let (dx, dy) = crate::paint::ifc_content_box_offset(tree, n);
        x = off_x + (n.layout.x + dx) as f64;
        y = off_y + (n.layout.y + dy) as f64;
        transform = crate::paint::compose_node_transform(n, x, y, 1.0, transform);
        off_x = x - n.scroll_offset.0;
        off_y = y - n.scroll_offset.1;
    }
    Some(transform.transform_rect_bbox(kurbo::Rect::new(x, y, x + w, y + h)))
}

/// What [`extend_with_child`] needs of the scroll container itself.
pub(super) struct ContainerFrame {
    /// Its `(padding-right, padding-bottom)`: what Chrome extends the in-flow
    /// area by and nothing else, so a box measured where it is drawn gives it
    /// back.
    end_padding: (f64, f64),
    /// Its `padding-left`: an rtl container's inline-end padding, which
    /// plays `end_padding.0`'s part for its leftward reach (#2546).
    padding_left: f64,
    /// Whether its inline axis runs right to left ([`Measured::rtl`]).
    rtl: bool,
    /// Its content box `(width, height)` — the containing block a child's
    /// percentage margins and `relative` offsets resolve against.
    content_box: (f64, f64),
    /// Whether the container's own inline layout is already measured, so a
    /// text node it lays out is skipped (see [`extend_with_children`]).
    inline_measured: bool,
    /// Its Taffy node, when the walk reads boxes laid out directly in it at
    /// their unrounded rects ([`scroller_extents`](super::extents::scroller_extents), #2116), and its
    /// unrounded origin in its Taffy root's frame ([`taffy_origin`]), which
    /// a child's rounded location is rounded from (#2437).
    unrounded: Option<(taffy::NodeId, (f32, f32))>,
    /// The in-flow boxes' margin boxes' furthest `(right, bottom)`, kept
    /// apart from the walk's other contributions ([`Measured::flow`]).
    flow: std::cell::Cell<Reach>,
    /// Whether it is a grid container, whose items count their border box
    /// alone: Chrome counts the grid's tracks, which layout does not keep
    /// (see [`extend_with_child`]).
    grid: bool,
}

impl ContainerFrame {
    /// Grow `reach` by a box measured where it is drawn (`x0..x1`, down to
    /// `y1`): with no end padding, so it comes off here — `padding-left`
    /// on the left, the inline end of an rtl container.
    #[inline(always)]
    fn grow_drawn(&self, reach: &mut Reach, x0: f64, x1: f64, y1: f64) {
        reach.grow(
            x0 + self.padding_left,
            x1 - self.end_padding.0,
            y1 - self.end_padding.1,
        );
    }
}

/// Grow `reach` by one box-tree child of the walk — by how it
/// contributes, which [`content_extents`](super::content_extents) spells out.
pub(super) fn extend_with_child<const UNROUNDED: bool>(
    tree: &NodeTree,
    container_id: RawNodeId,
    child_id: RawNodeId,
    child: &crate::node::Node,
    frame: &ContainerFrame,
    reach: &mut Reach,
) {
    if child.taffy_style_owned_by_contents_splice() {
        extend_with_children::<UNROUNDED>(tree, container_id, child_id, frame, reach);
        return;
    }
    if frame.inline_measured && child.is_text() && child.ifc_root == Some(container_id) {
        return;
    }
    if !crate::out_of_flow::contributes_to_scrollable_overflow(tree, container_id, child) {
        return;
    }
    let ((x, y, w, h), unrounded) = if UNROUNDED {
        child_rect(tree, frame, child)
    } else {
        let l = &child.layout;
        (
            (l.x as f64, l.y as f64, l.width as f64, l.height as f64),
            false,
        )
    };
    let cs = &child.computed_style;
    let (cb_w, cb_h) = (frame.content_box.0 as f32, frame.content_box.1 as f32);
    // In flow: the margin box at the static position, which the container's
    // end padding extends — and the frame difference cancels (see
    // `content_extents`). Not for an `absolute` box: it has no in-flow area.
    let mut offset = (0.0, 0.0);
    if cs.position != PositionValue::Absolute {
        if matches!(cs.position, PositionValue::Relative | PositionValue::Sticky) {
            // Taffy lays a `relative` (and a `sticky`, which it is handed as
            // one) box out at its offset position; `left` wins over `right`
            // and `top` over `bottom`, percentages of the content box.
            let dx = cs
                .left
                .resolve(cb_w)
                .or_else(|| cs.right.resolve(cb_w).map(|r| -r))
                .unwrap_or(0.0);
            let dy = cs
                .top
                .resolve(cb_h)
                .or_else(|| cs.bottom.resolve(cb_h).map(|b| -b))
                .unwrap_or(0.0);
            // Rounded as Taffy rounds the layout rect it applied the offset
            // to: a `top: -10%` of 74px is placed 7px up, and taking 7.4 off
            // it left the static box 0.4px low.
            // Unrounded where the rect is (#2116). No offset — a `relative`
            // row that only anchors its badges — rounds nothing.
            offset = if dx == 0.0 && dy == 0.0 {
                (0.0, 0.0)
            } else if unrounded {
                (dx as f64, dy as f64)
            } else {
                (dx.round() as f64, dy.round() as f64)
            };
        }
        // A percentage margin, on either axis, resolves against the
        // containing block's width; `auto` counts as 0, and a negative margin
        // never pulls the end inside the border box.
        //
        // A grid item counts its border box alone, as before #776: Chrome
        // counts the grid's *tracks* rather than an item's margin box, and
        // layout does not keep them. A margin box overshoots where a margin
        // overflows its area (`margin-right: 300px` on a 50px item in a 100px
        // column: Chrome 0, a margin box 150; `margin-right: 100%` resolves
        // against the area, not the container: Chrome 0, a margin box 50),
        // and the border box under-counts where a margin grows an auto track
        // (`margin-bottom: 200%` in an auto row: Chrome 120, here 0).
        //
        // A non-replaced `display: inline` box's block-axis margins do
        // nothing (CSS 2.1 §10.6.1) and its inline-axis ones are already in
        // the line the container's inline layout measured, so it counts its
        // border box alone: `<span style="margin-bottom: 200px">` scrolls 0
        // in Chrome. A replaced element at `display: inline` is
        // `DisplayMode::InlineBlock` (#1089) and keeps its margins, as an
        // `inline-block`, `inline-flex` and `inline-grid` box does.
        //
        // An rtl container's inline end is its left (#2546): there the
        // margin box's left edge counts, `margin-left` as `margin-right`
        // counts on the right.
        let margin = |m: &crate::computed_style::LengthPercentageAutoValue| {
            m.resolve(cb_w).unwrap_or(0.0).max(0.0) as f64
        };
        let (margin_left, margin_right, margin_bottom) =
            if frame.grid || child.display_mode == DisplayMode::Inline {
                (0.0, 0.0, 0.0)
            } else {
                (
                    if frame.rtl {
                        margin(&cs.margin_left)
                    } else {
                        0.0
                    },
                    margin(&cs.margin_right),
                    margin(&cs.margin_bottom),
                )
            };
        let mut flow = frame.flow.get();
        flow.grow(
            x - offset.0 - margin_left,
            x - offset.0 + w + margin_right,
            y - offset.1 + h + margin_bottom,
        );
        frame.flow.set(flow);
    } else if is_empty_box(child) {
        // An `absolute` box with no area counts nothing, wherever it is:
        // Chrome 156 gives a 200x100 scroller no range for a `0x500` one, a
        // `10x0` one at `top: 300px` or a `0x50` one at `left: 300px`
        // (`tests/abs_scroll_range_770_tests.rs`).
        return;
    }
    // Where it is drawn: an `absolute` box, an offset `relative` one and a
    // transformed one count their border box, transformed, with no end
    // padding — so it comes off here.
    let transformed = child.has_applied_transform();
    if cs.position == PositionValue::Absolute || transformed || offset != (0.0, 0.0) {
        let drawn = kurbo::Rect::new(x, y, x + w, y + h);
        let drawn = if transformed {
            crate::paint::compose_node_transform(child, x, y, 1.0, kurbo::Affine::IDENTITY)
                .transform_rect_bbox(drawn)
        } else {
            drawn
        };
        frame.grow_drawn(reach, drawn.x0, drawn.x1, drawn.y1);
    }
    // An anonymous box's lines are the container's own text, so they are
    // measured as an IFC-root container's are: a `nowrap` line wider than
    // the box (which is the container's content width) is overflow the box
    // rect does not show. The box has no padding or border
    // (`ComputedStyle::for_anonymous_box`), so its lines start at its own
    // origin. Only the width: the box's height *is* its lines' height.
    // Only an anonymous box: a block child's lines are its own content,
    // which the walk does not descend into (it stops at the first box).
    if child.is_anonymous_block_box
        && let Some(inline) = &child.text_layout
    {
        reach.right = reach.right.max(x + inline.layout.width() as f64);
        if frame.rtl {
            reach.left = reach.left.min(x + lines_left(&inline.layout));
        }
    }
}

/// Where an inline layout's leftmost line starts, from its origin: an rtl
/// line wider than the box is aligned to its right edge and overflows to the
/// left (parley's alignment), which an rtl scroll container can scroll to
/// (#2546). `0` or more when no line overflows to the left.
pub(super) fn lines_left(layout: &parley::layout::Layout<peniko::Brush>) -> f64 {
    layout
        .lines()
        .map(|line| line.metrics().offset as f64)
        .fold(f64::INFINITY, f64::min)
}

/// A box's layout rect `(x, y, width, height)` in its parent's frame, as
/// [`extend_with_child`] measures it: the rounded `layout`, or — in a walk for
/// [`scroller_extents`](super::extents::scroller_extents) — Taffy's unrounded rect for a box Taffy laid out
/// directly in the container, where `layout` is a rounding of it (Taffy rounds
/// the location as an absolute edge, [`rounded_location`], and the size
/// within a pixel). A box placed
/// after layout (an absolute one moved by a scroll, an inline one placed by
/// its line) fails that test and is read rounded. The flag says which.
#[inline(always)]
pub(super) fn child_rect(
    tree: &NodeTree,
    frame: &ContainerFrame,
    child: &crate::node::Node,
) -> ((f64, f64, f64, f64), bool) {
    let l = &child.layout;
    let rounded = (
        (l.x as f64, l.y as f64, l.width as f64, l.height as f64),
        false,
    );
    let (Some((container, (ox, oy))), Some(taffy_id)) = (frame.unrounded, child.taffy_id) else {
        return rounded;
    };
    if tree.taffy.parent(taffy_id) != Some(container) {
        return rounded;
    }
    let u = tree.taffy.unrounded_layout(taffy_id);
    let is_rounding = rounded_location(ox, u.location.x) == l.x
        && rounded_location(oy, u.location.y) == l.y
        && (u.size.width - l.width).abs() < 1.0
        && (u.size.height - l.height).abs() < 1.0;
    if !is_rounding {
        return rounded;
    }
    (
        (
            u.location.x as f64,
            u.location.y as f64,
            u.size.width as f64,
            u.size.height as f64,
        ),
        true,
    )
}

/// Where `taffy_id`'s box starts in its Taffy root's frame, unrounded: the
/// sum `round_layout` rounds a child's location from, accumulated root-first
/// in `f32` as it accumulates it, so the two agree to the bit.
pub(crate) fn taffy_origin(
    taffy: &taffy::TaffyTree<crate::node::NodeContext>,
    taffy_id: taffy::NodeId,
) -> (f32, f32) {
    let mut chain = vec![taffy_id];
    while let Some(parent) = taffy.parent(*chain.last().expect("non-empty")) {
        chain.push(parent);
    }
    chain.iter().rev().fold((0.0, 0.0), |(x, y), &n| {
        let l = taffy.unrounded_layout(n).location;
        (x + l.x, y + l.y)
    })
}

/// The location Taffy's `round_layout` gives a box at unrounded offset
/// `offset` in a parent whose unrounded origin is `origin`: its rounded
/// absolute edge less its parent's (#2437), Taffy's `round` (half up).
pub(super) fn rounded_location(origin: f32, offset: f32) -> f32 {
    let round = |v: f32| (v + 0.5).floor();
    round(origin + offset) - round(origin)
}
