//! Where a scroll container's overlay scrollbars are.
//!
//! One computation, three consumers: the paint pass draws the thumbs from it,
//! and the desktop input path hit-tests and drags them by it. They used to
//! derive it separately — paint inline in `super::paint_node`, input in
//! `rinch/src/app/hit_testing/` and the `MouseDown`/`MouseMove` arms — and
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
//!
//! # Modules
//!
//! This module holds the bars themselves: their geometry ([`scrollbars`]) and
//! the axis queries input asks ([`overflows`], [`scroll_bounds`], …). The
//! range they are computed from lives below it:
//!
//! * `extents` — a box's scrollable range ([`content_extents`],
//!   [`scroll_extents`]) and the in-flow ranges layout's clamp asks first;
//! * `measure` — measuring a container's children where they are laid out or
//!   drawn;
//! * `overflow_abs` — the absolute boxes listed for a container from below
//!   its children (#770);
//! * `deep_walk` — the walk of a scroll container's descendants below its
//!   own children (#2054), and what it reads of each box;
//! * `deep_record` — what a kept walk records of each box, and the walk that
//!   records a subtree or brings its records up to date;
//! * `deep_memo` — the per-container memo of that walk, and the check that
//!   finds what changed under it.

use crate::NodeTree;
use crate::computed_style::ScrollbarWidthValue;
use crate::node::RawNodeId;
use peniko::color::{AlphaColor, Srgb};

mod deep_mark;
mod deep_memo;
mod deep_record;
mod deep_slots;
mod deep_walk;
mod extents;
mod measure;
mod overflow_abs;

pub(crate) use deep_memo::DeepOverflow;
pub(crate) use extents::{
    ExactFlow, ROUNDED_RANGE_SLACK, content_extents_in_flow, content_extents_in_flow_rounded,
};
pub use extents::{content_extents, scroll_extents};
pub(crate) use measure::taffy_origin;
pub(crate) use overflow_abs::OverflowAbsolutes;

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
    /// The offset at the thumb's start (the track's left or top end): `0`,
    /// or `-max_scroll` for the horizontal bar of a container that scrolls
    /// from its right edge ([`scrolls_from_right`], #2546), whose offset
    /// runs from `-max_scroll` (thumb at the left) to `0` (thumb at the
    /// right). The offsets this track takes and answers run
    /// `min_scroll..=min_scroll + max_scroll`.
    pub min_scroll: f64,
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
            ((scroll - self.min_scroll) / self.max_scroll).clamp(0.0, 1.0)
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
        let (lo, hi) = (self.min_scroll, self.min_scroll + self.max_scroll);
        if self.thumb_travel <= 0.0 {
            return start_scroll.clamp(lo, hi);
        }
        (start_scroll + moved * self.max_scroll / self.thumb_travel).clamp(lo, hi)
    }

    /// The scroll offset for a press `pos` along the track, measured from the
    /// container's border-box origin — rinch's jump-to-click, where a position
    /// along the track maps linearly onto the scroll range.
    ///
    /// For a press on the empty track only: a press on the thumb
    /// ([`thumb_contains`](Self::thumb_contains)) grabs it where it is (#443).
    pub fn scroll_for_click(&self, pos: f64) -> f64 {
        if self.track_len <= 0.0 {
            return self.min_scroll;
        }
        self.min_scroll
            + ((pos - self.track_start) / self.track_len).clamp(0.0, 1.0) * self.max_scroll
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
/// axis exceeds this. Exposed so `rinch/src/app/hit_testing/scroll.rs`'s
/// `find_*_scroll_container` walks (the wheel's routing) ask the exact same
/// question rather than re-deriving "visible" against the **border** box,
/// which is strictly larger whenever there is any padding or border: that
/// drift is #769 (a container whose content overflows its content box but
/// not its border box painted, and let you drag, a bar the wheel routed
/// past). The window is only `padding + border` wide, so it opened only when
/// the overflow was smaller than that — which is why it was rarely seen, not
/// because components avoided it: the Drawer body, the FloatingPanel body,
/// the Select dropdown, the menu-bar dropdowns and `Code` are all padded or
/// bordered scroll regions. `DomScroll::client_height`/`client_width` and
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

/// Whether the wheel can move `node_id` toward `(dx, dy)` (the DOM's signs:
/// positive scrolls down / right; only the signs count): on an axis whose
/// delta is non-zero it [`overflows`], and its offset is short of the end of
/// its range in that direction — the end being the range's end on the device
/// pixel grid, where the wheel's write stops (#1543) — and no `lock_scroll`
/// overlay locks it out ([`NodeTree::scroll_locked_out`]). Either axis is
/// enough.
///
/// The one rule for "can this container take the wheel": the desktop wheel
/// picks its container by it (the nearest one from the pointer up), and
/// `DomScroll::can_scroll_toward` answers with it, so a wheel handler that
/// asks before declining gets what the default scroll will do.
pub fn can_scroll_toward(tree: &NodeTree, node_id: RawNodeId, dx: f64, dy: f64) -> bool {
    let Some(node) = tree.get(node_id) else {
        return false;
    };
    if tree.scroll_locked_out(node_id) {
        return false;
    }
    let (at_x, at_y) = node.scroll_offset;
    let room = |axis: ScrollbarAxis, at: f64, d: f64| {
        if d == 0.0 || !overflows(tree, node_id, axis) {
            return false;
        }
        let (min, max) = scroll_bounds(tree, node_id, axis);
        let (min, max) = (tree.snap_scroll_offset(min), tree.snap_scroll_offset(max));
        (d < 0.0 && at > min) || (d > 0.0 && at < max)
    };
    room(ScrollbarAxis::Vertical, at_y, dy) || room(ScrollbarAxis::Horizontal, at_x, dx)
}

/// Whether `node_id` scrolls horizontally from its **right** edge: a
/// `direction: rtl` box, as Chrome scrolls one (#2546). Its scrollable
/// overflow is measured leftward from its content box's right edge
/// ([`content_extents`]), and its horizontal offset (`scrollLeft`) is `0` at
/// the start — the right end of the content showing — and runs **negative**
/// toward the left, down to `-(scroll_width - client_width)`. Paint and hit
/// testing take the offset off as they do any other, so a box laid out at
/// `x = -100` shows at its left edge at `scrollLeft = -100`.
pub fn scrolls_from_right(tree: &NodeTree, node_id: RawNodeId) -> bool {
    tree.get(node_id)
        .is_some_and(|n| n.computed_style.direction_rtl)
}

/// The scroll offsets `node_id` can take on `axis`, `(min, max)`: `(0,
/// range)`, or `(-range, 0)` (never `-0.0`) on the horizontal axis of a box that scrolls
/// from its right edge ([`scrolls_from_right`], #2546), where `range` is the
/// content extent less the visible one (never negative). Not snapped to the
/// device grid: a writer snaps each end with `NodeTree::snap_scroll_offset`,
/// which rounds toward 0 on both sides, so a snapped end stays in range.
pub fn scroll_bounds(tree: &NodeTree, node_id: RawNodeId, axis: ScrollbarAxis) -> (f64, f64) {
    let (content_w, content_h) = content_extents(tree, node_id);
    let (visible_w, visible_h) = visible_extents(tree, node_id);
    match axis {
        ScrollbarAxis::Vertical => (0.0, (content_h - visible_h).max(0.0)),
        ScrollbarAxis::Horizontal => {
            let range = (content_w - visible_w).max(0.0);
            if scrolls_from_right(tree, node_id) {
                // `0.0 - range`, not `-range`: no `-0.0` for an empty range.
                (0.0 - range, 0.0)
            } else {
                (0.0, range)
            }
        }
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
    let from_right = scrolls_from_right(tree, node_id);
    let track_of =
        |box_extent: f64, visible: f64, content: f64, other_bar: bool, from_end: bool| {
            let reserved = if other_bar { corner } else { 0.0 };
            let track_len = (box_extent - MARGIN * 2.0 - reserved).max(0.0);
            let thumb_len = (track_len * (visible / content))
                .max(MIN_THUMB)
                .min(track_len);
            ScrollbarTrack {
                content: content * scale,
                visible: visible * scale,
                max_scroll: (content - visible).max(0.0) * scale,
                min_scroll: if from_end {
                    0.0 - (content - visible).max(0.0) * scale
                } else {
                    0.0
                },
                track_start: MARGIN * scale,
                track_len: track_len * scale,
                thumb_len: thumb_len * scale,
                thumb_travel: (track_len - thumb_len).max(0.0) * scale,
            }
        };

    Scrollbars {
        vertical: show_vertical
            .then(|| track_of(box_h, visible_h, content_h, show_horizontal, false)),
        horizontal: show_horizontal
            .then(|| track_of(box_w, visible_w, content_w, show_vertical, from_right)),
        ..empty
    }
}
