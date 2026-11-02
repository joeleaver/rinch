//! An inline element's horizontal margin, border and padding take room on
//! its line (issue #1426): the inline-start ones before its first fragment's
//! content, the inline-end ones after its last (`box-decoration-break:
//! slice`), and a line does not break between them and that content.
//!
//! Every number is Chrome 155's (`--headless=new`, standards mode,
//! `* { box-sizing: border-box }`), measured with `getBoundingClientRect`
//! relative to the block container's border box, with this crate's bundled
//! Inter loaded through `@font-face` under the name the fixtures register it
//! with. The scaffold:
//!
//! ```html
//! <div C="position: relative; width: 400px; font: 16px/20px ProbeFace;
//!         margin: 13px 0 0 17px; padding: 7px 0 0 11px">…</div>
//! ```
//!
//! Positions are read off zero-size `inline-block` markers (`<i M>`), which
//! stand where the content after them starts; a marker's `y` names its line
//! (23, 43, 63 for lines one to three). Inter at 16px has a rounded ascent +
//! descent of 20px, the declared line-height, so the vertical numbers are
//! declarations.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);
const C: &str = "position: relative; width: 400px; font: 16px/20px ProbeFace; \
                 margin: 13px 0 0 17px; padding: 7px 0 0 11px;";
const M: &str = "display: inline-block; width: 0; height: 0";

struct Case {
    doc: RinchDocument,
    c: u64,
}

fn document() -> RinchDocument {
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

impl Case {
    /// `inner` in the container `C`, with `c_extra` appended to its style.
    /// `<i M=x>` in `inner` is a marker named `x`.
    fn with(c_extra: &str, inner: &str) -> Self {
        let inner = inner.replace("<i M=", &format!(r#"<i style="{M}" data-m="#));
        let mut doc = document();
        let body = doc.body();
        let wrap = doc.create_element("div");
        doc.set_inner_html(
            wrap,
            &format!(r#"<div data-m="cnt" style="{C}{c_extra}">{inner}</div>"#),
        );
        doc.append_child(body, wrap);
        doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
        let c = one(&doc, "[data-m=cnt]");
        Case { doc, c }
    }

    fn new(inner: &str) -> Self {
        Self::with("", inner)
    }

    /// `m`'s on-screen box relative to the container's border box.
    fn rect(&self, m: &str) -> [f32; 4] {
        let id = one(&self.doc, &format!("[data-m={m}]"));
        let (x, y) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, id, 1.0);
        let (cx, cy) = rinch_dom::paint::compute_absolute_position(&self.doc.tree, self.c, 1.0);
        let l = self.doc.tree.get(id).unwrap().layout;
        [(x - cx) as f32, (y - cy) as f32, l.width, l.height]
    }

    /// Marker `m` is at Chrome's `(x, y)`: `x` to within the half pixel a
    /// laid-out box is rounded by, `y` to within its line.
    #[track_caller]
    fn at(&self, m: &str, x: f32, y: f32) {
        let [gx, gy, ..] = self.rect(m);
        assert!(
            (gx - x).abs() <= 0.55 && (gy - y).abs() <= 1.5,
            "marker {m}: got ({gx}, {gy}), Chrome 155 gives ({x}, {y})"
        );
    }
}

fn one(doc: &RinchDocument, selector: &str) -> u64 {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

/// The issue's case: `padding: 3px 5px 4px 6px` moves what follows by 11px.
/// Before: 75.969, the padding taking no room.
#[test]
fn padding_moves_what_follows_the_span() {
    let c = Case::new(r#"lead <span style="padding: 3px 5px 4px 6px">text</span><i M=a></i>tail"#);
    c.at("a", 86.969, 23.0);
}

/// Margin and border take room too: `3 + 2 + 5` before, `5 + 2 + 4` after.
#[test]
fn margin_and_border_take_room_too() {
    let c = Case::new(
        r#"lead <span style="margin: 0 4px 0 3px; border: 2px solid; padding: 0 5px">text</span><i M=a></i>tail"#,
    );
    c.at("a", 96.969, 23.0);
}

/// Pimble's pill (the issue's comment): a span that starts the line starts
/// its content after its padding, so its background is inside the box.
/// Before: the content at 11, and the pill drawn from 4.
#[test]
fn a_span_at_the_line_start_starts_its_content_after_its_padding() {
    let c = Case::new(
        r#"<span style="padding: 0 7px; background: #228be6"><i M=a></i>Pay<i M=b></i></span><i M=c></i>"#,
    );
    c.at("a", 18.0, 23.0);
    c.at("b", 45.969, 23.0);
    c.at("c", 52.969, 23.0);
}

/// The same in a `nowrap` container that clips with `text-overflow:
/// ellipsis` (the app's own markup): the line fits, so nothing is cut.
#[test]
fn a_pill_in_an_ellipsis_box_keeps_its_padding() {
    let c = Case::with(
        "width: 60px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        r#"<span style="padding: 0 7px">Pay</span><i M=a></i>"#,
    );
    c.at("a", 52.969, 23.0);
}

/// A span that wraps: the end padding is on its last line only, after `ddd`.
#[test]
fn a_wrapped_span_ends_its_last_line_with_its_end_padding() {
    let c = Case::with(
        "width: 120px;",
        r#"aaa <span style="padding: 0 10px">bbb ccc ddd<i M=a></i></span><i M=b></i>"#,
    );
    c.at("a", 40.391, 43.0);
    c.at("b", 50.391, 43.0);
}

/// A span that wraps to the next line at the space before it takes its
/// start padding with it: the content of the second line is `9 + bbbb + 9`.
#[test]
fn a_span_wrapped_at_the_space_before_it_takes_its_start_padding_along() {
    let c = Case::with(
        "width: 90px;",
        r#"aaaa <span style="padding: 0 9px">bbbb</span><i M=a></i>"#,
    );
    c.at("a", 68.188, 43.0);
}

/// No break between a word and a padded span inside it: `xxxx`, the span
/// and `zzzz` are one word and go to the second line together (the marker
/// after them, an opportunity of its own, to the third, with `ww` so that
/// line has text: a line of only atomic inlines has no strut, #624).
#[test]
fn a_padded_span_inside_a_word_is_no_break_opportunity() {
    let c = Case::with(
        "width: 110px;",
        r#"aa xxxx<span style="padding: 0 6px">yyyy</span>zzzz<i M=b></i>ww"#,
    );
    c.at("b", 11.0, 63.0);
}

/// An empty span still takes its padding.
#[test]
fn an_empty_span_takes_its_padding() {
    let c = Case::new(r#"a<span style="padding: 0 5px"></span><i M=a></i>b"#);
    c.at("a", 29.984, 23.0);
}

/// Right to left: `padding: 0 3px 0 9px` — the right (start) padding is 3px,
/// the left (end) one 9px. Chrome 155: `s` 375.141, `a` 372.141, `b` 353.438,
/// `c` 344.438. The Hebrew falls back to a host font, whose advances differ
/// from Chrome's, so only the paddings — the gaps around the content — are
/// asserted.
#[test]
fn right_to_left_takes_the_right_padding_first() {
    let c = Case::with(
        "direction: rtl;",
        "אבג <i M=s></i><span style=\"padding: 0 3px 0 9px\"><i M=a></i>דהו<i M=b></i></span><i M=c></i> זח",
    );
    let x = |m: &str| c.rect(m)[0];
    assert!(
        (x("s") - x("a") - 3.0).abs() <= 0.55,
        "start padding: {} {}",
        x("s"),
        x("a")
    );
    assert!(
        (x("b") - x("c") - 9.0).abs() <= 0.55,
        "end padding: {} {}",
        x("b"),
        x("c")
    );
    assert!(x("a") > x("b"), "the content runs right to left");
}

/// A positioned span's padding box is the containing block of an absolute
/// box in it (#631), and now starts where Chrome's does: the padding is room.
#[test]
fn a_positioned_spans_padding_box_starts_at_its_padding() {
    let c = Case::new(
        r#"lead <span style="position: relative; padding: 3px 5px 4px 6px">text<div data-m="abs" style="position: absolute; inset: 0"></div>tail</span>"#,
    );
    let [x, y, w, h] = c.rect("abs");
    assert!(
        (x - 47.641).abs() <= 0.55
            && (y - 4.0).abs() <= 0.55
            && (w - 61.375).abs() <= 0.55
            && (h - 27.0).abs() <= 0.55,
        "got {:?}, Chrome 155 gives [47.641, 4, 61.375, 27]",
        [x, y, w, h]
    );
}

// ── Paint ───────────────────────────────────────────────────────────────────

/// The columns, relative to the container's border box, of the fully opaque
/// pixels of exactly `rgb` in the container's rows `y0..y1`.
fn columns(c: &Case, rgb: (u8, u8, u8), y0: f32, y1: f32) -> Vec<i32> {
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    let (w, h) = (VIEWPORT.0 as u32, VIEWPORT.1 as u32);
    let mut painter = TinySkiaPainter::new(w, h);
    let mut layout_cx = parley::LayoutContext::new();
    let mut font_cx = c.doc.font_cx.clone();
    rinch_dom::paint::paint_document(
        &c.doc.tree,
        &mut painter,
        1.0,
        VIEWPORT,
        &mut font_cx,
        &mut layout_cx,
    );
    let px = painter.pixels();
    let (cx, cy) = rinch_dom::paint::compute_absolute_position(&c.doc.tree, c.c, 1.0);
    let mut out = Vec::new();
    for y in (cy as f32 + y0) as u32..(cy as f32 + y1) as u32 {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if px[i + 3] == 255 && (px[i], px[i + 1], px[i + 2]) == rgb {
                out.push(x as i32 - cx as i32);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Pimble's pill: its background is drawn from the content box's left edge
/// (Chrome's fragment: `11 .. 52.969`), not 7px left of it, where the
/// container's `overflow: hidden` cut its cap off.
#[test]
fn a_pill_at_the_line_start_is_painted_inside_the_box() {
    let c = Case::with(
        "width: 100px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
        r#"<span style="padding: 0 7px; background: rgb(0, 0, 255); color: rgb(0, 0, 255)">Pay</span>"#,
    );
    let blue = columns(&c, (0, 0, 255), 8.0, 26.0);
    let (lo, hi) = (blue[0], *blue.last().unwrap());
    assert!(
        (10..=11).contains(&lo) && (52..=53).contains(&hi),
        "the pill spans {lo}..={hi}; Chrome 155 paints 11..53"
    );
}

/// An inline border is drawn, on the sides its fragment closes: its left
/// edge where the span starts (`lead ` ends at 47.641) and its right edge
/// `2 + 5 + text + 5 + 2` = 42.328 later, at 89.969 (Chrome's border box with
/// `margin: 0 4px 0 3px` is `50.641, 5, 42.328 x 24`; here without them).
#[test]
fn an_inline_border_is_drawn() {
    let c = Case::new(
        r#"lead <span style="border: 2px solid rgb(0, 200, 0); padding: 0 5px">text</span>tail"#,
    );
    let green = columns(&c, (0, 200, 0), 5.0, 31.0);
    assert!(!green.is_empty(), "no border drawn");
    let (lo, hi) = (green[0], *green.last().unwrap());
    assert!(
        (47..=48).contains(&lo) && (88..=89).contains(&hi),
        "the border spans {lo}..={hi}"
    );
}

/// A restyle that gives an inline element its margin re-measures the line:
/// the marker after it moves as in a fresh layout (`padding_moves_what_
/// follows_the_span`'s 86.969), with no other change to the document.
#[test]
fn a_padding_restyle_moves_what_follows() {
    let mut c = Case::new(r#"lead <span data-m="s">text</span><i M=a></i>tail"#);
    c.at("a", 75.969, 23.0);
    let s = one(&c.doc, "[data-m=s]");
    c.doc
        .set_attribute(rinch_core::dom::NodeId(s), "style", "margin: 0 5px 0 6px");
    // Another viewport height, so the pass is not skipped while no width in
    // the fixture moves.
    c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1 + 40.0);
    c.at("a", 86.969, 23.0);
}

/// A span whose content ends in a forced break keeps its end padding on the
/// line the break ends: what follows the span starts the next line at the
/// content edge, 11 (review of #1823: it started 4px in). A `<br>`, a
/// preserved newline, and a `<br>` inside an element inside the span.
#[test]
fn end_padding_stays_before_a_trailing_forced_break() {
    for (white_space, inner) in [
        ("", "a<br>"),
        ("white-space: pre;", "a\n"),
        ("", "<b>a<br></b>"),
    ] {
        let c = Case::with(
            white_space,
            &format!(r#"x <span style="padding: 0 4px">{inner}</span><i M=a></i>t"#),
        );
        c.at("a", 11.0, 43.0);
    }
}

/// Timing probe (review of #1823): a paragraph of 1600 words, each in a
/// padded span, laid out at a wrapping width, against the same words
/// unpadded. `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn timing_every_word_in_a_padded_span() {
    for (name, html) in [
        ("unpadded", r#"<span>w{i}</span> "#),
        (
            "padded, space outside",
            r#"<span style="padding: 0 3px">w{i}</span> "#,
        ),
        (
            "padded, space inside",
            r#"<span style="padding: 0 3px">w{i} </span>"#,
        ),
    ] {
        let inner: String = (0..1600)
            .map(|i| html.replace("{i}", &i.to_string()))
            .collect();
        let mut c = Case::new(&inner);
        let cnt = rinch_core::dom::NodeId(c.c);
        let mut best = f64::MAX;
        for k in 0..6 {
            c.doc
                .set_attribute(cnt, "style", &format!("{C} width: {}px", 300 + k * 7));
            let t = std::time::Instant::now();
            c.doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        eprintln!("{name}: {best:.2} ms (best of 6 relayouts at a new width)");
    }
}

/// A right-to-left selection across a padded span is one highlight: the
/// padding inside the selected text is neither a gap nor a piece of its own
/// (review of #1823: `bidi_line_pieces` closed the piece at it).
#[test]
fn a_right_to_left_selection_across_a_padded_span_is_one_piece() {
    let c = Case::with(
        "direction: rtl;",
        "אבג <span style=\"padding: 0 6px\">דהו</span> זח",
    );
    let n = c.doc.tree.get(c.c).unwrap();
    let il = n.text_layout.as_ref().unwrap();
    let len = il.text_content.len();
    // As the editor asks: naming the selected boxes (none).
    let rects = rinch_dom::text_query::selection_rects_with_boxes(&il.layout, 0, len, &[]);
    assert_eq!(rects.len(), 1, "one piece: {rects:?}");
}
