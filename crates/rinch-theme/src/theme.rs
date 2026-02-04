//! Theme struct and configuration.
//!
//! The `Theme` struct holds all theme values and can be customized.

use crate::colors::{ColorName, ColorPalettes, DEFAULT_COLORS};
use crate::radius::{DEFAULT_RADIUS, RadiusScale, RadiusSize};
use crate::shadows::{DEFAULT_SHADOWS, ShadowScale};
use crate::spacing::{DEFAULT_SPACING, SpacingScale};
use crate::typography::{
    DEFAULT_FONT_FAMILY, DEFAULT_FONT_FAMILY_MONOSPACE, DEFAULT_FONT_SIZES, DEFAULT_HEADINGS,
    DEFAULT_LINE_HEIGHTS, FontSizeScale, HeadingStyles, LineHeightScale,
};

/// Focus ring style configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusRing {
    /// Always show focus ring.
    Always,
    /// Show focus ring only on keyboard focus (not mouse).
    #[default]
    Auto,
    /// Never show focus ring.
    Never,
}

/// The complete theme configuration.
#[derive(Debug, Clone)]
pub struct Theme {
    /// The primary color name (used for buttons, links, etc.).
    pub primary_color: ColorName,
    /// All color palettes.
    pub colors: ColorPalettes,
    /// Spacing scale.
    pub spacing: SpacingScale,
    /// Border radius scale.
    pub radius: RadiusScale,
    /// Font size scale.
    pub font_sizes: FontSizeScale,
    /// Line height scale.
    pub line_heights: LineHeightScale,
    /// Heading styles.
    pub headings: HeadingStyles,
    /// Shadow scale.
    pub shadows: ShadowScale,
    /// Primary font family.
    pub font_family: String,
    /// Monospace font family.
    pub font_family_monospace: String,
    /// Default border radius for components.
    pub default_radius: RadiusSize,
    /// Focus ring style.
    pub focus_ring: FocusRing,
    /// Whether dark mode is enabled.
    pub dark_mode: bool,
    /// Primary shade index (0-9) for filled components.
    pub primary_shade: usize,
    /// Whether to respect system color scheme preference.
    pub respect_reduced_motion: bool,
    /// Cursor type for interactive elements.
    pub cursor_type: CursorType,
}

/// Cursor type for interactive elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorType {
    /// Default cursor.
    Default,
    /// Pointer cursor for clickable elements.
    #[default]
    Pointer,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary_color: ColorName::Blue,
            colors: DEFAULT_COLORS,
            spacing: DEFAULT_SPACING,
            radius: DEFAULT_RADIUS,
            font_sizes: DEFAULT_FONT_SIZES,
            line_heights: DEFAULT_LINE_HEIGHTS,
            headings: DEFAULT_HEADINGS,
            shadows: DEFAULT_SHADOWS,
            font_family: DEFAULT_FONT_FAMILY.to_string(),
            font_family_monospace: DEFAULT_FONT_FAMILY_MONOSPACE.to_string(),
            default_radius: RadiusSize::Sm,
            focus_ring: FocusRing::Auto,
            dark_mode: false,
            primary_shade: 6,
            respect_reduced_motion: true,
            cursor_type: CursorType::Pointer,
        }
    }
}

impl Theme {
    /// Create a new theme with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a theme builder for customization.
    pub fn builder() -> ThemeBuilder {
        ThemeBuilder::new()
    }

    /// Get the primary color palette.
    pub fn primary_palette(&self) -> &crate::colors::ColorPalette {
        self.colors.get(self.primary_color)
    }

    /// Get the primary color (at the primary shade).
    pub fn primary_color_value(&self) -> &'static str {
        self.primary_palette().shade(self.primary_shade).hex()
    }
}

/// Builder for creating customized themes.
#[derive(Debug, Clone)]
pub struct ThemeBuilder {
    theme: Theme,
}

impl ThemeBuilder {
    /// Create a new theme builder with default values.
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
        }
    }

    /// Set the primary color.
    pub fn primary_color(mut self, color: ColorName) -> Self {
        self.theme.primary_color = color;
        self
    }

    /// Set the primary color from a string.
    pub fn primary_color_str(mut self, color: &str) -> Self {
        if let Ok(name) = color.parse() {
            self.theme.primary_color = name;
        }
        self
    }

    /// Set the default border radius.
    pub fn default_radius(mut self, radius: RadiusSize) -> Self {
        self.theme.default_radius = radius;
        self
    }

    /// Set the default border radius from a string.
    pub fn default_radius_str(mut self, radius: &str) -> Self {
        if let Ok(size) = radius.parse() {
            self.theme.default_radius = size;
        }
        self
    }

    /// Set the font family.
    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        self.theme.font_family = family.into();
        self
    }

    /// Set the monospace font family.
    pub fn font_family_monospace(mut self, family: impl Into<String>) -> Self {
        self.theme.font_family_monospace = family.into();
        self
    }

    /// Enable or disable dark mode.
    pub fn dark_mode(mut self, enabled: bool) -> Self {
        self.theme.dark_mode = enabled;
        self
    }

    /// Set the focus ring style.
    pub fn focus_ring(mut self, style: FocusRing) -> Self {
        self.theme.focus_ring = style;
        self
    }

    /// Set the primary shade index (0-9).
    pub fn primary_shade(mut self, shade: usize) -> Self {
        self.theme.primary_shade = shade.min(9);
        self
    }

    /// Set the cursor type for interactive elements.
    pub fn cursor_type(mut self, cursor: CursorType) -> Self {
        self.theme.cursor_type = cursor;
        self
    }

    /// Build the theme.
    pub fn build(self) -> Theme {
        self.theme
    }
}

impl Default for ThemeBuilder {
    fn default() -> Self {
        Self::new()
    }
}
