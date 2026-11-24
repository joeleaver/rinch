//! Moving the selection: setting it, caret affinity, arrow motion and Tab
//! in tables.

use super::*;

impl EditorHandle {
    /// Move the selection (and re-project, so the caret follows once geometry lands).
    ///
    /// A selection that changed brings the caret into view on the next caret pass
    /// of a focused editor (the arrow keys and clicks all land here). It clears
    /// the caret-affinity hint, even for the selection already held: the caret
    /// is drawn [`CaretAffinity::Downstream`] — see
    /// [`set_selection_with_affinity`](Self::set_selection_with_affinity).
    pub fn set_selection(&self, selection: Selection) {
        self.set_selection_hinted(selection, None);
    }

    /// [`set_selection`](Self::set_selection), and draw the caret at the head on
    /// `affinity`'s side of a soft line wrap (#301).
    ///
    /// The end of one visual line and the start of the next are one model
    /// position; a caret there is drawn at the start of the lower line unless it
    /// is told otherwise. The platforms say `Upstream` for a caret that arrived
    /// from the upper line's side — End, a press past a wrapped line's end, a
    /// vertical move whose column lies past the target line's end — and
    /// `Downstream` for Home. The hint is **view state**, not part of the
    /// model's [`Selection`]: it lives beside the selection it was set with and
    /// applies only while the selection is still that one, so any other change
    /// of selection — typing, an edit before the caret in its own paragraph,
    /// undo, a load, a plain `set_selection` — returns the caret to downstream.
    /// A remote edit, or a local one, that leaves the caret's paragraph
    /// unchanged up to it — an edit in an earlier paragraph, a peer typing above
    /// or after the caret — carries it along, since the caret's line is
    /// unchanged. Right before a hard break the caret is always drawn upstream,
    /// at the end of its line.
    ///
    /// It reaches the caret overlay, [`caret_rect`](Self::caret_rect) at the
    /// head, and through them the platforms' visual-line motions (Home / End,
    /// the soft-line deletes, Up / Down), which measure from the head's rect.
    pub fn set_selection_with_affinity(&self, selection: Selection, affinity: CaretAffinity) {
        self.set_selection_hinted(selection, Some(affinity));
    }

    /// The side of a soft line wrap the caret at the selection's head is drawn
    /// on: the hint [`set_selection_with_affinity`](Self::set_selection_with_affinity)
    /// set while the selection is still that one, else
    /// [`CaretAffinity::Downstream`].
    pub fn caret_affinity(&self) -> CaretAffinity {
        self.core().caret_affinity()
    }

    /// [`caret_affinity`](Self::caret_affinity) for a caret at `pos`: the hint
    /// when `pos` is the selection's head, else [`CaretAffinity::Downstream`] —
    /// and [`CaretAffinity::Upstream`] right before a hard break, which ends its
    /// line. What a platform's geometry query for `pos` should draw with.
    pub fn caret_affinity_at(&self, pos: Pos) -> CaretAffinity {
        self.core().affinity_at(pos)
    }

    fn set_selection_hinted(&self, selection: Selection, hint: Option<CaretAffinity>) {
        let before = {
            let mut core = self.core_mut();
            let before = (core.state.selection.clone(), core.caret_affinity());
            // Stored BEFORE the dispatch, so the selection-change callbacks the
            // commit fires already see it; it is inert unless the dispatch lands
            // exactly `selection`.
            core.caret_hint = hint.map(|a| (selection.clone(), a));
            before
        };
        self.dispatch(
            |state| {
                let mut tr = state.tr();
                tr.set_selection(selection.clone());
                Some(tr)
            },
            true,
        );
        let doc_key = {
            let core = self.core_mut();
            // Only the drawing side changed — the same selection, the other side
            // of the wrap: no transaction moved anything, so nothing else owes
            // the caret pass that redraws it.
            let redraw = core.state.selection == before.0 && core.caret_affinity() != before.1;
            core.view.as_ref().filter(|_| redraw).map(|v| v.doc_key())
        };
        if let Some(doc_key) = doc_key {
            crate::registry::owe_overlay_pass(doc_key);
            crate::registry::request_overlay_refresh();
        }
    }

    /// Select the word around model `pos` (the double-click gesture). Returns whether
    /// it resolved to a non-empty range.
    pub fn select_word_at(&self, pos: Pos) -> bool {
        let doc = self.doc();
        let (from, to) = rinch_editor_core::word_range_at(&doc, pos);
        self.set_selection(Selection::text(from, to));
        from != to
    }

    /// Select the whole textblock around model `pos` (the triple-click gesture).
    /// Returns whether it resolved to a non-empty range.
    pub fn select_block_at(&self, pos: Pos) -> bool {
        let doc = self.doc();
        let (from, to) = rinch_editor_core::block_range_at(&doc, pos);
        self.set_selection(Selection::text(from, to));
        from != to
    }

    /// Apply a model caret `motion` (the horizontal / word / model-line / document
    /// motions — *not* vertical, which needs laid-out geometry the platform supplies).
    /// `extend` keeps the selection anchor (shift+arrow). Returns whether the cursor
    /// moved. Shared by every platform's keyboard glue; the desktop runtime layers
    /// visual-line and vertical geometry on top.
    pub fn move_cursor(&self, motion: CursorMotion, extend: bool) -> bool {
        let st = self.state();
        let horizontal = match motion {
            CursorMotion::CharLeft | CursorMotion::WordLeft => Some(-1),
            CursorMotion::CharRight | CursorMotion::WordRight => Some(1),
            _ => None,
        };
        // Over a cell selection a horizontal Shift+arrow steps its head cell;
        // the text motion has no head to start from.
        if extend
            && let Some(dir) = horizontal
            && matches!(st.selection, Selection::Cell(_))
        {
            return self.shift_arrow_cells(rinch_editor_core::tables::Axis::Horiz, dir, None);
        }
        let resolved = rinch_editor_core::resolve_cursor_motion(
            &st.doc,
            st.selection.head(),
            st.selection.anchor(),
            motion,
            extend,
        );
        if extend
            && let Some(dir) = horizontal
            && self.shift_arrow_cells(
                rinch_editor_core::tables::Axis::Horiz,
                dir,
                resolved.as_ref().map(|s| s.head()),
            )
        {
            return true;
        }
        match resolved {
            // A caret moved onto a node a view is drawn for offers it the
            // keyboard (`NodeViewSpec::on_caret_enter`).
            Some(sel) if !extend => {
                let from = match motion {
                    CursorMotion::CharLeft | CursorMotion::WordLeft | CursorMotion::DocStart => {
                        CaretFrom::Right
                    }
                    _ => CaretFrom::Left,
                };
                self.select_from_arrow(sel, from);
                true
            }
            Some(sel) => {
                self.set_selection(sel);
                true
            }
            None => false,
        }
    }

    /// Shift+arrow at a table cell's edge, or over a cell selection: select
    /// cells, as prosemirror-tables' `shiftArrow` does
    /// ([`shift_arrow`](rinch_editor_core::commands::table_ops::shift_arrow)).
    /// `new_head` is where the plain extension would put the head (the text
    /// motion, or the platform's vertical step; `None` when it goes nowhere).
    /// Returns whether the cell rule took the key, having set the selection;
    /// `false` leaves the plain extension to the caller.
    ///
    /// [`move_cursor`](Self::move_cursor) asks it for every horizontal
    /// Shift+arrow; the platforms ask it for Shift+Up/Down with their
    /// geometry's step (#1847).
    pub fn shift_arrow_cells(
        &self,
        axis: rinch_editor_core::tables::Axis,
        dir: i32,
        new_head: Option<Pos>,
    ) -> bool {
        let next = {
            let core = self.core();
            rinch_editor_core::commands::table_ops::shift_arrow(&core.state, axis, dir, new_head)
        };
        match next {
            Some(sel) => {
                self.set_selection(sel);
                true
            }
            None => false,
        }
    }

    /// Move the cursor to the next (`shift=false`) or previous (`shift=true`) table
    /// cell — the Tab/Shift-Tab gesture inside a table. Forward-Tab in the last cell
    /// appends a row first. A no-op (returns `false`) outside a table. Shared by every
    /// platform's keyboard glue.
    pub fn tab_cell(&self, shift: bool) -> bool {
        use rinch_editor_core::tables;
        let state = self.state();
        let head = state.selection.head();
        let dir = if shift { -1 } else { 1 };
        if let Some(target) = tables::next_cell_in_table(&state.doc, head, dir) {
            self.set_selection(Selection::near(&state.doc, Pos(target + 1), 1));
            return true;
        }
        if dir > 0 && tables::cell_at_pos(&state.doc, head).is_some() && self.command("addRowAfter")
        {
            let state = self.state();
            if let Some(target) = tables::next_cell_in_table(&state.doc, state.selection.head(), 1)
            {
                self.set_selection(Selection::near(&state.doc, Pos(target + 1), 1));
            }
            return true;
        }
        false
    }

    /// Choose what Tab and Shift+Tab do in this editor (#2210) — see
    /// [`TabBehavior`]. Takes effect at once and lives on the handle, so it
    /// survives a re-mount. The `Editor { tab_behavior: … }` prop calls this
    /// at mount.
    pub fn set_tab_behavior(&self, behavior: TabBehavior) {
        self.core_mut().tab_behavior = behavior;
    }

    /// What Tab does in this editor — see [`Self::set_tab_behavior`].
    pub fn tab_behavior(&self) -> TabBehavior {
        self.core().tab_behavior
    }

    /// Offer a plain Tab (`shift = false`) or Shift+Tab to the editor (#2210):
    /// `true` when the editor used it — it moved to another table cell, or ran
    /// a list binding that applied — and `false` when it did not, in which case
    /// the caller moves the focus to the next (previous) Tab stop. Always
    /// `false` under [`TabBehavior::MoveFocus`] and in a
    /// [read-only](Self::set_read_only) editor. Shared by every platform's
    /// keyboard glue, which offers Tab here instead of to the keymap.
    pub fn tab_key(&self, shift: bool) -> bool {
        if self.refuses_edits() || self.tab_behavior() == TabBehavior::MoveFocus {
            return false;
        }
        if self.tab_cell(shift) {
            return true;
        }
        let binding = KeyBinding::parse(if shift { "Shift-Tab" } else { "Tab" });
        binding.is_some_and(|b| self.dispatch_key(b) == Some(true))
    }
}
