//! A popup's placement as winit's `WindowPositioner` (#2552), pure:
//! `xdg_positioner`'s anchor, gravity, offset and constraint adjustment from
//! [`Placement`], [`Adjust`] and the anchor rectangle.
//!
//! - `BOTTOM_START` is anchor bottom-left with gravity bottom-right (below,
//!   left edges aligned); `BOTTOM_CENTER` anchor bottom, gravity bottom;
//!   `RIGHT_START` anchor top-right, gravity bottom-right (a submenu).
//! - [`Adjust`] is `WindowConstraintAdjustment` bit for bit.

use rinch_core::element::{Adjust, Align, Placement, Side};
use winit::dpi::{LogicalPosition, LogicalSize, Position, Size};
use winit::window::{WindowAnchor, WindowConstraintAdjustment, WindowGravity, WindowPositioner};

/// The anchor point and gravity for `placement`.
pub(crate) fn anchor_and_gravity(placement: Placement) -> (WindowAnchor, WindowGravity) {
    use Align::*;
    use Side::*;
    use WindowAnchor as A;
    use WindowGravity as G;
    match (placement.side, placement.align) {
        (Bottom, Start) => (A::BottomLeft, G::BottomRight),
        (Bottom, Center) => (A::Bottom, G::Bottom),
        (Bottom, End) => (A::BottomRight, G::BottomLeft),
        (Top, Start) => (A::TopLeft, G::TopRight),
        (Top, Center) => (A::Top, G::Top),
        (Top, End) => (A::TopRight, G::TopLeft),
        (Right, Start) => (A::TopRight, G::BottomRight),
        (Right, Center) => (A::Right, G::Right),
        (Right, End) => (A::BottomRight, G::TopRight),
        (Left, Start) => (A::TopLeft, G::BottomLeft),
        (Left, Center) => (A::Left, G::Left),
        (Left, End) => (A::BottomLeft, G::TopLeft),
    }
}

/// `adjust` as winit's constraint adjustment: the same bits.
pub(crate) fn constraint_adjustment(adjust: Adjust) -> WindowConstraintAdjustment {
    WindowConstraintAdjustment::from_bits_truncate(u32::from(adjust.bits()))
}

/// The positioner for a popup placed by `placement` against `anchor`
/// (logical px in its parent: x, y, width, height), nudged by `offset`
/// (logical px, rounded), adjusted as `adjust` allows.
pub(crate) fn positioner(
    placement: Placement,
    adjust: Adjust,
    anchor: [i32; 4],
    offset: (f32, f32),
) -> WindowPositioner {
    let (a, g) = anchor_and_gravity(placement);
    let [x, y, w, h] = anchor;
    WindowPositioner::new(
        a,
        (
            Position::Logical(LogicalPosition::new(f64::from(x), f64::from(y))),
            Size::Logical(LogicalSize::new(f64::from(w.max(1)), f64::from(h.max(1)))),
        ),
        Position::Logical(LogicalPosition::new(
            f64::from(offset.0).round(),
            f64::from(offset.1).round(),
        )),
        g,
        constraint_adjustment(adjust),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every placement's anchor and gravity, as `xdg_positioner` reads them:
    /// the popup hangs from the anchor point and grows in the gravity's
    /// direction.
    #[test]
    fn each_placement_maps_to_its_anchor_and_gravity() {
        use WindowAnchor as A;
        use WindowGravity as G;
        let cases = [
            (Placement::BOTTOM_START, A::BottomLeft, G::BottomRight),
            (Placement::BOTTOM_CENTER, A::Bottom, G::Bottom),
            (Placement::BOTTOM_END, A::BottomRight, G::BottomLeft),
            (Placement::TOP_START, A::TopLeft, G::TopRight),
            (Placement::TOP_CENTER, A::Top, G::Top),
            (Placement::TOP_END, A::TopRight, G::TopLeft),
            (Placement::RIGHT_START, A::TopRight, G::BottomRight),
            (Placement::RIGHT_CENTER, A::Right, G::Right),
            (Placement::RIGHT_END, A::BottomRight, G::TopRight),
            (Placement::LEFT_START, A::TopLeft, G::BottomLeft),
            (Placement::LEFT_CENTER, A::Left, G::Left),
            (Placement::LEFT_END, A::BottomLeft, G::TopLeft),
        ];
        for (placement, anchor, gravity) in cases {
            assert_eq!(
                anchor_and_gravity(placement),
                (anchor, gravity),
                "{placement:?}"
            );
        }
    }

    /// `Adjust` is winit's constraint adjustment bit for bit, each flag
    /// alone and together.
    #[test]
    fn adjust_is_the_constraint_adjustment_bit_for_bit() {
        use WindowConstraintAdjustment as W;
        let cases = [
            (Adjust::SLIDE_X, W::SLIDE_X),
            (Adjust::SLIDE_Y, W::SLIDE_Y),
            (Adjust::FLIP_X, W::FLIP_X),
            (Adjust::FLIP_Y, W::FLIP_Y),
            (Adjust::RESIZE_X, W::RESIZE_X),
            (Adjust::RESIZE_Y, W::RESIZE_Y),
            (Adjust::NONE, W::empty()),
            (
                Adjust::default(),
                W::FLIP_X | W::FLIP_Y | W::SLIDE_X | W::SLIDE_Y,
            ),
            (Adjust::FIT, W::all()),
            (
                Adjust::FLIP_Y | Adjust::SLIDE_X | Adjust::RESIZE_Y,
                W::FLIP_Y | W::SLIDE_X | W::RESIZE_Y,
            ),
        ];
        for (adjust, want) in cases {
            assert_eq!(constraint_adjustment(adjust), want, "{adjust:?}");
        }
    }

    /// The positioner carries the anchor rectangle and the offset in
    /// logical px, the offset rounded (a negative one for a submenu kept).
    #[test]
    fn the_positioner_carries_the_rect_and_the_offset() {
        let p = positioner(
            Placement::RIGHT_START,
            Adjust::default(),
            [10, 32, 120, 24],
            (0.0, -4.4),
        );
        assert_eq!(p.anchor, WindowAnchor::TopRight);
        assert_eq!(p.gravity, WindowGravity::BottomRight);
        assert_eq!(
            p.anchor_rect,
            (
                Position::Logical(LogicalPosition::new(10.0, 32.0)),
                Size::Logical(LogicalSize::new(120.0, 24.0))
            )
        );
        assert_eq!(p.offset, Position::Logical(LogicalPosition::new(0.0, -4.0)));
    }
}
