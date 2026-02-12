//! CSS property parsing and Taffy style building.

use std::collections::HashMap;

use taffy::prelude::*;
use taffy::{Overflow, Point};

/// Viewport dimensions for resolving vh/vw units.
/// Set before building taffy styles.
#[derive(Debug, Clone, Copy, Default)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

/// Resolve a viewport unit (vh/vw) to px if applicable, otherwise return None.
fn resolve_viewport_unit(value: &str, viewport: &Viewport) -> Option<f32> {
    if let Some(num_str) = value.strip_suffix("vh") {
        num_str
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v * viewport.height / 100.0)
    } else if let Some(num_str) = value.strip_suffix("vw") {
        num_str
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v * viewport.width / 100.0)
    } else {
        None
    }
}

/// Parse a CSS dimension value like "100px", "50%", "auto", "100vh", "100vw".
pub fn parse_dimension(value: &str) -> Dimension {
    parse_dimension_with_viewport(value, &Viewport::default())
}

/// Parse a CSS dimension value with viewport unit support.
pub fn parse_dimension_with_viewport(value: &str, viewport: &Viewport) -> Dimension {
    let value = value.trim();
    if value == "auto" {
        Dimension::auto()
    } else if let Some(px) = resolve_viewport_unit(value, viewport) {
        Dimension::length(px)
    } else if let Some(pct) = value.strip_suffix('%') {
        pct.trim()
            .parse::<f32>()
            .map(|v| Dimension::percent(v / 100.0))
            .unwrap_or(Dimension::auto())
    } else if let Some(px) = value.strip_suffix("px") {
        px.trim()
            .parse::<f32>()
            .map(Dimension::length)
            .unwrap_or(Dimension::auto())
    } else {
        value
            .parse::<f32>()
            .map(Dimension::length)
            .unwrap_or(Dimension::auto())
    }
}

/// Parse a CSS length-percentage value like "10px", "50%".
pub fn parse_length_percentage(value: &str) -> LengthPercentage {
    parse_length_percentage_with_viewport(value, &Viewport::default())
}

/// Parse a CSS length-percentage value with viewport unit support.
pub fn parse_length_percentage_with_viewport(value: &str, viewport: &Viewport) -> LengthPercentage {
    let value = value.trim();
    if let Some(px) = resolve_viewport_unit(value, viewport) {
        LengthPercentage::length(px)
    } else if let Some(pct) = value.strip_suffix('%') {
        pct.trim()
            .parse::<f32>()
            .map(|v| LengthPercentage::percent(v / 100.0))
            .unwrap_or(LengthPercentage::length(0.0))
    } else if let Some(px) = value.strip_suffix("px") {
        px.trim()
            .parse::<f32>()
            .map(LengthPercentage::length)
            .unwrap_or(LengthPercentage::length(0.0))
    } else {
        value
            .parse::<f32>()
            .map(LengthPercentage::length)
            .unwrap_or(LengthPercentage::length(0.0))
    }
}

/// Parse a CSS length-percentage-auto value.
#[allow(dead_code)]
fn parse_length_percentage_auto(value: &str) -> LengthPercentageAuto {
    parse_length_percentage_auto_with_viewport(value, &Viewport::default())
}

fn parse_length_percentage_auto_with_viewport(
    value: &str,
    viewport: &Viewport,
) -> LengthPercentageAuto {
    let value = value.trim();
    if value == "auto" {
        LengthPercentageAuto::auto()
    } else if let Some(px) = resolve_viewport_unit(value, viewport) {
        LengthPercentageAuto::length(px)
    } else if let Some(pct) = value.strip_suffix('%') {
        pct.trim()
            .parse::<f32>()
            .map(|v| LengthPercentageAuto::percent(v / 100.0))
            .unwrap_or(LengthPercentageAuto::auto())
    } else if let Some(px) = value.strip_suffix("px") {
        px.trim()
            .parse::<f32>()
            .map(LengthPercentageAuto::length)
            .unwrap_or(LengthPercentageAuto::auto())
    } else {
        value
            .parse::<f32>()
            .map(LengthPercentageAuto::length)
            .unwrap_or(LengthPercentageAuto::auto())
    }
}

/// Parse a CSS grid-template-columns/rows value into Taffy GridTemplateComponents.
///
/// Supports:
/// - `repeat(N, 1fr)` → N columns of 1fr
/// - `repeat(auto-fill, minmax(Xpx, 1fr))` → auto-fill with minmax
/// - `repeat(auto-fit, minmax(Xpx, 1fr))` → auto-fit with minmax
/// - `Npx Npx ...` → explicit pixel tracks
/// - `1fr 1fr ...` → explicit fr tracks
fn parse_grid_template(value: &str) -> Vec<taffy::GridTemplateComponent<String>> {
    use taffy::RepetitionCount;
    use taffy::style_helpers::repeat;

    let value = value.trim();

    // Handle repeat(...)
    if let Some(inner) = value
        .strip_prefix("repeat(")
        .and_then(|s| s.strip_suffix(')'))
        && let Some((count_str, tracks_str)) = inner.split_once(',')
    {
        let count_str = count_str.trim();
        let tracks_str = tracks_str.trim();

        let repetition = match count_str {
            "auto-fill" => RepetitionCount::AutoFill,
            "auto-fit" => RepetitionCount::AutoFit,
            _ => {
                if let Ok(n) = count_str.parse::<u16>() {
                    RepetitionCount::Count(n)
                } else {
                    return Vec::new();
                }
            }
        };

        let track = parse_single_track(tracks_str);
        return vec![repeat(repetition, vec![track])];
    }

    // Handle space-separated track list: "1fr 1fr 1fr" or "100px 200px"
    value
        .split_whitespace()
        .map(|part| taffy::GridTemplateComponent::Single(parse_single_track(part)))
        .collect()
}

/// Parse a single track sizing value, which may be `minmax(min, max)` or a simple value.
fn parse_single_track(value: &str) -> taffy::TrackSizingFunction {
    use taffy::style_helpers::minmax;

    let value = value.trim();

    // Handle minmax(min, max)
    if let Some(inner) = value
        .strip_prefix("minmax(")
        .and_then(|s| s.strip_suffix(')'))
        && let Some((min_str, max_str)) = inner.split_once(',')
    {
        let min = parse_min_track(min_str.trim());
        let max = parse_max_track(max_str.trim());
        return minmax(min, max);
    }

    parse_non_repeated_track(value)
}

/// Parse a non-repeated track sizing function from a string like "1fr", "100px", "auto".
fn parse_non_repeated_track(value: &str) -> taffy::TrackSizingFunction {
    use taffy::style_helpers::minmax;

    let value = value.trim();
    if let Some(fr_val) = value.strip_suffix("fr") {
        let f = fr_val.trim().parse::<f32>().unwrap_or(1.0);
        minmax(
            taffy::MinTrackSizingFunction::auto(),
            taffy::MaxTrackSizingFunction::fr(f),
        )
    } else if value == "auto" {
        minmax(
            taffy::MinTrackSizingFunction::auto(),
            taffy::MaxTrackSizingFunction::auto(),
        )
    } else if value == "min-content" {
        minmax(
            taffy::MinTrackSizingFunction::min_content(),
            taffy::MaxTrackSizingFunction::min_content(),
        )
    } else if value == "max-content" {
        minmax(
            taffy::MinTrackSizingFunction::max_content(),
            taffy::MaxTrackSizingFunction::max_content(),
        )
    } else if let Some(pct) = value.strip_suffix('%') {
        let p = pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
        minmax(
            taffy::MinTrackSizingFunction::percent(p),
            taffy::MaxTrackSizingFunction::percent(p),
        )
    } else {
        // Assume px value
        let px = value.strip_suffix("px").unwrap_or(value);
        let v = px.trim().parse::<f32>().unwrap_or(0.0);
        minmax(
            taffy::MinTrackSizingFunction::length(v),
            taffy::MaxTrackSizingFunction::length(v),
        )
    }
}

/// Parse a min track sizing function.
fn parse_min_track(value: &str) -> taffy::MinTrackSizingFunction {
    match value {
        "auto" => taffy::MinTrackSizingFunction::auto(),
        "min-content" => taffy::MinTrackSizingFunction::min_content(),
        "max-content" => taffy::MinTrackSizingFunction::max_content(),
        _ => {
            if let Some(pct) = value.strip_suffix('%') {
                let p = pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
                taffy::MinTrackSizingFunction::percent(p)
            } else {
                let px = value.strip_suffix("px").unwrap_or(value);
                let v = px.trim().parse::<f32>().unwrap_or(0.0);
                taffy::MinTrackSizingFunction::length(v)
            }
        }
    }
}

/// Parse a max track sizing function.
fn parse_max_track(value: &str) -> taffy::MaxTrackSizingFunction {
    match value {
        "auto" => taffy::MaxTrackSizingFunction::auto(),
        "min-content" => taffy::MaxTrackSizingFunction::min_content(),
        "max-content" => taffy::MaxTrackSizingFunction::max_content(),
        _ => {
            if let Some(fr_val) = value.strip_suffix("fr") {
                let f = fr_val.trim().parse::<f32>().unwrap_or(1.0);
                taffy::MaxTrackSizingFunction::fr(f)
            } else if let Some(pct) = value.strip_suffix('%') {
                let p = pct.trim().parse::<f32>().unwrap_or(0.0) / 100.0;
                taffy::MaxTrackSizingFunction::percent(p)
            } else {
                let px = value.strip_suffix("px").unwrap_or(value);
                let v = px.trim().parse::<f32>().unwrap_or(0.0);
                taffy::MaxTrackSizingFunction::length(v)
            }
        }
    }
}

/// Parse a CSS style string into key-value pairs.
pub fn parse_style_string(style: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for part in style.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once(':') {
            result.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    result
}

/// Build a Taffy Style from a CSS style string.
pub fn build_taffy_style(style_str: &str) -> Style {
    let props = parse_style_string(style_str);
    build_taffy_style_from_props_with_viewport(&props, &Viewport::default())
}

/// Build a Taffy Style from parsed CSS properties.
pub fn build_taffy_style_from_props(props: &HashMap<String, String>) -> Style {
    build_taffy_style_from_props_with_viewport(props, &Viewport::default())
}

/// Default display style for an element based on its tag type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefaultDisplay {
    /// Block-level element (div, p, h1, etc.) — display:block
    Block,
    /// Inline element (span, a, etc.) — display:flex row for inline layout
    Inline,
}

/// Build a Taffy Style from parsed CSS properties with viewport unit support.
pub fn build_taffy_style_from_props_with_viewport(
    props: &HashMap<String, String>,
    vp: &Viewport,
) -> Style {
    build_taffy_style_full(props, vp, DefaultDisplay::Block)
}

/// Build a Taffy Style from parsed CSS properties with viewport and default display.
pub fn build_taffy_style_full(
    props: &HashMap<String, String>,
    vp: &Viewport,
    default_display: DefaultDisplay,
) -> Style {
    // Shadow parse functions with viewport-aware versions
    let parse_dimension = |v: &str| parse_dimension_with_viewport(v, vp);
    let parse_length_percentage = |v: &str| parse_length_percentage_with_viewport(v, vp);
    let parse_length_percentage_auto = |v: &str| parse_length_percentage_auto_with_viewport(v, vp);

    let has_explicit_display = props.get("display").map(|v| v.as_str()) != Some("contents")
        && props.contains_key("display");

    // Choose defaults based on element type and whether CSS overrides are present.
    // Block elements use Taffy's native Block layout (children stack vertically, full width).
    // When CSS specifies display, use standard CSS flex defaults (row, nowrap).
    let (default_display_val, default_direction, default_wrap) = if has_explicit_display {
        (Display::Flex, FlexDirection::Row, FlexWrap::NoWrap)
    } else {
        match default_display {
            DefaultDisplay::Block => (Display::Block, FlexDirection::Row, FlexWrap::NoWrap),
            DefaultDisplay::Inline => (Display::Flex, FlexDirection::Row, FlexWrap::NoWrap),
        }
    };

    let mut style = Style {
        display: default_display_val,
        flex_direction: default_direction,
        flex_wrap: default_wrap,
        ..Default::default()
    };

    for (key, value) in props {
        match key.as_str() {
            "display" => {
                style.display = match value.as_str() {
                    "flex" => Display::Flex,
                    "inline-flex" => Display::Flex, // treat inline-flex as flex for Taffy
                    "grid" => Display::Grid,
                    "block" => Display::Block,
                    "none" => Display::None,
                    "contents" => Display::Flex, // transparent container: children participate in layout
                    "inline" => Display::Block,  // inline layout handled by IFC, Taffy sees block
                    "inline-block" => Display::Flex, // inline-block: Taffy handles internal layout
                    _ => Display::default(),
                };
            }
            "flex-direction" => {
                style.flex_direction = match value.as_str() {
                    "row" => FlexDirection::Row,
                    "column" => FlexDirection::Column,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => FlexDirection::default(),
                };
            }
            "flex-wrap" => {
                style.flex_wrap = match value.as_str() {
                    "wrap" => FlexWrap::Wrap,
                    "nowrap" => FlexWrap::NoWrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => FlexWrap::default(),
                };
            }
            "width" => style.size.width = parse_dimension(value),
            "height" => style.size.height = parse_dimension(value),
            "min-width" => style.min_size.width = parse_dimension(value),
            "min-height" => style.min_size.height = parse_dimension(value),
            "max-width" => style.max_size.width = parse_dimension(value),
            "max-height" => style.max_size.height = parse_dimension(value),
            "padding" => {
                let parts: Vec<&str> = value.split_whitespace().collect();
                match parts.len() {
                    1 => {
                        let lp = parse_length_percentage(parts[0]);
                        style.padding = Rect {
                            top: lp,
                            right: lp,
                            bottom: lp,
                            left: lp,
                        };
                    }
                    2 => {
                        let tb = parse_length_percentage(parts[0]);
                        let lr = parse_length_percentage(parts[1]);
                        style.padding = Rect {
                            top: tb,
                            right: lr,
                            bottom: tb,
                            left: lr,
                        };
                    }
                    4 => {
                        style.padding = Rect {
                            top: parse_length_percentage(parts[0]),
                            right: parse_length_percentage(parts[1]),
                            bottom: parse_length_percentage(parts[2]),
                            left: parse_length_percentage(parts[3]),
                        };
                    }
                    _ => {
                        let lp = parse_length_percentage(value);
                        style.padding = Rect {
                            top: lp,
                            right: lp,
                            bottom: lp,
                            left: lp,
                        };
                    }
                }
            }
            "padding-top" => style.padding.top = parse_length_percentage(value),
            "padding-right" => style.padding.right = parse_length_percentage(value),
            "padding-bottom" => style.padding.bottom = parse_length_percentage(value),
            "padding-left" => style.padding.left = parse_length_percentage(value),
            "margin" => {
                let parts: Vec<&str> = value.split_whitespace().collect();
                match parts.len() {
                    1 => {
                        let lpa = parse_length_percentage_auto(parts[0]);
                        style.margin = Rect {
                            top: lpa,
                            right: lpa,
                            bottom: lpa,
                            left: lpa,
                        };
                    }
                    2 => {
                        let tb = parse_length_percentage_auto(parts[0]);
                        let lr = parse_length_percentage_auto(parts[1]);
                        style.margin = Rect {
                            top: tb,
                            right: lr,
                            bottom: tb,
                            left: lr,
                        };
                    }
                    4 => {
                        style.margin = Rect {
                            top: parse_length_percentage_auto(parts[0]),
                            right: parse_length_percentage_auto(parts[1]),
                            bottom: parse_length_percentage_auto(parts[2]),
                            left: parse_length_percentage_auto(parts[3]),
                        };
                    }
                    _ => {
                        let lpa = parse_length_percentage_auto(value);
                        style.margin = Rect {
                            top: lpa,
                            right: lpa,
                            bottom: lpa,
                            left: lpa,
                        };
                    }
                }
            }
            "margin-top" => style.margin.top = parse_length_percentage_auto(value),
            "margin-right" => style.margin.right = parse_length_percentage_auto(value),
            "margin-bottom" => style.margin.bottom = parse_length_percentage_auto(value),
            "margin-left" => style.margin.left = parse_length_percentage_auto(value),
            "gap" => {
                let lp = parse_length_percentage(value);
                style.gap = Size {
                    width: lp,
                    height: lp,
                };
            }
            "row-gap" => style.gap.height = parse_length_percentage(value),
            "column-gap" => style.gap.width = parse_length_percentage(value),
            "align-items" => {
                style.align_items = Some(match value.as_str() {
                    "flex-start" | "start" => AlignItems::FlexStart,
                    "flex-end" | "end" => AlignItems::FlexEnd,
                    "center" => AlignItems::Center,
                    "baseline" => AlignItems::Baseline,
                    "stretch" => AlignItems::Stretch,
                    _ => AlignItems::Stretch,
                });
            }
            "justify-content" => {
                style.justify_content = Some(match value.as_str() {
                    "flex-start" | "start" => JustifyContent::FlexStart,
                    "flex-end" | "end" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => JustifyContent::FlexStart,
                });
            }
            "align-self" => {
                style.align_self = Some(match value.as_str() {
                    "flex-start" | "start" => AlignSelf::FlexStart,
                    "flex-end" | "end" => AlignSelf::FlexEnd,
                    "center" => AlignSelf::Center,
                    "baseline" => AlignSelf::Baseline,
                    "stretch" => AlignSelf::Stretch,
                    _ => AlignSelf::Stretch,
                });
            }
            "flex-grow" => {
                style.flex_grow = value.parse::<f32>().unwrap_or(0.0);
            }
            "flex-shrink" => {
                style.flex_shrink = value.parse::<f32>().unwrap_or(1.0);
            }
            "flex-basis" => {
                style.flex_basis = parse_dimension(value);
            }
            "flex" => {
                // flex shorthand: flex: 1 = flex-grow: 1, flex-shrink: 1, flex-basis: 0%
                let parts: Vec<&str> = value.split_whitespace().collect();
                match parts.len() {
                    1 => {
                        if value == "none" {
                            style.flex_grow = 0.0;
                            style.flex_shrink = 0.0;
                            style.flex_basis = Dimension::auto();
                        } else if value == "auto" {
                            style.flex_grow = 1.0;
                            style.flex_shrink = 1.0;
                            style.flex_basis = Dimension::auto();
                        } else if let Ok(grow) = value.parse::<f32>() {
                            style.flex_grow = grow;
                            style.flex_shrink = 1.0;
                            style.flex_basis = Dimension::length(0.0);
                        }
                    }
                    2 => {
                        if let Ok(grow) = parts[0].parse::<f32>() {
                            style.flex_grow = grow;
                        }
                        if let Ok(shrink) = parts[1].parse::<f32>() {
                            style.flex_shrink = shrink;
                        }
                        style.flex_basis = Dimension::length(0.0);
                    }
                    3 => {
                        if let Ok(grow) = parts[0].parse::<f32>() {
                            style.flex_grow = grow;
                        }
                        if let Ok(shrink) = parts[1].parse::<f32>() {
                            style.flex_shrink = shrink;
                        }
                        style.flex_basis = parse_dimension(parts[2]);
                    }
                    _ => {}
                }
            }
            "overflow" => {
                let ov = match value.as_str() {
                    "hidden" => Overflow::Hidden,
                    "scroll" => Overflow::Scroll,
                    "visible" => Overflow::Visible,
                    "clip" => Overflow::Clip,
                    _ => Overflow::Visible,
                };
                style.overflow = Point { x: ov, y: ov };
            }
            "overflow-x" => {
                style.overflow.x = match value.as_str() {
                    "hidden" => Overflow::Hidden,
                    "scroll" => Overflow::Scroll,
                    "visible" => Overflow::Visible,
                    "clip" => Overflow::Clip,
                    "auto" => Overflow::Scroll,
                    _ => Overflow::Visible,
                };
            }
            "overflow-y" => {
                style.overflow.y = match value.as_str() {
                    "hidden" => Overflow::Hidden,
                    "scroll" => Overflow::Scroll,
                    "visible" => Overflow::Visible,
                    "clip" => Overflow::Clip,
                    "auto" => Overflow::Scroll,
                    _ => Overflow::Visible,
                };
            }
            "position" => {
                style.position = match value.as_str() {
                    "absolute" | "fixed" => Position::Absolute,
                    "relative" | "static" => Position::Relative,
                    _ => Position::Relative,
                };
            }
            "top" => style.inset.top = parse_length_percentage_auto(value),
            "right" => style.inset.right = parse_length_percentage_auto(value),
            "bottom" => style.inset.bottom = parse_length_percentage_auto(value),
            "left" => style.inset.left = parse_length_percentage_auto(value),
            "inset" => {
                let lpa = parse_length_percentage_auto(value);
                style.inset = Rect {
                    top: lpa,
                    right: lpa,
                    bottom: lpa,
                    left: lpa,
                };
            }
            "border" => {
                // Parse border shorthand: "Npx solid color" -> extract width
                for part in value.split_whitespace() {
                    if let Some(px) = part.strip_suffix("px")
                        && let Ok(w) = px.parse::<f32>()
                    {
                        let lp = LengthPercentage::length(w);
                        style.border = Rect {
                            top: lp,
                            right: lp,
                            bottom: lp,
                            left: lp,
                        };
                        break;
                    }
                }
            }
            "border-width" => {
                let lp = parse_length_percentage(value);
                style.border = Rect {
                    top: lp,
                    right: lp,
                    bottom: lp,
                    left: lp,
                };
            }
            "grid-template-columns" => {
                style.grid_template_columns = parse_grid_template(value);
            }
            "grid-template-rows" => {
                style.grid_template_rows = parse_grid_template(value);
            }
            "grid-auto-flow" => {
                style.grid_auto_flow = match value.as_str() {
                    "column" => taffy::GridAutoFlow::Column,
                    "dense" => taffy::GridAutoFlow::RowDense,
                    "column dense" | "dense column" => taffy::GridAutoFlow::ColumnDense,
                    _ => taffy::GridAutoFlow::Row,
                };
            }
            _ => {}
        }
    }

    style
}

/// Parse font-size from a CSS style string.
///
/// Returns `Some(size)` if a `font-size` property is found, `None` otherwise.
pub fn parse_font_size(style_str: &str) -> Option<f32> {
    let props = parse_style_string(style_str);
    let value = props.get("font-size")?;
    let value = value.trim();
    if let Some(px) = value.strip_suffix("px") {
        px.trim().parse::<f32>().ok()
    } else {
        value.parse::<f32>().ok()
    }
}

/// Parse a CSS color value to a peniko Color.
///
/// Supports:
/// - Named colors: red, green, blue, black, white, gray, transparent, etc.
/// - Hex colors: #rgb, #rrggbb, #rrggbbaa
/// - rgb(r, g, b) and rgba(r, g, b, a)
pub fn parse_color(value: &str) -> Option<peniko::Color> {
    use peniko::color::{AlphaColor, Srgb};
    let value = value.trim();

    // Named colors
    match value {
        "red" => return Some(AlphaColor::<Srgb>::from_rgba8(255, 0, 0, 255)),
        "green" => return Some(AlphaColor::<Srgb>::from_rgba8(0, 128, 0, 255)),
        "blue" => return Some(AlphaColor::<Srgb>::from_rgba8(0, 0, 255, 255)),
        "black" => return Some(AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255)),
        "white" => return Some(AlphaColor::<Srgb>::from_rgba8(255, 255, 255, 255)),
        "gray" | "grey" => return Some(AlphaColor::<Srgb>::from_rgba8(128, 128, 128, 255)),
        "transparent" => return Some(AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 0)),
        "orange" => return Some(AlphaColor::<Srgb>::from_rgba8(255, 165, 0, 255)),
        "yellow" => return Some(AlphaColor::<Srgb>::from_rgba8(255, 255, 0, 255)),
        "purple" => return Some(AlphaColor::<Srgb>::from_rgba8(128, 0, 128, 255)),
        "pink" => return Some(AlphaColor::<Srgb>::from_rgba8(255, 192, 203, 255)),
        "cyan" => return Some(AlphaColor::<Srgb>::from_rgba8(0, 255, 255, 255)),
        _ => {}
    }

    // Hex colors
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                Some(AlphaColor::<Srgb>::from_rgba8(r * 17, g * 17, b * 17, 255))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(AlphaColor::<Srgb>::from_rgba8(r, g, b, 255))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(AlphaColor::<Srgb>::from_rgba8(r, g, b, a))
            }
            _ => None,
        };
    }

    // rgb(r, g, b) and rgba(r, g, b, a)
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 4 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            let a = parts[3].trim().parse::<f32>().ok()?;
            return Some(AlphaColor::<Srgb>::from_rgba8(r, g, b, (a * 255.0) as u8));
        }
    }
    if let Some(inner) = value.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r = parts[0].trim().parse::<u8>().ok()?;
            let g = parts[1].trim().parse::<u8>().ok()?;
            let b = parts[2].trim().parse::<u8>().ok()?;
            return Some(AlphaColor::<Srgb>::from_rgba8(r, g, b, 255));
        }
    }

    None
}

/// Check if a style string contains "display: contents".
pub fn is_display_contents(style_str: &str) -> bool {
    let props = parse_style_string(style_str);
    props.get("display").map(|v| v.as_str()) == Some("contents")
}

/// Parse the display mode from a style string, returning None if not specified.
pub fn parse_display_mode(style_str: &str) -> Option<crate::node::DisplayMode> {
    use crate::node::DisplayMode;
    let props = parse_style_string(style_str);
    match props.get("display")?.as_str() {
        "inline" => Some(DisplayMode::Inline),
        "inline-block" => Some(DisplayMode::InlineBlock),
        "inline-flex" => Some(DisplayMode::Flex),
        "block" => Some(DisplayMode::Block),
        "flex" => Some(DisplayMode::Flex),
        _ => None,
    }
}

/// Parse font-weight from a style string.
pub fn parse_font_weight(style_str: &str) -> Option<f32> {
    let props = parse_style_string(style_str);
    let value = props.get("font-weight")?;
    match value.as_str() {
        "normal" => Some(400.0),
        "bold" => Some(700.0),
        "lighter" => Some(300.0),
        "bolder" => Some(800.0),
        _ => value.parse::<f32>().ok(),
    }
}

/// Parse font-style from a style string.
pub fn parse_font_style(style_str: &str) -> Option<&'static str> {
    let props = parse_style_string(style_str);
    match props.get("font-style")?.as_str() {
        "italic" => Some("italic"),
        "oblique" => Some("oblique"),
        "normal" => Some("normal"),
        _ => None,
    }
}

/// Parse text-decoration from a style string.
pub fn parse_text_decoration(style_str: &str) -> Option<String> {
    let props = parse_style_string(style_str);
    props.get("text-decoration").cloned()
}

/// Parse text-align from a style string.
pub fn parse_text_align(style_str: &str) -> Option<String> {
    let props = parse_style_string(style_str);
    props.get("text-align").cloned()
}

/// Parse line-height from a style string.
pub fn parse_line_height(style_str: &str) -> Option<f32> {
    let props = parse_style_string(style_str);
    let value = props.get("line-height")?;
    let value = value.trim();
    if value == "normal" {
        return None; // use default
    }
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse::<f32>().ok();
    }
    // Unitless number = multiplier of font-size
    value.parse::<f32>().ok()
}

/// Parse raw CSS line-height value from a style string.
pub fn parse_line_height_css(style_str: &str) -> Option<String> {
    let props = parse_style_string(style_str);
    let value = props.get("line-height")?;
    let trimmed = value.trim();
    if trimmed == "normal" {
        return None;
    }
    Some(trimmed.to_string())
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

/// Parse font-family from a style string.
pub fn parse_font_family(style_str: &str) -> Option<String> {
    let props = parse_style_string(style_str);
    props.get("font-family").cloned()
}

/// Parse white-space from a style string.
pub fn parse_white_space(style_str: &str) -> Option<String> {
    let props = parse_style_string(style_str);
    props.get("white-space").cloned()
}

/// Convert a HashMap of CSS properties to an inline style string.
pub fn props_to_style_string(props: &HashMap<String, String>) -> String {
    props
        .iter()
        .map(|(k, v)| format!("{}: {}", k, v))
        .collect::<Vec<_>>()
        .join("; ")
}
