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

/// Whether `node` draws none of its own visuals and is not hit itself:
/// `visibility: hidden` or `collapse`. A text node is drawn with its parent
/// element's visibility (paint reads the parent's), so it is answered from
/// there.
#[inline]
pub fn hides_own(tree: &NodeTree, node: &Node) -> bool {
    let hidden = |n: &Node| {
        matches!(
            n.computed_style.visibility,
            VisibilityValue::Hidden | VisibilityValue::Collapse
        )
    };
    if node.is_text() {
        node.parent.and_then(|p| tree.get(p)).is_some_and(hidden)
    } else {
        hidden(node)
    }
}

/// Whether some element below `id` computes to `visibility: visible`, i.e.
/// whether a hidden `id`'s subtree can hold anything to draw or hit.
///
/// Walks the box-tree children (see the [module docs](self)). Text nodes are
/// skipped: they take their parent's visibility ([`hides_own`]). Memoised in
/// the hit cache, so a hidden subtree is walked once per generation, not once
/// per probe or per paint; asked only of hidden nodes, so a visible tree never
/// pays for it.
pub fn shows_below(tree: &NodeTree, id: RawNodeId) -> bool {
    if let Some(s) = tree.hit_cache.shows(id) {
        return s;
    }
    let shows = tree.get_live(id).is_some_and(|node| {
        crate::RinchDocument::box_tree_children(&tree.nodes, node.id)
            .iter()
            .any(|&child_id| {
                tree.get_live(child_id).is_some_and(|child| {
                    !child.is_text() && (!hides_own(tree, child) || shows_below(tree, child_id))
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
}

/// Turn the hidden-subtree skip in paint and the stacking collector off (or
/// back on) on this thread. For the differential tests that compare a picture
/// with the skip against one without it; hit testing is not affected.
#[doc(hidden)]
pub fn set_hidden_subtree_cull(on: bool) {
    CULL_HIDDEN.set(on);
}

/// Whether paint and the stacking collector may skip `node` and its whole
/// subtree: it is a hidden element (or anonymous box) and nothing below it is
/// visible ([`shows_below`]). Such a subtree draws nothing — every visual
/// paint emits is gated on the visibility of the element it belongs to — and
/// is hit nowhere, so neither its own descent nor the entries it would hoist
/// into an enclosing stacking sequence can change a pixel.
///
/// A visible node pays one enum test.
#[inline]
pub fn subtree_shows_nothing(tree: &NodeTree, node: &Node) -> bool {
    !node.is_text()
        && matches!(
            node.computed_style.visibility,
            VisibilityValue::Hidden | VisibilityValue::Collapse
        )
        && CULL_HIDDEN.get()
        && !shows_below(tree, node.id)
}
