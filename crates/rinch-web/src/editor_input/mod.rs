//! Browser input glue for the rich-text editor.
//!
//! The editor view ([`RinchDomEditorView`](rinch_editor_view::RinchDomEditorView))
//! projects the model onto the page's real DOM via [`WebDocument`](crate::WebDocument);
//! the container is deliberately **not** `contenteditable`. This module is the platform
//! half the desktop runtime provides on its side: it turns browser keyboard / pointer /
//! clipboard events into [`EditorHandle`] calls, mirroring `dispatch_new_editor_key` and
//! `try_new_editor_click` in the desktop `app::event_dispatch`. The renderer-agnostic
//! model logic (caret motion, word/block selection) is shared via the handle; only this
//! event plumbing and the pointer→`Pos` geometry are web-specific.
//!
//! Listeners are installed once on `document` (capture phase, so an editor consumes its
//! keys/clicks before the generic delegation runs) and leaked for the page lifetime,
//! matching [`setup_event_delegation`](crate::setup_event_delegation).
//!
//! **Focus is click-driven** (design A10, like desktop): a `mousedown` inside a
//! `[data-pm-editor]` focuses that editor; a `mousedown` into another text field blurs
//! it. Physical keys (all Latin text, shortcuts, navigation, selection) are handled via
//! the document-level listeners — no `contenteditable` needed. A *touch* tap also
//! focuses the capture target from its `pointerup` (see [`handle_touch_tap`]), because
//! the compatibility `mousedown` a tap synthesizes is too late to count as the user
//! gesture iOS requires before it will raise the on-screen keyboard.
//!
//! **Software keyboards never reach `keydown`.** Android reports every printable key as
//! `key: "Unidentified"` / `keyCode: 229` and carries the text on `beforeinput` alone;
//! the same is true of swipe typing, dictation, and the platform edit menu. So the
//! capture target also listens for `beforeinput` ([`on_before_input`]), which maps the
//! event's `inputType` onto the same handle calls the keymap uses. The two channels
//! cannot double up: a key `handle_keydown` consumes is `preventDefault`ed, and a
//! cancelled `keydown` fires no `beforeinput`.
//!
//! **Clipboard + IME ride a hidden capture target.** A plain `<div>` (the container is
//! deliberately not `contenteditable`) receives no `paste`/`cut`/`copy`/
//! `compositionstart` events, so we keep one shared, focused, off-screen `<textarea>`
//! ([`ensure_capture_target`]) as the browser's idea of the focused editable: focusing
//! it on editor-focus makes those native events fire (they target it), and makes focus
//! browser-native so keys can't route to the wrong control. It is never shown, and
//! outside a context-menu cycle (below) it holds exactly one textblock — the caret's —
//! mirrored there so a soft keyboard has real text to replace (see [`sync_mirror`]);
//! the document itself never lives in it.
//! Typed characters from a *physical* keyboard are consumed (and `preventDefault`ed) by
//! the keydown handler before the textarea sees them. This mirrors the CodeMirror /
//! ProseMirror hidden-input technique.
//!
//! **The same textarea is what gives the editor the browser's own right-click menu**
//! (issue #814). A right-click on the visible surface would target a plain element, and
//! the menu the browser builds for one has no Paste and no Cut; the editor draws no menu
//! of its own on the web, because a page's paste needs `navigator.clipboard.readText()`,
//! which prompts. So at a right-button press the capture textarea is **parked under
//! the pointer** — invisible, but hittable — for as long as it takes the browser to
//! fire `contextmenu` and hit-test the point for its menu; the menu it builds is then
//! the editing one, and its Paste / Cut / Copy fire the ordinary clipboard events on
//! the (still focused) textarea, which the listeners below answer from the model.
//! For that cycle the field holds the editor's selection between two sentinels
//! rather than the mirror. See [`start_context_menu_cycle`] for the whole cycle,
//! CodeMirror 5's `TextareaInput.onContextMenu` for the prior art.
//!
//! **Links are the app's.** An editor link is an `<a href>` in the page, which the
//! browser would follow on a click. A primary press on one is offered to the
//! editor's `on_link_click` first ([`handle_mousedown`]), the pointer entering and
//! leaving links is reported to `on_link_hover` ([`update_link_hover`]). A
//! click is where keyboard activation is offered and where the editor decides
//! whether the browser follows the link ([`handle_link_click`]): not in an
//! editable editor or one with a link-click callback, natively in a read-only
//! one without either. The right-click menu keeps the browser's own link menu
//! (Open link, Copy link address).
//!
//! [`ensure_capture_target`]: capture::ensure_capture_target
//! [`handle_link_click`]: pointer::handle_link_click
//! [`handle_mousedown`]: pointer::handle_mousedown
//! [`handle_touch_tap`]: touch::handle_touch_tap
//! [`on_before_input`]: text_input::on_before_input
//! [`start_context_menu_cycle`]: context_menu::start_context_menu_cycle
//! [`update_link_hover`]: pointer::update_link_hover

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rinch_editor_core::{CursorMotion, PasteContent, Pos, Selection};
use rinch_editor_view::{
    CaretAffinity, EditorHandle, ImageHover, ImageInputSource, LinkClick, LinkHover, LinkSpan,
    SelectionAnchor, registry, sniff_image_mime,
};

use crate::event_delegation::{
    drag_machine, modifiers_from_key_event, nearest_handler, set_click_context_for,
};
use crate::web_document::{find_text_node_at_byte_offset, get_nid, node_by_nid};

thread_local! {
    /// Container node-id of the focused editor (`None` = no editor focused). Drives
    /// keyboard routing and the caret pass. Set on `mousedown` into an editor.
    static FOCUSED_EDITOR: Cell<Option<u64>> = const { Cell::new(None) };
    /// The vertical-motion goal column (viewport x), kept across a run of Up/Down so the
    /// caret holds its horizontal position through short lines. Reset by any other key.
    static GOAL_X: Cell<Option<f32>> = const { Cell::new(None) };
    /// The caret's current blink phase (the `setInterval` flips it; an interaction
    /// resets it to visible so the caret is solid right after typing/moving).
    static BLINK_ON: Cell<bool> = const { Cell::new(true) };
    /// Install-once guard.
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// The shared hidden capture `<textarea>` (clipboard + IME focus target), created
    /// lazily on the first editor focus. There is only ever one focused editor, so one
    /// shared target suffices.
    static CAPTURE: RefCell<Option<web_sys::HtmlTextAreaElement>> = const { RefCell::new(None) };
    /// Whether an IME composition is in progress. While set, the keydown handler yields
    /// every key to the textarea + IME (the composed text arrives via composition events).
    static COMPOSING: Cell<bool> = const { Cell::new(false) };
    /// What we last wrote into the capture textarea — the caret's textblock, mirrored
    /// so a soft keyboard has real text to replace. `None` when no editor is focused.
    /// [`reconcile_mirror`] diffs the textarea against this to recover the edit.
    static MIRROR: RefCell<Option<Mirror>> = const { RefCell::new(None) };
    /// The in-flight touch/pen contact that started inside an editor, as
    /// `(pointer_id, client_x, client_y)`. A release near that point is a tap rather
    /// than a scroll, and focuses the capture target (see `handle_touch_tap`).
    static TOUCH_TAP: Cell<Option<(i32, f32, f32)>> = const { Cell::new(None) };
    /// The last touch tap [`handle_touch_tap`] serviced, waiting for the
    /// compatibility `mousedown` the browser synthesizes for it (issue #302). See
    /// [`is_compat_mousedown_of_serviced_tap`].
    static SERVICED_TAP: Cell<Option<ServicedTap>> = const { Cell::new(None) };
    /// Set by [`handle_mousedown`] when an app's `on_link_click` claimed the press,
    /// to the editor it landed in — read by [`handle_touch_tap`], which clears it
    /// before the call, so it can tell a claimed link tap from a caret tap.
    static LINK_CLAIMED_IN: Cell<Option<u64>> = const { Cell::new(None) };
    /// The context-menu cycle in flight (issue #814): the capture textarea is, or
    /// was just, parked under the pointer and holds the Select-All sentinels rather
    /// than the mirror. `None` when no cycle is live. See
    /// [`start_context_menu_cycle`].
    static MENU_CYCLE: RefCell<Option<MenuCycle>> = const { RefCell::new(None) };
    /// Issues each cycle and each park its id, so a timer armed for one is a no-op
    /// once a later one — or none — has replaced it.
    static MENU_CYCLE_SEQ: Cell<u64> = const { Cell::new(0) };
    /// Whether the last menu-key press went to an app's `data-oncontextmenu`
    /// rather than to the browser, so its release is withheld from the browser too
    /// (see [`handle_keydown`]).
    static MENU_KEY_TO_APP: Cell<bool> = const { Cell::new(false) };
}

mod capture;
mod clipboard;
mod context_menu;
mod install;
mod keyboard;
mod mirror;
mod motion;
mod pictures;
mod pointer;
#[cfg(test)]
mod tests;
mod text_input;
mod touch;

use capture::capture_target;
use context_menu::MenuCycle;
use mirror::{Mirror, sync_mirror};
use touch::ServicedTap;

pub(crate) use capture::{
    capture_target_doc, effective_active_element, give_editor_keyboard, keyboard_editor,
    release_editor,
};
pub(crate) use install::{add_capture, add_capture_on, install, install_before_build};

fn focused_editor() -> Option<u64> {
    FOCUSED_EDITOR.with(|c| c.get())
}
fn set_focused_editor(id: Option<u64>) {
    FOCUSED_EDITOR.with(|c| c.set(id));
}
fn goal_x() -> Option<f32> {
    GOAL_X.with(|c| c.get())
}
fn set_goal_x(x: Option<f32>) {
    GOAL_X.with(|c| c.set(x));
}

/// A resolved pointer hit inside an editor: the container + textblock host ids and the
/// model offset within the textblock's content ([`model_offset_in_block`]).
struct EditorHit {
    container_nid: u64,
    textblock_nid: u64,
    offset: usize,
}

impl EditorHit {
    /// The model position the hit names.
    fn pos(&self, handle: &EditorHandle) -> Option<Pos> {
        handle.pos_in_textblock(self.textblock_nid, self.offset)
    }
}

/// The model offset, within `textblock`'s content, of the DOM point
/// `(container, offset)` — a text character and an inline leaf (an element
/// carrying `data-pm-type`: an image, a hard break) each count one, as in the
/// model; mark and decoration wrappers count nothing of their own.
///
/// Counted from the DOM rather than as a flat byte offset because a leaf has no
/// text bytes: the byte offset just before a `<br>` and the one just after it
/// are the same number, which resolved a click at the start of the line after
/// a Shift+Enter to the position BEFORE the break (#1025). The DOM point knows
/// the side, whether it is `(text after the <br>, 0)` or an element point
/// `(<p>, index after the <br>)`.
fn model_offset_in_block(
    textblock: &web_sys::Element,
    container: &web_sys::Node,
    offset: u32,
) -> usize {
    /// Adds what `node` holds up to the point to `n`; `true` once the point is found.
    fn walk(node: &web_sys::Node, container: &web_sys::Node, offset: u32, n: &mut usize) -> bool {
        if node.node_type() == web_sys::Node::TEXT_NODE {
            let text = node.text_content().unwrap_or_default();
            if node == container {
                let mut units = 0u32;
                for c in text.chars() {
                    if units >= offset {
                        break;
                    }
                    units += c.len_utf16() as u32;
                    *n += 1;
                }
                return true;
            }
            *n += text.chars().count();
            return false;
        }
        let kids = node.child_nodes();
        let stop = if node == container {
            offset.min(kids.length())
        } else {
            kids.length()
        };
        for i in 0..stop {
            let Some(child) = kids.item(i) else { continue };
            let is_leaf = child
                .dyn_ref::<web_sys::Element>()
                .is_some_and(|el| el.has_attribute("data-pm-type"));
            if is_leaf && !child.contains(Some(container)) {
                *n += 1;
            } else if walk(&child, container, offset, n) {
                return true;
            }
        }
        node == container
    }
    let mut n = 0;
    walk(textblock, container, offset, &mut n);
    n
}

/// Resolve a viewport point to an editor position: walk up from the caret range's start
/// container to the innermost `[data-pm-type]` block (the textblock) and the enclosing
/// `[data-pm-editor]` container, then compute the within-block byte offset. Marks
/// (`[data-pm-mark]`) are skipped because only blocks carry `data-pm-type`.
fn resolve_editor_point(doc: &web_sys::Document, x: f32, y: f32) -> Option<EditorHit> {
    let (start, offset) = crate::event_delegation::caret_point_from_point(doc, x, y)?;

    let mut cur: Option<web_sys::Node> = Some(start.clone());
    let mut textblock: Option<web_sys::Element> = None;
    let mut container: Option<web_sys::Element> = None;
    while let Some(node) = cur {
        if let Ok(el) = node.clone().dyn_into::<web_sys::Element>() {
            if el.has_attribute("data-pm-editor") {
                container = Some(el);
                break;
            }
            // A point inside a node view's component names no editor position.
            if el.has_attribute(rinch_editor_view::NODE_VIEW_ATTR) && node != start {
                return None;
            }
            if textblock.is_none() && el.has_attribute("data-pm-type") {
                textblock = Some(el);
            }
        }
        cur = node.parent_node();
    }
    let container_nid = get_nid(&container?.into())?.0;
    let textblock = textblock?;
    let textblock_nid = get_nid(&textblock.clone().into())?.0;
    let offset = model_offset_in_block(&textblock, &start, offset);
    Some(EditorHit {
        container_nid,
        textblock_nid,
        offset,
    })
}

/// The focused editor's handle, if any (dropping focus if it has unmounted).
fn focused_handle() -> Option<(u64, EditorHandle)> {
    let id = focused_editor()?;
    match registry::editor_for(id) {
        Some(h) => Some((id, h)),
        None => {
            set_focused_editor(None);
            None
        }
    }
}

/// Re-render the focused editor's caret/selection from state (phase 2). On web the
/// geometry read forces a synchronous reflow, so this is correct right after the edit.
/// Resets the blink phase so the caret is solid immediately after an interaction.
fn refresh_caret() {
    BLINK_ON.with(|c| c.set(true));
    registry::update_all_carets(None, focused_editor());
    // Keep the capture textarea mirroring the caret's block. Every edit and every
    // caret move comes through here, which is exactly when a keyboard's idea of the
    // surrounding text goes stale.
    if let Some((_, handle)) = focused_handle() {
        sync_capture_read_only(&handle);
        sync_mirror(&handle);
    }
}

/// Queue [`refresh_caret`] for a microtask, if an overlay pass is still owed
/// then — see [`registry::set_overlay_pass_scheduler`]. Called with an editor's
/// borrow held, so it only queues.
fn schedule_overlay_pass() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let run = wasm_bindgen::closure::Closure::once_into_js(|| {
        if registry::any_overlay_pass_owed() {
            refresh_caret();
        }
    });
    window.queue_microtask(run.unchecked_ref());
}

/// Make the capture textarea `readonly` exactly while the focused editor is
/// read-only ([`EditorHandle::set_read_only`]).
///
/// The handle already refuses every edit, so this is not what keeps the document
/// safe. It is what makes the *browser* behave as it does for a read-only field,
/// which no amount of refusing afterwards can: a soft keyboard is not raised for
/// a tap, an IME does not start composing, the browser's own context menu (issue
/// #814) offers Copy but neither Paste nor Cut, and the field is never edited
/// behind the mirror by a keystroke the model turned down. Focus, the caret keys
/// and `copy` are untouched — a `readonly` field has all three.
///
/// One capture field serves every editor, so this is re-decided wherever the
/// focused editor or its switch can have changed: at focus (before the field is
/// focused, so the keyboard never flashes up) and on every [`refresh_caret`],
/// which `set_read_only` requests through the overlay refresher.
fn sync_capture_read_only(handle: &EditorHandle) {
    if let Some(ta) = capture_target() {
        let read_only = handle.refuses_edits();
        if ta.read_only() != read_only {
            ta.set_read_only(read_only);
        }
    }
}

thread_local! {
    /// A test's answer for [`is_mac`] (`__set_mac_platform`), so the macOS key
    /// bindings run in CI's Linux browsers. `None`: ask the browser.
    static MAC_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Make the editor's key handling take the page as macOS (`Some(true)`), as
/// not macOS (`Some(false)`), or ask the browser again (`None`). Test-only.
#[doc(hidden)]
pub fn __set_mac_platform(mac: Option<bool>) {
    MAC_OVERRIDE.with(|m| m.set(mac));
}

/// Whether the page runs on macOS, where a Control-click is the context click
/// (CodeMirror 5 asks `navigator.platform` the same question).
pub(crate) fn is_mac() -> bool {
    if let Some(mac) = MAC_OVERRIDE.with(std::cell::Cell::get) {
        return mac;
    }
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"navigator".into()).ok())
        .and_then(|n| js_sys::Reflect::get(&n, &"platform".into()).ok())
        .and_then(|p| p.as_string())
        .is_some_and(|p| p.starts_with("Mac"))
}

// ── Caret blink ──────────────────────────────────────────────────────────────

fn blink_tick() {
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    let on = !BLINK_ON.with(|c| c.get());
    BLINK_ON.with(|c| c.set(on));
    handle.set_caret_blink(on);
}
