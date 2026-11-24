//! Whether colour strings denote the same colour, at a format's or a
//! notation's resolution.

use super::convert::{format_color, hsv_to_hsl, hsv_to_rgb};
use super::parse::{parse_color, parse_color_with_notation};
use super::types::{ColorFormat, Hsva, Notation};

/// What a text field shows for `value` under the display `format`: the
/// colour re-spelled in that format, or the text itself when it parses as no
/// colour (an unfinished or garbage value has no other spelling).
///
/// This is the one rule behind `ColorInput`'s field (GH #237): the mount
/// value, a dropdown pick, a typed commit and an external `value_fn` write
/// all reach the field's colour in whatever notation they arrived in, and the
/// field is a pure display-format view of it. [`denotes_same`] — the field's
/// write-back guard — is defined through this same spelling, so what the
/// guard judges by and what the field is written are structurally the same
/// grid; a store speaking `hsl()` under `format: "hex"` cannot move the
/// colour while the field keeps a stale spelling.
pub fn display_spelling(value: &str, format: ColorFormat) -> String {
    parse_color(value)
        .map(|c| format_color(c, format))
        .unwrap_or_else(|| value.to_string())
}

/// Whether two colour strings denote the same colour when expressed in
/// `format`.
///
/// Equality of the two [`display_spelling`]s on the trimmed pair: both parse
/// → their `format_color` renderings compare, so every notation difference
/// `format` erases compares equal (short vs long hex, digit case, an alpha
/// pair a non-alpha format drops, an `rgb()` spelling under a hex format);
/// an unparseable side keeps its own text, which no canonical rendering can
/// equal, so two copies of the same unfinished text still match and nothing
/// else does.
///
/// This is `ColorInput`'s write-back guard (GH #231): a field whose text
/// already denotes the colour about to be displayed is the author's to keep,
/// mid-keystroke text included. `ColorPicker`'s guard and its `value_fn`
/// apply gate (GH #227) judge at the inbound text's own notation instead —
/// [`text_denotes`] / [`denotes_emitted`] — so nothing the display format
/// drops is folded away.
pub fn denotes_same(a: &str, b: &str, format: ColorFormat) -> bool {
    let (a, b) = (a.trim(), b.trim());
    // Identical text trivially denotes the same colour — and this is the
    // guard's steady state (the effect re-rendering the string it last
    // wrote), so skip the parse/format round entirely.
    a == b || display_spelling(a, format) == display_spelling(b, format)
}

/// Whether `parsed`, read from a string written in `notation`, denotes the
/// colour that `emitted` — a string this crate's serializer produced — spells,
/// judged at `notation`'s own grid (GH #242): 8-bit channels for hex and
/// rgb, whole degrees and whole percents for hsl, alpha at two decimals
/// under `rgb()`/`hsl()`. Every channel counts — the notation is always the
/// alpha-carrying one.
///
/// `emitted` is re-parsed rather than compared as text because format →
/// parse → format is not a fixed point everywhere: an alpha in [0.995, 1)
/// formats as `1.00`, which re-parses opaque and re-formats without the
/// alpha, and a store that normalises the emission spells it the second way.
/// Both sides go through the same parse → format, so the two spellings meet.
/// This is the one predicate behind [`text_denotes`] and `ColorPicker`'s
/// `value_fn` apply gate; the gate calls it directly with the parse it
/// already holds.
pub fn denotes_emitted(parsed: Hsva, notation: Notation, emitted: &str) -> bool {
    let format = notation.with_alpha();
    parse_color(emitted).is_some_and(|e| format_color(parsed, format) == format_color(e, format))
}

/// Whether `parsed`, read from a string written in `notation`, is a spelling
/// of `emitted` — a string this crate's serializer produced — that another
/// serializer could have written: [`denotes_emitted`], or else every channel
/// within half a grid step of `emitted`'s exact value on `notation`'s grid
/// (GH #262).
///
/// A store that re-spells an emission in another notation lands on the grid
/// point nearest the emission's exact value, the same point rinch's own
/// serializer lands on — except where that value sits on a tie between two
/// points, which each converter may round its own way (`#797e81`'s hue is
/// exactly 202.5°; an exact-rational converter writes 203 where rinch writes
/// 202). Accepting any rounding of the exact value accepts both neighbours
/// of a tie and nothing else: off a tie, half a step past the exact value is
/// still short of the next grid point. A value one grid point from rinch's
/// spelling that is *not* a rounding of the exact value — a peer's genuine
/// 1° move where the exact hue is 206⅔° and rinch writes 207 — is judged
/// foreign, exactly as [`denotes_emitted`] judges it.
///
/// The tolerance only does anything across notations. When `emitted` is
/// already spelled in `notation` its exact value *is* a grid point, whose one
/// rounding is itself, and this reduces to [`denotes_emitted`] — which is why
/// `ColorPicker`'s held-colour arm, whose string is spelled in the inbound
/// notation, keeps calling [`denotes_emitted`]: there is nothing for a store
/// to have rounded differently.
pub fn respells_emitted(parsed: Hsva, notation: Notation, emitted: &str) -> bool {
    if denotes_emitted(parsed, notation, emitted) {
        return true;
    }
    let Some(exact) = parse_color(emitted) else {
        return false;
    };
    // A float slack far below any genuine gap: an 8-bit colour's hsl channels
    // are rationals whose distance from a half step is at least 1/1020 of a
    // step, and an integer hsl colour's 8-bit channels at least 5e-5 of a step
    // (e.g. `hsl(7, 59%, 29%)`'s red is 40.49995) — both far above this slack.
    const HALF_STEP: f64 = 0.5 + 1e-6;
    let within = |a: f64, b: f64| (a - b).abs() <= HALF_STEP;
    let on_grid = |c: Hsva| -> [f64; 4] {
        match notation {
            Notation::Hex | Notation::Rgb => {
                let rgb = hsv_to_rgb(c);
                let alpha_steps = if notation == Notation::Hex {
                    255.0
                } else {
                    100.0
                };
                [
                    rgb.r * 255.0,
                    rgb.g * 255.0,
                    rgb.b * 255.0,
                    rgb.a * alpha_steps,
                ]
            }
            Notation::Hsl => {
                let hsl = hsv_to_hsl(c);
                [hsl.h, hsl.s * 100.0, hsl.l * 100.0, hsl.a * 100.0]
            }
        }
    };
    let (p, e) = (on_grid(parsed), on_grid(exact));
    let first_matches = if notation == Notation::Hsl {
        // Hue is circular: 359.6° and 0° are 0.4° apart. (Defensive: unreachable
        // for 8-bit emissions today, since rinch spells an exact 359.5° as
        // `hsl(0, …)` — no test witnesses this branch.)
        let d = (p[0] - e[0]).rem_euclid(360.0);
        d.min(360.0 - d) <= HALF_STEP
    } else {
        within(p[0], e[0])
    };
    first_matches && within(p[1], e[1]) && within(p[2], e[2]) && within(p[3], e[3])
}

/// Whether `text` denotes the colour `colour` holds — every channel, alpha
/// included — at the resolution of the notation `text` is written in (see
/// [`denotes_emitted`]).
///
/// This is `ColorPicker`'s write-back guard comparison (GH #231): unlike
/// [`denotes_same`] under the display format, it does **not** fold away
/// channels the format cannot express — a typed `#3333666c` agrees with the
/// picker while the picker's alpha really is `6c`, and stops agreeing the
/// moment the alpha slider moves it, so the field is rewritten exactly when
/// the colour leaves the text behind. Judging at the text's own notation is
/// what keeps the field in step with the apply gate (GH #242): a typed
/// `hsl(205, 3%, 49%)` stops agreeing when a peer moves the hue to 200°,
/// though both render `#797e81`. Unparseable text denotes nothing.
pub fn text_denotes(text: &str, colour: Hsva) -> bool {
    parse_color_with_notation(text).is_some_and(|(parsed, notation)| {
        denotes_emitted(
            parsed,
            notation,
            &format_color(colour, notation.with_alpha()),
        )
    })
}
