//! Selection state management.

use rinch_editable::Selection;

/// Manages selection state for the editor.
#[derive(Debug, Clone)]
pub struct SelectionState {
    /// Current selection
    pub selection: Selection,
}

impl SelectionState {
    /// Create a new selection state.
    pub fn new(selection: Selection) -> Self {
        Self { selection }
    }

    /// Update the selection.
    pub fn set_selection(&mut self, selection: Selection) {
        self.selection = selection;
    }

    /// Get the current selection.
    pub fn get_selection(&self) -> &Selection {
        &self.selection
    }
}
