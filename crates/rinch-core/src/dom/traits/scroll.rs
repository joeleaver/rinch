//! The [`DomScroll`] supertrait of [`DomDocument`](super::DomDocument):
//! scroll offsets and extents, programmatic scrolling, the scroll requests
//! and clamps the runtime drains after layout, and the page scroll lock.

use crate::dom::NodeId;

/// Scroll positions, extents and requests.
///
/// One of the five supertraits [`DomDocument`](super::DomDocument) is made
/// of; see that trait for how they fit together.
pub trait DomScroll {
    // ── Scroll query API ─────────────────────────────────────────────────

    /// Get the vertical scroll position of an element.
    /// Equivalent to `element.scrollTop` in the web DOM.
    fn scroll_top(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the horizontal scroll position of an element.
    /// Equivalent to `element.scrollLeft` in the web DOM — including its sign:
    /// a `direction: rtl` container is at 0 at the start (its content's right
    /// end showing) and negative toward the left, on every backend.
    fn scroll_left(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Set the scroll position of an element.
    /// For elements with overflow: auto/scroll, this sets the scroll offset.
    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64);

    /// Set the horizontal scroll position of an element.
    /// Equivalent to `element.scrollLeft = value` in the web DOM.
    fn set_scroll_left(&mut self, _node: NodeId, _scroll_left: f64) {}

    /// Scroll an element to `left` / `top` (an axis given `None` stays),
    /// moving as `behavior` says: `element.scrollTo({left, top, behavior})`
    /// in the web DOM (#2291). `Auto` follows the element's computed
    /// `scroll-behavior`, which is what [`Self::set_scroll_top`] and
    /// [`Self::set_scroll_left`] do. The default ignores `behavior`.
    fn scroll_to_with(
        &mut self,
        node: NodeId,
        left: Option<f64>,
        top: Option<f64>,
        behavior: crate::dom::ScrollBehavior,
    ) {
        let _ = behavior;
        if let Some(left) = left {
            self.set_scroll_left(node, left);
        }
        if let Some(top) = top {
            self.set_scroll_top(node, top);
        }
    }

    /// Get the total scrollable content height of an element.
    /// Equivalent to `element.scrollHeight` in the web DOM.
    fn scroll_height(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the total scrollable content width of an element.
    /// Equivalent to `element.scrollWidth` in the web DOM.
    fn scroll_width(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the visible content area height (layout height minus padding and border).
    /// Equivalent to `element.clientHeight` in the web DOM.
    fn client_height(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Get the visible content area width (layout width minus padding and border).
    /// Equivalent to `element.clientWidth` in the web DOM.
    fn client_width(&self, _node: NodeId) -> f64 {
        0.0
    }

    /// Whether a wheel moving `(dx, dy)` would scroll `node` itself: on an
    /// axis whose delta is non-zero, `node` is a scroll container (a computed
    /// `overflow` of `auto` or `scroll` with content overflowing its box, the
    /// rule the wheel follows), no `lock_scroll` overlay locks it out, and its
    /// offset is not yet at the end of its range in that direction. Only the
    /// signs count, in the DOM's sense (positive scrolls down / right). Either
    /// axis answering yes is a yes.
    ///
    /// The default (a document that does not scroll) is `false`.
    fn can_scroll_toward(&self, _node: NodeId, _dx: f64, _dy: f64) -> bool {
        false
    }

    /// Request that the given node be scrolled into view after the next layout.
    ///
    /// The scroll is deferred because layout must be resolved first to know
    /// the element's position relative to its scroll container.
    fn request_scroll_into_view(&mut self, _node: NodeId) {}

    /// Drain all pending scroll-into-view requests.
    ///
    /// Called by the runtime after `resolve_layout()` to apply deferred scrolls.
    fn drain_scroll_into_view_requests(&mut self) -> Vec<NodeId> {
        Vec::new()
    }

    /// Request that `node` be scrolled to a set place in its scroll container:
    /// its top `fraction` of the way down the container's visible height —
    /// its padding box, from the inside of its top border, a browser's
    /// `clientTop` / `clientHeight` (`0.0` the top edge, `1.0` the bottom), but never nearer than `margin`
    /// px to either edge, with the scroll clamped to what the content allows.
    /// Unlike [`Self::request_scroll_into_view`] it moves even when `node` is
    /// already in view. Deferred until after the next layout, like that one.
    ///
    /// The default treats it as a plain [`Self::request_scroll_into_view`], so
    /// a backend that does not implement it still brings the node on screen.
    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        let _ = (fraction, margin);
        self.request_scroll_into_view(node);
    }

    /// Drain the pending [`Self::request_scroll_to_fraction`] requests as
    /// `(node, fraction, margin)`. Called by the runtime right after
    /// [`Self::drain_scroll_into_view_requests`], and applied after those.
    fn drain_scroll_to_fraction_requests(&mut self) -> Vec<(NodeId, f32, f32)> {
        Vec::new()
    }

    /// Drain the scroll offsets clamped during layout, as (node, clamped
    /// offset) pairs — coalesced to one entry per node (last value wins).
    ///
    /// Layout clamps a container's scroll offset when its content shrinks or
    /// its viewport grows. Firing handlers mid-layout would re-enter user code
    /// while the document is borrowed, so the engine queues the clamps and the
    /// runtime drains them after `resolve_layout()` to dispatch the same
    /// scroll events input-driven scrolling produces.
    fn drain_scroll_clamps(&mut self) -> Vec<(NodeId, f64)> {
        Vec::new()
    }

    /// Lock or unlock document-level scrolling on behalf of `root` (issue #474).
    ///
    /// This is what `Modal`/`Drawer`'s `lock_scroll` reaches. An overlay calls it
    /// with `true` when it opens and `false` when it closes or unmounts; the
    /// backend decides what "locked" means, because the two have nothing in
    /// common mechanically:
    ///
    /// - **Desktop** rejects a *scroll gesture* whose container is outside every
    ///   locking root, so the page behind does not move while the overlay's own
    ///   `overflow: auto` body still does. Nothing is restyled.
    /// - **Web** sets `overflow: hidden` on the real `<html>`, which is the only
    ///   mechanism there is — rinch cannot gate the browser's own wheel.
    ///
    /// **`root` is the locking overlay's root node**, not the node to lock. It is
    /// the reason this takes a node at all: desktop needs to know which subtree
    /// is still allowed to scroll, and a bare `bool` cannot say. Web ignores it.
    ///
    /// **Implementations must count, not latch.** Two overlays open and the inner
    /// one closing must leave the page locked, so a lock is held per `root` and
    /// released per `root`. `NodeHandle::set_scroll_locked` is the caller-facing
    /// spelling; `rinch-components`' `overlay_scroll_lock` is the caller.
    ///
    /// Defaulted to a no-op so a backend with no page to lock (and
    /// `MockDomDocument`) keeps compiling.
    fn set_scroll_locked(&mut self, _locked: bool, _root: NodeId) {}
}
