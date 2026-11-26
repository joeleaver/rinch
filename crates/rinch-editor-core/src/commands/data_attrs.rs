//! An app's `data-*` attributes on the selection's textblocks
//! ([`crate::schema::data_attrs`]).

use crate::command::Command;
use crate::pos::Pos;
use crate::schema::data_attrs::node_app_data;
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::steps::SetNodeAttrStep;

use super::command_tr;
use super::queries::sel_ranges;

/// Set (`Some`) or remove (`None`) the app data attribute `name` on every
/// textblock of the selection that keeps data attributes (the starter kit's
/// paragraph and heading) — the caret's block, each block a range overlaps, or
/// every block of the selected table cells. One undo step.
///
/// Inapplicable when no block would change, and when the attribute is refused
/// ([`Transform::set_block_data_attr`](crate::transform::Transform::set_block_data_attr):
/// a reserved or invalid name, a value past the bound, a block that is full).
///
/// ```
/// use rinch_editor_core::commands::set_block_data_attr;
/// let style = set_block_data_attr("data-role", Some("subtitle"));
/// let clear = set_block_data_attr("data-role", None::<String>);
/// # let _ = (style, clear);
/// ```
pub fn set_block_data_attr(name: impl Into<String>, value: Option<impl Into<String>>) -> Command {
    let name = name.into();
    let value: Option<String> = value.map(Into::into);
    command_tr(move |state| {
        let mut tr = state.tr();
        for (from, to) in sel_ranges(state) {
            tr.set_block_data_attr(from, to, &name, value.as_deref())
                .ok()?;
        }
        tr.doc_changed().then_some(tr)
    })
}

/// The value of the app data attribute `name` that every textblock of the
/// selection which keeps data attributes holds: `None` when one of them lacks
/// it, when they differ, or when the selection has no such block. What a
/// toolbar shows as the current choice.
pub fn block_data_attr(state: &EditorState, name: &str) -> Option<String> {
    // `None`: no block seen yet. `Some(None)`: the blocks do not agree on one.
    let mut shared: Option<Option<String>> = None;
    for (from, to) in sel_ranges(state) {
        state
            .doc
            .nodes_between(from, to, &mut |node, _pos, _parent| {
                if node.is_textblock() && node.node_type().spec().data_attrs {
                    let own = node_app_data(node)
                        .find(|(n, _)| *n == name)
                        .map(|(_, v)| v.to_string());
                    match &shared {
                        None => shared = Some(own),
                        Some(seen) if *seen != own => shared = Some(None),
                        Some(_) => {}
                    }
                }
                true
            });
    }
    shared.flatten()
}

/// Remove every app data attribute of the node at `pos`.
/// Delete the empty textblock before `cut` instead of joining the block after
/// it into it, where either holds app data attributes: a join keeps the first
/// block's markup, which would hand the second one an empty block's attributes
/// (or take its own). `None` where neither block holds any, or they are not two
/// textblocks with the first empty: the ordinary join.
pub(super) fn delete_empty_block_before(state: &EditorState, cut: usize) -> Option<Transaction> {
    let r = state.doc.resolve(Pos(cut)).ok()?;
    let (before, after) = (r.node_before()?, r.node_after()?);
    let applies = before.is_textblock()
        && after.is_textblock()
        && before.content().size() == 0
        && (node_app_data(&before).next().is_some() || node_app_data(&after).next().is_some());
    if !applies {
        return None;
    }
    let from = cut - before.node_size();
    let mut tr = state.tr();
    tr.delete(from, cut).ok()?;
    tr.set_selection(Selection::text(Pos(from + 1), Pos(from + 1)));
    tr.doc_changed().then_some(tr)
}

pub(super) fn remove_app_data(tr: &mut Transaction, pos: usize) -> Option<()> {
    let names: Vec<String> = node_app_data(&tr.doc().node_at(pos)?)
        .map(|(name, _)| name.to_string())
        .collect();
    for attr in names {
        tr.step(Box::new(SetNodeAttrStep {
            pos,
            attr,
            value: None,
        }))
        .ok()?;
    }
    Some(())
}
