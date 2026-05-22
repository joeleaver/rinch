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
        // Component-state tokens (used by Select/DropdownMenu/etc).
        // In dark mode the primary-color-0 shade is near-white, so we substitute
        // neutral dark shades; selected items still get the primary text color
        // applied separately for an accent cue.
        css.push_str("  --rinch-color-state-disabled: var(--rinch-color-dark-6);\n");
        css.push_str("  --rinch-color-option-hover: var(--rinch-color-dark-5);\n");
        css.push_str("  --rinch-color-option-selected: var(--rinch-color-dark-4);\n");
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
        // Component-state tokens (used by Select/DropdownMenu/etc).
        // Light mode keeps the Mantine-style light-primary tint for hover/selected.
        css.push_str("  --rinch-color-state-disabled: var(--rinch-color-gray-1);\n");
        css.push_str("  --rinch-color-option-hover: var(--rinch-primary-color-0);\n");
        css.push_str("  --rinch-color-option-selected: var(--rinch-primary-color-0);\n");
        // Light mode titlebar: primary color background with white text/icons
        css.push_str("  --rinch-titlebar-bg: var(--rinch-primary-color);\n");
        css.push_str("  --rinch-titlebar-text: white;\n");
        css.push_str("  --rinch-titlebar-icon: white;\n");
        css.push_str("  --rinch-titlebar-hover: rgba(255, 255, 255, 0.2);\n");
        css.push_str("  --rinch-titlebar-active: rgba(255, 255, 255, 0.3);\n");
    }

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
