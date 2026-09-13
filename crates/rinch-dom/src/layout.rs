//! The CSS parsing this crate does outside Stylo's cascade, plus two small
//! types the Taffy conversion reads.
//!
//! - [`parse_color`] / [`parse_color_with_current`] — a bare `<color>` through
//!   Stylo's own parser, for SVG paint (`paint::svg`) and the
//!   `--rinch-scrollbar-color` custom property.
//! - [`css_line_height_to_parley`] — a raw `line-height` string to Parley's
//!   own type, for the two inline-layout call sites.
//! - [`Viewport`] and [`DefaultDisplay`] — inputs to `out_of_flow` and
//!   `ComputedStyle::to_taffy_style`.
//!
//! A second, parallel style engine used to live here as well: a
//! `HashMap<String, String>` → `taffy::Style` builder, plus a per-property
//! parser for each of its inputs. Nothing called any of it, so #458 removed it
//! — as #254 removed the `ComputedStyle::from_props` half it fed.

/// Viewport dimensions for resolving vh/vw units.
///
/// Set from the viewport size when layout is resolved, and read wherever a
/// `vh`/`vw` unit has to become px — `out_of_flow`, `calc_layout`, and the
/// `calc()` resolution in `layout_engine`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

/// Whether an element's *tag* is block- or inline-level, for the defaults CSS
/// leaves to the UA.
///
/// `style_resolution::default_display_for_node` derives it from the node's
/// `DisplayMode`; `ComputedStyle::to_taffy_style` reads it to pick the default
/// flex direction for an element whose author set no `display` — `Block` gives
/// a column, `Inline` a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefaultDisplay {
    /// Block-level element (div, p, h1, …).
    Block,
    /// Inline element (span, a, …).
    Inline,
}

/// The `about:blank` URL data every ad-hoc Stylo parse in this crate uses —
/// inline `style` attributes, author stylesheets, SVG colours. Building
/// `UrlExtraData` parses a URL; do that once, not once per `set_style` call or
/// per SVG child per paint (#259). Callers only borrow it.
pub(crate) static BLANK_URL_DATA: std::sync::LazyLock<style::stylesheets::UrlExtraData> =
    std::sync::LazyLock::new(|| {
        style::stylesheets::UrlExtraData::from(
            url::Url::parse("about:blank").expect("about:blank is a URL"),
        )
    });

/// Parse a CSS `<color>` to a peniko Color.
///
/// Delegates to stylo — the same CSS Color 4 parser the stylesheet and
/// `style=""` paths use — so it accepts everything stylo accepts as an
/// *absolute* colour: the full named table (matched case-insensitively),
/// `transparent`, 3/4/6/8-digit hex, legacy and modern `rgb()`/`rgba()`/
/// `hsl()`/`hsla()`, `hwb()`, `lab()`/`lch()`/`oklab()`/`oklch()`, `color()`,
/// and a `color-mix()` of absolute colours. Out-of-range components clamp as
/// CSS specifies (`rgb(300, 0, 0)` is red); trailing junk and unknown
/// identifiers are rejected.
///
/// Returns `None` for anything that is not an absolute colour — including
/// `currentcolor`, by design: it depends on the element, so a caller that has
/// one calls [`parse_color_with_current`] instead.
pub fn parse_color(value: &str) -> Option<peniko::Color> {
    use cssparser::{Parser, ParserInput};
    use style::context::QuirksMode;
    use style::parser::ParserContext;
    use style::stylesheets::{CssRuleType, Origin};
    use style::values::specified::Color;
    use style_traits::ParsingMode;

    // A bare-value context built the way stylo's own `parse_style_attribute`
    // builds it: author origin, `about:blank`, no quirks, no error reporting.
    let context = ParserContext::new(
        Origin::Author,
        &BLANK_URL_DATA,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        /* namespaces = */ Default::default(),
        None,
        None,
    );
    let mut input = ParserInput::new(value.trim());
    let mut parser = Parser::new(&mut input);
    // `parse_and_compute` parses the whole input (junk rejects) and computes
    // without a device, which leaves `currentcolor` & co. non-absolute.
    let computed = Color::parse_and_compute(&context, &mut parser, None)?;
    crate::computed_style::color_from_stylo(&computed)
}

/// Like [`parse_color`], but resolves against an element's `color` instead of
/// declining (#256).
///
/// `parse_color` answers `None` for every non-absolute colour, which is right
/// for a caller with no element in hand but wrong for one that has it: an SVG
/// `fill` is resolved per element, and `currentcolor`, `color-mix(in srgb,
/// currentcolor, blue)`, `rgb(from currentcolor r g b / 50%)` and
/// `contrast-color(currentcolor)` are all resolvable once `current` is known.
/// Only the first of those was handled, by a string compare in
/// `paint::svg::resolve_svg_color`; the rest painted nothing at all.
///
/// `parse_color` is deliberately left as it is — a bare-value parser with no
/// element context must keep declining, and `paint_tests` pins that.
pub fn parse_color_with_current(value: &str, current: peniko::Color) -> Option<peniko::Color> {
    use cssparser::{Parser, ParserInput};
    use style::context::QuirksMode;
    use style::parser::ParserContext;
    use style::stylesheets::{CssRuleType, Origin};
    use style::values::specified::Color;
    use style_traits::ParsingMode;

    let context = ParserContext::new(
        Origin::Author,
        &BLANK_URL_DATA,
        Some(CssRuleType::Style),
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        /* namespaces = */ Default::default(),
        None,
        None,
    );
    let mut input = ParserInput::new(value.trim());
    let mut parser = Parser::new(&mut input);
    let computed = Color::parse_and_compute(&context, &mut parser, None)?;
    crate::computed_style::color_from_computed(
        &computed,
        &crate::computed_style::absolute_from_peniko(current),
    )
}

/// Convert a raw CSS line-height string to a Parley LineHeight.
pub fn css_line_height_to_parley(css: &str) -> Option<parley::style::LineHeight> {
    let css = css.trim();
    if css.is_empty() || css == "normal" {
        return None;
    }
    if let Some(px) = css.strip_suffix("px")
        && let Ok(v) = px.trim().parse::<f32>()
    {
        return Some(parley::style::LineHeight::Absolute(v));
    }
    // Unitless = font-size relative multiplier
    if let Ok(v) = css.parse::<f32>() {
        return Some(parley::style::LineHeight::FontSizeRelative(v));
    }
    None
}
