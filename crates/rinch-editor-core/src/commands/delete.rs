//! Deletion: the selection, Backspace and Delete across block boundaries, and
//! the word deletes.

use crate::Fragment;
use crate::command::{Command, chain};
use crate::model::{Attrs, Node, NodeType};
use crate::pos::{Pos, ResolvedPos};
use crate::state::{EditorState, Transaction};
use crate::transform::block_range_simple;
use crate::transform::data_attrs::Spot;
use crate::transform::steps::{ReplaceAroundStep, ReplaceStep};

use super::wrapping::build_lift;
use super::{command_tr, table_ops};

/// Delete the current selection (only applies when it is non-empty). A cell
/// selection clears the selected cells' content (it is never empty) rather than
/// splicing its coarse range.
pub fn delete_selection() -> Command {
    command_tr(|state| {
        if matches!(state.selection, crate::selection::Selection::Cell(_)) {
            return table_ops::clear_cells(state);
        }
        if state.selection.is_empty() {
            return None;
        }
        let mut tr = state.tr();
        tr.delete_selection().ok()?;
        Some(tr)
    })
}

/// PM `findCutBefore`: the document position just before the nearest ancestor of
/// the cursor (walking up from it) that has a sibling before it — the boundary a
/// Backspace at block-start would join across. `None` when the cursor is at the
/// very start of its top-level context (so Backspace should *lift*, not join).
/// Stops at (and never crosses) an isolating boundary (e.g. a table cell, M7).
fn find_cut_before(r: &ResolvedPos) -> Option<usize> {
    if r.parent().node_type().spec().isolating {
        return None;
    }
    let mut depth = r.depth() as isize - 1;
    while depth >= 0 {
        let d = depth as usize;
        if r.index(d) > 0 {
            return r.before(d + 1);
        }
        if r.node(d).node_type().spec().isolating {
            break;
        }
        depth -= 1;
    }
    None
}

/// PM `findCutAfter`: the mirror of [`find_cut_before`] — the position just after
/// the nearest ancestor that has a sibling after it, the boundary a forward-Delete
/// at block-end joins across.
fn find_cut_after(r: &ResolvedPos) -> Option<usize> {
    if r.parent().node_type().spec().isolating {
        return None;
    }
    let mut depth = r.depth() as isize - 1;
    while depth >= 0 {
        let d = depth as usize;
        let parent = r.node(d);
        if r.index(d) + 1 < parent.child_count() {
            return r.after(d + 1);
        }
        if parent.node_type().spec().isolating {
            break;
        }
        depth -= 1;
    }
    None
}

/// Whether a join across `cut` must not touch the node before it: an isolating
/// node (a table cell), a table or a table row. ProseMirror's `deleteBarrier`
/// neither joins nor wraps across an `isolating` node, and prosemirror-tables
/// makes the table isolating; rinch's table is not (that would change paste and
/// lift), so the table parts are named here. Joining two tables, or wrapping the
/// block after a table into a new row of it, left a row with another cell count
/// than the table's (#1232).
///
/// Only the node *before* the cut is asked. The node after it is the caret's
/// own block for Backspace; for Delete a table after a block other than a table
/// cannot be joined (its rows are not that block's content, so the step fails)
/// and the lift that follows stops at the cell (`build_lift_after`'s floor).
fn table_barrier(doc: &Node, cut: usize) -> bool {
    let Ok(r) = doc.resolve(Pos(cut)) else {
        return true;
    };
    r.node_before().is_some_and(|n| {
        n.node_type().is_isolating() || crate::tables::is_table(&n) || crate::tables::is_row(&n)
    })
}

/// Whether `node`'s last descendant down its last children is a textblock (PM
/// `textblockAt(node, "end")`).
fn ends_in_textblock(node: &Node) -> bool {
    let mut n = node.clone();
    loop {
        if n.is_textblock() {
            return true;
        }
        if n.is_leaf() || n.child_count() == 0 {
            return false;
        }
        n = n.child(n.child_count() - 1).clone();
    }
}

/// Backspace at the start of a block right after a table (`cut` from
/// [`find_cut_before`]), ProseMirror's base keymap there (`joinBackward`'s
/// empty-block branch, then `selectNodeBackward`): an empty block is deleted
/// (with each ancestor it is the only child of, while that is not valid) and
/// the caret goes to the end of the table's last textblock (the table is
/// node-selected when it ends in no textblock); any other block leaves the
/// document as it is and **selects the table**, so a second Backspace deletes it
/// — the select-first rule rinch keeps next to an atom ([`select_atom_across`]).
/// Nothing is joined into, wrapped into or lifted out past the table (#1232).
/// `None` when the node before the cut is a row or a cell.
fn backspace_after_table(state: &EditorState, r: &ResolvedPos, cut: usize) -> Option<Transaction> {
    use crate::selection::{NodeSelection, Selection};
    let table = state.doc.resolve(Pos(cut)).ok()?.node_before()?;
    if !crate::tables::is_table(&table) {
        return None;
    }
    let table_pos = cut - table.node_size();
    let table_sel = || {
        Selection::Node(NodeSelection {
            anchor: Pos(table_pos),
            head: Pos(cut),
        })
    };
    if r.parent().is_textblock() && r.parent().content().size() == 0 {
        let mut d = r.depth();
        while d >= 1 {
            let (Some(from), Some(to)) = (r.before(d), r.after(d)) else {
                break;
            };
            let mut tr = state.tr();
            if tr
                .step(Box::new(ReplaceStep::new(from, to, crate::Slice::empty())))
                .is_ok()
                && tr.doc_changed()
            {
                // The deletion is after the table: `cut` and `table_pos` hold.
                let sel = if ends_in_textblock(&table) {
                    Selection::near(tr.doc(), Pos(cut), -1)
                } else {
                    table_sel()
                };
                tr.set_selection(sel);
                return Some(tr);
            }
            if d == 1 || r.node(d - 1).child_count() > 1 {
                break;
            }
            d -= 1;
        }
    }
    let mut tr = state.tr();
    tr.set_selection(table_sel());
    Some(tr)
}

/// Lift the block that begins just after `cut` out of its wrapper (PM
/// `deleteBarrier`'s lift path, used when a forward-Delete can't directly join
/// across the barrier). Returns `None` if there is no liftable block there.
fn build_lift_after(state: &EditorState, cut: usize) -> Option<Transaction> {
    let sel_after = crate::selection::Selection::near(&state.doc, Pos(cut), 1);
    let r_from = state.doc.resolve(sel_after.from()).ok()?;
    let r_to = state.doc.resolve(sel_after.to()).ok()?;
    let range = block_range_simple(&r_from, &r_to)?;
    if range.depth == 0 {
        return None;
    }
    // The isolating floor of [`build_lift`]: a block lifted out of a table cell
    // split the table around it (#1232).
    let floor = (1..=range.depth)
        .filter(|&d| r_from.node(d).node_type().is_isolating())
        .max()
        .unwrap_or(0);
    for target in (floor..range.depth).rev() {
        let mut tr = state.tr();
        if tr.lift(&range, target).is_ok() && tr.doc_changed() {
            return Some(tr);
        }
    }
    None
}

/// A wrapping chain of node types `[W1, .., Wn]` — PM `ContentMatch.findWrapping`
/// adapted to the simplified content model — such that `before_type` can contain
/// `W1`, each `Wi` can contain `Wi+1`, and `Wn` can contain `target_type`. A BFS
/// over node types (edge `A → B` iff `A`'s content accepts `B`), depth-bounded.
/// `None` if no chain exists. Used by [`wrap_into_before`] to move a block into a
/// preceding container.
fn find_wrapping(
    schema: &crate::Schema,
    before_type: &str,
    target_type: &str,
) -> Option<Vec<NodeType>> {
    use std::collections::{HashSet, VecDeque};
    // Deterministic candidate order (the node map is a HashMap).
    let mut names: Vec<&str> = schema.nodes.keys().map(String::as_str).collect();
    names.sort_unstable();
    let is_container = |n: &str| schema.node_type(n).is_some_and(|t| !t.is_leaf());

    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<Vec<&str>> = VecDeque::new();
    for &n in &names {
        if schema.allows_child(before_type, n) && is_container(n) {
            visited.insert(n);
            queue.push_back(vec![n]);
        }
    }
    while let Some(chain) = queue.pop_front() {
        let last = *chain.last()?;
        if schema.allows_child(last, target_type) {
            return chain
                .iter()
                .map(|&n| schema.node_type(n).cloned())
                .collect();
        }
        if chain.len() >= 4 {
            continue; // depth bound — no real schema nests this deep
        }
        for &n in &names {
            if !visited.contains(n) && schema.allows_child(last, n) && is_container(n) {
                visited.insert(n);
                let mut next = chain.clone();
                next.push(n);
                queue.push_back(next);
            }
        }
    }
    None
}

/// PM `deleteBarrier` find-wrapping branch: move the block just after `cut` into the
/// container just before it, wrapped to fit — a paragraph after a bullet list
/// becomes a new list item. `None` if the before-block isn't a container that can
/// take the after-block (wrapped), or either side is isolating.
fn wrap_into_before(state: &EditorState, cut: usize) -> Option<Transaction> {
    let r_cut = state.doc.resolve(Pos(cut)).ok()?;
    let before = r_cut.node_before()?;
    let after = r_cut.node_after()?;
    if before.node_type().spec().isolating || after.node_type().spec().isolating {
        return None;
    }
    let conn = find_wrapping(state.schema(), before.type_name(), after.type_name())?;
    // Build the wrapper chain innermost-first (`[list_item]` → `list_item(<gap>)`),
    // then a copy of the `before` container around it, so the slice (open at the
    // start) merges with the existing container. The gap (the after-block) fills the
    // innermost wrapper via the `ReplaceAroundStep`.
    let mut wrap = Fragment::empty();
    for typ in conn.iter().rev() {
        wrap = Fragment::from_node(Node::new_branch(typ.clone(), Attrs::default(), wrap));
    }
    let before_copy = before.copy_with_content(wrap);
    let slice = crate::Slice::new(Fragment::from_node(before_copy), 1, 0);
    let end = cut + after.node_size();
    let mut tr = state.tr();
    tr.step(Box::new(ReplaceAroundStep::new(
        cut - 1,
        end,
        cut,
        end,
        slice,
        conn.len(),
        true,
    )))
    .ok()?;
    tr.doc_changed().then_some(tr)
}

/// Whether `node` is what Backspace or Delete beside it node-selects rather than
/// joins with: a selectable leaf atom that is not text (an image, a horizontal
/// rule, an `embed`).
fn selectable_atom(node: &Node) -> bool {
    !node.is_text() && node.is_leaf() && node.node_type().spec().selectable
}

/// Backspace at the start of the block after an atom (`backward`), or Delete at
/// the end of the block before one, across `cut` (from [`find_cut_before`] /
/// [`find_cut_after`]): **select** the atom, so the next press removes it. When the
/// caret's block is empty and sits right beside the atom, that block is removed
/// first. `None` when the node across the cut is not a [`selectable_atom`].
///
/// A decision, and deliberately **unlike ProseMirror**, whose base keymap runs
/// `joinBackward` / `joinForward` before `selectNodeBackward` /
/// `selectNodeForward`: those delete the atom in one press when the caret's block
/// is not empty (and `joinForward` from an empty one too). An atom can be an
/// `embed` holding an app's component and its state, so rinch selects it first —
/// for every selectable atom alike (`tests/backspace_selects_atom.rs`).
fn select_atom_across(
    state: &EditorState,
    r: &ResolvedPos,
    cut: usize,
    backward: bool,
) -> Option<Transaction> {
    let r_cut = state.doc.resolve(Pos(cut)).ok()?;
    let atom = if backward {
        r_cut.node_before()?
    } else {
        r_cut.node_after()?
    };
    if !selectable_atom(&atom) {
        return None;
    }
    let atom_pos = if backward {
        cut - atom.node_size()
    } else {
        cut
    };
    let mut tr = state.tr();
    let depth = r.depth();
    let beside = depth >= 1 && r_cut.depth() + 1 == depth;
    if beside
        && r.parent().content().size() == 0
        && let (Some(from), Some(to)) = (r.before(depth), r.after(depth))
    {
        tr.delete(from, to).ok()?;
    }
    let atom_pos = tr.mapping().map(atom_pos, 1);
    let sel = crate::selection::Selection::node_at(tr.doc(), Pos(atom_pos))?;
    tr.set_selection(sel);
    Some(tr)
}

/// Backspace: delete the selection, else the char before the cursor, else — at the
/// start of a textblock — join onto the previous block, move into a preceding
/// container, or lift this block out of its wrapper.
///
/// At block start this is PM's `joinBackward`/`deleteBarrier`: it joins across the
/// nearest cut with a **structural** replace of the boundary token pair (its own
/// undo group, never coalesced into typing); when it cannot join directly but the
/// block before the cut is a **container** that can take this block (wrapped) — a
/// paragraph after a list — it **moves the block in** as a new item
/// (`wrap_into_before`, the find-wrapping branch); and when it crosses a wrapper
/// boundary from *inside* — a block at the start of a blockquote / list item — it
/// **lifts** this block out of the wrapper, so a second Backspace then joins.
///
/// An atom before the cut (a horizontal rule, an image, an `embed`) is
/// **selected** instead (`select_atom_across`), so a second Backspace removes
/// it; an empty block right after it goes in the same step. After a table,
/// `backspace_after_table`: nothing is joined, wrapped or lifted.
///
/// Not ported (a rare `deleteBarrier` tail): the content-move `ReplaceAround`
/// (merge into the before-container's last textblock).
pub fn delete_char_backward() -> Command {
    command_tr(|state| {
        let sel = &state.selection;
        if matches!(sel, crate::selection::Selection::Cell(_)) {
            return table_ops::clear_cells(state);
        }
        if !sel.is_empty() {
            let mut tr = state.tr();
            tr.delete_selection().ok()?;
            return Some(tr);
        }
        let pos = sel.from().0;
        if pos == 0 {
            return None;
        }
        let r = state.doc.resolve(Pos(pos)).ok()?;
        if r.parent_offset() > 0 {
            // Mid-text: delete the char before the cursor — or the whole emoji
            // sequence it ends (#1684).
            let from = crate::motion::backspace_start(&state.doc, Pos(pos)).unwrap_or(pos - 1);
            let mut tr = state.tr();
            tr.delete_at(from, pos, Spot::within(&r, from, pos)).ok()?;
            return tr.doc_changed().then_some(tr);
        }
        // At block start: join across the nearest cut, else move into the preceding
        // container, else lift out of the wrapper.
        if let Some(cut) = find_cut_before(&r) {
            // An atom before the cut is selected, not joined with (it has no
            // content to join).
            if let Some(tr) = select_atom_across(state, &r, cut, true) {
                return Some(tr);
            }
            // After a table (or at a row or cell boundary) nothing is joined,
            // wrapped or lifted: see [`backspace_after_table`].
            if table_barrier(&state.doc, cut) {
                return backspace_after_table(state, &r, cut);
            }
            if let Some(tr) = join_at(state, cut) {
                return Some(tr);
            }
            // Could not join directly across the cut — try moving this block
            // into the container before it (a paragraph after a list → a new
            // list item).
            if let Some(tr) = wrap_into_before(state, cut) {
                return Some(tr);
            }
        }
        build_lift(state)
    })
}

/// Delete forward: delete the selection, else the char after the cursor, else — at
/// the end of a textblock — join the next block onto this one, or lift the next
/// block out of its wrapper across the barrier (PM `joinForward`/`deleteBarrier`;
/// see [`delete_char_backward`]). Unlike Backspace there is no "no cut → lift"
/// case: with nothing after, forward-Delete is a no-op.
pub fn delete_char_forward() -> Command {
    command_tr(|state| {
        let sel = &state.selection;
        if matches!(sel, crate::selection::Selection::Cell(_)) {
            return table_ops::clear_cells(state);
        }
        if !sel.is_empty() {
            let mut tr = state.tr();
            tr.delete_selection().ok()?;
            return Some(tr);
        }
        let pos = sel.from().0;
        let r = state.doc.resolve(Pos(pos)).ok()?;
        if r.parent_offset() < r.parent().content().size() {
            // Mid-text: delete the grapheme cluster after the cursor (#1684).
            let to = crate::motion::grapheme_step(&state.doc, Pos(pos), true).unwrap_or(pos + 1);
            let mut tr = state.tr();
            tr.delete_at(pos, to, Spot::within(&r, pos, to)).ok()?;
            return tr.doc_changed().then_some(tr);
        }
        // At block end: join across the nearest cut, else lift the next block out.
        let cut = find_cut_after(&r)?;
        if let Some(tr) = select_atom_across(state, &r, cut, false) {
            return Some(tr);
        }
        if !table_barrier(&state.doc, cut)
            && let Some(tr) = join_at(state, cut)
        {
            return Some(tr);
        }
        build_lift_after(state, cut)
    })
}

/// Join the two nodes that meet at `cut`, the textblocks' app data attributes
/// placed by [the rules](crate::schema::data_attrs). `None` where they cannot
/// be joined directly.
fn join_at(state: &EditorState, cut: usize) -> Option<Transaction> {
    let mut tr = state.tr();
    tr.join_textblocks(cut).ok()?;
    tr.doc_changed().then_some(tr)
}

/// The range a word delete removes from a collapsed caret: from the caret to
/// the previous (`forward = false`) or next word boundary of
/// [`word_boundary`](crate::motion::word_boundary) — exactly what the word
/// motion (Ctrl+Left / Right) moves over, so the two cannot disagree. An inline
/// atom is a word of its own there (#938): beside the caret it goes alone,
/// with any whitespace between them, and a word beyond it stays. `None` when
/// the selection is not a collapsed text caret, or when the range is empty (at
/// a textblock edge).
fn word_delete_range(state: &EditorState, forward: bool) -> Option<(usize, usize)> {
    let sel = &state.selection;
    if matches!(sel, crate::selection::Selection::Cell(_)) || !sel.is_empty() {
        return None;
    }
    let head = sel.head();
    let target = crate::motion::word_boundary(&state.doc, head, forward)?;
    (target != head).then(|| (target.0.min(head.0), target.0.max(head.0)))
}

/// Delete from the caret to a word boundary (see [`word_delete_range`]), when
/// that range is not empty.
fn delete_word_range(forward: bool) -> Command {
    command_tr(move |state| {
        let (from, to) = word_delete_range(state, forward)?;
        let mut tr = state.tr();
        tr.delete(from, to).ok()?;
        tr.doc_changed().then_some(tr)
    })
}

/// Delete the word before the caret (Ctrl+Backspace; Alt+Backspace on macOS):
/// from the caret back to the boundary the word motion (Ctrl+Left) lands on,
/// taking any whitespace between them. A mark boundary is not a word boundary.
///
/// A word is what [`word_segments`](crate::motion::word_segments) says: a run
/// of punctuation is one apart from the letters beside it, and an inline atom
/// is one of its own, so a word delete never takes an atom along with text.
///
/// Where that range is empty it is [`delete_char_backward`]: with a non-empty
/// selection (deleted as it stands) and at a textblock's start (join / lift, as
/// Backspace does there).
pub fn delete_word_backward() -> Command {
    chain(vec![delete_word_range(false), delete_char_backward()])
}

/// Delete the word after the caret (Ctrl+Delete; Alt+Delete on macOS) — the
/// mirror of [`delete_word_backward`], falling back to [`delete_char_forward`].
pub fn delete_word_forward() -> Command {
    chain(vec![delete_word_range(true), delete_char_forward()])
}
