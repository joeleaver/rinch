//! Conversions between the colour models, and their CSS serializers.

use super::types::{ColorFormat, Hsla, Hsva, Rgba};

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
    // The [0, 360) contract, as `parse_hsl_css` keeps it: a vanishingly
    // negative sextant offset rounds `h + 360.0` up to exactly 360.0, and a
    // `-0.0` channel keeps its sign through the arithmetic — either would
    // put the hue thumb off the scale for what is pure red.
    let h = if h == 0.0 || h >= 360.0 { 0.0 } else { h };

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
    // emit side keeps the hue spelling a fixed point of `parse_color`. (Alpha
    // is not one everywhere: an alpha in [0.995, 1) writes `1.00`, which
    // re-parses opaque and re-formats without the alpha — the reason
    // `denotes_emitted` re-parses an emission rather than comparing text.)
    let h = hsl.h.round().rem_euclid(360.0) as i32;
    let s = (hsl.s * 100.0).round() as i32;
    let l = (hsl.l * 100.0).round() as i32;
    if hsl.a < 1.0 {
        format!("hsla({}, {}%, {}%, {:.2})", h, s, l, hsl.a)
    } else {
        format!("hsl({}, {}%, {}%)", h, s, l)
    }
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
