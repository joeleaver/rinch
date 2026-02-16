//! CSS value type enums used by ComputedStyle.

use serde::Serialize;

use crate::layout::Viewport;

/// Custom serialization for Option<peniko::Color> as hex string
pub(crate) mod color_serde {
    use peniko::Color;
    use serde::Serializer;

    pub fn serialize<S>(color: &Option<Color>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match color {
            Some(c) => {
                // Convert to RGBA8 struct
                let rgba = c.to_rgba8();
                if rgba.a == 255 {
                    serializer
                        .serialize_str(&format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b))
                } else {
                    serializer.serialize_str(&format!(
                        "#{:02x}{:02x}{:02x}{:02x}",
                        rgba.r, rgba.g, rgba.b, rgba.a
                    ))
                }
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn serialize_direct<S>(color: &Color, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let rgba = color.to_rgba8();
        if rgba.a == 255 {
            serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b))
        } else {
            serializer.serialize_str(&format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                rgba.r, rgba.g, rgba.b, rgba.a
            ))
        }
    }
}

// =============================================================================
// Value Enums
// =============================================================================

/// CSS display property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum DisplayValue {
    #[default]
    Flex,
    Block,
    Grid,
    None,
    Contents,
    Inline,
    InlineBlock,
    InlineFlex,
}

impl DisplayValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "flex" => Self::Flex,
            "block" => Self::Block,
            "grid" => Self::Grid,
            "none" => Self::None,
            "contents" => Self::Contents,
            "inline" => Self::Inline,
            "inline-block" => Self::InlineBlock,
            "inline-flex" => Self::InlineFlex,
            _ => Self::default(),
        }
    }

    /// Convert to Taffy Display.
    pub fn to_taffy(&self) -> taffy::Display {
        match self {
            Self::Flex | Self::InlineFlex => taffy::Display::Flex,
            Self::Block => taffy::Display::Block,
            Self::Grid => taffy::Display::Grid,
            Self::None => taffy::Display::None,
            Self::Contents => taffy::Display::Flex, // transparent container
            Self::Inline => taffy::Display::Block,  // inline handled by IFC
            Self::InlineBlock => taffy::Display::Flex,
        }
    }
}

/// CSS position property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum PositionValue {
    #[default]
    Relative,
    Absolute,
    Fixed,
    Static,
    Sticky,
}

impl PositionValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "absolute" => Self::Absolute,
            "fixed" => Self::Fixed,
            "static" => Self::Static,
            "sticky" => Self::Sticky,
            _ => Self::Relative,
        }
    }

    /// Convert to Taffy Position.
    pub fn to_taffy(&self) -> taffy::Position {
        match self {
            Self::Absolute | Self::Fixed => taffy::Position::Absolute,
            Self::Relative | Self::Static | Self::Sticky => taffy::Position::Relative,
        }
    }
}

/// CSS dimension value (width, height, min-*, max-*).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub enum DimensionValue {
    #[default]
    Auto,
    Length(f32),
    Percent(f32),
}

impl DimensionValue {
    /// Parse from CSS string value with viewport support.
    pub fn parse(value: &str, viewport: &Viewport) -> Self {
        let value = value.trim();
        if value == "auto" {
            return Self::Auto;
        }
        // Viewport units
        if let Some(num_str) = value.strip_suffix("vh")
            && let Ok(v) = num_str.trim().parse::<f32>()
        {
            return Self::Length(v * viewport.height / 100.0);
        }
        if let Some(num_str) = value.strip_suffix("vw")
            && let Ok(v) = num_str.trim().parse::<f32>()
        {
            return Self::Length(v * viewport.width / 100.0);
        }
        // Percentage
        if let Some(pct) = value.strip_suffix('%')
            && let Ok(v) = pct.trim().parse::<f32>()
        {
            return Self::Percent(v / 100.0);
        }
        // Pixels
        if let Some(px) = value.strip_suffix("px")
            && let Ok(v) = px.trim().parse::<f32>()
        {
            return Self::Length(v);
        }
        // Rem
        if let Some(rem_str) = value.strip_suffix("rem")
            && let Ok(v) = rem_str.trim().parse::<f32>()
        {
            return Self::Length(v * 16.0);
        }
        // Plain number
        if let Ok(v) = value.parse::<f32>() {
            return Self::Length(v);
        }
        Self::Auto
    }

    /// Convert to Taffy Dimension.
    pub fn to_taffy(&self) -> taffy::Dimension {
        match self {
            Self::Auto => taffy::Dimension::auto(),
            Self::Length(v) => taffy::Dimension::length(*v),
            Self::Percent(v) => taffy::Dimension::percent(*v),
        }
    }
}

/// CSS length-percentage value (padding, border-width, gap).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub enum LengthPercentageValue {
    #[default]
    Zero,
    Length(f32),
    Percent(f32),
}

impl LengthPercentageValue {
    /// Parse from CSS string value with viewport support.
    pub fn parse(value: &str, viewport: &Viewport) -> Self {
        let value = value.trim();
        // Viewport units
        if let Some(num_str) = value.strip_suffix("vh")
            && let Ok(v) = num_str.trim().parse::<f32>()
        {
            return Self::Length(v * viewport.height / 100.0);
        }
        if let Some(num_str) = value.strip_suffix("vw")
            && let Ok(v) = num_str.trim().parse::<f32>()
        {
            return Self::Length(v * viewport.width / 100.0);
        }
        // Percentage
        if let Some(pct) = value.strip_suffix('%')
            && let Ok(v) = pct.trim().parse::<f32>()
        {
            return Self::Percent(v / 100.0);
        }
        // Pixels
        if let Some(px) = value.strip_suffix("px")
            && let Ok(v) = px.trim().parse::<f32>()
        {
            return Self::Length(v);
        }
        // Rem
        if let Some(rem_str) = value.strip_suffix("rem")
            && let Ok(v) = rem_str.trim().parse::<f32>()
        {
            return Self::Length(v * 16.0);
        }
        // Plain number
        if let Ok(v) = value.parse::<f32>() {
            return Self::Length(v);
        }
        Self::Zero
    }

    /// Convert to Taffy LengthPercentage.
    pub fn to_taffy(&self) -> taffy::LengthPercentage {
        match self {
            Self::Zero => taffy::LengthPercentage::length(0.0),
            Self::Length(v) => taffy::LengthPercentage::length(*v),
            Self::Percent(v) => taffy::LengthPercentage::percent(*v),
        }
    }

    /// Get as f32 length (resolving percentage against container size).
    pub fn resolve(&self, container_size: f32) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Length(v) => *v,
            Self::Percent(v) => *v * container_size,
        }
    }

    /// Get as f32 length (percentages return 0.0).
    pub fn to_px(&self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Length(v) => *v,
            Self::Percent(_) => 0.0, // Percentages need container size
        }
    }
}

/// CSS length-percentage-auto value (margin, inset).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub enum LengthPercentageAutoValue {
    #[default]
    Auto,
    Length(f32),
    Percent(f32),
}

impl LengthPercentageAutoValue {
    /// Parse from CSS string value with viewport support.
    pub fn parse(value: &str, viewport: &Viewport) -> Self {
        let value = value.trim();
        if value == "auto" {
            return Self::Auto;
        }
        // Viewport units
        if let Some(num_str) = value.strip_suffix("vh")
            && let Ok(v) = num_str.trim().parse::<f32>()
        {
            return Self::Length(v * viewport.height / 100.0);
        }
        if let Some(num_str) = value.strip_suffix("vw")
            && let Ok(v) = num_str.trim().parse::<f32>()
        {
            return Self::Length(v * viewport.width / 100.0);
        }
        // Percentage
        if let Some(pct) = value.strip_suffix('%')
            && let Ok(v) = pct.trim().parse::<f32>()
        {
            return Self::Percent(v / 100.0);
        }
        // Pixels
        if let Some(px) = value.strip_suffix("px")
            && let Ok(v) = px.trim().parse::<f32>()
        {
            return Self::Length(v);
        }
        // Rem
        if let Some(rem_str) = value.strip_suffix("rem")
            && let Ok(v) = rem_str.trim().parse::<f32>()
        {
            return Self::Length(v * 16.0);
        }
        // Plain number
        if let Ok(v) = value.parse::<f32>() {
            return Self::Length(v);
        }
        Self::Auto
    }

    /// Convert to Taffy LengthPercentageAuto.
    pub fn to_taffy(&self) -> taffy::LengthPercentageAuto {
        match self {
            Self::Auto => taffy::LengthPercentageAuto::auto(),
            Self::Length(v) => taffy::LengthPercentageAuto::length(*v),
            Self::Percent(v) => taffy::LengthPercentageAuto::percent(*v),
        }
    }

    /// Get as f32 length (returns 0.0 for Auto or Percent).
    pub fn to_px(&self) -> f32 {
        match self {
            Self::Length(v) => *v,
            Self::Auto | Self::Percent(_) => 0.0,
        }
    }
}

/// CSS flex-direction property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum FlexDirectionValue {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

impl FlexDirectionValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "column" => Self::Column,
            "row-reverse" => Self::RowReverse,
            "column-reverse" => Self::ColumnReverse,
            _ => Self::Row,
        }
    }

    /// Convert to Taffy FlexDirection.
    pub fn to_taffy(&self) -> taffy::FlexDirection {
        match self {
            Self::Row => taffy::FlexDirection::Row,
            Self::Column => taffy::FlexDirection::Column,
            Self::RowReverse => taffy::FlexDirection::RowReverse,
            Self::ColumnReverse => taffy::FlexDirection::ColumnReverse,
        }
    }
}

/// CSS flex-wrap property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum FlexWrapValue {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

impl FlexWrapValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "wrap" => Self::Wrap,
            "wrap-reverse" => Self::WrapReverse,
            _ => Self::NoWrap,
        }
    }

    /// Convert to Taffy FlexWrap.
    pub fn to_taffy(&self) -> taffy::FlexWrap {
        match self {
            Self::NoWrap => taffy::FlexWrap::NoWrap,
            Self::Wrap => taffy::FlexWrap::Wrap,
            Self::WrapReverse => taffy::FlexWrap::WrapReverse,
        }
    }
}

/// CSS align-items property values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum AlignItemsValue {
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Stretch,
}

impl AlignItemsValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "flex-start" | "start" => Self::FlexStart,
            "flex-end" | "end" => Self::FlexEnd,
            "center" => Self::Center,
            "baseline" => Self::Baseline,
            "stretch" => Self::Stretch,
            _ => return None,
        })
    }

    /// Convert to Taffy AlignItems.
    pub fn to_taffy(&self) -> taffy::AlignItems {
        match self {
            Self::FlexStart => taffy::AlignItems::FlexStart,
            Self::FlexEnd => taffy::AlignItems::FlexEnd,
            Self::Center => taffy::AlignItems::Center,
            Self::Baseline => taffy::AlignItems::Baseline,
            Self::Stretch => taffy::AlignItems::Stretch,
        }
    }
}

/// CSS align-self property values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum AlignSelfValue {
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    Stretch,
}

impl AlignSelfValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "flex-start" | "start" => Self::FlexStart,
            "flex-end" | "end" => Self::FlexEnd,
            "center" => Self::Center,
            "baseline" => Self::Baseline,
            "stretch" => Self::Stretch,
            _ => return None,
        })
    }

    /// Convert to Taffy AlignSelf.
    pub fn to_taffy(&self) -> taffy::AlignSelf {
        match self {
            Self::FlexStart => taffy::AlignSelf::FlexStart,
            Self::FlexEnd => taffy::AlignSelf::FlexEnd,
            Self::Center => taffy::AlignSelf::Center,
            Self::Baseline => taffy::AlignSelf::Baseline,
            Self::Stretch => taffy::AlignSelf::Stretch,
        }
    }
}

/// CSS justify-content property values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum JustifyContentValue {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl JustifyContentValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "flex-start" | "start" => Self::FlexStart,
            "flex-end" | "end" => Self::FlexEnd,
            "center" => Self::Center,
            "space-between" => Self::SpaceBetween,
            "space-around" => Self::SpaceAround,
            "space-evenly" => Self::SpaceEvenly,
            _ => return None,
        })
    }

    /// Convert to Taffy JustifyContent.
    pub fn to_taffy(&self) -> taffy::JustifyContent {
        match self {
            Self::FlexStart => taffy::JustifyContent::FlexStart,
            Self::FlexEnd => taffy::JustifyContent::FlexEnd,
            Self::Center => taffy::JustifyContent::Center,
            Self::SpaceBetween => taffy::JustifyContent::SpaceBetween,
            Self::SpaceAround => taffy::JustifyContent::SpaceAround,
            Self::SpaceEvenly => taffy::JustifyContent::SpaceEvenly,
        }
    }
}

/// CSS overflow property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum OverflowValue {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Clip,
    Auto,
}

impl OverflowValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "hidden" => Self::Hidden,
            "scroll" => Self::Scroll,
            "clip" => Self::Clip,
            "auto" => Self::Auto,
            _ => Self::Visible,
        }
    }

    /// Convert to Taffy Overflow.
    pub fn to_taffy(&self) -> taffy::Overflow {
        match self {
            Self::Visible => taffy::Overflow::Visible,
            Self::Hidden => taffy::Overflow::Hidden,
            Self::Scroll | Self::Auto => taffy::Overflow::Scroll,
            Self::Clip => taffy::Overflow::Clip,
        }
    }
}

/// CSS font-style property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum FontStyleValue {
    #[default]
    Normal,
    Italic,
    Oblique,
}

impl FontStyleValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "italic" => Self::Italic,
            "oblique" => Self::Oblique,
            _ => Self::Normal,
        }
    }

    /// Convert to Parley FontStyle.
    pub fn to_parley(&self) -> parley::style::FontStyle {
        match self {
            Self::Normal => parley::style::FontStyle::Normal,
            Self::Italic => parley::style::FontStyle::Italic,
            Self::Oblique => parley::style::FontStyle::Oblique(None),
        }
    }
}

/// CSS line-height property values.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub enum LineHeightValue {
    #[default]
    Normal,
    /// Absolute value in pixels.
    Absolute(f32),
    /// Relative multiplier of font-size.
    Relative(f32),
}

impl LineHeightValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        let value = value.trim();
        if value == "normal" || value.is_empty() {
            return Self::Normal;
        }
        if let Some(px) = value.strip_suffix("px")
            && let Ok(v) = px.trim().parse::<f32>()
        {
            return Self::Absolute(v);
        }
        // Unitless = relative multiplier
        if let Ok(v) = value.parse::<f32>() {
            return Self::Relative(v);
        }
        Self::Normal
    }

    /// Convert to Parley LineHeight.
    pub fn to_parley(&self) -> Option<parley::style::LineHeight> {
        match self {
            Self::Normal => None,
            Self::Absolute(v) => Some(parley::style::LineHeight::Absolute(*v)),
            Self::Relative(v) => Some(parley::style::LineHeight::FontSizeRelative(*v)),
        }
    }
}

/// CSS text-align property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum TextAlignValue {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

impl TextAlignValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "center" => Self::Center,
            "right" | "end" => Self::End,
            "justify" => Self::Justify,
            _ => Self::Start,
        }
    }

    /// Convert to Parley Alignment.
    pub fn to_parley(&self) -> parley::layout::Alignment {
        match self {
            Self::Start => parley::layout::Alignment::Start,
            Self::Center => parley::layout::Alignment::Center,
            Self::End => parley::layout::Alignment::End,
            Self::Justify => parley::layout::Alignment::Justify,
        }
    }
}

/// CSS text-decoration property values.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TextDecorationValue {
    pub underline: bool,
    pub strikethrough: bool,
}

impl TextDecorationValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        let value = value.trim().to_lowercase();
        Self {
            underline: value.contains("underline"),
            strikethrough: value.contains("line-through"),
        }
    }
}

/// CSS white-space property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum WhiteSpaceValue {
    #[default]
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
}

impl WhiteSpaceValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "nowrap" => Self::NoWrap,
            "pre" => Self::Pre,
            "pre-wrap" => Self::PreWrap,
            "pre-line" => Self::PreLine,
            _ => Self::Normal,
        }
    }
}

/// CSS overflow-wrap (word-wrap) property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum OverflowWrapValue {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

impl OverflowWrapValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "break-word" => Self::BreakWord,
            "anywhere" => Self::Anywhere,
            _ => Self::Normal,
        }
    }

    /// Convert to Parley's OverflowWrap type.
    pub fn to_parley(self) -> parley::style::OverflowWrap {
        match self {
            Self::Normal => parley::style::OverflowWrap::Normal,
            Self::BreakWord => parley::style::OverflowWrap::BreakWord,
            Self::Anywhere => parley::style::OverflowWrap::Anywhere,
        }
    }
}

/// CSS border-style property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum BorderStyleValue {
    #[default]
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Hidden,
}

/// CSS visibility property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum VisibilityValue {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

/// CSS cursor property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum CursorValue {
    #[default]
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    Crosshair,
    Grab,
    Grabbing,
    ColResize,
    RowResize,
    NResize,
    SResize,
    EResize,
    WResize,
    NeResize,
    NwResize,
    SeResize,
    SwResize,
    EwResize,
    NsResize,
    Wait,
    Progress,
    Help,
    ZoomIn,
    ZoomOut,
    None,
}

/// CSS pointer-events property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum PointerEventsValue {
    #[default]
    Auto,
    None,
}

/// A single text-shadow value.
#[derive(Debug, Clone, Serialize)]
pub struct TextShadowValue {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
}

/// Pre-computed 2D affine transform.
#[derive(Debug, Clone, Serialize)]
pub struct TransformValue {
    /// Pre-computed 2D affine matrix [a, b, c, d, e, f].
    pub matrix: [f64; 6],
    /// Whether this is the identity transform (no-op).
    pub is_identity: bool,
}

impl Default for TransformValue {
    fn default() -> Self {
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            is_identity: true,
        }
    }
}

/// CSS background value — solid color or gradient.
#[derive(Debug, Clone, Default, Serialize)]
pub enum BackgroundValue {
    #[default]
    None,
    Color(#[serde(serialize_with = "color_serde::serialize_direct")] peniko::Color),
    LinearGradient {
        angle_degrees: f32,
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        stops: Vec<GradientStop>,
    },
}

/// A gradient color stop.
#[derive(Debug, Clone, Serialize)]
pub struct GradientStop {
    pub offset: f32,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
}
