// ── Free functions (platform-agnostic hit testing) ───────────────────────────

/// Simple hit testing: find the deepest node whose layout rect contains (x, y).
/// Respects CSS stacking contexts so that elements with higher z-index
/// are tested before visually-behind siblings.
pub(crate) fn hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<usize> {
    hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y, true)
}

/// A stacking context entry for hit testing, with accumulated offset.
struct HitTestScEntry {
    z_index: i32,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    dom_order: usize,
}

/// Collect descendant stacking contexts for hit testing, mirroring the paint
/// pipeline's `collect_stacking_contexts`. Walks children, stops at SC boundaries,
/// accumulates offsets through intermediate non-SC nodes.
fn collect_sc_for_hit_test(
    tree: &rinch_dom::NodeTree,
    children: &[usize],
    offset_x: f32,
    offset_y: f32,
    result: &mut Vec<HitTestScEntry>,
    order_counter: &mut usize,
) {
    for &child_id in children {
        let Some(child) = tree.get(child_id) else {
            continue;
        };

        if child.creates_stacking_context() {
            // position: fixed elements are viewport-relative — zero the offset
            let is_fixed =
                child.computed_style.position == rinch_dom::computed_style::PositionValue::Fixed;
            let z = child.computed_style.z_index.unwrap_or(0);
            result.push(HitTestScEntry {
                z_index: z,
                node_id: child_id,
                offset_x: if is_fixed { 0.0 } else { offset_x },
                offset_y: if is_fixed { 0.0 } else { offset_y },
                dom_order: *order_counter,
            });
            *order_counter += 1;
        } else {
            let child_x = offset_x + child.layout.x;
            let child_y = offset_y + child.layout.y;
            let sx = child.scroll_offset.0 as f32;
            let sy = child.scroll_offset.1 as f32;
            *order_counter += 1;
            collect_sc_for_hit_test(
                tree,
                &child.children,
                child_x - sx,
                child_y - sy,
                result,
                order_counter,
            );
        }
    }
}

fn hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    is_sc_root: bool,
) -> Option<usize> {
    let node = tree.get(node_id)?;

    // position: fixed elements are viewport-relative — ignore parent offsets
    let is_fixed = node.computed_style.position == rinch_dom::computed_style::PositionValue::Fixed;
    let nx = if is_fixed {
        node.layout.x
    } else {
        offset_x + node.layout.x
    };
    let ny = if is_fixed {
        node.layout.y
    } else {
        offset_y + node.layout.y
    };
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
        let is_sc = is_sc_root || node.creates_stacking_context();

        if is_sc {
            // Stacking context root: test children in reverse paint order.
            // Phase 3 (topmost): positive z-index SCs, highest first
            // Phase 2: non-SC children in reverse DOM order
            // Phase 1: negative z-index SCs, closest to 0 first
            let children: Vec<_> = node.children.clone();
            let mut entries = Vec::new();
            let mut order_counter = 0usize;
            collect_sc_for_hit_test(
                tree,
                &children,
                nx - sx,
                ny - sy,
                &mut entries,
                &mut order_counter,
            );

            // Phase 3 reversed: positive z-index SCs (highest z first, later DOM order first)
            let mut positive: Vec<&HitTestScEntry> =
                entries.iter().filter(|e| e.z_index > 0).collect();
            positive.sort_by(|a, b| {
                b.z_index
                    .cmp(&a.z_index)
                    .then(b.dom_order.cmp(&a.dom_order))
            });
            for entry in &positive {
                if let Some(hit) = hit_test_node(
                    tree,
                    entry.node_id,
                    entry.offset_x,
                    entry.offset_y,
                    x,
                    y,
                    true,
                ) {
                    return Some(hit);
                }
            }

            // z-index 0 SCs (reverse DOM order)
            let mut zero: Vec<&HitTestScEntry> =
                entries.iter().filter(|e| e.z_index == 0).collect();
            zero.sort_by(|a, b| b.dom_order.cmp(&a.dom_order));
            for entry in &zero {
                if let Some(hit) = hit_test_node(
                    tree,
                    entry.node_id,
                    entry.offset_x,
                    entry.offset_y,
                    x,
                    y,
                    true,
                ) {
                    return Some(hit);
                }
            }

            // Phase 2 reversed: non-SC children in reverse DOM order
            for &child_id in children.iter().rev() {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if child.creates_stacking_context() {
                    continue;
                }
                if let Some(hit) = hit_test_node(tree, child_id, nx - sx, ny - sy, x, y, false) {
                    return Some(hit);
                }
            }

            // Phase 1 reversed: negative z-index SCs (closest to 0 first)
            let mut negative: Vec<&HitTestScEntry> =
                entries.iter().filter(|e| e.z_index < 0).collect();
            negative.sort_by(|a, b| {
                b.z_index
                    .cmp(&a.z_index)
                    .then(b.dom_order.cmp(&a.dom_order))
            });
            for entry in &negative {
                if let Some(hit) = hit_test_node(
                    tree,
                    entry.node_id,
                    entry.offset_x,
                    entry.offset_y,
                    x,
                    y,
                    true,
                ) {
                    return Some(hit);
                }
            }
        } else {
            // Not a stacking context — only test non-SC children.
            // SC children are skipped; they'll be tested at the ancestor SC level.
            let children: Vec<_> = node.children.clone();
            for &child_id in children.iter().rev() {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if child.creates_stacking_context() {
                    continue;
                }
                if let Some(hit) = hit_test_node(tree, child_id, nx - sx, ny - sy, x, y, false) {
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

/// Detect whether a mouse position is near a window edge for resize.
///
/// All coordinates are in the same unit (physical pixels).
/// Returns the resize direction if the cursor is within the grab zone.
pub(crate) fn detect_resize_edge(
    x: f32,
    y: f32,
    window_width: f32,
    window_height: f32,
    inset: f32,
) -> Option<rinch_platform::ResizeDirection> {
    use rinch_platform::ResizeDirection::*;
    let grab = 3.0;
    let edge = inset + grab;
    let corner = inset + 6.0;

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
pub(crate) fn find_scroll_container(tree: &rinch_dom::NodeTree, start: usize) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        let overflow_y = &node.computed_style.overflow_y;
        match overflow_y {
            OverflowValue::Scroll | OverflowValue::Auto | OverflowValue::Hidden => {
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
    let mut max_bottom: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let bottom = (child.layout.y + child.layout.height) as f64;
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
            OverflowValue::Scroll | OverflowValue::Auto | OverflowValue::Hidden => {
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

/// Compute the total content width of a node from its children's layout bounds.
pub(crate) fn compute_content_width(tree: &rinch_dom::NodeTree, node_id: usize) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let mut max_right: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let right = (child.layout.x + child.layout.width) as f64;
            if right > max_right {
                max_right = right;
            }
        }
    }
    max_right
}

/// The visible content area width: layout.width minus padding and border.
/// Children are positioned relative to the content box, so this is the actual
/// viewport width for scroll calculations.
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

/// Check if a point (x, y) hits a scrollbar.
pub(crate) fn find_scrollbar_hit(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<(usize, f64, f64)> {
    find_scrollbar_hit_node(tree, tree.body_id, 0.0, 0.0, x, y)
}

fn find_scrollbar_hit_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
) -> Option<(usize, f64, f64)> {
    let node = tree.get(node_id)?;

    // Skip hidden subtrees
    if matches!(
        node.computed_style.visibility,
        rinch_dom::computed_style::VisibilityValue::Hidden
            | rinch_dom::computed_style::VisibilityValue::Collapse
    ) {
        return None;
    }

    // position: fixed elements are viewport-relative — ignore parent offsets
    let is_fixed = node.computed_style.position == rinch_dom::computed_style::PositionValue::Fixed;
    let nx = if is_fixed {
        node.layout.x
    } else {
        offset_x + node.layout.x
    };
    let ny = if is_fixed {
        node.layout.y
    } else {
        offset_y + node.layout.y
    };
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
        if let Some(hit) = find_scrollbar_hit_node(tree, child_id, nx - sx, ny - sy, x, y) {
            return Some(hit);
        }
    }

    use rinch_dom::computed_style::OverflowValue;
    let overflow_y = &node.computed_style.overflow_y;

    if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
        let content_height = compute_content_height(tree, node_id);
        let visible_height = compute_visible_content_area_height(tree, node_id);

        if content_height > visible_height {
            let scrollbar_hit_width: f32 = 16.0;
            let scrollbar_left = nx + nw - scrollbar_hit_width;

            if x >= scrollbar_left && x <= nx + nw && y >= ny && y <= ny + nh {
                return Some((node_id, content_height, visible_height));
            }
        }
    }

    None
}

/// Compute the absolute Y position of a node by walking up its parent chain.
pub(crate) fn compute_absolute_y(tree: &rinch_dom::NodeTree, node_id: usize) -> f32 {
    let mut y = 0.0_f32;
    let mut current = Some(node_id);
    while let Some(id) = current {
        if let Some(node) = tree.get(id) {
            y += node.layout.y;
            // position: fixed — viewport-relative, stop accumulating parent offsets
            if node.computed_style.position == rinch_dom::computed_style::PositionValue::Fixed {
                break;
            }
            if let Some(parent_id) = node.parent
                && let Some(parent) = tree.get(parent_id)
            {
                y -= parent.scroll_offset.1 as f32;
            }
            current = node.parent;
        } else {
            break;
        }
    }
    y
}
