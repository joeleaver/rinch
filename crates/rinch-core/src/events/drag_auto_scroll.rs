//! Auto-scroll during an element drag: the speed curve both backends share.
//!
//! While a `draggable="true"` drag is live and the pointer rests within
//! [`DRAG_AUTO_SCROLL_EDGE`] of the top or bottom edge of a scroll container
//! under it, that container scrolls, faster the closer the pointer is to the
//! edge, for as long as the pointer stays there. Desktop drives it from its
//! frame clock (`AboutToWait`), rinch-web from `requestAnimationFrame`; both
//! ask this module how fast. A drop target under a still pointer hears
//! `data-ondragover` again after every frame that scrolled, since what is
//! under the pointer has changed.

/// How far in from a scroll container's edge the band that scrolls it reaches,
/// in logical (CSS) pixels. A container shorter than three bands gets bands a
/// third of its height each, so its middle never scrolls.
pub const DRAG_AUTO_SCROLL_EDGE: f32 = 32.0;

/// Scroll speed with the pointer on the edge itself (or past it), in logical
/// pixels per second. It falls off with the square of the distance into the
/// band: half way in is a quarter of this.
pub const DRAG_AUTO_SCROLL_MAX_SPEED: f32 = 1200.0;

/// The longest frame one tick scrolls for, in seconds. A frame clock that
/// stalls (the window was hidden, a long layout) must not turn into one jump.
pub const DRAG_AUTO_SCROLL_MAX_STEP: f32 = 0.05;

/// The frame a tick assumes when it has no previous one to measure from (the
/// first frame in a band), in seconds.
pub const DRAG_AUTO_SCROLL_FIRST_STEP: f32 = 1.0 / 60.0;

/// The scroll velocity along one axis for a pointer at `pointer` over a
/// container whose visible extent on that axis is `start..end`, in logical
/// pixels per second: negative toward `start` (scroll up), positive toward
/// `end` (scroll down), zero outside both bands.
///
/// It says how fast the content *would* move, not whether it can: a container
/// already at its top is the caller's to skip.
pub fn drag_auto_scroll_velocity(pointer: f32, start: f32, end: f32) -> f32 {
    let len = end - start;
    if len.is_nan() || len <= 0.0 || !pointer.is_finite() {
        return 0.0;
    }
    let band = DRAG_AUTO_SCROLL_EDGE.min(len / 3.0);
    let speed = |into_band: f32| {
        let depth = ((band - into_band) / band).clamp(0.0, 1.0);
        DRAG_AUTO_SCROLL_MAX_SPEED * depth * depth
    };
    let from_start = pointer - start;
    let from_end = end - pointer;
    if from_start < band {
        -speed(from_start)
    } else if from_end < band {
        speed(from_end)
    } else {
        0.0
    }
}

/// How long a tick at `elapsed` seconds since the last one scrolls for:
/// [`DRAG_AUTO_SCROLL_FIRST_STEP`] with no last tick, else `elapsed` capped at
/// [`DRAG_AUTO_SCROLL_MAX_STEP`].
pub fn drag_auto_scroll_step(elapsed: Option<f32>) -> f32 {
    match elapsed {
        None => DRAG_AUTO_SCROLL_FIRST_STEP,
        Some(e) => e.clamp(0.0, DRAG_AUTO_SCROLL_MAX_STEP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_does_not_scroll() {
        assert_eq!(drag_auto_scroll_velocity(150.0, 0.0, 300.0), 0.0);
        assert_eq!(drag_auto_scroll_velocity(32.0, 0.0, 300.0), 0.0);
        assert_eq!(drag_auto_scroll_velocity(268.0, 0.0, 300.0), 0.0);
    }

    #[test]
    fn the_bands_scroll_toward_their_edge_faster_near_it() {
        let near_top = drag_auto_scroll_velocity(102.0, 100.0, 400.0);
        let into_top = drag_auto_scroll_velocity(120.0, 100.0, 400.0);
        assert!(
            near_top < into_top && into_top < 0.0,
            "{near_top} {into_top}"
        );
        let near_bottom = drag_auto_scroll_velocity(398.0, 100.0, 400.0);
        let into_bottom = drag_auto_scroll_velocity(380.0, 100.0, 400.0);
        assert!(near_bottom > into_bottom && into_bottom > 0.0);
        assert_eq!(near_top, -near_bottom, "the two bands are mirror images");
    }

    #[test]
    fn on_or_past_the_edge_is_full_speed() {
        assert_eq!(
            drag_auto_scroll_velocity(0.0, 0.0, 300.0),
            -DRAG_AUTO_SCROLL_MAX_SPEED
        );
        assert_eq!(
            drag_auto_scroll_velocity(310.0, 0.0, 300.0),
            DRAG_AUTO_SCROLL_MAX_SPEED
        );
    }

    #[test]
    fn a_short_container_keeps_a_still_middle() {
        // 60px tall: bands of 20px, not 32 (which would overlap).
        assert_eq!(drag_auto_scroll_velocity(30.0, 0.0, 60.0), 0.0);
        assert!(drag_auto_scroll_velocity(10.0, 0.0, 60.0) < 0.0);
        assert!(drag_auto_scroll_velocity(50.0, 0.0, 60.0) > 0.0);
    }

    #[test]
    fn nothing_to_scroll_in_is_still() {
        assert_eq!(drag_auto_scroll_velocity(0.0, 10.0, 10.0), 0.0);
        assert_eq!(drag_auto_scroll_velocity(f32::NAN, 0.0, 100.0), 0.0);
    }

    #[test]
    fn a_stalled_clock_is_one_capped_step() {
        assert_eq!(drag_auto_scroll_step(None), DRAG_AUTO_SCROLL_FIRST_STEP);
        assert_eq!(drag_auto_scroll_step(Some(0.01)), 0.01);
        assert_eq!(drag_auto_scroll_step(Some(3.0)), DRAG_AUTO_SCROLL_MAX_STEP);
    }
}
