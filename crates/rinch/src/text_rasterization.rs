//! Text rasterization: the app's choice and the OS's text settings (#2286).
//!
//! [`TextRasterization`](crate::text_rasterization::TextRasterization) is what
//! [`App::text_rasterization`] takes; every field is `Auto` by default, which
//! follows [`system_text_settings`](crate::text_rasterization::system_text_settings),
//! the OS's own settings, read once and cached:
//!
//! - **Linux**: fontconfig, as Chromium reads it. `sans-serif` is
//!   substituted and matched (`FcConfigSubstitute`, `FcDefaultSubstitute`,
//!   `FcFontMatch`), and the match's `antialias`, `hinting`, `hintstyle`,
//!   `rgba` and `lcdfilter` are taken. KDE's font settings write those into
//!   the user's `fonts.conf`; GNOME's (`org.gnome.desktop.interface
//!   font-antialiasing` and friends) reach only GTK apps through XSETTINGS
//!   and the settings portal, and are not read (fontconfig's own defaults,
//!   grayscale and slight hinting, are GNOME's defaults too). libfontconfig is
//!   opened at run time, so a system without it reads nothing.
//! - **Windows**: `SystemParametersInfo` — `SPI_GETFONTSMOOTHING`, and with
//!   ClearType (`SPI_GETFONTSMOOTHINGTYPE`) its `SPI_GETFONTSMOOTHINGORIENTATION`.
//! - **macOS**: grayscale and unhinted, which is all macOS has done since
//!   10.14. Subpixel text is never drawn there, even when the app asks for
//!   it (`SystemTextSettings::subpixel_capable` is `false`).
//!
//! The hinting is off for `hintstyle=hintnone` or `hinting=false`. A subpixel
//! order (`rgba` other than `none`, ClearType's orientation) turns on subpixel
//! (LCD) text through the glyph-mask atlas (#2286 stage 3), filtered with the
//! `lcdfilter` (FreeType's default five-tap filter where fontconfig names
//! none); an app overrides the order and the filter with
//! `TextRasterization::subpixel_order` and `lcd_filter`. `antialias=false`
//! has no aliased path and draws grayscale.
//!
//! The read happens on a thread of its own, started by [`App::run`], so it
//! does not hold up the window; the first frame that paints text waits for it
//! if it has not finished (fontconfig's match takes a few milliseconds with
//! its caches warm).
//!
//! [`App::text_rasterization`]: crate::app_builder::App::text_rasterization
//! [`App::run`]: crate::app_builder::App::run

use std::sync::OnceLock;

pub use rinch_dom::paint::text_rasterization::{
    DEFAULT_STEM_DARKENING_PX, LcdFilter, MAX_STEM_DARKENING_PX, ResolvedAntialias,
    ResolvedTextRasterization, StemDarkening, SubpixelOrder, SystemTextSettings, TextAntialias,
    TextHinting, TextRasterization,
};

static APP: OnceLock<TextRasterization> = OnceLock::new();
static SYSTEM: OnceLock<SystemTextSettings> = OnceLock::new();

/// Use `config` for every window's text, follow the OS where it says `Auto`,
/// and start reading the OS's settings in the background. [`App::run`] calls
/// it with the app's [`App::text_rasterization`]; an embedding host calls it
/// before its first frame (without a call an embedded document renders as if
/// the OS said nothing). The first call wins; it applies to frames painted
/// afterwards on threads that have not painted text yet.
///
/// [`App::run`]: crate::app_builder::App::run
/// [`App::text_rasterization`]: crate::app_builder::App::text_rasterization
pub fn set_text_rasterization(config: TextRasterization) {
    if APP.set(config).is_err() && APP.get() != Some(&config) {
        tracing::warn!("set_text_rasterization({config:?}) after it was set: ignored");
    }
    rinch_dom::paint::text_rasterization::set_policy_source(resolved);
    if SYSTEM.get().is_none() {
        // A failed spawn only means the first frame reads it instead.
        let _ = std::thread::Builder::new()
            .name("rinch-text-settings".into())
            .spawn(|| {
                system_text_settings();
            });
    }
}

/// The app's [`TextRasterization`] (the default before [`set_text_rasterization`]).
pub fn text_rasterization() -> TextRasterization {
    APP.get().copied().unwrap_or_default()
}

/// The app's choice resolved against the OS: what the painters use.
pub fn resolved() -> ResolvedTextRasterization {
    text_rasterization().resolve(&system_text_settings())
}

/// The OS's text settings, read on the first call (which a concurrent call
/// waits for) and cached for the process.
pub fn system_text_settings() -> SystemTextSettings {
    *SYSTEM.get_or_init(|| {
        let settings = read_system();
        tracing::debug!("system text settings: {settings:?}");
        settings
    })
}

#[cfg(all(target_os = "linux", feature = "desktop"))]
fn read_system() -> SystemTextSettings {
    fontconfig::read().unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn read_system() -> SystemTextSettings {
    win32::read()
}

#[cfg(target_os = "macos")]
fn read_system() -> SystemTextSettings {
    SystemTextSettings::default()
        .with_antialias(Some(true))
        .with_hinting(Some(TextHinting::None))
        .with_subpixel_order(Some(SubpixelOrder::None))
        .with_subpixel_capable(Some(false))
}

#[cfg(not(any(
    all(target_os = "linux", feature = "desktop"),
    target_os = "windows",
    target_os = "macos"
)))]
fn read_system() -> SystemTextSettings {
    SystemTextSettings::default()
}

/// fontconfig's integers, as `fontconfig.h` defines them.
#[cfg_attr(not(all(target_os = "linux", feature = "desktop")), allow(dead_code))]
mod fc_values {
    use super::{LcdFilter, SubpixelOrder, TextHinting};

    /// `FC_RGBA_*`; `FC_RGBA_UNKNOWN` (0) says nothing.
    pub(super) fn rgba(v: i32) -> Option<SubpixelOrder> {
        match v {
            1 => Some(SubpixelOrder::Rgb),
            2 => Some(SubpixelOrder::Bgr),
            3 => Some(SubpixelOrder::VRgb),
            4 => Some(SubpixelOrder::VBgr),
            5 => Some(SubpixelOrder::None),
            _ => None,
        }
    }

    /// `FC_HINT_*`.
    pub(super) fn hint_style(v: i32) -> Option<TextHinting> {
        match v {
            0 => Some(TextHinting::None),
            1 => Some(TextHinting::Slight),
            2 => Some(TextHinting::Medium),
            3 => Some(TextHinting::Full),
            _ => None,
        }
    }

    /// `FC_LCD_*`.
    pub(super) fn lcd_filter(v: i32) -> Option<LcdFilter> {
        match v {
            0 => Some(LcdFilter::None),
            1 => Some(LcdFilter::Default),
            2 => Some(LcdFilter::Light),
            3 => Some(LcdFilter::Legacy),
            _ => None,
        }
    }

    /// `hinting=false` turns hinting off whatever `hintstyle` says.
    pub(super) fn hinting(
        hinting: Option<bool>,
        style: Option<TextHinting>,
    ) -> Option<TextHinting> {
        match hinting {
            Some(false) => Some(TextHinting::None),
            _ => style,
        }
    }
}

#[cfg(all(target_os = "linux", feature = "desktop"))]
mod fontconfig {
    //! libfontconfig through `dlopen`, so nothing links it and a system
    //! without it reads nothing.

    use super::{SystemTextSettings, fc_values};
    use std::ffi::{CStr, c_char, c_int, c_void};

    type Config = c_void;
    type Pattern = c_void;
    const FC_MATCH_PATTERN: c_int = 0;
    const FC_RESULT_MATCH: c_int = 0;

    /// The match for `sans-serif`'s rendering properties, or `None` when
    /// libfontconfig cannot be opened or matches nothing.
    pub(super) fn read() -> Option<SystemTextSettings> {
        // SAFETY: the symbols are fontconfig's public API with the C
        // signatures below (fontconfig.h); every pattern and the config made
        // here are destroyed here, and the library is never unloaded.
        unsafe {
            // Never unloaded (leaked, on every path): fontique keeps
            // libfontconfig in the process anyway, and a `dlclose` of a library
            // that has run its initialisers (fontconfig's, FreeType's under it)
            // is not worth the few kilobytes it would give back.
            let lib: &'static libloading::Library = Box::leak(Box::new(
                libloading::Library::new("libfontconfig.so.1").ok()?,
            ));
            let init: libloading::Symbol<unsafe extern "C" fn() -> *mut Config> =
                lib.get(b"FcInitLoadConfigAndFonts\0").ok()?;
            let config_destroy: libloading::Symbol<unsafe extern "C" fn(*mut Config)> =
                lib.get(b"FcConfigDestroy\0").ok()?;
            let name_parse: libloading::Symbol<unsafe extern "C" fn(*const u8) -> *mut Pattern> =
                lib.get(b"FcNameParse\0").ok()?;
            let substitute: libloading::Symbol<
                unsafe extern "C" fn(*mut Config, *mut Pattern, c_int) -> c_int,
            > = lib.get(b"FcConfigSubstitute\0").ok()?;
            let default_substitute: libloading::Symbol<unsafe extern "C" fn(*mut Pattern)> =
                lib.get(b"FcDefaultSubstitute\0").ok()?;
            let font_match: libloading::Symbol<
                unsafe extern "C" fn(*mut Config, *mut Pattern, *mut c_int) -> *mut Pattern,
            > = lib.get(b"FcFontMatch\0").ok()?;
            let get_integer: libloading::Symbol<
                unsafe extern "C" fn(*const Pattern, *const c_char, c_int, *mut c_int) -> c_int,
            > = lib.get(b"FcPatternGetInteger\0").ok()?;
            let get_bool: libloading::Symbol<
                unsafe extern "C" fn(*const Pattern, *const c_char, c_int, *mut c_int) -> c_int,
            > = lib.get(b"FcPatternGetBool\0").ok()?;
            let pattern_destroy: libloading::Symbol<unsafe extern "C" fn(*mut Pattern)> =
                lib.get(b"FcPatternDestroy\0").ok()?;

            let config = init();
            if config.is_null() {
                return None;
            }
            let pattern = name_parse(c"sans-serif".as_ptr().cast());
            if pattern.is_null() {
                config_destroy(config);
                return None;
            }
            substitute(config, pattern, FC_MATCH_PATTERN);
            default_substitute(pattern);
            let mut result = 0;
            let matched = font_match(config, pattern, &mut result);
            pattern_destroy(pattern);
            if matched.is_null() {
                config_destroy(config);
                return None;
            }
            let int = |name: &CStr| {
                let mut v = 0;
                (get_integer(matched, name.as_ptr(), 0, &mut v) == FC_RESULT_MATCH).then_some(v)
            };
            let boolean = |name: &CStr| {
                let mut v = 0;
                (get_bool(matched, name.as_ptr(), 0, &mut v) == FC_RESULT_MATCH).then_some(v != 0)
            };
            let settings = SystemTextSettings::default()
                .with_antialias(boolean(c"antialias"))
                .with_hinting(fc_values::hinting(
                    boolean(c"hinting"),
                    int(c"hintstyle").and_then(fc_values::hint_style),
                ))
                .with_subpixel_order(int(c"rgba").and_then(fc_values::rgba))
                .with_lcd_filter(int(c"lcdfilter").and_then(fc_values::lcd_filter));
            pattern_destroy(matched);
            config_destroy(config);
            Some(settings)
        }
    }
}

#[cfg(target_os = "windows")]
mod win32 {
    use super::{SubpixelOrder, SystemTextSettings, TextHinting};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FE_FONTSMOOTHINGCLEARTYPE, SPI_GETFONTSMOOTHING, SPI_GETFONTSMOOTHINGORIENTATION,
        SPI_GETFONTSMOOTHINGTYPE, SystemParametersInfoW,
    };

    /// `FE_FONTSMOOTHINGORIENTATIONRGB`.
    const ORIENTATION_RGB: u32 = 1;

    fn get(action: u32) -> Option<u32> {
        let mut v: u32 = 0;
        // SAFETY: each of these actions writes one `UINT`/`BOOL` to `pvParam`.
        let ok = unsafe { SystemParametersInfoW(action, 0, (&mut v as *mut u32).cast(), 0) };
        (ok != 0).then_some(v)
    }

    pub(super) fn read() -> SystemTextSettings {
        let smoothing = get(SPI_GETFONTSMOOTHING).map(|v| v != 0);
        let cleartype = smoothing == Some(true)
            && get(SPI_GETFONTSMOOTHINGTYPE) == Some(FE_FONTSMOOTHINGCLEARTYPE);
        let subpixel_order = if cleartype {
            Some(match get(SPI_GETFONTSMOOTHINGORIENTATION) {
                Some(ORIENTATION_RGB) | None => SubpixelOrder::Rgb,
                Some(_) => SubpixelOrder::Bgr,
            })
        } else {
            smoothing.map(|_| SubpixelOrder::None)
        };
        // Windows hints (DirectWrite's and GDI's ClearType both fit to the
        // grid); "Full" names it, both painters hint the same.
        SystemTextSettings::default()
            .with_antialias(smoothing)
            .with_hinting(Some(TextHinting::Full))
            .with_subpixel_order(subpixel_order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fontconfig's integers map as `fontconfig.h` defines them; Joe's KDE
    /// (`rgba=1 lcdfilter=1 hintstyle=1 antialias=1`, #2286) reads as RGB
    /// subpixel through the default filter, slightly hinted.
    #[test]
    fn fontconfig_values_map_as_fontconfig_defines_them() {
        assert_eq!(fc_values::rgba(0), None);
        assert_eq!(fc_values::rgba(1), Some(SubpixelOrder::Rgb));
        assert_eq!(fc_values::rgba(2), Some(SubpixelOrder::Bgr));
        assert_eq!(fc_values::rgba(3), Some(SubpixelOrder::VRgb));
        assert_eq!(fc_values::rgba(4), Some(SubpixelOrder::VBgr));
        assert_eq!(fc_values::rgba(5), Some(SubpixelOrder::None));
        assert_eq!(fc_values::hint_style(0), Some(TextHinting::None));
        assert_eq!(fc_values::hint_style(1), Some(TextHinting::Slight));
        assert_eq!(fc_values::hint_style(3), Some(TextHinting::Full));
        assert_eq!(fc_values::lcd_filter(1), Some(LcdFilter::Default));
        assert_eq!(fc_values::lcd_filter(3), Some(LcdFilter::Legacy));
        assert_eq!(
            fc_values::hinting(Some(false), Some(TextHinting::Full)),
            Some(TextHinting::None)
        );
        assert_eq!(
            fc_values::hinting(Some(true), Some(TextHinting::Slight)),
            Some(TextHinting::Slight)
        );
        let kde = SystemTextSettings::default()
            .with_antialias(Some(true))
            .with_hinting(fc_values::hint_style(1))
            .with_subpixel_order(fc_values::rgba(1))
            .with_lcd_filter(fc_values::lcd_filter(1));
        let r = TextRasterization::default().resolve(&kde);
        assert!(r.hint());
        assert_eq!(
            r.antialias,
            ResolvedAntialias::Subpixel {
                order: SubpixelOrder::Rgb,
                filter: LcdFilter::Default
            }
        );
    }

    /// Prints what this host's OS says (`--ignored --nocapture`): a probe,
    /// not a check, since the answer is the host's.
    #[test]
    #[ignore]
    fn print_the_system_text_settings() {
        let t = std::time::Instant::now();
        let s = system_text_settings();
        eprintln!("system text settings: {s:?} (read in {:?})", t.elapsed());
    }
}
