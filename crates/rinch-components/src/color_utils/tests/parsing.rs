use super::*;

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
