//! Where a scroll container's overlay scrollbars are.
//!
//! One computation, three consumers: the paint pass draws the thumbs from it,
//! and the desktop input path hit-tests and drags them by it. They used to
//! derive it separately — paint inline in [`super::paint_node`], input in
//! `rinch/src/app/hit_testing.rs` and the `MouseDown`/`MouseMove` arms — and
//! the two had drifted apart in two ways, so a drag did not move the thumb the
//! distance the pointer moved (#400):
//!
//! * paint measured the **track** across the container's border box
//!   (`layout.height - 2 * margin`), input across its **content** box
//!   (`client_height - 4`). A 1px border is enough to separate them: measured
//!   live in `ui-zoo-desktop`, dragging that section's `overflow-x: auto`
//!   sample 80px moved its thumb 80.74px — exactly `216 / 214`;
//! * paint clamps a thumb to [`MIN_THUMB`] and input never knew, so a heavily
//!   overflowing container's thumb *lagged* the pointer instead. Measured live
//!   on the `overflow: auto` sample (both bars up, so the corner is reserved
//!   too): a 40px drag moved the thumb 35.41px.
//!
//! Both errors are small, and in opposite directions, which is why the drag
//! stayed plausible for so long. Neither is the "over-scrolls by
//! `content / max_scroll`" the issue was filed for: `moved / track * content`
//! is *algebraically identical* to `moved * max_scroll / thumb_travel`
//! whenever `thumb_len == track_len * visible / content`, so the one-line
//! change that issue proposed is a no-op unless the two track lengths are
//! reconciled as well. Hence a shared computation rather than a new formula.
//!
//! # Coordinate space
//!
//! Everything here is in the container's **own** space (the space
//! [`super::point_in_painted_box`] maps a pointer into), scaled by the `scale`
//! passed in — so paint asks at its device scale and input asks at 1.0.
//! Offsets are measured from the container's **border-box** origin, which is
//! what paint's `x`/`y` and Taffy's layout rect both use.

use crate::NodeTree;
use crate::computed_style::{PositionValue, ScrollbarWidthValue};
use crate::node::{DisplayMode, RawNodeId};
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo;

/// How thick a thumb is drawn, in logical pixels.
pub const THICKNESS: f64 = 6.0;
/// How thick a `--rinch-scrollbar-width: thin` thumb is, in logical pixels.
pub const THIN_THICKNESS: f64 = 4.0;
/// The gap between a thumb and the container's edge, in logical pixels. Also
/// the gap at each end of the track.
pub const MARGIN: f64 = 2.0;
/// A thumb is never drawn shorter than this, however little of the content is
/// visible — otherwise a long document's thumb shrinks to a point. The one
/// exception is a track shorter than this: there the thumb fills the track and
/// is no longer than it, since a longer one would be drawn past the track's
/// end (#1141).
pub const MIN_THUMB: f64 = 20.0;
/// How wide the strip along a container's edge that answers a press as its
/// scrollbar is, in logical pixels — deliberately wider than the thumb is
/// drawn, so the bar is easy to grab. Both axes use it.
///
/// It is also the side of the **corner**: where both bars are up, the
/// `HIT_THICKNESS` square at the bottom-right belongs to neither bar. Each hit
/// strip stops at it ([`Scrollbars::hit_strip`]), and each painted track gives
/// it up too ([`ScrollbarTrack::track_len`]), so a thumb that fits its track
/// is never drawn where a press cannot grab it (#444) — a thumb is never
/// longer than its track, including a track shorter than [`MIN_THUMB`], which
/// it fills (#1141). Paint used to reserve only the other
/// bar's drawn footprint (`THICKNESS + MARGIN`), which left the last ~6px of a
/// thumb at full scroll painted inside the square no strip claims.
pub const HIT_THICKNESS: f64 = 16.0;
/// How opaque the built-in thumb is over whatever it covers.
pub const AUTO_THUMB_ALPHA: f32 = 0.4;

/// Which of a container's two bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarAxis {
    /// Down the right-hand edge, moving `scroll_offset.1`.
    Vertical,
    /// Along the bottom edge, moving `scroll_offset.0`.
    Horizontal,
}

/// One bar's geometry, along its own axis.
///
/// All distances are in the container's own space at the requested scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollbarTrack {
    /// The content extent along this axis.
    pub content: f64,
    /// The visible (content-box) extent along this axis.
    pub visible: f64,
    /// How far the content can travel: `content - visible`, always `> 0` for a
    /// bar that exists at all.
    pub max_scroll: f64,
    /// Where the track starts, from the container's border-box origin. Always
    /// [`MARGIN`] at the requested scale; named so callers do not re-derive it.
    pub track_start: f64,
    /// How long the track is. Measured across the container's **border box**
    /// less a [`MARGIN`] at each end, less [`HIT_THICKNESS`] at the far end
    /// when the other bar is up — that reserved square is the corner neither
    /// bar claims, in paint and in hit testing alike (#444). The track's end
    /// therefore sits a [`MARGIN`] clear of the corner, and so inside its own
    /// bar's hit strip.
    pub track_len: f64,
    /// How long the thumb is: never below [`MIN_THUMB`] and never above
    /// [`track_len`](Self::track_len) — a track shorter than `MIN_THUMB` is
    /// filled by its thumb, which then cannot travel (#1141).
    pub thumb_len: f64,
    /// How far the thumb can travel: `track_len - thumb_len`, clamped at 0.
    ///
    /// This, not `track_len`, is the denominator that converts a pointer
    /// distance into a scroll distance.
    pub thumb_travel: f64,
}

impl ScrollbarTrack {
    /// Where the thumb's leading edge sits for a given scroll offset, from the
    /// container's border-box origin.
    pub fn thumb_start(&self, scroll: f64) -> f64 {
        let ratio = if self.max_scroll > 0.0 {
            (scroll / self.max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.track_start + self.thumb_travel * ratio
    }

    /// Whether a point `pos` along the track (from the container's border-box
    /// origin) is on the thumb as it is painted at `scroll`, ends included.
    ///
    /// This is the question both input sites ask: a press here grabs the thumb
    /// in place rather than jumping (#443), and it wins a borderless window's
    /// resize zone (#399, #420).
    pub fn thumb_contains(&self, scroll: f64, pos: f64) -> bool {
        let start = self.thumb_start(scroll);
        pos >= start && pos <= start + self.thumb_len
    }

    /// The scroll offset a thumb dragged `moved` along the track from
    /// `start_scroll` should land on, clamped to the scrollable range.
    ///
    /// The ratio's denominator is [`thumb_travel`](Self::thumb_travel) and its
    /// numerator [`max_scroll`](Self::max_scroll), which is what makes the
    /// thumb move exactly as far as the pointer did. A thumb that cannot
    /// travel (`thumb_travel == 0`, i.e. a `min`-clamped thumb filling its
    /// track) cannot be dragged either, so the offset does not move.
    pub fn scroll_for_drag(&self, start_scroll: f64, moved: f64) -> f64 {
        if self.thumb_travel <= 0.0 {
            return start_scroll.clamp(0.0, self.max_scroll);
        }
        (start_scroll + moved * self.max_scroll / self.thumb_travel).clamp(0.0, self.max_scroll)
    }

    /// The scroll offset for a press `pos` along the track, measured from the
    /// container's border-box origin — rinch's jump-to-click, where a position
    /// along the track maps linearly onto the scroll range.
    ///
    /// For a press on the empty track only: a press on the thumb
    /// ([`thumb_contains`](Self::thumb_contains)) grabs it where it is (#443).
    pub fn scroll_for_click(&self, pos: f64) -> f64 {
        if self.track_len <= 0.0 {
            return 0.0;
        }
        ((pos - self.track_start) / self.track_len).clamp(0.0, 1.0) * self.max_scroll
    }
}

/// Both of a container's bars, or `None` per axis where no bar is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scrollbars {
    pub vertical: Option<ScrollbarTrack>,
    pub horizontal: Option<ScrollbarTrack>,
    /// The thumb thickness at the requested scale — [`THICKNESS`], or
    /// [`THIN_THICKNESS`] under `--rinch-scrollbar-width: thin`.
    pub thickness: f64,
    /// [`MARGIN`] at the requested scale.
    pub margin: f64,
    /// [`HIT_THICKNESS`] at the requested scale.
    pub hit_thickness: f64,
    /// The container's border-box width at the requested scale.
    pub box_width: f64,
    /// The container's border-box height at the requested scale.
    pub box_height: f64,
    /// What to fill the thumb with.
    pub thumb_color: AlphaColor<Srgb>,
    /// What to fill the track with, or `None` for no track — the default, since
    /// rinch's bar is an overlay.
    pub track_color: Option<AlphaColor<Srgb>>,
}

impl Scrollbars {
    /// The bar on one axis, if it exists.
    pub fn axis(&self, axis: ScrollbarAxis) -> Option<ScrollbarTrack> {
        match axis {
            ScrollbarAxis::Vertical => self.vertical,
            ScrollbarAxis::Horizontal => self.horizontal,
        }
    }

    /// The strip that answers a press as `axis`'s bar, as `(x0, y0, x1, y1)`
    /// from the container's border-box origin, ends included — or `None`
    /// where that axis has no bar.
    ///
    /// [`HIT_THICKNESS`] wide against the container's edge, the full box long
    /// — except that where both bars are up each strip stops at the corner
    /// square, which belongs to neither: a press there falls through to the
    /// container. The painted track gives up the same square, so every thumb
    /// pixel lies inside its bar's strip (#444), and a thumb never outgrows
    /// its track (#1141).
    pub fn hit_strip(&self, axis: ScrollbarAxis) -> Option<(f64, f64, f64, f64)> {
        self.axis(axis)?;
        let (w, h, t) = (self.box_width, self.box_height, self.hit_thickness);
        Some(match axis {
            ScrollbarAxis::Vertical => {
                let end = if self.horizontal.is_some() { h - t } else { h };
                (w - t, 0.0, w, end)
            }
            ScrollbarAxis::Horizontal => {
                let end = if self.vertical.is_some() { w - t } else { w };
                (0.0, h - t, end, h)
            }
        })
    }
}

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
/// Not counted yet: a margin that collapses through a child's end (a
/// child of `height: auto` whose last child has `margin-bottom: 80px` scrolls
/// 30 in Chrome and 0 here) and a descendant's overflow below the first box
/// (#770).
///
/// In logical pixels — this is layout's own unit, and every caller either wants
/// it that way or scales the result itself.
///
/// The padding+border sum below is deliberately *not*
/// [`super::ifc_root_content_origin`]: that helper defines where an IFC draws
/// its inline content, and its doc contract is about anonymous roots — while
/// this one measures a scroll container's own scrollable overflow, and a
/// scroll container is never an anonymous box (an anonymous box's style
/// carries no `overflow`).
pub fn content_extents(tree: &NodeTree, node_id: RawNodeId) -> (f64, f64) {
    let Some(node) = tree.get(node_id) else {
        return (0.0, 0.0);
    };
    let cs = &node.computed_style;
    let content_left = (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64;
    let content_top = (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64;
    let (mut right, mut bottom) = (0.0_f64, 0.0_f64);
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
        right = content_left + inline.layout.width() as f64;
        bottom = content_top + inline.layout.height() as f64;
        true
    } else {
        false
    };
    // A `<textarea>`'s own text (#827): its value is an attribute, so no box
    // and no inline layout holds it. Its height as last shaped for its
    // scroll, at the content origin like an IFC's lines; it wraps at the
    // column, so it adds no width.
    if let Some(text_h) = crate::form_control::textarea_scroll_extent(node) {
        bottom = bottom.max(content_top + text_h as f64);
    }
    let frame = ContainerFrame {
        end_padding: (
            cs.padding_right.to_px() as f64,
            cs.padding_bottom.to_px() as f64,
        ),
        content_box: visible_extents(tree, node_id),
        inline_measured,
        grid: matches!(
            cs.display,
            crate::computed_style::DisplayValue::Grid
                | crate::computed_style::DisplayValue::InlineGrid
        ),
    };
    extend_with_children(tree, node_id, node_id, &frame, &mut right, &mut bottom);
    (
        (right - content_left).max(0.0),
        (bottom - content_top).max(0.0),
    )
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
fn extend_with_children(
    tree: &NodeTree,
    container_id: RawNodeId,
    parent_id: RawNodeId,
    frame: &ContainerFrame,
    right: &mut f64,
    bottom: &mut f64,
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
                extend_with_box_tree_children(tree, container_id, parent_id, frame, right, bottom);
                return;
            }
            extend_with_child(tree, container_id, child_id, child, frame, right, bottom);
        }
        return;
    }
    extend_with_box_tree_children(tree, container_id, parent_id, frame, right, bottom);
}

/// [`extend_with_children`] over `parent_id`'s box-tree children.
fn extend_with_box_tree_children(
    tree: &NodeTree,
    container_id: RawNodeId,
    parent_id: RawNodeId,
    frame: &ContainerFrame,
    right: &mut f64,
    bottom: &mut f64,
) {
    for &child_id in crate::RinchDocument::box_tree_children(&tree.nodes, parent_id).iter() {
        let Some(child) = tree.get_live(child_id) else {
            continue;
        };
        extend_with_child(tree, container_id, child_id, child, frame, right, bottom);
    }
}

/// What [`extend_with_child`] needs of the scroll container itself.
struct ContainerFrame {
    /// Its `(padding-right, padding-bottom)`: what Chrome extends the in-flow
    /// area by and nothing else, so a box measured where it is drawn gives it
    /// back.
    end_padding: (f64, f64),
    /// Its content box `(width, height)` — the containing block a child's
    /// percentage margins and `relative` offsets resolve against.
    content_box: (f64, f64),
    /// Whether the container's own inline layout is already measured, so a
    /// text node it lays out is skipped (see [`extend_with_children`]).
    inline_measured: bool,
    /// Whether it is a grid container, whose items count their border box
    /// alone: Chrome counts the grid's tracks, which layout does not keep
    /// (see [`extend_with_child`]).
    grid: bool,
}

/// Grow `right`/`bottom` by one box-tree child of the walk — by how it
/// contributes, which [`content_extents`] spells out.
fn extend_with_child(
    tree: &NodeTree,
    container_id: RawNodeId,
    child_id: RawNodeId,
    child: &crate::node::Node,
    frame: &ContainerFrame,
    right: &mut f64,
    bottom: &mut f64,
) {
    if child.taffy_style_owned_by_contents_splice() {
        extend_with_children(tree, container_id, child_id, frame, right, bottom);
        return;
    }
    if frame.inline_measured && child.is_text() && child.ifc_root == Some(container_id) {
        return;
    }
    if !crate::out_of_flow::contributes_to_scrollable_overflow(tree, container_id, child) {
        return;
    }
    let l = &child.layout;
    let (x, y, w, h) = (l.x as f64, l.y as f64, l.width as f64, l.height as f64);
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
            offset = (dx.round() as f64, dy.round() as f64);
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
        let (margin_right, margin_bottom) =
            if frame.grid || child.display_mode == DisplayMode::Inline {
                (0.0, 0.0)
            } else {
                (
                    cs.margin_right.resolve(cb_w).unwrap_or(0.0).max(0.0) as f64,
                    cs.margin_bottom.resolve(cb_w).unwrap_or(0.0).max(0.0) as f64,
                )
            };
        *right = right.max(x - offset.0 + w + margin_right);
        *bottom = bottom.max(y - offset.1 + h + margin_bottom);
    }
    // Where it is drawn: an `absolute` box, an offset `relative` one and a
    // transformed one count their border box, transformed, with no end
    // padding — so it comes off here.
    let transformed = child.has_applied_transform();
    if cs.position == PositionValue::Absolute || transformed || offset != (0.0, 0.0) {
        let drawn = kurbo::Rect::new(x, y, x + w, y + h);
        let drawn = if transformed {
            super::compose_node_transform(child, x, y, 1.0, kurbo::Affine::IDENTITY)
                .transform_rect_bbox(drawn)
        } else {
            drawn
        };
        *right = right.max(drawn.x1 - frame.end_padding.0);
        *bottom = bottom.max(drawn.y1 - frame.end_padding.1);
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
        *right = right.max(child.layout.x as f64 + inline.layout.width() as f64);
    }
}

/// What the built-in `auto` thumb looks like on this container.
///
/// 40% black is a good default over light chrome and *no scrollbar at all* over
/// dark chrome: composited over a `#0b0e13` panel it resolves to about
/// `#070810`, a delta of 3-8 per channel, which does not read on a monitor. A
/// dark app therefore scrolled with no visible sign that there was anything
/// below the fold (#416).
///
/// So the default follows the palette instead of ignoring it: 40% of **black or
/// white**, chosen by the luminance of the container's own computed `color`.
/// Only the polarity follows — the thumb stays neutral grey — so a container
/// that happens to set `color: red` does not get a red thumb, and every
/// light-themed app is pixel-identical to before (the initial `color` is black,
/// and so is every light theme's text). A dark app's text is light, so its
/// thumb flips to white and becomes visible with no opt-in at all.
///
/// `color` rather than `background-color` because it is always resolved:
/// backgrounds are transparent by default, so the container that most needs
/// this — a scroll region inheriting the page's dark background — would have
/// nothing to read.
fn auto_thumb_color(color: Option<peniko::Color>) -> AlphaColor<Srgb> {
    let light_text = color.is_some_and(|c| {
        let rgba = c.to_rgba8();
        // Relative-luminance weights on the sRGB values. Exact enough for a
        // binary polarity choice, and it does not need to be more.
        let l = (0.2126 * rgba.r as f32 + 0.7152 * rgba.g as f32 + 0.0722 * rgba.b as f32) / 255.0;
        l > 0.5
    });
    let c = if light_text { 1.0 } else { 0.0 };
    AlphaColor::<Srgb>::new([c, c, c, AUTO_THUMB_ALPHA])
}

/// The container's own **content-box** extent `(width, height)` — its
/// border-box [`LayoutResult`](crate::LayoutResult) size less its own padding
/// and border — in logical pixels.
///
/// This is the "visible" half of the overflow test [`scrollbars`] answers by
/// painting a bar: a node overflows an axis when [`content_extents`] on that
/// axis exceeds this. Exposed so `rinch/src/app/hit_testing.rs`'s
/// `find_*_scroll_container` walks (the wheel's routing) ask the exact same
/// question rather than re-deriving "visible" against the **border** box,
/// which is strictly larger whenever there is any padding or border: that
/// drift is #769 (a container whose content overflows its content box but
/// not its border box painted, and let you drag, a bar the wheel routed
/// past). The window is only `padding + border` wide, so it opened only when
/// the overflow was smaller than that — which is why it was rarely seen, not
/// because components avoided it: the Drawer body, the FloatingPanel body,
/// the Select dropdown, the menu-bar dropdowns and `Code` are all padded or
/// bordered scroll regions. `DomDocument::client_height`/`client_width` and
/// layout's `clamp_scroll_offsets` read it too. Prefer [`overflows`] when
/// only the yes/no answer is wanted.
pub fn visible_extents(tree: &NodeTree, node_id: RawNodeId) -> (f64, f64) {
    let Some(node) = tree.get(node_id) else {
        return (0.0, 0.0);
    };
    let cs = &node.computed_style;
    let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64;
    let border_v = (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64;
    let pad_h = (cs.padding_left.to_px() + cs.padding_right.to_px()) as f64;
    let border_h = (cs.border_left_width.to_px() + cs.border_right_width.to_px()) as f64;
    let box_w = node.layout.width as f64;
    let box_h = node.layout.height as f64;
    (
        (box_w - pad_h - border_h).max(0.0),
        (box_h - pad_v - border_v).max(0.0),
    )
}

/// Whether `node_id` is scrollable on `axis` **and** its content overflows —
/// the scrollable-and-overflowing half of the test [`scrollbars`] makes
/// before painting a bar on that axis, exposed as a boolean so a caller that
/// only wants "is this a scroll container the wheel should move with" is not
/// tempted to re-derive it against the wrong box (#769). `scrollable` alone
/// is not enough: a container can declare `overflow: auto` and still fit its
/// content, in which case neither the bar nor the wheel should claim it.
///
/// It is deliberately **not** `scrollbars(..).axis(..).is_some()`: it ignores
/// `--rinch-scrollbar-width: none`, which removes the bar and not the
/// scrolling — a browser's `scrollbar-width: none` box still takes the wheel
/// (measured in Chrome 153). Pinned by
/// `a_barless_padded_container_still_takes_the_wheel`.
pub fn overflows(tree: &NodeTree, node_id: RawNodeId, axis: ScrollbarAxis) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    let scrollable = match axis {
        ScrollbarAxis::Vertical => node.scrolls_y(),
        ScrollbarAxis::Horizontal => node.scrolls_x(),
    };
    if !scrollable {
        return false;
    }
    let (visible_w, visible_h) = visible_extents(tree, node_id);
    let (content_w, content_h) = content_extents(tree, node_id);
    match axis {
        ScrollbarAxis::Vertical => content_h > visible_h,
        ScrollbarAxis::Horizontal => content_w > visible_w,
    }
}

/// The overlay scrollbars of `node_id`, at `scale`.
///
/// A bar exists on an axis when that axis is scrollable **and** overflowing.
/// `scroll` and `auto` behave identically: rinch paints a thumb and no track,
/// so there is nothing for `scroll` to show when the content fits.
///
/// `--rinch-scrollbar-width: none` answers "no bars" here, which is what makes
/// it turn off the 16px hit strip as well as the paint — an app that draws its
/// own bar could otherwise only cover rinch's up, not switch it off.
///
/// Cheap for the overwhelming majority of nodes: a node scrollable on neither
/// axis pays two enum checks and returns without walking its children.
pub fn scrollbars(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> Scrollbars {
    let Some(node) = tree.get(node_id) else {
        return Scrollbars {
            vertical: None,
            horizontal: None,
            thickness: THICKNESS * scale,
            margin: MARGIN * scale,
            hit_thickness: HIT_THICKNESS * scale,
            box_width: 0.0,
            box_height: 0.0,
            thumb_color: auto_thumb_color(None),
            track_color: None,
        };
    };
    let cs = &node.computed_style;
    let thickness = match cs.scrollbar_width {
        ScrollbarWidthValue::Thin => THIN_THICKNESS,
        _ => THICKNESS,
    };
    let empty = Scrollbars {
        vertical: None,
        horizontal: None,
        thickness: thickness * scale,
        margin: MARGIN * scale,
        hit_thickness: HIT_THICKNESS * scale,
        box_width: node.layout.width as f64 * scale,
        box_height: node.layout.height as f64 * scale,
        thumb_color: cs
            .scrollbar_color
            .thumb
            .unwrap_or_else(|| auto_thumb_color(cs.color)),
        track_color: cs.scrollbar_color.track,
    };

    // A `display: contents` element answers `false` on both axes (#1038).
    let scrollable_y = node.scrolls_y();
    let scrollable_x = node.scrolls_x();
    if (!scrollable_y && !scrollable_x) || cs.scrollbar_width == ScrollbarWidthValue::None {
        return empty;
    }

    let (content_w, content_h) = content_extents(tree, node_id);
    let (visible_w, visible_h) = visible_extents(tree, node_id);
    let box_w = node.layout.width as f64;
    let box_h = node.layout.height as f64;

    let show_vertical = scrollable_y && content_h > visible_h;
    let show_horizontal = scrollable_x && content_w > visible_w;

    // Where both bars are up, each track gives up the corner square at its far
    // end so the two thumbs cannot pile into it. The square is the hit strips'
    // corner, not the painted bars' footprint: nothing is painted where
    // hit-testing gives the corner to neither bar, so every thumb pixel is
    // grabbable (#444). A thumb is never longer than its track: on a track
    // shorter than MIN_THUMB it fills the track and has no travel, rather than
    // being drawn past the track's end — into the corner with both bars up,
    // into the end margin with one (#1141).
    let corner = HIT_THICKNESS;
    let track_of = |box_extent: f64, visible: f64, content: f64, other_bar: bool| {
        let reserved = if other_bar { corner } else { 0.0 };
        let track_len = (box_extent - MARGIN * 2.0 - reserved).max(0.0);
        let thumb_len = (track_len * (visible / content))
            .max(MIN_THUMB)
            .min(track_len);
        ScrollbarTrack {
            content: content * scale,
            visible: visible * scale,
            max_scroll: (content - visible).max(0.0) * scale,
            track_start: MARGIN * scale,
            track_len: track_len * scale,
            thumb_len: thumb_len * scale,
            thumb_travel: (track_len - thumb_len).max(0.0) * scale,
        }
    };

    Scrollbars {
        vertical: show_vertical.then(|| track_of(box_h, visible_h, content_h, show_horizontal)),
        horizontal: show_horizontal.then(|| track_of(box_w, visible_w, content_w, show_vertical)),
        ..empty
    }
}
