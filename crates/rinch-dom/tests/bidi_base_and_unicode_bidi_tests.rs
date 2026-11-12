//! #2594 — a paragraph's bidi base direction is its block's `direction`, not
//! its first strong character's; #2588 — `unicode-bidi` (`embed`, `isolate`,
//! `bidi-override`, `isolate-override`, `plaintext`) on an inline element, and
//! `bidi-override` / `plaintext` on a block, as Chrome lays them out.
//!
//! Visual order only, so the pins hold with any fonts: Chrome 157
//! (`--headless=new`, `font: 16px/25px sans-serif`, 400px wide), the left
//! edge of each word relative to the paragraph (`<b>` around the words,
//! `unicode-bidi: normal`, no effect on the order):
//!
//! | paragraph | Chrome |
//! |---|---|
//! | `direction: ltr`, `<img 30px>אבג דהו` | image 0, Hebrew 30 |
//! | `אבג abc` | Hebrew 0, `abc` 30.3 |
//! | `unicode-bidi: plaintext` block, `אבג abc` | Hebrew 374.2, `abc` 342.1 |
//! | `direction: rtl; unicode-bidi: plaintext` block, `abc אבג` | `abc` 0, Hebrew 32 |
//! | `<span rtl>abc אבג</span>` (`normal`) | `abc` 0, Hebrew 32 |
//! | `<span rtl isolate>abc אבג</span>` (and `embed`, `isolate-override`) | Hebrew 0, `abc` 30.3 |
//! | `abc <span plaintext>אבג def</span> ghi` (and `<bdi>`) | `abc` 0, `def` 32, Hebrew 60.5, `ghi` 90.8 |
//! | `<span rtl bidi-override>abc</span>` (and `<bdo>`) | `c` 0, `b` 8, `a` 16.9 |
//! | `rtl bidi-override` block, `abc` | `c` 374.2, `b` 382.2, `a` 391.1 |
//! | an empty `isolate` span, an empty `bidi-override` block | height 0 |
//! | `direction: rtl`, `123<img 30px>abc` | `abc` 315.7, image 343.3, `123` 373.3 |

use parley::layout::PositionedLayoutItem;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn lay_out(style: &str, inner: &str) -> (RinchDocument, u64) {
    let mut d = RinchDocument::new();
    let body = d.body();
    d.set_attribute(body, "style", "margin: 0");
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("font:16px/25px sans-serif;width:400px;{style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    (d, c.0)
}

/// The left edge of the cluster at the first byte of each of `marks` in the
/// paragraph's text, and of the inline box `img` if there is one.
fn xs(style: &str, inner: &str, marks: &[&str]) -> Vec<f32> {
    let (d, c) = lay_out(style, inner);
    let il = d.tree.get(c).unwrap().text_layout.as_ref().unwrap();
    let text = &il.text_content;
    assert_eq!(il.layout.lines().count(), 1, "{inner}: one line");
    let img = d.query_selector_all("img").first().map(|n| n.0);
    marks
        .iter()
        .map(|m| {
            let byte = match *m {
                "<img>" => usize::MAX,
                _ => text.find(m).unwrap_or_else(|| panic!("{m:?} in {text:?}")),
            };
            let line = il.layout.lines().next().unwrap();
            for item in line.items() {
                match item {
                    PositionedLayoutItem::InlineBox(b) if *m == "<img>" => {
                        assert_eq!(Some(b.id), img);
                        return b.x;
                    }
                    PositionedLayoutItem::GlyphRun(g) => {
                        let mut x = g.offset();
                        for cluster in g.run().visual_clusters() {
                            if cluster.text_range().contains(&byte) {
                                return x;
                            }
                            x += cluster.advance();
                        }
                    }
                    _ => {}
                }
            }
            panic!("{m:?} is not on the line of {inner}")
        })
        .collect()
}

/// `xs` for marks that must stand in this order from left to right.
fn left_to_right(style: &str, inner: &str, marks: &[&str]) {
    let x = xs(style, inner, marks);
    assert!(
        x.windows(2).all(|w| w[0] < w[1]),
        "{style} {inner}: Chrome's order, left to right, is {marks:?}; rinch's x {x:?}"
    );
}

const HEB: &str = "\u{5d0}\u{5d1}\u{5d2}";

#[test]
fn a_ltr_paragraph_starting_with_hebrew_runs_left_to_right() {
    // The issue's row: rinch put the image at 312.8, Chrome at 0.
    let img = r#"<img style="width:30px;height:10px">"#;
    let x = xs(
        "direction:ltr",
        &format!("{img}{HEB} \u{5d3}\u{5d4}\u{5d5}"),
        // The Hebrew run's last letter is its leftmost.
        &["<img>", "\u{5d5}"],
    );
    assert!(x[0].abs() < 0.5, "the image starts the line: {x:?}");
    assert!((x[1] - 30.0).abs() < 0.5, "Hebrew follows it: {x:?}");
    // Without `direction` written: the initial value is `ltr` too.
    left_to_right("", &format!("{HEB} abc"), &[HEB, "abc"]);
}

#[test]
fn a_plaintext_block_takes_its_direction_from_its_text() {
    left_to_right(
        "unicode-bidi:plaintext",
        &format!("{HEB} abc"),
        &["abc", HEB],
    );
    // Its start is the resolved direction's: right aligned, as in Chrome.
    let x = xs("unicode-bidi:plaintext", &format!("{HEB} abc"), &["abc"]);
    assert!(x[0] > 300.0, "{x:?}");
    left_to_right(
        "direction:rtl;unicode-bidi:plaintext",
        &format!("abc {HEB}"),
        &["abc", HEB],
    );
    let x = xs(
        "direction:rtl;unicode-bidi:plaintext",
        &format!("abc {HEB}"),
        &["abc"],
    );
    assert!(x[0].abs() < 0.5, "left aligned: {x:?}");
}

#[test]
fn an_isolate_embed_or_override_span_orders_its_content_in_its_direction() {
    // `normal`: the span's direction does not reach the paragraph's order.
    left_to_right(
        "",
        &format!(r#"<span style="direction:rtl">abc {HEB}</span>"#),
        &["abc", HEB],
    );
    for bidi in ["isolate", "embed", "isolate-override"] {
        left_to_right(
            "",
            &format!(r#"<span style="direction:rtl;unicode-bidi:{bidi}">abc {HEB}</span>"#),
            &[HEB, "abc"],
        );
    }
}

#[test]
fn a_plaintext_span_and_bdi_isolate_by_their_first_strong_character() {
    for open in [r#"<span style="unicode-bidi:plaintext">"#, "<bdi>"] {
        let close = if open == "<bdi>" { "</bdi>" } else { "</span>" };
        left_to_right(
            "",
            &format!("abc {open}{HEB} def{close} ghi"),
            &["abc", "def", HEB, "ghi"],
        );
    }
}

#[test]
fn an_override_reverses_its_letters() {
    for inner in [
        r#"<span style="direction:rtl;unicode-bidi:bidi-override">abc</span>"#,
        r#"<bdo style="direction:rtl">abc</bdo>"#,
    ] {
        left_to_right("", inner, &["c", "b", "a"]);
    }
    left_to_right(
        "direction:rtl;unicode-bidi:bidi-override",
        "abc",
        &["c", "b", "a"],
    );
    let x = xs("direction:rtl;unicode-bidi:bidi-override", "abc", &["c"]);
    assert!(x[0] > 360.0, "right aligned: {x:?}");
}

#[test]
fn bidi_controls_alone_make_no_line() {
    for (style, inner) in [
        ("", r#"<span style="unicode-bidi:isolate"></span>"#),
        ("direction:rtl;unicode-bidi:bidi-override", ""),
        ("direction:rtl;unicode-bidi:bidi-override", "<span></span>"),
    ] {
        let (d, c) = lay_out(style, inner);
        let h = d.tree.get(c).unwrap().layout.height;
        assert_eq!(h, 0.0, "{style} {inner}: Chrome's height is 0");
    }
}

#[test]
fn an_isolate_keeps_collapsing_white_space_across_it() {
    // `a <span isolate> b</span>`: one space between, as in Chrome.
    let (d, c) = lay_out("", r#"a <span style="unicode-bidi:isolate"> b</span>"#);
    let il = d.tree.get(c).unwrap().text_layout.as_ref().unwrap();
    let visible: String = il
        .text_content
        .chars()
        .filter(|c| !('\u{2066}'..='\u{2069}').contains(c))
        .collect();
    assert_eq!(visible, "a b");
}

#[test]
fn a_box_after_digits_in_an_rtl_paragraph_is_a_neutral_among_the_text() {
    // The second row of #2594: `direction: rtl`, `123<img>abc`. The image is
    // an object replacement character resolved with the text (UAX #9 N1/N2
    // between a number and a Latin letter: the paragraph's direction), so
    // Chrome 157 lays it out `abc` 315.7, image 343.3, `123` 373.3.
    let img = r#"<img style="width:30px;height:10px">"#;
    let x = xs(
        "direction:rtl",
        &format!("123{img}abc"),
        &["a", "<img>", "1"],
    );
    assert!(x[0] < x[1] && x[1] < x[2], "abc, image, 123: {x:?}");
    assert!(
        (x[2] - x[1] - 30.0).abs() < 0.5,
        "the image is right before 123: {x:?}"
    );
}
