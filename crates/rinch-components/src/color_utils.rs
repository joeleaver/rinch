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

/// Parse a hex color string (#rgb, #rrggbb, #rrggbbaa).
pub fn hex_to_rgb(hex: &str) -> Option<Rgba> {
    let hex = hex.trim().trim_start_matches('#');
    // `len()` counts bytes and the arms below byte-slice: non-ASCII input
    // whose byte length happens to be 3, 6, or 8 ("#é3") would panic on a
    // char boundary. Hex digits are ASCII, so anything else is simply not a
    // colour. This runs on every keystroke of the colour fields.
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(Rgba {
                r: r as f64 / 255.0,
                g: g as f64 / 255.0,
                b: b as f64 / 255.0,
                a: 1.0,
            })
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Rgba {
                r: r as f64 / 255.0,
                g: g as f64 / 255.0,
                b: b as f64 / 255.0,
                a: 1.0,
            })
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Rgba {
                r: r as f64 / 255.0,
                g: g as f64 / 255.0,
                b: b as f64 / 255.0,
                a: a as f64 / 255.0,
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
    let h = hsl.h.round() as i32;
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
/// Supports: #rgb, #rrggbb, #rrggbbaa, rgb(), rgba(), hsl(), hsla()
pub fn parse_color(s: &str) -> Option<Hsva> {
    let s = s.trim();
    if s.starts_with('#') {
        let rgb = hex_to_rgb(s)?;
        Some(rgb_to_hsv(rgb))
    } else if s.starts_with("rgba(") || s.starts_with("rgb(") {
        parse_rgb_css(s)
    } else if s.starts_with("hsla(") || s.starts_with("hsl(") {
        parse_hsl_css(s)
    } else {
        // Try as bare hex
        hex_to_rgb(s).map(rgb_to_hsv)
    }
}

fn parse_rgb_css(s: &str) -> Option<Hsva> {
    let inner = s
        .trim_start_matches("rgba(")
        .trim_start_matches("rgb(")
        .trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let r = parts[0].trim().parse::<f64>().ok()? / 255.0;
    let g = parts[1].trim().parse::<f64>().ok()? / 255.0;
    let b = parts[2].trim().parse::<f64>().ok()? / 255.0;
    let a = if parts.len() >= 4 {
        parts[3].trim().parse::<f64>().ok()?
    } else {
        1.0
    };
    // f64::FromStr accepts "nan"/"inf": a NaN channel would poison the
    // picker's signals (every comparison involving NaN is false, so a
    // value_fn apply of such a string re-applies forever). Not a colour.
    if ![r, g, b, a].iter().all(|c| c.is_finite()) {
        return None;
    }
    Some(rgb_to_hsv(Rgba { r, g, b, a }))
}

fn parse_hsl_css(s: &str) -> Option<Hsva> {
    let inner = s
        .trim_start_matches("hsla(")
        .trim_start_matches("hsl(")
        .trim_end_matches(')');
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let h = parts[0].trim().parse::<f64>().ok()?;
    let s_val = parts[1].trim().trim_end_matches('%').parse::<f64>().ok()? / 100.0;
    let l = parts[2].trim().trim_end_matches('%').parse::<f64>().ok()? / 100.0;
    let a = if parts.len() >= 4 {
        parts[3].trim().parse::<f64>().ok()?
    } else {
        1.0
    };
    // See parse_rgb_css: reject "nan"/"inf" before they reach the signals.
    if ![h, s_val, l, a].iter().all(|c| c.is_finite()) {
        return None;
    }
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
}
