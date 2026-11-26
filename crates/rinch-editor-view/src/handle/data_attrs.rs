//! An app's `data-*` attributes on the selection's textblocks, from the handle.

use rinch_editor_core::commands;

use super::EditorHandle;

impl EditorHandle {
    /// Set (`Some`) or remove (`None`) the app data attribute `name` on every
    /// textblock of the selection that keeps data attributes (the starter kit's
    /// paragraph and heading), as one undo step
    /// ([`commands::set_block_data_attr`]). The editor writes it on each
    /// block's host element, so the app's stylesheet can select on it
    /// (`p[data-role="note"]`).
    ///
    /// Returns whether anything changed: `false` when no block would change,
    /// when the name or value is refused (a name rinch reserves or HTML does
    /// not allow, a value past the bound) and on a
    /// [read-only](EditorHandle::set_read_only) editor.
    pub fn set_block_data_attr(&self, name: &str, value: Option<&str>) -> bool {
        self.run_command(commands::set_block_data_attr(name, value))
    }

    /// The value of the app data attribute `name` that every textblock of the
    /// selection which keeps data attributes holds, if they share one
    /// ([`commands::block_data_attr`]).
    pub fn block_data_attr(&self, name: &str) -> Option<String> {
        commands::block_data_attr(&self.core().state, name)
    }
}
