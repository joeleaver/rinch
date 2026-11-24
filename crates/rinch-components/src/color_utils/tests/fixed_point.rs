use super::*;

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

/// The notation reported is the family the string was written in, for
/// every branch of the classifier. (The colour half needs no test:
/// `parse_color` is defined as this function minus the notation.)
#[test]
fn parse_color_with_notation_reports_the_written_notation() {
    use Notation::{Hex, Hsl, Rgb};
    let corpus = [
        // hex: 3/4/6/8 digits, with and without the '#', any digit case
        ("#abc", Some(Hex)),
        ("#abcd", Some(Hex)),
        ("#aabbcc", Some(Hex)),
        ("#aabbccdd", Some(Hex)),
        ("#AABBCC", Some(Hex)),
        ("abc", Some(Hex)),
        ("aabbcc", Some(Hex)),
        ("AABBCCDD", Some(Hex)),
        ("  #336  ", Some(Hex)),
        // named colours, transparent, mixed case
        ("red", Some(Hex)),
        ("rebeccapurple", Some(Hex)),
        ("Transparent", Some(Hex)),
        ("LightGoldenrodYellow", Some(Hex)),
        // rgb()/rgba(): legacy commas, modern spaces, case, percent alpha
        ("rgb(51, 51, 102)", Some(Rgb)),
        ("rgba(51, 51, 102, 0.5)", Some(Rgb)),
        ("rgb(51 51 102)", Some(Rgb)),
        ("rgb(51 51 102 / 0.5)", Some(Rgb)),
        ("rgba(51 51 102 / 50%)", Some(Rgb)),
        ("RGB(255, 0, 0)", Some(Rgb)),
        ("rgb(127.9999999999, 128, 128)", Some(Rgb)),
        ("rgb(51, 51, 102", Some(Rgb)),
        // hsl()/hsla(): legacy, modern, case, bare-number percents
        ("hsl(200, 3%, 49%)", Some(Hsl)),
        ("hsla(200, 3%, 49%, 0.5)", Some(Hsl)),
        ("hsl(200 3% 49%)", Some(Hsl)),
        ("hsl(200 3% 49% / 0.5)", Some(Hsl)),
        ("HSL(240, 0%, 50%)", Some(Hsl)),
        ("hsl(-30, 100, 50)", Some(Hsl)),
        ("hsl(205, 0.3%, 49%)", Some(Hsl)),
        // junk
        ("", None),
        ("#", None),
        ("#33", None),
        ("rgb(1, 2)", None),
        ("hsl(nan, 0%, 0%)", None),
    ];
    for (text, expected) in corpus {
        assert_eq!(
            parse_color_with_notation(text).map(|(_, n)| n),
            expected,
            "notation for {text:?}"
        );
    }
}

/// `text_denotes` judges at the text's own notation: a hue move the hsl
/// wire spells but 8-bit cannot see is a difference, and the alpha
/// spelling wart (`1.00` re-parses opaque) is folded by the re-parse of
/// the emission.
#[test]
fn text_denotes_judges_at_the_texts_own_notation() {
    let at_200 = parse_color("hsl(200, 3%, 49%)").unwrap();
    let at_205 = parse_color("hsl(205, 3%, 49%)").unwrap();
    assert_eq!(
        format_color(at_200, ColorFormat::Hex),
        format_color(at_205, ColorFormat::Hex),
        "the premise: both render #797e81"
    );
    assert!(text_denotes("hsl(205, 3%, 49%)", at_205));
    assert!(!text_denotes("hsl(205, 3%, 49%)", at_200));
    // The same two colours written in hex are one colour.
    assert!(text_denotes("#797e81", at_200));
    assert!(text_denotes("#797e81", at_205));

    // An alpha the serializer writes as `1.00` is opaque on its wire.
    let nearly_opaque = Hsva { a: 0.996, ..at_200 };
    assert!(text_denotes("hsl(200, 3%, 49%)", nearly_opaque));
    assert!(text_denotes("hsla(200, 3%, 49%, 1.00)", nearly_opaque));
    assert!(!text_denotes("hsla(200, 3%, 49%, 0.99)", nearly_opaque));
}

/// The `[0, 360)` hue contract holds on the RGB-family arms too, not only
/// through `parse_hsl_css`'s snap: a vanishingly negative sextant offset
/// lands `h + 360.0` on exactly 360.0 in f64, and a `-0.0` channel keeps
/// its sign through the arithmetic — both would put the hue thumb at
/// `left: 100%` / `left: -0%` for what is pure red.
#[test]
fn rgb_derived_hue_stays_inside_the_wrap_contract() {
    let h = parse_color("rgb(255, 0, 1e-300)").unwrap().h;
    assert!(
        h == 0.0 && h.is_sign_positive(),
        "h + 360 rounded to 360: {h}"
    );
    let h = parse_color("rgb(255, -0.0, 0)").unwrap().h;
    assert!(h.is_sign_positive(), "a -0.0 hue formats as '-0': {h}");
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

/// GH #262: the emission arm accepts both roundings of a tie and nothing
/// farther. `#797e81`'s hue is exactly 202.5°.
#[test]
fn a_respelled_emission_may_round_a_tie_either_way() {
    let respells = |text: &str, emitted: &str| {
        let (parsed, notation) = parse_color_with_notation(text).unwrap();
        respells_emitted(parsed, notation, emitted)
    };
    assert!(respells("hsl(202, 3%, 49%)", "#797e81"));
    assert!(respells("hsl(203, 3%, 49%)", "#797e81"));
    assert!(!respells("hsl(201, 3%, 49%)", "#797e81"));
    assert!(!respells("hsl(204, 3%, 49%)", "#797e81"));
    assert!(!respells("hsl(203, 4%, 49%)", "#797e81"), "only the tie");

    // Off a tie, one grid point from rinch's spelling is not a rounding:
    // `#797e82`'s hue is 206⅔°, rinch writes 207, and 206 is a peer's.
    assert!(respells("hsl(207, 4%, 49%)", "#797e82"));
    assert!(!respells("hsl(206, 4%, 49%)", "#797e82"));
    assert!(!respells("hsl(208, 4%, 49%)", "#797e82"));

    // Hue is circular: `#c85051`'s hue is 359.5°, and a store may write
    // 359 or wrap to 0.
    assert!(respells("hsl(359, 52%, 55%)", "#c85051"));
    assert!(respells("hsl(0, 52%, 55%)", "#c85051"));
    assert!(!respells("hsl(358, 52%, 55%)", "#c85051"));
    assert!(!respells("hsl(1, 52%, 55%)", "#c85051"));

    // The 8-bit grid, alpha included: alpha 0.50 is 127.5 of 255.
    let emitted = "hsla(0, 100%, 50%, 0.50)";
    assert!(respells("#ff000080", emitted));
    assert!(respells("#ff00007f", emitted));
    assert!(!respells("#ff00007e", emitted));
    assert!(
        !respells("#fe00007f", emitted),
        "a colour channel is not a tie"
    );
    assert!(respells("rgba(255, 0, 0, 0.5)", emitted));
    assert!(!respells("rgba(255, 0, 0, 0.51)", emitted));
    // And the 8-bit alpha grid is 255 steps, not 100: 0.51 is 130.05 of
    // 255, so 131 is a step away though it is within half a percent.
    let emitted = "hsla(0, 100%, 50%, 0.51)";
    assert!(respells("#ff000082", emitted));
    assert!(!respells("#ff000083", emitted));
}
