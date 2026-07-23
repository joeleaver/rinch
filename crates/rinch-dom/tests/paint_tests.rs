use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

/// Helper to paint a document, creating the paint-specific layout context.
fn paint(doc: &mut RinchDocument, painter: &mut VelloPainter) {
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
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
    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    // Note: text rendering depends on system fonts being available.
    // In headless/CI environments, parley may not find fonts and produce no glyphs.
    // We just verify painting doesn't panic.
    let _ = painter.scene().encoding().is_empty();
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(!painter.scene().encoding().is_empty());
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(!painter.scene().encoding().is_empty());
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

    let mut painter1 = VelloPainter::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter1,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );

    let mut painter2 = VelloPainter::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter2,
        2.0,
        (1600.0, 1200.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );

    assert!(!painter1.scene().encoding().is_empty());
    assert!(!painter2.scene().encoding().is_empty());
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(!painter.scene().encoding().is_empty());
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(!painter.scene().encoding().is_empty());
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    // Hidden elements should not produce draw commands for themselves, but we just verify no panic
    let _ = painter.scene().encoding().is_empty();
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    // Text rendering depends on fonts
    let _ = painter.scene().encoding().is_empty();
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    // Filters are extracted but may not affect paint output yet
    let _ = painter.scene().encoding().is_empty();
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    // Filters are extracted but may not affect paint output yet
    let _ = painter.scene().encoding().is_empty();
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
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

    let mut painter = VelloPainter::new();
    paint(&mut doc, &mut painter);
    assert!(
        !painter.scene().encoding().is_empty(),
        "double border should produce draw commands"
    );
}

// ── Transform-aware paint: zero-dimension boxes (#142) and dirty-region
// culling/tracking (#143). Pixel assertions use the software renderer.

#[cfg(feature = "software-renderer")]
mod transform_paint {
    use super::*;
    use peniko::kurbo::Rect;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Paint the document with the tiny-skia software painter for pixel
    /// assertions.
    fn paint_skia(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
        let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            painter,
            1.0,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut paint_layout_cx,
        );
    }

    /// Premultiplied RGBA pixel at (x, y).
    fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
    }

    /// Whether any pixel in [x0, x1) × [y0, y1) satisfies the predicate.
    fn any_pixel(
        painter: &TinySkiaPainter,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        pred: impl Fn([u8; 4]) -> bool,
    ) -> bool {
        for y in y0..y1.min(painter.height()) {
            for x in x0..x1.min(painter.width()) {
                if pred(pixel_at(painter, x, y)) {
                    return true;
                }
            }
        }
        false
    }

    fn is_opaque_red(p: [u8; 4]) -> bool {
        p[0] > 200 && p[1] < 50 && p[2] < 50 && p[3] > 200
    }

    /// #142: a container collapsed to zero height must still apply its own
    /// CSS transform to absolutely-positioned children.
    #[test]
    fn test_zero_height_container_applies_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "position: relative; width: 100px; height: 0; transform: translateY(50px)",
        );
        doc.append_child(body, container);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "position: absolute; top: 0; left: 0; width: 20px; height: 20px; \
             background-color: red",
        );
        doc.append_child(container, child);
        doc.resolve_layout(800.0, 600.0);

        let mut painter = TinySkiaPainter::new(200, 200);
        paint_skia(&mut doc, &mut painter);

        assert!(
            any_pixel(&painter, 0, 40, 60, 90, is_opaque_red),
            "child should paint at the translateY(50px) position"
        );
        assert!(
            !any_pixel(&painter, 0, 0, 60, 35, is_opaque_red),
            "child should not paint at the untransformed position"
        );
    }

    /// #142: a container collapsed to zero height must still apply its own
    /// opacity to absolutely-positioned children.
    #[test]
    fn test_zero_height_container_applies_opacity() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "position: relative; width: 100px; height: 0; \
             transform: translateY(50px); opacity: 0.5",
        );
        doc.append_child(body, container);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "position: absolute; top: 0; left: 0; width: 20px; height: 20px; \
             background-color: red",
        );
        doc.append_child(container, child);
        doc.resolve_layout(800.0, 600.0);

        let mut painter = TinySkiaPainter::new(200, 200);
        paint_skia(&mut doc, &mut painter);

        // Red at 0.5 opacity over a transparent background: premultiplied
        // r ≈ a ≈ 128, definitely not opaque.
        assert!(
            any_pixel(&painter, 0, 40, 60, 90, |p| {
                p[0] > 80 && p[0] < 180 && p[1] < 50 && p[3] > 80 && p[3] < 180
            }),
            "child should paint alpha-blended at the transformed position"
        );
        assert!(
            !any_pixel(&painter, 0, 0, 200, 200, is_opaque_red),
            "no fully opaque red anywhere — opacity must apply"
        );
    }

    /// #142 (offset half): children of a collapsed box must paint relative to
    /// the box's own origin, not its parent's.
    #[test]
    fn test_zero_height_container_uses_own_origin() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // A 100px-tall sibling pushes the collapsed container down to y=100.
        let sibling = doc.create_element("div");
        doc.set_attribute(sibling, "style", "width: 100px; height: 100px");
        doc.append_child(body, sibling);
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "position: relative; width: 100px; height: 0",
        );
        doc.append_child(body, container);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "position: absolute; top: 0; left: 0; width: 20px; height: 20px; \
             background-color: red",
        );
        doc.append_child(container, child);
        doc.resolve_layout(800.0, 600.0);

        let mut painter = TinySkiaPainter::new(200, 200);
        paint_skia(&mut doc, &mut painter);

        assert!(
            any_pixel(&painter, 0, 90, 60, 140, is_opaque_red),
            "child should paint below the 100px sibling"
        );
        assert!(
            !any_pixel(&painter, 0, 0, 60, 60, is_opaque_red),
            "child should not paint at the parent's origin"
        );
    }

    /// #143 (cull half): a node whose transformed position intersects the
    /// dirty region must not be culled against its untransformed layout rect.
    #[test]
    fn test_dirty_region_cull_uses_transformed_position() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            "width: 50px; height: 50px; background-color: red; \
             transform: translateY(200px)",
        );
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);

        // Dirty region covers only the translated position (y 190..300).
        rinch_dom::paint::set_dirty_region(Some(Rect::new(0.0, 190.0, 300.0, 300.0)));
        let mut painter = TinySkiaPainter::new(300, 300);
        paint_skia(&mut doc, &mut painter);
        rinch_dom::paint::set_dirty_region(None);

        assert!(
            any_pixel(&painter, 0, 190, 100, 280, is_opaque_red),
            "transformed node inside the dirty region should paint"
        );
    }

    /// #143 (tracking half): compute_dirty_region must cover a transformed
    /// node's visual position, not its layout position.
    #[test]
    fn test_compute_dirty_region_uses_transformed_position() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            "width: 50px; height: 50px; background-color: red; \
             transform: translateY(200px)",
        );
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);

        // Initial layout marks every node paint-dirty — reset so the region
        // reflects only the transformed div.
        doc.tree.paint_dirty_nodes.clear();
        doc.tree.paint_dirty_removed_rects.clear();
        doc.tree.paint_dirty_nodes.push(div.0);
        let region = rinch_dom::paint::compute_dirty_region(&doc.tree, 1.0, 800.0, 600.0)
            .expect("dirty node should produce a region");
        doc.tree.paint_dirty_nodes.clear();

        assert!(
            region.y1 > 200.0,
            "region should extend past the translated position, got {region:?}"
        );
        assert!(
            region.y0 > 100.0,
            "region should not start at the untransformed layout rect, got {region:?}"
        );
    }
}
