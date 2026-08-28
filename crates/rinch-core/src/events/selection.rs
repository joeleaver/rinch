//! Text selection callbacks, sync, snapshot, and focus request mechanisms.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ============================================================================
// Text Selection Callback
// ============================================================================

/// Actions for controlling native text selection.
#[derive(Debug, Clone)]
pub enum SelectionAction {
    /// Set selection with anchor and focus (both node ID + byte offset).
    Set {
        anchor_node: usize,
        anchor_offset: usize,
        focus_node: usize,
        focus_offset: usize,
    },
    /// Extend selection to a viewport point (for drag-to-select).
    ExtendToPoint { x: f32, y: f32 },
    /// Clear the selection.
    Clear,
    /// Query the current selection ranges. Returns Vec<(node_id, start_offset, end_offset)>.
    QueryRanges,
}

/// Type alias for the text selection callback.
/// Returns selection ranges for QueryRanges, empty vec otherwise.
pub type SelectionCallback = Rc<dyn Fn(SelectionAction) -> Vec<(usize, usize, usize)>>;

thread_local! {
    static SELECTION_CALLBACK: RefCell<Option<SelectionCallback>> = const { RefCell::new(None) };
    static SAVED_SELECTION: RefCell<Vec<(usize, usize, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Set the global text selection callback, which delegates selection operations
/// to the document that owns the text.
///
/// Nothing in the tree registers one today — the claim that "the window manager
/// sets this" was stale — so this is a public seam rather than a live path.
///
/// **Released on unmount.** Registering from inside a render ties the callback
/// to the ambient scope, so disposing that scope clears it — a callback that
/// captured a `Signal` cannot outlive the signal and read freed state
/// (issue #183). The cleanup only clears the slot if this callback is *still*
/// the one installed, so a later `set_selection_callback` is never clobbered by
/// an earlier component unmounting. Registering outside any render has no owner
/// and so lives for the life of the app, as before.
pub fn set_selection_callback<F>(cb: F)
where
    F: Fn(SelectionAction) -> Vec<(usize, usize, usize)> + 'static,
{
    crate::reactive::install_scoped_slot(&SELECTION_CALLBACK, Rc::new(cb));
}

/// Clear the global text selection callback.
pub fn clear_selection_callback() {
    crate::reactive::clear_scoped_slot(&SELECTION_CALLBACK);
}

/// Dispatch a selection action to the callback.
///
/// The `Rc` is cloned out before the call so the callback may re-enter (install
/// a different callback, query the selection again) without a double borrow.
pub fn dispatch_selection(action: SelectionAction) -> Vec<(usize, usize, usize)> {
    match crate::reactive::read_scoped_slot(&SELECTION_CALLBACK) {
        Some(cb) => cb(action),
        None => Vec::new(),
    }
}

/// Save the current selection ranges as a snapshot.
/// Called before PointerDown dispatch, which may clear selection.
pub fn save_selection_snapshot() {
    let ranges = dispatch_selection(SelectionAction::QueryRanges);
    SAVED_SELECTION.with(|s| {
        *s.borrow_mut() = ranges;
    });
}

/// Clear the saved selection snapshot.
pub fn clear_selection_snapshot() {
    SAVED_SELECTION.with(|s| {
        s.borrow_mut().clear();
    });
}

/// Get the saved selection snapshot.
pub fn get_saved_selection() -> Vec<(usize, usize, usize)> {
    SAVED_SELECTION.with(|s| s.borrow().clone())
}

/// Query the current text selection ranges.
/// Returns Vec<(node_id, start_byte_offset, end_byte_offset)>.
/// Falls back to the saved snapshot if the live selection is empty
/// (e.g., the runtime cleared it on mousedown before a toolbar command runs).
pub fn query_selection_ranges() -> Vec<(usize, usize, usize)> {
    let live = dispatch_selection(SelectionAction::QueryRanges);
    if !live.is_empty() {
        return live;
    }
    // Fall back to saved snapshot (before the runtime cleared it)
    get_saved_selection()
}

// ============================================================================
// Selection Sync Callback (mouseup drag-to-select -> editor)
// ============================================================================

/// Callback invoked on mouseup to notify that drag selection may have changed.
/// The callback receives the current selection ranges (block_index, start, end).
pub type SelectionSyncCallback = Rc<dyn Fn(Vec<(usize, usize, usize)>)>;

thread_local! {
    static SELECTION_SYNC_CALLBACK: RefCell<Option<SelectionSyncCallback>> = const { RefCell::new(None) };
}

/// Set the callback invoked on mouseup to sync drag selection.
///
/// **Released on unmount**, on the same terms as
/// [`set_selection_callback`] (issue #183).
pub fn set_selection_sync_callback<F>(cb: F)
where
    F: Fn(Vec<(usize, usize, usize)>) + 'static,
{
    crate::reactive::install_scoped_slot(&SELECTION_SYNC_CALLBACK, Rc::new(cb));
}

/// Clear the selection sync callback.
pub fn clear_selection_sync_callback() {
    crate::reactive::clear_scoped_slot(&SELECTION_SYNC_CALLBACK);
}

/// Fire the selection sync callback with current LIVE ranges only.
/// Uses dispatch_selection directly instead of query_selection_ranges()
/// to avoid the saved-snapshot fallback (which is only for toolbar commands).
///
/// The `Rc` is cloned out before the call so the callback may re-enter.
pub fn fire_selection_sync() {
    let ranges = dispatch_selection(SelectionAction::QueryRanges);
    if let Some(cb) = crate::reactive::read_scoped_slot(&SELECTION_SYNC_CALLBACK) {
        cb(ranges);
    }
}

// --- Deferred selection clear ---
// When the DOM is rebuilt (e.g., editor re-render via Effect), the text selection
// may reference nodes that no longer exist. The Effect cannot call dispatch_selection(Clear)
// directly because the DOM doc may be mutably borrowed by the event handler that triggered
// the Effect. Instead, the Effect sets this flag, and the window manager clears the selection
// in redraw() before paint, when no borrows are active.

thread_local! {
    static PENDING_SELECTION_CLEAR: Cell<bool> = const { Cell::new(false) };
}

/// Request that the text selection be cleared before the next paint.
/// Safe to call from Effects (does not borrow the DOM doc).
pub fn request_selection_clear() {
    PENDING_SELECTION_CLEAR.with(|c| c.set(true));
}

/// Check and consume the pending selection clear flag.
/// Called by the window manager in redraw() before paint.
pub fn take_pending_selection_clear() -> bool {
    PENDING_SELECTION_CLEAR.with(|c| {
        if c.get() {
            c.set(false);
            true
        } else {
            false
        }
    })
}

// --- Focus request mechanism ---
// Allows the document/editor to request that a specific element be focused.
// The runtime checks for and applies focus requests during event processing.

thread_local! {
    /// `(doc_key, node_id)` — the document key scopes the request so a runtime
    /// driving one document never consumes (and misapplies) a focus request
    /// posted by another document on the same thread (issue #134).
    static PENDING_FOCUS_REQUEST: Cell<Option<(u64, usize)>> = const { Cell::new(None) };
}

/// Request that a specific element be focused, identified by its document's
/// [`doc_key`](crate::dom::DomDocument::doc_key) and node id.
/// The runtime will apply this focus before the next event processing cycle.
pub fn request_focus(doc_key: u64, node_id: usize) {
    PENDING_FOCUS_REQUEST.with(|c| c.set(Some((doc_key, node_id))));
}

/// Consume the pending focus request **if it targets the given document**.
/// Called by the runtime during event processing with its own document's key;
/// a request posted by a different document is left in place for that
/// document's runtime to pick up.
pub fn take_pending_focus_request(doc_key: u64) -> Option<usize> {
    PENDING_FOCUS_REQUEST.with(|c| match c.get() {
        Some((key, node_id)) if key == doc_key => {
            c.set(None);
            Some(node_id)
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::reactive::{Scope, Signal};

    fn one_range() -> Vec<(usize, usize, usize)> {
        vec![(1, 2, 3)]
    }

    /// #183: a selection callback that captured its component's state must not
    /// survive the component.
    #[test]
    fn a_selection_callback_registered_in_a_scope_is_released_when_the_scope_disposes() {
        clear_selection_callback();
        let scope = Scope::new();
        scope.run(|| set_selection_callback(|_| one_range()));
        assert_eq!(
            dispatch_selection(SelectionAction::QueryRanges),
            one_range(),
            "the callback is live while its scope is"
        );

        scope.dispose();
        assert!(
            dispatch_selection(SelectionAction::QueryRanges).is_empty(),
            "disposing the owning scope must release the selection callback"
        );
    }

    #[test]
    fn a_selection_sync_callback_registered_in_a_scope_is_released_when_the_scope_disposes() {
        clear_selection_callback();
        clear_selection_sync_callback();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = Scope::new();
        scope.run(move || {
            set_selection_sync_callback(move |_| flag.set(true));
        });
        fire_selection_sync();
        assert!(ran.get(), "the sync callback is live while its scope is");
        ran.set(false);

        scope.dispose();
        fire_selection_sync();
        assert!(
            !ran.get(),
            "disposing the owning scope must release the sync callback"
        );
    }

    /// An earlier component unmounting must not clear a *later* component's
    /// callback — the cleanup only reclaims the slot it still owns.
    #[test]
    fn an_earlier_scopes_cleanup_does_not_clobber_a_later_selection_callback() {
        clear_selection_callback();
        clear_selection_sync_callback();

        let first = Scope::new();
        first.run(|| {
            set_selection_callback(|_| Vec::new());
            set_selection_sync_callback(|_| {});
        });

        let synced = Rc::new(Cell::new(false));
        let flag = synced.clone();
        let second = Scope::new();
        second.run(move || {
            set_selection_callback(|_| one_range());
            set_selection_sync_callback(move |_| flag.set(true));
        });

        first.dispose();
        assert_eq!(
            dispatch_selection(SelectionAction::QueryRanges),
            one_range(),
            "the second callback must survive the first scope's disposal"
        );
        fire_selection_sync();
        assert!(synced.get(), "and so must the second sync callback");

        second.dispose();
        assert!(dispatch_selection(SelectionAction::QueryRanges).is_empty());
    }

    /// Registering outside any render has no owner, so nothing releases it.
    #[test]
    fn a_selection_callback_registered_with_no_ambient_owner_lives_on() {
        clear_selection_callback();
        set_selection_callback(|_| one_range());
        Scope::new().dispose();
        assert_eq!(
            dispatch_selection(SelectionAction::QueryRanges),
            one_range(),
            "an ownerless callback keeps app lifetime"
        );
        clear_selection_callback();
    }

    /// The #183 shape for this registry: the callback reads a signal its
    /// component owned, and disposal freed it.
    #[test]
    fn a_released_selection_callback_does_not_read_its_components_freed_signal() {
        clear_selection_callback();
        let scope = Scope::new();
        let node = scope.run(|| {
            let node = Signal::new(1usize);
            set_selection_callback(move |_| vec![(node.get(), 2, 3)]);
            node
        });
        assert_eq!(
            dispatch_selection(SelectionAction::QueryRanges),
            one_range()
        );

        scope.dispose();
        assert!(!node.is_alive(), "disposal freed the component's signal");
        assert!(
            dispatch_selection(SelectionAction::QueryRanges).is_empty(),
            "a released callback must not read its component's freed state"
        );
    }

    /// The dispatch must not hold the slot's borrow across user code.
    #[test]
    fn a_selection_callback_may_reenter_from_inside_dispatch() {
        clear_selection_callback();
        set_selection_callback(|_| {
            set_selection_callback(|_| Vec::new());
            one_range()
        });

        assert_eq!(
            dispatch_selection(SelectionAction::QueryRanges),
            one_range()
        );
        assert!(
            dispatch_selection(SelectionAction::QueryRanges).is_empty(),
            "the replacement installed from inside dispatch is now live"
        );
        clear_selection_callback();
    }

    /// Same for the sync slot.
    #[test]
    fn a_selection_sync_callback_may_reenter_from_inside_dispatch() {
        clear_selection_callback();
        clear_selection_sync_callback();
        let replaced = Rc::new(Cell::new(false));
        let flag = replaced.clone();
        set_selection_sync_callback(move |_| {
            let flag = flag.clone();
            set_selection_sync_callback(move |_| flag.set(true));
        });

        fire_selection_sync();
        assert!(!replaced.get());
        fire_selection_sync();
        assert!(replaced.get(), "the replacement ran on the next fire");
        clear_selection_sync_callback();
    }
}
