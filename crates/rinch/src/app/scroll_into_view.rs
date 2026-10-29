//! Scrolling a box into view: every scrolling ancestor, innermost first (#842).
//!
//! What CSSOM View's "scroll an element into view" does, and so what the
//! browser's `scrollIntoView({block: "nearest", inline: "nearest"})` — the
//! call `rinch-web` makes for the same request — does:
//!
//! - **Every** ancestor that scrolls is adjusted, innermost first and the page
//!   scroller (`<body>`, the UA sheet's `overflow-y: auto`) last. Each one
//!   brings the target's painted box, *as the scrollers inside it have just
//!   placed it*, into its scrollport. Desktop used to move the nearest
//!   `overflow-y` scroller and stop, so a target in a scroller below the fold
//!   of a scrolling page stayed below the fold.
//! - What an outer scroller brings into view is the target **clipped to the
//!   scrollports it is in**, as Chrome does (`ScrollRectToVisible` intersects
//!   the rect with each scroller it leaves): not the scroller around the
//!   target, whose box may be taller than the window.
//! - A scrollport is the **padding box** (`clientTop`/`clientHeight`): a
//!   padded scroller used to be measured from its border-box top against its
//!   content-box height and moved by its top padding for nothing (#1367).
//! - Both axes, each `nearest`: an edge already inside moves nothing, a box
//!   larger than the scrollport that covers it moves nothing, otherwise the
//!   near edge is aligned (the start edge for a box larger than the scrollport
//!   that sticks out past the end).
//! - The walk stops above a `position: fixed` box: the viewport holds it, and
//!   no scroller outside it can move it.
//!
//! A placed request (`NodeHandle::scroll_to_fraction`) places the target in
//! the nearest scroller that scrolls vertically and is then `nearest` in every
//! other scroller and on the other axis — what the web backend does by setting
//! that scroller's `scrollTop` and then calling `scrollIntoView`.
use peniko::kurbo::Rect;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::PositionValue;

/// How far inside a scrollport an edge may stray and still be called inside:
/// layout's float sums put an edge that is exactly at the scrollport's edge a
/// hair past it, and the device-pixel round-up below would turn that hair into
/// a whole pixel of scroll.
const EDGE_SLOP: f64 = 1e-3;

/// Bring `target` into view of every scroller it is in. `placement` is
/// `Some((fraction, margin))` for a placed request. Returns whether any
/// scroller moved.
pub(super) fn reveal(d: &mut RinchDocument, target: u64, placement: Option<(f32, f32)>) -> bool {
    let mut moved = false;
    let mut placement = placement;
    // The scrollers already adjusted: their scrollports clip what the next one
    // out brings into view.
    let mut passed: Vec<u64> = Vec::new();
    let mut below = target;
    while let Some(node) = d.tree.nodes.get(below) {
        // Nothing outside a fixed box can move it.
        if node.box_position() == PositionValue::Fixed {
            break;
        }
        let Some(scroller) = node.parent else { break };
        below = scroller;
        let Some(s) = d.tree.nodes.get(scroller) else {
            break;
        };
        // `scrolls_*`, not `overflow_*`: a `display: contents` ancestor has no
        // box to scroll (#1038).
        let (on_x, on_y) = (s.scrolls_x(), s.scrolls_y());
        if !on_x && !on_y {
            continue;
        }
        let mut rect = rinch_dom::paint::painted_border_box(&d.tree, target, 1.0);
        for &inner in &passed {
            rect = rect.intersect(scrollport_in_window(d, inner));
        }
        let place = if on_y { placement.take() } else { None };
        match scroll_one(d, scroller, rect, (on_x, on_y), place) {
            Some(changed) => moved |= changed,
            // A non-invertible transform on the chain: the subtree paints to
            // zero area, so no scroll position makes it visible.
            None => break,
        }
        passed.push(scroller);
    }
    moved
}

/// `scroller`'s scrollport — its padding box — where it is painted, in window
/// space.
fn scrollport_in_window(d: &RinchDocument, scroller: u64) -> Rect {
    let (left, top, width, height) = scrollport(d, scroller);
    let (x, y, w, h) =
        rinch_dom::paint::rect_from_painted_box(&d.tree, scroller, 1.0, (left, top, width, height));
    Rect::new(x, y, x + w, y + h)
}

/// `scroller`'s padding box in its own border-box space: `(left, top, width,
/// height)`.
fn scrollport(d: &RinchDocument, scroller: u64) -> (f64, f64, f64, f64) {
    let cs = &d.tree.nodes[scroller].computed_style;
    let id = NodeId(scroller);
    (
        f64::from(cs.border_left_width.to_px()),
        f64::from(cs.border_top_width.to_px()),
        d.client_width(id)
            + f64::from(cs.padding_left.to_px())
            + f64::from(cs.padding_right.to_px()),
        d.client_height(id)
            + f64::from(cs.padding_top.to_px())
            + f64::from(cs.padding_bottom.to_px()),
    )
}

/// Adjust one scroller so `rect` (window space) is in its scrollport. `None`
/// when `rect` cannot be mapped into the scroller's space; otherwise whether
/// its offset changed.
fn scroll_one(
    d: &mut RinchDocument,
    scroller: u64,
    rect: Rect,
    (on_x, on_y): (bool, bool),
    placement: Option<(f32, f32)>,
) -> Option<bool> {
    // The rect in the scroller's own space, from its border-box origin at the
    // current scroll: the space `scroll_offset` moves content in. A scroll of
    // N moves a painted box by N there, at any scale between the two (paint
    // composes a descendant's transform about the descendant's own origin,
    // which the scroll moves too), so the adjustments below are read straight
    // off it — `a_scaled_subtree_scrolls_by_the_painted_distance`. A rotated
    // chain gives the bounding box, as `painted_border_box` does everywhere.
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (px, py) in [
        (rect.x0, rect.y0),
        (rect.x1, rect.y0),
        (rect.x0, rect.y1),
        (rect.x1, rect.y1),
    ] {
        let (lx, ly) = rinch_dom::paint::point_in_painted_box(&d.tree, scroller, 1.0, px, py)?;
        x0 = x0.min(lx);
        y0 = y0.min(ly);
        x1 = x1.max(lx);
        y1 = y1.max(ly);
    }

    let (left, top, width, height) = scrollport(d, scroller);
    let id = NodeId(scroller);
    let (cur_x, cur_y) = d.tree.nodes[scroller].scroll_offset;
    let max_x = (d.scroll_width(id) - d.client_width(id)).max(0.0);
    let max_y = (d.scroll_height(id) - d.client_height(id)).max(0.0);

    let new_x = if on_x {
        cur_x + nearest(x0 - left, x1 - left, width)
    } else {
        cur_x
    };
    let new_y = if !on_y {
        cur_y
    } else if let Some((fraction, margin)) = placement {
        // The target's top at `fraction` of the scrollport's height, at least
        // `margin` inside either edge (the top edge wins when the target is
        // too tall for both).
        let (fraction, margin) = (f64::from(fraction), f64::from(margin));
        let at = (fraction * height)
            .min(height - margin - (y1 - y0))
            .max(margin);
        cur_y + (y0 - top) - at
    } else {
        cur_y + nearest(y0 - top, y1 - top, height)
    };

    let scale = d.tree.device_scale;
    let new_x = settle(new_x, cur_x, max_x, scale);
    let new_y = settle(new_y, cur_y, max_y, scale);
    if new_x == cur_x && new_y == cur_y {
        return Some(false);
    }
    d.tree.nodes[scroller].scroll_offset = (new_x, new_y);
    // A scroll moves every box inside the scroller: repaint it, as the wheel
    // does (which also tells the hit cache).
    d.tree.mark_scrolled(scroller);
    Some(true)
}

/// The scroll delta that brings the edges `start..end` (from the scrollport's
/// start) into a scrollport `len` long, CSSOM View's `nearest`.
fn nearest(start: f64, end: f64, len: f64) -> f64 {
    let before = start < -EDGE_SLOP;
    let after = end > len + EDGE_SLOP;
    let larger = end - start > len;
    match (before, after) {
        // Inside, or covering the whole scrollport: nothing to gain.
        (false, false) | (true, true) => 0.0,
        (true, false) if !larger => start,
        (false, true) if larger => start,
        _ => end - len,
    }
}

/// Clamp a new offset to `0..=max`. `mark_scrolled` floors the offset to the
/// device pixel grid (#1543); scrolling forward to reveal a box, a floor would
/// stop up to a device pixel short of its far edge, so that one is rounded up
/// — as far as the range allows, where the floor takes over again.
fn settle(new: f64, current: f64, max: f64, scale: f64) -> f64 {
    let mut clamped = new.clamp(0.0, max);
    if clamped > current && scale.is_finite() && scale > 0.0 {
        clamped = ((clamped * scale - 1e-6).ceil() / scale).clamp(clamped, max);
    }
    clamped
}
