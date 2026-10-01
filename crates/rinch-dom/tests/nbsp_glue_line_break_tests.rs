//! A no-break space glues; it is not a place to wrap (#1218).
//!
//! U+00A0 is UAX #14 class GL: no break after it, and none before it but
//! after a space, tab or hyphen. A word glued by one that does not fit on the
//! line is wrapped at the last opportunity *before* it, and when there is
//! none it overflows — the NBSP never ends a line on its own account.
//!
//! parley 0.11.1's line breaker hangs an overflowing NBSP exactly as it hangs
//! an overflowing space (`Whitespace::is_space_or_nbsp` in its hang branch)
//! and commits the line right after it, so `x yyx&nbsp;Wkxxpq` in 40px broke
//! as `x yyx&nbsp;` / `Wkxxpq`. `ifc::break_lines_hanging_spaces` (and
//! `break_leaf_lines`, a flex item's own text) break such a line again.
//!
//! Every expectation is Chrome 153's, on the same markup: the bundled Inter
//! registered as `ProbeFace`, `font: 16px/25px`, `* { margin: 0 }`, and the
//! lines read from `Range.getClientRects` per character.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const N: &str = "\u{a0}";

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
    doc
}

/// A `<div>` with `style` (after the font) holding the one text `text`, laid out.
fn lay_out(style: &str, text: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font: 16px/25px ProbeFace; {style}"));
    d.append_child(body, c);
    let t = d.create_text(text);
    d.append_child(c, t);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

/// The text of each line of the container's inline layout.
fn lines(style: &str, text: &str) -> Vec<String> {
    let (d, c) = lay_out(style, text);
    let node = d.tree.get(c.0).unwrap();
    let il = node
        .text_layout
        .as_ref()
        .expect("the container is an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].to_string())
        .collect()
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.replace('~', N)).collect()
}

/// The issue's two rows. In the first the glued word overflows the line it
/// is wrapped onto; in the second it fits there (61.66 of 64px), and rinch
/// had kept `mno&nbsp;` on the first line, 66.6px wide.
#[test]
fn a_glued_word_that_does_not_fit_wraps_at_the_opportunity_before_it() {
    assert_eq!(
        lines("width: 40px", &"x yyx~Wkxxpq".replace('~', N)),
        s(&["x ", "yyx~Wkxxpq"])
    );
    assert_eq!(
        lines("width: 64px", &"Wk mno~pq~".replace('~', N)),
        s(&["Wk ", "mno~pq~"])
    );
}

/// The opportunity before the glued word is a hyphen's, not a space's — and
/// a hyphen right before the NBSP is one too (LB12a: no break before GL but
/// after a space, BA or HY), for U+2010 HYPHEN as for `-`.
#[test]
fn the_opportunity_before_the_glued_word_can_be_a_hyphen() {
    assert_eq!(
        lines("width: 38px", &"a-bb~cc".replace('~', N)),
        s(&["a-", "bb~cc"])
    );
    assert_eq!(
        lines("width: 28px", &"aa-~bb".replace('~', N)),
        s(&["aa-", "~bb"])
    );
    assert_eq!(
        lines("width: 28px", &"aa\u{2010}~bb".replace('~', N)),
        s(&["aa\u{2010}", "~bb"])
    );
    assert_eq!(
        lines("width: 40px", &"x aa-~bb".replace('~', N)),
        s(&["x aa-", "~bb"])
    );
}

/// With no opportunity before it on the line the glued word overflows, and
/// the line ends at the first opportunity after it — through every NBSP of a
/// run and every NBSP of a chain.
#[test]
fn with_no_opportunity_before_it_the_glued_word_overflows() {
    assert_eq!(
        lines("width: 40px", &"yyyyyy~b c".replace('~', N)),
        s(&["yyyyyy~b ", "c"])
    );
    assert_eq!(
        lines("width: 40px", &"yyyy~~bb cc".replace('~', N)),
        s(&["yyyy~~bb ", "cc"])
    );
    assert_eq!(
        lines("width: 40px", &"x aa~bb~cc dd".replace('~', N)),
        s(&["x ", "aa~bb~cc ", "dd"])
    );
}

/// `pre-wrap` keeps the spaces and hangs them, and still glues at the NBSP.
#[test]
fn pre_wrap_text_glues_at_an_nbsp_too() {
    assert_eq!(
        lines(
            "width: 64px; white-space: pre-wrap",
            &"Wk mno~pq~".replace('~', N)
        ),
        s(&["Wk ", "mno~pq~"])
    );
    assert_eq!(
        lines(
            "width: 64px; white-space: pre-wrap",
            &"Wk  mno~pq".replace('~', N)
        ),
        s(&["Wk  ", "mno~pq"])
    );
}

/// `overflow-wrap` lets a word break where it has to — and with no other
/// opportunity Chrome breaks right *before* the NBSP, which then starts the
/// next line. (A regular opportunity still wins over it.)
#[test]
fn overflow_wrap_breaks_before_the_nbsp_when_nothing_else_can() {
    for wrap in ["break-word", "anywhere"] {
        assert_eq!(
            lines(
                &format!("width: 40px; overflow-wrap: {wrap}"),
                &"yyyy~bb".replace('~', N)
            ),
            s(&["yyyy", "~bb"]),
            "{wrap}"
        );
    }
    assert_eq!(
        lines(
            "width: 40px; overflow-wrap: break-word",
            &"x yyyy~bb".replace('~', N)
        ),
        s(&["x ", "yyyy", "~bb"])
    );
}

/// A flex item's own text is broken by the leaf path, not an IFC: Chrome lays
/// `x aa&nbsp;bb&nbsp;cc dd` out in three lines there too (75px).
#[test]
fn a_flex_items_own_text_glues_at_an_nbsp() {
    let (d, c) = lay_out(
        "width: 40px; display: flex; flex-direction: column",
        &"x aa~bb~cc dd".replace('~', N),
    );
    assert_eq!(d.tree.get(c.0).unwrap().layout.height, 75.0);
}

/// On a line after a forced break, the `overflow-wrap` break before the NBSP
/// still lands right before it.
#[test]
fn overflow_wrap_breaks_before_the_nbsp_on_a_line_after_a_newline() {
    assert_eq!(
        lines(
            "width: 40px; white-space: pre-wrap; overflow-wrap: break-word",
            &"aa\nyyyy~bb".replace('~', N)
        ),
        s(&["aa\n", "yyyy", "~bb"])
    );
}

/// The lines (text only: a line holding nothing but a box has none) and the
/// height of a 40px container holding `html`.
fn box_lines(html: &str) -> (Vec<String>, f32) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", "font: 16px/25px ProbeFace; width: 40px");
    d.append_child(body, c);
    d.set_inner_html(c, &html.replace('~', "&nbsp;"));
    d.resolve_layout(800.0, 600.0);
    let node = d.tree.get(c.0).unwrap();
    let il = node.text_layout.as_ref().expect("an IFC root");
    let lines = il
        .layout
        .lines()
        .map(|l| {
            il.text_content
                .get(l.text_range())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    (lines, node.layout.height)
}

/// Chrome 153 breaks between an atomic inline and the NBSP after it, a box
/// too wide for any line included. (Only the first height is compared: a line holding nothing but the box is
/// the box's 10px tall in rinch and a whole 25px line in Chrome, which has
/// nothing to do with the NBSP.)
#[test]
fn an_atomic_inline_before_the_nbsp_is_an_opportunity() {
    const B: &str = "display: inline-block; height: 10px; vertical-align: top";
    assert_eq!(
        box_lines(&format!("x <span style=\"{B}; width: 26px\"></span>~bb")),
        (s(&["x ", "~bb"]), 50.0)
    );
    assert_eq!(
        box_lines(&format!("<span style=\"{B}; width: 50px\"></span>~bb cc")).0,
        s(&["", "~bb ", "cc"])
    );
}
