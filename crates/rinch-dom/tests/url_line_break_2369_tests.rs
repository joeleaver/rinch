//! A URL stays whole where Chrome keeps it whole (#2369, #1910).
//!
//! UAX #14 has a break opportunity after a solidus (`/`, class SY) before a
//! letter: no rule keeps `SY AL` together (LB13 only forbids a break
//! *before* SY, LB25 keeps `SY NU`), so LB31 breaks there, and ICU4X —
//! parley's segmenter — answers so. The same holds after `!` (EX, LB31), `|`
//! (BA, LB21 forbids only the break before it) and `}` (CL, LB30 has no
//! `CL × AL`). Chrome does not take UAX #14's answer for printable ASCII:
//! Blink's `NextBreakablePosition` reads its own ASCII pair table
//! (`character_property_data_generator.cc`, `kAsciiLineBreakTable`), which
//! has no break after any of those four, and has one after a hyphen before
//! a digit when a letter or digit comes before the hyphen (`cccc-1234`),
//! where LB25 has none. So `https://example.com/a/long/url` broke as
//! `https://` / `example.com/` / `a/long/url` in a 120px box, and is one
//! overflowing line in Chrome.
//!
//! Every expectation is Chrome 156's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0 }`, lines
//! read from `Range.getClientRects` per character. A box `width: 1px` takes
//! every opportunity there is, so its lines list them all.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    doc
}

/// The text of each line of `node`'s inline layout.
fn lines_of(d: &RinchDocument, node: NodeId) -> Vec<String> {
    let n = d.tree.get(node.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].to_string())
        .collect()
}

/// A `<div>` with `style` holding the one text `text`, laid out.
fn lay_out(style: &str, text: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font: 16px/20px ProbeFace; {style}"));
    d.append_child(body, c);
    let t = d.create_text(text);
    d.append_child(c, t);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

fn lines(style: &str, text: &str) -> Vec<String> {
    let (d, c) = lay_out(style, text);
    lines_of(&d, c)
}

fn html(markup: &str) -> RinchDocument {
    let mut d = doc();
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 600.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

fn size(d: &RinchDocument, id: &str) -> (f32, f32) {
    let l = d.tree.get(by_id(d, id).0).unwrap().layout;
    (l.width, l.height)
}

/// The widest line's advance.
fn widest_line(d: &RinchDocument, node: NodeId) -> f32 {
    let n = d.tree.get(node.0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| l.metrics().advance)
        .fold(0.0, f32::max)
}

/// The issue's own row: one line, 234px wide, overflowing its 120px box.
/// rinch made three, `https://` / `example.com/` / `a/long/url`.
#[test]
fn a_url_is_one_overflowing_line_in_a_narrow_box() {
    let (d, c) = lay_out("width: 120px", "https://example.com/a/long/url");
    assert_eq!(lines_of(&d, c), ["https://example.com/a/long/url"]);
    // Chrome 156: 234 (its scrollWidth; the span is 234 too).
    assert!(
        (widest_line(&d, c) - 234.0).abs() < 0.5,
        "{}",
        widest_line(&d, c)
    );
    assert_eq!(d.tree.get(c.0).unwrap().layout.height, 20.0);
}

/// The URL's own opportunities, at `width: 1px`: none after a slash, before
/// a letter or a digit; one after `?` and after `-`.
#[test]
fn a_urls_opportunities_are_chromes() {
    let w0 = "width: 1px";
    assert_eq!(
        lines(w0, "https://example.com/a/long/url"),
        ["https://example.com/a/long/url"]
    );
    assert_eq!(
        lines(w0, "https://example.com/search?q=rinch&lang=en_US#top"),
        ["https://example.com/search?", "q=rinch&lang=en_US#top"]
    );
    assert_eq!(
        lines(w0, "https://ex-ample.com/path-to/the-thing-2024"),
        [
            "https://ex-",
            "ample.com/path-",
            "to/the-",
            "thing-",
            "2024"
        ]
    );
    assert_eq!(
        lines(w0, "https://example.com/2024/11/07/post-9"),
        ["https://example.com/2024/11/07/post-", "9"]
    );
    assert_eq!(
        lines(w0, "see (https://example.com/a/b) here"),
        ["see ", "(https://example.com/a/b) ", "here"]
    );
    assert_eq!(
        lines(w0, "a/b a/1 1/a 1/2 a//b a/(b) a/-b"),
        [
            "a/b ", "a/1 ", "1/a ", "1/2 ", "a//b ", "a/(b) ", "a/-", "b"
        ]
    );
}

/// Every printable ASCII mark between letters and digits, at `width: 1px`.
/// Only `?` and `-` offer a break; `/`, `!`, `|` and `}` are the four
/// where UAX #14 (ICU4X) has one and Chrome has none.
#[test]
fn ascii_punctuation_between_letters_and_digits_breaks_as_in_chrome() {
    let marks = "/?&=-._!|}:#%~+@,;)]\"'*$^`<>([{\\";
    for p in marks.chars() {
        let text = format!("cccc{p}dddd cccc{p}1234 1234{p}dddd 1234{p}5678");
        let want: Vec<String> = if matches!(p, '?' | '-') {
            [
                "cccc{p}", "dddd ", "cccc{p}", "1234 ", "1234{p}", "dddd ", "1234{p}", "5678",
            ]
            .iter()
            .map(|s| s.replace("{p}", &p.to_string()))
            .collect()
        } else {
            [
                "cccc{p}dddd ",
                "cccc{p}1234 ",
                "1234{p}dddd ",
                "1234{p}5678",
            ]
            .iter()
            .map(|s| s.replace("{p}", &p.to_string()))
            .collect()
        };
        assert_eq!(lines("width: 1px", &text), want, "mark {p:?}");
    }
}

/// A hyphen before a digit breaks only after a letter or digit: `-5` after
/// a space or an opening parenthesis may be a minus sign (Blink's rule).
#[test]
fn a_hyphen_before_a_digit_breaks_only_after_a_letter_or_digit() {
    assert_eq!(
        lines("width: 1px", "x -5 (-5) a-5 5-5 .-5"),
        ["x ", "-5 ", "(-5) ", "a-", "5 ", "5-", "5 ", ".-5"]
    );
}

/// A narrow box with `overflow-wrap` breaks the URL anywhere once no
/// opportunity fits, and with `normal` it overflows on a line of its own.
#[test]
fn a_long_url_in_a_narrow_box() {
    let text = "Visit https://example.com/a/very/long/path/to/a/thing today";
    assert_eq!(
        lines("width: 100px", text),
        [
            "Visit ",
            "https://example.com/a/very/long/path/to/a/thing ",
            "today"
        ]
    );
    assert_eq!(
        lines(
            "width: 100px; overflow-wrap: break-word",
            "https://example.com/a/very/long/path"
        ),
        ["https://exam", "ple.com/a/ve", "ry/long/path"]
    );
}

/// CJK and Thai next to a URL still break between their own characters and
/// words; the URL between them stays whole.
#[test]
fn cjk_and_thai_beside_a_url_still_break() {
    assert_eq!(
        lines("width: 1px", "日本語https://example.com/a/b日本語"),
        [
            "日",
            "本",
            "語",
            "https://example.com/a/b",
            "日",
            "本",
            "語"
        ]
    );
    assert_eq!(
        lines("width: 1px", "日本語 https://example.com/a/b 日本語"),
        [
            "日",
            "本",
            "語 ",
            "https://example.com/a/b ",
            "日",
            "本",
            "語"
        ]
    );
    assert_eq!(
        lines(
            "width: 1px",
            "ภาษาไทยง่ายมาก https://example.com/a/b ภาษาไทยง่ายมาก"
        ),
        [
            "ภาษา",
            "ไทย",
            "ง่าย",
            "มาก ",
            "https://example.com/a/b ",
            "ภาษา",
            "ไทย",
            "ง่าย",
            "มาก"
        ]
    );
}

/// A mail signature: a URL beside a float does not fit the 120px beside
/// it, and its line is the whole URL (Chrome 156: three lines). rinch broke
/// it after `https://`, which fit beside the float.
#[test]
fn a_url_beside_a_float_stays_whole() {
    let d = html(
        "<div id=f style='width:200px'><div style='float:left;width:80px;height:40px'></div>\
         Sent from https://example.com/a/long/url and more text after it</div>",
    );
    assert_eq!(
        lines_of(&d, by_id(&d, "f")),
        [
            "Sent from ",
            "https://example.com/a/long/url ",
            "and more text after it"
        ]
    );
}

/// Chrome moves that line below the 40px float, so the box is 80px tall;
/// rinch leaves an overflowing line beside the float (60px). That is #2360,
/// fixed by PR #2374's float work, not by the break opportunities.
#[test]
#[ignore = "needs #2360 (PR #2374): a line too wide beside a float moves below it"]
fn a_url_beside_a_float_moves_below_it_at_chromes_height() {
    let d = html(
        "<div id=f style='width:200px'><div style='float:left;width:80px;height:40px'></div>\
         Sent from https://example.com/a/long/url and more text after it</div>",
    );
    assert_eq!(size(&d, "f"), (200.0, 80.0));
}

/// A mail footer: a table cell's min-content width is the whole URL's, so
/// a table narrower than it grows to 234 (Chrome 156), and so does a cell
/// with a smaller `width`.
#[test]
fn a_url_in_a_table_cell_widens_the_table_to_the_url() {
    let d = html(
        "<table id=t style='width:150px;border-collapse:collapse' cellpadding=0 cellspacing=0>\
         <tr><td id=td>Visit https://example.com/a/long/url</td></tr></table>\
         <table id=t2 cellpadding=0 cellspacing=0><tr><td id=td2 style='width:100px'>\
         Visit https://example.com/a/long/url</td><td>x</td></tr></table>",
    );
    assert_eq!(
        lines_of(&d, by_id(&d, "td")),
        ["Visit ", "https://example.com/a/long/url"]
    );
    let (tw, th) = size(&d, "t");
    assert!((tw - 234.0).abs() < 0.5 && th == 40.0, "{tw}x{th}");
    let (w2, h2) = size(&d, "td2");
    assert!((w2 - 234.0).abs() < 0.5 && h2 == 40.0, "{w2}x{h2}");
}

/// Every pair of printable ASCII characters, between letters (`a` x y `b`)
/// and between digits (`1` x y `1`): one paragraph per x of the 94 words,
/// joined by spaces, in a 1px box, against Chrome 156's line starts on the
/// same text (`url_line_break_2369/chrome156_ascii_pairs.tsv`: a context,
/// x in hex, then each word's three inner positions, `1` where a line
/// starts). This is the whole of Blink's ASCII pair table as it shows.
#[test]
fn every_printable_ascii_pair_breaks_as_in_chrome() {
    // Inter shapes `->` as one arrow glyph (`calt`), and parley cannot end
    // a line inside a ligature; Chrome breaks it there and reshapes (#2381).
    // The table has the opportunity (`ifc::chrome_break_tests`).
    const LIGATURES: [&str; 2] = ["a->b", "1->1"];
    let table = include_str!("url_line_break_2369/chrome156_ascii_pairs.tsv");
    let ys: Vec<char> = (0x21u8..=0x7e).map(char::from).collect();
    let mut wrong = Vec::new();
    let mut rows = 0;
    for row in table.lines() {
        let mut f = row.split('\t');
        let (ctx, x, masks) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        let x = char::from(u8::from_str_radix(x, 16).unwrap());
        let (l, r) = if ctx == "a" { ('a', 'b') } else { ('1', '1') };
        let words: Vec<String> = ys.iter().map(|y| format!("{l}{x}{y}{r}")).collect();
        let text = words.join(" ");
        // The byte offsets where rinch starts a line.
        let starts: std::collections::HashSet<usize> = {
            let (d, c) = lay_out("width: 1px", &text);
            let n = d.tree.get(c.0).unwrap();
            let il = n.text_layout.as_ref().unwrap();
            assert_eq!(il.text_content, text);
            il.layout.lines().map(|l| l.text_range().start).collect()
        };
        for ((w, word), want) in words.iter().enumerate().zip(masks.split(' ')) {
            let got: String = (1..=3)
                .map(|k| {
                    if starts.contains(&(w * 5 + k)) {
                        '1'
                    } else {
                        '0'
                    }
                })
                .collect();
            if got != want && !LIGATURES.contains(&word.as_str()) {
                wrong.push(format!("{word:?}: chrome {want} rinch {got}"));
            }
        }
        rows += 1;
    }
    assert_eq!(rows, 188, "two contexts of 94 rows");
    assert!(
        wrong.is_empty(),
        "{} differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
