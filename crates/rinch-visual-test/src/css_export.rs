//! CSS export - converts ComputedStyle JSON to CSS property strings.

use serde_json::Value;

/// Convert a computed_styles JSON object to an inline CSS string.
pub fn computed_style_to_css(styles: &Value) -> String {
    let mut css = String::new();

    let obj = match styles.as_object() {
        Some(o) => o,
        None => return css,
    };

    // Display
    if let Some(v) = obj.get("display").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Flex" => "flex",
            "Block" => "block",
            "Grid" => "grid",
            "None" => "none",
            "Contents" => "contents",
            "Inline" => "inline",
            "InlineBlock" => "inline-block",
            "InlineFlex" => "inline-flex",
            _ => "flex",
        };
        css.push_str(&format!("display: {}; ", css_val));
    }

    // Position. `static` is the CSS *and* rinch default (`PositionValue`'s
    // `#[default]`), so that is the one to skip — emitting it for every node
    // while dropping `relative` (as this used to) silently moved the containing
    // block of every absolutely-positioned descendant in the browser render.
    if let Some(v) = obj.get("position").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Absolute" => Some("absolute"),
            "Fixed" => Some("fixed"),
            "Relative" => Some("relative"),
            "Sticky" => Some("sticky"),
            // "Static" is the default; anything unrecognised is treated as it.
            _ => None,
        };
        if let Some(css_val) = css_val {
            css.push_str(&format!("position: {}; ", css_val));
        }
    }

    // Overflow
    emit_overflow(&mut css, obj, "overflow_x", "overflow-x");
    emit_overflow(&mut css, obj, "overflow_y", "overflow-y");

    // Dimensions
    emit_dimension(&mut css, obj, "width", "width");
    emit_dimension(&mut css, obj, "height", "height");
    emit_dimension(&mut css, obj, "min_width", "min-width");
    emit_dimension(&mut css, obj, "min_height", "min-height");
    emit_dimension(&mut css, obj, "max_width", "max-width");
    emit_dimension(&mut css, obj, "max_height", "max-height");

    // Flexbox
    emit_flex_direction(&mut css, obj);
    emit_flex_wrap(&mut css, obj);
    emit_f32(&mut css, obj, "flex_grow", "flex-grow", 0.0);
    emit_f32(&mut css, obj, "flex_shrink", "flex-shrink", 1.0);
    emit_dimension(&mut css, obj, "flex_basis", "flex-basis");
    emit_align(&mut css, obj, "align_items", "align-items");
    emit_align(&mut css, obj, "align_self", "align-self");
    emit_justify(&mut css, obj, "justify_content", "justify-content");

    // Padding
    emit_length_percentage(&mut css, obj, "padding_top", "padding-top");
    emit_length_percentage(&mut css, obj, "padding_right", "padding-right");
    emit_length_percentage(&mut css, obj, "padding_bottom", "padding-bottom");
    emit_length_percentage(&mut css, obj, "padding_left", "padding-left");

    // Margin
    emit_length_percentage_auto(&mut css, obj, "margin_top", "margin-top");
    emit_length_percentage_auto(&mut css, obj, "margin_right", "margin-right");
    emit_length_percentage_auto(&mut css, obj, "margin_bottom", "margin-bottom");
    emit_length_percentage_auto(&mut css, obj, "margin_left", "margin-left");

    // Gap
    emit_length_percentage(&mut css, obj, "gap_row", "row-gap");
    emit_length_percentage(&mut css, obj, "gap_column", "column-gap");

    // Positioning (inset)
    emit_length_percentage_auto(&mut css, obj, "top", "top");
    emit_length_percentage_auto(&mut css, obj, "right", "right");
    emit_length_percentage_auto(&mut css, obj, "bottom", "bottom");
    emit_length_percentage_auto(&mut css, obj, "left", "left");

    // Border widths
    emit_length_percentage(&mut css, obj, "border_top_width", "border-top-width");
    emit_length_percentage(&mut css, obj, "border_right_width", "border-right-width");
    emit_length_percentage(&mut css, obj, "border_bottom_width", "border-bottom-width");
    emit_length_percentage(&mut css, obj, "border_left_width", "border-left-width");

    // Per-side border styles. `ComputedStyle` carries `border_*_style`, so read
    // it rather than inferring "solid" from a non-zero width: `border-style:
    // none` with a resolved width paints nothing in rinch, and inferring solid
    // put a phantom border in the browser reference on every such node.
    emit_border_style(&mut css, obj, "border_top_style", "border-top-style");
    emit_border_style(&mut css, obj, "border_right_style", "border-right-style");
    emit_border_style(&mut css, obj, "border_bottom_style", "border-bottom-style");
    emit_border_style(&mut css, obj, "border_left_style", "border-left-style");

    // Per-side border colors. These serialize as "#rrggbb"/"#rrggbbaa" or null;
    // there is no aggregate `border_color` field (looking one up here always
    // missed, so every border fell back to the browser's `currentColor`).
    emit_color(&mut css, obj, "border_top_color", "border-top-color");
    emit_color(&mut css, obj, "border_right_color", "border-right-color");
    emit_color(&mut css, obj, "border_bottom_color", "border-bottom-color");
    emit_color(&mut css, obj, "border_left_color", "border-left-color");

    // Border radius (now supports percentages)
    emit_length_percentage(
        &mut css,
        obj,
        "border_radius_top_left",
        "border-top-left-radius",
    );
    emit_length_percentage(
        &mut css,
        obj,
        "border_radius_top_right",
        "border-top-right-radius",
    );
    emit_length_percentage(
        &mut css,
        obj,
        "border_radius_bottom_right",
        "border-bottom-right-radius",
    );
    emit_length_percentage(
        &mut css,
        obj,
        "border_radius_bottom_left",
        "border-bottom-left-radius",
    );

    // Background. `ComputedStyle` has no `background_color` field — it carries a
    // `background: BackgroundValue` enum, serialized as "None",
    // `{"Color": "#rrggbb"}` or a gradient/image variant. Reading the
    // non-existent flat key meant the exported page had no element backgrounds
    // at all, which is most of the ink on a real screen.
    emit_background(&mut css, obj);

    // Text color (already serialized as "#rrggbb" or "#rrggbbaa")
    emit_color(&mut css, obj, "color", "color");

    // Opacity / visibility / stacking
    emit_f32(&mut css, obj, "opacity", "opacity", 1.0);
    emit_visibility(&mut css, obj);
    if let Some(z) = obj.get("z_index").and_then(|v| v.as_i64()) {
        css.push_str(&format!("z-index: {}; ", z));
    }

    // Typography
    if let Some(v) = obj.get("font_size").and_then(|v| v.as_f64())
        && (v - 16.0).abs() > 0.01
    {
        css.push_str(&format!("font-size: {}px; ", v));
    }
    if let Some(v) = obj.get("font_weight").and_then(|v| v.as_f64())
        && (v - 400.0).abs() > 0.01
    {
        css.push_str(&format!("font-weight: {}; ", v as i32));
    }
    if let Some(v) = obj.get("font_family").and_then(|v| v.as_str())
        && !v.is_empty()
    {
        css.push_str(&format!("font-family: {}; ", v));
    }
    emit_font_style(&mut css, obj);
    emit_line_height(&mut css, obj);
    emit_letter_spacing(&mut css, obj);
    emit_word_spacing(&mut css, obj);
    emit_text_align(&mut css, obj);
    emit_text_decoration(&mut css, obj);
    emit_white_space(&mut css, obj);

    css.trim_end().to_string()
}

// Helper functions for each value type

/// Emit a color field. Colors serialize as `"#rrggbb"`/`"#rrggbbaa"`, or `null`
/// when unset — in which case nothing is emitted and the property inherits.
fn emit_color(css: &mut String, obj: &serde_json::Map<String, Value>, key: &str, prop: &str) {
    if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
        css.push_str(&format!("{}: {}; ", prop, v));
    }
}

/// Emit `background` from the `BackgroundValue` enum.
///
/// Serialized shapes: `"None"`, `{"Color": "#rrggbb"}`,
/// `{"LinearGradient": {angle_degrees, stops}}`, `{"RadialGradient": {stops}}`,
/// `{"Image": {url}}`.
fn emit_background(css: &mut String, obj: &serde_json::Map<String, Value>) {
    let Some(v) = obj.get("background") else {
        return;
    };
    // "None" — nothing to paint.
    if v.as_str().is_some() {
        return;
    }
    let Some(o) = v.as_object() else { return };

    if let Some(color) = o.get("Color").and_then(|c| c.as_str()) {
        css.push_str(&format!("background-color: {}; ", color));
        return;
    }
    if let Some(g) = o.get("LinearGradient").and_then(|g| g.as_object()) {
        let angle = g
            .get("angle_degrees")
            .and_then(|a| a.as_f64())
            .unwrap_or(180.0);
        if let Some(stops) = gradient_stops(g.get("stops")) {
            css.push_str(&format!(
                "background-image: linear-gradient({}deg, {}); ",
                angle, stops
            ));
        }
        return;
    }
    if let Some(g) = o.get("RadialGradient").and_then(|g| g.as_object()) {
        if let Some(stops) = gradient_stops(g.get("stops")) {
            css.push_str(&format!("background-image: radial-gradient({}); ", stops));
        }
        return;
    }
    if let Some(img) = o.get("Image").and_then(|i| i.as_object())
        && let Some(url) = img.get("url").and_then(|u| u.as_str())
    {
        // Quote the URL so a path containing ')' or whitespace stays intact.
        css.push_str(&format!(
            "background-image: url(\"{}\"); ",
            url.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
}

/// Render a `Vec<GradientStop>` as a CSS color-stop list, or `None` if empty /
/// any stop is missing a color (a partial gradient would be worse than none).
fn gradient_stops(stops: Option<&Value>) -> Option<String> {
    let stops = stops?.as_array()?;
    if stops.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(stops.len());
    for stop in stops {
        let color = stop.get("color")?.as_str()?;
        let offset = stop.get("offset").and_then(|o| o.as_f64()).unwrap_or(0.0);
        out.push(format!("{} {:.2}%", color, offset * 100.0));
    }
    Some(out.join(", "))
}

/// Emit one side's `border-style` from a `BorderStyleValue`.
fn emit_border_style(
    css: &mut String,
    obj: &serde_json::Map<String, Value>,
    key: &str,
    prop: &str,
) {
    if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
        let css_val = match v {
            "Solid" => "solid",
            "Dashed" => "dashed",
            "Dotted" => "dotted",
            "Double" => "double",
            "Hidden" => "hidden",
            // "None" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("{}: {}; ", prop, css_val));
    }
}

fn emit_visibility(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("visibility").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Hidden" => "hidden",
            "Collapse" => "collapse",
            // "Visible" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("visibility: {}; ", css_val));
    }
}

fn emit_overflow(css: &mut String, obj: &serde_json::Map<String, Value>, key: &str, prop: &str) {
    if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
        let css_val = match v {
            "Hidden" => "hidden",
            "Scroll" => "scroll",
            "Auto" => "auto",
            "Clip" => "clip",
            // "Visible" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("{}: {}; ", prop, css_val));
    }
}

fn emit_dimension(css: &mut String, obj: &serde_json::Map<String, Value>, key: &str, prop: &str) {
    if let Some(v) = obj.get(key) {
        if let Some(s) = v.as_str()
            && s == "Auto"
        {
            return;
        } // Skip auto
        if let Some(o) = v.as_object() {
            if let Some(len) = o.get("Length").and_then(|v| v.as_f64()) {
                css.push_str(&format!("{}: {}px; ", prop, len));
            } else if let Some(pct) = o.get("Percent").and_then(|v| v.as_f64()) {
                css.push_str(&format!("{}: {}%; ", prop, pct * 100.0));
            }
        }
    }
}

fn emit_length_percentage(
    css: &mut String,
    obj: &serde_json::Map<String, Value>,
    key: &str,
    prop: &str,
) {
    if let Some(v) = obj.get(key) {
        if let Some(s) = v.as_str()
            && s == "Zero"
        {
            return;
        } // Skip zero
        if let Some(o) = v.as_object() {
            if let Some(len) = o.get("Length").and_then(|v| v.as_f64()) {
                if len.abs() > 0.01 {
                    css.push_str(&format!("{}: {}px; ", prop, len));
                }
            } else if let Some(pct) = o.get("Percent").and_then(|v| v.as_f64()) {
                css.push_str(&format!("{}: {}%; ", prop, pct * 100.0));
            }
        }
    }
}

fn emit_length_percentage_auto(
    css: &mut String,
    obj: &serde_json::Map<String, Value>,
    key: &str,
    prop: &str,
) {
    if let Some(v) = obj.get(key) {
        if let Some(s) = v.as_str()
            && s == "Auto"
        {
            return;
        } // Skip auto
        if let Some(o) = v.as_object() {
            if let Some(len) = o.get("Length").and_then(|v| v.as_f64()) {
                css.push_str(&format!("{}: {}px; ", prop, len));
            } else if let Some(pct) = o.get("Percent").and_then(|v| v.as_f64()) {
                css.push_str(&format!("{}: {}%; ", prop, pct * 100.0));
            }
        }
    }
}

fn emit_f32(
    css: &mut String,
    obj: &serde_json::Map<String, Value>,
    key: &str,
    prop: &str,
    default: f64,
) {
    if let Some(v) = obj.get(key).and_then(|v| v.as_f64())
        && (v - default).abs() > 0.01
    {
        css.push_str(&format!("{}: {}; ", prop, v));
    }
}

fn emit_flex_direction(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("flex_direction").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Column" => "column",
            "RowReverse" => "row-reverse",
            "ColumnReverse" => "column-reverse",
            // "Row" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("flex-direction: {}; ", css_val));
    }
}

fn emit_flex_wrap(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("flex_wrap").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Wrap" => "wrap",
            "WrapReverse" => "wrap-reverse",
            // "NoWrap" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("flex-wrap: {}; ", css_val));
    }
}

fn emit_align(css: &mut String, obj: &serde_json::Map<String, Value>, key: &str, prop: &str) {
    if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
        let css_val = match v {
            "FlexStart" => "flex-start",
            "FlexEnd" => "flex-end",
            "Center" => "center",
            "Baseline" => "baseline",
            "Stretch" => "stretch",
            _ => return,
        };
        css.push_str(&format!("{}: {}; ", prop, css_val));
    }
}

fn emit_justify(css: &mut String, obj: &serde_json::Map<String, Value>, key: &str, prop: &str) {
    if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
        let css_val = match v {
            "FlexStart" => "flex-start",
            "FlexEnd" => "flex-end",
            "Center" => "center",
            "SpaceBetween" => "space-between",
            "SpaceAround" => "space-around",
            "SpaceEvenly" => "space-evenly",
            _ => return,
        };
        css.push_str(&format!("{}: {}; ", prop, css_val));
    }
}

fn emit_font_style(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("font_style").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Italic" => "italic",
            "Oblique" => "oblique",
            // "Normal" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("font-style: {}; ", css_val));
    }
}

fn emit_line_height(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("line_height") {
        if let Some(s) = v.as_str()
            && s == "Normal"
        {
            return;
        }
        if let Some(o) = v.as_object() {
            if let Some(n) = o.get("Relative").and_then(|v| v.as_f64()) {
                css.push_str(&format!("line-height: {:.2}; ", n));
            } else if let Some(len) = o.get("Absolute").and_then(|v| v.as_f64()) {
                css.push_str(&format!("line-height: {:.2}px; ", len));
            }
        }
    }
}

fn emit_letter_spacing(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("letter_spacing").and_then(|v| v.as_f64())
        && v.abs() > 0.001
    {
        // Only emit if non-zero
        css.push_str(&format!("letter-spacing: {:.2}px; ", v));
    }
}

fn emit_word_spacing(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("word_spacing").and_then(|v| v.as_f64())
        && v.abs() > 0.001
    {
        // Only emit if non-zero
        css.push_str(&format!("word-spacing: {:.2}px; ", v));
    }
}

fn emit_text_align(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("text_align").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Center" => "center",
            "End" => "right",
            "Justify" => "justify",
            // "Start" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("text-align: {}; ", css_val));
    }
}

fn emit_text_decoration(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("text_decoration")
        && let Some(o) = v.as_object()
    {
        let underline = o
            .get("underline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let strikethrough = o
            .get("strikethrough")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if underline && strikethrough {
            css.push_str("text-decoration: underline line-through; ");
        } else if underline {
            css.push_str("text-decoration: underline; ");
        } else if strikethrough {
            css.push_str("text-decoration: line-through; ");
        }
    }
}

fn emit_white_space(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("white_space").and_then(|v| v.as_str()) {
        let css_val = match v {
            "NoWrap" => "nowrap",
            "Pre" => "pre",
            "PreWrap" => "pre-wrap",
            "PreLine" => "pre-line",
            // "Normal" is the default; nothing to emit.
            _ => return,
        };
        css.push_str(&format!("white-space: {}; ", css_val));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_styles() {
        let styles = json!({
            "display": "Flex",
            "flex_direction": "Column",
            "padding_top": {"Length": 16.0},
            // `ComputedStyle` has no flat `background_color` field; it carries a
            // `background: BackgroundValue` enum. Fixtures here must use the
            // shape rinch-dom actually serializes, or they pass while the
            // exporter emits nothing for a real screen.
            "background": {"Color": "#1a1a1a"}
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("display: flex"));
        assert!(css.contains("flex-direction: column"));
        assert!(css.contains("padding-top: 16px"));
        assert!(css.contains("background-color: #1a1a1a"));
    }

    #[test]
    fn test_skip_defaults() {
        // `Static` is the default `PositionValue`, so it is the one to omit.
        let styles = json!({
            "display": "Flex",
            "position": "Static",
            "overflow_x": "Visible",
            "flex_direction": "Row",
            "flex_wrap": "NoWrap",
            "background": "None",
            "visibility": "Visible",
            "border_top_style": "None"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("display: flex"));
        assert!(!css.contains("position"));
        assert!(!css.contains("overflow"));
        assert!(!css.contains("flex-direction"));
        assert!(!css.contains("flex-wrap"));
        assert!(!css.contains("background"));
        assert!(!css.contains("visibility"));
        assert!(!css.contains("border-top-style"));
    }

    /// `position: relative` establishes a containing block, so dropping it (as
    /// the exporter used to, treating `relative` rather than `static` as the
    /// default) re-parents every absolutely-positioned descendant in the
    /// browser reference.
    #[test]
    fn test_relative_position_is_emitted() {
        let css = computed_style_to_css(&json!({"position": "Relative"}));
        assert!(css.contains("position: relative"), "{css}");

        let css = computed_style_to_css(&json!({"position": "Sticky"}));
        assert!(css.contains("position: sticky"), "{css}");
    }

    #[test]
    fn test_border_style_and_color_come_from_their_own_fields() {
        // A resolved non-zero width with `border-style: none` paints nothing in
        // rinch; inferring "solid" from the width alone put a phantom border in
        // the browser reference.
        let css = computed_style_to_css(&json!({
            "border_top_width": {"Length": 1.0},
            "border_top_style": "None",
        }));
        assert!(!css.contains("border-top-style"), "{css}");

        let css = computed_style_to_css(&json!({
            "border_top_width": {"Length": 1.0},
            "border_top_style": "Solid",
            "border_top_color": "#ff0000",
        }));
        assert!(css.contains("border-top-width: 1px"), "{css}");
        assert!(css.contains("border-top-style: solid"), "{css}");
        assert!(css.contains("border-top-color: #ff0000"), "{css}");
    }

    #[test]
    fn test_background_gradient_and_image() {
        let css = computed_style_to_css(&json!({
            "background": {
                "LinearGradient": {
                    "angle_degrees": 90.0,
                    "stops": [
                        {"offset": 0.0, "color": "#000000"},
                        {"offset": 1.0, "color": "#ffffff"}
                    ]
                }
            }
        }));
        assert!(
            css.contains(
                "background-image: linear-gradient(90deg, #000000 0.00%, #ffffff 100.00%)"
            ),
            "{css}"
        );

        let css = computed_style_to_css(&json!({
            "background": {"Image": {"url": "/tmp/a b.png"}}
        }));
        assert!(
            css.contains("background-image: url(\"/tmp/a b.png\")"),
            "{css}"
        );
    }

    #[test]
    fn test_visibility_and_z_index() {
        let css = computed_style_to_css(&json!({"visibility": "Hidden", "z_index": 5}));
        assert!(css.contains("visibility: hidden"), "{css}");
        assert!(css.contains("z-index: 5"), "{css}");
    }

    #[test]
    fn test_dimensions() {
        let styles = json!({
            "width": {"Length": 200.0},
            "height": {"Percent": 0.5},
            "min_width": "Auto"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("width: 200px"));
        assert!(css.contains("height: 50%"));
        assert!(!css.contains("min-width"));
    }

    #[test]
    fn test_colors() {
        let styles = json!({
            "background": {"Color": "#ff5733"},
            "color": "#000000ff",
            "border_left_color": "#00000080"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("background-color: #ff5733"));
        assert!(css.contains("color: #000000ff"));
        assert!(css.contains("border-left-color: #00000080"));
    }

    #[test]
    fn test_typography() {
        let styles = json!({
            "font_size": 20.0,
            "font_weight": 700.0,
            "font_family": "Arial, sans-serif",
            "font_style": "Italic",
            "line_height": {"Relative": 1.5},
            "text_align": "Center"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("font-size: 20px"));
        assert!(css.contains("font-weight: 700"));
        assert!(css.contains("font-family: Arial, sans-serif"));
        assert!(css.contains("font-style: italic"));
        assert!(css.contains("line-height: 1.5"));
        assert!(css.contains("text-align: center"));
    }

    #[test]
    fn test_text_decoration() {
        let styles = json!({
            "text_decoration": {
                "underline": true,
                "strikethrough": false
            }
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("text-decoration: underline"));

        let styles = json!({
            "text_decoration": {
                "underline": true,
                "strikethrough": true
            }
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("text-decoration: underline line-through"));
    }

    #[test]
    fn test_flexbox() {
        let styles = json!({
            "display": "Flex",
            "flex_grow": 1.0,
            "flex_shrink": 0.0,
            "align_items": "Center",
            "justify_content": "SpaceBetween"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("display: flex"));
        assert!(css.contains("flex-grow: 1"));
        assert!(css.contains("flex-shrink: 0"));
        assert!(css.contains("align-items: center"));
        assert!(css.contains("justify-content: space-between"));
    }

    #[test]
    fn test_border_radius() {
        let styles = json!({
            "border_radius_top_left": {"Length": 8.0},
            "border_radius_top_right": {"Length": 8.0},
            "border_radius_bottom_right": "Zero",
            "border_radius_bottom_left": "Zero"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("border-top-left-radius: 8px"));
        assert!(css.contains("border-top-right-radius: 8px"));
        assert!(!css.contains("border-bottom-right-radius"));
        assert!(!css.contains("border-bottom-left-radius"));
    }
}
