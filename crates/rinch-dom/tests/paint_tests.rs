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
        paint_skia_at(doc, painter, 1.0);
    }

    /// Same, at an explicit DPI scale (layout stays in CSS px — the paint
    /// pipeline is what scales, exactly as the embed/Android runtimes drive it).
    fn paint_skia_at(doc: &mut RinchDocument, painter: &mut TinySkiaPainter, scale: f64) {
        let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            painter,
            scale,
            (800.0 * scale as f32, 600.0 * scale as f32),
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

    /// Bounding box of the opaque-red pixels as `(x0, y0, x1, y1)`, inclusive.
    fn red_bbox(painter: &TinySkiaPainter) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
        let mut found = false;
        for y in 0..painter.height() {
            for x in 0..painter.width() {
                if is_opaque_red(pixel_at(painter, x, y)) {
                    found = true;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        found.then_some((x0, y0, x1, y1))
    }

    /// #202 (pixel level): a translated element must paint at
    /// `scale · (layout + translate)`, not `scale · layout + translate`. The
    /// painted box at scale 2 must therefore land at exactly twice the
    /// coordinates it occupies at scale 1 — the covariance oracle, observed
    /// through the software rasterizer.
    #[test]
    fn test_translate_scales_with_dpi() {
        fn paint_at(scale: f64) -> (u32, u32, u32, u32) {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let div = doc.create_element("div");
            doc.set_attribute(
                div,
                "style",
                "width: 20px; height: 20px; background-color: red; \
                 transform: translate(40px, 30px)",
            );
            doc.append_child(body, div);
            doc.resolve_layout(800.0, 600.0);

            let mut painter = TinySkiaPainter::new(400, 400);
            paint_skia_at(&mut doc, &mut painter, scale);
            red_bbox(&painter).expect("translated red box should paint somewhere")
        }

        let at1 = paint_at(1.0);
        let at2 = paint_at(2.0);

        for (label, one, two) in [
            ("x0", at1.0, at2.0),
            ("y0", at1.1, at2.1),
            ("x1", at1.2, at2.2),
            ("y1", at1.3, at2.3),
        ] {
            let expected = 2 * one as i64;
            assert!(
                (two as i64 - expected).abs() <= 2,
                "{label} at scale 2 should be ~2x its scale-1 value \
                 (expected ~{expected}, got {two}); scale-1 bbox {at1:?}, \
                 scale-2 bbox {at2:?} — an under-translated box means the CSS \
                 translate was not multiplied by the DPI scale (#202)"
            );
        }
    }
}

// ── #202: DPI-scale covariance of transform composition ──────────────────────

/// The mechanical correctness criterion for `compose_node_transform`: it must be
/// *covariant* under the physical/layout change of units. Composing at
/// `(s·x, s·y, s)` has to equal `S · compose(x, y, 1.0) · S⁻¹` for `S = scale(s)`
/// — i.e. "compose in physical px" and "compose in layout px, then convert" are
/// the same map. That identity is exactly what lets hit testing (`local_point`
/// in `crates/rinch/src/app/hit_testing.rs`) compose at `scale = 1.0` in layout
/// space and still mirror paint at any DPI.
///
/// The linear part (rotate/scale/skew) is unit-invariant, so it satisfies this
/// for free; the *translate* part is a length and only satisfies it when scaled
/// (#202).
mod transform_dpi_covariance {
    use super::*;
    use peniko::kurbo::Affine;

    /// One `<div>` carrying `style`, laid out, plus its node id.
    fn styled_div(style: &str) -> (RinchDocument, usize) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", style);
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);
        (doc, div.0)
    }

    fn affine_mismatch(actual: Affine, expected: Affine) -> Option<String> {
        let (a, e) = (actual.as_coeffs(), expected.as_coeffs());
        let bad = (0..6).find(|&i| (a[i] - e[i]).abs() >= 1e-6)?;
        Some(format!(
            "coefficient {bad} differs — got {a:?}, expected {e:?}"
        ))
    }

    #[test]
    fn compose_node_transform_is_covariant_in_scale() {
        // (name, transform declaration)
        let transforms: [(&str, &str); 6] = [
            ("translate(px)", "transform: translate(30px, 15px)"),
            ("translate(%)", "transform: translate(50%, -25%)"),
            ("rotate", "transform: rotate(30deg)"),
            ("scale", "transform: scale(1.5, 0.5)"),
            (
                "rotate+scale+translate",
                "transform: rotate(20deg) scale(1.3) translate(12px, 7px)",
            ),
            (
                "matrix() with m4/m5",
                "transform: matrix(1.2, 0.3, -0.4, 0.9, 25, -18)",
            ),
        ];
        let origins: [(&str, &str); 2] = [
            ("default origin", ""),
            ("non-default origin", "transform-origin: 10px 70%;"),
        ];

        // An arbitrary non-zero layout-space position for the node's box.
        let (x, y) = (37.0_f64, 23.0_f64);
        // Every case is checked before failing, so a regression report names
        // *all* the transform kinds that broke, not just the first.
        let mut failures: Vec<String> = Vec::new();

        for (tf_name, tf) in transforms {
            for (origin_name, origin) in origins {
                let style = format!("width: 80px; height: 40px; {origin} {tf}");
                let (doc, id) = styled_div(&style);
                let node = doc.tree.get(id).expect("div should be in the tree");
                assert!(
                    !node.computed_style.transform.is_identity,
                    "{tf_name} / {origin_name}: transform did not parse"
                );

                let base =
                    rinch_dom::paint::compose_node_transform(node, x, y, 1.0, Affine::IDENTITY);

                for s in [1.0_f64, 1.5, 2.0] {
                    let scaled = rinch_dom::paint::compose_node_transform(
                        node,
                        s * x,
                        s * y,
                        s,
                        Affine::IDENTITY,
                    );
                    let conjugated = Affine::scale(s) * base * Affine::scale(1.0 / s);
                    if let Some(why) = affine_mismatch(scaled, conjugated) {
                        failures.push(format!("{tf_name} / {origin_name} at scale {s}: {why}"));
                    }
                }
            }
        }

        assert!(
            failures.is_empty(),
            "compose_node_transform is not covariant in scale for {} case(s) (#202):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}

/// Regression for the paint half of issue #61 (see `ifc.rs`,
/// `mark_inline_descendants`).
///
/// rsx emits a comment marker for every `if`/`for`/`match`/component, and a
/// `display:contents` wrapper for `Vec<NodeHandle>` children. Comments count as
/// inline children, so a block container holding a marker becomes an inline
/// formatting context root. The IFC machinery then marked *every*
/// `display:contents` child of that root with `ifc_root`, which tells the paint
/// tree-walk "the IFC draws this, skip it" — so a wrapper full of **block**
/// content, and its entire subtree, silently vanished from the scene while
/// keeping perfectly correct layout boxes.
#[test]
fn test_block_content_behind_display_contents_still_paints() {
    /// Builds `<div width:400px><!--for--> [wrapper] <div 100x40 red></div>`,
    /// with the red block either behind a `display:contents` wrapper or a
    /// direct child, and returns (draw op count, wrapper's `ifc_root`).
    fn build(wrapped: bool) -> (usize, Option<usize>) {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        // A block container that becomes an IFC root purely because of an rsx
        // control-flow marker comment.
        let container = doc.create_element("div");
        doc.set_attribute(container, "style", "width: 400px");
        doc.append_child(body, container);
        let marker = doc.create_comment("for");
        doc.append_child(container, marker);

        let parent = if wrapped {
            let wrapper = doc.create_element("div");
            doc.set_attribute(wrapper, "style", "display: contents");
            doc.append_child(container, wrapper);
            wrapper
        } else {
            container
        };

        let painted = doc.create_element("div");
        doc.set_attribute(
            painted,
            "style",
            "width: 100px; height: 40px; background-color: red",
        );
        doc.append_child(parent, painted);

        doc.resolve_layout(800.0, 600.0);

        // Layout is unaffected either way — the box is there, it just never drew.
        let l = doc.tree.get(painted.0).unwrap().layout;
        assert_eq!((l.width, l.height), (100.0, 40.0), "block box laid out");

        let mut painter = VelloPainter::new();
        paint(&mut doc, &mut painter);
        let draws = painter.scene().encoding().draw_tags.len();
        let ifc_root = if wrapped {
            doc.tree.get(parent.0).unwrap().ifc_root
        } else {
            None
        };
        (draws, ifc_root)
    }

    let (control_draws, _) = build(false);
    let (wrapped_draws, wrapper_ifc_root) = build(true);

    assert_eq!(
        wrapped_draws, control_draws,
        "a block box behind a display:contents wrapper must produce the same \
         draw operations as the same box without the wrapper \
         (wrapped={wrapped_draws}, control={control_draws})"
    );

    // The mechanism: the wrapper wraps a block, so it is not IFC content.
    assert_eq!(
        wrapper_ifc_root, None,
        "a display:contents wrapper holding block content must not be marked \
         as IFC content — paint skips every node whose ifc_root is set"
    );
}
