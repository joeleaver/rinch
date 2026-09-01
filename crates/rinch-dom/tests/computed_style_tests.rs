use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{
    BackgroundValue, BorderStyleValue, CursorValue, DisplayValue, OverflowValue,
    PointerEventsValue, PositionValue, TextOverflowValue, VisibilityValue, WhiteSpaceValue,
};

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
        matches!(node.computed_style.pointer_events, PointerEventsValue::Auto),
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
        matches!(node.computed_style.pointer_events, PointerEventsValue::None),
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
    doc.set_attribute(div, "style", "cursor: pointer; width: 100px; height: 100px");
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
        matches!(node.computed_style.border_top_style, BorderStyleValue::None),
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
        matches!(node.computed_style.outline_style, BorderStyleValue::Solid),
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

/// The four coefficients `TransformValue` carries for percentage translates
/// are accumulated in the frame each translate actually runs in (#212), so a
/// regression names the accumulation rather than the composition downstream.
#[test]
fn test_transform_percentage_translate_coefficients() {
    // (declaration, expected pct_translate_w, expected pct_translate_h)
    let cases: [(&str, [f64; 2], [f64; 2]); 4] = [
        // Leading translate: the frame is the identity, so the coefficients
        // are just the fractions. This is what every shipped component does.
        ("translate(-50%, -50%)", [-0.5, 0.0], [0.0, -0.5]),
        // Behind a scale, the x fraction picks up the scale.
        ("scale(2) translateX(50%)", [1.0, 0.0], [0.0, 0.0]),
        // Behind a 90° rotation, `translateX` moves along the *y* axis, so its
        // contribution lands in the second coefficient.
        ("rotate(90deg) translateX(50%)", [0.0, 0.5], [0.0, 0.0]),
        // Two translates in different frames: the second is rotated, the
        // first is not.
        (
            "translateX(50%) rotate(90deg) translateY(50%)",
            [0.5, 0.0],
            [-0.5, 0.0],
        ),
    ];

    for (decl, want_w, want_h) in cases {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            &format!("width: 100px; height: 40px; transform: {decl}"),
        );
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);

        let tf = &doc.tree.get(div.0).unwrap().computed_style.transform;
        assert!(
            !tf.is_identity,
            "{decl}: a percentage translate is not a no-op"
        );
        for i in 0..2 {
            assert!(
                (tf.pct_translate_w[i] - want_w[i]).abs() < 1e-9,
                "{decl}: pct_translate_w = {:?}, expected {want_w:?}",
                tf.pct_translate_w
            );
            assert!(
                (tf.pct_translate_h[i] - want_h[i]).abs() < 1e-9,
                "{decl}: pct_translate_h = {:?}, expected {want_h:?}",
                tf.pct_translate_h
            );
        }
    }
}

/// `translate3d()` is the 2D translate with a z that flattening drops — not the
/// identity it used to fall through to, and its percentage part accumulates
/// like any other translate's (#212, #405).
#[test]
fn test_transform_translate3d_is_not_identity() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 100px; height: 40px; transform: scale(2) translate3d(50%, 10px, 5px)",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let tf = &doc.tree.get(div.0).unwrap().computed_style.transform;
    assert!(!tf.is_identity, "translate3d must not flatten to identity");
    assert!(
        (tf.pct_translate_w[0] - 1.0).abs() < 1e-9,
        "the 50% must be doubled by the preceding scale, got {:?}",
        tf.pct_translate_w
    );
    // The px part rides the matrix and is doubled by the scale like any other
    // length: e = 2 * 0, f = 2 * 10.
    assert!(
        tf.matrix[4].abs() < 1e-9 && (tf.matrix[5] - 20.0).abs() < 1e-9,
        "translate3d's px part should compose normally, got {:?}",
        tf.matrix
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

// ===== Text Overflow Tests =====

#[test]
fn test_text_overflow_ellipsis() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "overflow: hidden; white-space: nowrap; text-overflow: ellipsis; width: 200px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert_eq!(
        node.computed_style.text_overflow,
        TextOverflowValue::Ellipsis,
        "text_overflow should be Ellipsis, got {:?}",
        node.computed_style.text_overflow
    );
    assert_eq!(
        node.computed_style.white_space,
        WhiteSpaceValue::NoWrap,
        "white_space should be NoWrap, got {:?}",
        node.computed_style.white_space
    );
    assert!(
        matches!(node.computed_style.overflow_x, OverflowValue::Hidden),
        "overflow_x should be Hidden, got {:?}",
        node.computed_style.overflow_x
    );
}

#[test]
fn test_text_overflow_default_is_clip() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 200px");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let node = doc.tree.get(div.0).unwrap();
    assert_eq!(
        node.computed_style.text_overflow,
        TextOverflowValue::Clip,
        "default text_overflow should be Clip"
    );
}

// ===== Tag-selector matching (stylo local_name regression) =====

#[test]
fn tag_selector_matches_table_cells_and_custom_tags() {
    // Regression: stylo's `local_name()` returned an interned atom only for a
    // hardcoded tag list; unlisted tags (`td`/`tr`/`th`, custom elements) fell back
    // to the empty atom and silently failed every type selector. Now any tag is
    // interned on demand, so bare tag selectors match it.
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let style = doc.create_element("style");
    let css = doc.create_text("td { display: flex; } widget-x { display: flex; }");
    doc.append_child(style, css);
    doc.append_child(body, style);

    let td = doc.create_element("td");
    doc.append_child(body, td);
    let widget = doc.create_element("widget-x");
    doc.append_child(body, widget);

    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(td.0).unwrap().computed_style.display,
        DisplayValue::Flex,
        "a `td` tag selector now matches (was silently dropped)"
    );
    assert_eq!(
        doc.tree.get(widget.0).unwrap().computed_style.display,
        DisplayValue::Flex,
        "an unlisted custom-element tag selector matches via dynamic interning"
    );
}

/// Removing a selector-affecting attribute must re-resolve styles (symmetric with
/// setting one — issue #67). A `[data-x]` rule must stop applying once the
/// attribute is gone (e.g. a popup option losing its highlight).
#[test]
fn removing_attribute_restyles_the_node() {
    let blue = |bg: &BackgroundValue| {
        matches!(bg, BackgroundValue::Color(c)
            if c.components[2] > 0.9 && c.components[0] < 0.1)
    };
    let mut doc = RinchDocument::new();
    doc.load_css(
        "div { background-color: rgb(255, 255, 255); } \
         div[data-hl] { background-color: rgb(0, 0, 255); }",
    );
    let body = doc.body();
    let el = doc.create_element("div");
    doc.set_attribute(el, "data-hl", "");
    doc.append_child(body, el);
    doc.resolve_layout(400.0, 300.0);
    assert!(
        blue(&doc.tree.get(el.0).unwrap().computed_style.background),
        "with [data-hl] the option is blue"
    );

    doc.remove_attribute(el, "data-hl");
    doc.resolve_layout(400.0, 300.0);
    assert!(
        !blue(&doc.tree.get(el.0).unwrap().computed_style.background),
        "removing [data-hl] must drop the highlight background"
    );
}

/// #228: a `:focus-visible` rule applies only while the keyboard focus ring
/// flag is set — pointer-driven `:focus` alone must not match it, and clearing
/// the flag must drop the rule again.
#[test]
fn focus_visible_rule_needs_the_keyboard_flag() {
    let mut doc = RinchDocument::new();
    doc.load_css("div:focus-visible { outline-width: 3px; outline-style: solid; }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.set_attribute(div, "tabindex", "0");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let outline = |doc: &RinchDocument| doc.tree.get(div.0).unwrap().computed_style.outline_width;
    assert_eq!(outline(&doc), 0.0, "unfocused: no ring");

    // Pointer-driven focus: :focus only.
    doc.update_focus(Some(div.0));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        outline(&doc),
        0.0,
        ":focus alone must not match :focus-visible"
    );

    // Keyboard-driven focus sets the ring flag.
    doc.set_focus_visible(div.0, true);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        outline(&doc),
        3.0,
        "keyboard focus must match :focus-visible"
    );

    // Pointer interaction clears it again.
    doc.set_focus_visible(div.0, false);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(outline(&doc), 0.0, "clearing the flag must drop the ring");

    // And losing focus clears the flag itself.
    doc.set_focus_visible(div.0, true);
    doc.update_focus(None);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(outline(&doc), 0.0, "blur must drop the ring with the focus");
    assert!(
        !doc.tree.get(div.0).unwrap().is_focus_visible,
        "update_focus(None) must clear is_focus_visible on the old node"
    );
}

/// The theme's focus ring is a *bare* `:focus-visible { ... }` rule — stylo
/// buckets it into a state-gated `rare_pseudo_classes` map that is never
/// consulted for an unfocused node, so `focus_sensitive` cannot be set ahead
/// of the first focus. The `has_bare_focus_rules` fallback must invalidate
/// anyway or the default theme's ring never paints on the first Tab. (The
/// test above sidesteps this with `div:focus-visible`, which buckets by tag.)
#[test]
fn bare_focus_visible_rule_paints_on_first_focus() {
    let mut doc = RinchDocument::new();
    doc.load_css(":focus-visible { outline-width: 2px; outline-style: solid; }");
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 100px");
    doc.set_attribute(div, "tabindex", "0");
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let outline = |doc: &RinchDocument| doc.tree.get(div.0).unwrap().computed_style.outline_width;
    assert_eq!(outline(&doc), 0.0, "unfocused: no ring");

    // First keyboard focus this node has ever seen: with the bare rule in a
    // state-gated bucket, only the has_bare_focus_rules fallback invalidates.
    doc.update_focus(Some(div.0));
    doc.set_focus_visible(div.0, true);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        outline(&doc),
        2.0,
        "a bare :focus-visible ring must paint on the FIRST focus"
    );

    doc.update_focus(None);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(outline(&doc), 0.0, "blur drops the bare ring");
}

// ===== #250: inline-style colours go through stylo (regression guard) =====

/// #250 was reported as "a named colour outside rinch-dom's legacy dozen (or an
/// `hsl()`) paints blank from an inline style". It does not: `style=""` is
/// compiled by stylo's full CSS Color 4 parser, not by `layout::parse_color`.
/// This pins that so the legacy parser's gaps can never leak back in here.
#[test]
fn inline_style_named_and_hsl_colours_resolve_via_stylo() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let named = doc.create_element("div");
    doc.set_attribute(
        named,
        "style",
        "background-color: rebeccapurple; width: 10px; height: 10px",
    );
    doc.append_child(body, named);

    let hsl = doc.create_element("div");
    doc.set_attribute(
        hsl,
        "style",
        "background-color: hsl(270 50% 40%); width: 10px; height: 10px",
    );
    doc.append_child(body, hsl);

    doc.resolve_layout(800.0, 600.0);

    for (id, authored) in [(named, "rebeccapurple"), (hsl, "hsl(270 50% 40%)")] {
        let node = doc.tree.get(id.0).unwrap();
        let BackgroundValue::Color(color) = node.computed_style.background else {
            panic!("`background-color: {authored}` should resolve to a solid colour");
        };
        let rgba = color.to_rgba8();
        assert_eq!(
            (rgba.r, rgba.g, rgba.b, rgba.a),
            (102, 51, 153, 255),
            "`background-color: {authored}` should compute to #663399"
        );
    }
}
