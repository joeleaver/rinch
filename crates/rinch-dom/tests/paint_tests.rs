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
