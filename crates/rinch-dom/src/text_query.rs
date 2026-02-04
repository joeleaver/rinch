//! Text position query utilities for Parley layouts.
//!
//! This module provides utilities for converting between byte offsets and
//! screen coordinates in text layouts, as well as querying glyph bounds.

use parley::layout::PositionedLayoutItem;
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
        return Some(caret_position_for_offset_layout(&inline_layout.layout, byte_offset));
    }

    // Check cached standalone text layout
    if let Some(ref layout) = node.cached_text_parley {
        return Some(caret_position_for_offset_layout(layout, byte_offset));
    }

    // For block elements with a single text child (non-IFC case),
    // check the first text child's cached layout
    for &child_id in &node.children {
        if let Some(child) = doc.tree.nodes.get(child_id) {
            if let Some(ref layout) = child.cached_text_parley {
                return Some(caret_position_for_offset_layout(layout, byte_offset));
            }
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
        if let Some(child) = doc.tree.nodes.get(child_id) {
            if let Some(ref layout) = child.cached_text_parley {
                return glyph_bounds_for_offset_layout(layout, byte_offset);
            }
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
    // Get first line metrics for default values
    let first_line_top = layout
        .lines()
        .next()
        .map(|line| line.metrics().baseline - line.metrics().ascent)
        .unwrap_or(0.0);

    if byte_offset == 0 {
        return (0.0, first_line_top);
    }

    let mut last_x = 0.0f32;
    let mut last_y = first_line_top;

    for line in layout.lines() {
        let line_metrics = line.metrics();
        let line_y = line_metrics.baseline - line_metrics.ascent;

        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let run_start = run.text_range().start;
                let run_end = run.text_range().end;

                if byte_offset <= run_start {
                    return (glyph_run.offset(), line_y);
                }

                if byte_offset <= run_end {
                    // Byte offset is within this run - iterate glyphs
                    let mut gx = glyph_run.offset();
                    for cluster in run.cluster_range() {
                        let Some(cluster_data) = run.get(cluster) else {
                            continue;
                        };
                        let cluster_start = cluster_data.text_range().start;
                        let cluster_end = cluster_data.text_range().end;

                        if byte_offset <= cluster_start {
                            return (gx, line_y);
                        }
                        if byte_offset < cluster_end {
                            // Strictly within this cluster - return start of cluster
                            return (gx, line_y);
                        }
                        // byte_offset == cluster_end means after this cluster
                        gx += cluster_data.advance();
                    }
                    return (gx, line_y);
                }

                // Track end position
                last_x = glyph_run.offset();
                for cluster in run.cluster_range() {
                    if let Some(cluster_data) = run.get(cluster) {
                        last_x += cluster_data.advance();
                    }
                }
                last_y = line_y;
            }
        }
    }
    (last_x, last_y)
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
    for line in layout.lines() {
        let line_metrics = line.metrics();
        let line_y = line_metrics.baseline - line_metrics.ascent;
        let line_height = line_metrics.line_height;

        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let run_start = run.text_range().start;
                let run_end = run.text_range().end;

                // Skip runs that don't contain this offset
                if byte_offset < run_start || byte_offset >= run_end {
                    continue;
                }

                // Find the cluster containing this offset
                let mut gx = glyph_run.offset();
                for cluster in run.cluster_range() {
                    let Some(cluster_data) = run.get(cluster) else {
                        continue;
                    };
                    let cluster_start = cluster_data.text_range().start;
                    let cluster_end = cluster_data.text_range().end;

                    if byte_offset >= cluster_start && byte_offset < cluster_end {
                        return Some(GlyphBounds {
                            x: gx,
                            y: line_y,
                            width: cluster_data.advance(),
                            height: line_height,
                        });
                    }

                    gx += cluster_data.advance();
                }
            }
        }
    }

    None
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
pub fn byte_offset_from_position(
    layout: &parley::layout::Layout<Brush>,
    x: f32,
    y: f32,
) -> usize {
    if x <= 0.0 && y <= 0.0 {
        return 0;
    }

    // First, find which line was clicked based on Y coordinate
    let mut target_line_idx = 0usize;
    let mut cumulative_height = 0.0f32;

    for (idx, line) in layout.lines().enumerate() {
        let line_height = line.metrics().line_height;
        if y < cumulative_height + line_height {
            target_line_idx = idx;
            break;
        }
        cumulative_height += line_height;
        target_line_idx = idx; // Default to last line if click is below all lines
    }

    // Now find the character position within that line
    let mut best_offset = 0usize;

    for (line_idx, line) in layout.lines().enumerate() {
        if line_idx < target_line_idx {
            // Track the end of previous lines
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    best_offset = run.text_range().end;
                }
            }
            continue;
        }

        if line_idx > target_line_idx {
            break;
        }

        // This is the target line - find the character position based on X
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let mut gx = glyph_run.offset();

                for cluster in run.cluster_range() {
                    let Some(cluster_data) = run.get(cluster) else {
                        continue;
                    };
                    let cluster_start = cluster_data.text_range().start;
                    let advance = cluster_data.advance();

                    let mid_x = gx + advance / 2.0;

                    if x <= mid_x {
                        return cluster_start;
                    }

                    gx += advance;
                    best_offset = cluster_data.text_range().end;
                }
            }
        }
        break;
    }

    best_offset
}
