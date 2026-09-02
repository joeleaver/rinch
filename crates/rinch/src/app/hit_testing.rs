// ── Free functions (platform-agnostic hit testing) ───────────────────────────

use super::ScrollAxis;
use rinch_dom::stacking::{paints_at_stacking_root, stacking_paint_order};

/// Simple hit testing: find the deepest node whose layout rect contains (x, y).
/// Respects CSS stacking contexts so that elements with higher z-index
/// are tested before visually-behind siblings, and CSS transforms so that a
/// transformed subtree is hit where it is *painted* (#199).
pub(crate) fn hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<usize> {
    hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y, x, y)
}

/// Map `(px, py)` — a point in the coordinate space the node's layout box lives
/// in — into the node's own space by inverting the node's CSS transform.
///
/// `paint_node` draws a node's *untransformed* box under the affine composed by
/// [`rinch_dom::paint::compose_node_transform`] and hands that same affine to the
/// node's children. So the mirror image on the input side is to push the probe
/// point through the inverse: every bounds test then happens in the box's own
/// space, against the plain layout rect. Composition falls out for free — each
/// level inverts only its own step, and the product of those inverses is the
/// inverse of paint's accumulated transform.
///
/// `scale = 1.0` because hit testing works in layout pixels while paint works in
/// physical pixels. `compose_node_transform` is covariant in that argument — it
/// multiplies the CSS-px translate components by `scale`, and the linear part
/// (scale/rotate/skew) is unit-invariant — so composing at `scale = 1.0` yields
/// exactly paint's transform expressed in layout pixels, at any DPI (#202).
///
/// Returns `None` when the transform is not invertible (`scale(0)`, a degenerate
/// matrix): such a subtree paints to zero area, so nothing in it can be hit.
fn local_point(node: &rinch_dom::Node, nx: f32, ny: f32, px: f32, py: f32) -> Option<(f32, f32)> {
    use peniko::kurbo::{Affine, Point};

    if node.computed_style.transform.is_identity {
        return Some((px, py));
    }
    // A display:contents node has no box, so paint gives it no transform box
    // either — `paint_node`'s zero-size branch passes the parent transform
    // straight through to the children.
    if node.computed_style.display == rinch_dom::computed_style::DisplayValue::Contents
        && (node.layout.width == 0.0 || node.layout.height == 0.0)
    {
        return Some((px, py));
    }

    let t =
        rinch_dom::paint::compose_node_transform(node, nx as f64, ny as f64, 1.0, Affine::IDENTITY);
    let det = t.determinant();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let p = t.inverse() * Point::new(px as f64, py as f64);
    if !p.x.is_finite() || !p.y.is_finite() {
        return None;
    }
    Some((p.x as f32, p.y as f32))
}

/// One level of a top-down probe-point descent: where the node's box is, and
/// where the probe point is once it has been moved into the node's own space.
///
/// Every walk in this file that asks "what is under this point" goes through
/// [`descend`] to produce one, so the scroll searches and the scrollbar search
/// cannot drift away from `hit_test` — or from paint — the way four hand-rolled
/// copies of the arithmetic did (#203).
struct Frame {
    /// The node's painted origin, in the space `x`/`y` are expressed in.
    nx: f32,
    ny: f32,
    /// The probe point in this node's own space — the space paint draws the
    /// node's untransformed box in, so a plain rect test is valid against it.
    x: f32,
    y: f32,
    /// The probe point in viewport space, carried down for `position: fixed`
    /// descendants.
    vx: f32,
    vy: f32,
}

/// Enter a node during a top-down probe-point walk, mirroring `paint_node`'s
/// own descent.
///
/// Four adjustments, in paint's order:
///
/// - a `position: fixed` box is viewport-relative, because paint hoists it to
///   the body level with zeroed offsets — so the probe resumes from `vx`/`vy`
///   and the node's own `layout.x`/`layout.y` alone;
/// - an inline-block an IFC positions stores `layout.x`/`layout.y` against the
///   IFC root's *content* box while the offsets accumulated here are border-box
///   origins, so [`rinch_dom::paint::ifc_content_box_offset`] bridges the two
///   (a fixed box is exempt — paint never routes it through
///   `paint_inline_layout`);
/// - the node's CSS transform is inverted into the probe point ([`local_point`]),
///   so everything below works in the box's own space;
/// - the body re-seeds viewport space from its own post-transform point, which
///   is the space paint hands the hoisted fixed entries.
///
/// `None` means nothing in this subtree can be under any point: the transform is
/// not invertible, so the subtree paints to zero area.
#[allow(clippy::too_many_arguments)]
fn descend(
    tree: &rinch_dom::NodeTree,
    node: &rinch_dom::Node,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<Frame> {
    let is_fixed = node.computed_style.position == rinch_dom::computed_style::PositionValue::Fixed;
    let (x, y) = if is_fixed { (vx, vy) } else { (x, y) };
    let (nx, ny) = if is_fixed {
        (node.layout.x, node.layout.y)
    } else {
        let (dx, dy) = rinch_dom::paint::ifc_content_box_offset(tree, node);
        (offset_x + node.layout.x + dx, offset_y + node.layout.y + dy)
    };

    let (x, y) = local_point(node, nx, ny, x, y)?;

    // Fixed descendants resolve against the viewport, which paint models as the
    // body level — so viewport space is body's *post*-transform space.
    let (vx, vy) = if node_id == tree.body_id {
        (x, y)
    } else {
        (vx, vy)
    };

    Some(Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    })
}

/// Hit-test a subtree.
///
/// `x`/`y` is the probe point in the coordinate space `offset_x`/`offset_y` live
/// in — i.e. the local (pre-transform) space of the nearest transformed ancestor,
/// which is the same space paint accumulates its offsets in. `vx`/`vy` is the
/// probe point in viewport space, kept for `position: fixed` subtrees which paint
/// hoists out of every ancestor's offset *and* transform.
#[allow(clippy::too_many_arguments)]
fn hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<usize> {
    let node = tree.get(node_id)?;

    // The `position: fixed` hoist, the IFC content-box offset, the transform
    // inverse and the body's viewport re-seed all live in `descend`, which every
    // probe-point walk in this file shares.
    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;

    let nw = node.layout.width;
    let nh = node.layout.height;

    // Skip entire subtree when visibility: hidden — per CSS spec, hidden elements
    // and their descendants don't receive pointer events. This also guards against
    // Stylo not cascading visibility to children when a parent's class changes.
    if matches!(
        node.computed_style.visibility,
        rinch_dom::computed_style::VisibilityValue::Hidden
            | rinch_dom::computed_style::VisibilityValue::Collapse
    ) {
        return None;
    }

    let point_in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;

    // Nodes with overflow clipping must restrict child hit testing to within bounds
    let clips_overflow = !matches!(
        node.computed_style.overflow_x,
        rinch_dom::computed_style::OverflowValue::Visible
    ) || !matches!(
        node.computed_style.overflow_y,
        rinch_dom::computed_style::OverflowValue::Visible
    );
    let check_children = !clips_overflow || point_in_bounds;

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    if check_children {
        // An IFC text node's layout is stretched to the whole container
        // (write_inline_positions, for scroll-height) — those artificial bounds
        // would shadow inline-block siblings laid out in the same text flow,
        // swallowing clicks meant for e.g. a button. Skip it: the IFC root
        // itself is returned for plain-text hits and text selection resolves by
        // walking up to it.
        let is_stretched_ifc_text =
            |child: &rinch_dom::Node| child.is_text() && child.ifc_root.is_some();

        let is_body = node_id == tree.body_id;
        if is_body || node.creates_stacking_context() {
            // A stacking-context root probes exactly the sequence paint draws,
            // read backwards — the last box painted is the first one tapped.
            // Same function, same offsets, opposite direction: the two cannot
            // drift apart the way two hand-written phase walks did.
            for entry in stacking_paint_order(
                tree,
                node_id,
                is_body,
                1.0,
                (nx - sx) as f64,
                (ny - sy) as f64,
            )
            .iter()
            .rev()
            {
                let Some(child) = tree.get(entry.node_id) else {
                    continue;
                };
                if is_stretched_ifc_text(child) {
                    continue;
                }
                if let Some(hit) = hit_test_node(
                    tree,
                    entry.node_id,
                    entry.offset_x as f32,
                    entry.offset_y as f32,
                    x,
                    y,
                    vx,
                    vy,
                ) {
                    return Some(hit);
                }
            }
        } else {
            // Not a stacking-context root — test only the children that were
            // not hoisted to an ancestor's sequence, in reverse tree order.
            for &child_id in node.children.iter().rev() {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if paints_at_stacking_root(child) || is_stretched_ifc_text(child) {
                    continue;
                }
                if let Some(hit) = hit_test_node(tree, child_id, nx - sx, ny - sy, x, y, vx, vy) {
                    return Some(hit);
                }
            }
        }
    }

    if !point_in_bounds {
        return None;
    }

    // Skip this element if pointer-events: none (children still checked above)
    if matches!(
        node.computed_style.pointer_events,
        rinch_dom::computed_style::PointerEventsValue::None
    ) {
        return None;
    }

    Some(node_id)
}

/// The box a node is *painted* in, in layout pixels, shaped as the four
/// `element_*` fields a [`rinch_core::events::ClickContext`] carries.
///
/// One conversion for every producer of those fields — the click path, the
/// context-menu path, the drag- and mouse-attribute paths, keyboard activation,
/// the ancestor chain and `bounds_signal` — so they cannot report different
/// rects for the same node. It is the `getBoundingClientRect()` answer
/// rinch-web already fills them from, which is what makes the same markup
/// behave the same on both backends (#203).
pub(crate) fn painted_element_box(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
) -> (f32, f32, f32, f32) {
    let r = rinch_dom::paint::painted_border_box(tree, node_id, 1.0);
    (
        r.x0 as f32,
        r.y0 as f32,
        r.width() as f32,
        r.height() as f32,
    )
}

/// A pointer position mapped **into** a node's own space, relative to the
/// node's border-box origin — the inverse of [`painted_element_box`].
///
/// The direction anything consuming a point *inside* a box needs: how far along
/// a scrollbar track a press landed, where in a render surface the pointer is,
/// which character a click chose. Subtracting the painted box's origin is not
/// the same answer — under `transform: scale(2)` a pointer 40px right of the
/// painted left edge is 20px into the box (#203).
///
/// Falls back to the raw point when the composed transform is not invertible
/// (a `scale(0)` subtree paints nothing, so there is no meaningful local
/// position and the caller's clamp is as good an answer as any).
pub(crate) fn pointer_in_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    x: f32,
    y: f32,
) -> (f32, f32) {
    rinch_dom::paint::point_in_painted_box(tree, node_id, 1.0, x as f64, y as f64)
        .map(|(lx, ly)| (lx as f32, ly as f32))
        .unwrap_or((x, y))
}

/// Detect whether a mouse position is near a window edge for resize.
///
/// All five arguments are in the same unit: **logical (CSS) pixels** — the
/// pointer position `PlatformEvent` carries, the logical layout viewport, and
/// `WindowProps::resize_inset`, which is documented as matching a CSS margin.
/// The caller used to hand this the physical `window_size` and a
/// `inset * scale_factor`, because the desktop shell fed physical pointer
/// coordinates; #299 moved the pointer into logical space, so the whole
/// comparison moved with it.
///
/// Returns the resize direction if the cursor is within the grab zone.
pub(crate) fn detect_resize_edge(
    x: f32,
    y: f32,
    window_width: f32,
    window_height: f32,
    inset: f32,
) -> Option<rinch_platform::ResizeDirection> {
    use rinch_platform::ResizeDirection::*;
    // The inset defines the full resize grab zone from the window edge.
    // No additional grab extension — keep resize handles within the inset
    // so they don't overlap content (e.g. scrollbars).
    let edge = inset;
    let corner = inset * 2.0;

    let near_left = x < edge;
    let near_right = x > window_width - edge;
    let near_top = y < edge;
    let near_bottom = y > window_height - edge;

    let corner_left = x < corner;
    let corner_right = x > window_width - corner;
    let corner_top = y < corner;
    let corner_bottom = y > window_height - corner;

    match (near_top, near_bottom, near_left, near_right) {
        (true, _, true, _) if corner_top || corner_left => Some(NorthWest),
        (true, _, _, true) if corner_top || corner_right => Some(NorthEast),
        (_, true, true, _) if corner_bottom || corner_left => Some(SouthWest),
        (_, true, _, true) if corner_bottom || corner_right => Some(SouthEast),
        (true, _, _, _) => Some(North),
        (_, true, _, _) => Some(South),
        (_, _, true, _) => Some(West),
        (_, _, _, true) => Some(East),
        _ => None,
    }
}

/// Map a resize direction to the appropriate cursor style.
pub(crate) fn resize_direction_to_cursor(
    dir: &rinch_platform::ResizeDirection,
) -> rinch_platform::CursorStyle {
    use rinch_platform::CursorStyle as CS;
    use rinch_platform::ResizeDirection as RD;
    match dir {
        RD::North => CS::NResize,
        RD::South => CS::SResize,
        RD::East => CS::EResize,
        RD::West => CS::WResize,
        RD::NorthEast => CS::NeResize,
        RD::NorthWest => CS::NwResize,
        RD::SouthEast => CS::SeResize,
        RD::SouthWest => CS::SwResize,
    }
}

/// Convert a CursorValue from computed style to a platform CursorStyle.
pub(super) fn cursor_value_to_style(
    cursor: &rinch_dom::computed_style::CursorValue,
) -> rinch_platform::CursorStyle {
    use rinch_dom::computed_style::CursorValue as CV;
    use rinch_platform::CursorStyle as CS;
    match cursor {
        CV::Auto => CS::Auto,
        CV::Default => CS::Default,
        CV::Pointer => CS::Pointer,
        CV::Text => CS::Text,
        CV::Move => CS::Move,
        CV::NotAllowed => CS::NotAllowed,
        CV::Crosshair => CS::Crosshair,
        CV::Grab => CS::Grab,
        CV::Grabbing => CS::Grabbing,
        CV::ColResize => CS::ColResize,
        CV::RowResize => CS::RowResize,
        CV::NResize => CS::NResize,
        CV::SResize => CS::SResize,
        CV::EResize => CS::EResize,
        CV::WResize => CS::WResize,
        CV::NeResize => CS::NeResize,
        CV::NwResize => CS::NwResize,
        CV::SeResize => CS::SeResize,
        CV::SwResize => CS::SwResize,
        CV::EwResize => CS::EwResize,
        CV::NsResize => CS::NsResize,
        CV::ZoomIn => CS::ZoomIn,
        CV::ZoomOut => CS::ZoomOut,
        CV::Wait => CS::Wait,
        CV::Progress => CS::Progress,
        CV::Help => CS::Help,
        CV::None => CS::None,
    }
}

/// Find the nearest ancestor (or self) that is a scroll container.
///
/// Only `overflow: scroll` and `overflow: auto` are considered scrollable.
/// `overflow: hidden` clips content but should NOT consume mousewheel events —
/// the wheel should bubble up to the nearest actual scroll container.
pub(crate) fn find_scroll_container(tree: &rinch_dom::NodeTree, start: usize) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        let overflow_y = &node.computed_style.overflow_y;
        match overflow_y {
            OverflowValue::Scroll | OverflowValue::Auto => {
                let content_h = compute_content_height(tree, node_id);
                if content_h > node.layout.height as f64 {
                    return Some(node_id);
                }
            }
            _ => {}
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    let body = tree.get(tree.body_id)?;
    let content_h = compute_content_height(tree, tree.body_id);
    if content_h > body.layout.height as f64 {
        return Some(tree.body_id);
    }
    None
}

/// Compute the total content height of a node from its children's layout bounds.
pub(crate) fn compute_content_height(tree: &rinch_dom::NodeTree, node_id: usize) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    // Taffy child.layout.y is relative to the parent's border box,
    // so it includes padding-top + border-top. Subtract that offset
    // to get the content-relative height (consistent with
    // compute_visible_content_area_height).
    let content_top = (node.computed_style.padding_top.to_px()
        + node.computed_style.border_top_width.to_px()) as f64;
    let mut max_bottom: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let bottom = (child.layout.y + child.layout.height) as f64 - content_top;
            if bottom > max_bottom {
                max_bottom = bottom;
            }
        }
    }
    max_bottom
}

/// The visible content area height: layout.height minus padding and border.
/// Children are positioned relative to the content box, so this is the actual
/// viewport height for scroll calculations.
pub(crate) fn compute_visible_content_area_height(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let cs = &node.computed_style;
    let pad_top = cs.padding_top.to_px() as f64;
    let pad_bottom = cs.padding_bottom.to_px() as f64;
    let border_top = cs.border_top_width.to_px() as f64;
    let border_bottom = cs.border_bottom_width.to_px() as f64;
    (node.layout.height as f64 - pad_top - pad_bottom - border_top - border_bottom).max(0.0)
}

/// Find the nearest ancestor (or self) that is a horizontal scroll container.
pub(crate) fn find_horizontal_scroll_container(
    tree: &rinch_dom::NodeTree,
    start: usize,
) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        let overflow_x = &node.computed_style.overflow_x;
        match overflow_x {
            OverflowValue::Scroll | OverflowValue::Auto => {
                let content_w = compute_content_width(tree, node_id);
                if content_w > node.layout.width as f64 {
                    return Some(node_id);
                }
            }
            _ => {}
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    let body = tree.get(tree.body_id)?;
    let content_w = compute_content_width(tree, tree.body_id);
    if content_w > body.layout.width as f64 {
        return Some(tree.body_id);
    }
    None
}

/// Find a vertical scroll container at (x, y) by geometric search.
///
/// When the hit-tested node lives in a different DOM branch from the scroll
/// container (e.g., an absolutely-positioned overlay sibling), the parent-chain
/// walk in `find_scroll_container` fails. This function walks the tree from the
/// body, looking for the deepest scroll container whose *painted* bounds
/// contain the point — the same descent `hit_test` makes, via [`descend`], so a
/// scroller under a CSS transform or inside a `position: fixed` overlay is
/// found where the wheel pointer actually is (#203). Picking the wrong
/// container here is silent: the wheel scrolls something else.
pub(crate) fn find_scroll_container_at_point(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<usize> {
    find_scroll_container_at_point_recursive(tree, tree.body_id, 0.0, 0.0, x, y, x, y)
}

#[allow(clippy::too_many_arguments)]
fn find_scroll_container_at_point_recursive(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let node = tree.get(node_id)?;
    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;
    let nw = node.layout.width;
    let nh = node.layout.height;

    let in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;
    if !in_bounds {
        return None;
    }

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    // Check children first (deepest match wins)
    for &child_id in node.children.iter().rev() {
        if let Some(found) =
            find_scroll_container_at_point_recursive(tree, child_id, nx - sx, ny - sy, x, y, vx, vy)
        {
            return Some(found);
        }
    }

    // Check this node
    let overflow_y = &node.computed_style.overflow_y;
    if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
        let content_h = compute_content_height(tree, node_id);
        if content_h > node.layout.height as f64 {
            return Some(node_id);
        }
    }

    None
}

/// Find a horizontal scroll container at (x, y) by geometric search.
///
/// The horizontal twin of [`find_scroll_container_at_point`], sharing its
/// [`descend`] geometry.
pub(crate) fn find_horizontal_scroll_container_at_point(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<usize> {
    find_hscroll_container_at_point_recursive(tree, tree.body_id, 0.0, 0.0, x, y, x, y)
}

#[allow(clippy::too_many_arguments)]
fn find_hscroll_container_at_point_recursive(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let node = tree.get(node_id)?;
    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;
    let nw = node.layout.width;
    let nh = node.layout.height;

    let in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;
    if !in_bounds {
        return None;
    }

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    for &child_id in node.children.iter().rev() {
        if let Some(found) = find_hscroll_container_at_point_recursive(
            tree,
            child_id,
            nx - sx,
            ny - sy,
            x,
            y,
            vx,
            vy,
        ) {
            return Some(found);
        }
    }

    let overflow_x = &node.computed_style.overflow_x;
    if matches!(overflow_x, OverflowValue::Scroll | OverflowValue::Auto) {
        let content_w = compute_content_width(tree, node_id);
        if content_w > node.layout.width as f64 {
            return Some(node_id);
        }
    }

    None
}

/// Compute the total content width of a node from its children's layout bounds.
pub(crate) fn compute_content_width(tree: &rinch_dom::NodeTree, node_id: usize) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    // Taffy child.layout.x is relative to the parent's border box, so it
    // includes padding-left + border-left. Subtract that offset to get the
    // content-relative width, the same way `compute_content_height` does and
    // the same way `DomDocument::scroll_width` already did — without this the
    // horizontal scrollbar would decide a padded container overflows when it
    // does not, and size its thumb against a width the paint pass disagrees
    // with.
    let content_left = (node.computed_style.padding_left.to_px()
        + node.computed_style.border_left_width.to_px()) as f64;
    let mut max_right: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let right = (child.layout.x + child.layout.width) as f64 - content_left;
            if right > max_right {
                max_right = right;
            }
        }
    }
    max_right
}

/// The visible content area width: layout.width minus padding and border.
///
/// The horizontal twin of [`compute_visible_content_area_height`].
pub(crate) fn compute_visible_content_area_width(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let cs = &node.computed_style;
    let pad_left = cs.padding_left.to_px() as f64;
    let pad_right = cs.padding_right.to_px() as f64;
    let border_left = cs.border_left_width.to_px() as f64;
    let border_right = cs.border_right_width.to_px() as f64;
    (node.layout.width as f64 - pad_left - pad_right - border_left - border_right).max(0.0)
}

/// The width of the invisible strip along a container's edge that counts as
/// its scrollbar for hit-testing — deliberately wider than the 6px thumb the
/// paint pass draws, so the bar is easy to grab. Both axes use it, so the
/// corner the two strips would share is a square of this side.
pub(crate) const SCROLLBAR_HIT_THICKNESS: f32 = 16.0;

/// A scrollbar the pointer is over.
///
/// # Coordinate space
///
/// The `x`/`y` fed to [`find_scrollbar_hit`] are the pointer coordinates the
/// shell hands `PlatformEvent`, compared here against **logical** layout rects
/// straight out of Taffy — no scale factor is applied anywhere in this
/// function. That is the same space the vertical bar has always been tested
/// in, and it is exactly the mismatch #299 describes (the desktop shell passes
/// winit's *physical* coordinates), so at a scale factor other than 1 both
/// bars are displaced by the same amount and #299 fixes both at once. The
/// paint pass is unaffected: it multiplies by `scale` and works in device
/// pixels.
pub(crate) struct ScrollbarHit {
    /// The scroll container whose bar was hit.
    pub node_id: usize,
    /// Which of its two bars.
    pub axis: ScrollAxis,
    /// Content extent along `axis`.
    pub content_size: f64,
    /// Visible content-area extent along `axis`.
    pub container_size: f64,
}

/// Check if a point (x, y) hits a scrollbar.
///
/// The strips are tested in the container's **own** space, which is where the
/// paint pass draws the thumbs — it composes `node_transform` before stroking
/// them — so a scroller inside a `transform: scale()` or `translate()` is
/// grabbable at the bar you can see rather than at the untransformed layout
/// rect (#203). The caller wanting the pointer's position *along* the track
/// (jump-to-click, drag) asks [`rinch_dom::paint::point_in_painted_box`] for
/// the same mapping.
pub(crate) fn find_scrollbar_hit(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<ScrollbarHit> {
    find_scrollbar_hit_node(tree, tree.body_id, 0.0, 0.0, x, y, x, y)
}

#[allow(clippy::too_many_arguments)]
fn find_scrollbar_hit_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<ScrollbarHit> {
    let node = tree.get(node_id)?;

    // Skip hidden subtrees
    if matches!(
        node.computed_style.visibility,
        rinch_dom::computed_style::VisibilityValue::Hidden
            | rinch_dom::computed_style::VisibilityValue::Collapse
    ) {
        return None;
    }

    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;
    let nw = node.layout.width;
    let nh = node.layout.height;

    let point_in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;
    if !point_in_bounds {
        return None;
    }

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = find_scrollbar_hit_node(tree, child_id, nx - sx, ny - sy, x, y, vx, vy) {
            return Some(hit);
        }
    }

    use rinch_dom::computed_style::OverflowValue;
    let cs = &node.computed_style;

    // A bar exists on an axis when that axis is scrollable AND overflowing.
    // `scroll` and `auto` behave identically here, matching what the vertical
    // bar has always done: rinch paints a thumb and no track, so there is
    // nothing for `scroll` to show when the content fits.
    // The extents are only measured for an axis that is scrollable at all, so
    // an ordinary node in the recursion pays nothing beyond the enum check.
    let vertical = matches!(cs.overflow_y, OverflowValue::Scroll | OverflowValue::Auto)
        .then(|| {
            (
                compute_content_height(tree, node_id),
                compute_visible_content_area_height(tree, node_id),
            )
        })
        .filter(|(content, visible)| content > visible);
    let horizontal = matches!(cs.overflow_x, OverflowValue::Scroll | OverflowValue::Auto)
        .then(|| {
            (
                compute_content_width(tree, node_id),
                compute_visible_content_area_width(tree, node_id),
            )
        })
        .filter(|(content, visible)| content > visible);

    // The corner. Where both bars are present their strips would overlap in a
    // square at the far end, and one of them would silently win the click.
    // Neither claims it: each strip stops short by the other's thickness, so
    // the corner falls through to ordinary click handling on the container.
    // The paint pass shortens both tracks the same way, so no thumb is ever
    // drawn in a square that cannot be grabbed.
    let t = SCROLLBAR_HIT_THICKNESS;
    let v_end = if horizontal.is_some() {
        ny + nh - t
    } else {
        ny + nh
    };
    let h_end = if vertical.is_some() {
        nx + nw - t
    } else {
        nx + nw
    };

    if let Some((content_size, container_size)) = vertical {
        let scrollbar_left = nx + nw - t;
        if x >= scrollbar_left && x <= nx + nw && y >= ny && y <= v_end {
            return Some(ScrollbarHit {
                node_id,
                axis: ScrollAxis::Vertical,
                content_size,
                container_size,
            });
        }
    }

    if let Some((content_size, container_size)) = horizontal {
        let scrollbar_top = ny + nh - t;
        if y >= scrollbar_top && y <= ny + nh && x >= nx && x <= h_end {
            return Some(ScrollbarHit {
                node_id,
                axis: ScrollAxis::Horizontal,
                content_size,
                container_size,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::hit_test;
    use rinch_core::dom::{DomDocument, NodeId};
    use rinch_dom::RinchDocument;

    /// Absolute (border-box) origin of a node as a **plain parent-chain sum**:
    /// layout offsets only, no transform composed, no `position: fixed`
    /// exception, no IFC content-box offset.
    ///
    /// This is a deliberate independent oracle, not a helper — it is the walk
    /// the production code used to run, kept so the tests below can state the
    /// box a transform-blind walk would have produced and assert it is *not*
    /// the answer. Do not "fix" it to call `painted_element_box`: it exists
    /// precisely to disagree with it. (`paint::tests::untransformed_origin`
    /// serves the same purpose in rinch-dom.)
    fn abs_origin(doc: &RinchDocument, node_id: usize) -> (f32, f32) {
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let body = doc.body().0;
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            let n = doc.tree.get(id).unwrap();
            x += n.layout.x;
            y += n.layout.y;
            if id == body {
                break;
            }
            cur = n.parent;
        }
        (x, y)
    }

    /// Regression (found while fixing #61): an inline-block button laid out in a
    /// text flow (an IFC) inside a padded block container must be hit-testable
    /// across its whole *painted* box. Two pre-existing bugs conspired to swallow
    /// its clicks: (A) the surrounding IFC text nodes have their layout stretched
    /// to the entire container and shadowed the button, and (B) hit-testing used
    /// the container's border-box origin while the button is painted at the
    /// content-box origin, so the hit rect was offset up-left by the padding.
    #[test]
    fn inline_block_in_ifc_is_hittable_over_its_painted_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        // Padding is the crux of bug (B).
        doc.set_attribute(
            container,
            "style",
            "width: 400px; padding: 20px; font-size: 16px",
        );
        doc.append_child(body, container);

        // "Click the " <button>OK</button> " now" — text on BOTH sides so a
        // full-container text node exists after the button in DOM order (bug A).
        let before = doc.create_text("Click the ");
        doc.append_child(container, before);
        let button = doc.create_element("button"); // inline-block by default
        doc.set_attribute(button, "style", "width: 60px; height: 24px");
        doc.append_child(container, button);
        let label = doc.create_text("OK");
        doc.append_child(button, label);
        let after = doc.create_text(" now");
        doc.append_child(container, after);

        doc.resolve_layout(800.0, 600.0);

        // The container establishes the IFC; the button is detached from Taffy
        // and positioned by Parley relative to the *content* box.
        assert!(
            doc.tree.get(container.0).unwrap().text_layout.is_some(),
            "container should be the IFC root"
        );
        let bl = doc.tree.get(button.0).unwrap().layout;
        let (abs_x, abs_y) = abs_origin(&doc, button.0);
        // Painted box origin = border-box origin + container content-box offset.
        let paint_x = abs_x + 20.0;
        let paint_y = abs_y + 20.0;

        // Center of the painted button hits the button (not a text sibling).
        assert_eq!(
            hit_test(
                &doc.tree,
                paint_x + bl.width / 2.0,
                paint_y + bl.height / 2.0
            ),
            Some(button.0),
            "painted center of the inline-block button must hit the button"
        );
        // The bottom edge — the part bug (B) shifted out of the hit rect — hits too.
        assert_eq!(
            hit_test(
                &doc.tree,
                paint_x + bl.width / 2.0,
                paint_y + bl.height - 1.0
            ),
            Some(button.0),
            "bottom edge of the painted button must still hit the button"
        );
        // A point on the leading text well left of the button must NOT hit it.
        assert_ne!(
            hit_test(&doc.tree, paint_x - bl.width, paint_y + bl.height / 2.0),
            Some(button.0),
            "a point on the leading text must not resolve to the button"
        );
    }

    // ── CSS transforms (#199) ────────────────────────────────────────────
    //
    // The correctness criterion for every test below: a point hits a node iff
    // that point lands inside where the node is *painted*. Paint draws each box
    // under `compose_node_transform` (rinch-dom/src/paint/mod.rs), so the
    // expected rects here are hand-computed from the CSS transform, not from
    // any hit-test helper.

    /// Build a document with a `400x300` relative container as `body`'s only
    /// child, returning `(doc, container)`. The container sits at the viewport
    /// origin so absolute child offsets *are* viewport coordinates.
    fn container_doc() -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(
            container,
            "style",
            "position: relative; width: 400px; height: 300px",
        );
        doc.append_child(body, container);
        (doc, container)
    }

    fn child_of(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
        let el = doc.create_element("div");
        doc.set_attribute(el, "style", style);
        doc.append_child(parent, el);
        el
    }

    /// The motivating bug: the `left:50%; top:50%; translate(-50%,-50%)` modal
    /// idiom rendered centered but hit-tested at its *un*translated layout box,
    /// so every control inside it was unclickable.
    #[test]
    fn translate_centered_modal_is_hit_at_its_visual_position() {
        let (mut doc, container) = container_doc();
        // Layout box (200,150)-(300,210); painted box shifted by (-50,-30) to
        // (150,120)-(250,180), i.e. centered on the container.
        let modal = child_of(
            &mut doc,
            container,
            "position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); \
             width: 100px; height: 60px",
        );
        // In-flow child at the modal's origin: layout (200,150)-(240,170),
        // painted (150,120)-(190,140).
        let button = child_of(&mut doc, modal, "width: 40px; height: 20px");
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 200.0, 150.0),
            Some(modal.0),
            "the modal's visual centre (= the container's centre) must hit the modal"
        );
        assert_eq!(
            hit_test(&doc.tree, 170.0, 130.0),
            Some(button.0),
            "the button's painted centre must hit the button"
        );
        assert_eq!(
            hit_test(&doc.tree, 275.0, 195.0),
            Some(container.0),
            "a point in the modal's untransformed layout box must miss the modal"
        );
        assert_eq!(
            hit_test(&doc.tree, 220.0, 160.0),
            Some(modal.0),
            "the button's untransformed layout centre must miss the button"
        );
    }

    /// A percentage translate that is *not* the first function in the list
    /// composes in the frame the functions before it establish (#212) — and
    /// paint and hit testing have to say so together.
    ///
    /// `scale(2) translateX(50%)` on a 100px-wide box moves it 100px, not 50:
    /// the offset is scaled by the `scale` that precedes it, exactly as a
    /// `translateX(50px)` in the same position would be. The whole point of
    /// #203 is that the box a click reports is the box paint drew, so this
    /// asserts both ends against one hand-computed rect.
    #[test]
    fn a_scaled_percentage_translate_paints_and_hits_in_the_same_place() {
        let (mut doc, container) = container_doc();
        // Layout box (100,60)-(200,100), origin at its centre (150,80).
        // m = [2,0,0,2, 100, 0]  (the 50% of 100px doubled by the scale), so
        // the composed map is p -> (2x - 50, 2y - 80) and the painted box is
        // (150,40)-(350,120). Before the fix m[4] was 50, putting it at
        // (100,40)-(300,120).
        let boxed = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 60px; width: 100px; height: 40px; \
             transform: scale(2) translateX(50%)",
        );
        doc.resolve_layout(800.0, 600.0);

        let (px, py, pw, ph) = super::painted_element_box(&doc.tree, boxed.0);
        for (got, want, what) in [
            (px, 150.0, "x"),
            (py, 40.0, "y"),
            (pw, 200.0, "width"),
            (ph, 80.0, "height"),
        ] {
            assert!(
                (got - want).abs() < 0.01,
                "painted {what}: got {got}, expected {want}"
            );
        }

        assert_eq!(
            hit_test(&doc.tree, 340.0, 80.0),
            Some(boxed.0),
            "a point inside the painted box's right end must hit it"
        );
        assert_eq!(
            hit_test(&doc.tree, 110.0, 80.0),
            Some(container.0),
            "a point where the un-scaled offset used to put the box must miss it"
        );
    }

    /// The same reconvergence with a rotation, where the offset changes
    /// direction rather than magnitude.
    ///
    /// `rotate(45deg) translateX(50%)` sends the box's centre 50px along the
    /// rotated x-axis, i.e. to `(150, 80) + (35.36, 35.36)`. Before the fix the
    /// offset was applied in the outer frame, landing the centre at `(200, 80)`
    /// — far enough away that each position misses the other's quad.
    #[test]
    fn a_rotated_percentage_translate_paints_and_hits_in_the_same_place() {
        let (mut doc, container) = container_doc();
        let boxed = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 60px; width: 100px; height: 40px; \
             transform: rotate(45deg) translateX(50%)",
        );
        doc.resolve_layout(800.0, 600.0);

        // The bounding box of an affine image of a rectangle is centred on the
        // image of the rectangle's centre, so this reads paint's answer for
        // where the box went without hand-computing the rotated corners.
        let (px, py, pw, ph) = super::painted_element_box(&doc.tree, boxed.0);
        let (cx, cy) = (px + pw / 2.0, py + ph / 2.0);
        let offset = 50.0 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (cx - (150.0 + offset)).abs() < 0.01 && (cy - (80.0 + offset)).abs() < 0.01,
            "painted centre: got ({cx}, {cy}), expected ({}, {})",
            150.0 + offset,
            80.0 + offset
        );

        assert_eq!(
            hit_test(&doc.tree, cx, cy),
            Some(boxed.0),
            "the painted centre must hit the box"
        );
        assert_eq!(
            hit_test(&doc.tree, 200.0, 80.0),
            Some(container.0),
            "where the un-rotated offset used to put the box must now miss it"
        );
    }

    /// Nested transforms compose: the child is hit where the parent's translate
    /// and its own scale put it, not at either one alone.
    #[test]
    fn nested_transforms_compose() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = doc.create_element("div");
        doc.set_attribute(
            outer,
            "style",
            "position: relative; width: 400px; height: 300px; transform: translate(100px, 50px)",
        );
        doc.append_child(body, outer);
        // Layout (0,0)-(100,50) inside `outer`; scale(2) about its centre
        // (50,25) → (-50,-25)-(150,75); + outer's translate → (50,25)-(250,125).
        let inner = child_of(
            &mut doc,
            outer,
            "position: absolute; left: 0; top: 0; width: 100px; height: 50px; transform: scale(2)",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 60.0, 30.0),
            Some(inner.0),
            "top-left of the composed quad must hit the inner box"
        );
        assert_eq!(
            hit_test(&doc.tree, 240.0, 120.0),
            Some(inner.0),
            "bottom-right of the composed quad must hit the inner box"
        );
        assert_eq!(
            hit_test(&doc.tree, 260.0, 130.0),
            Some(outer.0),
            "just outside the composed quad falls through to the translated parent"
        );
        assert_eq!(
            hit_test(&doc.tree, 20.0, 20.0),
            Some(body.0),
            "the parent's own layout box (before its translate) must not be hit"
        );
    }

    /// A scaled box is hit over its scaled bounds, about its transform-origin.
    #[test]
    fn scaled_node_is_hit_over_its_scaled_bounds() {
        let (mut doc, container) = container_doc();
        // Default origin 50% 50%: (100,100)-(150,150) scaled 2x about (125,125)
        // → (75,75)-(175,175).
        let centred = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 100px; width: 50px; height: 50px; \
             transform: scale(2)",
        );
        // origin top left: (300,100)-(350,150) → (300,100)-(400,200).
        let corner = child_of(
            &mut doc,
            container,
            "position: absolute; left: 300px; top: 100px; width: 50px; height: 50px; \
             transform: scale(2); transform-origin: top left",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 80.0, 80.0),
            Some(centred.0),
            "inside the scaled bounds but outside the layout box must hit"
        );
        assert_eq!(hit_test(&doc.tree, 125.0, 125.0), Some(centred.0));
        assert_eq!(
            hit_test(&doc.tree, 180.0, 180.0),
            Some(container.0),
            "outside the scaled bounds must miss"
        );
        assert_eq!(
            hit_test(&doc.tree, 390.0, 190.0),
            Some(corner.0),
            "a top-left-origin scale grows down/right only"
        );
        assert_eq!(
            hit_test(&doc.tree, 285.0, 85.0),
            Some(container.0),
            "a top-left-origin scale must not grow up/left (that would be a 50% origin)"
        );
    }

    /// Rotation is supported by the paint transform (`GenericTransformOperation::Rotate`),
    /// so hit testing must follow the rotated quad.
    #[test]
    fn rotated_node_is_hit_inside_its_rotated_quad() {
        let (mut doc, container) = container_doc();
        // (100,140)-(300,160) rotated 90° about its centre (200,150)
        // → a 20x200 quad, (190,50)-(210,250).
        let bar = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 140px; width: 200px; height: 20px; \
             transform: rotate(90deg)",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 200.0, 150.0),
            Some(bar.0),
            "the centre is fixed by the rotation"
        );
        assert_eq!(
            hit_test(&doc.tree, 200.0, 60.0),
            Some(bar.0),
            "the top of the rotated quad must hit"
        );
        assert_eq!(
            hit_test(&doc.tree, 200.0, 240.0),
            Some(bar.0),
            "the bottom of the rotated quad must hit"
        );
        assert_eq!(
            hit_test(&doc.tree, 110.0, 150.0),
            Some(container.0),
            "the unrotated box's left end is outside the rotated quad"
        );
    }

    /// Stacking-context order is unchanged by the transform mapping: a
    /// transformed SC is tested at its visual position but in its z-index slot,
    /// and an untransformed higher-z sibling still wins.
    #[test]
    fn transformed_sc_keeps_z_index_order() {
        let (mut doc, container) = container_doc();
        let back = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 0; width: 200px; height: 200px; z-index: 1",
        );
        // Layout (300,50)-(400,150); painted (200,50)-(300,150).
        let mid = child_of(
            &mut doc,
            container,
            "position: absolute; left: 300px; top: 50px; width: 100px; height: 100px; \
             z-index: 2; transform: translate(-100px, 0)",
        );
        let front = child_of(
            &mut doc,
            container,
            "position: absolute; left: 250px; top: 100px; width: 100px; height: 100px; z-index: 3",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 210.0, 60.0),
            Some(mid.0),
            "the transformed SC beats the lower-z sibling it visually covers"
        );
        assert_eq!(
            hit_test(&doc.tree, 260.0, 110.0),
            Some(front.0),
            "an untransformed higher-z sibling still wins over the transformed SC"
        );
        assert_eq!(
            hit_test(&doc.tree, 390.0, 60.0),
            Some(container.0),
            "the transformed SC is not hit at its untransformed layout box"
        );
        assert_eq!(hit_test(&doc.tree, 110.0, 190.0), Some(back.0));
    }

    /// A non-invertible transform (`scale(0)`) paints nothing, so it and its
    /// subtree are unhittable — and the traversal must survive it (no panic, no
    /// NaN leaking into the sibling that is tested next).
    #[test]
    fn scale_zero_subtree_is_unhittable() {
        let (mut doc, container) = container_doc();
        let collapsed = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 100px; width: 100px; height: 100px; \
             transform: scale(0)",
        );
        let inside = child_of(&mut doc, collapsed, "width: 50px; height: 50px");
        let sibling = child_of(
            &mut doc,
            container,
            "position: absolute; left: 250px; top: 100px; width: 50px; height: 50px",
        );
        doc.resolve_layout(800.0, 600.0);

        for (x, y) in [(150.0, 150.0), (100.0, 100.0), (105.0, 105.0)] {
            let hit = hit_test(&doc.tree, x, y);
            assert_eq!(
                hit,
                Some(container.0),
                "({x}, {y}) must fall through the scale(0) subtree to the container"
            );
            assert_ne!(hit, Some(collapsed.0));
            assert_ne!(hit, Some(inside.0));
        }
        assert_eq!(
            hit_test(&doc.tree, 275.0, 125.0),
            Some(sibling.0),
            "the sibling tested after the degenerate subtree still hits"
        );
    }

    /// `position: fixed` inside a transformed ancestor. Per CSS a transform
    /// makes a containing block for fixed descendants, but rinch's paint path
    /// hoists fixed SCs to the body level with zeroed offsets and the *body's*
    /// transform (`collect_stacking_contexts_root`), so they render at their
    /// viewport box with no ancestor transform. Hit testing mirrors paint —
    /// consistency beats spec here, and diverging is the bug we are fixing.
    #[test]
    fn fixed_in_transformed_ancestor_is_hit_where_paint_puts_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = doc.create_element("div");
        doc.set_attribute(
            outer,
            "style",
            "position: relative; width: 400px; height: 300px; transform: translate(200px, 100px)",
        );
        doc.append_child(body, outer);
        let fixed = child_of(
            &mut doc,
            outer,
            "position: fixed; left: 50px; top: 60px; width: 100px; height: 40px; z-index: 5",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 60.0, 70.0),
            Some(fixed.0),
            "the fixed box is hit at its viewport position, as paint draws it"
        );
        assert_ne!(
            hit_test(&doc.tree, 260.0, 170.0),
            Some(fixed.0),
            "the ancestor's transform must not displace the fixed box's hit area"
        );
    }

    /// Pins the viewport-point re-seed at the body level. A transform on `body`
    /// itself is the only thing that makes it do work: paint paints the hoisted
    /// fixed entries with *body's* `node_transform`, so viewport space for a
    /// fixed box is body's post-transform space, not the raw input point.
    #[test]
    fn body_transform_applies_to_a_hoisted_fixed_descendant() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "transform: translate(40px, 30px)");
        let outer = child_of(
            &mut doc,
            body,
            "position: relative; width: 400px; height: 300px; transform: translate(200px, 100px)",
        );
        // Viewport box (50,60)-(150,100); painted at body's translate →
        // (90,90)-(190,130). `outer`'s translate must not reach it.
        let fixed = child_of(
            &mut doc,
            outer,
            "position: fixed; left: 50px; top: 60px; width: 100px; height: 40px; z-index: 5",
        );
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 140.0, 110.0),
            Some(fixed.0),
            "the fixed box is hit at its viewport box shifted by body's transform"
        );
        assert_ne!(
            hit_test(&doc.tree, 100.0, 80.0),
            Some(fixed.0),
            "the un-shifted viewport box must not hit — body's transform does apply"
        );
        assert_ne!(
            hit_test(&doc.tree, 290.0, 190.0),
            Some(fixed.0),
            "the transformed ancestor's own translate must still not reach the fixed box"
        );
    }

    /// Pins the `display: contents` half of the zero-size guard in `local_point`.
    /// A box collapsed on one axis keeps its transform box — `paint_node`'s
    /// `(w == 0) != (h == 0)` branch composes the transform and paints the
    /// overflowing children under it — so the guard must *not* fire here.
    #[test]
    fn one_axis_zero_size_node_still_transforms_its_children() {
        let (mut doc, container) = container_doc();
        let zerobox = child_of(
            &mut doc,
            container,
            "position: absolute; left: 100px; top: 100px; width: 200px; height: 0; \
             transform: translate(50px, 20px)",
        );
        // Overflows the zero-height parent: layout (100,100)-(140,130),
        // painted (150,120)-(190,150).
        let child = child_of(
            &mut doc,
            zerobox,
            "position: absolute; left: 0; top: 0; width: 40px; height: 30px",
        );
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(
            doc.tree.get(zerobox.0).unwrap().layout.height,
            0.0,
            "the test needs the one-axis-collapsed paint branch"
        );

        assert_eq!(
            hit_test(&doc.tree, 170.0, 135.0),
            Some(child.0),
            "the child is hit under the collapsed parent's transform"
        );
        assert_eq!(
            hit_test(&doc.tree, 120.0, 115.0),
            Some(container.0),
            "the child's untransformed layout box must miss"
        );
    }

    /// Pins the zero-size half of the same guard. A `display: contents` node has
    /// no box, so `paint_node` passes the parent transform straight through to
    /// its children and the node's own `transform` never renders — hit testing
    /// must ignore it too.
    #[test]
    fn display_contents_node_ignores_its_own_transform() {
        let (mut doc, container) = container_doc();
        let contents = child_of(
            &mut doc,
            container,
            "display: contents; transform: translate(60px, 40px)",
        );
        let child = child_of(&mut doc, contents, "width: 40px; height: 30px");
        doc.resolve_layout(800.0, 600.0);
        let cn = doc.tree.get(contents.0).unwrap();
        assert_eq!(
            (cn.layout.width, cn.layout.height),
            (0.0, 0.0),
            "the test needs display:contents to have no box"
        );
        assert!(
            !cn.computed_style.transform.is_identity,
            "the test needs a real transform declared on the display:contents node"
        );

        assert_eq!(
            hit_test(&doc.tree, 20.0, 15.0),
            Some(child.0),
            "the child is hit at its laid-out box, untouched by the contents node's transform"
        );
        assert_eq!(
            hit_test(&doc.tree, 80.0, 55.0),
            Some(container.0),
            "where that transform would have put the child must miss"
        );
    }
    // ── CSS 2.1 Appendix E step 8: positioned descendants with `z-index: auto`
    //
    // These drive the real `hit_test`, which reads
    // `rinch_dom::stacking::stacking_paint_order` backwards. The ordering itself
    // is pinned in `rinch-dom/tests/stacking_tests.rs`, together with the pixels
    // the same sequence produces read forwards; what is pinned here is that the
    // taps land where those pixels are.

    /// The bug: a floating action button over a scrolling list was untappable.
    /// The scroller is a stacking context (Rinch makes one for `overflow`) and
    /// the FAB is `position: absolute` with no `z-index`, so hit testing walked
    /// the z == 0 stacking contexts — the scroller — before the plain children,
    /// and every tap fell through to the row underneath.
    #[test]
    fn a_tap_on_a_positioned_z_auto_box_over_a_scroller_hits_the_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let scroller = doc.create_element("div");
        doc.set_attribute(
            scroller,
            "style",
            "overflow: auto; width: 200px; height: 200px",
        );
        doc.append_child(body, scroller);
        let row = child_of(&mut doc, scroller, "width: 200px; height: 400px");

        let fab = doc.create_element("div");
        doc.set_attribute(
            fab,
            "style",
            "position: absolute; left: 150px; top: 150px; width: 56px; height: 56px",
        );
        doc.append_child(body, fab);

        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 170.0, 170.0),
            Some(fab.0),
            "the FAB paints over the scroller there, so the tap is the FAB's"
        );
        assert_eq!(
            hit_test(&doc.tree, 40.0, 40.0),
            Some(row.0),
            "and clear of the FAB the list still takes its own taps"
        );
    }

    /// The same rule against in-flow content rather than a stacking context: a
    /// positioned box is step 8, an in-flow block is step 4, so the box takes
    /// the tap even where the block is written after it.
    #[test]
    fn a_tap_on_a_positioned_z_auto_box_over_later_in_flow_content_hits_the_box() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let fab = doc.create_element("div");
        doc.set_attribute(
            fab,
            "style",
            "position: absolute; left: 20px; top: 20px; width: 40px; height: 40px",
        );
        doc.append_child(body, fab);
        let block = child_of(&mut doc, body, "width: 200px; height: 100px");

        doc.resolve_layout(800.0, 600.0);

        assert_eq!(hit_test(&doc.tree, 30.0, 30.0), Some(fab.0));
        assert_eq!(hit_test(&doc.tree, 120.0, 50.0), Some(block.0));
    }

    /// A stacking context nested under a positioned `z-index: auto` box is the
    /// *ancestor's*, so its `z-index` still outranks a later sibling of the box
    /// it is written in — and is tapped accordingly.
    #[test]
    fn a_z_indexed_box_under_a_positioned_z_auto_box_is_still_ordered_by_its_z() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "position: relative");

        let panel = doc.create_element("div");
        doc.set_attribute(
            panel,
            "style",
            "position: absolute; left: 0; top: 0; width: 100px; height: 100px",
        );
        doc.append_child(body, panel);
        let badge = child_of(
            &mut doc,
            panel,
            "position: absolute; z-index: 3; left: 0; top: 0; width: 40px; height: 40px",
        );

        let cover = doc.create_element("div");
        doc.set_attribute(
            cover,
            "style",
            "position: absolute; left: 0; top: 0; width: 100px; height: 100px",
        );
        doc.append_child(body, cover);

        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 20.0, 20.0),
            Some(badge.0),
            "z-index: 3 beats a z == 0 sibling written later, wherever it is nested"
        );
        assert_eq!(
            hit_test(&doc.tree, 80.0, 80.0),
            Some(cover.0),
            "outside the badge, the later z == 0 box wins on tree order"
        );
    }

    /// #204: an absolute box with no positioned ancestor resolves against the
    /// initial containing block. Layout keeps `LayoutResult` parent-relative and
    /// writes the delta, so hit testing needs no rule of its own — this pins
    /// that it agrees with paint anyway, which is the whole point of the delta.
    #[test]
    fn icb_absolute_is_hit_over_the_whole_viewport() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // Unpositioned, small, and pushed away from the origin: before the fix
        // the overlay's hit area was this box.
        let host = child_of(
            &mut doc,
            body,
            "width: 100px; height: 50px; margin-left: 200px; margin-top: 100px",
        );
        let overlay = child_of(&mut doc, host, "position: absolute; inset: 0");
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            hit_test(&doc.tree, 10.0, 10.0),
            Some(overlay.0),
            "the overlay covers the viewport, so a click near the origin lands on it"
        );
        assert_eq!(
            hit_test(&doc.tree, 780.0, 580.0),
            Some(overlay.0),
            "and so does one near the far corner"
        );
    }

    // ── The auxiliary walks (#203) ───────────────────────────────────────
    //
    // Same criterion as the transform block above, applied to the three walks
    // that search the tree for something other than "which node was clicked":
    // the wheel's scroll container on each axis, and the scrollbar strips. Each
    // of them ran its own copy of the offset arithmetic with no transform and —
    // for the two scroll searches — no `position: fixed` exception, so they
    // could answer with a container the pointer is nowhere near. That failure is
    // silent: the wheel scrolls the wrong thing, or the bar cannot be grabbed.
    //
    // Expected rects hand-computed from the CSS; each test also states where the
    // untransformed walk would have looked and asserts that is *not* the answer.

    /// A scroller with `overflow: auto` and content taller than it, so both
    /// scroll searches and the scrollbar search have something to find.
    fn scroller(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
        let el = child_of(doc, parent, style);
        child_of(doc, el, "width: 100%; height: 2000px");
        el
    }

    /// The thumb is stroked under the container's own `node_transform`
    /// (`paint_scrollbars`), so the grab strip is the one on screen. Under
    /// `scale(2)` that is twice as far right and twice as tall as the layout
    /// rect's — the case an origin-only fix would still get wrong, because the
    /// strip's *position within* the container has to scale too.
    #[test]
    fn scrollbar_is_grabbed_where_its_thumb_paints() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let sc = scroller(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 200px; height: 100px; \
             overflow: auto; transform: scale(2); transform-origin: 0 0",
        );
        doc.resolve_layout(800.0, 600.0);

        // Layout box (0,0)-(200,100); the 16px vertical strip is x∈[184,200].
        // Doubled about the origin: painted box (0,0)-(400,200), strip
        // x∈[368,400], y∈[0,200].
        let hit = super::find_scrollbar_hit(&doc.tree, 390.0, 100.0)
            .expect("the painted strip is grabbable");
        assert_eq!(hit.node_id, sc.0);
        assert!(matches!(hit.axis, super::ScrollAxis::Vertical));

        // The untransformed strip: inside the painted container, but 200px left
        // of the thumb anyone can see.
        assert!(
            super::find_scrollbar_hit(&doc.tree, 192.0, 50.0).is_none(),
            "the layout-box strip is not where the thumb is drawn"
        );
        // And just left of the painted strip is content, not the bar.
        assert!(
            super::find_scrollbar_hit(&doc.tree, 360.0, 100.0).is_none(),
            "the strip is 16 container-pixels wide, i.e. 32 painted ones"
        );
    }

    /// The silent-wrong-target failure. Two same-sized scrollers, one moved by a
    /// `translate`: a point over the *untranslated* one used to be claimed by
    /// the translated one, because it was tested first (reverse tree order) and
    /// its layout box still sat there. The wheel then scrolled a container the
    /// pointer was 400px away from, with nothing to show for it.
    #[test]
    fn wheel_over_a_translated_scroller_scrolls_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        // `still` first in DOM order, `moved` second — so `moved` is tested
        // first, and a transform-blind walk hands it every point in (0,0)-(200,200).
        let still = scroller(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 200px; height: 200px; overflow: auto",
        );
        let moved = scroller(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 200px; height: 200px; \
             overflow: auto; transform: translate(400px, 0)",
        );
        doc.resolve_layout(800.0, 600.0);

        // Over the translated scroller's painted box (400,0)-(600,200).
        assert_eq!(
            super::find_scroll_container_at_point(&doc.tree, 450.0, 100.0),
            Some(moved.0),
            "a wheel over the painted box scrolls the box it is over"
        );
        // Over the still one. The translated scroller's *layout* box also covers
        // this point, and it is tested first — this is the assertion that fails
        // without the transform.
        assert_eq!(
            super::find_scroll_container_at_point(&doc.tree, 100.0, 100.0),
            Some(still.0),
            "the translated scroller's layout box must not claim this point"
        );
    }

    /// The horizontal twin, one assertion deep — the two functions share
    /// `descend`, so the point is that the horizontal search is wired to it too.
    #[test]
    fn horizontal_wheel_over_a_translated_scroller_scrolls_it() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let el = child_of(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 200px; height: 200px; \
             overflow-x: auto; transform: translate(400px, 0)",
        );
        child_of(&mut doc, el, "width: 2000px; height: 50px");
        doc.resolve_layout(800.0, 600.0);

        assert_eq!(
            super::find_horizontal_scroll_container_at_point(&doc.tree, 450.0, 100.0),
            Some(el.0),
            "found at the painted box"
        );
        assert_eq!(
            super::find_horizontal_scroll_container_at_point(&doc.tree, 100.0, 100.0),
            None,
            "and not at the untransformed one"
        );
    }

    /// The `Fixed` exception the two scroll searches never had — wrong today
    /// with no transform anywhere in sight. A fixed overlay inside a scrolled
    /// container had the container's scroll subtracted from its rect, so the
    /// further the page scrolled the further the overlay's own scroller drifted
    /// from where it is painted, and the wheel fell through to the page behind.
    #[test]
    fn a_fixed_scroller_is_found_after_its_container_scrolls() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let page = scroller(
            &mut doc,
            body,
            "position: absolute; left: 0; top: 0; width: 400px; height: 300px; overflow: auto",
        );
        let overlay = scroller(
            &mut doc,
            page,
            "position: fixed; left: 20px; top: 20px; width: 200px; height: 100px; overflow: auto",
        );
        doc.resolve_layout(800.0, 600.0);
        doc.tree.nodes[page.0].scroll_offset.1 = 150.0;

        // The overlay's viewport box is (20,20)-(220,120) whatever `page` does.
        assert_eq!(
            super::find_scroll_container_at_point(&doc.tree, 100.0, 60.0),
            Some(overlay.0),
            "a fixed overlay stays put while the page under it scrolls"
        );
        // Where the un-excepted walk put it: 150px up, off the top of itself.
        assert_eq!(
            super::find_scroll_container_at_point(&doc.tree, 100.0, 250.0),
            Some(page.0),
            "and the point 150px lower belongs to the page, not to the overlay"
        );
    }

    /// The scrollbar search kept its `Fixed` exception but composed no
    /// transform, so a fixed toolbar's own scroller was grabbable only if the
    /// body had no transform of its own — the same body-transform re-seed
    /// `body_transform_applies_to_a_hoisted_fixed_descendant` pins for hits.
    #[test]
    fn a_fixed_scrollers_bar_follows_the_body_transform() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        doc.set_attribute(body, "style", "transform: translate(40px, 30px)");
        let overlay = scroller(
            &mut doc,
            body,
            "position: fixed; left: 0; top: 0; width: 200px; height: 100px; \
             overflow: auto; z-index: 5",
        );
        doc.resolve_layout(800.0, 600.0);

        // Viewport box (0,0)-(200,100) shifted by the body's translate:
        // (40,30)-(240,130); the vertical strip is x∈[224,240].
        let hit = super::find_scrollbar_hit(&doc.tree, 232.0, 80.0)
            .expect("the bar is where the body's transform puts it");
        assert_eq!(hit.node_id, overlay.0);
        assert!(
            super::find_scrollbar_hit(&doc.tree, 192.0, 50.0).is_none(),
            "and not at the un-shifted viewport box"
        );
    }
}
