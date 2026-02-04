//! Typography system for the theme.
//!
//! Provides font sizes, line heights, and font families.

/// A font size in the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontSize {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl FontSize {
    /// Get all font sizes.
    pub const fn all() -> &'static [FontSize] {
        &[
            FontSize::Xs,
            FontSize::Sm,
            FontSize::Md,
            FontSize::Lg,
            FontSize::Xl,
        ]
    }

    /// Get the CSS variable name for this size.
    pub const fn css_name(&self) -> &'static str {
        match self {
            FontSize::Xs => "xs",
            FontSize::Sm => "sm",
            FontSize::Md => "md",
            FontSize::Lg => "lg",
            FontSize::Xl => "xl",
        }
    }
}

impl std::fmt::Display for FontSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.css_name())
    }
}

impl std::str::FromStr for FontSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(FontSize::Xs),
            "sm" => Ok(FontSize::Sm),
            "md" => Ok(FontSize::Md),
            "lg" => Ok(FontSize::Lg),
            "xl" => Ok(FontSize::Xl),
            _ => Err(()),
        }
    }
}

/// The font size scale.
#[derive(Debug, Clone, Copy)]
pub struct FontSizeScale {
    pub xs: &'static str,
    pub sm: &'static str,
    pub md: &'static str,
    pub lg: &'static str,
    pub xl: &'static str,
}

impl FontSizeScale {
    /// Get a font size by size.
    pub const fn get(&self, size: FontSize) -> &'static str {
        match size {
            FontSize::Xs => self.xs,
            FontSize::Sm => self.sm,
            FontSize::Md => self.md,
            FontSize::Lg => self.lg,
            FontSize::Xl => self.xl,
        }
    }
}

impl Default for FontSizeScale {
    fn default() -> Self {
        DEFAULT_FONT_SIZES
    }
}

/// Default font size scale (matches Mantine).
pub const DEFAULT_FONT_SIZES: FontSizeScale = FontSizeScale {
    xs: "0.75rem",  // 12px
    sm: "0.875rem", // 14px
    md: "1rem",     // 16px
    lg: "1.125rem", // 18px
    xl: "1.25rem",  // 20px
};

/// The line height scale.
#[derive(Debug, Clone, Copy)]
pub struct LineHeightScale {
    pub xs: &'static str,
    pub sm: &'static str,
    pub md: &'static str,
    pub lg: &'static str,
    pub xl: &'static str,
}

impl LineHeightScale {
    /// Get a line height by size.
    pub const fn get(&self, size: FontSize) -> &'static str {
        match size {
            FontSize::Xs => self.xs,
            FontSize::Sm => self.sm,
            FontSize::Md => self.md,
            FontSize::Lg => self.lg,
            FontSize::Xl => self.xl,
        }
    }
}

impl Default for LineHeightScale {
    fn default() -> Self {
        DEFAULT_LINE_HEIGHTS
    }
}

/// Default line height scale (matches Mantine).
pub const DEFAULT_LINE_HEIGHTS: LineHeightScale = LineHeightScale {
    xs: "1.4",
    sm: "1.45",
    md: "1.55",
    lg: "1.6",
    xl: "1.65",
};

/// Heading sizes for titles (h1-h6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
}

impl HeadingLevel {
    /// Get all heading levels.
    pub const fn all() -> &'static [HeadingLevel] {
        &[
            HeadingLevel::H1,
            HeadingLevel::H2,
            HeadingLevel::H3,
            HeadingLevel::H4,
            HeadingLevel::H5,
            HeadingLevel::H6,
        ]
    }

    /// Get the CSS variable name for this heading level.
    pub const fn css_name(&self) -> &'static str {
        match self {
            HeadingLevel::H1 => "h1",
            HeadingLevel::H2 => "h2",
            HeadingLevel::H3 => "h3",
            HeadingLevel::H4 => "h4",
            HeadingLevel::H5 => "h5",
            HeadingLevel::H6 => "h6",
        }
    }
}

/// A heading style definition.
#[derive(Debug, Clone, Copy)]
pub struct HeadingStyle {
    pub font_size: &'static str,
    pub line_height: &'static str,
    pub font_weight: &'static str,
}

/// The heading styles scale.
#[derive(Debug, Clone, Copy)]
pub struct HeadingStyles {
    pub h1: HeadingStyle,
    pub h2: HeadingStyle,
    pub h3: HeadingStyle,
    pub h4: HeadingStyle,
    pub h5: HeadingStyle,
    pub h6: HeadingStyle,
}

impl HeadingStyles {
    /// Get a heading style by level.
    pub const fn get(&self, level: HeadingLevel) -> &HeadingStyle {
        match level {
            HeadingLevel::H1 => &self.h1,
            HeadingLevel::H2 => &self.h2,
            HeadingLevel::H3 => &self.h3,
            HeadingLevel::H4 => &self.h4,
            HeadingLevel::H5 => &self.h5,
            HeadingLevel::H6 => &self.h6,
        }
    }
}

impl Default for HeadingStyles {
    fn default() -> Self {
        DEFAULT_HEADINGS
    }
}

/// Default heading styles (matches Mantine).
pub const DEFAULT_HEADINGS: HeadingStyles = HeadingStyles {
    h1: HeadingStyle {
        font_size: "2.125rem", // 34px
        line_height: "1.3",
        font_weight: "700",
    },
    h2: HeadingStyle {
        font_size: "1.625rem", // 26px
        line_height: "1.35",
        font_weight: "700",
    },
    h3: HeadingStyle {
        font_size: "1.375rem", // 22px
        line_height: "1.4",
        font_weight: "700",
    },
    h4: HeadingStyle {
        font_size: "1.125rem", // 18px
        line_height: "1.45",
        font_weight: "700",
    },
    h5: HeadingStyle {
        font_size: "1rem", // 16px
        line_height: "1.5",
        font_weight: "700",
    },
    h6: HeadingStyle {
        font_size: "0.875rem", // 14px
        line_height: "1.5",
        font_weight: "700",
    },
};

/// Default font family stack.
pub const DEFAULT_FONT_FAMILY: &str = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif, 'Apple Color Emoji', 'Segoe UI Emoji'";

/// Default monospace font family stack.
pub const DEFAULT_FONT_FAMILY_MONOSPACE: &str = "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace";
