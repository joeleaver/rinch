//! Mark commands: toggling simple marks and links, and select-all.

use crate::command::Command;
use crate::model::{Attrs, Mark, MarkType, Node};
use crate::state::Transaction;

use super::queries::{ranges_have_mark, sel_ranges};
use super::{command_tr, marks_at};

// ===================================================================
// Marks
// ===================================================================

/// Toggle a simple (attribute-less) mark: bold, italic, underline, strike, code,
/// subscript, superscript, highlight. For a collapsed cursor this flips the stored
/// marks (the "click bold then type" path); for a range it adds or removes the
/// mark across it (remove if the range already has it anywhere — PM's toggle).
pub fn toggle_mark(mark_type: &'static str) -> Command {
    command_tr(move |state| {
        let mt = state.schema().mark_type(mark_type)?.clone();
        let sel = &state.selection;
        let mut tr = state.tr();
        if sel.is_empty() {
            let marks = state
                .stored_marks
                .clone()
                .unwrap_or_else(|| marks_at(state, sel.from().0));
            let mark = Mark::simple(mt.clone());
            if marks.iter().any(|m| m.typ == mt) {
                tr.remove_stored_mark(&mark);
            } else {
                tr.add_stored_mark(mark);
            }
        } else {
            // Mark steps move no position, so every range stays valid.
            let mark = Mark::simple(mt.clone());
            let remove = ranges_have_mark(state, &mt);
            for (from, to) in sel_ranges(state) {
                if remove {
                    tr.remove_mark(from, to, mark.clone()).ok()?;
                } else {
                    tr.add_mark(from, to, mark.clone()).ok()?;
                }
            }
        }
        Some(tr)
    })
}

/// Apply a `link` mark with `href` across the selection (no-op for a collapsed
/// cursor — a link needs a range). A builder (the href is supplied at call time).
pub fn toggle_link(href: String) -> Command {
    command_tr(move |state| {
        let mt = state.schema().mark_type("link")?.clone();
        let sel = &state.selection;
        if sel.is_empty() {
            return None;
        }
        let mut tr = state.tr();
        let remove = ranges_have_mark(state, &mt);
        for (from, to) in sel_ranges(state) {
            if remove {
                // Remove every link in the range, regardless of href.
                remove_marks_of_type(&mut tr, &state.doc, from, to, &mt).ok()?;
            } else {
                let attrs = Attrs::from_iter([("href", crate::AttrValue::from(href.clone()))]);
                tr.add_mark(from, to, Mark::new(mt.clone(), attrs)).ok()?;
            }
        }
        Some(tr)
    })
}

/// Remove all `link` marks from the selection.
pub fn remove_link() -> Command {
    command_tr(|state| {
        let mt = state.schema().mark_type("link")?.clone();
        if state.selection.is_empty() || !ranges_have_mark(state, &mt) {
            return None;
        }
        let mut tr = state.tr();
        for (from, to) in sel_ranges(state) {
            remove_marks_of_type(&mut tr, &state.doc, from, to, &mt).ok()?;
        }
        Some(tr)
    })
}

/// Select the entire document — the `Ctrl/Cmd+A` command. Returns a selection-only
/// transaction **unconditionally** (never gated on a change), so the key binding is
/// always consumed even when the whole document is already selected.
pub fn select_all() -> Command {
    command_tr(|state| {
        let from = crate::selection::Selection::at_start(&state.doc).head();
        let to = crate::selection::Selection::at_end(&state.doc).head();
        let mut tr = state.tr();
        tr.set_selection(crate::selection::Selection::text(from, to));
        Some(tr)
    })
}

/// Remove every mark of `mark_type` (any attrs) from `from..to`.
pub(super) fn remove_marks_of_type(
    tr: &mut Transaction,
    doc: &Node,
    from: usize,
    to: usize,
    mark_type: &MarkType,
) -> Result<(), crate::transform::StepError> {
    let mut targets: Vec<Mark> = Vec::new();
    doc.nodes_between(from, to, &mut |node, _pos, _parent| {
        for m in node.marks() {
            if &m.typ == mark_type && !targets.contains(m) {
                targets.push(m.clone());
            }
        }
        true
    });
    for m in targets {
        tr.remove_mark(from, to, m)?;
    }
    Ok(())
}
