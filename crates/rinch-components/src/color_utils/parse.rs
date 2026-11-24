//! Parsing CSS colour strings.

use super::convert::{hex_to_rgb, hsl_to_hsv, rgb_to_hsv};
use super::named::named_to_rgb;
use super::types::{Hsla, Hsva, Notation, Rgba};

/// Parse a CSS color string into HSVA.
///
/// Supports: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (with or without the
/// `#`), `rgb()`/`rgba()` and `hsl()`/`hsla()` in both the legacy comma
/// syntax and the modern space syntax (`rgb(51 51 102 / 0.5)`), and the CSS
/// named colours (`red`, `rebeccapurple`, `transparent`). Keywords and
/// function names are matched case-insensitively, as in CSS.
///
/// Out-of-range channels clamp to their CSS ranges (rgb 0–255, percentages
/// 0–100%, alpha 0–1); hue wraps into [0, 360).
pub fn parse_color(s: &str) -> Option<Hsva> {
    parse_color_with_notation(s).map(|(colour, _)| colour)
}

/// [`parse_color`], also reporting the [`Notation`] the string was written
/// in: [`Notation::Hex`] for hex (with or without the `#`) and the named
/// colours, [`Notation::Rgb`] for `rgb()`/`rgba()`, [`Notation::Hsl`] for
/// `hsl()`/`hsla()`.
///
/// The notation names the grid this crate's own serializer writes the family
/// at (whole degrees and percents for hsl, 8-bit channels for hex and rgb) —
/// the parser itself accepts finer channels. A consumer judging whether an
/// inbound string denotes a colour compares at that grid, in the inbound
/// string's own notation (GH #242): see [`denotes_emitted`](super::denotes_emitted) and the
/// `value_fn` gate in `color_picker.rs`.
///
/// `parse_color` is defined *through* this function, so the two dispatch
/// identically by construction: there is one classifier, and the notation
/// it reports is the branch that actually parsed the string.
pub fn parse_color_with_notation(s: &str) -> Option<(Hsva, Notation)> {
    let s = s.trim();
    // A keyword names an 8-bit colour, so its notation is hex's.
    let eight_bit = |rgb: Rgba| (rgb_to_hsv(rgb), Notation::Hex);
    if s.starts_with('#') {
        return hex_to_rgb(s).map(eight_bit);
    }
    if let Some(inner) = strip_function_ci(s, "rgba").or_else(|| strip_function_ci(s, "rgb")) {
        return parse_rgb_css(inner).map(|colour| (colour, Notation::Rgb));
    }
    if let Some(inner) = strip_function_ci(s, "hsla").or_else(|| strip_function_ci(s, "hsl")) {
        return parse_hsl_css(inner).map(|colour| (colour, Notation::Hsl));
    }
    if let Some(rgb) = named_to_rgb(s) {
        // Before the bare-hex fallback, though no name collides with one: a
        // keyword is a keyword wherever hex would also be legal.
        return Some(eight_bit(rgb));
    }
    // Try as bare hex
    hex_to_rgb(s).map(eight_bit)
}

/// Strip a CSS function wrapper: `name(` from the front — ASCII
/// case-insensitively, since CSS function names are case-insensitive
/// (`RGB(255, 0, 0)` is as valid as `rgb(255, 0, 0)`) — and one `)` from the
/// back. Exactly one of each: repeat-stripping (`trim_start_matches`/
/// `trim_end_matches`) quietly accepted `rgb(rgb(1, 2, 3))` and
/// `rgb(1, 2, 3)))`, which are not colours. The close paren stays optional,
/// as it always has been: `rgb(51, 51, 102` mid-typing previews, and the
/// commit boundary normalizes it.
fn strip_function_ci<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let prefix_len = name.len() + 1;
    if s.len() < prefix_len {
        return None;
    }
    // Byte-wise, not a str slice: `s` flows in per keystroke and may be
    // non-ASCII, where slicing at an arbitrary byte offset panics mid-char.
    let head = &s.as_bytes()[..prefix_len];
    if !head[..name.len()].eq_ignore_ascii_case(name.as_bytes()) || head[name.len()] != b'(' {
        return None;
    }
    // The matched prefix bytes are ASCII, so this offset is a char boundary.
    let tail = &s[prefix_len..];
    Some(tail.strip_suffix(')').unwrap_or(tail))
}

/// Split the inside of an `rgb()`/`hsl()` function into its three channel
/// tokens plus optional alpha token, accepting exactly one of the two CSS
/// syntaxes: legacy commas (`51, 51, 102, 0.5`) or modern spaces with a
/// slash before the alpha (`51 51 102 / 0.5`). Mixtures, missing channels,
/// and surplus parts are not a colour.
fn split_function_args(inner: &str) -> Option<(Vec<&str>, Option<&str>)> {
    if inner.contains(',') {
        let mut parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        // A part still holding whitespace after the trim is a mixture
        // ("51, 51 102, 3") — f64's FromStr rejects it downstream.
        if parts.len() < 3 || parts.len() > 4 {
            return None;
        }
        let alpha = if parts.len() == 4 { parts.pop() } else { None };
        Some((parts, alpha))
    } else {
        let (channels, alpha) = match inner.split_once('/') {
            Some((c, a)) => {
                let a = a.trim();
                if a.is_empty() || a.contains('/') {
                    return None;
                }
                (c, Some(a))
            }
            None => (inner, None),
        };
        let parts: Vec<&str> = channels.split_whitespace().collect();
        if parts.len() != 3 {
            return None;
        }
        Some((parts, alpha))
    }
}

/// Parse one channel token: a number clamped into [0, `max`] and scaled into
/// [0, 1]. Every channel of every function syntax funnels through here, so
/// the two ordering invariants live in one place:
///
/// - f64::FromStr accepts "nan"/"inf": a NaN channel would poison the
///   picker's signals (every comparison involving NaN is false, so a
///   value_fn apply of such a string re-applies forever). Not a colour.
/// - `clamp` alone would pass NaN through, so the finite check comes first.
///
/// Clamping in the parser (GH #243): everything downstream — the picker's
/// signals, the thumb positions, the #241 merge and echo gates — assumes
/// canonical CSS ranges, and CSS itself clamps out-of-range channels.
fn parse_channel(token: &str, max: f64) -> Option<f64> {
    let v = token.parse::<f64>().ok()?;
    v.is_finite().then(|| v.clamp(0.0, max) / max)
}

/// An alpha token: a number 0–1 or a percentage, clamped into [0, 1].
fn parse_alpha(token: &str) -> Option<f64> {
    match token.strip_suffix('%') {
        Some(p) => parse_channel(p, 100.0),
        None => parse_channel(token, 1.0),
    }
}

/// The inside of an `rgb()`/`rgba()` wrapper (see [`strip_function_ci`]).
fn parse_rgb_css(inner: &str) -> Option<Hsva> {
    let (channels, alpha) = split_function_args(inner)?;
    let r = parse_channel(channels[0], 255.0)?;
    let g = parse_channel(channels[1], 255.0)?;
    let b = parse_channel(channels[2], 255.0)?;
    let a = match alpha {
        Some(t) => parse_alpha(t)?,
        None => 1.0,
    };
    Some(rgb_to_hsv(Rgba { r, g, b, a }))
}

/// The inside of an `hsl()`/`hsla()` wrapper (see [`strip_function_ci`]).
fn parse_hsl_css(inner: &str) -> Option<Hsva> {
    let (channels, alpha) = split_function_args(inner)?;
    let h = channels[0].parse::<f64>().ok()?;
    if !h.is_finite() {
        // Before `rem_euclid`, which maps ±inf/NaN to NaN — see
        // `parse_channel` for why NaN must never reach the signals.
        return None;
    }
    // Hue's range discipline is wrapping, not clamping: hsl(-30, …) means
    // 330°. `hsv_to_rgb` already wraps at render (#235); wrapping here too
    // keeps the raw parse out of the hue *signal*, where -30 would put the
    // hue thumb at a negative offset (GH #243).
    let h = h.rem_euclid(360.0);
    // rem_euclid can land on exactly 360.0: for a vanishingly negative hue,
    // `h + 360.0` rounds up to 360.0 in f64 (-1e-14 does). Snap the boundary
    // so the documented [0, 360) contract — and the hue thumb — hold.
    let h = if h >= 360.0 { 0.0 } else { h };
    // At most one '%' (`strip_suffix`, not `trim_end_matches`): "50%%" is
    // not a percentage. The bare-number form stays accepted, as it always
    // has been.
    let percent = |t: &str| parse_channel(t.strip_suffix('%').unwrap_or(t), 100.0);
    let s_val = percent(channels[1])?;
    let l = percent(channels[2])?;
    let a = match alpha {
        Some(t) => parse_alpha(t)?,
        None => 1.0,
    };
    Some(hsl_to_hsv(Hsla { h, s: s_val, l, a }))
}
