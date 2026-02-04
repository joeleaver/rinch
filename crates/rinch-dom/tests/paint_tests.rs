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
