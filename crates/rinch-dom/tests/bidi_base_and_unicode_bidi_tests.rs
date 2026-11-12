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
        r#"<bdo dir="rtl">abc</bdo>"#,
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

#[test]
fn a_wrapped_hebrew_line_in_a_ltr_block_starts_at_its_left() {
    // The space a line of Hebrew wraps after takes the paragraph's level
    // (UAX #9 L1, parley fork): it ends the line at its right and hangs,
    // rather than standing at the Hebrew's visual left and pushing it a
    // space in. Chrome 157: `אבג` at 0 in a 30px `ltr` block.
    let (d, c) = lay_out("width:30px", &format!("{HEB} \u{5d3}\u{5d4}\u{5d5}"));
    let il = d.tree.get(c).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.layout.lines().count(), 2, "it wraps");
    // The left edge of each Hebrew cluster on line one.
    let line = il.layout.lines().next().unwrap();
    let mut hebrew = Vec::new();
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(g) = item {
            let mut x = g.offset();
            for cluster in g.run().visual_clusters() {
                if il.text_content[cluster.text_range()]
                    .chars()
                    .any(|c| ('\u{5d0}'..='\u{5ea}').contains(&c))
                {
                    hebrew.push(x);
                }
                x += cluster.advance();
            }
        }
    }
    assert_eq!(hebrew.len(), 3, "line one is the first word: {hebrew:?}");
    let hebrew = [hebrew.iter().copied().fold(f32::INFINITY, f32::min)];
    assert!(hebrew[0].abs() < 0.5, "it starts the line: {hebrew:?}");
}

/// The left and right edge of the cluster holding byte `byte` of the first
/// line of `il`.
fn cluster_edges(il: &rinch_dom::node::InlineLayout, byte: usize) -> (f32, f32) {
    let line = il.layout.lines().next().unwrap();
    for item in line.items() {
        if let PositionedLayoutItem::GlyphRun(g) = item {
            let mut x = g.offset();
            for cluster in g.run().visual_clusters() {
                if cluster.text_range().contains(&byte) {
                    return (x, x + cluster.advance());
                }
                x += cluster.advance();
            }
        }
    }
    panic!("byte {byte} of {:?} is not on line one", il.text_content)
}

/// A Hebrew email: `dir="rtl"` on a wrapper `<div>`, on `<html>` or on
/// `<body>`, or `dir="auto"` on the paragraph. Chrome 157 (`16px/25px
/// sans-serif`, 400px paragraphs, `margin: 0`) lays each paragraph out right
/// to left: `שלום, hello 123!` has `ש` at 388.9–400 (rinch put it at 20.9
/// once its `ltr` base stopped being guessed from the text, and reads `dir`
/// since), `hello 123` left to right before it (after W7, `123` follows
/// `hello`: `1` at 332.4), and `!` leftmost at 289.7; `שלום עולם.` puts
/// the period left of `עולם` (329.7 against 334.1).
#[test]
fn a_hebrew_email_with_dir_rtl_runs_right_to_left() {
    use rinch_core::dom::NodeId;
    for place in ["div", "html", "body", "auto"] {
        let mut d = RinchDocument::new();
        let body = d.body();
        d.set_attribute(body, "style", "margin: 0");
        let para = r#"style="margin:0;font:16px/25px sans-serif;width:400px""#;
        let inner = format!("<p {para}>שלום, hello 123!</p><p {para}>שלום עולם.</p>");
        let wrap = d.create_element("div");
        match place {
            "div" => d.set_attribute(wrap, "dir", "rtl"),
            "html" => d.set_attribute(NodeId(d.tree.html_id), "dir", "rtl"),
            "body" => d.set_attribute(body, "dir", "rtl"),
            _ => {}
        }
        let inner = if place == "auto" {
            inner.replace("<p ", r#"<p dir="auto" "#)
        } else {
            inner
        };
        d.set_inner_html(wrap, &inner);
        d.append_child(body, wrap);
        d.resolve_layout(800.0, 600.0);
        let ps = d.query_selector_all("p");
        assert_eq!(ps.len(), 2);
        let il = |n: usize| d.tree.get(ps[n].0).unwrap().text_layout.as_ref().unwrap();
        let first = il(0);
        let t = &first.text_content;
        let shin = cluster_edges(first, 0);
        let h = cluster_edges(first, t.find('h').unwrap());
        let one = cluster_edges(first, t.find('1').unwrap());
        let bang = cluster_edges(first, t.find('!').unwrap());
        assert!(
            (shin.1 - 400.0).abs() < 0.5,
            "{place}: ש ends the line at the right: {shin:?}"
        );
        assert!(
            bang.0 < h.0 && h.0 < one.0 && one.0 < shin.0,
            "{place}: Chrome's order is !, hello, 123, ש: {bang:?} {h:?} {one:?} {shin:?}"
        );
        let second = il(1);
        let t = &second.text_content;
        let period = cluster_edges(second, t.find('.').unwrap());
        let mem = cluster_edges(second, t.find('\u{5dd}').unwrap());
        let mem = if t.matches('\u{5dd}').count() > 1 {
            cluster_edges(second, t.rfind('\u{5dd}').unwrap())
        } else {
            mem
        };
        assert!(
            period.1 <= mem.0 + 0.5,
            "{place}: the period is left of עולם: {period:?} {mem:?}"
        );
    }
}

#[test]
fn dir_on_a_span_isolates_it_and_bdo_dir_overrides() {
    // `a <span dir=rtl>abc אבג</span>`: as `direction: rtl; unicode-bidi:
    // isolate` (Chrome 157: the Hebrew left of `abc`).
    left_to_right(
        "",
        &format!(r#"<span dir="rtl">abc {HEB}</span>"#),
        &[HEB, "abc"],
    );
    // `<bdo dir=rtl>abc</bdo>`: `c`, `b`, `a` (Chrome: 0, 8, 16.9).
    left_to_right("", r#"<bdo dir="rtl">abc</bdo>"#, &["c", "b", "a"]);
    // A `<bdi>` with an explicit `dir=rtl` isolates in that direction, not
    // its content's: `abc def` stays one left-to-right run inside it.
    left_to_right(
        "",
        &format!(r#"x <bdi dir="ltr">{HEB} def</bdi>"#),
        &[HEB, "def"],
    );
}

#[test]
fn an_embed_differs_from_an_isolate_for_a_number_after_it() {
    // `x <span rtl embed>אבג</span>1`: the embedding leaves the `1` a
    // European number after R (W2/W7 see the embedding's level as `sos`), so
    // it runs right to left with the Hebrew, at its left; an isolate is a
    // neutral to the paragraph, and the `1` follows it. Chrome 157: embed
    // `1` 12, Hebrew 21; isolate Hebrew 12, `1` 37. (A neutral such as `!`
    // after the span lands in the same place for both.)
    let span =
        |bidi: &str| format!(r#"x <span style="direction:rtl;unicode-bidi:{bidi}">{HEB}</span>1"#);
    left_to_right("", &span("embed"), &["1", HEB]);
    left_to_right("", &span("isolate"), &[HEB, "1"]);
}

#[test]
fn a_flat_offset_in_an_isolates_controls_maps_to_the_text_beside_them() {
    // `abc <span isolate>xyz</span> def`: `abc ` 0..4, LRI 4..7, `xyz`
    // 7..10, PDI 10..13, ` def` 13..17. An offset in a control's bytes, or
    // at the gap's start, is the end of the text before it, not the
    // paragraph's end; so is one in a `<wbr>`'s.
    let (d, c) = lay_out(
        "",
        r#"abc <span style="unicode-bidi:isolate">xyz</span> def<wbr>gh"#,
    );
    let il = d.tree.get(c).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "abc \u{2066}xyz\u{2069} def\u{200b}gh");
    let texts: Vec<u64> = il.text_ranges.iter().map(|r| r.node_id).collect();
    let at = |flat| rinch_dom::text_query::ifc_offset_to_dom_cursor(&il.text_ranges, flat, false);
    for flat in 4..7 {
        assert_eq!(at(flat), Some((texts[0], 4)), "flat {flat}");
    }
    for flat in 10..13 {
        assert_eq!(at(flat), Some((texts[1], 3)), "flat {flat}");
    }
    assert_eq!(at(7), Some((texts[1], 0)));
    assert_eq!(at(13), Some((texts[2], 0)));
    for flat in 17..20 {
        assert_eq!(at(flat), Some((texts[2], 4)), "flat {flat}");
    }
    assert_eq!(at(20), Some((texts[3], 0)));
    assert_eq!(at(22), Some((texts[3], 2)));
}
