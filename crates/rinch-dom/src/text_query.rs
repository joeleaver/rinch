//! Text position query utilities for Parley layouts.
//!
//! This module provides utilities for converting between byte offsets and
//! screen coordinates in text layouts, as well as querying glyph bounds.

use parley::Cursor;
use parley::layout::Affinity;
use peniko::Brush;
use rinch_core::dom::CaretAffinity;

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
    caret_position_for_offset_with_affinity(doc, node_id, byte_offset, CaretAffinity::Downstream)
}

/// [`caret_position_for_offset`] for a caret with `affinity` at a soft wrap
/// (#301): `Upstream` there is the end of the upper line.
pub fn caret_position_for_offset_with_affinity(
    doc: &crate::dom_impl::RinchDocument,
    node_id: u64,
    byte_offset: usize,
    affinity: CaretAffinity,
) -> Option<(f32, f32)> {
    let node = doc.tree.nodes.get(node_id as usize)?;
    let at = |layout: &parley::layout::Layout<Brush>| {
        caret_position_for_offset_layout_with_affinity(layout, byte_offset, affinity)
    };

    // Check inline layout first (IFC root). `vertical_align_shift_at` (#724)
    // is the same lookup `paint::text::draw_text` shifts glyphs by, so a
    // caret inside a `<sub>`/`<sup>` lands where its glyph is actually drawn
    // rather than on the line's unshifted baseline.
    if let Some(ref inline_layout) = node.text_layout {
        let (x, y) = at(&inline_layout.layout);
        let shift = inline_layout.vertical_align_shift_at(byte_offset);
        return Some((x, y + shift));
    }

    // Check cached standalone text layout
    if let Some(ref layout) = node.cached_text_parley {
        return Some(at(layout));
    }

    // For block elements with a single text child (non-IFC case),
    // check the first text child's cached layout
    for &child_id in &node.children {
        if let Some(child) = doc.tree.nodes.get(child_id)
            && let Some(ref layout) = child.cached_text_parley
        {
            return Some(at(layout));
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

    // Check inline layout first (IFC root). Shifted the same way
    // `caret_position_for_offset_with_affinity` is, and for the same reason
    // (#724): this is what makes `get_glyph_bounds` agree with where the
    // glyph is actually painted.
    if let Some(ref inline_layout) = node.text_layout {
        let mut bounds = glyph_bounds_for_offset_layout(&inline_layout.layout, byte_offset)?;
        bounds.y += inline_layout.vertical_align_shift_at(byte_offset);
        return Some(bounds);
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
    caret_position_for_offset_layout_with_affinity(layout, byte_offset, CaretAffinity::Downstream)
}

/// The byte range of the **visual line** a caret at `byte_offset` with
/// `affinity` is drawn on, as its Home and End want it (#1107): `start` is the
/// line's first byte; `end` is its wrap point on a soft-wrapped line (after
/// any hanging whitespace — the same position as the next line's start, which
/// End draws upstream, #301), the byte **before** the break on a line a hard
/// break ends, and the text's end on the last line. `None` for an empty
/// layout.
///
/// The line is the one Parley draws the caret on, in the layout's own space —
/// no hit test and no window geometry — so it answers the same for a caret
/// scrolled out of view, clipped, or transformed. An `Upstream` caret at a
/// soft wrap point is drawn on the upper line; a caret after a hard break on
/// the far line whatever its affinity; a caret right before an inline box
/// (an image, which has no bytes) that ends a line on that line (#1118
/// review).
pub fn visual_line_range(
    layout: &parley::layout::Layout<Brush>,
    byte_offset: usize,
    affinity: CaretAffinity,
) -> Option<std::ops::Range<usize>> {
    // The line the caret is DRAWN on (Parley's own `Cursor`, which puts an
    // upstream caret at a soft wrap on the upper line and a caret after a hard
    // break on the far one), not the line whose byte range holds the caret
    // byte: an inline box has no bytes, so a caret right before an image that
    // ends a line is drawn on that line while its byte starts the next one.
    let line = layout
        .lines()
        .nth(caret_line_index(layout, byte_offset, affinity)?)?;
    Some(line_edge_range(layout, &line))
}

/// The line box `(top, height)` of the visual line a caret at `byte_offset`
/// with `affinity` is drawn on, in the layout's own space: the line Parley
/// draws that caret on (as [`visual_line_range`] finds it), from its
/// `block_min_coord` to its `block_max_coord` — the whole line, its leading
/// included, so every position on one line answers the same box. `None` for
/// an empty layout.
pub fn line_box_for_offset(
    layout: &parley::layout::Layout<Brush>,
    byte_offset: usize,
    affinity: CaretAffinity,
) -> Option<(f32, f32)> {
    let line = layout
        .lines()
        .nth(caret_line_index(layout, byte_offset, affinity)?)?;
    let m = line.metrics();
    Some((m.block_min_coord, m.block_max_coord - m.block_min_coord))
}

/// [`visual_line_range`] in the text-bearing element `node_id`'s layout.
pub fn visual_line_range_for_node(
    doc: &crate::dom_impl::RinchDocument,
    node_id: u64,
    byte_offset: usize,
    affinity: CaretAffinity,
) -> Option<std::ops::Range<usize>> {
    let node = doc.tree.nodes.get(node_id as usize)?;
    if let Some(ref inline_layout) = node.text_layout {
        return visual_line_range(&inline_layout.layout, byte_offset, affinity);
    }
    if let Some(ref layout) = node.cached_text_parley {
        return visual_line_range(layout, byte_offset, affinity);
    }
    None
}

/// Which visual line [`visual_line_edges`] is asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOf {
    /// The line Parley draws a caret at this byte with this affinity on (the
    /// line [`visual_line_range`] finds).
    Caret(usize, CaretAffinity),
    /// The line holding the inline box with this id: where a caret drawn
    /// against that box's edge is (#1116). An inline box has no bytes, so a
    /// caret beside an image is drawn from the image's box, which can be on
    /// another line than the one the byte query picks.
    InlineBox(u64),
}

/// A visual line's edges, as Home and End want them (#1116).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualLineEdges {
    /// The line's bytes, as [`visual_line_range`] gives them.
    pub range: std::ops::Range<usize>,
    /// The inline box that is the line's first item, if one is.
    pub first_box: Option<u64>,
    /// The inline box that is the line's last item, if one is.
    pub last_box: Option<u64>,
    /// No glyph run on the line is right-to-left. Items are in visual order,
    /// so `first_box` and `last_box` are the line's logical edges only then.
    pub ltr: bool,
}

/// The edges of the visual line `at` names: its bytes, and the inline boxes
/// that are its first and last items. An inline box (an image) has no bytes,
/// so the line's edge byte is the byte of both sides of a box there, and of
/// a box at that byte on the neighbouring line: the items say which side of
/// the box the line's edge is (#1116). `None` for an empty layout, or a box
/// the layout does not hold.
pub fn visual_line_edges(
    layout: &parley::layout::Layout<Brush>,
    at: LineOf,
) -> Option<VisualLineEdges> {
    use parley::layout::PositionedLayoutItem;
    let line = layout.lines().nth(line_index(layout, at)?)?;
    let as_box = |item: PositionedLayoutItem<'_, Brush>| match item {
        PositionedLayoutItem::InlineBox(b) => Some(b.id),
        PositionedLayoutItem::GlyphRun(_) => None,
    };
    let ltr = line.items().all(|it| match it {
        PositionedLayoutItem::GlyphRun(g) => !g.run().is_rtl(),
        PositionedLayoutItem::InlineBox(_) => true,
    });
    Some(VisualLineEdges {
        range: line_edge_range(layout, &line),
        first_box: line.items().next().and_then(as_box),
        last_box: line.items().last().and_then(as_box),
        ltr,
    })
}

/// [`visual_line_edges`] in the text-bearing element `node_id`'s layout.
pub fn visual_line_edges_for_node(
    doc: &crate::dom_impl::RinchDocument,
    node_id: u64,
    at: LineOf,
) -> Option<VisualLineEdges> {
    let node = doc.tree.nodes.get(node_id as usize)?;
    if let Some(ref inline_layout) = node.text_layout {
        return visual_line_edges(&inline_layout.layout, at);
    }
    if let Some(ref layout) = node.cached_text_parley {
        return visual_line_edges(layout, at);
    }
    None
}

/// The line box `(top, height)` of the visual line above (`down = false`) or
/// below (`down = true`) the line `at` names, in the layout's own space —
/// where ArrowUp / ArrowDown move a caret inside one inline formatting
/// context (#1121). Read from the layout, not hit-tested, so it answers the
/// same for a line scrolled out of view. `None` when `at` is the layout's
/// first (up) or last (down) line, or names nothing in it.
pub fn adjacent_line_box(
    layout: &parley::layout::Layout<Brush>,
    at: LineOf,
    down: bool,
) -> Option<(f32, f32)> {
    let index = line_index(layout, at)?;
    let target = if down {
        index + 1
    } else {
        index.checked_sub(1)?
    };
    let m = *layout.lines().nth(target)?.metrics();
    Some((m.block_min_coord, m.block_max_coord - m.block_min_coord))
}

/// [`adjacent_line_box`] in the text-bearing element `node_id`'s layout.
pub fn adjacent_line_box_for_node(
    doc: &crate::dom_impl::RinchDocument,
    node_id: u64,
    at: LineOf,
    down: bool,
) -> Option<(f32, f32)> {
    let node = doc.tree.nodes.get(node_id as usize)?;
    if let Some(ref inline_layout) = node.text_layout {
        return adjacent_line_box(&inline_layout.layout, at, down);
    }
    if let Some(ref layout) = node.cached_text_parley {
        return adjacent_line_box(layout, at, down);
    }
    None
}

/// A line's bytes as [`visual_line_range`] gives them: up to the hard break
/// that ends it, if one does.
fn line_edge_range(
    layout: &parley::layout::Layout<Brush>,
    line: &parley::layout::Line<'_, Brush>,
) -> std::ops::Range<usize> {
    use parley::layout::{BreakReason, Cluster};
    let range = line.text_range();
    let mut end = range.end;
    if line.break_reason() == BreakReason::Explicit
        && end > range.start
        && let Some(cluster) = Cluster::from_byte_index(layout, end - 1)
        && cluster.is_hard_line_break()
    {
        end = cluster.text_range().start;
    }
    range.start..end.max(range.start)
}

/// The index of the line `at` names (see [`LineOf`]).
fn line_index(layout: &parley::layout::Layout<Brush>, at: LineOf) -> Option<usize> {
    use parley::layout::PositionedLayoutItem;
    match at {
        LineOf::Caret(byte, affinity) => caret_line_index(layout, byte, affinity),
        LineOf::InlineBox(id) => layout.lines().position(|l| {
            l.items()
                .any(|it| matches!(it, PositionedLayoutItem::InlineBox(b) if b.id == id))
        }),
    }
}

/// The index of the line Parley draws a caret at `byte_offset` with
/// `affinity` on (see [`visual_line_range`]). `None` for an empty layout.
fn caret_line_index(
    layout: &parley::layout::Layout<Brush>,
    byte_offset: usize,
    affinity: CaretAffinity,
) -> Option<usize> {
    layout.lines().len().checked_sub(1)?;
    // Parley draws a caret from its line box's top, `block_min_coord`: the
    // caret's line is the last one starting at or above it. Line boxes can
    // overlap — a line holding an image on its baseline reaches below the
    // next line's top (`-2..42` over `40..64` for a 16px image in 24px
    // lines) — so the first box that merely CONTAINS the caret's top named
    // the line above, and Up, Down, Home and End stepped from it (#1121).
    let (_, cy) = caret_position_for_offset_layout_with_affinity(layout, byte_offset, affinity);
    let mut index = 0;
    for (i, line) in layout.lines().enumerate() {
        if line.metrics().block_min_coord > cy + 0.5 {
            break;
        }
        index = i;
    }
    Some(index)
}

/// [`caret_position_for_offset_layout`] for a caret with `affinity` at a soft
/// wrap (#301): Parley's own `Cursor` affinity, whose geometry puts an
/// `Upstream` caret at a soft line break at the end of the upper line.
pub fn caret_position_for_offset_layout_with_affinity(
    layout: &parley::layout::Layout<Brush>,
    byte_offset: usize,
    affinity: CaretAffinity,
) -> (f32, f32) {
    let affinity = match affinity {
        CaretAffinity::Downstream => Affinity::Downstream,
        CaretAffinity::Upstream => Affinity::Upstream,
    };
    let cursor = Cursor::from_byte_index(layout, byte_offset, affinity);
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
    // The line box's top, as the caret has it (`Cursor::geometry`) — the
    // content area's, under a line-height smaller than it — not
    // `baseline - ascent`, which is one half-leading lower (#1008).
    let line_y = line_metrics.block_min_coord;

    // Use the cluster's advance for width
    let advance = cluster.advance();

    // Get the x position from cursor geometry
    let geom = cursor.geometry(layout, 0.0);

    Some(GlyphBounds {
        x: geom.x0 as f32,
        y: line_y,
        width: advance,
        height: line_metrics.block_max_coord - line_metrics.block_min_coord,
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

/// [`byte_offset_from_position`] for a caret: the answer is a grapheme
/// boundary of `text`, the text `layout` was built from (#1684). Parley
/// splits a glyph that draws several characters — an emoji ZWJ sequence, a
/// flag, a keycap, `❤️` — into one caret stop per character, so a click on the
/// glyph's middle would put the caret inside the cluster; it goes to whichever
/// of the cluster's two edges is nearer `x` instead.
pub fn grapheme_offset_from_position(
    layout: &parley::layout::Layout<Brush>,
    text: &str,
    x: f32,
    y: f32,
) -> usize {
    let offset = byte_offset_from_position(layout, x, y);
    snap_to_grapheme_boundary(layout, text, offset, x)
}

/// `offset` if it is a grapheme boundary of `text`, else whichever edge of
/// the cluster around it has its caret nearer `x` in `layout` (#1684).
pub fn snap_to_grapheme_boundary(
    layout: &parley::layout::Layout<Brush>,
    text: &str,
    offset: usize,
    x: f32,
) -> usize {
    if offset > text.len() || rinch_words::is_grapheme_boundary(text, offset) {
        return offset;
    }
    let before = rinch_words::prev_grapheme_boundary(text, offset);
    let after = rinch_words::next_grapheme_boundary(text, offset);
    let (bx, _) = caret_position_for_offset_layout(layout, before);
    let (ax, _) =
        caret_position_for_offset_layout_with_affinity(layout, after, CaretAffinity::Upstream);
    if (x - bx).abs() <= (x - ax).abs() {
        before
    } else {
        after
    }
}

/// The caret beside the inline box `box_id` in `layout`, as `(x, y, height)`
/// layout-local: on the box's left edge, or its right edge when `after`, at the
/// top of the box's line and one line high — the line box a text caret on that
/// line is drawn in ([`glyph_bounds_for_offset_layout`]). `None` when no line
/// holds the box.
///
/// An inline box occupies no bytes of the layout's text, so no byte offset can
/// say which side of it a caret is on (#1104). Left-to-right only: `after` is
/// the right edge whatever the box's bidi level.
pub fn inline_box_caret(
    layout: &parley::layout::Layout<Brush>,
    box_id: u64,
    after: bool,
) -> Option<(f32, f32, f32)> {
    for line in layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::InlineBox(b) = item
                && b.id == box_id
            {
                let m = line.metrics();
                let x = if after { b.x + b.width } else { b.x };
                return Some((x, m.block_min_coord, m.block_max_coord - m.block_min_coord));
            }
        }
    }
    None
}

/// The inline box under layout-local `(x, y)`, as `(box id, after)`: `after`
/// when the point is on the box's right half. `None` when the point is on no
/// inline box — over text, beside a line's end, or above or below the text.
///
/// Parley's own hit test ([`byte_offset_from_position`]) steps over an inline
/// box to the byte of what follows it, which is the byte of both of the box's
/// sides; this is what tells them apart (#1104). Left-to-right only, like
/// [`inline_box_caret`].
pub fn inline_box_at_point(
    layout: &parley::layout::Layout<Brush>,
    x: f32,
    y: f32,
) -> Option<(u64, bool)> {
    for line in layout.lines() {
        let m = line.metrics();
        if !(m.block_min_coord..m.block_max_coord).contains(&y) {
            continue;
        }
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::InlineBox(b) = item
                && (b.x..b.x + b.width).contains(&x)
            {
                return Some((b.id, x >= b.x + b.width / 2.0));
            }
        }
        return None;
    }
    None
}

/// The byte range of the glyph cluster **under** layout-local `(x, y)` — the
/// character the pointer is over — or `None` when no glyph is under it: beside
/// the end of a line, above or below the text, in an inline box.
///
/// Not [`byte_offset_from_position`], which answers the nearest *caret*
/// boundary: over the right half of a word's last letter that is the position
/// after the letter, which says nothing about the letter itself. Link hover and
/// link activation ask about the letter.
pub fn cluster_range_at_point(
    layout: &parley::layout::Layout<Brush>,
    x: f32,
    y: f32,
) -> Option<std::ops::Range<usize>> {
    // `from_point_exact` clamps `y` to the first and last lines.
    if !(0.0..layout.height()).contains(&y) {
        return None;
    }
    parley::layout::Cluster::from_point_exact(layout, x, y).map(|(c, _)| c.text_range())
}

/// Selection rectangles `(x, y, width, height)` (layout-local) covering the
/// byte range `[a, b)` in `layout`: one rect per visual piece of the range on
/// each line it spans — one per line for left-to-right text, and on a line
/// holding a right-to-left run one per contiguous visual stretch (#1114).
/// Used to render a text selection's highlight.
pub fn selection_rects_for_layout(
    layout: &parley::layout::Layout<Brush>,
    a: usize,
    b: usize,
) -> Vec<(f32, f32, f32, f32)> {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let mut rects = Vec::new();
    let mut lines = layout.lines().peekable();
    while let Some(line) = lines.next() {
        let range = line.text_range();
        let start = a.max(range.start);
        let end = b.min(range.end);
        if start >= end {
            continue;
        }
        let metrics = line.metrics();
        // The line box, `block_min_coord..block_max_coord` — the box Parley's
        // caret (`Cursor::geometry`) stands in and a browser paints a selection
        // over. (It is the content area when the line-height is smaller than
        // that: Parley clamps the leading at 0, so the rect is taller than the
        // line box and overlaps its neighbours, and Chrome paints that too.)
        // `baseline - ascent` is the top plus the half-leading, which put the
        // highlight half a leading below its line and into the next (#1008).
        let top = metrics.block_min_coord;
        // The bottom is where the next line's box begins (#1024). Parley
        // quantizes each line's coords on its own — the top is the line's
        // rounded `y`, the bottom the rounded ascent, descent and leading added
        // to it — so at a fractional line-height two neighbours overlap by a
        // pixel or leave one between them, and a translucent highlight drew a
        // darker (or an empty) row there. Chrome's highlights are contiguous
        // (16px x 1.65: `0-26, 26-53, 53-79, 79-106`, which this reproduces).
        // Not under negative leading, where the rect is the content area and
        // overlapping the neighbours is what Chrome paints; and the last line
        // keeps its own bottom.
        let bottom = match lines.peek() {
            Some(next) if metrics.leading >= 0.0 => next.metrics().block_min_coord,
            _ => metrics.block_max_coord,
        };
        let height = bottom - top;
        // A line holding a right-to-left run is walked in visual order: a
        // logical range there is as many pieces as it has contiguous visual
        // stretches, and its logical end can stand left of its start (#1114).
        // A line with none — every line of left-to-right text — keeps the one
        // start-to-end rect below and pays one pass over its runs.
        if line.runs().any(|run| run.is_rtl()) {
            bidi_line_pieces(layout, &line, start, end, |l, r| {
                rects.push((l, top, (r - l).max(1.0), height))
            });
            continue;
        }
        let left = Cursor::from_byte_index(layout, start, Affinity::Downstream)
            .geometry(layout, 0.0)
            .x0 as f32;
        // A range that runs to (or past) the line's end ends at the line's
        // trailing edge: an *upstream* caret there. A downstream caret at a
        // soft line break stands at the start of the NEXT line (x = 0), which
        // made every wrapped line's highlight a 1px sliver (#1010).
        //
        // A line ended by a hard break (`<br>`, a preserved `\n`) holds the
        // break in its range, and even the upstream caret after it stands at
        // the next line's start. Such a line is highlighted to the caret
        // before the break plus a newline's width, the rule Parley's own
        // `Selection::geometry` uses (a quarter of ascent + descent) — Chrome
        // 153 paints `abc<br>def` line 0 across `0..32`, "abc" being 28px.
        let right = if end == range.end
            && range.end > range.start
            && line.break_reason() == parley::layout::BreakReason::Explicit
        {
            let before_break = Cursor::from_byte_index(layout, range.end - 1, Affinity::Downstream)
                .geometry(layout, 0.0)
                .x0 as f32;
            before_break + (metrics.ascent + metrics.descent) * 0.25
        } else {
            let end_affinity = if end == range.end {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            };
            Cursor::from_byte_index(layout, end, end_affinity)
                .geometry(layout, 0.0)
                .x0 as f32
        };
        rects.push((left, top, (right - left).max(1.0), height));
    }
    rects
}

/// The visual pieces `(left, right)` of the byte range `[a, b)` on `line`,
/// left to right: each contiguous stretch of clusters whose first byte is in
/// the range, walked in visual order (Parley's `Selection::geometry_with`
/// rule). An inline box (an image, an `inline-block`) whose index is in the
/// range is selected; a cluster with no advance (a bidi control, a zero-width
/// space) adds no piece. A selected hard break (`<br>`, a preserved `\n`) that
/// is in no cluster of the line is a newline's width (a quarter of ascent +
/// descent) from the caret before it, joined to the piece it touches — what
/// [`selection_rects_for_layout`] gives it on a left-to-right line. In a
/// right-to-left paragraph the newline is a cluster of the line with no
/// advance, so a selected one shows no width there.
fn bidi_line_pieces(
    layout: &parley::layout::Layout<Brush>,
    line: &parley::layout::Line<'_, Brush>,
    a: usize,
    b: usize,
    mut emit: impl FnMut(f32, f32),
) {
    let mut pieces: Vec<(f32, f32)> = Vec::new();
    let mut open: Option<(f32, f32)> = None;
    for item in line.items() {
        let glyph_run = match item {
            parley::layout::PositionedLayoutItem::GlyphRun(g) => g,
            parley::layout::PositionedLayoutItem::InlineBox(bx) => {
                let index = layout
                    .inline_boxes()
                    .iter()
                    .find(|ib| ib.id == bx.id)
                    .map(|ib| ib.index);
                if index.is_some_and(|i| (a..b).contains(&i)) {
                    let (x0, x1) = (bx.x, bx.x + bx.width);
                    open = Some(match open {
                        Some((l, _)) => (l, x1),
                        None => (x0, x1),
                    });
                } else if let Some(piece) = open.take() {
                    pieces.push(piece);
                }
                continue;
            }
        };
        let mut x = glyph_run.offset();
        for cluster in glyph_run.run().visual_clusters() {
            let x0 = x;
            x += cluster.advance();
            if cluster.advance() == 0.0 {
                continue;
            }
            if (a..b).contains(&cluster.text_range().start) {
                open = Some(match open {
                    Some((l, _)) => (l, x),
                    None => (x0, x),
                });
            } else if let Some(piece) = open.take() {
                pieces.push(piece);
            }
        }
    }
    pieces.extend(open);
    let range = line.text_range();
    if line.break_reason() == parley::layout::BreakReason::Explicit
        && range.end > range.start
        && (a..b).contains(&(range.end - 1))
    {
        let metrics = line.metrics();
        let nx = Cursor::from_byte_index(layout, range.end - 1, Affinity::Downstream)
            .geometry(layout, 0.0)
            .x0 as f32;
        let nr = nx + (metrics.ascent + metrics.descent) * 0.25;
        match pieces.iter_mut().find(|(l, r)| *l <= nx && nx <= *r) {
            Some(piece) => piece.1 = piece.1.max(nr),
            None => pieces.push((nx, nr)),
        }
    }
    for (l, r) in pieces {
        emit(l, r);
    }
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
                        let local = prev.dom_text_len + prev.node_offset;
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
            // A collapsed run or an expanded tab puts the flat and DOM
            // offsets out of step; the range's `offset_map` says where.
            let local = r.dom_for_flat(ifc_offset - r.flat_start);
            return Some((r.node_id, local + r.node_offset));
        }
    }

    // Offset is at or past the end — prefer last non-br range (unless allow_br)
    for r in ranges.iter().rev() {
        if allow_br || !r.is_br {
            let local = r.dom_text_len + r.node_offset;
            return Some((r.node_id, local));
        }
    }
    let last = ranges.last().unwrap();
    let local = last.dom_text_len + last.node_offset;
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
            let local = node_offset.saturating_sub(r.node_offset);
            return Some(r.flat_start + r.flat_for_dom(local));
        }
    }
    None
}
