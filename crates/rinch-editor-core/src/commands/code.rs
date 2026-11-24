//! Line breaks in code blocks (#2993): ports of ProseMirror's `newlineInCode`
//! and `exitCode` (prosemirror-commands), and the Shift+Enter chain built on
//! them.
//!
//! A code block is a node whose spec says [`code`](crate::NodeSpec::code) — the
//! starter kit's `code_block` (`text*`, no marks). It holds no `hard_break`, so
//! a line inside it is a `"\n"` character in its text: Enter and Shift+Enter
//! type one, and Mod+Enter leaves the block for a paragraph after it.

use crate::Fragment;
use crate::command::{Command, chain};
use crate::model::Attrs;
use crate::pos::{Pos, ResolvedPos};
use crate::selection::Selection;
use crate::state::EditorState;

use super::{command_tr, insert_hard_break};

/// The selection's head, resolved, when it is in a code block and the anchor is
/// in that same block (ProseMirror's `$head.parent.type.spec.code &&
/// $head.sameParent($anchor)`). `None` for any other selection — a range that
/// leaves the block, a node or cell selection, a caret in a plain textblock.
fn head_in_code(state: &EditorState) -> Option<ResolvedPos> {
    let head = state.doc.resolve(state.selection.head()).ok()?;
    let anchor = state.doc.resolve(state.selection.anchor()).ok()?;
    let same_parent =
        head.pos().0 - head.parent_offset() == anchor.pos().0 - anchor.parent_offset();
    (head.parent().node_type().spec().code && same_parent).then_some(head)
}

/// Type a newline (`"\n"`) in a code block, replacing the selection — Enter's
/// first step, and Shift+Enter's. Port of ProseMirror's `newlineInCode`: it
/// applies only when the selection's head is in a code block and the anchor is
/// in the same block; anywhere else it is inapplicable (`false`), and Enter's
/// chain goes on to split the block.
pub fn newline_in_code() -> Command {
    command_tr(|state| {
        head_in_code(state)?;
        let mut tr = state.tr();
        tr.insert_text("\n").ok()?;
        Some(tr)
    })
}

/// Leave a code block: put an empty paragraph right after the block and the
/// caret in it (Mod+Enter). Port of ProseMirror's `exitCode`, under the same
/// condition as [`newline_in_code`]; the selection is left as it is in the
/// code block, wherever in it the caret was. ProseMirror creates the default
/// block of the parent's content at that point; here it is a paragraph, and
/// the command is inapplicable where the parent does not take one.
pub fn exit_code() -> Command {
    command_tr(|state| {
        let head = head_in_code(state)?;
        let after = head.after(head.depth())?;
        let para = state
            .schema()
            .create_node("paragraph", Attrs::new(), Fragment::empty())
            .ok()?;
        let mut tr = state.tr();
        // The schema gate decides whether the parent takes a paragraph there.
        tr.replace_with(after, after, Fragment::from_node(para))
            .ok()?;
        let sel = Selection::near(tr.doc(), Pos(after), 1);
        tr.set_selection(sel);
        Some(tr)
    })
}

/// Shift+Enter: a newline in a code block ([`newline_in_code`]), a hard break
/// anywhere else ([`insert_hard_break`]). Named
/// `insertLineBreak`, after the `beforeinput` input type the web view maps to
/// it.
pub fn insert_line_break() -> Command {
    chain(vec![newline_in_code(), insert_hard_break()])
}
