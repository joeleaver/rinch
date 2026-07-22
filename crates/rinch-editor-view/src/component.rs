//! The [`Editor`] component — declarative placement of a rich-text editor in
//! `rsx!`, mirroring the `RenderSurface` "factory handle + component" pattern.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

use crate::create_editor;
use crate::handle::EditorHandle;

/// A rich-text editor placed declaratively in `rsx!`.
///
/// Create a handle with [`create_editor`](super::create_editor) when you need
/// programmatic control (toolbar commands, load/save), then place it:
///
/// ```ignore
/// let editor = create_editor();
/// editor.load_html("<p>Hello</p>");
/// rsx! {
///     div {
///         button { onclick: move || { editor.command("toggleBold"); }, "Bold" }
///         Editor { editor: editor.clone() }
///     }
/// }
/// ```
///
/// With no handle, `Editor {}` mounts a self-contained editor — editable via the
/// keyboard once clicked into, but with no external control surface. Pass initial
/// content as HTML with the `content` prop.
#[derive(Debug, Default)]
pub struct Editor {
    /// A handle from [`create_editor`](super::create_editor) for programmatic
    /// control. When `None`, the component creates and owns an internal handle.
    ///
    /// Place a given handle in **one** `Editor` at a time: a handle owns a single
    /// projection, so mounting it in two live components would abandon the first's
    /// view. (Re-mounting across a reactive re-render is fine — the old scope is
    /// disposed before the new one mounts, and the view rebuilds from the handle's
    /// preserved state.)
    pub editor: Option<EditorHandle>,
    /// Initial content as schema-whitelisted HTML, loaded once when the editor
    /// mounts. Empty means "start with an empty paragraph".
    pub content: String,
}

impl Component for Editor {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let handle = self.editor.clone().unwrap_or_else(create_editor);
        // Load initial content *before* mounting so the view's first build renders
        // it directly (no empty→content diff). A no-op for empty content; on an
        // already-loaded external handle, the `content` prop wins.
        if !self.content.is_empty() {
            handle.load_html(&self.content);
        }
        let container = handle.mount(scope);
        // Stop the runtime from driving this mount once the scope is disposed
        // (conditional hide, tab switch) — the handle itself may live on.
        let container_id = container.node_id().0;
        let doc_key = scope
            .doc_weak()
            .upgrade()
            .map(|d| d.borrow().doc_key())
            .unwrap_or(0);
        scope.on_cleanup(move || super::registry::unregister_editor(doc_key, container_id));
        container
    }
}
