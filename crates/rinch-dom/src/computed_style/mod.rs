//! Typed computed style representation.
//!
//! Provides a pre-parsed ComputedStyle struct to avoid re-parsing CSS properties
//! on every layout and paint operation.

mod from_stylo;
mod taffy_conversion;
mod text_layout;
pub mod values;

pub(crate) use from_stylo::color::{
    absolute_from_peniko, color_from_computed, color_from_specified, color_from_stylo,
};
pub(crate) use from_stylo::visual::accumulate_pct;
pub use values::*;

use serde::Serialize;

/// Pre-parsed CSS properties for efficient layout and paint.
#[derive(Debug, Clone, Serialize)]
pub struct ComputedStyle {
    // Display/position
    pub display: DisplayValue,
    pub position: PositionValue,
    pub overflow_x: OverflowValue,
    pub overflow_y: OverflowValue,
    /// How the overlay scrollbar of a scroll container is drawn. Both come
    /// from `--rinch-*` custom properties rather than the real CSS
    /// `scrollbar-color` / `scrollbar-width`, which the servo build of Stylo
    /// compiles out — see [`ScrollbarColorValue`].
    pub scrollbar_color: ScrollbarColorValue,
    pub scrollbar_width: ScrollbarWidthValue,

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
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub border_top_color: Option<peniko::Color>,
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub border_right_color: Option<peniko::Color>,
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub border_bottom_color: Option<peniko::Color>,
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub border_left_color: Option<peniko::Color>,

    // Colors
    pub background: BackgroundValue,
    #[serde(serialize_with = "values::color_serde::serialize")]
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

    // Box shadow
    pub box_shadow: Vec<BoxShadowValue>,

    // Outline
    pub outline_width: f32,
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub outline_color: Option<peniko::Color>,
    pub outline_style: BorderStyleValue,
    pub outline_offset: f32,

    // Filters (partial — non-blur only)
    pub filter_brightness: f32,
    pub filter_grayscale: f32,
    pub filter_saturate: f32,
    pub filter_hue_rotate: f32,

    // Object fit (for <img> elements)
    pub object_fit: ObjectFitValue,

    // Cursor
    pub cursor: CursorValue,

    // Pointer events
    pub pointer_events: PointerEventsValue,

    // User select
    pub user_select: UserSelectValue,

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
    pub text_transform: TextTransformValue,
    pub text_underline_offset: Option<f32>,
    pub white_space: WhiteSpaceValue,
    pub overflow_wrap: OverflowWrapValue,
    pub text_overflow: TextOverflowValue,

    // Grid properties - skip serialization (taffy types don't impl Serialize)
    #[serde(skip)]
    pub grid_template_columns: Vec<taffy::GridTemplateComponent<String>>,
    #[serde(skip)]
    pub grid_template_rows: Vec<taffy::GridTemplateComponent<String>>,
    #[serde(skip)]
    pub grid_auto_flow: taffy::GridAutoFlow,
    /// Grid item column placement (`grid-column`) — honors `span N` and line
    /// numbers, so a `colspan`-style cell occupies several grid columns.
    #[serde(skip)]
    pub grid_column: taffy::Line<taffy::GridPlacement<String>>,
    /// Grid item row placement (`grid-row`) — the `rowspan` analogue.
    #[serde(skip)]
    pub grid_row: taffy::Line<taffy::GridPlacement<String>>,

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
            scrollbar_color: ScrollbarColorValue::default(),
            scrollbar_width: ScrollbarWidthValue::default(),

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
            box_shadow: Vec::new(),

            outline_width: 0.0,
            outline_color: None,
            outline_style: BorderStyleValue::None,
            outline_offset: 0.0,

            filter_brightness: 1.0,
            filter_grayscale: 0.0,
            filter_saturate: 1.0,
            filter_hue_rotate: 0.0,

            object_fit: ObjectFitValue::default(),

            cursor: CursorValue::default(),
            pointer_events: PointerEventsValue::default(),
            user_select: UserSelectValue::default(),

            font_size: 16.0,
            font_weight: 400.0,
            font_family: String::new(),
            font_style: FontStyleValue::default(),
            line_height: LineHeightValue::default(),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_align: TextAlignValue::default(),
            text_decoration: TextDecorationValue::default(),
            text_transform: TextTransformValue::default(),
            text_underline_offset: None,
            white_space: WhiteSpaceValue::default(),
            overflow_wrap: OverflowWrapValue::default(),
            text_overflow: TextOverflowValue::default(),

            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_flow: taffy::GridAutoFlow::Row,
            grid_column: taffy::Line {
                start: taffy::GridPlacement::Auto,
                end: taffy::GridPlacement::Auto,
            },
            grid_row: taffy::Line {
                start: taffy::GridPlacement::Auto,
                end: taffy::GridPlacement::Auto,
            },

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

    /// Resolve `line-height` to pixels against this style's `font-size`.
    ///
    /// `normal` is the 1.2 factor rinch uses everywhere a line box has to be
    /// sized without Parley metrics (the empty-block floor, `<textarea rows>`).
    pub fn line_height_px(&self) -> f32 {
        match self.line_height {
            LineHeightValue::Normal => self.font_size * 1.2,
            LineHeightValue::Relative(r) => self.font_size * r,
            LineHeightValue::Absolute(px) => px,
        }
    }
}

#[cfg(test)]
mod tests;
