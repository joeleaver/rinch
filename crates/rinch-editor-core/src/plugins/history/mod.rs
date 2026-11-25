//! The history plugin — a single, Step-based, grouped, typing-merged undo/redo.
//!
//! This is the **only** undo system (design §5; it replaces the old engine's two
//! competing interpreters). History stores **inverted Steps**, not byte positions:
//! a recorded event is "the steps that, applied in reverse, revert this edit",
//! plus the selection to restore. Undo applies an event's inverse steps as a
//! transaction (`addToHistory=false`); redo re-applies the forward steps. The same
//! event machinery serves both branches.
//!
//! **Changes the history does not record** (a transaction with `addToHistory=false`
//! that changes the document: a collaboration peer's edit, a cure) are kept as
//! their step maps, and an undo maps each inverse step through every change made
//! after it before applying it. This is ProseMirror's history (`Branch.addMaps`,
//! `Branch.popEvent`): a branch is a list of items, each the step map of one step
//! plus, for a recorded step, its inverse, and the undo walks the popped event's
//! items newest first, building the mapping as it goes. A step that no longer maps
//! (a peer deleted all of its range) or no longer applies is dropped, and the rest
//! of the event still undoes. Before #1249 such a change was ignored, so an undo
//! replayed its steps at their old positions — in a peer's text.
//!
//! **Cost.** A branch is a persistent list (newest item first), so the state
//! copy every transaction makes is a pointer copy, and recording a step or a
//! peer's change is O(1). A peer's change folds into the peer item before it when
//! that is exact ([`StepMap::compose_exact`]; a run of one-char deletions is kept
//! as a count), so a peer typing or backspacing for an hour is one item; a change
//! that does not fold (an insertion beside a deletion) is one small item more.
//! Events past the depth cap are hidden at once and dropped from the list in a
//! batch, with the items before them. [`__history_work`] counts the items each
//! operation touches; the pins in `tests/history_remote_1249.rs` hold it flat.
//!
//! Grouping: consecutive **typing** transactions within a time window merge into a
//! single undo step (so a word is one undo, not one-per-keystroke); any non-typing
//! transaction (a mark, a block change, a split) is its own group, and an explicit
//! caret move breaks the current group. The time window is driven by
//! [`Transaction::set_time`]: the core reads no clock, and `rinch-editor-view`'s
//! `EditorHandle` stamps every transaction it dispatches (#940). With times unset
//! (`0`, a headless state driven directly), consecutive typing always merges,
//! which tests pin by spacing timestamps apart.
//!
//! [`StepMap::compose_exact`]: crate::transform::step_map::StepMap::compose_exact

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use crate::command::Command;
use crate::keymap::KeyBinding;
use crate::model::Node;
use crate::plugin::{Plugin, PluginKey};
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::steps::ReplaceStep;

mod branch;
mod unrecorded;

use branch::Branch;
use unrecorded::unrecorded_maps;

/// The history plugin's key.
pub const HISTORY_KEY: PluginKey = PluginKey("history");
/// Transaction meta key marking an undo/redo (so the plugin shifts branches
/// instead of recording a new event).
const HISTORY_META: &str = "history";
/// How many events past the depth cap a branch hides before it drops them from
/// its list in one pass (ProseMirror's `DEPTH_OVERFLOW`).
const DEPTH_OVERFLOW: usize = 20;

thread_local! {
    static HISTORY_WORK: Cell<u64> = const { Cell::new(0) };
}

fn work(n: usize) {
    HISTORY_WORK.with(|w| w.set(w.get() + n as u64));
}

/// How many history items this thread's history has created or walked. A work
/// counter for the cost pins (#1249): recording a keystroke or a peer's change
/// touches a fixed number of items, whatever the history holds. Not part of the
/// API.
#[doc(hidden)]
pub fn __history_work() -> u64 {
    HISTORY_WORK.with(|w| w.get())
}

/// Marks a transaction as an undo or redo for the history plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryAction {
    Undo,
    Redo,
}

/// An undo/redo transaction's meta: which one it is, the branch it popped with
/// that event removed (and the maps of what the undo did appended, so older
/// events still map through it), and the popped event's classification.
#[derive(Clone)]
struct HistoryMeta {
    action: HistoryAction,
    remaining: Branch,
    typing: bool,
}

/// What an event's first (oldest) item carries: the selection to restore and
/// whether it is a typing group.
#[derive(Clone)]
struct EventInfo {
    selection: Selection,
    typing: bool,
}

/// What [`Branch::pop_event`] answers: the transaction, the selection to
/// restore, the branch without the event, the event, and whether a step was
/// mapped through a change the history did not record.
type Popped = (Transaction, Selection, Branch, EventInfo, bool);

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
        self.done.event_count
    }
    /// Number of available redo steps.
    pub fn redo_depth(&self) -> usize {
        self.undone.event_count
    }

    /// How many items both branches hold — steps, peer changes and hidden events
    /// not yet dropped. A cost pin reads it (#1249); not part of the API.
    #[doc(hidden)]
    pub fn __stored_items(&self) -> usize {
        let count = |b: &Branch| {
            let mut n = 0;
            let mut cur = b.items.0.as_deref();
            while let Some(c) = cur {
                n += 1;
                cur = c.next.0.as_deref();
            }
            n
        };
        count(&self.done) + count(&self.undone)
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

/// Whether a new typing transaction at `time` should merge into the last
/// recorded done event.
fn should_merge(hist: &HistoryState, typing: bool, time: u64, delay: u64) -> bool {
    if hist.break_group || !typing || hist.done.event_count == 0 {
        return false;
    }
    match hist.done.last {
        Some((last_time, last_typing)) => {
            last_typing && time >= last_time && time - last_time < delay
        }
        None => false,
    }
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

    /// Chrome's: Mod-z undoes, Mod-Shift-z redoes, and Mod-y redoes too off
    /// macOS (Ctrl+Y on Windows and Linux) — see
    /// [`keys_not_on_macos`](Plugin::keys_not_on_macos).
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

    /// Mod-y is not a macOS chord: Cmd+Y opens History in Chrome there, and
    /// redo is Cmd+Shift+Z only (#3062).
    fn keys_not_on_macos(&self) -> Vec<KeyBinding> {
        KeyBinding::parse("Mod-y").into_iter().collect()
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

        let meta = tr
            .get_meta(HISTORY_META)
            .and_then(|m| m.downcast_ref::<HistoryMeta>());
        match meta {
            Some(meta) => {
                // The command reverted the newest event of one branch; record what
                // it did as an event on the other. The new event inherits the
                // reverted event's `typing` classification rather than recomputing
                // it: an event's inverse steps are always non-structural (PM's
                // `ReplaceStep.invert` drops the structure flag), so a redone
                // structural edit (e.g. a split) must NOT be reclassified as typing
                // and merged with the next keystroke.
                let event = EventInfo {
                    selection: old.selection.clone(),
                    typing: meta.typing,
                };
                match meta.action {
                    HistoryAction::Undo => {
                        hist.done = meta.remaining.clone();
                        hist.undone.add_transform(tr, Some(event));
                    }
                    HistoryAction::Redo => {
                        hist.undone = meta.remaining.clone();
                        hist.done.add_transform(tr, Some(event));
                    }
                }
                // The next edit starts a step of its own, as ProseMirror's
                // `prevTime = 0` makes it: otherwise typing within the window of
                // the group now on top of `done` joins it. (After a redo this was
                // already true — a redo needs an undo before it, which set it, and
                // only a recorded edit, which empties `undone`, clears it — and is
                // set here so the rule does not rest on that.)
                hist.break_group = true;
            }
            None => {
                if tr.add_to_history() && tr.doc_changed() {
                    let typing = is_typing(tr);
                    if should_merge(&hist, typing, tr.time(), self.new_group_delay) {
                        hist.done.add_transform(tr, None);
                    } else {
                        let event = EventInfo {
                            selection: old.selection.clone(),
                            typing,
                        };
                        hist.done.add_transform(tr, Some(event));
                    }
                    hist.break_group = false;
                    hist.undone = Branch::default(); // a fresh edit invalidates redo
                } else if tr.doc_changed() {
                    // A change the history does not record (a peer's edit): both
                    // branches' events are mapped through it when they are undone.
                    if hist.done.event_count > 0 || hist.undone.event_count > 0 {
                        let maps = unrecorded_maps(tr);
                        if maps.is_empty() {
                            hist.done.note_unmapped();
                            hist.undone.note_unmapped();
                        }
                        hist.done.add_maps(&maps);
                        hist.undone.add_maps(&maps);
                    } else {
                        hist.done = Branch::default();
                        hist.undone = Branch::default();
                    }
                } else if tr.selection_set() || tr.stored_marks_set() {
                    // A caret-only move or a stored-mark toggle (e.g. cursor-bold)
                    // ends the current typing group, matching PM's storedMarksSet
                    // group break.
                    hist.break_group = true;
                }
            }
        }

        // Cap retained undo history.
        hist.done.cut_to_depth(self.depth);
        Some(Rc::new(hist))
    }
}

/// Revert the newest event of `done` (undo) or `undone` (redo) as a transaction,
/// restoring the event's selection and tagging it so the plugin shifts branches.
fn hist_transaction(state: &EditorState, action: HistoryAction) -> Option<Transaction> {
    let hist = state.plugin_state::<HistoryState>(HISTORY_KEY)?;
    let branch = match action {
        HistoryAction::Undo => &hist.done,
        HistoryAction::Redo => &hist.undone,
    };
    let (mut tr, selection, mut remaining, event, remapped) = branch.pop_event(state)?;
    // A table the undo left ragged is repaired here, in its transaction (#1937),
    // and the selection follows the repair. Only an undo whose steps were mapped
    // through a change the history did not record (a peer's) can leave one: one
    // that was not applies the recorded inverses to the very documents they were
    // made for, and brings back the document before the event, table for table.
    // So a local undo checks nothing, whatever the table's size.
    let steps = tr.steps().len();
    if remapped || crate::state::transaction::repair_every_undo() {
        // A repair that fails part way leaves the steps it made; none should.
        let _ = tr.repair_tables_after_history();
    }
    let selection = if tr.steps().len() > steps {
        // The repair moved positions after the branch's last item: the events
        // left on it are mapped through it too, as through a peer's change, or
        // the next undo lands its steps where the repair's cells now are
        // (#1940: an undo of typing in a cell a merge emptied, after a peer's
        // merge, missed the typed char). A repair step that moved nothing (a
        // span set) is marked as a peer's attribute change is
        // ([`Branch::note_unmapped`]).
        remaining.add_repair_maps(&tr.mapping().maps()[steps..]);
        selection.map(tr.doc(), &tr.mapping().slice(steps, tr.mapping().len()))
    } else {
        selection
    };
    tr.set_selection(selection);
    tr.set_add_to_history(false);
    tr.set_meta(
        HISTORY_META,
        HistoryMeta {
            action,
            remaining,
            typing: event.typing,
        },
    );
    Some(tr)
}

/// A history command. The transaction is built eagerly so the applicability
/// query (`dispatch == None`) and the dispatch path share one code path (no
/// "reports applicable but then fails to dispatch" divergence — matching the
/// `command_tr` discipline elsewhere in the catalogue). An event whose steps all
/// failed to map (a peer deleted everything it touched) still pops: the command
/// applies and changes nothing, as ProseMirror's does.
fn history_command(action: HistoryAction) -> Command {
    Rc::new(move |state: &EditorState, dispatch| {
        let Some(tr) = hist_transaction(state, action) else {
            return false;
        };
        if let Some(dispatch) = dispatch {
            dispatch(tr);
        }
        true
    })
}

/// The undo command: revert the most recent recorded event.
pub fn undo() -> Command {
    history_command(HistoryAction::Undo)
}

/// The redo command: re-apply the most recently undone event.
pub fn redo() -> Command {
    history_command(HistoryAction::Redo)
}

#[cfg(test)]
mod tests;
