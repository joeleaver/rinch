//! How the legacy number attributes read their values, as Chrome 156 reads
//! them: `cellspacing="10px"` is 10px of spacing, not an error.
//!
//! The HTML Standard has three parsers here, and Chrome follows them:
//! - `cellspacing`, `hspace`, `vspace`: the rules for parsing dimension
//!   values (digits, an optional fraction, the rest ignored; `cellspacing`
//!   drops a percentage, the two spaces keep it);
//! - a table's and an image's `border`: the rules for parsing non-negative
//!   integers (an optional `+`, digits, the rest ignored). A table's
//!   `border` that fails to parse is 1px; an image's is none;
//! - `cellpadding` and `<hr size>`: Chrome's lenient `ToInt`, which also
//!   skips a leading vertical tab; an error is 0. An empty `cellpadding`
//!   maps nothing.
//! - `<font size>`: the rules for parsing a legacy font size.
//!
//! The integers are Chrome's `i32` (`u32` for the non-negative ones): an
//! overflow is an error, not a clamp.
//!
//! A mail's `<table cellspacing="10px">` came out with the 2px default and
//! was 16px short of Chrome. And Chrome keeps `border-spacing` as a whole
//! number of device pixels (Blink's `RoundForImpreciseConversion<short>`:
//! 0.01 added, the fraction cut off, 0 past 32767), so at 1x
//! `cellspacing="10.5"` and `border-spacing: 10.5px` both space by 10px,
//! and at 2x by 10.5px.
//!
//! Every expected number is Chrome 156's for the same markup at 720x800. The
//! boxes are sized empty cells and images, so no number depends on fonts.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str = "body { margin: 0; font-size: 10px; line-height: 0; }";

fn doc(html: &str) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(720.0, 800.0);
    d
}

/// The border box of `#id`, in page coordinates.
fn abs(d: &RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let l = d.tree.get(n.0).unwrap().layout;
    let (mut x, mut y) = (0.0, 0.0);
    let mut cur = Some(n.0);
    while let Some(i) = cur {
        let node = d.tree.get(i).unwrap();
        x += node.layout.x;
        y += node.layout.y;
        cur = node.parent;
    }
    (x, y, l.width, l.height)
}

/// The values every case below is read with.
const VALUES: [&str; 12] = [
    "10", "10px", "10.5", "10.5px", " 10", "10%", "abc", "+10", "-3", "1e1", "10 20", "",
];

const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// `<table cellspacing=v>` around one 10x10 cell: the table is 10px plus
/// two spacings wide (the default spacing is 2px).
#[test]
fn cellspacing_reads_a_dimension_without_a_percentage() {
    let chrome = [
        30.0, 30.0, 30.0, 30.0, 30.0, 14.0, 14.0, 14.0, 14.0, 12.0, 30.0, 14.0,
    ];
    for (v, want) in VALUES.iter().zip(chrome) {
        let d = doc(&format!(
            "<table id=t cellspacing='{v}' cellpadding=0><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>"
        ));
        let (_, _, w, h) = abs(&d, "t");
        assert_eq!((w, h), (want, want), "cellspacing={v:?}");
    }
}

/// `doc`, laid out at a device pixel ratio of `dpr`.
fn doc_at(html: &str, dpr: f32) -> RinchDocument {
    let mut d = RinchDocument::new();
    d.set_device_pixel_ratio(dpr);
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, html);
    d.resolve_layout(720.0, 800.0);
    d
}

/// The `#t` table's size, one 10x10 cell inside.
fn spaced(attrs: &str, dpr: f32) -> (f32, f32) {
    let d = doc_at(
        &format!(
            "<table id=t cellpadding=0 {attrs}><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>"
        ),
        dpr,
    );
    let (_, _, w, h) = abs(&d, "t");
    (w, h)
}

/// `<table cellpadding=v>` around one 10x10 cell: the cell is 10px plus two
/// paddings (the UA's 1px when the attribute is empty).
#[test]
fn cellpadding_reads_an_integer_and_an_error_is_zero() {
    let chrome = [
        30.0, 30.0, 30.0, 30.0, 30.0, 30.0, 10.0, 30.0, 10.0, 12.0, 30.0, 12.0,
    ];
    for (v, want) in VALUES.iter().zip(chrome) {
        let d = doc(&format!(
            "<table cellspacing=0 cellpadding='{v}'><tr><td id=c style='width:10px;height:10px'></td></tr></table>"
        ));
        let (_, _, w, h) = abs(&d, "c");
        assert_eq!((w, h), (want, want), "cellpadding={v:?}");
    }
}

/// `<table border=v>` around one 10x10 cell: the table is 10px plus two
/// table borders plus the cell's own two 1px borders (none when the table's
/// border is 0). An error is a 1px border.
#[test]
fn a_tables_border_reads_a_non_negative_integer_and_an_error_is_one() {
    let chrome = [
        32.0, 32.0, 32.0, 32.0, 32.0, 32.0, 14.0, 32.0, 14.0, 14.0, 32.0, 14.0,
    ];
    for (v, want) in VALUES.iter().zip(chrome) {
        let d = doc(&format!(
            "<table id=t cellspacing=0 cellpadding=0 border='{v}'><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>"
        ));
        let (_, _, w, h) = abs(&d, "t");
        assert_eq!((w, h), (want, want), "border={v:?}");
    }
}

/// `<img border=v width=10 height=10>`: 10px plus two borders; an error is
/// no border.
#[test]
fn an_images_border_reads_a_non_negative_integer_and_an_error_is_none() {
    let chrome = [
        30.0, 30.0, 30.0, 30.0, 30.0, 30.0, 10.0, 30.0, 10.0, 12.0, 30.0, 10.0,
    ];
    for (v, want) in VALUES.iter().zip(chrome) {
        let d = doc(&format!(
            "<div><img id=i border='{v}' width=10 height=10 src='{PNG}'></div>"
        ));
        let (_, _, w, h) = abs(&d, "i");
        assert_eq!((w, h), (want, want), "border={v:?}");
    }
}

/// A floated `<img hspace=v vspace=v>` in a 400px-wide block: its margins
/// put it at (v, v). A percentage is of the block's width.
#[test]
fn hspace_and_vspace_read_a_dimension_with_a_percentage() {
    let chrome = [
        10.0, 10.0, 10.5, 10.5, 10.0, 40.0, 0.0, 0.0, 0.0, 1.0, 10.0, 0.0,
    ];
    for (v, want) in VALUES.iter().zip(chrome) {
        let d = doc(&format!(
            "<div style='width:400px'><img id=i align=left hspace='{v}' vspace='{v}' width=10 height=10 src='{PNG}'></div>"
        ));
        let (x, y, w, h) = abs(&d, "i");
        // rinch rounds a box to whole pixels where Chrome keeps 10.5: 11
        // is Chrome's 10.5, and 10 would be the fraction dropped.
        let want = f32::round(want);
        assert_eq!(
            (x, y, w, h),
            (want, want, 10.0, 10.0),
            "hspace/vspace={v:?}"
        );
    }
}

/// `border-spacing` is whole device pixels in Chrome: 0.01 is added and
/// the fraction cut off, in CSS as in the attribute.
#[test]
fn border_spacing_is_whole_device_pixels_at_1x() {
    // (spacing, Chrome's table width, height) around one 10x10 cell.
    let chrome = [
        ("style='border-spacing:10.9px'", 30.0, 30.0),
        ("style='border-spacing:0.5px'", 10.0, 10.0),
        ("style='border-spacing:1.5px 3.7px'", 12.0, 16.0),
        ("style='border-spacing:0.75em'", 24.0, 24.0),
        ("style='border-spacing:9.99px'", 30.0, 30.0),
        ("style='border-spacing:9.995px'", 30.0, 30.0),
        ("style='border-spacing:9.994px'", 30.0, 30.0),
        ("cellspacing='10.9'", 30.0, 30.0),
        ("cellspacing='0.5'", 10.0, 10.0),
        ("cellspacing='9.99'", 30.0, 30.0),
    ];
    for (attrs, w, h) in chrome {
        assert_eq!(spaced(attrs, 1.0), (w, h), "{attrs}");
    }
}

/// At 2x a device pixel is half a CSS px, so half a pixel of spacing
/// survives: `10.5px` stays 10.5, `9.99px` is 9.5.
#[test]
fn border_spacing_is_whole_device_pixels_at_2x() {
    let chrome = [
        ("style='border-spacing:10.5px'", 31.0),
        ("style='border-spacing:9.99px'", 29.0),
        ("style='border-spacing:9.994px'", 29.0),
        ("style='border-spacing:9.995px'", 30.0),
        ("style='border-spacing:10.25px'", 30.0),
        ("style='border-spacing:10.75px'", 31.0),
        ("cellspacing='10.5'", 31.0),
        ("cellspacing='9.99'", 29.0),
    ];
    for (attrs, want) in chrome {
        assert_eq!(spaced(attrs, 2.0), (want, want), "{attrs} at 2x");
    }
}

/// A spacing past Blink's `short` is 0: `32767px` + 0.01 is already past
/// it. At 2x the limit is in device pixels, so `32767px` is 0 there too.
#[test]
fn border_spacing_past_a_short_is_none() {
    for attrs in [
        "style='border-spacing:32767px'",
        "style='border-spacing:40000px'",
        "cellspacing='32767'",
        "cellspacing='40000'",
    ] {
        assert_eq!(spaced(attrs, 1.0), (10.0, 10.0), "{attrs}");
        assert_eq!(spaced(attrs, 2.0), (10.0, 10.0), "{attrs} at 2x");
    }
    // Control, below the limit: 32766px is kept at 1x (Chrome: 65542 tall,
    // two spacings and the cell).
    assert_eq!(spaced("style='border-spacing:32766px'", 1.0).1, 65542.0);
}

/// Integers overflow as Chrome's do: `cellpadding` is an `i32` and a
/// `border` a `u32`, and a number past them is an error (`cellpadding` 0,
/// a table's `border` 1px, an image's none), not a huge box.
#[test]
fn an_integer_past_its_type_is_an_error() {
    for (v, want) in [
        ("3000000000", 10.0),
        ("2147483648", 10.0),
        ("-2147483649", 10.0),
    ] {
        let d = doc(&format!(
            "<table cellspacing=0 cellpadding='{v}'><tr><td id=c style='width:10px;height:10px'></td></tr></table>"
        ));
        let (_, _, w, h) = abs(&d, "c");
        assert_eq!((w, h), (want, want), "cellpadding={v:?}");
    }
    for v in ["99999999999", "4294967296"] {
        let d = doc(&format!(
            "<table id=t cellspacing=0 cellpadding=0 border='{v}'><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>"
        ));
        let (_, _, w, h) = abs(&d, "t");
        assert_eq!((w, h), (14.0, 14.0), "table border={v:?}");
        let d = doc(&format!(
            "<div><img id=i border='{v}' width=10 height=10 src='{PNG}'></div>"
        ));
        let (_, _, w, h) = abs(&d, "i");
        assert_eq!((w, h), (10.0, 10.0), "img border={v:?}");
    }
}

/// Whitespace: `cellpadding` (Chrome's `ToInt`) skips a leading vertical tab
/// and a form feed; the HTML parsers (`cellspacing`, `border`) skip the form
/// feed only. Neither skips a no-break space.
#[test]
fn which_leading_whitespace_is_skipped() {
    for (v, want) in [("\u{b}10", 30.0), ("\u{c}10", 30.0), ("\u{a0}10", 10.0)] {
        let d = doc(&format!(
            "<table cellspacing=0 cellpadding='{v}'><tr><td id=c style='width:10px;height:10px'></td></tr></table>"
        ));
        let (_, _, w, _) = abs(&d, "c");
        assert_eq!(w, want, "cellpadding={v:?}");
    }
    assert_eq!(spaced("cellspacing='\u{b}10'", 1.0), (14.0, 14.0));
    let d = doc(
        "<table id=t cellspacing=0 cellpadding=0 border='\u{b}10'><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>",
    );
    assert_eq!(abs(&d, "t").2, 14.0, "table border with a vertical tab");
}

/// `<font size=v>` by the rules for parsing a legacy font size, around a
/// `1em` box in a 20px parent. Chrome's sizes: 1 is 10px, 2 13px, 3 16px,
/// 4 18px, 7 48px; an error maps nothing (20px).
#[test]
fn font_size_reads_its_leading_digits() {
    let chrome = [
        ("4", 18.0),
        ("4px", 18.0),
        ("+1abc", 18.0),
        (" 4", 18.0),
        ("\u{c}4", 18.0),
        ("-1x", 13.0),
        ("10", 48.0),
        ("0", 10.0),
        ("-0", 16.0),
        ("-9", 10.0),
        ("abc", 20.0),
        ("", 20.0),
        ("\u{b}4", 20.0),
        ("+ 1", 20.0),
    ];
    for (v, want) in chrome {
        let d = doc(&format!(
            "<div style='font-size:20px'><font size='{v}'><span id=s style='display:inline-block;width:1em;height:1em'></span></font></div>"
        ));
        assert_eq!(abs(&d, "s").2, want, "font size={v:?}");
    }
}

/// `<hr size=v>`: Chrome reads it with `ToInt`; above 1 it is the rule's
/// height plus its two 1px borders, and 1 or less (an error is 0) drops
/// the bottom border, leaving a 1px rule.
#[test]
fn hr_size_sets_the_rules_height() {
    let chrome = [
        ("10", 10.0),
        ("10px", 10.0),
        ("10.7", 10.0),
        ("+10", 10.0),
        ("\u{b}10", 10.0),
        ("3", 3.0),
        ("2", 2.0),
        ("1", 1.0),
        ("0", 1.0),
        ("-5", 1.0),
        ("abc", 1.0),
        ("", 1.0),
    ];
    for (v, want) in chrome {
        let d = doc(&format!("<hr id=r size='{v}' style='margin:0'>"));
        assert_eq!(abs(&d, "r").3, want, "hr size={v:?}");
    }
    let d = doc("<hr id=r style='margin:0'>");
    assert_eq!(abs(&d, "r").3, 2.0, "control: no size is the UA's 2px rule");
}
