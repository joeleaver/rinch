//! The colour value types and the format and notation names.

/// HSV color with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsva {
    /// Hue: 0-360
    pub h: f64,
    /// Saturation: 0-1
    pub s: f64,
    /// Value: 0-1
    pub v: f64,
    /// Alpha: 0-1
    pub a: f64,
}

/// RGB color with alpha (all channels 0-1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// HSL color with alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsla {
    /// Hue: 0-360
    pub h: f64,
    /// Saturation: 0-1
    pub s: f64,
    /// Lightness: 0-1
    pub l: f64,
    /// Alpha: 0-1
    pub a: f64,
}

/// Output format for color strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFormat {
    Hex,
    Hexa,
    Rgb,
    Rgba,
    Hsl,
    Hsla,
}

impl ColorFormat {
    /// The alpha-carrying counterpart of this format: `hex` → `hexa`, `rgb`
    /// → `rgba`, `hsl` → `hsla`; an alpha-carrying format is its own. The
    /// spelling for a surface that must show every channel a colour holds —
    /// a preview swatch under a display format that drops alpha.
    pub fn with_alpha(self) -> ColorFormat {
        match self {
            ColorFormat::Hex | ColorFormat::Hexa => ColorFormat::Hexa,
            ColorFormat::Rgb | ColorFormat::Rgba => ColorFormat::Rgba,
            ColorFormat::Hsl | ColorFormat::Hsla => ColorFormat::Hsla,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "hex" => Some(ColorFormat::Hex),
            "hexa" => Some(ColorFormat::Hexa),
            "rgb" => Some(ColorFormat::Rgb),
            "rgba" => Some(ColorFormat::Rgba),
            "hsl" => Some(ColorFormat::Hsl),
            "hsla" => Some(ColorFormat::Hsla),
            _ => None,
        }
    }
}

/// The family a colour string is *written* in — what
/// [`parse_color_with_notation`](super::parse_color_with_notation) reports.
///
/// Distinct from [`ColorFormat`], which names an output spelling and has
/// alpha-dropping variants: a notation never drops a channel, so a consumer
/// judging two strings at a notation's resolution keeps every channel, and
/// an inbound value that differs in alpha alone still reads as different.
/// [`Notation::with_alpha`] is the serializer that writes the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notation {
    /// `#rrggbb`-family hex, with or without the `#`, and the CSS named
    /// colours: 8-bit channels, alpha included.
    Hex,
    /// `rgb()` / `rgba()`: 8-bit channels, alpha at two decimals.
    Rgb,
    /// `hsl()` / `hsla()`: whole degrees and whole percents, alpha at two
    /// decimals.
    Hsl,
}

impl Notation {
    /// The alpha-carrying [`ColorFormat`] whose serializer writes this family
    /// — the resolution a colour has once it has been through
    /// [`format_color`](super::format_color) in this notation.
    pub fn with_alpha(self) -> ColorFormat {
        match self {
            Notation::Hex => ColorFormat::Hexa,
            Notation::Rgb => ColorFormat::Rgba,
            Notation::Hsl => ColorFormat::Hsla,
        }
    }
}
