use super::super::ScrollAxis;
#[cfg(doc)]
use super::scroll::{find_scroll_container_at_point, find_scrollbar_hit};
use super::targets::{hides_own_hit, takes_pointer};
#[cfg(doc)]
use super::walk::hit_test_node;
use rinch_dom::paint::scrollbar::ScrollbarAxis;
use rinch_dom::stacking::{DeferredBars, PaintOrder};
use std::cell::{Cell, RefCell};

/// What a walk of [`hit_test_node`] looks for. The plain hit test is one;
/// the scrollbar search a press runs ([`find_scrollbar_hit`]) and the wheel's
/// geometric fallback ([`find_scroll_container_at_point`]) are the others.
/// All three are the same walk, so they agree on everything that decides
/// what reaches a point: paint order, the clip chain each entry was hoisted
/// past, `mask-image`, `visibility` and `pointer-events` (#1727, #2684).
/// Monomorphised, so the plain hit test carries none of the others' checks.
pub(super) trait Probe {
    /// Whether the walk tries scroll containers' overlay bars, each where
    /// paint draws it (#1727).
    const BARS: bool = false;
    /// The counter each visited node bumps.
    const VISITS: rinch_dom::perf::Counter;

    /// Whether a node the walk finds under the point is the answer. One that
    /// is not is passed over, and the walk goes on to what is painted below
    /// it.
    #[inline]
    fn accepts(&self, _tree: &rinch_dom::NodeTree, _node_id: u64) -> bool {
        true
    }

    /// The scrollbar search's state, when [`Self::BARS`].
    #[inline]
    fn bars(&self) -> Option<&BarSearch> {
        None
    }
}

/// The plain hit test: the topmost node painted under the point.
pub(super) struct PlainHit;

impl Probe for PlainHit {
    const VISITS: rinch_dom::perf::Counter = rinch_dom::perf::Counter::HitTestNodesVisited;
}

/// The scrollbar search: the walk ends at the topmost thing painted under
/// the point, and when that is a scroll container's bar, [`Self::found`]
/// names it.
#[derive(Default)]
pub(super) struct BarSearch {
    /// The scrollers whose bars the stacking sequences being walked draw
    /// themselves, after their last contained entry (#1717): their own node
    /// does not offer them. A stack, as paint's `DEFERRED_BARS` is.
    pub(super) deferred: RefCell<Vec<u64>>,
    /// The bar the walk ended on.
    pub(super) found: Cell<Option<(u64, ScrollAxis)>>,
}

impl Probe for BarSearch {
    const BARS: bool = true;
    const VISITS: rinch_dom::perf::Counter = rinch_dom::perf::Counter::ScrollProbeNodesVisited;

    #[inline]
    fn bars(&self) -> Option<&BarSearch> {
        Some(self)
    }
}

impl BarSearch {
    /// End the walk on `node_id`'s bar on `axis`.
    pub(super) fn found(&self, node_id: u64, axis: ScrollAxis) -> Option<u64> {
        self.found.set(Some((node_id, axis)));
        Some(node_id)
    }
}

/// Pushes a sequence's deferred scrollers onto [`BarSearch::deferred`] and
/// takes them back off on drop.
pub(super) struct DeferredScope<'a>(&'a BarSearch, usize);

impl<'a> DeferredScope<'a> {
    pub(super) fn enter(search: &'a BarSearch, bars: &[DeferredBars]) -> Self {
        let mut d = search.deferred.borrow_mut();
        let len = d.len();
        d.extend(bars.iter().map(|b| b.node_id));
        Self(search, len)
    }
}

impl Drop for DeferredScope<'_> {
    fn drop(&mut self) {
        self.0.deferred.borrow_mut().truncate(self.1);
    }
}

/// The wheel's fallback: the topmost scroll container painted under the
/// point that overflows on the axis, past whatever is painted over it.
pub(super) struct ScrollerSearch(pub(super) ScrollbarAxis);

impl Probe for ScrollerSearch {
    const VISITS: rinch_dom::perf::Counter = rinch_dom::perf::Counter::ScrollProbeNodesVisited;

    #[inline]
    fn accepts(&self, tree: &rinch_dom::NodeTree, node_id: u64) -> bool {
        rinch_dom::paint::scrollbar::overflows(tree, node_id, self.0)
    }
}

/// The bar of scroll container `node` under `(lx, ly)`, a point relative to
/// its border-box origin in its own space: the hit strips are the paint
/// pass's own geometry (#400, #444). None for a container that takes no
/// pointer events or is hidden, nor for a bar a scroll lock shuts out (#474).
pub(super) fn bar_under(
    tree: &rinch_dom::NodeTree,
    node: &rinch_dom::Node,
    node_id: u64,
    lx: f32,
    ly: f32,
) -> Option<ScrollAxis> {
    if !takes_pointer(node) {
        return None;
    }
    let bars = rinch_dom::paint::scrollbar::scrollbars(tree, node_id, 1.0);
    let (lx, ly) = (lx as f64, ly as f64);
    let axis = [
        (ScrollbarAxis::Vertical, ScrollAxis::Vertical),
        (ScrollbarAxis::Horizontal, ScrollAxis::Horizontal),
    ]
    .into_iter()
    .find_map(|(axis, scroll_axis)| {
        bars.axis(axis)?;
        let (x0, y0, x1, y1) = bars.hit_strip(axis)?;
        (lx >= x0 && lx <= x1 && ly >= y0 && ly <= y1).then_some(scroll_axis)
    })?;
    (!tree.scroll_locked_out(node_id)).then_some(axis)
}

/// The deferred bar `bar` of `order` under `(x, y)`, a point in the
/// sequence's own space: drawn under the clips its scroller is drawn under,
/// at the scroller's border-box origin (#1717).
pub(super) fn deferred_bar_under(
    tree: &rinch_dom::NodeTree,
    order: &PaintOrder,
    bar: &DeferredBars,
    x: f32,
    y: f32,
) -> Option<ScrollAxis> {
    let owner = tree.get_live(bar.node_id)?;
    if hides_own_hit(tree, owner)
        || !order
            .clips_for_span(bar.clips)
            .iter()
            .all(|c| c.contains(x as f64, y as f64))
    {
        return None;
    }
    bar_under(
        tree,
        owner,
        bar.node_id,
        x - bar.origin_x as f32,
        y - bar.origin_y as f32,
    )
}
