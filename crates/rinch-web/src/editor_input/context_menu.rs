//! The browser's own context menu over an editor (issue #814): parking the
//! capture textarea under the pointer for one menu cycle.

use super::capture::capture_target;
use super::mirror::clear_mirror;
use super::*;

// ── The browser's own context menu (issue #814) ───────────────────────────────
//
// The browser builds its right-click menu for the element its own hit test finds
// at the press point, and the editor's surface is a plain `<div>`: the menu for one
// has no Paste and no Cut. A page cannot draw its own — `navigator.clipboard.
// readText()` prompts — so the fix is CodeMirror 5's: park the capture textarea
// under the pointer at the right-button press, so the hit test finds an editable.
//
// A cycle runs like this:
//
// 1. `handle_mousedown` — a right press, or a Control-click on macOS — applies the
//    right-press caret rule, then `start_context_menu_cycle` fills the textarea
//    with the editor's selection as text between two **sentinels**, selected
//    between them, so the menu offers Copy / Cut exactly when the editor has a
//    selection; and **parks** it under the pointer — `pointer-events: auto`, on
//    top, `opacity: 0`. The mirror is dropped for the duration (`sync_mirror` is a
//    no-op meanwhile). The menu key and Shift+F10 start a cycle too, parked at the
//    caret — unless an app's `data-oncontextmenu` claims the editor, which then
//    gets the key's menu instead. A right press on a link or an image outside the
//    selection starts none: the browser's own link or image menu is the one
//    wanted there.
// 2. The browser fires `contextmenu` at the textarea and hit-tests the point for
//    its menu while it is still handling the input event that parked it (Chrome,
//    measured: before any timer that event's listener set). So a park lasts
//    `PARK_MS` whether or not a menu came, and a click on the page after that
//    reaches the page. Inside those 100 ms a click in the 30 px box goes to the
//    textarea rather than to what is under it (measured). Where the menu follows
//    the **release** instead (Windows, by reading Chromium), a release over the
//    cycle's own editor parks the textarea again if no `contextmenu` has come for
//    the cycle yet.
// 3. The menu's items fire on the *focused* editable, wherever it is: `paste`,
//    `cut` and `copy` are answered from the model by the listeners above, and so is
//    `beforeinput`. Undo and Redo may be aimed at another field on the page
//    instead, and are taken back for the editor (`on_foreign_history_input`).
//    Select All is the exception — it selects the textarea's contents, which the
//    sentinels make detectable (`select_all_chosen`) — and is translated into the
//    editor's `selectAll` when the field's `select` / `selectionchange` reports it.
// 4. The cycle **ends** — `end_context_menu_cycle` restores the hidden style and
//    re-mirrors the caret's block — on the first of: any of those events, the next
//    key or pointer press, or focus moving to another element. Never on a timer: a
//    menu stays open as long as the user leaves it, and its Select All can only be
//    detected while the sentinels are in the field.

/// The textarea's resting style: off-screen, invisible, and non-interactive
/// (`pointer-events: none` so it never becomes a mouse target / steals a click)
/// yet still programmatically focusable.
pub(super) const HIDDEN_CAPTURE_STYLE: &str = "position: fixed; top: 0; left: 0; width: 1px; \
     height: 1px; padding: 0; margin: -1px; border: 0; opacity: 0; overflow: hidden; \
     resize: none; pointer-events: none; outline: none; z-index: -1; white-space: pre;";

/// The side of the square the parked textarea covers.
pub(super) const PARK_SIZE: f32 = 30.0;

/// How long a park lasts. The browser fires the `contextmenu` for the press, key
/// or release that parked the textarea — and hit-tests for its menu — while still
/// handling that input event, before a timer its listener set can run (measured in
/// Chrome 153 for a right press, a held right press, the menu key and Shift+F10),
/// so any delay covers it; this one leaves the margin CodeMirror leaves. It is
/// also how long a click inside the parked box is the textarea's rather than the
/// page's.
const PARK_MS: i32 = 100;

/// The zero-width space the parked field starts and ends with. The editor's own
/// writes select between the two, so a selection from 0 to the end can only be the
/// browser's Select All (CodeMirror's sentinel, which it puts at the start alone).
/// The second one is for a word selection: a right-click on macOS selects the word
/// under the pointer, inside the parked field, and one sentinel alone — the whole
/// field when the editor has no selection — is a word by itself (measured in Chrome,
/// double-clicking such a field: 0..1), where no word spans a sentinel at each end.
/// That macOS behaviour is not verified on a Mac.
const SELECT_ALL_SENTINEL: char = '\u{200b}';

/// The most of the selection's text the parked textarea carries. It is there to
/// make the field *have* a selection — Copy and Cut are answered from the model —
/// so a document-sized selection need not be serialized into it.
const PARKED_TEXT_CAP: usize = 1000;

/// What started a context-menu cycle: where a release parks the textarea again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuTrigger {
    /// A right press, or a Control-click on macOS: parked under the pointer.
    Pointer,
    /// The menu key or Shift+F10: parked at the caret.
    Key,
}

/// A context-menu cycle in flight.
pub(super) struct MenuCycle {
    id: u64,
    /// The host id of the editor the cycle belongs to.
    container_nid: u64,
    trigger: MenuTrigger,
    /// The park holding the textarea under the pointer, if any — the id of that
    /// park, so its timer does not end a later one.
    park: Option<u64>,
    /// Whether the browser has fired this cycle's `contextmenu` at the textarea.
    /// Until it has, a release may be what brings the menu, and parks again.
    menu_fired: bool,
    /// The model selection when the cycle began. Select All is translated only
    /// while it is unchanged: an edit in between means the menu is long gone.
    selection: Selection,
}

/// The live cycle's id, if any.
pub(super) fn menu_cycle_id() -> Option<u64> {
    MENU_CYCLE.with(|c| c.borrow().as_ref().map(|m| m.id))
}

/// A fresh id for a cycle or a park.
fn next_menu_id() -> u64 {
    MENU_CYCLE_SEQ.with(|c| {
        let next = c.get() + 1;
        c.set(next);
        next
    })
}

/// The style that parks the textarea as a `PARK_SIZE` square with its top-left
/// corner at `(left, top)` (viewport coordinates): hittable and on top of
/// everything, invisible.
pub(super) fn parked_capture_style(left: f32, top: f32) -> String {
    format!(
        "position: fixed; left: {left}px; top: {top}px; width: {PARK_SIZE}px; height: {PARK_SIZE}px; \
         padding: 0; margin: 0; border: 0; opacity: 0; overflow: hidden; resize: none; \
         pointer-events: auto; outline: none; z-index: 2147483647; white-space: pre; \
         background: transparent;"
    )
}

/// The top-left corner of a park centred on the pointer at `(x, y)`.
pub(super) fn park_around(x: f32, y: f32) -> (f32, f32) {
    (x - PARK_SIZE / 2.0, y - PARK_SIZE / 2.0)
}

/// The top-left corner of a park whose **left edge** is at `x`, centred on `y`.
/// Chrome opens a keyboard-invoked menu at the focused box's left edge
/// (measured), so a park for the menu key starts at the caret rather than
/// around it.
pub(super) fn park_right_of(x: f32, y: f32) -> (f32, f32) {
    (x, y - PARK_SIZE / 2.0)
}

/// Whether a selection of `start..end` over a field of `len` UTF-16 units is the
/// browser's Select All: the whole field, both sentinels included. The editor's own
/// writes never start at 0 while a sentinel is in place.
pub(super) fn select_all_chosen(start: u32, end: u32, len: u32) -> bool {
    len > 0 && start == 0 && end == len
}

/// Whether a right-click at `target` belongs to an app handler: an element up its
/// chain carrying a **live** `data-oncontextmenu` (the same test the generic
/// delegation makes before dispatching one). An app that wants its own menu over
/// an editor attaches one, and then gets it, as over anything else it wraps.
pub(super) fn context_menu_claimed_by_app(target: &web_sys::Element) -> bool {
    nearest_handler(target, "data-oncontextmenu")
        .is_some_and(|(_, id)| rinch_core::events::has_click_handler(id))
}

/// Whether a `mousedown` opens the browser's context menu: the right button
/// anywhere, and on macOS a *left* press with Control held, which the browser
/// answers with `contextmenu` just as it does a right press (Chromium's
/// `WebFrameWidgetImpl::HandleMouseDown`, by reading; not verified on a Mac).
pub(super) fn is_context_press(event: &web_sys::MouseEvent) -> bool {
    event.button() == 2 || (event.button() == 0 && event.ctrl_key() && is_mac())
}

/// Run `f` after `ms` on the page's timer.
fn after(ms: i32, f: impl FnOnce() + 'static) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let cb = Closure::once_into_js(f);
    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), ms);
}

/// Start a context-menu cycle inside the focused editor: fill the capture
/// textarea with the sentinels around the editor's selection and park it with its
/// top-left corner at `spot` (see the section comment above).
pub(super) fn start_context_menu_cycle(
    handle: &EditorHandle,
    container_nid: u64,
    trigger: MenuTrigger,
    spot: (f32, f32),
) {
    // A cycle still open from an earlier press ends first, so its sentinels are
    // not read as this one's.
    end_context_menu_cycle();
    let Some(ta) = capture_target() else {
        return;
    };
    let selection = handle.selection();
    let selected = if selection.is_empty() {
        String::new()
    } else {
        let text: String = handle
            .selection_clipboard()
            .map(|(_, text)| text)
            .unwrap_or_default()
            .chars()
            .take(PARKED_TEXT_CAP)
            .collect();
        // A node selection (an image, a rule) has no text; the field still needs a
        // character to select or the menu greys Copy and Cut out.
        if text.is_empty() {
            " ".to_string()
        } else {
            text
        }
    };
    let value = format!("{SELECT_ALL_SENTINEL}{selected}{SELECT_ALL_SENTINEL}");
    // The field no longer mirrors anything; `end_context_menu_cycle` rebuilds it.
    MIRROR.with(|m| *m.borrow_mut() = None);
    ta.set_value(&value);
    let len = value.encode_utf16().count() as u32;
    let _ = ta.set_selection_range(1, len - 1);
    let id = next_menu_id();
    MENU_CYCLE.with(|c| {
        *c.borrow_mut() = Some(MenuCycle {
            id,
            container_nid,
            trigger,
            park: None,
            menu_fired: false,
            selection,
        })
    });
    park_capture(spot);
}

/// Park the textarea of the live cycle with its top-left corner at `spot`, for
/// `PARK_MS`.
fn park_capture(spot: (f32, f32)) {
    let park = next_menu_id();
    let parked = MENU_CYCLE.with(|c| match c.borrow_mut().as_mut() {
        Some(m) => {
            m.park = Some(park);
            true
        }
        None => false,
    });
    if !parked {
        return;
    }
    if let Some(ta) = capture_target() {
        let _ = ta.set_attribute("style", &parked_capture_style(spot.0, spot.1));
    }
    after(PARK_MS, move || unpark_capture(park));
}

/// Send the textarea back off-screen if park `park` still holds it. The cycle
/// itself stays open for the menu's items.
fn unpark_capture(park: u64) {
    let was_parked = MENU_CYCLE.with(|c| match c.borrow_mut().as_mut() {
        Some(m) if m.park == Some(park) => {
            m.park = None;
            true
        }
        _ => false,
    });
    if was_parked && let Some(ta) = capture_target() {
        let _ = ta.set_attribute("style", HIDDEN_CAPTURE_STYLE);
    }
}

/// A release of whatever started a `trigger` cycle — the right button, the menu
/// key. Where the browser opens its menu at the release rather than the press
/// (Windows, by reading Chromium), the park from the press is long gone by then,
/// so a release that no `contextmenu` has preceded parks the textarea again, at
/// the `spot` it gives for the cycle's editor (`None` parks nothing).
pub(super) fn repark_on_release(
    trigger: MenuTrigger,
    spot: impl FnOnce(u64) -> Option<(f32, f32)>,
) {
    let waiting = MENU_CYCLE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|m| m.trigger == trigger && !m.menu_fired)
            .map(|m| m.container_nid)
    });
    if let Some(container_nid) = waiting
        && let Some(spot) = spot(container_nid)
    {
        park_capture(spot);
    }
}

/// Whether viewport point `(x, y)` is over the editor whose container is
/// `container_nid`, looking past the capture textarea, which may still be parked
/// there. A right press dragged out of the editor and released over page content
/// is that content's to answer.
pub(super) fn point_over_editor(
    doc: &web_sys::Document,
    x: f32,
    y: f32,
    container_nid: u64,
) -> bool {
    doc.elements_from_point(x, y)
        .iter()
        .filter_map(|v| v.dyn_into::<web_sys::Element>().ok())
        .find(|el| !el.has_attribute("data-pm-capture"))
        .and_then(|el| el.closest("[data-pm-editor]").ok().flatten())
        .and_then(|editor| get_nid(&editor.into()))
        .is_some_and(|nid| nid.0 == container_nid)
}

/// [`select_all_chosen`] read off the field.
fn select_all_chosen_in(ta: &web_sys::HtmlTextAreaElement) -> bool {
    let len = ta.value().encode_utf16().count() as u32;
    let start = ta.selection_start().ok().flatten().unwrap_or(0);
    let end = ta.selection_end().ok().flatten().unwrap_or(0);
    select_all_chosen(start, end, len)
}

/// The field's `select` / `selectionchange`: how the page hears of the menu's
/// Select All, which changes nothing but the field's selection, however long
/// after the menu opened it is chosen.
pub(super) fn on_select() {
    if menu_cycle_id().is_some()
        && let Some(ta) = capture_target()
        && select_all_chosen_in(&ta)
    {
        end_context_menu_cycle();
    }
}

/// End the cycle in flight, if any: unpark the textarea, translate a Select All
/// the menu chose, and put the mirror back — the field held the sentinels, so it
/// is rebuilt from the caret's block. A no-op when no cycle is live, which is
/// what lets every event path call it unconditionally.
pub(super) fn end_context_menu_cycle() {
    let Some(cycle) = MENU_CYCLE.with(|c| c.borrow_mut().take()) else {
        return;
    };
    let Some(ta) = capture_target() else {
        return;
    };
    if cycle.park.is_some() {
        let _ = ta.set_attribute("style", HIDDEN_CAPTURE_STYLE);
    }
    let chose_select_all = select_all_chosen_in(&ta);
    match focused_handle() {
        Some((_, handle)) => {
            if chose_select_all && handle.selection() == cycle.selection {
                handle.command("selectAll");
            }
            clear_mirror(&ta);
            refresh_caret();
        }
        None => clear_mirror(&ta),
    }
}

/// The `contextmenu` the browser fires for a context press or key. When it targets
/// the parked textarea, the browser's hit test has found the editable and builds
/// its menu for it in this event's default action; the cycle's park ends on its
/// own timer, and no release needs to park again.
pub(super) fn handle_contextmenu(event: &web_sys::MouseEvent) {
    if event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|el| el.has_attribute("data-pm-capture"))
    {
        MENU_CYCLE.with(|c| {
            if let Some(m) = c.borrow_mut().as_mut() {
                m.menu_fired = true;
            }
        });
    }
}

/// Undo / Redo from the browser's menu, aimed at the wrong field.
///
/// The menu's Undo runs the page's own undo stack, and Chrome dispatches its
/// `historyUndo` at the element that owns the newest step there, not at the focused
/// field: with the editor focused and an `<input>` elsewhere typed into earlier, it
/// undoes that input and moves focus to it (measured in Chrome 153). The editor never
/// adds a step to another field — its keys and edits are cancelled, and an IME commit
/// adds one to the capture textarea at most — so while the capture textarea holds
/// focus, such an event aimed at another element can only be the user asking to undo
/// the editor. One aimed at the textarea itself is `on_before_input`'s.
pub(super) fn on_foreign_history_input(event: &web_sys::InputEvent) {
    let command = match event.input_type().as_str() {
        "historyUndo" => "undo",
        "historyRedo" => "redo",
        _ => return,
    };
    let Some(ta) = capture_target() else {
        return;
    };
    let on_capture = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|el| el.has_attribute("data-pm-capture"));
    let focused = ta
        .owner_document()
        .and_then(|d| d.active_element())
        .as_deref()
        == Some(ta.as_ref());
    if on_capture || !focused {
        return;
    }
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    event.prevent_default();
    end_context_menu_cycle();
    if handle.command(command) {
        refresh_caret();
    }
}
