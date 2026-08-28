//! Editor state: the single source of truth, and the transaction that evolves it.
//!
//! [`EditorState`] is an immutable value `{ doc, selection, stored_marks, plugin
//! state }`; the only way to change it is to [`apply`](EditorState::apply) a
//! [`Transaction`], producing a *new* state. This is design §5's non-negotiable
//! principle #1 (single source of truth) and #2 (all changes are steps). `apply`
//! is **pure and side-effect-free**: no DOM, no window, no thread-locals.

pub mod transaction;

pub use transaction::{ADD_TO_HISTORY, Transaction};

use crate::command::{Command, Dispatch};
use crate::decoration::DecorationSet;
use crate::input_rules::InputRule;
use crate::keymap::Keymap;
use crate::model::{Mark, Node};
use crate::plugin::{Plugin, PluginKey};
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::Mapping;
use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

/// The immutable, shared configuration of an editor: its schema, plugins, and the
/// command / keymap / input-rule tables aggregated from those plugins once at
/// construction. Shared (via `Rc`) by every state derived from the initial one, so
/// applying a transaction never re-walks the plugin list to rebuild these.
struct Config {
    schema: Rc<Schema>,
    plugins: Vec<Rc<dyn Plugin>>,
    commands: HashMap<String, Command>,
    keymap: Keymap,
    input_rules: Vec<InputRule>,
}

/// The editor's entire state at one instant: the document, the selection, the
/// pending stored marks, and each plugin's folded state. Immutable —
/// [`apply`](EditorState::apply) returns a *new* `EditorState`.
#[derive(Clone)]
pub struct EditorState {
    config: Rc<Config>,
    /// The document (root node).
    pub doc: Node,
    /// The current selection (the caret is rendered from here, never from the host).
    pub selection: Selection,
    /// Marks the next typed text will carry (the "click bold then type" tri-state):
    /// `None` = inherit, `Some(vec![])` = explicitly none, `Some([bold])` = bold.
    pub stored_marks: Option<Vec<Mark>>,
    plugin_state: HashMap<PluginKey, Rc<dyn Any>>,
}

impl EditorState {
    /// Create a fresh state for `doc` with `plugins`, placing the selection at the
    /// start of the document and aggregating the plugins' commands / keymap / input
    /// rules into the shared config.
    pub fn create(schema: Rc<Schema>, doc: Node, plugins: Vec<Rc<dyn Plugin>>) -> EditorState {
        let mut commands: HashMap<String, Command> = HashMap::new();
        let mut keymap = Keymap::new();
        let mut input_rules: Vec<InputRule> = Vec::new();
        for p in &plugins {
            for (name, cmd) in p.commands() {
                commands.insert(name.to_string(), cmd);
            }
            for (binding, command) in p.keymap() {
                keymap.add(binding, command);
            }
            input_rules.extend(p.input_rules());
        }

        let mut plugin_state: HashMap<PluginKey, Rc<dyn Any>> = HashMap::new();
        for p in &plugins {
            if let Some(st) = p.init_state(&doc) {
                plugin_state.insert(p.key(), st);
            }
        }

        let selection = Selection::at_start(&doc);
        let config = Rc::new(Config {
            schema,
            plugins,
            commands,
            keymap,
            input_rules,
        });
        EditorState {
            config,
            doc,
            selection,
            stored_marks: None,
            plugin_state,
        }
    }

    /// A fresh [`Transaction`] starting from this state.
    pub fn tr(&self) -> Transaction {
        Transaction::new(
            self.config.schema.clone(),
            self.doc.clone(),
            self.selection.clone(),
            self.stored_marks.clone(),
        )
    }

    /// Apply a transaction, returning a **new** state. Pure: runs no I/O. The new
    /// document/selection come from the transaction; stored marks survive only into
    /// a collapsed text cursor (design §5); each plugin folds its state forward.
    pub fn apply(&self, tr: Transaction) -> EditorState {
        let new_doc = tr.doc().clone();
        let new_selection = tr.selection();
        // Stored marks survive only if the resulting selection is a bare cursor.
        let stored_marks = if new_selection.is_text_cursor() {
            tr.stored_marks().cloned()
        } else {
            None
        };

        let mut plugin_state: HashMap<PluginKey, Rc<dyn Any>> = HashMap::new();
        for p in &self.config.plugins {
            let prev = self.plugin_state.get(&p.key()).map(|r| r.as_ref());
            if let Some(next) = p.apply(&tr, self, &new_doc, prev) {
                plugin_state.insert(p.key(), next);
            }
        }

        EditorState {
            config: self.config.clone(),
            doc: new_doc,
            selection: new_selection,
            stored_marks,
            plugin_state,
        }
    }

    /// The schema.
    pub fn schema(&self) -> &Schema {
        &self.config.schema
    }

    /// A named command from the aggregated registry.
    pub fn command(&self, name: &str) -> Option<Command> {
        self.config.commands.get(name).cloned()
    }

    /// The aggregated keymap.
    pub fn keymap(&self) -> &Keymap {
        &self.config.keymap
    }

    /// The aggregated input rules.
    pub fn input_rules(&self) -> &[InputRule] {
        &self.config.input_rules
    }

    /// The decorations to render for this state, aggregated across every plugin
    /// (design A4/A8). These are view-side annotations — the empty-editor
    /// placeholder, later the IME preedit and collaborator carets — that are
    /// rendered but are **not** part of the document. The view diffs this against
    /// the previous state's set independently of the document diff.
    pub fn decorations(&self) -> DecorationSet {
        let mut set = DecorationSet::empty();
        for p in &self.config.plugins {
            set.merge(p.decorations(self));
        }
        set
    }

    /// A plugin's folded state, downcast to `T`.
    pub fn plugin_state<T: 'static>(&self, key: PluginKey) -> Option<&T> {
        self.plugin_state
            .get(&key)
            .and_then(|r| r.as_ref().downcast_ref::<T>())
    }

    /// Run `cmd` against this state, returning the resulting state if it applied
    /// and dispatched, else `None`. The standard "command → apply" glue.
    pub fn run_command(&self, cmd: &Command) -> Option<EditorState> {
        self.run_command_mapped(cmd).map(|(state, _)| state)
    }

    /// [`run_command`](Self::run_command), also returning the dispatched
    /// transaction's position [`Mapping`].
    ///
    /// A caller holding a document position from *before* the command — an
    /// asynchronous insertion that captured where the user asked for it — needs
    /// the mapping to carry that position across the edit. `run_command` drops
    /// the transaction, and with it the only record of how positions moved, so
    /// there is nowhere else to recover it from.
    pub fn run_command_mapped(&self, cmd: &Command) -> Option<(EditorState, Mapping)> {
        let mut result: Option<(EditorState, Mapping)> = None;
        let applied = {
            let mut dispatch = |tr: Transaction| {
                // Take the mapping before `apply` consumes the transaction.
                let mapping = tr.mapping().clone();
                result = Some((self.apply(tr), mapping));
            };
            let dispatch: Dispatch = &mut dispatch;
            cmd(self, Some(dispatch))
        };
        if applied { result } else { None }
    }

    /// Run the command named `name`; `None` if no such command or it did not apply.
    pub fn run(&self, name: &str) -> Option<EditorState> {
        let cmd = self.command(name)?;
        self.run_command(&cmd)
    }

    /// [`run`](Self::run), also returning the transaction's position
    /// [`Mapping`] — see [`run_command_mapped`](Self::run_command_mapped).
    pub fn run_mapped(&self, name: &str) -> Option<(EditorState, Mapping)> {
        let cmd = self.command(name)?;
        self.run_command_mapped(&cmd)
    }

    /// Whether `cmd` would apply (toolbar enablement) — no edit performed.
    pub fn command_applies(&self, cmd: &Command) -> bool {
        cmd(self, None)
    }

    /// Whether the named command would apply.
    pub fn can_run(&self, name: &str) -> bool {
        self.command(name).is_some_and(|cmd| cmd(self, None))
    }
}

impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("doc", &self.doc)
            .field("selection", &self.selection)
            .field("stored_marks", &self.stored_marks)
            .finish_non_exhaustive()
    }
}
