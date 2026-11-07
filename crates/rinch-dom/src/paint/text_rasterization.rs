//! How glyphs are rasterised: the app's text-rendering choice, the OS's text
//! settings it defaults to, and the policy both painters read (#2286).
//!
//! An app states what it wants in a [`TextRasterization`]; every field defaults to
//! `Auto`, which follows the OS as Chrome does. The OS's own settings are a
//! [`SystemTextSettings`] (fontconfig on Linux, `SystemParametersInfo` on
//! Windows, fixed on macOS; the facade crate reads them). The two resolve into
//! a [`ResolvedTextRasterization`], which is what a painter consults per glyph run.
//!
//! Painters use three things out of it:
//! - **hinting** — `hint` is off when the resolved hinting is
//!   [`TextHinting::None`];
//! - **stem darkening of light text** — [`ResolvedTextRasterization::darkening_for`],
//!   applied by the Vello painter through `DrawGlyphs::font_embolden` and by
//!   the tiny-skia painter through swash's embolden, so both thicken the same
//!   glyphs by the same amount;
//! - the antialiasing choice: [`ResolvedAntialias::Subpixel`] carries the
//!   subpixel order and LCD filter both painters draw subpixel (LCD) text with
//!   (stage 3, [`ResolvedTextRasterization::subpixel`]), where they can
//!   composite it; [`ResolvedAntialias::None`] (fontconfig `antialias=false`,
//!   Windows font smoothing off) has no aliased path and draws grayscale.
//!
//! The resolved policy is per thread ([`set_thread_policy`] /
//! [`thread_policy`]): the runtime paints on the thread that runs its event
//! loop, and a test that changes it changes only its own thread. A thread that
//! never set one asks the source installed with [`set_policy_source`]
//! once (the facade installs one that reads the OS), and without a source
//! takes [`ResolvedTextRasterization::default`].

use std::cell::Cell;
use std::sync::OnceLock;

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::Affine;

/// The app's text-rendering choice: what [`App::text_rasterization`] takes.
///
/// Every field is `Auto` by default, which follows the OS (see
/// [`SystemTextSettings`]) and, where the OS says nothing, Chrome. The struct
/// is `#[non_exhaustive]` so later stages can add fields (subpixel order, LCD
/// filter, contrast); build one with [`Default`] and the setters:
///
/// ```
/// use rinch_dom::paint::text_rasterization::{StemDarkening, TextHinting, TextRasterization};
/// let r = TextRasterization::default()
///     .hinting(TextHinting::None)
///     .stem_darkening(StemDarkening::Off);
/// assert_eq!(r.hinting, TextHinting::None);
/// ```
///
/// [`App::text_rasterization`]: https://docs.rs/rinch/latest/rinch/app_builder/struct.App.html#method.text_rasterization
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextRasterization {
    /// Grayscale, subpixel (LCD) or no antialiasing. See [`TextAntialias`].
    pub antialias: TextAntialias,
    /// How hard outlines are fitted to the pixel grid. See [`TextHinting`].
    pub hinting: TextHinting,
    /// Thickening of light text. See [`StemDarkening`].
    pub stem_darkening: StemDarkening,
    /// Draw every glyph from its outline on the GPU, not from the glyph-mask
    /// atlas (#2286 stage 2). Off by default; for text whose size changes
    /// every frame (an animated zoom), where each size is a new set of masks.
    /// The software renderer always draws masks.
    pub gpu_glyph_outlines: bool,
    /// The panel's subpixel order for subpixel text, over the OS's (#2286
    /// stage 3): for a display the OS reports wrongly or not at all, such as
    /// a BGR panel or one rotated to portrait (`VRgb`). `None` follows the
    /// OS. [`SubpixelOrder::None`] turns subpixel text off, as fontconfig's
    /// `rgba=none` does.
    pub subpixel_order: Option<SubpixelOrder>,
    /// The filter across subpixels, over the OS's (fontconfig `lcdfilter`).
    /// `None` follows the OS, and FreeType's default five-tap filter where it
    /// says nothing.
    pub lcd_filter: Option<LcdFilter>,
}

impl TextRasterization {
    /// Set [`antialias`](Self::antialias).
    #[must_use]
    pub fn antialias(mut self, antialias: TextAntialias) -> Self {
        self.antialias = antialias;
        self
    }

    /// Set [`hinting`](Self::hinting).
    #[must_use]
    pub fn hinting(mut self, hinting: TextHinting) -> Self {
        self.hinting = hinting;
        self
    }

    /// Set [`stem_darkening`](Self::stem_darkening).
    #[must_use]
    pub fn stem_darkening(mut self, stem_darkening: StemDarkening) -> Self {
        self.stem_darkening = stem_darkening;
        self
    }

    /// Set [`gpu_glyph_outlines`](Self::gpu_glyph_outlines).
    #[must_use]
    pub fn gpu_glyph_outlines(mut self, on: bool) -> Self {
        self.gpu_glyph_outlines = on;
        self
    }

    /// Set [`subpixel_order`](Self::subpixel_order).
    #[must_use]
    pub fn subpixel_order(mut self, order: Option<SubpixelOrder>) -> Self {
        self.subpixel_order = order;
        self
    }

    /// Set [`lcd_filter`](Self::lcd_filter).
    #[must_use]
    pub fn lcd_filter(mut self, filter: Option<LcdFilter>) -> Self {
        self.lcd_filter = filter;
        self
    }

    /// Resolve every `Auto` against the OS's settings.
    pub fn resolve(&self, system: &SystemTextSettings) -> ResolvedTextRasterization {
        let filter = self
            .lcd_filter
            .or(system.lcd_filter)
            .unwrap_or(LcdFilter::Default);
        // The app's order over the OS's.
        let order = self.subpixel_order.or(system.subpixel_order);
        let antialias = match self.antialias {
            TextAntialias::Grayscale => ResolvedAntialias::Grayscale,
            TextAntialias::None => ResolvedAntialias::None,
            // Never where the OS cannot show it (macOS), whatever the app asks.
            TextAntialias::Subpixel if system.subpixel_capable == Some(false) => {
                ResolvedAntialias::Grayscale
            }
            TextAntialias::Subpixel => match self.subpixel_order {
                // The app turned the order off: grey.
                Some(SubpixelOrder::None) => ResolvedAntialias::Grayscale,
                // Asked for without the OS naming an order: RGB, the order of
                // nearly every desktop panel and Chrome's default.
                _ => ResolvedAntialias::Subpixel {
                    order: match order {
                        Some(SubpixelOrder::None) | None => SubpixelOrder::Rgb,
                        Some(order) => order,
                    },
                    filter,
                },
            },
            TextAntialias::Auto => match (system.antialias, order) {
                (Some(false), _) => ResolvedAntialias::None,
                _ if system.subpixel_capable == Some(false) => ResolvedAntialias::Grayscale,
                (_, Some(order)) if order != SubpixelOrder::None => {
                    ResolvedAntialias::Subpixel { order, filter }
                }
                _ => ResolvedAntialias::Grayscale,
            },
        };
        let hinting = match self.hinting {
            TextHinting::Auto => system.hinting.unwrap_or(TextHinting::Slight),
            other => other,
        };
        let darkening_px = match self.stem_darkening {
            StemDarkening::Auto => DEFAULT_STEM_DARKENING_PX,
            StemDarkening::Off => 0.0,
            StemDarkening::Px(px) if px.is_finite() => px.clamp(0.0, MAX_STEM_DARKENING_PX),
            StemDarkening::Px(_) => 0.0,
        };
        ResolvedTextRasterization {
            antialias,
            hinting,
            darkening_px,
            gpu_glyph_outlines: self.gpu_glyph_outlines,
        }
    }
}

/// How glyph edges are antialiased.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextAntialias {
    /// Follow the OS: no antialiasing where it turns it off, subpixel where it
    /// names a subpixel order, grayscale otherwise (macOS is always grayscale).
    #[default]
    Auto,
    /// One coverage value per pixel.
    Grayscale,
    /// Per-channel (LCD) coverage, in the OS's subpixel order (RGB where it
    /// names none) unless [`TextRasterization::subpixel_order`] says
    /// otherwise. Never on macOS. Painters still draw grey text where
    /// subpixel text cannot be composited: inside an opacity, blend or
    /// isolated layer, over a pixel that is not opaque, under a rotation,
    /// skew or uneven scale, while a run's size animates, and for colour
    /// glyphs.
    Subpixel,
    /// No antialiasing. Recorded; both painters draw grayscale (there is no
    /// aliased path).
    None,
}

/// How hard glyph outlines are fitted to the pixel grid.
///
/// Both painters have one hinted mode (skrifa's and swash's interpreters, each
/// in its smooth/light flavour), so `Slight`, `Medium` and `Full` all hint and
/// `None` does not. The levels are kept apart for the OS's own value and for a
/// later stage that can tell them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextHinting {
    /// Follow the OS (fontconfig `hintstyle`); `Slight` where it says nothing,
    /// and `None` on macOS, which does not hint.
    #[default]
    Auto,
    /// No hinting: outlines at their exact scaled positions.
    None,
    /// Vertical-only fitting (fontconfig `hintslight`).
    Slight,
    /// fontconfig `hintmedium`.
    Medium,
    /// fontconfig `hintfull`.
    Full,
}

/// Stem darkening: how much light text is thickened.
///
/// Painters blend glyph coverage in sRGB-encoded space, as FreeType's callers,
/// Vello and tiny-skia all do, and that makes light-on-dark text look about a
/// quarter lighter in weight than the same text dark-on-light (#2286's
/// measurements: 0.73–0.79 of the linear-light weight, against 1.21–1.27).
/// Darkening grows each glyph's outline by a fraction of a pixel per side, for
/// light text only, which brings it to about 0.93.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum StemDarkening {
    /// The default: up to [`DEFAULT_STEM_DARKENING_PX`] per side.
    #[default]
    Auto,
    /// Never thicken.
    Off,
    /// Up to this many device pixels per side (clamped to
    /// `0..=`[`MAX_STEM_DARKENING_PX`]); the luminance and size taper of
    /// [`ResolvedTextRasterization::darkening_for`] still apply.
    Px(f32),
}

/// [`StemDarkening::Auto`]'s amount: 0.10 device pixels per side, at which
/// #2286 measured light-on-dark text at 0.93 of its linear-light weight.
pub const DEFAULT_STEM_DARKENING_PX: f32 = 0.10;

/// The largest amount [`StemDarkening::Px`] takes.
pub const MAX_STEM_DARKENING_PX: f32 = 0.5;

/// The order of a panel's subpixels, for subpixel (LCD) antialiasing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubpixelOrder {
    /// The OS turns subpixel rendering off (fontconfig `rgba=none`).
    None,
    /// Horizontal red, green, blue.
    Rgb,
    /// Horizontal blue, green, red.
    Bgr,
    /// Vertical red, green, blue (top to bottom).
    VRgb,
    /// Vertical blue, green, red.
    VBgr,
}

/// The filter applied across subpixels (fontconfig `lcdfilter`), FreeType's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LcdFilter {
    /// No filter.
    None,
    /// FreeType's default five-tap filter (`lcddefault`).
    Default,
    /// The lighter filter (`lcdlight`).
    Light,
    /// The legacy filter (`lcdlegacy`).
    Legacy,
}

/// What the OS says about text rendering. A `None` field is one it does not
/// say (or could not be read); `Auto` falls back to Chrome's choice for it.
///
/// `#[non_exhaustive]`, like the other policy types here, so later stages can
/// add fields and variants: outside this crate it is built with `default()`
/// and the `with_` setters, and matches on the enums need a `_` arm.
///
/// ```compile_fail
/// use rinch_dom::paint::text_rasterization::SystemTextSettings;
/// let s = SystemTextSettings {
///     antialias: None,
///     hinting: None,
///     subpixel_order: None,
///     lcd_filter: None,
/// };
/// ```
///
/// ```compile_fail
/// use rinch_dom::paint::text_rasterization::TextHinting;
/// fn hints(h: TextHinting) -> bool {
///     match h {
///         TextHinting::Auto | TextHinting::Slight | TextHinting::Medium | TextHinting::Full => true,
///         TextHinting::None => false,
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SystemTextSettings {
    /// Whether text is antialiased at all (fontconfig `antialias`, Windows
    /// `SPI_GETFONTSMOOTHING`).
    pub antialias: Option<bool>,
    /// fontconfig `hintstyle` (`hintnone` .. `hintfull`); macOS: `None`.
    pub hinting: Option<TextHinting>,
    /// fontconfig `rgba`, Windows ClearType's orientation; macOS:
    /// [`SubpixelOrder::None`].
    pub subpixel_order: Option<SubpixelOrder>,
    /// fontconfig `lcdfilter`.
    pub lcd_filter: Option<LcdFilter>,
    /// Whether the OS's text can be subpixel text at all: `Some(false)` on
    /// macOS, which has drawn none since 10.14, so neither `Auto` nor an
    /// app's [`TextAntialias::Subpixel`] turns it on there.
    pub subpixel_capable: Option<bool>,
}

// `#[non_exhaustive]`, so built with `default()` and these.
impl SystemTextSettings {
    /// Set [`antialias`](Self::antialias).
    #[must_use]
    pub fn with_antialias(mut self, v: Option<bool>) -> Self {
        self.antialias = v;
        self
    }

    /// Set [`hinting`](Self::hinting).
    #[must_use]
    pub fn with_hinting(mut self, v: Option<TextHinting>) -> Self {
        self.hinting = v;
        self
    }

    /// Set [`subpixel_order`](Self::subpixel_order).
    #[must_use]
    pub fn with_subpixel_order(mut self, v: Option<SubpixelOrder>) -> Self {
        self.subpixel_order = v;
        self
    }

    /// Set [`lcd_filter`](Self::lcd_filter).
    #[must_use]
    pub fn with_lcd_filter(mut self, v: Option<LcdFilter>) -> Self {
        self.lcd_filter = v;
        self
    }

    /// Set [`subpixel_capable`](Self::subpixel_capable).
    #[must_use]
    pub fn with_subpixel_capable(mut self, v: Option<bool>) -> Self {
        self.subpixel_capable = v;
        self
    }
}

/// The antialiasing [`TextRasterization::antialias`] resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolvedAntialias {
    /// One coverage value per pixel.
    Grayscale,
    /// Per-channel coverage in this order through this filter (#2286 stage
    /// 3), where the painters can composite it.
    Subpixel {
        /// The panel's subpixel order.
        order: SubpixelOrder,
        /// The filter across subpixels.
        filter: LcdFilter,
    },
    /// No antialiasing (drawn grayscale).
    None,
}

/// A [`TextRasterization`] with every `Auto` resolved: what painters read.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResolvedTextRasterization {
    /// The antialiasing.
    pub antialias: ResolvedAntialias,
    /// The hinting.
    pub hinting: TextHinting,
    /// The most stem darkening, in device pixels per side, that the lightest
    /// small text gets; `0` turns it off.
    pub darkening_px: f32,
    /// The GPU painter draws glyphs from outlines, not the glyph-mask atlas.
    pub gpu_glyph_outlines: bool,
}

impl Default for ResolvedTextRasterization {
    /// [`TextRasterization::default`] with nothing known about the OS: grayscale,
    /// hinted, darkening on.
    fn default() -> Self {
        TextRasterization::default().resolve(&SystemTextSettings::default())
    }
}

/// The device font size up to which light text gets the whole darkening.
const DARKENING_FULL_BELOW_PX: f32 = 24.0;
/// The device font size from which light text gets none.
const DARKENING_NONE_FROM_PX: f32 = 64.0;
/// Below this luma (sRGB-encoded, over black) text is not "light".
const LIGHT_LUMA_FROM: f32 = 0.5;
/// From this luma text is fully "light".
const LIGHT_LUMA_FULL: f32 = 0.85;
/// The amounts are quantised to this many steps per pixel, so Vello's glyph
/// cache and the software glyph cache see a handful of distinct values.
const DARKENING_STEPS_PER_PX: f32 = 64.0;

impl ResolvedTextRasterization {
    /// This policy with `darkening_px` replaced (for tests and tools).
    #[must_use]
    pub fn with_darkening(mut self, px: f32) -> Self {
        self.darkening_px = px;
        self
    }

    /// The subpixel order and LCD filter text is rasterised with, when the
    /// antialiasing is subpixel (#2286 stage 3); `None` for grey text.
    /// Painters draw grey text anyway where subpixel text cannot be
    /// composited (inside layers, off the device grid, colour glyphs).
    #[inline]
    pub fn subpixel(&self) -> Option<(SubpixelOrder, LcdFilter)> {
        match self.antialias {
            ResolvedAntialias::Subpixel { order, filter } if order != SubpixelOrder::None => {
                Some((order, filter))
            }
            _ => None,
        }
    }

    /// [`Self::subpixel`] as the glyph-mask atlas's key.
    #[cfg(feature = "glyph-atlas")]
    #[inline]
    pub fn lcd_mode(&self) -> Option<super::glyph_atlas::LcdMode> {
        self.subpixel()
            .map(|(order, filter)| super::glyph_atlas::LcdMode { order, filter })
    }

    /// Whether outlines are hinted.
    #[inline]
    pub fn hint(&self) -> bool {
        self.hinting != TextHinting::None
    }

    /// The stem darkening for a glyph run in `color` at `font_size` device
    /// pixels per em, in device pixels per side (`0` for none).
    ///
    /// Only light text is darkened, judged by the text colour alone, as Skia
    /// keys its contrast curve: paint does not know what is under the text
    /// (layers, images, gradients, transparent windows), and light text is
    /// what sRGB-encoded blending thins. "Light" is the colour's
    /// sRGB-encoded luma composited over black (so translucent light text
    /// counts as dimmer), ramping from nothing at 0.5 to the whole amount at
    /// 0.85. The amount tapers off with size, whole up to 24 device pixels per
    /// em and none from 64: a large glyph's stems are wide enough that the
    /// thinning is not seen. The result is quantised to 1/64 px.
    ///
    /// `font_size` is the size in **device** pixels; for a run drawn under a
    /// transform, use [`run_darkening_px`](Self::run_darkening_px).
    #[inline]
    pub fn darkening_for(&self, color: AlphaColor<Srgb>, font_size: f32) -> f32 {
        if self.darkening_px <= 0.0 {
            return 0.0;
        }
        let [r, g, b, a] = color.components;
        let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) * a.clamp(0.0, 1.0);
        if luma <= LIGHT_LUMA_FROM {
            return 0.0;
        }
        let light = ((luma - LIGHT_LUMA_FROM) / (LIGHT_LUMA_FULL - LIGHT_LUMA_FROM)).min(1.0);
        let taper = ((DARKENING_NONE_FROM_PX - font_size)
            / (DARKENING_NONE_FROM_PX - DARKENING_FULL_BELOW_PX))
            .clamp(0.0, 1.0);
        let px = self.darkening_px * light * taper;
        (px * DARKENING_STEPS_PER_PX).round() / DARKENING_STEPS_PER_PX
    }

    /// The stem darkening of a glyph run, in **device** pixels per side: a
    /// solid `brush` only, judged at the run's device size, `font_size` times
    /// the scale of `transform` ([`device_scale`]). Each painter turns it into
    /// the amount its outline space needs, so both grow a glyph by the same
    /// device amount under any transform.
    #[inline]
    pub fn run_darkening_px(
        &self,
        brush: &peniko::Brush,
        font_size: f32,
        transform: Affine,
    ) -> f32 {
        match brush {
            peniko::Brush::Solid(color) => {
                self.darkening_for(*color, font_size * device_scale(transform) as f32)
            }
            _ => 0.0,
        }
    }
}

/// How much `transform` scales lengths: the square root of its linear part's
/// determinant (`1` for a translation or a rotation, `2` for `scale(2)`). A degenerate transform reads as `0`.
#[inline]
pub fn device_scale(transform: Affine) -> f64 {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    (a * d - b * c).abs().sqrt()
}

thread_local! {
    static CURRENT: Cell<Option<ResolvedTextRasterization>> = const { Cell::new(None) };
}

static SOURCE: OnceLock<fn() -> ResolvedTextRasterization> = OnceLock::new();

/// Install the function a thread asks for its policy the first time it paints
/// text without having [set](set_thread_policy) one. The first install wins.
/// The facade installs one that resolves the app's [`TextRasterization`] against
/// the OS's settings.
pub fn set_policy_source(source: fn() -> ResolvedTextRasterization) {
    let _ = SOURCE.set(source);
}

/// Set this thread's text-rendering policy. Glyph runs painted on this thread
/// afterwards use it.
pub fn set_thread_policy(policy: ResolvedTextRasterization) {
    CURRENT.with(|c| c.set(Some(policy)));
}

/// Forget this thread's policy, so the next glyph run asks the source again.
pub fn reset_thread_policy() {
    CURRENT.with(|c| c.set(None));
}

/// This thread's text-rendering policy.
#[inline]
pub fn thread_policy() -> ResolvedTextRasterization {
    CURRENT.with(|c| match c.get() {
        Some(policy) => policy,
        None => {
            let policy = SOURCE
                .get()
                .map_or_else(ResolvedTextRasterization::default, |f| f());
            c.set(Some(policy));
            policy
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> AlphaColor<Srgb> {
        AlphaColor::from_rgba8(r, g, b, 255)
    }

    #[test]
    fn light_small_text_gets_the_whole_darkening() {
        let p = ResolvedTextRasterization::default();
        assert_eq!(p.darkening_for(rgb(0xfc, 0xfc, 0xfc), 13.0), 0.09375);
        // 0.10 quantised to 1/64: 6.4 steps round to 6.
        assert_eq!(p.darkening_for(rgb(0xff, 0xff, 0xff), 24.0), 6.0 / 64.0);
    }

    #[test]
    fn dark_and_mid_text_gets_none() {
        let p = ResolvedTextRasterization::default();
        assert_eq!(p.darkening_for(rgb(0x23, 0x26, 0x29), 13.0), 0.0);
        assert_eq!(p.darkening_for(rgb(0x7f, 0x7f, 0x7f), 13.0), 0.0);
    }

    #[test]
    fn darkening_ramps_with_luma_and_tapers_with_size() {
        let p = ResolvedTextRasterization::default().with_darkening(0.32);
        // #b0b0b0: luma 0.690 -> light 0.543 -> 0.1737 px -> 11/64.
        assert_eq!(p.darkening_for(rgb(0xb0, 0xb0, 0xb0), 13.0), 11.0 / 64.0);
        // White at 44 px: taper 0.5 -> 0.16 px -> 10/64 (10.24 steps).
        assert_eq!(p.darkening_for(rgb(0xff, 0xff, 0xff), 44.0), 10.0 / 64.0);
        assert_eq!(p.darkening_for(rgb(0xff, 0xff, 0xff), 64.0), 0.0);
        // Half-transparent white reads as luma 0.5: none.
        let half = AlphaColor::from_rgba8(255, 255, 255, 128);
        assert_eq!(p.darkening_for(half, 13.0), 0.0);
    }

    #[test]
    fn off_turns_it_off() {
        let p = TextRasterization::default()
            .stem_darkening(StemDarkening::Off)
            .resolve(&SystemTextSettings::default());
        assert_eq!(p.darkening_for(rgb(0xff, 0xff, 0xff), 13.0), 0.0);
        let p = TextRasterization::default()
            .stem_darkening(StemDarkening::Px(f32::NAN))
            .resolve(&SystemTextSettings::default());
        assert_eq!(p.darkening_px, 0.0);
        let p = TextRasterization::default()
            .stem_darkening(StemDarkening::Px(9.0))
            .resolve(&SystemTextSettings::default());
        assert_eq!(p.darkening_px, MAX_STEM_DARKENING_PX);
    }

    #[test]
    fn auto_follows_the_os() {
        let kde = SystemTextSettings {
            antialias: Some(true),
            hinting: Some(TextHinting::Slight),
            subpixel_order: Some(SubpixelOrder::Rgb),
            lcd_filter: Some(LcdFilter::Default),
            subpixel_capable: None,
        };
        let r = TextRasterization::default().resolve(&kde);
        assert_eq!(
            r.antialias,
            ResolvedAntialias::Subpixel {
                order: SubpixelOrder::Rgb,
                filter: LcdFilter::Default
            }
        );
        assert!(r.hint());
        let none = SystemTextSettings {
            hinting: Some(TextHinting::None),
            antialias: Some(false),
            subpixel_order: Some(SubpixelOrder::Rgb),
            ..Default::default()
        };
        let r = TextRasterization::default().resolve(&none);
        assert_eq!(r.antialias, ResolvedAntialias::None);
        assert!(!r.hint());
        // `rgba=none` is grayscale.
        let gray = SystemTextSettings {
            subpixel_order: Some(SubpixelOrder::None),
            ..Default::default()
        };
        let r = TextRasterization::default().resolve(&gray);
        assert_eq!(r.antialias, ResolvedAntialias::Grayscale);
        // Nothing known: grayscale, hinted.
        let r = ResolvedTextRasterization::default();
        assert_eq!(r.antialias, ResolvedAntialias::Grayscale);
        assert_eq!(r.hinting, TextHinting::Slight);
    }

    #[test]
    fn an_app_choice_overrides_the_os() {
        let kde = SystemTextSettings {
            antialias: Some(true),
            hinting: Some(TextHinting::Slight),
            subpixel_order: Some(SubpixelOrder::Bgr),
            lcd_filter: Some(LcdFilter::Light),
            subpixel_capable: None,
        };
        let r = TextRasterization::default()
            .antialias(TextAntialias::Grayscale)
            .hinting(TextHinting::None)
            .resolve(&kde);
        assert_eq!(r.antialias, ResolvedAntialias::Grayscale);
        assert!(!r.hint());
        let r = TextRasterization::default()
            .antialias(TextAntialias::Subpixel)
            .resolve(&SystemTextSettings::default());
        assert_eq!(
            r.antialias,
            ResolvedAntialias::Subpixel {
                order: SubpixelOrder::Rgb,
                filter: LcdFilter::Default
            }
        );
    }

    /// macOS draws no subpixel text, by policy: neither `Auto` nor an app's
    /// `Subpixel` turns it on there, whatever order is named.
    #[test]
    fn never_subpixel_on_macos() {
        let mac = SystemTextSettings::default()
            .with_antialias(Some(true))
            .with_hinting(Some(TextHinting::None))
            .with_subpixel_order(Some(SubpixelOrder::None))
            .with_subpixel_capable(Some(false));
        for app in [
            TextRasterization::default(),
            TextRasterization::default().antialias(TextAntialias::Subpixel),
            TextRasterization::default()
                .antialias(TextAntialias::Subpixel)
                .subpixel_order(Some(SubpixelOrder::Rgb)),
            TextRasterization::default().subpixel_order(Some(SubpixelOrder::Bgr)),
        ] {
            let r = app.resolve(&mac);
            assert_eq!(r.antialias, ResolvedAntialias::Grayscale, "{app:?}");
            assert_eq!(r.subpixel(), None);
        }
    }

    /// The app's subpixel order and filter override the OS's (a BGR or
    /// rotated panel the OS reports wrongly); `SubpixelOrder::None` turns
    /// subpixel text off. Joe's KDE (`rgba=1`, `lcddefault`) is RGB through
    /// the default filter.
    #[test]
    fn the_app_overrides_order_and_filter() {
        let kde = SystemTextSettings::default()
            .with_antialias(Some(true))
            .with_subpixel_order(Some(SubpixelOrder::Rgb))
            .with_lcd_filter(Some(LcdFilter::Default));
        let r = TextRasterization::default().resolve(&kde);
        assert_eq!(r.subpixel(), Some((SubpixelOrder::Rgb, LcdFilter::Default)));
        let r = TextRasterization::default()
            .subpixel_order(Some(SubpixelOrder::VBgr))
            .lcd_filter(Some(LcdFilter::Light))
            .resolve(&kde);
        assert_eq!(r.subpixel(), Some((SubpixelOrder::VBgr, LcdFilter::Light)));
        for aa in [TextAntialias::Auto, TextAntialias::Subpixel] {
            let r = TextRasterization::default()
                .antialias(aa)
                .subpixel_order(Some(SubpixelOrder::None))
                .resolve(&kde);
            assert_eq!(r.antialias, ResolvedAntialias::Grayscale, "{aa:?}");
        }
        // An order the OS does not name turns `Auto` on.
        let r = TextRasterization::default()
            .subpixel_order(Some(SubpixelOrder::Bgr))
            .resolve(&SystemTextSettings::default());
        assert_eq!(r.subpixel(), Some((SubpixelOrder::Bgr, LcdFilter::Default)));
    }

    #[test]
    fn the_policy_is_per_thread() {
        set_thread_policy(ResolvedTextRasterization::default().with_darkening(0.0));
        assert_eq!(thread_policy().darkening_px, 0.0);
        std::thread::spawn(|| {
            assert_eq!(thread_policy(), ResolvedTextRasterization::default());
        })
        .join()
        .unwrap();
        reset_thread_policy();
        assert_eq!(thread_policy(), ResolvedTextRasterization::default());
    }
}
