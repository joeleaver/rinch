use rinch_core::dom::DomDocument;
use rinch_dom::computed_style::{
    BackgroundValue, BorderStyleValue, CursorValue, PointerEventsValue, PositionValue,
    VisibilityValue,
};
use rinch_dom::RinchDocument;

// Helper to check approximate float equality
fn approx_eq(a: f32, b: f32, epsilon: f32) -> bool {
    (a - b).abs() < epsilon
}

// ===== Visibility Tests =====

#[test]
fn test_visibility_default() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.visibility, VisibilityValue::Visible),
        "default visibility should be Visible"
    );
}

#[test]
fn test_visibility_hidden() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "visibility: hidden; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.visibility, VisibilityValue::Hidden),
        "visibility should be Hidden"
    );
}

#[test]
fn test_visibility_collapse() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "visibility: collapse; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.visibility, VisibilityValue::Collapse),
        "visibility should be Collapse"
    );
}

// ===== Pointer-events Tests =====

#[test]
fn test_pointer_events_default() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.pointer_events,
            PointerEventsValue::Auto
        ),
        "default pointer-events should be Auto"
    );
}

#[test]
fn test_pointer_events_none() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "pointer-events: none; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.pointer_events,
            PointerEventsValue::None
        ),
        "pointer-events should be None"
    );
}

// ===== Cursor Tests =====

#[test]
fn test_cursor_default() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.cursor, CursorValue::Auto),
        "default cursor should be Auto"
    );
}

#[test]
fn test_cursor_pointer() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "cursor: pointer; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.cursor, CursorValue::Pointer),
        "cursor should be Pointer"
    );
}

#[test]
fn test_cursor_text() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "cursor: text; width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.cursor, CursorValue::Text),
        "cursor should be Text"
    );
}

// ===== Border Colors Tests =====

#[test]
fn test_border_color_uniform() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 2px solid red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        node.computed_style.border_top_color.is_some(),
        "border_top_color should be set"
    );
    assert!(
        node.computed_style.border_right_color.is_some(),
        "border_right_color should be set"
    );
    assert!(
        node.computed_style.border_bottom_color.is_some(),
        "border_bottom_color should be set"
    );
    assert!(
        node.computed_style.border_left_color.is_some(),
        "border_left_color should be set"
    );

    // Check that all border colors are red (approximately)
    let top_rgba = node.computed_style.border_top_color.unwrap().to_rgba8();
    assert_eq!(top_rgba.r, 255, "border-top should be red");
    assert_eq!(top_rgba.g, 0, "border-top should be red");
    assert_eq!(top_rgba.b, 0, "border-top should be red");
}

#[test]
fn test_border_color_per_side() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border-style: solid; border-width: 2px; border-top-color: red; border-bottom-color: blue; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();

    // Check top is red
    let top_rgba = node
        .computed_style
        .border_top_color
        .expect("border_top_color should be set")
        .to_rgba8();
    assert_eq!(top_rgba.r, 255, "border-top should be red");
    assert_eq!(top_rgba.g, 0, "border-top should be red");
    assert_eq!(top_rgba.b, 0, "border-top should be red");

    // Check bottom is blue
    let bottom_rgba = node
        .computed_style
        .border_bottom_color
        .expect("border_bottom_color should be set")
        .to_rgba8();
    assert_eq!(bottom_rgba.r, 0, "border-bottom should be blue");
    assert_eq!(bottom_rgba.g, 0, "border-bottom should be blue");
    assert_eq!(bottom_rgba.b, 255, "border-bottom should be blue");
}

// ===== Border Styles Tests =====

#[test]
fn test_border_style_solid() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 2px solid red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.border_top_style,
            BorderStyleValue::Solid
        ),
        "border-top-style should be Solid"
    );
    assert!(
        matches!(
            node.computed_style.border_right_style,
            BorderStyleValue::Solid
        ),
        "border-right-style should be Solid"
    );
}

#[test]
fn test_border_style_dashed() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 2px dashed red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.border_top_style,
            BorderStyleValue::Dashed
        ),
        "border-top-style should be Dashed"
    );
}

#[test]
fn test_border_style_dotted() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 2px dotted red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.border_top_style,
            BorderStyleValue::Dotted
        ),
        "border-top-style should be Dotted"
    );
}

#[test]
fn test_border_style_none() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.border_top_style,
            BorderStyleValue::None
        ),
        "default border-top-style should be None"
    );
}

// ===== Outline Tests =====

#[test]
fn test_outline_width_and_color() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "outline: 3px solid red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        approx_eq(node.computed_style.outline_width, 3.0, 0.01),
        "outline_width should be 3.0, got {}",
        node.computed_style.outline_width
    );
    assert!(
        node.computed_style.outline_color.is_some(),
        "outline_color should be set"
    );
    assert!(
        matches!(
            node.computed_style.outline_style,
            BorderStyleValue::Solid
        ),
        "outline_style should be Solid"
    );

    let rgba = node.computed_style.outline_color.unwrap().to_rgba8();
    assert_eq!(rgba.r, 255, "outline color should be red");
}

#[test]
fn test_outline_offset() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "outline: 1px solid black; outline-offset: 5px; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        approx_eq(node.computed_style.outline_offset, 5.0, 0.01),
        "outline_offset should be 5.0, got {}",
        node.computed_style.outline_offset
    );
}

// ===== Z-index Tests =====

#[test]
fn test_z_index_auto() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        node.computed_style.z_index.is_none(),
        "default z_index should be None (auto)"
    );
}

#[test]
fn test_z_index_positive() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "z-index: 10; position: relative; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert_eq!(
        node.computed_style.z_index,
        Some(10),
        "z_index should be Some(10)"
    );
}

#[test]
fn test_z_index_negative() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "z-index: -5; position: relative; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert_eq!(
        node.computed_style.z_index,
        Some(-5),
        "z_index should be Some(-5)"
    );
}

// ===== Transform Tests =====

#[test]
fn test_transform_none() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        node.computed_style.transform.is_identity,
        "default transform should be identity"
    );
}

#[test]
fn test_transform_rotate() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "transform: rotate(45deg); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        !node.computed_style.transform.is_identity,
        "transform should not be identity after rotate"
    );
}

#[test]
fn test_transform_scale() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "transform: scale(2); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        !node.computed_style.transform.is_identity,
        "transform should not be identity after scale"
    );
    // Check that matrix[0] (scale_x) is approximately 2.0
    assert!(
        approx_eq(node.computed_style.transform.matrix[0] as f32, 2.0, 0.01),
        "transform matrix[0] should be approximately 2.0, got {}",
        node.computed_style.transform.matrix[0]
    );
    assert!(
        approx_eq(node.computed_style.transform.matrix[3] as f32, 2.0, 0.01),
        "transform matrix[3] should be approximately 2.0, got {}",
        node.computed_style.transform.matrix[3]
    );
}

// ===== Background Gradient Tests =====

#[test]
fn test_background_solid_color() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-color: red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.background, BackgroundValue::Color(_)),
        "background should be Color variant"
    );

    if let BackgroundValue::Color(color) = node.computed_style.background {
        let rgba = color.to_rgba8();
        assert_eq!(rgba.r, 255, "background color should be red");
        assert_eq!(rgba.g, 0, "background color should be red");
        assert_eq!(rgba.b, 0, "background color should be red");
    }
}

#[test]
fn test_background_linear_gradient() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background: linear-gradient(to right, red, blue); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.background,
            BackgroundValue::LinearGradient { .. }
        ),
        "background should be LinearGradient variant"
    );

    if let BackgroundValue::LinearGradient { stops, .. } = &node.computed_style.background {
        assert!(
            stops.len() >= 2,
            "linear gradient should have at least 2 stops"
        );
    }
}

#[test]
fn test_background_radial_gradient() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background: radial-gradient(red, blue); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(
            node.computed_style.background,
            BackgroundValue::RadialGradient { .. }
        ),
        "background should be RadialGradient variant"
    );

    if let BackgroundValue::RadialGradient { stops } = &node.computed_style.background {
        assert!(
            stops.len() >= 2,
            "radial gradient should have at least 2 stops"
        );
    }
}

// ===== Text Shadow Tests =====

#[test]
fn test_text_shadow_none() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        node.computed_style.text_shadow.is_empty(),
        "default text_shadow should be empty vec"
    );
}

#[test]
fn test_text_shadow_single() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "text-shadow: 2px 2px 4px black; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert_eq!(
        node.computed_style.text_shadow.len(),
        1,
        "text_shadow should have 1 entry"
    );

    let shadow = &node.computed_style.text_shadow[0];
    assert!(
        approx_eq(shadow.offset_x, 2.0, 0.01),
        "shadow offset_x should be 2.0, got {}",
        shadow.offset_x
    );
    assert!(
        approx_eq(shadow.offset_y, 2.0, 0.01),
        "shadow offset_y should be 2.0, got {}",
        shadow.offset_y
    );
    assert!(
        approx_eq(shadow.blur_radius, 4.0, 0.01),
        "shadow blur_radius should be 4.0, got {}",
        shadow.blur_radius
    );
}

// ===== Position Tests =====

#[test]
fn test_position_sticky() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "position: sticky; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.position, PositionValue::Sticky),
        "position should be Sticky"
    );
}

#[test]
fn test_position_static_default() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        matches!(node.computed_style.position, PositionValue::Static),
        "default position should be Static"
    );
}

// ===== Filter Tests =====

#[test]
fn test_filter_brightness() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "filter: brightness(0.5); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        approx_eq(node.computed_style.filter_brightness, 0.5, 0.01),
        "filter_brightness should be approximately 0.5, got {}",
        node.computed_style.filter_brightness
    );
}

#[test]
fn test_filter_grayscale() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "filter: grayscale(100%); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert!(
        approx_eq(node.computed_style.filter_grayscale, 1.0, 0.01),
        "filter_grayscale should be approximately 1.0, got {}",
        node.computed_style.filter_grayscale
    );
}
