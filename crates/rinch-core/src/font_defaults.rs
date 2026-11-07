//! The browser's font settings ([`FontDefaults`]), shared by every backend's
//! startup configuration. rinch-dom applies them (`rinch_dom::fonts`); a
//! browser has its own and ignores them.

/// The browser's font settings: the family an exhausted `font-family` list
/// falls back to, and the family each of the `serif` and `sans-serif`
/// generics means (#2255). Chrome's settings page has the same entries
/// ("standard", "serif", "sans-serif", and "fixed-width", which is left to
/// the platform here: Chrome's Linux default, `Monospace`, is fontconfig's
/// alias).
///
/// The default is Chrome's for the platform (`locale_settings_*.grd`): Times
/// New Roman, Times New Roman and Arial, or Times, Times and Helvetica on
/// macOS. An app sets others with `App::font_defaults`. A theme whose
/// `font-family` ends in a generic, as rinch's does, never reaches
/// `standard`.
///
/// `#[non_exhaustive]`, so a later setting (a `monospace` one) is not a
/// breaking change: build one from [`FontDefaults::default`] with the
/// `with_*` methods, or assign its fields.
///
/// ```
/// use rinch_core::font_defaults::FontDefaults;
/// let mine = FontDefaults::default()
///     .with_standard("Inter")
///     .with_serif(None);
/// assert_eq!(mine.standard, "Inter");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FontDefaults {
    /// A CSS `font-family` list naming the default font: what a list whose
    /// every family is missing ends in, the user agent's default font in CSS
    /// terms. Its first family the font context has (a named family, its
    /// Blink alternate, or a generic's first face) is appended to such a list
    /// **by name**. When it has none, the `sans-serif` slot's first face is,
    /// which is where Chrome on Linux goes when Times New Roman is missing
    /// (fontconfig's "Sans", not its serif slot).
    pub standard: String,
    /// The family put at the head of the `serif` slot where it (or its Blink
    /// alternate) is installed; `None` leaves the platform's slot. Applied on
    /// a desktop only.
    pub serif: Option<String>,
    /// As [`serif`](Self::serif), for `sans-serif`.
    pub sans_serif: Option<String>,
}

impl FontDefaults {
    /// These settings with [`standard`](Self::standard) set to `stack`.
    pub fn with_standard(mut self, stack: impl Into<String>) -> Self {
        self.standard = stack.into();
        self
    }

    /// These settings with [`serif`](Self::serif) set to `family`.
    pub fn with_serif(mut self, family: Option<&str>) -> Self {
        self.serif = family.map(str::to_owned);
        self
    }

    /// These settings with [`sans_serif`](Self::sans_serif) set to `family`.
    pub fn with_sans_serif(mut self, family: Option<&str>) -> Self {
        self.sans_serif = family.map(str::to_owned);
        self
    }
}

impl Default for FontDefaults {
    fn default() -> Self {
        // `standard` names Times alone, with no generic after it: where the
        // host has no Times, Chrome falls back to the sans face, not to its
        // serif setting (review of PR #2337: DejaVu Sans, 609.94px, on a
        // DejaVu-only host, where `serif` gave DejaVu Serif, 618px).
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let (standard, serif, sans) = ("Times", "Times", "Helvetica");
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        let (standard, serif, sans) = ("'Times New Roman'", "Times New Roman", "Arial");
        Self {
            standard: standard.to_owned(),
            serif: Some(serif.to_owned()),
            sans_serif: Some(sans.to_owned()),
        }
    }
}
