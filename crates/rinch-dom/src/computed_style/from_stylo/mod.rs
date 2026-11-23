//! Stylo-to-ComputedStyle conversion methods.

mod box_model;
mod calc;
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
pub(crate) use typography::{font_family_from_stylo, font_style_from_stylo};
use visual::*;

thread_local! {
    /// The two custom-property names, interned once per thread rather than on
    /// every node: `from_stylo` runs for each dirty node on each style pass,
    /// and `Atom::from(&str)` is an atom-table lookup, not free.
    ///
    /// No leading `--`: Stylo's `custom_properties::Name` is the atom *without*
    /// the prefix.
    static SCROLLBAR_COLOR: style::Atom = style::Atom::from("rinch-scrollbar-color");
    static SCROLLBAR_WIDTH: style::Atom = style::Atom::from("rinch-scrollbar-width");
    /// `hyphens`, rewritten into a custom property (#2621).
    static HYPHENS: style::Atom = style::Atom::from(crate::webkit_box::HYPHENS);
    /// Every `word-break`, mirrored into a custom property (#2628).
    static WORD_BREAK: style::Atom = style::Atom::from(crate::webkit_box::WORD_BREAK);
    /// `text-indent`'s keywords, mirrored into a custom property (#2628).
    static TEXT_INDENT: style::Atom = style::Atom::from(crate::webkit_box::TEXT_INDENT);
    /// The text-paint properties, rewritten into custom properties (#2719).
    static TEXT_FILL_COLOR: style::Atom = style::Atom::from(crate::webkit_box::TEXT_FILL_COLOR);
    static TEXT_STROKE_WIDTH: style::Atom =
        style::Atom::from(crate::webkit_box::TEXT_STROKE_WIDTH);
    static TEXT_STROKE_COLOR: style::Atom =
        style::Atom::from(crate::webkit_box::TEXT_STROKE_COLOR);
    static PAINT_ORDER: style::Atom = style::Atom::from(crate::webkit_box::PAINT_ORDER);
}

/// `-webkit-text-fill-color`, `-webkit-text-stroke-*` and `paint-order`
/// (#2719), from the custom properties they were rewritten into. Each
/// inherits as a custom property does, as its token string: `currentcolor`
/// and an `em` width are resolved here, against this element's `color` and
/// `font-size` (Chrome resolves an `em` width where it is declared and
/// inherits the length). [`TextPaintValue::PLAIN`] on a page that never
/// wrote one, with no look-up.
///
/// A value that is no value of its property — what a `var()` substituted
/// into it can leave (#2820) — is read as the initial value here; the
/// cascade then gives such an element its parent's value instead, as Chrome
/// does for a declaration invalid at computed-value time
/// ([`ComputedStyle::inherit_invalid_text_paint`]).
fn text_paint_from_stylo(
    cv: &ComputedValues,
    color: Option<peniko::Color>,
    font_size: f32,
) -> TextPaintValue {
    if !crate::webkit_box::text_paint_seen() {
        return TextPaintValue::PLAIN;
    }
    let props = cv.custom_properties();
    if props.inherited.is_empty() && props.non_inherited.is_empty() {
        return TextPaintValue::PLAIN;
    }
    text_paint_with(cv, color, font_size, |_| false, &TextPaintValue::PLAIN)
}

/// The text-paint values, each read off its custom property: a computed
/// value that parses is taken; one that does not, or a property `inherits`
/// says the element takes from its parent though it has no value, is
/// `parent`'s.
fn text_paint_with(
    cv: &ComputedValues,
    color: Option<peniko::Color>,
    font_size: f32,
    inherits: impl Fn(&str) -> bool,
    parent: &TextPaintValue,
) -> TextPaintValue {
    use crate::webkit_box::{
        PAINT_ORDER as ORDER, TEXT_FILL_COLOR as FILL, TEXT_STROKE_COLOR as STROKE,
        TEXT_STROKE_WIDTH as WIDTH,
    };
    let current = color.unwrap_or(peniko::Color::BLACK);
    // A substituted `initial` (`var(--x, initial)`, #2870) is the initial
    // value; any other CSS-wide keyword parses as no value, and inherits.
    let initial = |v: &str| v.eq_ignore_ascii_case("initial");
    // `Some(Some(v))` parsed, `Some(None)` is no value of the property, `None`
    // no computed value at all.
    let fill = match custom_property(cv, &TEXT_FILL_COLOR, |v| {
        if v.eq_ignore_ascii_case("currentcolor") || initial(v) {
            Some(None)
        } else {
            crate::layout::parse_color_with_current(v, current).map(Some)
        }
    }) {
        Some(Some(fill)) => fill,
        Some(None) => parent.fill,
        None if inherits(FILL) => parent.fill,
        None => None,
    };
    // Each `-webkit-text-stroke-*` value is the longhand's, or its half of a
    // shorthand that held a `var()` (#2820, `webkit_box::stroke_half`).
    let stroke_width = match custom_property(cv, &TEXT_STROKE_WIDTH, |v| {
        if initial(v) {
            return Some(0.0);
        }
        crate::webkit_box::stroke_half(v, true)
            .and_then(|v| crate::webkit_box::parse_stroke_width(&v, font_size))
    }) {
        Some(Some(w)) => w,
        Some(None) => parent.stroke_width,
        None if inherits(WIDTH) => parent.stroke_width,
        None => 0.0,
    };
    // The colour, and whether it is `currentcolor`: the parent's
    // `currentcolor` is resolved against this element's `color` (#2870).
    let inherited = || match parent.stroke_color {
        Some(c) if !parent.stroke_currentcolor => (c, false),
        _ => (current, true),
    };
    let stroke = (stroke_width > 0.0).then(|| {
        match custom_property(cv, &TEXT_STROKE_COLOR, |v| {
            if initial(v) {
                return Some((current, true));
            }
            crate::webkit_box::stroke_half(v, false).and_then(|v| {
                if v.eq_ignore_ascii_case("currentcolor") {
                    Some((current, true))
                } else {
                    crate::layout::parse_color_with_current(&v, current).map(|c| (c, false))
                }
            })
        }) {
            Some(Some(c)) => c,
            Some(None) => inherited(),
            None if inherits(STROKE) => inherited(),
            None => (current, true),
        }
    });
    let stroke_first = stroke_width > 0.0
        && match custom_property(cv, &PAINT_ORDER, |v| {
            if initial(v) {
                Some(false)
            } else {
                crate::paint::parse_paint_order(v)
            }
        }) {
            Some(Some(first)) => first,
            Some(None) => parent.stroke_first,
            None if inherits(ORDER) => parent.stroke_first,
            None => false,
        };
    TextPaintValue {
        fill,
        stroke_width,
        stroke_color: stroke.map(|(c, _)| c),
        stroke_currentcolor: stroke.is_some_and(|(_, current)| current),
        stroke_first,
    }
}

impl ComputedStyle {
    /// Give the text-paint values a `var()` left invalid their parent's
    /// (#2820), as Chrome does for a declaration invalid at computed-value
    /// time (the four properties inherit): a computed value that is no value
    /// of its property, or none at all where the element's own winning
    /// declaration is not `initial` (a substitution that failed leaves the
    /// custom property unset). `self` was just converted from `cv`; `parent`
    /// is the parent element's style. Only a parent whose text is painted
    /// otherwise than plainly can make a difference, so nothing is read
    /// for any other.
    pub(crate) fn inherit_invalid_text_paint(
        &mut self,
        cv: &ComputedValues,
        guard: &style::shared_lock::SharedRwLockReadGuard,
        parent: &ComputedStyle,
    ) {
        if !crate::webkit_box::text_paint_seen() || parent.text_paint.is_plain() {
            return;
        }
        self.text_paint = text_paint_with(
            cv,
            self.color,
            self.font_size,
            |name| crate::webkit_box::own_declaration_inherits(cv, guard, name),
            &parent.text_paint,
        );
    }
}

/// `text-indent`: Stylo's length, and the `hanging` / `each-line` keywords
/// from `--rinch-text-indent` (#2628). A declaration holding a `var()` is
/// mirrored behind `webkit_box::VAR_MARKER` and split here, substituted
/// (#2870): where it has a keyword, Stylo's servo build could not parse it
/// (and gave the parent's indent), so the length is read here too. One that
/// substituted no `text-indent` value is left as Stylo made it, for
/// [`ComputedStyle::inherit_invalid_text`].
fn text_indent_with(
    cv: &ComputedValues,
    ti: &style::properties::longhands::text_indent::computed_value::T,
    font_size: f32,
) -> TextIndentValue {
    let mut out = text_indent_from_stylo(ti);
    custom_property(cv, &TEXT_INDENT, |v| {
        let has = |v: &str, k: &str| v.split_whitespace().any(|w| w.eq_ignore_ascii_case(k));
        let Some(sub) = v.strip_prefix(crate::webkit_box::VAR_MARKER) else {
            out.hanging |= has(v, "hanging");
            out.each_line |= has(v, "each-line");
            return;
        };
        let Some((length, keywords)) = crate::webkit_box::split_text_indent(sub.trim()) else {
            return;
        };
        if keywords.eq_ignore_ascii_case("initial") {
            out = TextIndentValue::default();
            return;
        }
        if keywords.eq_ignore_ascii_case("none") {
            return;
        }
        if let Some((px, pct)) = crate::webkit_box::parse_length_percentage(&length, font_size) {
            out.length = match (px, pct) {
                (px, 0.0) => LengthPercentageValue::Length(px),
                (0.0, pct) => LengthPercentageValue::Percent(pct),
                (px, pct) => LengthPercentageValue::Calc { px, pct },
            };
        }
        out.hanging = has(&keywords, "hanging");
        out.each_line = has(&keywords, "each-line");
    });
    out
}

impl ComputedStyle {
    /// Give `hyphens`, `word-break` and `text-indent` values a `var()` left
    /// invalid their parent's (#2870), as Chrome does for a declaration
    /// invalid at computed-value time (the three inherit): the custom
    /// property each is carried in has no value of its property, or none at
    /// all where the element's own winning declaration does not reset it (a
    /// substitution that failed leaves the custom property unset). A
    /// `text-indent` substituted on an ancestor, with no declaration of the
    /// element's own, is the parent's too: its `em` is the ancestor's.
    /// `self` was just converted from `cv`; `parent` is the parent element's
    /// style. Only a value that differs from the parent's is looked at.
    pub(crate) fn inherit_invalid_text(
        &mut self,
        cv: &ComputedValues,
        guard: &style::shared_lock::SharedRwLockReadGuard,
        parent: &ComputedStyle,
    ) {
        use crate::webkit_box::{own_declaration_inherits, own_declares};
        if !crate::webkit_box::var_rewrite_seen() {
            return;
        }
        let one_of =
            |v: &str, keywords: &[&str]| keywords.iter().any(|k| v.eq_ignore_ascii_case(k));
        if self.hyphens_none != parent.hyphens_none
            && !custom_property(cv, &HYPHENS, |v| {
                one_of(v, &["none", "manual", "auto", "initial"])
            })
            .unwrap_or(false)
            && own_declaration_inherits(cv, guard, crate::webkit_box::HYPHENS)
        {
            self.hyphens_none = parent.hyphens_none;
        }
        if (self.word_break != parent.word_break
            || self.word_break_break_word != parent.word_break_break_word)
            && !custom_property(cv, &WORD_BREAK, |v| {
                one_of(
                    v,
                    &["normal", "break-all", "keep-all", "break-word", "initial"],
                )
            })
            .unwrap_or(false)
            && own_declaration_inherits(cv, guard, crate::webkit_box::WORD_BREAK)
        {
            self.word_break = parent.word_break;
            if parent.word_break_break_word {
                self.overflow_wrap = OverflowWrapValue::Anywhere;
            }
            self.word_break_break_word = parent.word_break_break_word;
        }
        if self.text_indent != parent.text_indent {
            use crate::webkit_box::TEXT_INDENT as NAME;
            // `Some(true)` a `var()` substituted, `Some(false)` one that
            // substituted no value, `None` no `var()`.
            let substituted = custom_property(cv, &TEXT_INDENT, |v| {
                v.strip_prefix(crate::webkit_box::VAR_MARKER).map(|sub| {
                    crate::webkit_box::split_text_indent(sub.trim()).is_some_and(|(l, k)| {
                        k.eq_ignore_ascii_case("initial")
                            || (!crate::webkit_box::is_wide_keyword(&k)
                                && crate::webkit_box::parse_length_percentage(&l, 16.0).is_some())
                    })
                })
            });
            let inherits = match substituted {
                Some(Some(true)) => !own_declares(cv, guard, NAME),
                Some(None) => false,
                Some(Some(false)) | None => own_declaration_inherits(cv, guard, NAME),
            };
            if inherits {
                self.text_indent = parent.text_indent;
            }
        }
    }
}

/// Read one custom property's post-`var()` token string off a cascaded style.
///
/// Unregistered custom properties are declared to inherit
/// (`PropertyRegistrationData::unregistered()`), and each element's map is
/// seeded from its parent's, so a declaration on `:root` is readable here from
/// any descendant — which is what makes a single app-wide
/// `--rinch-scrollbar-color` work.
///
/// The value is always the universal (untyped) form for an unregistered
/// property, but `to_variable_value` covers the registered case too rather than
/// relying on that.
fn custom_property<R>(
    cv: &ComputedValues,
    key: &'static std::thread::LocalKey<style::Atom>,
    f: impl FnOnce(&str) -> R,
) -> Option<R> {
    key.with(|name| {
        cv.custom_properties()
            .get(
                style::properties_and_values::rule::Descriptors::unregistered(),
                name,
            )
            .map(|v| f(v.to_variable_value().css.trim()))
    })
}

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
        // The first background layer (#468); the rest are
        // `background_layers` (#1142).
        let background_first = background_geometry_from_stylo(background, 0);
        // Shared by the `font_size` field below and by
        // `letter_spacing_from_stylo`/`word_spacing_from_stylo` (#743), so the
        // percentage basis can never disagree with the element's own computed
        // font-size.
        let font_size_px = font.font_size.computed_size().px();
        let inherited_table = cv.get_inherited_table();
        // `word-break: break-word` (#2628) is `overflow-wrap: anywhere`
        // whatever the element's own `overflow-wrap` (Chrome 157), with
        // `word-break: normal`: what Stylo was given for it, except through a
        // `var()`, which Stylo cannot parse it from (#2870).
        let break_word = custom_property(cv, &WORD_BREAK, |v| v.eq_ignore_ascii_case("break-word"))
            .unwrap_or(false);

        Self {
            // Display/position
            display: display_from_stylo(&box_style.display),
            table_part: table_part_from_stylo(&box_style.display),
            box_sizing_border_box: cv.get_position().box_sizing
                == style::computed_values::box_sizing::T::BorderBox,
            border_collapse: inherited_table.border_collapse
                == style::computed_values::border_collapse::T::Collapse,
            border_spacing: (
                inherited_table.border_spacing.0.width().px(),
                inherited_table.border_spacing.0.height().px(),
            ),
            table_layout_fixed: cv.get_table().table_layout
                == style::computed_values::table_layout::T::Fixed,
            caption_bottom: inherited_table.caption_side
                == style::computed_values::caption_side::T::Bottom,
            flow_root: flow_root_from_stylo(&box_style.display),
            contains_fixed: contains_fixed_from_stylo(box_style, effects),
            contains_fixed_inline: contains_fixed_inline_from_stylo(box_style, effects),
            inline_level_before_blockify: box_style.original_display.outside()
                == style::values::specified::box_::DisplayOutside::Inline,
            anonymous_inline_table: None,
            // Set after the conversion, from the element's own declarations
            // (`crate::webkit_box::apply`, #1703).
            webkit_line_clamp: None,
            webkit_box_vertical: false,
            line_clamp: None,
            webkit_box: false,
            webkit_box_flex: 0.0,
            webkit_box_item: None,
            webkit_box_pack: super::WebkitBoxPack::Start,
            webkit_box_align: super::WebkitBoxAlign::Stretch,
            webkit_box_reverse: false,
            position: position_from_stylo(&box_style.position),
            float: match box_style.float {
                style::computed_values::float::T::Left => taffy::Float::Left,
                style::computed_values::float::T::Right => taffy::Float::Right,
                _ => taffy::Float::None,
            },
            clear: match box_style.clear {
                style::computed_values::clear::T::Left => taffy::Clear::Left,
                style::computed_values::clear::T::Right => taffy::Clear::Right,
                style::computed_values::clear::T::Both => taffy::Clear::Both,
                _ => taffy::Clear::None,
            },
            vertical_align: vertical_align_from_stylo(
                box_style.alignment_baseline,
                &box_style.baseline_shift,
            ),
            overflow_x: overflow_from_stylo(&box_style.overflow_x),
            overflow_y: overflow_from_stylo(&box_style.overflow_y),
            scrollbar_color: custom_property(cv, &SCROLLBAR_COLOR, ScrollbarColorValue::parse)
                .unwrap_or_default(),
            scrollbar_width: custom_property(cv, &SCROLLBAR_WIDTH, ScrollbarWidthValue::parse)
                .unwrap_or_default(),

            // Dimensions
            width: size_from_stylo("width", &position_style.width),
            height: size_from_stylo("height", &position_style.height),
            min_width: size_from_stylo("min-width", &position_style.min_width),
            min_height: size_from_stylo("min-height", &position_style.min_height),
            max_width: max_size_from_stylo("max-width", &position_style.max_width),
            max_height: max_size_from_stylo("max-height", &position_style.max_height),

            // Flexbox
            flex_direction: flex_direction_from_stylo(&position_style.flex_direction),
            flex_wrap: flex_wrap_from_stylo(&position_style.flex_wrap),
            flex_grow: position_style.flex_grow.0,
            flex_shrink: position_style.flex_shrink.0,
            order: position_style.order,
            flex_basis: flex_basis_from_stylo(&position_style.flex_basis),
            align_items: align_items_from_stylo(&position_style.align_items),
            align_self: align_self_from_stylo(&position_style.align_self),
            justify_content: justify_content_from_stylo(&position_style.justify_content),
            justify_items: justify_items_from_stylo(&position_style.justify_items),
            justify_self: justify_self_from_stylo(&position_style.justify_self),
            direction_rtl: !cv.writing_mode.is_bidi_ltr(),

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
            border_radius_top_left_y: border_radius_y_from_stylo(&border.border_top_left_radius),
            border_radius_top_right_y: border_radius_y_from_stylo(&border.border_top_right_radius),
            border_radius_bottom_right_y: border_radius_y_from_stylo(
                &border.border_bottom_right_radius,
            ),
            border_radius_bottom_left_y: border_radius_y_from_stylo(
                &border.border_bottom_left_radius,
            ),

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
            background_underlay: background_underlay_from_stylo(background, &text.color),
            background_size: background_first.size,
            background_position_x: background_first.position_x,
            background_position_y: background_first.position_y,
            background_repeat_x: background_first.repeat_x,
            background_repeat_y: background_first.repeat_y,
            background_origin: background_first.origin,
            background_blend_mode: background_blend_from_stylo(background, 0),
            background_layers: background_layers_from_stylo(background, &text.color),
            mask_image: mask_image_from_stylo(cv.get_svg(), &text.color),
            clip_path: !matches!(
                cv.get_svg().clip_path,
                style::values::generics::basic_shape::ClipPath::None
            ),
            // Set after the conversion, from the element's own declarations
            // (`crate::webkit_box::apply`, #2719).
            background_clip_text: Vec::new(),
            color: color_from_absolute(&text.color),

            // Visual
            opacity: effects.opacity,
            visibility: visibility_from_stylo(&inherited_box.visibility),

            // Transforms
            transform: transform_from_stylo(
                &box_style.transform,
                box_style.transform_origin.depth.px() as f64,
            ),
            transform_origin_z: box_style.transform_origin.depth.px(),
            backface_visibility_hidden: box_style.backface_visibility
                == style::properties::longhands::backface_visibility::computed_value::T::Hidden,
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
            text_paint: text_paint_from_stylo(cv, color_from_absolute(&text.color), font_size_px),

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
            filter: filter_list_from_stylo(&effects.filter.0, &text.color),
            backdrop_filter: filter_list_from_stylo(&effects.backdrop_filter.0, &text.color),
            filter_brightness: extract_filter_brightness(&effects.filter),
            filter_grayscale: extract_filter_grayscale(&effects.filter),
            filter_saturate: extract_filter_saturate(&effects.filter),
            filter_hue_rotate: extract_filter_hue_rotate(&effects.filter),

            // Object fit
            object_fit: object_fit_from_stylo(&cv.clone_object_fit()),
            image_rendering: image_rendering_from_stylo(&inherited_box.image_rendering),
            aspect_ratio: aspect_ratio_from_stylo(&cv.clone_aspect_ratio()),

            // Cursor
            cursor: cursor_from_stylo(&inherited_ui.cursor.keyword),

            // Pointer events
            pointer_events: pointer_events_from_stylo(&inherited_ui.pointer_events),

            // `user-select` and its `-webkit-`/`-moz-` aliases, from every
            // origin through the cascade (#2231). Not inherited: `auto` is
            // resolved from the parent where selection asks.
            user_select: user_select_from_stylo(cv.get_ui().user_select),

            // Typography
            font_size: font_size_px,
            font_weight: font.font_weight.value(),
            font_family: font_family_from_stylo(&font.font_family),
            font_style: font_style_from_stylo(&font.font_style),
            font_synthesis_weight: !matches!(
                font.font_synthesis_weight,
                style::values::computed::FontSynthesis::None
            ),
            line_height: line_height_from_stylo(&font.line_height),
            letter_spacing: letter_spacing_from_stylo(&text.letter_spacing, font_size_px),
            word_spacing: word_spacing_from_stylo(&text.word_spacing, font_size_px),
            text_align: text_align_from_stylo(&text.text_align),
            html_align: html_align_from_stylo(&text.text_align),
            text_decoration: text_decoration_from_stylo(
                &cv.get_text().clone_text_decoration_line(),
                cv.get_text().clone_text_decoration_style(),
                &cv.get_text().clone_text_decoration_color(),
                &text.color,
            ),
            text_transform: text_transform_from_stylo(&text.text_transform),
            // `text-underline-offset` is not read (#580). With stylo 0.11 it
            // was gecko-only and the servo build dropped the declaration at
            // parse; since stylo 0.21 the servo build parses it (behind
            // `layout.unimplemented`, which `set_stylo_prefs` turns on) and
            // computes it into `get_inherited_text().text_underline_offset`,
            // but this line still writes `None` unconditionally, and it is the
            // only place CSS could reach the field. The other two writers are
            // not CSS paths — `Default`, and `for_anonymous_box`, which copies
            // the parent's value because the property inherits. The witness is
            // `crates/rinch-dom/tests/underline_offset_tests.rs`, whose header
            // records what the consumers do once the field is non-`None`.
            text_underline_offset: None,
            white_space: white_space_from_stylo(&text.white_space_collapse, &text.text_wrap_mode),
            overflow_wrap: if break_word {
                OverflowWrapValue::Anywhere
            } else {
                overflow_wrap_from_stylo(&text.overflow_wrap)
            },
            word_break: if break_word {
                WordBreakValue::Normal
            } else {
                word_break_from_stylo(&text.word_break)
            },
            word_break_break_word: break_word,
            hyphens_none: custom_property(cv, &HYPHENS, |v| v.eq_ignore_ascii_case("none"))
                .unwrap_or(false),
            line_break: line_break_from_stylo(&text.line_break),
            text_indent: text_indent_with(cv, &text.text_indent, font_size_px),
            text_overflow: text_overflow_from_stylo(&cv.clone_text_overflow()),
            tab_size: tab_size_from_stylo(&text.tab_size),

            // Grid - extract from Stylo
            grid_template_columns: grid_template_tracks_from_stylo(
                &position_style.grid_template_columns,
            ),
            grid_template_rows: grid_template_tracks_from_stylo(&position_style.grid_template_rows),
            grid_auto_flow: grid_auto_flow_from_stylo(&position_style.grid_auto_flow),
            grid_auto_columns: grid_auto_tracks_from_stylo(&position_style.grid_auto_columns),
            grid_auto_rows: grid_auto_tracks_from_stylo(&position_style.grid_auto_rows),
            grid_template_areas: grid_template_areas_from_stylo(
                &position_style.grid_template_areas,
            ),
            grid_template_column_names: grid_template_line_names_from_stylo(
                &position_style.grid_template_columns,
            ),
            grid_template_row_names: grid_template_line_names_from_stylo(
                &position_style.grid_template_rows,
            ),
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
