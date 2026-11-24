//! IME composition: preedit, commit and surrounding-text deletion.

use super::*;

impl EditorHandle {
    /// Show the IME composition string `text` as a transient overlay at the caret
    /// (never inserted into the document). An empty `text` clears the overlay.
    /// `cursor` is the candidate cursor within `text`; the overlay ignores it for
    /// now (the platform candidate box is placed from the model caret instead). A
    /// no-op before mount.
    ///
    /// A [read-only](Self::set_read_only) editor, or one frozen by a collaboration
    /// freeze ([`Self::collab_oversized_tables`]), shows no composition: the commit it
    /// would lead to is refused, so the overlay would be text that can never land.
    pub fn ime_set_preedit(&self, text: &str, _cursor: Option<(usize, usize)>) {
        let mut core = self.core_mut();
        let text = if core.locked() { "" } else { text };
        if let Some(view) = core.view.as_mut() {
            view.set_preedit(text);
        }
    }

    /// Clear the IME composition overlay without inserting anything (composition
    /// cancelled / disabled). A no-op before mount.
    pub fn ime_clear_preedit(&self) {
        if let Some(view) = self.core_mut().view.as_mut() {
            view.set_preedit("");
        }
    }

    /// Commit composed `text`: clear the preedit overlay, then insert the text at
    /// the selection as one ordinary edit (so it joins the undo history like
    /// typing). An empty commit just clears the overlay.
    pub fn ime_commit(&self, text: &str) {
        if let Some(view) = self.core_mut().view.as_mut() {
            view.set_preedit("");
        }
        if !text.is_empty() {
            self.insert_text(text);
        }
    }

    /// Delete `before` characters before the caret and `after` after it — the
    /// surrounding-text edit some IMEs use to recompose. Clears any preedit first,
    /// then deletes the clamped `[head - before, head + after)` range in one edit.
    /// A defensive no-op if the range is empty or the delete is invalid (e.g. it
    /// would cross a block boundary the schema rejects). Only reached once a backend
    /// advertises surrounding-text support.
    pub fn ime_delete_surrounding(&self, before: usize, after: usize) {
        if let Some(view) = self.core_mut().view.as_mut() {
            view.set_preedit("");
        }
        if before == 0 && after == 0 {
            return;
        }
        self.dispatch(
            |state| {
                let head = state.selection.head().0;
                let from = head.saturating_sub(before);
                let to = (head + after).min(state.doc.content().size());
                if from >= to {
                    return None;
                }
                // Through the selection delete, so a range across a cell's
                // edge (fitted since #1930) gets the table repair the other
                // delete paths get (#1847).
                let mut tr = state.tr();
                tr.set_selection(Selection::text(Pos(from), Pos(to)));
                tr.delete_selection().ok()?;
                Some(tr)
            },
            true,
        );
    }
}
