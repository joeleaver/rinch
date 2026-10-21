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
///         // `data-nofocus`: the press keeps the keyboard in the editor.
///         div { data-nofocus: "",
///             button { onclick: move || { editor.command("toggleBold"); }, "Bold" }
///         }
///         Editor { editor: editor.clone() }
///     }
/// }
/// ```
///
/// With no handle, `Editor {}` mounts a self-contained editor — editable via the
/// keyboard once clicked into, but with no external control surface. Pass initial
/// content as HTML with the `content` prop.
///
/// **Children are an overlay slot** (#1677): they are placed in a layer inside
/// the editor's container, at its padding-box origin ([`OVERLAY_SLOT_ATTR`]),
/// which scrolls with the text as the caret does — so an absolutely positioned
/// child at `top: {line.top}px` sits beside the line a
/// [`MarginLine`](crate::MarginLine) reported, in the same scroller, with no
/// frame of lag. They are never part of the document, the editor's diff, its
/// caret map or its text, and a press or a key in them is theirs (a button
/// there runs its own handler and places no caret). The layer sets no
/// `pointer-events`; style the children as the app needs.
///
/// ```ignore
/// rsx! {
///     Editor { editor: editor.clone(),
///         div { class: "comment-marker", style: {move || format!("top: {}px", top.get())} }
///     }
/// }
/// ```
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
    /// Mount the editor **read-only**: selectable and copyable, and refusing every
    /// local edit — see [`EditorHandle::set_read_only`], which is also how to
    /// change it afterwards (a prop is read once, at mount).
    ///
    /// `true` switches the handle to read-only. `false` (the default) leaves the
    /// handle as it is rather than forcing it editable, so a handle the app made
    /// read-only before mounting — or across a re-mount — stays read-only.
    ///
    /// **It only ever locks, so it is the wrong prop to drive from a signal.** A
    /// reactive `read_only: {|| !can_edit.get()}` re-renders (and re-mounts) the
    /// component and locks when it turns `true`, and then does *not* unlock when it
    /// turns `false` again — a `bool` prop cannot tell "leave it alone" from
    /// "unlock it", and leaving it alone is what a handle locked elsewhere needs.
    /// Call [`EditorHandle::set_read_only`] for an answer that changes.
    pub read_only: bool,
}

impl Component for Editor {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let handle = self.editor.clone().unwrap_or_else(create_editor);
        // Lock *before* the content load, and before the mount so the container
        // carries `data-pm-readonly` from its first frame. The order is only
        // observable on a handle that is already collaborating, and there it is the
        // whole point: a load with a session attached is a write to the shared
        // document, which a read-only editor refuses (`set_read_only`). Filling
        // first would have sent this component's `content` to the peers of a
        // document the app has just declared this user may not change. With no
        // session — every ordinary mount — a load is never refused, so the content
        // still loads exactly as it did.
        if self.read_only {
            handle.set_read_only(true);
        }
        // Load initial content *before* mounting so the view's first build renders
        // it directly (no empty→content diff). A no-op for empty content; on an
        // already-loaded external handle, the `content` prop wins.
        if !self.content.is_empty() {
            handle.load_html(&self.content);
        }
        // `mount` also releases the registration when the scope is disposed
        // (conditional hide, tab switch) — the handle itself may live on.
        let container = handle.mount(scope);
        if !children.is_empty() {
            container.append_child(&overlay_slot(scope, children));
        }
        container
    }
}

/// The attribute on the layer that holds an [`Editor`]'s children — its
/// **overlay slot** (#1677).
pub const OVERLAY_SLOT_ATTR: &str = "data-pm-overlay";

/// The overlay slot: a 0×0 layer at the container's padding-box origin
/// holding `children`, so a child at `top: {MarginLine::top}px` sits level
/// with that line and scrolls with the text, as the caret does.
///
/// It also carries [`NODE_VIEW_ATTR`](crate::NODE_VIEW_ATTR), which is how
/// both platforms already treat an app's own subtree inside an editor: a press
/// or a key inside it is its own — never a caret placement, a node selection
/// or an editor key — and the caret map and the editor's text skip it. The
/// view never diffs it: it is not a block (no `data-pm-type`).
fn overlay_slot(scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
    let layer = scope.create_element("div");
    layer.set_attribute(OVERLAY_SLOT_ATTR, "");
    layer.set_attribute(crate::NODE_VIEW_ATTR, "");
    // Inline, so a consumer's stylesheet cannot move it off the origin; it
    // sets no `pointer-events` (a 0×0 box catches nothing, and its children
    // are the app's to style).
    layer.set_styles(&[
        ("position", "absolute"),
        ("left", "0"),
        ("top", "0"),
        ("width", "0"),
        ("height", "0"),
        ("padding", "0"),
        ("margin", "0"),
        ("border", "0"),
        ("overflow", "visible"),
        ("z-index", "2"),
        ("white-space", "normal"),
    ]);
    for child in children {
        layer.append_child(child);
    }
    layer
}
