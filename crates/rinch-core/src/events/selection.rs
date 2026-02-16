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
    static SELECTION_CALLBACK: RefCell<Option<SelectionCallback>> = RefCell::new(None);
    static SAVED_SELECTION: RefCell<Vec<(usize, usize, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Set the global text selection callback.
/// The window manager sets this to delegate selection operations to BaseDocument.
pub fn set_selection_callback<F>(cb: F)
where
    F: Fn(SelectionAction) -> Vec<(usize, usize, usize)> + 'static,
{
    SELECTION_CALLBACK.with(|s| {
        *s.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the global text selection callback.
pub fn clear_selection_callback() {
    SELECTION_CALLBACK.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Dispatch a selection action to the callback.
pub fn dispatch_selection(action: SelectionAction) -> Vec<(usize, usize, usize)> {
    SELECTION_CALLBACK.with(|s| {
        if let Some(ref cb) = *s.borrow() {
            cb(action)
        } else {
            Vec::new()
        }
    })
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
    static SELECTION_SYNC_CALLBACK: RefCell<Option<SelectionSyncCallback>> = RefCell::new(None);
}

/// Set the callback invoked on mouseup to sync drag selection.
pub fn set_selection_sync_callback<F>(cb: F)
where
    F: Fn(Vec<(usize, usize, usize)>) + 'static,
{
    SELECTION_SYNC_CALLBACK.with(|s| {
        *s.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the selection sync callback.
pub fn clear_selection_sync_callback() {
    SELECTION_SYNC_CALLBACK.with(|s| {
        *s.borrow_mut() = None;
    });
}

/// Fire the selection sync callback with current LIVE ranges only.
/// Uses dispatch_selection directly instead of query_selection_ranges()
/// to avoid the saved-snapshot fallback (which is only for toolbar commands).
pub fn fire_selection_sync() {
    let ranges = dispatch_selection(SelectionAction::QueryRanges);
    SELECTION_SYNC_CALLBACK.with(|s| {
        if let Some(ref cb) = *s.borrow() {
            cb(ranges);
        }
    });
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
    static PENDING_FOCUS_REQUEST: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Request that a specific element be focused.
/// The runtime will apply this focus before the next event processing cycle.
pub fn request_focus(node_id: usize) {
    PENDING_FOCUS_REQUEST.with(|c| c.set(Some(node_id)));
}

/// Check and consume the pending focus request.
/// Called by the runtime during event processing.
pub fn take_pending_focus_request() -> Option<usize> {
    PENDING_FOCUS_REQUEST.with(|c| c.take())
}
