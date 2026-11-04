//! Whether a `visibility: hidden` subtree can show anything at all (#843,
//! #2137): one answer for paint, the stacking collector and hit testing.
//!
//! `visibility: hidden` is per node, not per subtree. A hidden box draws none
//! of its own visuals and is not hit, but `visibility` inherits and a
//! descendant may set it back to `visible`, which is drawn and hit — so a walk
//! cannot stop at the first hidden node the way it stops at `display: none`.
//! It can stop at a hidden node with **nothing visible below it**, and that is
//! the common case: rinch's own `Drawer`, `Popover` and menus are
//! `visibility: hidden` while closed (so their transitions can run, #751), and
//! nothing inside them overrides it.
//!
//! [`shows_below`] answers it, memoised in the hit cache
//! ([`crate::hit_cache::HitCache::shows`]) for the cache's generation. That
//! generation moves on every change that can change the answer: a restyle
//! (`resolve_styles`), every mutating `DomDocument` method (the tree's shape),
//! `resolve_layout` (the box tree's anonymous boxes), a transition or animation
//! tick that changes a node's [`crate::hit_cache::HitStyleKey`] (which holds
//! `visibility` and `display`), and `transition::propagate_inherited_visibility`
//! when it hands a held value down. A write that reaches into `tree.nodes[..]`
//! and changes `visibility` without one of those must call
//! `tree.hit_cache.invalidate()`, as the hit cache's own module doc says.
//!
//! **The box tree, not the element tree.** Paint, the collector and hit testing
//! all descend through `RinchDocument::box_tree_children`, so that is what this
//! walks: an anonymous block's run members are what it shows, and a hoisted
//! out-of-flow box (#591) is found under the block that hosts it, not under the
//! inline element it was written in — which is exactly where the walks that
//! would draw it reach it. So `false` is a proof that no walk entering the node
//! finds a visible element.

use std::cell::Cell;

use crate::computed_style::VisibilityValue;
use crate::node::{Node, NodeTree, RawNodeId};

/// The `visibility` `node` is drawn and hit with, read live.
///
/// An element's is its own. A text node is drawn with its parent element's
/// (paint's text mask reads the parent's), and so is an **anonymous block
/// box**: it copies its container's `visibility` once, when layout builds it
/// (`ComputedStyle::for_anonymous_box`), and a later change of the container's
/// `visibility` alone — opening a closed `Drawer` — is paint-only, so nothing
/// refreshes the copy. Read from the copy, a box built while its container
/// was hidden stayed hidden after the container opened: paint skipped its text
/// (review of #2137) and hit testing skipped it too, which predates #2137.
fn live_visibility(tree: &NodeTree, node: &Node) -> Option<VisibilityValue> {
    if node.is_text() || node.is_anonymous_block_box {
        node.parent
            .and_then(|p| tree.get(p))
            .map(|p| p.computed_style.visibility)
    } else {
        Some(node.computed_style.visibility)
    }
}

/// Whether `node` draws none of its own visuals and is not hit itself:
/// `visibility: hidden` or `collapse`, read live ([`live_visibility`]: a text
/// node and an anonymous block box answer from their container).
#[inline]
pub fn hides_own(tree: &NodeTree, node: &Node) -> bool {
    matches!(
        live_visibility(tree, node),
        Some(VisibilityValue::Hidden | VisibilityValue::Collapse)
    )
}

/// Whether some node below `id` is drawn `visibility: visible`, i.e. whether a
/// hidden `id`'s subtree can hold anything to draw or hit.
///
/// Walks the box-tree children (see the [module docs](self)). A text child
/// counts by its parent element's visibility ([`hides_own`]), the rule the
/// text mask draws it by: under a hidden element that is the element itself,
/// but an anonymous block box's run members answer from its container.
/// Memoised in the hit cache, so a hidden subtree is walked once per
/// generation, not once per probe or per paint; asked only of hidden nodes, so
/// a visible tree never pays for it.
pub fn shows_below(tree: &NodeTree, id: RawNodeId) -> bool {
    if let Some(s) = tree.hit_cache.shows(id) {
        return s;
    }
    let shows = tree.get_live(id).is_some_and(|node| {
        crate::RinchDocument::box_tree_children(&tree.nodes, node.id)
            .iter()
            .any(|&child_id| {
                tree.get_live(child_id).is_some_and(|child| {
                    !hides_own(tree, child) || (!child.is_text() && shows_below(tree, child_id))
                })
            })
    });
    tree.hit_cache.store_shows(id, shows);
    shows
}

thread_local! {
    /// Whether paint and the stacking collector skip a subtree that shows
    /// nothing. On, always, outside tests: the switch exists so the
    /// differential tests can paint the same document both ways and compare
    /// the pixels.
    static CULL_HIDDEN: Cell<bool> = const { Cell::new(true) };

    /// A paint-order watch is on (`paint::set_paint_order_watch`, #1865 S4):
    /// paint reports when it starts each watched node, and a node it skips is
    /// not reported. While one is on nothing is skipped — by paint or by the
    /// collector, so a watched hidden node is started where it always was,
    /// positioned or not.
    static PAINT_WATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Turn the hidden-subtree skip in paint and the stacking collector off (or
/// back on) on this thread. For the differential tests that compare a picture
/// with the skip against one without it; hit testing is not affected.
#[doc(hidden)]
pub fn set_hidden_subtree_cull(on: bool) {
    CULL_HIDDEN.set(on);
}

/// A paint-order watch started (`true`) or ended.
pub(crate) fn set_paint_watching(on: bool) {
    PAINT_WATCHING.set(on);
}

/// Whether paint and the stacking collector may skip `node` and its whole
/// subtree: it is a hidden element (or anonymous box, judged by its
/// container) and nothing below it is visible ([`shows_below`]). Such a
/// subtree draws nothing — every visual paint emits is gated on the
/// visibility of the element it belongs to — and is hit nowhere, so neither
/// its own descent nor the entries it would hoist into an enclosing stacking
/// sequence can change a pixel. Never while a paint-order watch is on.
///
/// A visible element pays one enum test.
#[inline]
pub fn subtree_shows_nothing(tree: &NodeTree, node: &Node) -> bool {
    !node.is_text()
        && hides_own(tree, node)
        && CULL_HIDDEN.get()
        && !PAINT_WATCHING.get()
        && !shows_below(tree, node.id)
}
