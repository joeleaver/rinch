//! Theme system for Rinch.
//!
//! Provides a comprehensive theming system with colors, spacing, typography,
//! and CSS variable generation. Inspired by [Mantine](https://mantine.dev/).
//!
//! # Quick Start
//!
//! ```rust
//! use rinch_theme::{Theme, generate_theme_css};
//!
//! // Create a default theme
//! let theme = Theme::default();
//!
//! // Generate CSS
//! let css = generate_theme_css(&theme);
//! ```
//!
//! # Customization
//!
//! Use the builder pattern to customize the theme:
//!
//! ```rust
//! use rinch_theme::{Theme, ColorName, RadiusSize};
//!
//! let theme = Theme::builder()
//!     .primary_color(ColorName::Cyan)
//!     .default_radius(RadiusSize::Md)
//!     .dark_mode(true)
//!     .build();
//! ```
//!
//! # CSS Variables
//!
//! The theme generates CSS custom properties (variables) that can be used
//! throughout your application:
//!
//! ```css
//! .my-button {
//!     background-color: var(--rinch-primary-color);
//!     padding: var(--rinch-spacing-sm) var(--rinch-spacing-md);
//!     border-radius: var(--rinch-radius-default);
//!     font-size: var(--rinch-font-size-sm);
//! }
//! ```
//!
//! # Available Variables
//!
//! ## Colors
//!
//! Each color has 10 shades (0-9):
//! - `--rinch-color-{name}-{shade}` (e.g., `--rinch-color-blue-5`)
//! - `--rinch-primary-color` - The primary color at the primary shade
//! - `--rinch-primary-color-{shade}` - Primary color aliases
//!
//! Available colors: dark, gray, red, pink, grape, violet, indigo, blue,
//! cyan, teal, green, lime, yellow, orange.
//!
//! ## Spacing
//!
//! - `--rinch-spacing-xs` (10px)
//! - `--rinch-spacing-sm` (12px)
//! - `--rinch-spacing-md` (16px)
//! - `--rinch-spacing-lg` (20px)
//! - `--rinch-spacing-xl` (32px)
//!
//! ## Border Radius
//!
//! - `--rinch-radius-xs` (2px)
//! - `--rinch-radius-sm` (4px)
//! - `--rinch-radius-md` (8px)
//! - `--rinch-radius-lg` (16px)
//! - `--rinch-radius-xl` (32px)
//! - `--rinch-radius-default` - The theme's default radius
//!
//! ## Font Sizes
//!
//! - `--rinch-font-size-xs` (12px)
//! - `--rinch-font-size-sm` (14px)
//! - `--rinch-font-size-md` (16px)
//! - `--rinch-font-size-lg` (18px)
//! - `--rinch-font-size-xl` (20px)
//!
//! ## Shadows
//!
//! - `--rinch-shadow-xs`
//! - `--rinch-shadow-sm`
//! - `--rinch-shadow-md`
//! - `--rinch-shadow-lg`
//! - `--rinch-shadow-xl`
//!
//! ## Semantic Colors
//!
//! - `--rinch-color-body` - Background color
//! - `--rinch-color-text` - Text color
//! - `--rinch-color-dimmed` - Dimmed/secondary text
//! - `--rinch-color-border` - Border color
//! - `--rinch-color-placeholder` - Placeholder text color

pub mod colors;
pub mod css;
pub mod radius;
pub mod shadows;
pub mod spacing;
pub mod theme;
pub mod typography;

// Re-export main types at crate root
pub use colors::{Color, ColorName, ColorPalette, ColorPalettes, DEFAULT_COLORS};
pub use css::{generate_base_styles, generate_css_variables, generate_theme_css};
pub use radius::{DEFAULT_RADIUS, RadiusScale, RadiusSize};
pub use shadows::{DEFAULT_SHADOWS, ShadowScale, ShadowSize};
pub use spacing::{DEFAULT_SPACING, SpacingScale, SpacingSize};
pub use theme::{CursorType, FocusRing, Theme, ThemeBuilder};
pub use typography::{
    DEFAULT_FONT_FAMILY, DEFAULT_FONT_FAMILY_MONOSPACE, DEFAULT_FONT_SIZES, DEFAULT_HEADINGS,
    DEFAULT_LINE_HEIGHTS, FontSize, FontSizeScale, HeadingLevel, HeadingStyle, HeadingStyles,
    LineHeightScale,
};
