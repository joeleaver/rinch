//! Spacing scale for the theme system.
//!
//! Provides consistent spacing values for margins, padding, and gaps.

/// A spacing size in the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingSize {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl SpacingSize {
    /// Get all spacing sizes.
    pub const fn all() -> &'static [SpacingSize] {
        &[
            SpacingSize::Xs,
            SpacingSize::Sm,
            SpacingSize::Md,
            SpacingSize::Lg,
            SpacingSize::Xl,
        ]
    }

    /// Get the CSS variable name for this size.
    pub const fn css_name(&self) -> &'static str {
        match self {
            SpacingSize::Xs => "xs",
            SpacingSize::Sm => "sm",
            SpacingSize::Md => "md",
            SpacingSize::Lg => "lg",
            SpacingSize::Xl => "xl",
        }
    }
}

impl std::fmt::Display for SpacingSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.css_name())
    }
}

impl std::str::FromStr for SpacingSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(SpacingSize::Xs),
            "sm" => Ok(SpacingSize::Sm),
            "md" => Ok(SpacingSize::Md),
            "lg" => Ok(SpacingSize::Lg),
            "xl" => Ok(SpacingSize::Xl),
            _ => Err(()),
        }
    }
}

/// The spacing scale.
#[derive(Debug, Clone, Copy)]
pub struct SpacingScale {
    pub xs: &'static str,
    pub sm: &'static str,
    pub md: &'static str,
    pub lg: &'static str,
    pub xl: &'static str,
}

impl SpacingScale {
    /// Get a spacing value by size.
    pub const fn get(&self, size: SpacingSize) -> &'static str {
        match size {
            SpacingSize::Xs => self.xs,
            SpacingSize::Sm => self.sm,
            SpacingSize::Md => self.md,
            SpacingSize::Lg => self.lg,
            SpacingSize::Xl => self.xl,
        }
    }
}

impl Default for SpacingScale {
    fn default() -> Self {
        DEFAULT_SPACING
    }
}

/// Default spacing scale (matches Mantine).
pub const DEFAULT_SPACING: SpacingScale = SpacingScale {
    xs: "0.625rem",  // 10px
    sm: "0.75rem",   // 12px
    md: "1rem",      // 16px
    lg: "1.25rem",   // 20px
    xl: "2rem",      // 32px
};
