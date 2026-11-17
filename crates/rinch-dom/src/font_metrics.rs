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
//!   glyphs **hinted at the used size** (in device pixels) by skrifa's
//!   autohinter in its light mode, which is what Chrome's FreeType `slight`
//!   hinting measures ([`MeasuredFace`]). That lands on whole pixels, and not
//!   always on the rounded unhinted top: DejaVu Serif's `x` is 8.31px at 16px
//!   unhinted, 9px hinted, and Chrome's `10ex` is 90px. No `x` (or `H`):
//!   `None`, the spec's fallback.
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
#[derive(Clone, Debug, Default)]
pub(crate) struct EmMetrics {
    /// x-height from the OS/2 table.
    x_height: Option<f32>,
    /// Cap height from the OS/2 table.
    cap_height: Option<f32>,
    /// The face, when either height is not in its OS/2 table and has to be
    /// measured from a hinted glyph at each size.
    measured: Option<Arc<MeasuredFace>>,
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
            return found.clone();
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
    let tops = match &m.measured {
        Some(face) if m.x_height.is_none() || m.cap_height.is_none() => face.tops(size * dpr),
        _ => (None, None),
    };
    let height = |os2: Option<f32>, top: Option<f32>| match os2 {
        Some(em) => Some(Length::new(em * size)),
        None => top.map(|device_px| Length::new(device_px / dpr)),
    };
    FontMetrics {
        x_height: height(m.x_height, tops.0),
        cap_height: height(m.cap_height, tops.1),
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
    face_metrics(&font.blob, font.index, font.synthesis.variation_settings())
}

/// A face's metrics in ems, read the way Chrome reads them (module docs).
pub(crate) fn face_metrics(
    blob: &parley::fontique::Blob<u8>,
    index: u32,
    variations: &[(skrifa::raw::types::Tag, f32)],
) -> Option<EmMetrics> {
    use skrifa::MetadataProvider;
    use skrifa::instance::Size;
    let face = skrifa::FontRef::from_index(blob.as_ref(), index).ok()?;
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
    // A zero (or absent) OS/2 height is measured from the glyph, as Skia does.
    let os2 = |h: Option<f32>| h.filter(|h| *h > 0.0).map(|h| h / upem);
    let (x_height, cap_height) = (os2(metrics.x_height), os2(metrics.cap_height));
    let measured = (x_height.is_none() || cap_height.is_none()).then(|| {
        Arc::new(MeasuredFace {
            blob: blob.clone(),
            index,
            variations: variations.to_vec(),
            tops: Mutex::default(),
        })
    });
    Some(EmMetrics {
        x_height,
        cap_height,
        measured,
        zero_advance: advance('0'),
        ic_width: advance('\u{6C34}'),
        ascent: metrics.ascent / upem,
    })
}

/// The tops of `x` and `H`, in device pixels.
type Tops = (Option<f32>, Option<f32>);

/// A face whose x-height or cap height is not in its OS/2 table, so it is
/// measured from the top of its `x` and `H` glyphs. Chrome 157 on Linux
/// (Skia's FreeType scaler, `slight` hinting, which FreeType's light target
/// serves with its autohinter whatever the font's own instructions) measures
/// the **hinted** outline at the used size. skrifa's autohinter in its light
/// mode gives Chrome's number for every face and size measured: Times New
/// Roman, Arial, Georgia, Verdana, Courier New, DejaVu Sans, Serif and Sans
/// Mono, at 11, 12, 13, 14, 16, 18, 20 and 24px (both `ex` and `cap`). The
/// unhinted top rounded to a pixel misses a quarter of them (Times New Roman
/// 16px: 7.16 rounds to 7, the hinted `x` is 8).
pub(crate) struct MeasuredFace {
    blob: parley::fontique::Blob<u8>,
    index: u32,
    variations: Vec<(skrifa::raw::types::Tag, f32)>,
    /// `(x top, H top)` in device pixels, per ppem (its bits).
    tops: Mutex<HashMap<u32, Tops>>,
}

impl std::fmt::Debug for MeasuredFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeasuredFace")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

/// Sizes one measured face keeps before it starts over.
const TOPS_PER_FACE: usize = 64;

impl MeasuredFace {
    /// The tops of `x` and `H` at `ppem` device pixels, hinted.
    fn tops(&self, ppem: f32) -> Tops {
        if !(ppem.is_finite() && ppem > 0.0) {
            return (None, None);
        }
        let key = ppem.to_bits();
        if let Some(found) = self
            .tops
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return *found;
        }
        let found = self.measure(ppem);
        let mut tops = self.tops.lock().unwrap_or_else(|e| e.into_inner());
        if tops.len() >= TOPS_PER_FACE {
            tops.clear();
        }
        tops.insert(key, found);
        found
    }

    fn measure(&self, ppem: f32) -> Tops {
        use skrifa::MetadataProvider;
        use skrifa::instance::Size;
        use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions};
        use skrifa::outline::{SmoothMode, Target};
        let Ok(face) = skrifa::FontRef::from_index(self.blob.as_ref(), self.index) else {
            return (None, None);
        };
        let location = face.axes().location(self.variations.iter().copied());
        let outlines = face.outline_glyphs();
        let charmap = face.charmap();
        let options = HintingOptions {
            engine: Engine::Auto(None),
            target: Target::Smooth {
                mode: SmoothMode::Light,
                symmetric_rendering: true,
                preserve_linear_metrics: false,
            },
        };
        let hinter = HintingInstance::new(&outlines, Size::new(ppem), &location, options).ok();
        let top = |c: char| -> Option<f32> {
            let glyph = outlines.get(charmap.map(c)?)?;
            let hinted = hinter.as_ref().and_then(|hinter| {
                let mut pen = TopPen(None);
                glyph
                    .draw(DrawSettings::hinted(hinter, false), &mut pen)
                    .ok()?;
                pen.0
            });
            // No hinter for this face: the unhinted top, on a whole pixel.
            let top = hinted.or_else(|| {
                let mut pen = TopPen(None);
                glyph
                    .draw(DrawSettings::unhinted(Size::new(ppem), &location), &mut pen)
                    .ok()?;
                pen.0.map(f32::round)
            })?;
            (top > 0.0).then_some(top)
        };
        (top('x'), top('H'))
    }
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
    /// 1292, 2048 units per em, no `水`; nothing to measure.
    #[test]
    fn inter_reads_its_os2_heights_and_zero_advance() {
        let blob = parley::fontique::Blob::new(Arc::new(INTER));
        let m = face_metrics(&blob, 0, &[]).unwrap();
        assert_eq!(m.x_height, Some(1118.0 / 2048.0));
        assert_eq!(m.cap_height, Some(1490.0 / 2048.0));
        assert!(m.measured.is_none());
        assert_eq!(m.zero_advance, Some(1292.0 / 2048.0));
        assert_eq!(m.ic_width, None);
    }

    /// An OS/2 height is scaled as it is. A vertical query has no horizontal
    /// advances.
    #[test]
    fn scaling_keeps_os2_heights_unrounded() {
        let m = EmMetrics {
            x_height: Some(0.546),
            cap_height: Some(0.7275),
            measured: None,
            zero_advance: Some(0.6309),
            ic_width: None,
            ascent: 0.96,
        };
        let f = scale(&m, 16.0, 1.0, false);
        assert_eq!(f.x_height, Some(Length::new(0.546 * 16.0)));
        assert_eq!(f.cap_height, Some(Length::new(0.7275 * 16.0)));
        assert_eq!(scale(&m, 16.0, 1.0, true).zero_advance_measure, None);
    }
}
