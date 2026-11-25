//! [`DomScroll`] for [`WebDocument`]: scroll offsets, extents and requests,
//! and the page scroll lock.

use super::*;

use rinch_core::dom::{DomScroll, NodeId};
use wasm_bindgen::JsCast;

impl DomScroll for WebDocument {
    fn set_scroll_top(&mut self, node: NodeId, scroll_top: f64) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            el.set_scroll_top(scroll_px(scroll_top));
        }
    }

    fn scroll_top(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_top() as f64)
            .unwrap_or(0.0)
    }

    fn scroll_left(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_left() as f64)
            .unwrap_or(0.0)
    }

    fn set_scroll_left(&mut self, node: NodeId, scroll_left: f64) {
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            el.set_scroll_left(scroll_px(scroll_left));
        }
    }

    /// `element.scrollTo({left, top, behavior})`: the browser animates or
    /// jumps, as it does for the page's own scrolls (#2291).
    fn scroll_to_with(
        &mut self,
        node: NodeId,
        left: Option<f64>,
        top: Option<f64>,
        behavior: rinch_core::dom::ScrollBehavior,
    ) {
        use rinch_core::dom::ScrollBehavior;
        if let Some(n) = self.nodes.get(&node.0)
            && let Ok(el) = n.clone().dyn_into::<web_sys::Element>()
        {
            let opts = web_sys::ScrollToOptions::new();
            if let Some(left) = left {
                opts.set_left(left);
            }
            if let Some(top) = top {
                opts.set_top(top);
            }
            opts.set_behavior(match behavior {
                ScrollBehavior::Auto => web_sys::ScrollBehavior::Auto,
                ScrollBehavior::Smooth => web_sys::ScrollBehavior::Smooth,
                ScrollBehavior::Instant => web_sys::ScrollBehavior::Instant,
            });
            el.scroll_to_with_scroll_to_options(&opts);
        }
    }

    fn scroll_height(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_height() as f64)
            .unwrap_or(0.0)
    }

    fn scroll_width(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.scroll_width() as f64)
            .unwrap_or(0.0)
    }

    fn client_height(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.client_height() as f64)
            .unwrap_or(0.0)
    }

    fn client_width(&self, node: NodeId) -> f64 {
        self.nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
            .map(|el| el.client_width() as f64)
            .unwrap_or(0.0)
    }

    /// A computed `overflow-x`/`overflow-y` of `auto`/`scroll` with content
    /// overflowing the box, not locked out by a `lock_scroll` overlay
    /// (`scroll_locked_out`), and room left toward the delta, the offset
    /// read unrounded. The far end is the browser's own:
    ///
    /// - Firefox says where it is (`scrollTopMax` / `scrollLeftMax`,
    ///   fractional), so its end is that, to a twentieth of a pixel.
    /// - Chrome has no such property, and its rounded sizes put the range's
    ///   end up to one device pixel past where the offset stops (a 200.4px
    ///   list at 1.5 stops at 599.33 where `scrollHeight - clientHeight` is
    ///   600). So the slack there is one device pixel (and a hair) above a
    ///   device scale of 1, half a pixel at 1 or below, where the end is
    ///   whole (measured, Chrome 153), so a list one pixel short of it still
    ///   has room.
    ///
    /// An RTL box's `scrollLeft` runs from `-max` to 0.
    fn can_scroll_toward(&self, node: NodeId, dx: f64, dy: f64) -> bool {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return false;
        };
        let overflow_y = dy != 0.0 && el.scroll_height() > el.client_height();
        let overflow_x = dx != 0.0 && el.scroll_width() > el.client_width();
        if !(overflow_x || overflow_y) || scroll_locked_out(&el) {
            return false;
        }
        let Some(win) = web_sys::window() else {
            return false;
        };
        let Some(style) = win.get_computed_style(&el).ok().flatten() else {
            return false;
        };
        let scrolls = |prop: &str| {
            matches!(
                style.get_property_value(prop).unwrap_or_default().as_str(),
                "auto" | "scroll"
            )
        };
        let number = |prop: &str| {
            js_sys::Reflect::get(&el, &prop.into())
                .ok()
                .and_then(|v| v.as_f64())
        };
        let dpr = win.device_pixel_ratio();
        let rounded_slack = if dpr > 1.0 { 1.0 / dpr + 0.01 } else { 0.5 };
        // (offset, low end, high end, slack): Firefox's own end when it
        // gives one (`scrollTopMax`, `scrollLeftMax`).
        let range = |axis: &str, span: i32, rtl: bool, fallback: i32| {
            let at = number(&format!("scroll{axis}")).unwrap_or(f64::from(fallback));
            let lo = if rtl { -f64::from(span) } else { 0.0 };
            match number(&format!("scroll{axis}Max")) {
                // An RTL box's range ends at 0, which Firefox's
                // `scrollLeftMax` says too.
                Some(max) => (at, lo, max, 0.05),
                None if rtl => (at, lo, 0.0, rounded_slack),
                None => (at, lo, f64::from(span), rounded_slack),
            }
        };
        let room = |(at, lo, hi, slack): (f64, f64, f64, f64), d: f64| {
            (d < 0.0 && at > lo + slack) || (d > 0.0 && at < hi - slack)
        };
        let vertical = overflow_y
            && scrolls("overflow-y")
            && room(
                range(
                    "Top",
                    el.scroll_height() - el.client_height(),
                    false,
                    el.scroll_top(),
                ),
                dy,
            );
        let horizontal = overflow_x
            && scrolls("overflow-x")
            && room(
                range(
                    "Left",
                    el.scroll_width() - el.client_width(),
                    style.get_property_value("direction").unwrap_or_default() == "rtl",
                    el.scroll_left(),
                ),
                dx,
            );
        vertical || horizontal
    }

    /// Scroll `node` into view **now**, rather than queueing the request the way
    /// the desktop backend does. The trait defers because Taffy layout has to be
    /// resolved before an element's position relative to its scroll container is
    /// known; in the browser layout is synchronous, so by the time a caller holds
    /// a node its box is already measurable and `scrollIntoView` reads it itself.
    /// There is correspondingly nothing for `drain_scroll_into_view_requests` to
    /// return — the default (empty) is right here.
    ///
    /// `block`/`inline: "nearest"` is the minimal scroll: an element already in
    /// view does not move, and one out of view is brought just inside the edge it
    /// left by. Centring would jump the page on every caret move. The behaviour is
    /// left at the default (`auto`, i.e. instant) deliberately — `smooth` animates,
    /// and a caret that keeps typing would chase a still-moving viewport.
    fn request_scroll_into_view(&mut self, node: NodeId) {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
        opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
        el.scroll_into_view_with_scroll_into_view_options(&opts);
    }

    /// Place `node` in its scroll container **now** (see
    /// [`Self::request_scroll_into_view`] for why the browser needs no queue):
    /// the nearest ancestor whose `overflow-y` scrolls, or the page's scrolling
    /// element when none does, gets the `scrollTop` that puts the node's top
    /// `fraction` of the way down its visible height (at least `margin` inside
    /// either edge), clamped to `0..=scrollHeight - clientHeight`. The node then
    /// gets the same "nearest" `scrollIntoView` a caret gets, which leaves that
    /// container alone (the node is inside it now) and brings it into the view
    /// of every scroller further out, the page included.
    fn request_scroll_to_fraction(&mut self, node: NodeId, fraction: f32, margin: f32) {
        let Some(el) = self
            .nodes
            .get(&node.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let mut scroller = None;
        let mut cur = el.parent_element();
        while let Some(ancestor) = cur {
            let scrolls = window
                .get_computed_style(&ancestor)
                .ok()
                .flatten()
                .and_then(|cs| cs.get_property_value("overflow-y").ok())
                .is_some_and(|o| o == "auto" || o == "scroll");
            if scrolls {
                scroller = Some(ancestor);
                break;
            }
            cur = ancestor.parent_element();
        }
        let page = scroller.is_none();
        let Some(scroller) =
            scroller.or_else(|| window.document().and_then(|d| d.scrolling_element()))
        else {
            return;
        };
        let rect = el.get_bounding_client_rect();
        // The top of the scroller's visible area in client px: the page's is
        // the viewport's top; an element's is its padding box's top.
        let view_top = if page {
            0.0
        } else {
            scroller.get_bounding_client_rect().top() + f64::from(scroller.client_top())
        };
        let visible = f64::from(scroller.client_height());
        let (fraction, margin) = (f64::from(fraction), f64::from(margin));
        let at = (fraction * visible)
            .min(visible - margin - rect.height())
            .max(margin);
        let current = f64::from(scroller.scroll_top());
        let max = f64::from(scroller.scroll_height() - scroller.client_height()).max(0.0);
        let target = (current + (rect.top() - view_top) - at).clamp(0.0, max);
        scroller.set_scroll_top(target.round() as i32);
        if !page {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    }

    /// The browser owns the wheel, so `root` gates nothing the user does; it is
    /// recorded for rinch's own programmatic scrolling (a drag's auto-scroll,
    /// `scroll_locked_out`). See `set_page_scroll_locked`.
    fn set_scroll_locked(&mut self, locked: bool, root: NodeId) {
        let el = self
            .nodes
            .get(&root.0)
            .and_then(|n| n.clone().dyn_into::<web_sys::Element>().ok());
        set_page_scroll_locked(locked, root.0, el);
    }
}
