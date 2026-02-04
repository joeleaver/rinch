//! Border radius scale for the theme system.
//!
//! Provides consistent border radius values for rounded corners.

/// A radius size in the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RadiusSize {
    Xs,
    #[default]
    Sm,
    Md,
    Lg,
    Xl,
}

impl RadiusSize {
    /// Get all radius sizes.
    pub const fn all() -> &'static [RadiusSize] {
        &[
            RadiusSize::Xs,
            RadiusSize::Sm,
            RadiusSize::Md,
            RadiusSize::Lg,
            RadiusSize::Xl,
        ]
    }

    /// Get the CSS variable name for this size.
    pub const fn css_name(&self) -> &'static str {
        match self {
            RadiusSize::Xs => "xs",
            RadiusSize::Sm => "sm",
            RadiusSize::Md => "md",
            RadiusSize::Lg => "lg",
            RadiusSize::Xl => "xl",
        }
    }
}

impl std::fmt::Display for RadiusSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.css_name())
    }
}

impl std::str::FromStr for RadiusSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(RadiusSize::Xs),
            "sm" => Ok(RadiusSize::Sm),
            "md" => Ok(RadiusSize::Md),
            "lg" => Ok(RadiusSize::Lg),
            "xl" => Ok(RadiusSize::Xl),
            _ => Err(()),
        }
    }
}

/// The border radius scale.
#[derive(Debug, Clone, Copy)]
pub struct RadiusScale {
    pub xs: &'static str,
    pub sm: &'static str,
    pub md: &'static str,
    pub lg: &'static str,
    pub xl: &'static str,
}

impl RadiusScale {
    /// Get a radius value by size.
    pub const fn get(&self, size: RadiusSize) -> &'static str {
        match size {
            RadiusSize::Xs => self.xs,
            RadiusSize::Sm => self.sm,
            RadiusSize::Md => self.md,
            RadiusSize::Lg => self.lg,
            RadiusSize::Xl => self.xl,
        }
    }
}

impl Default for RadiusScale {
    fn default() -> Self {
        DEFAULT_RADIUS
    }
}

/// Default border radius scale (matches Mantine).
pub const DEFAULT_RADIUS: RadiusScale = RadiusScale {
    xs: "0.125rem", // 2px
    sm: "0.25rem",  // 4px
    md: "0.5rem",   // 8px
    lg: "1rem",     // 16px
    xl: "2rem",     // 32px
};
