//! Stylo-to-ComputedStyle conversion methods.

mod box_model;
pub(crate) mod color;
mod grid;
mod layout;
mod typography;
pub(crate) mod visual;

use super::ComputedStyle;
use super::values::*;
use style::properties::ComputedValues;

use box_model::*;
use color::*;
use grid::*;
use layout::*;
use typography::*;
use visual::*;

impl ComputedStyle {
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
            display: display_from_stylo(&box_style.display),
            position: position_from_stylo(&box_style.position),
            overflow_x: overflow_from_stylo(&box_style.overflow_x),
            overflow_y: overflow_from_stylo(&box_style.overflow_y),

            // Dimensions
            width: size_from_stylo(&position_style.width),
            height: size_from_stylo(&position_style.height),
            min_width: size_from_stylo(&position_style.min_width),
            min_height: size_from_stylo(&position_style.min_height),
            max_width: max_size_from_stylo(&position_style.max_width),
            max_height: max_size_from_stylo(&position_style.max_height),

            // Flexbox
            flex_direction: flex_direction_from_stylo(&position_style.flex_direction),
            flex_wrap: flex_wrap_from_stylo(&position_style.flex_wrap),
            flex_grow: position_style.flex_grow.0,
            flex_shrink: position_style.flex_shrink.0,
            flex_basis: flex_basis_from_stylo(&position_style.flex_basis),
            align_items: align_items_from_stylo(&position_style.align_items),
            align_self: align_self_from_stylo(&position_style.align_self),
            justify_content: justify_content_from_stylo(&position_style.justify_content),

            // Padding
            padding_top: length_percentage_from_stylo(&padding.padding_top),
            padding_right: length_percentage_from_stylo(&padding.padding_right),
            padding_bottom: length_percentage_from_stylo(&padding.padding_bottom),
            padding_left: length_percentage_from_stylo(&padding.padding_left),

            // Margin
            margin_top: margin_from_stylo_generic(&margin.margin_top),
            margin_right: margin_from_stylo_generic(&margin.margin_right),
            margin_bottom: margin_from_stylo_generic(&margin.margin_bottom),
            margin_left: margin_from_stylo_generic(&margin.margin_left),

            // Gap
            gap_row: gap_from_stylo(&position_style.row_gap),
            gap_column: gap_from_stylo(&position_style.column_gap),

            // Inset
            top: inset_from_stylo_generic(&position_style.top),
            right: inset_from_stylo_generic(&position_style.right),
            bottom: inset_from_stylo_generic(&position_style.bottom),
            left: inset_from_stylo_generic(&position_style.left),

            // Border widths (BorderSideWidth is a newtype wrapper around NonNegativeLength)
            // Check border-style: if style is 'none' or 'hidden', width should be 0
            border_top_width: if border_style_is_none(&border.border_top_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_top_width.0.to_f32_px())
            },
            border_right_width: if border_style_is_none(&border.border_right_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_right_width.0.to_f32_px())
            },
            border_bottom_width: if border_style_is_none(&border.border_bottom_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_bottom_width.0.to_f32_px())
            },
            border_left_width: if border_style_is_none(&border.border_left_style) {
                LengthPercentageValue::Length(0.0)
            } else {
                LengthPercentageValue::Length(border.border_left_width.0.to_f32_px())
            },

            // Border radius
            border_radius_top_left: border_radius_from_stylo(&border.border_top_left_radius),
            border_radius_top_right: border_radius_from_stylo(&border.border_top_right_radius),
            border_radius_bottom_right: border_radius_from_stylo(
                &border.border_bottom_right_radius,
            ),
            border_radius_bottom_left: border_radius_from_stylo(&border.border_bottom_left_radius),

            // Border styles (per-side)
            border_top_style: border_style_from_stylo(&border.border_top_style),
            border_right_style: border_style_from_stylo(&border.border_right_style),
            border_bottom_style: border_style_from_stylo(&border.border_bottom_style),
            border_left_style: border_style_from_stylo(&border.border_left_style),

            // Border colors (per-side, resolve currentColor)
            border_top_color: color_from_computed(&border.border_top_color, &text.color),
            border_right_color: color_from_computed(&border.border_right_color, &text.color),
            border_bottom_color: color_from_computed(&border.border_bottom_color, &text.color),
            border_left_color: color_from_computed(&border.border_left_color, &text.color),

            // Background (color or gradient)
            background: background_from_stylo(background, &text.color),
            color: color_from_absolute(&text.color),

            // Visual
            opacity: effects.opacity,
            visibility: visibility_from_stylo(&inherited_box.visibility),

            // Transforms
            transform: transform_from_stylo(&box_style.transform),
            transform_origin_x: transform_origin_component_from_stylo(
                &box_style.transform_origin.horizontal,
            ),
            transform_origin_y: transform_origin_component_from_stylo(
                &box_style.transform_origin.vertical,
            ),

            // Z-index
            z_index: z_index_from_stylo(&position_style.z_index),

            // Text shadow
            text_shadow: text_shadow_from_stylo(&text.text_shadow, &text.color),

            // Box shadow
            box_shadow: box_shadow_from_stylo(&effects.box_shadow, &text.color),

            // Outline
            outline_width: if border_style_is_none_val(&border_style_from_stylo_outline(
                &outline_style.outline_style,
            )) {
                0.0
            } else {
                outline_style.outline_width.0.to_f32_px()
            },
            outline_color: color_from_computed(&outline_style.outline_color, &text.color),
            outline_style: border_style_from_stylo_outline(&outline_style.outline_style),
            outline_offset: outline_style.outline_offset.to_f32_px(),

            // Filters
            filter_brightness: extract_filter_brightness(&effects.filter),
            filter_grayscale: extract_filter_grayscale(&effects.filter),
            filter_saturate: extract_filter_saturate(&effects.filter),
            filter_hue_rotate: extract_filter_hue_rotate(&effects.filter),

            // Object fit
            object_fit: object_fit_from_stylo(&cv.clone_object_fit()),

            // Cursor
            cursor: cursor_from_stylo(&inherited_ui.cursor.keyword),

            // Pointer events
            pointer_events: pointer_events_from_stylo(&inherited_ui.pointer_events),

            // User select (not available in Stylo servo build — defaults to Auto,
            // resolved per-tag in style_resolution/mod.rs)
            user_select: UserSelectValue::Auto,

            // Typography
            font_size: font.font_size.computed_size().px(),
            font_weight: font.font_weight.value(),
            font_family: font_family_from_stylo(&font.font_family),
            font_style: font_style_from_stylo(&font.font_style),
            line_height: line_height_from_stylo(&font.line_height),
            letter_spacing: letter_spacing_from_stylo(&text.letter_spacing),
            word_spacing: word_spacing_from_stylo(&text.word_spacing),
            text_align: text_align_from_stylo(&text.text_align),
            text_decoration: text_decoration_from_stylo(
                &cv.get_text().clone_text_decoration_line(),
            ),
            text_transform: text_transform_from_stylo(&text.text_transform),
            // text-underline-offset is gecko-only in Stylo; parsed from inline styles
            text_underline_offset: None,
            white_space: white_space_from_stylo(&text.white_space_collapse, &text.text_wrap_mode),
            overflow_wrap: overflow_wrap_from_stylo(&text.overflow_wrap),
            text_overflow: text_overflow_from_stylo(&cv.clone_text_overflow()),

            // Grid - extract from Stylo
            grid_template_columns: grid_template_tracks_from_stylo(
                &position_style.grid_template_columns,
            ),
            grid_template_rows: grid_template_tracks_from_stylo(&position_style.grid_template_rows),
            grid_auto_flow: grid_auto_flow_from_stylo(&position_style.grid_auto_flow),
            grid_column: grid_placement_from_stylo(
                &position_style.grid_column_start,
                &position_style.grid_column_end,
            ),
            grid_row: grid_placement_from_stylo(
                &position_style.grid_row_start,
                &position_style.grid_row_end,
            ),

            // Display was explicitly set by Stylo
            has_explicit_display: !box_style.display.is_none(),
        }
    }
}
