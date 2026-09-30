//! A collapsible space at the edge of an inline element is kept (#1180).
//!
//! CSS Text 3 §4.1.1 collapses white space across the whole inline formatting
//! context: phase I removes a collapsible space that follows another one —
//! "even one outside the boundary of the inline containing that space" — and
//! phase II removes the spaces at a *line's* start and end. An inline
//! element's own edges remove nothing: `a<span> b</span>` is "a b".
//!
//! parley 0.11.1's `WhiteSpaceCollapse::Collapse` trims every style span's
//! start and end, and rinch pushed one parley span per inline element, so the
//! space was dropped: `a<span> b</span>` laid out "ab", and
//! `rsx! { p { "Hello" b { " world" } } }` "Helloworld".
//!
//! Every number is Chrome 153's on the same markup (bundled Inter registered
//! as `ProbeFace`, 16px/25px, `* { margin: 0 }`). Box edges are rounded to
//! whole pixels by Taffy, hence the half-pixel tolerance.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
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
    d
}

/// A 300px container at Inter 16px/25px holding `html`; returns the document
/// and the container.
fn lay_out(container_style: &str, html: &str) -> (RinchDocument, NodeId) {
    let mut d = doc();
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(
        c,
        "style",
        &format!("width:300px;font:16px/25px ProbeFace;{container_style}"),
    );
    d.append_child(body, c);
    d.set_inner_html(c, html);
    d.resolve_layout(800.0, 600.0);
    (d, c)
}

fn find(d: &RinchDocument, id: usize, want: &str) -> Option<usize> {
    let n = d.tree.get(id)?;
    if n.attributes.get("id").map(String::as_str) == Some(want) {
        return Some(id);
    }
    n.children.iter().find_map(|&c| find(d, c, want))
}

fn ib(s: &str) -> String {
    format!("<span id=\"m\" style=\"display:inline-block\">{s}</span>")
}

const IB10: &str = "<span style=\"display:inline-block;width:10px;height:10px\"></span>";

/// The laid-out text and the `(width, height)` of the shrink-to-fit
/// `inline-block` `#m`.
fn measure(container_style: &str, html: &str) -> (String, f32, f32) {
    let (d, c) = lay_out(container_style, html);
    let m = find(&d, c.0, "m").expect("#m");
    let n = d.tree.get(m).unwrap();
    let text = n
        .text_layout
        .as_ref()
        .map(|l| l.text_content.clone())
        .unwrap_or_default();
    (text, n.layout.width, n.layout.height)
}

fn assert_width(name: &str, got: f32, want: f32) {
    assert!(
        (got - want).abs() <= 0.5 + 1e-3,
        "{name}: Chrome 153 gives {want}, rinch {got}"
    );
}

/// The issue's table and its neighbours. `a` is 8.98, `b` 9.8 and a space
/// 4.5 wide in Inter 16px.
#[test]
fn a_space_at_an_inline_elements_edge_is_kept() {
    let cases: &[(&str, String, &str, f32)] = &[
        ("a|<span> b</span>", ib("a<span> b</span>"), "a b", 23.28),
        ("<span>a </span>|b", ib("<span>a </span>b"), "a b", 23.28),
        ("a<span> </span>b", ib("a<span> </span>b"), "a b", 23.28),
        (
            "display:contents a|<c> b</c> (rsx's reactive-text wrapper)",
            ib("a<span style=\"display:contents\"> b</span>"),
            "a b",
            23.28,
        ),
        (
            "display:contents <c>a </c>|b",
            ib("<span style=\"display:contents\">a </span>b"),
            "a b",
            23.28,
        ),
        (
            "atomic inline, then a span's leading space",
            ib(&format!("a{IB10}<span> b</span>")),
            "a\u{fffc} b",
            33.28,
        ),
        ("tab at a span's edge", ib("a\t<span>\tb</span>"), "a b", 23.28),
        ("newline at a span's edge", ib("a\n<span>\n b</span>"), "a b", 23.28),
        ("a b (control)", ib("a b"), "a b", 23.28),
    ];
    for (name, html, text, w) in cases {
        let (got_text, gw, gh) = measure("", html);
        assert_eq!(
            got_text.replace('\u{fffc}', "").as_str(),
            text.replace('\u{fffc}', "").as_str(),
            "{name}: laid-out text"
        );
        assert_width(name, gw, *w);
        assert_eq!(gh, 25.0, "{name}: one 25px line");
    }
}

/// Phase I across element boundaries: two collapsible spaces with only an
/// inline element's edge (or an empty inline element) between them are one.
/// The survivor is the *first* space, in its own element's style: after `a `
/// the 32px span's space is removed (33.08 = a + a 16px space + a 32px `b`),
/// while in `a<span 32px> </span> b` the 32px space survives (27.78).
#[test]
fn a_doubled_space_across_an_inline_boundary_is_one_space_in_the_first_ones_style() {
    let cases: &[(&str, String, f32)] = &[
        ("a |<span> b</span>", ib("a <span> b</span>"), 23.28),
        ("a <span> </span> b", ib("a <span> </span> b"), 23.28),
        (
            "a <span nbsp b> (an NBSP is not collapsible)",
            ib("a <span>\u{a0} b</span>"),
            32.28,
        ),
        (
            "a<span 32px> </span>b",
            ib("a<span style=\"font-size:32px\"> </span>b"),
            27.78,
        ),
        (
            "a <span 32px> b</span>",
            ib("a <span style=\"font-size:32px\"> b</span>"),
            33.08,
        ),
        (
            "a<span 32px> </span> b",
            ib("a<span style=\"font-size:32px\"> </span> b"),
            27.78,
        ),
    ];
    for (name, html, w) in cases {
        let (_, gw, _) = measure("", html);
        assert_width(name, gw, *w);
    }
}

/// Phase II: a space at the start or the end of a *line* goes — the block's
/// start and end, beside a `<br>`, and past an out-of-flow box, which is not
/// content. A space before an atomic inline is not at a line's end and stays.
#[test]
fn a_space_at_a_lines_start_or_end_is_still_removed() {
    let cases: &[(&str, String, &str, f32, f32)] = &[
        ("<span> a</span>", ib("<span> a</span>"), "a", 8.98, 25.0),
        ("<span>a </span>", ib("<span>a </span>"), "a", 8.98, 25.0),
        ("a <br> b", ib("a <br> b"), "a\nb", 9.8, 50.0),
        ("a<span> <br></span> b", ib("a<span> <br></span> b"), "a\nb", 9.8, 50.0),
        (
            "a <abs> (end)",
            ib("a <span style=\"position:absolute\"></span>"),
            "a",
            8.98,
            25.0,
        ),
        (
            "<abs> a (start)",
            ib("<span style=\"position:absolute\"></span> a"),
            "a",
            8.98,
            25.0,
        ),
        ("a <ib>", ib(&format!("a {IB10}")), "a \u{fffc}", 23.48, 25.0),
        (
            "a <ib> b",
            ib(&format!("a {IB10} b")),
            "a \u{fffc} b",
            37.78,
            25.0,
        ),
    ];
    for (name, html, text, w, h) in cases {
        let (got_text, gw, gh) = measure("", html);
        assert_eq!(
            got_text.replace('\u{fffc}', "").as_str(),
            text.replace('\u{fffc}', "").as_str(),
            "{name}: laid-out text"
        );
        assert_width(name, gw, *w);
        assert_eq!(gh, *h, "{name}: height");
    }
}

/// The x of the first glyph run on `line` of `#root`'s IFC.
fn first_glyph_x(d: &RinchDocument, root: usize, line: usize) -> f32 {
    let il = d.tree.get(root).unwrap().text_layout.as_ref().expect("IFC");
    let line = il.layout.get(line).expect("line");
    for item in line.items() {
        if let parley::PositionedLayoutItem::GlyphRun(r) = item {
            return r.offset();
        }
    }
    panic!("no glyph run");
}

/// The x of the glyph run whose text starts with `word` on line `line`.
fn word_x(d: &RinchDocument, root: usize, line: usize, word: &str) -> f32 {
    let il = d.tree.get(root).unwrap().text_layout.as_ref().expect("IFC");
    let l = il.layout.get(line).expect("line");
    for item in l.items() {
        if let parley::PositionedLayoutItem::GlyphRun(r) = item {
            let range = r.run().text_range();
            let mut x = r.offset();
            for c in r.run().visual_clusters() {
                let cr = c.text_range();
                if cr.start >= range.start && il.text_content[cr.start..].starts_with(word) {
                    return x;
                }
                x += c.advance();
            }
        }
    }
    panic!("{word:?} not on line {line}");
}

/// Aligned lines count the kept space (Chrome 153: `a` at 276.72 right-aligned
/// and 138.36 centred in 300px, as in the plain `a b` control), and a line
/// after a soft wrap or a `<br>` still starts at 0.
#[test]
fn aligned_and_wrapped_lines_see_the_kept_space() {
    for (align, want) in [("right", 276.72), ("center", 138.36)] {
        for html in ["a<span> b</span>", "<span>a </span>b", "a b"] {
            let (d, c) = lay_out(&format!("text-align:{align};line-height:20px"), html);
            let x = first_glyph_x(&d, c.0, 0);
            assert!(
                (x - want).abs() <= 0.5,
                "{align} {html}: Chrome 153 puts `a` at {want}, rinch {x}"
            );
        }
    }
    // A soft wrap at the span's leading space: `bbbb` starts line 1 at 0.
    for html in ["aaaa<span> bbbb</span>", "aaaa <span>bbbb</span>"] {
        let (d, c) = lay_out("width:40px;line-height:20px", html);
        let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
        assert_eq!(il.layout.len(), 2, "{html}: two lines");
        assert_eq!(first_glyph_x(&d, c.0, 1), 0.0, "{html}: line 1 starts at 0");
    }
    // After a `<br>` a span's leading space is at the line's start.
    let (d, c) = lay_out("line-height:20px", "a<br><span> b</span>");
    assert_eq!(first_glyph_x(&d, c.0, 1), 0.0, "a<br><span> b</span>");
    // Justified: the kept space is a justification opportunity like any.
    let plain = {
        let (d, c) = lay_out(
            "text-align:justify;width:100px;line-height:20px",
            "aaa bbb ccc dddddddddd eee",
        );
        word_x(&d, c.0, 0, "bbb")
    };
    let (d, c) = lay_out(
        "text-align:justify;width:100px;line-height:20px",
        "aaa<span> bbb</span> ccc dddddddddd eee",
    );
    assert_eq!(word_x(&d, c.0, 0, "bbb"), plain, "justify");
}

/// `white-space: nowrap` collapses like `normal`; `pre-line` and `pre-wrap`
/// keep the span's space as they always did (`pre-wrap` is the editor's mode,
/// which takes its text verbatim).
#[test]
fn other_white_space_values() {
    let (t, w, _) = measure("white-space:nowrap", &ib("a<span> b</span>"));
    assert_eq!(t, "a b", "nowrap");
    assert_width("nowrap", w, 23.28);
    let (t, w, _) = measure("white-space:pre-line", &ib("a<span> b</span>"));
    assert_eq!(t, "a b", "pre-line");
    assert_width("pre-line", w, 23.28);
    let (t, _, _) = measure("white-space:pre-wrap", &ib("a  <span> b\t</span>"));
    assert_eq!(t, "a   b    ", "pre-wrap is verbatim (a tab is four spaces)");
}

/// `rsx! { p { "Hello" b { " world" } } }`, built node by node as the macro
/// builds it.
#[test]
fn the_rsx_shape_from_the_issue() {
    let mut d = doc();
    let body = d.body();
    let p = d.create_element("p");
    d.set_attribute(p, "style", "font:16px/25px ProbeFace");
    d.append_child(body, p);
    let hello = d.create_text("Hello");
    d.append_child(p, hello);
    let b = d.create_element("b");
    d.append_child(p, b);
    let world = d.create_text(" world");
    d.append_child(b, world);
    d.resolve_layout(800.0, 600.0);
    let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "Hello world");
}

/// The flat offsets every consumer reads — the DOM↔flat caret maps, an inline
/// background's range — are offsets into the text as laid out, collapsed
/// spaces removed. They used to count the pushed text, so after `a  ` every
/// offset was one byte late: a background on `b` covered nothing.
#[test]
fn flat_offsets_are_offsets_into_the_collapsed_text() {
    use rinch_dom::text_query::{dom_cursor_to_ifc_offset, ifc_offset_to_dom_cursor};
    let (d, c) = lay_out(
        "",
        "a  <span style=\"background:red\">b</span>  c <span>\n d </span>",
    );
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.text_content, "a b c d");
    assert_eq!(il.background_spans.len(), 1);
    let bg = &il.background_spans[0];
    assert_eq!(&il.text_content[bg.start..bg.end], "b", "background on b");

    // Each text node's range covers exactly its laid-out text.
    let got: Vec<&str> = il
        .text_ranges
        .iter()
        .map(|r| &il.text_content[r.flat_start..r.flat_end])
        .collect();
    assert_eq!(got, ["a ", "b", " c ", "d"]);

    // Caret maps: in "  c " (the third node) the `c` is DOM byte 2 and flat
    // byte 4; in "\n d " the `d` is DOM byte 2 and flat byte 6.
    let third = il.text_ranges[2].node_id;
    let fourth = il.text_ranges[3].node_id;
    assert_eq!(dom_cursor_to_ifc_offset(&il.text_ranges, third, 2), Some(4));
    assert_eq!(ifc_offset_to_dom_cursor(&il.text_ranges, 4, false), Some((third, 2)));
    assert_eq!(dom_cursor_to_ifc_offset(&il.text_ranges, fourth, 2), Some(6));
    assert_eq!(ifc_offset_to_dom_cursor(&il.text_ranges, 6, false), Some((fourth, 2)));
    // After `d`: DOM byte 3, flat byte 7 (the end; the trailing space went).
    assert_eq!(dom_cursor_to_ifc_offset(&il.text_ranges, fourth, 3), Some(7));
    // A DOM offset inside a removed run maps to where the run was.
    assert_eq!(dom_cursor_to_ifc_offset(&il.text_ranges, third, 1), Some(4));
}
