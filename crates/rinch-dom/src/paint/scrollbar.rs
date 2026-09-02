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
use crate::computed_style::{OverflowValue, ScrollbarWidthValue};
use peniko::color::{AlphaColor, Srgb};

/// How thick a thumb is drawn, in logical pixels.
pub const THICKNESS: f64 = 6.0;
/// How thick a `--rinch-scrollbar-width: thin` thumb is, in logical pixels.
pub const THIN_THICKNESS: f64 = 4.0;
/// The gap between a thumb and the container's edge, in logical pixels. Also
/// the gap at each end of the track.
pub const MARGIN: f64 = 2.0;
/// A thumb is never drawn shorter than this, however little of the content is
/// visible — otherwise a long document's thumb shrinks to a point.
pub const MIN_THUMB: f64 = 20.0;
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
    /// less a [`MARGIN`] at each end, less [`THICKNESS`] + [`MARGIN`] at the
    /// far end when the other bar is up — that reserved square is the corner
    /// neither bar claims.
    pub track_len: f64,
    /// How long the thumb is, never below [`MIN_THUMB`].
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
}

/// The content extent `(width, height)` along both axes, relative to the
/// container's content box, from its direct children's layout rects.
///
/// Taffy's `child.layout.{x,y}` are relative to the **parent's border box**, so
/// they include the leading padding and border; subtracting that offset is what
/// keeps a padded container from deciding it overflows when it does not.
///
/// In logical pixels — this is layout's own unit, and every caller either wants
/// it that way or scales the result itself.
pub fn content_extents(tree: &NodeTree, node_id: usize) -> (f64, f64) {
    let Some(node) = tree.get(node_id) else {
        return (0.0, 0.0);
    };
    let cs = &node.computed_style;
    let content_left = (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64;
    let content_top = (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64;
    let (mut width, mut height) = (0.0_f64, 0.0_f64);
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let right = (child.layout.x + child.layout.width) as f64 - content_left;
            if right > width {
                width = right;
            }
            let bottom = (child.layout.y + child.layout.height) as f64 - content_top;
            if bottom > height {
                height = bottom;
            }
        }
    }
    (width, height)
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
pub fn scrollbars(tree: &NodeTree, node_id: usize, scale: f64) -> Scrollbars {
    let Some(node) = tree.get(node_id) else {
        return Scrollbars {
            vertical: None,
            horizontal: None,
            thickness: THICKNESS * scale,
            margin: MARGIN * scale,
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
        thumb_color: cs
            .scrollbar_color
            .thumb
            .unwrap_or_else(|| auto_thumb_color(cs.color)),
        track_color: cs.scrollbar_color.track,
    };

    let scrollable_y = matches!(cs.overflow_y, OverflowValue::Scroll | OverflowValue::Auto);
    let scrollable_x = matches!(cs.overflow_x, OverflowValue::Scroll | OverflowValue::Auto);
    if (!scrollable_y && !scrollable_x) || cs.scrollbar_width == ScrollbarWidthValue::None {
        return empty;
    }

    let (content_w, content_h) = content_extents(tree, node_id);
    let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64;
    let border_v = (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64;
    let pad_h = (cs.padding_left.to_px() + cs.padding_right.to_px()) as f64;
    let border_h = (cs.border_left_width.to_px() + cs.border_right_width.to_px()) as f64;
    let box_w = node.layout.width as f64;
    let box_h = node.layout.height as f64;
    let visible_w = (box_w - pad_h - border_h).max(0.0);
    let visible_h = (box_h - pad_v - border_v).max(0.0);

    let show_vertical = scrollable_y && content_h > visible_h;
    let show_horizontal = scrollable_x && content_w > visible_w;

    // Where both bars are up, each track gives up the other bar's footprint at
    // its far end so the two thumbs cannot pile into the same square. Nothing
    // is painted there, and hit-testing gives the corner to neither bar.
    let corner = thickness + MARGIN;
    let track_of = |box_extent: f64, visible: f64, content: f64, other_bar: bool| {
        let reserved = if other_bar { corner } else { 0.0 };
        let track_len = (box_extent - MARGIN * 2.0 - reserved).max(0.0);
        let thumb_len = (track_len * (visible / content)).max(MIN_THUMB);
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
