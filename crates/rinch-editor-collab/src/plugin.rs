//! [`CollabPlugin`] — collaboration is a plugin (design §5: history, tables, collab and
//! IME are *all* plugins; nothing is special-cased in the state core).
//!
//! The plugin folds a tiny piece of state forward on every transaction: the **version**
//! (how many remote integrations this state has absorbed) and the **unconfirmed local
//! steps** (the ProseMirror collab protocol — steps applied locally but not yet
//! reflected to peers). A local edit appends its steps; a remote-origin transaction
//! ([`ORIGIN_REMOTE`](crate::remote::ORIGIN_REMOTE)) bumps the version.
//!
//! The default [`CollabSession`](crate::CollabSession) drives the CRDT by **immediate
//! projection** and so never lets a backlog build up; the unconfirmed buffer is the seam
//! a throttled / authority-style driver (and [`crate::rebase`]) builds on. It is a
//! plain plugin over the public step API — there is no downcasting to a concrete engine
//! (design §8: `as_any`/`as_any_mut` are gone).

use std::any::Any;
use std::rc::Rc;

use rinch_editor_core::plugin::{Plugin, PluginKey};
use rinch_editor_core::state::{EditorState, Transaction};
use rinch_editor_core::{Node, Step};

use crate::remote::ORIGIN_REMOTE;

/// The collab plugin's key.
pub const COLLAB_KEY: PluginKey = PluginKey("collab");

/// The collab plugin's folded state.
#[derive(Clone, Default)]
pub struct CollabState {
    /// How many remote integrations this state has absorbed.
    version: usize,
    /// Local steps applied but not yet confirmed to peers (cloned via `clone_box`).
    unconfirmed: Vec<Box<dyn Step>>,
}

impl CollabState {
    /// The current confirmed version.
    pub fn version(&self) -> usize {
        self.version
    }
    /// The not-yet-confirmed local steps.
    pub fn unconfirmed(&self) -> &[Box<dyn Step>] {
        &self.unconfirmed
    }
    /// Whether there are local steps awaiting projection/broadcast.
    pub fn has_unconfirmed(&self) -> bool {
        !self.unconfirmed.is_empty()
    }
}

impl std::fmt::Debug for CollabState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollabState")
            .field("version", &self.version)
            .field("unconfirmed", &self.unconfirmed.len())
            .finish()
    }
}

/// The collaboration plugin. Add it to the editor's plugin list to track collab state.
#[derive(Debug, Default)]
pub struct CollabPlugin;

impl Plugin for CollabPlugin {
    fn key(&self) -> PluginKey {
        COLLAB_KEY
    }

    fn init_state(&self, _doc: &Node) -> Option<Rc<dyn Any>> {
        Some(Rc::new(CollabState::default()))
    }

    fn apply(
        &self,
        tr: &Transaction,
        _old: &EditorState,
        _new_doc: &Node,
        prev: Option<&dyn Any>,
    ) -> Option<Rc<dyn Any>> {
        let mut st = prev
            .and_then(|p| p.downcast_ref::<CollabState>())
            .cloned()
            .unwrap_or_default();

        if tr.get_meta_as::<bool>(ORIGIN_REMOTE).unwrap_or(false) {
            // A remote integration: a new confirmed version; our local backlog is now
            // merged into the shared CRDT.
            if tr.doc_changed() {
                st.version += 1;
                st.unconfirmed.clear();
            }
        } else if tr.doc_changed() {
            // A local edit: record its steps as unconfirmed.
            st.unconfirmed
                .extend(tr.steps().iter().map(|s| s.clone_box()));
        }

        Some(Rc::new(st))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_editor_core::{EditorState, Fragment, Pos, Schema, Selection};

    fn editor() -> EditorState {
        let s = std::rc::Rc::new(Schema::starter_kit());
        let p = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(p)).unwrap();
        EditorState::create(
            s,
            doc,
            vec![
                Rc::new(rinch_editor_core::BaseCommandsPlugin),
                Rc::new(CollabPlugin),
            ],
        )
    }

    fn collab(state: &EditorState) -> CollabState {
        state
            .plugin_state::<CollabState>(COLLAB_KEY)
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn local_edits_accumulate_as_unconfirmed() {
        let state = editor();
        assert_eq!(collab(&state).version(), 0);
        assert!(!collab(&state).has_unconfirmed());

        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(3)));
        tr.insert_text("X").unwrap();
        let state = state.apply(tr);
        assert!(
            collab(&state).has_unconfirmed(),
            "a local edit is unconfirmed"
        );
        assert_eq!(collab(&state).version(), 0);
    }

    #[test]
    fn remote_transaction_bumps_version_and_clears_backlog() {
        let state = editor();
        // local edit -> unconfirmed
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(3)));
        tr.insert_text("X").unwrap();
        let state = state.apply(tr);
        assert!(collab(&state).has_unconfirmed());

        // remote-origin edit -> version++, backlog cleared
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(1)));
        tr.insert_text("Y").unwrap();
        tr.set_meta(crate::remote::ORIGIN_REMOTE, true);
        let state = state.apply(tr);
        assert_eq!(collab(&state).version(), 1);
        assert!(
            !collab(&state).has_unconfirmed(),
            "remote integration clears backlog"
        );
    }
}
