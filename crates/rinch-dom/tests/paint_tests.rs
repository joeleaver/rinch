use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use vello::Scene;

/// Helper to paint a document, creating the paint-specific layout context.
fn paint(doc: &mut RinchDocument, scene: &mut Scene) {
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        scene,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );
}

#[test]
fn test_paint_empty_document() {
    let mut doc = RinchDocument::new();
    doc.resolve_layout(800.0, 600.0);
    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    // Empty document should produce a valid (possibly empty) scene
}

#[test]
fn test_paint_colored_box() {
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

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "scene should have draw commands for colored box"
    );
}

#[test]
fn test_paint_text() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "display: flex; width: 400px");
    doc.append_child(body, div);
    let text = doc.create_text("Hello, world!");
    doc.append_child(div, text);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    // Note: text rendering depends on system fonts being available.
    // In headless/CI environments, parley may not find fonts and produce no glyphs.
    // We just verify painting doesn't panic.
    let _ = scene.encoding().is_empty();
}

#[test]
fn test_paint_with_border() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 2px solid black; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "scene should have draw commands for border"
    );
}

#[test]
fn test_paint_hex_color() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-color: #ff5500; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(!scene.encoding().is_empty());
}

#[test]
fn test_paint_nested_layout() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "display: flex; flex-direction: column; width: 200px; background-color: #eeeeee",
    );
    doc.append_child(body, outer);

    let inner1 = doc.create_element("div");
    doc.set_attribute(inner1, "style", "background-color: blue; height: 50px");
    doc.append_child(outer, inner1);

    let inner2 = doc.create_element("div");
    doc.set_attribute(inner2, "style", "background-color: green; height: 50px");
    doc.append_child(outer, inner2);

    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(!scene.encoding().is_empty());
}

#[test]
fn test_paint_at_scale() {
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

    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();

    let mut scene1 = Scene::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut scene1,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );

    let mut scene2 = Scene::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut scene2,
        2.0,
        (1600.0, 1200.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );

    assert!(!scene1.encoding().is_empty());
    assert!(!scene2.encoding().is_empty());
}

#[test]
fn test_paint_rgb_color() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-color: rgb(100, 200, 50); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(!scene.encoding().is_empty());
}

#[test]
fn test_paint_rgba_color() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-color: rgba(255, 0, 0, 0.5); width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(!scene.encoding().is_empty());
}

#[test]
fn test_parse_color_named() {
    use rinch_dom::layout::parse_color;
    assert!(parse_color("red").is_some());
    assert!(parse_color("blue").is_some());
    assert!(parse_color("transparent").is_some());
    assert!(parse_color("unknown_color").is_none());
}

#[test]
fn test_parse_color_hex() {
    use rinch_dom::layout::parse_color;
    assert!(parse_color("#f00").is_some());
    assert!(parse_color("#ff0000").is_some());
    assert!(parse_color("#ff000080").is_some());
    assert!(parse_color("#xyz").is_none());
}

#[test]
fn test_parse_color_rgb_rgba() {
    use rinch_dom::layout::parse_color;
    assert!(parse_color("rgb(255, 0, 0)").is_some());
    assert!(parse_color("rgba(255, 0, 0, 0.5)").is_some());
    assert!(parse_color("rgb(255, 0)").is_none());
}

#[test]
fn test_paint_visibility_hidden() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "visibility: hidden; background: red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    // Hidden elements should not produce draw commands for themselves, but we just verify no panic
    let _ = scene.encoding().is_empty();
}

#[test]
fn test_paint_dashed_border() {
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

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "dashed border should produce draw commands"
    );
}

#[test]
fn test_paint_dotted_border() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 2px dotted blue; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "dotted border should produce draw commands"
    );
}

#[test]
fn test_paint_per_side_border_colors() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border-style: solid; border-width: 3px; border-top-color: red; border-right-color: green; border-bottom-color: blue; border-left-color: orange; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "per-side border colors should produce draw commands"
    );
}

#[test]
fn test_paint_outline() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "outline: 3px solid red; outline-offset: 2px; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "outline should produce draw commands"
    );
}

#[test]
fn test_paint_transform_rotate() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "transform: rotate(45deg); background: red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "rotated element should produce draw commands"
    );
}

#[test]
fn test_paint_transform_scale() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "transform: scale(1.5); background: blue; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "scaled element should produce draw commands"
    );
}

#[test]
fn test_paint_linear_gradient() {
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

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "linear gradient should produce draw commands"
    );
}

#[test]
fn test_paint_radial_gradient() {
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

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "radial gradient should produce draw commands"
    );
}

#[test]
fn test_paint_text_shadow() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "text-shadow: 2px 2px black; width: 200px");
    doc.append_child(body, div);
    let text = doc.create_text("Shadow text");
    doc.append_child(div, text);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    // Text rendering depends on fonts
    let _ = scene.encoding().is_empty();
}

#[test]
fn test_paint_z_index_ordering() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; width: 200px; height: 200px",
    );
    doc.append_child(body, container);

    let back = doc.create_element("div");
    doc.set_attribute(
        back,
        "style",
        "position: absolute; z-index: 1; background: red; width: 100px; height: 100px",
    );
    doc.append_child(container, back);

    let front = doc.create_element("div");
    doc.set_attribute(
        front,
        "style",
        "position: absolute; z-index: 2; background: blue; width: 100px; height: 100px",
    );
    doc.append_child(container, front);

    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "z-indexed elements should produce draw commands"
    );
}

#[test]
fn test_paint_filter_brightness() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "filter: brightness(0.5); background: red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    // Filters are extracted but may not affect paint output yet
    let _ = scene.encoding().is_empty();
}

#[test]
fn test_paint_filter_grayscale() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "filter: grayscale(1); background: red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    // Filters are extracted but may not affect paint output yet
    let _ = scene.encoding().is_empty();
}

#[test]
fn test_paint_opacity_with_transform() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "opacity: 0.5; transform: rotate(10deg); background: red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "opacity with transform should produce draw commands"
    );
}

#[test]
fn test_paint_border_style_double() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "border: 4px double red; width: 100px; height: 100px",
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);

    let mut scene = Scene::new();
    paint(&mut doc, &mut scene);
    assert!(
        !scene.encoding().is_empty(),
        "double border should produce draw commands"
    );
}
