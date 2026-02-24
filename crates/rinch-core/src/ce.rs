//! ContentEditable API types.
//!
//! Defines the trait, cursor types, events, and event dispatcher for the
//! ContentEditable component API. Both app.rs (keyboard input) and the
//! editor bridge (formatting commands) call the same API surface. The CE
//! component owns the DOM, performs all mutations, and broadcasts events.

use std::cell::RefCell;
use std::rc::Rc;

// ============================================================================
// Cursor and Selection Types
// ============================================================================

/// A cursor position within a contentEditable DOM tree.
///
/// Points to a specific location in the DOM: a text node + byte offset,
/// or an element node (offset 0 for empty blocks, or offset pointing
/// at a child position like a `<br>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DomCursor {
    /// DOM node ID — either a text node or an element node.
    pub node_id: usize,
    /// Byte offset within the text node's content (0 for element cursors).
    pub offset: usize,
}

impl DomCursor {
    pub fn new(node_id: usize, offset: usize) -> Self {
        Self { node_id, offset }
    }
}

/// Selection state in a contentEditable element.
///
/// A collapsed cursor has `anchor == head`. An extended selection has
/// different anchor and head positions. The anchor is where the user
/// started selecting; the head is where the selection currently ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CeSelection {
    /// The fixed end of the selection (where selection started).
    pub anchor: DomCursor,
    /// The moving end of the selection (current caret position).
    pub head: DomCursor,
}

impl CeSelection {
    /// Create a collapsed selection (cursor, no range).
    pub fn collapsed(cursor: DomCursor) -> Self {
        Self {
            anchor: cursor,
            head: cursor,
        }
    }

    /// Create a range selection.
    pub fn range(anchor: DomCursor, head: DomCursor) -> Self {
        Self { anchor, head }
    }

    /// Whether the selection is collapsed (no range, just a cursor).
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }
}

// ============================================================================
// ContentEditable Events
// ============================================================================

/// Events broadcast after each CE mutation.
///
/// Carry rich semantic info so observers (the editor bridge) know exactly
/// what happened without needing to diff the DOM.
#[derive(Debug, Clone)]
pub enum CeEvent {
    // ── Text ──────────────────────────────────────────────────────────
    /// Text was inserted at a position.
    TextInserted {
        /// The text node that was modified (or created).
        node_id: usize,
        /// Byte offset where the text was inserted.
        offset: usize,
        /// The text that was inserted.
        text: String,
    },

    /// Text was deleted from a node.
    TextDeleted {
        /// The text node that was modified.
        node_id: usize,
        /// Byte offset where deletion started.
        offset: usize,
        /// Number of bytes deleted.
        length: usize,
    },

    /// A text node was created (e.g. first character in empty block).
    TextNodeCreated {
        /// The new text node ID.
        node_id: usize,
        /// Parent element the text node was added to.
        parent_id: usize,
        /// The initial text content.
        text: String,
    },

    /// A node was removed from the DOM.
    NodeRemoved {
        /// The removed node's ID.
        node_id: usize,
        /// The parent it was removed from.
        parent_id: usize,
    },

    // ── Selection ─────────────────────────────────────────────────────
    /// The cursor/selection position changed.
    SelectionChanged {
        /// New selection state.
        selection: CeSelection,
    },

    // ── Block Structure ───────────────────────────────────────────────
    /// A block was split (Enter key).
    BlockSplit {
        /// The original block's DOM node ID.
        original_block_id: usize,
        /// The new block's DOM node ID (created after the split).
        new_block_id: usize,
        /// Byte offset in the original block's text where the split occurred.
        split_offset: usize,
    },

    /// Two blocks were joined (Backspace at block start).
    BlockJoined {
        /// The surviving block's DOM node ID.
        surviving_block_id: usize,
        /// The removed block's DOM node ID (already removed from DOM).
        removed_block_id: usize,
        /// Byte offset in the surviving block where content was merged.
        merge_offset: usize,
    },

    /// A block's tag was changed (e.g. heading -> paragraph).
    BlockTypeChanged {
        /// The old DOM node ID (removed).
        old_node_id: usize,
        /// The new DOM node ID (replacement).
        new_node_id: usize,
        /// The old tag name.
        old_tag: String,
        /// The new tag name.
        new_tag: String,
    },

    // ── Inline Formatting ─────────────────────────────────────────────
    /// Selection was wrapped in a formatting element.
    SelectionWrapped {
        /// The wrapping element tag (e.g. "strong", "em").
        tag: String,
        /// The wrapping DOM node that was created.
        wrapper_node_id: usize,
        /// Text nodes now inside the wrapper.
        wrapped_node_ids: Vec<usize>,
    },

    /// A formatting wrapper was removed from selection.
    SelectionUnwrapped {
        /// The tag that was removed (e.g. "strong", "em").
        tag: String,
        /// Text nodes that were unwrapped.
        unwrapped_node_ids: Vec<usize>,
    },

    // ── List Structure ────────────────────────────────────────────────
    /// A list item was outdented (converted from li to div, or moved up a level).
    ListItemOutdented {
        /// The old list item node ID (removed).
        old_li_id: usize,
        /// The new block node ID (replacement).
        new_block_id: usize,
    },

    /// A block was indented into a list.
    BlockIndented {
        /// The old block node ID.
        old_block_id: usize,
        /// The new list item node ID.
        new_li_id: usize,
        /// The list element ID (ul/ol) the item was added to.
        list_id: usize,
    },

    // ── Table ─────────────────────────────────────────────────────────
    /// A table was inserted.
    TableInserted {
        /// The table block's DOM node ID.
        block_node_id: usize,
        /// Number of rows.
        rows: usize,
        /// Number of columns.
        cols: usize,
    },

    /// A table was deleted.
    TableDeleted {
        /// The removed table block's DOM node ID.
        block_node_id: usize,
    },

    // ── Undo/Redo ─────────────────────────────────────────────────────
    /// An undo operation was applied.
    UndoApplied,

    /// A redo operation was applied.
    RedoApplied,

    // ── Clipboard ─────────────────────────────────────────────────────
    /// HTML content was pasted.
    HtmlPasted {
        /// Nodes created by the paste operation.
        created_node_ids: Vec<usize>,
    },
}

// ============================================================================
// Event Dispatcher
// ============================================================================

/// Callback type for CE event listeners.
pub type CeEventCallback = Rc<dyn Fn(&CeEvent)>;

/// Broadcasts CE events to registered listeners.
///
/// The CE component calls `dispatch()` after each DOM mutation.
/// The editor bridge subscribes to receive events and sync the
/// EditorDocument model.
#[derive(Default)]
pub struct CeEventDispatcher {
    listeners: Vec<CeEventCallback>,
}

impl CeEventDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to CE events. Returns a listener index for unsubscribing.
    pub fn subscribe(&mut self, callback: CeEventCallback) -> usize {
        let idx = self.listeners.len();
        self.listeners.push(callback);
        idx
    }

    /// Dispatch an event to all listeners.
    pub fn dispatch(&self, event: &CeEvent) {
        for listener in &self.listeners {
            listener(event);
        }
    }

    /// Remove all listeners.
    pub fn clear(&mut self) {
        self.listeners.clear();
    }

    /// Number of registered listeners.
    pub fn listener_count(&self) -> usize {
        self.listeners.len()
    }
}

impl std::fmt::Debug for CeEventDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CeEventDispatcher")
            .field("listener_count", &self.listeners.len())
            .finish()
    }
}

// ============================================================================
// ContentEditable API Trait
// ============================================================================

/// The ContentEditable API — single mutation bottleneck for all CE operations.
///
/// app.rs implements this for keyboard input. The editor bridge calls these
/// same methods for formatting commands. Every method performs the DOM
/// mutation AND dispatches a `CeEvent` to notify observers.
pub trait ContentEditableApi {
    // ── Text Operations ──────────────────────────────────────────────

    /// Insert text at the current cursor position.
    fn insert_text(&mut self, text: &str);

    /// Delete the character before the cursor (Backspace).
    fn delete_backward(&mut self);

    /// Delete the character after the cursor (Delete key).
    fn delete_forward(&mut self);

    /// Delete the current selection.
    fn delete_selection(&mut self);

    // ── Block Structure ──────────────────────────────────────────────

    /// Split the current block at the cursor position (Enter key).
    fn split_block(&mut self);

    /// Set the block type of the block at the cursor.
    /// `tag` is the HTML tag name (e.g. "h1", "blockquote", "p").
    fn set_block_type(&mut self, tag: &str);

    // ── Inline Formatting ────────────────────────────────────────────

    /// Wrap the current selection in a formatting element.
    /// `tag` is the element name (e.g. "strong", "em", "u", "s", "code").
    fn wrap_selection(&mut self, tag: &str);

    /// Remove a formatting wrapper from the current selection.
    fn unwrap_selection(&mut self, tag: &str);

    /// Toggle a formatting wrapper on the current selection.
    fn toggle_wrap(&mut self, tag: &str);

    // ── List Operations ──────────────────────────────────────────────

    /// Indent the current block (convert to list item or increase nesting).
    fn indent(&mut self);

    /// Outdent the current block (decrease nesting or convert from list item).
    fn outdent(&mut self);

    // ── Selection ────────────────────────────────────────────────────

    /// Get the current cursor/selection state.
    fn get_selection(&self) -> CeSelection;

    /// Set the cursor/selection state.
    fn set_selection(&mut self, sel: CeSelection);

    // ── Undo/Redo ────────────────────────────────────────────────────

    /// Undo the last operation.
    fn undo(&mut self);

    /// Redo the last undone operation.
    fn redo(&mut self);

    // ── Event Access ─────────────────────────────────────────────────

    /// Get a reference to the event dispatcher for subscribing to events.
    fn event_dispatcher(&self) -> &CeEventDispatcher;

    /// Get a mutable reference to the event dispatcher.
    fn event_dispatcher_mut(&mut self) -> &mut CeEventDispatcher;
}

// ============================================================================
// Thread-local Event Dispatching
// ============================================================================

thread_local! {
    /// Global CE event dispatcher.
    ///
    /// app.rs calls `dispatch_ce_event()` after each DOM mutation.
    /// The editor bridge calls `subscribe_ce_events()` to observe changes.
    static CE_EVENT_DISPATCHER: RefCell<CeEventDispatcher> = RefCell::new(CeEventDispatcher::new());
}

/// Subscribe to CE events globally.
///
/// Called by the editor bridge when mounting. Returns a listener index.
pub fn subscribe_ce_events(callback: CeEventCallback) -> usize {
    CE_EVENT_DISPATCHER.with(|d| d.borrow_mut().subscribe(callback))
}

/// Dispatch a CE event to all global listeners.
///
/// Called by app.rs after each contentEditable DOM mutation.
pub fn dispatch_ce_event(event: &CeEvent) {
    CE_EVENT_DISPATCHER.with(|d| d.borrow().dispatch(event));
}

/// Clear all global CE event listeners.
///
/// Called when the editor bridge unmounts.
pub fn clear_ce_event_listeners() {
    CE_EVENT_DISPATCHER.with(|d| d.borrow_mut().clear());
}

/// Get the number of global CE event listeners (for debugging).
pub fn ce_event_listener_count() -> usize {
    CE_EVENT_DISPATCHER.with(|d| d.borrow().listener_count())
}

// ============================================================================
// Thread-local CE API Access
// ============================================================================

thread_local! {
    /// Thread-local storage for the active CE API instance.
    ///
    /// This allows the bridge to access the CE API without a direct reference,
    /// using the same pattern as the keyboard/click interceptors.
    static ACTIVE_CE_API: RefCell<Option<Rc<RefCell<dyn ContentEditableApi>>>> = RefCell::new(None);
}

/// Set the active CE API instance.
///
/// Called by app.rs when a contentEditable element gains focus.
pub fn set_active_ce_api(api: Rc<RefCell<dyn ContentEditableApi>>) {
    ACTIVE_CE_API.with(|a| {
        *a.borrow_mut() = Some(api);
    });
}

/// Clear the active CE API instance.
///
/// Called when the contentEditable element loses focus.
pub fn clear_active_ce_api() {
    ACTIVE_CE_API.with(|a| {
        *a.borrow_mut() = None;
    });
}

/// Execute a closure with the active CE API, if one is set.
///
/// Returns `Some(result)` if a CE API was available, `None` otherwise.
pub fn with_active_ce_api<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&Rc<RefCell<dyn ContentEditableApi>>) -> R,
{
    ACTIVE_CE_API.with(|a| {
        let borrow = a.borrow();
        borrow.as_ref().map(f)
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dom_cursor_equality() {
        let a = DomCursor::new(1, 5);
        let b = DomCursor::new(1, 5);
        let c = DomCursor::new(1, 6);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn selection_collapsed() {
        let cursor = DomCursor::new(1, 0);
        let sel = CeSelection::collapsed(cursor);
        assert!(sel.is_collapsed());
        assert_eq!(sel.anchor, sel.head);
    }

    #[test]
    fn selection_range() {
        let anchor = DomCursor::new(1, 0);
        let head = DomCursor::new(2, 5);
        let sel = CeSelection::range(anchor, head);
        assert!(!sel.is_collapsed());
    }

    #[test]
    fn event_dispatcher_subscribe_and_dispatch() {
        let mut dispatcher = CeEventDispatcher::new();
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_clone = received.clone();

        dispatcher.subscribe(Rc::new(move |event: &CeEvent| {
            received_clone.borrow_mut().push(format!("{:?}", event));
        }));

        assert_eq!(dispatcher.listener_count(), 1);

        dispatcher.dispatch(&CeEvent::TextInserted {
            node_id: 1,
            offset: 0,
            text: "hello".into(),
        });

        assert_eq!(received.borrow().len(), 1);
        assert!(received.borrow()[0].contains("TextInserted"));
    }

    #[test]
    fn event_dispatcher_multiple_listeners() {
        let mut dispatcher = CeEventDispatcher::new();
        let count = Rc::new(RefCell::new(0));

        let c1 = count.clone();
        dispatcher.subscribe(Rc::new(move |_| *c1.borrow_mut() += 1));

        let c2 = count.clone();
        dispatcher.subscribe(Rc::new(move |_| *c2.borrow_mut() += 1));

        assert_eq!(dispatcher.listener_count(), 2);

        dispatcher.dispatch(&CeEvent::SelectionChanged {
            selection: CeSelection::collapsed(DomCursor::new(1, 0)),
        });

        assert_eq!(*count.borrow(), 2);
    }

    #[test]
    fn event_dispatcher_clear() {
        let mut dispatcher = CeEventDispatcher::new();
        dispatcher.subscribe(Rc::new(|_| {}));
        dispatcher.subscribe(Rc::new(|_| {}));
        assert_eq!(dispatcher.listener_count(), 2);

        dispatcher.clear();
        assert_eq!(dispatcher.listener_count(), 0);
    }

    #[test]
    fn thread_local_ce_api_none_by_default() {
        clear_active_ce_api();
        let result = with_active_ce_api(|_| 42);
        assert!(result.is_none());
    }

    #[test]
    fn global_dispatch_subscribe_and_receive() {
        clear_ce_event_listeners();

        let received = Rc::new(RefCell::new(Vec::new()));
        let received_clone = received.clone();

        subscribe_ce_events(Rc::new(move |event: &CeEvent| {
            received_clone.borrow_mut().push(format!("{:?}", event));
        }));

        assert_eq!(ce_event_listener_count(), 1);

        dispatch_ce_event(&CeEvent::TextInserted {
            node_id: 42,
            offset: 0,
            text: "x".into(),
        });

        assert_eq!(received.borrow().len(), 1);
        assert!(received.borrow()[0].contains("42"));

        clear_ce_event_listeners();
        assert_eq!(ce_event_listener_count(), 0);
    }
}
