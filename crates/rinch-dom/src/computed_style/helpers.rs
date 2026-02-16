//! Standalone helper functions for parsing CSS values.

/// Parse a pixel value like "10px" or "10" to f32.
pub(super) fn parse_px_value(value: &str) -> f32 {
    let v = value.trim().strip_suffix("px").unwrap_or(value.trim());
    v.parse().unwrap_or(0.0)
}

/// Parse font-size CSS value to f32 pixels.
pub(super) fn parse_font_size_value(value: &str) -> f32 {
    let value = value.trim();
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse().unwrap_or(16.0);
    }
    if let Some(rem) = value.strip_suffix("rem") {
        return rem.trim().parse::<f32>().unwrap_or(1.0) * 16.0;
    }
    value.parse().unwrap_or(16.0)
}

/// Parse font-weight CSS value to f32.
pub(super) fn parse_font_weight_value(value: &str) -> f32 {
    match value.trim() {
        "normal" => 400.0,
        "bold" => 700.0,
        "lighter" => 300.0,
        "bolder" => 800.0,
        _ => value.parse().unwrap_or(400.0),
    }
}

/// Parse a CSS grid-template-columns/rows value into Taffy GridTemplateComponent.
///
/// Supports:
/// - `repeat(N, 1fr)` → N columns of 1fr
/// - `repeat(auto-fill, minmax(Xpx, 1fr))` → auto-fill with minmax
/// - `repeat(auto-fit, minmax(Xpx, 1fr))` → auto-fit with minmax
/// - `Npx Npx ...` → explicit pixel tracks
/// - `1fr 1fr ...` → explicit fr tracks
pub(super) fn parse_grid_template(value: &str) -> Vec<taffy::GridTemplateComponent<String>> {
    use taffy::RepetitionCount;
    use taffy::style_helpers::repeat;

    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }

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
pub(super) fn parse_single_track(value: &str) -> taffy::TrackSizingFunction {
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

    // Simple value
    parse_non_repeated_track(value)
}

/// Parse a non-repeated track sizing function from a string like "1fr", "100px", "auto".
pub(super) fn parse_non_repeated_track(value: &str) -> taffy::TrackSizingFunction {
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
pub(super) fn parse_min_track(value: &str) -> taffy::MinTrackSizingFunction {
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
pub(super) fn parse_max_track(value: &str) -> taffy::MaxTrackSizingFunction {
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
