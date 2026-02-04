//! Color palettes for the theme system.
//!
//! Colors are organized as palettes with 10 shades (0-9), where:
//! - 0: Lightest shade (background tints)
//! - 5: Primary shade (default for filled variants)
//! - 9: Darkest shade (text on light backgrounds)

/// A color in the theme system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    /// The color as a hex string (e.g., "#228be6").
    hex: &'static str,
}

impl Color {
    /// Create a new color from a hex string.
    pub const fn new(hex: &'static str) -> Self {
        Self { hex }
    }

    /// Get the hex value of the color.
    pub const fn hex(&self) -> &'static str {
        self.hex
    }
}

/// A palette of 10 color shades.
#[derive(Debug, Clone, Copy)]
pub struct ColorPalette {
    shades: [Color; 10],
}

impl ColorPalette {
    /// Create a new color palette from 10 hex strings.
    pub const fn new(shades: [&'static str; 10]) -> Self {
        Self {
            shades: [
                Color::new(shades[0]),
                Color::new(shades[1]),
                Color::new(shades[2]),
                Color::new(shades[3]),
                Color::new(shades[4]),
                Color::new(shades[5]),
                Color::new(shades[6]),
                Color::new(shades[7]),
                Color::new(shades[8]),
                Color::new(shades[9]),
            ],
        }
    }

    /// Get a shade by index (0-9).
    pub const fn shade(&self, index: usize) -> Color {
        self.shades[index]
    }

    /// Get all shades.
    pub const fn shades(&self) -> &[Color; 10] {
        &self.shades
    }
}

/// Available color names in the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorName {
    Dark,
    Gray,
    Red,
    Pink,
    Grape,
    Violet,
    Indigo,
    Blue,
    Cyan,
    Teal,
    Green,
    Lime,
    Yellow,
    Orange,
}

impl ColorName {
    /// Get all available color names.
    pub const fn all() -> &'static [ColorName] {
        &[
            ColorName::Dark,
            ColorName::Gray,
            ColorName::Red,
            ColorName::Pink,
            ColorName::Grape,
            ColorName::Violet,
            ColorName::Indigo,
            ColorName::Blue,
            ColorName::Cyan,
            ColorName::Teal,
            ColorName::Green,
            ColorName::Lime,
            ColorName::Yellow,
            ColorName::Orange,
        ]
    }

    /// Get the CSS variable name for this color.
    pub const fn css_name(&self) -> &'static str {
        match self {
            ColorName::Dark => "dark",
            ColorName::Gray => "gray",
            ColorName::Red => "red",
            ColorName::Pink => "pink",
            ColorName::Grape => "grape",
            ColorName::Violet => "violet",
            ColorName::Indigo => "indigo",
            ColorName::Blue => "blue",
            ColorName::Cyan => "cyan",
            ColorName::Teal => "teal",
            ColorName::Green => "green",
            ColorName::Lime => "lime",
            ColorName::Yellow => "yellow",
            ColorName::Orange => "orange",
        }
    }
}

impl std::fmt::Display for ColorName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.css_name())
    }
}

impl std::str::FromStr for ColorName {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dark" => Ok(ColorName::Dark),
            "gray" | "grey" => Ok(ColorName::Gray),
            "red" => Ok(ColorName::Red),
            "pink" => Ok(ColorName::Pink),
            "grape" => Ok(ColorName::Grape),
            "violet" => Ok(ColorName::Violet),
            "indigo" => Ok(ColorName::Indigo),
            "blue" => Ok(ColorName::Blue),
            "cyan" => Ok(ColorName::Cyan),
            "teal" => Ok(ColorName::Teal),
            "green" => Ok(ColorName::Green),
            "lime" => Ok(ColorName::Lime),
            "yellow" => Ok(ColorName::Yellow),
            "orange" => Ok(ColorName::Orange),
            _ => Err(()),
        }
    }
}

/// All color palettes in the theme.
#[derive(Debug, Clone, Copy)]
pub struct ColorPalettes {
    pub dark: ColorPalette,
    pub gray: ColorPalette,
    pub red: ColorPalette,
    pub pink: ColorPalette,
    pub grape: ColorPalette,
    pub violet: ColorPalette,
    pub indigo: ColorPalette,
    pub blue: ColorPalette,
    pub cyan: ColorPalette,
    pub teal: ColorPalette,
    pub green: ColorPalette,
    pub lime: ColorPalette,
    pub yellow: ColorPalette,
    pub orange: ColorPalette,
}

impl ColorPalettes {
    /// Get a palette by name.
    pub const fn get(&self, name: ColorName) -> &ColorPalette {
        match name {
            ColorName::Dark => &self.dark,
            ColorName::Gray => &self.gray,
            ColorName::Red => &self.red,
            ColorName::Pink => &self.pink,
            ColorName::Grape => &self.grape,
            ColorName::Violet => &self.violet,
            ColorName::Indigo => &self.indigo,
            ColorName::Blue => &self.blue,
            ColorName::Cyan => &self.cyan,
            ColorName::Teal => &self.teal,
            ColorName::Green => &self.green,
            ColorName::Lime => &self.lime,
            ColorName::Yellow => &self.yellow,
            ColorName::Orange => &self.orange,
        }
    }
}

impl Default for ColorPalettes {
    fn default() -> Self {
        DEFAULT_COLORS
    }
}

/// Default color palettes (Mantine colors).
pub const DEFAULT_COLORS: ColorPalettes = ColorPalettes {
    dark: ColorPalette::new([
        "#C9C9C9", // 0
        "#b8b8b8", // 1
        "#828282", // 2
        "#696969", // 3
        "#424242", // 4
        "#3b3b3b", // 5
        "#2e2e2e", // 6
        "#242424", // 7
        "#1f1f1f", // 8
        "#141414", // 9
    ]),
    gray: ColorPalette::new([
        "#f8f9fa", // 0
        "#f1f3f5", // 1
        "#e9ecef", // 2
        "#dee2e6", // 3
        "#ced4da", // 4
        "#adb5bd", // 5
        "#868e96", // 6
        "#495057", // 7
        "#343a40", // 8
        "#212529", // 9
    ]),
    red: ColorPalette::new([
        "#fff5f5", // 0
        "#ffe3e3", // 1
        "#ffc9c9", // 2
        "#ffa8a8", // 3
        "#ff8787", // 4
        "#ff6b6b", // 5
        "#fa5252", // 6
        "#f03e3e", // 7
        "#e03131", // 8
        "#c92a2a", // 9
    ]),
    pink: ColorPalette::new([
        "#fff0f6", // 0
        "#ffdeeb", // 1
        "#fcc2d7", // 2
        "#faa2c1", // 3
        "#f783ac", // 4
        "#f06595", // 5
        "#e64980", // 6
        "#d6336c", // 7
        "#c2255c", // 8
        "#a61e4d", // 9
    ]),
    grape: ColorPalette::new([
        "#f8f0fc", // 0
        "#f3d9fa", // 1
        "#eebefa", // 2
        "#e599f7", // 3
        "#da77f2", // 4
        "#cc5de8", // 5
        "#be4bdb", // 6
        "#ae3ec9", // 7
        "#9c36b5", // 8
        "#862e9c", // 9
    ]),
    violet: ColorPalette::new([
        "#f3f0ff", // 0
        "#e5dbff", // 1
        "#d0bfff", // 2
        "#b197fc", // 3
        "#9775fa", // 4
        "#845ef7", // 5
        "#7950f2", // 6
        "#7048e8", // 7
        "#6741d9", // 8
        "#5f3dc4", // 9
    ]),
    indigo: ColorPalette::new([
        "#edf2ff", // 0
        "#dbe4ff", // 1
        "#bac8ff", // 2
        "#91a7ff", // 3
        "#748ffc", // 4
        "#5c7cfa", // 5
        "#4c6ef5", // 6
        "#4263eb", // 7
        "#3b5bdb", // 8
        "#364fc7", // 9
    ]),
    blue: ColorPalette::new([
        "#e7f5ff", // 0
        "#d0ebff", // 1
        "#a5d8ff", // 2
        "#74c0fc", // 3
        "#4dabf7", // 4
        "#339af0", // 5
        "#228be6", // 6
        "#1c7ed6", // 7
        "#1971c2", // 8
        "#1864ab", // 9
    ]),
    cyan: ColorPalette::new([
        "#e3fafc", // 0
        "#c5f6fa", // 1
        "#99e9f2", // 2
        "#66d9e8", // 3
        "#3bc9db", // 4
        "#22b8cf", // 5
        "#15aabf", // 6
        "#1098ad", // 7
        "#0c8599", // 8
        "#0b7285", // 9
    ]),
    teal: ColorPalette::new([
        "#e6fcf5", // 0
        "#c3fae8", // 1
        "#96f2d7", // 2
        "#63e6be", // 3
        "#38d9a9", // 4
        "#20c997", // 5
        "#12b886", // 6
        "#0ca678", // 7
        "#099268", // 8
        "#087f5b", // 9
    ]),
    green: ColorPalette::new([
        "#ebfbee", // 0
        "#d3f9d8", // 1
        "#b2f2bb", // 2
        "#8ce99a", // 3
        "#69db7c", // 4
        "#51cf66", // 5
        "#40c057", // 6
        "#37b24d", // 7
        "#2f9e44", // 8
        "#2b8a3e", // 9
    ]),
    lime: ColorPalette::new([
        "#f4fce3", // 0
        "#e9fac8", // 1
        "#d8f5a2", // 2
        "#c0eb75", // 3
        "#a9e34b", // 4
        "#94d82d", // 5
        "#82c91e", // 6
        "#74b816", // 7
        "#66a80f", // 8
        "#5c940d", // 9
    ]),
    yellow: ColorPalette::new([
        "#fff9db", // 0
        "#fff3bf", // 1
        "#ffec99", // 2
        "#ffe066", // 3
        "#ffd43b", // 4
        "#fcc419", // 5
        "#fab005", // 6
        "#f59f00", // 7
        "#f08c00", // 8
        "#e67700", // 9
    ]),
    orange: ColorPalette::new([
        "#fff4e6", // 0
        "#ffe8cc", // 1
        "#ffd8a8", // 2
        "#ffc078", // 3
        "#ffa94d", // 4
        "#ff922b", // 5
        "#fd7e14", // 6
        "#f76707", // 7
        "#e8590c", // 8
        "#d9480f", // 9
    ]),
};
