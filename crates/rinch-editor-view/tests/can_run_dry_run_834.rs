//! Issue #834: `EditorHandle::can_run` is a query. It answers from the
//! command's applicability branch (`cmd(state, None)`) and never runs its
//! dispatch branch — on an editable editor or a read-only one — so an
//! app-supplied command whose dispatch branch does something of its own never
//! sees it fire from a toolbar asking, and no plugin folds a transaction that
//! nobody commits.

use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{
    Command, EditorState, Node, Plugin, PluginKey, Pos, Selection, Transaction,
};
use rinch_editor_view::{EditorHandle, create_editor};
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

/// What the test plugin saw: how many times each command's **dispatch**
/// branch ran, how many times its applicability branch ran, and how many
/// transactions were folded through `Plugin::apply`.
#[derive(Default)]
struct Seen {
    dispatched: Cell<u32>,
    asked: Cell<u32>,
    folded: Cell<u32>,
}

struct Counting(Rc<Seen>);

impl Plugin for Counting {
    fn key(&self) -> PluginKey {
        PluginKey("test.counting-834")
    }

    fn commands(&self) -> Vec<(&'static str, Command)> {
        let seen = self.0.clone();
        // A document-changing command: inserts "!" at the caret.
        let edit: Command = Rc::new(move |state: &EditorState, dispatch| {
            let Some(dispatch) = dispatch else {
                seen.asked.set(seen.asked.get() + 1);
                return true;
            };
            seen.dispatched.set(seen.dispatched.get() + 1);
            let mut tr = state.tr();
            tr.insert_text("!").expect("insert");
            dispatch(tr);
            true
        });
        vec![("test.bang834", edit)]
    }

    fn apply(
        &self,
        _tr: &Transaction,
        _old: &EditorState,
        _new_doc: &Node,
        _prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        self.0.folded.set(self.0.folded.get() + 1);
        None
    }
}

fn editor() -> (EditorHandle, Rc<Seen>) {
    let e = create_editor();
    let seen = Rc::new(Seen::default());
    assert!(e.add_plugin(Rc::new(Counting(seen.clone()))));
    assert!(e.load_html("<p>hello world</p>"));
    e.set_selection(Selection::cursor(Pos(3)));
    (e, seen)
}

/// The positive control: the counters count. `command` runs the dispatch
/// branch once and folds the transaction it built.
#[test]
fn control_command_runs_the_dispatch_branch_and_folds() {
    let (e, seen) = editor();
    let folded = seen.folded.get();
    assert!(e.command("test.bang834"));
    assert_eq!(seen.dispatched.get(), 1);
    assert!(seen.folded.get() > folded, "the edit was folded");
    assert_eq!(node_to_html(&e.doc()), "<p>he!llo world</p>");
}

/// Editable: `can_run` asks, and only asks.
#[test]
fn editable_can_run_never_dispatches() {
    let (e, seen) = editor();
    let folded = seen.folded.get();
    for _ in 0..3 {
        assert!(e.can_run("test.bang834"));
    }
    assert_eq!(seen.dispatched.get(), 0, "a query ran the dispatch branch");
    assert_eq!(seen.folded.get(), folded, "a query folded a transaction");
    assert_eq!(node_to_html(&e.doc()), "<p>hello world</p>");
}

/// Read-only: the document-changing command reports `false`, and answering
/// that ran neither its dispatch branch nor any plugin's fold — the issue's
/// repro (it used to build, apply and fold the transaction, then discard it).
#[test]
fn read_only_can_run_never_dispatches() {
    let (e, seen) = editor();
    e.set_read_only(true);
    let folded = seen.folded.get();
    let (doc, selection) = (e.doc(), e.selection());
    for _ in 0..3 {
        assert!(!e.can_run("test.bang834"), "an edit is unavailable read-only");
    }
    for name in ["toggleBold", "setHeading1", "undo", "enter"] {
        assert!(!e.can_run(name), "{name} is unavailable read-only");
    }
    assert_eq!(seen.dispatched.get(), 0, "a query ran the dispatch branch");
    assert_eq!(seen.folded.get(), folded, "a query folded a transaction");
    assert!(e.doc().same_ref(&doc) && e.selection() == selection);
    assert!(!e.command("test.bang834"), "and running it is refused");
    assert_eq!(node_to_html(&e.doc()), "<p>hello world</p>");
}

