//! CSS property parsing into ComputedStyle.

use std::collections::HashMap;

use super::ComputedStyle;
use super::helpers::*;
use super::values::*;
use crate::layout::{Viewport, parse_color};

impl ComputedStyle {
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
                "background-image" => {
                    if let Some(url) = value.strip_prefix("url(").and_then(|v| v.strip_suffix(')'))
                    {
                        let url = url.trim_matches(|c| c == '"' || c == '\'').trim();
                        if !url.is_empty() {
                            style.background = BackgroundValue::Image {
                                url: url.to_string(),
                            };
                        }
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
}
