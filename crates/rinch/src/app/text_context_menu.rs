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
//!
//! **Spelling (#2862, stage 3).** A right press on a word the spellchecker
//! marked selects the word (Chrome's rule, measured in Chrome 158) and the
//! menu opens with its suggestions on top (up to
//! [`SpellcheckConfig::menu_suggestions`](rinch_spell::runtime::SpellcheckConfig::menu_suggestions),
//! Chrome's three), then "Add to dictionary", then the edit rows.
//! [`TextEditState::spelling`] carries them to a shell, and
//! [`TextEditAction::Suggestion`] / [`TextEditAction::AddToDictionary`] run
//! them. The suggestions are computed at the press on the suggestion thread:
//! the press waits up to
//! [`menu_suggest_wait_ms`](rinch_spell::runtime::SpellcheckConfig::menu_suggest_wait_ms)
//! for them, and if they come later the open menu is filled in when they
//! arrive.

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
/// **All three clipboard flags are `false` when `rinch` was built without the
/// `clipboard` feature**: there is no clipboard for Copy or Paste to reach, and
/// an enabled Cut would delete text it never copied. The chord and
/// [`RinchApp::perform_text_edit`] agree: without the feature, Cut on an
/// `<input>`/`<textarea>` does nothing (issue #823), as it does in the editor.
///
/// A `disabled` field never takes the keyboard (issue #315), so it never
/// reaches here; a `readonly` one does, with `can_cut` and `can_paste` off —
/// and so does a read-only editor (`EditorHandle::set_read_only`), the same way.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextEditState {
    pub can_cut: bool,
    pub can_copy: bool,
    pub can_paste: bool,
    pub can_select_all: bool,
    /// The selection's — or, collapsed, the caret's — rect in **logical window
    /// px** as `(x, y, w, h)`: where a floating toolbar anchors. A collapsed
    /// caret is a 1px-wide rect at the caret; a selection on one line is the
    /// rect from its start caret to its end caret over that line; a selection
    /// spanning lines (a `<textarea>`, a multi-block editor selection) is the
    /// **bounding box of the two caret rects** — left and right from the two
    /// carets, top of the first line to the bottom of the last — rather than
    /// the union of the full line boxes. The field's own text layout is
    /// measured for it (a `password` field through its bullets), which is why
    /// [`RinchApp::text_edit_state`] takes `&mut self`.
    pub anchor: (f32, f32, f32, f32),
    /// The misspelled word the selection is, when a context gesture landed
    /// on a word the spellchecker marked (and selected it), with what the
    /// menu offers for it. `None` otherwise, and again once the selection or
    /// the word changes.
    pub spelling: Option<TextSpelling>,
}

/// A misspelled word the text context menu offers corrections for (#2862):
/// the spelling half of [`TextEditState`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextSpelling {
    /// The word, as the field or editor holds it.
    pub word: String,
    /// The corrections, best first, at most
    /// [`SpellcheckConfig::menu_suggestions`](rinch_spell::runtime::SpellcheckConfig::menu_suggestions).
    /// [`TextEditAction::Suggestion`]`(i)` puts `suggestions[i]` in the
    /// word's place.
    pub suggestions: Vec<String>,
    /// The suggestions are still being computed: `suggestions` is empty for
    /// now and fills in when they arrive (the DOM menu redraws itself; a
    /// shell reads the state again).
    pub suggestions_pending: bool,
    /// Whether "Add to dictionary" is offered
    /// ([`SpellcheckConfig::menu_add_to_dictionary`](rinch_spell::runtime::SpellcheckConfig::menu_add_to_dictionary)).
    pub can_add_to_dictionary: bool,
    /// Whether a disabled "No spelling suggestions" row stands in for an
    /// empty list
    /// ([`SpellcheckConfig::menu_no_suggestions_row`](rinch_spell::runtime::SpellcheckConfig::menu_no_suggestions_row)).
    pub show_no_suggestions: bool,
}

/// The label of the disabled row that stands in for no suggestions, Chrome's
/// wording (`IDS_CONTENT_CONTEXT_NO_SPELLING_SUGGESTIONS`).
pub const NO_SUGGESTIONS_LABEL: &str = "No spelling suggestions";

/// The label of the "Add to dictionary" row, Chrome's.
pub const ADD_TO_DICTIONARY_LABEL: &str = "Add to dictionary";

/// The misspelled word a context gesture landed on: what
/// [`TextEditState::spelling`] reports while the word stays selected.
pub(crate) struct SpellTarget {
    pub target: FocusTarget,
    /// The word's byte range in a field's value, or its document range in
    /// the editor.
    pub range: std::ops::Range<usize>,
    pub word: String,
    pub suggestions: Vec<String>,
    pub pending: bool,
    pub request: u64,
    pub add: bool,
    pub no_row: bool,
}

/// One of the built-in text-editing operations: the four edit rows, and the
/// two spelling rows a misspelled word adds (#2862).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextEditAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
    /// Replace the misspelled word with suggestion `i` of
    /// [`TextEditState::spelling`], as one edit the user can undo, through the
    /// path typing takes (so `oninput` fires, and the editor's `on_change`).
    Suggestion(usize),
    /// Add the misspelled word to the personal dictionary
    /// ([`rinch_spell::runtime::add_to_dictionary`]): the app's dictionary
    /// listener hears of it, and every mark of it goes, in every field and
    /// editor.
    AddToDictionary,
}

impl TextEditAction {
    /// The menu rows, in the order the menu shows them.
    const ALL: [TextEditAction; 4] = [
        TextEditAction::Cut,
        TextEditAction::Copy,
        TextEditAction::Paste,
        TextEditAction::SelectAll,
    ];

    /// An edit row's label (the spelling rows are labelled by
    /// [`RinchApp::open_text_context_menu`]).
    fn label(self) -> &'static str {
        match self {
            TextEditAction::Cut => "Cut",
            TextEditAction::Copy => "Copy",
            TextEditAction::Paste => "Paste",
            TextEditAction::SelectAll => "Select all",
            TextEditAction::Suggestion(_) => "",
            TextEditAction::AddToDictionary => ADD_TO_DICTIONARY_LABEL,
        }
    }

    /// The letter of the item's chord, shown as its hint.
    fn chord_letter(self) -> &'static str {
        match self {
            TextEditAction::Cut => "X",
            TextEditAction::Copy => "C",
            TextEditAction::Paste => "V",
            TextEditAction::SelectAll => "A",
            TextEditAction::Suggestion(_) | TextEditAction::AddToDictionary => "",
        }
    }

    fn enabled_in(self, state: &TextEditState) -> bool {
        match self {
            TextEditAction::Cut => state.can_cut,
            TextEditAction::Copy => state.can_copy,
            TextEditAction::Paste => state.can_paste,
            TextEditAction::SelectAll => state.can_select_all,
            TextEditAction::Suggestion(i) => state
                .spelling
                .as_ref()
                .is_some_and(|s| i < s.suggestions.len()),
            TextEditAction::AddToDictionary => state
                .spelling
                .as_ref()
                .is_some_and(|s| s.can_add_to_dictionary),
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

/// The menu's nodes, built once per app and kept: a trailing `<body>` child
/// hidden with `display: none` between opens. Rebuilding per open leaked 22
/// slab entries per right-click for the session — `rinch-dom` frees nothing on
/// `remove_node` (#723) — and every later layout walked the dead panels
/// (measured by #817's review: 37x on the layout pass after 1000 opens).
pub(crate) struct TextMenuPanel {
    pub panel_id: u64,
    /// `(action, row node id)` in menu order.
    pub rows: Vec<(TextEditAction, u64)>,
    /// The spelling rows, built at the first open on a misspelled word (an
    /// app that never has one never builds them), kept and hidden like the
    /// panel.
    pub spell: Option<SpellRows>,
}

/// The spelling rows above the edit rows: the suggestions (one row per
/// suggestion, grown as needed), the "No spelling suggestions" row, a
/// separator, "Add to dictionary", and the separator above the edit rows.
pub(crate) struct SpellRows {
    /// `(row, its label's text node)`, in order.
    suggestions: Vec<(u64, u64)>,
    none_row: u64,
    sep_top: u64,
    add_row: u64,
    sep_bottom: u64,
    /// Whether any of them is showing (a menu with no spelling hides them
    /// only once).
    shown: bool,
}

/// The open DOM menu: which target, the rows' states at open, the highlight.
pub(crate) struct OpenTextMenu {
    /// The text target the menu was opened for. The menu lives exactly as long
    /// as this target holds the keyboard.
    pub target: FocusTarget,
    /// The panel showing — [`TextMenuPanel::panel_id`], copied for the readers.
    pub panel_id: u64,
    /// The item rows, in menu order.
    pub items: Vec<TextMenuRow>,
    /// The highlighted row, if any. None until the pointer or an arrow key
    /// picks one, as a native menu opens with nothing highlighted.
    pub highlighted: Option<usize>,
    /// Where it was opened, and in what viewport: suggestions that arrive
    /// after the menu opened redraw it there.
    pub at: (f32, f32),
    pub viewport: (f32, f32),
    /// The suggestion request its spelling rows show, if any.
    pub spell_request: Option<u64>,
}

pub(crate) struct TextMenuRow {
    pub action: TextEditAction,
    pub node_id: u64,
    pub enabled: bool,
}

/// A text target under a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextTarget {
    /// A text-like `<input>` or a `<textarea>`.
    Input(u64),
    /// A rich-text editor, by container node id.
    #[cfg(feature = "windowing")]
    Editor(u64),
}

/// An editor context press resolved on the tree as it was pressed — see
/// [`RinchApp::resolve_editor_context_press`].
#[cfg(feature = "windowing")]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditorContextPress {
    /// The editor's container node id.
    pub(crate) container: u64,
    /// What the press pointed at; `None` when it missed every textblock.
    pub(crate) at: Option<EditorPressAt>,
}

/// What an [`EditorContextPress`] pointed at.
#[cfg(feature = "windowing")]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EditorPressAt {
    /// An image or rule: the node selection a press on it makes.
    Node(rinch_editor_core::Selection),
    /// A document position in a textblock.
    Pos(rinch_editor_core::Pos),
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
    /* Explicit (the `<select>` popup sizes itself the same way): a `width:
       auto` fixed block shrinks to fit its rows since #893, and the fixed
       width is kept so the panel does not change size. */
    width: 200px;
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
    padding: 4px 0;
}
.rinch-tcm-sep-line {
    height: 1px;
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
    /// shell may poll it to keep a toolbar's items current — it is cheap
    /// enough for that (no serialization, no document walk; the field's text
    /// layout is rebuilt for the anchor, which is what `&mut self` is for).
    pub fn text_edit_state(&mut self) -> Option<TextEditState> {
        match self.focus_target {
            FocusTarget::Input(node_id) => {
                // An app write since the last frame (a timer, a drained
                // `Signal::send`) is in the DOM's `value` but not yet in the
                // editable state, whose offsets would then index the old text
                // while the layout below is built from the new one — slicing a
                // password value at them panicked. Adopt first, as a frame does.
                self.adopt_focused_input_value_from_dom();
                let doc = self.doc.clone()?;
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
                let (start, end) = (state.selection.start().0, state.selection.end().0);
                let head = state.selection.head.0;
                let anchor = {
                    let d = doc.borrow();
                    // The head is drawn with its affinity: an upstream caret
                    // at a soft wrap ends the upper line (#941).
                    let head_affinity = d
                        .tree
                        .get(node_id)
                        .map(rinch_dom::form_control::caret_affinity)
                        .unwrap_or_default();
                    let mut caret = |offset: usize| {
                        Self::input_caret_rect_with_affinity(
                            &d.tree,
                            &mut self.hit_test_font_cx,
                            &mut self.hit_test_layout_cx,
                            node_id,
                            offset,
                            if offset == head {
                                head_affinity
                            } else {
                                rinch_core::dom::CaretAffinity::Downstream
                            },
                        )
                    };
                    // Clipped to the field's own box. Paint does not clip there:
                    // it draws a `<textarea>`'s first-line caret 1px above the
                    // box (measured: caret top y 24, box top 25), and a
                    // `<textarea>`'s text can overflow it (#827). A toolbar
                    // anchored to the field should not overhang it.
                    match (caret(start), caret(end)) {
                        (Some(a), Some(b)) => Some(clip_rect(
                            union_of_caret_rects(a, b),
                            painted_element_box(&d.tree, node_id),
                        )),
                        _ => None,
                    }
                };
                // An empty field has no layout to measure: the IME candidate
                // box's old approximation (the text origin, full height) stands in.
                let anchor = anchor
                    .or_else(|| self.input_text_origin_area(node_id))
                    .unwrap_or_else(|| painted_element_box(&doc.borrow().tree, node_id));
                // None of the three is ever decided by reading the clipboard —
                // see the type's doc — and all three are off without one.
                let clipboard = cfg!(feature = "clipboard");
                Some(TextEditState {
                    can_cut: has_selection && writable && !password && clipboard,
                    can_copy: has_selection && !password && clipboard,
                    can_paste: writable && clipboard,
                    can_select_all: has_content,
                    anchor,
                    spelling: self.live_spelling(),
                })
            }
            #[cfg(feature = "windowing")]
            FocusTarget::Editor(container) => {
                let handle = crate::editor::editor_for_doc(self.doc_key(), container)?;
                let selection = handle.selection();
                // A non-empty selection is what `editor_copy` serializes; the
                // serialization itself is not repeated here (this is polled).
                let can_copy = !selection.is_empty() && cfg!(feature = "clipboard");
                let anchor = self
                    .editor_selection_anchor(&handle, &selection)
                    .or_else(|| {
                        let doc = self.doc.as_ref()?;
                        Some(painted_element_box(&doc.borrow().tree, container))
                    })?;
                // `content_size` is a stored count, not a walk: an empty editor
                // is one empty paragraph, size 2, so anything larger holds a
                // character, an atom or a further block to select.
                let can_select_all = handle.doc().content_size() > 2;
                // A read-only editor (`EditorHandle::set_read_only`) is a
                // `readonly` field: it copies and selects, and offers neither
                // Cut (copy plus a delete it would refuse) nor Paste.
                let writable = !handle.refuses_edits();
                Some(TextEditState {
                    can_cut: can_copy && writable,
                    can_copy,
                    can_paste: writable && cfg!(feature = "clipboard"),
                    can_select_all,
                    anchor,
                    spelling: self.live_spelling(),
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
    ///
    /// [`TextEditAction::Suggestion`] and [`TextEditAction::AddToDictionary`]
    /// act on [`TextEditState::spelling`]'s word, and do nothing when there is
    /// none (the word is no longer selected, or no longer marked).
    pub fn perform_text_edit(&mut self, action: TextEditAction) -> Vec<AppAction> {
        match action {
            TextEditAction::Suggestion(i) => return self.replace_misspelling(i),
            TextEditAction::AddToDictionary => return self.add_misspelling_to_dictionary(),
            _ => {}
        }
        match self.focus_target {
            FocusTarget::Input(_) => {
                match action {
                    TextEditAction::Cut => self.handle_cut(),
                    TextEditAction::Copy => self.handle_copy(),
                    TextEditAction::Paste => {
                        self.handle_paste();
                    }
                    TextEditAction::SelectAll => self.handle_select_all(),
                    TextEditAction::Suggestion(_) | TextEditAction::AddToDictionary => {}
                }
                self.scene_dirty = true;
                vec![AppAction::RequestRedraw]
            }
            #[cfg(feature = "windowing")]
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
                    TextEditAction::Suggestion(_) | TextEditAction::AddToDictionary => {}
                }
                self.refresh_editor_overlays();
                self.scene_dirty = true;
                vec![AppAction::RequestRedraw]
            }
            _ => Vec::new(),
        }
    }

    /// A context-menu gesture at `(x, y)`: if the press lands on a text target,
    /// run the ordinary right-press path (focus and the #316 claim rule; a
    /// right press clicks no `data-rid`, #1093) and then apply the **caret rule** — a press outside the current
    /// selection moves the caret to the press point, a press inside it keeps
    /// the selection, and a press on a misspelled word selects the word
    /// (#2862) — and answer the target's state. `None` when the press is
    /// not on a text target, or when the target refused the keyboard (a
    /// `disabled` field, issue #315).
    ///
    /// This is the half a shell with its own toolbar still needs; the DOM menu
    /// is [`Self::open_text_context_menu`], and
    /// `Self::text_context_menu_gesture` is the two composed. Actions the
    /// click path produced are appended to `actions`.
    ///
    /// **Side effects when it answers `Some`, and when it answers `None` on a
    /// text target**: the click path has run. The caller must not run it
    /// again — `Self::text_context_menu_gesture` reports that with its own
    /// return value.
    pub fn prepare_text_context_target(
        &mut self,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
    ) -> Option<TextEditState> {
        // A shell may call this outside `handle_event`, which is where the
        // shared hit-test memo is otherwise dropped (#908): a tick since the
        // last event may have moved a transform without moving the generation.
        self.hit_memo.set(None);
        let target = self.text_target_at(x, y)?;
        self.prepare_target(
            target,
            x,
            y,
            vp_w,
            vp_h,
            actions,
            #[cfg(feature = "windowing")]
            None,
        )
    }

    /// The whole gesture: [`Self::prepare_text_context_target`], then the
    /// presentation the shell asked for. Returns whether the press landed on a
    /// text target — in which case the click path has run and the caller must
    /// not run it again, whether or not a menu opened.
    ///
    /// `press` is an editor press already resolved on the tree as it was
    /// pressed ([`Self::resolve_editor_context_press`]); `None` resolves it here.
    pub(crate) fn text_context_menu_gesture(
        &mut self,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
        #[cfg(feature = "windowing")] press: Option<EditorContextPress>,
    ) -> bool {
        let Some(target) = self.text_target_at(x, y) else {
            return false;
        };
        if self
            .prepare_target(
                target,
                x,
                y,
                vp_w,
                vp_h,
                actions,
                #[cfg(feature = "windowing")]
                press,
            )
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
    pub(super) fn text_target_at(&self, x: f32, y: f32) -> Option<TextTarget> {
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        let hit = self.shared_hit(&d, x, y);
        let mut cur = hit;
        while let Some(nid) = cur {
            let node = d.tree.get(nid)?;
            if Self::is_text_like_field(node) {
                return Some(TextTarget::Input(nid));
            }
            // Past a node view's host from inside its component: the press is the
            // component's, not the editor's.
            #[cfg(feature = "windowing")]
            if Some(nid) != hit && node.attributes.contains_key(crate::editor::NODE_VIEW_ATTR) {
                return None;
            }
            #[cfg(feature = "windowing")]
            if node.attributes.get("data-pm-editor").map(String::as_str) == Some("true") {
                return Some(TextTarget::Editor(nid));
            }
            cur = node.parent;
        }
        None
    }

    /// Resolve what a context press at `(x, y)` points at in an editor — the
    /// node it selects (an image, a rule) or the document position it lands on
    /// — **before** anything moves the caret.
    ///
    /// Resolve once, apply later, because applying can invalidate the geometry
    /// the resolution read. A plugin whose decoration depends on the selection (a
    /// spellchecker that withdraws its squiggle once the caret enters the word)
    /// re-renders the pressed run the moment the caret moves, and nothing lays
    /// the document out again before the next read: a second resolution from
    /// `(x, y)` then maps the point against a text node with no layout and lands
    /// at the block start (review of #836, finding 2). So the press is resolved
    /// here, on the tree as it was pressed, and [`Self::apply_editor_context_press`]
    /// applies it — from the claimed path and the built-in menu's path alike.
    ///
    /// `None` unless the press lands inside an editor.
    #[cfg(feature = "windowing")]
    pub(crate) fn resolve_editor_context_press(
        &self,
        x: f32,
        y: f32,
    ) -> Option<EditorContextPress> {
        let Some(TextTarget::Editor(container)) = self.text_target_at(x, y) else {
            return None;
        };
        let handle = crate::editor::editor_for_doc(self.doc_key(), container)?;
        let at = if let Some(leaf) = self.editor_leaf_at(x, y)
            && let Some(node_sel) = handle.node_selection_at_host(leaf)
        {
            Some(EditorPressAt::Node(node_sel))
        } else if let Some(hit) = self.editor_point(x, y)
            && hit.container == container
            && let Some(pressed) = hit.pos(&handle)
        {
            Some(EditorPressAt::Pos(pressed))
        } else {
            None
        };
        Some(EditorContextPress { container, at })
    }

    /// Apply the **caret rule** for a press [resolved](Self::resolve_editor_context_press)
    /// earlier: a press on an image or rule selects the node (unless it is the
    /// node already selected), a press outside the selection moves the caret to
    /// the press point, a press inside it keeps the selection, and a press that
    /// missed every textblock keeps it too. Answers the handle, or `None` when
    /// the editor is gone.
    #[cfg(feature = "windowing")]
    pub(crate) fn apply_editor_context_press(
        &mut self,
        press: &EditorContextPress,
    ) -> Option<crate::editor::EditorHandle> {
        let handle = crate::editor::editor_for_doc(self.doc_key(), press.container)?;
        match &press.at {
            Some(EditorPressAt::Node(node_sel)) => {
                if handle.selection() != *node_sel {
                    handle.set_selection(node_sel.clone());
                }
            }
            Some(EditorPressAt::Pos(pressed)) => {
                let selection = handle.selection();
                let inside = !selection.is_empty()
                    && selection.from() <= *pressed
                    && *pressed <= selection.to();
                if !inside {
                    handle.set_selection(rinch_editor_core::Selection::cursor(*pressed));
                }
            }
            None => {}
        }
        Some(handle)
    }

    /// `press` is the editor press the caller already resolved on the tree as it
    /// was pressed, if any; `None` resolves it here, before the click path runs.
    #[allow(clippy::too_many_arguments)]
    fn prepare_target(
        &mut self,
        target: TextTarget,
        x: f32,
        y: f32,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
        #[cfg(feature = "windowing")] press: Option<EditorContextPress>,
    ) -> Option<TextEditState> {
        match target {
            TextTarget::Input(node_id) => {
                // The press offset, and the selection it may land inside,
                // captured BEFORE the click path rebuilds the field's state
                // with a collapsed caret at the press. Both must index the
                // same text: the offset is measured on the DOM's `value`, so an
                // app write the focused field has not adopted yet is adopted
                // first — otherwise a stale selection is compared with, and
                // restored onto, text it does not index.
                if self.focus_target == FocusTarget::Input(node_id) {
                    self.adopt_focused_input_value_from_dom();
                }
                // The text is saved with the selection: the selection is put
                // back only onto the text it indexes.
                let saved = (self.focus_target == FocusTarget::Input(node_id))
                    .then(|| {
                        self.focused_input_state
                            .as_ref()
                            .map(|s| (s.document.to_text(), s.selection.clone()))
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
                // press, commits a blurred field's `onchange`. It clicks no
                // `data-rid` above the field: a right press is not a click
                // (#1093). `scale_factor` is unused by it.
                let click_actions =
                    self.handle_click_with_button(x, y, 1.0, MouseButton::Right, vp_w, vp_h);
                actions.extend(click_actions);
                if self.focus_target != FocusTarget::Input(node_id) {
                    return None;
                }
                // The click path used to click a `data-rid` above the field
                // here (#1093 took that away). The adoption stays so that a
                // write made by any user code the click path does run is taken
                // in before anything syncs the state back over it.
                self.adopt_focused_input_value_from_dom();
                // The caret rule's other half: a press inside the selection
                // keeps it. The click path collapsed it, so put it back — unless
                // the text changed under the press, when the old offsets index
                // nothing and the write already placed the caret.
                if let Some((text, sel)) = saved
                    && !sel.is_cursor()
                    && sel.start().0 <= offset
                    && offset <= sel.end().0
                    && let Some(state) = self.focused_input_state.as_mut()
                    && state.document.to_text() == text
                {
                    state.selection = sel;
                    self.sync_input_cursor_to_dom();
                    self.scene_dirty = true;
                }
                // A press on a misspelled word selects it (#2862).
                self.text_spell = None;
                self.spell_select_in_input(node_id, offset);
                self.text_edit_state()
            }
            #[cfg(feature = "windowing")]
            TextTarget::Editor(container) => {
                // Resolved before anything below can re-render the editor.
                let press = press
                    .filter(|p| p.container == container)
                    .or_else(|| self.resolve_editor_context_press(x, y))
                    .filter(|p| p.container == container)?;
                // The ordinary right-press path first (the blur of whatever
                // else held the keyboard, whose `onchange` commit is user
                // code), then the claim the left press would have made.
                // Re-checked after, because that code may have re-rendered the
                // editor away. A right press clicks no `data-rid` (#1093).
                let click_actions =
                    self.handle_click_with_button(x, y, 1.0, MouseButton::Right, vp_w, vp_h);
                actions.extend(click_actions);
                crate::editor::editor_for_doc(self.doc_key(), container)?;
                self.set_focus_target(FocusTarget::Editor(container));
                self.editor_goal_x = None;
                self.apply_editor_context_press(&press)?;
                // A press on a misspelled word selects it (#2862).
                self.text_spell = None;
                self.spell_select_in_editor(container, &press);
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
    #[cfg(feature = "windowing")]
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
    /// Replaces an open menu. The nodes are built once per app, appended to
    /// `<body>` outside every reactive scope like the `<select>` popup's, and
    /// **kept**: an open shows the same panel again with its position, each
    /// row's `data-disabled` and the highlight rewritten, and a close hides it
    /// with `display: none` — out of layout, paint, hit testing and the Tab
    /// order (the rows carry no `tabindex` in any case).
    ///
    /// `display: none` costs every later layout pass a little: text under it
    /// is laid out again each pass (#826; +0.05 ms a pass, release, measured
    /// on a themed page). `visibility: hidden` would not, and was tried: set
    /// through `set_style` it does not reach the rows (#508), and set by a
    /// stylesheet rule it hid the rows but not their text, which paint drew
    /// whatever the IFC root's visibility until #829 fixed that half. Not
    /// exempted from the scroll lock (#474): the panel declares no `overflow`,
    /// so it is no scroll container and the lock has nothing to refuse on it.
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
        if self.text_menu_panel.is_none() {
            self.text_menu_panel = Some(Self::build_text_menu_panel(&mut doc.borrow_mut()));
        }
        let (panel_id, rows) = {
            let p = self.text_menu_panel.as_ref().expect("built above");
            (p.panel_id, p.rows.clone())
        };
        let panel = NodeId(panel_id);

        let mut d = doc.borrow_mut();
        d.set_style(panel, "left", &format!("{x}px"));
        d.set_style(panel, "top", &format!("{y}px"));
        let mut items = Vec::with_capacity(rows.len());
        // The spelling rows first (#2862), when the selection is a
        // misspelled word; hidden again when it is not.
        let spell_request = state
            .spelling
            .as_ref()
            .and_then(|_| self.text_spell.as_ref().map(|t| t.request));
        if let Some(p) = self.text_menu_panel.as_mut() {
            Self::show_spell_rows(&mut d, p, state.spelling.as_ref(), &mut items);
        }
        for (action, node_id) in rows {
            let enabled = action.enabled_in(&state);
            let row = NodeId(node_id);
            if enabled {
                d.remove_attribute(row, "data-disabled");
            } else {
                d.set_attribute(row, "data-disabled", "");
            }
            d.remove_attribute(row, "data-highlighted");
            items.push(TextMenuRow {
                action,
                node_id,
                enabled,
            });
        }
        d.set_style(panel, "display", "block");
        drop(d);

        self.open_text_menu = Some(OpenTextMenu {
            target,
            panel_id,
            items,
            highlighted: None,
            at: (x, y),
            viewport: (vp_w, vp_h),
            spell_request,
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
                .get(panel_id)
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

    /// Close the DOM menu (idempotent): hide its panel and release its
    /// dismiss entry. This is the one place `open_text_menu` becomes `None`,
    /// so it is the one place the release has to be. Does not touch focus.
    pub fn close_text_context_menu(&mut self) {
        let Some(open) = self.open_text_menu.take() else {
            return;
        };
        self.text_menu_dismiss_handle = None;
        self.text_menu_dismiss_asked.set(false);
        if let Some(doc) = self.doc.clone() {
            doc.borrow_mut()
                .set_style(NodeId(open.panel_id), "display", "none");
        }
        self.scene_dirty = true;
    }

    /// Build the panel once: rows with their labels and chord hints, hidden
    /// until the first open shows it.
    fn build_text_menu_panel(d: &mut RinchDocument) -> TextMenuPanel {
        let chord_prefix = if cfg!(target_os = "macos") {
            "\u{2318}"
        } else {
            "Ctrl+"
        };
        let body = d.body();
        let panel = d.create_element("div");
        d.set_attribute(panel, "class", "rinch-tcm-panel");
        d.set_style(panel, "display", "none");
        let mut rows = Vec::with_capacity(TextEditAction::ALL.len());
        for (i, action) in TextEditAction::ALL.into_iter().enumerate() {
            if action == TextEditAction::SelectAll {
                let sep = d.create_element("div");
                d.set_attribute(sep, "class", "rinch-tcm-sep");
                let line = d.create_element("div");
                d.set_attribute(line, "class", "rinch-tcm-sep-line");
                d.append_child(sep, line);
                d.append_child(panel, sep);
            }
            let row = d.create_element("div");
            d.set_attribute(row, "class", "rinch-tcm-item");
            d.set_attribute(row, "data-tcm-item", &i.to_string());
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
            rows.push((action, row.0));
        }
        d.append_child(body, panel);
        TextMenuPanel {
            panel_id: panel.0,
            rows,
            spell: None,
        }
    }

    /// One plain row, `label` in a `rinch-tcm-label`, hidden.
    fn build_spell_row(d: &mut RinchDocument, class: &str, label: &str) -> (NodeId, NodeId) {
        let row = d.create_element("div");
        d.set_attribute(row, "class", class);
        d.set_style(row, "display", "none");
        let span = d.create_element("span");
        d.set_attribute(span, "class", "rinch-tcm-label");
        let text = d.create_text(label);
        d.append_child(span, text);
        d.append_child(row, span);
        (row, text)
    }

    fn build_spell_sep(d: &mut RinchDocument) -> NodeId {
        let sep = d.create_element("div");
        d.set_attribute(sep, "class", "rinch-tcm-sep");
        d.set_style(sep, "display", "none");
        let line = d.create_element("div");
        d.set_attribute(line, "class", "rinch-tcm-sep-line");
        d.append_child(sep, line);
        sep
    }

    /// Show the spelling rows for `spelling` (or hide them for `None`), adding
    /// the enabled ones to `items` in menu order. Built at the first word
    /// that needs them; a suggestion row is added when a list is longer than
    /// any before it, and kept.
    fn show_spell_rows(
        d: &mut RinchDocument,
        panel: &mut TextMenuPanel,
        spelling: Option<&TextSpelling>,
        items: &mut Vec<TextMenuRow>,
    ) {
        let Some(spelling) = spelling else {
            if let Some(rows) = panel.spell.as_mut().filter(|r| r.shown) {
                let all = rows.suggestions.iter().map(|&(row, _)| row).chain([
                    rows.none_row,
                    rows.sep_top,
                    rows.add_row,
                    rows.sep_bottom,
                ]);
                for id in all {
                    d.set_style(NodeId(id), "display", "none");
                }
                rows.shown = false;
            }
            return;
        };
        let pid = NodeId(panel.panel_id);
        let first_edit = NodeId(panel.rows[0].1);
        let rows = panel.spell.get_or_insert_with(|| {
            let (none_row, _) = Self::build_spell_row(
                d,
                "rinch-tcm-item rinch-tcm-no-suggestions",
                NO_SUGGESTIONS_LABEL,
            );
            d.set_attribute(none_row, "data-disabled", "");
            let sep_top = Self::build_spell_sep(d);
            let (add_row, _) = Self::build_spell_row(
                d,
                "rinch-tcm-item rinch-tcm-add-to-dictionary",
                ADD_TO_DICTIONARY_LABEL,
            );
            d.set_attribute(add_row, "data-tcm-item", "add");
            let sep_bottom = Self::build_spell_sep(d);
            for id in [none_row, sep_top, add_row, sep_bottom] {
                d.insert_before(pid, id, first_edit);
            }
            SpellRows {
                suggestions: Vec::new(),
                none_row: none_row.0,
                sep_top: sep_top.0,
                add_row: add_row.0,
                sep_bottom: sep_bottom.0,
                shown: false,
            }
        });
        while rows.suggestions.len() < spelling.suggestions.len() {
            let (row, text) = Self::build_spell_row(d, "rinch-tcm-item rinch-tcm-suggestion", "");
            d.set_attribute(row, "data-tcm-item", "suggestion");
            d.insert_before(pid, row, NodeId(rows.none_row));
            rows.suggestions.push((row.0, text.0));
        }
        let show = |d: &mut RinchDocument, id: u64, on: bool, display: &str| {
            d.set_style(NodeId(id), "display", if on { display } else { "none" });
        };
        for (i, &(row, text)) in rows.suggestions.iter().enumerate() {
            match spelling.suggestions.get(i) {
                Some(word) => {
                    d.set_text_content(NodeId(text), word);
                    d.remove_attribute(NodeId(row), "data-highlighted");
                    show(d, row, true, "flex");
                    items.push(TextMenuRow {
                        action: TextEditAction::Suggestion(i),
                        node_id: row,
                        enabled: true,
                    });
                }
                None => show(d, row, false, "flex"),
            }
        }
        let none = spelling.suggestions.is_empty()
            && !spelling.suggestions_pending
            && spelling.show_no_suggestions;
        show(d, rows.none_row, none, "flex");
        let above = none || !spelling.suggestions.is_empty();
        let add = spelling.can_add_to_dictionary;
        show(d, rows.sep_top, above && add, "block");
        show(d, rows.add_row, add, "flex");
        if add {
            d.remove_attribute(NodeId(rows.add_row), "data-highlighted");
            items.push(TextMenuRow {
                action: TextEditAction::AddToDictionary,
                node_id: rows.add_row,
                enabled: true,
            });
        }
        show(d, rows.sep_bottom, above || add, "block");
        rows.shown = true;
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
        let in_document = |id: u64| {
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
            // A field disabled under the menu closes it: its rows would act
            // on a control that refuses them (#822).
            FocusTarget::Input(id) => {
                d.tree.get(id).is_some_and(Self::is_text_like_field)
                    && in_document(id)
                    && !Self::node_is_disabled_in_tree(&d.tree, id)
            }
            #[cfg(feature = "windowing")]
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
        let mut cur = self.shared_hit(&d, x, y);
        // A row is a `data-tcm-item` **inside the panel**: one anywhere else
        // (pasted into a document, an app's own markup) is no row of this menu.
        // It is found by its node among the open rows (the spelling rows
        // shift the edit rows' places, #2862).
        let mut row = None;
        while let Some(nid) = cur {
            let Some(node) = d.tree.get(nid) else { break };
            if row.is_none() && node.attributes.contains_key("data-tcm-item") {
                row = Some(nid);
            }
            if nid == open.panel_id {
                return row
                    .and_then(|r| open.items.iter().position(|i| i.node_id == r))
                    .map_or(MenuHit::InsidePanel, MenuHit::Item);
            }
            cur = node.parent;
        }
        MenuHit::Outside
    }

    /// What an open **runtime popup** — this menu, or the native `<select>`
    /// popup — does with the events that are neither a pointer event nor a
    /// key (issue #822). The rule, decided once for both: a popup owns the
    /// input while it is open, as a native popup menu does, and closes when
    /// the window changes under it.
    ///
    /// - **A resize closes it** (Chrome closes its menus and its `<select>`
    ///   popup on a bounds change): its panel was placed against the old
    ///   window, and would otherwise hang off the new edge until closed. The
    ///   `<select>` hands the keyboard back to its control, as Escape does.
    /// - **An OS file drag closes it**, and goes on to the page: the user is
    ///   doing something else now, and dropping their file silently would be
    ///   worse than a closed menu.
    /// - **Composition is the menu's while it is open**: the input method is
    ///   off ([`Self::ime_state`]), and a preedit or commit that was already
    ///   on its way is swallowed, as every plain key is. `Enabled`/`Disabled`
    ///   and surrounding-text edits pass, so the field's composition state is
    ///   still cleared.
    ///
    /// Returns `true` when the event is consumed.
    pub(crate) fn runtime_popup_intercept(
        &mut self,
        event: &PlatformEvent,
        vp_w: f32,
        vp_h: f32,
        actions: &mut Vec<AppAction>,
    ) -> bool {
        let menu = self.open_text_menu.is_some();
        let select = self.is_select_open();
        if !menu && !select {
            return false;
        }
        match event {
            PlatformEvent::Resized { .. }
            | PlatformEvent::FileHoverEnter { .. }
            | PlatformEvent::FileDropped { .. } => {
                self.close_text_context_menu();
                if select {
                    self.close_select_popup_returning_focus();
                }
                self.resolve_and_repaint(vp_w, vp_h);
                actions.push(AppAction::RequestRedraw);
                false
            }
            PlatformEvent::Ime(ImeEvent::Preedit { .. } | ImeEvent::Commit(_)) => menu,
            _ => false,
        }
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
            // A **secondary** press outside closes this menu and goes on, so
            // it opens the context menu of whatever it landed on — this one
            // again at the new point on a field — as GTK and Chrome do (#822).
            // Other presses outside are swallowed.
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Right,
            } if matches!(self.text_menu_hit(x, y), MenuHit::Outside) => {
                self.close_text_context_menu();
                self.resolve_and_repaint(vp_w, vp_h);
                None
            }
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
                // A disabled row is not highlighted by hover, as in a native
                // menu; an enabled one is.
                let hovered = match self.text_menu_hit(x, y) {
                    MenuHit::Item(idx)
                        if self
                            .open_text_menu
                            .as_ref()
                            .is_some_and(|m| m.items.get(idx).is_some_and(|r| r.enabled)) =>
                    {
                        Some(idx)
                    }
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

// ── Spelling (#2862, stage 3) ───────────────────────────────────────────────

impl RinchApp {
    /// [`TextEditState::spelling`]: the misspelled word the last context
    /// gesture selected, while it is still the selection and still marked.
    fn live_spelling(&self) -> Option<TextSpelling> {
        let t = self.live_spell_target()?;
        Some(TextSpelling {
            word: t.word.clone(),
            suggestions: t.suggestions.clone(),
            suggestions_pending: t.pending,
            can_add_to_dictionary: t.add,
            show_no_suggestions: t.no_row,
        })
    }

    fn live_spell_target(&self) -> Option<&SpellTarget> {
        let t = self.text_spell.as_ref()?;
        if t.target != self.focus_target {
            return None;
        }
        match t.target {
            FocusTarget::Input(id) => {
                let state = self.focused_input_state.as_ref()?;
                if state.selection.start().0 != t.range.start
                    || state.selection.end().0 != t.range.end
                {
                    return None;
                }
                let text = state.document.to_text();
                (text.get(t.range.clone()) == Some(t.word.as_str())
                    && self.spell_misspelling_at(id, t.range.start, &text) == Some(t.range.clone()))
                .then_some(t)
            }
            #[cfg(feature = "windowing")]
            FocusTarget::Editor(container) => {
                let handle = crate::editor::editor_for_doc(self.doc_key(), container)?;
                let selection = handle.selection();
                let m = handle.misspelling_at(rinch_editor_core::Pos(t.range.start))?;
                (selection.from().0 == t.range.start
                    && selection.to().0 == t.range.end
                    && m.from.0 == t.range.start
                    && m.to.0 == t.range.end
                    && m.word == t.word)
                    .then_some(t)
            }
            _ => None,
        }
    }

    /// After the caret rule in a field: a press at byte `offset` of a word
    /// the spellchecker marked selects the word (Chrome's rule), unless the
    /// press kept a selection that is something else (a press inside a wider
    /// selection keeps it and offers no spelling, as in Chrome), and starts
    /// its suggestions.
    fn spell_select_in_input(&mut self, node_id: u64, offset: usize) {
        if self.spelling.is_empty() {
            return;
        }
        let Some(state) = self.focused_input_state.as_ref() else {
            return;
        };
        let text = state.document.to_text();
        let Some(range) = self.spell_misspelling_at(node_id, offset, &text) else {
            return;
        };
        let (start, end) = (state.selection.start().0, state.selection.end().0);
        if !state.selection.is_cursor() && (start, end) != (range.start, range.end) {
            return;
        }
        let word = text[range.clone()].to_owned();
        if let Some(state) = self.focused_input_state.as_mut() {
            state.selection = Selection::new(range.start, range.end);
        }
        self.sync_input_cursor_to_dom();
        self.scene_dirty = true;
        self.text_spell = self.new_spell_target(FocusTarget::Input(node_id), range, word);
    }

    /// The editor's half of [`Self::spell_select_in_input`].
    #[cfg(feature = "windowing")]
    fn spell_select_in_editor(&mut self, container: u64, press: &EditorContextPress) {
        let Some(EditorPressAt::Pos(pos)) = press.at else {
            return;
        };
        let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container) else {
            return;
        };
        let Some(m) = handle.misspelling_at(pos) else {
            return;
        };
        let selection = handle.selection();
        if !selection.is_empty() && (selection.from(), selection.to()) != (m.from, m.to) {
            return;
        }
        handle.set_selection(rinch_editor_core::Selection::text(m.from, m.to));
        self.text_spell =
            self.new_spell_target(FocusTarget::Editor(container), m.from.0..m.to.0, m.word);
    }

    /// The word's menu entry, its suggestions asked for. They are computed on
    /// the one suggestion thread ([`rinch_spell::runtime::suggest`], which
    /// drops a request a newer one supersedes, so repeated presses never pile
    /// up), and the press waits up to
    /// [`menu_suggest_wait_ms`](rinch_spell::runtime::SpellcheckConfig::menu_suggest_wait_ms)
    /// for them; later ones arrive through the app's inbox
    /// ([`Self::text_spell_arrived`]). Under inline jobs (tests, wasm) they are
    /// computed here.
    fn new_spell_target(
        &mut self,
        target: FocusTarget,
        range: std::ops::Range<usize>,
        word: String,
    ) -> Option<SpellTarget> {
        let session = rinch_spell::runtime::session()?;
        self.text_spell_requests += 1;
        let request = self.text_spell_requests;
        let max = session.menu_suggestions();
        let checker = session.loaded_checker().filter(|_| max > 0);
        let (suggestions, pending) = match checker {
            None => (Vec::new(), false),
            Some(checker) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let sender = self.app_work_sender();
                rinch_spell::runtime::suggest(checker, word.clone(), max, move |found| {
                    // Inline, the receiver below takes it and the inbox copy
                    // finds the target no longer pending.
                    let _ = tx.send(found.clone());
                    sender.send(move |app| app.text_spell_arrived(request, found));
                });
                let wait =
                    std::time::Duration::from_millis(u64::from(session.menu_suggest_wait_ms()));
                match rx.recv_timeout(wait) {
                    Ok(found) => (found, false),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (Vec::new(), true),
                    // Superseded unrun (another window's press): none.
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => (Vec::new(), false),
                }
            }
        };
        Some(SpellTarget {
            target,
            range,
            word,
            suggestions,
            pending,
            request,
            add: session.menu_add_to_dictionary(),
            no_row: session.menu_no_suggestions_row(),
        })
    }

    /// Suggestions that took longer than the press's wait arrived: fill them
    /// in, and redraw the menu that is waiting for them.
    pub(crate) fn text_spell_arrived(&mut self, request: u64, found: Vec<String>) {
        let Some(t) = self.text_spell.as_mut() else {
            return;
        };
        if t.request != request || !t.pending {
            return;
        }
        t.suggestions = found;
        t.pending = false;
        if let Some(open) = &self.open_text_menu
            && open.spell_request == Some(request)
        {
            let ((x, y), (w, h)) = (open.at, open.viewport);
            // The row the user had arrowed or pointed onto stays highlighted
            // (by its action: the new rows shift its place).
            let kept = open.highlighted.map(|i| open.items[i].action);
            self.open_text_context_menu(x, y, w, h);
            let at = kept.and_then(|action| {
                self.open_text_menu
                    .as_ref()?
                    .items
                    .iter()
                    .position(|r| r.action == action && r.enabled)
            });
            if at.is_some() {
                self.set_text_menu_highlight(at);
                self.resolve_and_repaint(w, h);
            }
        }
    }

    /// [`TextEditAction::Suggestion`]: the word is selected (the spelling
    /// state says so), so the suggestion goes in as typed text replacing the
    /// selection, through the path a typed key takes: one undo step, `oninput`
    /// (the field) or `on_change` (the editor), the caret after it, and the
    /// word's mark dropped by the edit itself.
    fn replace_misspelling(&mut self, index: usize) -> Vec<AppAction> {
        let Some(t) = self.live_spell_target() else {
            return Vec::new();
        };
        let Some(replacement) = t.suggestions.get(index).cloned() else {
            return Vec::new();
        };
        let target = t.target;
        self.text_spell = None;
        match target {
            FocusTarget::Input(_) => {
                self.handle_input_edit_command(EditCommand::InsertText(replacement));
            }
            #[cfg(feature = "windowing")]
            FocusTarget::Editor(container) => {
                if let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container) {
                    handle.insert_text(&replacement);
                }
                self.refresh_editor_overlays();
            }
            _ => return Vec::new(),
        }
        self.scene_dirty = true;
        vec![AppAction::RequestRedraw]
    }

    /// [`TextEditAction::AddToDictionary`]: the word joins the personal
    /// dictionary; every mark of it goes now, in every field (here) and every
    /// editor (their own listeners). The selection stays, as in Chrome.
    fn add_misspelling_to_dictionary(&mut self) -> Vec<AppAction> {
        let Some(word) = self
            .live_spell_target()
            .filter(|t| t.add)
            .map(|t| t.word.clone())
        else {
            return Vec::new();
        };
        self.text_spell = None;
        if !rinch_spell::runtime::add_to_dictionary(&word) {
            return Vec::new();
        }
        self.spell_words_accepted();
        #[cfg(feature = "windowing")]
        self.refresh_editor_overlays();
        self.scene_dirty = true;
        vec![AppAction::RequestRedraw]
    }
}

/// `r` intersected with `bounds`, both `(x, y, w, h)`; a rect entirely outside
/// collapses to a 1px-wide, 0-high rect at the nearest edge.
fn clip_rect(r: (f32, f32, f32, f32), bounds: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let left = r.0.max(bounds.0).min(bounds.0 + bounds.2);
    let top = r.1.max(bounds.1).min(bounds.1 + bounds.3);
    let right = (r.0 + r.2).min(bounds.0 + bounds.2).max(left);
    let bottom = (r.1 + r.3).min(bounds.1 + bounds.3).max(top);
    (left, top, (right - left).max(1.0), bottom - top)
}

/// The bounding box of two caret rects `(x, y, w, h)`: on one line, the rect
/// from the first caret to the second; across lines, left and right from the
/// two carets and top of the first to the bottom of the last.
fn union_of_caret_rects(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let left = a.0.min(b.0);
    let top = a.1.min(b.1);
    let right = (a.0 + a.2).max(b.0 + b.2);
    let bottom = (a.1 + a.3).max(b.1 + b.3);
    (left, top, (right - left).max(1.0), bottom - top)
}
