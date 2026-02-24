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
    pub fn get(&self) -> Option<T> {
        self.data.get()
    }

    /// Take the data (sets internal state to `None`). Call from `ondrop`.
    pub fn take(&self) -> Option<T> {
        let d = self.data.get();
        self.data.set(None);
        d
    }

    /// Check if a drag is in progress (data is set).
    pub fn is_active(&self) -> bool {
        self.data.get().is_some()
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
