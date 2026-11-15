//! Glyph outlines as paths, for `-webkit-text-stroke` (#2719).
//!
//! A stroked glyph is not a coverage mask: the glyph-mask atlas
//! (`glyph_atlas.rs`) caches fills, and a stroke of any width around the
//! outline is drawn by the painter's own `stroke` from the outline itself,
//! on both painters. The outline is read with skrifa, unhinted (Chrome's
//! stroked text is the glyph's path, which Skia does not hint), at the run's
//! size and variation coordinates, with a faux italic's skew. A faux bold's
//! emboldening is not applied to the path: the stroke is drawn around the
//! face's own outline.

use peniko::FontData;
use peniko::kurbo::{Affine, BezPath, Point};

use super::painter::PaintGlyph;

/// Append the outlines of `glyphs` (a run in `font` at `font_size`, the
/// painter's units, with variation `coords` and the faux italic `skew`) to
/// `path`, each at its glyph's origin, y down. Glyphs the face has no
/// outline for (a colour bitmap, an unreadable font) add nothing.
pub(super) fn append_glyph_outlines(
    path: &mut BezPath,
    font: &FontData,
    font_size: f32,
    coords: &[i16],
    skew: f32,
    glyphs: &[PaintGlyph],
) {
    use skrifa::MetadataProvider;
    use skrifa::instance::{LocationRef, NormalizedCoord, Size};
    use skrifa::outline::DrawSettings;

    let Ok(face) = skrifa::FontRef::from_index(font.data.data(), font.index) else {
        return;
    };
    let outlines = face.outline_glyphs();
    let coords: Vec<NormalizedCoord> = coords
        .iter()
        .map(|&c| NormalizedCoord::from_bits(c))
        .collect();
    let location = LocationRef::new(&coords);
    for g in glyphs {
        let Some(glyph) = outlines.get(skrifa::GlyphId::new(g.id)) else {
            continue;
        };
        let mut pen = PathPen {
            path,
            place: Affine::new([1.0, 0.0, skew as f64, -1.0, g.x as f64, g.y as f64]),
        };
        let _ = glyph.draw(
            DrawSettings::unhinted(Size::new(font_size), location),
            &mut pen,
        );
    }
}

/// An outline pen that writes into a [`BezPath`], each point through `place`
/// (the face's y-up space to the painter's, at the glyph's origin).
struct PathPen<'a> {
    path: &'a mut BezPath,
    place: Affine,
}

impl PathPen<'_> {
    fn p(&self, x: f32, y: f32) -> Point {
        self.place * Point::new(x as f64, y as f64)
    }
}

impl skrifa::outline::OutlinePen for PathPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.path.move_to(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        self.path.line_to(p);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.p(cx0, cy0), self.p(x, y));
        self.path.quad_to(c, p);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c0, c1, p) = (self.p(cx0, cy0), self.p(cx1, cy1), self.p(x, y));
        self.path.curve_to(c0, c1, p);
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}
