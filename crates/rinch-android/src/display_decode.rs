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

// ── The system-theme readings (#572) ──────────────────────────────────────
//
// Three more getters, split the same way and for the same reason. Each is a
// JNI call welded to a decode: fetch `Configuration.uiMode` then mask it, fetch
// a `WallpaperColors` then unpack an ARGB `int`, resolve a resource id then
// unpack the same `int`. The calls are unbindable off-device; the decodes are
// the contract, and they are arithmetic.

/// `android.content.res.Configuration`'s night-mode field.
///
/// Named here rather than fetched as static fields over JNI because they are
/// `public static final int` in the platform API — the compiler inlines them
/// into every app ever built against Android, so they cannot change without
/// breaking the world, and three JNI round-trips to look up numbers frozen by
/// ABI buy nothing.
const UI_MODE_NIGHT_MASK: i32 = 0x30;
const UI_MODE_NIGHT_NO: i32 = 0x10;
const UI_MODE_NIGHT_YES: i32 = 0x20;

/// Decode `Configuration.uiMode` into "is the system in night mode".
///
/// `Some(true)` for dark, `Some(false)` for light, **`None` for undecided** —
/// which is a real third answer rather than a failure code.
/// `UI_MODE_NIGHT_UNDEFINED` (`0x00`) is what the field carries before anything
/// has decided, and some OEM skins and TV/automotive UI modes leave it that way
/// for the life of the process. An app that folds `None` into "light" has
/// hard-coded a preference where the platform said it had none.
///
/// **The mask is the whole of the decode and the easiest thing to lose.**
/// `uiMode` packs the night bits at `0x30` alongside `UI_MODE_TYPE_*` in the
/// low nibble, so a real value is never `0x20` on its own — it is `0x21` on a
/// handset (`UI_MODE_TYPE_NORMAL | UI_MODE_NIGHT_YES`). Dropping the mask and
/// comparing the raw field still answers correctly for the bare constants,
/// which is why every fixture below carries a type bit.
pub fn decode_night_mode(ui_mode: i32) -> Option<bool> {
    match ui_mode & UI_MODE_NIGHT_MASK {
        UI_MODE_NIGHT_YES => Some(true),
        UI_MODE_NIGHT_NO => Some(false),
        // `UI_MODE_NIGHT_UNDEFINED`, or a value from a future mask this build
        // has never heard of. Both mean "the platform did not answer".
        _ => None,
    }
}

/// Unpack a packed sRGB colour — `Color.toArgb()` or `Resources.getColor` —
/// into `(r, g, b)`, dropping alpha.
///
/// **One spelling for two callers.** `wallpaper_primary` and `system_accent`
/// both end in this identical unpack, and this crate has paid for the same
/// arithmetic living at two sites before; #476, #518 and #568 in `rinch-dom`
/// were each two sites asking one question two ways.
///
/// Alpha is dropped rather than returned because both sources are opaque by
/// construction: a wallpaper's primary colour and a framework palette entry are
/// colours, not compositing instructions, and a caller that wanted to blend
/// would be blending against something this function cannot see.
///
/// **What cannot be pinned here, said rather than implied.** The three `& 0xff`
/// masks are redundant with the `as u8` truncation that follows them — for any
/// input, `((argb >> 16) & 0xff) as u8` and `(argb >> 16) as u8` are the same
/// byte — so no fixture can distinguish a build with them from one without. The
/// masks stay because they say what is meant, not because a test holds them.
/// The **shift amounts and their order** are what the tests below bind, and
/// those are where a real defect would live.
pub fn decode_argb(argb: i32) -> (u8, u8, u8) {
    (
        ((argb >> 16) & 0xff) as u8,
        ((argb >> 8) & 0xff) as u8,
        (argb & 0xff) as u8,
    )
}

/// The published Material You accent tones, **in the order the framework's
/// resource table lists them**.
///
/// Public, and asserted in order by a fixture, so that "in the order the
/// resource table lists them" is a checked claim rather than a comment. A
/// membership test alone would leave the ordering inert — `contains` does not
/// care — and the sentence would then be one of the confident-but-unbound sort
/// this repository keeps finding.
///
/// Dumping `/system/framework/framework-res.apk` on a moto g stylus 5G
/// (SDK 33) with `aapt2` lists exactly these thirteen for each of
/// `system_accent1`, `system_accent2`, `system_accent3` and the two neutral
/// ramps, and nothing in between: there is no `system_accent1_550`.
pub const ACCENT_TONES: [u16; 13] = [0, 10, 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];

/// The framework resource name for one Material You accent tone, or `None` if
/// that tone was never published.
///
/// Rejecting an unpublished tone here rather than letting `getIdentifier`
/// answer `0` is not an optimisation: it is the difference between two failures
/// that deserve different treatment. A tone nobody ever published is a caller's
/// mistake; a tone that is published but absent is a device below API 31, and
/// only the second should reach JNI at all.
///
/// The name itself is half the contract — `system_accent1_500`, not
/// `system_accent_500` or `system_accent2_500` — and a typo in it is invisible
/// on a host build and answers `None` on every device. It is pinned exactly.
pub fn system_accent_resource_name(tone: u16) -> Option<String> {
    ACCENT_TONES
        .contains(&tone)
        .then(|| format!("system_accent1_{tone}"))
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

    // ── The system-theme decodes (#572) ───────────────────────────────────

    /// **Every fixture carries a `UI_MODE_TYPE_NORMAL` bit** (`0x01`), which is
    /// what a real handset reports and what a bare constant does not.
    ///
    /// This is the fixed point the obvious fixture sits on: with `0x20` and
    /// `0x10` alone, a decode that dropped the mask entirely and compared the
    /// raw field still answers correctly, because the bare constants *are* the
    /// masked values. `0x21` and `0x11` separate them.
    #[test]
    fn night_mode_reads_the_masked_field_and_not_the_raw_one() {
        assert_eq!(
            decode_night_mode(0x21),
            Some(true),
            "UI_MODE_TYPE_NORMAL | UI_MODE_NIGHT_YES is what a dark handset reports"
        );
        assert_eq!(
            decode_night_mode(0x11),
            Some(false),
            "and this is the light one — the type bit is why the mask is load-bearing"
        );
    }

    /// Widening or narrowing the mask is the other way to get it wrong, and
    /// each is caught by a different one of the two fixtures above: a mask of
    /// `0x3f` breaks the dark case (`0x21 & 0x3f` matches neither constant), a
    /// mask of `0x20` breaks the light one (`0x11 & 0x20` is zero). Asserted
    /// here directly so the pairing is stated rather than incidental.
    #[test]
    fn night_mode_masks_exactly_the_two_night_bits() {
        assert_eq!(0x21 & UI_MODE_NIGHT_MASK, UI_MODE_NIGHT_YES);
        assert_eq!(0x11 & UI_MODE_NIGHT_MASK, UI_MODE_NIGHT_NO);
        assert_eq!(
            0x0f & UI_MODE_NIGHT_MASK,
            0,
            "nothing in the type nibble may reach the night comparison"
        );
    }

    /// `UI_MODE_NIGHT_UNDEFINED` is a third answer, not a failure. A caller
    /// told `Some(false)` here has been handed a preference the platform
    /// explicitly declined to express.
    #[test]
    fn an_undecided_ui_mode_is_none_rather_than_light() {
        assert_eq!(
            decode_night_mode(0x01),
            None,
            "normal type, night undefined"
        );
        assert_eq!(decode_night_mode(0x00), None);
    }

    /// Both night bits set is not a value the platform publishes, so it is a
    /// mask this build has never heard of — which degrades to "no answer"
    /// rather than to whichever arm happens to be first.
    #[test]
    fn an_unknown_night_value_is_none_rather_than_a_guess() {
        assert_eq!(decode_night_mode(0x31), None);
    }

    /// **Three distinct components, and an alpha distinct from all of them.**
    ///
    /// The grey fixture is the trap here: at `0x808080` every byte-order defect
    /// — red and blue swapped, all three read from the same shift — is
    /// invisible. These channels are pairwise different, so any transposition
    /// shows, and alpha is `0x12`, which matches none of them, so a decode that
    /// returned alpha for any channel is visible too.
    #[test]
    fn argb_unpacks_red_green_blue_in_that_order_and_drops_alpha() {
        assert_eq!(decode_argb(0x12_34_56_78), (0x34, 0x56, 0x78));
        assert_ne!(
            decode_argb(0x12_34_56_78),
            (0x78, 0x56, 0x34),
            "red is the high byte of the three, not the low one"
        );
    }

    /// The realistic case, and the one that exercises the sign bit: every
    /// **opaque** colour has alpha `0xff`, which makes the packed `i32`
    /// negative, and `>>` on a negative `i32` is an arithmetic shift that feeds
    /// ones down from the top.
    ///
    /// A wallpaper primary or a palette entry is always opaque, so this is the
    /// ordinary input rather than an edge case — and a decode written with
    /// `u32` semantics in mind would be wrong on all of it.
    #[test]
    fn an_opaque_colour_survives_the_arithmetic_shift() {
        // 0xFF_11_22_33 as a signed i32 is negative.
        let argb = 0xFF_11_22_33_u32 as i32;
        assert!(argb < 0, "precondition: an opaque colour is a negative i32");
        assert_eq!(decode_argb(argb), (0x11, 0x22, 0x33));
    }

    /// Black and white, where a defect that returns a constant or loses a
    /// channel can still look plausible.
    #[test]
    fn the_extremes_decode_to_themselves() {
        assert_eq!(decode_argb(0xFF_00_00_00_u32 as i32), (0x00, 0x00, 0x00));
        assert_eq!(decode_argb(0xFF_FF_FF_FF_u32 as i32), (0xff, 0xff, 0xff));
    }

    /// **The published tone list, in order** — which is what makes the doc
    /// comment's "in the order the resource table lists them" a checked claim.
    ///
    /// `system_accent_resource_name` uses `contains`, which does not care about
    /// order, so without this assertion an off-by-one *in the table* — `50`
    /// written as `60`, or two entries transposed — would be caught only where
    /// it changed membership, and a transposition never does.
    #[test]
    fn the_accent_tone_table_is_the_published_set_in_the_published_order() {
        assert_eq!(
            ACCENT_TONES,
            [0, 10, 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000]
        );
    }

    /// Every published tone is accepted, and the **name format is pinned
    /// exactly**. A slip to `system_accent2_` or `system_accent_` compiles, is
    /// invisible on a host build, and answers `None` on every device — a defect
    /// with no symptom but a missing colour.
    #[test]
    fn every_published_tone_resolves_to_its_framework_resource_name() {
        for tone in ACCENT_TONES {
            assert_eq!(
                system_accent_resource_name(tone),
                Some(format!("system_accent1_{tone}")),
                "tone {tone} is published and must resolve"
            );
        }
        // **Spelled out as well as generated, and the reason is not the
        // obvious one.** The loop above is not self-referential about the
        // *format* — it writes its own `system_accent1_` literal, so it does
        // bind a slip to `system_accent2_`. It **is** self-referential about
        // the **table**: it iterates `ACCENT_TONES`, which is the very list the
        // function guards on, so a collapsed table makes it vacuous rather than
        // red. Measured — against a table mutated to `[0; 13]` the loop alone
        // passes and only the ordering fixture fires; with these three it fires
        // too.
        assert_eq!(
            system_accent_resource_name(0).as_deref(),
            Some("system_accent1_0")
        );
        assert_eq!(
            system_accent_resource_name(500).as_deref(),
            Some("system_accent1_500")
        );
        assert_eq!(
            system_accent_resource_name(1000).as_deref(),
            Some("system_accent1_1000")
        );
    }

    /// **Near misses, not just far ones.** Rejecting `1234` proves nothing a
    /// membership test could get wrong; the tones that matter are the ones
    /// adjacent to real entries, where an off-by-one in the table would show.
    /// `550` is named in the doc comment as the canonical example of a tone
    /// that looks published and is not.
    #[test]
    fn an_unpublished_tone_is_rejected_before_it_reaches_jni() {
        for tone in [1u16, 5, 20, 49, 51, 150, 550, 999, 1001] {
            assert_eq!(
                system_accent_resource_name(tone),
                None,
                "tone {tone} was never published and must not reach getIdentifier"
            );
        }
    }
}
