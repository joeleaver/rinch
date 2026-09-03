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
    /// A mixed `calc()` combining a length and a percentage (#278/#404):
    /// resolves to `px + pct * basis` (`pct` is a fraction, 0.5 = 50%).
    /// A non-affine calc (`min()`/`max()`/`clamp()` inside) is stored as its
    /// large-basis linearization — see `from_stylo/calc.rs`.
    Calc {
        px: f32,
        pct: f32,
    },
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
    ///
    /// A `Calc` cannot be represented in a Taffy value — Taffy 0.12's calc
    /// pointer (`CompactLength::calc`) only works for callers implementing the
    /// layout-tree traits themselves; `TaffyTree`'s `resolve_calc_value` is
    /// hardcoded to `0.0` (taffy-0.12.2, `src/tree/taffy_tree.rs:391`). So the
    /// length part goes in as a *seed* and `resolve_layout_calcs`
    /// (`calc_layout.rs`) overwrites it with the resolved length before any
    /// layout result is read.
    pub fn to_taffy(&self) -> taffy::Dimension {
        match self {
            Self::Auto => taffy::Dimension::auto(),
            Self::Length(v) => taffy::Dimension::length(*v),
            Self::Percent(v) => taffy::Dimension::percent(*v),
            Self::Calc { px, .. } => taffy::Dimension::length(px.max(0.0)),
        }
    }

    /// Whether this is Auto.
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
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
#[derive(Debug, Clone, Copy, Default, Serialize)]
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
    ///
    /// A `Calc` cannot be represented in a Taffy value (see
    /// [`DimensionValue::to_taffy`]); the clamped length part is a seed that
    /// `resolve_layout_calcs` (`calc_layout.rs`) overwrites with the resolved
    /// length before any layout result is read. Every Taffy consumer of this
    /// enum (padding, gap) is non-negative, hence the floor.
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
    ///
    /// A `Calc` cannot be represented in a Taffy value (see
    /// [`DimensionValue::to_taffy`]); the length part is a seed that
    /// `resolve_layout_calcs` (`calc_layout.rs`) overwrites with the resolved
    /// length before any layout result is read. No floor — margins and insets
    /// are legally negative.
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
            Self::FlexStart => taffy::AlignItems::FLEX_START,
            Self::FlexEnd => taffy::AlignItems::FLEX_END,
            Self::Center => taffy::AlignItems::CENTER,
            Self::Baseline => taffy::AlignItems::BASELINE,
            Self::Stretch => taffy::AlignItems::STRETCH,
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
            Self::FlexStart => taffy::AlignSelf::FLEX_START,
            Self::FlexEnd => taffy::AlignSelf::FLEX_END,
            Self::Center => taffy::AlignSelf::CENTER,
            Self::Baseline => taffy::AlignSelf::BASELINE,
            Self::Stretch => taffy::AlignSelf::STRETCH,
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
            Self::FlexStart => taffy::JustifyContent::FLEX_START,
            Self::FlexEnd => taffy::JustifyContent::FLEX_END,
            Self::Center => taffy::JustifyContent::CENTER,
            Self::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
            Self::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
            Self::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
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
/// The real property is **gecko-only in Stylo** (`engines="gecko"` on the
/// longhand in `properties/longhands/inherited_ui.mako.rs`), and that is a
/// codegen-time filter, not a `#[cfg]`: the servo build rinch uses emits no
/// `LonghandId` for it, no parser entry and no field on any style struct, so
/// `scrollbar-color: red blue` in a stylesheet is an unknown declaration and
/// Stylo drops it. Grepping this repo's own generated `properties.rs` for
/// `scrollbar_color` finds nothing.
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

impl TextOverflowValue {
    /// Parse from CSS string value.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "ellipsis" => Self::Ellipsis,
            _ => Self::Clip,
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
    /// Whether this is the identity transform (no-op).
    pub is_identity: bool,
    /// The `(e, f)` contribution per unit of the element's **width**, summed
    /// over every percentage `translateX` in the list, each in its own frame.
    pub pct_translate_w: [f64; 2],
    /// The same per unit of the element's **height**, for `translateY`.
    pub pct_translate_h: [f64; 2],
}

impl Default for TransformValue {
    fn default() -> Self {
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            is_identity: true,
            pct_translate_w: [0.0, 0.0],
            pct_translate_h: [0.0, 0.0],
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
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "contain" => Self::Contain,
            "cover" => Self::Cover,
            "none" => Self::None,
            "scale-down" => Self::ScaleDown,
            _ => Self::Fill,
        }
    }

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
    LinearGradient {
        angle_degrees: f32,
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        stops: Vec<GradientStop>,
    },
    /// A background image loaded from a URL or file path.
    Image {
        url: String,
    },
}

/// A gradient color stop.
#[derive(Debug, Clone, Serialize)]
pub struct GradientStop {
    pub offset: f32,
    #[serde(serialize_with = "color_serde::serialize")]
    pub color: Option<peniko::Color>,
}
