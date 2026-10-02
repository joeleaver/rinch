//! #1181: U+2028 LINE SEPARATOR, U+2029 PARAGRAPH SEPARATOR, U+0085 NEXT LINE
//! and U+000C FORM FEED, against Chrome 153 under every `white-space` value.
//!
//! Chrome 153 (bundled Inter as `ProbeFace`, 16px/25px; an `x` is 8.73px):
//! - U+2028 / U+2029 are an ordinary 4.5px character (a space's advance)
//!   under every `white-space`, never collapsed (two stay two), never
//!   trimmed at a line's edge, never a forced break — and a break
//!   opportunity after them.
//! - U+0085 is zero-width under every `white-space`, and a break
//!   opportunity.
//! - U+000C is drawn (13.79px) where white space collapses (`normal`,
//!   `nowrap`, `pre-line`) and zero-width where it is preserved (`pre`,
//!   `pre-wrap`, `break-spaces`), with no break opportunity.
//!
//! parley 0.11.1 reads U+2028 / U+2029 as `Whitespace::Newline`, a forced
//! break, and shapes U+0085 and a preserved U+000C to a visible glyph.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::text_query::{dom_cursor_to_ifc_offset, ifc_offset_to_dom_cursor};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn measure(container_style: &str, html: &str) -> (f32, f32) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
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
    fn find(d: &RinchDocument, id: usize) -> Option<usize> {
        let n = d.tree.get(id)?;
        if n.attributes.get("id").map(String::as_str) == Some("m") {
            return Some(id);
        }
        n.children.iter().find_map(|&c| find(d, c))
    }
    let m = find(&d, c.0).expect("#m");
    let l = d.tree.get(m).unwrap().layout;
    (l.width, l.height)
}

fn ib(ws: &str, s: &str) -> String {
    format!("<span id=\"m\" style=\"display:inline-block;white-space:{ws}\">{s}</span>")
}

const ALL: [&str; 5] = ["normal", "nowrap", "pre", "pre-wrap", "pre-line"];

/// `(name, char, [mid, end, start, two] widths)` for a mode.
fn chrome(c: char, ws: &str) -> [f32; 4] {
    match c {
        '\u{2028}' | '\u{2029}' => [21.97, 13.23, 13.23, 26.47],
        '\u{85}' => [17.47, 8.73, 8.73, 17.47],
        '\u{c}' => match ws {
            "pre" | "pre-wrap" => [17.47, 8.73, 8.73, 17.47],
            _ => [31.25, 22.52, 22.52, 45.03],
        },
        _ => unreachable!(),
    }
}

fn check(c: char, modes: &[&str]) {
    let mut bad = Vec::new();
    for &ws in modes {
        let want = chrome(c, ws);
        for (pos, text, w) in [
            ("mid", format!("x{c}x"), want[0]),
            ("end", format!("x{c}"), want[1]),
            ("start", format!("{c}x"), want[2]),
            ("two", format!("x{c}{c}x"), want[3]),
        ] {
            let (gw, gh) = measure("", &ib(ws, &text));
            if (gw - w).abs() > 0.5 + 1e-3 || gh != 25.0 {
                bad.push(format!(
                    "U+{:04X} {ws} {pos}: Chrome 153 {w}x25, rinch {gw}x{gh}",
                    c as u32
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_line_separator_is_a_space_wide_character_on_the_line() {
    check('\u{2028}', &ALL);
}

#[test]
fn a_paragraph_separator_is_a_space_wide_character_on_the_line() {
    check('\u{2029}', &ALL);
}

#[test]
fn a_next_line_is_zero_width() {
    check('\u{85}', &ALL);
}

/// Where white space collapses Chrome draws U+000C at 13.79px and rinch at
/// parley's 10.27px (another glyph): not pinned to Chrome, only to being
/// drawn at all, so a substitution that reaches those modes fails here.
#[test]
fn a_form_feed_is_zero_width_where_preserved_and_drawn_where_it_collapses() {
    check('\u{c}', &["pre", "pre-wrap"]);
    // And takes no letter-spacing: `x\fx` at 10px is `xx`'s 37.47 in Chrome
    // 153 (a zero-width character laid out in its place would add 10px).
    for ws in ["pre", "pre-wrap"] {
        let html = format!(
            "<span id=\"m\" style=\"display:inline-block;letter-spacing:10px;white-space:{ws}\">x\u{c}x</span>"
        );
        let (gw, _) = measure("", &html);
        assert!(
            (gw - 37.47).abs() <= 0.5 + 1e-3,
            "U+000C {ws} letter-spacing: Chrome 153 37.47, rinch {gw}"
        );
    }
    for ws in ["normal", "nowrap", "pre-line"] {
        let (gw, gh) = measure("", &ib(ws, "x\u{c}x"));
        assert!(
            gw > 17.47 + 5.0 && gh == 25.0,
            "U+000C {ws}: drawn, rinch {gw}x{gh}"
        );
    }
}

/// Each is a break opportunity: `xxx?xxx` in a 30px block is two 25px lines
/// in Chrome 153 (`xxx` is 26.2px) for all three, under `normal` and
/// `pre-wrap`. U+000C is not (one line, overflowing).
#[test]
fn each_is_a_break_opportunity_and_form_feed_is_not() {
    for (c, h) in [
        ('\u{2028}', 50.0),
        ('\u{2029}', 50.0),
        ('\u{85}', 50.0),
        ('\u{c}', 25.0),
    ] {
        for ws in ["normal", "pre-wrap"] {
            let html =
                format!("<div id=\"m\" style=\"width:30px;white-space:{ws}\">xxx{c}xxx</div>");
            let (_, gh) = measure("", &html);
            assert_eq!(
                gh, h,
                "U+{:04X} {ws}: Chrome 153 {h} tall, rinch {gh}",
                c as u32
            );
        }
    }
}

/// No break before a line separator, and one after each: two in a row at
/// 30px are three lines in Chrome 153 (`xxx` and its separator overflow by
/// 0.7px, the second separator is a line of its own); and `nowrap` keeps
/// it on one line.
#[test]
fn two_separators_wrap_after_each_and_nowrap_does_not_wrap() {
    for (ws, text, h) in [
        ("normal", "xxx\u{2028}\u{2028}xxx", 75.0),
        ("pre-wrap", "xxx\u{2028}\u{2028}xxx", 75.0),
        ("nowrap", "xxx\u{2028}xxx", 25.0),
        ("pre", "xxx\u{2028}xxx", 25.0),
    ] {
        let html = format!("<div id=\"m\" style=\"width:30px;white-space:{ws}\">{text}</div>");
        let (_, gh) = measure("", &html);
        assert_eq!(gh, h, "{ws} {text:?}: Chrome 153 {h} tall, rinch {gh}");
    }
}

/// The caret maps of a text node holding them: every DOM character
/// boundary maps to a flat one and back, and no flat position — the one
/// between the two characters U+2028 is laid out as included — maps into a
/// DOM character's UTF-8 bytes.
#[test]
fn caret_offsets_map_around_them_at_character_boundaries() {
    for ws in ["normal", "pre-wrap"] {
        for (dom, flat_text, d2f, f2d) in [
            (
                "a\u{2028}b",
                "a\u{a0}\u{200b}b",
                vec![(0, 0), (1, 1), (4, 6), (5, 7)],
                vec![(0, 0), (1, 1), (3, 4), (6, 4), (7, 5)],
            ),
            (
                "a\u{85}b",
                "a\u{200b}b",
                vec![(0, 0), (1, 1), (3, 4), (4, 5)],
                vec![(0, 0), (1, 1), (4, 3), (5, 4)],
            ),
        ] {
            use parley::fontique::{Blob, FontInfoOverride};
            let mut d = RinchDocument::new();
            d.font_cx.collection.register_fonts(
                Blob::new(std::sync::Arc::new(FACE)),
                Some(FontInfoOverride {
                    family_name: Some("ProbeFace"),
                    ..Default::default()
                }),
            );
            let body = d.body();
            let p = d.create_element("div");
            d.set_attribute(
                p,
                "style",
                &format!("width:300px;font:16px/25px ProbeFace;white-space:{ws}"),
            );
            d.append_child(body, p);
            let t = d.create_text(dom);
            d.append_child(p, t);
            d.resolve_layout(800.0, 600.0);
            let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
            assert_eq!(il.text_content, flat_text, "{ws}");
            for (o, f) in d2f {
                assert_eq!(
                    dom_cursor_to_ifc_offset(&il.text_ranges, t.0, o),
                    Some(f),
                    "{ws} {dom:?}: DOM {o}"
                );
            }
            for (f, o) in f2d {
                let got = ifc_offset_to_dom_cursor(&il.text_ranges, f, false)
                    .unwrap()
                    .1;
                assert_eq!(got, o, "{ws} {dom:?}: flat {f}");
                assert!(dom.is_char_boundary(got));
            }
        }
    }
}

/// What rinch-dom tells the editor's caret map each substituted char is worth
/// (`DomDocument::substituted_char_flat_bytes`) is what its layout makes of
/// it in preserved text, and in a `contenteditable` root.
#[test]
fn the_flat_bytes_the_host_reports_are_the_ones_it_lays_out() {
    let mut d = RinchDocument::new();
    let table = d.substituted_char_flat_bytes();
    assert_eq!(table.len(), 4, "U+2028, U+2029, U+0085, U+000C");
    for &(c, n) in table {
        for attr in [
            ("style", "white-space:pre-wrap"),
            ("contenteditable", "true"),
        ] {
            let body = d.body();
            let p = d.create_element("div");
            d.set_attribute(p, attr.0, attr.1);
            d.append_child(body, p);
            let t = d.create_text(&format!("a{c}b"));
            d.append_child(p, t);
            d.resolve_layout(800.0, 600.0);
            let il = d.tree.get(p.0).unwrap().text_layout.as_ref().unwrap();
            assert_eq!(
                il.text_content.len(),
                2 + n,
                "U+{:04X} under {attr:?}: {:?}",
                c as u32,
                il.text_content
            );
        }
    }
}
