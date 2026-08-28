//! Abstract scene building for rinch-dom.
//!
//! Walks the node tree and emits drawing commands via the `Painter` trait
//! for backgrounds, borders, and text.

mod borders;
mod contenteditable;
pub mod image;
pub mod painter;
mod select;
mod svg;
mod text;
pub mod vello_painter;

#[cfg(feature = "software-renderer")]
pub mod skia_painter;

use borders::*;
use contenteditable::*;
use svg::*;
use text::*;

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Rect, RoundedRect, RoundedRectRadii, Shape};
use peniko::{Brush, Fill};

use painter::{BlendMode, Painter};

use crate::computed_style::{
    BackgroundValue, DisplayValue, OverflowValue, PositionValue, VisibilityValue,
};
use crate::node::{Node, NodeKind, NodeTree, RawNodeId};
use crate::stacking::{PaintKind, paints_at_stacking_root, stacking_paint_order};

/// Compute the dirty region (union of all paint-dirty node rects) in physical pixels.
///
/// Returns `None` if no nodes are dirty. Includes both current and previous
/// layout positions so moved/resized nodes get their old area cleared too.
/// Expands the region by a margin to account for anti-aliasing and box-shadows.
pub fn compute_dirty_region(
    tree: &NodeTree,
    scale: f64,
    viewport_w: f64,
    viewport_h: f64,
) -> Option<Rect> {
    if tree.paint_dirty_nodes.is_empty() {
        return None;
    }

    let margin = 4.0; // pixels margin for anti-aliasing / shadows
    let mut region: Option<Rect> = None;

    // Deduplicate — paint_dirty_nodes may have duplicates
    let mut seen = HashSet::new();
    for &node_id in &tree.paint_dirty_nodes {
        if !seen.insert(node_id) {
            continue;
        }

        // Current position. CSS transforms displace where a node renders, so
        // use the transform-aware absolute rect — the region must cover the
        // node's visual position, not its untransformed layout rect (#143).
        let (ax, ay, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
        if let Some(node) = tree.get(node_id) {
            let w = node.layout.width as f64 * scale;
            let h = node.layout.height as f64 * scale;
            if w > 0.0 && h > 0.0 {
                let r = transform.transform_rect_bbox(Rect::new(ax, ay, ax + w, ay + h));
                let r = Rect::new(r.x0 - margin, r.y0 - margin, r.x1 + margin, r.y1 + margin);
                region = Some(region.map_or(r, |prev| prev.union(r)));
            }

            // Previous position (for moved/resized nodes)
            let pw = node.prev_layout.width as f64 * scale;
            let ph = node.prev_layout.height as f64 * scale;
            if (pw > 0.0 && ph > 0.0) && node.prev_layout != node.layout {
                // Approximate old absolute position: use current abs pos adjusted
                // by the difference in layout offsets, under the current transform
                // chain. Not perfectly accurate for deep ancestor layout changes
                // (or a simultaneous transform change), but covers the common case.
                let dx = (node.layout.x - node.prev_layout.x) as f64 * scale;
                let dy = (node.layout.y - node.prev_layout.y) as f64 * scale;
                let old_x = ax - dx;
                let old_y = ay - dy;
                let r =
                    transform.transform_rect_bbox(Rect::new(old_x, old_y, old_x + pw, old_y + ph));
                let r = Rect::new(r.x0 - margin, r.y0 - margin, r.x1 + margin, r.y1 + margin);
                region = Some(region.map_or(r, |prev| prev.union(r)));
            }
        }
    }

    // Include rects from removed nodes (saved before deletion).
    for &(rx, ry, rw, rh) in &tree.paint_dirty_removed_rects {
        if rw > 0.0 && rh > 0.0 {
            // Rects stored at scale=1; apply current scale
            let r = Rect::new(
                rx * scale - margin,
                ry * scale - margin,
                (rx + rw) * scale + margin,
                (ry + rh) * scale + margin,
            );
            region = Some(region.map_or(r, |prev| prev.union(r)));
        }
    }

    // Clamp to viewport bounds
    region.map(|r| {
        Rect::new(
            r.x0.max(0.0),
            r.y0.max(0.0),
            r.x1.min(viewport_w),
            r.y1.min(viewport_h),
        )
    })
}

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Pixel data for a render surface, keyed by surface ID.
pub struct SurfacePixelData {
    /// RGBA8 pixel data.
    pub data: Vec<u8>,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

thread_local! {
    /// Viewport names that have active surface frames this paint cycle.
    /// When set, only these viewports get holes punched in backgrounds.
    /// When empty, ALL viewports get holes (GPU compositor default).
    static ACTIVE_VIEWPORTS: RefCell<Option<HashSet<String>>> = const { RefCell::new(None) };

    /// Dirty region for incremental painting. When set, paint_node can
    /// skip subtrees entirely outside this rect (in physical pixels).
    static DIRTY_REGION: RefCell<Option<Rect>> = const { RefCell::new(None) };

    /// Surface pixel data for inline painting, keyed by surface ID.
    /// Set before paint_document() and cleared after.
    static SURFACE_PIXELS: RefCell<Option<HashMap<usize, SurfacePixelData>>> = const { RefCell::new(None) };
}

/// Set the active viewport names for hole-punching during this paint cycle.
///
/// Only viewports in this set will have holes cut in ancestor backgrounds.
/// Call with `None` to revert to the default (all viewports get holes).
pub fn set_active_viewports(names: Option<HashSet<String>>) {
    ACTIVE_VIEWPORTS.with(|v| *v.borrow_mut() = names);
}

/// Set surface pixel data for inline painting during the current paint cycle.
///
/// Call with `Some(map)` before `paint_document()` and `None` after.
/// When set, `paint_node()` will draw surface pixels inline at the
/// element's position, like `<img>` elements.
pub fn set_surface_pixels(pixels: Option<HashMap<usize, SurfacePixelData>>) {
    SURFACE_PIXELS.with(|v| *v.borrow_mut() = pixels);
}

/// Set the dirty region for incremental painting.
///
/// When set, `paint_node` will skip subtrees whose bounds are entirely
/// outside this rect, avoiding expensive glyph rasterization and path
/// operations for unchanged content. Set to `None` for full repaint.
pub fn set_dirty_region(region: Option<Rect>) {
    DIRTY_REGION.with(|v| *v.borrow_mut() = region);
}

/// Check whether a node rect intersects the current dirty region.
/// Returns true if there is no dirty region (full repaint) or if the
/// node's absolute rect overlaps the dirty region.
fn intersects_dirty_region(x: f64, y: f64, w: f64, h: f64) -> bool {
    DIRTY_REGION.with(|v| {
        let guard = v.borrow();
        match guard.as_ref() {
            None => true, // No dirty region → full repaint, paint everything
            Some(dr) => {
                // AABB intersection test
                x < dr.x1 && x + w > dr.x0 && y < dr.y1 && y + h > dr.y0
            }
        }
    })
}

/// Compute the absolute position of a node in physical pixels by walking
/// up the parent chain, summing layout offsets and subtracting scroll offsets.
pub fn compute_absolute_position(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> (f64, f64) {
    let mut x = 0.0_f64;
    let mut y = 0.0_f64;
    let mut current = Some(node_id);
    while let Some(id) = current {
        if let Some(node) = tree.get(id) {
            x += node.layout.x as f64 * scale;
            y += node.layout.y as f64 * scale;
            // position: fixed elements are viewport-relative — stop accumulating
            // parent offsets so the element stays at its top/left position
            // regardless of where it sits in the DOM tree or scroll state.
            if node.computed_style.position == crate::computed_style::PositionValue::Fixed {
                break;
            }
            if let Some(parent_id) = node.parent {
                if let Some(parent) = tree.get(parent_id) {
                    x -= parent.scroll_offset.0 * scale;
                    y -= parent.scroll_offset.1 * scale;
                }
            }
            current = node.parent;
        } else {
            break;
        }
    }
    (x, y)
}

/// Compose a node's CSS transform onto `parent_transform`, applied about the
/// node's transform-origin. Percentage-based translate values are resolved
/// against the node's layout box (so they resolve to 0 on a collapsed axis).
/// Returns `parent_transform` unchanged for identity transforms.
///
/// `x`/`y` are the node's absolute position in physical pixels. This is the
/// single source of truth for transform composition — the paint arms, the
/// dirty-region cull test, dirty-region tracking, and hit testing must all
/// agree on it (#142, #143, #199).
///
/// The result is covariant in `scale`: composing at `(s·x, s·y, s)` equals
/// `S · compose(x, y, 1) · S⁻¹` for `S = scale(s)`, which is what lets hit
/// testing pass `scale = 1.0` and work in layout pixels (#202).
pub fn compose_node_transform(
    node: &Node,
    x: f64,
    y: f64,
    scale: f64,
    parent_transform: Affine,
) -> Affine {
    let tf = &node.computed_style.transform;
    if tf.is_identity {
        return parent_transform;
    }
    let mut m = tf.matrix;
    // Resolve percentage-based translate against element dimensions
    if tf.translate_x_pct.abs() > 1e-9 || tf.translate_y_pct.abs() > 1e-9 {
        m[4] += tf.translate_x_pct * node.layout.width as f64;
        m[5] += tf.translate_y_pct * node.layout.height as f64;
    }
    // The translate components are *lengths*: `m[4]`/`m[5]` come from the
    // stylesheet in CSS px and the percentage part resolves against the CSS-px
    // layout box, so both need converting to the physical-pixel space this
    // function composes in (its `x`/`y` inputs and the origin below already
    // are). The linear part (a, b, c, d) is a pure ratio — unit-invariant —
    // and must NOT be scaled (#202).
    m[4] *= scale;
    m[5] *= scale;
    let cs = &node.computed_style;
    let ox = cs.transform_origin_x.resolve(node.layout.width);
    let oy = cs.transform_origin_y.resolve(node.layout.height);
    let cx = x + ox as f64 * scale;
    let cy = y + oy as f64 * scale;
    parent_transform * Affine::translate((cx, cy)) * Affine::new(m) * Affine::translate((-cx, -cy))
}

/// Compute a node's absolute position in physical pixels together with the
/// composed CSS transform affecting it (its own and its ancestors'),
/// mirroring paint-side composition (`compose_node_transform`) so that
/// dirty-region tracking agrees with where paint actually renders (#143).
///
/// The common untransformed case takes an allocation-free fast path and
/// returns `Affine::IDENTITY`.
fn compute_absolute_position_and_transform(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
) -> (f64, f64, Affine) {
    // First pass: check for transforms on the chain (cheap pointer walk),
    // stopping at position:fixed like compute_absolute_position — fixed
    // elements are viewport-relative and hoisted to body level at paint time.
    let mut any_transform = false;
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else { break };
        any_transform |= !node.computed_style.transform.is_identity;
        if node.computed_style.position == PositionValue::Fixed {
            break;
        }
        current = node.parent;
    }
    if !any_transform {
        let (x, y) = compute_absolute_position(tree, node_id, scale);
        return (x, y, Affine::IDENTITY);
    }

    // Transformed chain: collect node → root, then walk root → node
    // accumulating offsets and composing affines top-down — the same order
    // paint_node applies them.
    let mut chain: Vec<RawNodeId> = Vec::new();
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else { break };
        chain.push(id);
        if node.computed_style.position == PositionValue::Fixed {
            break;
        }
        current = node.parent;
    }

    let mut off_x = 0.0_f64;
    let mut off_y = 0.0_f64;
    let mut transform = Affine::IDENTITY;
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    for &id in chain.iter().rev() {
        let Some(node) = tree.get(id) else { break };
        x = off_x + node.layout.x as f64 * scale;
        y = off_y + node.layout.y as f64 * scale;
        transform = compose_node_transform(node, x, y, scale, transform);
        // Children resolve against this node's scroll-adjusted origin.
        off_x = x - node.scroll_offset.0 * scale;
        off_y = y - node.scroll_offset.1 * scale;
    }
    (x, y, transform)
}

/// Find viewport descendant rects (absolute pixel positions) for hole-punching.
/// Returns a list of viewport rects in physical pixel coordinates.
fn find_viewport_rects(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    result: &mut Vec<Rect>,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };
    let nx = offset_x + node.layout.x as f64 * scale;
    let ny = offset_y + node.layout.y as f64 * scale;

    if let Some(viewport_name) = node.attributes.get("data-viewport") {
        // If ACTIVE_VIEWPORTS is set, only cut holes for viewports with active frames
        let active = ACTIVE_VIEWPORTS.with(|v| {
            let guard = v.borrow();
            match guard.as_ref() {
                None => true, // No filter set — all viewports get holes (GPU default)
                Some(set) => set.contains(viewport_name),
            }
        });
        if active {
            let vw = node.layout.width as f64 * scale;
            let vh = node.layout.height as f64 * scale;
            result.push(Rect::new(nx, ny, nx + vw, ny + vh));
        }
        return;
    }

    // Account for scroll offset when recursing into children
    let sx = node.scroll_offset.0 * scale;
    let sy = node.scroll_offset.1 * scale;
    for &child_id in &node.children {
        find_viewport_rects(tree, child_id, scale, nx - sx, ny - sy, result);
    }
}

/// Build a BezPath for the background shape with viewport holes cut out.
/// Uses EvenOdd fill rule: outer shape wound clockwise, holes wound counter-clockwise.
fn build_background_with_holes(
    rect: Rect,
    radii: RoundedRectRadii,
    radius: f64,
    holes: &[Rect],
) -> BezPath {
    let mut path = BezPath::new();

    // Outer contour: the background shape (clockwise)
    if radius > 0.0 {
        let rrect = rect.to_rounded_rect(radii);
        path.extend(rrect.path_elements(0.1));
    } else {
        path.extend(rect.path_elements(0.1));
    }

    // Inner contours: viewport holes (counter-clockwise = reversed winding)
    for hole in holes {
        // Rect path_elements goes clockwise, so we reverse for counter-clockwise
        path.move_to((hole.x0, hole.y0));
        path.line_to((hole.x0, hole.y1));
        path.line_to((hole.x1, hole.y1));
        path.line_to((hole.x1, hole.y0));
        path.close_path();
    }

    path
}

/// Paint a subtree rooted at `root_node_id` into `painter`, positioned at (0, 0).
///
/// Used for drag-and-drop snapshot capture. The subtree is translated so its
/// top-left corner is at the scene origin, making it easy to reposition later.
pub fn paint_subtree(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    root_node_id: RawNodeId,
    scale: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    // paint_node computes each node's position as: offset + layout.x * scale.
    // To place the subtree root at (0, 0), we negate just the root's own
    // layout position — NOT the full absolute position (which would over-shift
    // by the entire ancestor chain).
    let Some(node) = tree.get(root_node_id) else {
        return;
    };
    let offset_x = -(node.layout.x as f64 * scale);
    let offset_y = -(node.layout.y as f64 * scale);
    paint_node(
        tree,
        root_node_id,
        painter,
        scale,
        offset_x,
        offset_y,
        font_cx,
        layout_cx,
        Affine::IDENTITY,
    );
}

/// Paint the entire document using a Painter.
///
/// `scale` is the DPI scale factor (1.0 = 96dpi).
/// `viewport` is the viewport size in physical pixels.
pub fn paint_document(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    scale: f64,
    _viewport: (f32, f32),
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    paint_node(
        tree,
        tree.body_id,
        painter,
        scale,
        0.0,
        0.0,
        font_cx,
        layout_cx,
        Affine::IDENTITY,
    );
}

/// Paint the children of `node_id`, front to back.
///
/// A stacking-context root (and the body, which is one by fiat) paints the
/// sequence [`stacking_paint_order`] gives it — CSS 2.1 Appendix E order, with
/// descendant stacking contexts and positioned `z-index: auto` boxes hoisted out
/// of their parents and sorted in among each other.
///
/// Any other node paints only the children that were *not* hoisted, in tree
/// order; an ancestor stacking context has the rest. [`paints_at_stacking_root`]
/// is the one predicate that decides which is which, and hit testing asks it the
/// same question so that what is on top is also what is tapped.
#[allow(clippy::too_many_arguments)]
fn paint_children_with_stacking(
    tree: &NodeTree,
    node_id: RawNodeId,
    painter: &mut dyn Painter,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    node_transform: Affine,
    skip_ifc_children: bool,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // Content this node has already drawn as inline boxes, via
    // `paint_inline_layout`: painting it again as a box would double it.
    let already_drawn_inline = |child: &Node, kind: PaintKind| {
        skip_ifc_children && kind != PaintKind::StackingContext && child.ifc_root == Some(node_id)
    };

    let is_body = node_id == tree.body_id;
    if is_body || node.creates_stacking_context() {
        for entry in stacking_paint_order(tree, node_id, is_body, scale, offset_x, offset_y) {
            if let Some(child) = tree.get(entry.node_id)
                && already_drawn_inline(child, entry.kind)
            {
                continue;
            }
            paint_node(
                tree,
                entry.node_id,
                painter,
                scale,
                entry.offset_x,
                entry.offset_y,
                font_cx,
                layout_cx,
                node_transform,
            );
        }
    } else {
        for &child_id in &node.children {
            let Some(child) = tree.get(child_id) else {
                continue;
            };
            if paints_at_stacking_root(child) || already_drawn_inline(child, PaintKind::InFlow) {
                continue;
            }
            paint_node(
                tree,
                child_id,
                painter,
                scale,
                offset_x,
                offset_y,
                font_cx,
                layout_cx,
                node_transform,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_node(
    tree: &NodeTree,
    node_id: RawNodeId,
    painter: &mut dyn Painter,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    parent_transform: Affine,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // Skip hidden elements (display: none, style/script tags)
    if let NodeKind::Element(ref el) = node.kind
        && matches!(
            el.tag.as_str(),
            "style" | "script" | "head" | "meta" | "link"
        )
    {
        return;
    }

    // Skip collapsed blocks (virtualized contenteditable) — they have
    // estimated heights for layout but no Parley text layout for painting.
    if node.estimated_height.is_some() {
        return;
    }
    let layout = &node.layout;

    // Skip zero-size elements (display: none produces 0x0 layout).
    // display:contents nodes also have 0x0 layout (no box), but their children
    // still need to be painted — recurse into children then return.
    // Also skip elements with either dimension zero — they have no fillable area
    // for backgrounds/borders, and attempting to paint them produces degenerate
    // zero-area rects that trigger warnings in renderers like tiny-skia.
    if layout.width == 0.0 || layout.height == 0.0 {
        if node.computed_style.display == DisplayValue::Contents {
            // display:contents has no box, so it never forms a SC itself and
            // has no transform box — its children are laid out in the
            // grandparent's coordinate space, so recurse with the parent's
            // offsets and transform unchanged.
            //
            // Use full stacking-context-aware painting so that SC children
            // (e.g. z-indexed absolute children) are properly collected and
            // painted in z-index order, rather than being skipped.
            paint_children_with_stacking(
                tree,
                node_id,
                painter,
                scale,
                offset_x,
                offset_y,
                font_cx,
                layout_cx,
                parent_transform,
                false,
            );
        } else if (layout.width == 0.0) != (layout.height == 0.0) {
            // A real box collapsed to zero in one dimension (e.g. an
            // auto-height container with only absolutely-positioned children)
            // may still have overflowing children that need painting. Unlike
            // display:contents it keeps its own origin, transform, and
            // opacity — only the degenerate background/border fill is
            // skipped (#142).
            let x = offset_x + layout.x as f64 * scale;
            let y = offset_y + layout.y as f64 * scale;
            let node_transform = compose_node_transform(node, x, y, scale, parent_transform);

            let opacity = node.computed_style.opacity;
            let has_opacity = opacity < 1.0;
            if has_opacity {
                // The element's own rect is zero-area and push_layer clips to
                // its bounds shape, which would blank the subtree — use a
                // conservative large rect instead (tiny-skia ignores layer
                // bounds; Vello merely clips to them).
                let bounds = Rect::new(-1e7, -1e7, 1e7, 1e7);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds.into());
            }

            let scroll_x = node.scroll_offset.0 * scale;
            let scroll_y = node.scroll_offset.1 * scale;
            paint_children_with_stacking(
                tree,
                node_id,
                painter,
                scale,
                x - scroll_x,
                y - scroll_y,
                font_cx,
                layout_cx,
                node_transform,
                false,
            );

            if has_opacity {
                painter.pop_layer();
            }
        }
        return;
    }

    // Absolute position of this node
    let x = offset_x + layout.x as f64 * scale;
    let y = offset_y + layout.y as f64 * scale;
    let w = layout.width as f64 * scale;
    let h = layout.height as f64 * scale;

    // Sticky position adjustment
    let (x, y) = if node.computed_style.position == PositionValue::Sticky {
        // Find the nearest scroll ancestor's scroll offset
        let mut scroll_y = 0.0_f64;
        let mut ancestor_id = node.parent;
        while let Some(aid) = ancestor_id {
            if let Some(ancestor) = tree.get(aid) {
                let ov = ancestor.computed_style.overflow_y;
                if matches!(
                    ov,
                    OverflowValue::Auto | OverflowValue::Scroll | OverflowValue::Hidden
                ) {
                    scroll_y = ancestor.scroll_offset.1 * scale;
                    break;
                }
                ancestor_id = ancestor.parent;
            } else {
                break;
            }
        }

        // Apply sticky top constraint: element should not scroll above `top` offset from container
        let sticky_top = node.computed_style.top.to_px() as f64 * scale;
        let adjusted_y = y.max(offset_y + scroll_y + sticky_top);
        (x, adjusted_y)
    } else {
        (x, y)
    };

    // Compose this node's CSS transform (about its transform-origin) onto
    // the parent transform. Shared by every paint arm below and by the cull
    // test — a transformed node renders at its visual position, not its
    // layout rect (#143).
    let node_transform = compose_node_transform(node, x, y, scale, parent_transform);

    // Dirty region optimization: skip drawing for nodes entirely outside
    // the dirty region. If overflow is clipped or node is a leaf, skip the
    // entire subtree. Otherwise skip only this node's drawing but still
    // recurse into children (they may have absolute positioning inside the
    // dirty region).
    //
    // Cull against the transformed bounding box; untransformed nodes keep
    // the plain AABB test (#143).
    let node_outside_dirty = if node_transform == Affine::IDENTITY {
        !intersects_dirty_region(x, y, w, h)
    } else {
        let bbox = node_transform.transform_rect_bbox(Rect::new(x, y, x + w, y + h));
        !intersects_dirty_region(bbox.x0, bbox.y0, bbox.width(), bbox.height())
    };
    if node_outside_dirty {
        let overflow_clips = matches!(
            node.computed_style.overflow_x,
            OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
        ) || matches!(
            node.computed_style.overflow_y,
            OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
        );
        if overflow_clips || node.children.is_empty() {
            return;
        }
        // Skip drawing this node but recurse into children
        paint_children_with_stacking(
            tree,
            node_id,
            painter,
            scale,
            x,
            y,
            font_cx,
            layout_cx,
            node_transform,
            false,
        );
        return;
    }

    match &node.kind {
        NodeKind::Element(el) if el.tag == "svg" => {
            paint_svg(tree, node, painter, scale, x, y, w, h, node_transform);
        }
        NodeKind::Element(el) if el.tag == "img" => {
            let rect = Rect::new(x, y, x + w, y + h);

            let fit = node.computed_style.object_fit;

            // Opacity
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &rect.into());
            }

            // Paint background (if any) before image
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            if visible {
                if let BackgroundValue::Color(bg_color) = &node.computed_style.background {
                    painter.fill_color(Fill::NonZero, node_transform, *bg_color, &rect.into());
                }

                // Paint the image itself
                if let Some(src) = node.attributes.get("src")
                    && let Some(decoded) = tree.image_cache.get(src)
                {
                    image::paint_image(painter, decoded, rect, scale, fit, node_transform);
                }

                // Borders (no border-radius for img elements)
                paint_borders(painter, node, scale, x, y, w, h, 0.0.into(), node_transform);
            }

            if opacity < 1.0 {
                painter.pop_layer();
            }
        }
        // Inline painting for render surfaces — draws pixels at the element's
        // position like <img>, participating in normal stacking and clipping.
        NodeKind::Element(_) if node.attributes.contains_key("data-render-surface") => {
            let surface_painted = node
                .attributes
                .get("data-render-surface")
                .and_then(|id_str| id_str.parse::<usize>().ok())
                .map(|surface_id| {
                    SURFACE_PIXELS.with(|sp| {
                        let guard = sp.borrow();
                        if let Some(map) = guard.as_ref() {
                            if let Some(pixels) = map.get(&surface_id) {
                                if pixels.width > 0 && pixels.height > 0 && !pixels.data.is_empty()
                                {
                                    let rect = Rect::new(x, y, x + w, y + h);
                                    let opacity = node.computed_style.opacity;
                                    if opacity < 1.0 {
                                        painter.push_layer(
                                            BlendMode::Normal,
                                            opacity,
                                            node_transform,
                                            &rect.into(),
                                        );
                                    }

                                    // Paint background behind the surface if any
                                    if let BackgroundValue::Color(bg_color) =
                                        &node.computed_style.background
                                    {
                                        painter.fill_color(
                                            Fill::NonZero,
                                            node_transform,
                                            *bg_color,
                                            &rect.into(),
                                        );
                                    }

                                    // Paint the surface pixels inline, like an image
                                    let decoded = crate::image_cache::DecodedImage {
                                        data: pixels.data.clone(),
                                        width: pixels.width,
                                        height: pixels.height,
                                    };
                                    image::paint_image(
                                        painter,
                                        &decoded,
                                        rect,
                                        scale,
                                        crate::computed_style::ObjectFitValue::Contain,
                                        node_transform,
                                    );

                                    if opacity < 1.0 {
                                        painter.pop_layer();
                                    }
                                    return true;
                                }
                            }
                        }
                        false
                    })
                })
                .unwrap_or(false);

            if !surface_painted {
                // No pixel data yet — paint as a normal element (shows background)
                // Fall through to generic element painting below
            } else {
                return; // Surface painted inline, done
            }

            // Fall through: paint as generic element if no surface data
            let rect = Rect::new(x, y, x + w, y + h);
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &rect.into());
            }
            if visible {
                if let BackgroundValue::Color(bg_color) = &node.computed_style.background {
                    let radius =
                        node.computed_style.border_radius_top_left.to_px().max(0.0) as f64 * scale;
                    if radius > 0.0 {
                        let rrect = rect.to_rounded_rect(radius);
                        painter.fill_color(Fill::NonZero, node_transform, *bg_color, &rrect.into());
                    } else {
                        painter.fill_color(Fill::NonZero, node_transform, *bg_color, &rect.into());
                    }
                }
            }
            if opacity < 1.0 {
                painter.pop_layer();
            }
        }
        NodeKind::Element(_) => {
            let rect = Rect::new(x, y, x + w, y + h);
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );

            // Get border-radius from computed style (use average of all 4 corners)
            // Resolve percentage values against element dimensions
            let (radius, radii) = {
                let cs = &node.computed_style;
                let resolve_size = node.layout.width.min(node.layout.height);
                let tl = cs.border_radius_top_left.resolve(resolve_size) as f64 * scale;
                let tr = cs.border_radius_top_right.resolve(resolve_size) as f64 * scale;
                let br = cs.border_radius_bottom_right.resolve(resolve_size) as f64 * scale;
                let bl = cs.border_radius_bottom_left.resolve(resolve_size) as f64 * scale;
                let radii = RoundedRectRadii::new(tl, tr, br, bl);
                // Uniform radius for code paths that don't support per-corner yet
                let avg = (tl + tr + br + bl) / 4.0;
                (avg, radii)
            };

            // Get opacity from computed style and push layer if needed
            let opacity = node.computed_style.opacity;
            let has_opacity = opacity < 1.0;
            if has_opacity {
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &rect.into());
            }

            // Handle overflow clipping — detect early so we can cut holes
            // in the background for viewport descendants.
            let overflow_y = node.computed_style.overflow_y;
            let clips = matches!(
                overflow_y,
                OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
            );

            // Find viewport descendants — their rects will be cut out of
            // the background fill so the compositor layer shows through.
            let mut viewport_holes = Vec::new();
            if clips {
                find_viewport_rects(
                    tree,
                    node_id,
                    scale,
                    offset_x,
                    offset_y,
                    &mut viewport_holes,
                );
            }

            // Only paint this element's own visuals if visible
            // (children may override with visibility: visible)
            if visible {
                // Render box-shadow from computed style
                if !node.computed_style.box_shadow.is_empty() {
                    paint_box_shadow(
                        painter,
                        &node.computed_style.box_shadow,
                        x,
                        y,
                        w,
                        h,
                        scale,
                        node,
                        node_transform,
                    );
                }

                // Get background from computed style (solid color or gradient).
                // When viewport_holes is non-empty, we paint the background with
                // holes cut out (EvenOdd fill) so compositor layers show through.
                if !viewport_holes.is_empty() {
                    // Build a compound path: outer shape + inner holes (wound opposite)
                    let bg_path = build_background_with_holes(rect, radii, radius, &viewport_holes);
                    match &node.computed_style.background {
                        BackgroundValue::Color(bg_color) => {
                            painter.fill_color(
                                Fill::EvenOdd,
                                node_transform,
                                *bg_color,
                                &bg_path.into(),
                            );
                        }
                        BackgroundValue::LinearGradient {
                            angle_degrees,
                            stops,
                        } => {
                            let brush = build_linear_gradient_brush(*angle_degrees, stops, &rect);
                            painter.fill(Fill::EvenOdd, node_transform, &brush, &bg_path.into());
                        }
                        BackgroundValue::RadialGradient { stops } => {
                            let brush = build_radial_gradient_brush(stops, &rect);
                            painter.fill(Fill::EvenOdd, node_transform, &brush, &bg_path.into());
                        }
                        BackgroundValue::Image { url } => {
                            if let Some(decoded) = tree.image_cache.get(url) {
                                image::paint_image(
                                    painter,
                                    decoded,
                                    rect,
                                    scale,
                                    crate::computed_style::ObjectFitValue::Fill,
                                    node_transform,
                                );
                            }
                        }
                        BackgroundValue::None => {}
                    }
                } else {
                    match &node.computed_style.background {
                        BackgroundValue::Color(bg_color) => {
                            if radius > 0.0 {
                                let rrect = rect.to_rounded_rect(radii);
                                painter.fill_color(
                                    Fill::NonZero,
                                    node_transform,
                                    *bg_color,
                                    &rrect.into(),
                                );
                            } else {
                                painter.fill_color(
                                    Fill::NonZero,
                                    node_transform,
                                    *bg_color,
                                    &rect.into(),
                                );
                            }
                        }
                        BackgroundValue::LinearGradient {
                            angle_degrees,
                            stops,
                        } => {
                            let brush = build_linear_gradient_brush(*angle_degrees, stops, &rect);
                            if radius > 0.0 {
                                let rrect = rect.to_rounded_rect(radii);
                                painter.fill(Fill::NonZero, node_transform, &brush, &rrect.into());
                            } else {
                                painter.fill(Fill::NonZero, node_transform, &brush, &rect.into());
                            }
                        }
                        BackgroundValue::RadialGradient { stops } => {
                            let brush = build_radial_gradient_brush(stops, &rect);
                            if radius > 0.0 {
                                let rrect = rect.to_rounded_rect(radii);
                                painter.fill(Fill::NonZero, node_transform, &brush, &rrect.into());
                            } else {
                                painter.fill(Fill::NonZero, node_transform, &brush, &rect.into());
                            }
                        }
                        BackgroundValue::Image { url } => {
                            if let Some(decoded) = tree.image_cache.get(url) {
                                image::paint_image(
                                    painter,
                                    decoded,
                                    rect,
                                    scale,
                                    crate::computed_style::ObjectFitValue::Fill,
                                    node_transform,
                                );
                            }
                        }
                        BackgroundValue::None => {}
                    }
                }

                // Render borders per-side with style support
                paint_borders(painter, node, scale, x, y, w, h, radii, node_transform);

                // Render outline (drawn outside the box model)
                paint_outline(painter, node, scale, x, y, w, h, radii, node_transform);

                // Render input element value
                if matches!(node.tag(), Some("input" | "textarea")) {
                    paint_input_value(
                        node,
                        painter,
                        scale,
                        x,
                        y,
                        w,
                        h,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                } else if node.tag() == Some("select") {
                    // The closed control's label + arrow. Options are display:none
                    // (they don't lay out); the selected label is painted here.
                    select::paint_select_value(
                        tree,
                        node_id,
                        painter,
                        scale,
                        x,
                        y,
                        w,
                        h,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                }
            }

            if clips {
                if radius > 0.0 {
                    let clip_rrect = rect.to_rounded_rect(radii);
                    painter.push_clip(Fill::NonZero, node_transform, &clip_rrect.into());
                } else {
                    painter.push_clip(Fill::NonZero, node_transform, &rect.into());
                }
            }

            // Render read-only text selection highlight (user-select: text).
            if node
                .attributes
                .get("data-text-sel")
                .map(|s| s == "true")
                .unwrap_or(false)
            {
                let sel_start = node
                    .attributes
                    .get("data-text-sel-start")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let sel_end = node
                    .attributes
                    .get("data-text-sel-end")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);

                if sel_start != sel_end {
                    if let Some(ref inline_layout) = node.text_layout {
                        let cs = &node.computed_style;
                        let pad_x =
                            (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64 * scale;
                        let pad_y =
                            (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64 * scale;
                        let text_x = x + pad_x;
                        let text_y = y + pad_y;
                        let text_len = inline_layout.text_content.len();
                        let content_width = node.layout.width as f64 * scale
                            - (cs.padding_left.to_px() + cs.padding_right.to_px()) as f64 * scale;
                        paint_text_selection_highlight(
                            node,
                            painter,
                            scale,
                            text_x,
                            text_y,
                            &inline_layout.layout,
                            text_len,
                            sel_start,
                            sel_end,
                            content_width,
                            node_transform,
                        );
                    }
                }
            }

            // Check if this is an IFC root with a cached inline layout
            if let Some(inline_layout) = &node.text_layout {
                // Paint inline content at the content-box origin (inside padding+border),
                // accounting for scroll offset.
                let cs = &node.computed_style;
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                let content_x = x
                    + (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64 * scale
                    - scroll_x;
                let content_y = y
                    + (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64 * scale
                    - scroll_y;
                let ifc_text_shadows = node.computed_style.text_shadow.as_slice();
                paint_inline_layout(
                    tree,
                    painter,
                    scale,
                    content_x,
                    content_y,
                    inline_layout,
                    font_cx,
                    layout_cx,
                    ifc_text_shadows,
                    node_transform,
                );

                // Still paint non-inline (block) children normally
                paint_children_with_stacking(
                    tree,
                    node_id,
                    painter,
                    scale,
                    x - scroll_x,
                    y - scroll_y,
                    font_cx,
                    layout_cx,
                    node_transform,
                    true, // skip IFC children
                );
            } else {
                // Normal paint path: recurse into all children
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                paint_children_with_stacking(
                    tree,
                    node_id,
                    painter,
                    scale,
                    x - scroll_x,
                    y - scroll_y,
                    font_cx,
                    layout_cx,
                    node_transform,
                    false,
                );
            }

            if clips {
                painter.pop_layer();
            }

            // Paint scrollbar overlay for scroll containers
            if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
                let node = tree.get(node_id).unwrap(); // re-borrow after children done
                let cs = &node.computed_style;
                // Taffy child.layout.y is relative to the parent's border box,
                // so content_height includes the top padding+border offset.
                // Subtract it to get content-relative height.
                let content_top =
                    (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64 * scale;
                let mut content_height: f64 = 0.0;
                for &child_id in &node.children {
                    if let Some(child) = tree.get(child_id) {
                        let bottom =
                            (child.layout.y + child.layout.height) as f64 * scale - content_top;
                        if bottom > content_height {
                            content_height = bottom;
                        }
                    }
                }
                // Visible content area = layout height minus padding and border
                let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64 * scale;
                let border_v =
                    (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64 * scale;
                let visible_h = (h - pad_v - border_v).max(0.0);
                if content_height > visible_h {
                    let scrollbar_width = 6.0 * scale;
                    let scrollbar_margin = 2.0 * scale;
                    let scrollbar_x = x + w - scrollbar_width - scrollbar_margin;

                    // Thumb sizing
                    let visible_ratio = visible_h / content_height;
                    let max_scroll = content_height - visible_h;
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
                    painter.fill(
                        Fill::NonZero,
                        node_transform,
                        &Brush::Solid(thumb_color),
                        &thumb_rect.into(),
                    );
                }
            }

            // Apply CSS filter approximations (after content is painted, before opacity pop)
            let cs = &tree.get(node_id).unwrap().computed_style;
            let has_filter = cs.filter_brightness != 1.0 || cs.filter_grayscale > 0.0;

            if has_filter {
                // Brightness: overlay black (darken) or white (brighten) with calculated alpha
                if cs.filter_brightness != 1.0 {
                    let brightness = cs.filter_brightness;
                    if brightness < 1.0 {
                        // Darken: overlay black with alpha = 1.0 - brightness
                        let alpha = ((1.0 - brightness).clamp(0.0, 1.0) * 255.0) as u8;
                        let dark = AlphaColor::<Srgb>::from_rgba8(0, 0, 0, alpha);
                        if radius > 0.0 {
                            let rrect = rect.to_rounded_rect(radii);
                            painter.fill_color(Fill::NonZero, node_transform, dark, &rrect.into());
                        } else {
                            painter.fill_color(Fill::NonZero, node_transform, dark, &rect.into());
                        }
                    } else if brightness > 1.0 {
                        // Brighten: overlay white with alpha proportional to excess brightness
                        let alpha = ((brightness - 1.0).clamp(0.0, 1.0) * 255.0) as u8;
                        let light = AlphaColor::<Srgb>::from_rgba8(255, 255, 255, alpha);
                        if radius > 0.0 {
                            let rrect = rect.to_rounded_rect(radii);
                            painter.fill_color(Fill::NonZero, node_transform, light, &rrect.into());
                        } else {
                            painter.fill_color(Fill::NonZero, node_transform, light, &rect.into());
                        }
                    }
                }

                // Grayscale approximation: desaturation effect
                if cs.filter_grayscale > 0.0 {
                    let grayscale = cs.filter_grayscale.clamp(0.0, 1.0);
                    // Push a saturation layer: gray rect with Saturation blend at grayscale alpha
                    painter.push_layer(
                        BlendMode::Saturation,
                        grayscale,
                        node_transform,
                        &rect.into(),
                    );
                    // Fill with neutral gray
                    let gray = AlphaColor::<Srgb>::from_rgba8(128, 128, 128, 255);
                    if radius > 0.0 {
                        let rrect = rect.to_rounded_rect(radii);
                        painter.fill_color(Fill::NonZero, node_transform, gray, &rrect.into());
                    } else {
                        painter.fill_color(Fill::NonZero, node_transform, gray, &rect.into());
                    }
                    painter.pop_layer();
                }
            }

            if has_opacity {
                painter.pop_layer();
            }
        }

        NodeKind::Text(text_data) => {
            if text_data.content.is_empty() {
                return;
            }

            // Check visibility (inherited from parent)
            let parent_visibility = node
                .parent
                .and_then(|p| tree.get(p))
                .map(|p| &p.computed_style.visibility);
            if matches!(
                parent_visibility,
                Some(VisibilityValue::Hidden | VisibilityValue::Collapse)
            ) {
                return;
            }

            // Use cached layout if available (built after Taffy layout with final widths)
            if let Some(cached_layout) = &node.cached_text_parley {
                // Layout is already aligned during caching, use it directly
                let text_shadows = node
                    .parent
                    .and_then(|p| tree.get(p))
                    .map(|p| p.computed_style.text_shadow.as_slice())
                    .unwrap_or(&[]);
                render_text_with_shadow(
                    painter,
                    cached_layout,
                    x,
                    y,
                    text_shadows,
                    parent_transform,
                    scale,
                );
                return;
            }

            // Fallback: build layout on demand (should not happen with caching)
            let parent_node = node.parent.and_then(|p| tree.get(p));
            let parent_computed = parent_node.map(|p| &p.computed_style);

            let font_size = parent_computed.map(|s| s.font_size).unwrap_or(16.0);
            let font_weight = parent_computed.map(|s| s.font_weight).unwrap_or(400.0);
            let font_family = parent_computed
                .map(|s| {
                    if s.font_family.is_empty() {
                        "sans-serif".to_string()
                    } else {
                        s.font_family.clone()
                    }
                })
                .unwrap_or_else(|| "sans-serif".to_string());

            let color = parent_computed
                .and_then(|s| s.color)
                .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));

            // Build the Parley layout at the *logical* font size (scale 1.0) — the
            // same space `measure_inline_blocks` sized the box in and the same space
            // the cached IFC path (`build_inline_layout(..., 1.0, ...)`) uses. The
            // painter re-applies `scale` in `render_text`. (Building at `font_size *
            // scale` here would both double-scale the glyphs at render *and*, because
            // glyph advances don't scale perfectly linearly, make this layout's width
            // diverge from the width the box was measured at — breaking the wrap
            // constraint below at fractional scale factors.)
            let mut builder = layout_cx.ranged_builder(font_cx, &text_data.content, 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(font_size));
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

            // Lay out unwrapped first to get the natural (single-line) width & height.
            text_layout.break_all_lines(None);

            // This fallback fires for text that is not part of a cached IFC layout —
            // notably the sole text child of an `inline-block`, which is not an IFC
            // root (so it never gets a `cached_text_parley`). Such text was measured
            // at the parent's max-content width, so for an *auto*-width box painting
            // it unwrapped is correct. But once the box is clamped
            // (`width`/`max-width`/`min-width` narrower than the content) layout wraps
            // the text and sizes the box for the *wrapped* height, while these glyphs
            // would still paint one overflowing line (#127).
            //
            // We can't decide "is it clamped?" by comparing this layout's width to the
            // box: this on-demand layout's width drifts a pixel or two from the width
            // the box was measured at (a different Parley build path), so a width test
            // spuriously wraps auto-width boxes. Instead compare *heights*: layout
            // records this text node's wrapped height, and line counts are discrete —
            // if the node is more than ~1.5 single lines tall, layout wrapped it, so
            // the box is clamped and paint must wrap to the content box too.
            let one_line_h = text_layout.height();
            let clamped = one_line_h > 0.0 && node.layout.height > one_line_h * 1.5;
            if clamped && let Some(p) = parent_node {
                let cs = &p.computed_style;
                let padding_h = cs.padding_left.to_px() + cs.padding_right.to_px();
                let border_h = cs.border_left_width.to_px() + cs.border_right_width.to_px();
                let content_width = p.layout.width - padding_h - border_h;
                if content_width > 0.0 {
                    // Wrap to the content box, but 2% wider. This on-demand layout is a
                    // hair wider per glyph run than the one layout wrapped the box with,
                    // so breaking at exactly `content_width` fits fewer words per line
                    // and can spill one extra line past the box's reserved height. The
                    // small proportional slack reproduces layout's line breaks; any
                    // residual right overflow stays within the element's padding.
                    text_layout.break_all_lines(Some(content_width * 1.02));
                }
            }

            // Read text-align from parent's computed style
            let alignment = parent_computed
                .map(|s| s.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);
            text_layout.align(alignment, parley::layout::AlignmentOptions::default());

            // Render text glyphs
            let text_shadows = parent_computed
                .map(|s| s.text_shadow.as_slice())
                .unwrap_or(&[]);
            render_text_with_shadow(
                painter,
                &text_layout,
                x,
                y,
                text_shadows,
                parent_transform,
                scale,
            );
        }

        _ => {} // Document, Comment -- invisible
    }
}
