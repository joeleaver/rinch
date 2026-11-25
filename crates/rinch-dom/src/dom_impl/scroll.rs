//! [`DomScroll`] for [`RinchDocument`]: scroll offsets, extents and requests,
//! and the page scroll lock.

use rinch_core::dom::{DomScroll, NodeId};

use super::RinchDocument;

impl DomScroll for RinchDocument {
    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        self.scroll_to_with(
            node,
            None,
            Some(scroll_top),
            rinch_core::dom::ScrollBehavior::Auto,
        );
    }

    /// A scroll write on either axis (#2291). `Auto` follows the box's
    /// computed `scroll-behavior`; a smooth write becomes a request the app
    /// animates ([`crate::NodeTree::request_smooth_scroll`]) and leaves the
    /// offset where it is for now, as `scrollTop` reads in a browser right
    /// after a smooth write. An instant write replaces a pending request.
    fn scroll_to_with(
        &mut self,
        node: NodeId,
        left: Option<f64>,
        top: Option<f64>,
        behavior: rinch_core::dom::ScrollBehavior,
    ) {
        use rinch_core::dom::ScrollBehavior;
        // #723: an id that names no live node is a no-op, as on the web.
        if !self.tree.nodes.contains(node.0) {
            return;
        }
        let smooth = match behavior {
            ScrollBehavior::Smooth => true,
            ScrollBehavior::Instant => false,
            ScrollBehavior::Auto => self.tree.scroll_behavior_smooth(node.0),
        };
        let mut wrote = false;
        let now = self.animation_now_ms();
        for (vertical, value) in [(false, left), (true, top)] {
            let Some(value) = value else { continue };
            if smooth {
                self.tree
                    .request_smooth_scroll(node.0, vertical, value, now);
                continue;
            }
            self.tree
                .smooth_scroll_requests
                .retain(|r| !(r.node == node.0 && r.vertical == vertical));
            if let Some(n) = self.tree.nodes.get_mut(node.0) {
                if vertical {
                    n.scroll_offset.1 = value;
                } else {
                    n.scroll_offset.0 = value;
                }
                wrote = true;
            }
        }
        if wrote {
            // Paint-dirty, and the hit cache's scroll invalidation: the
            // extents inside the scroller survive (#911).
            self.tree.mark_scrolled(node.0);
        }
    }

    /// Desktop's half of `lock_scroll` (#474): record the locking overlay's root
    /// so the wheel and scrollbar paths can refuse a gesture outside it.
    ///
    /// Nothing is restyled. Setting `overflow: hidden` on the body would be the
    /// browser's move, but rinch's scrollbars are overlays with no gutter, so the
    /// only thing it would buy is a reflow — and the body's retained
    /// `scroll_offset` would then be sitting behind a fresh clip. Gating the
    /// gesture leaves the page exactly where it was.
    ///
    /// The unlock removes **one** occurrence, from the back: two overlays holding
    /// locks means two entries, and the inner one closing must leave the outer's
    /// behind. An unlock with no matching entry is ignored rather than panicking
    /// — an overlay's `on_cleanup` releases a lock its `opened_fn` effect may
    /// already have released.
    fn set_scroll_locked(&mut self, locked: bool, root: NodeId) {
        if locked {
            self.tree.scroll_lock_roots.push(root.0);
        } else if let Some(i) = self
            .tree
            .scroll_lock_roots
            .iter()
            .rposition(|n| *n == root.0)
        {
            self.tree.scroll_lock_roots.remove(i);
        }
    }

    // ── Scroll query API ─────────────────────────────────────────────────

    fn scroll_top(&self, node: NodeId) -> f64 {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.scroll_offset.1)
            .unwrap_or(0.0)
    }

    fn scroll_left(&self, node: NodeId) -> f64 {
        self.tree
            .nodes
            .get(node.0)
            .map(|n| n.scroll_offset.0)
            .unwrap_or(0.0)
    }

    fn set_scroll_left(&mut self, node: NodeId, scroll_left: f64) {
        self.scroll_to_with(
            node,
            Some(scroll_left),
            None,
            rinch_core::dom::ScrollBehavior::Auto,
        );
    }

    /// The container's content height, from
    /// [`crate::paint::scrollbar::content_extents`] — the extent the scrollbar
    /// is painted and hit-tested from, so the wheel (which clamps to this) and
    /// the bar cannot disagree about what is content (issue #396). This used to
    /// be a second copy of that walk, without the containing-block rule (#765)
    /// and without the descent through `display: contents` wrappers.
    ///
    /// A box that does not scroll has no bar to agree with: it answers its
    /// descendants' overflow too, as Chrome does
    /// ([`crate::paint::scrollbar::scroll_extents`], #2223).
    fn scroll_height(&self, node: NodeId) -> f64 {
        crate::paint::scrollbar::scroll_extents(&self.tree, node.0).1
    }

    /// The container's content width; see [`Self::scroll_height`].
    fn scroll_width(&self, node: NodeId) -> f64 {
        crate::paint::scrollbar::scroll_extents(&self.tree, node.0).0
    }

    /// The container's own content-box height, from
    /// [`crate::paint::scrollbar::visible_extents`] — the same "visible"
    /// extent `scrollbars()` compares [`Self::scroll_height`] against to
    /// decide whether to paint a bar (issue #769). This used to be a second,
    /// independent copy of that padding/border subtraction, while
    /// `rinch/src/app/hit_testing/scroll.rs`'s scroll-container walks skipped the
    /// subtraction entirely and compared against the border box.
    fn client_height(&self, node: NodeId) -> f64 {
        crate::paint::scrollbar::visible_extents(&self.tree, node.0).1
    }

    /// The container's content-box width; see [`Self::client_height`].
    fn client_width(&self, node: NodeId) -> f64 {
        crate::paint::scrollbar::visible_extents(&self.tree, node.0).0
    }

    /// [`crate::paint::scrollbar::can_scroll_toward`]: the rule the desktop
    /// wheel picks its container by, so the answer is what the default scroll
    /// will do.
    fn can_scroll_toward(&self, node: NodeId, dx: f64, dy: f64) -> bool {
        crate::paint::scrollbar::can_scroll_toward(&self.tree, node.0, dx, dy)
    }

    fn request_scroll_into_view(&mut self, node: NodeId) {
        self.tree.scroll_into_view_requests.push(node.0);
    }

    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        self.tree
            .scroll_to_fraction_requests
            .push((node.0, fraction, margin));
    }

    fn drain_scroll_to_fraction_requests(&mut self) -> Vec<(NodeId, f32, f32)> {
        std::mem::take(&mut self.tree.scroll_to_fraction_requests)
            .into_iter()
            .map(|(n, f, m)| (NodeId(n), f, m))
            .collect()
    }

    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.tree.scroll_into_view_requests)
            .into_iter()
            .map(NodeId)
            .collect()
    }

    fn drain_scroll_clamps(&mut self) -> Vec<(NodeId, f64)> {
        std::mem::take(&mut self.tree.pending_scroll_clamps)
            .into_iter()
            .map(|(id, offset)| (NodeId(id), offset))
            .collect()
    }
}
