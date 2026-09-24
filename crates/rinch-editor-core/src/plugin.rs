//! [`Plugin`] — the extension seam. History, input rules, tables (M7), collab
//! (M9) and IME-preedit decorations (M6) are **all plugins**; nothing is
//! special-cased in the state core (design §5).
//!
//! A plugin contributes commands (and, once those layers land, keymap entries and
//! input rules), optionally holds **plugin state** that is folded forward on every
//! transaction ([`Plugin::apply`]), and is keyed by a unique [`PluginKey`] so its
//! state can be looked up from [`EditorState`](crate::state::EditorState).
//!
//! A plugin may also claim a **paste** ([`Plugin::handle_paste`]): the one place
//! an app sees what the user pasted and rewrites it, on every platform.

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

/// What a paste carries: the clipboard's `text/plain` and `text/html` flavours,
/// handed to [`Plugin::handle_paste`].
///
/// Either may be absent. A copy from a browser usually offers both (the page's
/// markup and the text it reads as); a copy from a terminal or an address bar
/// offers text alone; Ctrl+Shift+V ("paste as plain text") offers text alone
/// whatever the clipboard holds. [`PasteContent::new`] turns an empty flavour
/// into `None`, so a plugin never has to tell "absent" from "empty".
///
/// A bitmap (a screenshot, a "copy image") is not described here: a paste that
/// carries only an image never reaches [`Plugin::handle_paste`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PasteContent {
    /// The `text/plain` flavour. `None` when there is none, or it is empty.
    pub text: Option<String>,
    /// The `text/html` flavour. `None` when there is none, or it is only
    /// whitespace.
    pub html: Option<String>,
}

impl PasteContent {
    /// A paste of `text` and `html`, with an empty `text` and a whitespace-only
    /// `html` taken as absent.
    pub fn new(text: Option<String>, html: Option<String>) -> PasteContent {
        PasteContent {
            text: text.filter(|t| !t.is_empty()),
            html: html.filter(|h| !h.trim().is_empty()),
        }
    }

    /// A plain-text paste: `text/plain` only.
    pub fn text(text: impl Into<String>) -> PasteContent {
        PasteContent::new(Some(text.into()), None)
    }

    /// A rich paste: `text/html` only.
    pub fn html(html: impl Into<String>) -> PasteContent {
        PasteContent::new(None, Some(html.into()))
    }

    /// Whether the paste carries neither flavour.
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none()
    }
}

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

    /// Claim a paste: return the transaction to apply **instead of** the
    /// default paste, or `None` to leave it to the next plugin and, after the
    /// last, to the default (`text/html` parsed into structure, else
    /// `text/plain` one paragraph per line).
    ///
    /// Consulted in plugin order by
    /// [`EditorState::handle_paste`](crate::state::EditorState::handle_paste),
    /// which every paste the platform reports as a paste event goes through on
    /// desktop and on the web (Ctrl+V / Cmd+V, Ctrl+Shift+V, the context menu's
    /// Paste, the browser's `paste` event), so this one method is how an app
    /// sees and rewrites what the user pasted. A mobile keyboard's clipboard
    /// chip inserts text directly and does not reach it. A typical claim: a URL
    /// pasted over selected words that links them instead of replacing them.
    /// The first `Some` wins, and later plugins are not asked.
    ///
    /// `state` is the editor's state **at the paste**: its selection is where
    /// the paste lands (on desktop, where Ctrl+V was pressed, mapped through
    /// anything typed while the clipboard was read). Build the transaction from
    /// `state.tr()`. It is applied as one transaction, so one undo step, and it
    /// is the user's input: the caret is scrolled into view, a collaborating
    /// editor records and broadcasts it, and `on_change` fires if the document
    /// changed. Returning a transaction that changes nothing
    /// (`Some(state.tr())`) swallows the paste.
    ///
    /// Runs while the editor is borrowed, like a command: it must not call back
    /// into the editor's handle. A read-only editor (`EditorHandle::set_read_only`)
    /// still asks, and refuses the transaction a plugin returns as it refuses
    /// every edit; a plugin need not check.
    fn handle_paste(&self, _state: &EditorState, _paste: &PasteContent) -> Option<Transaction> {
        None
    }
}
