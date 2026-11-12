//! #2542 — an inline box takes the bidi level UAX #9 gives the object
//! replacement character it stands for, a neutral: N1 counts European and
//! Arabic numbers as right to left, so a box between a Hebrew letter and
//! digits in a left-to-right paragraph runs right to left with them.
//!
//! Visual order only, from left to right, so the pins hold with any fonts:
//! Chrome 157 (`--headless=new`, `font: 16px/25px sans-serif`, 400px wide),
//! spans around the pieces (`unicode-bidi: normal`, no effect on the order):
//!
//! | paragraph | Chrome, left to right |
//! |---|---|
//! | `x א<img>1234567890` | `x`, digits 12.5, image 101.4, `א` 131.4 |
//! | `x א<img>١٢٣٤٥` | `x`, digits 12.5, image 54.5, `א` 84.5 |
//! | `x א <img> 123` | `x`, digits 12.5, image 43.6, `א` 78.1 |
//!
//! The box used to take the level of the run before it compared with the one
//! after it, and the digits' level 2 read as left to right: the box was put
//! between `x` and the digits (rinch 26.8 for the first, Chrome 126.8 at
//! another size).

use parley::layout::PositionedLayoutItem;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// The left edges of the run holding `hebrew`, the image, and the run
/// holding `digits`, in a paragraph `inner`.
fn order(inner: &str, hebrew: &str, digits: &str) -> (f32, f32, f32) {
    let mut d = RinchDocument::new();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", "font:16px/25px sans-serif;width:400px");
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    let img = d.query_selector_all("img")[0].0;
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let text = &il.text_content;
    let byte_of = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s} in {text:?}"));
    let (h, n) = (byte_of(hebrew), byte_of(digits));
    let (mut hx, mut ix, mut nx) = (None, None, None);
    let line = il.layout.lines().next().unwrap();
    assert_eq!(il.layout.lines().count(), 1, "{inner}: one line");
    for item in line.items() {
        match item {
            PositionedLayoutItem::InlineBox(b) if b.id == img => ix = Some(b.x),
            PositionedLayoutItem::GlyphRun(g) => {
                let r = g.run().text_range();
                if r.contains(&h) {
                    hx = Some(g.offset());
                }
                if r.contains(&n) {
                    nx = Some(g.offset());
                }
            }
            _ => {}
        }
    }
    (hx.unwrap(), ix.unwrap(), nx.unwrap())
}

#[test]
fn a_box_between_hebrew_and_digits_runs_right_to_left_with_them() {
    let img = r#"<img style="width:30px;height:10px">"#;
    for (inner, digits) in [
        (format!("x \u{5d0}{img}1234567890"), "1234567890"),
        (
            format!("x \u{5d0}{img}\u{661}\u{662}\u{663}\u{664}\u{665}"),
            "\u{661}",
        ),
        (format!("x \u{5d0} {img} 123"), "123"),
    ] {
        let (hebrew, image, digit) = order(&inner, "\u{5d0}", digits);
        assert!(
            digit < image && image < hebrew,
            "{inner}: Chrome's order is digits, image, Hebrew; rinch digits {digit}, image {image}, Hebrew {hebrew}"
        );
    }
}
