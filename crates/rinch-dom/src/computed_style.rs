//! Typed computed style representation.
//!
//! Provides a pre-parsed ComputedStyle struct to avoid re-parsing CSS properties
//! on every layout and paint operation.

use std::collections::HashMap;

use serde::Serialize;

use crate::layout::{Viewport, parse_color};
use style::properties::ComputedValues;

// Stylo grid types
use style::computed_values::grid_auto_flow::T as GridAutoFlow;
use style::values::computed::GridTemplateComponent;
use style::values::generics::grid::{RepeatCount, TrackBreadth, TrackListValue, TrackSize};
use style::values::specified::GenericGridTemplateComponent;

/// Custom serialization for Option<peniko::Color> as hex string
mod color_serde {
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

// =============================================================================
// ComputedStyle Struct
// =============================================================================

/// Pre-parsed CSS properties for efficient layout and paint.
#[derive(Debug, Clone, Serialize)]
pub struct ComputedStyle {
    // Display/position
    pub display: DisplayValue,
    pub position: PositionValue,
    pub overflow_x: OverflowValue,
    pub overflow_y: OverflowValue,

    // Dimensions
    pub width: DimensionValue,
    pub height: DimensionValue,
    pub min_width: DimensionValue,
    pub min_height: DimensionValue,
    pub max_width: DimensionValue,
    pub max_height: DimensionValue,

    // Flexbox
    pub flex_direction: FlexDirectionValue,
    pub flex_wrap: FlexWrapValue,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: DimensionValue,
    pub align_items: Option<AlignItemsValue>,
    pub align_self: Option<AlignSelfValue>,
    pub justify_content: Option<JustifyContentValue>,

    // Spacing - padding
    pub padding_top: LengthPercentageValue,
    pub padding_right: LengthPercentageValue,
    pub padding_bottom: LengthPercentageValue,
    pub padding_left: LengthPercentageValue,

    // Spacing - margin
    pub margin_top: LengthPercentageAutoValue,
    pub margin_right: LengthPercentageAutoValue,
    pub margin_bottom: LengthPercentageAutoValue,
    pub margin_left: LengthPercentageAutoValue,

    // Spacing - gap
    pub gap_row: LengthPercentageValue,
    pub gap_column: LengthPercentageValue,

    // Positioning (inset)
    pub top: LengthPercentageAutoValue,
    pub right: LengthPercentageAutoValue,
    pub bottom: LengthPercentageAutoValue,
    pub left: LengthPercentageAutoValue,

    // Border widths
    pub border_top_width: LengthPercentageValue,
    pub border_right_width: LengthPercentageValue,
    pub border_bottom_width: LengthPercentageValue,
    pub border_left_width: LengthPercentageValue,

    // Border radius (can be percentage, resolved at paint time)
    pub border_radius_top_left: LengthPercentageValue,
    pub border_radius_top_right: LengthPercentageValue,
    pub border_radius_bottom_right: LengthPercentageValue,
    pub border_radius_bottom_left: LengthPercentageValue,

    // Border styles (per-side)
    pub border_top_style: BorderStyleValue,
    pub border_right_style: BorderStyleValue,
    pub border_bottom_style: BorderStyleValue,
    pub border_left_style: BorderStyleValue,

    // Border colors (per-side)
    #[serde(serialize_with = "color_serde::serialize")]
    pub border_top_color: Option<peniko::Color>,
    #[serde(serialize_with = "color_serde::serialize")]
    pub border_right_color: Option<peniko::Color>,
    #[serde(serialize_with = "color_serde::serialize")]
    pub border_bottom_color: Option<peniko::Color>,
    #[serde(serialize_with = "color_serde::serialize")]
    pub border_left_color: Option<peniko::Color>,

    // Colors
    pub background: BackgroundValue,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,

    // Visual
    pub opacity: f32,
    pub visibility: VisibilityValue,

    // Transforms
    pub transform: TransformValue,
    pub transform_origin_x: LengthPercentageValue,
    pub transform_origin_y: LengthPercentageValue,

    // Z-index
    pub z_index: Option<i32>,

    // Text shadow
    pub text_shadow: Vec<TextShadowValue>,

    // Outline
    pub outline_width: f32,
    #[serde(serialize_with = "color_serde::serialize")]
    pub outline_color: Option<peniko::Color>,
    pub outline_style: BorderStyleValue,
    pub outline_offset: f32,

    // Filters (partial — non-blur only)
    pub filter_brightness: f32,
    pub filter_grayscale: f32,
    pub filter_saturate: f32,
    pub filter_hue_rotate: f32,

    // Cursor
    pub cursor: CursorValue,

    // Pointer events
    pub pointer_events: PointerEventsValue,

    // Typography
    pub font_size: f32,
    pub font_weight: f32,
    pub font_family: String,
    pub font_style: FontStyleValue,
    pub line_height: LineHeightValue,
    pub letter_spacing: f32, // in pixels, 0.0 means normal
    pub word_spacing: f32,   // in pixels, 0.0 means normal
    pub text_align: TextAlignValue,
    pub text_decoration: TextDecorationValue,
    pub white_space: WhiteSpaceValue,
    pub overflow_wrap: OverflowWrapValue,

    // Grid properties - skip serialization (taffy types don't impl Serialize)
    #[serde(skip)]
    pub grid_template_columns: Vec<taffy::GridTemplateComponent<String>>,
    #[serde(skip)]
    pub grid_template_rows: Vec<taffy::GridTemplateComponent<String>>,
    #[serde(skip)]
    pub grid_auto_flow: taffy::GridAutoFlow,

    // Parsing metadata
    /// Whether display was explicitly set in CSS (affects flex defaults).
    pub has_explicit_display: bool,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: DisplayValue::default(),
            position: PositionValue::default(),
            overflow_x: OverflowValue::default(),
            overflow_y: OverflowValue::default(),

            width: DimensionValue::Auto,
            height: DimensionValue::Auto,
            min_width: DimensionValue::Auto,
            min_height: DimensionValue::Auto,
            max_width: DimensionValue::Auto,
            max_height: DimensionValue::Auto,

            flex_direction: FlexDirectionValue::default(),
            flex_wrap: FlexWrapValue::default(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: DimensionValue::Auto,
            align_items: None,
            align_self: None,
            justify_content: None,

            padding_top: LengthPercentageValue::Zero,
            padding_right: LengthPercentageValue::Zero,
            padding_bottom: LengthPercentageValue::Zero,
            padding_left: LengthPercentageValue::Zero,

            margin_top: LengthPercentageAutoValue::Length(0.0),
            margin_right: LengthPercentageAutoValue::Length(0.0),
            margin_bottom: LengthPercentageAutoValue::Length(0.0),
            margin_left: LengthPercentageAutoValue::Length(0.0),

            gap_row: LengthPercentageValue::Zero,
            gap_column: LengthPercentageValue::Zero,

            top: LengthPercentageAutoValue::Auto,
            right: LengthPercentageAutoValue::Auto,
            bottom: LengthPercentageAutoValue::Auto,
            left: LengthPercentageAutoValue::Auto,

            border_top_width: LengthPercentageValue::Zero,
            border_right_width: LengthPercentageValue::Zero,
            border_bottom_width: LengthPercentageValue::Zero,
            border_left_width: LengthPercentageValue::Zero,

            border_radius_top_left: LengthPercentageValue::Zero,
            border_radius_top_right: LengthPercentageValue::Zero,
            border_radius_bottom_right: LengthPercentageValue::Zero,
            border_radius_bottom_left: LengthPercentageValue::Zero,

            border_top_style: BorderStyleValue::None,
            border_right_style: BorderStyleValue::None,
            border_bottom_style: BorderStyleValue::None,
            border_left_style: BorderStyleValue::None,

            border_top_color: None,
            border_right_color: None,
            border_bottom_color: None,
            border_left_color: None,

            background: BackgroundValue::None,
            color: None,

            opacity: 1.0,
            visibility: VisibilityValue::default(),

            transform: TransformValue::default(),
            transform_origin_x: LengthPercentageValue::Percent(0.5),
            transform_origin_y: LengthPercentageValue::Percent(0.5),

            z_index: None,

            text_shadow: Vec::new(),

            outline_width: 0.0,
            outline_color: None,
            outline_style: BorderStyleValue::None,
            outline_offset: 0.0,

            filter_brightness: 1.0,
            filter_grayscale: 0.0,
            filter_saturate: 1.0,
            filter_hue_rotate: 0.0,

            cursor: CursorValue::default(),
            pointer_events: PointerEventsValue::default(),

            font_size: 16.0,
            font_weight: 400.0,
            font_family: String::new(),
            font_style: FontStyleValue::default(),
            line_height: LineHeightValue::default(),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_align: TextAlignValue::default(),
            text_decoration: TextDecorationValue::default(),
            white_space: WhiteSpaceValue::default(),
            overflow_wrap: OverflowWrapValue::default(),

            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_flow: taffy::GridAutoFlow::Row,

            has_explicit_display: false,
        }
    }
}

impl ComputedStyle {
    /// Get the background color (convenience accessor for BackgroundValue::Color).
    pub fn background_color(&self) -> Option<peniko::Color> {
        match &self.background {
            BackgroundValue::Color(c) => Some(*c),
            _ => None,
        }
    }

    /// Get the effective border color for a side (for backwards compat).
    pub fn border_color(&self) -> Option<peniko::Color> {
        self.border_top_color
    }

    /// Parse CSS properties from a HashMap into a typed ComputedStyle.
    pub fn from_props(props: &HashMap<String, String>, viewport: &Viewport) -> Self {
        let mut style = Self::default();

        // Helper closures for parsing
        let parse_dim = |v: &str| DimensionValue::parse(v, viewport);
        let parse_lp = |v: &str| LengthPercentageValue::parse(v, viewport);
        let parse_lpa = |v: &str| LengthPercentageAutoValue::parse(v, viewport);

        for (key, value) in props {
            match key.as_str() {
                // Display/position
                "display" => {
                    style.display = DisplayValue::parse(value);
                    // Track that display was explicitly set (affects flex defaults)
                    // "contents" doesn't count as explicit for this purpose
                    style.has_explicit_display = value.trim() != "contents";
                }
                "position" => style.position = PositionValue::parse(value),
                "overflow" => {
                    let ov = OverflowValue::parse(value);
                    style.overflow_x = ov;
                    style.overflow_y = ov;
                }
                "overflow-x" => style.overflow_x = OverflowValue::parse(value),
                "overflow-y" => style.overflow_y = OverflowValue::parse(value),

                // Dimensions
                "width" => style.width = parse_dim(value),
                "height" => style.height = parse_dim(value),
                "min-width" => style.min_width = parse_dim(value),
                "min-height" => style.min_height = parse_dim(value),
                "max-width" => style.max_width = parse_dim(value),
                "max-height" => style.max_height = parse_dim(value),

                // Flexbox
                "flex-direction" => style.flex_direction = FlexDirectionValue::parse(value),
                "flex-wrap" => style.flex_wrap = FlexWrapValue::parse(value),
                "flex-grow" => style.flex_grow = value.parse().unwrap_or(0.0),
                "flex-shrink" => style.flex_shrink = value.parse().unwrap_or(1.0),
                "flex-basis" => style.flex_basis = parse_dim(value),
                "align-items" => style.align_items = AlignItemsValue::parse(value),
                "align-self" => style.align_self = AlignSelfValue::parse(value),
                "justify-content" => style.justify_content = JustifyContentValue::parse(value),

                // Flex shorthand
                "flex" => {
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    match parts.len() {
                        1 => {
                            if value == "none" {
                                style.flex_grow = 0.0;
                                style.flex_shrink = 0.0;
                                style.flex_basis = DimensionValue::Auto;
                            } else if value == "auto" {
                                style.flex_grow = 1.0;
                                style.flex_shrink = 1.0;
                                style.flex_basis = DimensionValue::Auto;
                            } else if let Ok(grow) = value.parse::<f32>() {
                                style.flex_grow = grow;
                                style.flex_shrink = 1.0;
                                style.flex_basis = DimensionValue::Length(0.0);
                            }
                        }
                        2 => {
                            if let Ok(grow) = parts[0].parse::<f32>() {
                                style.flex_grow = grow;
                            }
                            if let Ok(shrink) = parts[1].parse::<f32>() {
                                style.flex_shrink = shrink;
                            }
                            style.flex_basis = DimensionValue::Length(0.0);
                        }
                        3 => {
                            if let Ok(grow) = parts[0].parse::<f32>() {
                                style.flex_grow = grow;
                            }
                            if let Ok(shrink) = parts[1].parse::<f32>() {
                                style.flex_shrink = shrink;
                            }
                            style.flex_basis = parse_dim(parts[2]);
                        }
                        _ => {}
                    }
                }

                // Padding shorthand
                "padding" => {
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    match parts.len() {
                        1 => {
                            let lp = parse_lp(parts[0]);
                            style.padding_top = lp;
                            style.padding_right = lp;
                            style.padding_bottom = lp;
                            style.padding_left = lp;
                        }
                        2 => {
                            let tb = parse_lp(parts[0]);
                            let lr = parse_lp(parts[1]);
                            style.padding_top = tb;
                            style.padding_right = lr;
                            style.padding_bottom = tb;
                            style.padding_left = lr;
                        }
                        3 => {
                            style.padding_top = parse_lp(parts[0]);
                            let lr = parse_lp(parts[1]);
                            style.padding_right = lr;
                            style.padding_left = lr;
                            style.padding_bottom = parse_lp(parts[2]);
                        }
                        4 => {
                            style.padding_top = parse_lp(parts[0]);
                            style.padding_right = parse_lp(parts[1]);
                            style.padding_bottom = parse_lp(parts[2]);
                            style.padding_left = parse_lp(parts[3]);
                        }
                        _ => {}
                    }
                }
                // Padding (longhands)
                "padding-top" => style.padding_top = parse_lp(value),
                "padding-right" => style.padding_right = parse_lp(value),
                "padding-bottom" => style.padding_bottom = parse_lp(value),
                "padding-left" => style.padding_left = parse_lp(value),

                // Margin shorthand
                "margin" => {
                    let parts: Vec<&str> = value.split_whitespace().collect();
                    match parts.len() {
                        1 => {
                            let lpa = parse_lpa(parts[0]);
                            style.margin_top = lpa;
                            style.margin_right = lpa;
                            style.margin_bottom = lpa;
                            style.margin_left = lpa;
                        }
                        2 => {
                            let tb = parse_lpa(parts[0]);
                            let lr = parse_lpa(parts[1]);
                            style.margin_top = tb;
                            style.margin_right = lr;
                            style.margin_bottom = tb;
                            style.margin_left = lr;
                        }
                        3 => {
                            style.margin_top = parse_lpa(parts[0]);
                            let lr = parse_lpa(parts[1]);
                            style.margin_right = lr;
                            style.margin_left = lr;
                            style.margin_bottom = parse_lpa(parts[2]);
                        }
                        4 => {
                            style.margin_top = parse_lpa(parts[0]);
                            style.margin_right = parse_lpa(parts[1]);
                            style.margin_bottom = parse_lpa(parts[2]);
                            style.margin_left = parse_lpa(parts[3]);
                        }
                        _ => {}
                    }
                }
                // Margin (longhands)
                "margin-top" => style.margin_top = parse_lpa(value),
                "margin-right" => style.margin_right = parse_lpa(value),
                "margin-bottom" => style.margin_bottom = parse_lpa(value),
                "margin-left" => style.margin_left = parse_lpa(value),

                // Gap
                "gap" => {
                    let lp = parse_lp(value);
                    style.gap_row = lp;
                    style.gap_column = lp;
                }
                "row-gap" => style.gap_row = parse_lp(value),
                "column-gap" => style.gap_column = parse_lp(value),

                // Positioning
                "top" => style.top = parse_lpa(value),
                "right" => style.right = parse_lpa(value),
                "bottom" => style.bottom = parse_lpa(value),
                "left" => style.left = parse_lpa(value),

                // Border widths (longhands)
                "border-top-width" => style.border_top_width = parse_lp(value),
                "border-right-width" => style.border_right_width = parse_lp(value),
                "border-bottom-width" => style.border_bottom_width = parse_lp(value),
                "border-left-width" => style.border_left_width = parse_lp(value),

                // Border radius (longhands) - supports percentages
                "border-top-left-radius" => {
                    style.border_radius_top_left = parse_lp(value);
                }
                "border-top-right-radius" => {
                    style.border_radius_top_right = parse_lp(value);
                }
                "border-bottom-right-radius" => {
                    style.border_radius_bottom_right = parse_lp(value);
                }
                "border-bottom-left-radius" => {
                    style.border_radius_bottom_left = parse_lp(value);
                }

                // Colors
                "background-color" => {
                    if let Some(c) = parse_color(value) {
                        style.background = BackgroundValue::Color(c);
                    }
                }
                "color" => style.color = parse_color(value),
                "border-color" => {
                    let c = parse_color(value);
                    style.border_top_color = c;
                    style.border_right_color = c;
                    style.border_bottom_color = c;
                    style.border_left_color = c;
                }

                // Visual
                "opacity" => style.opacity = value.parse().unwrap_or(1.0),

                // Typography
                "font-size" => {
                    style.font_size = parse_font_size_value(value);
                }
                "font-weight" => {
                    style.font_weight = parse_font_weight_value(value);
                }
                "font-family" => {
                    style.font_family = value.clone();
                }
                "font-style" => style.font_style = FontStyleValue::parse(value),
                "line-height" => style.line_height = LineHeightValue::parse(value),
                "letter-spacing" => {
                    style.letter_spacing = parse_px_value(value);
                }
                "word-spacing" => {
                    style.word_spacing = parse_px_value(value);
                }
                "text-align" => style.text_align = TextAlignValue::parse(value),
                "text-decoration" => style.text_decoration = TextDecorationValue::parse(value),
                "white-space" => style.white_space = WhiteSpaceValue::parse(value),
                "overflow-wrap" | "word-wrap" => {
                    style.overflow_wrap = OverflowWrapValue::parse(value)
                }

                // Grid properties
                "grid-template-columns" => {
                    style.grid_template_columns = parse_grid_template(value);
                }
                "grid-template-rows" => {
                    style.grid_template_rows = parse_grid_template(value);
                }
                "grid-auto-flow" => {
                    style.grid_auto_flow = match value.trim() {
                        "column" => taffy::GridAutoFlow::Column,
                        "dense" => taffy::GridAutoFlow::RowDense,
                        "column dense" | "dense column" => taffy::GridAutoFlow::ColumnDense,
                        _ => taffy::GridAutoFlow::Row,
                    };
                }

                // Handle border shorthand for width extraction
                "border" => {
                    // Handle "border: none" or "border: 0" - sets width to 0
                    let trimmed = value.trim();
                    if trimmed == "none" || trimmed == "0" {
                        let lp = LengthPercentageValue::Length(0.0);
                        style.border_top_width = lp;
                        style.border_right_width = lp;
                        style.border_bottom_width = lp;
                        style.border_left_width = lp;
                    } else {
                        // Parse border shorthand: "Npx solid color"
                        for part in value.split_whitespace() {
                            if let Some(px) = part.strip_suffix("px")
                                && let Ok(w) = px.parse::<f32>()
                            {
                                let lp = LengthPercentageValue::Length(w);
                                style.border_top_width = lp;
                                style.border_right_width = lp;
                                style.border_bottom_width = lp;
                                style.border_left_width = lp;
                                break;
                            }
                            // Also handle "none" within border shorthand parts
                            if part == "none" {
                                let lp = LengthPercentageValue::Length(0.0);
                                style.border_top_width = lp;
                                style.border_right_width = lp;
                                style.border_bottom_width = lp;
                                style.border_left_width = lp;
                                break;
                            }
                        }
                        // Also try to extract border-color
                        for part in value.split_whitespace() {
                            if let Some(c) = parse_color(part) {
                                style.border_top_color = Some(c);
                                style.border_right_color = Some(c);
                                style.border_bottom_color = Some(c);
                                style.border_left_color = Some(c);
                                break;
                            }
                        }
                    }
                }

                _ => {}
            }
        }

        style
    }

    /// Convert to Taffy Style using the stored `has_explicit_display` flag.
    pub fn to_taffy_style(&self, default_display: crate::layout::DefaultDisplay) -> taffy::Style {
        self.to_taffy_style_with_explicit(default_display, self.has_explicit_display)
    }

    /// Convert to Taffy Style.
    ///
    /// `has_explicit_display` should be true when the user explicitly set a display value in CSS.
    /// When false, we use element-type defaults (block elements get flex-column, inline get flex-row).
    pub fn to_taffy_style_with_explicit(
        &self,
        default_display: crate::layout::DefaultDisplay,
        has_explicit_display: bool,
    ) -> taffy::Style {
        use taffy::prelude::*;

        // Choose defaults based on element type and whether CSS overrides are present
        let (default_direction, default_wrap) = if has_explicit_display {
            (FlexDirectionValue::Row, FlexWrapValue::NoWrap)
        } else {
            match default_display {
                crate::layout::DefaultDisplay::Block => {
                    (FlexDirectionValue::Column, FlexWrapValue::NoWrap)
                }
                crate::layout::DefaultDisplay::Inline => {
                    (FlexDirectionValue::Row, FlexWrapValue::NoWrap)
                }
            }
        };

        let flex_direction =
            if self.flex_direction == FlexDirectionValue::Row && !has_explicit_display {
                default_direction.to_taffy()
            } else {
                self.flex_direction.to_taffy()
            };

        let flex_wrap = if self.flex_wrap == FlexWrapValue::NoWrap && !has_explicit_display {
            default_wrap.to_taffy()
        } else {
            self.flex_wrap.to_taffy()
        };

        Style {
            display: self.display.to_taffy(),
            position: self.position.to_taffy(),
            overflow: taffy::Point {
                x: self.overflow_x.to_taffy(),
                y: self.overflow_y.to_taffy(),
            },

            size: Size {
                width: self.width.to_taffy(),
                height: self.height.to_taffy(),
            },
            min_size: Size {
                width: self.min_width.to_taffy(),
                height: self.min_height.to_taffy(),
            },
            max_size: Size {
                width: self.max_width.to_taffy(),
                height: self.max_height.to_taffy(),
            },

            flex_direction,
            flex_wrap,
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            flex_basis: self.flex_basis.to_taffy(),
            align_items: self.align_items.map(|v| v.to_taffy()),
            align_self: self.align_self.map(|v| v.to_taffy()),
            justify_content: self.justify_content.map(|v| v.to_taffy()),

            padding: Rect {
                top: self.padding_top.to_taffy(),
                right: self.padding_right.to_taffy(),
                bottom: self.padding_bottom.to_taffy(),
                left: self.padding_left.to_taffy(),
            },
            margin: Rect {
                top: self.margin_top.to_taffy(),
                right: self.margin_right.to_taffy(),
                bottom: self.margin_bottom.to_taffy(),
                left: self.margin_left.to_taffy(),
            },
            gap: Size {
                width: self.gap_column.to_taffy(),
                height: self.gap_row.to_taffy(),
            },

            inset: Rect {
                top: self.top.to_taffy(),
                right: self.right.to_taffy(),
                bottom: self.bottom.to_taffy(),
                left: self.left.to_taffy(),
            },

            border: Rect {
                top: self.border_top_width.to_taffy(),
                right: self.border_right_width.to_taffy(),
                bottom: self.border_bottom_width.to_taffy(),
                left: self.border_left_width.to_taffy(),
            },

            // Grid properties
            grid_template_columns: self.grid_template_columns.clone(),
            grid_template_rows: self.grid_template_rows.clone(),
            grid_auto_flow: self.grid_auto_flow,

            ..Default::default()
        }
    }

    /// Build a Parley text layout from this style's typography fields.
    ///
    /// This is the SINGLE SOURCE OF TRUTH for text layout configuration,
    /// used by measurement, painting, and hit testing.
    ///
    /// # Arguments
    /// * `text` - The text content to lay out
    /// * `scale` - DPI scale factor
    /// * `font_cx` - Parley font context
    /// * `layout_cx` - Parley layout context
    /// * `max_width` - Optional maximum width for line breaking
    pub fn build_parley_layout(
        &self,
        text: &str,
        scale: f32,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<peniko::Brush>,
        max_width: Option<f32>,
    ) -> parley::layout::Layout<peniko::Brush> {
        use parley::style::{FontStack, FontWeight as ParleyFontWeight, StyleProperty};
        use std::borrow::Cow;

        let scaled_font_size = self.font_size * scale;

        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);

        // Set font size (scaled for DPI)
        builder.push_default(StyleProperty::FontSize(scaled_font_size));

        // Set font family (default to sans-serif if empty)
        let font_family = if self.font_family.is_empty() {
            "sans-serif"
        } else {
            &self.font_family
        };
        builder.push_default(StyleProperty::FontStack(FontStack::Source(Cow::Owned(
            font_family.to_string(),
        ))));

        // Set font weight if not normal (400)
        if (self.font_weight - 400.0).abs() > 1.0 {
            builder.push_default(StyleProperty::FontWeight(ParleyFontWeight::new(
                self.font_weight,
            )));
        }

        // Set font style if not normal
        if self.font_style != FontStyleValue::Normal {
            builder.push_default(StyleProperty::FontStyle(self.font_style.to_parley()));
        }

        // Set line height if not normal
        if let Some(line_height) = self.line_height.to_parley() {
            builder.push_default(StyleProperty::LineHeight(line_height));
        }

        // Set letter spacing if not zero
        if self.letter_spacing != 0.0 {
            builder.push_default(StyleProperty::LetterSpacing(self.letter_spacing));
        }

        // Set word spacing if not zero
        if self.word_spacing != 0.0 {
            builder.push_default(StyleProperty::WordSpacing(self.word_spacing));
        }

        // Set overflow-wrap for emergency line-breaking
        if self.overflow_wrap != OverflowWrapValue::Normal {
            builder.push_default(StyleProperty::OverflowWrap(self.overflow_wrap.to_parley()));
        }

        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);

        // Debug-only verification: catch layout drift
        #[cfg(debug_assertions)]
        {
            // Verify layout was built with expected parameters
            if let Some(line) = layout.lines().next() {
                for item in line.items() {
                    if let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                        let run = glyph_run.run();
                        let actual_size = run.font_size();
                        let expected_size = scaled_font_size;
                        debug_assert!(
                            (actual_size - expected_size).abs() < 0.1,
                            "Text layout drift: expected font_size={}, got={}",
                            expected_size,
                            actual_size
                        );
                        break;
                    }
                }
            }
        }

        layout
    }

    // =========================================================================
    // Stylo Conversion Methods
    // =========================================================================

    /// Create ComputedStyle from Stylo's ComputedValues.
    ///
    /// This extracts all relevant CSS properties from Stylo's cascade result
    /// and converts them to our typed ComputedStyle representation.
    pub fn from_stylo(cv: &ComputedValues) -> Self {
        let box_style = cv.get_box();
        let position_style = cv.get_position();
        let margin = cv.get_margin();
        let padding = cv.get_padding();
        let border = cv.get_border();
        let background = cv.get_background();
        let font = cv.get_font();
        let text = cv.get_inherited_text();
        let effects = cv.get_effects();
        let inherited_box = cv.get_inherited_box();
        let outline_style = cv.get_outline();
        let inherited_ui = cv.get_inherited_ui();

        Self {
            // Display/position
            display: Self::display_from_stylo(&box_style.display),
            position: Self::position_from_stylo(&box_style.position),
            overflow_x: Self::overflow_from_stylo(&box_style.overflow_x),
            overflow_y: Self::overflow_from_stylo(&box_style.overflow_y),

            // Dimensions
            width: Self::size_from_stylo(&position_style.width),
            height: Self::size_from_stylo(&position_style.height),
            min_width: Self::size_from_stylo(&position_style.min_width),
            min_height: Self::size_from_stylo(&position_style.min_height),
            max_width: Self::max_size_from_stylo(&position_style.max_width),
            max_height: Self::max_size_from_stylo(&position_style.max_height),

            // Flexbox
            flex_direction: Self::flex_direction_from_stylo(&position_style.flex_direction),
            flex_wrap: Self::flex_wrap_from_stylo(&position_style.flex_wrap),
            flex_grow: position_style.flex_grow.0,
            flex_shrink: position_style.flex_shrink.0,
            flex_basis: Self::flex_basis_from_stylo(&position_style.flex_basis),
            align_items: Self::align_items_from_stylo(&position_style.align_items),
            align_self: Self::align_self_from_stylo(&position_style.align_self),
            justify_content: Self::justify_content_from_stylo(&position_style.justify_content),

            // Padding
            padding_top: Self::length_percentage_from_stylo(&padding.padding_top),
            padding_right: Self::length_percentage_from_stylo(&padding.padding_right),
            padding_bottom: Self::length_percentage_from_stylo(&padding.padding_bottom),
            padding_left: Self::length_percentage_from_stylo(&padding.padding_left),

            // Margin
            margin_top: Self::margin_from_stylo_generic(&margin.margin_top),
            margin_right: Self::margin_from_stylo_generic(&margin.margin_right),
            margin_bottom: Self::margin_from_stylo_generic(&margin.margin_bottom),
            margin_left: Self::margin_from_stylo_generic(&margin.margin_left),

            // Gap
            gap_row: Self::gap_from_stylo(&position_style.row_gap),
            gap_column: Self::gap_from_stylo(&position_style.column_gap),

            // Inset
            top: Self::inset_from_stylo_generic(&position_style.top),
            right: Self::inset_from_stylo_generic(&position_style.right),
            bottom: Self::inset_from_stylo_generic(&position_style.bottom),
            left: Self::inset_from_stylo_generic(&position_style.left),

            // Border widths (BorderSideWidth is a newtype wrapper around NonNegativeLength)
            // Check border-style: if style is 'none' or 'hidden', width should be 0
            border_top_width: if Self::border_style_is_none(&border.border_top_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_top_width.0.to_f32_px())
            },
            border_right_width: if Self::border_style_is_none(&border.border_right_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_right_width.0.to_f32_px())
            },
            border_bottom_width: if Self::border_style_is_none(&border.border_bottom_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_bottom_width.0.to_f32_px())
            },
            border_left_width: if Self::border_style_is_none(&border.border_left_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_left_width.0.to_f32_px())
            },

            // Border radius
            border_radius_top_left: Self::border_radius_from_stylo(&border.border_top_left_radius),
            border_radius_top_right: Self::border_radius_from_stylo(
                &border.border_top_right_radius,
            ),
            border_radius_bottom_right: Self::border_radius_from_stylo(
                &border.border_bottom_right_radius,
            ),
            border_radius_bottom_left: Self::border_radius_from_stylo(
                &border.border_bottom_left_radius,
            ),

            // Border styles (per-side)
            border_top_style: Self::border_style_from_stylo(&border.border_top_style),
            border_right_style: Self::border_style_from_stylo(&border.border_right_style),
            border_bottom_style: Self::border_style_from_stylo(&border.border_bottom_style),
            border_left_style: Self::border_style_from_stylo(&border.border_left_style),

            // Border colors (per-side, resolve currentColor)
            border_top_color: if border.border_top_color.is_currentcolor() {
                Self::color_from_absolute(&text.color)
            } else {
                Self::color_from_stylo(&border.border_top_color)
            },
            border_right_color: if border.border_right_color.is_currentcolor() {
                Self::color_from_absolute(&text.color)
            } else {
                Self::color_from_stylo(&border.border_right_color)
            },
            border_bottom_color: if border.border_bottom_color.is_currentcolor() {
                Self::color_from_absolute(&text.color)
            } else {
                Self::color_from_stylo(&border.border_bottom_color)
            },
            border_left_color: if border.border_left_color.is_currentcolor() {
                Self::color_from_absolute(&text.color)
            } else {
                Self::color_from_stylo(&border.border_left_color)
            },

            // Background (color or gradient)
            background: Self::background_from_stylo(background, &text.color),
            color: Self::color_from_absolute(&text.color),

            // Visual
            opacity: effects.opacity,
            visibility: Self::visibility_from_stylo(&inherited_box.visibility),

            // Transforms
            transform: Self::transform_from_stylo(&box_style.transform),
            transform_origin_x: Self::transform_origin_component_from_stylo(
                &box_style.transform_origin.horizontal,
            ),
            transform_origin_y: Self::transform_origin_component_from_stylo(
                &box_style.transform_origin.vertical,
            ),

            // Z-index
            z_index: Self::z_index_from_stylo(&position_style.z_index),

            // Text shadow
            text_shadow: Self::text_shadow_from_stylo(&text.text_shadow, &text.color),

            // Outline
            outline_width: if Self::border_style_is_none_val(
                &Self::border_style_from_stylo_outline(&outline_style.outline_style),
            ) {
                0.0
            } else {
                outline_style.outline_width.0.to_f32_px()
            },
            outline_color: if outline_style.outline_color.is_currentcolor() {
                Self::color_from_absolute(&text.color)
            } else {
                Self::color_from_stylo(&outline_style.outline_color)
            },
            outline_style: Self::border_style_from_stylo_outline(&outline_style.outline_style),
            outline_offset: outline_style.outline_offset.to_f32_px(),

            // Filters
            filter_brightness: Self::extract_filter_brightness(&effects.filter),
            filter_grayscale: Self::extract_filter_grayscale(&effects.filter),
            filter_saturate: Self::extract_filter_saturate(&effects.filter),
            filter_hue_rotate: Self::extract_filter_hue_rotate(&effects.filter),

            // Cursor
            cursor: Self::cursor_from_stylo(&inherited_ui.cursor.keyword),

            // Pointer events
            pointer_events: Self::pointer_events_from_stylo(&inherited_ui.pointer_events),

            // Typography
            font_size: font.font_size.computed_size().px(),
            font_weight: font.font_weight.value(),
            font_family: Self::font_family_from_stylo(&font.font_family),
            font_style: Self::font_style_from_stylo(&font.font_style),
            line_height: Self::line_height_from_stylo(&font.line_height),
            letter_spacing: Self::letter_spacing_from_stylo(&text.letter_spacing),
            word_spacing: Self::word_spacing_from_stylo(&text.word_spacing),
            text_align: Self::text_align_from_stylo(&text.text_align),
            text_decoration: TextDecorationValue::default(), // TODO: text-decoration from Stylo
            white_space: Self::white_space_from_stylo(
                &text.white_space_collapse,
                &text.text_wrap_mode,
            ),
            overflow_wrap: Self::overflow_wrap_from_stylo(&text.overflow_wrap),

            // Grid - extract from Stylo
            grid_template_columns: Self::grid_template_tracks_from_stylo(
                &position_style.grid_template_columns,
            ),
            grid_template_rows: Self::grid_template_tracks_from_stylo(
                &position_style.grid_template_rows,
            ),
            grid_auto_flow: Self::grid_auto_flow_from_stylo(&position_style.grid_auto_flow),

            // Display was explicitly set by Stylo
            has_explicit_display: !box_style.display.is_none(),
        }
    }

    // -------------------------------------------------------------------------
    // Helper conversion functions for Stylo types
    // -------------------------------------------------------------------------

    fn display_from_stylo(display: &style::values::computed::Display) -> DisplayValue {
        use style::values::specified::box_::{DisplayInside, DisplayOutside};

        if display.is_none() {
            return DisplayValue::None;
        }
        if display.is_contents() {
            return DisplayValue::Contents;
        }

        let outside = display.outside();
        let inside = display.inside();

        match (outside, inside) {
            (DisplayOutside::Inline, DisplayInside::Flow) => DisplayValue::Inline,
            (DisplayOutside::Inline, DisplayInside::FlowRoot) => DisplayValue::InlineBlock,
            (DisplayOutside::Inline, DisplayInside::Flex) => DisplayValue::InlineFlex,
            (DisplayOutside::Block, DisplayInside::Flow) => DisplayValue::Block,
            (DisplayOutside::Block, DisplayInside::FlowRoot) => DisplayValue::Block,
            (DisplayOutside::Block, DisplayInside::Flex) => DisplayValue::Flex,
            (DisplayOutside::Block, DisplayInside::Grid) => DisplayValue::Grid,
            (DisplayOutside::Inline, DisplayInside::Grid) => DisplayValue::Grid, // inline-grid
            _ => DisplayValue::Flex, // Default to flex for unknown
        }
    }

    fn position_from_stylo(pos: &style::values::computed::PositionProperty) -> PositionValue {
        use style::values::computed::PositionProperty;
        match *pos {
            PositionProperty::Static => PositionValue::Static,
            PositionProperty::Relative => PositionValue::Relative,
            PositionProperty::Absolute => PositionValue::Absolute,
            PositionProperty::Fixed => PositionValue::Fixed,
            PositionProperty::Sticky => PositionValue::Sticky,
        }
    }

    fn overflow_from_stylo(overflow: &style::values::computed::Overflow) -> OverflowValue {
        use style::values::computed::Overflow;
        match *overflow {
            Overflow::Visible => OverflowValue::Visible,
            Overflow::Hidden => OverflowValue::Hidden,
            Overflow::Scroll => OverflowValue::Scroll,
            Overflow::Auto => OverflowValue::Auto,
            Overflow::Clip => OverflowValue::Clip,
        }
    }

    fn size_from_stylo(size: &style::values::computed::Size) -> DimensionValue {
        use style::values::computed::Size;
        match size {
            Size::Auto => DimensionValue::Auto,
            Size::LengthPercentage(lp) => {
                if let Some(len) = lp.0.to_length() {
                    DimensionValue::Length(len.px())
                } else if let Some(pct) = lp.0.to_percentage() {
                    DimensionValue::Percent(pct.0)
                } else {
                    DimensionValue::Auto
                }
            }
            Size::MaxContent | Size::MinContent | Size::FitContent | Size::Stretch => {
                DimensionValue::Auto
            }
            _ => DimensionValue::Auto,
        }
    }

    fn max_size_from_stylo(size: &style::values::computed::MaxSize) -> DimensionValue {
        use style::values::computed::MaxSize;
        match size {
            MaxSize::None => DimensionValue::Auto,
            MaxSize::LengthPercentage(lp) => {
                if let Some(len) = lp.0.to_length() {
                    DimensionValue::Length(len.px())
                } else if let Some(pct) = lp.0.to_percentage() {
                    DimensionValue::Percent(pct.0)
                } else {
                    DimensionValue::Auto
                }
            }
            MaxSize::MaxContent | MaxSize::MinContent | MaxSize::FitContent | MaxSize::Stretch => {
                DimensionValue::Auto
            }
            _ => DimensionValue::Auto,
        }
    }

    fn flex_direction_from_stylo(
        dir: &style::properties::longhands::flex_direction::computed_value::T,
    ) -> FlexDirectionValue {
        use style::properties::longhands::flex_direction::computed_value::T as FlexDir;
        match *dir {
            FlexDir::Row => FlexDirectionValue::Row,
            FlexDir::RowReverse => FlexDirectionValue::RowReverse,
            FlexDir::Column => FlexDirectionValue::Column,
            FlexDir::ColumnReverse => FlexDirectionValue::ColumnReverse,
        }
    }

    fn flex_wrap_from_stylo(
        wrap: &style::properties::longhands::flex_wrap::computed_value::T,
    ) -> FlexWrapValue {
        use style::properties::longhands::flex_wrap::computed_value::T as FlexWr;
        match *wrap {
            FlexWr::Nowrap => FlexWrapValue::NoWrap,
            FlexWr::Wrap => FlexWrapValue::Wrap,
            FlexWr::WrapReverse => FlexWrapValue::WrapReverse,
        }
    }

    fn flex_basis_from_stylo(basis: &style::values::computed::FlexBasis) -> DimensionValue {
        use style::values::computed::FlexBasis;
        match basis {
            FlexBasis::Content => DimensionValue::Auto,
            FlexBasis::Size(size) => Self::size_from_stylo(size),
        }
    }

    fn align_items_from_stylo(
        align: &style::values::computed::ItemPlacement,
    ) -> Option<AlignItemsValue> {
        use style::values::specified::align::AlignFlags;
        let flags = align.0.value();
        if flags == AlignFlags::FLEX_START
            || flags == AlignFlags::START
            || flags == AlignFlags::SELF_START
        {
            Some(AlignItemsValue::FlexStart)
        } else if flags == AlignFlags::FLEX_END
            || flags == AlignFlags::END
            || flags == AlignFlags::SELF_END
        {
            Some(AlignItemsValue::FlexEnd)
        } else if flags == AlignFlags::CENTER {
            Some(AlignItemsValue::Center)
        } else if flags == AlignFlags::BASELINE {
            Some(AlignItemsValue::Baseline)
        } else if flags == AlignFlags::STRETCH || flags == AlignFlags::NORMAL {
            Some(AlignItemsValue::Stretch) // Normal defaults to stretch for flex items
        } else {
            None
        }
    }

    fn align_self_from_stylo(
        align: &style::values::computed::SelfAlignment,
    ) -> Option<AlignSelfValue> {
        use style::values::specified::align::AlignFlags;
        let flags = align.0.value();
        if flags == AlignFlags::AUTO {
            None // Auto means inherit from align-items
        } else if flags == AlignFlags::FLEX_START
            || flags == AlignFlags::START
            || flags == AlignFlags::SELF_START
        {
            Some(AlignSelfValue::FlexStart)
        } else if flags == AlignFlags::FLEX_END
            || flags == AlignFlags::END
            || flags == AlignFlags::SELF_END
        {
            Some(AlignSelfValue::FlexEnd)
        } else if flags == AlignFlags::CENTER {
            Some(AlignSelfValue::Center)
        } else if flags == AlignFlags::BASELINE {
            Some(AlignSelfValue::Baseline)
        } else if flags == AlignFlags::STRETCH {
            Some(AlignSelfValue::Stretch)
        } else {
            None
        }
    }

    fn justify_content_from_stylo(
        justify: &style::values::computed::ContentDistribution,
    ) -> Option<JustifyContentValue> {
        use style::values::specified::align::AlignFlags;
        let flags = justify.primary();
        let value = flags.value();
        if value == AlignFlags::SPACE_BETWEEN {
            Some(JustifyContentValue::SpaceBetween)
        } else if value == AlignFlags::SPACE_AROUND {
            Some(JustifyContentValue::SpaceAround)
        } else if value == AlignFlags::SPACE_EVENLY {
            Some(JustifyContentValue::SpaceEvenly)
        } else if value == AlignFlags::FLEX_START || value == AlignFlags::START {
            Some(JustifyContentValue::FlexStart)
        } else if value == AlignFlags::FLEX_END || value == AlignFlags::END {
            Some(JustifyContentValue::FlexEnd)
        } else if value == AlignFlags::CENTER {
            Some(JustifyContentValue::Center)
        } else {
            None
        }
    }

    fn length_percentage_from_stylo(
        lp: &style::values::computed::NonNegativeLengthPercentage,
    ) -> LengthPercentageValue {
        if let Some(len) = lp.0.to_length() {
            LengthPercentageValue::Length(len.px())
        } else if let Some(pct) = lp.0.to_percentage() {
            LengthPercentageValue::Percent(pct.0)
        } else {
            LengthPercentageValue::Zero
        }
    }

    fn margin_from_stylo_generic(
        margin: &style::values::computed::Margin,
    ) -> LengthPercentageAutoValue {
        use style::values::generics::length::GenericMargin;
        match margin {
            GenericMargin::Auto => LengthPercentageAutoValue::Auto,
            GenericMargin::LengthPercentage(lp) => {
                if let Some(len) = lp.to_length() {
                    LengthPercentageAutoValue::Length(len.px())
                } else if let Some(pct) = lp.to_percentage() {
                    LengthPercentageAutoValue::Percent(pct.0)
                } else {
                    LengthPercentageAutoValue::Length(0.0)
                }
            }
            _ => LengthPercentageAutoValue::Auto,
        }
    }

    fn gap_from_stylo(
        gap: &style::values::computed::length::NonNegativeLengthPercentageOrNormal,
    ) -> LengthPercentageValue {
        use style::values::computed::length::NonNegativeLengthPercentageOrNormal;
        match gap {
            NonNegativeLengthPercentageOrNormal::Normal => LengthPercentageValue::Zero,
            NonNegativeLengthPercentageOrNormal::LengthPercentage(lp) => {
                Self::length_percentage_from_stylo(lp)
            }
        }
    }

    fn inset_from_stylo_generic(
        inset: &style::values::computed::Inset,
    ) -> LengthPercentageAutoValue {
        use style::values::generics::position::GenericInset;
        match inset {
            GenericInset::Auto => LengthPercentageAutoValue::Auto,
            GenericInset::LengthPercentage(lp) => {
                if let Some(len) = lp.to_length() {
                    LengthPercentageAutoValue::Length(len.px())
                } else if let Some(pct) = lp.to_percentage() {
                    LengthPercentageAutoValue::Percent(pct.0)
                } else {
                    LengthPercentageAutoValue::Length(0.0)
                }
            }
            _ => LengthPercentageAutoValue::Auto,
        }
    }

    fn border_radius_from_stylo(
        radius: &style::values::computed::BorderCornerRadius,
    ) -> LengthPercentageValue {
        // BorderCornerRadius is a Size with width and height for elliptical radii
        // We just take the width (horizontal radius) for simplicity
        let width = &radius.0.width;
        if let Some(len) = width.0.to_length() {
            LengthPercentageValue::Length(len.px())
        } else if let Some(pct) = width.0.to_percentage() {
            // Store the percentage to be resolved at paint time when dimensions are known
            LengthPercentageValue::Percent(pct.0)
        } else {
            LengthPercentageValue::Zero
        }
    }

    /// Check if a border style is 'none' or 'hidden' (meaning no border should be painted).
    fn border_style_is_none(style: &style::values::computed::BorderStyle) -> bool {
        use style::values::computed::BorderStyle;
        matches!(style, BorderStyle::None | BorderStyle::Hidden)
    }

    fn color_from_stylo(color: &style::values::computed::Color) -> Option<peniko::Color> {
        // Stylo's Color has as_absolute() to get resolved RGBA
        color.as_absolute().map(|abs| {
            let rgba = abs.to_color_space(style::color::ColorSpace::Srgb);
            let r = (rgba.components.0.clamp(0.0, 1.0) * 255.0) as u8;
            let g = (rgba.components.1.clamp(0.0, 1.0) * 255.0) as u8;
            let b = (rgba.components.2.clamp(0.0, 1.0) * 255.0) as u8;
            let a = (rgba.alpha.clamp(0.0, 1.0) * 255.0) as u8;
            peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(r, g, b, a)
        })
    }

    fn color_from_absolute(color: &style::color::AbsoluteColor) -> Option<peniko::Color> {
        let rgba = color.to_color_space(style::color::ColorSpace::Srgb);
        let r = (rgba.components.0.clamp(0.0, 1.0) * 255.0) as u8;
        let g = (rgba.components.1.clamp(0.0, 1.0) * 255.0) as u8;
        let b = (rgba.components.2.clamp(0.0, 1.0) * 255.0) as u8;
        let a = (rgba.alpha.clamp(0.0, 1.0) * 255.0) as u8;
        Some(peniko::color::AlphaColor::<peniko::color::Srgb>::from_rgba8(r, g, b, a))
    }

    fn font_family_from_stylo(family: &style::values::computed::font::FontFamily) -> String {
        use style::values::computed::font::{GenericFontFamily, SingleFontFamily};
        let mut result = String::new();
        for (i, f) in family.families.iter().enumerate() {
            if i > 0 {
                result.push_str(", ");
            }
            match f {
                SingleFontFamily::FamilyName(name) => {
                    result.push_str(name.name.as_ref());
                }
                SingleFontFamily::Generic(generic) => {
                    result.push_str(match *generic {
                        GenericFontFamily::None => "sans-serif",
                        GenericFontFamily::Serif => "serif",
                        GenericFontFamily::SansSerif => "sans-serif",
                        GenericFontFamily::Monospace => "monospace",
                        GenericFontFamily::Cursive => "cursive",
                        GenericFontFamily::Fantasy => "fantasy",
                        GenericFontFamily::SystemUi => "system-ui",
                    });
                }
            }
        }
        result
    }

    fn font_style_from_stylo(
        font_style: &style::values::computed::font::FontStyle,
    ) -> FontStyleValue {
        use style::values::computed::font::FontStyle as StyloFontStyle;
        if *font_style == StyloFontStyle::NORMAL {
            FontStyleValue::Normal
        } else if *font_style == StyloFontStyle::ITALIC {
            FontStyleValue::Italic
        } else {
            // Oblique - any other angle is oblique
            FontStyleValue::Oblique
        }
    }

    fn line_height_from_stylo(lh: &style::values::computed::LineHeight) -> LineHeightValue {
        use style::values::generics::font::LineHeight;
        match lh {
            LineHeight::Normal => LineHeightValue::Normal,
            LineHeight::Number(n) => LineHeightValue::Relative(n.0),
            LineHeight::Length(len) => {
                // NonNegativeLength - get the px value directly
                LineHeightValue::Absolute(len.0.px())
            }
        }
    }

    fn letter_spacing_from_stylo(ls: &style::values::computed::text::LetterSpacing) -> f32 {
        // LetterSpacing wraps a LengthPercentage
        if let Some(len) = ls.0.to_length() {
            len.px()
        } else {
            0.0
        }
    }

    fn word_spacing_from_stylo(ws: &style::values::computed::text::WordSpacing) -> f32 {
        // WordSpacing wraps a LengthPercentage - use to_length() method
        ws.to_length().map(|l| l.px()).unwrap_or(0.0)
    }

    fn text_align_from_stylo(align: &style::values::computed::TextAlign) -> TextAlignValue {
        use style::values::computed::TextAlign;
        match *align {
            TextAlign::Start => TextAlignValue::Start,
            TextAlign::End => TextAlignValue::End,
            TextAlign::Left => TextAlignValue::Start,
            TextAlign::Right => TextAlignValue::End,
            TextAlign::Center => TextAlignValue::Center,
            TextAlign::Justify => TextAlignValue::Justify,
            _ => TextAlignValue::Start,
        }
    }

    fn white_space_from_stylo(
        collapse: &style::properties::longhands::white_space_collapse::computed_value::T,
        wrap_mode: &style::properties::longhands::text_wrap_mode::computed_value::T,
    ) -> WhiteSpaceValue {
        use style::properties::longhands::text_wrap_mode::computed_value::T as TWMode;
        use style::properties::longhands::white_space_collapse::computed_value::T as WSCollapse;
        match (*collapse, *wrap_mode) {
            (WSCollapse::Collapse, TWMode::Wrap) => WhiteSpaceValue::Normal,
            (WSCollapse::Collapse, TWMode::Nowrap) => WhiteSpaceValue::NoWrap,
            (WSCollapse::Preserve, TWMode::Nowrap) => WhiteSpaceValue::Pre,
            (WSCollapse::Preserve, TWMode::Wrap) => WhiteSpaceValue::PreWrap,
            (WSCollapse::PreserveBreaks, TWMode::Wrap) => WhiteSpaceValue::PreLine,
            _ => WhiteSpaceValue::Normal,
        }
    }

    fn overflow_wrap_from_stylo(
        ow: &style::properties::longhands::overflow_wrap::computed_value::T,
    ) -> OverflowWrapValue {
        use style::values::specified::text::OverflowWrap as StyloOW;
        match *ow {
            StyloOW::Normal => OverflowWrapValue::Normal,
            StyloOW::BreakWord => OverflowWrapValue::BreakWord,
            StyloOW::Anywhere => OverflowWrapValue::Anywhere,
        }
    }

    // -------------------------------------------------------------------------
    // Grid conversion functions (from Stylo types to Taffy types)
    // -------------------------------------------------------------------------

    fn grid_auto_flow_from_stylo(input: &GridAutoFlow) -> taffy::GridAutoFlow {
        let is_row = input.contains(GridAutoFlow::ROW);
        let is_dense = input.contains(GridAutoFlow::DENSE);

        match (is_row, is_dense) {
            (true, false) => taffy::GridAutoFlow::Row,
            (true, true) => taffy::GridAutoFlow::RowDense,
            (false, false) => taffy::GridAutoFlow::Column,
            (false, true) => taffy::GridAutoFlow::ColumnDense,
        }
    }

    fn grid_template_tracks_from_stylo(
        input: &GridTemplateComponent,
    ) -> Vec<taffy::GridTemplateComponent<String>> {
        match input {
            GenericGridTemplateComponent::None => Vec::new(),
            GenericGridTemplateComponent::TrackList(list) => list
                .values
                .iter()
                .map(|track| match track {
                    TrackListValue::TrackSize(size) => {
                        taffy::GridTemplateComponent::Single(Self::track_size_from_stylo(size))
                    }
                    TrackListValue::TrackRepeat(repeat) => {
                        taffy::GridTemplateComponent::Repeat(taffy::GridTemplateRepetition {
                            count: Self::track_repeat_from_stylo(repeat.count),
                            tracks: repeat
                                .track_sizes
                                .iter()
                                .map(Self::track_size_from_stylo)
                                .collect(),
                            line_names: repeat
                                .line_names
                                .iter()
                                .map(|line_name_set| {
                                    line_name_set
                                        .iter()
                                        .map(|ident| ident.0.to_string())
                                        .collect::<Vec<_>>()
                                })
                                .collect::<Vec<_>>(),
                        })
                    }
                })
                .collect(),

            // TODO: Implement subgrid and masonry
            GenericGridTemplateComponent::Subgrid(_) => Vec::new(),
            GenericGridTemplateComponent::Masonry => Vec::new(),
        }
    }

    fn track_repeat_from_stylo(input: RepeatCount<i32>) -> taffy::RepetitionCount {
        match input {
            RepeatCount::Number(val) => taffy::RepetitionCount::Count(val.try_into().unwrap_or(1)),
            RepeatCount::AutoFill => taffy::RepetitionCount::AutoFill,
            RepeatCount::AutoFit => taffy::RepetitionCount::AutoFit,
        }
    }

    fn track_size_from_stylo(
        input: &TrackSize<style::values::computed::LengthPercentage>,
    ) -> taffy::TrackSizingFunction {
        match input {
            TrackSize::Breadth(breadth) => taffy::MinMax {
                min: Self::min_track_from_stylo(breadth),
                max: Self::max_track_from_stylo(breadth),
            },
            TrackSize::Minmax(min, max) => taffy::MinMax {
                min: Self::min_track_from_stylo(min),
                max: Self::max_track_from_stylo(max),
            },
            TrackSize::FitContent(limit) => taffy::MinMax {
                min: taffy::MinTrackSizingFunction::auto(),
                max: match limit {
                    TrackBreadth::Breadth(lp) => {
                        use taffy::style_helpers::TaffyFitContent;
                        taffy::MaxTrackSizingFunction::fit_content(
                            Self::length_percentage_from_stylo_lp(lp),
                        )
                    }
                    // Fr, Auto, MinContent, MaxContent shouldn't appear in fit-content
                    _ => taffy::MaxTrackSizingFunction::auto(),
                },
            },
        }
    }

    fn min_track_from_stylo(
        input: &TrackBreadth<style::values::computed::LengthPercentage>,
    ) -> taffy::MinTrackSizingFunction {
        match input {
            TrackBreadth::Breadth(lp) => {
                taffy::MinTrackSizingFunction::from(Self::length_percentage_from_stylo_lp(lp))
            }
            TrackBreadth::Fr(_) => taffy::MinTrackSizingFunction::auto(),
            TrackBreadth::Auto => taffy::MinTrackSizingFunction::auto(),
            TrackBreadth::MinContent => taffy::MinTrackSizingFunction::min_content(),
            TrackBreadth::MaxContent => taffy::MinTrackSizingFunction::max_content(),
        }
    }

    fn max_track_from_stylo(
        input: &TrackBreadth<style::values::computed::LengthPercentage>,
    ) -> taffy::MaxTrackSizingFunction {
        match input {
            TrackBreadth::Breadth(lp) => {
                taffy::MaxTrackSizingFunction::from(Self::length_percentage_from_stylo_lp(lp))
            }
            TrackBreadth::Fr(val) => taffy::MaxTrackSizingFunction::fr(*val),
            TrackBreadth::Auto => taffy::MaxTrackSizingFunction::auto(),
            TrackBreadth::MinContent => taffy::MaxTrackSizingFunction::min_content(),
            TrackBreadth::MaxContent => taffy::MaxTrackSizingFunction::max_content(),
        }
    }

    /// Convert Stylo LengthPercentage to Taffy LengthPercentage.
    /// This is similar to length_percentage_from_stylo but works with plain LengthPercentage
    /// instead of NonNegativeLengthPercentage.
    fn length_percentage_from_stylo_lp(
        lp: &style::values::computed::LengthPercentage,
    ) -> taffy::LengthPercentage {
        if let Some(len) = lp.to_length() {
            taffy::LengthPercentage::length(len.px())
        } else if let Some(pct) = lp.to_percentage() {
            taffy::LengthPercentage::percent(pct.0)
        } else {
            taffy::LengthPercentage::length(0.0)
        }
    }

    // -------------------------------------------------------------------------
    // New CSS feature extraction helpers
    // -------------------------------------------------------------------------

    fn border_style_from_stylo(bs: &style::values::computed::BorderStyle) -> BorderStyleValue {
        use style::values::computed::BorderStyle;
        match *bs {
            BorderStyle::None => BorderStyleValue::None,
            BorderStyle::Solid => BorderStyleValue::Solid,
            BorderStyle::Dashed => BorderStyleValue::Dashed,
            BorderStyle::Dotted => BorderStyleValue::Dotted,
            BorderStyle::Double => BorderStyleValue::Double,
            BorderStyle::Hidden => BorderStyleValue::Hidden,
            // groove, ridge, inset, outset → render as solid
            _ => BorderStyleValue::Solid,
        }
    }

    fn border_style_from_stylo_outline(
        os: &style::values::computed::OutlineStyle,
    ) -> BorderStyleValue {
        use style::values::computed::OutlineStyle;
        match *os {
            OutlineStyle::Auto => BorderStyleValue::Solid,
            OutlineStyle::BorderStyle(ref bs) => Self::border_style_from_stylo(bs),
        }
    }

    fn border_style_is_none_val(style: &BorderStyleValue) -> bool {
        matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden)
    }

    fn visibility_from_stylo(
        vis: &style::properties::longhands::visibility::computed_value::T,
    ) -> VisibilityValue {
        use style::properties::longhands::visibility::computed_value::T as Vis;
        match *vis {
            Vis::Visible => VisibilityValue::Visible,
            Vis::Hidden => VisibilityValue::Hidden,
            Vis::Collapse => VisibilityValue::Collapse,
        }
    }

    fn cursor_from_stylo(cursor: &style::values::specified::ui::CursorKind) -> CursorValue {
        use style::values::specified::ui::CursorKind;
        match *cursor {
            CursorKind::Auto => CursorValue::Auto,
            CursorKind::Default => CursorValue::Default,
            CursorKind::Pointer => CursorValue::Pointer,
            CursorKind::Text => CursorValue::Text,
            CursorKind::Move => CursorValue::Move,
            CursorKind::NotAllowed => CursorValue::NotAllowed,
            CursorKind::Crosshair => CursorValue::Crosshair,
            CursorKind::Grab => CursorValue::Grab,
            CursorKind::Grabbing => CursorValue::Grabbing,
            CursorKind::ColResize => CursorValue::ColResize,
            CursorKind::RowResize => CursorValue::RowResize,
            CursorKind::NResize => CursorValue::NResize,
            CursorKind::SResize => CursorValue::SResize,
            CursorKind::EResize => CursorValue::EResize,
            CursorKind::WResize => CursorValue::WResize,
            CursorKind::NeResize => CursorValue::NeResize,
            CursorKind::NwResize => CursorValue::NwResize,
            CursorKind::SeResize => CursorValue::SeResize,
            CursorKind::SwResize => CursorValue::SwResize,
            CursorKind::EwResize => CursorValue::EwResize,
            CursorKind::NsResize => CursorValue::NsResize,
            CursorKind::Wait => CursorValue::Wait,
            CursorKind::Progress => CursorValue::Progress,
            CursorKind::Help => CursorValue::Help,
            CursorKind::ZoomIn => CursorValue::ZoomIn,
            CursorKind::ZoomOut => CursorValue::ZoomOut,
            CursorKind::None => CursorValue::None,
            _ => CursorValue::Auto,
        }
    }

    fn pointer_events_from_stylo(
        pe: &style::values::specified::ui::PointerEvents,
    ) -> PointerEventsValue {
        use style::values::specified::ui::PointerEvents;
        match *pe {
            PointerEvents::None => PointerEventsValue::None,
            _ => PointerEventsValue::Auto,
        }
    }

    fn z_index_from_stylo(z: &style::values::computed::ZIndex) -> Option<i32> {
        use style::values::generics::position::ZIndex;
        match z {
            ZIndex::Integer(val) => Some(*val),
            ZIndex::Auto => None,
        }
    }

    fn transform_from_stylo(transform: &style::values::computed::Transform) -> TransformValue {
        use style::values::generics::transform::GenericTransformOperation;

        if transform.0.is_empty() {
            return TransformValue::default();
        }

        // Compose all operations into a single 2D affine matrix [a, b, c, d, e, f]
        let mut m = [1.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0]; // identity

        for op in &*transform.0 {
            let op_matrix = match op {
                GenericTransformOperation::Matrix(mat) => [
                    mat.a as f64,
                    mat.b as f64,
                    mat.c as f64,
                    mat.d as f64,
                    mat.e as f64,
                    mat.f as f64,
                ],
                GenericTransformOperation::Rotate(angle) => {
                    let rad = angle.radians64();
                    let cos = rad.cos();
                    let sin = rad.sin();
                    [cos, sin, -sin, cos, 0.0, 0.0]
                }
                GenericTransformOperation::Scale(sx, sy) => {
                    [*sx as f64, 0.0, 0.0, *sy as f64, 0.0, 0.0]
                }
                GenericTransformOperation::ScaleX(sx) => [*sx as f64, 0.0, 0.0, 1.0, 0.0, 0.0],
                GenericTransformOperation::ScaleY(sy) => [1.0, 0.0, 0.0, *sy as f64, 0.0, 0.0],
                GenericTransformOperation::TranslateX(tx) => {
                    let tx_px = Self::length_or_pct_to_px(tx);
                    [1.0, 0.0, 0.0, 1.0, tx_px, 0.0]
                }
                GenericTransformOperation::TranslateY(ty) => {
                    let ty_px = Self::length_or_pct_to_px(ty);
                    [1.0, 0.0, 0.0, 1.0, 0.0, ty_px]
                }
                GenericTransformOperation::Translate(tx, ty) => {
                    let tx_px = Self::length_or_pct_to_px(tx);
                    let ty_px = Self::length_or_pct_to_px(ty);
                    [1.0, 0.0, 0.0, 1.0, tx_px, ty_px]
                }
                GenericTransformOperation::SkewX(angle) => {
                    let tan = angle.radians64().tan();
                    [1.0, 0.0, tan, 1.0, 0.0, 0.0]
                }
                GenericTransformOperation::SkewY(angle) => {
                    let tan = angle.radians64().tan();
                    [1.0, tan, 0.0, 1.0, 0.0, 0.0]
                }
                GenericTransformOperation::Skew(ax, ay) => {
                    let tan_x = ax.radians64().tan();
                    let tan_y = ay.radians64().tan();
                    [1.0, tan_y, tan_x, 1.0, 0.0, 0.0]
                }
                // 3D transforms — flatten to 2D identity (skip)
                _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            };

            // Matrix multiply: m = m * op_matrix
            let a = m[0] * op_matrix[0] + m[2] * op_matrix[1];
            let b = m[1] * op_matrix[0] + m[3] * op_matrix[1];
            let c = m[0] * op_matrix[2] + m[2] * op_matrix[3];
            let d = m[1] * op_matrix[2] + m[3] * op_matrix[3];
            let e = m[0] * op_matrix[4] + m[2] * op_matrix[5] + m[4];
            let f = m[1] * op_matrix[4] + m[3] * op_matrix[5] + m[5];
            m = [a, b, c, d, e, f];
        }

        let is_identity = (m[0] - 1.0).abs() < 1e-6
            && m[1].abs() < 1e-6
            && m[2].abs() < 1e-6
            && (m[3] - 1.0).abs() < 1e-6
            && m[4].abs() < 1e-6
            && m[5].abs() < 1e-6;

        TransformValue {
            matrix: m,
            is_identity,
        }
    }

    fn length_or_pct_to_px(lp: &style::values::computed::LengthPercentage) -> f64 {
        if let Some(len) = lp.to_length() {
            len.px() as f64
        } else {
            // Percentage transforms need element size — store 0 for now
            // (will be resolved at paint time for percentage-based translates)
            0.0
        }
    }

    fn transform_origin_component_from_stylo(
        origin: &style::values::computed::LengthPercentage,
    ) -> LengthPercentageValue {
        if let Some(len) = origin.to_length() {
            LengthPercentageValue::Length(len.px())
        } else if let Some(pct) = origin.to_percentage() {
            LengthPercentageValue::Percent(pct.0)
        } else {
            LengthPercentageValue::Percent(0.5) // default 50%
        }
    }

    fn text_shadow_from_stylo(
        shadows: &style::properties::longhands::text_shadow::computed_value::T,
        text_color: &style::color::AbsoluteColor,
    ) -> Vec<TextShadowValue> {
        shadows
            .0
            .iter()
            .map(|s| {
                let color = if s.color.is_currentcolor() {
                    Self::color_from_absolute(text_color)
                } else {
                    Self::color_from_stylo(&s.color)
                };
                TextShadowValue {
                    offset_x: s.horizontal.px(),
                    offset_y: s.vertical.px(),
                    blur_radius: s.blur.0.px(),
                    color,
                }
            })
            .collect()
    }

    fn background_from_stylo(
        bg: &style::properties::style_structs::Background,
        text_color: &style::color::AbsoluteColor,
    ) -> BackgroundValue {
        use style::values::computed::image::Image;
        use style::values::generics::image::GenericGradient;

        // Check for gradient in background-image first
        if !bg.background_image.0.is_empty()
            && let Image::Gradient(boxed_gradient) = &bg.background_image.0[0]
        {
            let gradient = &**boxed_gradient;
            match gradient {
                GenericGradient::Linear {
                    direction, items, ..
                } => {
                    let angle = Self::gradient_direction_to_angle(direction);
                    let stops = Self::gradient_stops_from_stylo(items, text_color);
                    if !stops.is_empty() {
                        return BackgroundValue::LinearGradient {
                            angle_degrees: angle,
                            stops,
                        };
                    }
                }
                GenericGradient::Radial { items, .. } => {
                    let stops = Self::gradient_stops_from_stylo(items, text_color);
                    if !stops.is_empty() {
                        return BackgroundValue::RadialGradient { stops };
                    }
                }
                _ => {} // conic gradients not supported yet
            }
        }

        // Fall back to background-color
        let color = if bg.background_color.is_currentcolor() {
            Self::color_from_absolute(text_color)
        } else {
            Self::color_from_stylo(&bg.background_color)
        };
        match color {
            Some(c) => BackgroundValue::Color(c),
            None => BackgroundValue::None,
        }
    }

    fn gradient_direction_to_angle(
        direction: &style::values::computed::image::LineDirection,
    ) -> f32 {
        use style::values::computed::image::LineDirection;
        use style::values::specified::position::{
            HorizontalPositionKeyword, VerticalPositionKeyword,
        };
        match direction {
            LineDirection::Angle(angle) => angle.degrees(),
            LineDirection::Horizontal(h) => match *h {
                HorizontalPositionKeyword::Left => 270.0,
                HorizontalPositionKeyword::Right => 90.0,
            },
            LineDirection::Vertical(v) => match *v {
                VerticalPositionKeyword::Top => 0.0,
                VerticalPositionKeyword::Bottom => 180.0,
            },
            LineDirection::Corner(h, v) => match (h, v) {
                (HorizontalPositionKeyword::Right, VerticalPositionKeyword::Top) => 45.0,
                (HorizontalPositionKeyword::Right, VerticalPositionKeyword::Bottom) => 135.0,
                (HorizontalPositionKeyword::Left, VerticalPositionKeyword::Bottom) => 225.0,
                (HorizontalPositionKeyword::Left, VerticalPositionKeyword::Top) => 315.0,
            },
        }
    }

    fn gradient_stops_from_stylo(
        items: &[style::values::generics::image::GenericGradientItem<
            style::values::computed::color::Color,
            style::values::computed::LengthPercentage,
        >],
        text_color: &style::color::AbsoluteColor,
    ) -> Vec<GradientStop> {
        use style::values::generics::image::GenericGradientItem;

        let mut stops = Vec::new();
        let total = items.len();

        for (i, item) in items.iter().enumerate() {
            match item {
                GenericGradientItem::SimpleColorStop(color) => {
                    let c = if color.is_currentcolor() {
                        Self::color_from_absolute(text_color)
                    } else {
                        Self::color_from_stylo(color)
                    };
                    // Auto-distribute position
                    let offset = if total <= 1 {
                        0.0
                    } else {
                        i as f32 / (total - 1) as f32
                    };
                    stops.push(GradientStop { offset, color: c });
                }
                GenericGradientItem::ComplexColorStop { color, position } => {
                    let c = if color.is_currentcolor() {
                        Self::color_from_absolute(text_color)
                    } else {
                        Self::color_from_stylo(color)
                    };
                    let offset = if let Some(pct) = position.to_percentage() {
                        pct.0
                    } else if let Some(len) = position.to_length() {
                        // Length stops need container size to resolve — approximate
                        len.px() / 100.0
                    } else {
                        i as f32 / (total - 1).max(1) as f32
                    };
                    stops.push(GradientStop { offset, color: c });
                }
                _ => {}
            }
        }
        stops
    }

    fn extract_filter_brightness(
        filter: &style::properties::longhands::filter::computed_value::T,
    ) -> f32 {
        use style::values::generics::effects::GenericFilter;
        for f in &*filter.0 {
            if let GenericFilter::Brightness(val) = f {
                return val.0;
            }
        }
        1.0
    }

    fn extract_filter_grayscale(
        filter: &style::properties::longhands::filter::computed_value::T,
    ) -> f32 {
        use style::values::generics::effects::GenericFilter;
        for f in &*filter.0 {
            if let GenericFilter::Grayscale(val) = f {
                return val.0;
            }
        }
        0.0
    }

    fn extract_filter_saturate(
        filter: &style::properties::longhands::filter::computed_value::T,
    ) -> f32 {
        use style::values::generics::effects::GenericFilter;
        for f in &*filter.0 {
            if let GenericFilter::Saturate(val) = f {
                return val.0;
            }
        }
        1.0
    }

    fn extract_filter_hue_rotate(
        filter: &style::properties::longhands::filter::computed_value::T,
    ) -> f32 {
        use style::values::generics::effects::GenericFilter;
        for f in &*filter.0 {
            if let GenericFilter::HueRotate(angle) = f {
                return angle.degrees();
            }
        }
        0.0
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parse a pixel value like "10px" or "10" to f32.
fn parse_px_value(value: &str) -> f32 {
    let v = value.trim().strip_suffix("px").unwrap_or(value.trim());
    v.parse().unwrap_or(0.0)
}

/// Parse font-size CSS value to f32 pixels.
fn parse_font_size_value(value: &str) -> f32 {
    let value = value.trim();
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse().unwrap_or(16.0);
    }
    if let Some(rem) = value.strip_suffix("rem") {
        return rem.trim().parse::<f32>().unwrap_or(1.0) * 16.0;
    }
    value.parse().unwrap_or(16.0)
}

/// Parse font-weight CSS value to f32.
fn parse_font_weight_value(value: &str) -> f32 {
    match value.trim() {
        "normal" => 400.0,
        "bold" => 700.0,
        "lighter" => 300.0,
        "bolder" => 800.0,
        _ => value.parse().unwrap_or(400.0),
    }
}

/// Parse a CSS grid-template-columns/rows value into Taffy GridTemplateComponent.
///
/// Supports:
/// - `repeat(N, 1fr)` → N columns of 1fr
/// - `repeat(auto-fill, minmax(Xpx, 1fr))` → auto-fill with minmax
/// - `repeat(auto-fit, minmax(Xpx, 1fr))` → auto-fit with minmax
/// - `Npx Npx ...` → explicit pixel tracks
/// - `1fr 1fr ...` → explicit fr tracks
fn parse_grid_template(value: &str) -> Vec<taffy::GridTemplateComponent<String>> {
    use taffy::RepetitionCount;
    use taffy::style_helpers::repeat;

    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }

    // Handle repeat(...)
    if let Some(inner) = value
        .strip_prefix("repeat(")
        .and_then(|s| s.strip_suffix(')'))
        && let Some((count_str, tracks_str)) = inner.split_once(',')
    {
        let count_str = count_str.trim();
        let tracks_str = tracks_str.trim();

        let repetition = match count_str {
            "auto-fill" => RepetitionCount::AutoFill,
            "auto-fit" => RepetitionCount::AutoFit,
            _ => {
                if let Ok(n) = count_str.parse::<u16>() {
                    RepetitionCount::Count(n)
                } else {
                    return Vec::new();
                }
            }
        };

        let track = parse_single_track(tracks_str);
        return vec![repeat(repetition, vec![track])];
    }

    // Handle space-separated track list: "1fr 1fr 1fr" or "100px 200px"
    value
        .split_whitespace()
        .map(|part| taffy::GridTemplateComponent::Single(parse_single_track(part)))
        .collect()
}

/// Parse a single track sizing value, which may be `minmax(min, max)` or a simple value.
fn parse_single_track(value: &str) -> taffy::TrackSizingFunction {
    use taffy::style_helpers::minmax;

    let value = value.trim();

    // Handle minmax(min, max)
    if let Some(inner) = value
        .strip_prefix("minmax(")
        .and_then(|s| s.strip_suffix(')'))
        && let Some((min_str, max_str)) = inner.split_once(',')
    {
        let min = parse_min_track(min_str.trim());
        let max = parse_max_track(max_str.trim());
        return minmax(min, max);
    }

    // Simple value
    parse_non_repeated_track(value)
}

/// Parse a non-repeated track sizing function from a string like "1fr", "100px", "auto".
fn parse_non_repeated_track(value: &str) -> taffy::TrackSizingFunction {
    use taffy::style_helpers::minmax;

    let value = value.trim();
    if let Some(fr_val) = value.strip_suffix("fr") {
        let f = fr_val.trim().parse::<f32>().unwrap_or(1.0);
        minmax(
            taffy::MinTrackSizingFunction::auto(),
            taffy::MaxTrackSizingFunction::fr(f),
        )
    } else if value == "auto" {
        minmax(
            taffy::MinTrackSizingFunction::auto(),
            taffy::MaxTrackSizingFunction::auto(),
        )
    } else if value == "min-content" {
        minmax(
            taffy::MinTrackSizingFunction::min_content(),
            taffy::MaxTrackSizingFunction::min_content(),
        )
    } else if value == "max-content" {
        minmax(
            taffy::MinTrackSizingFunction::max_content(),
            taffy::MaxTrackSizingFunction::max_content(),
        )
    } else if let Some(pct) = value.strip_suffix('%') {
        let p = pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
        minmax(
            taffy::MinTrackSizingFunction::percent(p),
            taffy::MaxTrackSizingFunction::percent(p),
        )
    } else {
        // Assume px value
        let px = value.strip_suffix("px").unwrap_or(value);
        let v = px.trim().parse::<f32>().unwrap_or(0.0);
        minmax(
            taffy::MinTrackSizingFunction::length(v),
            taffy::MaxTrackSizingFunction::length(v),
        )
    }
}

/// Parse a min track sizing function.
fn parse_min_track(value: &str) -> taffy::MinTrackSizingFunction {
    match value {
        "auto" => taffy::MinTrackSizingFunction::auto(),
        "min-content" => taffy::MinTrackSizingFunction::min_content(),
        "max-content" => taffy::MinTrackSizingFunction::max_content(),
        _ => {
            if let Some(pct) = value.strip_suffix('%') {
                let p = pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
                taffy::MinTrackSizingFunction::percent(p)
            } else {
                let px = value.strip_suffix("px").unwrap_or(value);
                let v = px.trim().parse::<f32>().unwrap_or(0.0);
                taffy::MinTrackSizingFunction::length(v)
            }
        }
    }
}

/// Parse a max track sizing function.
fn parse_max_track(value: &str) -> taffy::MaxTrackSizingFunction {
    match value {
        "auto" => taffy::MaxTrackSizingFunction::auto(),
        "min-content" => taffy::MaxTrackSizingFunction::min_content(),
        "max-content" => taffy::MaxTrackSizingFunction::max_content(),
        _ => {
            if let Some(fr_val) = value.strip_suffix("fr") {
                let f = fr_val.trim().parse::<f32>().unwrap_or(1.0);
                taffy::MaxTrackSizingFunction::fr(f)
            } else if let Some(pct) = value.strip_suffix('%') {
                let p = pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
                taffy::MaxTrackSizingFunction::percent(p)
            } else {
                let px = value.strip_suffix("px").unwrap_or(value);
                let v = px.trim().parse::<f32>().unwrap_or(0.0);
                taffy::MaxTrackSizingFunction::length(v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_value_parse() {
        assert_eq!(DisplayValue::parse("flex"), DisplayValue::Flex);
        assert_eq!(DisplayValue::parse("block"), DisplayValue::Block);
        assert_eq!(DisplayValue::parse("none"), DisplayValue::None);
        assert_eq!(DisplayValue::parse("inline-flex"), DisplayValue::InlineFlex);
    }

    #[test]
    fn test_dimension_value_parse() {
        let vp = Viewport {
            width: 1000.0,
            height: 800.0,
        };
        assert!(matches!(
            DimensionValue::parse("auto", &vp),
            DimensionValue::Auto
        ));
        assert!(
            matches!(DimensionValue::parse("100px", &vp), DimensionValue::Length(v) if (v - 100.0).abs() < 0.01)
        );
        assert!(
            matches!(DimensionValue::parse("50%", &vp), DimensionValue::Percent(v) if (v - 0.5).abs() < 0.01)
        );
        assert!(
            matches!(DimensionValue::parse("10vh", &vp), DimensionValue::Length(v) if (v - 80.0).abs() < 0.01)
        );
        assert!(
            matches!(DimensionValue::parse("10vw", &vp), DimensionValue::Length(v) if (v - 100.0).abs() < 0.01)
        );
        assert!(
            matches!(DimensionValue::parse("2rem", &vp), DimensionValue::Length(v) if (v - 32.0).abs() < 0.01)
        );
    }

    #[test]
    fn test_computed_style_from_props() {
        let vp = Viewport {
            width: 1000.0,
            height: 800.0,
        };
        let mut props = HashMap::new();
        props.insert("display".to_string(), "flex".to_string());
        props.insert("width".to_string(), "100px".to_string());
        props.insert("flex-grow".to_string(), "1".to_string());
        props.insert("padding-top".to_string(), "10px".to_string());

        let style = ComputedStyle::from_props(&props, &vp);
        assert_eq!(style.display, DisplayValue::Flex);
        assert!(matches!(style.width, DimensionValue::Length(v) if (v - 100.0).abs() < 0.01));
        assert!((style.flex_grow - 1.0).abs() < 0.01);
        assert!(
            matches!(style.padding_top, LengthPercentageValue::Length(v) if (v - 10.0).abs() < 0.01)
        );
    }

    #[test]
    fn test_flex_shorthand() {
        let vp = Viewport::default();
        let mut props = HashMap::new();
        props.insert("flex".to_string(), "1".to_string());

        let style = ComputedStyle::from_props(&props, &vp);
        assert!((style.flex_grow - 1.0).abs() < 0.01);
        assert!((style.flex_shrink - 1.0).abs() < 0.01);
        assert!(matches!(style.flex_basis, DimensionValue::Length(v) if v.abs() < 0.01));
    }

    #[test]
    fn test_to_taffy_style() {
        let vp = Viewport::default();
        let mut props = HashMap::new();
        props.insert("display".to_string(), "flex".to_string());
        props.insert("flex-direction".to_string(), "column".to_string());
        props.insert("width".to_string(), "200px".to_string());

        let style = ComputedStyle::from_props(&props, &vp);
        let taffy_style = style.to_taffy_style(crate::layout::DefaultDisplay::Block);

        assert_eq!(taffy_style.display, taffy::Display::Flex);
        assert_eq!(taffy_style.flex_direction, taffy::FlexDirection::Column);
    }
}
