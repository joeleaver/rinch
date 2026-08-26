//! Color conversion utilities for the color picker components.
//!
//! Provides HSV/RGB/Hex/HSL conversions with alpha support.

/// HSV color with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    /// Hue: 0-360
    pub h: f64,
    /// Saturation: 0-1
    pub s: f64,
    /// Value: 0-1
    pub v: f64,
    /// Alpha: 0-1
    pub a: f64,
}

/// RGB color with alpha (all channels 0-1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// HSL color with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsla {
    /// Hue: 0-360
    pub h: f64,
    /// Saturation: 0-1
    pub s: f64,
    /// Lightness: 0-1
    pub l: f64,
    /// Alpha: 0-1
    pub a: f64,
}

/// Output format for color strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFormat {
    Hex,
    Hexa,
    Rgb,
    Rgba,
    Hsl,
    Hsla,
}

impl ColorFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "hex" => Some(ColorFormat::Hex),
            "hexa" => Some(ColorFormat::Hexa),
            "rgb" => Some(ColorFormat::Rgb),
            "rgba" => Some(ColorFormat::Rgba),
            "hsl" => Some(ColorFormat::Hsl),
            "hsla" => Some(ColorFormat::Hsla),
            _ => None,
        }
    }
}

/// Convert HSV to RGB.
pub fn hsv_to_rgb(hsv: Hsva) -> Rgba {
    // rem_euclid, not `%`: a negative hue is valid CSS ("hsl(-30, …)" means
    // 330°), but truncating `%` keeps the sign, driving `x` negative and
    // picking the wrong sextant below.
    let h = hsv.h.rem_euclid(360.0);
    let s = hsv.s;
    let v = hsv.v;

    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    Rgba {
        r: r1 + m,
        g: g1 + m,
        b: b1 + m,
        a: hsv.a,
    }
}

/// Convert RGB to HSV.
pub fn rgb_to_hsv(rgb: Rgba) -> Hsva {
    let r = rgb.r;
    let g = rgb.g;
    let b = rgb.b;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };

    let s = if max == 0.0 { 0.0 } else { delta / max };
    let v = max;

    Hsva { h, s, v, a: rgb.a }
}

/// Convert HSV to HSL.
pub fn hsv_to_hsl(hsv: Hsva) -> Hsla {
    let l = hsv.v * (1.0 - hsv.s / 2.0);
    let s = if l == 0.0 || l == 1.0 {
        0.0
    } else {
        (hsv.v - l) / l.min(1.0 - l)
    };

    Hsla {
        h: hsv.h,
        s,
        l,
        a: hsv.a,
    }
}

/// Convert HSL to HSV.
pub fn hsl_to_hsv(hsl: Hsla) -> Hsva {
    let v = hsl.l + hsl.s * hsl.l.min(1.0 - hsl.l);
    let s = if v == 0.0 {
        0.0
    } else {
        2.0 * (1.0 - hsl.l / v)
    };

    Hsva {
        h: hsl.h,
        s,
        v,
        a: hsl.a,
    }
}

/// Convert RGB (0-1 channels) to hex string (#rrggbb or #rrggbbaa).
pub fn rgb_to_hex(rgb: Rgba, include_alpha: bool) -> String {
    let r = (rgb.r * 255.0).round() as u8;
    let g = (rgb.g * 255.0).round() as u8;
    let b = (rgb.b * 255.0).round() as u8;

    if include_alpha && rgb.a < 1.0 {
        let a = (rgb.a * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a)
    } else {
        format!("#{:02x}{:02x}{:02x}", r, g, b)
    }
}

/// Parse a hex color string (#rgb, #rgba, #rrggbb, #rrggbbaa).
pub fn hex_to_rgb(hex: &str) -> Option<Rgba> {
    let hex = hex.trim().trim_start_matches('#');
    // `len()` counts bytes and the arms below byte-slice: non-ASCII input
    // whose byte length happens to be 3, 4, 6, or 8 ("#é3") would panic on a
    // char boundary. Hex digits are ASCII, so anything else is simply not a
    // colour. This runs on every keystroke of the colour fields.
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        // #rgb / #rgba (GH #243): each digit doubles, alpha included — the
        // CSS Color 4 short forms ("3" → 0x33, i.e. digit × 17).
        3 | 4 => {
            let d = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
            Some(Rgba {
                r: d(0)? as f64 / 255.0,
                g: d(1)? as f64 / 255.0,
                b: d(2)? as f64 / 255.0,
                a: if hex.len() == 4 {
                    d(3)? as f64 / 255.0
                } else {
                    1.0
                },
            })
        }
        6 | 8 => {
            let d = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            Some(Rgba {
                r: d(0)? as f64 / 255.0,
                g: d(2)? as f64 / 255.0,
                b: d(4)? as f64 / 255.0,
                a: if hex.len() == 8 {
                    d(6)? as f64 / 255.0
                } else {
                    1.0
                },
            })
        }
        _ => None,
    }
}

/// Format an RGBA value as a CSS rgb()/rgba() string.
pub fn rgba_to_css(rgb: Rgba) -> String {
    let r = (rgb.r * 255.0).round() as u8;
    let g = (rgb.g * 255.0).round() as u8;
    let b = (rgb.b * 255.0).round() as u8;
    if rgb.a < 1.0 {
        format!("rgba({}, {}, {}, {:.2})", r, g, b, rgb.a)
    } else {
        format!("rgb({}, {}, {})", r, g, b)
    }
}

/// Format an HSLA value as a CSS hsl()/hsla() string.
pub fn hsla_to_css(hsl: Hsla) -> String {
    // Round, then wrap: a hue in [359.5, 360) rounds to 360, and emitting
    // "hsl(360, …)" would desync emit from parse — `parse_color` wraps hue
    // into [0, 360), so the emitted string would re-parse as hue 0, format
    // back as "hsl(0, …)", and `denotes_same` under an hsl display format
    // would call two spellings of the same colour different (rewriting the
    // field under the author's caret — the GH #231 class). Wrapping on the
    // emit side keeps `format_color` a fixed point of `parse_color`.
    let h = hsl.h.round().rem_euclid(360.0) as i32;
    let s = (hsl.s * 100.0).round() as i32;
    let l = (hsl.l * 100.0).round() as i32;
    if hsl.a < 1.0 {
        format!("hsla({}, {}%, {}%, {:.2})", h, s, l, hsl.a)
    } else {
        format!("hsl({}, {}%, {}%)", h, s, l)
    }
}

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
    let s = s.trim();
    if s.starts_with('#') {
        return hex_to_rgb(s).map(rgb_to_hsv);
    }
    if let Some(inner) = strip_function_ci(s, "rgba").or_else(|| strip_function_ci(s, "rgb")) {
        return parse_rgb_css(inner);
    }
    if let Some(inner) = strip_function_ci(s, "hsla").or_else(|| strip_function_ci(s, "hsl")) {
        return parse_hsl_css(inner);
    }
    if let Some(rgb) = named_to_rgb(s) {
        // Before the bare-hex fallback, though no name collides with one: a
        // keyword is a keyword wherever hex would also be legal.
        return Some(rgb_to_hsv(rgb));
    }
    // Try as bare hex
    hex_to_rgb(s).map(rgb_to_hsv)
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

/// Format an HSVA color to a string in the specified format.
pub fn format_color(hsv: Hsva, format: ColorFormat) -> String {
    match format {
        ColorFormat::Hex => {
            let rgb = hsv_to_rgb(hsv);
            rgb_to_hex(rgb, false)
        }
        ColorFormat::Hexa => {
            let rgb = hsv_to_rgb(hsv);
            rgb_to_hex(rgb, true)
        }
        ColorFormat::Rgb => {
            let rgb = hsv_to_rgb(hsv);
            rgba_to_css(Rgba { a: 1.0, ..rgb })
        }
        ColorFormat::Rgba => {
            let rgb = hsv_to_rgb(hsv);
            rgba_to_css(rgb)
        }
        ColorFormat::Hsl => {
            let hsl = hsv_to_hsl(hsv);
            hsla_to_css(Hsla { a: 1.0, ..hsl })
        }
        ColorFormat::Hsla => {
            let hsl = hsv_to_hsl(hsv);
            hsla_to_css(hsl)
        }
    }
}

/// Whether two colour strings denote the same colour when expressed in
/// `format`.
///
/// Both parse → compare their `format_color` renderings, so every notation
/// difference `format` erases compares equal: short vs long hex, digit case,
/// an alpha pair a non-alpha format drops, an `rgb()` spelling under a hex
/// format. Either fails to parse → compare the trimmed strings, so two copies
/// of the same unfinished text still match and nothing else does.
///
/// This is the write-back guard the colour components share (GH #231): a
/// field whose text already denotes the colour about to be displayed is the
/// author's to keep, mid-keystroke text included. Deliberately public — the
/// `value_fn` apply gate (GH #227) reuses the same equivalence, passing
/// [`ColorFormat::Hexa`] so the comparison keeps every channel instead of
/// folding away what the display format drops.
pub fn denotes_same(a: &str, b: &str, format: ColorFormat) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        // Identical text trivially denotes the same colour — and this is the
        // guard's steady state (the effect re-rendering the string it last
        // wrote), so skip the parse/format round entirely.
        return true;
    }
    match (parse_color(a), parse_color(b)) {
        (Some(a), Some(b)) => format_color(a, format) == format_color(b, format),
        _ => false,
    }
}

/// Whether `text` denotes exactly the colour `colour` holds — every channel,
/// alpha included, quantized to 8 bits (the finest any supported notation
/// expresses).
///
/// This is `ColorPicker`'s write-back guard comparison (GH #231): unlike
/// [`denotes_same`] under the display format, it does **not** fold away
/// channels the format cannot express — a typed `#3333666c` agrees with the
/// picker while the picker's alpha really is `6c`, and stops agreeing the
/// moment the alpha slider moves it, so the field is rewritten exactly when
/// the colour leaves the text behind. Unparseable text denotes nothing.
pub fn text_denotes(text: &str, colour: Hsva) -> bool {
    parse_color(text).is_some_and(|parsed| {
        format_color(parsed, ColorFormat::Hexa) == format_color(colour, ColorFormat::Hexa)
    })
}

/// The CSS named colours (CSS Color Level 4 §6.1), matched case-insensitively.
///
/// `transparent` is a colour keyword too: transparent black, the only entry
/// with an alpha. The table is exhaustive — all 148 names, the gray/grey
/// alias pairs included — because a missing name falls to the bare-hex arm
/// and silently "isn't a colour", the exact hole GH #243 closes. No name is
/// also a 3/4/6/8-digit hex string (every candidate length carries a letter
/// past `f`), so keyword-before-hex ordering in `parse_color` decides
/// nothing; it just mirrors CSS, where keywords resolve first.
fn named_to_rgb(name: &str) -> Option<Rgba> {
    if name.len() > 20 || !name.is_ascii() {
        // Longest name: "lightgoldenrodyellow". Bounds the per-keystroke
        // lowercase allocation and skips the match for arbitrary text.
        return None;
    }
    let name = name.to_ascii_lowercase();
    if name == "transparent" {
        return Some(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        });
    }
    let rgb: u32 = match name.as_str() {
        "aliceblue" => 0xf0f8ff,
        "antiquewhite" => 0xfaebd7,
        "aqua" => 0x00ffff,
        "aquamarine" => 0x7fffd4,
        "azure" => 0xf0ffff,
        "beige" => 0xf5f5dc,
        "bisque" => 0xffe4c4,
        "black" => 0x000000,
        "blanchedalmond" => 0xffebcd,
        "blue" => 0x0000ff,
        "blueviolet" => 0x8a2be2,
        "brown" => 0xa52a2a,
        "burlywood" => 0xdeb887,
        "cadetblue" => 0x5f9ea0,
        "chartreuse" => 0x7fff00,
        "chocolate" => 0xd2691e,
        "coral" => 0xff7f50,
        "cornflowerblue" => 0x6495ed,
        "cornsilk" => 0xfff8dc,
        "crimson" => 0xdc143c,
        "cyan" => 0x00ffff,
        "darkblue" => 0x00008b,
        "darkcyan" => 0x008b8b,
        "darkgoldenrod" => 0xb8860b,
        "darkgray" => 0xa9a9a9,
        "darkgreen" => 0x006400,
        "darkgrey" => 0xa9a9a9,
        "darkkhaki" => 0xbdb76b,
        "darkmagenta" => 0x8b008b,
        "darkolivegreen" => 0x556b2f,
        "darkorange" => 0xff8c00,
        "darkorchid" => 0x9932cc,
        "darkred" => 0x8b0000,
        "darksalmon" => 0xe9967a,
        "darkseagreen" => 0x8fbc8f,
        "darkslateblue" => 0x483d8b,
        "darkslategray" => 0x2f4f4f,
        "darkslategrey" => 0x2f4f4f,
        "darkturquoise" => 0x00ced1,
        "darkviolet" => 0x9400d3,
        "deeppink" => 0xff1493,
        "deepskyblue" => 0x00bfff,
        "dimgray" => 0x696969,
        "dimgrey" => 0x696969,
        "dodgerblue" => 0x1e90ff,
        "firebrick" => 0xb22222,
        "floralwhite" => 0xfffaf0,
        "forestgreen" => 0x228b22,
        "fuchsia" => 0xff00ff,
        "gainsboro" => 0xdcdcdc,
        "ghostwhite" => 0xf8f8ff,
        "gold" => 0xffd700,
        "goldenrod" => 0xdaa520,
        "gray" => 0x808080,
        "green" => 0x008000,
        "greenyellow" => 0xadff2f,
        "grey" => 0x808080,
        "honeydew" => 0xf0fff0,
        "hotpink" => 0xff69b4,
        "indianred" => 0xcd5c5c,
        "indigo" => 0x4b0082,
        "ivory" => 0xfffff0,
        "khaki" => 0xf0e68c,
        "lavender" => 0xe6e6fa,
        "lavenderblush" => 0xfff0f5,
        "lawngreen" => 0x7cfc00,
        "lemonchiffon" => 0xfffacd,
        "lightblue" => 0xadd8e6,
        "lightcoral" => 0xf08080,
        "lightcyan" => 0xe0ffff,
        "lightgoldenrodyellow" => 0xfafad2,
        "lightgray" => 0xd3d3d3,
        "lightgreen" => 0x90ee90,
        "lightgrey" => 0xd3d3d3,
        "lightpink" => 0xffb6c1,
        "lightsalmon" => 0xffa07a,
        "lightseagreen" => 0x20b2aa,
        "lightskyblue" => 0x87cefa,
        "lightslategray" => 0x778899,
        "lightslategrey" => 0x778899,
        "lightsteelblue" => 0xb0c4de,
        "lightyellow" => 0xffffe0,
        "lime" => 0x00ff00,
        "limegreen" => 0x32cd32,
        "linen" => 0xfaf0e6,
        "magenta" => 0xff00ff,
        "maroon" => 0x800000,
        "mediumaquamarine" => 0x66cdaa,
        "mediumblue" => 0x0000cd,
        "mediumorchid" => 0xba55d3,
        "mediumpurple" => 0x9370db,
        "mediumseagreen" => 0x3cb371,
        "mediumslateblue" => 0x7b68ee,
        "mediumspringgreen" => 0x00fa9a,
        "mediumturquoise" => 0x48d1cc,
        "mediumvioletred" => 0xc71585,
        "midnightblue" => 0x191970,
        "mintcream" => 0xf5fffa,
        "mistyrose" => 0xffe4e1,
        "moccasin" => 0xffe4b5,
        "navajowhite" => 0xffdead,
        "navy" => 0x000080,
        "oldlace" => 0xfdf5e6,
        "olive" => 0x808000,
        "olivedrab" => 0x6b8e23,
        "orange" => 0xffa500,
        "orangered" => 0xff4500,
        "orchid" => 0xda70d6,
        "palegoldenrod" => 0xeee8aa,
        "palegreen" => 0x98fb98,
        "paleturquoise" => 0xafeeee,
        "palevioletred" => 0xdb7093,
        "papayawhip" => 0xffefd5,
        "peachpuff" => 0xffdab9,
        "peru" => 0xcd853f,
        "pink" => 0xffc0cb,
        "plum" => 0xdda0dd,
        "powderblue" => 0xb0e0e6,
        "purple" => 0x800080,
        "rebeccapurple" => 0x663399,
        "red" => 0xff0000,
        "rosybrown" => 0xbc8f8f,
        "royalblue" => 0x4169e1,
        "saddlebrown" => 0x8b4513,
        "salmon" => 0xfa8072,
        "sandybrown" => 0xf4a460,
        "seagreen" => 0x2e8b57,
        "seashell" => 0xfff5ee,
        "sienna" => 0xa0522d,
        "silver" => 0xc0c0c0,
        "skyblue" => 0x87ceeb,
        "slateblue" => 0x6a5acd,
        "slategray" => 0x708090,
        "slategrey" => 0x708090,
        "snow" => 0xfffafa,
        "springgreen" => 0x00ff7f,
        "steelblue" => 0x4682b4,
        "tan" => 0xd2b48c,
        "teal" => 0x008080,
        "thistle" => 0xd8bfd8,
        "tomato" => 0xff6347,
        "turquoise" => 0x40e0d0,
        "violet" => 0xee82ee,
        "wheat" => 0xf5deb3,
        "white" => 0xffffff,
        "whitesmoke" => 0xf5f5f5,
        "yellow" => 0xffff00,
        "yellowgreen" => 0x9acd32,
        _ => return None,
    };
    Some(Rgba {
        r: ((rgb >> 16) & 0xff) as f64 / 255.0,
        g: ((rgb >> 8) & 0xff) as f64 / 255.0,
        b: (rgb & 0xff) as f64 / 255.0,
        a: 1.0,
    })
}

/// Convert a hue value (0-360) to a hex color at full saturation and value.
pub fn hue_to_rgb_hex(hue: f64) -> String {
    let rgb = hsv_to_rgb(Hsva {
        h: hue,
        s: 1.0,
        v: 1.0,
        a: 1.0,
    });
    rgb_to_hex(rgb, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn test_hsv_rgb_roundtrip() {
        let cases = vec![
            Hsva {
                h: 0.0,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }, // red
            Hsva {
                h: 120.0,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }, // green
            Hsva {
                h: 240.0,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }, // blue
            Hsva {
                h: 0.0,
                s: 0.0,
                v: 0.0,
                a: 1.0,
            }, // black
            Hsva {
                h: 0.0,
                s: 0.0,
                v: 1.0,
                a: 1.0,
            }, // white
            Hsva {
                h: 210.0,
                s: 0.5,
                v: 0.8,
                a: 0.5,
            },
        ];

        for hsv in cases {
            let rgb = hsv_to_rgb(hsv);
            let back = rgb_to_hsv(rgb);
            // For achromatic colors, hue is arbitrary
            if hsv.s > 0.01 {
                assert!(
                    approx_eq(hsv.h, back.h),
                    "hue mismatch: {} vs {}",
                    hsv.h,
                    back.h
                );
            }
            assert!(
                approx_eq(hsv.s, back.s),
                "sat mismatch: {} vs {}",
                hsv.s,
                back.s
            );
            assert!(
                approx_eq(hsv.v, back.v),
                "val mismatch: {} vs {}",
                hsv.v,
                back.v
            );
            assert!(
                approx_eq(hsv.a, back.a),
                "alpha mismatch: {} vs {}",
                hsv.a,
                back.a
            );
        }
    }

    #[test]
    fn test_hsv_hsl_roundtrip() {
        let cases = vec![
            Hsva {
                h: 0.0,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            },
            Hsva {
                h: 180.0,
                s: 0.5,
                v: 0.75,
                a: 0.8,
            },
            Hsva {
                h: 0.0,
                s: 0.0,
                v: 0.5,
                a: 1.0,
            },
        ];

        for hsv in cases {
            let hsl = hsv_to_hsl(hsv);
            let back = hsl_to_hsv(hsl);
            if hsv.s > 0.01 {
                assert!(approx_eq(hsv.h, back.h));
            }
            assert!(approx_eq(hsv.s, back.s));
            assert!(approx_eq(hsv.v, back.v));
            assert!(approx_eq(hsv.a, back.a));
        }
    }

    #[test]
    fn test_hex_parsing() {
        // 3-digit hex
        let rgb = hex_to_rgb("#f00").unwrap();
        assert!(approx_eq(rgb.r, 1.0));
        assert!(approx_eq(rgb.g, 0.0));
        assert!(approx_eq(rgb.b, 0.0));

        // 6-digit hex
        let rgb = hex_to_rgb("#00ff00").unwrap();
        assert!(approx_eq(rgb.r, 0.0));
        assert!(approx_eq(rgb.g, 1.0));
        assert!(approx_eq(rgb.b, 0.0));

        // 8-digit hex with alpha
        let rgb = hex_to_rgb("#0000ff80").unwrap();
        assert!(approx_eq(rgb.b, 1.0));
        assert!(approx_eq(rgb.a, 128.0 / 255.0));

        // Invalid
        assert!(hex_to_rgb("#xyz").is_none());
        assert!(hex_to_rgb("#12345").is_none());
    }

    #[test]
    fn test_hex_output() {
        let red = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        assert_eq!(rgb_to_hex(red, false), "#ff0000");
        assert_eq!(rgb_to_hex(red, true), "#ff0000"); // alpha=1.0 omitted

        let semi = Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 0.5,
        };
        assert_eq!(rgb_to_hex(semi, true), "#0000ff80");
    }

    #[test]
    fn test_parse_color_formats() {
        // Hex
        let hsv = parse_color("#ff0000").unwrap();
        assert!(approx_eq(hsv.h, 0.0));
        assert!(approx_eq(hsv.s, 1.0));
        assert!(approx_eq(hsv.v, 1.0));

        // RGB
        let hsv = parse_color("rgb(0, 255, 0)").unwrap();
        assert!(approx_eq(hsv.h, 120.0));

        // RGBA
        let hsv = parse_color("rgba(0, 0, 255, 0.5)").unwrap();
        assert!(approx_eq(hsv.h, 240.0));
        assert!(approx_eq(hsv.a, 0.5));

        // HSL
        let hsv = parse_color("hsl(120, 100%, 50%)").unwrap();
        assert!(approx_eq(hsv.h, 120.0));

        // HSLA
        let hsv = parse_color("hsla(240, 100%, 50%, 0.75)").unwrap();
        assert!(approx_eq(hsv.a, 0.75));
    }

    #[test]
    fn test_format_color() {
        let red = Hsva {
            h: 0.0,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        };
        assert_eq!(format_color(red, ColorFormat::Hex), "#ff0000");
        assert_eq!(format_color(red, ColorFormat::Rgb), "rgb(255, 0, 0)");
        assert_eq!(format_color(red, ColorFormat::Hsl), "hsl(0, 100%, 50%)");

        let semi_red = Hsva {
            h: 0.0,
            s: 1.0,
            v: 1.0,
            a: 0.5,
        };
        assert_eq!(
            format_color(semi_red, ColorFormat::Rgba),
            "rgba(255, 0, 0, 0.50)"
        );
    }

    #[test]
    fn test_hue_to_rgb_hex() {
        assert_eq!(hue_to_rgb_hex(0.0), "#ff0000");
        assert_eq!(hue_to_rgb_hex(120.0), "#00ff00");
        assert_eq!(hue_to_rgb_hex(240.0), "#0000ff");
    }

    #[test]
    fn denotes_same_folds_notation_the_format_erases() {
        // Short hex expands to the long form.
        assert!(denotes_same("#336", "#333366", ColorFormat::Hex));
        // Digit case is not a colour difference.
        assert!(denotes_same("#FF0000", "#ff0000", ColorFormat::Hex));
        // An alpha pair is dropped by a non-alpha format...
        assert!(denotes_same("#3333666c", "#333366", ColorFormat::Hex));
        // ...but is a real difference under one that keeps it.
        assert!(!denotes_same("#3333666c", "#333366", ColorFormat::Hexa));
        // Spelling across colour syntaxes folds too.
        assert!(denotes_same("rgb(255, 0, 0)", "#ff0000", ColorFormat::Hex));
    }

    #[test]
    fn denotes_same_separates_actual_colours() {
        assert!(!denotes_same("#336", "#337", ColorFormat::Hex));
        assert!(!denotes_same("rgb(255, 0, 0)", "#ff0001", ColorFormat::Hex));
    }

    #[test]
    fn denotes_same_compares_unparseable_text_literally() {
        // Two copies of the same unfinished prefix match, whitespace aside.
        assert!(denotes_same("#33", "#33", ColorFormat::Hex));
        assert!(denotes_same(" #33 ", "#33", ColorFormat::Hex));
        // Different unfinished text does not.
        assert!(!denotes_same("#33", "#34", ColorFormat::Hex));
        // A prefix too short to parse never equals a parseable colour, even
        // the one it is on its way to.
        assert!(!denotes_same("#33", "#333333", ColorFormat::Hex));
    }

    #[test]
    fn text_denotes_compares_the_full_colour() {
        let navy = parse_color("#333366").unwrap();
        assert!(text_denotes("#336", navy));
        assert!(text_denotes("rgb(51, 51, 102)", navy));
        assert!(!text_denotes("#333367", navy));

        // Alpha is part of the colour even when a display format drops it:
        // the typed pair agrees only while the colour really holds it.
        let translucent = parse_color("#3333666c").unwrap();
        assert!(text_denotes("#3333666c", translucent));
        assert!(!text_denotes("#3333666c", navy));
        assert!(!text_denotes("#333366", translucent));

        // Unparseable text denotes nothing.
        assert!(!text_denotes("#33", navy));
        assert!(!text_denotes("", navy));
    }

    #[test]
    fn non_ascii_input_is_rejected_not_a_panic() {
        // "#é3" has a 3-BYTE hex part; pre-guard, hex_to_rgb byte-sliced it
        // mid-char and panicked. It flows in on every keystroke and through
        // the write-back guard on every display-effect run.
        assert!(parse_color("#é3").is_none());
        assert!(parse_color("#aé").is_none());
        assert!(hex_to_rgb("#12345é6").is_none()); // 8-byte hex part
        assert!(!denotes_same("#é3", "#333366", ColorFormat::Hex));
        assert!(denotes_same("#é3", "#é3", ColorFormat::Hex));
    }

    #[test]
    fn a_negative_css_hue_wraps_instead_of_breaking_a_sextant() {
        // hsl(-30, …) is valid CSS for hsl(330, …); truncating `%` kept the
        // sign and rendered red instead of rose.
        let negative = parse_color("hsl(-30, 100%, 50%)").unwrap();
        let wrapped = parse_color("hsl(330, 100%, 50%)").unwrap();
        assert_eq!(
            format_color(negative, ColorFormat::Hex),
            format_color(wrapped, ColorFormat::Hex),
        );
        assert_eq!(format_color(wrapped, ColorFormat::Hex), "#ff0080");
    }

    #[test]
    fn nan_and_infinity_are_not_colours() {
        // f64::FromStr accepts these; a NaN channel in the picker's signals
        // makes every tolerance comparison false and re-applies forever.
        assert!(parse_color("rgb(nan, 0, 0)").is_none());
        assert!(parse_color("rgba(0, 0, 0, nan)").is_none());
        assert!(parse_color("rgb(inf, 0, 0)").is_none());
        assert!(parse_color("hsl(inf, 100%, 50%)").is_none());
        assert!(parse_color("hsla(0, 100%, 50%, nan)").is_none());
    }

    // === GH #243: clamping ===

    #[test]
    fn out_of_range_channels_clamp_to_their_css_ranges() {
        // Pre-#243 these parsed unclamped: hsl(240, -5%, 50%) put a negative
        // saturation into the picker's signals and drove thumbs off-scale.
        let clamped = parse_color("hsl(240, -5%, 50%)").unwrap();
        assert!(
            clamped.s >= 0.0,
            "saturation must not go negative: {}",
            clamped.s
        );
        assert_eq!(
            format_color(clamped, ColorFormat::Hex),
            format_color(parse_color("hsl(240, 0%, 50%)").unwrap(), ColorFormat::Hex),
        );

        assert_eq!(
            parse_color("rgb(300, -20, 128)").map(|c| format_color(c, ColorFormat::Hex)),
            parse_color("rgb(255, 0, 128)").map(|c| format_color(c, ColorFormat::Hex)),
        );

        assert_eq!(parse_color("rgba(10, 20, 30, 1.5)").unwrap().a, 1.0);
        assert_eq!(parse_color("rgba(10, 20, 30, -0.5)").unwrap().a, 0.0);

        assert_eq!(
            parse_color("hsl(0, 150%, 50%)").map(|c| format_color(c, ColorFormat::Hex)),
            parse_color("hsl(0, 100%, 50%)").map(|c| format_color(c, ColorFormat::Hex)),
        );
        assert!(parse_color("hsl(0, 100%, 120%)").unwrap().v <= 1.0);
    }

    #[test]
    fn hue_parses_already_wrapped_so_signals_stay_on_scale() {
        // hsv_to_rgb wraps at render (#235), but the raw parse used to reach
        // the hue signal as -30 and put the hue thumb at a negative offset.
        // Hue's range discipline is wrapping, not clamping: -30 means 330.
        assert!(approx_eq(
            parse_color("hsl(-30, 100%, 50%)").unwrap().h,
            330.0
        ));
        assert!(approx_eq(
            parse_color("hsl(750, 100%, 50%)").unwrap().h,
            30.0
        ));
    }

    // === GH #243: named colours ===

    #[test]
    fn named_colours_parse() {
        let hexa = |name: &str| parse_color(name).map(|c| format_color(c, ColorFormat::Hexa));
        assert_eq!(hexa("red"), Some("#ff0000".into()));
        assert_eq!(hexa("white"), Some("#ffffff".into()));
        assert_eq!(hexa("black"), Some("#000000".into()));
        assert_eq!(hexa("navy"), Some("#000080".into()));
        assert_eq!(hexa("teal"), Some("#008080".into()));
        assert_eq!(hexa("rebeccapurple"), Some("#663399".into()));
        assert_eq!(hexa("cornflowerblue"), Some("#6495ed".into()));
        assert_eq!(hexa("goldenrod"), Some("#daa520".into()));
        assert_eq!(hexa("lightgoldenrodyellow"), Some("#fafad2".into()));
        // The grey/gray pairs are aliases, not near-misses.
        assert_eq!(hexa("darkslategrey"), hexa("darkslategray"));
        assert!(hexa("darkslategrey").is_some());
    }

    #[test]
    fn transparent_is_transparent_black() {
        let t = parse_color("transparent").unwrap();
        assert_eq!(t.a, 0.0);
        assert_eq!(format_color(t, ColorFormat::Hexa), "#00000000");
    }

    #[test]
    fn named_colours_are_case_insensitive() {
        let hex = |name: &str| parse_color(name).map(|c| format_color(c, ColorFormat::Hex));
        assert_eq!(hex("RED"), Some("#ff0000".into()));
        assert_eq!(hex("CornflowerBlue"), Some("#6495ed".into()));
        assert_eq!(hex("Transparent").unwrap(), "#000000");
    }

    #[test]
    fn near_names_and_prefixes_are_not_colours() {
        // Mid-typing prefixes of a name must stay unparseable (the #231
        // contract: unfinished text denotes nothing), and lookalikes must
        // not fuzzy-match. Non-ASCII flows in per keystroke, like the hex
        // arms — a lookup, never a panic.
        assert!(parse_color("r").is_none());
        assert!(parse_color("re").is_none());
        assert!(parse_color("reddish").is_none());
        assert!(parse_color("réd").is_none());
    }

    // === GH #243: modern space-separated syntax ===

    #[test]
    fn space_separated_css_parses_like_the_comma_syntax() {
        let hexa = |s: &str| parse_color(s).map(|c| format_color(c, ColorFormat::Hexa));
        assert!(hexa("rgb(51 51 102)").is_some(), "space syntax parses");
        assert_eq!(hexa("rgb(51 51 102)"), hexa("rgb(51, 51, 102)"));
        assert_eq!(hexa("rgb(51 51 102 / 0.5)"), hexa("rgba(51, 51, 102, 0.5)"));
        assert_eq!(hexa("hsl(0 100% 50%)"), hexa("hsl(0, 100%, 50%)"));
        assert_eq!(
            hexa("hsla(240 100% 50% / 0.25)"),
            hexa("hsla(240, 100%, 50%, 0.25)"),
        );
    }

    #[test]
    fn a_percentage_alpha_parses_in_both_syntaxes() {
        assert!(approx_eq(
            parse_color("rgb(51 51 102 / 50%)").unwrap().a,
            0.5
        ));
        assert!(approx_eq(
            parse_color("rgba(51, 51, 102, 50%)").unwrap().a,
            0.5
        ));
    }

    #[test]
    fn malformed_function_syntax_is_rejected() {
        assert!(parse_color("rgb(51 51)").is_none());
        // An alpha in the space syntax needs the slash.
        assert!(parse_color("rgb(51 51 102 51)").is_none());
        // Mixed separators are neither syntax.
        assert!(parse_color("rgb(51, 51 102)").is_none());
        assert!(parse_color("rgb(51 51 102 / )").is_none());
        assert!(parse_color("rgb(51 51 102 / 0.5 / 0.5)").is_none());
        // Surplus comma parts used to be silently ignored; garbage is not a
        // colour.
        assert!(parse_color("rgb(1, 2, 3, 0.5, 9)").is_none());
        // The finite check covers the new arms too.
        assert!(parse_color("hsl(0 100% 50% / nan)").is_none());
        assert!(parse_color("rgb(inf 0 0)").is_none());
    }

    // === GH #243: 4-digit #rgba hex ===

    #[test]
    fn four_digit_hex_parses_like_its_eight_digit_expansion() {
        let short = hex_to_rgb("#3366").unwrap();
        let long = hex_to_rgb("#33336666").unwrap();
        assert_eq!(short, long);
        assert!(approx_eq(short.a, 102.0 / 255.0));

        let semi_red = hex_to_rgb("#f00c").unwrap();
        assert!(approx_eq(semi_red.r, 1.0));
        assert!(approx_eq(semi_red.a, 204.0 / 255.0));

        // The odd lengths stay unparseable.
        assert!(hex_to_rgb("#12345").is_none());
        assert!(hex_to_rgb("#1234567").is_none());
        // The non-ASCII guard covers the new arm ("é" is 2 bytes, so this
        // hex part is 4 bytes long).
        assert!(hex_to_rgb("#é33").is_none());
    }

    // === GH #243: the new notations flow through the write-back guards ===

    #[test]
    fn new_notations_round_trip_through_the_guards() {
        // Pre-#243 these fell to denotes_same's unparseable-literal fallback:
        // a consumer echoing the picker's colour as "red" or in the space
        // syntax read as a foreign change, defeating the #231/#227 gates.
        assert!(denotes_same("red", "#ff0000", ColorFormat::Hex));
        assert!(denotes_same("rgb(51 51 102)", "#333366", ColorFormat::Hex));
        assert!(denotes_same(
            "hsl(0 100% 50%)",
            "rgb(255, 0, 0)",
            ColorFormat::Hex
        ));
        assert!(denotes_same("transparent", "#00000000", ColorFormat::Hexa));
        assert!(!denotes_same("red", "#ff0001", ColorFormat::Hex));

        let navy = parse_color("#333366").unwrap();
        assert!(text_denotes("rgb(51 51 102)", navy));
        let translucent = parse_color("#33336666").unwrap();
        assert!(text_denotes("#3366", translucent));
        assert!(
            !text_denotes("#3366", navy),
            "the 4-digit alpha is part of the colour"
        );
    }

    // === Review follow-ups: emit/parse stay a fixed point ===

    #[test]
    fn the_emitted_hsl_never_desyncs_from_the_parser() {
        // #ff0001 has hue 359.76…, which rounds to 360 in hsl output. When
        // the emit side printed "hsl(360, …)" while the parser wrapped hue
        // into [0, 360), one colour had two spellings `denotes_same` called
        // different — and ColorInput's display effect rewrote the field
        // under the author's caret (the GH #231 class, resurrected under an
        // hsl display format). The emit side wraps too: round, then wrap.
        let emitted = format_color(parse_color("#ff0001").unwrap(), ColorFormat::Hsl);
        assert_eq!(emitted, "hsl(0, 100%, 50%)");
        assert!(
            denotes_same("#ff0001", &emitted, ColorFormat::Hsl),
            "a colour must denote the same colour as its own emission"
        );

        // The same desync leaked through the picker's serializer for a grey
        // held at hue ∈ [359.5, 360): "hsl(360, 0%, l)" re-parsed as the
        // convention-grey hue 0 instead of a stated hue.
        let grey = Hsva {
            h: 359.8,
            s: 0.0,
            v: 0.5,
            a: 1.0,
        };
        assert_eq!(format_color(grey, ColorFormat::Hsl), "hsl(0, 0%, 50%)");
    }

    #[test]
    fn a_vanishingly_negative_hue_stays_inside_the_wrap_contract() {
        // f64: -1e-14 + 360.0 rounds up to exactly 360.0, so `rem_euclid`
        // alone can return 360.0 — outside the documented [0, 360), and a
        // full-offset hue thumb.
        let h = parse_color("hsl(-0.00000000000001, 100%, 50%)").unwrap().h;
        assert!((0.0..360.0).contains(&h), "hue {h} escaped [0, 360)");
    }

    #[test]
    fn css_function_names_are_case_insensitive() {
        let hex = |s: &str| parse_color(s).map(|c| format_color(c, ColorFormat::Hex));
        assert_eq!(hex("RGB(255, 0, 0)"), Some("#ff0000".into()));
        assert_eq!(hex("Hsl(0 100% 50%)"), Some("#ff0000".into()));
        assert!(approx_eq(
            parse_color("RGBA(0, 0, 255, 0.5)").unwrap().a,
            0.5
        ));
        assert_eq!(hex("HSLA(240, 100%, 50%, 1)"), Some("#0000ff".into()));
    }

    #[test]
    fn a_function_wrapper_is_stripped_exactly_once() {
        // Repeat-stripping used to accept all of these.
        assert!(parse_color("rgb(rgb(1, 2, 3))").is_none());
        assert!(parse_color("rgb(1, 2, 3)))").is_none());
        assert!(parse_color("hsl(0, 50%%, 50%)").is_none());
        // The close paren stays optional: mid-typing text previews, and the
        // commit boundary normalizes it.
        assert!(parse_color("rgb(255, 0, 0").is_some());
        assert!(parse_color("hsl(0, 100%, 50%").is_some());
    }
}
