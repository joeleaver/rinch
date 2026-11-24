use super::*;

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
