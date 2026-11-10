//! `ex`, `ch` and `cap` resolve against the element's first available font
//! (#2422), not Stylo's `0.5em` / `0.5em` / `0.7em` fallbacks.
//!
//! Expected values are Chrome 157.0.8103.50's (Linux, headless), read with
//! `getComputedStyle(el).marginLeft` (and `getBoundingClientRect` for the
//! widths and the nested blockquotes) with the bundled `Inter-Regular.ttf`
//! loaded through `@font-face` under its own family name, `body { margin: 0 }`:
//!
//! | `font-family` | size | `10ex` | `10ch` | `10cap` | `10ic` | `0.8ex` |
//! |---|---|---|---|---|---|---|
//! | Inter | 16px | 87.3438 | 100.938 | 116.406 | 160 | 6.9875 |
//! | Inter | 13px | 70.9668 | 82.0117 | 94.5801 | 130 | 5.67734 |
//! | Inter, OS/2 cut to version 1 | 16px | 90 | 100.938 | 120 | 160 | 7.2 |
//! | Inter, OS/2 cut to version 1 | 13px | 70 | 82.0117 | 90 | 130 | 5.6 |
//! | `monospace` (DejaVu Sans Mono) | 16px | 90 | 96.3281 | 120 | 160 | 7.2 |
//! | `monospace` (DejaVu Sans Mono) | 13px | 70 | 78.2666 | 90 | 130 | 5.6 |
//!
//! Inter's OS/2 table has `sxHeight` 1118 and `sCapHeight` 1490 (2048 units
//! per em) and its `0` advances 1292: Chrome's numbers are those, unrounded.
//! A font whose OS/2 table predates version 2 has neither field; Chrome (Skia
//! over FreeType) then measures the top of the hinted `x` and `H` glyphs,
//! which lands on a whole pixel: 8.73px becomes 9px at 16px, 7.10px becomes
//! 7px at 13px. DejaVu Sans Mono is such a font (its OS/2 table is version 1).
//! No font here has U+6C34, so `ic` is the spec's 1em fallback throughout.
//!
//! rinch before #2422 answered every one of these from Stylo's fallbacks:
//! `10ex` = `10ch` = 80px at 16px and 65px at 13px, and `10cap` 0 (the
//! fallback is the ascent, which the old provider left at 0).
//!
//! Every text-free fixture: only box geometry is read, from font metrics, so
//! no line box or glyph advance of the host's fonts is involved.

use parley::fontique::{Blob, FontInfoOverride};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{DimensionValue, LengthPercentageAutoValue, LengthPercentageValue};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `FACE` with its OS/2 table cut to version 1 (86 bytes, no `sxHeight` or
/// `sCapHeight`), the way old fonts such as DejaVu ship it. The same bytes
/// Chrome measured above.
fn face_without_x_height() -> Vec<u8> {
    let mut b = FACE.to_vec();
    let be16 = |b: &[u8], at: usize| u16::from_be_bytes([b[at], b[at + 1]]) as usize;
    let be32 = |b: &[u8], at: usize| u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
    let tables = be16(&b, 4);
    let mut patched = false;
    for i in 0..tables {
        let rec = 12 + 16 * i;
        if &b[rec..rec + 4] == b"OS/2" {
            let off = be32(&b, rec + 8);
            assert!(be16(&b, off) >= 2, "the bundled Inter has an OS/2 x-height");
            b[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
            b[rec + 12..rec + 16].copy_from_slice(&86u32.to_be_bytes());
            patched = true;
        }
    }
    assert!(patched, "no OS/2 table in the bundled Inter");
    b
}

fn doc_with_faces() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(face_without_x_height())),
        Some(FontInfoOverride {
            family_name: Some("InterNoXH"),
            ..Default::default()
        }),
    );
    doc
}

/// Lay `html` out at 800px; the computed `margin-left` of each `[data-m]`.
///
/// The computed value, not the box's x: rinch rounds layout to whole pixels
/// (Taffy's rounding), Chrome reports fractions. The geometry is checked as
/// well, to the pixel ([`x_of`]).
fn xs(doc: &mut RinchDocument, html: &str, marks: &[&str]) -> Vec<f32> {
    load(doc, html);
    marks
        .iter()
        .map(|m| {
            let got = margin_left(doc, find(doc, m));
            let x = x_of(doc, m);
            assert!(
                (x - got).abs() <= 0.5,
                "[data-m={m}] is at x {x}, margin {got}"
            );
            got
        })
        .collect()
}

fn load(doc: &mut RinchDocument, html: &str) {
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0;");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(800.0, 600.0);
}

fn find(doc: &RinchDocument, m: &str) -> u64 {
    let found = rinch_dom::testing::query_selector(&doc.tree, &format!("[data-m={m}]"));
    assert_eq!(found.len(), 1, "[data-m={m}]");
    found[0]
}

fn margin_left(doc: &RinchDocument, id: u64) -> f32 {
    match doc.tree.get(id).unwrap().computed_style.margin_left {
        LengthPercentageAutoValue::Length(px) => px,
        other => panic!("margin-left computed to {other:?}"),
    }
}

fn x_of(doc: &RinchDocument, m: &str) -> f32 {
    rinch_dom::paint::compute_absolute_position(&doc.tree, find(doc, m), 1.0).0 as f32
}

fn width_of(doc: &RinchDocument, m: &str) -> f32 {
    match doc.tree.get(find(doc, m)).unwrap().computed_style.width {
        DimensionValue::Length(px) => px,
        other => panic!("width computed to {other:?}"),
    }
}

/// One 1px-high block per margin, in `family` at `size`.
fn margins(family: &str, size: u32, values: &[&str]) -> Vec<f32> {
    let mut html = String::new();
    let marks: Vec<String> = (0..values.len()).map(|i| format!("m{i}")).collect();
    for (value, m) in values.iter().zip(&marks) {
        html.push_str(&format!(
            "<div data-m=\"{m}\" style=\"font-family:{family};font-size:{size}px;\
             line-height:1px;height:1px;margin-left:{value}\"></div>"
        ));
    }
    let mut doc = doc_with_faces();
    let marks: Vec<&str> = marks.iter().map(String::as_str).collect();
    xs(&mut doc, &html, &marks)
}

#[track_caller]
fn close(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g - w).abs() < 0.01,
            "{what}: got {got:?}, Chrome 157 {want:?}"
        );
    }
}

const UNITS: [&str; 5] = ["10ex", "10ch", "10cap", "10ic", "0.8ex"];

#[test]
fn inter_16px_reads_the_os2_x_height_cap_height_and_zero_advance() {
    close(
        &margins("InterT", 16, &UNITS),
        &[87.3438, 100.938, 116.406, 160.0, 6.9875],
        "Inter 16px",
    );
}

#[test]
fn inter_13px_scales_the_same_metrics() {
    close(
        &margins("InterT", 13, &UNITS),
        &[70.9668, 82.0117, 94.5801, 130.0, 5.67734],
        "Inter 13px",
    );
}

/// No OS/2 `sxHeight`/`sCapHeight`: the `x` and `H` glyphs' tops, on whole
/// pixels as Chrome's hinted outlines put them. `ch` is unaffected.
#[test]
fn a_font_without_os2_x_height_measures_the_x_and_h_glyphs() {
    close(
        &margins("InterNoXH", 16, &UNITS),
        &[90.0, 100.938, 120.0, 160.0, 7.2],
        "Inter without OS/2 x-height, 16px",
    );
    close(
        &margins("InterNoXH", 13, &UNITS),
        &[70.0, 82.0117, 90.0, 130.0, 5.6],
        "Inter without OS/2 x-height, 13px",
    );
}

/// A generic family resolves to its first face, as text does: DejaVu Sans
/// Mono on CI's and this host's fontconfig.
#[test]
fn the_monospace_generic_uses_its_first_face() {
    let mut cx = rinch_dom::fonts::new_font_context();
    let first = cx
        .collection
        .generic_families(parley::fontique::GenericFamily::Monospace)
        .next();
    let first = first.and_then(|id| cx.collection.family_name(id).map(str::to_owned));
    assert_eq!(
        first.as_deref(),
        Some("DejaVu Sans Mono"),
        "these numbers are Chrome's for DejaVu Sans Mono; this host's monospace is another face"
    );
    close(
        &margins("monospace", 16, &UNITS),
        &[90.0, 96.3281, 120.0, 160.0, 7.2],
        "monospace 16px",
    );
    close(
        &margins("monospace", 13, &UNITS),
        &[70.0, 78.2666, 90.0, 130.0, 5.6],
        "monospace 13px",
    );
}

/// The face matters, not the weight: a bold that Inter Regular synthesizes
/// has Regular's metrics (Chrome: 87.3438).
#[test]
fn a_synthesized_bold_keeps_the_regular_faces_metrics() {
    let mut doc = doc_with_faces();
    let x = xs(
        &mut doc,
        "<div data-m=\"b\" style=\"font:bold 16px/1px InterT;height:1px;margin-left:10ex\"></div>",
        &["b"],
    );
    close(&x, &[87.3438], "bold Inter 16px");
}

/// `width: 10ch` (Chrome 100.9375) and `calc(10ex + 3ch)` (Chrome 117.625).
#[test]
fn ch_sizes_a_box_and_mixes_in_calc() {
    let mut doc = doc_with_faces();
    let x = xs(
        &mut doc,
        "<div style=\"font:16px/1px InterT\">\
         <div data-m=\"w\" style=\"width:10ch;height:1px\"></div>\
         <div data-m=\"c\" style=\"margin-left:calc(10ex + 3ch);height:1px\"></div></div>",
        &["c"],
    );
    close(&x, &[117.625], "calc(10ex + 3ch)");
    close(&[width_of(&doc, "w")], &[100.9375], "width: 10ch");
}

/// `font-size: 2ex` reads the *parent's* font (Chrome: 17.4688px, so the
/// child's `margin-left: 1em` is 17.4688).
#[test]
fn font_size_in_ex_reads_the_parents_font() {
    let mut doc = doc_with_faces();
    let x = xs(
        &mut doc,
        "<div style=\"font:16px/1px InterT\">\
         <div data-m=\"c\" style=\"font-size:2ex;height:1px;margin-left:1em\"></div></div>",
        &["c"],
    );
    close(&x, &[17.4688], "font-size: 2ex");
    let size = doc
        .tree
        .get(find(&doc, "c"))
        .unwrap()
        .computed_style
        .font_size;
    close(&[size], &[17.4688], "font-size: 2ex");
}

/// A `::before` box cascades through its own path (Chrome: 70.9668 at 13px).
#[test]
fn a_pseudo_elements_ex_uses_its_font() {
    let mut doc = doc_with_faces();
    doc.load_css(".ps::before { content: \"\"; display: block; height: 1px; margin-left: 10ex; }");
    let _ = xs(
        &mut doc,
        "<div data-m=\"p\" class=\"ps\" style=\"font:13px/1px InterT\"></div>",
        &["p"],
    );
    let before = doc.tree.get(find(&doc, "p")).unwrap().children[0];
    let x = margin_left(&doc, before);
    close(&[x], &[70.9668], "::before 10ex at 13px");
}

/// Gmail's quoted reply, nested ten deep, at 13px Inter: the innermost
/// content starts at 137.65625 in Chrome (each level's `0.8ex` and `1ex`
/// snapped to Blink's 1/64px layout unit; unsnapped it is 137.74). rinch
/// before #2422: 130.
///
/// The sum of the computed insets is checked exactly (137.74). The box's x is
/// 140: rinch rounds each level's offset to a whole pixel (#2437, which has
/// nothing to do with `ex`: ten levels of `padding-left: 13.774px` land at 140
/// too). When #2437 is fixed this is 138 and the bound tightens to half a
/// pixel.
#[test]
fn gmail_quote_nesting_insets_like_chrome() {
    let mut html = String::from("<div style=\"font:13px/16px InterT;width:600px\">");
    for i in 0..10 {
        html.push_str(&format!(
            "<blockquote data-m=\"q{i}\" \
             style=\"margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex\">"
        ));
    }
    html.push_str("<div data-m=\"inner\" style=\"height:1px\"></div>");
    for _ in 0..10 {
        html.push_str("</blockquote>");
    }
    html.push_str("</div>");
    let mut doc = doc_with_faces();
    load(&mut doc, &html);
    let mut inset = 0.0;
    for i in 0..10 {
        let q = doc.tree.get(find(&doc, &format!("q{i}"))).unwrap();
        let LengthPercentageValue::Length(padding) = q.computed_style.padding_left else {
            panic!(
                "padding-left computed to {:?}",
                q.computed_style.padding_left
            );
        };
        inset += margin_left(&doc, find(&doc, &format!("q{i}"))) + 1.0 + padding;
    }
    close(&[inset], &[137.74], "ten levels of 0.8ex + 1px + 1ex");
    let x = x_of(&doc, "inner");
    assert!(
        (x - 137.65625).abs() <= 2.5,
        "innermost quote content at {x}, Chrome 137.65625 (rinch 140 until #2437)"
    );
}

/// A family registered after a style resolved without it: the metrics follow
/// the new face once `note_fonts_registered` reports it (the cache is
/// dropped and the `ex` users are re-cascaded).
#[test]
fn a_late_registered_face_replaces_the_cached_metrics() {
    let mut doc = RinchDocument::new();
    let before = xs(
        &mut doc,
        "<div data-m=\"l\" style=\"font:16px/1px LateFace, monospace;height:1px;margin-left:10ex\"></div>",
        &["l"],
    )[0];
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("LateFace"),
            ..Default::default()
        }),
    );
    doc.note_fonts_registered();
    doc.resolve_layout(800.0, 600.0);
    let after = margin_left(&doc, find(&doc, "l"));
    assert!(
        (before - after).abs() > 1.0,
        "the first resolve must not already be Inter's ({before})"
    );
    close(&[after], &[87.3438], "after registering LateFace");
}
