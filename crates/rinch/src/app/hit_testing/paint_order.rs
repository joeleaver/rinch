// `PlainHit` names the probe only `plain_paint_order` asks for, and that
// function exists only with `windowing`.
#[cfg(feature = "windowing")]
use super::probe::PlainHit;
use super::probe::Probe;
use super::targets::{takes_pointer, visible_text_at};
use super::walk::hit_test_node;
use std::rc::Rc;

use rinch_dom::stacking::{
    PaintOrder, paints_at_stacking_root, stacking_hit_order, stacking_paint_order,
};

/// What the content and float passes of a run painted in Appendix E phases
/// (#2276) draw at `(x, y)`, topmost first: every line, atomic inline and
/// flex or grid item of the run's children (deep, through in-flow blocks),
/// in reverse, then every float. `None` leaves the point to the backgrounds
/// pass, the ordinary walk of the children. `kids` are the run's children in
/// tree order, each with the origin its `hit_test_node` is entered at.
#[inline(never)]
pub(super) fn phased_run_hit<P: Probe>(
    probe: &P,
    tree: &rinch_dom::NodeTree,
    kids: impl DoubleEndedIterator<Item = (u64, f32, f32)> + Clone,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<u64> {
    let enter =
        |id: u64, ox: f32, oy: f32| hit_test_node(probe, tree, id, ox, oy, x, y, vx, vy, None);
    let accepts = |id: u64| probe.accepts(tree, id);
    phased_run_hit_with(tree, kids, x, y, &enter, &accepts)
}

/// [`phased_run_hit`] with the walk it enters a box by, and the probe's
/// acceptance, given: the oracle of the prune tests runs the same passes
/// with its own walk.
pub(super) fn phased_run_hit_with(
    tree: &rinch_dom::NodeTree,
    kids: impl DoubleEndedIterator<Item = (u64, f32, f32)> + Clone,
    x: f32,
    y: f32,
    enter: &dyn Fn(u64, f32, f32) -> Option<u64>,
    accepts: &dyn Fn(u64) -> bool,
) -> Option<u64> {
    let run = PhasedRun {
        tree,
        x,
        y,
        enter,
        accepts,
    };
    for floats in [false, true] {
        for (id, ox, oy) in kids.clone().rev() {
            if let Some(hit) = run.member(id, ox, oy, floats) {
                return Some(hit);
            }
        }
    }
    None
}

/// The point a phased run is probed at, and how a box is entered.
struct PhasedRun<'a> {
    tree: &'a rinch_dom::NodeTree,
    x: f32,
    y: f32,
    enter: &'a dyn Fn(u64, f32, f32) -> Option<u64>,
    accepts: &'a dyn Fn(u64) -> bool,
}

impl PhasedRun<'_> {
    /// One member of the run, entered at `(ox, oy)`, and its in-flow block
    /// descendants: their line content (`floats == false`) or their floats.
    fn member(&self, id: u64, ox: f32, oy: f32, floats: bool) -> Option<u64> {
        let (tree, x, y) = (self.tree, self.x, self.y);
        let node = tree.get_live(id)?;
        if paints_at_stacking_root(node) || (node.is_text() && node.ifc_root.is_some()) {
            return None;
        }
        // Atomic units are hit whole, in the pass that paints them.
        if node.is_flex_or_grid_item.get() {
            return if floats {
                None
            } else {
                (self.enter)(id, ox, oy)
            };
        }
        if rinch_dom::paint::is_float_box(node) {
            return if floats {
                (self.enter)(id, ox, oy)
            } else {
                None
            };
        }
        let (dx, dy) = rinch_dom::paint::ifc_content_box_offset(tree, node);
        let (nx, ny) = (ox + node.layout.x + dx, oy + node.layout.y + dy);
        let (clip_x, clip_y) = node.clip_axes();
        if clip_x || clip_y {
            let (w, h) = (node.layout.width, node.layout.height);
            let (left, top, right, bottom) = rinch_dom::paint::padding_box_insets(node);
            if (clip_x && !(x >= nx + left && x <= nx + w - right))
                || (clip_y && !(y >= ny + top && y <= ny + h - bottom))
            {
                return None;
            }
        }
        let (ix, iy) = (
            nx - node.scroll_offset.0 as f32,
            ny - node.scroll_offset.1 as f32,
        );
        let kids = rinch_dom::RinchDocument::box_tree_children(&tree.nodes, id);
        for &child in kids.iter().rev() {
            let Some(c) = tree.get_live(child) else {
                continue;
            };
            if rinch_dom::paint::drawn_by_its_ifc(tree, c) {
                // An atomic inline is its line's content, hit whole.
                if !floats
                    && c.display_mode.is_atomic_inline()
                    && !paints_at_stacking_root(c)
                    && let Some(hit) = (self.enter)(child, ix, iy)
                {
                    return Some(hit);
                }
                continue;
            }
            if let Some(hit) = self.member(child, ix, iy, floats) {
                return Some(hit);
            }
        }
        // The box's own lines: the text under the point is its block's, or
        // its inline element's — the walk into the box finds which — wherever
        // the line is, inside the box or overflowing it, as Chrome 158 hits
        // it.
        if !floats && node.text_layout.is_some() && visible_text_at(tree, node, x - nx, y - ny) {
            if let Some(hit) = (self.enter)(id, ox, oy) {
                return Some(hit);
            }
            if takes_pointer(node) && (self.accepts)(id) {
                return Some(id);
            }
        }
        None
    }
}

/// Whether box `child` stands inside a `position: relative` split inline
/// (#513, #2408): the inline is positioned, so it and what it holds are above
/// the in-flow boxes around it in Chrome — the **unmoved** anonymous box of
/// the run after the block among them, which its offset can move the block
/// over (review of #2497). Hit testing tries such boxes before their
/// siblings. One parent check for nearly every box.
pub(super) fn in_relative_split_inline(
    tree: &rinch_dom::NodeTree,
    child: &rinch_dom::Node,
) -> bool {
    use rinch_dom::computed_style::{DisplayValue, PositionValue};
    let mut cur = child.parent;
    while let Some(p) = cur.and_then(|p| tree.get_live(p)) {
        if p.is_split_inline() {
            if p.computed_style.position == PositionValue::Relative {
                return true;
            }
        } else if p.computed_style.display != DisplayValue::Contents {
            return false;
        }
        cur = p.parent;
    }
    false
}

/// `root`'s stacking sequence at base `(ox, oy)`, from the tree's hit cache
/// when nothing has invalidated it since it was built (see
/// [`rinch_dom::hit_cache`]). Paint builds its own at the DPI scale; this one
/// is at `scale = 1.0`, the unit hit testing works in, so it is cached apart.
///
/// The scrollbar search ([`Probe::BARS`]) walks the sequence with
/// [`PaintOrder::bars`] (`stacking_paint_order`), cached apart from the plain
/// one: it tries each deferred bar where paint draws it.
pub(super) fn cached_paint_order<P: Probe>(
    tree: &rinch_dom::NodeTree,
    root: u64,
    ox: f64,
    oy: f64,
) -> Rc<PaintOrder> {
    if let Some(order) = tree.hit_cache.order(root, ox, oy, P::BARS) {
        return order;
    }
    let order = Rc::new(if P::BARS {
        stacking_paint_order(tree, root, 1.0, ox, oy)
    } else {
        stacking_hit_order(tree, root, 1.0, ox, oy)
    });
    tree.hit_cache
        .store_order(root, ox, oy, P::BARS, order.clone());
    order
}

/// `root`'s stacking sequence for the plain hit test, from the hit cache: the
/// one an input-region walk ([`super::input_region`](super::super::input_region)) follows, so it agrees
/// with the hit test on which clips each hoisted box is under.
#[cfg(feature = "windowing")]
pub(crate) fn plain_paint_order(
    tree: &rinch_dom::NodeTree,
    root: u64,
    ox: f64,
    oy: f64,
) -> Rc<PaintOrder> {
    cached_paint_order::<PlainHit>(tree, root, ox, oy)
}
