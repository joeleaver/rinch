//! A `<textarea>`'s text breaks where ordinary text does (#2387): Chrome uses
//! one line breaker for both. `form_control::field_text_layout` builds the
//! text through `ComputedStyle::build_parley_layout` (#320), which now hands
//! parley Chrome's ASCII pair table (`ifc::CHROME_LINE_BREAKS`) and the
//! control's `word-break` and `line-break` (#2388). Before, a textarea broke
//! by plain UAX #14: after every `/`, `!`, `|` and `}` before a letter.
//!
//! Every expectation is Chrome 156's line count (`scrollHeight` / 20) for a
//! `<textarea>` styled `font: 16px/20px` (the bundled Inter as `ProbeFace`),
//! `padding: 0; border: 0`.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The lines of a textarea styled `style` holding `value`.
fn lines(style: &str, value: &str) -> Vec<String> {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    d.load_css("* { margin: 0 } textarea { font: 16px/20px ProbeFace; padding: 0; border: 0 }");
    let body = d.body();
    d.set_inner_html(body, &format!("<textarea id=t style='{style}'></textarea>"));
    let t = d.query_selector("#t").unwrap();
    d.set_attribute(t, "value", value);
    d.resolve_layout(800.0, 600.0);
    let node = d.tree.get(t.0).unwrap();
    let mut layout_cx = parley::LayoutContext::new();
    let layout = rinch_dom::form_control::field_text_layout(
        node,
        value,
        1.0,
        None,
        &mut d.font_cx,
        &mut layout_cx,
    );
    layout
        .lines()
        .map(|l| value[l.text_range()].to_string())
        .collect()
}

/// The issue's row: Chrome keeps the URL on one overflowing line, two lines
/// in all. rinch broke after every `/`.
#[test]
fn a_url_stays_whole_in_a_textarea() {
    assert_eq!(
        lines(
            "width:120px;overflow-wrap:normal",
            "https://example.com/a/long/url end"
        ),
        ["https://example.com/a/long/url ", "end"]
    );
}

/// No break after `/`, `!`, `|` or `}` before a letter: four lines at 40px,
/// one per token (UAX #14 made eight).
#[test]
fn ascii_punctuation_glues_as_chrome_has_it() {
    assert_eq!(
        lines(
            "width:40px;overflow-wrap:normal",
            "cccc/dddd cccc!dddd cccc|dddd cccc}dddd"
        )
        .len(),
        4
    );
}

/// `word-break: break-all` reaches the control too: three lines.
#[test]
fn break_all_in_a_textarea() {
    assert_eq!(
        lines(
            "width:120px;overflow-wrap:normal;word-break:break-all",
            "https://example.com/a/long/url end"
        )
        .len(),
        3
    );
}

/// A textarea's UA `overflow-wrap: break-word` breaks the URL at the box:
/// three lines.
#[test]
fn the_ua_break_word_breaks_a_long_url() {
    assert_eq!(
        lines("width:120px", "https://example.com/a/long/url end").len(),
        3
    );
}
