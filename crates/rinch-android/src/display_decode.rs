//! The Rust half of `RinchActivity`'s display-getter contract, on the host
//! side of the JNI boundary.
//!
//! **Why this is a module and not four lines inside `display.rs`.** A
//! JNI getter is two things welded together: a call that only a device can
//! make, and a *decode* that turns the primitive the call returned into the
//! answer the caller was promised. The call is unbindable off-device and
//! always will be. The decode is ordinary arithmetic on an `i32`, and it is
//! where the whole of each getter's contract lives — which of `-1` and `0` is
//! "no answer" and which is "the answer is nothing", which axis is width, what
//! makes a size implausible.
//!
//! Leaving the decode welded to the call put the contract behind
//! `#[cfg(target_os = "android")]`, and issue #516 is the record of what that
//! costs: for Android-only paths CI answers "does it compile" and never "does
//! it work". Measured on this feature's first draft — four one-character
//! mutants applied to the decode as it stood inside the getters, each run
//! against every gate this repository has:
//!
//! | mutant | host `check` | host `test` | `--target aarch64-linux-android` |
//! |---|---|---|---|
//! | `px >= 0` → `px > 0` (the sentinel collapse) | clean | pass | clean |
//! | `(buf[0], buf[1])` → `(buf[1], buf[0])` (axes swapped) | clean | pass | clean |
//! | size gate `&&` → `\|\|` | clean | pass | clean |
//! | JNI descriptor `"()[I"` → `"()[J"` | clean | pass | clean |
//!
//! Four survivors out of four. The first is the sharp one, because
//! `tests/java_contract_tests.rs` *does* pin the matching Java detail — that
//! `getImeInset` answers `-1` and not `0` when there are no insets yet. So the
//! contract had a test on the Java side of the boundary and none on the Rust
//! side, and one character in this file was enough to reintroduce, in the
//! decode, precisely the defect the Java test exists to prevent. **A contract
//! test on one side of a foreign-function boundary does not bind the other
//! side's implementation of it.** Both halves have to be pinned, or the rule is
//! written down twice and checked once.
//!
//! **This module is deliberately not gated at all** — not
//! `#[cfg(target_os = "android")]`, and not the `#[cfg(any(target_os =
//! "android", test))]` that `location` and `sensors` use for their host-testable
//! halves. `display.rs` is `cfg(android)` *whole*, so moving a pure function
//! into it changes nothing about what CI can see; and the `any(android, test)`
//! shape is invisible to `cargo check`, which is the three-configuration trap
//! recorded on #516. `scoped.rs` has always had this shape. Everything here
//! calls only `core`, so there is nothing to gate.

/// Decode `RinchActivity.getImeInset()`'s return value.
///
/// The Java side answers **`-1` for "no insets to report yet"** and **`0` for
/// "the keyboard is down"**, and those are opposite instructions to a caller
/// that moves its layout with the keyboard: the first says leave it where it
/// is, the second says put it back. A window that has not been laid out is an
/// ordinary moment in an activity's life — it happens on the way into every
/// screen — so the sentinel is on the common path, not an error path.
///
/// Hence `>= 0` and not `> 0`. `Some(0)` is a real answer and has to survive
/// the decode; every negative is the sentinel, so an unexpected `-2` from a
/// future Java revision degrades to "ask again" rather than to a `u32` cast of
/// a negative number.
pub fn decode_ime_inset(px: i32) -> Option<u32> {
    (px >= 0).then_some(px as u32)
}

/// Decode `RinchActivity.getViewportSize()`'s `int[] { width, height }`.
///
/// Order is `[width, height]`, matching the Java literal, and both axes must be
/// positive. A zero or negative dimension is not a small window, it is a window
/// that was not there to measure — the activity asked before it had one.
/// `None` sends the caller to its own fallback; dividing by it would send the
/// caller somewhere much stranger.
///
/// Both axes, not either: a `0 x 2460` is exactly as unusable as a `0 x 0`, and
/// the caller wants a size it can divide by in both directions.
pub fn decode_viewport_size(raw: [i32; 2]) -> Option<(u32, u32)> {
    let [width, height] = raw;
    (width > 0 && height > 0).then_some((width as u32, height as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0` is an answer and `-1` is the absence of one, and the decode has to
    /// keep them apart. This is the fixture that fails against `px > 0`, which
    /// is the mutant that survived every gate before this module existed.
    #[test]
    fn the_ime_sentinel_is_not_the_same_answer_as_a_closed_keyboard() {
        assert_eq!(
            decode_ime_inset(0),
            Some(0),
            "0 is `the keyboard is down`, which is an answer — a caller told \
             `None` here would leave a footer raised over a keyboard that is \
             not there"
        );
        assert_eq!(
            decode_ime_inset(-1),
            None,
            "-1 is `no insets yet`, which is not an answer"
        );
    }

    /// The value a moto g stylus 5G reported with the keyboard up (SDK 33),
    /// off both the zero and the sentinel so neither branch can carry it.
    #[test]
    fn an_open_keyboard_decodes_to_its_own_height() {
        assert_eq!(decode_ime_inset(885), Some(885));
    }

    /// Every negative is the sentinel, not just the documented one — a `-2`
    /// from a future Java revision must not become `4294967294`.
    #[test]
    fn any_negative_ime_inset_is_the_sentinel_and_never_a_cast() {
        assert_eq!(decode_ime_inset(-2), None);
        assert_eq!(decode_ime_inset(i32::MIN), None);
    }

    /// Deliberately **non-square**, and the real handset's numbers: at
    /// 1080x2460 a swapped decode answers 2460x1080, which is wrong, whereas at
    /// any square fixture the swap is invisible. This is the fixture that fails
    /// against `(buf[1], buf[0])`.
    #[test]
    fn a_viewport_keeps_width_first_and_height_second() {
        assert_eq!(decode_viewport_size([1080, 2460]), Some((1080, 2460)));
        assert_ne!(
            decode_viewport_size([1080, 2460]),
            Some((2460, 1080)),
            "the axes are `[width, height]`, matching the Java literal"
        );
    }

    /// One axis zero at a time, because a fixture with *both* zero passes
    /// against `||` as happily as against `&&`. These are the fixtures that
    /// fail against the loosened gate.
    #[test]
    fn a_viewport_with_either_axis_unmeasured_is_no_viewport() {
        assert_eq!(
            decode_viewport_size([0, 2460]),
            None,
            "a zero width is a window that was not there to measure"
        );
        assert_eq!(
            decode_viewport_size([1080, 0]),
            None,
            "a zero height is too — and one axis at a time is what separates \
             `&&` from `||`"
        );
        assert_eq!(decode_viewport_size([0, 0]), None);
    }

    /// A negative dimension is the same fault as a zero one, and must not be
    /// cast.
    #[test]
    fn a_negative_viewport_dimension_is_rejected_rather_than_cast() {
        assert_eq!(decode_viewport_size([-1, -1]), None);
        assert_eq!(decode_viewport_size([-1, 2460]), None);
        assert_eq!(decode_viewport_size([1080, -1]), None);
    }
}
