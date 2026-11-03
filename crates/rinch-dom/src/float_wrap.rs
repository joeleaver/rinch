//! Lines beside floats.
//!
//! An IFC root is a Taffy leaf, and Taffy hands a leaf in a block formatting
//! context that has floats the context itself (rinch's Taffy fork,
//! `TaffyTree::compute_layout_with_block_measure`). [`FloatBands::of`] reads
//! from it how much of the root's content box the floats leave free at each
//! height, and the line breaker gives each line the room beside the floats
//! its whole height overlaps (`parley`'s per-line `line_x` and
//! `line_max_advance`). A line that cannot hold even its first piece of
//! content beside a float is moved down to where there is more room (CSS 2.2
//! §9.5: "the line box is shifted downward until either some content fits or
//! there are no more floats present"), and lines below the floats get the
//! full width again.

/// How far a float intrudes into an IFC root's content box over one span of
/// its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Band {
    /// The top and bottom of the span, from the content box's top.
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    /// How far in from the content box's left and right edges lines start
    /// and end over the span.
    pub(crate) left: f32,
    pub(crate) right: f32,
}

/// Where an IFC root's lines may go beside the floats of its block
/// formatting context, from its content box's top down. Empty when no float
/// is beside it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FloatBands {
    bands: Vec<Band>,
    /// The content-box width the bands were read at: where a right float's
    /// room ends is measured from it, not from the width a line is broken
    /// at (paint breaks an auto-width root a pixel wider).
    width: f32,
}

/// More bands than any real float context has; a guard against a slot that
/// does not move down.
const MAX_BANDS: usize = 4096;

impl FloatBands {
    /// The bands a leaf whose content box starts `content_left` and
    /// `content_top` in from its border box, and is `content_width` wide,
    /// sees in `ctx`.
    pub(crate) fn of(
        ctx: &taffy::BlockContext<'_>,
        content_left: f32,
        content_top: f32,
        content_width: f32,
    ) -> Self {
        let mut bands = Vec::new();
        let mut y = content_top;
        while bands.len() < MAX_BANDS {
            let slot = ctx.find_content_slot(y, taffy::Clear::None, None);
            if slot.segment_id.is_none() || !slot.height.is_finite() || slot.height <= 0.0 {
                break;
            }
            let left = (slot.x - content_left).max(0.0);
            let right = (content_left + content_width - (slot.x + slot.width)).max(0.0);
            let top = slot.y.max(y) - content_top;
            let bottom = slot.y + slot.height - content_top;
            if left > 0.0 || right > 0.0 {
                bands.push(Band {
                    top,
                    bottom,
                    left,
                    right,
                });
            }
            y = slot.y + slot.height;
        }
        Self {
            bands,
            width: content_width,
        }
    }

    /// The bands a leaf with `style`, laid out with `inputs`, sees in `ctx`.
    /// Empty when the leaf's width is not known yet.
    pub(crate) fn of_leaf(
        ctx: &taffy::BlockContext<'_>,
        inputs: &taffy::LayoutInput,
        style: &taffy::Style,
    ) -> Self {
        match LeafBox::of(inputs, style) {
            Some(leaf) if ctx.has_active_floats(0.0) => leaf.bands(ctx),
            _ => Self::default(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bands.is_empty()
    }

    /// The left inset and width of a line from `top` to `bottom` broken at
    /// `full`: the room left beside every band the line overlaps. A line of
    /// no height overlaps the band it starts in.
    pub(crate) fn room(&self, top: f32, bottom: f32, full: f32) -> (f32, f32) {
        let bottom = bottom.max(top + f32::EPSILON);
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for b in &self.bands {
            if b.top < bottom && top < b.bottom {
                left = left.max(b.left);
                right = right.max(b.right);
            }
        }
        let end = if right > 0.0 {
            (self.width - right).min(full)
        } else {
            full
        };
        (left, (end - left).max(0.0))
    }

    /// The first height below `y` where the room changes: the bottom of a
    /// band that a line starting at `y` overlaps, or `None` when no band
    /// reaches below `y`.
    pub(crate) fn next_change_below(&self, y: f32) -> Option<f32> {
        self.bands
            .iter()
            .map(|b| b.bottom)
            .filter(|&bottom| bottom > y + 0.001)
            .reduce(f32::min)
    }
}

/// Where a leaf's content box is in its border box, and how wide it is: what
/// turns the block context's coordinates into the lines'.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LeafBox {
    left: f32,
    top: f32,
    width: f32,
}

impl LeafBox {
    /// The content box of a leaf with `style`, laid out with `inputs`;
    /// `None` when its width is not known yet.
    pub(crate) fn of(inputs: &taffy::LayoutInput, style: &taffy::Style) -> Option<Self> {
        use taffy::{CoreStyle, ResolveOrZero};
        let cb = inputs.parent_size.width;
        let padding = style.padding().resolve_or_zero(cb, |_, _| 0.0);
        let border = style.border().resolve_or_zero(cb, |_, _| 0.0);
        let width = inputs
            .known_dimensions
            .width
            .or(match inputs.available_space.width {
                taffy::AvailableSpace::Definite(w) => Some(w),
                _ => None,
            })?;
        let left = padding.left + border.left;
        Some(Self {
            left,
            top: padding.top + border.top,
            width: (width - left - padding.right - border.right).max(0.0),
        })
    }

    /// The room the floats in `ctx` leave this leaf's lines.
    pub(crate) fn bands(&self, ctx: &taffy::BlockContext<'_>) -> FloatBands {
        FloatBands::of(ctx, self.left, self.top, self.width)
    }
}

/// Whether Taffy leaf `leaf` is an IFC root's measure leaf: the leaf an IFC
/// root lays its lines out in when it keeps out-of-flow children (floats
/// among them) as Taffy children. The floats right before such a leaf are
/// its root's own, which its line breaker places at the lines they stand in;
/// an IFC root that is its own leaf has no float children, so the floats
/// before it are its siblings, placed where they stand.
pub(crate) fn is_measure_leaf(
    nodes: &crate::node_slab::NodeSlab,
    leaf: taffy::NodeId,
    context: Option<&crate::node::NodeContext>,
) -> bool {
    matches!(context, Some(crate::node::NodeContext::InlineRoot(root))
        if nodes.get(*root).is_some_and(|n| n.taffy_id != Some(leaf)))
}

/// [`crate::ifc::PlaceFloats`] over the block context a paragraph is laid
/// out in: its pending floats (`BlockContext::pending_floats`, the floats
/// right before its measure leaf, which rinch's Taffy fork leaves for the
/// leaf to place) are placed where the line breaker reaches them.
pub(crate) struct BlockFloats<'c, 'b> {
    pub(crate) ctx: &'c mut taffy::BlockContext<'b>,
    pub(crate) nodes: &'c crate::node_slab::NodeSlab,
    pub(crate) leaf: LeafBox,
}

impl BlockFloats<'_, '_> {
    /// The pending float for node `float`, if it is one and still waits.
    fn waiting(&self, float: crate::node::RawNodeId) -> Option<usize> {
        let taffy_id = self.nodes.get(float)?.taffy_id?;
        self.ctx
            .pending_floats()
            .iter()
            .position(|p| p.node_id == taffy_id && p.placed().is_none())
    }
}

impl crate::ifc::PlaceFloats for BlockFloats<'_, '_> {
    fn width(&self, float: crate::node::RawNodeId) -> Option<f32> {
        let index = self.waiting(float)?;
        Some(self.ctx.pending_floats()[index].size.width)
    }

    fn place(&mut self, float: crate::node::RawNodeId, y: f32) -> Option<FloatBands> {
        let index = self.waiting(float)?;
        self.ctx.place_pending_float(index, y + self.leaf.top);
        Some(self.leaf.bands(self.ctx))
    }
}

/// Where `cluster` starts, from the content box's left edge.
///
/// parley 0.11.1 places a line's glyph runs and inline boxes from the line's
/// start (`LineMetrics::inline_min_coord`, its `line_x`), and so do its hit
/// test and selection rectangles, but `Cluster::visual_offset` (and
/// `Cursor::geometry`, which uses it) leave the start out. It is `0` but for a
/// line beside a float, where it is where the line starts.
pub(crate) fn cluster_x(cluster: &parley::layout::Cluster<'_, peniko::Brush>) -> Option<f32> {
    Some(cluster.visual_offset()? + cluster.line().metrics().inline_min_coord)
}

/// `cursor`'s caret box in `layout`, with its line's start counted
/// ([`cluster_x`]): `Cursor::geometry` at no width, moved to the line its top
/// is the top of.
pub(crate) fn caret_box(
    layout: &parley::layout::Layout<peniko::Brush>,
    cursor: &parley::editing::Cursor,
) -> parley::BoundingBox {
    let mut geom = cursor.geometry(layout, 0.0);
    if let Some(line) = layout
        .lines()
        .find(|l| f64::from(l.metrics().block_min_coord) == geom.y0)
    {
        let start = f64::from(line.metrics().inline_min_coord);
        geom.x0 += start;
        geom.x1 += start;
    }
    geom
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bands(b: &[(f32, f32, f32, f32)]) -> FloatBands {
        FloatBands {
            bands: b
                .iter()
                .map(|&(top, bottom, left, right)| Band {
                    top,
                    bottom,
                    left,
                    right,
                })
                .collect(),
            width: 300.0,
        }
    }

    #[test]
    fn a_line_gets_the_room_beside_every_band_it_overlaps() {
        let b = bands(&[(0.0, 50.0, 100.0, 0.0), (40.0, 80.0, 0.0, 60.0)]);
        assert_eq!(b.room(0.0, 20.0, 300.0), (100.0, 200.0));
        assert_eq!(b.room(30.0, 50.0, 300.0), (100.0, 140.0));
        assert_eq!(b.room(50.0, 70.0, 300.0), (0.0, 240.0));
        assert_eq!(b.room(80.0, 100.0, 300.0), (0.0, 300.0));
        // Broken a pixel wider: the right float's edge stays where it is.
        assert_eq!(b.room(50.0, 70.0, 301.0), (0.0, 240.0));
        assert_eq!(b.room(80.0, 100.0, 301.0), (0.0, 301.0));
    }

    #[test]
    fn the_room_changes_at_the_bottom_of_a_band() {
        let b = bands(&[(0.0, 50.0, 100.0, 0.0), (40.0, 80.0, 0.0, 60.0)]);
        assert_eq!(b.next_change_below(0.0), Some(50.0));
        assert_eq!(b.next_change_below(50.0), Some(80.0));
        assert_eq!(b.next_change_below(80.0), None);
    }
}
