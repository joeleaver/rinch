//! The history plugin — a single, Step-based, grouped, typing-merged undo/redo.
//!
//! This is the **only** undo system (design §5; it replaces the old engine's two
//! competing interpreters). History stores **inverted Steps**, not byte positions:
//! a recorded event is "the steps that, applied in reverse, revert this edit",
//! plus the selection to restore and a timestamp. Undo applies an event's inverse
//! steps as a transaction (`addToHistory=false`); redo re-applies the forward
//! steps. The same event machinery serves both branches.
//!
//! Grouping: consecutive **typing** transactions within a time window merge into a
//! single undo step (so a word is one undo, not one-per-keystroke); any non-typing
//! transaction (a mark, a block change, a split) is its own group, and an explicit
//! caret move breaks the current group. The time window is driven by
//! [`Transaction::set_time`]; with times unset (`0`), consecutive typing merges
//! (the default headless behavior), which tests pin by spacing timestamps apart.

use std::any::Any;
use std::rc::Rc;

use crate::command::Command;
use crate::keymap::KeyBinding;
use crate::model::Node;
use crate::plugin::{Plugin, PluginKey};
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::step::Step;
use crate::transform::steps::ReplaceStep;

/// The history plugin's key.
pub const HISTORY_KEY: PluginKey = PluginKey("history");
/// Transaction meta key marking an undo/redo (so the plugin shifts branches
/// instead of recording a new event).
const HISTORY_META: &str = "history";

/// Marks a transaction as an undo or redo for the history plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryAction {
    Undo,
    Redo,
}

/// One undo/redo step: the inverse steps (forward-collected; applied in reverse to
/// revert), the selection to restore, a timestamp, and whether it is a typing
/// group (mergeable).
#[derive(Clone)]
struct HistoryEvent {
    steps: Vec<Box<dyn Step>>,
    selection: Selection,
    time: u64,
    typing: bool,
}

/// One side of the history (the undo stack or the redo stack).
#[derive(Clone, Default)]
struct Branch {
    events: Vec<HistoryEvent>,
}

/// The history plugin's folded state.
#[derive(Clone, Default)]
pub struct HistoryState {
    done: Branch,
    undone: Branch,
    /// Set by a caret-only transaction; suppresses the next typing merge.
    break_group: bool,
}

impl HistoryState {
    /// Number of available undo steps.
    pub fn undo_depth(&self) -> usize {
        self.done.events.len()
    }
    /// Number of available redo steps.
    pub fn redo_depth(&self) -> usize {
        self.undone.events.len()
    }
}

/// Build a history event from a transaction's steps (inverting each against its
/// pre-step document) plus the selection of the state *before* the transaction.
fn event_from(tr: &Transaction, old: &EditorState) -> HistoryEvent {
    let mut steps = Vec::with_capacity(tr.steps().len());
    for (i, step) in tr.steps().iter().enumerate() {
        steps.push(step.invert(&tr.docs()[i]));
    }
    HistoryEvent {
        steps,
        selection: old.selection.clone(),
        time: tr.time(),
        typing: is_typing(tr),
    }
}

/// A transaction is "typing" if all its steps are non-structural replaces (text
/// insert/delete) — the class that should coalesce into one undo group.
fn is_typing(tr: &Transaction) -> bool {
    !tr.steps().is_empty()
        && tr.steps().iter().all(|s| {
            s.as_any()
                .downcast_ref::<ReplaceStep>()
                .is_some_and(|r| !r.structure)
        })
}

/// Whether a new typing event should merge into the last recorded done event.
fn should_merge(hist: &HistoryState, event: &HistoryEvent, delay: u64) -> bool {
    if hist.break_group || !event.typing {
        return false;
    }
    match hist.done.events.last() {
        Some(last) => last.typing && event.time >= last.time && event.time - last.time < delay,
        None => false,
    }
}

/// Append `event` onto the end of the last group (combine their inverse steps).
fn merge_into_last(branch: &mut Branch, event: HistoryEvent) {
    let last = branch.events.last_mut().expect("merge with no prior event");
    last.steps.extend(event.steps);
    last.time = event.time; // slide the window forward; keep the group's selection
}

/// The history plugin. `new_group_delay` is the typing-merge window in
/// milliseconds; `depth` caps the number of retained undo events.
#[derive(Debug)]
pub struct HistoryPlugin {
    new_group_delay: u64,
    depth: usize,
}

impl Default for HistoryPlugin {
    fn default() -> Self {
        HistoryPlugin {
            new_group_delay: 500,
            depth: 200,
        }
    }
}

impl HistoryPlugin {
    /// A history plugin with default grouping (500ms window, depth 200).
    pub fn new() -> HistoryPlugin {
        HistoryPlugin::default()
    }

    /// Override the typing-merge window (ms) and the retained-event cap.
    pub fn with(new_group_delay: u64, depth: usize) -> HistoryPlugin {
        HistoryPlugin {
            new_group_delay,
            depth,
        }
    }
}

impl Plugin for HistoryPlugin {
    fn key(&self) -> PluginKey {
        HISTORY_KEY
    }

    fn commands(&self) -> Vec<(&'static str, Command)> {
        vec![("undo", undo()), ("redo", redo())]
    }

    fn keymap(&self) -> Vec<(KeyBinding, &'static str)> {
        [
            ("Mod-z", "undo"),
            ("Mod-y", "redo"),
            ("Mod-Shift-z", "redo"),
        ]
        .into_iter()
        .filter_map(|(b, c)| KeyBinding::parse(b).map(|kb| (kb, c)))
        .collect()
    }

    fn init_state(&self, _doc: &Node) -> Option<Rc<dyn Any>> {
        Some(Rc::new(HistoryState::default()))
    }

    fn apply(
        &self,
        tr: &Transaction,
        old: &EditorState,
        _new_doc: &Node,
        prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        let mut hist = prev
            .and_then(|p| p.downcast_ref::<HistoryState>())
            .cloned()
            .unwrap_or_default();

        match tr.get_meta_as::<HistoryAction>(HISTORY_META) {
            Some(HistoryAction::Undo) => {
                // The undo command reverted `done.last`; record the redo event.
                // Inherit the reverted event's `typing` classification rather than
                // recomputing it: an event's inverse steps are always non-structural
                // (PM's `ReplaceStep.invert` drops the structure flag), so a redone
                // structural edit (e.g. a split) must NOT be reclassified as typing
                // and merged with the next keystroke.
                let reverted = hist.done.events.pop();
                let mut event = event_from(tr, old);
                if let Some(prev) = &reverted {
                    event.typing = prev.typing;
                }
                hist.undone.events.push(event);
            }
            Some(HistoryAction::Redo) => {
                let reverted = hist.undone.events.pop();
                let mut event = event_from(tr, old);
                if let Some(prev) = &reverted {
                    event.typing = prev.typing;
                }
                hist.done.events.push(event);
            }
            None => {
                if tr.add_to_history() && tr.doc_changed() {
                    let event = event_from(tr, old);
                    if should_merge(&hist, &event, self.new_group_delay) {
                        merge_into_last(&mut hist.done, event);
                    } else {
                        hist.done.events.push(event);
                    }
                    hist.break_group = false;
                    hist.undone.events.clear(); // a fresh edit invalidates redo
                } else if (tr.selection_set() || tr.stored_marks_set()) && !tr.doc_changed() {
                    // A caret-only move or a stored-mark toggle (e.g. cursor-bold)
                    // ends the current typing group, matching PM's storedMarksSet
                    // group break.
                    hist.break_group = true;
                }
            }
        }

        // Cap retained undo history.
        while hist.done.events.len() > self.depth {
            hist.done.events.remove(0);
        }
        Some(Rc::new(hist))
    }
}

/// Apply an event's inverse steps (in reverse) as a transaction, restoring the
/// event's selection and tagging it with `action` so the plugin shifts branches.
fn apply_event(
    state: &EditorState,
    event: &HistoryEvent,
    action: HistoryAction,
) -> Option<Transaction> {
    let mut tr = state.tr();
    for step in event.steps.iter().rev() {
        tr.step(step.clone()).ok()?;
    }
    tr.set_selection(event.selection.clone());
    tr.set_add_to_history(false);
    tr.set_meta(HISTORY_META, action);
    Some(tr)
}

/// The undo command: revert the most recent recorded event.
pub fn undo() -> Command {
    Rc::new(|state: &EditorState, dispatch| {
        // Build the transaction eagerly so the applicability query (`dispatch ==
        // None`) and the dispatch path share one code path (no "reports applicable
        // but then fails to dispatch" divergence — matching the `command_tr`
        // discipline elsewhere in the catalogue).
        let Some(hist) = state.plugin_state::<HistoryState>(HISTORY_KEY) else {
            return false;
        };
        let Some(event) = hist.done.events.last() else {
            return false;
        };
        let Some(tr) = apply_event(state, event, HistoryAction::Undo) else {
            return false;
        };
        if let Some(dispatch) = dispatch {
            dispatch(tr);
        }
        true
    })
}

/// The redo command: re-apply the most recently undone event.
pub fn redo() -> Command {
    Rc::new(|state: &EditorState, dispatch| {
        let Some(hist) = state.plugin_state::<HistoryState>(HISTORY_KEY) else {
            return false;
        };
        let Some(event) = hist.undone.events.last() else {
            return false;
        };
        let Some(tr) = apply_event(state, event, HistoryAction::Redo) else {
            return false;
        };
        if let Some(dispatch) = dispatch {
            dispatch(tr);
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::commands::BaseCommandsPlugin;
    use crate::model::Fragment;
    use crate::pos::Pos;

    fn editor(text: &str) -> EditorState {
        let s = Rc::new(Schema::starter_kit());
        let para = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        EditorState::create(
            s,
            doc,
            vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
        )
    }

    fn all_text(node: &Node) -> String {
        if let Some(t) = node.text() {
            return t.to_string();
        }
        node.content().iter().map(all_text).collect()
    }

    fn type_char(state: &EditorState, text: &str, time: u64) -> EditorState {
        let mut tr = state.tr();
        tr.set_time(time);
        tr.insert_text(text).unwrap();
        state.apply(tr)
    }

    #[test]
    fn undo_and_redo_a_single_edit() {
        let state = editor("ab");
        let typed = type_char(&state, "X", 0); // cursor at start (pos 1) → "Xab"
        assert_eq!(all_text(&typed.doc), "Xab");
        let undone = typed.run("undo").expect("undo applies");
        assert_eq!(all_text(&undone.doc), "ab");
        let redone = undone.run("redo").expect("redo applies");
        assert_eq!(all_text(&redone.doc), "Xab");
    }

    #[test]
    fn typing_coalesces_into_one_undo_group() {
        // three chars typed within the window merge into a single undo
        let state = editor("");
        let s1 = type_char(&state, "a", 0);
        let s2 = type_char(&s1, "b", 10);
        let s3 = type_char(&s2, "c", 20);
        assert_eq!(all_text(&s3.doc), "abc");
        let undone = s3.run("undo").expect("undo applies");
        assert_eq!(
            all_text(&undone.doc),
            "",
            "one undo reverts all coalesced typing"
        );
        // and a single redo restores all of it
        let redone = undone.run("redo").unwrap();
        assert_eq!(all_text(&redone.doc), "abc");
    }

    #[test]
    fn time_gap_breaks_typing_group() {
        let state = editor("");
        let s1 = type_char(&state, "a", 0);
        let s2 = type_char(&s1, "b", 1000); // beyond the 500ms window
        assert_eq!(all_text(&s2.doc), "ab");
        let u1 = s2.run("undo").unwrap();
        assert_eq!(all_text(&u1.doc), "a", "second char is its own group");
        let u2 = u1.run("undo").unwrap();
        assert_eq!(all_text(&u2.doc), "");
    }

    #[test]
    fn type_then_bold_undo_reverts_bold_only() {
        // design M4 test: typing then a mark are separate groups
        let state = editor("ab");
        let typed = type_char(&state, "X", 0); // "Xab", cursor at 2
        let mut bolded_state = typed.clone();
        bolded_state.selection = Selection::text(Pos(1), Pos(4)); // select "Xab"
        let bolded = bolded_state.run("toggleBold").expect("bold applies");
        assert!(
            bolded
                .doc
                .child(0)
                .content()
                .iter()
                .all(|n| n.marks().iter().any(|m| m.type_name() == "bold"))
        );
        // first undo removes the bold (its own group), text remains
        let u1 = bolded.run("undo").expect("undo bold");
        assert_eq!(all_text(&u1.doc), "Xab");
        assert!(!range_has_bold(&u1));
        // second undo removes the typed X
        let u2 = u1.run("undo").expect("undo typing");
        assert_eq!(all_text(&u2.doc), "ab");
    }

    fn range_has_bold(state: &EditorState) -> bool {
        let bold = state.schema().mark_type("bold").unwrap();
        crate::commands::range_has_mark(&state.doc, 0, state.doc.content_size(), bold)
    }

    #[test]
    fn new_edit_clears_redo() {
        let state = editor("");
        let s1 = type_char(&state, "a", 0);
        let undone = s1.run("undo").unwrap();
        assert!(undone.run("redo").is_some());
        // typing again should drop the redo stack
        let s2 = type_char(&undone, "b", 0);
        assert!(s2.run("redo").is_none(), "a fresh edit invalidates redo");
        assert_eq!(all_text(&s2.doc), "b");
    }

    #[test]
    fn undo_restores_selection() {
        let state = editor("ab");
        let mut s0 = state.clone();
        s0.selection = Selection::cursor(Pos(3)); // end of "ab"
        let typed = {
            let mut tr = s0.tr();
            tr.insert_text("Z").unwrap();
            s0.apply(tr)
        };
        assert_eq!(typed.selection, Selection::cursor(Pos(4)));
        let undone = typed.run("undo").unwrap();
        // selection restored to where it was before the edit (pos 3)
        assert_eq!(undone.selection, Selection::cursor(Pos(3)));
    }

    #[test]
    fn undo_unavailable_on_empty_history() {
        let state = editor("ab");
        assert!(!state.can_run("undo"));
        assert!(!state.can_run("redo"));
    }

    #[test]
    fn redone_split_does_not_merge_with_later_typing() {
        // verification finding #1: a redone structural edit (split) must keep its
        // non-typing classification so a following keystroke is its own undo group.
        let mut state = editor("ab");
        state.selection = Selection::cursor(Pos(2)); // a|b
        let split = state.run("splitBlock").expect("split");
        assert_eq!(split.doc.child_count(), 2);
        let undone = split.run("undo").expect("undo split");
        assert_eq!(undone.doc.child_count(), 1);
        let redone = undone.run("redo").expect("redo split");
        assert_eq!(redone.doc.child_count(), 2);
        // type after the redone split, within the merge window
        let typed = type_char(&redone, "Z", 0);
        let after = typed.run("undo").expect("undo typing");
        assert_eq!(
            after.doc.child_count(),
            2,
            "undo reverts only the typing, not the redone split"
        );
    }

    #[test]
    fn stored_mark_toggle_breaks_typing_group() {
        // verification finding #2: a cursor mark toggle between keystrokes ends the
        // typing group (PM storedMarksSet group break).
        let mut state = editor("");
        state.selection = Selection::cursor(Pos(1));
        let s1 = type_char(&state, "a", 0);
        let s2 = s1.run("toggleBold").expect("cursor bold (stored marks)");
        let s3 = type_char(&s2, "b", 10); // within the window
        assert_eq!(all_text(&s3.doc), "ab");
        let undone = s3.run("undo").expect("undo");
        assert_eq!(
            all_text(&undone.doc),
            "a",
            "only 'b' reverted, not the whole word"
        );
    }

    #[test]
    fn block_join_starts_new_undo_group() {
        // verification finding #4: a structural block join is its own undo group and
        // does not coalesce into preceding typing.
        let s = Rc::new(Schema::starter_kit());
        let p = |t: &str| {
            s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let doc = s
            .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
            .unwrap();
        let mut state = EditorState::create(
            s,
            doc,
            vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
        );
        state.selection = Selection::cursor(Pos(3)); // end of first paragraph
        let typed = type_char(&state, "Z", 0); // "abZ" / "cd"
        assert_eq!(typed.doc.child_count(), 2);
        let joined = typed.run("deleteCharForward").expect("forward join");
        assert_eq!(joined.doc.child_count(), 1);
        assert_eq!(all_text(&joined.doc), "abZcd");
        // undo reverts ONLY the join, keeping the typed Z
        let after = joined.run("undo").expect("undo join");
        assert_eq!(after.doc.child_count(), 2, "join is its own undo group");
        assert_eq!(
            all_text(&after.doc),
            "abZcd",
            "the typed Z survives the undo"
        );
    }
}
