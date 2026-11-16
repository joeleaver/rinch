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

/// Glyph outlines already read: by font blob id, face index, size and skew
/// bits, a hash of the variation coordinates, and glyph id; `None` for a
/// glyph with no outline. Each is at its glyph's origin, y down, skewed.
/// One per thread, shared by every pass that strokes a glyph (the
/// `background-clip: text` mask and the stroke itself) and by every frame:
/// a HUD whose numerals change every frame reads each digit's outline once.
type OutlineKey = ((u64, u32, u32, u32, u64), u32);

thread_local! {
    static OUTLINES: std::cell::RefCell<std::collections::HashMap<OutlineKey, Option<BezPath>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// How many outlines [`OUTLINES`] keeps before it starts again: a few fonts'
/// worth of glyphs at a few sizes.
const MAX_OUTLINES: usize = 4096;

/// Append the outlines of `glyphs` (a run in `font` at `font_size`, the
/// painter's units, with variation `coords` and the faux italic `skew`) to
/// `path`, each at its glyph's origin, y down. Glyphs the face has no
/// outline for (a colour bitmap, an unreadable font) add nothing. Each glyph
/// is read with skrifa once and kept ([`OUTLINES`]).
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
    use std::hash::{Hash, Hasher};

    let coords_hash = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        coords.hash(&mut h);
        h.finish()
    };
    let run = (
        font.data.id(),
        font.index,
        font_size.to_bits(),
        skew.to_bits(),
        coords_hash,
    );
    OUTLINES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > MAX_OUTLINES {
            cache.clear();
        }
        // The face, read only when a glyph is missing.
        let face = std::cell::OnceCell::new();
        let location: Vec<NormalizedCoord> = coords
            .iter()
            .map(|&c| NormalizedCoord::from_bits(c))
            .collect();
        for g in glyphs {
            let outline = cache.entry((run, g.id)).or_insert_with(|| {
                let face = face
                    .get_or_init(|| skrifa::FontRef::from_index(font.data.data(), font.index).ok());
                let glyph = face
                    .as_ref()?
                    .outline_glyphs()
                    .get(skrifa::GlyphId::new(g.id))?;
                let mut out = BezPath::new();
                let mut pen = PathPen {
                    path: &mut out,
                    place: Affine::new([1.0, 0.0, skew as f64, -1.0, 0.0, 0.0]),
                };
                glyph
                    .draw(
                        DrawSettings::unhinted(Size::new(font_size), LocationRef::new(&location)),
                        &mut pen,
                    )
                    .ok()?;
                Some(out)
            });
            if let Some(outline) = outline {
                let at = Affine::translate((g.x as f64, g.y as f64));
                path.extend(outline.elements().iter().map(|&el| at * el));
            }
        }
    });
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
