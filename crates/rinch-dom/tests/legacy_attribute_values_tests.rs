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
//! - `cellpadding`: an integer read as Chrome reads it; one that fails to
//!   parse, or is negative, is 0px. An empty `cellpadding` maps nothing.
//!
//! A mail's `<table cellspacing="10px">` came out with the 2px default and
//! was 16px short of Chrome. And Chrome keeps `border-spacing` in whole
//! pixels, cutting off the fraction, so `cellspacing="10.5"` and
//! `border-spacing: 10.5px` both space by 10px.
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
        // rinch puts a box on whole pixels where Chrome keeps 10.5.
        assert!(
            (x - want).abs() <= 0.5 && (y - want).abs() <= 0.5 && (w, h) == (10.0, 10.0),
            "hspace/vspace={v:?}: ({x}, {y}, {w}, {h}), Chrome ({want}, {want}, 10, 10)"
        );
    }
}

/// `border-spacing` is whole pixels in Chrome: the fraction is cut off,
/// in CSS as in the attribute.
#[test]
fn border_spacing_drops_its_fraction() {
    // (spacing, Chrome's table width, height) around one 10x10 cell.
    let chrome = [
        ("border-spacing:10.9px", 30.0, 30.0),
        ("border-spacing:0.5px", 10.0, 10.0),
        ("border-spacing:1.5px 3.7px", 12.0, 16.0),
        ("border-spacing:0.75em", 24.0, 24.0),
    ];
    for (style, w, h) in chrome {
        let d = doc(&format!(
            "<table id=t cellpadding=0 style='{style}'><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>"
        ));
        let (_, _, tw, th) = abs(&d, "t");
        assert_eq!((tw, th), (w, h), "{style}");
    }
    for (v, want) in [("10.9", 30.0), ("0.5", 10.0)] {
        let d = doc(&format!(
            "<table id=t cellpadding=0 cellspacing='{v}'><tr><td style='width:10px;height:10px;padding:0'></td></tr></table>"
        ));
        let (_, _, tw, th) = abs(&d, "t");
        assert_eq!((tw, th), (want, want), "cellspacing={v:?}");
    }
}
