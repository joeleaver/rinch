//! Abstract scene building for rinch-dom.
//!
//! Walks the node tree and emits drawing commands via the `Painter` trait
//! for backgrounds, borders, and text.

mod borders;
mod contenteditable;
pub mod image;
mod layer_bounds;
pub mod painter;
pub mod scrollbar;
mod select;
mod svg;
mod text;
pub mod vello_painter;

#[cfg(feature = "software-renderer")]
pub mod skia_painter;

use borders::*;
use contenteditable::*;
use layer_bounds::opacity_layer_shape;
pub use layer_bounds::{UNBOUNDED, opacity_layer_bounds};
use svg::*;
use text::*;

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Point, Rect, RoundedRect, RoundedRectRadii, Shape};
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

    /// Surface pixel data for inline painting, keyed by **viewport name** —
    /// the software backend's video frames (issue #358).
    ///
    /// `SURFACE_PIXELS` is keyed by the `usize` surface id a `RenderSurface`
    /// component stamps into `data-render-surface`; a video viewport carries no
    /// such id, only the `data-viewport` name its player was created with, so
    /// the two registries cannot share a key space. Set before
    /// `paint_document()` and cleared after, like `SURFACE_PIXELS`.
    static VIEWPORT_PIXELS: RefCell<Option<HashMap<String, SurfacePixelData>>> = const { RefCell::new(None) };
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

/// Set viewport frame data for inline painting during the current paint cycle,
/// keyed by `data-viewport` name.
///
/// This is the **software** backend's video path (issue #358). A `data-viewport`
/// node with an entry here paints its frame inline, during paint, at its own
/// z-order — so anything drawn above it (a drawer, a modal, a dropdown) covers
/// it by ordinary paint order. A node with no entry falls through to normal
/// element painting, which is what leaves the GPU compositor path untouched:
/// that backend never sets this map, so every `data-viewport` node there still
/// paints as a plain element and gets its hole punched.
///
/// Call with `Some(map)` before `paint_document()` and `None` after.
pub fn set_viewport_pixels(pixels: Option<HashMap<String, SurfacePixelData>>) {
    VIEWPORT_PIXELS.with(|v| *v.borrow_mut() = pixels);
}

/// Whether a **usable** inline frame is available for the viewport named
/// `name`.
///
/// The dimensions are validated here, exactly as the `data-render-surface` arm
/// validates its own before taking the inline path. An entry whose pixels
/// cannot be drawn — zero-sized, or a buffer shorter than `width * height * 4`
/// (`submit_frame` only `debug_assert!`s that, so a release build can deliver
/// one) — must leave the node on the ordinary element path and its `#000`
/// placeholder background, rather than take the inline arm and paint a bare
/// black box with no frame inside it.
fn has_viewport_pixels(name: &str) -> bool {
    VIEWPORT_PIXELS.with(|v| {
        v.borrow()
            .as_ref()
            .and_then(|map| map.get(name))
            .is_some_and(|pixels| viewport_frame_bytes(pixels).is_some())
    })
}

/// The byte length a frame must have to be drawable, or `None` if it is not.
fn viewport_frame_bytes(pixels: &SurfacePixelData) -> Option<usize> {
    if pixels.width == 0 || pixels.height == 0 {
        return None;
    }
    let needed = pixels.width as usize * pixels.height as usize * 4;
    (pixels.data.len() >= needed).then_some(needed)
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

/// A node's origin in the space its *own* box is laid out in — the accumulated
/// parent-chain offset, before any composed CSS transform is applied.
///
/// This is [`compute_absolute_position_and_transform`] with the affine dropped,
/// and it exists for the two callers that want exactly that: layout's
/// out-of-flow correction (#204), which writes a **parent-relative** delta back
/// into `LayoutResult` and so must stay in untransformed layout space, and
/// anything else reasoning about where Taffy put a box rather than where paint
/// draws it.
///
/// **It is not a screen rect.** For "where is this box on screen" — the answer
/// a click, a drag or a dirty region needs — use [`painted_border_box`], which
/// applies the transform this function discards.
pub fn compute_absolute_position(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> (f64, f64) {
    let (x, y, _) = compute_absolute_position_and_transform(tree, node_id, scale);
    (x, y)
}

/// The offset from a node's summed layout origin to the origin it is *painted*
/// at, for a box an IFC positions.
///
/// An inline-block laid out by an inline formatting context stores its
/// `layout.x`/`layout.y` relative to the IFC root's **content** box, while a
/// parent-chain sum like [`compute_absolute_position`] adds up **border**-box
/// origins. Paint bridges the two: it hands `paint_inline_layout` the root's
/// content-box origin, so the box lands one padding+border in from where the
/// sum alone puts it. Anything mapping a screen point back into such a box —
/// hit testing, caret placement — has to add the same offset or it is looking
/// at the box's old address.
///
/// Returns `(0.0, 0.0)` for every box the IFC does not position, which is all
/// of them outside a text flow.
pub fn ifc_content_box_offset(tree: &NodeTree, node: &Node) -> (f32, f32) {
    if node.display_mode != crate::DisplayMode::InlineBlock {
        return (0.0, 0.0);
    }
    let Some(root) = node.ifc_root.and_then(|id| tree.get(id)) else {
        return (0.0, 0.0);
    };
    let cs = &root.computed_style;
    (
        cs.padding_left.to_px() + cs.border_left_width.to_px(),
        cs.padding_top.to_px() + cs.border_top_width.to_px(),
    )
}

/// A box whose IFC has a live inline layout is drawn **by that IFC and by
/// nothing else** — not by a tree-order walk, not by a stacking sequence
/// (#365).
///
/// This replaced a positional predicate that had two independent ways to miss:
///
/// ```text
/// skip_ifc_children && kind != PaintKind::StackingContext && child.ifc_root == Some(node_id)
/// ```
///
/// The third term can never match for a subtree hoisted to an *ancestor* — the
/// box's `ifc_root` names its own IFC, not the node being painted — and the
/// second excluded any inline-level box that is itself a stacking context,
/// even as a direct child of the node doing the painting. Either miss drew the
/// box twice: once by `paint_inline_layout` at the IFC root's **content**
/// origin, once by the stacking sequence at its **border-box** origin, exactly
/// one padding+border apart. `position: relative` alone is enough to reach it
/// (it makes an inline-block `is_positioned_z_auto`), so
/// `<button style="position: relative">` inside a padded paragraph — the
/// ordinary tooltip-anchor idiom — reproduced it.
///
/// `text_layout.is_some()` is load-bearing, not a nicety: an IFC root that is
/// virtualized (`estimated_height`) or has no cached layout draws nothing at
/// all, and skipping its children there would make them **vanish** rather than
/// double. That is what the old `skip_ifc_children` flag was standing in for,
/// positionally and only at the sites that happened to pass it.
///
/// **Known divergence.** The IFC's draw is the survivor, so an inline-level
/// box paints in *inline order* rather than at its `z-index` — visible for
/// something like `<button style="position: relative; z-index: -1">` inside
/// text. Preserving z-order instead means making the hoisted entry the
/// survivor and having `paint_inline_layout` skip boxes that
/// `paints_at_stacking_root`, which needs the offset correction below to be
/// exactly right first. Tracked separately; the simple rule is correct about
/// *where* and *how many*, which is what was broken.
///
/// **Caveat (#366).** The invariant assumes the stamp is honest, and today it
/// is not always: `mark_inline_descendants` continues past a block-level child
/// where `walk_inline_children` breaks, so an inline-level box after a block
/// sibling inside an inline element carries an `ifc_root` whose IFC never
/// draws it. Such a box satisfies this predicate and paints **nowhere**
/// (before this guard it painted once at the viewport origin — garbage of a
/// different shape; hit testing tapped it somewhere else again either way).
/// If a box vanishes and its markup matches that shape, the bug is #366's
/// overmark, not a new miss here.
pub(crate) fn drawn_by_its_ifc(tree: &NodeTree, child: &Node) -> bool {
    child
        .ifc_root
        .and_then(|r| tree.get(r))
        .is_some_and(|r| r.text_layout.is_some())
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
    // A percentage translate resolves against the element's own border box —
    // but *in the frame its position in the function list establishes*, so its
    // contribution is a linear form in the box's width and height, not a pair
    // of pixel offsets added to the end of the composed matrix (#212).
    // `TransformValue` carries the four coefficients; this is where the box
    // they multiply finally arrives.
    let (w, h) = (node.layout.width as f64, node.layout.height as f64);
    m[4] += tf.pct_translate_w[0] * w + tf.pct_translate_h[0] * h;
    m[5] += tf.pct_translate_w[1] * w + tf.pct_translate_h[1] * h;
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

/// [`compose_node_transform`], plus the one case paint never asks it about.
///
/// A `display: contents` node generates no box, so `paint_node`'s zero-size
/// branch hands `parent_transform` straight down to the children rather than
/// composing anything for it — a transform declared on such a node has no box
/// to apply about and is simply not drawn. Hit testing makes the same exception
/// (`local_point`, `crates/rinch/src/app/hit_testing.rs`); a forward walk that
/// composed it anyway would place the node's descendants somewhere paint never
/// puts them.
fn compose_transform_step(
    node: &Node,
    x: f64,
    y: f64,
    scale: f64,
    parent_transform: Affine,
) -> Affine {
    if node.computed_style.display == DisplayValue::Contents
        && (node.layout.width == 0.0 || node.layout.height == 0.0)
    {
        return parent_transform;
    }
    compose_node_transform(node, x, y, scale, parent_transform)
}

/// The transform every hoisted `position: fixed` box paints under.
///
/// `paint_document` enters at the body with zero offsets and the identity
/// transform, and `paint_children_with_stacking` hands the body's own composed
/// transform to each entry of its stacking sequence — the hoisted fixed boxes
/// included. So viewport space, for a fixed box, is the body's *post*-transform
/// space, which is what `hit_test_node` re-seeds `vx`/`vy` from.
fn body_paint_transform(tree: &NodeTree, scale: f64) -> Affine {
    let Some(body) = tree.get(tree.body_id) else {
        return Affine::IDENTITY;
    };
    // Through the same origin step the chain below uses, so the body's
    // transform is composed about the same point either way in.
    let (bx, by) = painted_origin_step(tree, body, 0.0, 0.0, scale);
    compose_transform_step(body, bx, by, scale, Affine::IDENTITY)
}

/// One step of the descent: the node's own painted origin, given the origin its
/// parent resolves children against.
///
/// Kept as one function so the transformed and untransformed paths below cannot
/// drift: both add the layout offset and the IFC content-box offset, and both
/// hand children a scroll-adjusted origin.
///
/// `hit_test_node` additionally exempts a `position: fixed` box from the IFC
/// offset. That exemption is not repeated here because it cannot fire: Stylo
/// blockifies an out-of-flow box, so a fixed one never keeps
/// `display_mode == InlineBlock` and [`ifc_content_box_offset`] already answers
/// `(0, 0)` for it. `a_fixed_inline_block_keeps_its_viewport_origin` pins the
/// outcome, so if that ever stops being true the exemption gets added here
/// rather than discovered in the wild.
fn painted_origin_step(
    tree: &NodeTree,
    node: &Node,
    off_x: f64,
    off_y: f64,
    scale: f64,
) -> (f64, f64) {
    let (dx, dy) = ifc_content_box_offset(tree, node);
    (
        off_x + (node.layout.x + dx) as f64 * scale,
        off_y + (node.layout.y + dy) as f64 * scale,
    )
}

/// Compute a node's absolute position in physical pixels together with the
/// composed CSS transform affecting it (its own and its ancestors'), mirroring
/// paint's own descent so that anything asking "where does this node end up"
/// gets the answer paint acts on (#143, #203).
///
/// The mirror is the whole contract, so each of paint's four adjustments is
/// made here too:
///
/// - the chain stops at a `position: fixed` node, whose box is viewport-relative
///   because `collect_stacking_contexts_root` hoists it to the body level with
///   zeroed offsets — **and resumes from the body's own transform**, which is
///   what paint hands those hoisted entries;
/// - a `display: contents` node contributes no transform (see
///   [`compose_transform_step`]);
/// - an inline-block an IFC positions gets [`ifc_content_box_offset`] added,
///   because its `layout.x`/`layout.y` are relative to the IFC root's content
///   box while the chain sums border-box origins;
/// - children resolve against a scroll-adjusted origin.
///
/// The chain also stops at the body, which is where `paint_document` enters:
/// nothing above it is painted, so nothing above it may displace or transform
/// what is.
///
/// `scale` is the DPI scale factor the result is expressed in.
/// [`compose_node_transform`] is covariant in it (#202), so a caller working in
/// layout pixels — hit testing, click bounds — passes `1.0` and gets paint's
/// transform expressed in layout pixels.
///
/// The common untransformed case takes an allocation-free fast path and returns
/// `Affine::IDENTITY`.
pub fn compute_absolute_position_and_transform(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
) -> (f64, f64, Affine) {
    // First pass: does anything on the chain transform (cheap pointer walk)?
    let mut any_transform = false;
    let mut hoisted_fixed = false;
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = tree.get(id) else { break };
        any_transform |= !node.computed_style.transform.is_identity;
        if node.computed_style.position == PositionValue::Fixed {
            hoisted_fixed = true;
            break;
        }
        if id == tree.body_id {
            break;
        }
        current = node.parent;
    }
    // A hoisted fixed box drops its ancestors' transforms but not the body's.
    if hoisted_fixed && let Some(body) = tree.get(tree.body_id) {
        any_transform |= !body.computed_style.transform.is_identity;
    }

    if !any_transform {
        // Same sum, bottom-up: the offsets do not depend on the direction of
        // travel, only the transform composition does.
        let (mut x, mut y) = (0.0_f64, 0.0_f64);
        let mut current = Some(node_id);
        while let Some(id) = current {
            let Some(node) = tree.get(id) else { break };
            let (nx, ny) = painted_origin_step(tree, node, x, y, scale);
            x = nx;
            y = ny;
            if node.computed_style.position == PositionValue::Fixed || id == tree.body_id {
                break;
            }
            if let Some(parent) = node.parent.and_then(|pid| tree.get(pid)) {
                x -= parent.scroll_offset.0 * scale;
                y -= parent.scroll_offset.1 * scale;
            }
            current = node.parent;
        }
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
        if node.computed_style.position == PositionValue::Fixed || id == tree.body_id {
            break;
        }
        current = node.parent;
    }

    let mut off_x = 0.0_f64;
    let mut off_y = 0.0_f64;
    // Seed with the body's transform for a hoisted fixed box — unless the body
    // *is* that box, in which case the loop below composes it and seeding here
    // would apply it twice.
    let mut transform = if hoisted_fixed && chain.last() != Some(&tree.body_id) {
        body_paint_transform(tree, scale)
    } else {
        Affine::IDENTITY
    };
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    for &id in chain.iter().rev() {
        let Some(node) = tree.get(id) else { break };
        let (nx, ny) = painted_origin_step(tree, node, off_x, off_y, scale);
        x = nx;
        y = ny;
        transform = compose_transform_step(node, x, y, scale, transform);
        // Children resolve against this node's scroll-adjusted origin.
        off_x = x - node.scroll_offset.0 * scale;
        off_y = y - node.scroll_offset.1 * scale;
    }
    (x, y, transform)
}

/// The axis-aligned box a node is *painted* in — the desktop answer to
/// `getBoundingClientRect()`.
///
/// This is [`compute_absolute_position_and_transform`] with the node's own size
/// pushed through the composed transform. A rotated box has no axis-aligned
/// answer, so this is its bounding box; for the translate and scale that every
/// real transformed layout uses it is exact, and it is the same approximation
/// the browser makes.
///
/// `scale` is the DPI scale factor the result is expressed in — pass `1.0` to
/// work in layout pixels.
pub fn painted_border_box(tree: &NodeTree, node_id: RawNodeId, scale: f64) -> Rect {
    let (x, y, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
    let (w, h) = tree.get(node_id).map_or((0.0, 0.0), |node| {
        (
            node.layout.width as f64 * scale,
            node.layout.height as f64 * scale,
        )
    });
    transform.transform_rect_bbox(Rect::new(x, y, x + w, y + h))
}

/// Map a viewport point **into** a node's own painted space, expressed relative
/// to the node's border-box origin.
///
/// The inverse direction of [`painted_border_box`], and the one every
/// screen-point-to-content question needs: which character was clicked, where
/// inside a render surface the pointer is, how far along a scrollbar track a
/// press landed. Subtracting a painted AABB's origin is **not** a substitute —
/// under `transform: scale(2)` a point 40px right of the box's painted left
/// edge is 20px into the box's own space, not 40.
///
/// Returns `None` when the composed transform is not invertible (`scale(0)`, a
/// degenerate matrix): such a subtree paints to zero area, so no screen point
/// corresponds to anything inside it. `hit_test`'s `local_point` takes the same
/// exit for the same reason.
///
/// `scale` is the DPI scale factor `px`/`py` are expressed in — pass `1.0` to
/// work in layout pixels.
pub fn point_in_painted_box(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    px: f64,
    py: f64,
) -> Option<(f64, f64)> {
    let (x, y, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
    if transform == Affine::IDENTITY {
        return Some((px - x, py - y));
    }
    let det = transform.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let p = transform.inverse() * Point::new(px, py);
    if !p.x.is_finite() || !p.y.is_finite() {
        return None;
    }
    Some((p.x - x, p.y - y))
}

/// Map a point given relative to a node's border-box origin, in the node's own
/// painted space, **out** to viewport coordinates.
///
/// The forward direction of [`point_in_painted_box`], for the callers that
/// produce a position inside a box and need to place something at it in window
/// space — the caret, the IME candidate rectangle, a probe point for a
/// subsequent hit test.
///
/// `scale` is the DPI scale factor `lx`/`ly` are expressed in and the result is
/// returned in — pass `1.0` to work in layout pixels.
pub fn point_from_painted_box(
    tree: &NodeTree,
    node_id: RawNodeId,
    scale: f64,
    lx: f64,
    ly: f64,
) -> (f64, f64) {
    let (x, y, transform) = compute_absolute_position_and_transform(tree, node_id, scale);
    let p = transform * Point::new(x + lx, y + ly);
    (p.x, p.y)
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
        // A hole is only worth cutting if something will fill it. A viewport
        // that declares itself not ready — rinch-video before its first decoded
        // frame, or after a `PlaybackState::Error` — keeps its ancestors'
        // backgrounds intact; otherwise a video that never loads is see-through
        // to the desktop on a transparent window (issue #186).
        //
        // The attribute is an opt-OUT: a node that does not carry it punches
        // unconditionally, which is what `GameViewport` wants — the game owns
        // its hole from the first frame and stamps nothing. A node that does
        // carry it must say `"true"` to punch, so a mis-stamped value fails to
        // the safe side (an opaque placeholder, never a see-through window).
        let ready = node
            .attributes
            .get("data-viewport-ready")
            .is_none_or(|v| v == "true");
        if active && ready {
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
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // Content already drawn as inline boxes, via `paint_inline_layout`:
    // painting it again as a box would double it. See `drawn_by_its_ifc`.
    let already_drawn_inline = |child: &Node, _kind: PaintKind| drawn_by_its_ifc(tree, child);

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

    // Nothing inside `opacity: 0` can be seen, so nothing inside it is drawn.
    //
    // This is not an approximation. `opacity < 1` makes a node a stacking
    // context (`Node::creates_stacking_context`), so its whole subtree is
    // composited through the one group layer this node opens, and that layer is
    // composited back with `SourceOver` at alpha 0 — which writes no pixel,
    // whatever was painted into it. Every draw between the `push_layer` and the
    // `pop_layer` is therefore work with no output.
    //
    // Skipping the group rather than painting it is worth a great deal more
    // than it looks, because of what the group *costs* in the software painter:
    // `TinySkiaPainter::push_layer` allocates a second pixmap the size of the
    // whole surface and `pop_layer` composites all of it back. The idiom this
    // was found in is the always-mounted bottom sheet — a full-screen scrim
    // that fades in, parked at `opacity: 0` until a chip is tapped. On the
    // moto g stylus 5G, at 1080×2460, one such scrim cost about 48ms a frame
    // while being invisible: a 10MB pixmap allocation, an 18ms full-screen fill
    // into it, and a 28ms full-screen composite back out. The app carries three
    // of those sheets on its library screen, so roughly 145ms of every 290ms
    // frame was spent drawing three things that were not there. See card K24.
    //
    // The cut is on paint only. Hit-testing and layout do not come through
    // here, and CSS keeps an `opacity: 0` element in the box tree and reachable
    // by the pointer — which is exactly what a sheet parked at zero relies on.
    //
    // `display: contents` is the one exception, and it is the same exception
    // the zero-size branch below already makes: it generates no box, so there
    // is no group for opacity to composite and no layer is pushed for it — its
    // children paint into the grandparent at full strength, before this change
    // and after it.
    if node.computed_style.opacity <= 0.0 && node.computed_style.display != DisplayValue::Contents {
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
                // The element's own rect is zero-area and Vello clips a layer
                // to its bounds shape, which would blank the subtree. The
                // subtree's own extent is the right answer here as it is
                // everywhere else, and [`opacity_layer_bounds`] falls back to
                // the conservative `UNBOUNDED` rect this branch used to pass
                // whenever it cannot work one out (card K36).
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
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

            // Opacity. Bounds as in the general element arm below: the
            // subtree's painted extent, not this element's border box.
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
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
        // Inline painting for a `data-viewport` node whose frame arrives by
        // name — the software backend's video path (issue #358).
        //
        // Sibling of the `data-render-surface` arm below and for the same
        // reason: a frame drawn *here*, during paint, sits at the node's own
        // z-order, so a drawer or a modal painted after it covers it by
        // ordinary paint order. The software backend used to blit video onto
        // the finished pixel buffer instead, clipped only by its
        // overflow-clipping ancestors, which destroyed every overlay above it.
        //
        // The guard is the map, not the attribute: with no entry for this name
        // the node falls through to normal element painting, which is what
        // leaves `GameViewport` and the whole GPU compositor path untouched —
        // that backend never sets `VIEWPORT_PIXELS` at all.
        NodeKind::Element(_)
            if node
                .attributes
                .get("data-viewport")
                .is_some_and(|name| has_viewport_pixels(name)) =>
        {
            let rect = Rect::new(x, y, x + w, y + h);
            let opacity = node.computed_style.opacity;
            if opacity < 1.0 {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }

            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );
            if visible {
                // Opaque black over the whole box, then the frame fitted inside
                // it — so the letterbox bars are black, which is what a browser
                // paints for `<video>` (issue #354's software half).
                //
                // The black is paint's to draw, not the element's `background`:
                // rinch-video flips that to `transparent` the moment a frame
                // arrives, because the GPU backend composites video *under* the
                // UI and an opaque element background would hide it. rinch-video
                // cannot know which backend it is running on, so the backend
                // that paints the frame is the one that owns its backdrop.
                //
                // Rounded like any other background: a `border-radius` on the
                // viewport must not leave square black corners poking out of
                // the shape the author asked for, and the frame is clipped to
                // the same shape.
                let (backdrop, has_radius) = {
                    let cs = &node.computed_style;
                    let resolve_size = node.layout.width.min(node.layout.height);
                    let tl =
                        cs.border_radius_top_left.resolve(resolve_size).max(0.0) as f64 * scale;
                    let tr =
                        cs.border_radius_top_right.resolve(resolve_size).max(0.0) as f64 * scale;
                    let br =
                        cs.border_radius_bottom_right.resolve(resolve_size).max(0.0) as f64 * scale;
                    let bl =
                        cs.border_radius_bottom_left.resolve(resolve_size).max(0.0) as f64 * scale;
                    if tl > 0.0 || tr > 0.0 || br > 0.0 || bl > 0.0 {
                        let radii = RoundedRectRadii::new(tl, tr, br, bl);
                        (RoundedRect::from_rect(rect, radii).into(), true)
                    } else {
                        (painter::PaintShape::from(rect), false)
                    }
                };
                painter.fill_color(
                    Fill::NonZero,
                    node_transform,
                    AlphaColor::<Srgb>::BLACK,
                    &backdrop,
                );

                if has_radius {
                    painter.push_clip(Fill::NonZero, node_transform, &backdrop);
                }
                VIEWPORT_PIXELS.with(|vp| {
                    let guard = vp.borrow();
                    // The guard above already proved this entry exists and is
                    // drawable; `viewport_frame_bytes` re-derives the exact
                    // slice length the painter needs, because a buffer longer
                    // than `w * h * 4` would be rejected outright.
                    let Some((pixels, bytes)) = guard
                        .as_ref()
                        .and_then(|map| map.get(node.attributes.get("data-viewport")?))
                        .and_then(|pixels| Some((pixels, viewport_frame_bytes(pixels)?)))
                    else {
                        return;
                    };
                    image::paint_image_data(
                        painter,
                        &pixels.data[..bytes],
                        pixels.width,
                        pixels.height,
                        rect,
                        scale,
                        crate::computed_style::ObjectFitValue::Contain,
                        node_transform,
                    );
                });
                if has_radius {
                    painter.pop_layer();
                }
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
                                        let bounds =
                                            opacity_layer_shape(tree, node_id, scale, x, y);
                                        painter.push_layer(
                                            BlendMode::Normal,
                                            opacity,
                                            node_transform,
                                            &bounds,
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

                                    // Paint the surface pixels inline, like an
                                    // image — over the borrowed buffer. A live
                                    // frame source must not be cloned into a
                                    // `DecodedImage` first: that is a whole
                                    // frame of memcpy per frame, for nothing.
                                    image::paint_image_data(
                                        painter,
                                        &pixels.data,
                                        pixels.width,
                                        pixels.height,
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
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
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
                let tl = cs.border_radius_top_left.resolve(resolve_size).max(0.0) as f64 * scale;
                let tr = cs.border_radius_top_right.resolve(resolve_size).max(0.0) as f64 * scale;
                let br =
                    cs.border_radius_bottom_right.resolve(resolve_size).max(0.0) as f64 * scale;
                let bl = cs.border_radius_bottom_left.resolve(resolve_size).max(0.0) as f64 * scale;
                let radii = RoundedRectRadii::new(tl, tr, br, bl);
                // Uniform radius for code paths that don't support per-corner yet
                let avg = (tl + tr + br + bl) / 4.0;
                (avg, radii)
            };

            // Get opacity from computed style and push layer if needed
            // The layer's bounds are the union of what the subtree
            // actually paints, not this element's border box —
            // [`opacity_layer_bounds`]. tiny-skia ignores the shape; Vello
            // clips every command in the layer to it, and a stacking context
            // does not clip its descendants, so a box shadow or an overflowing
            // child used to be drawn by one painter and thrown away by the
            // other (card K36).
            let opacity = node.computed_style.opacity;
            let has_opacity = opacity < 1.0;
            if has_opacity {
                let bounds = opacity_layer_shape(tree, node_id, scale, x, y);
                painter.push_layer(BlendMode::Normal, opacity, node_transform, &bounds);
            }

            // Handle overflow clipping — detect early so we can cut holes
            // in the background for viewport descendants.
            let overflow_y = node.computed_style.overflow_y;
            let overflow_x = node.computed_style.overflow_x;
            let clips = matches!(
                overflow_y,
                OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
            );

            // Whether the background below is going to write a pixel at all.
            //
            // Usually it is not, because the colour is fully transparent, which
            // is the case for most elements on most pages. `background-color`'s
            // initial value is `transparent`, and Stylo hands that back as a
            // real colour
            // rather than as an absence, so `from_stylo` turns it into
            // `BackgroundValue::Color(rgba(0, 0, 0, 0))` — a value this code
            // then dutifully filled. A `SourceOver` fill at alpha 0 writes no
            // pixel, so every one of those was a rasterisation of the element's
            // whole box for no output at all, and it cost in proportion to the
            // box: on the moto g stylus 5G a full-screen one measured 11–12ms,
            // and a single frame of the library screen spent about 70ms on
            // eight of them. See card K24.
            //
            // The skip is here, in paint, and deliberately not in `from_stylo`:
            // `transition/diff.rs` interpolates a `background-color` transition
            // only when both ends are `Color`, so collapsing transparent to
            // `None` in the computed style would silently stop
            // `transparent → red` from animating. Paint is the layer that gets
            // to decide something is not worth drawing; the style has to keep
            // the colour it was given.
            let paints_a_background = visible
                && match &node.computed_style.background {
                    BackgroundValue::None => false,
                    BackgroundValue::Color(c) => c.components[3] > 0.0,
                    _ => true,
                };

            // Find viewport descendants — their rects will be cut out of
            // the background fill so the compositor layer shows through.
            //
            // Only worth walking the subtree for when there *is* a background
            // fill to cut them out of: the holes have no other consumer, and
            // `clips` is true of every `overflow: hidden` box on the page.
            let mut viewport_holes = Vec::new();
            if clips && paints_a_background {
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
                // `viewport_holes` is only ever collected for a background
                // that will be painted, so testing it first also covers the
                // "nothing to fill" case.
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
                } else if paints_a_background {
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
                );
            }

            if clips {
                painter.pop_layer();
            }

            // Paint scrollbar overlays for scroll containers.
            //
            // Both axes, from one set of metrics (#178): thickness 6, margin 2,
            // minimum thumb 20, 40% black, fully rounded — the numbers the
            // vertical bar has always used, so the two read as one feature.
            //
            // The geometry itself lives in `paint::scrollbar`, shared with the
            // desktop input path, because the two used to derive it separately
            // and had drifted apart (#400): a drag did not move the thumb the
            // distance the pointer moved. Anything about *where* a bar is
            // belongs there; what is left here is how it is drawn.
            //
            // Gated up front on the two overflow enums captured before the
            // children were painted, so a node that scrolls on neither axis —
            // almost every node — pays two enum checks and nothing else.
            if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto)
                || matches!(overflow_x, OverflowValue::Scroll | OverflowValue::Auto)
            {
                let node = tree.get(node_id).unwrap(); // re-borrow after children done
                let bars = scrollbar::scrollbars(tree, node_id, scale);
                let thickness = bars.thickness;
                let margin = bars.margin;

                // The thumb's colour follows `--rinch-scrollbar-color`, or the
                // container's own palette when that says `auto` (#416).
                // `--rinch-scrollbar-width: none` reaches here as "no bars",
                // so nothing below runs for it.
                let mut fill_rounded = |rect: Rect, colour: AlphaColor<Srgb>| {
                    let shape = RoundedRect::from_rect(rect, thickness * 0.5);
                    painter.fill(
                        Fill::NonZero,
                        node_transform,
                        &Brush::Solid(colour),
                        &shape.into(),
                    );
                };

                if let Some(track) = bars.vertical {
                    let scrollbar_x = x + w - thickness - margin;
                    // A track is only painted when asked for: rinch's bar is an
                    // overlay, and a track under it would change the look of
                    // every existing app.
                    if let Some(track_color) = bars.track_color {
                        fill_rounded(
                            Rect::new(
                                scrollbar_x,
                                y + track.track_start,
                                scrollbar_x + thickness,
                                y + track.track_start + track.track_len,
                            ),
                            track_color,
                        );
                    }
                    let thumb_y = y + track.thumb_start(node.scroll_offset.1 * scale);
                    fill_rounded(
                        Rect::new(
                            scrollbar_x,
                            thumb_y,
                            scrollbar_x + thickness,
                            thumb_y + track.thumb_len,
                        ),
                        bars.thumb_color,
                    );
                }

                if let Some(track) = bars.horizontal {
                    let scrollbar_y = y + h - thickness - margin;
                    if let Some(track_color) = bars.track_color {
                        fill_rounded(
                            Rect::new(
                                x + track.track_start,
                                scrollbar_y,
                                x + track.track_start + track.track_len,
                                scrollbar_y + thickness,
                            ),
                            track_color,
                        );
                    }
                    let thumb_x = x + track.thumb_start(node.scroll_offset.0 * scale);
                    fill_rounded(
                        Rect::new(
                            thumb_x,
                            scrollbar_y,
                            thumb_x + track.thumb_len,
                            scrollbar_y + thickness,
                        ),
                        bars.thumb_color,
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
                        // Darken: overlay black with alpha = 1.0 - brightness.
                        // Hand the painter the float and let it do the single
                        // 8-bit quantisation, by rounding — `x as u8` truncated,
                        // biasing every filtered element one level light (#260).
                        let dark = AlphaColor::<Srgb>::BLACK
                            .with_alpha((1.0 - brightness).clamp(0.0, 1.0));
                        if radius > 0.0 {
                            let rrect = rect.to_rounded_rect(radii);
                            painter.fill_color(Fill::NonZero, node_transform, dark, &rrect.into());
                        } else {
                            painter.fill_color(Fill::NonZero, node_transform, dark, &rect.into());
                        }
                    } else if brightness > 1.0 {
                        // Brighten: overlay white with alpha proportional to
                        // excess brightness. Same rounding note as above.
                        let light = AlphaColor::<Srgb>::WHITE
                            .with_alpha((brightness - 1.0).clamp(0.0, 1.0));
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

#[cfg(test)]
mod tests {
    use crate::RinchDocument;
    use peniko::kurbo::Affine;
    use rinch_core::dom::DomDocument;

    // The correctness criterion for every test here: `painted_border_box`
    // returns the box the node is *painted* in. So the expected rects are
    // hand-computed from the CSS — from the padding, the transform, the
    // hoisting rule — never from another geometry helper, or the test would
    // just re-derive whatever bug is in one.
    //
    // Each test also asserts the *wrong* answer explicitly: the box the
    // unhardened walk produced. A positive assertion alone survives most of
    // these regressions, because the two boxes overlap.

    fn child_of(doc: &mut RinchDocument, parent: rinch_core::dom::NodeId, style: &str) -> usize {
        let el = doc.create_element("div");
        doc.set_attribute(el, "style", style);
        doc.append_child(parent, el);
        el.0
    }

    /// The plain parent-chain sum: layout offsets minus ancestor scroll, with
    /// no transform and no IFC content-box offset. This is what every walk in
    /// the codebase used to do, and it is the box the assertions below must
    /// *not* return.
    fn untransformed_origin(doc: &RinchDocument, node_id: usize) -> (f32, f32) {
        let (mut x, mut y) = (0.0_f32, 0.0_f32);
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            let Some(n) = doc.tree.get(id) else { break };
            x += n.layout.x;
            y += n.layout.y;
            if let Some(p) = n.parent.and_then(|pid| doc.tree.get(pid)) {
                x -= p.scroll_offset.0 as f32;
                y -= p.scroll_offset.1 as f32;
            }
            cur = n.parent;
        }
        (x, y)
    }

    /// A plain translated ancestor: the painted box is the layout box shifted
    /// by the ancestor's translate. This is the case `Drag::percent()` gets
    /// wrong on a slider inside a centred modal (#203).
    #[test]
    fn an_ancestor_translate_moves_the_painted_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let panel = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: translate(-50px, -30px)",
        );
        let track = child_of(
            &mut doc,
            rinch_core::dom::NodeId(panel),
            "position: absolute; left: 100px; top: 60px; width: 200px; height: 20px",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout box (100,60)-(300,80); translate(-50,-30) paints it at
        // (50,30)-(250,50).
        let r = super::painted_border_box(&doc.tree, track, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (50.0, 30.0, 250.0, 50.0));
        assert_eq!(
            untransformed_origin(&doc, track),
            (100.0, 60.0),
            "the untransformed sum is the box this must NOT report"
        );
    }

    /// A `scale()` ancestor changes the painted *size*, not just the origin —
    /// which is why an AABB helper cannot just offset the layout box.
    #[test]
    fn an_ancestor_scale_resizes_the_painted_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // transform-origin at the top-left keeps the arithmetic hand-checkable.
        let zoom = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: scale(2); transform-origin: 0 0",
        );
        let track = child_of(
            &mut doc,
            rinch_core::dom::NodeId(zoom),
            "position: absolute; left: 40px; top: 25px; width: 100px; height: 10px",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout box (40,25)-(140,35), doubled about (0,0): (80,50)-(280,70).
        let r = super::painted_border_box(&doc.tree, track, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (80.0, 50.0, 280.0, 70.0));
        assert_eq!(
            (r.width(), r.height()),
            (200.0, 20.0),
            "a scaled box is painted at twice its layout size"
        );
    }

    /// Gap 1. A `display: contents` node generates no box, so `paint_node`'s
    /// zero-size branch passes the parent transform straight to the children —
    /// a transform declared on it is never applied. The forward walk used to
    /// compose it anyway and moved the child somewhere paint does not draw it.
    #[test]
    fn a_display_contents_ancestor_contributes_no_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; padding: 30px 20px",
        );
        let ghost = child_of(
            &mut doc,
            rinch_core::dom::NodeId(container),
            "display: contents; transform: translate(50px, 20px)",
        );
        let child = child_of(
            &mut doc,
            rinch_core::dom::NodeId(ghost),
            "width: 80px; height: 40px",
        );
        doc.resolve_layout(800.0, 600.0);

        // The transform is really declared — otherwise this test proves nothing.
        assert!(
            !doc.tree
                .get(ghost)
                .unwrap()
                .computed_style
                .transform
                .is_identity,
            "the display:contents node must actually carry a transform"
        );

        // The child is flattened into the container's content box: (20,30),
        // and nothing translates it.
        let (_, _, t) = super::compute_absolute_position_and_transform(&doc.tree, child, 1.0);
        assert_eq!(t, Affine::IDENTITY, "no transform reaches the child");
        let r = super::painted_border_box(&doc.tree, child, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (20.0, 30.0, 100.0, 70.0));
        assert_ne!(
            (r.x0, r.y0),
            (70.0, 50.0),
            "the display:contents translate must not displace the child"
        );
    }

    /// Gap 2. A hoisted `position: fixed` box drops every ancestor transform
    /// but keeps the body's: `paint_children_with_stacking` paints the body's
    /// stacking sequence — the hoisted fixed entries included — under the
    /// body's own composed transform. Hit testing already agrees
    /// (`body_transform_applies_to_a_hoisted_fixed_descendant`); this walk
    /// started from the identity, so the two disagreed.
    #[test]
    fn a_hoisted_fixed_box_paints_under_the_body_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "transform: translate(40px, 30px)");
        let outer = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: translate(200px, 100px)",
        );
        let fixed = child_of(
            &mut doc,
            rinch_core::dom::NodeId(outer),
            "position: fixed; left: 50px; top: 60px; width: 100px; height: 40px; z-index: 5",
        );
        doc.resolve_layout(800.0, 600.0);

        // Viewport box (50,60)-(150,100), shifted by body's translate only:
        // (90,90)-(190,130). `outer`'s translate must not reach it.
        let r = super::painted_border_box(&doc.tree, fixed, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (90.0, 90.0, 190.0, 130.0));
        assert_ne!(
            (r.x0, r.y0),
            (50.0, 60.0),
            "the body's transform does reach a hoisted fixed box"
        );
        assert_ne!(
            (r.x0, r.y0),
            (290.0, 190.0),
            "the transformed ancestor's translate must not reach it"
        );
    }

    /// Gap 3. An inline-block an IFC positions stores `layout.x`/`layout.y`
    /// relative to the IFC root's *content* box, while the parent chain sums
    /// border-box origins. Paint bridges the two by handing
    /// `paint_inline_layout` the content-box origin, so a walk that omits
    /// `ifc_content_box_offset` reports the box one padding+border up-left of
    /// where it is drawn.
    #[test]
    fn an_inline_block_is_placed_against_its_ifc_content_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 20px; border: 4px solid black; font-size: 16px",
        );
        doc.append_child(body, container);
        let before = doc.create_text("Click the ");
        doc.append_child(container, before);
        let button = doc.create_element("button"); // inline-block by default
        doc.set_attribute(button, "style", "width: 60px; height: 24px");
        doc.append_child(container, button);
        let label = doc.create_text("OK");
        doc.append_child(button, label);
        doc.resolve_layout(800.0, 600.0);

        assert!(
            doc.tree.get(container.0).unwrap().text_layout.is_some(),
            "the container must be the IFC root"
        );
        // The button's own layout.x/y are Parley's, relative to the content
        // box; the chain sum lands on the border-box origin. The painted origin
        // is that sum plus padding (20) + border (4) on each axis.
        let (sum_x, sum_y) = untransformed_origin(&doc, button.0);
        let r = super::painted_border_box(&doc.tree, button.0, 1.0);
        assert_eq!(
            (r.x0 as f32, r.y0 as f32),
            (sum_x + 24.0, sum_y + 24.0),
            "the inline-block paints inside the IFC root's padding and border"
        );
        assert_ne!(
            (r.x0 as f32, r.y0 as f32),
            (sum_x, sum_y),
            "the plain border-box sum is one padding+border off"
        );
    }

    /// Gaps 2 and 3 meeting. A hoisted `position: fixed` box is painted by
    /// `paint_node` from the body's sequence at a zero offset, never by
    /// `paint_inline_layout`, so no IFC content-box offset may reach it even
    /// when it is written inline in a padded text flow. `hit_test_node` spends
    /// an explicit branch on this; the walk here does not need one, because
    /// Stylo blockifies an out-of-flow box and `ifc_content_box_offset` then
    /// answers `(0, 0)` on its own — which the assertion below states, so that
    /// a Stylo change removing that blockification fails here instead of
    /// silently displacing every fixed overlay written inside a paragraph.
    #[test]
    fn a_fixed_inline_block_keeps_its_viewport_origin() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 20px; border: 4px solid black; font-size: 16px",
        );
        doc.append_child(body, container);
        let before = doc.create_text("Overlay: ");
        doc.append_child(container, before);
        // Two `<button>`s, inline-block by tag, in the same IFC. The in-flow one
        // is the control: it must take the 24px offset, or the fixed one taking
        // none proves nothing about the exemption.
        let inflow = doc.create_element("button");
        doc.set_attribute(inflow, "style", "width: 60px; height: 24px");
        doc.append_child(container, inflow);
        let overlay = doc.create_element("button");
        doc.set_attribute(
            overlay,
            "style",
            "position: fixed; left: 30px; top: 40px; width: 60px; height: 24px",
        );
        doc.append_child(container, overlay);
        doc.resolve_layout(800.0, 600.0);

        let control = doc.tree.get(inflow.0).unwrap();
        assert_eq!(
            super::ifc_content_box_offset(&doc.tree, control),
            (24.0, 24.0),
            "the control really is an inline-block this IFC displaces"
        );
        let (cx, _) = untransformed_origin(&doc, inflow.0);
        assert_eq!(
            super::painted_border_box(&doc.tree, inflow.0, 1.0).x0 as f32,
            cx + 24.0,
            "the in-flow inline-block paints inside the padding and border"
        );

        // Why the fixed one takes none: `<button>` is inline-block by tag, but
        // out-of-flow blockifies it, so `ifc_content_box_offset` already
        // answers zero and the walk needs no branch of its own.
        let n = doc.tree.get(overlay.0).unwrap();
        assert_eq!(
            super::ifc_content_box_offset(&doc.tree, n),
            (0.0, 0.0),
            "an out-of-flow box takes no IFC content-box offset"
        );

        let r = super::painted_border_box(&doc.tree, overlay.0, 1.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (30.0, 40.0, 90.0, 64.0));
        assert_ne!(
            (r.x0, r.y0),
            (54.0, 64.0),
            "the IFC root's padding+border must not displace a hoisted fixed box"
        );
    }

    /// The consumer this walk already had. `compute_dirty_region` narrows a
    /// software repaint to the changed nodes' rects, so a rect computed
    /// somewhere other than where the node paints leaves part of the real draw
    /// outside the region and unpainted — a defect that shows on a cold full
    /// repaint and not on a partial one, which is the shape #369 describes.
    ///
    /// The oracle is `painted_border_box` because that is now the same walk
    /// `paint_node` follows; the assertion is containment, which is the
    /// property the region has to have.
    #[test]
    fn the_dirty_region_covers_where_an_inline_block_paints() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 40px; border: 6px solid black; font-size: 16px",
        );
        doc.append_child(body, container);
        let before = doc.create_text("Press ");
        doc.append_child(container, before);
        let button = doc.create_element("button");
        doc.set_attribute(button, "style", "width: 60px; height: 24px");
        doc.append_child(container, button);
        doc.resolve_layout(800.0, 600.0);

        doc.tree.paint_dirty_nodes.clear();
        doc.tree.paint_dirty_nodes.push(button.0);
        let region =
            super::compute_dirty_region(&doc.tree, 1.0, 800.0, 600.0).expect("a dirty node");
        let painted = super::painted_border_box(&doc.tree, button.0, 1.0);

        // The 46px content-box offset is real, so the two candidate rects are
        // disjoint enough for containment to be a meaningful test.
        let (sum_x, sum_y) = untransformed_origin(&doc, button.0);
        assert_eq!(
            (painted.x0 as f32, painted.y0 as f32),
            (sum_x + 46.0, sum_y + 46.0)
        );

        assert!(
            region.x0 <= painted.x0
                && region.y0 <= painted.y0
                && region.x1 >= painted.x1
                && region.y1 >= painted.y1,
            "the dirty region {region:?} must cover the painted box {painted:?}"
        );
        // And the box it would have covered instead does not reach the draw:
        // 46px of offset against a 4px anti-aliasing margin.
        assert!(
            (sum_x as f64) + 60.0 + 4.0 < painted.x1,
            "the un-offset rect stops short of the painted box's right edge"
        );
    }

    /// The inverse direction, and why it is a separate helper. Under `scale(2)`
    /// a point 120 painted pixels right of the box's painted left edge is 60
    /// pixels into the box; subtracting the painted AABB's origin would answer
    /// 120 and pick the wrong character, the wrong scrollbar position, the wrong
    /// pixel of a render surface.
    #[test]
    fn a_point_maps_into_a_scaled_box_divided_not_merely_offset() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let zoom = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; \
             transform: scale(2); transform-origin: 0 0",
        );
        let field = child_of(
            &mut doc,
            rinch_core::dom::NodeId(zoom),
            "position: absolute; left: 30px; top: 20px; width: 200px; height: 40px",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout origin (30,20) → painted (60,40).
        let r = super::painted_border_box(&doc.tree, field, 1.0);
        assert_eq!((r.x0, r.y0), (60.0, 40.0));

        let local = super::point_in_painted_box(&doc.tree, field, 1.0, 180.0, 100.0)
            .expect("an invertible transform");
        assert_eq!(local, (60.0, 30.0), "(180-60)/2 and (100-40)/2");
        assert_ne!(
            local,
            (120.0, 60.0),
            "subtracting the painted origin without dividing is the wrong answer"
        );

        // And back out again.
        assert_eq!(
            super::point_from_painted_box(&doc.tree, field, 1.0, 60.0, 30.0),
            (180.0, 100.0),
            "the forward map is the inverse of the backward one"
        );
    }

    /// A `scale(0)` subtree paints to zero area, so no screen point corresponds
    /// to anything inside it — the same exit `hit_test`'s `local_point` takes.
    #[test]
    fn a_degenerate_transform_has_no_local_point() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let collapsed = child_of(
            &mut doc,
            body,
            "position: relative; width: 200px; height: 100px; transform: scale(0)",
        );
        let inner = child_of(
            &mut doc,
            rinch_core::dom::NodeId(collapsed),
            "width: 50px; height: 20px",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            super::point_in_painted_box(&doc.tree, inner, 1.0, 10.0, 10.0),
            None
        );
    }

    /// A `position: fixed` box maps points against its viewport box, not against
    /// one displaced by a scrolled ancestor — the exception seven of the walks
    /// this PR converted never had.
    #[test]
    fn a_fixed_box_maps_points_against_its_viewport_origin() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let page = child_of(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; overflow: auto",
        );
        child_of(
            &mut doc,
            rinch_core::dom::NodeId(page),
            "width: 100%; height: 2000px",
        );
        let overlay = child_of(
            &mut doc,
            rinch_core::dom::NodeId(page),
            "position: fixed; left: 40px; top: 60px; width: 120px; height: 50px",
        );
        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[page].scroll_offset.1 = 150.0;

        assert_eq!(
            super::point_in_painted_box(&doc.tree, overlay, 1.0, 100.0, 80.0),
            Some((60.0, 20.0)),
            "measured from the viewport box (40,60)"
        );
        assert_ne!(
            super::point_in_painted_box(&doc.tree, overlay, 1.0, 100.0, 80.0),
            Some((60.0, 170.0)),
            "the page's scroll must not be added back in"
        );
    }
}
