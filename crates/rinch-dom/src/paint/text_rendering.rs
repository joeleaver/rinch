//! How glyphs are rasterised: the app's text-rendering choice, the OS's text
//! settings it defaults to, and the policy both painters read (#2286).
//!
//! An app states what it wants in a [`TextRendering`]; every field defaults to
//! `Auto`, which follows the OS as Chrome does. The OS's own settings are a
//! [`SystemTextSettings`] (fontconfig on Linux, `SystemParametersInfo` on
//! Windows, fixed on macOS; the facade crate reads them). The two resolve into
//! a [`ResolvedTextRendering`], which is what a painter consults per glyph run.
//!
//! Stage 1 of #2286 uses three things out of it:
//! - **hinting** — `hint` is off when the resolved hinting is
//!   [`TextHinting::None`];
//! - **stem darkening of light text** — [`ResolvedTextRendering::darkening_for`],
//!   applied by the Vello painter through `DrawGlyphs::font_embolden` and by
//!   the tiny-skia painter through swash's embolden, so both thicken the same
//!   glyphs by the same amount;
//! - the antialiasing choice, which is recorded but renders as grayscale on
//!   both painters until the glyph-mask atlas (stage 2) and subpixel
//!   compositing (stage 3) land: [`ResolvedAntialias::Subpixel`] carries the
//!   OS's subpixel order and LCD filter for that stage, and
//!   [`ResolvedAntialias::None`] (fontconfig `antialias=false`, Windows font
//!   smoothing off) has no aliased path in Vello's compute renderer.
//!
//! The resolved policy is per thread ([`set_text_rendering`] /
//! [`text_rendering`]): the runtime paints on the thread that runs its event
//! loop, and a test that changes it changes only its own thread. A thread that
//! never set one asks the source installed with [`set_text_rendering_source`]
//! once (the facade installs one that reads the OS), and without a source
//! takes [`ResolvedTextRendering::default`].

use std::cell::Cell;
use std::sync::OnceLock;

use peniko::color::{AlphaColor, Srgb};

/// The app's text-rendering choice: what [`App::text_rendering`] takes.
///
/// Every field is `Auto` by default, which follows the OS (see
/// [`SystemTextSettings`]) and, where the OS says nothing, Chrome. The struct
/// is `#[non_exhaustive]` so later stages can add fields (subpixel order, LCD
/// filter, contrast); build one with [`Default`] and the setters:
///
/// ```
/// use rinch_dom::paint::text_rendering::{StemDarkening, TextHinting, TextRendering};
/// let r = TextRendering::default()
///     .hinting(TextHinting::None)
///     .stem_darkening(StemDarkening::Off);
/// assert_eq!(r.hinting, TextHinting::None);
/// ```
///
/// [`App::text_rendering`]: https://docs.rs/rinch/latest/rinch/app_builder/struct.App.html#method.text_rendering
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TextRendering {
    /// Grayscale, subpixel (LCD) or no antialiasing. See [`TextAntialias`].
    pub antialias: TextAntialias,
    /// How hard outlines are fitted to the pixel grid. See [`TextHinting`].
    pub hinting: TextHinting,
    /// Thickening of light text. See [`StemDarkening`].
    pub stem_darkening: StemDarkening,
}

impl TextRendering {
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

    /// Resolve every `Auto` against the OS's settings.
    pub fn resolve(&self, system: &SystemTextSettings) -> ResolvedTextRendering {
        let antialias = match self.antialias {
            TextAntialias::Grayscale => ResolvedAntialias::Grayscale,
            TextAntialias::None => ResolvedAntialias::None,
            TextAntialias::Subpixel => ResolvedAntialias::Subpixel {
                // Asked for without the OS naming an order: RGB, the order of
                // nearly every desktop panel and Chrome's default.
                order: match system.subpixel_order {
                    Some(SubpixelOrder::None) | None => SubpixelOrder::Rgb,
                    Some(order) => order,
                },
                filter: system.lcd_filter.unwrap_or(LcdFilter::Default),
            },
            TextAntialias::Auto => match (system.antialias, system.subpixel_order) {
                (Some(false), _) => ResolvedAntialias::None,
                (_, Some(order)) if order != SubpixelOrder::None => ResolvedAntialias::Subpixel {
                    order,
                    filter: system.lcd_filter.unwrap_or(LcdFilter::Default),
                },
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
        ResolvedTextRendering {
            antialias,
            hinting,
            darkening_px,
        }
    }
}

/// How glyph edges are antialiased.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAntialias {
    /// Follow the OS: no antialiasing where it turns it off, subpixel where it
    /// names a subpixel order, grayscale otherwise (macOS is always grayscale).
    #[default]
    Auto,
    /// One coverage value per pixel.
    Grayscale,
    /// Per-channel (LCD) coverage. Stage 1 of #2286 records it and renders
    /// grayscale; subpixel compositing is stage 3.
    Subpixel,
    /// No antialiasing. Recorded; both painters render grayscale until the
    /// glyph-mask atlas (stage 2 of #2286).
    None,
}

/// How hard glyph outlines are fitted to the pixel grid.
///
/// Both painters have one hinted mode (skrifa's and swash's interpreters, each
/// in its smooth/light flavour), so `Slight`, `Medium` and `Full` all hint and
/// `None` does not. The levels are kept apart for the OS's own value and for a
/// later stage that can tell them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
pub enum StemDarkening {
    /// The default: up to [`DEFAULT_STEM_DARKENING_PX`] per side.
    #[default]
    Auto,
    /// Never thicken.
    Off,
    /// Up to this many device pixels per side (clamped to
    /// `0..=`[`MAX_STEM_DARKENING_PX`]); the luminance and size taper of
    /// [`ResolvedTextRendering::darkening_for`] still apply.
    Px(f32),
}

/// [`StemDarkening::Auto`]'s amount: 0.10 device pixels per side, at which
/// #2286 measured light-on-dark text at 0.93 of its linear-light weight.
pub const DEFAULT_STEM_DARKENING_PX: f32 = 0.10;

/// The largest amount [`StemDarkening::Px`] takes.
pub const MAX_STEM_DARKENING_PX: f32 = 0.5;

/// The order of a panel's subpixels, for subpixel antialiasing (stage 3).
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

/// The filter applied across subpixels (fontconfig `lcdfilter`), for stage 3.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

/// The antialiasing [`TextRendering::antialias`] resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedAntialias {
    /// One coverage value per pixel.
    Grayscale,
    /// Per-channel coverage in this order through this filter (rendered
    /// grayscale until stage 3).
    Subpixel {
        /// The panel's subpixel order.
        order: SubpixelOrder,
        /// The filter across subpixels.
        filter: LcdFilter,
    },
    /// No antialiasing (rendered grayscale until stage 2).
    None,
}

/// A [`TextRendering`] with every `Auto` resolved: what painters read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTextRendering {
    /// The antialiasing.
    pub antialias: ResolvedAntialias,
    /// The hinting.
    pub hinting: TextHinting,
    /// The most stem darkening, in device pixels per side, that the lightest
    /// small text gets; `0` turns it off.
    pub darkening_px: f32,
}

impl Default for ResolvedTextRendering {
    /// [`TextRendering::default`] with nothing known about the OS: grayscale,
    /// hinted, darkening on.
    fn default() -> Self {
        TextRendering::default().resolve(&SystemTextSettings::default())
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

impl ResolvedTextRendering {
    /// This policy with `darkening_px` replaced (for tests and tools).
    #[must_use]
    pub fn with_darkening(mut self, px: f32) -> Self {
        self.darkening_px = px;
        self
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
}

thread_local! {
    static CURRENT: Cell<Option<ResolvedTextRendering>> = const { Cell::new(None) };
}

static SOURCE: OnceLock<fn() -> ResolvedTextRendering> = OnceLock::new();

/// Install the function a thread asks for its policy the first time it paints
/// text without having [set](set_text_rendering) one. The first install wins.
/// The facade installs one that resolves the app's [`TextRendering`] against
/// the OS's settings.
pub fn set_text_rendering_source(source: fn() -> ResolvedTextRendering) {
    let _ = SOURCE.set(source);
}

/// Set this thread's text-rendering policy. Glyph runs painted on this thread
/// afterwards use it.
pub fn set_text_rendering(policy: ResolvedTextRendering) {
    CURRENT.with(|c| c.set(Some(policy)));
}

/// Forget this thread's policy, so the next glyph run asks the source again.
pub fn reset_text_rendering() {
    CURRENT.with(|c| c.set(None));
}

/// This thread's text-rendering policy.
#[inline]
pub fn text_rendering() -> ResolvedTextRendering {
    CURRENT.with(|c| match c.get() {
        Some(policy) => policy,
        None => {
            let policy = SOURCE
                .get()
                .map_or_else(ResolvedTextRendering::default, |f| f());
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
        let p = ResolvedTextRendering::default();
        assert_eq!(p.darkening_for(rgb(0xfc, 0xfc, 0xfc), 13.0), 0.09375);
        // 0.10 quantised to 1/64: 6.4 steps round to 6.
        assert_eq!(p.darkening_for(rgb(0xff, 0xff, 0xff), 24.0), 6.0 / 64.0);
    }

    #[test]
    fn dark_and_mid_text_gets_none() {
        let p = ResolvedTextRendering::default();
        assert_eq!(p.darkening_for(rgb(0x23, 0x26, 0x29), 13.0), 0.0);
        assert_eq!(p.darkening_for(rgb(0x7f, 0x7f, 0x7f), 13.0), 0.0);
    }

    #[test]
    fn darkening_ramps_with_luma_and_tapers_with_size() {
        let p = ResolvedTextRendering::default().with_darkening(0.32);
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
        let p = TextRendering::default()
            .stem_darkening(StemDarkening::Off)
            .resolve(&SystemTextSettings::default());
        assert_eq!(p.darkening_for(rgb(0xff, 0xff, 0xff), 13.0), 0.0);
        let p = TextRendering::default()
            .stem_darkening(StemDarkening::Px(f32::NAN))
            .resolve(&SystemTextSettings::default());
        assert_eq!(p.darkening_px, 0.0);
        let p = TextRendering::default()
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
        };
        let r = TextRendering::default().resolve(&kde);
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
        let r = TextRendering::default().resolve(&none);
        assert_eq!(r.antialias, ResolvedAntialias::None);
        assert!(!r.hint());
        // `rgba=none` is grayscale.
        let gray = SystemTextSettings {
            subpixel_order: Some(SubpixelOrder::None),
            ..Default::default()
        };
        let r = TextRendering::default().resolve(&gray);
        assert_eq!(r.antialias, ResolvedAntialias::Grayscale);
        // Nothing known: grayscale, hinted.
        let r = ResolvedTextRendering::default();
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
        };
        let r = TextRendering::default()
            .antialias(TextAntialias::Grayscale)
            .hinting(TextHinting::None)
            .resolve(&kde);
        assert_eq!(r.antialias, ResolvedAntialias::Grayscale);
        assert!(!r.hint());
        let r = TextRendering::default()
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

    #[test]
    fn the_policy_is_per_thread() {
        set_text_rendering(ResolvedTextRendering::default().with_darkening(0.0));
        assert_eq!(text_rendering().darkening_px, 0.0);
        std::thread::spawn(|| {
            assert_eq!(text_rendering(), ResolvedTextRendering::default());
        })
        .join()
        .unwrap();
        reset_text_rendering();
        assert_eq!(text_rendering(), ResolvedTextRendering::default());
    }
}
