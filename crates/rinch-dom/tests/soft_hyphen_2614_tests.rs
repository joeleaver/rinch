//! A soft hyphen (U+00AD) that a line breaks at draws a hyphen, and the line
//! is fitted with it (#2614); `hyphens: none` takes the break away (#2621);
//! and Hebrew text breaks at a soft hyphen (#2622).
//!
//! Every expectation is Chrome 157's for the same markup. A line is written
//! as rinch's text of it, a soft hyphen as `~` (the space a line ends with is
//! on it); a width is the line's advance, the hyphen in it (Inter's U+2010 is
//! 5.73px at 16px). Text is the bundled Inter registered as `ProbeFace`,
//! `font: 16px/20px`; widths are picked off the points where the hyphen does
//! not matter.
//!
//! The hyphen is the soft hyphen's own glyph while its line ends at it, so
//! the text queries see it where it is drawn: the soft hyphen's selection
//! box is the hyphen's, as in Chrome (in a left-to-right line), and a caret
//! before the soft hyphen stands before the hyphen. It is not text: copying
//! gives the DOM's soft hyphen, as Chrome's does, not a `-`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Text that does not wrap right before wrapping text: a text seam (#2390),
/// which sends the paragraph through rinch's own line breaks.
const TEXT_SEAM: &str = "<span style='white-space:nowrap'>abc-</span>123 ";

/// Inter's hyphen at 16px, Chrome 157 (`Extra-` 44.64 less `Extra` 38.91).
const HYPHEN: f32 = 5.73;

fn html(markup: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 800.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

struct Laid {
    lines: Vec<String>,
    advances: Vec<f32>,
    height: f32,
    doc: RinchDocument,
}

/// `markup` in a `width`px div styled `style`.
fn lay(markup: &str, width: u32, style: &str) -> Laid {
    let doc = html(&format!(
        "<div id=a style='width:{width}px;{style}'>{markup}</div>"
    ));
    let n = doc.tree.get(by_id(&doc, "a").0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    let mut lines = Vec::new();
    let mut advances = Vec::new();
    for l in il.layout.lines() {
        lines.push(
            il.text_content
                .get(l.text_range())
                .unwrap_or("")
                .replace('\u{ad}', "~"),
        );
        advances.push(l.metrics().advance);
    }
    let height = n.layout.height;
    Laid {
        lines,
        advances,
        height,
        doc,
    }
}

#[track_caller]
fn breaks(markup: &str, width: u32, chrome: &[&str]) -> Laid {
    breaks_styled(markup, width, "", chrome)
}

#[track_caller]
fn breaks_styled(markup: &str, width: u32, style: &str, chrome: &[&str]) -> Laid {
    let laid = lay(markup, width, style);
    assert_eq!(laid.lines, chrome, "{width}px {style}: {markup}");
    let tall = 20.0 * chrome.len() as f32;
    assert!(
        (laid.height - tall).abs() < 0.5,
        "{width}px {style}: {markup}: rinch {}, Chrome {tall}",
        laid.height
    );
    laid
}

#[track_caller]
fn close(rinch: f32, chrome: f32, what: &str) {
    near(rinch, chrome, 0.05, what);
}

#[track_caller]
fn near(rinch: f32, chrome: f32, tol: f32, what: &str) {
    assert!(
        (rinch - chrome).abs() < tol,
        "{what}: rinch {rinch}, Chrome {chrome}"
    );
}

/// Known difference: Inter kerns `x` and `t` 0.32px apart, and the kern is
/// applied across the soft hyphen between them (a default-ignorable that
/// shaping skips). parley does not shape a line's end again where it breaks,
/// so `Ex-` at a line's end keeps it; Chrome shapes the piece again and does
/// not. Not the hyphen's: `Extra-` (no kern across `a`/`o`) is Chrome's to
/// the hundredth.
const X_T_KERN: f32 = 0.32;

/// The hyphen is part of the line's width: `Extra-` is 44.64 wide, in 100px
/// and overflowing 40px and 1px (an overflowing line still takes its first
/// soft hyphen). The last line draws none.
#[test]
fn the_hyphen_is_in_the_lines_width() {
    for width in [100, 40, 1] {
        let laid = breaks("Extra&shy;ordinary", width, &["Extra~", "ordinary"]);
        close(laid.advances[0], 44.64, &format!("Extra- in {width}px"));
        close(laid.advances[1], 62.73, &format!("ordinary in {width}px"));
    }
}

/// `123 Ex` fits 50px and `123 Ex-` (54.73) does not: Chrome moves `Ex` down.
/// In 56px it stays. rinch kept `123 Ex` in 50px.
#[test]
fn a_piece_that_fits_only_without_its_hyphen_moves_down() {
    breaks(
        "123 Ex&shy;traordinary",
        50,
        &["123 ", "Ex~", "traordinary"],
    );
    let laid = breaks("123 Ex&shy;traordinary", 56, &["123 Ex~", "traordinary"]);
    near(laid.advances[0], 54.73 + X_T_KERN, 0.05, "123 Ex-");
}

/// The same, broken by rinch's own pass (a text seam, #2390).
#[test]
fn the_seams_pass_fits_the_hyphen_too() {
    let row = format!("{TEXT_SEAM}Ex&shy;traordinary");
    breaks(&row, 50, &["abc-", "123 ", "Ex~", "traordinary"]);
    let laid = breaks(&row, 59, &["abc-", "123 Ex~", "traordinary"]);
    near(
        laid.advances[1],
        54.73 + X_T_KERN,
        0.05,
        "123 Ex- after the seam",
    );
    breaks(&row, 63, &["abc-123 ", "Ex~", "traordinary"]);
}

/// A soft hyphen in every syllable, at widths where the hyphens decide.
#[test]
fn every_soft_hyphen_of_a_word() {
    let word = "Ex&shy;tra&shy;or&shy;di&shy;nary";
    breaks(word, 30, &["Ex~", "tra~", "or~", "di~", "nary"]);
    breaks(word, 43, &["Ex~", "tra~or~", "di~", "nary"]);
    let laid = breaks(word, 47, &["Ex~tra~", "or~di~", "nary"]);
    close(laid.advances[0], 44.64, "Extra-");
    close(laid.advances[1], 34.77, "ordi-");
    breaks(word, 70, &["Ex~tra~or~", "di~nary"]);
    let row = format!("{TEXT_SEAM}{word}");
    breaks(
        &row,
        30,
        &["abc-", "123 ", "Ex~", "tra~", "or~", "di~", "nary"],
    );
    breaks(&row, 50, &["abc-", "123 ", "Ex~tra~", "or~di~", "nary"]);
    breaks(&row, 70, &["abc-123 ", "Ex~tra~or~", "di~nary"]);
}

/// A soft hyphen that ends a `nowrap` element breaks there (the element's
/// end is the opportunity) with no hyphen: `Extra` is 38.91.
#[test]
fn a_soft_hyphen_in_text_that_does_not_wrap_draws_none() {
    let laid = breaks(
        "<span style='white-space:nowrap'>Extra&shy;</span>ordinary",
        50,
        &["Extra~", "ordinary"],
    );
    close(laid.advances[0], 38.91, "Extra, no hyphen");
}

/// `hyphens: none` (#2621): no break at a soft hyphen, even in 1px; the soft
/// hyphen's own style decides. `manual` and `auto` (no dictionary) break.
/// Chrome 157 ignores `-webkit-hyphens: none`.
#[test]
fn hyphens_none_keeps_the_word_whole() {
    breaks_styled("Extra&shy;ordinary", 1, "hyphens:none", &["Extra~ordinary"]);
    breaks_styled(
        "Extra&shy;ordinary",
        100,
        "hyphens:none",
        &["Extra~ordinary"],
    );
    breaks_styled(
        "Extra&shy;ordinary",
        1,
        "hyphens:manual",
        &["Extra~", "ordinary"],
    );
    breaks_styled(
        "Extra&shy;ordinary",
        1,
        "hyphens:auto",
        &["Extra~", "ordinary"],
    );
    breaks_styled(
        "Extra&shy;ordinary",
        1,
        "-webkit-hyphens:none",
        &["Extra~", "ordinary"],
    );
    breaks(
        "Extra<span style='hyphens:none'>&shy;</span>ordinary",
        50,
        &["Extra~ordinary"],
    );
    breaks(
        "<span style='hyphens:none'>Extra</span>&shy;ordinary",
        50,
        &["Extra~", "ordinary"],
    );
    // Inherited from a style sheet, and through rinch's own breaks.
    let mut d = html(&format!(
        "<p id=a style='width:1px'>{TEXT_SEAM}Extra&shy;ordinary</p>"
    ));
    d.load_css("p { hyphens: none }");
    d.resolve_layout(801.0, 800.0);
    let n = d.tree.get(by_id(&d, "a").0).unwrap();
    let il = n.text_layout.as_ref().unwrap();
    let last = il.layout.lines().last().unwrap();
    assert_eq!(
        il.text_content[last.text_range()].replace('\u{ad}', "~"),
        "Extra~ordinary"
    );
}

/// #2622: Hebrew breaks at a soft hyphen and at a hyphen between letters
/// (UAX #14 LB21a as Unicode 15.1 has it), and `hyphens: none` still keeps
/// it whole.
#[test]
fn hebrew_breaks_at_a_soft_hyphen() {
    breaks("שלום&shy;עולם", 1, &["שלום~", "עולם"]);
    breaks("שלום-עולם", 1, &["שלום-", "עולם"]);
    breaks_styled("שלום&shy;עולם", 1, "direction:rtl", &["שלום~", "עולם"]);
    breaks_styled("שלום&shy;עולם", 1, "hyphens:none", &["שלום~עולם"]);
}

/// The hyphen counts for alignment: `Extra-` centred in 50px starts at 2.68.
#[test]
fn a_centred_line_is_centred_with_its_hyphen() {
    let laid = breaks_styled(
        "Extra&shy;ordinary",
        50,
        "text-align:center",
        &["Extra~", "ordinary"],
    );
    let n = laid.doc.tree.get(by_id(&laid.doc, "a").0).unwrap();
    let il = n.text_layout.as_ref().unwrap();
    let rects = rinch_dom::text_query::selection_rects_for_layout(&il.layout, 0, 1);
    close(rects[0].0, 2.68, "the line's left edge");
}

/// The soft hyphen's box is the hyphen's (Chrome's selection rect of the
/// soft hyphen is x 38.91, 5.73 wide); the caret before it stands before the
/// hyphen; a point on the hyphen's right half is after the soft hyphen
/// (Chrome puts the caret there on all of it).
#[test]
fn text_queries_see_the_hyphen() {
    let laid = breaks("Extra&shy;ordinary", 50, &["Extra~", "ordinary"]);
    let n = laid.doc.tree.get(by_id(&laid.doc, "a").0).unwrap();
    let il = n.text_layout.as_ref().unwrap();
    let layout = &il.layout;
    let rects = rinch_dom::text_query::selection_rects_for_layout(layout, 5, 7);
    assert_eq!(rects.len(), 1, "{rects:?}");
    close(rects[0].0, 38.91, "the soft hyphen's left");
    close(rects[0].2, HYPHEN, "the soft hyphen's width");
    close(rects[0].1, 0.0, "on the first line");
    let (x, y) = rinch_dom::text_query::caret_position_for_offset_layout(layout, 5);
    close(x, 38.91, "the caret before the soft hyphen");
    close(y, 0.0, "on the first line");
    assert_eq!(
        rinch_dom::text_query::byte_offset_from_position(layout, 43.5, 10.0),
        7,
        "a point on the hyphen's right half"
    );
    // A right-to-left paragraph: the hyphen after `Extra` at its right.
    let laid = breaks_styled(
        "Extra&shy;ordinary",
        50,
        "direction:rtl;text-align:left",
        &["Extra~", "ordinary"],
    );
    close(laid.advances[0], 44.64, "Extra- in a rtl paragraph");
    let n = laid.doc.tree.get(by_id(&laid.doc, "a").0).unwrap();
    let rects = rinch_dom::text_query::selection_rects_for_layout(
        &n.text_layout.as_ref().unwrap().layout,
        5,
        7,
    );
    close(rects[0].0, 38.91, "the soft hyphen's left, rtl");
    close(rects[0].2, HYPHEN, "the soft hyphen's width, rtl");
}

/// A right-to-left run draws its hyphen at its visual end, the left: the
/// soft hyphen's box is left of `שלום` (Chrome 157 draws the hyphen at
/// 15.02–20.75, `שלום` at 20.75–50; rinch's Hebrew is a fallback font, so
/// only the order and the hyphen's width are compared).
#[test]
fn a_right_to_left_run_draws_its_hyphen_at_its_left() {
    let laid = breaks_styled("שלום&shy;עולם", 50, "direction:rtl", &["שלום~", "עולם"]);
    let n = laid.doc.tree.get(by_id(&laid.doc, "a").0).unwrap();
    let layout = &n.text_layout.as_ref().unwrap().layout;
    let shy = "שלום".len();
    let hyphen = rinch_dom::text_query::selection_rects_for_layout(layout, shy, shy + 2);
    let word = rinch_dom::text_query::selection_rects_for_layout(layout, 0, shy);
    assert!(hyphen[0].2 > 3.0, "the hyphen has a width: {hyphen:?}");
    close(
        hyphen[0].0 + hyphen[0].2,
        word[0].0,
        "the hyphen ends where the word starts",
    );
    close(word[0].0 + word[0].2, 50.0, "the line is at the right");
}

/// Text seams after a line that ends at a soft hyphen: rinch's own pass
/// places parley's hyphenated line by its advance, the hyphen in it, and
/// then breaks the seams' lines after it as CSS does.
#[test]
fn seams_after_a_hyphenated_line() {
    let first = "Extra&shy;ordinary <span style='white-space:nowrap'>xy abc-</span>123 456";
    breaks(
        first,
        57,
        &["Extra~", "ordinary ", "xy abc-", "123 ", "456"],
    );
    breaks(first, 61, &["Extra~", "ordinary ", "xy abc-", "123 456"]);
    breaks(first, 85, &["Extra~", "ordinary ", "xy abc-123 ", "456"]);
    let second = "Extra&shy;ordi <span style='white-space:nowrap'>abc-</span>123 456";
    breaks(second, 57, &["Extra~", "ordi ", "abc-", "123 ", "456"]);
    breaks(second, 61, &["Extra~", "ordi ", "abc-", "123 456"]);
    breaks(second, 65, &["Extra~", "ordi ", "abc-123 ", "456"]);
}

/// The software painter draws the hyphen: dark pixels right of `Extra`
/// (which ends at 38.91), where nothing was drawn before. The GPU painter's
/// twin is `crates/rinch/tests/gpu_soft_hyphen_2614.rs`.
#[test]
fn the_software_painter_draws_the_hyphen() {
    use peniko::Brush;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    let (w, h) = (80u32, 50u32);
    let mut d = html("<div style='width:50px;background:#fff;color:#000'>Extra&shy;ordinary</div>");
    d.resolve_layout(w as f32, h as f32);
    let mut painter = TinySkiaPainter::new(w, h);
    painter.pixels_mut().fill(255);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &d.tree,
        &mut painter,
        1.0,
        (w as f32, h as f32),
        &mut d.font_cx,
        &mut layout_cx,
    );
    let darkest = |x0: u32, x1: u32, y0: u32, y1: u32| {
        let mut min = 255u8;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * w + x) * 4) as usize;
                min = min.min(painter.pixels()[i]);
            }
        }
        min
    };
    assert!(darkest(40, 44, 0, 20) < 96, "the hyphen is drawn");
    // Nothing past it on the first line, nor on the second past `ordinary`.
    assert_eq!(darkest(46, 80, 0, 20), 255);
    assert_eq!(darkest(64, 80, 20, 40), 255);
}
