use super::super::ScrollAxis;
use super::probe::{BarSearch, ScrollerSearch};
#[cfg(doc)]
use super::resize::pointer_on_scrollbar_thumb;
use super::walk::hit_test_node;
use rinch_dom::paint::scrollbar::ScrollbarAxis;

/// Find the nearest ancestor (or self) that is a scroll container.
///
/// Only `overflow: scroll` and `overflow: auto` are considered scrollable.
/// `overflow: hidden` clips content but should NOT consume mousewheel events —
/// the wheel should bubble up to the nearest actual scroll container.
///
/// Test-only since the review of #2084: the wheel picks its container with
/// [`wheel_scroll_target`] (the nearest that can still move toward it, not the
/// nearest that overflows). The `overflows` tests (#769, #474) still ask this.
#[cfg(test)]
pub(crate) fn find_scroll_container(tree: &rinch_dom::NodeTree, start: u64) -> Option<u64> {
    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        if rinch_dom::paint::scrollbar::overflows(tree, node_id, ScrollbarAxis::Vertical) {
            return Some(node_id);
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    tree.get(tree.body_id)?;
    if rinch_dom::paint::scrollbar::overflows(tree, tree.body_id, ScrollbarAxis::Vertical) {
        return Some(tree.body_id);
    }
    None
}

/// The scroller a Page Up / Page Down in an editor moves (#2020): the nearest
/// ancestor of `start` (or `start`) that overflows vertically and that no
/// `lock_scroll` overlay locks out ([`rinch_dom::NodeTree::scroll_locked_out`],
/// the wheel's and drag auto-scroll's rule) — else the body, on the same
/// terms.
///
/// The walk stops at a `position: fixed` box (checked itself first): the
/// viewport holds it, so no scroller outside it moves it, and paging one would
/// scroll the page behind a modal and leave the caret where it was. Then
/// nothing scrolls (`None`).
#[cfg(feature = "windowing")]
pub(crate) fn page_scroller(tree: &rinch_dom::NodeTree, start: u64) -> Option<u64> {
    let usable = |id: u64| {
        rinch_dom::paint::scrollbar::overflows(tree, id, ScrollbarAxis::Vertical)
            && !tree.scroll_locked_out(id)
    };
    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        if usable(node_id) {
            return Some(node_id);
        }
        if node.box_position() == rinch_dom::computed_style::PositionValue::Fixed {
            return None;
        }
        current = node.parent;
    }
    tree.get(tree.body_id)?;
    usable(tree.body_id).then_some(tree.body_id)
}

/// Find the nearest ancestor (or self) that is a horizontal scroll container.
///
/// Test-only since the review of #2084, like [`find_scroll_container`].
#[cfg(test)]
pub(crate) fn find_horizontal_scroll_container(
    tree: &rinch_dom::NodeTree,
    start: u64,
) -> Option<u64> {
    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        if rinch_dom::paint::scrollbar::overflows(tree, node_id, ScrollbarAxis::Horizontal) {
            return Some(node_id);
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    tree.get(tree.body_id)?;
    if rinch_dom::paint::scrollbar::overflows(tree, tree.body_id, ScrollbarAxis::Horizontal) {
        return Some(tree.body_id);
    }
    None
}

/// The one container a wheel of `(dx, dy)` (the DOM's signs) at `(x, y)` over
/// `hit` scrolls: the nearest of `hit` and its ancestors, then the body, that
/// can still move toward the wheel
/// ([`rinch_dom::paint::scrollbar::can_scroll_toward`], which takes in
/// `lock_scroll`). The whole wheel goes to it: a diagonal wheel over a strip
/// that scrolls only sideways moves the strip and drops the rest, as Chrome
/// and Firefox do — it is not split into one container per axis.
///
/// A container at its end in that direction is passed over, so the wheel
/// chains to the one around it, as a browser does for a new gesture.
///
/// Only when nothing on that walk **overflows** on an axis the wheel moves on
/// (the hit is in another branch than its scroller: an absolutely positioned
/// overlay, a `position: fixed` modal's backdrop) is the container found
/// geometrically at the point (`find_scroll_container_at_point` and its
/// horizontal twin). A walk that met a container which could not move —
/// at its end, or locked out — does not fall back, so a locked page behind a
/// modal is not reached around the lock.
pub(crate) fn wheel_scroll_target(
    tree: &rinch_dom::NodeTree,
    hit: u64,
    x: f32,
    y: f32,
    dx: f64,
    dy: f64,
) -> Option<u64> {
    use rinch_dom::paint::scrollbar::{can_scroll_toward, overflows};
    let overflows_on_wheel = |id: u64| {
        (dy != 0.0 && overflows(tree, id, ScrollbarAxis::Vertical))
            || (dx != 0.0 && overflows(tree, id, ScrollbarAxis::Horizontal))
    };
    let mut met = false;
    let chain = std::iter::successors(Some(hit), |&id| tree.get(id).and_then(|n| n.parent))
        .chain(tree.get(tree.body_id).map(|_| tree.body_id));
    for id in chain {
        if tree.get(id).is_none() {
            continue;
        }
        if can_scroll_toward(tree, id, dx, dy) {
            return Some(id);
        }
        met |= overflows_on_wheel(id);
    }
    if met {
        return None;
    }
    let vertical = (dy != 0.0)
        .then(|| find_scroll_container_at_point(tree, x, y))
        .flatten();
    let horizontal = (dx != 0.0)
        .then(|| find_horizontal_scroll_container_at_point(tree, x, y))
        .flatten();
    [vertical, horizontal]
        .into_iter()
        .flatten()
        .find(|&id| can_scroll_toward(tree, id, dx, dy))
}

/// Find a vertical scroll container at (x, y) by geometric search.
///
/// When the hit-tested node lives in a different DOM branch from the scroll
/// container (e.g., an absolutely-positioned overlay sibling), the parent-chain
/// walk in `find_scroll_container` fails. This asks for the topmost scroll
/// container painted under the point that overflows vertically, passing over
/// whatever is painted above it: the hit test's own walk ([`ScrollerSearch`]),
/// so a scroller under a CSS transform or inside a `position: fixed` overlay is
/// found where the wheel pointer actually is (#203), and one that paint clips,
/// masks or hides there, or that takes no pointer events, is not (#2684).
/// Picking the wrong container here is silent: the wheel scrolls something
/// else.
pub(crate) fn find_scroll_container_at_point(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<u64> {
    let probe = ScrollerSearch(ScrollbarAxis::Vertical);
    hit_test_node(&probe, tree, tree.body_id, 0.0, 0.0, x, y, x, y, None)
}

/// Find a horizontal scroll container at (x, y) by geometric search.
///
/// The horizontal twin of [`find_scroll_container_at_point`].
pub(crate) fn find_horizontal_scroll_container_at_point(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<u64> {
    let probe = ScrollerSearch(ScrollbarAxis::Horizontal);
    hit_test_node(&probe, tree, tree.body_id, 0.0, 0.0, x, y, x, y, None)
}

/// The width of the invisible strip along a container's edge that counts as
/// its scrollbar for hit-testing — [`rinch_dom::paint::scrollbar::HIT_THICKNESS`],
/// which is where it is defined, beside the painted geometry it has to agree
/// with (#444). Only tests name it; the strips come from
/// [`rinch_dom::paint::scrollbar::Scrollbars::hit_strip`].
#[cfg(test)]
pub(crate) const SCROLLBAR_HIT_THICKNESS: f32 = rinch_dom::paint::scrollbar::HIT_THICKNESS as f32;

/// A scrollbar the pointer is over.
///
/// # Coordinate space
///
/// The `x`/`y` fed to [`find_scrollbar_hit`] are the pointer coordinates the
/// shell hands `PlatformEvent`, compared here against **logical** layout rects
/// straight out of Taffy — no scale factor is applied anywhere in this
/// function, and none is needed: since #299 those coordinates are logical on
/// every host, so both bars are grabbable at any scale factor. (Before #299
/// the desktop shell forwarded winit's *physical* position, which displaced
/// both bars by the same amount; fixing the shell fixed both at once, with no
/// change here.) The paint pass is unaffected: it multiplies by `scale` and
/// works in device pixels.
pub(crate) struct ScrollbarHit {
    /// The scroll container whose bar was hit.
    pub node_id: u64,
    /// Which of its two bars.
    pub axis: ScrollAxis,
    /// That bar's geometry, in the container's own space — the *paint pass's*
    /// own computation, so a press and a drag land where the thumb is drawn
    /// rather than on a separately derived track (#400).
    pub track: rinch_dom::paint::scrollbar::ScrollbarTrack,
}

/// Check if a point (x, y) hits a scrollbar.
///
/// **Only a bar that is the topmost thing painted at the point** (#1727), as
/// in Chrome: the search is the hit test's own walk ([`BarSearch`]), which
/// tries each scroll container's bars where paint draws them — a container's
/// own after everything it draws itself, a deferred one right after its last
/// contained entry in the enclosing sequence (#1717). So a modal backdrop, a
/// popover, a fixed header or a later positioned sibling drawn over a bar
/// takes the press, while a positioned or `z-index` child under the bar (an
/// `Editor`, whose sheet makes it `relative; z-index: 0`) does not. Being that
/// walk, it agrees with `hit_test` on what reaches the point: a scroller
/// standing past an ancestor's box that does not clip is reached (#2678,
/// #1441), and one past a clip on its containing-block chain, a masked
/// ancestor's box, or hidden or `pointer-events: none` is not (#2684); and it
/// costs what a hit test costs, pruned by the same extents.
///
/// The strips are tested in the container's **own** space, which is where the
/// paint pass draws the thumbs — it composes `node_transform` before stroking
/// them — so a scroller inside a `transform: scale()` or `translate()` is
/// grabbable at the bar you can see rather than at the untransformed layout
/// rect (#203). The caller wanting the pointer's position *along* the track
/// (jump-to-click, drag) asks [`rinch_dom::paint::point_in_painted_box`] for
/// the same mapping.
///
/// **A bar a scroll lock has shut out is not here** (#474): while a `lock_scroll`
/// overlay is open, the page's own bar is passed over and the overlay's answers
/// normally. Gating in this one function rather than at the press site keeps the
/// two readers agreeing — [`pointer_on_scrollbar_thumb`] arbitrates the
/// borderless-window resize edge against this answer, so a bar that cannot be
/// dragged also stops stealing the edge from the resize handle (#399/#420).
/// The bar is still **painted**: paint reads the same geometry from
/// `rinch_dom::paint::scrollbar` and knows nothing about the lock, so a locked
/// page's thumb is visible and inert. That is the deliberate half of the
/// divergence the guide documents — desktop suppresses the scroll, web
/// suppresses the scrollbar.
///
/// The strips, and the corner neither claims, come from the same module as
/// the painted tracks (#444): where both bars are present their strips would
/// overlap in a square at the far end and one would silently win the click,
/// so each stops at it and the corner falls through to ordinary click
/// handling on the container.
pub(crate) fn find_scrollbar_hit(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<ScrollbarHit> {
    let search = BarSearch::default();
    hit_test_node(&search, tree, tree.body_id, 0.0, 0.0, x, y, x, y, None);
    let (node_id, axis) = search.found.get()?;
    // The track the walk found the strip of, as paint computes it (#400).
    let bars = rinch_dom::paint::scrollbar::scrollbars(tree, node_id, 1.0);
    let track = bars.axis(match axis {
        ScrollAxis::Vertical => ScrollbarAxis::Vertical,
        ScrollAxis::Horizontal => ScrollbarAxis::Horizontal,
    })?;
    Some(ScrollbarHit {
        node_id,
        axis,
        track,
    })
}
