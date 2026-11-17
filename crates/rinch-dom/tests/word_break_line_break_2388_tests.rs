//! `word-break` and `line-break` (#2388). rinch read neither, so every value
//! laid out as `normal`. Wiring them to parley is not enough on its own:
//! rinch hands parley Chrome's ASCII pair table as a line-break override
//! (`ifc::CHROME_LINE_BREAKS`, #2369), which parley applies after its own
//! `break-all` and `anywhere` handling, and the table says "no break" for
//! almost every printable ASCII pair. Blink consults the table only for
//! `normal`, `break-all` and `keep-all`; under `break-all` a "no" from it
//! does not stop the break-all rule, and under `line-break: anywhere` it is
//! not used at all. So the override steps aside there.
//!
//! Every expectation is Chrome 156's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/20px`, `* { margin: 0 }`, lines
//! read from `Range.getClientRects` per character.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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

/// The text of each line of `#id`'s inline layout.
fn lines(d: &RinchDocument, id: &str) -> Vec<String> {
    let n = d.tree.get(by_id(d, id).0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].to_string())
        .collect()
}

fn one(style: &str, text: &str) -> Vec<String> {
    let d = html(&format!("<div id=a style='{style}'>{text}</div>"));
    lines(&d, "a")
}

/// The issue's first row: `break-all` breaks between letters, and keeps the
/// table's glue where it has none of its own (`s://`, `x.`, `m/`, `a/`).
/// rinch made one line.
#[test]
fn break_all_breaks_inside_a_url() {
    assert_eq!(
        one("width:1px;word-break:break-all", "https://ex.com/a/b"),
        [
            "h", "t", "t", "p", "s://", "e", "x.", "c", "o", "m/", "a/", "b"
        ]
    );
}

/// At a width, `break-all` fills each line.
#[test]
fn break_all_fills_each_line() {
    assert_eq!(
        one("width:60px;word-break:break-all", "abcdefghijklmnop"),
        ["abcdef", "ghijklm", "nop"]
    );
}

/// `line-break: anywhere` puts every character on its own line at 1px.
#[test]
fn line_break_anywhere_breaks_between_every_character() {
    assert_eq!(
        one("width:1px;line-break:anywhere", "https://ex.com/a"),
        [
            "h", "t", "t", "p", "s", ":", "/", "/", "e", "x", ".", "c", "o", "m", "/", "a"
        ]
    );
}

/// `keep-all` keeps a run of Han characters whole; `normal` breaks it.
#[test]
fn keep_all_keeps_han_together() {
    assert_eq!(
        one("width:1px;word-break:keep-all", "a-b a?b 日本語"),
        ["a-", "b ", "a?", "b ", "日本語"]
    );
    assert_eq!(
        one("width:1px", "a-b a?b 日本語"),
        ["a-", "b ", "a?", "b ", "日", "本", "語"]
    );
}

/// The property is the span's: a `normal` span inside a `break-all` block
/// keeps its URL whole.
#[test]
fn a_normal_span_in_a_break_all_block() {
    let d = html(
        "<div id=a style='width:1px;word-break:break-all'>ab <span style='word-break:normal'>cd/ef</span></div>",
    );
    assert_eq!(lines(&d, "a"), ["a", "b ", "cd/ef"]);
    // A `display: contents` element's too (Chrome: 60 tall, three lines):
    // it pushes a span only when its style differs from its parent's.
    let d = html(
        "<div id=a style='width:1px;word-break:break-all'>ab <span style='display:contents;word-break:normal'>cd/ef</span></div>",
    );
    assert_eq!(lines(&d, "a"), ["a", "b ", "cd/ef"]);
}

/// `strict` and `loose` give Chrome's lines for ASCII, as they did.
#[test]
fn strict_and_loose_keep_the_ascii_table() {
    for lb in ["strict", "loose"] {
        assert_eq!(
            one(&format!("width:1px;line-break:{lb}"), "a-b a?b a/b"),
            ["a-", "b ", "a?", "b ", "a/b"],
            "{lb}"
        );
    }
}

/// A restyle that sets `word-break` lays the text out again.
#[test]
fn a_restyle_to_break_all_breaks_again() {
    let mut d = html("<div id=a style='width:60px'>abcdefghijklmnop</div>");
    assert_eq!(lines(&d, "a"), ["abcdefghijklmnop"]);
    let a = by_id(&d, "a");
    d.set_attribute(a, "style", "width:60px;word-break:break-all");
    d.resolve_layout(800.0, 800.0);
    assert_eq!(lines(&d, "a"), ["abcdef", "ghijklm", "nop"]);
}

/// A flex item's own text (a text leaf, `ifc::shape_text_leaf`) reads them
/// too: Chrome makes the 60px container 60 tall under `break-all` and
/// `anywhere` (three lines), 20 under `normal` (one overflowing line).
#[test]
fn a_flex_items_text_reads_them() {
    let d = html(concat!(
        "<div id=a style='display:flex;width:60px;word-break:break-all'>abcdefghijklmnop</div>",
        "<div id=b style='display:flex;width:60px'>abcdefghijklmnop</div>",
        "<div id=c style='display:flex;width:60px;line-break:anywhere'>abcdefghijklmnop</div>",
    ));
    let h = |id: &str| d.tree.get(by_id(&d, id).0).unwrap().layout.height;
    assert_eq!((h("a"), h("b"), h("c")), (60.0, 20.0, 60.0));
}
