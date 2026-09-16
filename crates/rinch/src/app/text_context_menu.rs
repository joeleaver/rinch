//! The built-in text context menu (issue #813): a right-press on a text
//! target — an `<input>` of a text-like type, a `<textarea>`, or the rich-text
//! `Editor` — opens **Cut / Copy / Paste / Select all**, and each item runs the
//! exact code its keyboard chord runs.
//!
//! Two halves, and the split is the point:
//!
//! - **The seam** — [`TextEditState`], [`TextEditAction`],
//!   [`RinchApp::text_edit_state`], [`RinchApp::perform_text_edit`] and
//!   [`RinchApp::prepare_text_context_target`] — is platform-neutral. It knows
//!   what the focused text target can do, where its selection is, and how to do
//!   each thing, and it knows nothing about how the choice is presented.
//! - **The presentation** is a runtime-built DOM overlay on desktop
//!   ([`RinchApp::open_text_context_menu`]), the same shape as the native
//!   `<select>` popup (`select_widget.rs`): body-portal nodes outside every
//!   reactive scope, an entry on the dismiss stack pushed at *open* time
//!   (#671), and a close on any press outside it. A shell that has a native
//!   text toolbar of its own — Android's — asks for
//!   [`TextContextMenuPresentation::Shell`] and receives
//!   [`AppAction::ShowTextContextMenu`] instead, with the caret and selection
//!   already prepared, and drives the toolbar's items through
//!   [`RinchApp::perform_text_edit`].
//!
//! Precedence is the existing one: a live `data-oncontextmenu` on the pressed
//! node or an ancestor wins, and the built-in menu is consulted only when
//! nothing handled the press.

use super::*;
use rinch_core::dom::NodeId;

/// What the text target holding the keyboard can do right now, and where a
/// toolbar anchors.
///
/// `None` from [`RinchApp::text_edit_state`] means no text target holds the
/// keyboard. The four `can_*` flags are the enabled states of the four
/// [`TextEditAction`]s:
///
/// - `can_cut`: a non-empty selection, in a writable, non-`password` field.
/// - `can_copy`: a non-empty selection, not in a `password` field.
/// - `can_paste`: the field is writable. **The clipboard is not read to decide
///   this** — a read can block for up to four seconds on X11 (issue #149) — so
///   Paste stays enabled over an empty clipboard, where a native menu would
///   grey it out. Pasting nothing then does nothing.
/// - `can_select_all`: the field has content.
///
/// A `disabled` field never takes the keyboard (issue #315), so it never
/// reaches here; a `readonly` one does, with `can_cut` and `can_paste` off.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextEditState {
    pub can_cut: bool,
    pub can_copy: bool,
    pub can_paste: bool,
    pub can_select_all: bool,
    /// The selection's — or, collapsed, the caret's — rect in **logical window
    /// px** as `(x, y, w, h)`: where a floating toolbar anchors. For the editor
    /// it is the box spanned by the two caret rects at the selection's ends;
    /// for an `<input>`/`<textarea>` it is the caret rect the IME candidate box
    /// is placed at, which is approximated at the field's text origin.
    pub anchor: (f32, f32, f32, f32),
}

/// One of the four built-in text-editing operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextEditAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl TextEditAction {
    /// The menu rows, in the order the menu shows them.
    const ALL: [TextEditAction; 4] = [
        TextEditAction::Cut,
        TextEditAction::Copy,
        TextEditAction::Paste,
        TextEditAction::SelectAll,
    ];

    fn label(self) -> &'static str {
        match self {
            TextEditAction::Cut => "Cut",
            TextEditAction::Copy => "Copy",
            TextEditAction::Paste => "Paste",
            TextEditAction::SelectAll => "Select all",
        }
    }

    /// The letter of the item's chord, shown as its hint.
    fn chord_letter(self) -> &'static str {
        match self {
            TextEditAction::Cut => "X",
            TextEditAction::Copy => "C",
            TextEditAction::Paste => "V",
            TextEditAction::SelectAll => "A",
        }
    }

    fn enabled_in(self, state: &TextEditState) -> bool {
        match self {
            TextEditAction::Cut => state.can_cut,
            TextEditAction::Copy => state.can_copy,
            TextEditAction::Paste => state.can_paste,
            TextEditAction::SelectAll => state.can_select_all,
        }
    }
}

/// Who presents the menu once a context-menu gesture has landed on a text
/// target and the caret has been placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextContextMenuPresentation {
    /// The runtime opens its own DOM menu. The default, and what desktop uses.
    #[default]
    Dom,
    /// The runtime emits [`AppAction::ShowTextContextMenu`] and opens nothing;
    /// the shell reads [`RinchApp::text_edit_state`] and shows its own
    /// toolbar, driving each item through [`RinchApp::perform_text_edit`].
    Shell,
}

/// The open DOM menu: the app-created nodes plus the highlight.
pub(crate) struct OpenTextMenu {
    /// The text target the menu was opened for. The menu lives exactly as long
    /// as this target holds the keyboard.
    pub target: FocusTarget,
    /// The menu panel, a trailing `<body>` child.
    pub panel_id: usize,
    /// The item rows, in menu order.
    pub items: Vec<TextMenuRow>,
    /// The highlighted row, if any. None until the pointer or an arrow key
    /// picks one, as a native menu opens with nothing highlighted.
    pub highlighted: Option<usize>,
}

pub(crate) struct TextMenuRow {
    pub action: TextEditAction,
    pub node_id: usize,
    pub enabled: bool,
}

/// A text target under a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextTarget {
    /// A text-like `<input>` or a `<textarea>`.
    Input(usize),
    /// A rich-text editor, by container node id.
    #[cfg(feature = "desktop")]
    Editor(usize),
}

/// Where a press lands relative to the open menu.
enum MenuHit {
    Item(usize),
    InsidePanel,
    Outside,
}

/// Menu stylesheet, injected once. Theme variables with light fallbacks, like
/// the `<select>` popup's, so it looks right with or without the theme feature
/// and follows the palette when there is one.
const TEXT_CONTEXT_MENU_CSS: &str = r#"
.rinch-tcm-panel {
    position: fixed;
    z-index: 9999;
    box-sizing: border-box;
    min-width: 180px;
    padding: 4px;
    background: var(--rinch-color-body, #ffffff);
    border: 1px solid var(--rinch-color-gray-3, #dee2e6);
    border-radius: 6px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.15);
}
.rinch-tcm-item {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    padding: 6px 10px;
    border-radius: 4px;
    font-size: 14px;
    line-height: 1.4;
    color: var(--rinch-color-text, #212529);
    white-space: nowrap;
    cursor: default;
}
.rinch-tcm-hint {
    color: var(--rinch-color-dimmed, #868e96);
}
.rinch-tcm-item[data-highlighted] {
    background: var(--rinch-primary-color, #228be6);
    color: #ffffff;
}
.rinch-tcm-item[data-highlighted] .rinch-tcm-hint {
    color: #ffffff;
}
.rinch-tcm-item[data-disabled] {
    color: var(--rinch-color-gray-5, #adb5bd);
}
.rinch-tcm-item[data-disabled] .rinch-tcm-hint {
    color: var(--rinch-color-gray-5, #adb5bd);
}
.rinch-tcm-sep {
    height: 1px;
    margin: 4px 0;
    background: var(--rinch-color-gray-3, #dee2e6);
}
"#;

impl RinchApp {
    // ── The seam ─────────────────────────────────────────────────────────────

    /// Choose who presents the menu after a context-menu gesture on a text
    /// target. Desktop leaves the default, [`TextContextMenuPresentation::Dom`].
    pub fn set_text_context_menu_presentation(
        &mut self,
        presentation: TextContextMenuPresentation,
    ) {
        self.text_menu_presentation = presentation;
    }

    /// What the text target holding the keyboard can do right now, or `None`
    /// when no text target holds it.
    ///
    /// Read, not cached: it answers for the state as it is at the call, so a
    /// shell may poll it to keep a toolbar's items current.
    pub fn text_edit_state(&self) -> Option<TextEditState> {
        match self.focus_target {
            FocusTarget::Input(node_id) => {
                let doc = self.doc.as_ref()?;
                let state = self.focused_input_state.as_ref()?;
                let (has_selection, writable, password, has_content) = {
                    let d = doc.borrow();
                    let node = d.tree.get(node_id)?;
                    if !Self::is_text_like_field(node) {
                        return None;
                    }
                    (
                        !state.selection.is_cursor(),
                        !Self::node_is_readonly(node),
                        Self::is_password_field(node),
                        !state.document.to_text().is_empty(),
                    )
                };
                let anchor = self
                    .input_caret_area(node_id)
                    .unwrap_or_else(|| painted_element_box(&doc.borrow().tree, node_id));
                Some(TextEditState {
                    can_cut: has_selection && writable && !password,
                    can_copy: has_selection && !password,
                    // Never decided by reading the clipboard — see the field's
                    // doc. Without the `clipboard` feature there is nothing to
                    // paste from, on the chord path as much as here.
                    can_paste: writable && cfg!(feature = "clipboard"),
                    can_select_all: has_content,
                    anchor,
                })
            }
            #[cfg(feature = "desktop")]
            FocusTarget::Editor(container) => {
                let handle = crate::editor::editor_for_doc(self.doc_key(), container)?;
                let selection = handle.selection();
                // The same question `editor_copy` asks, so an item is enabled
                // exactly when its chord would do something.
                let can_copy = !selection.is_empty() && handle.selection_clipboard().is_some();
                let anchor = self
                    .editor_selection_anchor(&handle, &selection)
                    .or_else(|| {
                        let doc = self.doc.as_ref()?;
                        Some(painted_element_box(&doc.borrow().tree, container))
                    })?;
                Some(TextEditState {
                    // The editor has no read-only mode; cut is copy plus delete.
                    can_cut: can_copy,
                    can_copy,
                    can_paste: cfg!(feature = "clipboard"),
                    can_select_all: editor_node_has_content(&handle.doc()),
                    anchor,
                })
            }
            _ => None,
        }
    }

    /// Run `action` on the text target holding the keyboard, through **the
    /// same code its chord runs**: `handle_cut` / `handle_copy` /
    /// `handle_paste` / `handle_select_all` for an `<input>`/`<textarea>`, and
    /// `editor_cut` / `editor_copy` / the asynchronous anchored paste (#149) /
    /// the `selectAll` command for the editor. A no-op, returning no actions,
    /// when no text target holds the keyboard.
    ///
    /// The enabled state is **not** re-checked here: a disabled item is never
    /// dispatched by the menu, and a shell driving its own toolbar is expected
    /// to grey its items from [`Self::text_edit_state`] the same way. Running
    /// an action its state forbids is harmless anyway — a read-only field
    /// refuses the mutation at the command level, and cutting nothing cuts
    /// nothing — which is exactly what the chord does in the same state.
    pub fn perform_text_edit(&mut self, action: TextEditAction) -> Vec<AppAction> {
        match self.focus_target {
            FocusTarget::Input(_) => {
                match action {
                    TextEditAction::Cut => self.handle_cut(),
                    TextEditAction::Copy => self.handle_copy(),
                    TextEditAction::Paste => self.handle_paste(),
                    TextEditAction::SelectAll => self.handle_select_all(),
                }
                self.scene_dirty = true;
                vec![AppAction::RequestRedraw]
            }
            #[cfg(feature = "desktop")]
            FocusTarget::Editor(container) => {
                let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container) else {
                    return Vec::new();
                };
                match action {
                    #[cfg(feature = "clipboard")]
                    TextEditAction::Cut => {
                        self.editor_cut(&handle);
                    }
                    #[cfg(feature = "clipboard")]
                    TextEditAction::Copy => self.editor_copy(&handle),
                    #[cfg(feature = "clipboard")]
                    TextEditAction::Paste => {
                        self.editor_paste(&handle);
                    }
                    // Without a clipboard the chord path has nothing to bind
                    // these to either (its clipboard block is compiled out and
                    // the keymap never binds them), so they do nothing here too.
                    #[cfg(not(feature = "clipboard"))]
                    TextEditAction::Cut | TextEditAction::Copy | TextEditAction::Paste => {}
                    // `Mod-a` resolves to this same command through the keymap.
                    TextEditAction::SelectAll => {
                        handle.command("selectAll");
                    }
                }
                self.refresh_editor_overlays();
                self.scene_dirty = true;
                vec![AppAction::RequestRedraw]
            }
            _ => Vec::new(),
        }
    }

    /// A context-menu gesture at `(x, y)`: if the press lands on a text target,
    /// run the ordinary right-press path (focus, `data-rid`, the #316 claim
    /// rule) and then apply the **caret rule** — a press outside the current
    /// selection moves the caret to the press point, a press inside it keeps
    /// the selection — and answer the target's state. `None` when the press is
    /// not on a text target, or when the target refused the keyboard (a
    /// `disabled` field, issue #315).
    ///
    /// This is the half a shell with its own toolbar still needs; the DOM menu
    /// is [`Self::open_text_context_menu`], and
    /// [`Self::text_context_menu_gesture`] is the two composed. Actions the
    /// click path produced are appended to `actions`.
    ///
    /// **Side effects when it answers `Some`, and when it answers `None` on a
    /// text target**: the click path has run. The caller must not run it
    /// again — [`Self::text_context_menu_gesture`] reports that with its own
    /// return value.
    pub fn prepare_text_context_target(
        &mut self,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
    ) -> Option<TextEditState> {
        let target = self.text_target_at(x, y)?;
        self.prepare_target(target, x, y, vp_w, vp_h, actions)
    }

    /// The whole gesture: [`Self::prepare_text_context_target`], then the
    /// presentation the shell asked for. Returns whether the press landed on a
    /// text target — in which case the click path has run and the caller must
    /// not run it again, whether or not a menu opened.
    pub(crate) fn text_context_menu_gesture(
        &mut self,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
    ) -> bool {
        let Some(target) = self.text_target_at(x, y) else {
            return false;
        };
        if self
            .prepare_target(target, x, y, vp_w, vp_h, actions)
            .is_none()
        {
            return true;
        }
        match self.text_menu_presentation {
            TextContextMenuPresentation::Dom => {
                self.open_text_context_menu(x, y, vp_w, vp_h);
            }
            TextContextMenuPresentation::Shell => {
                actions.push(AppAction::ShowTextContextMenu);
            }
        }
        actions.push(AppAction::RequestRedraw);
        true
    }

    /// Whether the built-in DOM menu is open.
    pub fn is_text_context_menu_open(&self) -> bool {
        self.open_text_menu.is_some()
    }

    // ── Target resolution and the caret rule ─────────────────────────────────

    /// A text-like field: `<textarea>`, or an `<input>` whose `type` is one the
    /// text engine edits. The list is #812's: `checkbox`, `radio`, `range`,
    /// `button` and the rest have no text to cut.
    pub(crate) fn is_text_like_field(node: &rinch_dom::Node) -> bool {
        match node.tag() {
            Some("textarea") => true,
            Some("input") => node.attributes.get("type").is_none_or(|t| {
                matches!(
                    t.to_ascii_lowercase().as_str(),
                    "" | "text" | "search" | "url" | "tel" | "email" | "password" | "number"
                )
            }),
            _ => false,
        }
    }

    fn is_password_field(node: &rinch_dom::Node) -> bool {
        node.attributes
            .get("type")
            .is_some_and(|t| t.eq_ignore_ascii_case("password"))
    }

    /// The text target under `(x, y)`: the nearest text-like field or editor
    /// container on the hit node's ancestor chain.
    fn text_target_at(&self, x: f32, y: f32) -> Option<TextTarget> {
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        let mut cur = hit_test(&d.tree, x, y);
        while let Some(nid) = cur {
            let node = d.tree.get(nid)?;
            if Self::is_text_like_field(node) {
                return Some(TextTarget::Input(nid));
            }
            #[cfg(feature = "desktop")]
            if node.attributes.get("data-pm-editor").map(String::as_str) == Some("true") {
                return Some(TextTarget::Editor(nid));
            }
            cur = node.parent;
        }
        None
    }

    fn prepare_target(
        &mut self,
        target: TextTarget,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
    ) -> Option<TextEditState> {
        match target {
            TextTarget::Input(node_id) => {
                // The press offset, and the selection it may land inside,
                // captured BEFORE the click path rebuilds the field's state
                // with a collapsed caret at the press.
                let saved = (self.focus_target == FocusTarget::Input(node_id))
                    .then(|| {
                        self.focused_input_state
                            .as_ref()
                            .map(|s| s.selection.clone())
                    })
                    .flatten();
                let offset = {
                    let doc = self.doc.clone()?;
                    let d = doc.borrow();
                    Self::compute_input_cursor_from_click(
                        &d.tree,
                        &mut self.hit_test_font_cx,
                        &mut self.hit_test_layout_cx,
                        node_id,
                        x,
                        y,
                    )
                };
                // The ordinary right-press path, unchanged: it focuses the
                // field (or refuses a disabled one), places the caret at the
                // press, commits a blurred field's `onchange`, dispatches an
                // ancestor `data-rid`. `scale_factor` is unused by it.
                let click_actions =
                    self.handle_click_with_button(x, y, 1.0, MouseButton::Right, vp_w, vp_h);
                actions.extend(click_actions);
                if self.focus_target != FocusTarget::Input(node_id) {
                    return None;
                }
                // The caret rule's other half: a press inside the selection
                // keeps it. The click path collapsed it, so put it back.
                if let Some(sel) = saved
                    && !sel.is_cursor()
                    && sel.start().0 <= offset
                    && offset <= sel.end().0
                    && let Some(state) = self.focused_input_state.as_mut()
                {
                    state.selection = sel;
                    self.sync_input_cursor_to_dom();
                    self.scene_dirty = true;
                }
                self.text_edit_state()
            }
            #[cfg(feature = "desktop")]
            TextTarget::Editor(container) => {
                let handle = crate::editor::editor_for_doc(self.doc_key(), container)?;
                // The ordinary right-press path first (a `data-rid` above the
                // editor, the blur of whatever else held the keyboard), then
                // the claim the left press would have made. Re-checked after,
                // because a handler is user code and may have re-rendered the
                // editor away.
                let click_actions =
                    self.handle_click_with_button(x, y, 1.0, MouseButton::Right, vp_w, vp_h);
                actions.extend(click_actions);
                crate::editor::editor_for_doc(self.doc_key(), container)?;
                self.set_focus_target(FocusTarget::Editor(container));
                self.editor_goal_x = None;
                use rinch_editor_core::Selection;
                if let Some(leaf) = self.editor_leaf_at(x, y)
                    && let Some(node_sel) = handle.node_selection_at_host(leaf)
                {
                    // A press on an image or rule selects the node, as a left
                    // click does — unless it is the node already selected.
                    if handle.selection() != node_sel {
                        handle.set_selection(node_sel);
                    }
                } else if let Some((c, textblock, ifc_byte)) = self.editor_point_address(x, y)
                    && c == container
                    && let Some(pressed) = handle.pos_at(textblock, ifc_byte)
                {
                    let selection = handle.selection();
                    let inside = !selection.is_empty()
                        && selection.from() <= pressed
                        && pressed <= selection.to();
                    if !inside {
                        handle.set_selection(Selection::cursor(pressed));
                    }
                }
                // A press that missed every textblock keeps the selection.
                self.refresh_editor_overlays();
                self.resolve_and_repaint(vp_w, vp_h);
                self.text_edit_state()
            }
        }
    }

    /// The box spanned by the caret rects at the selection's two ends, in
    /// window px — a single-line selection's rect, and for a multi-line one
    /// the box from the first caret to the last.
    #[cfg(feature = "desktop")]
    fn editor_selection_anchor(
        &self,
        handle: &crate::editor::EditorHandle,
        selection: &rinch_editor_core::Selection,
    ) -> Option<(f32, f32, f32, f32)> {
        let (x1, y1, h1) = self.editor_caret_point(handle, selection.from())?;
        if selection.is_empty() {
            return Some((x1, y1, 1.0, h1));
        }
        let (x2, y2, h2) = self.editor_caret_point(handle, selection.to())?;
        let left = x1.min(x2);
        let top = y1.min(y2);
        let right = x1.max(x2);
        let bottom = (y1 + h1).max(y2 + h2);
        Some((left, top, (right - left).max(1.0), bottom - top))
    }

    // ── The DOM menu ─────────────────────────────────────────────────────────

    /// Open the DOM menu for the text target holding the keyboard, at `(x, y)`
    /// — the press point, pulled back inside the viewport once its laid-out
    /// size is known. Returns whether a menu opened; `false` when no text
    /// target holds the keyboard.
    ///
    /// Replaces an open menu. The nodes are appended to `<body>` outside every
    /// reactive scope, like the `<select>` popup's. Not exempted from the
    /// scroll lock (#474): the panel declares no `overflow`, so it is no scroll
    /// container and the lock has nothing to refuse on it.
    pub fn open_text_context_menu(&mut self, x: f32, y: f32, vp_w: f32, vp_h: f32) -> bool {
        let Some(state) = self.text_edit_state() else {
            return false;
        };
        self.close_text_context_menu();
        let target = self.focus_target;
        let Some(doc) = self.doc.clone() else {
            return false;
        };

        if !self.text_menu_css_injected {
            doc.borrow_mut().load_css(TEXT_CONTEXT_MENU_CSS);
            self.text_menu_css_injected = true;
        }

        let chord_prefix = if cfg!(target_os = "macos") {
            "\u{2318}"
        } else {
            "Ctrl+"
        };

        let mut d = doc.borrow_mut();
        let body = d.body();
        let panel = d.create_element("div");
        d.set_attribute(panel, "class", "rinch-tcm-panel");
        d.set_style(panel, "left", &format!("{x}px"));
        d.set_style(panel, "top", &format!("{y}px"));

        let mut items = Vec::with_capacity(TextEditAction::ALL.len());
        for (i, action) in TextEditAction::ALL.into_iter().enumerate() {
            if action == TextEditAction::SelectAll {
                let sep = d.create_element("div");
                d.set_attribute(sep, "class", "rinch-tcm-sep");
                d.append_child(panel, sep);
            }
            let enabled = action.enabled_in(&state);
            let row = d.create_element("div");
            d.set_attribute(row, "class", "rinch-tcm-item");
            d.set_attribute(row, "data-tcm-item", &i.to_string());
            if !enabled {
                d.set_attribute(row, "data-disabled", "");
            }
            let label = d.create_element("span");
            d.set_attribute(label, "class", "rinch-tcm-label");
            let label_text = d.create_text(action.label());
            d.append_child(label, label_text);
            d.append_child(row, label);
            let hint = d.create_element("span");
            d.set_attribute(hint, "class", "rinch-tcm-hint");
            let hint_text = d.create_text(&format!("{chord_prefix}{}", action.chord_letter()));
            d.append_child(hint, hint_text);
            d.append_child(row, hint);
            d.append_child(panel, row);
            items.push(TextMenuRow {
                action,
                node_id: row.0,
                enabled,
            });
        }
        d.append_child(body, panel);
        drop(d);

        self.open_text_menu = Some(OpenTextMenu {
            target,
            panel_id: panel.0,
            items,
            highlighted: None,
        });

        // Join the dismiss stack at open time (#671), above whatever overlay
        // the field sits in. `unowned`, so the entry keeps app lifetime rather
        // than dying with whatever scope the press happened to run under. The
        // handler can only *ask* — closing needs `&mut RinchApp` — and the
        // Escape path drains the flag the moment the dispatch returns, next to
        // the `<select>` popup's.
        let doc_key = self.doc_key();
        let asked = self.text_menu_dismiss_asked.clone();
        self.text_menu_dismiss_handle = Some(rinch_core::reactive::unowned(move || {
            rinch_core::push_dismiss_handler(doc_key, move || {
                asked.set(true);
                true
            })
        }));

        self.scene_dirty = true;
        self.resolve_and_repaint(vp_w, vp_h);

        // Now the panel has a size: keep it inside the window, as a native
        // menu flips or slides rather than hanging off the edge.
        let placed = {
            let d = doc.borrow();
            d.tree
                .get(panel.0)
                .map(|n| (n.layout.width, n.layout.height))
        };
        if let Some((w, h)) = placed {
            let left = if x + w > vp_w { (vp_w - w).max(0.0) } else { x };
            let top = if y + h > vp_h { (vp_h - h).max(0.0) } else { y };
            if left != x || top != y {
                let mut d = doc.borrow_mut();
                d.set_style(panel, "left", &format!("{left}px"));
                d.set_style(panel, "top", &format!("{top}px"));
                drop(d);
                self.scene_dirty = true;
                self.resolve_and_repaint(vp_w, vp_h);
            }
        }
        true
    }

    /// Close the DOM menu (idempotent): remove its nodes and release its
    /// dismiss entry. This is the one place `open_text_menu` becomes `None`,
    /// so it is the one place the release has to be. Does not touch focus.
    pub fn close_text_context_menu(&mut self) {
        let Some(open) = self.open_text_menu.take() else {
            return;
        };
        self.text_menu_dismiss_handle = None;
        self.text_menu_dismiss_asked.set(false);
        if let Some(doc) = self.doc.clone() {
            doc.borrow_mut().remove_node(NodeId(open.panel_id));
        }
        self.scene_dirty = true;
    }

    /// Whether the open menu's target still holds the keyboard and is still in
    /// the document. Checked at the top of every event: a field removed under
    /// an open menu, or a claim moved by something other than a press, would
    /// otherwise leave a menu whose items act on nothing (the #189/#463 shape
    /// — armed by one event, cleared only by another that may never come).
    pub(crate) fn text_menu_target_is_live(&self) -> bool {
        let Some(open) = &self.open_text_menu else {
            return true;
        };
        if self.focus_target != open.target {
            return false;
        }
        let Some(doc) = &self.doc else {
            return false;
        };
        let d = doc.borrow();
        let in_document = |id: usize| {
            let mut cur = Some(id);
            while let Some(nid) = cur {
                if nid == d.tree.root_id {
                    return true;
                }
                cur = d.tree.get(nid).and_then(|n| n.parent);
            }
            false
        };
        match open.target {
            FocusTarget::Input(id) => {
                d.tree.get(id).is_some_and(Self::is_text_like_field) && in_document(id)
            }
            #[cfg(feature = "desktop")]
            FocusTarget::Editor(id) => {
                in_document(id) && crate::editor::editor_for_doc(self.doc_key(), id).is_some()
            }
            _ => false,
        }
    }

    /// Where a press lands relative to the open menu.
    fn text_menu_hit(&self, x: f32, y: f32) -> MenuHit {
        let Some(open) = &self.open_text_menu else {
            return MenuHit::Outside;
        };
        let Some(doc) = &self.doc else {
            return MenuHit::Outside;
        };
        let d = doc.borrow();
        let mut cur = hit_test(&d.tree, x, y);
        while let Some(nid) = cur {
            let Some(node) = d.tree.get(nid) else { break };
            if let Some(idx) = node
                .attributes
                .get("data-tcm-item")
                .and_then(|s| s.parse::<usize>().ok())
            {
                return MenuHit::Item(idx);
            }
            if nid == open.panel_id {
                return MenuHit::InsidePanel;
            }
            cur = node.parent;
        }
        MenuHit::Outside
    }

    /// Pointer events while the menu is open, ahead of every other pointer
    /// arm: a press on an enabled item runs it, a press anywhere else closes
    /// the menu **without acting** and is swallowed — the page under a menu
    /// sees neither the press nor the pointer, as under a native menu. `None`
    /// for an event that is not a pointer event, or when no menu is open.
    pub(crate) fn text_menu_intercept_pointer(
        &mut self,
        event: &PlatformEvent,
        vp_w: f32,
        vp_h: f32,
    ) -> Option<Vec<AppAction>> {
        self.open_text_menu.as_ref()?;
        match *event {
            PlatformEvent::MouseDown { x, y, .. } => {
                match self.text_menu_hit(x, y) {
                    MenuHit::Item(idx) => self.run_text_menu_item(idx),
                    MenuHit::InsidePanel => {}
                    MenuHit::Outside => self.close_text_context_menu(),
                }
                self.resolve_and_repaint(vp_w, vp_h);
                Some(vec![AppAction::RequestRedraw])
            }
            // The release of the press that opened the menu arrives here, and
            // must not close it; items run on the press.
            PlatformEvent::MouseUp { .. } => Some(Vec::new()),
            PlatformEvent::MouseMove { x, y } => {
                let hovered = match self.text_menu_hit(x, y) {
                    MenuHit::Item(idx) => Some(idx),
                    _ => None,
                };
                let mut actions = vec![AppAction::SetCursor(rinch_platform::CursorStyle::Default)];
                if let Some(idx) = hovered
                    && self.set_text_menu_highlight(Some(idx))
                {
                    self.resolve_and_repaint(vp_w, vp_h);
                    actions.push(AppAction::RequestRedraw);
                }
                Some(actions)
            }
            // The menu is anchored to the field; scrolling would move the
            // field out from under it. Close, and let nothing else scroll.
            PlatformEvent::MouseWheel { .. } => {
                self.close_text_context_menu();
                self.resolve_and_repaint(vp_w, vp_h);
                Some(vec![AppAction::RequestRedraw])
            }
            _ => None,
        }
    }

    /// Keys while the menu is open. Every key is consumed, as by the `<select>`
    /// popup: Up/Down/Home/End move the highlight over the enabled items,
    /// Enter/Space run it. Escape normally never reaches here — the dismiss
    /// stack answers it first — but closes the menu if it does.
    pub(crate) fn handle_text_menu_key(&mut self, key: KeyCode, vp_w: f32, vp_h: f32) -> bool {
        let Some(open) = self.open_text_menu.as_ref() else {
            return false;
        };
        let count = open.items.len();
        let enabled = |i: usize| open.items.get(i).is_some_and(|r| r.enabled);
        let next = |from: Option<usize>, dir: isize| -> Option<usize> {
            let mut i = from.map_or(if dir > 0 { -1 } else { count as isize }, |i| i as isize);
            for _ in 0..count {
                i = (i + dir).rem_euclid(count as isize);
                if enabled(i as usize) {
                    return Some(i as usize);
                }
            }
            None
        };
        let highlighted = open.highlighted;
        match key {
            KeyCode::ArrowDown => {
                let target = next(highlighted, 1);
                self.set_text_menu_highlight(target);
            }
            KeyCode::ArrowUp => {
                let target = next(highlighted, -1);
                self.set_text_menu_highlight(target);
            }
            KeyCode::Home => {
                let target = next(None, 1);
                self.set_text_menu_highlight(target);
            }
            KeyCode::End => {
                let target = next(None, -1);
                self.set_text_menu_highlight(target);
            }
            KeyCode::Enter | KeyCode::Space => {
                if let Some(idx) = highlighted {
                    self.run_text_menu_item(idx);
                }
            }
            KeyCode::Escape => self.close_text_context_menu(),
            _ => {}
        }
        self.resolve_and_repaint(vp_w, vp_h);
        true
    }

    /// Run item `idx` if it is enabled: close the menu first, then perform the
    /// action on the target, which still holds the keyboard.
    fn run_text_menu_item(&mut self, idx: usize) {
        let Some(open) = self.open_text_menu.as_ref() else {
            return;
        };
        let Some(row) = open.items.get(idx) else {
            return;
        };
        if !row.enabled {
            return;
        }
        let action = row.action;
        self.close_text_context_menu();
        self.perform_text_edit(action);
    }

    /// Move the highlight; returns whether it moved.
    fn set_text_menu_highlight(&mut self, index: Option<usize>) -> bool {
        let Some(open) = self.open_text_menu.as_mut() else {
            return false;
        };
        if index == open.highlighted {
            return false;
        }
        let prev = open
            .highlighted
            .and_then(|i| open.items.get(i))
            .map(|r| r.node_id);
        let next = index.and_then(|i| open.items.get(i)).map(|r| r.node_id);
        open.highlighted = index;
        if let Some(doc) = self.doc.clone() {
            let mut d = doc.borrow_mut();
            if let Some(p) = prev {
                d.remove_attribute(NodeId(p), "data-highlighted");
            }
            if let Some(n) = next {
                d.set_attribute(NodeId(n), "data-highlighted", "");
            }
        }
        self.scene_dirty = true;
        true
    }
}

/// Whether an editor document holds anything a Select all would select: a
/// non-empty text node, or an atom (an image, a rule, a hard break).
#[cfg(feature = "desktop")]
fn editor_node_has_content(node: &rinch_editor_core::Node) -> bool {
    if node.is_text() {
        return node.text().is_some_and(|t| !t.is_empty());
    }
    if node.is_atom() {
        return true;
    }
    node.content()
        .children()
        .iter()
        .any(editor_node_has_content)
}
