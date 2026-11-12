//! #320 / #789: a form control's text — an `<input>`'s or a `<textarea>`'s
//! value, a closed `<select>`'s label — is shaped by the one builder ordinary
//! text is, `ComputedStyle::build_parley_layout`, so it reads every property
//! that builder reads, and its caret, its paint and its measure all come from
//! that one layout.
//!
//! Chrome 155 (headless, Inter as `ProbeFace`, 20px/28px, `letter-spacing:
//! 3.5px; word-spacing: 7px; font-style: italic`, "Wave hi to AV"): the ink of
//! an `<input>`'s value, a `<textarea>`'s first line and a `<select>`'s label
//! is 191px wide, the same as a `<div>`'s with that style; the textarea's
//! lines are 28px apart, as the div's are. Before this, the input and the
//! textarea read only the font's size, family and weight (no spacing, no
//! italic, the font's normal line pitch) and the select label no italic.
//!
//! The oracle is paint itself: a recording painter collects every glyph
//! `paint_document` draws, by colour, and the ordinary `<div>` beside each
//! control is the reference for what that style shapes to.
//!
//! `text-indent` is not in the matrix: rinch implements it nowhere (no
//! `ComputedStyle` field), so no producer reads it and none can disagree.

use peniko::kurbo::{Affine, Point, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{
    BlendMode, GlyphSynthesis, PaintGlyph, PaintImage, PaintShape, Painter,
};

const VW: f32 = 700.0;
const VH: f32 = 500.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The style every element of a fixture shares. Off every fixed point: a
/// fractional letter-spacing, a word-spacing that differs from it, an italic
/// the face has no file for (so the synthesis must reach the painter), a
/// line-height that is not the face's normal pitch.
const TYPE: &str = "font-family: ProbeFace; font-size: 20px; line-height: 28px; \
                    letter-spacing: 3.5px; word-spacing: 7px; font-style: italic";
/// Kerning (`AV`, `Wa`) and three word spaces.
const TEXT: &str = "Wave hi to AV";

#[derive(Debug, Clone)]
struct Run {
    rgb: [u8; 3],
    font_size: f32,
    synthesis: GlyphSynthesis,
    /// Glyph id and position, in the painter's (physical) space.
    glyphs: Vec<(u32, Point)>,
}

#[derive(Default)]
struct Recorder {
    runs: Vec<Run>,
}

impl Painter for Recorder {
    fn reset(&mut self) {
        self.runs.clear();
    }
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        font_size: f32,
        transform: Affine,
        synthesis: GlyphSynthesis,
        brush: &Brush,
        _: bool,
        _: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        let Brush::Solid(c) = brush else {
            panic!("text is painted with a solid brush");
        };
        let [r, g, b, _] = c.to_rgba8().to_u8_array();
        self.runs.push(Run {
            rgb: [r, g, b],
            font_size,
            synthesis,
            glyphs: glyphs
                .iter()
                .map(|gl| (gl.id, transform * Point::new(gl.x as f64, gl.y as f64)))
                .collect(),
        });
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

fn new_doc() -> RinchDocument {
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

fn element(doc: &mut RinchDocument, tag: &str, style: &str) -> NodeId {
    let body = doc.body();
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(body, el);
    el
}

fn text_child(doc: &mut RinchDocument, parent: NodeId, text: &str) {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
}

/// Every glyph run painted at `scale`, grouped by colour into one glyph list
/// per element (an element's runs, in paint order).
fn painted(doc: &mut RinchDocument, scale: f64) -> Vec<Run> {
    let mut painter = Recorder::default();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.runs
}

/// The glyphs painted in colour `rgb`, in logical px, with the run's size
/// (logical) and synthesis. Panics unless exactly one run has that colour.
fn one_run(runs: &[Run], rgb: [u8; 3], scale: f64) -> Run {
    let mine: Vec<&Run> = runs.iter().filter(|r| r.rgb == rgb).collect();
    assert_eq!(
        mine.len(),
        1,
        "exactly one run in rgb{rgb:?}, got {}: every run {runs:#?}",
        mine.len()
    );
    let r = mine[0];
    Run {
        rgb,
        font_size: (r.font_size as f64 / scale) as f32,
        synthesis: r.synthesis,
        glyphs: r
            .glyphs
            .iter()
            .map(|&(id, p)| (id, Point::new(p.x / scale, p.y / scale)))
            .collect(),
    }
}

/// Each glyph's pen x less the first's: the shape of the line, wherever the
/// element puts it.
fn advances(run: &Run) -> Vec<(u32, f64)> {
    let x0 = run.glyphs[0].1.x;
    run.glyphs.iter().map(|&(id, p)| (id, p.x - x0)).collect()
}

fn assert_same_shape(what: &str, got: &Run, reference: &Run, tol: f64) {
    assert_eq!(
        got.font_size, reference.font_size,
        "{what}: painted at {} px, the reference at {}",
        got.font_size, reference.font_size
    );
    assert_eq!(
        got.synthesis, reference.synthesis,
        "{what}: painted with synthesis {:?}, the reference with {:?} \
         (an italic the face has no file for is a skew)",
        got.synthesis, reference.synthesis
    );
    let (g, r) = (advances(got), advances(reference));
    assert_eq!(
        g.iter().map(|p| p.0).collect::<Vec<_>>(),
        r.iter().map(|p| p.0).collect::<Vec<_>>(),
        "{what}: a different glyph sequence"
    );
    for (i, ((_, gx), (_, rx))) in g.iter().zip(&r).enumerate() {
        assert!(
            (gx - rx).abs() <= tol,
            "{what}: glyph {i} at +{gx:.3}px, the reference's at +{rx:.3}px \
             (all: {g:?} vs {r:?})"
        );
    }
}

/// A `<div>` (the reference, ordinary text), an `<input>`, a `<textarea>` and
/// a `<select>`, every one in [`TYPE`] and its own colour.
fn controls(extra: &str) -> (RinchDocument, [NodeId; 4]) {
    let mut doc = new_doc();
    let reference = element(
        &mut doc,
        "div",
        &format!("{TYPE}; white-space: pre; color: rgb(0, 0, 1); {extra}"),
    );
    text_child(&mut doc, reference, TEXT);
    let input = element(
        &mut doc,
        "input",
        &format!(
            "{TYPE}; display: block; width: 500px; padding: 3px 11px; \
             color: rgb(0, 1, 0); {extra}"
        ),
    );
    doc.set_attribute(input, "value", TEXT);
    let textarea = element(
        &mut doc,
        "textarea",
        &format!(
            "{TYPE}; display: block; width: 500px; padding: 5px 7px; \
             color: rgb(1, 0, 0); {extra}"
        ),
    );
    doc.set_attribute(textarea, "value", TEXT);
    let select = element(
        &mut doc,
        "select",
        &format!("{TYPE}; display: block; padding: 2px 13px; color: rgb(1, 1, 0); {extra}"),
    );
    let option = doc.create_element("option");
    doc.append_child(select, option);
    text_child(&mut doc, option, TEXT);
    doc.resolve_layout(VW, VH);
    (doc, [reference, input, textarea, select])
}

/// The fault (#320, #789): every control's text is painted in the shape the
/// same style gives ordinary text — its spacing, its word spacing and its
/// italic, as Chrome does. Before, the input and textarea dropped all three
/// (+0 per glyph where the div has +3.5, a +7 jump at each space, no skew)
/// and the select its italic.
#[test]
fn every_control_paints_its_text_as_ordinary_text_is_shaped_320() {
    let (mut doc, _) = controls("");
    let runs = painted(&mut doc, 1.0);
    let reference = one_run(&runs, [0, 0, 1], 1.0);
    assert_eq!(
        reference.glyphs.len(),
        TEXT.chars().count(),
        "positive control: one glyph a character in the reference"
    );
    assert_ne!(
        reference.synthesis,
        GlyphSynthesis::NONE,
        "positive control: the reference's italic is synthesised"
    );
    for (what, rgb) in [
        ("<input>", [0, 1, 0]),
        ("<textarea>", [1, 0, 0]),
        ("<select>", [1, 1, 0]),
    ] {
        let got = one_run(&runs, rgb, 1.0);
        assert_same_shape(what, &got, &reference, 0.01);
    }
}

/// The same at a device scale of 2: a control is shaped at the device size
/// (`scale` multiplies its spacing as it does its font size), and lands
/// where the same text at scale 1 does, to under a device pixel. Kills a
/// builder that scales the font size and not the spacing (the line comes out
/// 3.5px short a glyph).
#[test]
fn a_control_keeps_its_spacing_at_a_device_scale_320() {
    let (mut doc, _) = controls("");
    let at1 = painted(&mut doc, 1.0);
    let at2 = painted(&mut doc, 2.0);
    for (what, rgb) in [
        ("<input>", [0, 1, 0]),
        ("<textarea>", [1, 0, 0]),
        ("<select>", [1, 1, 0]),
    ] {
        let (a, b) = (one_run(&at1, rgb, 1.0), one_run(&at2, rgb, 2.0));
        assert_same_shape(&format!("{what} at scale 2"), &b, &a, 0.5);
    }
}

/// The caret of an `<input>` and a `<textarea>` (`DomDocument::
/// text_caret_rect`, the MCP `get_caret_position` path) stands at the left
/// edge of the glyph painted for the character after it, at every offset:
/// the caret and the paint are one layout. Passes at HEAD too (they were one
/// builder since #827) — this is the guard that they stay one now that the
/// builder reads more of the style.
#[test]
fn the_caret_stands_on_the_painted_glyph_at_every_offset_320() {
    let (mut doc, [_, input, textarea, _]) = controls("");
    let runs = painted(&mut doc, 1.0);
    for (what, id, rgb) in [
        ("<input>", input, [0, 1, 0]),
        ("<textarea>", textarea, [1, 0, 0]),
    ] {
        let run = one_run(&runs, rgb, 1.0);
        for (i, &(_, p)) in run.glyphs.iter().enumerate() {
            let caret = doc
                .text_caret_rect(id, Some(i))
                .unwrap_or_else(|| panic!("{what}: a caret rect at {i}"));
            assert!(
                (caret.x as f64 - p.x).abs() < 0.01,
                "{what}: the caret at {i} is at x {}, the glyph painted for {:?} at {}",
                caret.x,
                &TEXT[i..i + 1],
                p.x
            );
        }
    }
}

/// The same in a right-to-left value: the caret after character `i` stands
/// at the left edge of character `i`'s painted glyph — in a right-to-left
/// run, the boundary *before* it in logical order is its right edge, the one
/// after it its left (to under a pixel: a face may offset a glyph from its pen). The glyphs are drawn left to right, so character `i` is
/// the `i`th glyph from the right.
#[test]
fn the_caret_stands_on_the_painted_glyph_in_rtl_text_320() {
    const RTL: &str = "שלום עולם";
    let mut doc = new_doc();
    let input = element(
        &mut doc,
        "input",
        "font-size: 20px; letter-spacing: 2.5px; word-spacing: 6px; display: block; \
         width: 500px; padding: 3px 11px; color: rgb(0, 1, 0)",
    );
    doc.set_attribute(input, "value", RTL);
    doc.resolve_layout(VW, VH);
    let runs = painted(&mut doc, 1.0);
    let glyphs: Vec<Point> = {
        let mut all: Vec<Point> = runs
            .iter()
            .filter(|r| r.rgb == [0, 1, 0])
            .flat_map(|r| r.glyphs.iter().map(|g| g.1))
            .collect();
        all.sort_by(|a, b| b.x.total_cmp(&a.x));
        all
    };
    let chars: Vec<(usize, char)> = RTL.char_indices().collect();
    assert_eq!(
        glyphs.len(),
        chars.len(),
        "positive control: one glyph a character (the host's Hebrew face)"
    );
    let units = |byte: usize| RTL[..byte].encode_utf16().count();
    for (k, &(_, ch)) in chars.iter().enumerate() {
        let after = chars.get(k + 1).map_or(RTL.len(), |c| c.0);
        let caret = doc
            .text_caret_rect(input, Some(units(after)))
            .expect("a caret rect");
        assert!(
            // Under a pixel, not exactly: the host's Hebrew face may place a
            // glyph a few tenths off its pen position (measured 0.4px on
            // nico-top's DejaVu). A caret from a second layout drifts by the
            // 2.5px spacing a character.
            (caret.x as f64 - glyphs[k].x).abs() < 0.75,
            "the caret after {ch:?} (char {k}) is at x {}, its painted glyph's left edge at {}",
            caret.x,
            glyphs[k].x
        );
    }
}

/// A `<textarea>`'s lines are painted `line-height` apart, as a div's are and
/// as Chrome draws them (28px here) — and that is the pitch its `rows` were
/// sized with, so three rows hold three lines. Before, its lines were the
/// face's normal pitch apart (about 24px) inside a box sized 3 × 28.
#[test]
fn a_textarea_paints_its_lines_at_its_line_height_320() {
    let mut doc = new_doc();
    let reference = element(
        &mut doc,
        "div",
        &format!("{TYPE}; white-space: pre; color: rgb(0, 0, 1)"),
    );
    text_child(&mut doc, reference, "Wa\nhi\nAV");
    let textarea = element(
        &mut doc,
        "textarea",
        &format!("{TYPE}; display: block; width: 300px; color: rgb(1, 0, 0)"),
    );
    doc.set_attribute(textarea, "rows", "3");
    doc.set_attribute(textarea, "value", "Wa\nhi\nAV");
    doc.resolve_layout(VW, VH);
    let runs = painted(&mut doc, 1.0);
    let baselines = |rgb: [u8; 3]| {
        let mut ys: Vec<f64> = runs
            .iter()
            .filter(|r| r.rgb == rgb)
            .flat_map(|r| r.glyphs.iter().map(|g| g.1.y))
            .collect();
        ys.sort_by(f64::total_cmp);
        ys.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        ys
    };
    let (div, ta) = (baselines([0, 0, 1]), baselines([1, 0, 0]));
    assert_eq!(div.len(), 3, "positive control: three div lines {div:?}");
    assert_eq!(ta.len(), 3, "three textarea lines {ta:?}");
    for i in 1..3 {
        assert!(
            (ta[i] - ta[i - 1] - 28.0).abs() < 0.01 && (div[i] - div[i - 1] - 28.0).abs() < 0.01,
            "lines 28px apart: textarea {ta:?}, div {div:?}"
        );
    }
    let content_h = {
        let n = doc.tree.get(textarea.0).expect("the textarea");
        let s = &n.computed_style;
        n.layout.height
            - s.border_top_width.to_px()
            - s.border_bottom_width.to_px()
            - s.padding_top.to_px()
            - s.padding_bottom.to_px()
    };
    assert_eq!(content_h, 84.0, "three rows of a 28px line");
}

/// A restyle that changes only the spacing of a focused `<input>` moves its
/// scroll with it: the caret at the end of a value wider than the field stays
/// inside the field's text column. The field's cached caret and extent
/// (`displayed_metrics`) are keyed on every style input the builder reads;
/// one keyed on the font alone keeps the old, narrower extent, the scroll
/// stays where it was and the caret is painted past the field's right edge.
#[test]
fn a_spacing_restyle_keeps_a_focused_fields_caret_in_view_320() {
    const VALUE: &str = "the quick brown fox jumps over";
    let base = "font-family: ProbeFace; font-size: 20px; display: block; width: 150px; \
                padding: 0 5px; border: 0; color: rgb(0, 1, 0)";
    let mut doc = new_doc();
    let input = element(&mut doc, "input", base);
    doc.set_attribute(input, "value", VALUE);
    doc.set_attribute(input, "data-focused", "true");
    doc.set_attribute(input, "data-cursor-pos", &VALUE.len().to_string());
    doc.resolve_layout(VW, VH);
    painted(&mut doc, 1.0);
    let in_view = |doc: &mut RinchDocument, when: &str| {
        let caret = doc
            .text_caret_rect(input, Some(VALUE.len()))
            .expect("a caret rect");
        let width = doc.tree.get(input.0).expect("the input").layout.width as f64;
        let (left, _) = rinch_dom::paint::point_from_painted_box(&doc.tree, input.0, 1.0, 5.0, 0.0);
        let right = left + width - 10.0;
        assert!(
            caret.x as f64 > left && caret.x as f64 <= right + 0.01,
            "{when}: the caret at x {} is outside the text column (right edge {right})",
            caret.x
        );
    };
    in_view(&mut doc, "before");
    doc.set_attribute(
        input,
        "style",
        &format!("{base}; letter-spacing: 4.5px; word-spacing: 9px"),
    );
    doc.resolve_layout(VW, VH);
    painted(&mut doc, 1.0);
    in_view(&mut doc, "after the spacing restyle");
}
