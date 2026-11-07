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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontDefaults {
    /// A CSS `font-family` list: what a list whose every family is missing
    /// ends in, the user agent's default font in CSS terms. Its first family
    /// the font context has (a named family, its Blink alternate, or a
    /// generic's first face) is appended to such a list **by name**; when it
    /// has none, the `sans-serif` slot's first face is (#1198).
    pub standard: String,
    /// The family put at the head of the `serif` slot where it (or its Blink
    /// alternate) is installed; `None` leaves the platform's slot. Applied on
    /// a desktop only.
    pub serif: Option<String>,
    /// As [`serif`](Self::serif), for `sans-serif`.
    pub sans_serif: Option<String>,
}

impl Default for FontDefaults {
    fn default() -> Self {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let (standard, serif, sans) = ("Times, serif", "Times", "Helvetica");
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        let (standard, serif, sans) = ("'Times New Roman', serif", "Times New Roman", "Arial");
        Self {
            standard: standard.to_owned(),
            serif: Some(serif.to_owned()),
            sans_serif: Some(sans.to_owned()),
        }
    }
}
