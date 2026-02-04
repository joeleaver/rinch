//! Vello scene building for rinch-dom.
//!
//! Walks the node tree and emits Vello drawing commands
//! for backgrounds, borders, and text.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Rect, RoundedRect, Stroke};
use peniko::{Brush, Fill};
use vello::Scene;

use crate::computed_style::OverflowValue;
use crate::layout::parse_color;
use crate::node::{Node, NodeKind, NodeTree, RawNodeId};
use crate::text_query::caret_position_for_offset;

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
            if let Some(shadow_str) = get_style_property(node, "box-shadow") {
                paint_box_shadow(scene, &shadow_str, x, y, w, h, scale, node);
            }

            // Get opacity from computed style and push layer if needed
            let opacity = node.computed_style.opacity;
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

            // Get border-radius from computed style (use average of all 4 corners)
            // Resolve percentage values against element dimensions
            let radius = {
                let cs = &node.computed_style;
                // For percentage border-radius, resolve against min(width, height) for uniform corners
                let resolve_size = node.layout.width.min(node.layout.height);
                let tl = cs.border_radius_top_left.resolve(resolve_size);
                let tr = cs.border_radius_top_right.resolve(resolve_size);
                let br = cs.border_radius_bottom_right.resolve(resolve_size);
                let bl = cs.border_radius_bottom_left.resolve(resolve_size);
                let avg = (tl + tr + br + bl) / 4.0;
                avg as f64 * scale
            };

            // Get background-color from computed style
            if let Some(bg_color) = node.computed_style.background_color {
                if radius > 0.0 {
                    let rrect = RoundedRect::from_rect(rect, radius);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rrect);
                } else {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, bg_color, None, &rect);
                }
            }

            // Get border from computed style
            let border_width = node.computed_style.border_top_width.to_px();
            let border_color = node.computed_style.border_color;

            if let Some(bc) = border_color {
                let bw = border_width;
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

            // Render input element value
            if matches!(node.tag(), Some("input" | "textarea")) {
                paint_input_value(node, scene, scale, x, y, w, h, font_cx, layout_cx);
            }

            // Handle overflow clipping from computed style
            let overflow_y = node.computed_style.overflow_y;
            let clips = matches!(overflow_y, OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto);

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
                let child_ids: Vec<usize> = node.children.iter().copied().collect();
                for child_id in child_ids {
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
                let child_ids: Vec<usize> = node.children.iter().copied().collect();
                for child_id in child_ids {
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

            // Paint scrollbar overlay for scroll containers
            if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
                let node = tree.get(node_id).unwrap(); // re-borrow after children done
                let mut content_height: f64 = 0.0;
                for &child_id in &node.children {
                    if let Some(child) = tree.get(child_id) {
                        let bottom = (child.layout.y + child.layout.height) as f64 * scale;
                        if bottom > content_height {
                            content_height = bottom;
                        }
                    }
                }
                if content_height > h {
                    let scrollbar_width = 6.0 * scale;
                    let scrollbar_margin = 2.0 * scale;
                    let scrollbar_x = x + w - scrollbar_width - scrollbar_margin;

                    // Thumb sizing
                    let visible_ratio = h / content_height;
                    let max_scroll = content_height - h;
                    let scroll_ratio = if max_scroll > 0.0 {
                        (node.scroll_offset.1 * scale / max_scroll).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };

                    let track_height = h - scrollbar_margin * 2.0;
                    let thumb_height = (track_height * visible_ratio).max(20.0 * scale);
                    let thumb_travel = track_height - thumb_height;
                    let thumb_y = y + scrollbar_margin + thumb_travel * scroll_ratio;

                    let thumb_rect = RoundedRect::from_rect(
                        Rect::new(
                            scrollbar_x,
                            thumb_y,
                            scrollbar_x + scrollbar_width,
                            thumb_y + thumb_height,
                        ),
                        scrollbar_width * 0.5,
                    );
                    let thumb_color = AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, 0.4_f32]);
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &Brush::Solid(thumb_color.into()), None, &thumb_rect);
                }
            }

            if has_opacity {
                scene.pop_layer();
            }
        }

        NodeKind::Text(text_data) => {
            if text_data.content.is_empty() {
                return;
            }

            // Use cached layout if available (built after Taffy layout with final widths)
            if let Some(cached_layout) = &node.cached_text_parley {
                // Layout is already aligned during caching, use it directly
                render_text(scene, cached_layout, x, y);
                return;
            }

            // Fallback: build layout on demand (should not happen with caching)
            let parent_computed = node.parent
                .and_then(|p| tree.get(p))
                .map(|p| &p.computed_style);

            let font_size = parent_computed.map(|s| s.font_size).unwrap_or(16.0);
            let font_weight = parent_computed.map(|s| s.font_weight).unwrap_or(400.0);
            let font_family = parent_computed
                .map(|s| if s.font_family.is_empty() { "sans-serif".to_string() } else { s.font_family.clone() })
                .unwrap_or_else(|| "sans-serif".to_string());

            let color = parent_computed
                .and_then(|s| s.color)
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
            if let Some(lh) = parent_computed.and_then(|s| s.line_height.to_parley()) {
                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
            }

            let mut text_layout = builder.build(&text_data.content);

            // Text was measured without width constraint to get natural width.
            // Paint should use the same (no constraint) to avoid re-wrapping.
            // The parent element's width constrains positioning, not text wrapping.
            text_layout.break_all_lines(None);

            // Read text-align from parent's computed style
            let alignment = parent_computed
                .map(|s| s.text_align.to_parley())
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

    // Get border-radius from computed style (use average of all 4 corners)
    // Resolve percentage values against element dimensions
    let radius = {
        let cs = &node.computed_style;
        let resolve_size = node.layout.width.min(node.layout.height);
        let tl = cs.border_radius_top_left.resolve(resolve_size);
        let tr = cs.border_radius_top_right.resolve(resolve_size);
        let br = cs.border_radius_bottom_right.resolve(resolve_size);
        let bl = cs.border_radius_bottom_left.resolve(resolve_size);
        let avg = (tl + tr + br + bl) / 4.0;
        avg as f64 * scale
    };

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
    let child_ids: Vec<usize> = node.children.iter().copied().collect();
    for child_id in child_ids {
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
            // Check computed_style.color
            if let Some(c) = n.computed_style.color {
                return c;
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

            // DEBUG: Print font info for first run only (to avoid spam)
            {
                use std::sync::atomic::{AtomicBool, Ordering};
                static PRINTED: AtomicBool = AtomicBool::new(false);
                if !PRINTED.swap(true, Ordering::SeqCst) {
                    use read_fonts::TableProvider;
                    if let Ok(font_ref) = read_fonts::FontRef::from_index(font.data.as_ref(), font.index) {
                        if let Ok(name_table) = font_ref.name() {
                            // Try to get font family name (nameID 1) or full name (nameID 4)
                            for record in name_table.name_record().iter() {
                                if record.name_id().to_u16() == 1 || record.name_id().to_u16() == 4 {
                                    if let Ok(name) = record.string(name_table.string_data()) {
                                        eprintln!("[DEBUG] Font selected: {:?} (nameID={})", name.to_string(), record.name_id().to_u16());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
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

/// Get a CSS style property value from inline styles.
///
/// NOTE: This function is deprecated. Most properties should be read from
/// `node.computed_style` directly. This function is only used for properties
/// not yet in ComputedStyle (like box-shadow).
fn get_style_property(node: &Node, property: &str) -> Option<String> {
    // Check computed_style_str (used during style resolution)
    if !node.computed_style_str.is_empty() {
        for part in node.computed_style_str.split(';') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once(':') {
                if key.trim() == property {
                    return Some(value.trim().to_string());
                }
            }
        }
        return None;
    }

    // Fallback: parse inline style attribute directly
    if let Some(style_str) = node.attributes.get("style") {
        for part in style_str.split(';') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once(':') {
                if key.trim() == property {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

/// Parse a pixel value like "10px" or "10" to f32.
fn parse_px(value: &str) -> Option<f32> {
    let v = value.trim().strip_suffix("px").unwrap_or(value.trim());
    v.parse().ok()
}

/// Paint the value of an input element.
fn paint_input_value(
    node: &Node,
    scene: &mut Scene,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    // Get the value or placeholder
    let value = node.attributes.get("value").map(|s| s.as_str()).unwrap_or("");
    let placeholder = node.attributes.get("placeholder").map(|s| s.as_str()).unwrap_or("");

    // Check if this input is focused
    let is_focused = node.attributes.get("data-focused").map(|s| s == "true").unwrap_or(false);
    let cursor_pos = node.attributes.get("data-cursor-pos")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let selection_start = node.attributes.get("data-selection-start")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(cursor_pos);
    let cursor_visible = node.attributes.get("data-cursor-visible")
        .map(|s| s == "true")
        .unwrap_or(true);

    let (text, is_placeholder) = if value.is_empty() && !placeholder.is_empty() {
        (placeholder, true)
    } else {
        (value, false)
    };

    // Get font properties from computed style
    let font_size = node.computed_style.font_size;
    let font_weight = node.computed_style.font_weight;
    let font_family = if node.computed_style.font_family.is_empty() {
        "sans-serif".to_string()
    } else {
        node.computed_style.font_family.clone()
    };

    // Get text color from computed style - dimmed for placeholder
    let base_color = node.computed_style.color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255)); // #212529

    let color = if is_placeholder {
        // Placeholder uses dimmed color
        AlphaColor::<Srgb>::from_rgba8(134, 142, 150, 255) // #868e96
    } else {
        base_color
    };

    // Get padding from computed style
    let padding_left = node.computed_style.padding_left.to_px() as f64 * scale;
    let padding_top = node.computed_style.padding_top.to_px() as f64 * scale;

    // Check if this is a textarea (multi-line) or input (single-line)
    let is_textarea = node.tag() == Some("textarea");

    // Build text layout
    let scaled_font_size = font_size * scale as f32;
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
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

    let mut text_layout = builder.build(text);
    text_layout.break_all_lines(Some((w - padding_left * 2.0) as f32));

    // For textarea, align to top with padding; for input, center vertically
    let text_height = text_layout.height() as f64;
    let text_y = if is_textarea {
        y + padding_top
    } else {
        y + (h - text_height) / 2.0
    };
    let text_x = x + padding_left;


    // Draw selection highlight if there's a selection
    if is_focused && cursor_pos != selection_start && !is_placeholder && !text.is_empty() {
        let sel_start_byte = cursor_pos.min(selection_start).min(text.len());
        let sel_end_byte = cursor_pos.max(selection_start).min(text.len());

        let (start_x, start_y) = crate::text_query::caret_position_for_offset_layout(&text_layout, sel_start_byte);
        let (end_x, end_y) = crate::text_query::caret_position_for_offset_layout(&text_layout, sel_end_byte);

        let sel_color = AlphaColor::<Srgb>::from_rgba8(51, 154, 240, 100); // Blue with alpha
        let line_height = scaled_font_size as f64 * 1.2;

        if (start_y - end_y).abs() < 0.1 {
            // Same line - draw single rectangle
            let sel_x = text_x + start_x as f64;
            let sel_width = (end_x - start_x) as f64;
            let sel_y = text_y + start_y as f64;

            let sel_rect = vello::kurbo::Rect::new(sel_x, sel_y, sel_x + sel_width, sel_y + line_height);
            scene.fill(vello::peniko::Fill::NonZero, vello::kurbo::Affine::IDENTITY, sel_color, None, &sel_rect);
        } else {
            // Multi-line selection - draw rectangles for each line
            let content_width = (w - padding_left * 2.0) as f32;

            for line in text_layout.lines() {
                let line_metrics = line.metrics();
                let line_top = line_metrics.baseline - line_metrics.ascent;

                // Skip lines outside selection range
                if line_top + line_metrics.line_height < start_y || line_top > end_y {
                    continue;
                }

                let rect_y = text_y + line_top as f64;
                let (rect_start_x, rect_end_x) = if (line_top - start_y).abs() < 0.1 {
                    // First line of selection: from start_x to end of line
                    (start_x, content_width)
                } else if (line_top - end_y).abs() < 0.1 {
                    // Last line of selection: from start of line to end_x
                    (0.0, end_x)
                } else {
                    // Middle line: full width
                    (0.0, content_width)
                };

                let sel_rect = vello::kurbo::Rect::new(
                    text_x + rect_start_x as f64,
                    rect_y,
                    text_x + rect_end_x as f64,
                    rect_y + line_height,
                );
                scene.fill(vello::peniko::Fill::NonZero, vello::kurbo::Affine::IDENTITY, sel_color, None, &sel_rect);
            }
        }
    }

    // Render text
    if !text.is_empty() {
        render_text(scene, &text_layout, text_x, text_y);
    }

    // Draw cursor/caret if focused and visible
    if is_focused && cursor_visible {
        let caret_pos = cursor_pos.min(text.len());
        let (caret_offset_x, caret_offset_y) = if text.is_empty() {
            (0.0, 0.0)
        } else {
            crate::text_query::caret_position_for_offset_layout(&text_layout, caret_pos)
        };
        let caret_x = text_x + caret_offset_x as f64;
        let caret_y = text_y + caret_offset_y as f64;

        let caret_height = scaled_font_size as f64 * 1.2;

        // Draw caret line
        let caret_color = base_color;
        let caret_rect = vello::kurbo::Rect::new(caret_x, caret_y, caret_x + 1.5 * scale, caret_y + caret_height);
        scene.fill(vello::peniko::Fill::NonZero, vello::kurbo::Affine::IDENTITY, caret_color, None, &caret_rect);
    }
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

}
