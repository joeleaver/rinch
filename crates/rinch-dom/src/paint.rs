//! Vello scene building for rinch-dom.
//!
//! Walks the node tree and emits Vello drawing commands
//! for backgrounds, borders, and text.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Rect, RoundedRect, Stroke};
use peniko::{Brush, Fill};
use vello::Scene;

use crate::layout::parse_color;
use crate::node::{Node, NodeKind, NodeTree, RawNodeId};
use crate::stylesheet::Stylesheet;

/// Paint the entire document to a Vello scene.
///
/// `scale` is the DPI scale factor (1.0 = 96dpi).
/// `viewport` is the viewport size in physical pixels.
pub fn paint_document(
    tree: &NodeTree,
    scene: &mut Scene,
    scale: f64,
    _viewport: (f32, f32),
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    scene.reset();
    paint_node(tree, tree.body_id, scene, scale, 0.0, 0.0, font_cx, layout_cx);
}

fn paint_node(
    tree: &NodeTree,
    node_id: RawNodeId,
    scene: &mut Scene,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // Skip hidden elements (display: none, style/script tags)
    if let NodeKind::Element(ref el) = node.kind {
        if matches!(el.tag.as_str(), "style" | "script" | "head" | "meta" | "link") {
            return;
        }
    }
    let layout = &node.layout;

    // Skip zero-size elements (display: none produces 0x0 layout)
    if layout.width == 0.0 && layout.height == 0.0 {
        return;
    }

    // Absolute position of this node
    let x = offset_x + layout.x as f64 * scale;
    let y = offset_y + layout.y as f64 * scale;
    let w = layout.width as f64 * scale;
    let h = layout.height as f64 * scale;

    match &node.kind {
        NodeKind::Element(el) if el.tag == "svg" => {
            paint_svg(tree, node, scene, scale, x, y, w, h);
            return;
        }
        NodeKind::Element(_) => {
            let rect = Rect::new(x, y, x + w, y + h);

            // Parse and render box-shadow
            if let Some(shadow_str) = get_style_property(node, &tree.stylesheet, "box-shadow") {
                paint_box_shadow(scene, &shadow_str, x, y, w, h, scale, node, &tree.stylesheet);
            }

            // Parse opacity and push layer if needed
            let opacity = get_style_property(node, &tree.stylesheet, "opacity")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1.0);
            let has_opacity = opacity < 1.0;
            if has_opacity {
                scene.push_layer(
                    Fill::NonZero,
                    peniko::Mix::Normal,
                    opacity,
                    Affine::IDENTITY,
                    &rect,
                );
            }

            // Parse border-radius for rounded rect usage
            let radius = get_style_property(node, &tree.stylesheet,"border-radius")
                .and_then(|v| parse_px(&v))
                .unwrap_or(0.0) as f64
                * scale;

            // Parse background-color from style
            if let Some(bg_color) = get_style_property(node, &tree.stylesheet,"background-color")
                .and_then(|v| parse_color(&v))
            {
                if radius > 0.0 {
                    let rrect = RoundedRect::from_rect(rect, radius);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rrect);
                } else {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rect);
                }
            }

            // Parse border
            let border_width = get_style_property(node, &tree.stylesheet,"border-width")
                .and_then(|v| parse_px(&v))
                .or_else(|| {
                    get_style_property(node, &tree.stylesheet,"border").and_then(|v| parse_border_width(&v))
                });
            let border_color = get_style_property(node, &tree.stylesheet,"border-color")
                .and_then(|v| parse_color(&v))
                .or_else(|| {
                    get_style_property(node, &tree.stylesheet,"border").and_then(|v| parse_border_color(&v))
                });

            if let (Some(bw), Some(bc)) = (border_width, border_color) {
                if bw > 0.0 {
                    let stroke = Stroke::new(bw as f64 * scale);
                    let half = bw as f64 * scale * 0.5;
                    let border_rect = Rect::new(x + half, y + half, x + w - half, y + h - half);

                    if radius > 0.0 {
                        let rrect = RoundedRect::from_rect(border_rect, radius);
                        scene.stroke(&stroke, Affine::IDENTITY, bc, None, &rrect);
                    } else {
                        scene.stroke(&stroke, Affine::IDENTITY, bc, None, &border_rect);
                    }
                }
            }

            // Handle overflow clipping
            let overflow = get_style_property(node, &tree.stylesheet,"overflow").unwrap_or_default();
            let clips =
                overflow == "hidden" || overflow == "scroll" || overflow == "auto";

            if clips {
                scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
            }

            // Check if this is an IFC root with a cached inline layout
            if let Some(inline_layout) = &node.text_layout {
                // Paint inline content from the Parley layout
                paint_inline_layout(tree, scene, scale, x, y, inline_layout, font_cx, layout_cx);

                // Still paint non-inline (block) children normally
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                let children: Vec<_> = node.children.clone();
                for child_id in children {
                    let child = match tree.get(child_id) {
                        Some(c) => c,
                        None => continue,
                    };
                    // Skip inline children — they're painted via the Parley layout
                    if child.ifc_root.is_some() {
                        continue;
                    }
                    paint_node(
                        tree,
                        child_id,
                        scene,
                        scale,
                        x - scroll_x,
                        y - scroll_y,
                        font_cx,
                        layout_cx,
                    );
                }
            } else {
                // Normal paint path: recurse into all children
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                let children: Vec<_> = node.children.clone();
                for child_id in children {
                    paint_node(
                        tree,
                        child_id,
                        scene,
                        scale,
                        x - scroll_x,
                        y - scroll_y,
                        font_cx,
                        layout_cx,
                    );
                }
            }

            if clips {
                scene.pop_layer();
            }

            if has_opacity {
                scene.pop_layer();
            }
        }

        NodeKind::Text(text_data) => {
            if text_data.content.is_empty() {
                return;
            }

            // Read all font properties from parent's computed_style_str
            // (same source as Taffy measurement via sync_text_contexts)
            let parent_style = node.parent
                .and_then(|p| tree.get(p))
                .map(|p| {
                    if !p.computed_style_str.is_empty() {
                        &p.computed_style_str as &str
                    } else {
                        p.attributes.get("style").map(|s| s.as_str()).unwrap_or("")
                    }
                })
                .unwrap_or("");

            let font_size = crate::layout::parse_font_size(parent_style).unwrap_or(16.0);
            let font_weight = crate::layout::parse_font_weight(parent_style).unwrap_or(400.0);
            let font_family = crate::layout::parse_font_family(parent_style)
                .unwrap_or_else(|| "sans-serif".to_string());

            let color = crate::layout::parse_style_string(parent_style)
                .get("color")
                .and_then(|v| parse_color(v))
                .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));

            // Build Parley layout with identical parameters to measurement
            let scaled_font_size = font_size * scale as f32;
            let mut builder =
                layout_cx.ranged_builder(font_cx, &text_data.content, 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(scaled_font_size));
            builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(color)));
            builder.push_default(parley::style::StyleProperty::FontStack(
                parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family)),
            ));
            if (font_weight - 400.0).abs() > 1.0 {
                builder.push_default(parley::style::StyleProperty::FontWeight(
                    parley::style::FontWeight::new(font_weight),
                ));
            }
            let line_height_css = crate::layout::parse_line_height_css(parent_style)
                .unwrap_or_default();
            if let Some(lh) = crate::layout::css_line_height_to_parley(&line_height_css) {
                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
            }

            let mut text_layout = builder.build(&text_data.content);

            // Use the text node's own layout width for text wrapping
            // (matches the width Taffy computed during measurement)
            let max_width = if layout.width > 0.0 {
                Some(layout.width * scale as f32)
            } else {
                None
            };
            text_layout.break_all_lines(max_width);

            // Read text-align from parent's computed style
            let alignment = crate::layout::parse_style_string(parent_style)
                .get("text-align")
                .map(|a| match a.as_str() {
                    "center" => parley::layout::Alignment::Center,
                    "right" | "end" => parley::layout::Alignment::End,
                    "justify" => parley::layout::Alignment::Justify,
                    _ => parley::layout::Alignment::Start,
                })
                .unwrap_or(parley::layout::Alignment::Start);
            text_layout.align(alignment, parley::layout::AlignmentOptions::default());

            // Render text glyphs to scene
            render_text(scene, &text_layout, x, y);
        }

        _ => {} // Document, Comment -- invisible
    }
}

/// Paint inline content from a cached InlineLayout (IFC root).
///
/// Renders glyph runs directly from the Parley layout. Inline boxes
/// (inline-block elements) are painted by looking up the child node
/// and painting it at the Parley-computed position.
fn paint_inline_layout(
    tree: &NodeTree,
    scene: &mut Scene,
    scale: f64,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    // Render the Parley layout at the IFC root's position
    // Scale is already applied to font sizes during layout building
    render_text(scene, &inline_layout.layout, parent_x, parent_y);

    // Paint inline-block boxes by looking them up in tree and painting
    for line in inline_layout.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::InlineBox(positioned_box) = item {
                let child_id = positioned_box.id as usize;
                paint_node(tree, child_id, scene, scale, parent_x, parent_y, font_cx, layout_cx);
            }
        }
    }
}

/// Paint a CSS box-shadow effect.
///
/// Parses shadow format: `offset-x offset-y blur-radius color`
/// Approximates blur by drawing expanded, semi-transparent rounded rects.
fn paint_box_shadow(
    scene: &mut Scene,
    shadow_str: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
    node: &crate::node::Node,
    stylesheet: &Stylesheet,
) {
    // Parse box-shadow: offset-x offset-y blur-radius [spread-radius] color
    // May also have "none" or multiple shadows separated by commas
    if shadow_str == "none" {
        return;
    }

    // Handle single shadow (multi-shadow not yet supported)
    let parts: Vec<&str> = shadow_str.split_whitespace().collect();
    if parts.len() < 3 {
        return;
    }

    let offset_x = parse_px(parts[0]).unwrap_or(0.0) as f64 * scale;
    let offset_y = parse_px(parts[1]).unwrap_or(0.0) as f64 * scale;
    let blur = parse_px(parts[2]).unwrap_or(0.0) as f64 * scale;

    // Remaining parts form the color (could be "rgba(0, 0, 0, 0.1)" spread across multiple parts)
    let (spread, color_start) = if parts.len() > 4 {
        // Check if parts[3] is a number (spread-radius) or start of color
        if parse_px(parts[3]).is_some() && !parts[3].starts_with('#') && !parts[3].starts_with("rgb") {
            (parse_px(parts[3]).unwrap_or(0.0) as f64 * scale, 4)
        } else {
            (0.0, 3)
        }
    } else {
        (0.0, 3)
    };

    let color_str = parts[color_start..].join(" ");
    let color = parse_color(&color_str).unwrap_or_else(|| {
        AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 40) // default shadow color
    });

    let radius = get_style_property(node, stylesheet, "border-radius")
        .and_then(|v| parse_px(&v))
        .unwrap_or(0.0) as f64
        * scale;

    // Draw shadow as expanded rounded rect behind the element
    // Approximate blur with multiple layers of decreasing opacity
    let expand = blur * 0.5 + spread;
    let shadow_rect = Rect::new(
        x + offset_x - expand,
        y + offset_y - expand,
        x + w + offset_x + expand,
        y + h + offset_y + expand,
    );

    if blur > 0.0 {
        // Multi-layer blur approximation: 3 layers with increasing expansion
        let layers = 3;
        for i in 0..layers {
            let t = (i as f64 + 1.0) / layers as f64;
            let layer_expand = expand * t;
            let layer_rect = Rect::new(
                x + offset_x - layer_expand,
                y + offset_y - layer_expand,
                x + w + offset_x + layer_expand,
                y + h + offset_y + layer_expand,
            );
            let alpha_scale = (1.0 - t * 0.6) / layers as f64;
            let layer_color = AlphaColor::<Srgb>::from_rgba8(
                0, 0, 0,
                (color.components[3] * alpha_scale as f32 * 255.0) as u8,
            );
            if radius > 0.0 {
                let rrect = RoundedRect::from_rect(layer_rect, radius + layer_expand);
                scene.fill(Fill::NonZero, Affine::IDENTITY, layer_color, None, &rrect);
            } else {
                scene.fill(Fill::NonZero, Affine::IDENTITY, layer_color, None, &layer_rect);
            }
        }
    } else {
        // No blur: simple offset shadow
        if radius > 0.0 {
            let rrect = RoundedRect::from_rect(shadow_rect, radius);
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rrect);
        } else {
            scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &shadow_rect);
        }
    }
}

// =============================================================================
// SVG rendering
// =============================================================================

/// Paint an `<svg>` element and its children (path, rect, circle, line, polyline).
fn paint_svg(
    tree: &NodeTree,
    node: &Node,
    scene: &mut Scene,
    _scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    // Parse viewBox (default "0 0 24 24" for most icon SVGs)
    let viewbox = node.attributes.get("viewBox")
        .and_then(|v| parse_viewbox(v))
        .unwrap_or((0.0, 0.0, 24.0, 24.0));

    let (vb_x, vb_y, vb_w, vb_h) = viewbox;
    if vb_w <= 0.0 || vb_h <= 0.0 || w <= 0.0 || h <= 0.0 {
        return;
    }

    // Compute transform: scale viewBox to fit layout bounds, then translate to position
    let sx = w / vb_w;
    let sy = h / vb_h;
    let s = sx.min(sy); // uniform scale (preserveAspectRatio default)
    let tx = x + (w - vb_w * s) * 0.5 - vb_x * s;
    let ty = y + (h - vb_h * s) * 0.5 - vb_y * s;
    let transform = Affine::new([s, 0.0, 0.0, s, tx, ty]);

    // Resolve "currentColor" — walk up the tree to find a `color` CSS property
    let current_color = resolve_current_color(tree, node);

    // Parse SVG-level fill/stroke defaults
    let svg_fill = node.attributes.get("fill").map(|v| v.as_str());
    let svg_stroke = node.attributes.get("stroke").map(|v| v.as_str());
    let svg_stroke_width = node.attributes.get("stroke-width")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0);
    let svg_stroke_linecap = node.attributes.get("stroke-linecap")
        .map(|v| parse_linecap(v))
        .unwrap_or(Cap::Butt);
    let svg_stroke_linejoin = node.attributes.get("stroke-linejoin")
        .map(|v| parse_linejoin(v))
        .unwrap_or(Join::Miter);

    // Paint each child SVG element
    let children: Vec<_> = node.children.clone();
    for child_id in children {
        let Some(child) = tree.get(child_id) else { continue };
        let NodeKind::Element(ref el) = child.kind else { continue };

        // Per-element fill/stroke (override SVG defaults)
        let fill_attr = child.attributes.get("fill").map(|v| v.as_str()).or(svg_fill);
        let stroke_attr = child.attributes.get("stroke").map(|v| v.as_str()).or(svg_stroke);
        let stroke_w = child.attributes.get("stroke-width")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(svg_stroke_width);
        let linecap = child.attributes.get("stroke-linecap")
            .map(|v| parse_linecap(v))
            .unwrap_or(svg_stroke_linecap);
        let linejoin = child.attributes.get("stroke-linejoin")
            .map(|v| parse_linejoin(v))
            .unwrap_or(svg_stroke_linejoin);

        let fill_color = resolve_svg_color(fill_attr, current_color);
        let stroke_color = resolve_svg_color(stroke_attr, current_color);

        match el.tag.as_str() {
            "path" => {
                if let Some(d) = child.attributes.get("d") {
                    if let Ok(path) = BezPath::from_svg(d) {
                        if let Some(fc) = fill_color {
                            scene.fill(Fill::NonZero, transform, fc, None, &path);
                        }
                        if let Some(sc) = stroke_color {
                            let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                            scene.stroke(&stroke, transform, sc, None, &path);
                        }
                    }
                }
            }
            "rect" => {
                let rx = child.attributes.get("x").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let ry = child.attributes.get("y").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let rw = child.attributes.get("width").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let rh = child.attributes.get("height").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let rect = Rect::new(rx, ry, rx + rw, ry + rh);
                if let Some(fc) = fill_color {
                    scene.fill(Fill::NonZero, transform, fc, None, &rect);
                }
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    scene.stroke(&stroke, transform, sc, None, &rect);
                }
            }
            "circle" => {
                let cx = child.attributes.get("cx").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let cy = child.attributes.get("cy").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let r = child.attributes.get("r").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let circle = peniko::kurbo::Circle::new((cx, cy), r);
                if let Some(fc) = fill_color {
                    scene.fill(Fill::NonZero, transform, fc, None, &circle);
                }
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    scene.stroke(&stroke, transform, sc, None, &circle);
                }
            }
            "line" => {
                let x1 = child.attributes.get("x1").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let y1 = child.attributes.get("y1").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let x2 = child.attributes.get("x2").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let y2 = child.attributes.get("y2").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let line = peniko::kurbo::Line::new((x1, y1), (x2, y2));
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    scene.stroke(&stroke, transform, sc, None, &line);
                }
            }
            "polyline" | "polygon" => {
                if let Some(points_str) = child.attributes.get("points") {
                    if let Some(path) = parse_polyline_points(points_str, el.tag == "polygon") {
                        if let Some(fc) = fill_color {
                            scene.fill(Fill::NonZero, transform, fc, None, &path);
                        }
                        if let Some(sc) = stroke_color {
                            let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                            scene.stroke(&stroke, transform, sc, None, &path);
                        }
                    }
                }
            }
            _ => {} // Unsupported SVG elements (text, g, etc.)
        }
    }
}

/// Parse SVG viewBox attribute: "minX minY width height"
fn parse_viewbox(s: &str) -> Option<(f64, f64, f64, f64)> {
    let parts: Vec<f64> = s.split_whitespace()
        .flat_map(|p| p.split(','))
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

/// Resolve `currentColor` by walking up the DOM tree to find a CSS `color` property.
fn resolve_current_color(tree: &NodeTree, node: &Node) -> AlphaColor<Srgb> {
    let mut current = Some(node.id);
    while let Some(id) = current {
        if let Some(n) = tree.get(id) {
            // Check computed style
            if !n.computed_style_str.is_empty() {
                let props = crate::layout::parse_style_string(&n.computed_style_str);
                if let Some(color_str) = props.get("color") {
                    if let Some(c) = parse_color(color_str) {
                        return c;
                    }
                }
            }
            // Check inline style
            if let Some(style) = n.attributes.get("style") {
                let props = crate::layout::parse_style_string(style);
                if let Some(color_str) = props.get("color") {
                    if let Some(c) = parse_color(color_str) {
                        return c;
                    }
                }
            }
            current = n.parent;
        } else {
            break;
        }
    }
    // Default: black
    AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255)
}

/// Resolve an SVG color attribute value, handling "none" and "currentColor".
fn resolve_svg_color(attr: Option<&str>, current_color: AlphaColor<Srgb>) -> Option<AlphaColor<Srgb>> {
    match attr {
        None => None,
        Some("none") => None,
        Some("currentColor") => Some(current_color),
        Some(v) => parse_color(v),
    }
}

/// Parse SVG stroke-linecap attribute.
fn parse_linecap(v: &str) -> Cap {
    match v {
        "round" => Cap::Round,
        "square" => Cap::Square,
        _ => Cap::Butt,
    }
}

/// Parse SVG stroke-linejoin attribute.
fn parse_linejoin(v: &str) -> Join {
    match v {
        "round" => Join::Round,
        "bevel" => Join::Bevel,
        _ => Join::Miter,
    }
}

/// Parse SVG polyline/polygon points attribute into a BezPath.
fn parse_polyline_points(points_str: &str, close: bool) -> Option<BezPath> {
    let nums: Vec<f64> = points_str.split_whitespace()
        .flat_map(|p| p.split(','))
        .filter_map(|p| p.parse().ok())
        .collect();
    if nums.len() < 4 || nums.len() % 2 != 0 {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to((nums[0], nums[1]));
    for i in (2..nums.len()).step_by(2) {
        path.line_to((nums[i], nums[i + 1]));
    }
    if close {
        path.close_path();
    }
    Some(path)
}

/// Render a Parley text layout to a Vello scene.
fn render_text(scene: &mut Scene, layout: &parley::layout::Layout<Brush>, x: f64, y: f64) {
    let transform = Affine::translate((x, y));
    for line in layout.lines() {
        for item in line.items() {
            let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let mut gx = glyph_run.offset();
            let gy = glyph_run.baseline();
            let run = glyph_run.run();
            let font = run.font();
            let font_size = run.font_size();
            let synthesis = run.synthesis();
            let glyph_xform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
            let style = glyph_run.style();
            let brush = style.brush.clone();

            // Track run width for decorations
            let run_x = glyph_run.offset();

            scene
                .draw_glyphs(font)
                .font_size(font_size)
                .transform(transform)
                .glyph_transform(glyph_xform)
                .brush(&brush)
                .hint(true)
                .normalized_coords(run.normalized_coords())
                .draw(
                    Fill::NonZero,
                    glyph_run.glyphs().map(|glyph| {
                        let px = gx + glyph.x;
                        let py = gy - glyph.y;
                        gx += glyph.advance;
                        vello::Glyph {
                            id: glyph.id,
                            x: px,
                            y: py,
                        }
                    }),
                );

            // Draw underline decoration
            if let Some(underline) = &style.underline {
                let run_metrics = run.metrics();
                let offset = underline.offset.unwrap_or(run_metrics.underline_offset);
                let size = underline.size.unwrap_or(run_metrics.underline_size);
                let dec_brush = &underline.brush;
                // Clamp underline closer to baseline — many fonts place it too low.
                // Use at most 1/6 of font_size below baseline.
                let max_offset = font_size / 6.0;
                let clamped_offset = offset.min(max_offset);
                let line_y = (gy + clamped_offset) as f64;
                let run_width = (gx - run_x) as f64;
                let line = peniko::kurbo::Line::new(
                    (run_x as f64, line_y),
                    (run_x as f64 + run_width, line_y),
                );
                let stroke = Stroke::new(size.max(1.0) as f64);
                scene.stroke(&stroke, transform, dec_brush, None, &line);
            }

            // Draw strikethrough decoration
            if let Some(strikethrough) = &style.strikethrough {
                let run_metrics = run.metrics();
                let offset = strikethrough.offset.unwrap_or(run_metrics.strikethrough_offset);
                let size = strikethrough.size.unwrap_or(run_metrics.strikethrough_size);
                let dec_brush = &strikethrough.brush;
                let line_y = (gy - offset) as f64;
                let run_width = (gx - run_x) as f64;
                let line = peniko::kurbo::Line::new(
                    (run_x as f64, line_y),
                    (run_x as f64 + run_width, line_y),
                );
                let stroke = Stroke::new(size.max(1.0) as f64);
                scene.stroke(&stroke, transform, dec_brush, None, &line);
            }
        }
    }
}

/// Get a CSS style property value from a node, checking inline style first,
/// then class-based styles from the stylesheet.
fn get_style_property(node: &Node, stylesheet: &Stylesheet, property: &str) -> Option<String> {
    // Check inline style first (highest priority)
    if let Some(style_str) = node.attributes.get("style") {
        for part in style_str.split(';') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once(':') {
                if key.trim() == property {
                    let value = value.trim().to_string();
                    let resolved = stylesheet.resolve_value(&value);
                    return Some(resolved);
                }
            }
        }
    }

    // Fall back to class-based styles
    if let Some(class_attr) = node.attributes.get("class") {
        let class_props = stylesheet.resolve_class_styles(class_attr);
        if let Some(value) = class_props.get(property) {
            return Some(value.clone());
        }
    }

    None
}

/// Parse a pixel value like "10px" or "10" to f32.
fn parse_px(value: &str) -> Option<f32> {
    let v = value.trim().strip_suffix("px").unwrap_or(value.trim());
    v.parse().ok()
}

/// Parse border shorthand width, e.g. "1px solid #ccc" -> 1.0
fn parse_border_width(value: &str) -> Option<f32> {
    for part in value.split_whitespace() {
        if let Some(px) = parse_px(part) {
            return Some(px);
        }
    }
    None
}

/// Parse border shorthand color, e.g. "1px solid #ccc" -> Color
fn parse_border_color(value: &str) -> Option<peniko::Color> {
    for part in value.split_whitespace() {
        if let Some(c) = parse_color(part) {
            return Some(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_px() {
        assert_eq!(parse_px("10px"), Some(10.0));
        assert_eq!(parse_px("10"), Some(10.0));
        assert_eq!(parse_px("0"), Some(0.0));
        assert_eq!(parse_px("abc"), None);
    }

    #[test]
    fn test_parse_border_width() {
        assert_eq!(parse_border_width("2px solid black"), Some(2.0));
        assert_eq!(parse_border_width("1px"), Some(1.0));
    }

    #[test]
    fn test_parse_border_color() {
        assert!(parse_border_color("2px solid black").is_some());
        assert!(parse_border_color("1px solid #ff0000").is_some());
    }
}
