//! Shadow definitions for the theme system.
//!
//! Provides consistent box-shadow values for elevation effects.

/// A shadow size in the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowSize {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl ShadowSize {
    /// Get all shadow sizes.
    pub const fn all() -> &'static [ShadowSize] {
        &[
            ShadowSize::Xs,
            ShadowSize::Sm,
            ShadowSize::Md,
            ShadowSize::Lg,
            ShadowSize::Xl,
        ]
    }

    /// Get the CSS variable name for this size.
    pub const fn css_name(&self) -> &'static str {
        match self {
            ShadowSize::Xs => "xs",
            ShadowSize::Sm => "sm",
            ShadowSize::Md => "md",
            ShadowSize::Lg => "lg",
            ShadowSize::Xl => "xl",
        }
    }
}

impl std::fmt::Display for ShadowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.css_name())
    }
}

impl std::str::FromStr for ShadowSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ShadowSize::Xs),
            "sm" => Ok(ShadowSize::Sm),
            "md" => Ok(ShadowSize::Md),
            "lg" => Ok(ShadowSize::Lg),
            "xl" => Ok(ShadowSize::Xl),
            _ => Err(()),
        }
    }
}

/// The shadow scale.
#[derive(Debug, Clone, Copy)]
pub struct ShadowScale {
    pub xs: &'static str,
    pub sm: &'static str,
    pub md: &'static str,
    pub lg: &'static str,
    pub xl: &'static str,
}

impl ShadowScale {
    /// Get a shadow value by size.
    pub const fn get(&self, size: ShadowSize) -> &'static str {
        match size {
            ShadowSize::Xs => self.xs,
            ShadowSize::Sm => self.sm,
            ShadowSize::Md => self.md,
            ShadowSize::Lg => self.lg,
            ShadowSize::Xl => self.xl,
        }
    }
}

impl Default for ShadowScale {
    fn default() -> Self {
        DEFAULT_SHADOWS
    }
}

/// Default shadow scale (matches Mantine).
pub const DEFAULT_SHADOWS: ShadowScale = ShadowScale {
    xs: "0 0.0625rem 0.1875rem rgba(0, 0, 0, 0.05), 0 0.0625rem 0.125rem rgba(0, 0, 0, 0.1)",
    sm: "0 0.0625rem 0.1875rem rgba(0, 0, 0, 0.05), rgba(0, 0, 0, 0.05) 0 0.625rem 0.9375rem -0.3125rem, rgba(0, 0, 0, 0.04) 0 0.4375rem 0.4375rem -0.3125rem",
    md: "0 0.0625rem 0.1875rem rgba(0, 0, 0, 0.05), rgba(0, 0, 0, 0.05) 0 1.25rem 1.5625rem -0.3125rem, rgba(0, 0, 0, 0.04) 0 0.625rem 0.625rem -0.3125rem",
    lg: "0 0.0625rem 0.1875rem rgba(0, 0, 0, 0.05), rgba(0, 0, 0, 0.05) 0 1.75rem 1.4375rem -0.4375rem, rgba(0, 0, 0, 0.04) 0 0.75rem 0.75rem -0.4375rem",
    xl: "0 0.0625rem 0.1875rem rgba(0, 0, 0, 0.05), rgba(0, 0, 0, 0.05) 0 2.25rem 1.75rem -0.4375rem, rgba(0, 0, 0, 0.04) 0 1.0625rem 1.0625rem -0.4375rem",
};
