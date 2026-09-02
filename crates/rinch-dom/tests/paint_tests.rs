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

/// `parse_color` as `(r, g, b, a)`, for exact assertions.
fn rgba(value: &str) -> Option<(u8, u8, u8, u8)> {
    rinch_dom::layout::parse_color(value).map(|c| {
        let c = c.to_rgba8();
        (c.r, c.g, c.b, c.a)
    })
}

#[test]
fn test_parse_color_named() {
    assert_eq!(rgba("red"), Some((255, 0, 0, 255)));
    assert_eq!(rgba("blue"), Some((0, 0, 255, 255)));
    assert_eq!(rgba("transparent"), Some((0, 0, 0, 0)));
    assert_eq!(rgba("unknown_color"), None);
}

/// #250: the whole CSS named-colour table, matched case-insensitively — not a
/// private dozen. `rebeccapurple` is the last name the spec added.
#[test]
fn test_parse_color_full_named_table_case_insensitive() {
    assert_eq!(rgba("rebeccapurple"), Some((102, 51, 153, 255)));
    assert_eq!(rgba("aqua"), Some((0, 255, 255, 255)));
    assert_eq!(rgba("Aqua"), Some((0, 255, 255, 255)));
    assert_eq!(rgba("AQUA"), Some((0, 255, 255, 255)));
    assert_eq!(rgba("  RebeccaPurple  "), Some((102, 51, 153, 255)));
}

#[test]
fn test_parse_color_hex() {
    assert_eq!(rgba("#f00"), Some((255, 0, 0, 255)));
    assert_eq!(rgba("#ff0000"), Some((255, 0, 0, 255)));
    assert_eq!(rgba("#ff000080"), Some((255, 0, 0, 128)));
    // #250: 4-digit hex is CSS Color 4 (#rgba).
    assert_eq!(rgba("#1234"), Some((0x11, 0x22, 0x33, 0x44)));
    assert_eq!(rgba("#xyz"), None);
    assert_eq!(rgba("#12"), None);
}

#[test]
fn test_parse_color_rgb_rgba() {
    assert_eq!(rgba("rgb(255, 0, 0)"), Some((255, 0, 0, 255)));
    assert_eq!(rgba("rgba(255, 0, 0, 0.5)"), Some((255, 0, 0, 128)));
    // #250: modern space-separated syntax, with and without a `/ alpha`.
    assert_eq!(rgba("rgb(0 128 255)"), Some((0, 128, 255, 255)));
    assert_eq!(rgba("rgb(0 128 255 / 50%)"), Some((0, 128, 255, 128)));
    // #250: CSS clamps out-of-range components; it does not reject them.
    assert_eq!(rgba("rgb(300, 0, 0)"), Some((255, 0, 0, 255)));
    assert_eq!(rgba("rgb(255, 0)"), None);
    assert_eq!(rgba("rgb(0, 0)"), None);
}

/// #250: `hsl()`/`hsla()` were not parsed at all.
#[test]
fn test_parse_color_hsl() {
    assert_eq!(rgba("hsl(270 50% 40%)"), Some((102, 51, 153, 255)));
    assert_eq!(rgba("hsla(270, 50%, 40%, 0.5)"), Some((102, 51, 153, 128)));
}

/// The rest of the CSS Color 4 grammar `parse_color`'s doc promises, one
/// vector per family; `in srgb` keeps the mix exact.
#[test]
fn test_parse_color_css_color_4_functions() {
    assert_eq!(rgba("hwb(270 20% 40%)"), Some((102, 51, 153, 255)));
    assert_eq!(rgba("color(srgb 0.4 0.2 0.6)"), Some((102, 51, 153, 255)));
    assert_eq!(rgba("oklch(0 0 0)"), Some((0, 0, 0, 255)));
    assert_eq!(
        rgba("color-mix(in srgb, red 25%, blue)"),
        Some((64, 0, 191, 255))
    );
    // A mix over `currentcolor` is not absolute on its own.
    assert_eq!(rgba("color-mix(in srgb, currentcolor, blue)"), None);
}

#[test]
fn test_parse_color_rejects_junk() {
    assert_eq!(rgba(""), None);
    assert_eq!(rgba("   "), None);
    assert_eq!(rgba("red junk"), None);
    assert_eq!(rgba("red; background: blue"), None);
    assert_eq!(rgba("red !important"), None);
}

/// `currentcolor` is not an absolute colour: the caller owns its resolution
/// (`resolve_svg_color` walks the tree for it), so `parse_color` must say no
/// rather than invent a value.
#[test]
fn test_parse_color_currentcolor_is_not_absolute() {
    assert_eq!(rgba("currentcolor"), None);
    assert_eq!(rgba("currentColor"), None);
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
    pub(super) fn paint_skia(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
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
    pub(super) fn pixel_at(painter: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
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

    pub(super) fn is_opaque_red(p: [u8; 4]) -> bool {
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

    /// #212 × #402: an opacity layer's bounds are computed by a walk that
    /// mirrors paint's arithmetic, [`compose_node_transform`] included — so a
    /// percentage translate that composes in a *rotated* frame has to carry the
    /// layer with it. Where the two disagree, Vello clips every command in the
    /// layer to bounds sized for somewhere the content no longer is, and the
    /// subtree vanishes on the GPU path while the software path draws it (the
    /// software painter ignores the shape, which is why this asserts on the
    /// rect rather than on pixels).
    ///
    /// Nothing pinned the pairing: `layer_bounds.rs` landed on `main` after
    /// this branch was cut, so its walk has never met a #212-shaped transform.
    #[test]
    fn an_opacity_layers_bounds_follow_a_percentage_translate_into_its_rotated_frame() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "margin: 0");

        let layer = doc.create_element("div");
        doc.set_attribute(
            layer,
            "style",
            "position: relative; margin-left: 100px; margin-top: 100px; \
             width: 50px; height: 20px; opacity: 0.5",
        );
        doc.append_child(body, layer);

        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "position: absolute; left: 0; top: 0; width: 50px; height: 20px; \
             background-color: red; transform-origin: 0 0; \
             transform: rotate(90deg) translateX(100%)",
        );
        doc.append_child(layer, child);
        doc.resolve_layout(800.0, 600.0);

        let layer_node = doc.tree.get(layer.0).expect("the layer is in the tree");
        let (lx, ly) = (layer_node.layout.x as f64, layer_node.layout.y as f64);
        let bounds = rinch_dom::paint::opacity_layer_bounds(&doc.tree, layer.0, 1.0, lx, ly);
        assert!(
            bounds != rinch_dom::paint::UNBOUNDED,
            "this subtree is small enough for the walk to bound it; \
             an UNBOUNDED answer would make the assertion below vacuous"
        );

        // Where paint says the child went. Read rather than hand-computed, so
        // this asserts layer/paint *agreement* — `transform_percentage_translate`
        // owns #212's arithmetic itself.
        let painted = rinch_dom::paint::painted_border_box(&doc.tree, child.0, 1.0);
        assert!(
            bounds.x0 <= painted.x0 + 0.01
                && bounds.y0 <= painted.y0 + 0.01
                && bounds.x1 >= painted.x1 - 0.01
                && bounds.y1 >= painted.y1 - 0.01,
            "the opacity layer's bounds {bounds:?} must cover the box paint draws, {painted:?}"
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

// ── #236: a set_style inset move must paint where layout puts it ─────────────

/// The set_style inset fast path skips Stylo; the pixel it paints must still
/// be the one a full resolve of the same declaration would paint. Before the
/// fix the fast path wrote `layout.x = left_px` (no parent border) and never
/// marked layout dirty, so the child painted short of its true position.
#[cfg(feature = "software-renderer")]
mod inset_fast_path_paint {
    use super::transform_paint::{is_opaque_red, paint_skia, pixel_at};
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    #[test]
    fn set_style_left_paints_at_layout_position() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // A 30px border is wider than the child, so the box the old fast path
        // painted ([40, 60)) and the true box ([70, 90)) do not overlap.
        let parent = doc.create_element("div");
        doc.set_attribute(
            parent,
            "style",
            "position: relative; width: 300px; height: 200px; \
             border-left: 30px solid black; border-top: 0 solid black",
        );
        doc.append_child(body, parent);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "position: absolute; left: 0; top: 0; width: 20px; height: 20px; \
             background-color: red",
        );
        doc.append_child(parent, child);
        doc.resolve_layout(800.0, 600.0);

        doc.set_style(child, "left", "40px");
        doc.resolve_layout(800.0, 600.0);

        let mut painter = TinySkiaPainter::new(200, 100);
        paint_skia(&mut doc, &mut painter);

        let layout = doc.tree.get(child.0).unwrap().layout;
        assert!(
            is_opaque_red(pixel_at(&painter, 80, 10)),
            "the child must paint at its laid-out centre (80, 10) — border (30) + \
             left (40) + half its width; layout.x = {}",
            layout.x
        );
        assert!(
            !is_opaque_red(pixel_at(&painter, 50, 10)),
            "nothing at the padding-box-relative position the old fast path wrote"
        );
    }
}

// ── #204: an ICB-absolute paints over the viewport, not over its parent ──────

/// A local pixel oracle for the containing-block fix. The absolute box is the
/// only thing in the document with a background, and its parent is a small box
/// pushed well away from the origin — so the top-left corner of the canvas is a
/// region where the correct output is provably red and the buggy output is
/// provably blank. (A whole-screen comparison would not separate the two; see
/// the note in `reference_visual_regression_gap`.)
#[cfg(feature = "software-renderer")]
mod icb_absolute_paint {
    use super::transform_paint::{is_opaque_red, paint_skia, pixel_at};
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    #[test]
    fn inset_zero_under_an_unpositioned_parent_paints_over_the_viewport() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let parent = doc.create_element("div");
        doc.set_attribute(
            parent,
            "style",
            "width: 100px; height: 50px; margin-left: 200px; margin-top: 100px",
        );
        doc.append_child(body, parent);
        let overlay = doc.create_element("div");
        doc.set_attribute(
            overlay,
            "style",
            "position: absolute; inset: 0; background-color: red",
        );
        doc.append_child(parent, overlay);
        doc.resolve_layout(800.0, 600.0);

        let mut painter = TinySkiaPainter::new(400, 300);
        paint_skia(&mut doc, &mut painter);

        let layout = doc.tree.get(overlay.0).unwrap().layout;
        assert!(
            is_opaque_red(pixel_at(&painter, 10, 10)),
            "the overlay fills the viewport, so the canvas corner is red;              before #204 the red started at the parent's origin (200, 100) and              this pixel was blank. layout = {layout:?}"
        );
        assert!(
            is_opaque_red(pixel_at(&painter, 390, 290)),
            "and it still covers the far side of the canvas"
        );
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
    pub(super) fn styled_div(style: &str) -> (RinchDocument, usize) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(div, "style", style);
        doc.append_child(body, div);
        doc.resolve_layout(800.0, 600.0);
        (doc, div.0)
    }

    pub(super) fn affine_mismatch(actual: Affine, expected: Affine) -> Option<String> {
        let (a, e) = (actual.as_coeffs(), expected.as_coeffs());
        let bad = (0..6).find(|&i| (a[i] - e[i]).abs() >= 1e-6)?;
        Some(format!(
            "coefficient {bad} differs — got {a:?}, expected {e:?}"
        ))
    }

    #[test]
    fn compose_node_transform_is_covariant_in_scale() {
        // (name, transform declaration)
        let transforms: [(&str, &str); 8] = [
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
            // The #212 shape: the percentage offset is resolved inside a
            // non-identity frame, so it reaches `m[4]`/`m[5]` already rotated
            // or scaled. It is still a pure length and must still scale.
            (
                "rotate+translate(%)",
                "transform: rotate(45deg) translateX(50%)",
            ),
            ("scale+translate(%)", "transform: scale(2) translateX(50%)"),
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

// ── A percentage translate composes in list order (#212) ────────────────────
//
// `TransformValue` keeps the percentage part of a `translate` out of `matrix`,
// because it cannot be resolved until Taffy has produced the border box. What
// went wrong is *where* it was put back: it was added to the composed matrix's
// final `e`/`f`, i.e. in the outer frame, after every linear function. CSS
// composes transform functions in list order, so a percentage translate must
// take effect in the frame the functions before it establish.
//
// Every expectation below is hand-computed from that rule and matches what a
// browser reports for the same declaration.

mod transform_percentage_translate {
    use super::transform_dpi_covariance::{affine_mismatch, styled_div};
    use peniko::kurbo::Affine;

    /// cos 45° = sin 45°.
    const C45: f64 = std::f64::consts::FRAC_1_SQRT_2;

    /// Compose `style`'s transform at the viewport origin, at scale 1, under no
    /// parent transform — so the assertion is on the raw matrix and does not
    /// depend on body margins or where the box happened to be laid out.
    fn composed(style: &str) -> Affine {
        let (doc, id) = styled_div(style);
        let node = doc.tree.get(id).expect("div should be in the tree");
        assert!(
            !node.computed_style.transform.is_identity,
            "transform did not parse: {style}"
        );
        rinch_dom::paint::compose_node_transform(node, 0.0, 0.0, 1.0, Affine::IDENTITY)
    }

    /// A `100x40` box whose transform-origin is pinned to its top-left corner,
    /// so the origin conjugation is the identity and the assertion is purely
    /// about what went *into* the matrix.
    fn box_100x40(transform: &str) -> Affine {
        composed(&format!(
            "width: 100px; height: 40px; transform-origin: 0 0; transform: {transform}"
        ))
    }

    #[track_caller]
    fn assert_matrix(actual: Affine, expected: [f64; 6], what: &str) {
        if let Some(why) = affine_mismatch(actual, Affine::new(expected)) {
            panic!("{what}: {why}");
        }
    }

    /// **A.** `rotate(45deg) translateX(50%)` on a 100px-wide box: the 50px
    /// offset is applied in the rotated frame, so it lands on the diagonal.
    /// Chrome: `matrix(0.707107, 0.707107, -0.707107, 0.707107, 35.3553,
    /// 35.3553)`. Before the fix rinch produced `(…, 50, 0)`.
    #[test]
    fn rotation_before_a_percentage_translate_rotates_the_offset() {
        assert_matrix(
            box_100x40("rotate(45deg) translateX(50%)"),
            [C45, C45, -C45, C45, 50.0 * C45, 50.0 * C45],
            "rotate(45deg) translateX(50%)",
        );
    }

    /// **B.** `scale(2) translateX(50%)`: the offset is scaled too — this is
    /// why the bug is not confined to rotation and skew. Chrome reports
    /// `matrix(2, 0, 0, 2, 100, 0)`; before the fix rinch said `100 → 50`.
    #[test]
    fn scale_before_a_percentage_translate_scales_the_offset() {
        assert_matrix(
            box_100x40("scale(2) translateX(50%)"),
            [2.0, 0.0, 0.0, 2.0, 100.0, 0.0],
            "scale(2) translateX(50%)",
        );
    }

    /// **C (positive control).** The reverse order was always right and must
    /// stay right: the translate is first, so the linear part in effect at that
    /// point is the identity and the following `scale` touches only the linear
    /// part.
    #[test]
    fn a_leading_percentage_translate_is_not_scaled_by_what_follows() {
        assert_matrix(
            box_100x40("translateX(50%) scale(2)"),
            [2.0, 0.0, 0.0, 2.0, 50.0, 0.0],
            "translateX(50%) scale(2)",
        );
    }

    /// **D (positive control).** The centring idiom — the overwhelmingly common
    /// real-world case, and the one every shipped component uses. Unchanged.
    #[test]
    fn the_centring_idiom_is_unchanged() {
        assert_matrix(
            composed(
                "width: 200px; height: 100px; transform-origin: 0 0; \
                 transform: translate(-50%, -50%)",
            ),
            [1.0, 0.0, 0.0, 1.0, -100.0, -50.0],
            "translate(-50%, -50%)",
        );
    }

    /// **E (positive control, and the cleanest statement of the bug).** CSS
    /// guarantees that on a 100px-wide box `translateX(50%)` *is*
    /// `translateX(50px)` — the percentage resolves against the border box and
    /// nothing else distinguishes them. So the two declarations must compose to
    /// the same matrix under any prefix. They did not.
    #[test]
    fn fifty_percent_of_a_hundred_px_box_is_fifty_px_in_any_frame() {
        let pct = box_100x40("rotate(45deg) translateX(50%)");
        let px = box_100x40("rotate(45deg) translateX(50px)");
        if let Some(why) = affine_mismatch(pct, px) {
            panic!(
                "rotate(45deg) translateX(50%) must equal rotate(45deg) translateX(50px) \
                 on a 100px-wide box: {why}"
            );
        }
    }

    /// **F.** Both axes, with a non-identity frame between the two translates —
    /// this is the case a fix that only repaired `translateX` would fail.
    ///
    /// `translateX(50%)` runs in the identity frame → `(50, 0)`. After
    /// `rotate(90deg)` the local x-axis points at `(0, 1)` and the local y-axis
    /// at `(-1, 0)`, so `translateY(50%)` of the 40px height moves `(-20, 0)`.
    /// Total `(30, 0)`. Before the fix: `(50, 20)` — the second offset applied
    /// unrotated, on the wrong axis.
    #[test]
    fn two_percentage_translates_each_use_their_own_frame() {
        assert_matrix(
            box_100x40("translateX(50%) rotate(90deg) translateY(50%)"),
            [0.0, 1.0, -1.0, 0.0, 30.0, 0.0],
            "translateX(50%) rotate(90deg) translateY(50%)",
        );
    }

    /// **G.** `transform-origin` and the percentage translate are not
    /// conflated, even though both resolve against the same border box. The
    /// origin is a conjugation applied *outside* the composed matrix and is
    /// untouched by this fix; only what goes into the matrix changed.
    ///
    /// `rotate(90deg) translateX(50%)` gives `m = [0, 1, -1, 0, 0, 50]`.
    /// Conjugating by the origin `(100, 20)` maps `(x, y) → (120 − y, x − 30)`.
    #[test]
    fn transform_origin_is_not_conflated_with_the_percentage_translate() {
        assert_matrix(
            composed(
                "width: 100px; height: 40px; transform-origin: 100% 50%; \
                 transform: rotate(90deg) translateX(50%)",
            ),
            [0.0, 1.0, -1.0, 0.0, 120.0, -30.0],
            "rotate(90deg) translateX(50%) about 100% 50%",
        );
    }
}

// ── SVG presentation attributes (#250): `fill`/`stroke` are parsed with the
// same CSS colour parser as stylesheets, and `currentcolor` is matched the
// way the spec spells it. Pixel assertions use the software renderer.

#[cfg(feature = "software-renderer")]
mod svg_paint {
    use super::transform_paint::{paint_skia, pixel_at};
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Paint a 20×20 `<svg viewBox="0 0 10 10">` holding one full-viewBox
    /// `<rect>` at the top-left of the page and return the pixel at its
    /// centre. `svg_fill`/`rect_fill` are the `fill` attributes (`None` for
    /// absent); `svg_style`/`rect_style` the inline styles (for `color`).
    fn svg_rect_centre_pixel(
        svg_fill: Option<&str>,
        svg_style: &str,
        rect_fill: Option<&str>,
        rect_style: &str,
    ) -> [u8; 4] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let svg = doc.create_element("svg");
        doc.set_attribute(svg, "viewBox", "0 0 10 10");
        if let Some(fill) = svg_fill {
            doc.set_attribute(svg, "fill", fill);
        }
        doc.set_attribute(
            svg,
            "style",
            &format!("display: block; width: 20px; height: 20px; {svg_style}"),
        );
        doc.append_child(body, svg);
        let rect = doc.create_element("rect");
        doc.set_attribute(rect, "width", "10");
        doc.set_attribute(rect, "height", "10");
        if let Some(fill) = rect_fill {
            doc.set_attribute(rect, "fill", fill);
        }
        if !rect_style.is_empty() {
            doc.set_attribute(rect, "style", rect_style);
        }
        doc.append_child(svg, rect);
        doc.resolve_layout(800.0, 600.0);

        let layout = doc.tree.get(svg.0).unwrap().layout;
        assert_eq!(
            (layout.x, layout.y, layout.width, layout.height),
            (0.0, 0.0, 20.0, 20.0),
            "the svg should sit at the page origin at its styled size"
        );

        let mut painter = TinySkiaPainter::new(40, 40);
        paint_skia(&mut doc, &mut painter);
        pixel_at(&painter, 10, 10)
    }

    /// `svg_rect_centre_pixel` for a rect with its own `fill`.
    fn rect_centre_pixel(fill: &str, svg_style: &str) -> [u8; 4] {
        svg_rect_centre_pixel(None, svg_style, Some(fill), "")
    }

    #[test]
    fn svg_fill_named_colour_outside_legacy_table_paints() {
        assert_eq!(rect_centre_pixel("rebeccapurple", ""), [102, 51, 153, 255]);
    }

    #[test]
    fn svg_fill_hsl_paints() {
        assert_eq!(
            rect_centre_pixel("hsl(270 50% 40%)", ""),
            [102, 51, 153, 255]
        );
    }

    #[test]
    fn svg_fill_currentcolor_lowercase_resolves_to_css_color() {
        assert_eq!(
            rect_centre_pixel("currentcolor", "color: rgb(1, 2, 3)"),
            [1, 2, 3, 255]
        );
    }

    #[test]
    fn svg_fill_currentcolor_camelcase_resolves_to_css_color() {
        assert_eq!(
            rect_centre_pixel("currentColor", "color: rgb(1, 2, 3)"),
            [1, 2, 3, 255]
        );
    }

    #[test]
    fn svg_fill_none_paints_nothing() {
        assert_eq!(rect_centre_pixel("none", ""), [0, 0, 0, 0]);
    }

    /// SVG's initial `fill` is black: a shape with no `fill` anywhere paints.
    #[test]
    fn svg_fill_absent_paints_black() {
        assert_eq!(svg_rect_centre_pixel(None, "", None, ""), [0, 0, 0, 255]);
    }

    /// A child with no `fill` inherits the `<svg>`'s.
    #[test]
    fn svg_fill_inherits_svg_level_fill() {
        assert_eq!(
            svg_rect_centre_pixel(Some("rebeccapurple"), "", None, ""),
            [102, 51, 153, 255]
        );
    }

    /// `currentcolor` resolves against the child's own `color`, not the
    /// `<svg>`'s — whether the child says it or inherits it.
    #[test]
    fn svg_fill_currentcolor_uses_child_colour() {
        assert_eq!(
            svg_rect_centre_pixel(
                None,
                "color: rgb(1, 2, 3)",
                Some("currentcolor"),
                "color: rgb(4, 5, 6)"
            ),
            [4, 5, 6, 255]
        );
        assert_eq!(
            svg_rect_centre_pixel(
                Some("currentColor"),
                "color: rgb(1, 2, 3)",
                None,
                "color: rgb(4, 5, 6)"
            ),
            [4, 5, 6, 255]
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

/// The same guarantee as `test_block_content_behind_display_contents_still_paints`,
/// but for content that becomes block-level *after* the first layout pass.
///
/// `ifc_root` is derived state that the marking pass only ever sets, so a
/// wrapper that legitimately joined the IFC while it was empty (an `if` branch
/// that starts hidden, a `for` list that starts empty, a reactive component that
/// first renders nothing) kept that mark forever once real content arrived —
/// and paint skips every node whose `ifc_root` is set.
#[test]
fn test_block_appended_into_a_marked_contents_wrapper_still_paints() {
    /// `<div width:400px><!--show--><div display:contents/></div>`, laid out
    /// once while the wrapper is empty, then given a 100x40 block child.
    /// Returns (draw op count after the second pass, wrapper's `ifc_root`).
    fn build(wrapped: bool) -> (usize, Option<usize>) {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let container = doc.create_element("div");
        doc.set_attribute(container, "style", "width: 400px");
        doc.append_child(body, container);
        let marker = doc.create_comment("show");
        doc.append_child(container, marker);

        let parent = if wrapped {
            let wrapper = doc.create_element("div");
            doc.set_attribute(wrapper, "style", "display: contents");
            doc.append_child(container, wrapper);
            wrapper
        } else {
            container
        };

        // First pass: the wrapper holds nothing at all.
        doc.resolve_layout(800.0, 600.0);

        // Second pass: the branch turns on and renders a block.
        let painted = doc.create_element("div");
        doc.set_attribute(
            painted,
            "style",
            "width: 100px; height: 40px; background-color: red",
        );
        doc.append_child(parent, painted);
        doc.resolve_layout(800.0, 600.0);

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
        wrapper_ifc_root, None,
        "a wrapper that was IFC content while empty must lose that mark once it \
         holds a block — paint skips every node whose ifc_root is set"
    );
    assert_eq!(
        wrapped_draws, control_draws,
        "a block box appended into a display:contents wrapper after the first \
         layout must paint like the same box with no wrapper \
         (wrapped={wrapped_draws}, control={control_draws})"
    );
}

/// Inline content that follows a block-holding `display:contents` wrapper must
/// not be marked as this IFC's content: `walk_inline_children` stops building
/// the line at that wrapper, so anything marked after it would be skipped by
/// paint *and* never drawn by the IFC — invisible in both directions.
#[test]
fn test_inline_after_a_block_wrapper_is_not_orphaned() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 400px");
    doc.append_child(body, container);
    let marker = doc.create_comment("for");
    doc.append_child(container, marker);

    // A contents wrapper holding a block box.
    let block_wrapper = doc.create_element("div");
    doc.set_attribute(block_wrapper, "style", "display: contents");
    doc.append_child(container, block_wrapper);
    let block = doc.create_element("div");
    doc.set_attribute(block, "style", "width: 100px; height: 40px");
    doc.append_child(block_wrapper, block);

    // A contents wrapper holding inline text, *after* it.
    let text_wrapper = doc.create_element("span");
    doc.set_attribute(text_wrapper, "style", "display: contents");
    doc.append_child(container, text_wrapper);
    let text = doc.create_text("TRAILING");
    doc.append_child(text_wrapper, text);

    doc.resolve_layout(800.0, 600.0);

    let ifc_text = doc
        .tree
        .get(container.0)
        .unwrap()
        .text_layout
        .as_ref()
        .map(|l| l.text_content.clone())
        .unwrap_or_default();

    // Either the IFC draws the text, or the text keeps its own box — but it
    // must never be both skipped by paint and absent from the inline layout.
    if !ifc_text.contains("TRAILING") {
        assert_eq!(
            doc.tree.get(text.0).unwrap().ifc_root,
            None,
            "text the IFC does not lay out must not be marked as IFC content — \
             paint would skip it and nothing would ever draw it"
        );
        assert_eq!(
            doc.tree.get(text_wrapper.0).unwrap().ifc_root,
            None,
            "the wrapper around that text must not be marked either"
        );
    }
}

/// A `display:none` child generates no box, so it must not change how the
/// surrounding `display:contents` wrapper is classified.
///
/// `display:none` resolves to `DisplayMode::Block`, so the scan that decides
/// whether a wrapper is transparent to the enclosing inline formatting context
/// counted a hidden child as a block-level box — and a wrapper whose only
/// "block" is hidden was pushed out of the IFC. Adding a hidden sibling next to
/// inline text is a no-op in CSS; it must be a no-op here too.
#[test]
fn test_a_display_none_child_does_not_push_a_contents_wrapper_out_of_the_ifc() {
    /// `<div width:400px><!--show--><span display:contents>VISIBLE[hidden?]</span></div>`.
    /// Returns (the wrapper's `ifc_root`, the container IFC's laid-out text).
    fn build(with_hidden_child: bool) -> (Option<usize>, String) {
        let mut doc = RinchDocument::new();
        let body = doc.body();

        let container = doc.create_element("div");
        doc.set_attribute(container, "style", "width: 400px");
        doc.append_child(body, container);
        let marker = doc.create_comment("show");
        doc.append_child(container, marker);

        let wrapper = doc.create_element("span");
        doc.set_attribute(wrapper, "style", "display: contents");
        doc.append_child(container, wrapper);
        let text = doc.create_text("VISIBLE");
        doc.append_child(wrapper, text);

        if with_hidden_child {
            let hidden = doc.create_element("div");
            doc.set_attribute(hidden, "style", "display: none");
            doc.append_child(wrapper, hidden);
        }

        doc.resolve_layout(800.0, 600.0);

        let ifc_text = doc
            .tree
            .get(container.0)
            .unwrap()
            .text_layout
            .as_ref()
            .map(|l| l.text_content.clone())
            .unwrap_or_default();
        (doc.tree.get(wrapper.0).unwrap().ifc_root, ifc_text)
    }

    let (control_root, control_text) = build(false);
    let (hidden_root, hidden_text) = build(true);

    assert!(
        control_text.contains("VISIBLE"),
        "precondition: the inline text is laid out by the container's IFC \
         (got {control_text:?})"
    );
    assert_eq!(
        hidden_root, control_root,
        "a display:contents wrapper's IFC classification must not change when a \
         display:none child — which generates no box at all — is added"
    );
    assert_eq!(
        hidden_text, control_text,
        "the inline text beside a hidden sibling must still be laid out by the \
         same inline formatting context"
    );
}

// ── Scrollbar overlays (#178) ────────────────────────────────────────────────
//
// The horizontal bar is new; the vertical one is the reference it mirrors.
// Pixel assertions, because "is the thumb actually drawn where a user can
// grab it" is not something the scene graph answers.

#[cfg(feature = "software-renderer")]
mod scrollbar_paint {
    use super::transform_paint::pixel_at;
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// The thumb is 40% black over the container's white background, so a
    /// mid-grey opaque pixel is a thumb pixel and a white one is bare track.
    fn is_thumb(p: [u8; 4]) -> bool {
        p[3] > 200 && p[0] < 200 && p[0] > 100 && p[1] == p[0] && p[2] == p[0]
    }

    fn any_thumb(painter: &TinySkiaPainter, x0: u32, y0: u32, x1: u32, y1: u32) -> bool {
        for y in y0..y1.min(painter.height()) {
            for x in x0..x1.min(painter.width()) {
                if is_thumb(pixel_at(painter, x, y)) {
                    return true;
                }
            }
        }
        false
    }

    /// A 200×100 white scroll container at the document origin, with one child
    /// sized by the caller, painted at scale 1.
    fn paint_scroller(
        container_style: &str,
        content_style: &str,
        scroll: (f64, f64),
    ) -> TinySkiaPainter {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            &format!("width: 200px; height: 100px; background-color: white; {container_style}"),
        );
        doc.append_child(body, container);
        let content = doc.create_element("div");
        doc.set_attribute(content, "style", content_style);
        doc.append_child(container, content);
        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[container.0].scroll_offset = scroll;

        let mut painter = TinySkiaPainter::new(800, 600);
        let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut paint_layout_cx,
        );
        painter
    }

    /// The container is 200×100 at the origin; the bar is 6px thick with a 2px
    /// margin, so the horizontal thumb occupies y ∈ [92, 98).
    #[test]
    fn a_horizontal_scroller_paints_a_thumb_along_its_bottom_edge() {
        let p = paint_scroller("overflow-x: auto", "width: 800px; height: 40px", (0.0, 0.0));
        assert!(
            any_thumb(&p, 0, 92, 60, 98),
            "a thumb sits at the left end of the bottom track"
        );
        assert!(
            !any_thumb(&p, 190, 0, 200, 90),
            "and nothing down the right-hand edge — this container does not \
             scroll vertically"
        );
    }

    /// The reference bar, unchanged.
    #[test]
    fn a_vertical_scroller_still_paints_a_thumb_down_its_right_edge() {
        let p = paint_scroller("overflow-y: auto", "width: 40px; height: 800px", (0.0, 0.0));
        assert!(
            any_thumb(&p, 192, 0, 198, 40),
            "a thumb sits at the top of the right-hand track"
        );
        assert!(
            !any_thumb(&p, 0, 90, 190, 100),
            "and nothing along the bottom edge"
        );
    }

    /// The thumb tracks the offset: scrolled to the end, it is at the far end
    /// of the track rather than still at the start.
    #[test]
    fn the_horizontal_thumb_moves_with_scroll_left() {
        let at_start = paint_scroller("overflow-x: auto", "width: 800px; height: 40px", (0.0, 0.0));
        let at_end = paint_scroller(
            "overflow-x: auto",
            "width: 800px; height: 40px",
            (600.0, 0.0),
        );

        assert!(any_thumb(&at_start, 0, 92, 40, 98));
        assert!(
            !any_thumb(&at_start, 160, 92, 198, 98),
            "precondition: nothing at the right end before scrolling"
        );
        assert!(
            any_thumb(&at_end, 160, 92, 198, 98),
            "scrolled to the end, the thumb is at the end of the track"
        );
        assert!(
            !any_thumb(&at_end, 0, 92, 40, 98),
            "and no longer at the start"
        );
    }

    /// The corner, on the paint side: with both bars up each track gives up the
    /// other bar's footprint, so the bottom-right square stays empty — the same
    /// square hit-testing gives to neither bar.
    #[test]
    fn neither_thumb_paints_into_the_corner() {
        // Both scrolled hard to the end, which is when the two thumbs would
        // otherwise pile into the same square.
        let p = paint_scroller(
            "overflow: auto",
            "width: 800px; height: 800px",
            (600.0, 700.0),
        );
        assert!(
            any_thumb(&p, 150, 92, 190, 98),
            "the horizontal thumb reaches the end of its shortened track"
        );
        assert!(
            any_thumb(&p, 192, 50, 198, 90),
            "the vertical thumb reaches the end of its shortened track"
        );
        assert!(
            !any_thumb(&p, 192, 92, 200, 100),
            "and the corner square is bare"
        );
    }
}

// ── #186: a viewport hole is only cut for something that will fill it ────────
//
// The punch removes the ancestor's background (an EvenOdd compound path), so on
// a transparent window a hole nothing fills is see-through to the desktop. A
// video that errored, or that has not decoded its first frame yet, is exactly
// that case. Pixel assertions, because "is there still a background here" is
// the whole question and the scene graph does not answer it.

#[cfg(feature = "software-renderer")]
mod viewport_hole_punch {
    use super::transform_paint::{paint_skia, pixel_at};
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// `TinySkiaPainter::new` zeroes the pixmap, so alpha 0 at a pixel means
    /// nothing painted there — a hole — and an opaque white pixel means the
    /// ancestor's background survived.
    fn is_opaque_white(p: [u8; 4]) -> bool {
        p[3] == 255 && p[0] > 250 && p[1] > 250 && p[2] > 250
    }

    /// A 200×100 white clipping box at the document origin wrapping a full-size
    /// `data-viewport` hole, painted at scale 1. `ready` is the value of
    /// `data-viewport-ready`, or `None` to omit the attribute entirely — the
    /// shape a `GameViewport` produces.
    ///
    /// `wrap_in_card` chooses which node is the clipping ancestor that owns the
    /// white background: a `overflow: hidden` card inside `<body>`, or `<body>`
    /// itself (`overflow-y: auto` in the UA sheet, which is why the hole reaches
    /// the compositor instead of stopping at the card).
    fn paint_viewport(ready: Option<&str>, wrap_in_card: bool) -> TinySkiaPainter {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let host = if wrap_in_card {
            let card = doc.create_element("div");
            doc.set_attribute(
                card,
                "style",
                "width: 200px; height: 100px; background-color: white; overflow: hidden;",
            );
            doc.append_child(body, card);
            card
        } else {
            doc.set_attribute(
                body,
                "style",
                "width: 200px; height: 100px; background-color: white;",
            );
            body
        };
        let viewport = doc.create_element("div");
        doc.set_attribute(viewport, "style", "width: 100%; height: 100%;");
        doc.set_attribute(viewport, "data-viewport", "v");
        if let Some(ready) = ready {
            doc.set_attribute(viewport, "data-viewport-ready", ready);
        }
        doc.append_child(host, viewport);
        doc.resolve_layout(800.0, 600.0);

        let mut painter = TinySkiaPainter::new(800, 600);
        paint_skia(&mut doc, &mut painter);
        painter
    }

    /// The same hole inside an `overflow: hidden` card.
    fn paint_card_with_viewport(ready: Option<&str>) -> TinySkiaPainter {
        paint_viewport(ready, true)
    }

    /// The same hole directly under `<body>`, with no card in between.
    fn paint_body_with_viewport(ready: Option<&str>) -> TinySkiaPainter {
        paint_viewport(ready, false)
    }

    /// The bug: the card's background is cut away under a viewport that has
    /// nothing to show, leaving alpha 0 — the desktop, on a transparent window.
    #[test]
    fn an_unready_viewport_does_not_cut_a_hole() {
        let p = paint_card_with_viewport(Some("false"));
        assert!(
            is_opaque_white(pixel_at(&p, 100, 50)),
            "the clipping ancestor keeps its background under a viewport that \
             declares itself not ready (#186), got {:?}",
            pixel_at(&p, 100, 50)
        );
    }

    /// The same, one level up: `<body>` must not cut the hole either, or the
    /// transparency reaches the compositor however opaque the card is.
    #[test]
    fn body_does_not_cut_a_hole_for_an_unready_viewport() {
        let p = paint_body_with_viewport(Some("false"));
        assert!(
            is_opaque_white(pixel_at(&p, 100, 50)),
            "<body> keeps its background under an unready viewport too (#186), \
             got {:?}",
            pixel_at(&p, 100, 50)
        );
    }

    /// The documented fail-safe direction: a node that carries the attribute
    /// must say exactly `"true"`, so a mis-stamped value yields an opaque
    /// placeholder rather than a see-through window.
    #[test]
    fn a_mis_stamped_readiness_value_fails_safe() {
        for value in ["True", "1", "yes", " true", ""] {
            let p = paint_card_with_viewport(Some(value));
            assert!(
                is_opaque_white(pixel_at(&p, 100, 50)),
                "data-viewport-ready={value:?} is not `true`, so it must not \
                 punch (#186), got {:?}",
                pixel_at(&p, 100, 50)
            );
        }
    }

    /// Guard: `GameViewport` stamps no readiness attribute and legitimately
    /// wants an unconditional hole (#207/#209). Absence must mean ready.
    #[test]
    fn a_viewport_without_a_readiness_attribute_still_cuts_a_hole() {
        let p = paint_card_with_viewport(None);
        assert_eq!(
            pixel_at(&p, 100, 50)[3],
            0,
            "absence of data-viewport-ready means ready — GameViewport is untouched"
        );
    }

    /// Guard: the video path once a frame has arrived.
    #[test]
    fn a_ready_viewport_cuts_a_hole() {
        let p = paint_card_with_viewport(Some("true"));
        assert_eq!(
            pixel_at(&p, 100, 50)[3],
            0,
            "a viewport that says it is ready gets its hole"
        );
    }
}

// ── #358 / #354: a software video frame paints inline, at its own z-order ────
//
// The software backend used to blit decoded video frames onto the *finished*
// pixel buffer, after `paint_document` had drawn the whole UI. Clipped only by
// its overflow-clipping ancestors and with no notion of occlusion, that blit
// destroyed every overlay above a video — drawer, modal, dropdown, tooltip.
//
// The frame now goes through paint instead: a `data-viewport` node with an
// entry in `set_viewport_pixels` fills opaque black over its box (the letterbox
// bars, #354's software half) and draws the frame `object-fit: contain` inside
// it. Everything painted later covers it, for free.
//
// Pixel assertions, because "what ended up on this pixel, and in what order"
// is the entire question.

#[cfg(feature = "software-renderer")]
mod software_video_inline {
    use super::transform_paint::{paint_skia, pixel_at};
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;
    use rinch_dom::paint::{SurfacePixelData, set_active_viewports, set_viewport_pixels};
    use std::collections::{HashMap, HashSet};

    /// A 40×10 solid magenta frame — 4:1, against a 2:1 viewport box, so
    /// `contain` fits the width and leaves a letterbox bar above and below.
    fn magenta_frame() -> SurfacePixelData {
        SurfacePixelData {
            data: [255u8, 0, 255, 255].repeat(40 * 10),
            width: 40,
            height: 10,
        }
    }

    fn is_magenta(p: [u8; 4]) -> bool {
        p[3] == 255 && p[0] > 250 && p[1] < 5 && p[2] > 250
    }

    fn is_opaque_black(p: [u8; 4]) -> bool {
        p[3] == 255 && p[0] < 5 && p[1] < 5 && p[2] < 5
    }

    fn is_opaque_blue(p: [u8; 4]) -> bool {
        p[3] == 255 && p[0] < 5 && p[1] < 5 && p[2] > 250
    }

    /// A 200×100 white `overflow: hidden` card at the document origin holding a
    /// full-size `data-viewport="v"` node, painted at scale 1.
    ///
    /// The viewport declares **no background of its own** — rinch-video flips it
    /// to `transparent` the moment a frame arrives, because the GPU backend
    /// composites video *under* the UI and an opaque element background would
    /// hide it. So the black behind the frame has to come from paint.
    ///
    /// `overlay` adds an absolutely-positioned opaque blue box over the whole
    /// card, painted after it — a stand-in for the nav drawer of #358.
    fn paint_video(
        frames: HashMap<String, SurfacePixelData>,
        active: Option<HashSet<String>>,
        overlay: bool,
    ) -> TinySkiaPainter {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let card = doc.create_element("div");
        doc.set_attribute(
            card,
            "style",
            "width: 200px; height: 100px; background-color: white; overflow: hidden;",
        );
        doc.append_child(body, card);

        let viewport = doc.create_element("div");
        doc.set_attribute(
            viewport,
            "style",
            "width: 100%; height: 100%; background: transparent;",
        );
        doc.set_attribute(viewport, "data-viewport", "v");
        doc.set_attribute(viewport, "data-viewport-ready", "true");
        doc.append_child(card, viewport);

        if overlay {
            let panel = doc.create_element("div");
            doc.set_attribute(
                panel,
                "style",
                "position: absolute; left: 0; top: 0; width: 200px; height: 100px; \
                 background-color: rgb(0, 0, 255);",
            );
            doc.append_child(body, panel);
        }

        doc.resolve_layout(800.0, 600.0);

        set_active_viewports(active);
        set_viewport_pixels(Some(frames));
        let mut painter = TinySkiaPainter::new(800, 600);
        paint_skia(&mut doc, &mut painter);
        set_viewport_pixels(None);
        set_active_viewports(None);
        painter
    }

    /// The software configuration: video is not a compositor layer, so no
    /// viewport name is active and nothing punches a hole.
    fn software_frames() -> (HashMap<String, SurfacePixelData>, Option<HashSet<String>>) {
        (
            HashMap::from([("v".to_string(), magenta_frame())]),
            Some(HashSet::new()),
        )
    }

    /// The frame lands on the viewport's own box, during paint.
    #[test]
    fn a_named_viewport_frame_paints_inline() {
        let (frames, active) = software_frames();
        let p = paint_video(frames, active, false);
        assert!(
            is_magenta(pixel_at(&p, 100, 50)),
            "the video frame is painted at the viewport's centre, got {:?}",
            pixel_at(&p, 100, 50)
        );
    }

    /// #354's software half: `contain` fits a 4:1 source into a 2:1 box, and the
    /// 25px bars above and below it are opaque black — what a browser paints for
    /// `<video>`, and never see-through on a transparent window.
    #[test]
    fn the_letterbox_bars_are_opaque_black() {
        let (frames, active) = software_frames();
        let p = paint_video(frames, active, false);
        for (x, y) in [(100u32, 10u32), (100, 90), (5, 5), (195, 95)] {
            assert!(
                is_opaque_black(pixel_at(&p, x, y)),
                "the letterbox bar at ({x}, {y}) is opaque black (#354), got {:?}",
                pixel_at(&p, x, y)
            );
        }
    }

    /// #358 itself: the frame is inside the paint order, so anything drawn after
    /// it covers it. On `main` the blit ran after `paint_document` and this pixel
    /// was the video.
    #[test]
    fn an_overlay_painted_after_the_video_covers_it() {
        let (frames, active) = software_frames();
        let p = paint_video(frames, active, true);
        for (x, y) in [(100u32, 50u32), (100, 10), (20, 80)] {
            assert!(
                is_opaque_blue(pixel_at(&p, x, y)),
                "the overlay above the video survives at ({x}, {y}) — frame and \
                 letterbox bars alike (#358), got {:?}",
                pixel_at(&p, x, y)
            );
        }
    }

    /// The GPU path sets no viewport pixels at all. A `data-viewport` node with
    /// no entry must therefore behave exactly as it did before: a plain element,
    /// with its hole punched out of the card's background for the compositor
    /// layer to show through.
    #[test]
    fn a_viewport_with_no_entry_falls_through_to_normal_painting() {
        let other = HashMap::from([("someone-else".to_string(), magenta_frame())]);
        let p = paint_video(other, None, false);
        assert_eq!(
            pixel_at(&p, 100, 50)[3],
            0,
            "no entry for this viewport — the hole is still punched and nothing \
             is painted into it, exactly as on the GPU backend"
        );
    }
}

// ── The two painters and the bounds of an `opacity` layer (K24, K35, K36) ───
//
// Card K24 recorded, without exercising it, that "Vello clips content
// overflowing an `opacity` element where the software path does not". K35 went
// looking for a screen in the app that would show the difference and could not
// find one — every `opacity` the app writes is on an element with nothing
// painted outside its own border box (a full-bleed sheet scrim with no
// children; a 40x40 icon button holding a centred glyph). So the claim could
// not be settled by looking at a phone, and it was settled here instead.
//
// It was true, and the mechanism was in three lines of two files:
//
//   * `paint/mod.rs` passed the element's **border box** as the layer bounds:
//     `painter.push_layer(BlendMode::Normal, opacity, node_transform, &rect.into())`,
//     where `rect` is the same `Rect` the background and borders are painted
//     into.
//   * `paint/skia_painter.rs` names that parameter `_bounds` and never reads
//     it. Its layer is a full-surface pixmap and `pop_layer` composites the
//     whole thing back, so nothing is clipped, ever.
//   * `paint/vello_painter.rs` hands it to `vello::Scene::push_layer`, whose
//     documentation is unambiguous: "Every drawing command after this call
//     will be clipped by the shape until the layer is popped."
//
// Children are painted between the push and the pop, so on the GPU path a
// descendant that escaped its parent's border box was drawn and then thrown
// away. CSS is clear that a stacking context does not clip its descendants, so
// the Vello behaviour was the wrong one, and it is what kept the GPU path from
// becoming the default.
//
// K36 fixed it, and the shape of the fix decides what these tests can assert.
// The bounds were not thrown away — Vello still gets a shape and still clips to
// it — they were made *true*: `paint::opacity_layer_bounds` walks the subtree
// and returns the union of what it will actually paint. So the two painters
// still do different things with the shape, and the encoding still carries a
// clip the opaque document does not have. What has to hold now is not that the
// clip is gone but that it cannot cut anything off, which is what the tests
// below check: the software painter still draws the escaping child, and the
// bounds the Vello layer is given still contain it.
#[cfg(feature = "software-renderer")]
mod opacity_overflow {
    use super::transform_paint::{paint_skia, pixel_at};
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// A 100x100 box with a red child sitting entirely outside it, at
    /// x = 150..170. Nothing else is on the page. `opacity` is a parameter so
    /// that the same document can be painted with and without it, which is the
    /// only way to attribute a clip to it — the encoding already carries a few
    /// from the root and the stacking context.
    fn overflowing_doc(opacity: &str) -> RinchDocument {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let parent = doc.create_element("div");
        doc.set_attribute(
            parent,
            "style",
            &format!(
                "position: relative; width: 100px; height: 100px; opacity: {opacity}; \
                 background-color: blue"
            ),
        );
        doc.append_child(body, parent);
        let child = doc.create_element("div");
        doc.set_attribute(
            child,
            "style",
            "position: absolute; left: 150px; top: 10px; width: 20px; height: 20px; \
             background-color: red",
        );
        doc.append_child(parent, child);
        doc.resolve_layout(800.0, 600.0);
        doc
    }

    /// The software painter draws the escaping child. This is the behaviour the
    /// shipping Android build and the desktop build have, and it is what the
    /// app's layouts have always been able to assume.
    #[test]
    fn software_painter_does_not_clip_an_opacity_layer() {
        let mut doc = overflowing_doc("0.5");
        let mut painter = TinySkiaPainter::new(300, 300);
        paint_skia(&mut doc, &mut painter);

        // The child is at x 150..170, y 10..30 — outside the parent's
        // 0..100 x 0..100 border box. Half-opacity red, so the channel test is
        // "much more red than blue" rather than "pure red". Widened to `i16`
        // because a bright-blue background would make `px[2] + 40` overflow a
        // `u8` and report the regression as an arithmetic panic instead of as
        // this message.
        let px = pixel_at(&painter, 160, 20);
        assert!(
            px[0] as i16 > px[2] as i16 + 40,
            "the child that overflows an opacity element must still be painted \
             by the software painter, got RGBA {px:?}"
        );
    }

    /// Vello is still given a clip, and the invariant that matters is what the
    /// clip is *shaped* like: it has to contain everything painted inside the
    /// layer, or it is the K24 bug again under a new number.
    ///
    /// This test asserts the shape rather than the absence of the child,
    /// because a `vello::Scene` cannot be rasterised without a GPU and there
    /// are no pixels here to look at. It checks both halves of the fix: the
    /// layer is still pushed (`n_clips` counts encoded clips/layers, and the
    /// translucent document still encodes more of them than the opaque one —
    /// so the bounds are still a real hint the GPU can use), and the rect those
    /// bounds are, computed the way paint computes it, contains the child that
    /// escapes the parent's border box.
    ///
    /// **If the bounds ever stop containing the child, do not widen this test.**
    /// A too-small bounds rect is the whole bug; `opacity_layer_bounds` is
    /// supposed to answer `UNBOUNDED` for anything it cannot work out.
    #[test]
    fn vello_painter_clips_an_opacity_layer_to_bounds_that_contain_the_child() {
        let clips = |opacity: &str| {
            let mut doc = overflowing_doc(opacity);
            let mut painter = VelloPainter::new();
            paint(&mut doc, &mut painter);
            painter.scene().encoding().n_clips
        };
        let (translucent, opaque) = (clips("0.5"), clips("1"));
        assert!(
            translucent > opaque,
            "an element with opacity < 1 must still push a bounded vello layer \
             that the same element at full opacity does not; got {translucent} \
             against {opaque}"
        );

        // The parent is the body's only child; the escaping child is its only
        // child in turn.
        let doc = overflowing_doc("0.5");
        let parent = doc.tree.get(doc.tree.body_id).unwrap().children[0];
        let child = doc.tree.get(parent).unwrap().children[0];
        let (px, py) = rinch_dom::paint::compute_absolute_position(&doc.tree, parent, 1.0);
        let bounds = rinch_dom::paint::opacity_layer_bounds(&doc.tree, parent, 1.0, px, py);
        let (cx, cy) = rinch_dom::paint::compute_absolute_position(&doc.tree, child, 1.0);
        let cl = doc.tree.get(child).unwrap().layout;

        assert!(
            bounds.x0 <= cx
                && bounds.y0 <= cy
                && bounds.x1 >= cx + cl.width as f64
                && bounds.y1 >= cy + cl.height as f64,
            "the bounds the opacity layer is clipped to must contain the child \
             that escapes the parent's border box: bounds {bounds:?}, child at \
             ({cx}, {cy}) {}x{}",
            cl.width,
            cl.height
        );
    }
}

/// Direct tests for `paint::opacity_layer_bounds` — the union of what a
/// subtree paints, which is the shape every `opacity` layer is now given.
///
/// The rule the whole function is written to is that a rect which is too large
/// costs a little GPU fill while a rect which is too small silently deletes
/// content, so each of these comes in one of two flavours: *this must be
/// inside the bounds*, or *this case must give up and return `UNBOUNDED`*.
/// There is deliberately no test asserting a tight rect for anything the walk
/// is not certain about.
mod opacity_layer_bounds {
    use super::*;
    use peniko::kurbo::Rect;
    use rinch_dom::paint::{UNBOUNDED, compute_absolute_position, opacity_layer_bounds};

    /// `<body><div id=subject style=...>{children}</div></body>`, laid out.
    /// Returns the document and the subject's id.
    fn doc_with(subject_style: &str, children: &[&str]) -> (RinchDocument, usize) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let subject = doc.create_element("div");
        doc.set_attribute(subject, "style", subject_style);
        doc.append_child(body, subject);
        for style in children {
            let child = doc.create_element("div");
            doc.set_attribute(child, "style", style);
            doc.append_child(subject, child);
        }
        doc.resolve_layout(800.0, 600.0);
        (doc, subject.0)
    }

    /// The bounds paint would hand the layer it opens around `id`.
    fn bounds_of(doc: &RinchDocument, id: usize) -> Rect {
        let (x, y) = compute_absolute_position(&doc.tree, id, 1.0);
        opacity_layer_bounds(&doc.tree, id, 1.0, x, y)
    }

    /// Where a node's border box actually is, in the same space as the bounds.
    fn box_of(doc: &RinchDocument, id: usize) -> Rect {
        let (x, y) = compute_absolute_position(&doc.tree, id, 1.0);
        let l = doc.tree.get(id).unwrap().layout;
        Rect::new(x, y, x + l.width as f64, y + l.height as f64)
    }

    /// Containment with a hair of slack, so a rounding difference in the last
    /// bit is not read as a clipped pixel.
    fn contains(outer: Rect, inner: Rect) -> bool {
        outer.x0 <= inner.x0 + 0.01
            && outer.y0 <= inner.y0 + 0.01
            && outer.x1 + 0.01 >= inner.x1
            && outer.y1 + 0.01 >= inner.y1
    }

    /// The floor: with nothing to widen it, the bounds are the element's own
    /// border box — the rect that used to be passed unconditionally. Every
    /// other case in this module only ever adds to this, which is why the fix
    /// cannot make any layer smaller than it was.
    #[test]
    fn a_childless_element_gets_its_own_border_box() {
        let (doc, subject) = doc_with("width: 100px; height: 60px; opacity: 0.5", &[]);
        let bounds = bounds_of(&doc, subject);
        assert_eq!(bounds, box_of(&doc, subject));
    }

    /// The case K35 could not find on a phone and K36 fixed: an absolutely
    /// positioned child that lands entirely outside its parent's border box.
    #[test]
    fn an_overflowing_absolute_child_is_inside_the_bounds() {
        let (doc, subject) = doc_with(
            "position: relative; width: 100px; height: 100px; opacity: 0.5",
            &["position: absolute; left: 150px; top: 10px; width: 20px; height: 20px"],
        );
        let child = doc.tree.get(subject).unwrap().children[0];
        let bounds = bounds_of(&doc, subject);
        assert!(
            contains(bounds, box_of(&doc, child)),
            "bounds {bounds:?} must contain the escaping child {:?}",
            box_of(&doc, child)
        );
        // …and must not have given up: this case is analysable, so it should
        // come out as a real rect rather than as the unbounded fallback.
        assert_ne!(bounds, UNBOUNDED);
    }

    /// An outset `box-shadow` is painted outside the border box. The walk
    /// allows `offset ± (blur + spread)`, which is deliberately wider than the
    /// `blur * 0.5 + spread` `paint_box_shadow` actually reaches.
    #[test]
    fn an_outset_box_shadow_is_inside_the_bounds() {
        let (doc, subject) = doc_with(
            "width: 100px; height: 100px; opacity: 0.5; \
             box-shadow: 20px 30px 10px 5px rgba(0, 0, 0, 0.5)",
            &[],
        );
        let own = box_of(&doc, subject);
        let bounds = bounds_of(&doc, subject);
        // What the painter draws: the outermost of its eight concentric layers
        // sits at `offset ± (blur * 0.5 + spread)` = 10 from the offset box.
        let painted = Rect::new(
            own.x0 + 20.0 - 10.0,
            own.y0 + 30.0 - 10.0,
            own.x1 + 20.0 + 10.0,
            own.y1 + 30.0 + 10.0,
        );
        assert!(
            contains(bounds, painted),
            "bounds {bounds:?} must contain the shadow's painted extent {painted:?}"
        );
        assert!(
            contains(bounds, own),
            "bounds {bounds:?} must still contain the border box {own:?}"
        );
    }

    /// A rotated child is not its own layout rect. The walk composes the same
    /// affine paint composes and takes the transformed bounding box, so the
    /// corners that swing outside the rect are inside the bounds.
    #[test]
    fn a_rotated_child_is_inside_the_bounds() {
        let (doc, subject) = doc_with(
            "position: relative; width: 200px; height: 200px; opacity: 0.5",
            &["position: absolute; left: 160px; top: 80px; width: 40px; \
               height: 40px; transform: rotate(45deg)"],
        );
        let child = doc.tree.get(subject).unwrap().children[0];
        let rect = box_of(&doc, child);
        // rotate(45deg) about the default centre origin: the 40x40 box's
        // bounding box grows to 40*sqrt(2), i.e. ~8.28 past each edge.
        let out = (40.0 * std::f64::consts::SQRT_2 - 40.0) / 2.0;
        let swung = Rect::new(rect.x0 - out, rect.y0 - out, rect.x1 + out, rect.y1 + out);
        let bounds = bounds_of(&doc, subject);
        assert!(
            contains(bounds, swung),
            "bounds {bounds:?} must contain the rotated child's bounding box {swung:?}"
        );
    }

    /// The other direction: the bounds have to *shrink* where the subtree is
    /// genuinely clipped. A 500x500 box inside a 50x50 `overflow: hidden` child
    /// contributes 50x50, not 500x500 — otherwise the "optimisation hint" is
    /// no hint at all on any screen with a scroller on it.
    #[test]
    fn overflow_hidden_shrinks_what_a_descendant_contributes() {
        let clipped = |overflow: &str| {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let subject = doc.create_element("div");
            doc.set_attribute(
                subject,
                "style",
                "width: 100px; height: 100px; opacity: 0.5",
            );
            doc.append_child(body, subject);
            let scroller = doc.create_element("div");
            doc.set_attribute(
                scroller,
                "style",
                &format!("width: 50px; height: 50px; overflow: {overflow}"),
            );
            doc.append_child(subject, scroller);
            let big = doc.create_element("div");
            doc.set_attribute(big, "style", "width: 500px; height: 500px; flex-shrink: 0");
            doc.append_child(scroller, big);
            doc.resolve_layout(800.0, 600.0);
            let bounds = bounds_of(&doc, subject.0);
            (bounds, box_of(&doc, subject.0), box_of(&doc, big.0))
        };

        let (visible_bounds, _, big) = clipped("visible");
        assert!(
            contains(visible_bounds, big),
            "with overflow: visible the 500x500 box is painted in full, so the \
             bounds {visible_bounds:?} must contain it ({big:?})"
        );

        let (hidden_bounds, own, _) = clipped("hidden");
        assert_eq!(
            hidden_bounds, own,
            "with overflow: hidden nothing reaches past the subject's own box, \
             so the bounds must not grow to the clipped child's size"
        );
    }

    /// `position: fixed` is viewport content that happens to live in this
    /// markup: `stacking::collect_hoisted` paints it at the body, outside this
    /// layer, so it must not widen these bounds. This is the one case where a
    /// descendant is deliberately left out of the union rather than included.
    #[test]
    fn a_fixed_descendant_is_painted_elsewhere_and_does_not_widen_the_bounds() {
        let (doc, subject) = doc_with(
            "position: relative; width: 100px; height: 100px; opacity: 0.5",
            &["position: fixed; left: 600px; top: 400px; width: 40px; height: 40px"],
        );
        let bounds = bounds_of(&doc, subject);
        assert_eq!(
            bounds,
            box_of(&doc, subject),
            "a fixed descendant is hoisted to the body and painted outside this \
             layer, so it must not appear in the layer's bounds"
        );
    }

    /// `position: sticky` is painted at a position `paint_node` derives by
    /// walking up to the nearest scroll ancestor, which can be above the
    /// element the layer belongs to. The subtree does not contain the answer,
    /// so the walk gives up rather than guessing.
    #[test]
    fn a_sticky_descendant_falls_back_to_unbounded() {
        let (doc, subject) = doc_with(
            "width: 100px; height: 100px; opacity: 0.5",
            &["position: sticky; top: 0px; width: 40px; height: 40px"],
        );
        assert_eq!(bounds_of(&doc, subject), UNBOUNDED);
    }

    /// The walk is per frame, per translucent element, so it is capped. A
    /// subtree wider than the visit budget is not measured at all — it answers
    /// `UNBOUNDED`, which is the behaviour every one of these layers had before
    /// the walk existed.
    #[test]
    fn a_subtree_past_the_visit_budget_falls_back_to_unbounded() {
        let wide: Vec<&str> = vec!["width: 1px; height: 1px"; 600];
        let (doc, subject) = doc_with("width: 100px; height: 100px; opacity: 0.5", &wide);
        assert_eq!(bounds_of(&doc, subject), UNBOUNDED);

        // …and a subtree just inside it is measured normally, so the budget is
        // a backstop and not the common path.
        let narrow: Vec<&str> = vec!["width: 1px; height: 1px"; 20];
        let (doc, subject) = doc_with("width: 100px; height: 100px; opacity: 0.5", &narrow);
        assert_ne!(bounds_of(&doc, subject), UNBOUNDED);
    }

    /// The same backstop in the other dimension.
    #[test]
    fn a_subtree_past_the_depth_cap_falls_back_to_unbounded() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let subject = doc.create_element("div");
        doc.set_attribute(
            subject,
            "style",
            "width: 100px; height: 100px; opacity: 0.5",
        );
        doc.append_child(body, subject);
        let mut parent = subject;
        for _ in 0..40 {
            let child = doc.create_element("div");
            doc.set_attribute(child, "style", "width: 10px; height: 10px");
            doc.append_child(parent, child);
            parent = child;
        }
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(bounds_of(&doc, subject.0), UNBOUNDED);
    }

    /// An element with no box of its own — the zero-area path in `paint_node`,
    /// which pushes a layer for a container collapsed on one axis that still
    /// has overflowing children. It used to pass `UNBOUNDED` unconditionally;
    /// now it gets the children's extent, and falls back to `UNBOUNDED` only
    /// when there is nothing to measure.
    #[test]
    fn a_collapsed_container_is_measured_by_its_children() {
        let (doc, subject) = doc_with(
            "position: relative; width: 100px; height: 0px; opacity: 0.5",
            &["position: absolute; left: 10px; top: 20px; width: 30px; height: 40px"],
        );
        let child = doc.tree.get(subject).unwrap().children[0];
        let bounds = bounds_of(&doc, subject);
        // `assert_ne!` first, and it is not decoration: `UNBOUNDED` contains
        // every rect, so a containment assertion alone passes just as happily
        // when the walk gives up on this branch entirely as when it measures
        // it. This is the one branch where giving up is a *regression* — it is
        // what the branch did before — so the test has to say that the answer
        // is a measured rect and not the fallback.
        assert_ne!(
            bounds, UNBOUNDED,
            "a collapsed container with a measurable child must be measured, \
             not answered with the fallback"
        );
        assert!(
            contains(bounds, box_of(&doc, child)),
            "bounds {bounds:?} must contain the child of a collapsed container \
             {:?}",
            box_of(&doc, child)
        );

        let (doc, subject) = doc_with("width: 100px; height: 0px; opacity: 0.5", &[]);
        assert_eq!(
            bounds_of(&doc, subject),
            UNBOUNDED,
            "with no box and nothing inside it there is nothing to measure, and \
             a degenerate clip would blank whatever paint draws anyway"
        );
    }
    /// An inline-block is reached *only* through the inline formatting context
    /// that positions it: `children` skips it in the ordinary child walk
    /// (`ifc_root == Some(subject)` and it forms no stacking context), exactly
    /// as `paint_children_with_stacking`'s `already_drawn_inline` does, so the
    /// only thing that can put it in the union is the walk over the Parley
    /// layout's inline boxes.
    ///
    /// The assertion is on the inline-block's outset `box-shadow` rather than
    /// on its box, because the box alone is already covered by the IFC's own
    /// measured extent — a test written on the box passes with the inline-box
    /// walk deleted and proves nothing. The shadow reaches past the line box,
    /// so only the per-node measurement of the inline-block itself can see it.
    #[test]
    fn an_inline_block_is_reached_through_its_ifc() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let subject = doc.create_element("div");
        doc.set_attribute(subject, "style", "width: 200px; height: 60px; opacity: 0.5");
        doc.append_child(body, subject);
        let text = doc.create_text("hi ");
        doc.append_child(subject, text);
        let inline_block = doc.create_element("span");
        doc.set_attribute(
            inline_block,
            "style",
            "display: inline-block; width: 20px; height: 20px; \
             box-shadow: 0 0 0 40px rgba(0, 0, 0, 0.5)",
        );
        doc.append_child(subject, inline_block);
        doc.resolve_layout(800.0, 600.0);

        // Precondition: the ordinary child walk really does skip it, so the
        // assertion below is about the IFC path and nothing else.
        let ib = doc.tree.get(inline_block.0).unwrap();
        assert_eq!(
            ib.ifc_root,
            Some(subject.0),
            "precondition: the inline-block is positioned by the subject's IFC"
        );
        assert!(
            !ib.creates_stacking_context(),
            "precondition: it forms no stacking context, so the ordinary child \
             walk skips it as already drawn inline"
        );

        let ib_box = box_of(&doc, inline_block.0);
        let spread = Rect::new(
            ib_box.x0 - 40.0,
            ib_box.y0 - 40.0,
            ib_box.x1 + 40.0,
            ib_box.y1 + 40.0,
        );
        let bounds = bounds_of(&doc, subject.0);
        assert!(
            contains(bounds, spread),
            "bounds {bounds:?} must contain the inline-block's shadow {spread:?} \
             — it is reachable only through the IFC's inline boxes"
        );
    }

    /// The layer's *own* transform is applied by the painter to the shape these
    /// bounds become, so the walk must return them in the element's
    /// untransformed space. Composing the root's transform here as well would
    /// rotate the rect twice and hand Vello a clip at the wrong angle.
    #[test]
    fn the_layers_own_transform_is_not_composed_into_its_bounds() {
        let (doc, subject) = doc_with(
            "width: 40px; height: 40px; opacity: 0.5; transform: rotate(45deg)",
            &[],
        );
        assert_eq!(
            bounds_of(&doc, subject),
            box_of(&doc, subject),
            "push_layer applies the node's transform to the bounds shape, so the \
             bounds are the untransformed border box"
        );
    }
}

/// #260: every colour channel the paint pipeline hands a painter is *rounded*
/// to 8 bits, never truncated.
///
/// Truncation is not a rounding-mode preference, it is a systematic one-sided
/// bias: it can only ever move a value down, and box shadows stack eight
/// layers, so eight truncations compound. Every assertion below is an exact
/// byte, and every one of them was one level lighter before.
mod channel_rounding {
    use super::transform_paint::{paint_skia, pixel_at};
    use super::*;
    use rinch_core::dom::NodeId;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// One absolutely-positioned 100x100 div at (100, 100), painted.
    fn painted(style: &str) -> TinySkiaPainter {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let div = doc.create_element("div");
        doc.set_attribute(
            div,
            "style",
            &format!(
                "position: absolute; left: 100px; top: 100px; width: 100px; height: 100px; {style}"
            ),
        );
        doc.append_child(body, div);
        let _: NodeId = div;
        doc.resolve_layout(800.0, 600.0);
        let mut painter = TinySkiaPainter::new(800, 600);
        paint_skia(&mut doc, &mut painter);
        painter
    }

    /// `filter: brightness(b)` overlays black at alpha `1 - b`, which is exact
    /// for a darken: white through `brightness(0.35)` is `255 x 0.35 = 89.25`.
    ///
    /// The overlay alpha is `0.65 x 255 = 165.75`. Truncated to 165 the pixel
    /// came out 90; rounded to 166 it is 89, the byte a browser shows.
    #[test]
    fn brightness_darken_rounds_the_overlay_alpha() {
        let painter = painted("background-color: rgb(255,255,255); filter: brightness(0.35)");
        assert_eq!(
            pixel_at(&painter, 150, 150),
            [89, 89, 89, 255],
            "255 x 0.35 = 89.25; truncating the 165.75 overlay alpha gave 90"
        );
    }

    /// The brighten half of the same overlay: white at alpha `b - 1` over
    /// black, so the pixel *is* the alpha. `0.65 x 255 = 165.75` again.
    #[test]
    fn brightness_brighten_rounds_the_overlay_alpha() {
        let painter = painted("background-color: rgb(0,0,0); filter: brightness(1.65)");
        assert_eq!(
            pixel_at(&painter, 150, 150),
            [166, 166, 166, 255],
            "the overlay alpha is 165.75; truncating gave 165"
        );
    }

    /// A blurred box shadow is eight concentric layers. The outermost band is
    /// covered by exactly *one* of them — the `t = 1.0` layer, whose alpha is
    /// `255 x (1 - 0.7) / 8 = 9.5625` for an opaque shadow — so it reads the
    /// per-layer quantiser directly, with nothing composited on top.
    #[test]
    fn shadow_outer_layer_rounds_its_alpha() {
        let painter = painted("box-shadow: 0 0 40px 0 rgb(0,0,0)");
        assert_eq!(
            pixel_at(&painter, 100 - 19, 150),
            [0, 0, 0, 10],
            "the outermost layer's alpha is 9.5625; truncating gave 9"
        );
    }

    /// And the bias compounds where the layers overlap: nearer the box, ten
    /// layers' worth of truncation used to add up to three whole levels of
    /// missing shadow.
    #[test]
    fn shadow_stacked_layers_do_not_compound_a_truncation_bias() {
        let painter = painted("box-shadow: 0 0 40px 0 rgb(0,0,0)");
        assert_eq!(
            pixel_at(&painter, 100 - 10, 150),
            [0, 0, 0, 69],
            "eight truncated layers summed to 66"
        );
    }

    /// The shadow's own RGB reaches the layers unchanged. This never broke,
    /// which is exactly why it needs pinning: the per-layer colour used to be
    /// rebuilt from three hand-extracted bytes, and now it is the shadow colour
    /// with its alpha scaled — a rebuild that dropped the hue would be
    /// invisible to every other test here, all of which use a black shadow.
    #[test]
    fn a_coloured_shadow_keeps_its_hue_in_every_layer() {
        let painter = painted("box-shadow: 0 0 40px 0 rgb(255,0,0)");
        assert_eq!(
            pixel_at(&painter, 100 - 19, 150),
            [10, 0, 0, 10],
            "premultiplied red at the outermost layer's alpha"
        );
    }

    /// A translucent shadow's own alpha *scales* each layer's alpha; it does
    /// not merely set the colour. Every other shadow case here is opaque, where
    /// scaling and setting give the same byte — so this is the only assertion
    /// that can tell them apart.
    #[test]
    fn a_translucent_shadow_scales_every_layer_by_its_own_alpha() {
        let painter = painted("box-shadow: 0 0 40px 0 rgba(0,0,0,0.5)");
        assert_eq!(
            pixel_at(&painter, 100 - 19, 150),
            [0, 0, 0, 5],
            "(128/255) x 9.5625 = 4.80; ignoring the shadow's alpha would give 10"
        );
    }

    /// A shadow colour is already 8-bit-quantised before paint ever sees it,
    /// which is why removing the `to_rgba8` re-snap in `paint_shadows` is
    /// redundancy removal rather than a precision gain.
    ///
    /// `rgb(0.5% 99.5% 33.3%)` is as far from byte-aligned as CSS lets you
    /// write; it paints identically to the `rgb(1, 254, 85)` it rounds to,
    /// because `color_from_absolute` rounded it on the way into
    /// `ComputedStyle`. So no input exists that the re-snap could have
    /// changed — including the `color-mix()` and percentage cases you would
    /// expect to be the counterexamples.
    #[test]
    fn a_shadow_colour_is_already_quantised_before_paint() {
        let percent = painted("box-shadow: 0 0 40px 0 rgb(0.5% 99.5% 33.3%)");
        let bytes = painted("box-shadow: 0 0 40px 0 rgb(1, 254, 85)");
        assert_eq!(
            pixel_at(&percent, 100 - 19, 150),
            pixel_at(&bytes, 100 - 19, 150)
        );
        assert_eq!(
            pixel_at(&percent, 100 - 10, 150),
            pixel_at(&bytes, 100 - 10, 150)
        );
    }

    /// `brightness()` has no upper bound in CSS, but an overlay alpha does.
    /// Anything at or past `brightness(2)` is a fully opaque white wash.
    #[test]
    fn an_out_of_range_brightness_clamps_the_overlay_alpha() {
        let painter = painted("background-color: rgb(0,0,0); filter: brightness(3)");
        assert_eq!(
            pixel_at(&painter, 150, 150),
            [255, 255, 255, 255],
            "alpha 2.0 must clamp to 1.0, not wrap"
        );
    }

    /// A faint shadow used to paint **nothing at all** — not a dim shadow, an
    /// absent one.
    ///
    /// The per-layer skip threshold was `(alpha * 255.0) as u8 == 0`, which
    /// truncates, so it discarded every layer whose alpha was under a *whole*
    /// level rather than under half of one. `alpha_scale` peaks at 0.114, so
    /// for a faint shadow that is every layer there is: at `rgba(0,0,0,0.03)`
    /// and below, the entire 800x600 framebuffer came back zero.
    ///
    /// Nothing in the suite noticed — every other shadow test here is opaque
    /// or half-opaque, well clear of the threshold — so this is the third
    /// blind spot of the same kind as `a_translucent_shadow_...`: an input
    /// class no test reached, rather than a line no test covered.
    #[test]
    fn a_faint_shadow_is_painted_rather_than_skipped() {
        for alpha in ["0.02", "0.025", "0.03"] {
            let painter = painted(&format!("box-shadow: 0 0 40px 0 rgba(0,0,0,{alpha})"));
            let lit = painter.pixels().iter().filter(|b| **b != 0).count();
            assert!(
                lit > 0,
                "`rgba(0,0,0,{alpha})` must paint something; the whole buffer was blank"
            );
            assert_eq!(
                pixel_at(&painter, 100 - 2, 150)[3],
                match alpha {
                    "0.02" => 2,
                    "0.025" => 3,
                    _ => 5,
                },
                "just outside the box, 2px out (alpha = {alpha})"
            );
        }
    }

    /// The guard the truncation used to provide by accident: a layer whose
    /// alpha rounds to zero is skipped rather than filled. A fully transparent
    /// shadow must paint nothing at all.
    #[test]
    fn a_transparent_shadow_still_paints_nothing() {
        let painter = painted("box-shadow: 0 0 40px 0 rgba(0,0,0,0)");
        for dx in [1u32, 10, 19] {
            assert_eq!(
                pixel_at(&painter, 100 - dx, 150),
                [0, 0, 0, 0],
                "a zero-alpha shadow must not tint anything (dx = {dx})"
            );
        }
    }
}
