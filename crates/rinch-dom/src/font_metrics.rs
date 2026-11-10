//! The font metrics Stylo resolves `ex`, `ch`, `cap` and `ic` against (#2422).
//!
//! CSS resolves those units against the element's [first available font]:
//! `ex` is its x-height, `cap` its cap height, `ch` the advance of its `0`, `ic`
//! the advance of its `水` (U+6C34). Stylo asks the [`Device`]'s
//! `FontMetricsProvider` for them, and for anything the provider leaves `None`
//! falls back to `0.5em` (`ex`, `ch`), the font's ascent (`cap`) and `1em`
//! (`ic`). Before #2422 rinch's provider answered `FontMetrics::default()`, so
//! `ex` and `ch` were `0.5em` and `cap` was 0 (the default ascent): Gmail's
//! quoted replies (`margin-left: 0.8ex; padding-left: 1ex`, one level per
//! reply) inset each level about 1px less than Chrome at 13px Inter (12.7px
//! against 13.77px), so text quoted ten deep was about 10px wider. Every query now carries the
//! face's ascent too, so a face without a cap height falls back to it as the
//! spec says.
//!
//! # The face, and the numbers
//!
//! The face is fontique's first available font for the element's computed
//! `font-family` (rewritten by [`fonts::parley_font_family`] exactly as for its
//! text), weight and style: the first family in the stack with a face, which is
//! what CSS and Chrome use. From it, as Chrome 157 on Linux (Skia over
//! FreeType) reads them:
//!
//! - **x-height and cap height** from the OS/2 table (`sxHeight`,
//!   `sCapHeight`, version 2 and later), unrounded. A table without them (older
//!   than version 2, or a zero) is measured instead: the top of the `x` and `H`
//!   glyphs, **rounded to whole device pixels**, because Chrome measures a
//!   hinted outline there (Inter with its OS/2 table cut to version 1: `10ex` is
//!   90px at 16px, 70px at 13px; DejaVu Sans Mono, whose table is version 1,
//!   the same). No `x` (or `H`): `None`, the spec's fallback.
//! - **`0` advance** from the `hmtx` advance of the face's `0` glyph,
//!   unrounded; no `0` in the face, `None`. Horizontal only: rinch lays out no
//!   vertical writing mode, so a vertical query gets `None`.
//! - **`水` advance** the same way; most Latin faces have none.
//!
//! Variable faces are read at the location fontique's synthesis asks for (its
//! `wght` for a bold, ...), so a variable face's x-height follows its weight.
//!
//! # Where the face comes from, and the cache
//!
//! Stylo calls the provider from inside a cascade, through `&Device`, and the
//! provider must be `Sync`; the font collection is the document's
//! `font_cx`, borrowed mutably everywhere else and never shared. So the
//! provider does not resolve anything itself. It answers from a cache of
//! per-em metrics keyed by the computed `font-family`, `font-weight` and
//! `font-style` ([`FontMetricsCache`], one per document, carried across Device
//! rebuilds in `DeviceParams`). A key it does not have yet is **recorded** and
//! answered with Stylo's fallbacks; the document's cascade call sites then
//! resolve every recorded key against `font_cx`
//! ([`FontMetricsCache::resolve_pending`]) and cascade that element again,
//! which now hits. A key is resolved once per document (a key whose stack
//! finds no face is cached as having none), so a second cascade happens only
//! for the first element of each new font, and never on a page that uses none
//! of these units: Stylo queries the provider only to resolve one.
//!
//! A font registered later can change a key's face:
//! `RinchDocument::note_fonts_registered` drops the cache and restyles the
//! elements whose style Stylo flagged as depending on font metrics
//! (`DEPENDS_ON_SELF_FONT_METRICS`, `DEPENDS_ON_INHERITED_FONT_METRICS`), with
//! their subtrees.
//!
//! [first available font]: https://drafts.csswg.org/css-fonts-4/#first-available-font
//! [`Device`]: style::device::Device
//! [`fonts::parley_font_family`]: crate::fonts::parley_font_family

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use style::font_metrics::FontMetrics;
use style::properties::style_structs::Font as StyloFont;
use style::values::computed::font::{FontFamily, FontStyle, GenericFontFamily};
use style::values::computed::{CSSPixelLength, Length};
use style::values::specified::font::QueryFontMetricsFlags;

/// What a face's metrics depend on in a computed style: the family list, the
/// weight and the style (rinch hands parley no `font-width`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MetricsKey {
    family: FontFamily,
    /// `font-weight`'s number, as bits (`FontWeight` is not `Eq`).
    weight: u32,
    style: FontStyle,
}

impl MetricsKey {
    fn of(font: &StyloFont) -> Self {
        Self {
            family: font.font_family.clone(),
            weight: font.font_weight.value().to_bits(),
            style: font.font_style,
        }
    }
}

/// One face's metrics, in ems.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct EmMetrics {
    /// x-height, and whether it was measured from the `x` glyph (rounded to
    /// device pixels when scaled).
    x_height: Option<(f32, bool)>,
    /// Cap height, and whether it was measured from the `H` glyph.
    cap_height: Option<(f32, bool)>,
    /// Advance of `0`.
    zero_advance: Option<f32>,
    /// Advance of `水` (U+6C34).
    ic_width: Option<f32>,
    /// Ascent (Stylo always wants one).
    ascent: f32,
}

#[derive(Debug, Default)]
struct State {
    /// Resolved keys; `None` when the stack found no face.
    resolved: HashMap<MetricsKey, Option<EmMetrics>>,
    /// Keys queried since the last [`FontMetricsCache::resolve_pending`],
    /// not yet resolved.
    pending: Vec<MetricsKey>,
}

/// One document's font metrics, shared by every [`Device`] built for it.
///
/// [`Device`]: style::device::Device
#[derive(Debug, Default)]
pub(crate) struct FontMetricsCache {
    state: Mutex<State>,
}

impl FontMetricsCache {
    fn lookup(&self, font: &StyloFont) -> Option<EmMetrics> {
        let key = MetricsKey::of(font);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = state.resolved.get(&key) {
            return *found;
        }
        if !state.pending.contains(&key) {
            state.pending.push(key);
        }
        None
    }

    /// Resolve every key queried since the last call against `font_cx`.
    /// `true` when there was one: the cascade that queried it answered with
    /// Stylo's fallbacks and has to run again.
    pub(crate) fn resolve_pending(&self, font_cx: &mut parley::FontContext) -> bool {
        let pending = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.pending.is_empty() {
                return false;
            }
            std::mem::take(&mut state.pending)
        };
        let resolved: Vec<_> = pending
            .into_iter()
            .map(|key| {
                let metrics = resolve(font_cx, &key);
                (key, metrics)
            })
            .collect();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.resolved.extend(resolved);
        true
    }

    /// Forget every resolved key (fonts were registered). `true` when there
    /// was one, i.e. some style may have resolved against a face that is no
    /// longer the first available one.
    pub(crate) fn clear(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let had = !state.resolved.is_empty();
        state.resolved.clear();
        state.pending.clear();
        had
    }
}

/// The Device's provider: [`FontMetricsCache`]'s answers, scaled.
#[derive(Debug)]
pub(crate) struct RinchFontMetricsProvider {
    pub(crate) cache: Arc<FontMetricsCache>,
    /// Device pixels per CSS pixel: a glyph-measured height is whole device
    /// pixels.
    pub(crate) device_pixel_ratio: f32,
}

impl style::device::servo::FontMetricsProvider for RinchFontMetricsProvider {
    fn query_font_metrics(
        &self,
        vertical: bool,
        font: &StyloFont,
        base_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> FontMetrics {
        let Some(m) = self.cache.lookup(font) else {
            return FontMetrics::default();
        };
        scale(&m, base_size.px(), self.device_pixel_ratio, vertical)
    }

    fn base_size_for_generic(&self, _generic: GenericFontFamily) -> Length {
        // Default base font size (16px for most generics)
        Length::new(16.0)
    }
}

/// `m` at `size` CSS px.
fn scale(m: &EmMetrics, size: f32, dpr: f32, vertical: bool) -> FontMetrics {
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let height = |h: Option<(f32, bool)>| {
        h.map(|(em, from_glyph)| {
            let px = em * size;
            Length::new(if from_glyph {
                (px * dpr).round() / dpr
            } else {
                px
            })
        })
    };
    FontMetrics {
        x_height: height(m.x_height),
        cap_height: height(m.cap_height),
        zero_advance_measure: if vertical {
            None
        } else {
            m.zero_advance.map(|em| Length::new(em * size))
        },
        ic_width: if vertical {
            None
        } else {
            m.ic_width.map(|em| Length::new(em * size))
        },
        ascent: Length::new(m.ascent * size),
        script_percent_scale_down: None,
        script_script_percent_scale_down: None,
    }
}

/// The first available font of `key` in `font_cx`, and its metrics.
fn resolve(font_cx: &mut parley::FontContext, key: &MetricsKey) -> Option<EmMetrics> {
    use parley::fontique::{Attributes, QueryFamily};
    use parley::style::{FontFamily as ParleyFamily, FontFamilyName};
    let stack = crate::computed_style::font_family_css(&key.family);
    let ParleyFamily::Source(stack) = crate::fonts::parley_font_family(font_cx, &stack) else {
        return None;
    };
    let names: Vec<FontFamilyName<'_>> = FontFamilyName::parse_css_list(&stack)
        .map_while(Result::ok)
        .collect();
    let style = crate::computed_style::font_style_css(&key.style).to_parley();
    let font = {
        let mut query = font_cx.collection.query(&mut font_cx.source_cache);
        query.set_families(names.iter().map(|name| match name {
            FontFamilyName::Named(name) => QueryFamily::Named(name),
            FontFamilyName::Generic(generic) => QueryFamily::Generic(*generic),
        }));
        query.set_attributes(Attributes {
            weight: parley::style::FontWeight::new(f32::from_bits(key.weight)),
            style,
            ..Default::default()
        });
        query.first_available_font()?
    };
    face_metrics(
        font.blob.as_ref(),
        font.index,
        font.synthesis.variation_settings(),
    )
}

/// A face's metrics in ems, read the way Chrome reads them (module docs).
pub(crate) fn face_metrics(
    data: &[u8],
    index: u32,
    variations: &[(skrifa::raw::types::Tag, f32)],
) -> Option<EmMetrics> {
    use skrifa::MetadataProvider;
    use skrifa::instance::Size;
    let face = skrifa::FontRef::from_index(data, index).ok()?;
    let location = face.axes().location(variations.iter().copied());
    let metrics = skrifa::metrics::Metrics::new(&face, Size::unscaled(), &location);
    let upem = f32::from(metrics.units_per_em);
    if upem <= 0.0 {
        return None;
    }
    let glyphs = face.glyph_metrics(Size::unscaled(), &location);
    let charmap = face.charmap();
    let advance = |c: char| {
        charmap
            .map(c)
            .and_then(|gid| glyphs.advance_width(gid))
            .map(|w| w / upem)
    };
    let outlines = face.outline_glyphs();
    let glyph_top = |c: char| -> Option<f32> {
        let glyph = outlines.get(charmap.map(c)?)?;
        let mut pen = TopPen(None);
        glyph
            .draw(
                skrifa::outline::DrawSettings::unhinted(Size::unscaled(), &location),
                &mut pen,
            )
            .ok()?;
        pen.0.filter(|top| *top > 0.0).map(|top| top / upem)
    };
    // A zero (or absent) OS/2 height is measured from the glyph, as Skia does.
    let height = |os2: Option<f32>, c: char| match os2.filter(|h| *h > 0.0) {
        Some(h) => Some((h / upem, false)),
        None => glyph_top(c).map(|em| (em, true)),
    };
    Some(EmMetrics {
        x_height: height(metrics.x_height, 'x'),
        cap_height: height(metrics.cap_height, 'H'),
        zero_advance: advance('0'),
        ic_width: advance('\u{6C34}'),
        ascent: metrics.ascent / upem,
    })
}

/// The highest point of an outline (its control points: exact for the
/// straight-topped `x` and `H` these measure).
struct TopPen(Option<f32>);

impl TopPen {
    fn point(&mut self, y: f32) {
        self.0 = Some(self.0.map_or(y, |top| top.max(y)));
    }
}

impl skrifa::outline::OutlinePen for TopPen {
    fn move_to(&mut self, _x: f32, y: f32) {
        self.point(y);
    }
    fn line_to(&mut self, _x: f32, y: f32) {
        self.point(y);
    }
    fn quad_to(&mut self, _cx0: f32, cy0: f32, _x: f32, y: f32) {
        self.point(cy0);
        self.point(y);
    }
    fn curve_to(&mut self, _cx0: f32, cy0: f32, _cx1: f32, cy1: f32, _x: f32, y: f32) {
        self.point(cy0);
        self.point(cy1);
        self.point(y);
    }
    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

    /// Inter's OS/2 table: `sxHeight` 1118, `sCapHeight` 1490, `0` advance
    /// 1292, 2048 units per em, no `水`.
    #[test]
    fn inter_reads_its_os2_heights_and_zero_advance() {
        let m = face_metrics(INTER, 0, &[]).unwrap();
        assert_eq!(m.x_height, Some((1118.0 / 2048.0, false)));
        assert_eq!(m.cap_height, Some((1490.0 / 2048.0, false)));
        assert_eq!(m.zero_advance, Some(1292.0 / 2048.0));
        assert_eq!(m.ic_width, None);
    }

    /// A glyph-measured height lands on a whole device pixel; an OS/2 one is
    /// left as it is. A vertical query has no horizontal advances.
    #[test]
    fn scaling_rounds_only_glyph_measured_heights() {
        let m = EmMetrics {
            x_height: Some((0.546, true)),
            cap_height: Some((0.7275, false)),
            zero_advance: Some(0.6309),
            ic_width: None,
            ascent: 0.96,
        };
        let f = scale(&m, 16.0, 1.0, false);
        assert_eq!(f.x_height, Some(Length::new(9.0)));
        assert_eq!(f.cap_height, Some(Length::new(0.7275 * 16.0)));
        let f2 = scale(&m, 16.0, 2.0, false);
        assert_eq!(f2.x_height, Some(Length::new(8.5)));
        assert_eq!(scale(&m, 16.0, 1.0, true).zero_advance_measure, None);
    }
}
