//! The CSS values a component's `color` prop stands for.

/// Whether a component's `color` prop is a raw CSS colour (`#…`, `rgb(…)`,
/// `rgba(…)`, `hsl(…)`, `hsla(…)`) rather than a theme palette name.
///
/// The branch ActionIcon, Loader and others take before building
/// `var(--rinch-color-{name}-6)` from a palette name.
pub fn is_raw_css_color(color: &str) -> bool {
    color.starts_with('#') || color.starts_with("rgb") || color.starts_with("hsl")
}

/// The text colour that reads on a component's `color` prop when it fills
/// (#2208): for a palette name, its theme token
/// `var(--rinch-color-{name}-contrast)`; for a raw CSS colour,
/// [`rinch_theme::contrast_text`] of it (`white` or `black`; `white` for a
/// colour it cannot read, such as `hsl(…)`).
pub fn contrast_for(color: &str) -> String {
    contrast_value(color, &format!("--rinch-color-{color}"), None)
}

/// The text colour that reads on `color`'s solid fill: the one rule behind
/// [`contrast_for`] and [`ColorTokens::contrast`] (#3020). For a raw CSS
/// colour it is [`rinch_theme::contrast_text`] of it and `family` is unused;
/// for a name it is the `{family}-contrast` token, with `fallback` as the
/// `var()` fallback when one is given.
fn contrast_value(color: &str, family: &str, fallback: Option<&str>) -> String {
    if is_raw_css_color(color) {
        rinch_theme::contrast_text(color).to_string()
    } else if let Some(fallback) = fallback {
        format!("var({family}-contrast, {fallback})")
    } else {
        format!("var({family}-contrast)")
    }
}

/// The inline custom properties a component writes for its `color` prop:
/// `{prefix}-color` (the fill: the raw colour, or the palette's shade 6) and
/// `{prefix}-contrast` (the text on that fill, [`contrast_for`]).
///
/// ```
/// use rinch_components::color_utils::accent_vars;
/// assert_eq!(
///     accent_vars("--rinch-tabs", "yellow"),
///     "--rinch-tabs-color: var(--rinch-color-yellow-6); \
///      --rinch-tabs-contrast: var(--rinch-color-yellow-contrast)"
/// );
/// assert_eq!(
///     accent_vars("--rinch-tabs", "#ffe066"),
///     "--rinch-tabs-color: #ffe066; --rinch-tabs-contrast: black"
/// );
/// ```
pub fn accent_vars(prefix: &str, color: &str) -> String {
    let fill = if is_raw_css_color(color) {
        color.to_string()
    } else {
        format!("var(--rinch-color-{color}-6)")
    };
    format!(
        "{prefix}-color: {fill}; {prefix}-contrast: {}",
        contrast_for(color)
    )
}

/// The CSS values a component's `color` prop stands for, in the shape of the
/// theme's variant tokens (#2879): the solid colour and its `light` family.
///
/// - a raw CSS colour ([`is_raw_css_color`]) is the solid colour, with its
///   light tint mixed from it, the colour itself as the text on the tint,
///   and [`rinch_theme::contrast_text`] of it as the text on the solid colour;
/// - `"primary"` reads `--rinch-primary-color` and its `-light…` and
///   `-contrast` tokens;
/// - any other name reads `--rinch-color-{name}-6`,
///   `--rinch-color-{name}-light…` and `--rinch-color-{name}-contrast`, so a
///   palette name and an app's own token family named the same way both work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorTokens {
    /// The solid colour (a filled background, an outline's border).
    pub base: String,
    /// The `light` variant's background.
    pub light: String,
    /// The accent colour on the `light` background.
    pub light_color: String,
    /// Strong accent on the `light` background (a title).
    pub light_strong: String,
    /// Body text on the `light` background.
    pub light_text: String,
    /// Text on the solid colour (#2208): for a name, its family's contrast
    /// token with `white` as the `var()` fallback for a family that has
    /// none (`var(--rinch-color-{name}-contrast, white)`); for a raw colour,
    /// `white` or `black` computed from it, as [`contrast_for`] does.
    pub contrast: String,
}

/// [`ColorTokens`] for a non-empty `color` prop value.
pub fn color_tokens(color: &str) -> ColorTokens {
    // The token family a name reads (unused for a raw colour).
    let prefix = if color == "primary" {
        "--rinch-primary-color".to_string()
    } else {
        format!("--rinch-color-{color}")
    };
    let contrast = contrast_value(color, &prefix, Some("white"));
    if is_raw_css_color(color) {
        ColorTokens {
            base: color.to_string(),
            light: format!("color-mix(in srgb, {color} 10%, transparent)"),
            light_color: color.to_string(),
            light_strong: color.to_string(),
            light_text: color.to_string(),
            contrast,
        }
    } else {
        ColorTokens {
            base: if color == "primary" {
                "var(--rinch-primary-color)".to_string()
            } else {
                format!("var({prefix}-6)")
            },
            light: format!("var({prefix}-light)"),
            light_color: format!("var({prefix}-light-color)"),
            light_strong: format!("var({prefix}-light-color-strong)"),
            light_text: format!("var({prefix}-light-text)"),
            contrast,
        }
    }
}
