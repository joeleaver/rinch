//! Typed drag-and-drop data context.
//!
//! `DragContext<T>` wraps a `Signal<Option<T>>` to provide a convenient
//! typed container for drag-and-drop data transfer. Create one per drag
//! data type and share it between drag sources and drop targets.

use crate::reactive::Signal;

/// Typed drag data context for drag-and-drop operations.
///
/// Uses `Signal` internally, so it's `Copy` and can be used in multiple
/// closures without cloning. Create one per drag data type and share
/// between sources and targets.
///
/// # Example
///
/// ```ignore
/// let drag = DragContext::<TodoItem>::new();
///
/// // In drag source's ondragstart:
/// drag.set(item.clone());
///
/// // In drop target's ondrop:
/// if let Some(item) = drag.take() {
///     target_list.update(|list| list.push(item));
/// }
/// ```
pub struct DragContext<T: Clone + 'static> {
    data: Signal<Option<T>>,
}

// Manual Clone/Copy impls: Signal<Option<T>> is always Copy (it's just an ID),
// so DragContext<T> doesn't need T: Copy.
impl<T: Clone + 'static> Clone for DragContext<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + 'static> Copy for DragContext<T> {}

impl<T: Clone + 'static> DragContext<T> {
    /// Create a new empty drag context.
    pub fn new() -> Self {
        Self {
            data: Signal::new(None),
        }
    }

    /// Set the data being dragged. Call from `ondragstart`.
    pub fn set(&self, data: T) {
        self.data.set(Some(data));
    }

    /// Get a clone of the data being dragged, if any.
    ///
    /// Returns `None` both when no drag is in progress and when the context's
    /// backing signal has been freed — a `DragContext` is `Copy` and routinely
    /// captured by drop-target handlers that outlive the source component, so a
    /// query must never panic. (Reads go through `try_get` for this reason;
    /// see issue #141.)
    pub fn get(&self) -> Option<T> {
        self.data.try_get().flatten()
    }

    /// Take the data (sets internal state to `None`). Call from `ondrop`.
    ///
    /// Clears only when there was something to clear, so a `take` on an idle
    /// context no longer notifies subscribers with an unchanged `None`.
    pub fn take(&self) -> Option<T> {
        let d = self.get();
        if d.is_some() {
            self.data.set(None);
        }
        d
    }

    /// Check if a drag is in progress (data is set).
    pub fn is_active(&self) -> bool {
        self.get().is_some()
    }

    /// Clear the drag data. Call from `ondragend` if needed.
    pub fn clear(&self) {
        self.data.set(None);
    }
}

impl<T: Clone + 'static> Default for DragContext<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Drag ghost visibility
// ============================================================================

use std::cell::Cell;

thread_local! {
    /// When false, the framework's built-in drag ghost (element snapshot) is hidden.
    /// Drop targets and surfaces can suppress the ghost when they provide their own
    /// visual feedback (e.g., a 3D preview in a game viewport).
    static DRAG_GHOST_VISIBLE: Cell<bool> = const { Cell::new(true) };
}

/// Hide the framework's built-in drag ghost.
///
/// Call this from `ondragstart`, `ondragenter`, `DragEnter`, or `DragOver`
/// handlers when the drop target provides its own visual feedback. The ghost
/// remains hidden until `restore_drag_ghost()` is called or the drag ends.
///
/// ```ignore
/// // In a surface event handler:
/// SurfaceEvent::DragEnter { x, y } => {
///     suppress_drag_ghost();
///     spawn_3d_preview(x, y);
/// }
/// SurfaceEvent::DragLeave => {
///     restore_drag_ghost();
///     despawn_3d_preview();
/// }
/// ```
pub fn suppress_drag_ghost() {
    DRAG_GHOST_VISIBLE.with(|v| v.set(false));
}

/// Show the framework's built-in drag ghost again.
///
/// Call this from `ondragleave` or `DragLeave` when the cursor leaves a
/// drop target that was providing its own visual feedback.
pub fn restore_drag_ghost() {
    DRAG_GHOST_VISIBLE.with(|v| v.set(true));
}

/// Check whether the drag ghost should be rendered.
/// Called by the paint system.
pub fn is_drag_ghost_visible() -> bool {
    DRAG_GHOST_VISIBLE.with(|v| v.get())
}

/// Reset ghost visibility to default (visible). Called by the runtime
/// when a drag ends.
pub fn reset_drag_ghost_visibility() {
    DRAG_GHOST_VISIBLE.with(|v| v.set(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Item(u32);

    #[test]
    fn take_returns_the_data_and_clears() {
        let drag = DragContext::<Item>::new();
        assert!(!drag.is_active());

        drag.set(Item(1));
        assert!(drag.is_active());
        assert_eq!(drag.get(), Some(Item(1)));

        assert_eq!(drag.take(), Some(Item(1)));
        assert!(!drag.is_active());
        assert_eq!(drag.take(), None);
    }

    #[test]
    fn queries_are_safe_after_the_backing_signal_is_freed() {
        // A DragContext is Copy and is routinely captured by drop-target
        // handlers that outlive the source component. Once #141's disposal
        // lands, that source's scope can free the signal mid-drag — every query
        // must degrade to "no drag", not panic (issue #141, SD1).
        let drag = DragContext::<Item>::new();
        drag.set(Item(9));

        drag.data.free_for_tests();

        assert_eq!(drag.get(), None);
        assert_eq!(drag.take(), None);
        assert!(!drag.is_active());
        drag.clear(); // lenient write — must not panic either
    }
}
