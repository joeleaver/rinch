//! The glyph-mask atlas both painters draw text from (#2286, stage 2).
//!
//! Each glyph is rasterised once by swash — the rasteriser the software
//! painter has always used, FreeType-grade on the design's coverage metric —
//! per **(font, size, hinting, emboldening, variation instance, glyph, x
//! bucket)**, and kept as a one-byte-a-pixel coverage mask. The x bucket is the
//! glyph's horizontal sub-pixel position, quantised to [`SUBPIXEL_BUCKETS`]
//! steps, as upstream Vello's `glifo` does: a glyph at `x = 10.3` is the bucket
//! `0.25` mask drawn at whole pixel 10. Vertically a glyph sits on a whole
//! device pixel (the baseline is rounded), which is what hinting assumes.
//!
//! - **tiny-skia** blits the mask in the text colour
//!   ([`TinySkiaPainter::draw_glyphs`](super::skia_painter::TinySkiaPainter)).
//! - **Vello** draws the same mask, as an RGBA image with the coverage in
//!   every channel, through the fork's tinted alpha-mask image
//!   (`Scene::draw_image_mask`): the fine shader reads the coverage and paints
//!   the colour in it, so one mask serves every colour a glyph is drawn in
//!   (a `text-shadow`, a selection's recolouring, a hover). The image is one
//!   `Blob` for the mask's life, and Vello keys its GPU atlas on the blob's id,
//!   so a mask is uploaded once and stays resident while it is drawn.
//!
//! So the two painters draw identical glyph pixels wherever both use the
//! atlas. Why a tint, not a colour baked into each image (which works with
//! no fork change): stage 3's subpixel masks need a per-channel composite
//! against the destination, which only the fine shader can do, and the tint
//! word and mask mode are where that goes (mode 2). See
//! `docs/internals/paint-and-damage.md` "Glyph-mask atlas".
//!
//! **What the atlas does not draw** (each painter's own path does): a glyph
//! under a rotation, skew or non-uniform scale (a uniform scale is
//! rasterised again at the device size, so HiDPI and `scale()` stay crisp), a
//! run with a glyph transform (synthetic italic), a non-solid brush, and on
//! Vello a colour glyph (COLR, bitmap strikes: Vello draws those itself) and a
//! device size over [`ATLAS_MAX_FONT_PX`] (an outline is cheaper than its mask
//! there, and as sharp).
//!
//! **Eviction.** Every entry remembers the last generation it was drawn in
//! ([`GlyphAtlas::begin_frame`] starts one). Past [`GlyphAtlas::budget`]
//! accounted bytes, the entries drawn longest ago go first, down to three
//! quarters of the budget; if what one frame draws is bigger than the
//! budget, entries it drew go too (a frame never fails for it, the next one
//! rasterises again). Evicting is always safe: a Vello scene holds its own
//! reference to every mask it draws, and tiny-skia has already blitted.
//!
//! **GPU lifetime.** rinch holds no GPU resource here: the masks are CPU
//! blobs, and the texture they live in on the GPU is Vello's image atlas,
//! owned by the `Renderer`. A renderer made again after a device loss starts
//! with an empty atlas and uploads each mask the first time a scene draws it.

use std::collections::HashMap;

use peniko::kurbo::Affine;
use peniko::{Blob, FontData, ImageAlphaType, ImageData, ImageFormat};

/// Horizontal sub-pixel positions a glyph is rasterised at: `0`, `0.25`,
/// `0.5`, `0.75` of a pixel (upstream glifo's `SUBPIXEL_BUCKETS`).
pub const SUBPIXEL_BUCKETS: u8 = 4;

/// The largest device font size (px) the Vello painter draws through the
/// atlas; larger glyphs are drawn as outlines. A 64px glyph's mask is about
/// 64x64 coverage bytes (16 KiB on the GPU); an outline costs the same
/// whatever its size, and at this size it is as sharp as the mask.
pub const ATLAS_MAX_FONT_PX: f32 = 64.0;

/// Accounted bytes the atlas keeps by default before it evicts.
pub const DEFAULT_BUDGET: usize = 32 * 1024 * 1024;

/// Per-entry bookkeeping charged on top of the image bytes: the map slot, the
/// entry and the allocations' rounding (measured on the software painter's
/// former glyph cache: about 118 bytes; see the history of `GlyphCache`).
pub const ENTRY_OVERHEAD: usize = 128;

/// Distinct (font, size, hinting, emboldening, instance) runs kept at most:
/// past it, the runs drawn longest ago are dropped whole.
pub const MAX_RUNS: usize = 512;

/// What the atlas did, for the perf counters (`glyph_atlas_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    /// Glyphs drawn from a mask already in the atlas.
    pub hits: u64,
    /// Glyphs rasterised (then kept).
    pub misses: u64,
    /// Masks handed to the GPU for the first time: the RGBA image a Vello
    /// scene draws a mask through, made the first time one does. Vello uploads
    /// it when it first renders it, and again only if its own atlas dropped
    /// it (a renderer made again, or space it needed); those re-uploads are
    /// Vello's and are not counted.
    pub uploads: u64,
    /// Masks evicted to stay within the budget.
    pub evictions: u64,
}

impl AtlasStats {
    /// Add these counts to the current frame of `perf`.
    pub fn add_to(&self, perf: &crate::perf::PerfCounters) {
        use crate::perf::Counter;
        perf.add(Counter::GlyphAtlasHits, self.hits);
        perf.add(Counter::GlyphAtlasMisses, self.misses);
        perf.add(Counter::GlyphAtlasUploads, self.uploads);
        perf.add(Counter::GlyphAtlasEvictions, self.evictions);
    }

    /// The sum of two counts.
    pub fn merged(self, o: AtlasStats) -> AtlasStats {
        AtlasStats {
            hits: self.hits + o.hits,
            misses: self.misses + o.misses,
            uploads: self.uploads + o.uploads,
            evictions: self.evictions + o.evictions,
        }
    }
}

/// What a mask holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskKind {
    /// One coverage byte a pixel, painted in the text colour.
    Coverage,
    /// RGBA (a COLR glyph or a colour bitmap strike), drawn as it is; the
    /// text colour's alpha still applies. `premultiplied`: a scaled bitmap
    /// strike, premultiplied before it was scaled (#804); otherwise straight.
    Color {
        /// The data is premultiplied already.
        premultiplied: bool,
    },
}

/// One rasterised glyph.
pub struct GlyphMask {
    /// Left edge of the image relative to the glyph's whole-pixel origin.
    pub left: i32,
    /// Top edge above the baseline (swash's placement: positive is up).
    pub top: i32,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// What `data` holds.
    pub kind: MaskKind,
    /// The pixels: `width * height` bytes for [`MaskKind::Coverage`],
    /// four times that for a colour glyph.
    pub data: Vec<u8>,
    /// The GPU's copy of a coverage mask: RGBA, the coverage in every channel
    /// (so premultiplied), made the first time a Vello scene draws the mask.
    gpu: Option<ImageData>,
}

impl GlyphMask {
    /// The image a Vello scene draws this coverage mask through, if one has
    /// been made (the painter asks for it with `want_gpu`).
    pub fn gpu_image(&self) -> Option<&ImageData> {
        self.gpu.as_ref()
    }

    fn bytes(&self) -> usize {
        self.data.len() + self.gpu.as_ref().map_or(0, |g| g.data.len())
    }

    /// Make the GPU copy; returns its size in bytes.
    fn make_gpu(&mut self) -> usize {
        let mut rgba = Vec::with_capacity(self.data.len() * 4);
        for &c in &self.data {
            rgba.extend_from_slice(&[c, c, c, c]);
        }
        let n = rgba.len();
        self.gpu = Some(ImageData {
            data: Blob::from(rgba),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width: self.width,
            height: self.height,
        });
        n
    }
}

/// Everything about a glyph run that changes a glyph's pixels, except the
/// glyph and its bucket.
#[derive(Clone, PartialEq)]
struct RunKey {
    /// `Blob::id()` of the font file's bytes: unique per blob for the life
    /// of the process, so a font dropped and another loaded never alias.
    font_id: u64,
    font_index: u32,
    /// `f32::to_bits` of the device font size. Exact: real documents use a
    /// handful of sizes, and two sizes an ulp apart can rasterise differently.
    size_bits: u32,
    hint: bool,
    /// `f32::to_bits` of swash's emboldening strength (0 for none).
    embolden_bits: u32,
    coords: Box<[i16]>,
}

/// One cached glyph: its mask (`None` for a glyph with no pixels, a space)
/// and the generation it was last drawn in.
struct Entry {
    mask: Option<GlyphMask>,
    used: u64,
}

struct Run {
    key: RunKey,
    /// Keyed by `glyph_id << 2 | bucket`. Fx-hashed: a lookup a glyph a frame
    /// is on the hottest draw path.
    glyphs: rustc_hash::FxHashMap<u32, Entry>,
    used: u64,
}

/// The glyph-mask cache. One per thread by default ([`thread_atlas`]), shared
/// by every painter on it, so a Vello window, its snapshots and slices and a
/// software window all draw one set of masks.
pub struct GlyphAtlas {
    runs: Vec<Run>,
    last: usize,
    bytes: usize,
    budget: usize,
    generation: u64,
    scale: swash::scale::ScaleContext,
    /// A second context for colour bitmap strikes (#804), read at their own
    /// ppem by a scaler built while the run's scaler is alive.
    bitmap_scale: swash::scale::ScaleContext,
    /// One swash cache key per font file, so swash's own scaler cache is hit
    /// across calls (`FontRef::from_index` mints a fresh key every time).
    font_keys: HashMap<(u64, u32), swash::CacheKey>,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::with_budget(DEFAULT_BUDGET)
    }
}

/// The glyph run a lookup is for.
pub struct RunSpec<'a> {
    /// The font.
    pub font: &'a FontData,
    /// Device font size in px.
    pub size: f32,
    /// Hinted (the text painter always asks for hinting).
    pub hint: bool,
    /// swash emboldening strength in px (0 for none).
    pub embolden: f32,
    /// Variation coordinates.
    pub coords: &'a [i16],
}

/// Where a glyph lands on the device grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The whole device pixel the glyph's origin is drawn at.
    pub x: i32,
    /// The device row of the baseline.
    pub y: i32,
    /// The sub-pixel bucket, `0..SUBPIXEL_BUCKETS`.
    pub bucket: u8,
}

/// The device position `(x, y)` on the atlas's grid: the bucket nearest the
/// fractional x (a fraction past the last bucket rounds up to the next pixel's
/// bucket 0), and the baseline on the nearest row.
pub fn place(x: f64, y: f64) -> Placement {
    let n = SUBPIXEL_BUCKETS as f64;
    let q = (x * n).round();
    let ix = (q / n).floor();
    let bucket = (q - ix * n) as u8;
    Placement {
        x: ix as i32,
        y: (y + 0.5).floor() as i32,
        bucket: bucket.min(SUBPIXEL_BUCKETS - 1),
    }
}

/// The uniform scale `transform` draws glyphs at, if the atlas can draw under
/// it: no rotation or skew, the same positive scale on both axes, finite. A
/// glyph is then rasterised at `font_size * scale` and placed by mapping its
/// origin. `None` sends the run to the painter's own path.
pub fn atlas_scale(transform: Affine) -> Option<f64> {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    (b == 0.0
        && c == 0.0
        && a > 0.0
        && (a - d).abs() <= a * 1e-6
        && a.is_finite()
        && e.is_finite()
        && f.is_finite())
    .then_some(a)
}

impl GlyphAtlas {
    /// An atlas that evicts past `budget` accounted bytes.
    pub fn with_budget(budget: usize) -> Self {
        Self {
            runs: Vec::new(),
            last: 0,
            bytes: 0,
            budget,
            generation: 1,
            scale: swash::scale::ScaleContext::new(),
            bitmap_scale: swash::scale::ScaleContext::new(),
            font_keys: HashMap::new(),
        }
    }

    /// The accounted bytes held: mask bytes, GPU copies and
    /// [`ENTRY_OVERHEAD`] an entry.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The byte budget.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Change the budget; the next lookup evicts down to it.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
    }

    /// Masks held, empty glyphs included.
    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.glyphs.len()).sum()
    }

    /// No mask is held.
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.glyphs.is_empty())
    }

    /// Whether the mask for `id` at `bucket` in the run `spec` is held, with
    /// no lookup (nothing counted, nothing rasterised).
    pub fn holds(&self, spec: &RunSpec<'_>, id: u32, bucket: u8) -> bool {
        self.runs.iter().any(|r| {
            r.key.font_id == spec.font.data.id()
                && r.key.font_index == spec.font.index
                && r.key.size_bits == spec.size.to_bits()
                && r.key.hint == spec.hint
                && r.key.embolden_bits == spec.embolden.to_bits()
                && *r.key.coords == *spec.coords
                && r.glyphs.contains_key(&key(id, bucket))
        })
    }

    /// Drop every mask.
    pub fn clear(&mut self) {
        self.runs.clear();
        self.last = 0;
        self.bytes = 0;
        self.font_keys.clear();
    }

    /// Start a new generation: what is drawn from now on is newer than what
    /// was drawn before, for eviction. Painters call it when they start a
    /// frame; several painters a frame only make generations finer.
    pub fn begin_frame(&mut self) {
        self.generation += 1;
    }

    fn run_index(&mut self, spec: &RunSpec<'_>) -> usize {
        let font_id = spec.font.data.id();
        let size_bits = spec.size.to_bits();
        let embolden_bits = spec.embolden.to_bits();
        let matches = |k: &RunKey| {
            k.font_id == font_id
                && k.font_index == spec.font.index
                && k.size_bits == size_bits
                && k.hint == spec.hint
                && k.embolden_bits == embolden_bits
                && *k.coords == *spec.coords
        };
        let generation = self.generation;
        if let Some(r) = self.runs.get_mut(self.last).filter(|r| matches(&r.key)) {
            r.used = generation;
            return self.last;
        }
        if let Some(i) = self.runs.iter().position(|r| matches(&r.key)) {
            self.last = i;
            self.runs[i].used = generation;
            return i;
        }
        self.runs.push(Run {
            key: RunKey {
                font_id,
                font_index: spec.font.index,
                size_bits,
                hint: spec.hint,
                embolden_bits,
                coords: spec.coords.into(),
            },
            glyphs: Default::default(),
            used: generation,
        });
        self.last = self.runs.len() - 1;
        self.last
    }

    /// Look up (rasterising on a miss) each glyph of `glyphs` — `(id,
    /// bucket)` pairs — in the run `spec`, calling `draw` with its index in
    /// `glyphs` and its mask (`None`: the glyph has no pixels, or the font
    /// could not be read). With `want_gpu`, a coverage mask's GPU image is
    /// made before `draw` sees it.
    ///
    /// Counts into `stats`. Evicts (after the last `draw`) when the budget is
    /// exceeded.
    pub fn lookup(
        &mut self,
        spec: &RunSpec<'_>,
        glyphs: &[(u32, u8)],
        want_gpu: bool,
        stats: &mut AtlasStats,
        mut draw: impl FnMut(usize, Option<&GlyphMask>),
    ) {
        if glyphs.is_empty() {
            return;
        }
        let font_data: &[u8] = spec.font.data.as_ref();
        let Some(mut font_ref) = swash::FontRef::from_index(font_data, spec.font.index as usize)
        else {
            for i in 0..glyphs.len() {
                draw(i, None);
            }
            return;
        };
        if self.runs.len() >= MAX_RUNS {
            self.evict_runs(stats);
        }
        font_ref.key = *self
            .font_keys
            .entry((spec.font.data.id(), spec.font.index))
            .or_insert(font_ref.key);
        let run = self.run_index(spec);
        let generation = self.generation;

        let Self {
            runs,
            bytes,
            scale,
            bitmap_scale,
            ..
        } = self;
        let cached = &mut runs[run].glyphs;
        // Built only when something misses: a fully cached run never pays for
        // a scaler (which is where swash sets up hinting state).
        let mut scaler = glyphs
            .iter()
            .any(|&(id, b)| !cached.contains_key(&key(id, b)))
            .then(|| {
                scale
                    .builder(font_ref)
                    .size(spec.size)
                    .hint(spec.hint)
                    .normalized_coords(spec.coords)
                    .build()
            });
        for (i, &(id, bucket)) in glyphs.iter().enumerate() {
            let k = key(id, bucket);
            let entry = match cached.entry(k) {
                std::collections::hash_map::Entry::Occupied(o) => {
                    stats.hits += 1;
                    o.into_mut()
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    stats.misses += 1;
                    let mask = scaler
                        .as_mut()
                        .and_then(|s| rasterise(s, bitmap_scale, font_ref, spec, id, bucket));
                    *bytes += mask.as_ref().map_or(0, GlyphMask::bytes) + ENTRY_OVERHEAD;
                    v.insert(Entry { mask, used: 0 })
                }
            };
            entry.used = generation;
            if want_gpu
                && let Some(m) = entry.mask.as_mut()
                && m.kind == MaskKind::Coverage
                && m.gpu.is_none()
            {
                *bytes += m.make_gpu();
                stats.uploads += 1;
            }
            draw(i, entry.mask.as_ref());
        }
        if self.bytes > self.budget {
            self.evict(stats);
        }
    }

    /// Rasterise one glyph without keeping it (the software painter's
    /// reference mode, which paints as if there were no cache).
    pub fn rasterise_uncached(
        &mut self,
        spec: &RunSpec<'_>,
        id: u32,
        bucket: u8,
    ) -> Option<GlyphMask> {
        let font_data: &[u8] = spec.font.data.as_ref();
        let font_ref = swash::FontRef::from_index(font_data, spec.font.index as usize)?;
        let mut scaler = self
            .scale
            .builder(font_ref)
            .size(spec.size)
            .hint(spec.hint)
            .normalized_coords(spec.coords)
            .build();
        rasterise(
            &mut scaler,
            &mut self.bitmap_scale,
            font_ref,
            spec,
            id,
            bucket,
        )
    }

    /// Evict, oldest generation first, down to three quarters of the budget.
    fn evict(&mut self, stats: &mut AtlasStats) {
        let target = self.budget / 4 * 3;
        let mut all: Vec<(u64, usize, u32)> = Vec::new();
        for (ri, run) in self.runs.iter().enumerate() {
            for (&k, e) in &run.glyphs {
                all.push((e.used, ri, k));
            }
        }
        all.sort_unstable();
        for (_, ri, k) in all {
            if self.bytes <= target {
                break;
            }
            if let Some(e) = self.runs[ri].glyphs.remove(&k) {
                self.bytes -= e.mask.as_ref().map_or(0, GlyphMask::bytes) + ENTRY_OVERHEAD;
                stats.evictions += 1;
            }
        }
        self.runs.retain(|r| !r.glyphs.is_empty());
        self.last = 0;
    }

    /// Drop the quarter of the runs drawn longest ago, whole.
    fn evict_runs(&mut self, stats: &mut AtlasStats) {
        let mut order: Vec<(u64, usize)> = self
            .runs
            .iter()
            .enumerate()
            .map(|(i, r)| (r.used, i))
            .collect();
        order.sort_unstable();
        let drop: std::collections::HashSet<usize> = order
            .iter()
            .take(self.runs.len() / 4 + 1)
            .map(|&(_, i)| i)
            .collect();
        let mut i = 0;
        let bytes = &mut self.bytes;
        self.runs.retain(|r| {
            let keep = !drop.contains(&i);
            i += 1;
            if !keep {
                for e in r.glyphs.values() {
                    *bytes -= e.mask.as_ref().map_or(0, GlyphMask::bytes) + ENTRY_OVERHEAD;
                    stats.evictions += 1;
                }
            }
            keep
        });
        self.last = 0;
        let live: std::collections::HashSet<(u64, u32)> = self
            .runs
            .iter()
            .map(|r| (r.key.font_id, r.key.font_index))
            .collect();
        self.font_keys.retain(|k, _| live.contains(k));
    }
}

thread_local! {
    static THREAD_ATLAS: SharedAtlas = SharedAtlas::default();
}

/// A handle on an atlas a painter draws from. Painters share the thread's
/// ([`thread_atlas`]) unless given their own.
pub type SharedAtlas = std::rc::Rc<std::cell::RefCell<GlyphAtlas>>;

/// This thread's atlas, which every painter made on the thread draws from.
pub fn thread_atlas() -> SharedAtlas {
    THREAD_ATLAS.with(|a| a.clone())
}

fn key(id: u32, bucket: u8) -> u32 {
    (id << 2) | (bucket as u32 & 3)
}

/// Rasterise `id` at sub-pixel `bucket`: a colour outline or bitmap strike
/// as colour, else the outline as coverage. The order is swash's own, as the
/// software painter has always asked (#804).
fn rasterise(
    scaler: &mut swash::scale::Scaler<'_>,
    bitmap_scale: &mut swash::scale::ScaleContext,
    font: swash::FontRef<'_>,
    spec: &RunSpec<'_>,
    id: u32,
    bucket: u8,
) -> Option<GlyphMask> {
    use swash::scale::{Render, Source};
    let offset = swash::zeno::Vector::new(bucket as f32 / SUBPIXEL_BUCKETS as f32, 0.0);
    let render = |sources: &[Source], s: &mut swash::scale::Scaler<'_>| {
        let mut r = Render::new(sources);
        r.offset(offset);
        if spec.embolden != 0.0 {
            r.embolden(spec.embolden);
        }
        r.render(s, id as u16)
    };
    if !scaler.has_color_bitmaps() {
        return render(&[Source::ColorOutline(0), Source::Outline], scaler).and_then(to_mask);
    }
    if let Some(image) = render(&[Source::ColorOutline(0)], scaler) {
        return to_mask(image);
    }
    if let Some(g) = color_bitmap_glyph(bitmap_scale, font, spec.size, id as u16) {
        return Some(g);
    }
    render(&[Source::Outline], scaler).and_then(to_mask)
}

fn to_mask(image: swash::scale::image::Image) -> Option<GlyphMask> {
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }
    let (kind, data) = match image.content {
        swash::scale::image::Content::Mask => (MaskKind::Coverage, image.data),
        swash::scale::image::Content::Color => (
            MaskKind::Color {
                premultiplied: false,
            },
            image.data,
        ),
        swash::scale::image::Content::SubpixelMask => (
            MaskKind::Coverage,
            image
                .data
                .chunks(4)
                .map(|px| px.get(3).copied().unwrap_or(0))
                .collect(),
        ),
    };
    Some(GlyphMask {
        left: image.placement.left,
        top: image.placement.top,
        width: image.placement.width,
        height: image.placement.height,
        kind,
        data,
        gpu: None,
    })
}

/// A colour bitmap glyph (CBDT/CBLC, sbix) at `font_size` (#804), or `None`
/// when the face has no strike holding `glyph`.
///
/// The strike is the smallest whose ppem is at least the font size (shrinking
/// keeps detail), else the largest. It is decoded at its own size — a scaler
/// at exactly the strike's ppem, so swash does not resample it — then
/// premultiplied and only then scaled, by area averaging: swash's own resize
/// filters straight alpha, which drags the colour of the transparent pixels
/// (black, in Noto Color Emoji) into every edge.
pub(crate) fn color_bitmap_glyph(
    cx: &mut swash::scale::ScaleContext,
    font: swash::FontRef<'_>,
    font_size: f32,
    glyph: u16,
) -> Option<GlyphMask> {
    let best = pick_strike(
        font.color_strikes()
            .filter(|strike| strike.contains(glyph))
            .map(|strike| strike.ppem()),
        font_size,
    );
    let ppem = best.filter(|&p| p > 0)?;
    let mut scaler = cx.builder(font).size(ppem as f32).build();
    let image = scaler.scale_color_bitmap(glyph, swash::scale::StrikeWith::ExactSize)?;
    let (sw, sh) = (image.placement.width, image.placement.height);
    if sw == 0 || sh == 0 || image.content != swash::scale::image::Content::Color {
        return None;
    }
    let scale = font_size / ppem as f32;
    let dw = ((sw as f32 * scale).round() as u32).max(1);
    let dh = ((sh as f32 * scale).round() as u32).max(1);
    let mut data = image.data;
    data.truncate((sw * sh * 4) as usize);
    let data = scale_strike(data, sw, sh, dw, dh);
    Some(GlyphMask {
        left: (image.placement.left as f32 * scale).round() as i32,
        top: (image.placement.top as f32 * scale).round() as i32,
        width: dw,
        height: dh,
        kind: MaskKind::Color {
            premultiplied: true,
        },
        data,
        gpu: None,
    })
}

#[inline]
fn premultiply_channel(c: u8, alpha: u8) -> u8 {
    ((c as u16 * alpha as u16 + 127) / 255) as u8
}

/// A strike's straight-alpha RGBA, premultiplied and then scaled from
/// `sw`x`sh` to `dw`x`dh` (#804). Premultiplying FIRST is the point: the colour
/// under a transparent pixel is arbitrary, and averaging straight alpha would
/// blend it into the edge.
pub(crate) fn scale_strike(mut rgba: Vec<u8>, sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a < 255 {
            px[0] = premultiply_channel(px[0], a);
            px[1] = premultiply_channel(px[1], a);
            px[2] = premultiply_channel(px[2], a);
        }
    }
    if (dw, dh) == (sw, sh) {
        rgba
    } else {
        resample_area(&rgba, sw, sh, dw, dh)
    }
}

/// The strike a colour bitmap glyph is read from, among the `ppems` of the
/// strikes holding it, for a physical `font_size`: the smallest at least the
/// font size (shrinking keeps detail), else the largest.
pub(crate) fn pick_strike(ppems: impl Iterator<Item = u16>, font_size: f32) -> Option<u16> {
    let mut best: Option<u16> = None;
    for ppem in ppems {
        best = Some(match best {
            None => ppem,
            Some(b) if (b as f32) < font_size => b.max(ppem),
            Some(b) if (ppem as f32) >= font_size => b.min(ppem),
            Some(b) => b,
        });
    }
    best
}

/// Resize premultiplied RGBA from `sw`x`sh` to `dw`x`dh` by area averaging:
/// each output pixel is the mean of the source area it covers (fractional
/// pixels weighted by how much of them it covers). Separable: rows, then
/// columns.
pub(crate) fn resample_area(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    // The source spans `[lo, hi)` of one axis that output pixel `i` covers,
    // as (index, weight) pairs summing to 1.
    fn taps(n_src: u32, n_dst: u32) -> Vec<Vec<(usize, f32)>> {
        let step = n_src as f32 / n_dst as f32;
        (0..n_dst)
            .map(|i| {
                let (lo, hi) = (i as f32 * step, (i as f32 + 1.0) * step);
                let mut out = Vec::new();
                let mut j = lo.floor() as u32;
                while (j as f32) < hi && j < n_src {
                    let w = (hi.min(j as f32 + 1.0) - lo.max(j as f32)).max(0.0);
                    if w > 0.0 {
                        out.push((j as usize, w / step));
                    }
                    j += 1;
                }
                out
            })
            .collect()
    }
    let (sw_u, dw_u) = (sw as usize, dw as usize);
    let xt = taps(sw, dw);
    let yt = taps(sh, dh);
    // Rows: sh x dw, as f32.
    let mut mid = vec![0f32; sh as usize * dw_u * 4];
    for y in 0..sh as usize {
        for (x, t) in xt.iter().enumerate() {
            for &(j, w) in t {
                let s = (y * sw_u + j) * 4;
                let d = (y * dw_u + x) * 4;
                for c in 0..4 {
                    mid[d + c] += src[s + c] as f32 * w;
                }
            }
        }
    }
    let mut out = vec![0u8; dh as usize * dw_u * 4];
    for (y, t) in yt.iter().enumerate() {
        for x in 0..dw_u {
            let mut acc = [0f32; 4];
            for &(j, w) in t {
                let s = (j * dw_u + x) * 4;
                for c in 0..4 {
                    acc[c] += mid[s + c] * w;
                }
            }
            let d = (y * dw_u + x) * 4;
            let a = acc[3].round().clamp(0.0, 255.0);
            out[d + 3] = a as u8;
            for c in 0..3 {
                // A premultiplied channel never exceeds its alpha.
                out[d + c] = acc[c].round().clamp(0.0, a) as u8;
            }
        }
    }
    out
}

/// #804: a colour bitmap strike is premultiplied before it is scaled, so the
/// colour under a transparent pixel never reaches the scaled edge, and the
/// area filter weighs each source pixel by how much of it an output covers.
#[cfg(test)]
mod strike_scale_tests {
    use super::{pick_strike, resample_area, scale_strike};

    /// The smallest strike at least the physical size, else the largest —
    /// in any table order (the bundled face has one strike, so only this
    /// pins the choice).
    #[test]
    fn the_strike_is_the_smallest_at_least_the_size_else_the_largest() {
        let strikes = [64u16, 160, 20, 109, 32];
        let pick = |size: f32| pick_strike(strikes.iter().copied(), size);
        assert_eq!(pick(16.0), Some(20));
        assert_eq!(pick(32.0), Some(32));
        assert_eq!(pick(40.0), Some(64));
        assert_eq!(pick(100.0), Some(109));
        assert_eq!(pick(120.0), Some(160));
        assert_eq!(pick(400.0), Some(160));
        assert_eq!(pick_strike(std::iter::empty(), 16.0), None);
    }

    /// Opaque green beside a transparent pixel whose (arbitrary) straight
    /// colour is red, halved: the result is green at half alpha. Scaled
    /// straight, half the red would bleed in.
    #[test]
    fn transparent_colour_does_not_bleed_into_the_edge() {
        let src = vec![0, 255, 0, 255, 255, 0, 0, 0];
        assert_eq!(scale_strike(src, 2, 1, 1, 1), vec![0, 128, 0, 128]);
    }

    /// A 3-to-2 shrink: the middle source pixel is split half and half
    /// between the two outputs (off the 2:1 fixed point where every source
    /// pixel goes whole to one output).
    #[test]
    fn a_fractional_shrink_splits_the_shared_pixel() {
        let px = |v: u8| [v, v, v, 255];
        let src: Vec<u8> = [px(0), px(150), px(255)].concat();
        // out0 = (0 + 150/2) / 1.5 = 50; out1 = (150/2 + 255) / 1.5 = 220.
        assert_eq!(resample_area(&src, 3, 1, 2, 1), [px(50), px(220)].concat());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use peniko::FontData;

    const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
    const EMOJI: &[u8] = include_bytes!("../../assets/fonts/NotoColorEmoji-U1F600.ttf");

    fn font(bytes: &'static [u8]) -> FontData {
        FontData::new(Blob::new(std::sync::Arc::new(bytes)), 0)
    }

    fn glyph_id(bytes: &[u8], ch: char) -> u32 {
        use swash::FontRef;
        FontRef::from_index(bytes, 0).unwrap().charmap().map(ch) as u32
    }

    fn spec(f: &FontData, size: f32) -> RunSpec<'_> {
        RunSpec {
            font: f,
            size,
            hint: true,
            embolden: 0.0,
            coords: &[],
        }
    }

    /// The left edge column of the mask's ink: coverage summed per column,
    /// as an x position in pixels relative to the glyph's whole-pixel origin
    /// (the ink's centroid), so a quarter-pixel shift is visible.
    fn ink_centroid(m: &GlyphMask) -> f64 {
        let (mut sum, mut wsum) = (0.0, 0.0);
        for y in 0..m.height {
            for x in 0..m.width {
                let c = m.data[(y * m.width + x) as usize] as f64;
                sum += c * (x as f64 + m.left as f64 + 0.5);
                wsum += c;
            }
        }
        sum / wsum
    }

    /// Off the grid's fixed points: `x.3` is bucket 1, `x.6` bucket 2, and
    /// `x.9` rounds up to the next pixel's bucket 0 rather than being clamped
    /// to bucket 3. The baseline is rounded half up. Negative positions floor.
    #[test]
    fn placement_buckets_and_rows() {
        assert_eq!(
            place(10.3, 20.4),
            Placement {
                x: 10,
                y: 20,
                bucket: 1
            }
        );
        assert_eq!(
            place(10.6, 20.5),
            Placement {
                x: 10,
                y: 21,
                bucket: 2
            }
        );
        assert_eq!(
            place(10.9, 20.0),
            Placement {
                x: 11,
                y: 20,
                bucket: 0
            }
        );
        assert_eq!(
            place(10.74, 0.0),
            Placement {
                x: 10,
                y: 0,
                bucket: 3
            }
        );
        assert_eq!(
            place(-0.3, -0.6),
            Placement {
                x: -1,
                y: -1,
                bucket: 3
            }
        );
    }

    #[test]
    fn transforms_the_atlas_draws_under() {
        assert_eq!(atlas_scale(Affine::translate((3.5, 7.25))), Some(1.0));
        assert_eq!(atlas_scale(Affine::scale(2.0)), Some(2.0));
        assert_eq!(atlas_scale(Affine::scale_non_uniform(2.0, 1.0)), None);
        assert_eq!(atlas_scale(Affine::rotate(0.1)), None);
        assert_eq!(atlas_scale(Affine::skew(0.2, 0.0)), None);
        assert_eq!(atlas_scale(Affine::scale(-1.0)), None);
    }

    /// A miss rasterises and keeps; the same glyph and bucket again is a hit;
    /// another bucket is another mask, its ink shifted by the bucket's
    /// fraction of a pixel.
    #[test]
    fn buckets_are_separate_masks_shifted_by_their_fraction() {
        let f = font(INTER);
        let l = glyph_id(INTER, 'l');
        let mut atlas = GlyphAtlas::default();
        let mut stats = AtlasStats::default();
        let mut centroids = [0.0; 4];
        for b in 0..4u8 {
            atlas.lookup(&spec(&f, 13.0), &[(l, b)], false, &mut stats, |_, m| {
                centroids[b as usize] = ink_centroid(m.expect("an inked glyph"));
            });
        }
        assert_eq!((stats.misses, stats.hits), (4, 0));
        atlas.lookup(
            &spec(&f, 13.0),
            &[(l, 2), (l, 2)],
            false,
            &mut stats,
            |_, _| {},
        );
        assert_eq!((stats.misses, stats.hits), (4, 2));
        for b in 1..4 {
            let shift = centroids[b] - centroids[0];
            assert!(
                (shift - b as f64 * 0.25).abs() < 0.12,
                "bucket {b}: ink moved {shift} px, not {}",
                b as f64 * 0.25
            );
        }
        // A space has no pixels: cached as empty, a hit the second time.
        let space = glyph_id(INTER, ' ');
        let mut seen = Vec::new();
        atlas.lookup(
            &spec(&f, 13.0),
            &[(space, 0), (space, 0)],
            false,
            &mut stats,
            |_, m| seen.push(m.is_none()),
        );
        assert_eq!(seen, [true, true]);
        assert_eq!((stats.misses, stats.hits), (5, 3));
    }

    /// The GPU copy is made only when asked, once, with the coverage in all
    /// four channels (premultiplied white: the shader reads the alpha), and
    /// counted as an upload; its bytes are accounted.
    #[test]
    fn the_gpu_image_is_the_coverage_in_every_channel() {
        let f = font(INTER);
        let a = glyph_id(INTER, 'a');
        let mut atlas = GlyphAtlas::default();
        let mut stats = AtlasStats::default();
        atlas.lookup(&spec(&f, 13.0), &[(a, 1)], false, &mut stats, |_, m| {
            assert!(m.unwrap().gpu_image().is_none());
        });
        let before = atlas.bytes();
        let mut checked = false;
        atlas.lookup(
            &spec(&f, 13.0),
            &[(a, 1), (a, 1)],
            true,
            &mut stats,
            |_, m| {
                let m = m.unwrap();
                let img = m.gpu_image().expect("made for a GPU draw");
                assert_eq!((img.width, img.height), (m.width, m.height));
                assert_eq!(img.alpha_type, ImageAlphaType::AlphaPremultiplied);
                let px: &[u8] = img.data.as_ref();
                for (i, &c) in m.data.iter().enumerate() {
                    assert_eq!(px[i * 4..i * 4 + 4], [c, c, c, c]);
                }
                checked = true;
            },
        );
        assert!(checked);
        assert_eq!(stats.uploads, 1, "one upload, not one per draw");
        assert_eq!(
            atlas.bytes() - before,
            atlas_mask_len(&mut atlas, &f, a) * 4
        );
    }

    fn atlas_mask_len(atlas: &mut GlyphAtlas, f: &FontData, id: u32) -> usize {
        let mut n = 0;
        atlas.lookup(
            &spec(f, 13.0),
            &[(id, 1)],
            false,
            &mut AtlasStats::default(),
            |_, m| n = m.unwrap().data.len(),
        );
        n
    }

    /// A colour glyph (the bundled emoji face's CBDT strike) is a colour
    /// mask, premultiplied, and gets no GPU copy: Vello draws colour glyphs
    /// itself.
    #[test]
    fn a_colour_glyph_is_kept_as_colour() {
        let f = font(EMOJI);
        let id = glyph_id(EMOJI, '\u{1F600}');
        assert_ne!(id, 0);
        let mut atlas = GlyphAtlas::default();
        let mut stats = AtlasStats::default();
        let mut kind = None;
        atlas.lookup(&spec(&f, 20.0), &[(id, 0)], true, &mut stats, |_, m| {
            let m = m.expect("an emoji has pixels");
            kind = Some(m.kind);
            assert!(m.gpu_image().is_none());
            assert_eq!(m.data.len(), (m.width * m.height * 4) as usize);
        });
        assert_eq!(
            kind,
            Some(MaskKind::Color {
                premultiplied: true
            })
        );
        assert_eq!(stats.uploads, 0);
    }

    /// Past the budget the entries drawn longest ago go first, down to three
    /// quarters of it: of glyphs drawn in generations 1..=n, the newest stay.
    /// Red against an eviction that drops newest-first or clears everything.
    #[test]
    fn eviction_drops_the_least_recently_drawn() {
        let f = font(INTER);
        let ids: Vec<u32> = "abcdefghijklmnop"
            .chars()
            .map(|c| glyph_id(INTER, c))
            .collect();
        let mut atlas = GlyphAtlas::default();
        let mut stats = AtlasStats::default();
        // One glyph a generation.
        for &id in &ids {
            atlas.begin_frame();
            atlas.lookup(&spec(&f, 24.0), &[(id, 0)], false, &mut stats, |_, _| {});
        }
        let total = atlas.bytes();
        // Touch the oldest glyph again: it is now the newest.
        atlas.begin_frame();
        atlas.lookup(
            &spec(&f, 24.0),
            &[(ids[0], 0)],
            false,
            &mut stats,
            |_, _| {},
        );
        assert_eq!(stats.evictions, 0);
        // Shrink the budget so about half must go, and trigger it with a hit.
        atlas.set_budget(total / 2);
        atlas.begin_frame();
        atlas.lookup(
            &spec(&f, 24.0),
            &[(ids[0], 0)],
            false,
            &mut stats,
            |_, _| {},
        );
        assert!(
            atlas.bytes() <= total / 2 / 4 * 3,
            "{} > 3/4 of the budget",
            atlas.bytes()
        );
        assert!(stats.evictions > 0);
        // Which are left: ids[0] (just drawn) and a suffix of the rest.
        let kept: Vec<usize> = (0..ids.len())
            .filter(|&i| atlas.holds(&spec(&f, 24.0), ids[i], 0))
            .collect();
        assert!(kept.contains(&0), "the glyph drawn last stays: {kept:?}");
        let rest: Vec<usize> = kept.iter().copied().filter(|&i| i != 0).collect();
        assert!(!rest.is_empty(), "something besides ids[0] stays: {kept:?}");
        let first = rest[0];
        assert!(
            rest == (first..ids.len()).collect::<Vec<_>>(),
            "the newest survive, the oldest go: kept {kept:?}"
        );
        assert!(first > 1, "the oldest went: kept {kept:?}");
    }

    /// One frame drawing more than the whole budget still draws every glyph
    /// (each `draw` sees its mask), and the atlas is back under budget after.
    #[test]
    fn a_frame_bigger_than_the_budget_still_draws() {
        let f = font(INTER);
        let ids: Vec<(u32, u8)> = "abcdefghijklmnopqrstuvwxyz"
            .chars()
            .map(|c| (glyph_id(INTER, c), 0))
            .collect();
        let mut atlas = GlyphAtlas::with_budget(4096);
        let mut stats = AtlasStats::default();
        let mut drawn = 0;
        atlas.lookup(&spec(&f, 40.0), &ids, false, &mut stats, |_, m| {
            drawn += m.is_some() as usize
        });
        assert_eq!(drawn, ids.len());
        assert!(atlas.bytes() <= 4096, "{}", atlas.bytes());
        assert!(stats.evictions > 0);
    }

    /// The device size, the emboldening and the hinting are in the key.
    #[test]
    fn size_embolden_and_hinting_are_keys() {
        let f = font(INTER);
        let a = glyph_id(INTER, 'a');
        let mut atlas = GlyphAtlas::default();
        let mut stats = AtlasStats::default();
        let mut widths = Vec::new();
        for (size, embolden, hint) in [
            (13.0, 0.0, true),
            (26.0, 0.0, true),
            (13.0, 0.5, true),
            (13.0, 0.0, false),
        ] {
            let s = RunSpec {
                font: &f,
                size,
                hint,
                embolden,
                coords: &[],
            };
            atlas.lookup(&s, &[(a, 0)], false, &mut stats, |_, m| {
                let m = m.unwrap();
                widths.push((m.width, m.data.iter().map(|&c| c as u64).sum::<u64>()));
            });
        }
        assert_eq!(stats.misses, 4, "four runs, four masks");
        assert!(
            widths[1].0 > widths[0].0 * 3 / 2,
            "twice the size: {widths:?}"
        );
        assert!(
            widths[2].1 > widths[0].1,
            "emboldened has more ink: {widths:?}"
        );
    }
}
