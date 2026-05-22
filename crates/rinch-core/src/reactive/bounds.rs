//! Element-bounds signals: expose a node's computed pixel rect as a reactive
//! `Signal`, updated by the runtime after each layout pass.
//!
//! This is the lower-level primitive the timeline-component request (#30) was
//! ultimately asking for: a way to read "how wide is this element right now,
//! reactively?" Once available, app code can compose zoom + scroll + viewport
//! signals to drive arbitrary domain-coordinate layouts in userland.

use std::cell::RefCell;

use super::Signal;

/// Bounding box of a registered element, in absolute viewport-relative
/// logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ElementBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

struct BoundsEntry {
    node_id: u64,
    signal: Signal<ElementBounds>,
}

thread_local! {
    static BOUNDS_REGISTRY: RefCell<Vec<BoundsEntry>> = const { RefCell::new(Vec::new()) };
}

/// Register a reactive `Signal<ElementBounds>` tracking the node's current rect.
///
/// The runtime calls [`update_bounds_signals`] after each layout pass to refresh
/// the value. Subscribers only re-run when the bounds actually change
/// (uses [`Signal::set_if_changed`] internally).
///
/// The Signal's initial value is `ElementBounds::default()` (zero rect). The
/// first real bounds arrive after the next layout pass.
///
/// Most users should prefer [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal),
/// which calls this internally with the node's id.
pub fn register_bounds_signal(node_id: u64) -> Signal<ElementBounds> {
    let signal = Signal::new(ElementBounds::default());
    BOUNDS_REGISTRY.with(|reg| {
        reg.borrow_mut().push(BoundsEntry { node_id, signal });
    });
    signal
}

/// Refresh all registered bounds signals from the supplied absolute-bounds
/// lookup. Called by the rinch runtime after layout completes.
///
/// The lookup returns `None` for nodes that no longer exist; their signals
/// retain whatever value they last had (no panic, no removal — the registry
/// currently never shrinks; callers must accept the memory cost as the price
/// of the simple API).
pub fn update_bounds_signals<F>(mut absolute_bounds: F)
where
    F: FnMut(u64) -> Option<(f32, f32, f32, f32)>,
{
    BOUNDS_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        for entry in reg.iter() {
            if let Some((x, y, width, height)) = absolute_bounds(entry.node_id) {
                entry.signal.set_if_changed(ElementBounds {
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_zero_initially() {
        let signal = register_bounds_signal(99_001);
        assert_eq!(signal.get(), ElementBounds::default());
    }

    #[test]
    fn update_propagates_when_changed() {
        let id = 99_002;
        let signal = register_bounds_signal(id);
        assert_eq!(signal.get().width, 0.0);

        update_bounds_signals(|qid| {
            if qid == id {
                Some((10.0, 20.0, 300.0, 50.0))
            } else {
                None
            }
        });
        assert_eq!(
            signal.get(),
            ElementBounds {
                x: 10.0,
                y: 20.0,
                width: 300.0,
                height: 50.0
            }
        );

        // Same bounds → no change, no notify
        update_bounds_signals(|qid| {
            if qid == id {
                Some((10.0, 20.0, 300.0, 50.0))
            } else {
                None
            }
        });
        assert_eq!(signal.get().width, 300.0);

        // New bounds → updates
        update_bounds_signals(|qid| {
            if qid == id {
                Some((10.0, 20.0, 400.0, 50.0))
            } else {
                None
            }
        });
        assert_eq!(signal.get().width, 400.0);
    }

    #[test]
    fn missing_node_leaves_signal_alone() {
        let id = 99_003;
        let signal = register_bounds_signal(id);
        update_bounds_signals(|qid| {
            if qid == id {
                Some((1.0, 2.0, 3.0, 4.0))
            } else {
                None
            }
        });
        assert_eq!(signal.get().width, 3.0);
        // Node was removed mid-frame — return None, value persists.
        update_bounds_signals(|_| None);
        assert_eq!(signal.get().width, 3.0);
    }
}
