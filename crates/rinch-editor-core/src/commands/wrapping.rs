//! Wrapping commands: blockquotes, lists, and lifting and sinking list items.

use crate::Fragment;
use crate::command::Command;
use crate::model::{Attrs, Node};
use crate::pos::Pos;
use crate::state::{EditorState, Transaction};
use crate::transform::steps::{ReplaceAroundStep, ReplaceStep};
use crate::transform::{NodeRange, block_range, block_range_simple};

use super::{command_tr, in_node_type};

// ===================================================================
// Wrapping: blockquote, lists, lift/sink
// ===================================================================

/// Wrap the selected block range in `node_type` (e.g. `wrapInBlockquote`).
pub fn wrap_in(node_type: &'static str) -> Command {
    command_tr(move |state| {
        let ty = state.schema().node_type(node_type)?.clone();
        let r_from = state.doc.resolve(state.selection.from()).ok()?;
        let r_to = state.doc.resolve(state.selection.to()).ok()?;
        let range = block_range_simple(&r_from, &r_to)?;
        let mut tr = state.tr();
        tr.wrap(&range, &[(ty, Attrs::new())]).ok()?;
        tr.doc_changed().then_some(tr)
    })
}

/// Toggle a list of `list_type` (`toggleBulletList` / `toggleOrderedList`): wrap
/// the block range into `list > list_item` if not already in that list, or lift it
/// out if it is. **Wrap/lift, NOT set_block_type** (A6).
pub fn toggle_list(list_type: &'static str) -> Command {
    command_tr(move |state| {
        if in_node_type(state, list_type) {
            build_lift(state)
        } else {
            build_wrap_in_list(state, list_type)
        }
    })
}

/// Build the wrap-into-list transaction.
fn build_wrap_in_list(state: &EditorState, list_type: &str) -> Option<Transaction> {
    let list_ty = state.schema().node_type(list_type)?.clone();
    let item_name = if list_type == "task_list" {
        "task_item"
    } else {
        "list_item"
    };
    let item_ty = state.schema().node_type(item_name)?.clone();
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range_simple(&r_from, &r_to)?;
    let mut tr = state.tr();
    tr.wrap(&range, &[(list_ty, Attrs::new()), (item_ty, Attrs::new())])
        .ok()?;
    tr.doc_changed().then_some(tr)
}

/// The outdent / lift command: lift the selected block out of its enclosing wrapper.
/// A list/task item is lifted as a whole item (to the outer list, or un-listed at the
/// outermost level); other blocks (e.g. inside a blockquote) use the generic lift.
pub fn lift() -> Command {
    command_tr(build_lift_dispatch)
}

/// Build the lift transaction, choosing the shallowest schema-valid target depth.
pub(super) fn build_lift(state: &EditorState) -> Option<Transaction> {
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range_simple(&r_from, &r_to)?;
    if range.depth == 0 {
        return None;
    }
    // Never lift across an isolating boundary (a table cell): content lifted out of a
    // cell would tear it from its table. The floor is the deepest isolating ancestor
    // at-or-above the range's parent; only targets below that are allowed. If the
    // range's own parent is isolating, no lift applies. Mirrors PM's `liftTarget`.
    let floor = (1..=range.depth)
        .filter(|&d| r_from.node(d).node_type().is_isolating())
        .max()
        .unwrap_or(0);
    // Try lifting to each target from one level out (depth-1) down to `floor`; the
    // schema gate rejects invalid targets, so the first that applies wins.
    for target in (floor..range.depth).rev() {
        let mut tr = state.tr();
        if tr.lift(&range, target).is_ok() && tr.doc_changed() {
            return Some(tr);
        }
    }
    None
}

/// Outdent / lift for a **list item** — the list-aware port of PM's `liftListItem`.
///
/// The generic [`build_lift`] uses a *paragraph*-level range, so lifting a nested
/// item drops its paragraph into the previous item as a bullet-less continuation
/// block. This instead uses a *list*-level range (parent = the list, the range covers
/// the whole item), then dispatches like PM:
///
/// * **Nested in a parent list** (`liftToOuterList`): lift the item out by one list,
///   so it becomes a sibling in the outer list — the schema gate rejects the invalid
///   targets (a `list_item` can't hold a `list_item`) and keeps the one that lands it
///   in the outer list.
/// * **In the outermost list** (`liftOutOfList`): un-list it to top-level block(s) —
///   the generic paragraph-level lift already does exactly this, so reuse it.
pub(super) fn build_lift_list_item(state: &EditorState, item_type: &str) -> Option<Transaction> {
    let item_ty = state.schema().node_type(item_type)?.clone();
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range(&r_from, &r_to, |n| {
        n.child_count() > 0 && n.child(0).node_type() == &item_ty
    })?;
    if range.depth == 0 {
        return None;
    }
    // PM's dispatch: is the list itself wrapped in another item of the same type? If
    // not, this is the outermost list — un-list to the top level (generic lift). If so
    // (`in_parent_list`), the list nests inside an item inside an outer list, so
    // `range.depth >= 2` is guaranteed below.
    if r_from.node(range.depth - 1).node_type() != &item_ty {
        return build_lift(state);
    }

    // liftToOuterList. If the lifted item has trailing siblings in the same list,
    // first wrap them into a fresh item so they ride along NESTED under the lifted
    // item — otherwise the plain lift orphans them into a bullet-less wrapper. The
    // gap content (the trailing siblings) is dropped into the new item's empty
    // sublist at insert offset 1. Port of PM's `liftToOuterList`.
    let list_ty = range.parent().node_type().clone();
    let list_attrs = range.parent().attrs().clone();
    let from_pos = range.from.pos();
    let end = range.end();
    let end_of_list = r_to.end(range.depth);

    let mut tr = state.tr();
    let work = if end < end_of_list {
        let inner_list = Node::new_branch(list_ty, list_attrs, Fragment::empty());
        let new_item = Node::new_branch(
            item_ty.clone(),
            Attrs::new(),
            Fragment::from_node(inner_list),
        );
        let slice = crate::Slice::new(Fragment::from_node(new_item), 1, 0);
        tr.step(Box::new(ReplaceAroundStep::new(
            end - 1,
            end_of_list,
            end,
            end_of_list,
            slice,
            1,
            true,
        )))
        .ok()?;
        // Re-span the range over the lifted item plus its now-nested trailing siblings.
        // PM keeps the original `depth` and re-resolves the *raw* endpoints (the wrap
        // is balanced, so `end_of_list` still bounds the list's content).
        let nf = tr.doc().resolve(from_pos).ok()?;
        let nt = tr.doc().resolve(Pos(end_of_list)).ok()?;
        NodeRange {
            from: nf,
            to: nt,
            depth: range.depth,
        }
    } else {
        range
    };

    // Lift the (possibly extended) item range out by one list — `depth - 2` is the
    // outer list, so the item lands there as a sibling.
    let target = work.depth - 2;
    tr.lift(&work, target).ok()?;

    // After the lift, an item that had its OWN trailing sublist now sits directly
    // before the wrapped trailing-siblings' sublist — two adjacent lists of the same
    // type. Join them into one (PM's `canJoin` + `join`) so repeated outdents don't
    // fragment the list. The structural step self-guards against overwriting content.
    let boundary = tr.mapping().map(end, -1).saturating_sub(1);
    if can_join_sublists(tr.doc(), boundary) {
        let _ = tr.step(Box::new(ReplaceStep::structural(
            boundary - 1,
            boundary + 1,
            crate::Slice::empty(),
        )));
    }

    tr.doc_changed().then_some(tr)
}

/// PM's `canJoin`, specialised to the list-merge case: the block boundary at `pos`
/// sits between two same-type, non-textblock, non-leaf nodes (i.e. two adjacent
/// sublists). The structural join step itself rejects a schema-invalid merge, so this
/// only has to gate out unrelated boundaries (text, leaves, mismatched types).
fn can_join_sublists(doc: &Node, pos: usize) -> bool {
    let Ok(r) = doc.resolve(Pos(pos)) else {
        return false;
    };
    match (r.node_before(), r.node_after()) {
        (Some(before), Some(after)) => {
            !before.is_textblock() && !before.is_leaf() && before.type_name() == after.type_name()
        }
        _ => false,
    }
}

/// The list-aware lift used by both the `liftListItem` keymap and the Outdent button:
/// a real list/task item is lifted with [`build_lift_list_item`]; anything else
/// (e.g. a paragraph in a blockquote) falls back to the generic [`build_lift`].
pub(super) fn build_lift_dispatch(state: &EditorState) -> Option<Transaction> {
    if in_node_type(state, "list_item") {
        build_lift_list_item(state, "list_item")
    } else if in_node_type(state, "task_item") {
        build_lift_list_item(state, "task_item")
    } else {
        build_lift(state)
    }
}

/// The indent / sink-list-item command: nest the current list item under its
/// previous sibling. Faithful port of PM's `sinkListItem`, including the
/// `nestedBefore` merge so repeated indents keep nesting deeper instead of piling
/// up sibling sublists.
pub fn sink_list_item(item_type: &'static str) -> Command {
    command_tr(move |state| build_sink(state, item_type))
}

/// The Tab / sink-list-item command: nest the current list **or task** item under its
/// previous sibling. Unlike the Indent button (`indent`), a plain paragraph is left
/// untouched — Tab never margin-indents a textblock outside a list.
pub fn sink_list_item_any() -> Command {
    command_tr(build_sink_dispatch)
}

/// Sink the current item, picking `list_item` vs `task_item` from context. Returns
/// `None` outside any list (so Tab is a no-op there) or on a first item with nothing to
/// nest under.
pub(super) fn build_sink_dispatch(state: &EditorState) -> Option<Transaction> {
    if in_node_type(state, "list_item") {
        build_sink(state, "list_item")
    } else if in_node_type(state, "task_item") {
        build_sink(state, "task_item")
    } else {
        None
    }
}

/// Build the sink (indent) transaction.
pub(super) fn build_sink(state: &EditorState, item_type: &str) -> Option<Transaction> {
    let item_ty = state.schema().node_type(item_type)?.clone();
    let r_from = state.doc.resolve(state.selection.from()).ok()?;
    let r_to = state.doc.resolve(state.selection.to()).ok()?;
    let range = block_range(&r_from, &r_to, |n| {
        n.child_count() > 0 && n.child(0).node_type() == &item_ty
    })?;
    let start_index = range.start_index();
    if start_index == 0 {
        return None;
    }
    let parent = range.parent();
    let node_before = parent.child(start_index - 1);
    if node_before.node_type() != &item_ty {
        return None;
    }
    let list_ty = parent.node_type().clone();
    // If the previous item already ends with a sublist of this same list type, merge
    // the current item INTO that existing sublist rather than wrapping it in a fresh
    // adjacent one (PM's `nestedBefore`). Without this, a second indent leaves the
    // item the lone child of its own sublist — so it can never nest deeper, and the
    // doc grows a pile of sibling sublists instead of a clean tree.
    let nested_before = node_before.child_count() > 0
        && node_before.child(node_before.child_count() - 1).node_type() == &list_ty;
    // Slice shell: `item( list( inner ) )`. When merging, `inner` is a fresh empty
    // item and the slice opens 3 deep (item/list/item) so the gap content drops into
    // the previous item's existing sublist; otherwise it opens 1 deep (item).
    let inner = if nested_before {
        Fragment::from_node(Node::new_branch(
            item_ty.clone(),
            Attrs::new(),
            Fragment::empty(),
        ))
    } else {
        Fragment::empty()
    };
    let inner_list = Node::new_branch(list_ty, parent.attrs().clone(), inner);
    let new_item = Node::new_branch(
        item_ty.clone(),
        Attrs::new(),
        Fragment::from_node(inner_list),
    );
    let open_start = if nested_before { 3 } else { 1 };
    let slice = crate::Slice::new(Fragment::from_node(new_item), open_start, 0);
    let before = range.start();
    let after = range.end();
    let mut tr = state.tr();
    tr.step(Box::new(ReplaceAroundStep::new(
        before - open_start,
        after,
        before,
        after,
        slice,
        1,
        true,
    )))
    .ok()?;
    tr.doc_changed().then_some(tr)
}
