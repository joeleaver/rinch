//! Two text queries pinned to Chrome 157, each measured with the same markup
//! and the same face (the bundled Inter, registered under an override name so
//! the host's fonts cannot answer instead):
//!
//! - **#2575**: `glyph_bounds_for_offset` of the character right after an
//!   inline-block is that character's box, after the inline-block. An inline
//!   box has no bytes, so it and the text after it share one flat byte; the
//!   caret there is drawn before the box (#1104), and the glyph bounds took
//!   their x from that caret.
//! - **The caret over the glyphs**: a caret in a paragraph with a
//!   `line-height` other than the face's own is drawn over its text's content
//!   area — its font's rounded ascent above the baseline to its rounded
//!   descent below — not over the line box. At 16px Inter that is 20px tall,
//!   so under `line-height: 48px` Chrome starts it 14px down the line; rinch
//!   drew it 48px tall from the line's top.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{CaretAffinity, DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::glyph_bounds_for_offset;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// Chrome rounds a client rect to 1/64 px.
const TOLERANCE: f32 = 0.1;

fn document() -> RinchDocument {
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
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0");
    d
}

/// `<div>ab cd<span inline-block 12px></span> ef gh</div>`, 16px/25px.
/// Chrome's left edge and width of every character (`Range.getClientRects`);
/// the box is at 42.22, 12 wide.
#[test]
fn the_glyph_after_an_inline_block_is_after_the_box_2575() {
    let mut d = document();
    let body = d.body();
    let div = d.create_element("div");
    d.set_attribute(
        div,
        "style",
        "width: 300px; font-family: ProbeFace; font-size: 16px; line-height: 25px",
    );
    d.append_child(body, div);
    let before = d.create_text("ab cd");
    d.append_child(div, before);
    let ib = d.create_element("span");
    d.set_attribute(
        ib,
        "style",
        "display: inline-block; width: 12px; height: 10px",
    );
    d.append_child(div, ib);
    let after = d.create_text(" ef gh");
    d.append_child(div, after);
    d.resolve_layout(400.0, 300.0);

    // `(flat byte, Chrome's left edge, Chrome's width)`; the inline box has
    // no byte, so " ef gh" starts at byte 5.
    let chrome: &[(usize, f32, f32)] = &[
        (0, 0.00, 8.98),
        (1, 8.98, 9.80),
        (2, 18.78, 4.50),
        (3, 23.28, 9.14),
        (4, 32.42, 9.80),
        (5, 54.22, 4.50),
        (6, 58.72, 9.33),
        (7, 68.05, 5.92),
        (8, 73.97, 4.50),
        (9, 78.47, 9.81),
        (10, 88.28, 9.47),
    ];
    let mut wrong = Vec::new();
    for &(byte, x, width) in chrome {
        let g = glyph_bounds_for_offset(&d, div.0, byte).expect("a glyph");
        if (g.x - x).abs() > TOLERANCE || (g.width - width).abs() > TOLERANCE {
            wrong.push(format!(
                "byte {byte}: Chrome x {x} w {width}, rinch x {} w {}",
                g.x, g.width
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A `contenteditable` paragraph, `margin: 0`, in the face at `size` with
/// `line_height`; `parts` are plain text (`""`) or a span with that style.
fn paragraph(
    d: &mut RinchDocument,
    size: &str,
    line_height: &str,
    parts: &[(&str, &str)],
) -> NodeId {
    let body = d.body();
    let p = d.create_element("p");
    d.set_attribute(p, "contenteditable", "");
    d.set_attribute(
        p,
        "style",
        &format!(
            "margin: 0; width: 300px; font-family: ProbeFace; font-size: {size}; \
             line-height: {line_height}"
        ),
    );
    d.append_child(body, p);
    for (style, text) in parts {
        let t = d.create_text(text);
        if style.is_empty() {
            d.append_child(p, t);
        } else {
            let span = d.create_element("span");
            d.set_attribute(span, "style", style);
            d.append_child(p, span);
            d.append_child(span, t);
        }
    }
    p
}

/// Every caret `(flat byte, Chrome's top from the paragraph's top, Chrome's
/// height)` of each paragraph, as the editor's caret overlay is placed
/// (`query_caret_box_with_affinity`). Chrome's caret is its collapsed
/// `Range`'s rect, the selection's too.
#[test]
fn a_caret_is_drawn_over_its_texts_content_area_not_the_line_box() {
    struct Case {
        id: &'static str,
        size: &'static str,
        line_height: &'static str,
        parts: &'static [(&'static str, &'static str)],
        carets: &'static [(usize, f32, f32)],
    }
    let cases = [
        Case {
            id: "16px/48px",
            size: "16px",
            line_height: "48px",
            parts: &[("", "Hello world")],
            carets: &[(0, 14.0, 20.0), (3, 14.0, 20.0), (11, 14.0, 20.0)],
        },
        // A line box shorter than the glyphs: the caret stands out of it.
        Case {
            id: "16px/10px",
            size: "16px",
            line_height: "10px",
            parts: &[("", "Hello")],
            carets: &[(0, -5.0, 20.0), (2, -5.0, 20.0), (5, -5.0, 20.0)],
        },
        Case {
            id: "20px/41px",
            size: "20px",
            line_height: "41px",
            parts: &[("", "Hello")],
            carets: &[(0, 8.0, 24.0), (2, 8.0, 24.0), (5, 8.0, 24.0)],
        },
        // Two sizes on one line: each caret is its own text's. At a boundary
        // (bytes 2 and 4) it is the text it follows, as Chrome draws the
        // caret at the end of a text node.
        Case {
            id: "mixed",
            size: "16px",
            line_height: "48px",
            parts: &[("", "aa"), ("font-size: 32px", "bb"), ("", "cc")],
            carets: &[
                (0, 19.0, 20.0),
                (1, 19.0, 20.0),
                (2, 19.0, 20.0),
                (3, 4.0, 39.0),
                (4, 4.0, 39.0),
                (5, 19.0, 20.0),
                (6, 19.0, 20.0),
            ],
        },
    ];
    let mut wrong = Vec::new();
    let mut checked = 0;
    for case in &cases {
        let mut d = document();
        let p = paragraph(&mut d, case.size, case.line_height, case.parts);
        d.resolve_layout(400.0, 300.0);
        for &(byte, want_top, want_h) in case.carets {
            checked += 1;
            let (_, y, h) = d
                .query_caret_box_with_affinity(p.0, byte, CaretAffinity::Downstream)
                .expect("a caret box");
            if (y - want_top).abs() > TOLERANCE || (h - want_h).abs() > TOLERANCE {
                wrong.push(format!(
                    "{} byte {byte}: Chrome top {want_top} h {want_h}, rinch top {y} h {h}",
                    case.id
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!(checked, 16, "every listed caret was checked");
}
