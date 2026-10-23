//! The history plugin — a single, Step-based, grouped, typing-merged undo/redo.
//!
//! This is the **only** undo system (design §5; it replaces the old engine's two
//! competing interpreters). History stores **inverted Steps**, not byte positions:
//! a recorded event is "the steps that, applied in reverse, revert this edit",
//! plus the selection to restore and a timestamp. Undo applies an event's inverse
//! steps as a transaction (`addToHistory=false`); redo re-applies the forward
//! steps. The same event machinery serves both branches.
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
//! Grouping: consecutive **typing** transactions within a time window merge into a
//! single undo step (so a word is one undo, not one-per-keystroke); any non-typing
//! transaction (a mark, a block change, a split) is its own group, and an explicit
//! caret move breaks the current group. The time window is driven by
//! [`Transaction::set_time`]: the core reads no clock, and `rinch-editor-view`'s
//! `EditorHandle` stamps every transaction it dispatches (#940). With times unset
//! (`0`, a headless state driven directly), consecutive typing always merges,
//! which tests pin by spacing timestamps apart.

use std::any::Any;
use std::rc::Rc;

use crate::command::Command;
use crate::keymap::KeyBinding;
use crate::model::Node;
use crate::plugin::{Plugin, PluginKey};
use crate::selection::Selection;
use crate::state::{EditorState, Transaction};
use crate::transform::step::Step;
use crate::transform::step_map::{Mapping, StepMap};
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

/// An undo/redo transaction's meta: which one it is, the branch it popped with
/// that event removed (and the maps of what the undo did appended, so older
/// events still map through it), and the popped event's classification.
#[derive(Clone)]
struct HistoryMeta {
    action: HistoryAction,
    remaining: Branch,
    typing: bool,
}

/// What an event's first (oldest) item carries: the selection to restore, the
/// time of its last recorded transaction, and whether it is a typing group
/// (mergeable).
#[derive(Clone)]
struct EventInfo {
    selection: Selection,
    time: u64,
    typing: bool,
}

/// One entry of a branch: the forward step map of a step, and for a recorded
/// step its inverse. A map-only item (`step: None`) is a change the history did
/// not record, or the map of a step an undo applied.
#[derive(Clone)]
struct Item {
    map: StepMap,
    step: Option<Box<dyn Step>>,
    /// `Some` on the first item of an event.
    event: Option<EventInfo>,
    /// How many items back the item whose map this one inverts sits (PM's
    /// `mirrorOffset`), so mapping through both recovers a deleted position.
    mirror_offset: Option<usize>,
}

impl Item {
    fn map_only(map: StepMap, mirror_offset: Option<usize>) -> Item {
        Item {
            map,
            step: None,
            event: None,
            mirror_offset,
        }
    }
}

/// One side of the history (the undo stack or the redo stack). Port of
/// prosemirror-history's `Branch`, without its item compression.
#[derive(Clone, Default)]
struct Branch {
    items: Vec<Rc<Item>>,
    event_count: usize,
}

impl Branch {
    /// The newest event's info.
    fn last_event(&self) -> Option<&EventInfo> {
        if self.event_count == 0 {
            return None;
        }
        self.items.iter().rev().find_map(|i| i.event.as_ref())
    }

    /// Record `tr`'s steps (inverted against their pre-step documents). `event`
    /// starts a new event; `None` appends them to the newest one.
    fn add_transform(&mut self, tr: &Transaction, mut event: Option<EventInfo>) {
        for (i, step) in tr.steps().iter().enumerate() {
            let info = event.take();
            if info.is_some() {
                self.event_count += 1;
            }
            self.items.push(Rc::new(Item {
                map: step.get_map(),
                step: Some(step.invert(&tr.docs()[i])),
                event: info,
                mirror_offset: None,
            }));
        }
    }

    /// Record a change this branch's events must be mapped through (PM's
    /// `addMaps`). Nothing to map when the branch holds no event.
    fn add_maps(&mut self, maps: &[StepMap]) {
        if self.event_count == 0 {
            return;
        }
        self.items.extend(
            maps.iter()
                .map(|m| Rc::new(Item::map_only(m.clone(), None))),
        );
    }

    /// The maps of items `from..to`, with their mirror relations (PM's
    /// `remapping`).
    fn remapping(&self, from: usize, to: usize) -> Mapping {
        let mut maps = Mapping::new();
        for i in from..to {
            let item = &self.items[i];
            let mirror = item
                .mirror_offset
                .filter(|&off| i >= from + off)
                .map(|off| maps.len() - off);
            match mirror {
                Some(m) => maps.append_map_mirrored(item.map.clone(), m),
                None => maps.append_map(item.map.clone()),
            }
        }
        maps
    }

    /// Drop the oldest events (and any maps before them) until at most `depth`
    /// remain.
    fn cut_to_depth(&mut self, depth: usize) {
        if self.event_count <= depth {
            return;
        }
        let mut skip = self.event_count - depth;
        let cut = self
            .items
            .iter()
            .position(|i| {
                if i.event.is_some() {
                    if skip == 0 {
                        return true;
                    }
                    skip -= 1;
                }
                false
            })
            .unwrap_or(self.items.len());
        self.items.drain(..cut);
        self.event_count = depth;
    }

    /// Build the transaction that reverts the newest event (PM's `popEvent`).
    /// Each inverse step is mapped through every map recorded after it — a
    /// change the history did not record, and the steps this undo has already
    /// applied (mirrored, so a position the event's own step deleted and its
    /// inverse re-inserted is recovered). A step that no longer maps or applies
    /// is skipped. Returns the transaction (selection not set yet), the
    /// selection to restore, the branch without the event, and the event.
    fn pop_event(
        &self,
        state: &EditorState,
    ) -> Option<(Transaction, Selection, Branch, EventInfo)> {
        if self.event_count == 0 {
            return None;
        }
        let len = self.items.len();
        let end = (0..len).rev().find(|&i| self.items[i].event.is_some())?;
        let mut tr = state.tr();
        let mut remap: Option<Mapping> = None;
        let mut map_from = 0usize;
        let mut add_before: Vec<Rc<Item>> = Vec::new();
        let mut add_after: Vec<Rc<Item>> = Vec::new();
        for i in (end..len).rev() {
            let item = &self.items[i];
            let Some(step) = &item.step else {
                if remap.is_none() {
                    let m = self.remapping(end, i + 1);
                    map_from = m.len();
                    remap = Some(m);
                }
                map_from -= 1;
                add_before.push(item.clone());
                continue;
            };
            if let Some(remap) = remap.as_mut() {
                add_before.push(Rc::new(Item::map_only(item.map.clone(), None)));
                let mut applied = None;
                if let Some(mapped) = step.map(&remap.slice(map_from, remap.len()))
                    && tr.step(mapped).is_ok()
                {
                    let map = tr
                        .mapping()
                        .maps()
                        .last()
                        .cloned()
                        .unwrap_or_else(StepMap::empty);
                    let offset = add_after.len() + add_before.len();
                    add_after.push(Rc::new(Item::map_only(map.clone(), Some(offset))));
                    applied = Some(map);
                }
                map_from -= 1;
                if let Some(map) = applied {
                    remap.append_map_mirrored(map, map_from);
                }
            } else {
                // Nothing recorded after this step: it applies where it was made.
                // A step that fails is skipped, as PM's `maybeStep` does.
                let _ = tr.step(step.clone());
            }
            if let Some(info) = &item.event {
                let selection = match &remap {
                    Some(r) => info.selection.map(tr.doc(), &r.slice(map_from, r.len())),
                    None => info.selection.clone(),
                };
                let event_count = self.event_count - 1;
                let mut items = if event_count == 0 {
                    Vec::new()
                } else {
                    self.items[..end].to_vec()
                };
                if event_count > 0 {
                    add_before.reverse();
                    items.extend(add_before);
                    items.extend(add_after);
                }
                let remaining = Branch { items, event_count };
                return Some((tr, selection, remaining, info.clone()));
            }
        }
        None
    }
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
        self.done.event_count
    }
    /// Number of available redo steps.
    pub fn redo_depth(&self) -> usize {
        self.undone.event_count
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
    if hist.break_group || !typing {
        return false;
    }
    match hist.done.last_event() {
        Some(last) => last.typing && time >= last.time && time - last.time < delay,
        None => false,
    }
}

/// Slide the newest done event's window forward to `time` (the group keeps its
/// selection).
fn touch_last_event(branch: &mut Branch, time: u64) {
    if let Some(item) = branch.items.iter_mut().rev().find(|i| i.event.is_some())
        && let Some(info) = &mut Rc::make_mut(item).event
    {
        info.time = time;
    }
}

/// The maps the history keeps for a change it does not record: each step's map,
/// narrowed for a replace to the range the documents before and after it
/// actually differ in (PM's `findDiffStart` / `findDiffEnd`, as its view reads a
/// DOM change).
///
/// A collaboration peer's edit arrives as a replace of every block it changed
/// (`rinch-editor-collab`'s remote integration), and a position inside a replaced
/// range has nowhere to map: an undo step there is dropped. Narrowed to what
/// changed — the char a peer typed — a step of mine elsewhere in that paragraph
/// maps past it and still undoes. Everything outside the narrowed range is the
/// same content in both documents, so the map is exact for it.
fn unrecorded_maps(tr: &Transaction) -> Vec<StepMap> {
    let docs = tr.docs();
    tr.steps()
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let map = step.get_map();
            let Some(r) = step.as_any().downcast_ref::<ReplaceStep>() else {
                return map;
            };
            let before = &docs[i];
            let after = docs.get(i + 1).unwrap_or(tr.doc());
            let (a, b) = (before.content(), after.content());
            let Some(mut start) = a.find_diff_start(b, 0) else {
                return StepMap::empty();
            };
            let Some((mut end_a, mut end_b)) = a.find_diff_end(b, a.size(), b.size()) else {
                return StepMap::empty();
            };
            // The change repeats the content beside it (`ab` → `abab`): the two
            // scans overlap, and the range is placed at `start` (PM's `findDiff`).
            if end_a < start && a.size() < b.size() {
                end_b = start + (end_b - end_a);
                end_a = start;
            } else if end_b < start {
                end_a = start + (end_a - end_b);
                end_b = start;
            }
            start = start.min(end_a);
            if end_a - start < r.to - r.from {
                StepMap::new(vec![start, end_a - start, end_b - start])
            } else {
                map
            }
        })
        .collect()
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
                    time: tr.time(),
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
                    let time = tr.time();
                    if should_merge(&hist, typing, time, self.new_group_delay) {
                        hist.done.add_transform(tr, None);
                        touch_last_event(&mut hist.done, time);
                    } else {
                        let event = EventInfo {
                            selection: old.selection.clone(),
                            time,
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
                        hist.done.add_maps(&maps);
                        hist.undone.add_maps(&maps);
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
    let (mut tr, selection, remaining, event) = branch.pop_event(state)?;
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
