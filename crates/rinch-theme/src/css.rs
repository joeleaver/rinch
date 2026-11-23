//! CSS variable generation for the theme.
//!
//! Generates CSS custom properties (variables) from theme values.

use crate::colors::ColorName;
use crate::radius::RadiusSize;
use crate::shadows::ShadowSize;
use crate::spacing::SpacingSize;
use crate::theme::Theme;
use crate::typography::{FontSize, HeadingLevel};

/// Generate CSS variables from a theme.
pub fn generate_css_variables(theme: &Theme) -> String {
    let mut css = String::with_capacity(8192);

    css.push_str(":root {\n");

    // Colors
    for color_name in ColorName::all() {
        let palette = theme.colors.get(*color_name);
        let name = color_name.css_name();
        for (i, shade) in palette.shades().iter().enumerate() {
            css.push_str(&format!(
                "  --rinch-color-{}-{}: {};\n",
                name,
                i,
                shade.hex()
            ));
        }
    }

    // Primary color aliases
    css.push_str(&format!(
        "  --rinch-primary-color: var(--rinch-color-{}-{});\n",
        theme.primary_color.css_name(),
        theme.primary_shade
    ));
    for i in 0..10 {
        css.push_str(&format!(
            "  --rinch-primary-color-{}: var(--rinch-color-{}-{});\n",
            i,
            theme.primary_color.css_name(),
            i
        ));
    }

    // Spacing
    for size in SpacingSize::all() {
        css.push_str(&format!(
            "  --rinch-spacing-{}: {};\n",
            size.css_name(),
            theme.spacing.get(*size)
        ));
    }

    // Radius
    for size in RadiusSize::all() {
        css.push_str(&format!(
            "  --rinch-radius-{}: {};\n",
            size.css_name(),
            theme.radius.get(*size)
        ));
    }
    css.push_str(&format!(
        "  --rinch-radius-default: var(--rinch-radius-{});\n",
        theme.default_radius.css_name()
    ));

    // Shadows
    for size in ShadowSize::all() {
        css.push_str(&format!(
            "  --rinch-shadow-{}: {};\n",
            size.css_name(),
            theme.shadows.get(*size)
        ));
    }

    // Font sizes
    for size in FontSize::all() {
        css.push_str(&format!(
            "  --rinch-font-size-{}: {};\n",
            size.css_name(),
            theme.font_sizes.get(*size)
        ));
    }

    // Line heights
    for size in FontSize::all() {
        css.push_str(&format!(
            "  --rinch-line-height-{}: {};\n",
            size.css_name(),
            theme.line_heights.get(*size)
        ));
    }

    // Headings
    for level in HeadingLevel::all() {
        let style = theme.headings.get(*level);
        let name = level.css_name();
        css.push_str(&format!(
            "  --rinch-{}-font-size: {};\n",
            name, style.font_size
        ));
        css.push_str(&format!(
            "  --rinch-{}-line-height: {};\n",
            name, style.line_height
        ));
        css.push_str(&format!(
            "  --rinch-{}-font-weight: {};\n",
            name, style.font_weight
        ));
    }

    // Font families
    css.push_str(&format!("  --rinch-font-family: {};\n", theme.font_family));
    css.push_str(&format!(
        "  --rinch-font-family-monospace: {};\n",
        theme.font_family_monospace
    ));

    // Color scheme
    if theme.dark_mode {
        css.push_str("  color-scheme: dark;\n");
        css.push_str("  --rinch-color-scheme: dark;\n");
        // Dark mode body colors
        css.push_str("  --rinch-color-body: var(--rinch-color-dark-7);\n");
        css.push_str("  --rinch-color-surface: var(--rinch-color-dark-6);\n");
        css.push_str("  --rinch-color-text: var(--rinch-color-dark-0);\n");
        css.push_str("  --rinch-color-dimmed: var(--rinch-color-dark-2);\n");
        css.push_str("  --rinch-color-border: var(--rinch-color-dark-4);\n");
        // Alias retained for components that reference `--rinch-color-default-border`
        // (color picker, context menu, floating panel). Same value as `--rinch-color-border`.
        css.push_str("  --rinch-color-default-border: var(--rinch-color-dark-4);\n");
        css.push_str("  --rinch-color-placeholder: var(--rinch-color-dark-3);\n");
        // Dark mode control colors
        css.push_str("  --rinch-color-default: var(--rinch-color-dark-5);\n");
        css.push_str("  --rinch-color-default-hover: var(--rinch-color-dark-4);\n");
        css.push_str("  --rinch-color-filled: var(--rinch-color-dark-6);\n");
        css.push_str("  --rinch-color-filled-hover: var(--rinch-color-dark-5);\n");
        // Component-state tokens (used by Select and apps' own lists). As in
        // Mantine, a hovered option is a neutral shade (the same as a hovered
        // menu item, `--rinch-color-default`) and the selected one a light tint
        // of the primary colour (Mantine's `light` variant: in the dark scheme
        // primary-color-0 is near-white, so it is the primary at 15% instead).
        // Hover, selected and `default-hover` are three different colours (#2727).
        css.push_str("  --rinch-color-state-disabled: var(--rinch-color-dark-6);\n");
        css.push_str("  --rinch-color-option-hover: var(--rinch-color-dark-5);\n");
        css.push_str(
            "  --rinch-color-option-selected: color-mix(in srgb, var(--rinch-primary-color) 15%, transparent);\n",
        );
        // A hovered selected row (Tree, #2871): Mantine's `light-hover`.
        css.push_str(
            "  --rinch-color-option-selected-hover: color-mix(in srgb, var(--rinch-primary-color) 25%, transparent);\n",
        );
        // A keyboard focus ring (Checkbox, Radio, Switch) and the Mark /
        // Highlight background (#2879): primary-2 and yellow-2 are near-white
        // here, so they are translucent tints instead.
        css.push_str(
            "  --rinch-color-focus-ring: color-mix(in srgb, var(--rinch-primary-color) 50%, transparent);\n",
        );
        css.push_str(
            "  --rinch-color-mark: color-mix(in srgb, var(--rinch-color-yellow-6) 35%, transparent);\n",
        );
        // Dark mode titlebar: dark background with primary color text/icons
        css.push_str("  --rinch-titlebar-bg: var(--rinch-color-surface);\n");
        css.push_str("  --rinch-titlebar-text: var(--rinch-primary-color);\n");
        css.push_str("  --rinch-titlebar-icon: var(--rinch-primary-color);\n");
        css.push_str("  --rinch-titlebar-hover: rgba(255, 255, 255, 0.1);\n");
        css.push_str("  --rinch-titlebar-active: rgba(255, 255, 255, 0.15);\n");
    } else {
        css.push_str("  color-scheme: light;\n");
        css.push_str("  --rinch-color-scheme: light;\n");
        // Light mode body colors
        css.push_str("  --rinch-color-body: var(--rinch-color-gray-0);\n");
        css.push_str("  --rinch-color-surface: var(--rinch-color-white, #ffffff);\n");
        css.push_str("  --rinch-color-text: var(--rinch-color-gray-9);\n");
        css.push_str("  --rinch-color-dimmed: var(--rinch-color-gray-6);\n");
        css.push_str("  --rinch-color-border: var(--rinch-color-gray-3);\n");
        // Alias retained for components that reference `--rinch-color-default-border`.
        css.push_str("  --rinch-color-default-border: var(--rinch-color-gray-3);\n");
        css.push_str("  --rinch-color-placeholder: var(--rinch-color-gray-5);\n");
        // Light mode control colors
        css.push_str("  --rinch-color-default: var(--rinch-color-gray-2);\n");
        css.push_str("  --rinch-color-default-hover: var(--rinch-color-gray-3);\n");
        css.push_str("  --rinch-color-filled: var(--rinch-color-gray-1);\n");
        css.push_str("  --rinch-color-filled-hover: var(--rinch-color-gray-2);\n");
        // Component-state tokens: a hovered option is a neutral gray (as a
        // hovered menu item, `--rinch-color-default`), the selected one keeps
        // the Mantine light-primary tint. They used to share it (#2727).
        css.push_str("  --rinch-color-state-disabled: var(--rinch-color-gray-1);\n");
        css.push_str("  --rinch-color-option-hover: var(--rinch-color-gray-2);\n");
        css.push_str("  --rinch-color-option-selected: var(--rinch-primary-color-0);\n");
        css.push_str("  --rinch-color-option-selected-hover: var(--rinch-primary-color-1);\n");
        css.push_str("  --rinch-color-focus-ring: var(--rinch-primary-color-2);\n");
        css.push_str("  --rinch-color-mark: var(--rinch-color-yellow-2);\n");
        // Light mode titlebar: primary color background with white text/icons
        css.push_str("  --rinch-titlebar-bg: var(--rinch-primary-color);\n");
        css.push_str("  --rinch-titlebar-text: white;\n");
        css.push_str("  --rinch-titlebar-icon: white;\n");
        css.push_str("  --rinch-titlebar-hover: rgba(255, 255, 255, 0.2);\n");
        css.push_str("  --rinch-titlebar-active: rgba(255, 255, 255, 0.3);\n");
    }

    // Variant tokens (#2879), Mantine's `light` variant family, for every
    // palette colour and the primary. Light scheme: the palette's light shades,
    // the values these components always used. Dark scheme: the colour as a
    // translucent tint over the surface, and lighter text on it.
    for color_name in ColorName::all() {
        let name = color_name.css_name();
        push_variant_tokens(
            &mut css,
            theme.dark_mode,
            &format!("--rinch-color-{name}"),
            &format!("var(--rinch-color-{name}-6)"),
            &|i| format!("var(--rinch-color-{name}-{i})"),
        );
    }
    push_variant_tokens(
        &mut css,
        theme.dark_mode,
        "--rinch-primary-color",
        "var(--rinch-primary-color)",
        &|i| format!("var(--rinch-primary-color-{i})"),
    );

    // Reduced motion preference
    css.push_str("}\n\n");

    if theme.respect_reduced_motion {
        css.push_str("@media (prefers-reduced-motion: reduce) {\n");
        css.push_str("  *, *::before, *::after {\n");
        css.push_str("    animation-duration: 0.01ms !important;\n");
        css.push_str("    animation-iteration-count: 1 !important;\n");
        css.push_str("    transition-duration: 0.01ms !important;\n");
        css.push_str("  }\n");
        css.push_str("}\n\n");
    }

    css
}

/// Writes one colour's `light` variant tokens (#2879):
///
/// | token | light scheme | dark scheme |
/// |---|---|---|
/// | `{prefix}-light` | shade 0 | `tint` at 15% |
/// | `{prefix}-light-hover` | shade 1 | `tint` at 20% |
/// | `{prefix}-light-active` | shade 2 | `tint` at 25% |
/// | `{prefix}-light-color` | shade 6 | shade 4 |
/// | `{prefix}-light-color-strong` | shade 7 | shade 3 |
/// | `{prefix}-light-text` | shade 9 | `--rinch-color-text` |
///
/// The dark tints are `color-mix(... transparent)`, so they lie over whatever
/// surface the component sits on, as `--rinch-color-option-selected` does.
fn push_variant_tokens(
    css: &mut String,
    dark: bool,
    prefix: &str,
    tint: &str,
    shade: &dyn Fn(usize) -> String,
) {
    let mix = |pct: u32| format!("color-mix(in srgb, {tint} {pct}%, transparent)");
    let rows: [(&str, String); 6] = if dark {
        [
            ("light", mix(15)),
            ("light-hover", mix(20)),
            ("light-active", mix(25)),
            ("light-color", shade(4)),
            ("light-color-strong", shade(3)),
            ("light-text", "var(--rinch-color-text)".to_string()),
        ]
    } else {
        [
            ("light", shade(0)),
            ("light-hover", shade(1)),
            ("light-active", shade(2)),
            ("light-color", shade(6)),
            ("light-color-strong", shade(7)),
            ("light-text", shade(9)),
        ]
    };
    for (suffix, value) in rows {
        css.push_str(&format!("  {prefix}-{suffix}: {value};\n"));
    }
}

/// Generate base CSS styles for the theme (body styles, etc.).
pub fn generate_base_styles(theme: &Theme) -> String {
    let mut css = String::with_capacity(2048);

    // CSS reset and base styles
    css.push_str("*, *::before, *::after {\n");
    css.push_str("  box-sizing: border-box;\n");
    css.push_str("}\n\n");

    css.push_str("html {\n");
    css.push_str("  font-size: 16px;\n");
    css.push_str("  -webkit-font-smoothing: antialiased;\n");
    css.push_str("  -moz-osx-font-smoothing: grayscale;\n");
    css.push_str("}\n\n");

    css.push_str("body {\n");
    css.push_str("  margin: 0;\n");
    css.push_str("  padding: 0;\n");
    css.push_str("  font-family: var(--rinch-font-family);\n");
    css.push_str("  font-size: var(--rinch-font-size-md);\n");
    css.push_str("  line-height: var(--rinch-line-height-md);\n");
    // NOTE: No background-color here — transparent windows need body to be transparent.
    // Components that need an opaque background (e.g., BorderlessWindow) set their own.
    css.push_str("  color: var(--rinch-color-text);\n");
    css.push_str("}\n\n");

    // Heading styles
    for level in HeadingLevel::all() {
        let name = level.css_name();
        css.push_str(&format!("{} {{\n", name));
        css.push_str(&format!("  font-size: var(--rinch-{}-font-size);\n", name));
        css.push_str(&format!(
            "  line-height: var(--rinch-{}-line-height);\n",
            name
        ));
        css.push_str(&format!(
            "  font-weight: var(--rinch-{}-font-weight);\n",
            name
        ));
        css.push_str("  margin: 0;\n");
        css.push_str("}\n\n");
    }

    // Code/pre styles
    css.push_str("code, pre, kbd, samp {\n");
    css.push_str("  font-family: var(--rinch-font-family-monospace);\n");
    css.push_str("}\n\n");

    // Link styles
    css.push_str("a {\n");
    css.push_str("  color: var(--rinch-primary-color);\n");
    css.push_str("  text-decoration: none;\n");
    css.push_str("}\n\n");

    css.push_str("a:hover {\n");
    css.push_str("  text-decoration: underline;\n");
    css.push_str("}\n\n");

    // Form elements: inherit text color from the theme so they're visible in dark mode.
    // Stylo's UA stylesheet sets form element colors to system defaults (e.g. buttontext)
    // which don't adapt to dark mode without color-scheme support in the rendering engine.
    css.push_str("button, input, select, textarea {\n");
    css.push_str("  color: inherit;\n");
    css.push_str("  font-family: inherit;\n");
    css.push_str("}\n\n");

    // Focus styles based on theme setting
    match theme.focus_ring {
        crate::theme::FocusRing::Always => {
            css.push_str(":focus-visible {\n");
            css.push_str("  outline: 2px solid var(--rinch-primary-color);\n");
            css.push_str("  outline-offset: 2px;\n");
            css.push_str("}\n\n");
        }
        crate::theme::FocusRing::Auto => {
            css.push_str(":focus:not(:focus-visible) {\n");
            css.push_str("  outline: none;\n");
            css.push_str("}\n\n");
            css.push_str(":focus-visible {\n");
            css.push_str("  outline: 2px solid var(--rinch-primary-color);\n");
            css.push_str("  outline-offset: 2px;\n");
            css.push_str("}\n\n");
        }
        crate::theme::FocusRing::Never => {
            css.push_str(":focus {\n");
            css.push_str("  outline: none;\n");
            css.push_str("}\n\n");
        }
    }

    css
}

/// Generate the complete CSS for a theme (variables + base styles).
pub fn generate_theme_css(theme: &Theme) -> String {
    let mut css = String::with_capacity(16384);
    css.push_str(&generate_css_variables(theme));
    css.push_str(&generate_base_styles(theme));
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_css_variables() {
        let theme = Theme::default();
        let css = generate_css_variables(&theme);

        // Check that CSS contains expected variables
        assert!(css.contains("--rinch-color-blue-5"));
        assert!(css.contains("--rinch-spacing-md"));
        assert!(css.contains("--rinch-radius-sm"));
        assert!(css.contains("--rinch-font-size-md"));
        assert!(css.contains("--rinch-primary-color"));
    }

    #[test]
    fn test_generate_base_styles() {
        let theme = Theme::default();
        let css = generate_base_styles(&theme);

        assert!(css.contains("box-sizing: border-box"));
        assert!(css.contains("font-family: var(--rinch-font-family)"));
    }
}
