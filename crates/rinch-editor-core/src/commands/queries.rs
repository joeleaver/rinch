//! Toolbar queries: the marks and blocks at the selection, read from state.

use crate::model::{Mark, MarkType, Node, NodeType};
use crate::pos::Pos;
use crate::state::EditorState;

use super::wrapping::{build_lift_dispatch, build_sink_dispatch};

// ===================================================================
// Toolbar queries — read state, never the host (A6)
// ===================================================================

/// The marks active at a position (what a character typed there would inherit).
pub fn marks_at(state: &EditorState, pos: usize) -> Vec<Mark> {
    state
        .doc
        .resolve(Pos(pos))
        .map(|r| r.marks())
        .unwrap_or_default()
}

/// True if any inline node in `from..to` carries a mark of `mark_type` (PM's
/// `rangeHasMark`).
pub fn range_has_mark(doc: &Node, from: usize, to: usize, mark_type: &MarkType) -> bool {
    if to <= from {
        return false;
    }
    let mut found = false;
    doc.nodes_between(from, to, &mut |node, _pos, _parent| {
        if !found && node.is_inline() && node.marks().iter().any(|m| &m.typ == mark_type) {
            found = true;
        }
        !found
    });
    found
}

/// Whether `mark_type` is active for the current selection — for a collapsed cursor,
/// whether it is in the stored/inherited marks; for a range, whether the range has
/// the mark. Drives a toolbar button's "on" state.
pub fn is_mark_active(state: &EditorState, mark_type: &MarkType) -> bool {
    let sel = &state.selection;
    if sel.is_empty() {
        let marks = state
            .stored_marks
            .clone()
            .unwrap_or_else(|| marks_at(state, sel.from().0));
        marks.iter().any(|m| &m.typ == mark_type)
    } else {
        ranges_have_mark(state, mark_type)
    }
}

/// The selection's content ranges ([`Selection::ranges`](crate::Selection::ranges)):
/// each selected cell's for a cell selection.
pub(super) fn sel_ranges(state: &EditorState) -> Vec<(usize, usize)> {
    state
        .selection
        .ranges(&state.doc)
        .into_iter()
        .map(|(a, b)| (a.0, b.0))
        .collect()
}

/// Whether any of the selection's ranges carries `mark_type`. Over a cell
/// selection each selected cell is read as its own node, so asking costs in
/// the selected cells, not the table (a toolbar asks for every mark).
pub(super) fn ranges_have_mark(state: &EditorState, mark_type: &MarkType) -> bool {
    if let crate::selection::Selection::Cell(c) = &state.selection {
        return crate::tables::selected_cells(&state.doc, c.anchor_cell.0, c.head_cell.0)
            .iter()
            .any(|(_, cell)| range_has_mark(cell, 0, cell.content_size(), mark_type));
    }
    sel_ranges(state)
        .into_iter()
        .any(|(from, to)| range_has_mark(&state.doc, from, to, mark_type))
}

/// The node type of the textblock at the selection head — drives the block-style
/// dropdown's current value.
pub fn current_block_type(state: &EditorState) -> Option<NodeType> {
    let r = state.doc.resolve(state.selection.head()).ok()?;
    Some(r.parent().node_type().clone())
}

/// Whether any ancestor of the selection head is of `type_name` — drives the
/// List/Blockquote button active-state (A6: `current_block_type` alone can't,
/// because the textblock stays `paragraph` inside a `list_item`).
pub fn in_node_type(state: &EditorState, type_name: &str) -> bool {
    if let Ok(r) = state.doc.resolve(state.selection.head()) {
        for d in 0..=r.depth() {
            if r.node(d).type_name() == type_name {
                return true;
            }
        }
    }
    false
}

/// Whether the outdent (lift) command currently applies.
pub fn can_lift(state: &EditorState) -> bool {
    build_lift_dispatch(state).is_some()
}

/// Whether the indent (sink list item) command currently applies — in a list or task
/// list.
pub fn can_sink(state: &EditorState) -> bool {
    build_sink_dispatch(state).is_some()
}
