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

    // Position
    if let Some(v) = obj.get("position").and_then(|v| v.as_str()) {
        let css_val = match v {
            "Absolute" => "absolute",
            "Fixed" => "fixed",
            "Static" => "static",
            // "Relative" and anything unrecognised.
            _ => "relative",
        };
        if css_val != "relative" {
            // Skip default
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

    // After border widths, add border-style if any border has width
    let has_border = [
        "border_top_width",
        "border_right_width",
        "border_bottom_width",
        "border_left_width",
    ]
    .iter()
    .any(|key| {
        obj.get(*key)
            .and_then(|v| v.as_object())
            .and_then(|o| o.get("Length"))
            .and_then(|v| v.as_f64())
            .map(|len| len.abs() > 0.01)
            .unwrap_or(false)
    });
    if has_border {
        css.push_str("border-style: solid; ");
    }

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

    // Colors (already serialized as "#rrggbb" or "#rrggbbaa")
    if let Some(v) = obj.get("background_color").and_then(|v| v.as_str()) {
        css.push_str(&format!("background-color: {}; ", v));
    }
    if let Some(v) = obj.get("color").and_then(|v| v.as_str()) {
        css.push_str(&format!("color: {}; ", v));
    }
    if let Some(v) = obj.get("border_color").and_then(|v| v.as_str()) {
        css.push_str(&format!("border-color: {}; ", v));
    }

    // Opacity
    emit_f32(&mut css, obj, "opacity", "opacity", 1.0);

    // Typography
    if let Some(v) = obj.get("font_size").and_then(|v| v.as_f64()) {
        if (v - 16.0).abs() > 0.01 {
            css.push_str(&format!("font-size: {}px; ", v));
        }
    }
    if let Some(v) = obj.get("font_weight").and_then(|v| v.as_f64()) {
        if (v - 400.0).abs() > 0.01 {
            css.push_str(&format!("font-weight: {}; ", v as i32));
        }
    }
    if let Some(v) = obj.get("font_family").and_then(|v| v.as_str()) {
        if !v.is_empty() {
            css.push_str(&format!("font-family: {}; ", v));
        }
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
        if let Some(s) = v.as_str() {
            if s == "Auto" {
                return;
            } // Skip auto
        }
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
        if let Some(s) = v.as_str() {
            if s == "Zero" {
                return;
            } // Skip zero
        }
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
        if let Some(s) = v.as_str() {
            if s == "Auto" {
                return;
            } // Skip auto
        }
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
    if let Some(v) = obj.get(key).and_then(|v| v.as_f64()) {
        if (v - default).abs() > 0.01 {
            css.push_str(&format!("{}: {}; ", prop, v));
        }
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
        if let Some(s) = v.as_str() {
            if s == "Normal" {
                return;
            }
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
    if let Some(v) = obj.get("letter_spacing").and_then(|v| v.as_f64()) {
        if v.abs() > 0.001 {
            // Only emit if non-zero
            css.push_str(&format!("letter-spacing: {:.2}px; ", v));
        }
    }
}

fn emit_word_spacing(css: &mut String, obj: &serde_json::Map<String, Value>) {
    if let Some(v) = obj.get("word_spacing").and_then(|v| v.as_f64()) {
        if v.abs() > 0.001 {
            // Only emit if non-zero
            css.push_str(&format!("word-spacing: {:.2}px; ", v));
        }
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
    if let Some(v) = obj.get("text_decoration") {
        if let Some(o) = v.as_object() {
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
            "background_color": "#1a1a1a"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("display: flex"));
        assert!(css.contains("flex-direction: column"));
        assert!(css.contains("padding-top: 16px"));
        assert!(css.contains("background-color: #1a1a1a"));
    }

    #[test]
    fn test_skip_defaults() {
        let styles = json!({
            "display": "Flex",
            "position": "Relative",
            "overflow_x": "Visible",
            "flex_direction": "Row",
            "flex_wrap": "NoWrap"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("display: flex"));
        assert!(!css.contains("position"));
        assert!(!css.contains("overflow"));
        assert!(!css.contains("flex-direction"));
        assert!(!css.contains("flex-wrap"));
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
            "background_color": "#ff5733",
            "color": "#000000ff",
            "border_color": "#00000080"
        });
        let css = computed_style_to_css(&styles);
        assert!(css.contains("background-color: #ff5733"));
        assert!(css.contains("color: #000000ff"));
        assert!(css.contains("border-color: #00000080"));
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
