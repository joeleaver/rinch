//! Box edges on whole device pixels (#2368).
//!
//! At a fractional scale factor a box's edges fall between device pixels: at
//! 125% a box at `left: 13px` starts at device x 16.25. Chrome draws it from
//! 16 (`PhysicalRect` → `ToPixelSnappedRect`): every edge of the border box,
//! and the padding-box clip rect derived from it, is rounded to the nearest
//! device pixel, a tie (150%: 19.5) going up. Measured in Chrome 158
//! (headless, software raster) at 1.25x and 1.5x: a background, a 1px and a
//! 3px border, an `overflow: hidden` clip with and without a radius, and a clip
//! nested in a clip all have hard edges where rinch drew a one-pixel blend
//! (63 against 0 at 1.25x; two blended edges, a clip's and its child's,
//! multiplied to 143 where Chrome has 255).
//!
//! Three rules, each from that measurement:
//!
//! - **Each edge rounds on its own** ([`snap_edge`]): left and right, top and
//!   bottom, so a box's snapped width is `round(x + w) - round(x)`, not
//!   `round(w)`, and two boxes that touch still touch.
//! - **From the accumulated position, not the parent's snapped one.** A box
//!   at 1px inside a box at 13px is at 17.5 device px at 125% and starts at
//!   18, where snapping the parent first (16) and adding 1.25 would give 17.
//!   Paint therefore hands children the unsnapped origin; only what a box
//!   draws for itself is snapped.
//! - **In the box's own space, before its transform.** A box under
//!   `translateX(0.4px)` at 125% (half a device pixel) is snapped to 16 and
//!   then moved to 16.5, so its edge blends (127) as Chrome's does: a
//!   transform is never snapped. Snapping in device space would have given a
//!   hard edge at 17.
//!
//! Border widths are already whole device pixels: Stylo rounds a computed
//! border width down to the device grid (a 3px border is 3 device px at
//! 125%, as in Chrome), so a padding box inset from a snapped border box is
//! on the grid too.
//!
//! What paint snaps, what reaches the snapped box, and what stays where it
//! is: `docs/internals/paint-and-damage.md` ("Box edges on device pixels").

use peniko::kurbo::{Affine, Rect};

/// A box edge (physical px, in the box's own untransformed space) on the
/// nearest device pixel, a tie rounding up, as Chrome's `LayoutUnit::Round`
/// does (`-1.5` goes to `-1`, where `f64::round` would give `-2`). A
/// millionth of a device pixel of slack keeps an edge that is a tie up to
/// float error (a position summed from a parent's, `13 * 1.5 + 0.0`) on the
/// same side as the exact tie.
#[inline]
pub fn snap_edge(v: f64) -> f64 {
    (v + 0.5 + 1e-6).floor()
}

/// The border box at `(x, y)`, `w` x `h` physical px, with each edge on the
/// device grid ([`snap_edge`]): what paint draws a box's background, borders,
/// shadows and outline from, and replaced content into.
#[inline]
pub fn snapped_box(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect::new(
        snap_edge(x),
        snap_edge(y),
        snap_edge(x + w),
        snap_edge(y + h),
    )
}

/// `r` with each edge on the device grid.
#[inline]
pub fn snapped_rect(r: Rect) -> Rect {
    Rect::new(
        snap_edge(r.x0),
        snap_edge(r.y0),
        snap_edge(r.x1),
        snap_edge(r.y1),
    )
}

/// The box at `(x, y)`, `w` x `h` and its snapped self together: what a box
/// can cover however it is drawn, for the culls, layer bounds and damage that
/// must contain it. Snapping moves an edge by at most half a device pixel, so
/// in device space this changes nothing once a rect is rounded out to whole
/// pixels; under a transform that scales, the half pixel is scaled too and
/// can reach a pixel the unsnapped box does not touch.
#[inline]
pub fn box_reach(x: f64, y: f64, w: f64, h: f64) -> Rect {
    reach_of(Rect::new(x, y, x + w, y + h))
}

/// [`box_reach`] of a rect: `r` and its snapped self together. Rounding is
/// monotonic, so for a rect that bounds several untransformed boxes this
/// also bounds each of their snapped selves.
#[inline]
pub fn reach_of(r: Rect) -> Rect {
    r.union(snapped_rect(r))
}

/// How far a box edge can move when paint snaps it: half a device pixel, in
/// the box's own space before its transform ([`snap_edge`]).
pub const SNAP_SLACK: f64 = 0.5;

/// The snap gain ([`crate::node::SubtreeReach::Within`]) of a box whose own
/// space reaches the measured space through `t`, when its own extent can
/// grow by `inner` times `d` (`[1.0, 1.0]` for a box alone): a rect grown by `d` per edge in
/// the box's space has a bounding box under `t` grown by `d * |a| + d * |c|`
/// in x and `d * |b| + d * |d|` in y (`t`'s linear part `[a c; b d]`), so a
/// box whose own extent grows by `d * inner` grows by this times `d`.
pub fn snap_gain_through(t: Affine, inner: [f64; 2]) -> [f64; 2] {
    let [a, b, c, d, _, _] = t.as_coeffs();
    [
        a.abs() * inner[0] + c.abs() * inner[1],
        b.abs() * inner[0] + d.abs() * inner[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_round_to_the_nearest_device_pixel_ties_up() {
        assert_eq!(snap_edge(16.25), 16.0);
        assert_eq!(snap_edge(16.75), 17.0);
        assert_eq!(snap_edge(19.5), 20.0);
        assert_eq!(snap_edge(-1.5), -1.0);
        assert_eq!(snap_edge(-1.75), -2.0);
        assert_eq!(snap_edge(-0.25), 0.0);
        assert_eq!(snap_edge(7.0), 7.0);
        // A tie a few ulps short of exact is still a tie.
        assert_eq!(snap_edge(19.5 - 1e-12), 20.0);
    }

    #[test]
    fn each_edge_snaps_on_its_own() {
        // 13px wide at 1.25x from 16.25: 16.25..32.5 → 16..33, 17 wide, where
        // rounding the width alone (16.25 → 16) would end at 32.
        let r = snapped_box(16.25, 11.25, 16.25, 37.5);
        assert_eq!(r, Rect::new(16.0, 11.0, 33.0, 49.0));
    }

    #[test]
    fn the_snap_gain_follows_the_transform() {
        assert_eq!(snap_gain_through(Affine::IDENTITY, [1.0, 1.0]), [1.0, 1.0]);
        assert_eq!(
            snap_gain_through(Affine::scale_non_uniform(12.0, 3.0), [1.0, 2.0]),
            [12.0, 6.0]
        );
        // A quarter turn swaps the axes; a translation adds nothing.
        let turn = Affine::translate((40.0, 7.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let g = snap_gain_through(turn, [2.0, 5.0]);
        assert!(
            (g[0] - 5.0).abs() < 1e-9 && (g[1] - 2.0).abs() < 1e-9,
            "{g:?}"
        );
    }

    #[test]
    fn the_reach_holds_both_the_box_and_its_snapped_self() {
        let r = box_reach(16.25, 11.75, 50.0, 37.0);
        assert_eq!(r, Rect::new(16.0, 11.75, 66.25, 49.0));
        let on_grid = box_reach(16.0, 11.0, 50.0, 37.0);
        assert_eq!(on_grid, Rect::new(16.0, 11.0, 66.0, 48.0));
    }
}
