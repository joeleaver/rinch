//! Text position query utilities for Parley layouts.
//!
//! This module provides utilities for converting between byte offsets and
//! screen coordinates in text layouts, as well as querying glyph bounds.

use parley::Cursor;
use parley::layout::Affinity;
use peniko::Brush;

/// Bounding box for a glyph cluster.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct GlyphBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Get the caret position for a byte offset in a specific node.
///
/// This is a wrapper that looks up the text layout for the given node
/// and calls the layout-based function.
///
/// # Arguments
/// * `doc` - The RinchDocument containing the node
/// * `node_id` - The node ID to query
/// * `byte_offset` - The UTF-8 byte offset in the text
///
/// # Returns
/// Some((x, y)) if the node has text layout, None otherwise
pub fn caret_position_for_offset(
    doc: &crate::dom_impl::RinchDocument,
    node_id: u64,
    byte_offset: usize,
) -> Option<(f32, f32)> {
    let node = doc.tree.nodes.get(node_id as usize)?;

    // Check inline layout first (IFC root)
    if let Some(ref inline_layout) = node.text_layout {
        return Some(caret_position_for_offset_layout(
            &inline_layout.layout,
            byte_offset,
        ));
    }

    // Check cached standalone text layout
    if let Some(ref layout) = node.cached_text_parley {
        return Some(caret_position_for_offset_layout(layout, byte_offset));
    }

    // For block elements with a single text child (non-IFC case),
    // check the first text child's cached layout
    for &child_id in &node.children {
        if let Some(child) = doc.tree.nodes.get(child_id)
            && let Some(ref layout) = child.cached_text_parley
        {
            return Some(caret_position_for_offset_layout(layout, byte_offset));
        }
    }

    None
}

/// Get the glyph bounds for a byte offset in a specific node.
///
/// This is a wrapper that looks up the text layout for the given node
/// and calls the layout-based function.
///
/// # Arguments
/// * `doc` - The RinchDocument containing the node
/// * `node_id` - The node ID to query
/// * `byte_offset` - The UTF-8 byte offset in the text
///
/// # Returns
/// Some(GlyphBounds) if the node has text layout and offset is valid, None otherwise
pub fn glyph_bounds_for_offset(
    doc: &crate::dom_impl::RinchDocument,
    node_id: u64,
    byte_offset: usize,
) -> Option<GlyphBounds> {
    let node = doc.tree.nodes.get(node_id as usize)?;

    // Check inline layout first (IFC root)
    if let Some(ref inline_layout) = node.text_layout {
        return glyph_bounds_for_offset_layout(&inline_layout.layout, byte_offset);
    }

    // Check cached standalone text layout
    if let Some(ref layout) = node.cached_text_parley {
        return glyph_bounds_for_offset_layout(layout, byte_offset);
    }

    // For block elements with a single text child (non-IFC case),
    // check the first text child's cached layout
    for &child_id in &node.children {
        if let Some(child) = doc.tree.nodes.get(child_id)
            && let Some(ref layout) = child.cached_text_parley
        {
            return glyph_bounds_for_offset_layout(layout, byte_offset);
        }
    }

    None
}

/// Get the (x, y) position for a caret at the given byte offset in a layout.
///
/// Returns coordinates relative to the layout origin, where:
/// - x is the horizontal offset from the left edge
/// - y is the top of the line containing the offset
///
/// # Arguments
/// * `layout` - The Parley text layout
/// * `byte_offset` - The UTF-8 byte offset in the text
///
/// # Returns
/// A tuple (x, y) representing the caret position
pub fn caret_position_for_offset_layout(
    layout: &parley::layout::Layout<Brush>,
    byte_offset: usize,
) -> (f32, f32) {
    let cursor = Cursor::from_byte_index(layout, byte_offset, Affinity::Downstream);
    let geom = cursor.geometry(layout, 0.0);
    (geom.x0 as f32, geom.y0 as f32)
}

/// Get the bounding box for the glyph cluster containing the given byte offset in a layout.
///
/// Returns None if the offset is out of bounds or layout is empty.
///
/// # Arguments
/// * `layout` - The Parley text layout
/// * `byte_offset` - The UTF-8 byte offset in the text
///
/// # Returns
/// An optional `GlyphBounds` representing the cluster's bounding box
pub fn glyph_bounds_for_offset_layout(
    layout: &parley::layout::Layout<Brush>,
    byte_offset: usize,
) -> Option<GlyphBounds> {
    // Use downstream cursor to get geometry of the cluster at this offset
    let cursor = Cursor::from_byte_index(layout, byte_offset, Affinity::Downstream);
    let [upstream, downstream] = cursor.visual_clusters(layout);

    // Try to get the downstream (right) cluster first, then upstream
    let cluster = downstream.or(upstream)?;

    let line = cluster.line();
    let line_metrics = line.metrics();
    let line_y = line_metrics.baseline - line_metrics.ascent;

    // Use the cluster's advance for width
    let advance = cluster.advance();

    // Get the x position from cursor geometry
    let geom = cursor.geometry(layout, 0.0);

    Some(GlyphBounds {
        x: geom.x0 as f32,
        y: line_y,
        width: advance,
        height: line_metrics.line_height,
    })
}

/// Find the byte offset closest to the given (x, y) position.
///
/// This is the inverse of `caret_position_for_offset_layout`. It finds which
/// character position is closest to the given screen coordinates.
///
/// # Arguments
/// * `layout` - The Parley text layout
/// * `x` - The horizontal coordinate
/// * `y` - The vertical coordinate
///
/// # Returns
/// The byte offset of the character closest to the position
pub fn byte_offset_from_position(layout: &parley::layout::Layout<Brush>, x: f32, y: f32) -> usize {
    Cursor::from_point(layout, x, y).index()
}

// ── IFC ↔ DomCursor conversions ─────────────────────────────────────────

/// Convert an IFC flat byte offset to a DOM cursor `(node_id, offset_within_node)`.
///
/// Searches `ranges` for the range containing `ifc_offset`.  When `ifc_offset`
/// falls exactly on a boundary between two ranges the **earlier** range is
/// preferred (cursor at end-of-node rather than start-of-next).
///
/// When `allow_br` is false, `<br>` ranges are deflected to the adjacent
/// text range (end of previous text, or start of next text).
/// When `allow_br` is true, `<br>` ranges are returned directly — use this
/// for vertical cursor movement where blank lines should be valid targets.
///
/// Returns `None` if `ranges` is empty.
pub fn ifc_offset_to_dom_cursor(
    ranges: &[crate::node::IfcTextRange],
    ifc_offset: usize,
    allow_br: bool,
) -> Option<(usize, usize)> {
    if ranges.is_empty() {
        return None;
    }

    // Try to find a range that contains the offset (inclusive start, exclusive end).
    // For boundary cases (offset == flat_end of one range == flat_start of the next),
    // prefer the earlier range so the cursor stays at "end of previous node".
    for (i, r) in ranges.iter().enumerate() {
        if ifc_offset >= r.flat_start && ifc_offset < r.flat_end {
            // Deflect <br> cursor to adjacent text (unless allow_br is set)
            if r.is_br && !allow_br {
                // Prefer end of previous text range
                for prev in ranges[..i].iter().rev() {
                    if !prev.is_br {
                        let local = prev.flat_end - prev.flat_start + prev.node_offset;
                        return Some((prev.node_id, local));
                    }
                }
                // No previous text — try start of next text range
                for next in &ranges[i + 1..] {
                    if !next.is_br {
                        return Some((next.node_id, next.node_offset));
                    }
                }
                // All ranges are <br> — return it as last resort
            }
            let local = ifc_offset - r.flat_start + r.node_offset;
            return Some((r.node_id, local));
        }
    }

    // Offset is at or past the end — prefer last non-br range (unless allow_br)
    for r in ranges.iter().rev() {
        if allow_br || !r.is_br {
            let local = r.flat_end - r.flat_start + r.node_offset;
            return Some((r.node_id, local));
        }
    }
    let last = ranges.last().unwrap();
    let local = last.flat_end - last.flat_start + last.node_offset;
    Some((last.node_id, local))
}

/// Convert a DOM cursor `(node_id, offset_within_node)` to an IFC flat byte offset.
///
/// Returns `None` if no range maps the given `node_id`.
pub fn dom_cursor_to_ifc_offset(
    ranges: &[crate::node::IfcTextRange],
    node_id: usize,
    node_offset: usize,
) -> Option<usize> {
    for r in ranges {
        if r.node_id == node_id {
            // Clamp node_offset to the range length
            let range_len = r.flat_end - r.flat_start;
            let clamped = node_offset.saturating_sub(r.node_offset).min(range_len);
            return Some(r.flat_start + clamped);
        }
    }
    None
}
