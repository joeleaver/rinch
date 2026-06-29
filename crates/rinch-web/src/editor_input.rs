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
//! the document-level listeners — no `contenteditable` needed.
//!
//! **Clipboard + IME ride a hidden capture target.** A plain `<div>` (the container is
//! deliberately not `contenteditable`) receives no `paste`/`cut`/`copy`/
//! `compositionstart` events, so we keep one shared, focused, off-screen `<textarea>`
//! ([`ensure_capture_target`]) as the browser's idea of the focused editable: focusing
//! it on editor-focus makes those native events fire (they target it), and makes focus
//! browser-native so keys can't route to the wrong control. It is never shown and never
//! holds document text — typed characters are consumed (and `preventDefault`ed) by the
//! keydown handler before the textarea sees them; only IME composition flows through it,
//! and its value is cleared on commit. This mirrors the CodeMirror / ProseMirror
//! hidden-input technique.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rinch_editor_core::{CursorMotion, Pos, Selection};
use rinch_editor_view::{EditorHandle, registry};

use crate::event_delegation::compute_byte_offset_in_block;
use crate::web_document::{find_text_node_at_byte_offset, get_nid, node_by_nid};

thread_local! {
    /// Container node-id of the focused editor (`None` = no editor focused). Drives
    /// keyboard routing and the caret pass. Set on `mousedown` into an editor.
    static FOCUSED_EDITOR: Cell<Option<usize>> = const { Cell::new(None) };
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
}

fn focused_editor() -> Option<usize> {
    FOCUSED_EDITOR.with(|c| c.get())
}
fn set_focused_editor(id: Option<usize>) {
    FOCUSED_EDITOR.with(|c| c.set(id));
}
fn goal_x() -> Option<f32> {
    GOAL_X.with(|c| c.get())
}
fn set_goal_x(x: Option<f32>) {
    GOAL_X.with(|c| c.set(x));
}

/// A resolved pointer hit inside an editor: the container + textblock host ids and the
/// flat UTF-8 byte offset within the textblock's concatenated inline text.
struct EditorHit {
    container_nid: usize,
    textblock_nid: usize,
    byte: usize,
}

/// `document.caretRangeFromPoint(x, y)` — non-standard but available in Chromium/WebKit
/// (the same call the generic `resolve_text_hit` uses). `None` outside any text.
fn caret_range_from_point(doc: &web_sys::Document, x: f32, y: f32) -> Option<web_sys::Range> {
    let func = js_sys::Reflect::get(doc, &"caretRangeFromPoint".into()).ok()?;
    let func: js_sys::Function = func.dyn_into().ok()?;
    let val = func.call2(doc, &JsValue::from(x), &JsValue::from(y)).ok()?;
    if val.is_null() || val.is_undefined() {
        return None;
    }
    val.dyn_into::<web_sys::Range>().ok()
}

/// Resolve a viewport point to an editor position: walk up from the caret range's start
/// container to the innermost `[data-pm-type]` block (the textblock) and the enclosing
/// `[data-pm-editor]` container, then compute the within-block byte offset. Marks
/// (`[data-pm-mark]`) are skipped because only blocks carry `data-pm-type`.
fn resolve_editor_point(doc: &web_sys::Document, x: f32, y: f32) -> Option<EditorHit> {
    let range = caret_range_from_point(doc, x, y)?;
    let start = range.start_container().ok()?;
    let offset = range.start_offset().ok()?;

    let mut cur: Option<web_sys::Node> = Some(start.clone());
    let mut textblock: Option<web_sys::Element> = None;
    let mut container: Option<web_sys::Element> = None;
    while let Some(node) = cur {
        if let Ok(el) = node.clone().dyn_into::<web_sys::Element>() {
            if el.has_attribute("data-pm-editor") {
                container = Some(el);
                break;
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
    let byte = compute_byte_offset_in_block(&textblock, &start, offset);
    Some(EditorHit {
        container_nid,
        textblock_nid,
        byte,
    })
}

/// The focused editor's handle, if any (dropping focus if it has unmounted).
fn focused_handle() -> Option<(usize, EditorHandle)> {
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
    registry::update_all_carets(focused_editor());
}

// ── Hidden capture target (clipboard + IME) ───────────────────────────────────

/// Get-or-create the shared hidden capture `<textarea>`, appending it to `<body>`
/// and attaching its clipboard + composition listeners on first use. `None` only
/// if the DOM rejects the element (e.g. no `<body>` yet).
fn ensure_capture_target(doc: &web_sys::Document) -> Option<web_sys::HtmlTextAreaElement> {
    if let Some(ta) = capture_target() {
        return Some(ta);
    }
    let ta: web_sys::HtmlTextAreaElement = doc.create_element("textarea").ok()?.dyn_into().ok()?;
    ta.set_attribute("data-pm-capture", "true").ok();
    ta.set_attribute("aria-hidden", "true").ok();
    ta.set_attribute("tabindex", "-1").ok();
    ta.set_autocomplete("off");
    ta.set_spellcheck(false);
    ta.set_attribute("autocorrect", "off").ok();
    ta.set_attribute("autocapitalize", "off").ok();
    // Off-screen, invisible, and non-interactive (`pointer-events: none` so it never
    // becomes a mouse target / steals a click) yet still programmatically focusable.
    ta.set_attribute(
        "style",
        "position: fixed; top: 0; left: 0; width: 1px; height: 1px; padding: 0; \
         margin: -1px; border: 0; opacity: 0; overflow: hidden; resize: none; \
         pointer-events: none; outline: none; z-index: -1; white-space: pre;",
    )
    .ok();
    doc.body()?.append_child(&ta).ok()?;
    install_capture_listeners(&ta);
    CAPTURE.with(|c| *c.borrow_mut() = Some(ta.clone()));
    Some(ta)
}

/// The capture textarea, if it has been created.
fn capture_target() -> Option<web_sys::HtmlTextAreaElement> {
    CAPTURE.with(|c| c.borrow().clone())
}

/// Focus the capture target so the browser routes clipboard / IME events to it.
/// `preventScroll` keeps focusing the off-screen target from jumping the page.
fn focus_capture_target(doc: &web_sys::Document) {
    if let Some(ta) = ensure_capture_target(doc) {
        let opts = web_sys::FocusOptions::new();
        opts.set_prevent_scroll(true);
        let _ = ta.focus_with_options(&opts);
        ta.set_value("");
    }
}

/// Blur the capture target (when focus leaves the editor for another field).
fn blur_capture_target() {
    if let Some(ta) = capture_target() {
        let _ = ta.blur();
    }
}

/// Attach the clipboard + IME composition listeners to the capture target, once
/// when it is created. These events only fire on the focused editable, so scoping
/// them to our textarea avoids hijacking copy/paste for any other page input.
fn install_capture_listeners(ta: &web_sys::HtmlTextAreaElement) {
    let target: &web_sys::EventTarget = ta.as_ref();
    add_target_listener(target, "copy", |e: web_sys::ClipboardEvent| on_copy(&e));
    add_target_listener(target, "cut", |e: web_sys::ClipboardEvent| on_cut(&e));
    add_target_listener(target, "paste", |e: web_sys::ClipboardEvent| on_paste(&e));
    add_target_listener(
        target,
        "compositionstart",
        |_e: web_sys::CompositionEvent| {
            COMPOSING.with(|c| c.set(true));
        },
    );
    add_target_listener(
        target,
        "compositionupdate",
        |e: web_sys::CompositionEvent| {
            on_composition_update(&e);
        },
    );
    add_target_listener(target, "compositionend", |e: web_sys::CompositionEvent| {
        on_composition_end(&e);
    });
}

// ── Clipboard (copy / cut / paste) ─────────────────────────────────────────────
//
// Mirrors the desktop `editor_copy`/`editor_cut`/`editor_paste`, but reads/writes
// the browser's native `ClipboardEvent.clipboardData` (synchronous, the only path
// to `text/html`) instead of the OS clipboard crate. The model serialization
// (`selection_clipboard` / `replace_selection_with_*`) is the shared handle API.

/// Copy: write the selection's `(text/html, text/plain)` to the clipboard. Always
/// `preventDefault` so the empty hidden textarea is never copied (which would clobber
/// the clipboard); only write data when there is a non-empty selection.
fn on_copy(event: &web_sys::ClipboardEvent) {
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    event.prevent_default();
    if let Some((html, text)) = handle.selection_clipboard()
        && let Some(dt) = event.clipboard_data()
    {
        let _ = dt.set_data("text/html", &html);
        let _ = dt.set_data("text/plain", &text);
    }
}

/// Cut: copy the selection, then delete it.
fn on_cut(event: &web_sys::ClipboardEvent) {
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    event.prevent_default();
    if let Some((html, text)) = handle.selection_clipboard()
        && let Some(dt) = event.clipboard_data()
    {
        let _ = dt.set_data("text/html", &html);
        let _ = dt.set_data("text/plain", &text);
        if handle.command("deleteSelection") {
            refresh_caret();
        }
    }
}

/// Paste over the selection, preferring rich `text/html`, then a raw image file
/// (encoded as a `data:` URL via `FileReader`), then `text/plain`.
fn on_paste(event: &web_sys::ClipboardEvent) {
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    event.prevent_default();
    let Some(dt) = event.clipboard_data() else {
        return;
    };
    // 1. Rich HTML — preserves structure, links, marks, and URL-referenced images.
    if let Ok(html) = dt.get_data("text/html")
        && !html.trim().is_empty()
        && handle.replace_selection_with_html(&html)
    {
        refresh_caret();
        return;
    }
    // 2. A bitmap with no HTML wrapper (a screenshot / "copy image"): read the first
    //    image file as a data URL and insert it asynchronously.
    if paste_image_from(&dt, &handle) {
        return;
    }
    // 3. Plain text.
    if let Ok(text) = dt.get_data("text/plain")
        && !text.is_empty()
        && handle.replace_selection_with_text(&text)
    {
        refresh_caret();
    }
}

/// Find the first image file on the clipboard and insert it (async). Returns
/// whether an image read was started.
fn paste_image_from(dt: &web_sys::DataTransfer, handle: &EditorHandle) -> bool {
    let items = dt.items();
    for i in 0..items.length() {
        if let Some(item) = items.get(i)
            && item.kind() == "file"
            && item.type_().starts_with("image/")
            && let Ok(Some(file)) = item.get_as_file()
        {
            read_image_file(&file, handle.clone());
            return true;
        }
    }
    false
}

/// A self-dropping slot holding a `FileReader` `onload` closure alive until it fires.
type OnloadSlot = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// Read `file` as a `data:` URL and insert it as an image. The `FileReader`
/// `onload` closure keeps itself (and the reader) alive until it fires, then drops
/// itself — no unbounded leak per paste.
fn read_image_file(file: &web_sys::File, handle: EditorHandle) {
    let Ok(reader) = web_sys::FileReader::new() else {
        return;
    };
    let reader = Rc::new(reader);
    let slot: OnloadSlot = Rc::new(RefCell::new(None));
    let onload = {
        let reader = reader.clone();
        let slot = slot.clone();
        Closure::wrap(Box::new(move || {
            if let Ok(result) = reader.result()
                && let Some(url) = result.as_string()
                && !url.is_empty()
                && handle.insert_image(&url, "")
            {
                refresh_caret();
            }
            slot.borrow_mut().take();
        }) as Box<dyn FnMut()>)
    };
    reader.set_onload(Some(onload.as_ref().unchecked_ref()));
    *slot.borrow_mut() = Some(onload);
    let _ = reader.read_as_data_url(file);
}

// ── IME composition ────────────────────────────────────────────────────────────
//
// The preedit is a view-local overlay (design A5), never part of the document:
// `compositionupdate` shows it at the caret, `compositionend` commits the final
// text as one ordinary edit (so undo/history treat it like typing).

fn on_composition_update(event: &web_sys::CompositionEvent) {
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    handle.ime_set_preedit(&event.data().unwrap_or_default(), None);
    refresh_caret();
}

fn on_composition_end(event: &web_sys::CompositionEvent) {
    COMPOSING.with(|c| c.set(false));
    if let Some((_, handle)) = focused_handle() {
        handle.ime_commit(&event.data().unwrap_or_default());
        refresh_caret();
    }
    // The textarea accumulated the composed text; clear it so it stays empty.
    if let Some(ta) = capture_target() {
        ta.set_value("");
    }
}

// ── Pointer ──────────────────────────────────────────────────────────────────

/// Handle a `mousedown`. Returns whether it landed in an editor (so the listener
/// consumes it). Mirrors `try_new_editor_click`.
fn handle_mousedown(event: &web_sys::MouseEvent, doc: &web_sys::Document) -> bool {
    let Some(target) = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    let Some(editor_el) = target.closest("[data-pm-editor]").ok().flatten() else {
        // Outside any editor. Blur only when moving to another text field, so a
        // toolbar click keeps the editor focused (and its selection visible).
        if target
            .closest("input, textarea, [contenteditable], [data-pm-editor]")
            .ok()
            .flatten()
            .is_some()
        {
            set_focused_editor(None);
            blur_capture_target();
        }
        return false;
    };
    let Some(container_nid) = get_nid(&editor_el.into()).map(|n| n.0) else {
        return false;
    };
    let Some(handle) = registry::editor_for(container_nid) else {
        return false;
    };
    set_focused_editor(Some(container_nid));
    set_goal_x(None);
    // Focus the hidden capture target so the browser routes clipboard / IME events
    // to this editor (a non-`contenteditable` `<div>` would receive none).
    focus_capture_target(doc);

    // A click on a leaf atom (image / horizontal rule) node-selects it.
    if let Some(leaf) = target
        .closest("[data-pm-type='image'], [data-pm-type='horizontal_rule']")
        .ok()
        .flatten()
        && let Some(leaf_nid) = get_nid(&leaf.into()).map(|n| n.0)
        && let Some(sel) = handle.node_selection_at_host(leaf_nid)
    {
        handle.set_selection(sel);
        registry::end_drag();
        refresh_caret();
        return true;
    }

    let x = event.client_x() as f32;
    let y = event.client_y() as f32;
    if let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(clicked) = handle.pos_at(hit.textblock_nid, hit.byte)
    {
        match event.detail() {
            2 => {
                handle.select_word_at(clicked);
                registry::end_drag();
            }
            n if n >= 3 => {
                handle.select_block_at(clicked);
                registry::end_drag();
            }
            _ if event.shift_key() => {
                let anchor = handle.selection().anchor();
                handle.set_selection(Selection::text(anchor, clicked));
                registry::begin_drag(container_nid, anchor.0);
            }
            _ => {
                handle.set_selection(Selection::cursor(clicked));
                registry::begin_drag(container_nid, clicked.0);
            }
        }
    }
    refresh_caret();
    true
}

/// Handle a `mousemove` while a drag-select is active. Returns whether a drag was live.
fn handle_mousemove(event: &web_sys::MouseEvent, doc: &web_sys::Document) -> bool {
    let Some((container_nid, anchor)) = registry::drag_anchor() else {
        return false;
    };
    // If the primary button is no longer held (a mouseup was missed — e.g. released
    // outside the window), the drag is stale: end it instead of following the cursor.
    if event.buttons() & 1 == 0 {
        registry::end_drag();
        return false;
    }
    let Some(handle) = registry::editor_for(container_nid) else {
        return false;
    };
    let x = event.client_x() as f32;
    let y = event.client_y() as f32;
    if let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(head) = handle.pos_at(hit.textblock_nid, hit.byte)
    {
        handle.set_selection(Selection::text(Pos(anchor), head));
        refresh_caret();
    }
    true
}

// ── Keyboard ─────────────────────────────────────────────────────────────────

/// Handle a `keydown`. Returns whether the editor consumed the key (so the listener
/// `preventDefault`s + stops it). Mirrors `dispatch_new_editor_key`.
fn handle_keydown(event: &web_sys::KeyboardEvent, doc: &web_sys::Document) -> bool {
    // Never hijack a key destined for a real form control / editable element (e.g. a
    // search box the user clicked) — but our own hidden capture textarea
    // (`data-pm-capture`) IS the editor's focus target, so let its keys through.
    if let Some(t) = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        && !t.has_attribute("data-pm-capture")
        && t.closest("input, textarea, select, [contenteditable]")
            .ok()
            .flatten()
            .is_some()
    {
        return false;
    }
    // During an IME composition, yield every key to the textarea + IME — the composed
    // text arrives via the composition events, so inserting here would double up.
    if event.is_composing() || COMPOSING.with(|c| c.get()) {
        return false;
    }
    let Some((container_nid, handle)) = focused_handle() else {
        return false;
    };
    let key = event.key();
    let ctrl = event.ctrl_key() || event.meta_key();
    let shift = event.shift_key();
    let alt = event.alt_key();

    if key != "ArrowUp" && key != "ArrowDown" {
        set_goal_x(None);
    }

    let handled = match key.as_str() {
        "Backspace" => handle.command("deleteCharBackward"),
        "Delete" => handle.command("deleteCharForward"),
        "Enter" if !ctrl => handle.command("enter"),
        "Tab" => {
            handle.tab_cell(shift);
            true // consume Tab either way — no focus traversal mid-edit
        }
        "ArrowLeft" => handle.move_cursor(
            if ctrl {
                CursorMotion::WordLeft
            } else {
                CursorMotion::CharLeft
            },
            shift,
        ),
        "ArrowRight" => handle.move_cursor(
            if ctrl {
                CursorMotion::WordRight
            } else {
                CursorMotion::CharRight
            },
            shift,
        ),
        "ArrowUp" => vertical_step(&handle, container_nid, doc, false, shift),
        "ArrowDown" => vertical_step(&handle, container_nid, doc, true, shift),
        "Home" => handle.move_cursor(
            if ctrl {
                CursorMotion::DocStart
            } else {
                CursorMotion::LineStart
            },
            shift,
        ),
        "End" => handle.move_cursor(
            if ctrl {
                CursorMotion::DocEnd
            } else {
                CursorMotion::LineEnd
            },
            shift,
        ),
        "a" | "A" if ctrl => {
            handle.select_all();
            true
        }
        "b" | "B" if ctrl => handle.command("toggleBold"),
        "i" | "I" if ctrl => handle.command("toggleItalic"),
        "u" | "U" if ctrl => handle.command("toggleUnderline"),
        "z" | "Z" if ctrl && shift => handle.command("redo"),
        "z" | "Z" if ctrl => handle.command("undo"),
        "y" | "Y" if ctrl => handle.command("redo"),
        _ => {
            // Printable text input: a single non-control character with no Ctrl/Meta/Alt.
            // (Clipboard and other Ctrl shortcuts fall through unconsumed.)
            if ctrl || alt {
                false
            } else if key.chars().count() == 1 && !key.chars().next().unwrap().is_control() {
                handle.insert_text(&key)
            } else {
                false
            }
        }
    };
    if handled {
        refresh_caret();
    }
    handled
}

/// Vertical caret movement: hit-test at the goal column one line above/below the head's
/// current screen position. Mirrors the desktop `vertical_step` with browser geometry.
fn vertical_step(
    handle: &EditorHandle,
    container_nid: usize,
    doc: &web_sys::Document,
    down: bool,
    extend: bool,
) -> bool {
    let head = handle.selection().head();
    let Some((hx, hy, hh)) = head_screen_rect(handle, doc, head) else {
        return false;
    };
    let gx = goal_x().unwrap_or(hx);
    set_goal_x(Some(gx));
    let ty = if down { hy + hh * 1.5 } else { hy - hh * 0.5 };
    let Some(hit) = resolve_editor_point(doc, gx, ty) else {
        return false;
    };
    if hit.container_nid != container_nid {
        return false;
    }
    let Some(new_head) = handle.pos_at(hit.textblock_nid, hit.byte) else {
        return false;
    };
    let sel = if extend {
        Selection::text(handle.selection().anchor(), new_head)
    } else {
        Selection::cursor(new_head)
    };
    handle.set_selection(sel);
    true
}

/// The viewport `(x, y, height)` of the caret at model `pos`. Prefers a collapsed
/// `Range` at the host text node; for an *empty* block (no text node) falls back to the
/// block element's own box so vertical motion off a blank line still works.
fn head_screen_rect(
    handle: &EditorHandle,
    doc: &web_sys::Document,
    pos: Pos,
) -> Option<(f32, f32, f32)> {
    let (tb_nid, byte) = handle.caret_address(pos)?;
    let block = node_by_nid(tb_nid)?;
    if let Some((text_node, off)) = find_text_node_at_byte_offset(&block, byte)
        && let Ok(range) = doc.create_range()
        && range.set_start(&text_node, off).is_ok()
        && range.set_end(&text_node, off).is_ok()
    {
        let r = range.get_bounding_client_rect();
        if r.height() > 0.0 {
            return Some((r.x() as f32, r.y() as f32, r.height() as f32));
        }
    }
    let el = block.dyn_into::<web_sys::Element>().ok()?;
    let r = el.get_bounding_client_rect();
    let h = if r.height() > 0.0 {
        r.height() as f32
    } else {
        18.0
    };
    Some((r.x() as f32, r.y() as f32, h))
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

// ── Installation ─────────────────────────────────────────────────────────────

/// Add a capture-phase `document` listener leaked for the page lifetime.
fn add_capture<E: JsCast + 'static>(
    doc: &web_sys::Document,
    name: &str,
    handler: impl Fn(E) + 'static,
) {
    let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
        if let Ok(ev) = e.dyn_into::<E>() {
            handler(ev);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    doc.add_event_listener_with_callback_and_bool(name, closure.as_ref().unchecked_ref(), true)
        .ok();
    closure.forget();
}

/// Add a (bubble-phase) listener to a specific `target`, leaked for its lifetime.
/// Used for the capture textarea's clipboard / composition events, which target it
/// directly (no need for document-level capture).
fn add_target_listener<E: JsCast + 'static>(
    target: &web_sys::EventTarget,
    name: &str,
    handler: impl Fn(E) + 'static,
) {
    let closure = Closure::wrap(Box::new(move |e: web_sys::Event| {
        if let Ok(ev) = e.dyn_into::<E>() {
            handler(ev);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    target
        .add_event_listener_with_callback(name, closure.as_ref().unchecked_ref())
        .ok();
    closure.forget();
}

/// A small web-only override of the shared editor default stylesheet. The shared
/// stylesheet (`rinch_editor_view`'s `styles.rs`) sets `li { display: flex }` so the
/// *desktop* renderer (rinch-dom, which emits list markers as block siblings) can align
/// them inline with their content. On the web the browser draws the native `::marker`,
/// which `display: flex` suppresses — so bullets and numbers vanish. Restoring
/// `display: list-item` brings them back. Scoped one level deeper (`ul/ol > li`) than the
/// base `li` rule so it wins by specificity regardless of `<style>` source order.
const EDITOR_WEB_CSS: &str =
    "[data-pm-editor] ul > li, [data-pm-editor] ol > li { display: list-item; }";

/// Inject [`EDITOR_WEB_CSS`] into the document head once (idempotent), so the browser
/// renders native list markers despite the shared stylesheet's desktop `li` flex rule.
fn ensure_editor_web_styles(doc: &web_sys::Document) {
    if doc
        .query_selector("style[data-rinch-editor-web]")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let Ok(style) = doc.create_element("style") else {
        return;
    };
    let _ = style.set_attribute("data-rinch-editor-web", "true");
    style.set_text_content(Some(EDITOR_WEB_CSS));
    if let Some(head) = doc.head() {
        let _ = head.append_child(&style);
    }
}

/// Install the editor input listeners once. Called from `mount_tree` alongside
/// `ensure_event_delegation`. Idempotent.
pub(crate) fn install(browser_doc: &web_sys::Document) {
    if INSTALLED.with(|c| c.replace(true)) {
        return;
    }

    // Patch the one editor default-stylesheet rule that is desktop-specific (see below).
    ensure_editor_web_styles(browser_doc);

    let doc = browser_doc.clone();
    add_capture(browser_doc, "keydown", move |e: web_sys::KeyboardEvent| {
        if handle_keydown(&e, &doc) {
            e.prevent_default();
            e.stop_propagation();
        }
    });
    let doc = browser_doc.clone();
    add_capture(browser_doc, "mousedown", move |e: web_sys::MouseEvent| {
        if handle_mousedown(&e, &doc) {
            e.prevent_default();
            e.stop_propagation();
        }
    });
    let doc = browser_doc.clone();
    add_capture(browser_doc, "mousemove", move |e: web_sys::MouseEvent| {
        if handle_mousemove(&e, &doc) {
            e.prevent_default();
        }
    });
    add_capture(browser_doc, "mouseup", move |_e: web_sys::MouseEvent| {
        registry::end_drag();
    });

    // Bubble-phase refresh: after an *outside* click that wasn't consumed in capture
    // (e.g. a toolbar button dispatched `handle.command`), re-render the focused
    // editor's caret/selection. Registered after `ensure_event_delegation`, so it runs
    // once the button's handler has applied its edit.
    let refresh = Closure::wrap(Box::new(move |_e: web_sys::Event| {
        if focused_editor().is_some() {
            refresh_caret();
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    browser_doc
        .add_event_listener_with_callback("mousedown", refresh.as_ref().unchecked_ref())
        .ok();
    refresh.forget();

    // Caret blink (530 ms half-period — the platform default).
    if let Some(win) = web_sys::window() {
        let tick = Closure::wrap(Box::new(blink_tick) as Box<dyn FnMut()>);
        win.set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            530,
        )
        .ok();
        tick.forget();
    }
}
