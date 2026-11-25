//! `DomLayout::layout_box_in` on rinch-dom (#3065): the web's `offsetParent`
//! chain and `offsetLeft`/`offsetTop`/`offsetWidth`/`offsetHeight`, answered
//! from the layout, so both hosts give a sortable list's slot the same box.

use rinch_core::reactive::ElementBounds;

use crate::computed_style::{DisplayValue, PositionValue};
use crate::node::NodeTree;

/// `node`'s border box in `container`'s padding-box frame, as the web sums
/// `offsetLeft` (plus each `offsetParent`'s `clientLeft` on the way) up the
/// `offsetParent` chain to `container`; `None` where that chain does not
/// reach it.
///
/// Each step is rounded on its own, as the browser rounds each
/// `offsetLeft`: the offset of a box's unrounded border box from its
/// `offsetParent`'s unrounded padding edge, half up (`floor(x + 0.5)`,
/// Chrome 158's rounding; Firefox 160 rounds a negative tie away from zero),
/// and the width and height their own unrounded lengths rounded. Chrome
/// quantizes layout to 1/64 px (`LayoutUnit`) before it rounds, and this
/// rounds the `f32` layout, so within 1/64 px of a tie the two can answer a
/// pixel apart. Summing
/// the rounded layout instead takes the difference of two edges each rounded
/// where it falls in the page, which differs by a pixel wherever the
/// `offsetParent` sits at a fraction (#3065 review).
pub(super) fn layout_box_in(tree: &NodeTree, node: u64, container: u64) -> Option<ElementBounds> {
    let mut chain = vec![node];
    while *chain.last()? != container {
        chain.push(offset_parent(tree, *chain.last()?)?);
    }
    let (mut x, mut y) = (0.0, 0.0);
    for step in chain.windows(2) {
        let (at, parent) = (step[0], step[1]);
        let (ax, ay) = unrounded_origin(tree, at)?;
        let (px, py) = unrounded_origin(tree, parent)?;
        let (bl, bt) = border(tree, parent)?;
        x += round_half_up(ax - px - bl);
        y += round_half_up(ay - py - bt);
        if parent != container {
            // `clientLeft`/`clientTop`: whole pixels.
            x += round_half_up(bl);
            y += round_half_up(bt);
        }
    }
    let (width, height) = unrounded_size(tree, node)?;
    Some(ElementBounds {
        x,
        y,
        width: round_half_up(width),
        height: round_half_up(height),
    })
}

/// The browser's rounding of an `offset*` value: half up.
fn round_half_up(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// `id`'s border-left and border-top widths.
fn border(tree: &NodeTree, id: u64) -> Option<(f32, f32)> {
    let cs = &tree.nodes.get(id)?.computed_style;
    Some((
        cs.border_left_width.to_px().max(0.0),
        cs.border_top_width.to_px().max(0.0),
    ))
}

/// Where `id`'s border box starts in the page, as laid out and rounded: the
/// parent-relative layout origins summed up to the root.
fn rounded_origin(tree: &NodeTree, id: u64) -> Option<(f32, f32)> {
    let (mut x, mut y) = (0.0, 0.0);
    let mut at = Some(id);
    while let Some(a) = at {
        let n = tree.nodes.get(a)?;
        x += n.layout.x;
        y += n.layout.y;
        at = n.parent;
    }
    Some((x, y))
}

/// Where `id`'s border box starts in the page before Taffy rounded it: its
/// Taffy origin, wherever rounding that gives the box's laid-out origin.
/// rinch places some boxes itself after Taffy (an absolute one whose
/// containing block is not its parent, an atomic inline in a line), and
/// some boxes have no Taffy node (an inline element); for those the rounded
/// origin is all there is, so the answer is whole pixels as before.
fn unrounded_origin(tree: &NodeTree, id: u64) -> Option<(f32, f32)> {
    let rounded = rounded_origin(tree, id)?;
    let Some(taffy_id) = tree.nodes.get(id)?.taffy_id else {
        return Some(rounded);
    };
    let (ux, uy) = crate::paint::scrollbar::taffy_origin(&tree.taffy, taffy_id);
    let agrees = round_half_up(ux) == rounded.0 && round_half_up(uy) == rounded.1;
    Some(if agrees { (ux, uy) } else { rounded })
}

/// `id`'s border-box size before Taffy rounded it, where that is the size it
/// was laid out at (within the pixel rounding moves an edge); otherwise its
/// laid-out size.
fn unrounded_size(tree: &NodeTree, id: u64) -> Option<(f32, f32)> {
    let n = tree.nodes.get(id)?;
    let laid = (n.layout.width, n.layout.height);
    let Some(taffy_id) = n.taffy_id else {
        return Some(laid);
    };
    let u = tree.taffy.unrounded_layout(taffy_id).size;
    let agrees = (u.width - laid.0).abs() < 1.0 && (u.height - laid.1).abs() < 1.0;
    Some(if agrees { (u.width, u.height) } else { laid })
}

/// The web's `offsetParent` of `id` (CSSOM View, `HTMLElement.offsetParent`),
/// from the computed styles: `None` for the body, the root, a `position:
/// fixed` box, and a box not rendered (`display: none` on it or an ancestor,
/// or `display: contents` on it); otherwise the nearest ancestor that is a
/// containing block for absolutely positioned boxes
/// ([`crate::Node::establishes_abs_containing_block`], the set Chrome 158 and
/// Firefox 160 both answer), or the body, or (for a box that is not
/// positioned itself) a `td`, `th` or `table`.
fn offset_parent(tree: &NodeTree, id: u64) -> Option<u64> {
    let n = tree.nodes.get(id)?;
    let cs = &n.computed_style;
    if id == tree.body_id
        || n.parent.is_none()
        || matches!(cs.position, PositionValue::Fixed)
        || matches!(cs.display, DisplayValue::Contents)
    {
        return None;
    }
    let mut at = Some(id);
    while let Some(a) = at {
        let an = tree.nodes.get(a)?;
        if matches!(an.computed_style.display, DisplayValue::None) {
            return None;
        }
        at = an.parent;
    }
    let statically = matches!(cs.position, PositionValue::Static);
    let mut at = n.parent;
    while let Some(a) = at {
        let an = tree.nodes.get(a)?;
        if a == tree.body_id
            || an.establishes_abs_containing_block()
            || (statically && matches!(an.tag(), Some("td" | "th" | "table")))
        {
            return Some(a);
        }
        at = an.parent;
    }
    None
}
