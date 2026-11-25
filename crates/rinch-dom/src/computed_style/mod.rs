//! Typed computed style representation.
//!
//! Provides a pre-parsed ComputedStyle struct to avoid re-parsing CSS properties
//! on every layout and paint operation.
//!
//! This file is the struct, its fields and their documentation. The initial
//! values are in `initial.rs`, an anonymous box's style in `anonymous.rs`,
//! and what paint and layout ask of a style beyond its fields in
//! `queries.rs`; the cascade's conversion from Stylo is `from_stylo/`.

mod anonymous;
mod from_stylo;
mod initial;
mod queries;
pub(crate) use from_stylo::{
    font_family_from_stylo as font_family_css, font_style_from_stylo as font_style_css,
};
pub(crate) mod taffy_conversion;
mod text_layout;
pub mod values;

pub(crate) use from_stylo::color::{
    absolute_from_peniko, color_from_computed, color_from_specified, color_from_stylo,
};
pub use values::*;

use serde::Serialize;

/// A Taffy keyword, which has no `Serialize`, as its name.
fn serialize_debug<T: std::fmt::Debug, S: serde::Serializer>(
    v: &T,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("{v:?}"))
}

/// `-webkit-box-pack` (#2581); see [`ComputedStyle::webkit_box_pack`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebkitBoxPack {
    #[default]
    Start,
    End,
    Center,
    Justify,
}

/// `-webkit-box-align` (#2581); see [`ComputedStyle::webkit_box_align`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebkitBoxAlign {
    #[default]
    Stretch,
    Start,
    End,
    Center,
    Baseline,
}

/// Pre-parsed CSS properties for efficient layout and paint.
#[derive(Debug, Clone, Serialize)]
pub struct ComputedStyle {
    // Display/position
    pub display: DisplayValue,
    /// Which part of a CSS table this box is: `display` maps every
    /// table part to a flex or block container and cannot say.
    pub table_part: TablePart,
    /// `box-sizing: border-box`.
    pub box_sizing_border_box: bool,
    /// `border-collapse: collapse`.
    pub border_collapse: bool,
    /// `border-spacing`, horizontal then vertical, in px.
    pub border_spacing: (f32, f32),
    /// `table-layout: fixed`.
    pub table_layout_fixed: bool,
    /// `caption-side: bottom`.
    pub caption_bottom: bool,
    /// Whether the box was **inline-level before `position` blockified it**
    /// (Stylo's `original_display`): a `<span style="position: absolute">`,
    /// an `inline-block` one. `display` cannot say — an out-of-flow box
    /// computes to a block-level one — and CSS 2.1 §10.3.7 puts such a box's
    /// static position in its line, where the inline box would have been
    /// (issue #634; `out_of_flow::static_position`). Read for nothing else.
    #[serde(skip)]
    pub inline_level_before_blockify: bool,
    /// Block-level `display: flow-root` (#2351). It lays out as
    /// [`DisplayValue::Block`] does, and establishes a block formatting
    /// context of its own: it contains its floats, its children's margins do
    /// not collapse through its edges, and it is placed beside the floats
    /// outside it. `to_taffy_style` hands Taffy that as paint containment,
    /// which Taffy's block layout reads as "starts an independent formatting
    /// context" and which has no other effect on layout. `inline flow-root`
    /// is [`DisplayValue::InlineBlock`] and needs no flag.
    #[serde(skip)]
    pub flow_root: bool,
    /// Whether something other than `transform` makes this element the
    /// containing block of its `position: fixed` descendants (#1372): `contain:
    /// paint` or `layout` (and `strict`/`content`, which include them), a
    /// `filter` or `backdrop-filter` other than `none`, a `perspective` other
    /// than `none`, or a `will-change` naming one of those or `transform`
    /// (CSS Transforms 1 §2, Filter Effects 1 §5, CSS Containment 2 §3,
    /// css-will-change §3; Chrome 151 places such a box against the element).
    /// Read only by [`crate::node::Node::establishes_fixed_containing_block`].
    /// The `filter` half is also [`Self::contains_fixed_inline`].
    #[serde(skip)]
    pub contains_fixed: bool,
    /// The part of [`Self::contains_fixed`] that applies to a non-atomic
    /// inline element too: a `filter` or `backdrop-filter` other than `none`,
    /// or a `will-change` naming one (Chrome 151: a fixed box under such a
    /// `<span>` is placed against the span's padding box; under one with
    /// `contain: paint` or `perspective`, against the viewport).
    #[serde(skip)]
    pub contains_fixed_inline: bool,
    /// `will-change: position`: the element is a containing block for its
    /// absolutely positioned descendants, as a positioned one is, but not for
    /// fixed ones (Chrome 158, Firefox 160; #3065). Read only by
    /// [`crate::node::Node::establishes_abs_containing_block`].
    #[serde(skip)]
    pub will_change_position: bool,
    /// The `-webkit-line-clamp` the element declares (#1703), `None` for
    /// `none`. Not inherited. It clamps only through [`Self::line_clamp`].
    #[serde(skip)]
    pub webkit_line_clamp: Option<u32>,
    /// `-webkit-box-orient: vertical` (or `block-axis`) (#1703). Not
    /// inherited.
    #[serde(skip)]
    pub webkit_box_vertical: bool,
    /// How many lines the box's inline content is clamped to (#1703): a
    /// `display: -webkit-box` with `-webkit-box-orient: vertical` and a
    /// `-webkit-line-clamp`, laid out as a block, as Chrome lays it out. See
    /// `crate::webkit_box`.
    #[serde(skip)]
    pub line_clamp: Option<u32>,
    /// The box is a `display: -webkit-box` or `-webkit-inline-box` (#1703).
    /// One that does not clamp keeps a block's computed `display` and is a
    /// flex container in Taffy (`Self::legacy_box_taffy_style`, #2581); its
    /// text draws no `text-overflow` "…", as Chrome's anonymous flex item
    /// does not.
    #[serde(skip)]
    pub webkit_box: bool,
    /// `-webkit-box-flex` (#2581): how a legacy box's item grows and shrinks
    /// (both, by this factor; `0`, the initial value, neither). Not
    /// inherited. Read only where [`Self::webkit_box_item`] is set.
    #[serde(skip)]
    pub webkit_box_flex: f32,
    /// The box is an item of a legacy `-webkit-box` laid out as a flex
    /// container (one that does not clamp, #2581): `Some(true)` in a column
    /// (`-webkit-box-orient: vertical`), `Some(false)` in a row. Its
    /// `flex-grow` and `flex-shrink` are then [`Self::webkit_box_flex`].
    #[serde(skip)]
    pub webkit_box_item: Option<bool>,
    /// `-webkit-box-pack` (#2581): where a legacy flex `-webkit-box` puts
    /// its items on the main axis. Chrome reads it, and not
    /// `justify-content`, there. Not inherited.
    #[serde(skip)]
    pub webkit_box_pack: WebkitBoxPack,
    /// `-webkit-box-align` (#2581): how a legacy flex `-webkit-box` aligns
    /// its items on the cross axis, in place of `align-items` and every
    /// item's `align-self`. Not inherited.
    #[serde(skip)]
    pub webkit_box_align: WebkitBoxAlign,
    /// `-webkit-box-direction: reverse` (#2581): a legacy flex
    /// `-webkit-box` lays its items out from the main axis's end. Not
    /// inherited.
    #[serde(skip)]
    pub webkit_box_reverse: bool,
    /// A table cell whose parent box is an inline (CSS 2.1 §17.2.1): CSS
    /// wraps it in an anonymous `inline-table` on the line, which rinch,
    /// with no anonymous table box, lays out as the cell itself at `display:
    /// inline-block` (`RinchDocument::wrap_cell_in_inline_table`). This holds
    /// the cell's own `vertical-align`, which says where the table's baseline
    /// is (`ifc::atomic_inline_baseline`); its `vertical_align` is the
    /// table's, `baseline`. `None` for every other box.
    #[serde(skip)]
    pub anonymous_inline_table: Option<VerticalAlignValue>,
    pub position: PositionValue,
    /// `float`. Read by Taffy's block layout only (`floats`).
    #[serde(serialize_with = "serialize_debug")]
    pub float: taffy::Float,
    /// `clear`.
    #[serde(serialize_with = "serialize_debug")]
    pub clear: taffy::Clear,
    /// `vertical-align` (#724): non-inherited, not carried to an anonymous
    /// box (see `ComputedStyle::for_anonymous_box`).
    pub vertical_align: VerticalAlignValue,
    pub overflow_x: OverflowValue,
    pub overflow_y: OverflowValue,
    /// `scroll-behavior: smooth` (#2291): a programmatic scroll of this box
    /// (a `scroll_top` write, a scroll-into-view) animates. Not inherited.
    /// The page scroller takes the root element's, as Chrome's viewport does
    /// ([`crate::NodeTree::scroll_behavior_smooth`]).
    pub scroll_behavior_smooth: bool,
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
    /// `order`: read only by an `inline-flex`'s first baseline (`ifc::taffy_baseline`,
    /// #2580); rinch does not reorder flex items in layout.
    pub order: i32,
    pub flex_basis: DimensionValue,
    pub align_items: Option<AlignValue>,
    pub align_self: Option<AlignValue>,
    pub justify_content: Option<JustifyContentValue>,
    /// `justify-items`; `None` is `normal` (Taffy then stretches an
    /// auto-width grid item and starts a sized one).
    pub justify_items: Option<JustifyValue>,
    /// `justify-self`; `None` is `auto` (the parent's `justify-items`).
    pub justify_self: Option<JustifyValue>,
    /// `direction: rtl` (#1995). Taffy's grid, flex and block layout read
    /// it (columns, a flex row and block-level boxes start at the right),
    /// and it decides which of `start` / `end` `left` / `right` are.
    pub direction_rtl: bool,
    /// `unicode-bidi: plaintext` (#2940): the text a builder lays out for
    /// this element takes each paragraph's direction from its own first strong
    /// character, not `direction` ([`Self::base_direction`]). Not inherited.
    pub unicode_bidi_plaintext: bool,

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

    // Border radius (can be percentage, resolved at paint time). Each corner
    // is two radii (css-backgrounds-3 §5.5, #977): these four are the
    // **horizontal** ones, resolved against the border box's width, and the
    // `_y` four below the **vertical** ones, resolved against its height.
    // `border-radius: 8px` sets both to 8px. `paint::border_radii` is the one
    // place they are resolved and scaled for overlap.
    pub border_radius_top_left: LengthPercentageValue,
    pub border_radius_top_right: LengthPercentageValue,
    pub border_radius_bottom_right: LengthPercentageValue,
    pub border_radius_bottom_left: LengthPercentageValue,
    pub border_radius_top_left_y: LengthPercentageValue,
    pub border_radius_top_right_y: LengthPercentageValue,
    pub border_radius_bottom_right_y: LengthPercentageValue,
    pub border_radius_bottom_left_y: LengthPercentageValue,

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
    /// `background-color` painted **under** an image layer (#468). `None`
    /// when `background` is itself the colour, or the colour is transparent.
    /// An image layer and the colour below it used to be one or the other:
    /// `BackgroundValue` holds a single value, so a striped `Progress` bar
    /// (`background-color` + a stripe gradient) lost its fill.
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub background_underlay: Option<peniko::Color>,
    /// The first background layer's `background-size` (#468).
    pub background_size: BackgroundSizeValue,
    /// The first layer's `background-position-x`, from the positioning
    /// area's left edge; a percentage is a share of the area less the tile.
    pub background_position_x: LengthPercentageValue,
    /// The first layer's `background-position-y`, as `_x` from the top.
    pub background_position_y: LengthPercentageValue,
    pub background_repeat_x: BackgroundRepeatValue,
    pub background_repeat_y: BackgroundRepeatValue,
    pub background_origin: BackgroundOriginValue,
    /// The first layer's `background-blend-mode` (#2221): how it blends with
    /// the layers and the colour under it. The other layers carry theirs on
    /// their [`BackgroundLayer`]. Any entry other than `normal` paints the
    /// background as an isolated group (`paint::background`).
    pub background_blend_mode: BlendModeValue,
    /// The `background-image` layers below the first, top to bottom (#1142).
    /// The first layer is `background` and the six fields above, which is
    /// what a transition or animation of `background-position` moves; these
    /// are painted under it, bottom first, over the colour. Empty for a
    /// single image (no allocation), and a layer with nothing to paint
    /// (`none`) is left out.
    pub background_layers: Vec<BackgroundLayer>,
    /// `mask-image` (#1798, #2099): its gradient and `url()` layers, first
    /// listed first, each with its entries of `mask-size`, `mask-position`,
    /// `mask-repeat` and `mask-origin` (a shorter list repeating, as the
    /// background lists do). They are added together (`mask-composite:
    /// add`) into an alpha mask over the element and its descendants,
    /// clipped to the border box (`mask-clip` is not read). A `url()` layer's
    /// image is loaded as a `background-image`'s is; until it has loaded, or
    /// when it fails, it masks everything away (css-masking-1 §7.1: a
    /// transparent black layer). `mask-mode` is not read: every layer is an
    /// alpha mask (`match-source` is `alpha` for an image and a gradient
    /// alike, so only an explicit `luminance` differs). Empty for `none`, and
    /// for a list with nothing rinch can paint.
    pub mask_image: Vec<BackgroundLayer>,
    /// `clip-path` is not `none`. rinch does not paint a clip path; this is
    /// read only where its presence changes which clips apply to a box: an
    /// absolute box below such an element does not escape the clips above
    /// it (Chrome 158, #2815's review; `paint::LiftedClips::lift_absolute`).
    pub clip_path: bool,
    /// `background-clip: text` (and `-webkit-background-clip: text`, #2719),
    /// per layer as written, first listed first, a shorter list repeating:
    /// a `true` entry paints that layer only inside the glyphs of the
    /// element's text and its in-flow descendants' (`paint::text_clip`).
    /// Empty — the common case, no allocation — where no entry is `text`;
    /// every other `background-clip` value is painted as `border-box`, as
    /// before. The colour is clipped as the bottom layer is. Not inherited.
    /// Stylo's servo build drops a `text` value; it reaches the cascade as
    /// `--rinch-background-clip` (`crate::webkit_box::rewrite`).
    pub background_clip_text: Vec<bool>,
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub color: Option<peniko::Color>,

    // Visual
    pub opacity: f32,
    pub visibility: VisibilityValue,

    // Transforms
    pub transform: TransformValue,
    pub transform_origin_x: LengthPercentageValue,
    pub transform_origin_y: LengthPercentageValue,
    /// `transform-origin`'s z, in CSS px (a length; it takes no percentage).
    /// Already folded into [`TransformValue`]'s matrix (#997).
    pub transform_origin_z: f32,
    /// `backface-visibility: hidden`: a box whose own transform turns its back
    /// to the viewer ([`TransformValue::back_facing`]) is neither drawn nor
    /// hit, with its subtree, `position: fixed` descendants included: the
    /// transformed box is their containing block (#997, #1372).
    pub backface_visibility_hidden: bool,

    // Z-index
    pub z_index: Option<i32>,

    // Text shadow
    pub text_shadow: Vec<TextShadowValue>,
    /// `-webkit-text-fill-color`, `-webkit-text-stroke` and `paint-order`
    /// on text (#2719). Inherited.
    pub text_paint: TextPaintValue,

    // Box shadow
    pub box_shadow: Vec<BoxShadowValue>,

    // Outline
    pub outline_width: f32,
    #[serde(serialize_with = "values::color_serde::serialize")]
    pub outline_color: Option<peniko::Color>,
    pub outline_style: BorderStyleValue,
    pub outline_offset: f32,

    // Filters. The four scalars feed the overlay approximations paint draws
    // for a list with nothing that moves pixels; `filter` is the whole list,
    // in order (#2719), drawn through an offscreen raster when any function
    // in it moves pixels (`FilterOp::moves_pixels`).
    pub filter: Vec<FilterOp>,
    /// `backdrop-filter`, in order (#2719). Empty for `none`.
    pub backdrop_filter: Vec<FilterOp>,
    pub filter_brightness: f32,
    pub filter_grayscale: f32,
    pub filter_saturate: f32,
    pub filter_hue_rotate: f32,

    // Object fit (for <img> elements)
    pub object_fit: ObjectFitValue,
    /// `image-rendering` (#2098): nearest-neighbour sampling for an `<img>`,
    /// a `background-image` and a `mask-image` `url()` when not `auto`.
    pub image_rendering: ImageRenderingValue,

    /// `aspect-ratio` (#1286). A non-replaced box hands it to Taffy
    /// ([`Self::to_taffy_style`]); a replaced element's measure reads it
    /// through [`AspectRatioValue::used`] (`crate::replaced`).
    pub aspect_ratio: AspectRatioValue,

    // Cursor
    pub cursor: CursorValue,

    // Pointer events
    pub pointer_events: PointerEventsValue,

    // User select
    pub user_select: UserSelectValue,

    // Typography
    pub font_size: f32,
    pub font_weight: f32,
    /// The family list as a CSS list string, every name quoted and the
    /// generics bare — stylo's, and the unquoted CSS Fonts 4 ones parley knows
    /// (`ui-monospace`, `emoji`, ...) — so parley's `parse_css_list` reads it
    /// back whole (#1223).
    pub font_family: String,
    pub font_style: FontStyleValue,
    /// `font-synthesis-weight` (#2296): `false` for `none`, which draws a
    /// weight the family has no face for from the face fontique picked, not
    /// emboldened. Inherited. `true` (`auto`, the initial value) lets the
    /// painters draw Parley's faux bold. Stylo's servo build parses only this
    /// longhand: `font-synthesis-style` and the `font-synthesis` shorthand
    /// are Gecko-only there (#2345).
    pub font_synthesis_weight: bool,
    pub line_height: LineHeightValue,
    pub letter_spacing: f32, // in pixels, 0.0 means normal
    pub word_spacing: f32,   // in pixels, 0.0 means normal
    pub text_align: TextAlignValue,
    /// HTML's `align` attribute on a `div`, a cell, a row or a row group
    /// (`-moz-center` / `-webkit-center` and the left and right forms):
    /// besides the text, it aligns the block boxes under it that have a
    /// width of their own, as `<td align="center"><table>` centres the table
    /// (HTML rendering §15.3.3, "align descendants"). Inherited with
    /// `text-align`; a table does not inherit it (Stylo resets it there, as
    /// Chrome does).
    pub html_align: Option<TextAlignValue>,
    pub text_decoration: TextDecorationValue,
    pub text_transform: TextTransformValue,
    /// `text-underline-offset` in CSS pixels, with **CSS's sign** — and
    /// **nothing sets it from CSS today** (#580).
    ///
    /// `None` is `auto`: the font's own underline metric, which is what every
    /// node gets, because `text-underline-offset` is gecko-only in this Stylo
    /// build and the declaration is discarded before `from_stylo` runs. The
    /// field and its two consumers in `ifc.rs` are kept, tested and inert,
    /// waiting for a parse route.
    ///
    /// `Some(px)` is measured from the **alphabetic baseline** — the zero
    /// position css-text-decor-4 §2.3 gives for the default
    /// `text-underline-position: auto` in horizontal writing — and **positive is
    /// away from the text**, i.e. downwards. A length *replaces* the font's
    /// metric rather than adding to it. Measured in Chrome 150, with an
    /// inline-block's bottom edge marking the baseline: `auto` puts the stroke
    /// 1px below the baseline, `0px` exactly on it, `12px` 12px below, `-6px`
    /// 6px above.
    ///
    /// **parley's `underline_offset` is the same quantity with the opposite
    /// sign** — baseline-*up*, since `paint/text.rs` draws the line at
    /// `gy - offset`. So the two `ifc.rs` consumers negate on the way out, and
    /// that sign is the entire conversion: both spellings replace the font
    /// metric, so nothing per-font is involved. The fixtures in
    /// `crates/rinch-dom/tests/underline_offset_tests.rs` pin the direction and
    /// the magnitude together.
    ///
    /// A CSS `<percentage>` resolves against `1em`; that is a parse-route
    /// concern, since nothing here can receive one yet.
    pub text_underline_offset: Option<f32>,
    pub white_space: WhiteSpaceValue,
    pub overflow_wrap: OverflowWrapValue,
    /// CSS `word-break` (#2388).
    pub word_break: WordBreakValue,
    /// `word-break` computes `break-word` (#2628): `word_break` is then
    /// `Normal` and `overflow_wrap` `Anywhere`. Inherited; kept so that a
    /// child whose own `word-break` a `var()` left invalid inherits it
    /// (#2870).
    pub word_break_break_word: bool,
    /// CSS `hyphens: none` (#2621): a soft hyphen is no hyphenation
    /// opportunity, no break and no hyphen. Inherited. `manual` (the initial
    /// value) and `auto` (no dictionary here, as in Chrome without one for
    /// the content language) are `false`. Stylo's servo build parses no
    /// `hyphens`; it reaches the cascade as `--rinch-hyphens`
    /// (`crate::webkit_box::rewrite`).
    pub hyphens_none: bool,
    /// CSS `line-break` (#2388).
    pub line_break: LineBreakValue,
    /// CSS `text-indent` (#2628). Inherited.
    pub text_indent: TextIndentValue,
    pub text_overflow: TextOverflowValue,
    /// `tab-size` (#1584). Inherited.
    pub tab_size: TabSizeValue,

    // Grid properties - skip serialization (taffy types don't impl Serialize)
    #[serde(skip)]
    pub grid_template_columns: Vec<taffy::GridTemplateComponent<String>>,
    #[serde(skip)]
    pub grid_template_rows: Vec<taffy::GridTemplateComponent<String>>,
    #[serde(skip)]
    pub grid_auto_flow: taffy::GridAutoFlow,
    /// `grid-auto-columns` (#2716): the sizes of the implicit columns, and of
    /// explicit columns no track list sizes (those only the area template
    /// creates). Empty for the initial `auto`; several values cycle.
    #[serde(skip)]
    pub grid_auto_columns: Vec<taffy::TrackSizingFunction>,
    /// `grid-auto-rows` (#2716), as `grid_auto_columns`.
    #[serde(skip)]
    pub grid_auto_rows: Vec<taffy::TrackSizingFunction>,
    /// `grid-template-areas` (#2704): the named areas, and the size of the
    /// template, which is part of the explicit grid. `None` for `none`.
    #[serde(skip)]
    pub grid_template_areas: Option<taffy::GridTemplateAreas<String>>,
    /// The `[names]` of `grid-template-columns` (#2704), one set per line
    /// between its top-level components (a `repeat()` is one component, and
    /// carries its own names). Empty when the track list names no line.
    #[serde(skip)]
    pub grid_template_column_names: Vec<Vec<String>>,
    /// The `[names]` of `grid-template-rows`, as `grid_template_column_names`.
    #[serde(skip)]
    pub grid_template_row_names: Vec<Vec<String>>,
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

#[cfg(test)]
mod tests;
