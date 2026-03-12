//! Conversion from ComputedStyle to Taffy Style.

use super::ComputedStyle;
use super::values::*;

impl ComputedStyle {
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
            // CSS spec: absolute/fixed elements are NOT flex items, so the parent's
            // align-items: stretch shouldn't stretch them.  Taffy doesn't distinguish,
            // so force flex-start when align-self isn't explicitly set.
            align_self: if self.align_self.is_none()
                && matches!(
                    self.position,
                    PositionValue::Absolute | PositionValue::Fixed
                ) {
                Some(taffy::AlignSelf::FlexStart)
            } else {
                self.align_self.map(|v| v.to_taffy())
            },
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
}
