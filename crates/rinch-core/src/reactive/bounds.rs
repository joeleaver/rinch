//! Element-bounds signals: expose a node's computed pixel rect as a reactive
//! `Signal`, updated by the runtime after each layout pass.
//!
//! This is the lower-level primitive the timeline-component request (#30) was
//! ultimately asking for: a way to read "how wide is this element right now,
//! reactively?" Once available, app code can compose zoom + scroll + viewport
//! signals to drive arbitrary domain-coordinate layouts in userland.
//!
//! The runtime refreshes these after *every* layout pass — including pure
//! window resizes (#145) — so the root element's `bounds_signal()` doubles as
//! the supported way to observe viewport size changes without polling.

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

/// All fields are `Copy` (`Signal` is an index + generation), so
/// [`update_bounds_signals`] can snapshot the entries it needs and release the
/// registry borrow before running any user code.
#[derive(Clone, Copy)]
struct BoundsEntry {
    /// The owning document's [`doc_key`](crate::dom::DomDocument::doc_key).
    /// Node ids are per-document slab indices, so without this key two
    /// documents on one thread (two embedded `RinchContext`s, issue #134) would
    /// stomp each other's same-id entries on every layout pass.
    doc_key: u64,
    node_id: u64,
    signal: Signal<ElementBounds>,
}

thread_local! {
    static BOUNDS_REGISTRY: RefCell<Vec<BoundsEntry>> = const { RefCell::new(Vec::new()) };
}

/// Register a reactive `Signal<ElementBounds>` tracking the node's current rect,
/// scoped to the document identified by `doc_key`.
///
/// The runtime calls [`update_bounds_signals`] after each layout pass to refresh
/// the value. Subscribers only re-run when the bounds actually change
/// (uses [`Signal::set_if_changed`] internally).
///
/// The Signal's initial value is `ElementBounds::default()` (zero rect). The
/// first real bounds arrive after the next layout pass.
///
/// Most users should prefer [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal),
/// which calls this internally with the node's document key and id.
pub fn register_bounds_signal(doc_key: u64, node_id: u64) -> Signal<ElementBounds> {
    let signal = Signal::new(ElementBounds::default());
    BOUNDS_REGISTRY.with(|reg| {
        reg.borrow_mut().push(BoundsEntry {
            doc_key,
            node_id,
            signal,
        });
    });
    signal
}

/// Refresh the registered bounds signals belonging to the document identified
/// by `doc_key` from the supplied absolute-bounds lookup. Called by the rinch
/// runtime after that document's layout completes. Entries registered by other
/// documents are not *updated* — their own runtime pass does that.
///
/// The lookup returns `None` for nodes that no longer exist; their signals
/// retain whatever value they last had (no panic, no removal — a node can be
/// absent for a frame and come back).
///
/// # Lifetime
///
/// A bounds entry lives exactly as long as its signal. Entries whose signal has
/// been freed are dropped here, on any document's pass — a dead signal can never
/// be updated by anyone, so there is nothing to preserve. Registering from a
/// component therefore ties the entry to that component's scope, while
/// [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal) on a
/// long-lived root keeps application lifetime (issue #141, SD3).
///
/// # Re-entrancy
///
/// `absolute_bounds` and the resulting signal writes run with **no registry
/// borrow held**, so a lookup — or an effect woken by one of the writes — may
/// call [`register_bounds_signal`] or `update_bounds_signals` re-entrantly.
///
/// The writes flush effects **synchronously**, so a caller must also not be
/// holding any lock those effects need. In particular a runtime must release its
/// document borrow first: a reactive `style:` closure reading a measured width —
/// the idiom [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal)
/// documents — patches the DOM and takes `borrow_mut`. Measure everything under
/// the read borrow using [`registered_bounds_nodes`], drop it, then publish here.
pub fn update_bounds_signals<F>(doc_key: u64, mut absolute_bounds: F)
where
    F: FnMut(u64) -> Option<(f32, f32, f32, f32)>,
{
    // Snapshot this document's entries (all fields are Copy) and drop the
    // borrow before invoking the lookup or touching any signal.
    let entries: Vec<BoundsEntry> = BOUNDS_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .filter(|e| e.doc_key == doc_key)
            .copied()
            .collect()
    });

    for entry in entries {
        if !entry.signal.is_alive() {
            continue;
        }
        if let Some((x, y, width, height)) = absolute_bounds(entry.node_id) {
            entry.signal.set_if_changed(ElementBounds {
                x,
                y,
                width,
                height,
            });
        }
    }

    // Reap dead entries across *all* documents, not just `doc_key`'s. A dead
    // signal can never be updated by anyone, and scoping the reap to the
    // updating document would strand the entries of a document that stops
    // running layout entirely — a dropped `RinchContext` (issue #134) is
    // exactly that, and stranding is the leak this is meant to fix.
    //
    // One unconditional pass: `retain` over an all-live registry moves nothing,
    // and `is_alive` is an index + generation compare, so this is cheaper than
    // scanning for deadness first and retaining only on a hit.
    BOUNDS_REGISTRY.with(|reg| reg.borrow_mut().retain(|e| e.signal.is_alive()));
}

/// The node ids currently registered for `doc_key`, with live signals.
///
/// Lets a runtime compute every rect it needs while holding its document
/// borrow, then **release that borrow** before calling
/// [`update_bounds_signals`] — whose writes flush effects synchronously, and
/// those effects mutate the DOM. Without this two-step the document `RefCell`
/// is still borrowed when user code runs, and the documented
/// [`NodeHandle::bounds_signal`](crate::dom::NodeHandle::bounds_signal) idiom
/// (a reactive `style:` closure reading a measured width) is a `BorrowMutError`.
pub fn registered_bounds_nodes(doc_key: u64) -> Vec<u64> {
    BOUNDS_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .filter(|e| e.doc_key == doc_key && e.signal.is_alive())
            .map(|e| e.node_id)
            .collect()
    })
}

/// Number of registered bounds entries on this thread, across all documents.
/// Test-only: the self-pruning contract is about entries *disappearing*.
#[cfg(test)]
pub(crate) fn registry_len_for_tests() -> usize {
    BOUNDS_REGISTRY.with(|reg| reg.borrow().len())
}

/// The `(doc_key, node_id)` pairs currently registered. Test-only: pruning
/// assertions must check *which* entry survived, not merely how many did.
#[cfg(test)]
pub(crate) fn registry_entries_for_tests() -> Vec<(u64, u64)> {
    BOUNDS_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .map(|e| (e.doc_key, e.node_id))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_zero_initially() {
        let signal = register_bounds_signal(1, 99_001);
        assert_eq!(signal.get(), ElementBounds::default());
    }

    #[test]
    fn update_propagates_when_changed() {
        let id = 99_002;
        let signal = register_bounds_signal(1, id);
        assert_eq!(signal.get().width, 0.0);

        update_bounds_signals(1, |qid| {
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
        update_bounds_signals(1, |qid| {
            if qid == id {
                Some((10.0, 20.0, 300.0, 50.0))
            } else {
                None
            }
        });
        assert_eq!(signal.get().width, 300.0);

        // New bounds → updates
        update_bounds_signals(1, |qid| {
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
        let signal = register_bounds_signal(1, id);
        update_bounds_signals(1, |qid| {
            if qid == id {
                Some((1.0, 2.0, 3.0, 4.0))
            } else {
                None
            }
        });
        assert_eq!(signal.get().width, 3.0);
        // Node was removed mid-frame — return None, value persists.
        update_bounds_signals(1, |_| None);
        assert_eq!(signal.get().width, 3.0);
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use crate::reactive::Effect;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Answer every lookup with the same rect.
    fn any_rect(_: u64) -> Option<(f32, f32, f32, f32)> {
        Some((1.0, 2.0, 3.0, 4.0))
    }

    #[test]
    fn an_entry_is_dropped_once_its_signal_is_freed() {
        let doomed = register_bounds_signal(7, 1);
        let survivor = register_bounds_signal(7, 2);
        assert_eq!(registry_len_for_tests(), 2);

        doomed.free_for_tests();
        update_bounds_signals(7, any_rect);

        // Assert *which* entry survived, not merely how many — an inverted
        // retain predicate keeps the count right and the contents wrong.
        assert_eq!(
            registry_entries_for_tests(),
            vec![(7, 2)],
            "the dead entry is reaped and the live one is kept"
        );
        assert_eq!(survivor.get().width, 3.0, "the survivor still updates");
    }

    #[test]
    fn a_freed_entry_is_reaped_by_another_documents_pass() {
        // A document that stops running layout must not strand its entries.
        let orphan = register_bounds_signal(100, 1);
        let _other_doc = register_bounds_signal(200, 1);
        assert_eq!(registry_len_for_tests(), 2);

        orphan.free_for_tests();
        // Document 100 never runs again; document 200 does.
        update_bounds_signals(200, any_rect);

        assert_eq!(
            registry_entries_for_tests(),
            vec![(200, 1)],
            "document 100's stranded entry is reaped by document 200's pass"
        );
    }

    #[test]
    fn registered_bounds_nodes_lists_only_this_documents_live_entries() {
        let a = register_bounds_signal(31, 10);
        let _b = register_bounds_signal(31, 11);
        let _other = register_bounds_signal(32, 12);

        assert_eq!(registered_bounds_nodes(31), vec![10, 11]);
        assert_eq!(registered_bounds_nodes(32), vec![12]);

        a.free_for_tests();
        assert_eq!(
            registered_bounds_nodes(31),
            vec![11],
            "a freed entry must not be handed to the runtime to measure"
        );
    }

    #[test]
    fn a_lookup_may_register_another_bounds_signal() {
        // Pre-fix this was a BorrowMutError: `update_bounds_signals` held
        // `BOUNDS_REGISTRY.borrow()` across the lookup, and
        // `register_bounds_signal` takes `borrow_mut()` to push.
        let _existing = register_bounds_signal(11, 1);
        let registered = Rc::new(Cell::new(false));

        let r = Rc::clone(&registered);
        update_bounds_signals(11, move |_| {
            if !r.get() {
                r.set(true);
                let _nested = register_bounds_signal(11, 2);
            }
            Some((0.0, 0.0, 10.0, 10.0))
        });

        assert!(registered.get());
        assert_eq!(registry_len_for_tests(), 2);
    }

    #[test]
    fn a_bounds_driven_effect_may_read_a_bounds_signal() {
        // The idiom this module's own docs recommend: observe an element's rect
        // and, from that effect, register/read another. The write below flushes
        // effects synchronously, so pre-fix the effect ran under the registry
        // borrow and the nested register was a BorrowMutError.
        let observed = register_bounds_signal(21, 1);
        let widths = Rc::new(Cell::new(0.0f32));

        let w = Rc::clone(&widths);
        let _effect = Effect::new(move || {
            let b = observed.get();
            if b.width > 0.0 {
                // Register a second observer from inside the reaction.
                let nested = register_bounds_signal(21, 2);
                w.set(b.width + nested.get().width);
            }
        });

        update_bounds_signals(21, |id| (id == 1).then_some((0.0, 0.0, 40.0, 10.0)));

        assert_eq!(widths.get(), 40.0);
        assert_eq!(registry_len_for_tests(), 2);
    }
}
