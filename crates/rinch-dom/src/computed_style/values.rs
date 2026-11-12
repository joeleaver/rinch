//! CSS value type enums used by ComputedStyle.

use serde::Serialize;

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
///
/// The **authority on the declared value**, as against
/// [`crate::node::DisplayMode`], which coarsens several of these into one
/// answer for the layout passes. Their **insides** are told apart here and
/// almost nowhere else: `inline-block`, `inline-flex` and `inline-grid` all
/// carry an inline-level *outside* into `DisplayMode`, while which formatting
/// context their children get is carried by [`Self::to_taffy`].
///
/// "Almost", because of one exception, and it is the only place `DisplayMode`
/// answers about an inside: [`crate::node::DisplayMode::is_block_container`],
/// which admits `inline-block` since #592. It can, because the three atomic
/// inlines are three distinct `DisplayMode` variants; the coarsening that keeps
/// this type the authority is over the *block-level* values, where `grid`,
/// `contents` and `none` all arrive as `DisplayMode::Block`.
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
    /// `display: inline-grid` — inline-level outside, grid container inside
    /// (#607).
    ///
    /// A variant only since #607. Before it, `display_from_stylo` folded this
    /// value into [`Self::Grid`], so `(Inline, Grid)` and `(Block, Grid)`
    /// arrived at `style_resolution` as the same value — and an `inline-grid`
    /// box was therefore classified block-level and ended the inline run it
    /// should have joined.
    InlineGrid,
    /// `display: list-item` (#725) — **block-outside, flow-inside** only: a
    /// block box with a marker. Stylo carries "is a list item" as an
    /// orthogonal bit on `Display` (`Display::is_list_item`), so
    /// `display_from_stylo` returns this for `list-item` / `block list-item`
    /// and lets `inline list-item` take its plain `(outside, inside)` value —
    /// it stays inline-level, as in Chrome 153. (Stylo's servo build rejects
    /// `flex list-item`, `grid list-item` and the `flow-root` forms.)
    ///
    /// It lays out as [`Self::Block`] does, and every layout site that asks
    /// for `Block` must ask for this too (`to_taffy` → `taffy::Display::Block`,
    /// `DisplayMode::Block`). The marker
    /// itself does not read this value: `resolve_list_marker` reads Stylo's
    /// `is_list_item()`.
    ListItem,
}

/// Which part of a CSS table a box is, from its computed `display`.
///
/// `display_from_stylo` maps a table to [`DisplayValue::Grid`], a row or row
/// group to [`DisplayValue::Contents`] and a cell or caption to
/// [`DisplayValue::Block`], so `display` alone cannot say. See `table_layout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum TablePart {
    /// Not a table part.
    #[default]
    None,
    /// `table` / `inline-table`.
    Table,
    /// `table-row-group`.
    RowGroup,
    /// `table-header-group`: its rows come first, wherever it stands.
    HeaderGroup,
    /// `table-footer-group`: its rows come last, wherever it stands.
    FooterGroup,
    /// `table-row`.
    Row,
    /// `table-cell`.
    Cell,
    /// A column or a column group.
    Other,
    /// `table-caption`: a table part, but not an internal table box.
    Caption,
}

impl TablePart {
    /// Whether a box of this part, as a child of a table, is a row of its
    /// own or a group of them.
    pub fn is_row_like(self) -> bool {
        matches!(
            self,
            Self::RowGroup | Self::HeaderGroup | Self::FooterGroup | Self::Row
        )
    }

    /// Whether this is an internal table box (css-tables-3: a row, a row
    /// group, a cell, a column or a column group — not the table and not a
    /// caption). `aspect-ratio` does not apply to one (css-sizing-4 §5.1,
    /// #1286; Chrome 155).
    pub fn is_internal(self) -> bool {
        self.is_row_like() || matches!(self, Self::Cell | Self::Other)
    }
}

impl DisplayValue {
    /// Whether this box is a flex or grid container (either outside): its
    /// own text runs are anonymous flex / grid items. Such an item does not
    /// clip, so its text never takes the container's `text-overflow`
    /// ellipsis (#904, measured in Chrome 153).
    pub fn is_flex_or_grid_container(self) -> bool {
        matches!(
            self,
            Self::Flex | Self::InlineFlex | Self::Grid | Self::InlineGrid
        )
    }

    /// Convert to Taffy Display — the **inside** of the display value, i.e.
    /// which formatting context this box's children get.
    ///
    /// Each `inline-*` value answers exactly what its block-level partner does,
    /// because that pair differs only in its outside: `inline-block` → `Block`
    /// (#592), `inline-flex` → `Flex`, `inline-grid` → `Grid` (#607).
    ///
    /// **Not injective**, and several passes turn on that. `inline`, `block` and
    /// `inline-block` all give `Block`; `flex`, `inline-flex` and `contents` all
    /// give `Flex`; `grid` and `inline-grid` both give `Grid`. So a Taffy-style
    /// comparison cannot detect a display change — see the `ifc_dirty` trigger
    /// in `style_resolution`, which asks [`crate::node::DisplayMode`] instead.
    pub fn to_taffy(&self) -> taffy::Display {
        match self {
            Self::Flex | Self::InlineFlex => taffy::Display::Flex,
            Self::Block | Self::ListItem => taffy::Display::Block,
            Self::Grid | Self::InlineGrid => taffy::Display::Grid,
            Self::None => taffy::Display::None,
            Self::Contents => taffy::Display::Flex, // transparent container
            Self::Inline => taffy::Display::Block,  // inline handled by IFC
            // #592: a block container on the inside, like its `block` partner.
            Self::InlineBlock => taffy::Display::Block,
        }
    }
}

/// CSS position property values.
///
/// The default is `Static`, which is what CSS says the initial value of
/// `position` is — and, less obviously, the only value that keeps a node which
/// never reaches Stylo out of the stacking machinery.
///
/// This used to default to `Relative`, which looked harmless because
/// [`PositionValue::to_taffy`] maps `Static` and `Relative` to the same Taffy
/// position, so layout could not tell the two apart. Paint could. Style
/// resolution runs on elements only — `resolve_styles_recursive` returns early
/// for anything that is not an element — so every *text* node in the document
/// keeps `ComputedStyle::default()` for its whole life, and with `Relative` as
/// the default that made `is_positioned_z_auto` in `crate::stacking` answer
/// `true` for all of them. A positioned `z-index: auto` box is hoisted out of
/// its parent and painted from the nearest stacking-context ancestor's
/// sequence, so every text node was, and the guard that stops an IFC root's
/// children being drawn a second time (`already_drawn_inline` in
/// `crate::paint`) only recognises a child it is itself the `ifc_root` of. A
/// hoisted text node arrives at an *ancestor*, where that test cannot match, so
/// every run of text in an inline formatting context was painted twice: once by
/// its IFC root, out of the Parley layout that carries `text-transform`,
/// `letter-spacing` and the inline styling; and once by the standalone text
/// path in `paint_node`, which has none of that and draws the raw DOM string at
/// the IFC root's own box origin.
///
/// Both copies land on the same pixels when the run has no `text-transform`, no
/// `letter-spacing` and no padding on the root to displace the content box —
/// which is most text, which is why this survived so long looking like nothing
/// worse than slightly heavy antialiasing. Where it does not, you see it: a
/// group header styled `text-transform: uppercase; letter-spacing: 0.16em` drew
/// "SOLID" with "Solid" struck through it, at two widths, and a chip with
/// `padding: 6px 12px` drew its label twice, a line and a padding apart. Found
/// on an Android device (card K20) and reproducible on the desktop the whole
/// time; nothing about it was platform-specific.
///
/// `Static` costs nothing at layout time — `to_taffy` still returns
/// `taffy::Position::Relative` for it — and `is_positioned_z_auto` is the only
/// place in the tree that asks whether a position is non-static, so this is a
/// one-predicate change with the whole of the double paint behind it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum PositionValue {
    Relative,
    Absolute,
    Fixed,
    #[default]
    Static,
    Sticky,
}

impl PositionValue {
    /// Convert to Taffy Position.
    pub fn to_taffy(&self) -> taffy::Position {
        match self {
            Self::Absolute | Self::Fixed => taffy::Position::Absolute,
            Self::Relative | Self::Static | Self::Sticky => taffy::Position::Relative,
        }
    }
}

/// A CSS **intrinsic sizing keyword** (css-sizing-3 §5, css-sizing-4 §4).
///
/// They lay out on `width`, `height` and `flex-basis` (#691) and not on
/// `min-*`/`max-*` (#1275) — see [`DimensionValue::Intrinsic`]. The keyword is
/// carried this far rather than folded into `Auto` at style conversion so that
/// a consumer can tell a declared `max-content` from an undeclared size
/// (#626): `Auto` means the author wrote `auto` (or nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum IntrinsicSize {
    /// `max-content` — the box's preferred size, laid out with no wrapping.
    MaxContent,
    /// `min-content` — the box's smallest size that avoids overflow.
    MinContent,
    /// `fit-content`, i.e. `min(max-content, max(min-content, stretch))`.
    FitContent,
    /// `stretch` — fill the containing block's remaining space, margin box
    /// included. `-webkit-fill-available` lands here too: Chrome 150 computes
    /// the prefixed spelling to `stretch` (measured — `getComputedStyle` on
    /// `min-width: -webkit-fill-available` answers `stretch`).
    Stretch,
}

impl IntrinsicSize {
    /// The Taffy keyword for `width`/`height`/`flex-basis` (#691).
    pub fn to_taffy(self) -> taffy::Dimension {
        match self {
            Self::MaxContent => taffy::Dimension::max_content(),
            Self::MinContent => taffy::Dimension::min_content(),
            Self::FitContent => taffy::Dimension::fit_content(),
            Self::Stretch => taffy::Dimension::stretch(),
        }
    }

    /// The keyword as an author would write it, for diagnostics.
    pub fn css_name(self) -> &'static str {
        match self {
            Self::MaxContent => "max-content",
            Self::MinContent => "min-content",
            Self::FitContent => "fit-content",
            Self::Stretch => "stretch",
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
    /// A mixed `calc()` combining a length and a percentage (#278/#404):
    /// resolves to `px + pct * basis` (`pct` is a fraction, 0.5 = 50%).
    /// A non-affine calc (`min()`/`max()`/`clamp()` inside) is stored as its
    /// large-basis linearization — see `from_stylo/calc.rs`.
    Calc {
        px: f32,
        pct: f32,
    },
    /// An intrinsic sizing keyword.
    ///
    /// **On `width`, `height` and `flex-basis` it lays out** (#691):
    /// [`DimensionValue::to_taffy`] hands Taffy 0.14 the keyword itself, which
    /// lays it out on any box it computes as a child. Two kinds of box are not
    /// that, and rinch resolves the keyword itself for them:
    ///
    /// - an **atomic inline** is computed as a Taffy *root*, and a root
    ///   ignores its own size keyword — `ifc.rs::resolve_root_width_keyword`
    ///   measures the width keywords there, and
    ///   `ifc.rs::pin_containing_block_sizes` resolves `height: stretch`
    ///   against a definite containing-block height (#1277);
    /// - an out-of-flow box whose containing block is the **viewport** would
    ///   take `stretch` from its Taffy parent — `out_of_flow.rs` bakes it from
    ///   the viewport.
    ///
    /// **On `min-*`/`max-*` it still lays out as `auto`** (#1275): Taffy 0.14
    /// stores those as `LengthPercentageAuto`, which has no keyword, so
    /// [`DimensionValue::to_taffy_lpa`] answers `auto`. The declaration is at
    /// least inspectable — the MCP's `GetComputedStyles` serializes this enum
    /// (`{"Intrinsic": "MaxContent"}`) — and `from_stylo` prints one line per
    /// such property and keyword per process.
    ///
    /// All four keywords reach this variant, `-webkit-fill-available` folding
    /// into `Stretch`. `fit-content(<length-percentage>)` does not — it is
    /// `clamp(min-content, <lp>, max-content)`, a different value from the bare
    /// keyword, so it stays `Auto` and is reported separately.
    ///
    /// None of them is gated: Stylo guards `stretch`, `-webkit-fill-available`
    /// and `fit-content()` on `layout.css.stretch-size-keyword.enabled`,
    /// `layout.css.webkit-fill-available.enabled` and
    /// `layout.css.fit-content-function.enabled`, which `stylo_static_prefs`
    /// defaults to `true` and `set_stylo_prefs` leaves alone.
    Intrinsic(IntrinsicSize),
}

impl DimensionValue {
    /// Convert to Taffy Dimension.
    ///
    /// A `Calc` cannot be represented in a Taffy value — Taffy's calc pointer
    /// (`CompactLength::calc`) only works for callers implementing the
    /// layout-tree traits themselves; `TaffyTree`'s `resolve_calc_value` is
    /// hardcoded to `0.0` (taffy-0.14.0, `src/tree/taffy_tree.rs:387`). So the
    /// length part goes in as a *seed* and `resolve_layout_calcs`
    /// (`calc_layout.rs`) overwrites it with the resolved length before a
    /// layout result is read — on the converged path; a run that hits the
    /// fixpoint's iteration cap reads the last iterate (see `calc_layout.rs`).
    /// An [`Intrinsic`](Self::Intrinsic) keyword goes in as Taffy's own
    /// keyword (#691; [`IntrinsicSize::to_taffy`]).
    pub fn to_taffy(&self) -> taffy::Dimension {
        match self {
            Self::Auto => taffy::Dimension::auto(),
            Self::Length(v) => taffy::Dimension::length(*v),
            Self::Percent(v) => taffy::Dimension::percent(*v),
            Self::Calc { px, .. } => taffy::Dimension::length(px.max(0.0)),
            Self::Intrinsic(k) => k.to_taffy(),
        }
    }

    /// Convert to the `LengthPercentageAuto` Taffy 0.14 stores `min_size` and
    /// `max_size` as. Same mapping as [`Self::to_taffy`]; an
    /// [`Intrinsic`](Self::Intrinsic) keyword goes in as `auto`, since Taffy's
    /// min/max sizes have no keyword representation at all (#1275).
    pub fn to_taffy_lpa(&self) -> taffy::LengthPercentageAuto {
        match self {
            Self::Auto => taffy::LengthPercentageAuto::auto(),
            Self::Length(v) => taffy::LengthPercentageAuto::length(*v),
            Self::Percent(v) => taffy::LengthPercentageAuto::percent(*v),
            Self::Calc { px, .. } => taffy::LengthPercentageAuto::length(px.max(0.0)),
            Self::Intrinsic(_) => taffy::LengthPercentageAuto::auto(),
        }
    }

    /// Whether the author wrote `auto` (or nothing).
    ///
    /// This is the **specified** value: it answers `false` for an intrinsic
    /// keyword, which lays out as itself (#691) — ask
    /// [`Self::is_auto_or_keyword`] for "no declared length".
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Whether the author declared no size here: `auto`, or a sizing keyword.
    ///
    /// Either way the used size is derived — from the content or from the
    /// containing block — rather than read off the declaration, which is the
    /// question the IFC wrap tolerance asks.
    /// It is **not** "lays out like `auto`": since #691 the keywords lay out as
    /// themselves on `width`/`height`/`flex-basis`.
    pub fn is_auto_or_keyword(&self) -> bool {
        matches!(self, Self::Auto | Self::Intrinsic(_))
    }

    /// Whether a box with this size fills the space between two paired insets
    /// of a positioned box: `auto` and `stretch` do, a content keyword
    /// (`max-content`, `min-content`, `fit-content`) keeps its measured size.
    pub fn fills_between_insets(&self) -> bool {
        matches!(self, Self::Auto | Self::Intrinsic(IntrinsicSize::Stretch))
    }

    /// The intrinsic sizing keyword the author wrote, if any.
    pub fn intrinsic(&self) -> Option<IntrinsicSize> {
        match self {
            Self::Intrinsic(k) => Some(*k),
            _ => None,
        }
    }

    /// The resolved length a `Calc` takes at `basis`, floored at zero the way
    /// stylo's own `resolve()` floors a non-negative property. `None` for
    /// every other variant — Taffy resolves those itself.
    pub fn resolve_calc(&self, basis: f32) -> Option<f32> {
        match self {
            Self::Calc { px, pct } => Some((px + pct * basis).max(0.0)),
            _ => None,
        }
    }
}

/// CSS length-percentage value (padding, border-width, gap).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum LengthPercentageValue {
    #[default]
    Zero,
    Length(f32),
    Percent(f32),
    /// A mixed `calc()` combining a length and a percentage (#278/#404):
    /// resolves to `px + pct * basis` (`pct` is a fraction, 0.5 = 50%).
    /// The pair is the *unclamped* affine; a consumer of a non-negative
    /// property (padding, gap, border-radius) floors the resolution at zero
    /// itself, because this enum also carries `transform-origin`, which may
    /// legally resolve negative. A non-affine calc is stored as its
    /// large-basis linearization — see `from_stylo/calc.rs`.
    Calc {
        px: f32,
        pct: f32,
    },
}

impl LengthPercentageValue {
    /// Convert to Taffy LengthPercentage.
    ///
    /// A `Calc` cannot be represented in a Taffy value (see
    /// [`DimensionValue::to_taffy`]); the clamped length part is a seed that
    /// `resolve_layout_calcs` (`calc_layout.rs`) overwrites with the resolved
    /// length before a layout result is read (converged path; see there).
    /// Every Taffy consumer of this enum (padding, gap) is non-negative,
    /// hence the floor.
    pub fn to_taffy(&self) -> taffy::LengthPercentage {
        match self {
            Self::Zero => taffy::LengthPercentage::length(0.0),
            Self::Length(v) => taffy::LengthPercentage::length(*v),
            Self::Percent(v) => taffy::LengthPercentage::percent(*v),
            Self::Calc { px, .. } => taffy::LengthPercentage::length(px.max(0.0)),
        }
    }

    /// Get as f32 length (resolving percentage against container size).
    ///
    /// A `Calc` resolves unclamped — `transform-origin` may be negative; a
    /// non-negative consumer (border-radius) floors the result itself.
    pub fn resolve(&self, container_size: f32) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Length(v) => *v,
            Self::Percent(v) => *v * container_size,
            Self::Calc { px, pct } => *px + *pct * container_size,
        }
    }

    /// Get as f32 length (percentages return 0.0).
    pub fn to_px(&self) -> f32 {
        match self {
            Self::Zero => 0.0,
            Self::Length(v) => *v,
            Self::Percent(_) => 0.0,      // Percentages need container size
            Self::Calc { px, .. } => *px, // the percentage part needs container size
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
    /// A mixed `calc()` combining a length and a percentage (#278/#404):
    /// resolves to `px + pct * basis` (`pct` is a fraction, 0.5 = 50%).
    /// Never clamped — margins and insets are legally negative. A non-affine
    /// calc is stored as its large-basis linearization — see
    /// `from_stylo/calc.rs`.
    Calc {
        px: f32,
        pct: f32,
    },
}

impl LengthPercentageAutoValue {
    /// Convert to Taffy LengthPercentageAuto.
    ///
    /// A `Calc` cannot be represented in a Taffy value (see
    /// [`DimensionValue::to_taffy`]); the length part is a seed that
    /// `resolve_layout_calcs` (`calc_layout.rs`) overwrites with the resolved
    /// length before a layout result is read (converged path; see
    /// `calc_layout.rs`). No floor — margins and insets are legally negative.
    pub fn to_taffy(&self) -> taffy::LengthPercentageAuto {
        match self {
            Self::Auto => taffy::LengthPercentageAuto::auto(),
            Self::Length(v) => taffy::LengthPercentageAuto::length(*v),
            Self::Percent(v) => taffy::LengthPercentageAuto::percent(*v),
            Self::Calc { px, .. } => taffy::LengthPercentageAuto::length(*px),
        }
    }

    /// Get as f32 length (returns 0.0 for Auto or Percent).
    pub fn to_px(&self) -> f32 {
        match self {
            Self::Length(v) => *v,
            Self::Auto | Self::Percent(_) => 0.0,
            Self::Calc { px, .. } => *px, // the percentage part needs a reference size
        }
    }

    /// Resolve to a concrete pixel value given a reference size.
    /// Returns None for Auto.
    pub fn resolve(&self, reference: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Length(v) => Some(*v),
            Self::Percent(v) => Some(*v * reference),
            Self::Calc { px, pct } => Some(*px + *pct * reference),
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
    /// Convert to Taffy FlexWrap.
    pub fn to_taffy(&self) -> taffy::FlexWrap {
        match self {
            Self::NoWrap => taffy::FlexWrap::NoWrap,
            Self::Wrap => taffy::FlexWrap::Wrap,
            Self::WrapReverse => taffy::FlexWrap::WrapReverse,
        }
    }
}

/// An `align-items` / `align-self` position: the keyword and whether it is
/// `safe` (an item that overflows its area is then put at the start, #1987).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AlignValue {
    pub keyword: AlignKeyword,
    pub safe: bool,
}

/// The keyword of an `align-items` / `align-self` value. `start` / `end`
/// and `flex-start` / `flex-end` are kept apart: they differ in a
/// `wrap-reverse` flex container's cross axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AlignKeyword {
    Start,
    End,
    FlexStart,
    FlexEnd,
    SelfStart,
    SelfEnd,
    Center,
    Baseline,
    Stretch,
}

impl AlignValue {
    /// Convert to Taffy's `AlignItems` / `AlignSelf` (one type).
    pub fn to_taffy(&self) -> taffy::AlignItems {
        use taffy::{AlignItemsKeyword as K, AlignmentSafety as S};
        let keyword = match self.keyword {
            AlignKeyword::Start => K::Start,
            AlignKeyword::End => K::End,
            AlignKeyword::FlexStart => K::FlexStart,
            AlignKeyword::FlexEnd => K::FlexEnd,
            AlignKeyword::SelfStart => K::SelfStart,
            AlignKeyword::SelfEnd => K::SelfEnd,
            AlignKeyword::Center => K::Center,
            AlignKeyword::Baseline => K::Baseline,
            AlignKeyword::Stretch => K::Stretch,
        };
        // `safe stretch` / `safe baseline` are not valid CSS.
        let safe =
            self.safe && !matches!(self.keyword, AlignKeyword::Stretch | AlignKeyword::Baseline);
        taffy::AlignItems {
            keyword,
            safety: if safe { S::Safe } else { S::Unsafe },
        }
    }
}

/// Where `justify-items` / `justify-self` puts a grid item, a block-level
/// box in a block container (#1985), or an absolute box's static position,
/// on the inline axis (#1511). `left` and `right` are physical: which of
/// `start` and `end` they are depends on the alignment container's
/// `direction` (#1995), so they are resolved when the Taffy style is built
/// ([`JustifyValue::to_taffy`]). `baseline` has no inline-axis meaning for a
/// horizontal item: in a grid it falls back to `safe start` (`last baseline`
/// to `safe end`), as css-align-3 §9.3 says; a block container (Chrome 157)
/// starts both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum JustifyKeyword {
    Start,
    End,
    FlexStart,
    FlexEnd,
    SelfStart,
    SelfEnd,
    Left,
    Right,
    Center,
    Stretch,
    Baseline,
    LastBaseline,
}

/// A `justify-items` or `justify-self` value: a [`JustifyKeyword`] and the
/// `safe` overflow modifier (an item that overflows its area is then placed
/// at the start, not centred or end-aligned past the area's start edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct JustifyValue {
    pub keyword: JustifyKeyword,
    pub safe: bool,
}

impl JustifyValue {
    /// Convert to Taffy's `JustifyItems` / `JustifySelf` (one type).
    /// `container_rtl` is the alignment container's `direction` (`left` is
    /// its `end` when it is right to left); `in_block` says the alignment
    /// container is a block container, where a baseline value starts the
    /// box.
    pub fn to_taffy(&self, container_rtl: bool, in_block: bool) -> taffy::JustifyItems {
        use JustifyKeyword as K;
        use taffy::JustifyItems as J;
        let (start, end) = if container_rtl {
            (K::End, K::Start)
        } else {
            (K::Start, K::End)
        };
        let (keyword, safe) = match self.keyword {
            K::Left => (start, self.safe),
            K::Right => (end, self.safe),
            K::Baseline | K::LastBaseline if in_block => (K::Start, false),
            K::Baseline => (K::Start, true),
            K::LastBaseline => (K::End, true),
            k => (k, self.safe),
        };
        let unsafe_ = match keyword {
            K::End => J::END,
            K::FlexStart => J::FLEX_START,
            K::FlexEnd => J::FLEX_END,
            K::SelfStart => J::SELF_START,
            K::SelfEnd => J::SELF_END,
            K::Center => J::CENTER,
            // `safe stretch` is not valid CSS; stretch never overflows.
            K::Stretch => return J::STRETCH,
            _ => J::START,
        };
        if safe {
            taffy::JustifyItems {
                safety: taffy::AlignmentSafety::Safe,
                ..unsafe_
            }
        } else {
            unsafe_
        }
    }
}

/// CSS `justify-content` values. `start` / `end` and `flex-start` /
/// `flex-end` are kept apart (they differ in a reversed flex container),
/// and `left` / `right` are physical, resolved against the container's
/// `direction` and main axis when the Taffy style is built (#1995).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum JustifyContentValue {
    Start,
    End,
    FlexStart,
    FlexEnd,
    Left,
    Right,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl JustifyContentValue {
    /// Convert to Taffy `JustifyContent`. `rtl` is the container's
    /// `direction`; `horizontal` says its justify axis is the inline axis (a
    /// grid, a row flex container). `left` / `right` on a vertical axis are
    /// `start` (css-align-3 §4.2).
    pub fn to_taffy(&self, rtl: bool, horizontal: bool) -> taffy::JustifyContent {
        use taffy::JustifyContent as J;
        let physical = |left: bool| {
            if horizontal && (left == rtl) {
                J::END
            } else {
                J::START
            }
        };
        match self {
            Self::Start => J::START,
            Self::End => J::END,
            Self::FlexStart => J::FLEX_START,
            Self::FlexEnd => J::FLEX_END,
            Self::Left => physical(true),
            Self::Right => physical(false),
            Self::Center => J::CENTER,
            Self::Stretch => J::STRETCH,
            Self::SpaceBetween => J::SPACE_BETWEEN,
            Self::SpaceAround => J::SPACE_AROUND,
            Self::SpaceEvenly => J::SPACE_EVENLY,
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

/// How wide a scroll container's overlay scrollbar is drawn — the CSS
/// `scrollbar-width` keywords, read from the `--rinch-scrollbar-width` custom
/// property (see [`ScrollbarColorValue`] for why it is not the real property).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum ScrollbarWidthValue {
    /// The default 6px thumb.
    #[default]
    Auto,
    /// A narrower 4px thumb, for dense chrome.
    Thin,
    /// No bar at all: nothing is painted, and nothing is hit-tested either, so
    /// an app that draws its own scrollbar can turn rinch's off rather than
    /// covering it up.
    None,
}

impl ScrollbarWidthValue {
    /// Parse from a CSS keyword. Anything unrecognised is `auto`, matching how
    /// a browser treats an invalid keyword on this property.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "thin" => Self::Thin,
            "none" => Self::None,
            _ => Self::Auto,
        }
    }
}

/// What colour a scroll container's overlay scrollbar is drawn in — the CSS
/// `scrollbar-color: <thumb> <track>` shape.
///
/// # Why this is not `scrollbar-color`
///
/// With stylo 0.11 the real property was **gecko-only** (`engines="gecko"`,
/// a codegen-time filter): the servo build emitted no `LonghandId`, parser
/// entry or style-struct field for it, so `scrollbar-color: red blue` was an
/// unknown declaration and was dropped. Since stylo 0.21 the servo build
/// parses it (behind `layout.unimplemented`, which rinch turns on), but
/// nothing in rinch reads it.
///
/// So the value arrives through a **custom property**, `--rinch-scrollbar-color`,
/// which the servo build does support fully: it cascades, it inherits, and it
/// composes with `var()`. One declaration on `:root` therefore restyles every
/// scroll region in an app, which is the property the real one would have had.
/// If Stylo ever ships `scrollbar-color` for servo, this is where it plugs in.
///
/// `thumb: None` means `auto` — the built-in default, which is not a fixed
/// colour: see `paint::scrollbar::thumb_color`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct ScrollbarColorValue {
    /// The thumb's colour, or `None` for `auto`.
    #[serde(serialize_with = "color_serde::serialize")]
    pub thumb: Option<peniko::Color>,
    /// The track's colour. `None` means no track is painted — rinch's bar is
    /// an overlay with no track by default, so this stays absent unless asked
    /// for.
    #[serde(serialize_with = "color_serde::serialize")]
    pub track: Option<peniko::Color>,
}

impl ScrollbarColorValue {
    /// Parse `auto` or `<color> [<color>]`.
    ///
    /// Splits on whitespace **outside parentheses**, so `rgb(255 0 0)` stays
    /// one token; a naive `split_whitespace` would tear the modern space-
    /// separated colour syntaxes apart. An unparseable first colour leaves the
    /// whole declaration as `auto` rather than half-applying it.
    pub fn parse(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("auto") {
            return Self::default();
        }
        let mut parts: Vec<&str> = Vec::new();
        let (mut depth, mut start) = (0i32, 0usize);
        let bytes = value.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ if b.is_ascii_whitespace() && depth == 0 => {
                    if i > start {
                        parts.push(&value[start..i]);
                    }
                    start = i + 1;
                }
                _ => {}
            }
        }
        if start < value.len() {
            parts.push(&value[start..]);
        }
        let thumb = parts.first().and_then(|p| crate::layout::parse_color(p));
        if thumb.is_none() {
            return Self::default();
        }
        Self {
            thumb,
            track: parts.get(1).and_then(|p| crate::layout::parse_color(p)),
        }
    }
}

/// CSS text-overflow property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum TextOverflowValue {
    #[default]
    Clip,
    Ellipsis,
}

/// `tab-size` (css-text-3 §4.2, #1584). Inherited; the initial value is
/// `8`. Stylo 0.11 parsed it for Gecko only; since stylo 0.21 the servo build
/// parses and cascades it too.
///
/// What Chrome 155 does with it (`tests/tab_stop_1122_tests.rs`): each tab
/// takes the `tab-size` of the element its text is in, while a number counts
/// spaces of the **block container's** font, its `letter-spacing` and
/// `word-spacing` included; a length is the interval itself, spacing not
/// added. `0` lays a tab out zero wide.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum TabSizeValue {
    /// A `<number>`: that many spaces.
    Spaces(f32),
    /// A `<length>`, in CSS px.
    Length(f32),
}

impl Default for TabSizeValue {
    fn default() -> Self {
        Self::Spaces(8.0)
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
///
/// `left` and `right` are their own values, not `start` and `end`: they are
/// the same only in a left-to-right paragraph, and rinch hands parley a
/// `direction: rtl` block's direction (parley 0.12's base direction).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum TextAlignValue {
    #[default]
    Start,
    Center,
    End,
    Left,
    Right,
    Justify,
}

impl TextAlignValue {
    /// Convert to Parley Alignment.
    pub fn to_parley(&self) -> parley::layout::Alignment {
        match self {
            Self::Start => parley::layout::Alignment::Start,
            Self::Center => parley::layout::Alignment::Center,
            Self::End => parley::layout::Alignment::End,
            Self::Left => parley::layout::Alignment::Left,
            Self::Right => parley::layout::Alignment::Right,
            Self::Justify => parley::layout::Alignment::Justify,
        }
    }
}

/// CSS `vertical-align` property (#724). Non-inherited; the initial value is
/// `baseline`.
///
/// Only [`Baseline`](Self::Baseline), [`Sub`](Self::Sub), [`Super`](Self::Super)
/// and [`LengthPercentage`](Self::LengthPercentage) have a layout consumer
/// today — see `ifc::vertical_align_shift_px` and its module doc.
/// `Top`/`TextTop`/`Middle`/`Bottom`/`TextBottom` parse and round-trip through
/// `get_computed_styles` so an author's declaration is reported faithfully,
/// but lay out on a text span identically to `Baseline` (no shift) — #1357.
/// An atomic inline honours all of them (`ifc::atomic_vertical_align`), and
/// a table cell's is its own alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum VerticalAlignValue {
    #[default]
    Baseline,
    Sub,
    Super,
    Top,
    TextTop,
    Middle,
    Bottom,
    TextBottom,
    /// `<length>` or `<percentage>`. A percentage resolves against the
    /// element's own `line-height` (CSS 2.1 §10.8.1, measured in Chrome 153);
    /// a length is absolute. Positive raises the box, matching
    /// [`LengthPercentageValue::resolve`]'s sign.
    LengthPercentage(LengthPercentageValue),
}

/// CSS `text-decoration-style` — how the decoration line is drawn.
///
/// Only [`Solid`](Self::Solid) and [`Wavy`](Self::Wavy) are distinguished when
/// painting: a wavy underline is the spellcheck squiggle and is drawn by
/// `paint::text` as a zigzag path, everything else falls back to the straight
/// line Parley draws from the run metrics. `double`/`dotted`/`dashed` are
/// **parsed** (so the cascade and `get_computed_styles` report them faithfully)
/// but render solid until someone needs them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum TextDecorationStyleValue {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

/// CSS text-decoration property values: the lines (`text-decoration-line`), how
/// they are drawn (`text-decoration-style`) and in what colour
/// (`text-decoration-color`). The `text-decoration` shorthand expands to all
/// three in the cascade, so nothing here parses it separately.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TextDecorationValue {
    pub underline: bool,
    pub strikethrough: bool,
    /// How the line is drawn. `wavy` is the one that changes the painter's path.
    pub style: TextDecorationStyleValue,
    /// The line's colour. `None` is CSS's `currentcolor` — the text's own colour,
    /// which is what Parley uses when no decoration brush is set, so the painter
    /// needs no fallback of its own.
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
}

impl TextDecorationValue {
    /// True when this decoration asks for a **wavy underline** — the one case the
    /// native painter draws itself instead of letting Parley stroke a straight
    /// line from the run metrics.
    pub fn is_wavy_underline(&self) -> bool {
        self.underline && self.style == TextDecorationStyleValue::Wavy
    }
}

/// CSS text-transform property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum TextTransformValue {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl TextTransformValue {
    /// Apply the text transform to a string.
    pub fn apply(&self, text: &str) -> Option<String> {
        match self {
            Self::None => None,
            Self::Uppercase => Some(text.to_uppercase()),
            Self::Lowercase => Some(text.to_lowercase()),
            Self::Capitalize => {
                let mut result = String::with_capacity(text.len());
                let mut capitalize_next = true;
                for ch in text.chars() {
                    if capitalize_next && ch.is_alphabetic() {
                        for upper in ch.to_uppercase() {
                            result.push(upper);
                        }
                        capitalize_next = false;
                    } else {
                        result.push(ch);
                        if ch.is_whitespace() {
                            capitalize_next = true;
                        }
                    }
                }
                Some(result)
            }
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
    /// `break-spaces`: preserved like `pre-wrap`, but its spaces and tabs
    /// never hang — one that does not fit starts the next line — and there is
    /// a break opportunity after each of them (#1043).
    BreakSpaces,
}

/// CSS overflow-wrap (word-wrap) property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum OverflowWrapValue {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

/// CSS `word-break` (#2388). Stylo's servo build has no `break-word`
/// (Gecko-only compat value).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub enum WordBreakValue {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
}

impl WordBreakValue {
    /// Convert to parley's `WordBreak`.
    pub fn to_parley(self) -> parley::style::WordBreak {
        match self {
            Self::Normal => parley::style::WordBreak::Normal,
            Self::BreakAll => parley::style::WordBreak::BreakAll,
            Self::KeepAll => parley::style::WordBreak::KeepAll,
        }
    }
}

/// CSS `line-break` (#2388).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub enum LineBreakValue {
    #[default]
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}

impl LineBreakValue {
    /// Convert to parley's `LineBreak`. `auto` is parley's default,
    /// `strict`: the strictness rinch laid every text out with before it
    /// read the property. For ASCII, Chrome's pair table decides under every
    /// value but `anywhere` (`ifc::CHROME_LINE_BREAKS`).
    pub fn to_parley(self) -> parley::style::LineBreak {
        match self {
            Self::Auto | Self::Strict => parley::style::LineBreak::Strict,
            Self::Loose => parley::style::LineBreak::Loose,
            Self::Normal => parley::style::LineBreak::Normal,
            Self::Anywhere => parley::style::LineBreak::Anywhere,
        }
    }
}

impl OverflowWrapValue {
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
    /// Top/left darkened, bottom/right lightened — a "pressed in" bevel.
    Inset,
    /// The reverse of `Inset` — top/left lightened, bottom/right darkened —
    /// a "pushed out" bevel.
    Outset,
    /// Each edge split into two halves along its thickness: the half nearer
    /// the box's outer edge takes `Inset`'s shading, the half nearer the
    /// content takes `Outset`'s — a bevel that looks carved into the page.
    Groove,
    /// The reverse split of `Groove` — a bevel that looks raised off the
    /// page.
    Ridge,
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
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextShadowValue {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
}

/// A single box-shadow value.
#[derive(Debug, Clone, Serialize)]
pub struct BoxShadowValue {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
    pub inset: bool,
}

/// Pre-computed 2D affine transform.
///
/// `matrix` carries the whole transform list *except* the percentage part of
/// any `translate`, which cannot be resolved until the element's border box is
/// known. That part is not a pair of scalars bolted onto `matrix[4]`/`[5]` at
/// the end: CSS composes transform functions in list order, so a percentage
/// translate takes effect in the frame the functions *before* it establish —
/// in `rotate(45deg) translateX(50%)` the offset is rotated, and in
/// `scale(2) translateX(50%)` it is doubled (#212).
///
/// Its total contribution to the final translation is nevertheless *linear* in
/// the box's width and height, because each percentage translate contributes
/// `L·(pₓ·W, p_y·H)` for the accumulated linear part `L` in effect at its
/// position in the list. So four coefficients suffice however many translate
/// functions appear, and `compose_node_transform` resolves them with two
/// multiply-adds once the box is known.
#[derive(Debug, Clone, Serialize)]
pub struct TransformValue {
    /// Pre-computed 2D affine matrix [a, b, c, d, e, f], with the percentage
    /// part of every `translate` excluded (see the type doc).
    pub matrix: [f64; 6],
    /// Whether this is the identity transform (no-op): what paint and hit
    /// testing ask, through [`crate::Node::has_applied_transform`], to skip a
    /// transform that moves nothing. `true` for `none`, and also for a list
    /// that composes to the identity (`translateX(0)`, `rotate(0deg)`).
    pub is_identity: bool,
    /// Whether the computed value is `none` — an empty function list. This,
    /// not [`Self::is_identity`], decides a transform's side effects
    /// (#415): CSS makes any transform other than `none` a stacking context
    /// and the containing block of its absolute descendants, whatever matrix
    /// it composes to ([`crate::Node::has_transform`]). `!is_identity`
    /// implies `!is_none`.
    pub is_none: bool,
    /// The `(e, f)` contribution per unit of the element's **width**, summed
    /// over every percentage `translateX` in the list, each in its own frame.
    pub pct_translate_w: [f64; 2],
    /// The same per unit of the element's **height**, for `translateY`.
    pub pct_translate_h: [f64; 2],
    /// The function list the fields above were composed from, which is what a
    /// transition or animation interpolates (#414). Empty for `none`.
    ///
    /// Everything else — paint, hit testing, layout — reads the composed form
    /// and must go on doing so; this is kept only because CSS interpolates
    /// `rotate(0deg)` → `rotate(90deg)` as `rotate(45deg)`, which the composed
    /// matrices cannot express.
    #[serde(skip)]
    pub functions: Vec<crate::transition::TransformOp>,
    /// Whether the transform — about the element's `transform-origin`, z
    /// included — turns the element's back to the viewer
    /// ([`compose_about_origin_z`](crate::transition::compose_about_origin_z)).
    /// Only `backface-visibility: hidden` reads it (#997).
    pub back_facing: bool,
}

impl Default for TransformValue {
    fn default() -> Self {
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            is_identity: true,
            is_none: true,
            pct_translate_w: [0.0, 0.0],
            pct_translate_h: [0.0, 0.0],
            functions: Vec::new(),
            back_facing: false,
        }
    }
}

/// CSS `object-fit` property for replaced elements (`<img>`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum ObjectFitValue {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

impl ObjectFitValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Contain => "contain",
            Self::Cover => "cover",
            Self::None => "none",
            Self::ScaleDown => "scale-down",
        }
    }
}

/// CSS `image-rendering` (css-images-3 §5.3, #2098): how an image drawn at
/// another size than its own is sampled. Inherited. Stylo's servo build
/// parses `auto`, `crisp-edges` (and `-moz-crisp-edges`) and `pixelated`;
/// `smooth` and the legacy `optimizeSpeed`/`optimizeQuality` are gecko-only
/// there, so such a declaration is dropped and the inherited value stays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum ImageRenderingValue {
    /// The backend's own filter (bilinear on the GPU; see
    /// [`ImageSampling::Auto`](crate::paint::painter::ImageSampling::Auto)).
    #[default]
    Auto,
    /// Nearest-neighbour, up and down: Chrome 156 draws both keywords so.
    CrispEdges,
    /// Nearest-neighbour, up and down (Chrome 156; the spec lets a
    /// downscale stay smooth, Chrome does not).
    Pixelated,
}

impl ImageRenderingValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::CrispEdges => "crisp-edges",
            Self::Pixelated => "pixelated",
        }
    }

    /// Whether the value asks for nearest-neighbour sampling.
    pub fn is_nearest(self) -> bool {
        !matches!(self, Self::Auto)
    }
}

/// A `<blend-mode>` (compositing-1 §9.2): one entry of
/// `background-blend-mode` (#2221). `Normal` is source-over; every other
/// keyword is a [`MixMode`](crate::paint::painter::MixMode) the painters
/// blend with ([`Self::mix`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum BlendModeValue {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendModeValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Multiply => "multiply",
            Self::Screen => "screen",
            Self::Overlay => "overlay",
            Self::Darken => "darken",
            Self::Lighten => "lighten",
            Self::ColorDodge => "color-dodge",
            Self::ColorBurn => "color-burn",
            Self::HardLight => "hard-light",
            Self::SoftLight => "soft-light",
            Self::Difference => "difference",
            Self::Exclusion => "exclusion",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Color => "color",
            Self::Luminosity => "luminosity",
        }
    }

    /// The mode the painters blend with, `None` for `normal`.
    pub fn mix(self) -> Option<crate::paint::painter::MixMode> {
        use crate::paint::painter::MixMode as X;
        Some(match self {
            Self::Normal => return None,
            Self::Multiply => X::Multiply,
            Self::Screen => X::Screen,
            Self::Overlay => X::Overlay,
            Self::Darken => X::Darken,
            Self::Lighten => X::Lighten,
            Self::ColorDodge => X::ColorDodge,
            Self::ColorBurn => X::ColorBurn,
            Self::HardLight => X::HardLight,
            Self::SoftLight => X::SoftLight,
            Self::Difference => X::Difference,
            Self::Exclusion => X::Exclusion,
            Self::Hue => X::Hue,
            Self::Saturation => X::Saturation,
            Self::Color => X::Color,
            Self::Luminosity => X::Luminosity,
        })
    }
}

/// CSS `aspect-ratio` (css-sizing-4 §2, #1286): `auto`, `<ratio>`, or
/// `auto <ratio>`. `ratio` is width / height, `None` for `auto` alone and for
/// a degenerate ratio (`0 / 1`, `1 / 0`), which behaves as `auto`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AspectRatioValue {
    /// The `auto` keyword: a box with a natural aspect ratio uses that one.
    pub auto: bool,
    /// The preferred ratio, width / height.
    pub ratio: Option<f32>,
}

impl Default for AspectRatioValue {
    fn default() -> Self {
        Self {
            auto: true,
            ratio: None,
        }
    }
}

impl AspectRatioValue {
    /// The ratio a box with natural ratio `natural` (a replaced element's;
    /// `None` for everything else) is laid out with: a `<ratio>` without
    /// `auto` replaces the natural one, `auto <ratio>` is used only when there
    /// is no natural one.
    pub fn used(&self, natural: Option<f32>) -> Option<f32> {
        self.used_ratio(natural).map(|r| r.value)
    }

    /// [`Self::used`], saying which box it relates: a `<ratio>` without
    /// `auto` the `box-sizing` box, a natural ratio and `auto <ratio>`'s the
    /// content box (css-sizing-4 §2; Chrome 155).
    pub fn used_ratio(&self, natural: Option<f32>) -> Option<UsedRatio> {
        match (self.auto, self.ratio, natural) {
            (false, Some(value), _) => Some(UsedRatio {
                value,
                box_sizing_box: true,
            }),
            (_, r, n) => n.or(r).map(|value| UsedRatio {
                value,
                box_sizing_box: false,
            }),
        }
    }
}

/// The aspect ratio a replaced element is laid out with
/// ([`AspectRatioValue::used_ratio`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsedRatio {
    /// Width / height.
    pub value: f32,
    /// A `<ratio>` declared without `auto`, which relates the sizes of the
    /// `box-sizing` box; otherwise the content box's.
    pub box_sizing_box: bool,
}

/// CSS `user-select` property values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub enum UserSelectValue {
    #[default]
    Auto,
    Text,
    None,
    All,
    Contain,
}

impl UserSelectValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "text" => Self::Text,
            "none" => Self::None,
            "all" => Self::All,
            "contain" => Self::Contain,
            _ => Self::Auto,
        }
    }

    /// Whether text is selectable (resolves `auto` as `none` —
    /// UA stylesheet sets `text` on code/pre explicitly).
    pub fn is_selectable(&self) -> bool {
        matches!(self, Self::Text | Self::All | Self::Contain)
    }
}

/// CSS background value — solid color, gradient, or image URL.
#[derive(Debug, Clone, Default, Serialize)]
pub enum BackgroundValue {
    #[default]
    None,
    Color(#[serde(serialize_with = "color_serde::serialize_direct")] peniko::Color),
    /// `#[non_exhaustive]` (#1798): a gradient feature not read yet
    /// (interpolation hints, a colour space) can add a field without
    /// breaking a match.
    #[non_exhaustive]
    LinearGradient {
        angle_degrees: f32,
        stops: Vec<GradientStop>,
        /// `repeating-linear-gradient()` (#1798).
        repeating: bool,
    },
    #[non_exhaustive]
    RadialGradient {
        stops: Vec<GradientStop>,
        /// The ending shape and its size (#1798).
        shape: RadialShape,
        /// The centre, against the box's width and height (#1798).
        position_x: LengthPercentageValue,
        position_y: LengthPercentageValue,
        /// `repeating-radial-gradient()` (#1798).
        repeating: bool,
    },
    /// A background image loaded from a URL or file path.
    Image {
        url: String,
    },
}

/// `background-size` of the first background layer (#468).
///
/// `Explicit` carries `auto` per axis as `LengthPercentageAutoValue::Auto`,
/// so the initial value `auto auto` is `Explicit { Auto, Auto }`.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum BackgroundSizeValue {
    Explicit {
        width: LengthPercentageAutoValue,
        height: LengthPercentageAutoValue,
    },
    Cover,
    Contain,
}

impl Default for BackgroundSizeValue {
    fn default() -> Self {
        Self::Explicit {
            width: LengthPercentageAutoValue::Auto,
            height: LengthPercentageAutoValue::Auto,
        }
    }
}

/// One axis of `background-repeat` (#468). `space` and `round` are carried
/// but painted as `repeat` — see `paint::background`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum BackgroundRepeatValue {
    #[default]
    Repeat,
    Space,
    Round,
    NoRepeat,
}

/// `background-origin`: the box the image is positioned and sized in (#468).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum BackgroundOriginValue {
    BorderBox,
    #[default]
    PaddingBox,
    ContentBox,
}

/// How one background layer is sized, positioned and tiled: the per-layer
/// entries of `background-size`, `-position`, `-repeat` and `-origin` (#468,
/// #1142).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct BackgroundLayerGeometry {
    pub size: BackgroundSizeValue,
    pub position_x: LengthPercentageValue,
    pub position_y: LengthPercentageValue,
    pub repeat_x: BackgroundRepeatValue,
    pub repeat_y: BackgroundRepeatValue,
    pub origin: BackgroundOriginValue,
}

/// A `background-image` layer below the first (#1142): its image and the
/// entries of the other background lists at its index, each list repeated
/// to the number of images (css-backgrounds-3 §2.2).
#[derive(Debug, Clone, Serialize)]
pub struct BackgroundLayer {
    /// `LinearGradient`, `RadialGradient` or `Image`, never `None` or
    /// `Color`: a layer with nothing rinch can paint is not kept.
    pub image: BackgroundValue,
    pub geometry: BackgroundLayerGeometry,
    /// Its `background-blend-mode` entry (#2221): how it blends with the
    /// layers and the colour under it. Always `Normal` for a `mask-image`
    /// layer.
    pub blend: BlendModeValue,
}

/// A gradient color stop. `#[non_exhaustive]`, as the gradient variants
/// are (#1798).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct GradientStop {
    /// Where the stop is along the gradient line (a percentage of its
    /// length, a length, or both). `None` for a stop written without a
    /// position, which paint places between its neighbours
    /// (css-images-3 §3.5.3). Resolved against the line's length at paint
    /// (#1798: a length used to be read as a hundredth of the line, so
    /// `31px` was `31%`).
    pub position: Option<LengthPercentageValue>,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
}

/// A radial gradient's ending shape (#1798).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum RadialShape {
    /// `circle <length>`.
    Circle(f32),
    /// An ellipse with explicit radii, each against the box's width and
    /// height.
    Ellipse(LengthPercentageValue, LengthPercentageValue),
    /// A circle or ellipse sized to an extent keyword.
    Extent { circle: bool, extent: RadialExtent },
}

/// A radial gradient's size keyword (#1798).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum RadialExtent {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
}
