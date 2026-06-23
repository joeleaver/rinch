//! [`Plugin`] — the extension seam. History, input rules, tables (M7), collab
//! (M9) and IME-preedit decorations (M6) are **all plugins**; nothing is
//! special-cased in the state core (design §5).
//!
//! A plugin contributes commands (and, once those layers land, keymap entries and
//! input rules), optionally holds **plugin state** that is folded forward on every
//! transaction ([`Plugin::apply`]), and is keyed by a unique [`PluginKey`] so its
//! state can be looked up from [`EditorState`](crate::state::EditorState).

use crate::command::Command;
use crate::decoration::DecorationSet;
use crate::input_rules::InputRule;
use crate::keymap::KeyBinding;
use crate::model::Node;
use crate::state::{EditorState, Transaction};
use std::any::Any;
use std::rc::Rc;

/// A stable identity for a plugin, used to store and look up its state. Two
/// plugins in one state must not share a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PluginKey(pub &'static str);

/// An editor extension: commands, keymap, input rules, and optional folded state.
///
/// All methods have defaults, so a plugin overrides only what it contributes.
pub trait Plugin {
    /// This plugin's unique key.
    fn key(&self) -> PluginKey;

    /// Named commands this plugin contributes (aggregated into the state's command
    /// registry).
    fn commands(&self) -> Vec<(&'static str, Command)> {
        Vec::new()
    }

    /// Key bindings this plugin contributes, as `(binding, command name)` pairs.
    fn keymap(&self) -> Vec<(KeyBinding, &'static str)> {
        Vec::new()
    }

    /// Input rules (markdown-style shortcuts) this plugin contributes.
    fn input_rules(&self) -> Vec<InputRule> {
        Vec::new()
    }

    /// Build this plugin's initial state for a fresh document. `None` if the plugin
    /// is stateless.
    fn init_state(&self, _doc: &Node) -> Option<Rc<dyn Any>> {
        None
    }

    /// Fold this plugin's state forward across a transaction. Called on **every**
    /// applied transaction with the previous state value (`prev`), the old editor
    /// state, and the new document. Returns the new plugin-state value, or `None`
    /// to leave no state. A stateless plugin keeps the default (no state).
    fn apply(
        &self,
        _tr: &Transaction,
        _old: &EditorState,
        _new_doc: &Node,
        _prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        None
    }

    /// View-facing: the decorations this plugin wants rendered for `state` — the
    /// empty-editor placeholder ([`PlaceholderPlugin`](crate::plugins::PlaceholderPlugin)),
    /// and (in later milestones) the IME preedit, search highlights, and
    /// collaborator carets. The state aggregates these across all plugins in
    /// [`EditorState::decorations`](crate::state::EditorState::decorations), and the
    /// view diffs the result independently of the document (design A4). Default:
    /// no decorations.
    fn decorations(&self, _state: &EditorState) -> DecorationSet {
        DecorationSet::empty()
    }
}
