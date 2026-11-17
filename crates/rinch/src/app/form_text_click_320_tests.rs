//! #320: a press in a text field lands on the character painted under it,
//! for a field whose style spaces its glyphs (`letter-spacing`,
//! `word-spacing`, an italic, padding). The press map
//! (`compute_input_cursor_from_click`) and paint shape the value with one
//! builder, `form_control::field_text_layout`, which is now
//! `ComputedStyle::build_parley_layout`; the oracle here is paint's own
//! glyphs, recorded.

use super::*;
use peniko::kurbo::{Affine, Point, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_dom::paint::painter::{
    BlendMode, GlyphSynthesis, PaintGlyph, PaintImage, PaintShape, Painter,
};
use std::cell::Cell;

const VALUE: &str = "Wave hi to AV";
const STYLE: &str = "display: block; width: 600px; font: italic 24px/30px sans-serif; \
                     letter-spacing: 5.5px; word-spacing: 9px; padding: 3px 13px; \
                     border: 2px solid; color: rgb(0, 1, 0)";

/// The glyphs painted in `rgb(0, 1, 0)` (the field) and in `rgb(0, 0, 1)`
/// (the reference `<div>`).
#[derive(Default)]
struct Glyphs(Vec<Point>, Vec<Point>);

impl Painter for Glyphs {
    fn reset(&mut self) {
        self.0.clear();
        self.1.clear();
    }
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        _: f32,
        transform: Affine,
        _: GlyphSynthesis,
        brush: &Brush,
        _: bool,
        _: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        let Brush::Solid(c) = brush else {
            return;
        };
        let to = match c.to_rgba8().to_u8_array()[..3] {
            [0, 1, 0] => &mut self.0,
            [0, 0, 1] => &mut self.1,
            _ => return,
        };
        to.extend(
            glyphs
                .iter()
                .map(|g| transform * Point::new(g.x as f64, g.y as f64)),
        );
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

fn check(tag: &'static str) {
    let id = Rc::new(Cell::new(0u64));
    let id_in = id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let field = scope.create_element(tag);
        field.set_attribute("style", STYLE);
        field.set_attribute("value", VALUE);
        root.append_child(&field);
        let reference = scope.create_element("div");
        reference.set_attribute(
            "style",
            &format!("{STYLE}; white-space: pre; color: rgb(0, 0, 1)"),
        );
        reference.append_child(&scope.create_text(VALUE));
        root.append_child(&reference);
        id_in.set(field.node_id().0);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let field = id.get();

    let doc = app.doc.clone().expect("mounted");
    let mut d = doc.borrow_mut();
    let d = &mut *d;
    let mut painter = Glyphs::default();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &d.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut d.font_cx,
        &mut cx,
    );
    let xs: Vec<f64> = painter.0.iter().map(|p| p.x).collect();
    assert_eq!(
        xs.len(),
        VALUE.len(),
        "{tag}: positive control, one painted glyph a character"
    );
    let y = painter.0[0].y as f32 - 5.0;
    // Every cluster is wider than 6px here (the spacing alone is 5.5px).
    for (i, &x) in xs.iter().enumerate() {
        for (dx, want) in [(1.5, i), (-1.5, i)] {
            if i == 0 && dx < 0.0 {
                continue;
            }
            let got = RinchApp::compute_input_cursor_from_click(
                &d.tree,
                &mut app.hit_test_font_cx,
                &mut app.hit_test_layout_cx,
                field,
                (x + dx) as f32,
                y,
            );
            assert_eq!(
                got,
                want,
                "{tag}: a press {dx:+}px from the left edge of the glyph painted for \
                 {:?} (x {x:.2}) put the caret at {got}, not {want}",
                &VALUE[i..i + 1]
            );
        }
    }
    // The field's line is the reference div's, glyph for glyph: the spacing
    // and the italic reached the paint, so the agreement above is about a
    // spaced line, not two copies that both dropped it.
    let reference: Vec<f64> = painter.1.iter().map(|p| p.x - painter.1[0].x).collect();
    let field_line: Vec<f64> = xs.iter().map(|x| x - xs[0]).collect();
    assert_eq!(
        reference.len(),
        field_line.len(),
        "{tag}: the reference's glyphs"
    );
    for (i, (f, r)) in field_line.iter().zip(&reference).enumerate() {
        assert!(
            (f - r).abs() < 0.01,
            "{tag}: glyph {i} painted at +{f:.2}px, the div's at +{r:.2}px"
        );
    }
}

#[test]
fn a_press_in_a_spaced_input_lands_on_the_painted_character_320() {
    check("input");
}

#[test]
fn a_press_in_a_spaced_textarea_lands_on_the_painted_character_320() {
    check("textarea");
}
