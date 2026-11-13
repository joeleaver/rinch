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

        let display = self.display.to_taffy();
        // Whether `justify-content` aligns along the inline axis: a grid's
        // columns, a row flex container's main axis.
        let justify_axis_is_inline = match display {
            Display::Grid => true,
            Display::Flex => matches!(
                flex_direction,
                taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
            ),
            _ => false,
        };

        let mut style = Style {
            // `box-sizing` (#1278): Taffy's default is `border-box`, which
            // used to size every box whatever its CSS said; the initial value
            // is `content-box`. The theme sets `* { box-sizing: border-box }`.
            box_sizing: if self.box_sizing_border_box {
                BoxSizing::BorderBox
            } else {
                BoxSizing::ContentBox
            },
            display,
            position: self.position.to_taffy(),
            float: self.float,
            clear: self.clear,
            overflow: taffy::Point {
                x: self.overflow_x.to_taffy(),
                y: self.overflow_y.to_taffy(),
            },

            size: Size {
                width: self.width.to_taffy(),
                height: self.height.to_taffy(),
            },
            min_size: Size {
                width: self.min_width.to_taffy_lpa(),
                height: self.min_height.to_taffy_lpa(),
            },
            max_size: Size {
                width: self.max_width.to_taffy_lpa(),
                height: self.max_height.to_taffy_lpa(),
            },

            flex_direction,
            flex_wrap,
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            flex_basis: self.flex_basis.to_taffy(),
            align_items: self.align_items.map(|v| v.to_taffy()),
            // An out-of-flow child of a flex container takes its static
            // position as if it were the sole flex item (css-flexbox-1 §4.1):
            // its own `align-self`, else the container's `align-items`, on
            // the cross axis — `stretch` as the start, and nothing stretched,
            // which Taffy's absolute layout does itself. (This used to force
            // `flex-start` on every out-of-flow box with no `align-self` of
            // its own, so the cross axis ignored `align-items`: #1492.)
            align_self: self.align_self.map(|v| v.to_taffy()),
            justify_content: self
                .justify_content
                .map(|v| v.to_taffy(self.direction_rtl, justify_axis_is_inline)),
            // Read by Taffy's grid and block layout (#1985); a flex
            // container ignores both. `justify-self`'s `left` / `right` and
            // baselines are the parent's business: these are this box's own
            // answers, which `taffy_style_from_computed` redoes against the
            // box's parent.
            justify_items: self
                .justify_items
                .map(|v| v.to_taffy(self.direction_rtl, display == Display::Block)),
            justify_self: self
                .justify_self
                .map(|v| v.to_taffy(self.direction_rtl, false)),
            // `direction` (#1995): Taffy's grid and flex lay out their
            // columns and rows from the right, and its block layout puts a
            // block-level box at the right, under `rtl`.
            direction: if self.direction_rtl {
                taffy::Direction::Rtl
            } else {
                taffy::Direction::Ltr
            },

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
            grid_column: self.grid_column.clone(),
            grid_row: self.grid_row.clone(),

            // `aspect-ratio` (#1286). `auto` matters only to a box with a
            // natural ratio, which this is not: a replaced element's ratio is
            // its measure's business (`taffy_style_from_computed` clears this
            // for one, `crate::replaced`).
            aspect_ratio: self.aspect_ratio.ratio,

            // `display: flow-root` establishes a block formatting context
            // (#2351). Paint containment is how Taffy's block layout is told
            // so (the same predicate as a scroll container's), and it has no
            // other effect on layout: it does not suppress the baseline, as
            // layout containment would. CSS `contain` itself is not mapped.
            contain: if self.flow_root {
                taffy::Contain::PAINT
            } else {
                taffy::Contain::NONE
            },

            ..Default::default()
        };
        self.legacy_box_taffy_style(&mut style);
        style
    }

    /// What `display: -webkit-box` makes of the Taffy style (#2581), as
    /// Chrome 155 lays it out.
    ///
    /// - A box that clamps (`line_clamp`) is the `flow-root` Stylo was given,
    ///   a block formatting context (`ComputedStyle::flow_root`, #2351).
    /// - Any other legacy box is a flex container, a column under
    ///   `-webkit-box-orient: vertical` and a row otherwise, that does not
    ///   wrap. Its computed `display` stays the block Stylo was given, so
    ///   that its runs of inline content are each one anonymous block box
    ///   (`create_anonymous_block_boxes`), which Taffy then lays out as one
    ///   item, as Chrome does; a box holding only inline content is an IFC
    ///   root and a Taffy leaf, which no `display` changes.
    /// - Its items grow and shrink by `-webkit-box-flex` alone (`0`, neither,
    ///   unless declared), and in a row an item of `auto` width is at most as
    ///   wide as the box (Chrome sizes it to fit the box: `short` and a long
    ///   paragraph in a 230px box are 39.02 and 230 wide).
    pub(crate) fn legacy_box_taffy_style(&self, style: &mut taffy::Style) {
        if self.webkit_box && self.line_clamp.is_none() {
            style.display = taffy::Display::Flex;
            style.flex_direction = if self.webkit_box_vertical {
                taffy::FlexDirection::Column
            } else {
                taffy::FlexDirection::Row
            };
            style.flex_wrap = taffy::FlexWrap::NoWrap;
        }
        if let Some(vertical) = self.webkit_box_item {
            legacy_item_taffy_style(style, self.webkit_box_flex, vertical);
        }
    }
}

/// An item of a legacy flex `-webkit-box` (see
/// [`ComputedStyle::legacy_box_taffy_style`]), whose `-webkit-box-flex` is
/// `flex`: an element, or an anonymous block box holding a run of inline
/// content (`flex` 0).
pub(crate) fn legacy_item_taffy_style(style: &mut taffy::Style, flex: f32, vertical: bool) {
    style.flex_grow = flex;
    style.flex_shrink = flex;
    style.flex_basis = taffy::Dimension::auto();
    if !vertical && style.size.width.is_auto() && style.max_size.width.is_auto() {
        style.max_size.width = taffy::LengthPercentageAuto::percent(1.0);
    }
}
