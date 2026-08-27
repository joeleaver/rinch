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
//! browser-native so keys can't route to the wrong control. It is never shown, and it
//! holds exactly one textblock — the caret's — mirrored there so a soft keyboard has
//! real text to replace (see [`sync_mirror`]); the document itself never lives in it.
//! Typed characters from a *physical* keyboard are consumed (and `preventDefault`ed) by
//! the keydown handler before the textarea sees them. This mirrors the CodeMirror /
//! ProseMirror hidden-input technique.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rinch_editor_core::{CursorMotion, Pos, Selection};
use rinch_editor_view::{EditorHandle, registry};

use crate::event_delegation::{
    compute_byte_offset_in_block, drag_machine, utf16_offset_to_utf8_bytes,
};
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
    /// What we last wrote into the capture textarea — the caret's textblock, mirrored
    /// so a soft keyboard has real text to replace. `None` when no editor is focused.
    /// [`reconcile_mirror`] diffs the textarea against this to recover the edit.
    static MIRROR: RefCell<Option<Mirror>> = const { RefCell::new(None) };
    /// The in-flight touch/pen contact that started inside an editor, as
    /// `(pointer_id, client_x, client_y)`. A release near that point is a tap rather
    /// than a scroll, and focuses the capture target (see `handle_touch_tap`).
    static TOUCH_TAP: Cell<Option<(i32, f32, f32)>> = const { Cell::new(None) };
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
    registry::update_all_carets(None, focused_editor());
    // Keep the capture textarea mirroring the caret's block. Every edit and every
    // caret move comes through here, which is exactly when a keyboard's idea of the
    // surrounding text goes stale.
    if let Some((_, handle)) = focused_handle() {
        sync_mirror(&handle);
    }
}

// ── The capture mirror ────────────────────────────────────────────────────────
//
// The capture textarea used to be held empty, which cost us every edit a keyboard
// expresses as a *replacement* of text it believes is already there. Tapping an
// autocorrect suggestion, for instance, sets a composing region over the word behind
// the caret and then commits the correction; with an empty field that region covers
// nothing, so "word" + a tap on "world" committed as a bare insert and produced
// "wordworld".
//
// So the textarea mirrors the caret's textblock and tracks the model selection. The
// keyboard's ranges then land on real characters, and the edit is recovered by diffing
// the textarea against what we wrote — which handles insert, replace, and delete
// uniformly, whatever inputType (or composition) the keyboard chose to express it with.
// This is the same mirror-and-diff CodeMirror uses for exactly these keyboards.

/// The capture textarea's contents as we last wrote them: one textblock's text and the
/// DOM id of the block it came from.
#[derive(Clone)]
struct Mirror {
    textblock_nid: usize,
    text: String,
    /// The selection we wrote, in UTF-16 code units — what the textarea counts in.
    /// Kept so a redundant re-sync can be skipped.
    sel: (u32, u32),
}

/// The byte offset of char index `i` (its length in bytes for `i` past the end).
fn byte_of_char(text: &str, i: usize) -> usize {
    text.char_indices().nth(i).map_or(text.len(), |(b, _)| b)
}

/// The number of UTF-16 code units — the unit `selectionStart`/`selectionEnd` count in
/// — in `text` up to UTF-8 byte offset `byte`.
///
/// Deliberately walks rather than slicing `&text[..byte]`: `byte` comes from the *model*
/// (`EditorHandle::caret_address`) while `text` comes from the *DOM*, and the two are
/// only equal as long as nothing renders extra text inside a textblock. A slice at a
/// non-boundary offset panics, which in wasm takes the whole page down; counting whole
/// characters instead degrades to a caret at the nearest boundary.
fn utf16_len_upto(text: &str, byte: usize) -> u32 {
    text.char_indices()
        .take_while(|(b, _)| *b < byte)
        .map(|(_, c)| c.len_utf16() as u32)
        .sum()
}

/// The minimal replacement turning `base` into `now`: `(from, to, inserted)` where
/// `from..to` is a **char** range in `base`. `None` when they are equal.
///
/// Trimming the common prefix and suffix keeps the edit as small as the change really
/// was: a suggestion that rewrites "word" to "world" comes back as "insert `l` at 3",
/// not "replace the whole word", so marks either side survive and undo stays granular.
fn text_diff(base: &str, now: &str) -> Option<(usize, usize, String)> {
    if base == now {
        return None;
    }
    let b: Vec<char> = base.chars().collect();
    let n: Vec<char> = now.chars().collect();
    let mut prefix = 0;
    while prefix < b.len() && prefix < n.len() && b[prefix] == n[prefix] {
        prefix += 1;
    }
    // The suffix may not reach back into the prefix from either side.
    let mut suffix = 0;
    while suffix < b.len() - prefix
        && suffix < n.len() - prefix
        && b[b.len() - 1 - suffix] == n[n.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let inserted: String = n[prefix..n.len() - suffix].iter().collect();
    Some((prefix, b.len() - suffix, inserted))
}

/// Empty the capture textarea and drop the mirror **together**.
///
/// The two must never disagree. A field emptied behind a live mirror is read by the next
/// [`reconcile_mirror`] as "the user deleted the whole block", and that reconcile is
/// applied to the document — so every path that clears the field clears the mirror with
/// it, and vice versa.
fn clear_mirror(ta: &web_sys::HtmlTextAreaElement) {
    MIRROR.with(|m| *m.borrow_mut() = None);
    if !ta.value().is_empty() {
        ta.set_value("");
    }
}

/// Rewrite the capture textarea to mirror the caret's textblock, and remember what we
/// wrote. A no-op mid-composition — the IME owns the field until it commits.
fn sync_mirror(handle: &EditorHandle) {
    if COMPOSING.with(|c| c.get()) {
        return;
    }
    let Some(ta) = capture_target() else {
        return;
    };
    let selection = handle.selection();
    let Some((textblock_nid, head_byte)) = handle.caret_address(selection.head()) else {
        // No text caret (a node selection, or an unmounted editor): nothing to mirror.
        clear_mirror(&ta);
        return;
    };
    let Some(block) = node_by_nid(textblock_nid) else {
        // The caret's block isn't in the host node map (mid-unmount). Leaving the old
        // mirror standing would let the next `input` diff live text against a field
        // that no longer holds it, so drop both rather than half of the pair.
        clear_mirror(&ta);
        return;
    };
    let text = block.text_content().unwrap_or_default();
    let head = utf16_len_upto(&text, head_byte);
    // Mirror the selection too, but only when it lies in this same block — a
    // cross-block selection has no honest representation in one block's text.
    let anchor = match handle.caret_address(selection.anchor()) {
        Some((nid, byte)) if nid == textblock_nid => utf16_len_upto(&text, byte),
        _ => head,
    };
    let sel = (anchor.min(head), anchor.max(head));

    let unchanged = MIRROR.with(|m| {
        m.borrow()
            .as_ref()
            .is_some_and(|p| p.textblock_nid == textblock_nid && p.text == text && p.sel == sel)
    });
    if unchanged && ta.value() == text {
        return;
    }
    ta.set_value(&text);
    let _ = ta.set_selection_range(sel.0, sel.1);
    MIRROR.with(|m| {
        *m.borrow_mut() = Some(Mirror {
            textblock_nid,
            text,
            sel,
        })
    });
}

/// Apply whatever the browser did to the capture textarea to the model, by diffing it
/// against the mirror. Returns whether an edit was applied.
///
/// This is the recovery path for every edit we let the browser express in the textarea
/// rather than intercepting: composition commits, autocorrect replacements, and
/// anything else a keyboard writes without an `inputType` we recognise.
fn reconcile_mirror(handle: &EditorHandle) -> bool {
    let Some(ta) = capture_target() else {
        return false;
    };
    let Some(mirror) = MIRROR.with(|m| m.borrow().clone()) else {
        return false;
    };
    // The mirror is only refreshed from `refresh_caret`, so a document change that
    // arrives by another route — `load_html`, a collab delta, a toolbar command with no
    // pointer event behind it — leaves it describing text the block no longer holds.
    // Splicing a diff at those offsets would corrupt the document, so fail closed and
    // rebuild the mirror from the live block instead.
    if node_by_nid(mirror.textblock_nid)
        .and_then(|n| n.text_content())
        .as_deref()
        != Some(mirror.text.as_str())
    {
        sync_mirror(handle);
        return false;
    }
    let Some((from_char, to_char, inserted)) = text_diff(&mirror.text, &ta.value()) else {
        return false;
    };
    let from_byte = byte_of_char(&mirror.text, from_char);
    let to_byte = byte_of_char(&mirror.text, to_char);
    let (Some(from), Some(to)) = (
        handle.pos_at(mirror.textblock_nid, from_byte),
        handle.pos_at(mirror.textblock_nid, to_byte),
    ) else {
        return false;
    };
    // Remember where the user actually was: moving the selection is how the edit is
    // targeted, but an edit that then declines to apply must not leave the caret parked
    // on the diff range it never touched.
    let before = handle.selection();
    handle.set_selection(Selection::text(from, to));
    let applied = if inserted.is_empty() {
        handle.command("deleteSelection")
    } else {
        // `insert_text` replaces a non-empty selection, so this covers both a pure
        // insertion and a replacement in one call.
        handle.insert_text(&inserted)
    };
    if applied {
        adopt_field_caret(handle, &ta, mirror.textblock_nid);
    } else {
        handle.set_selection(before);
    }
    applied
}

/// Move the model caret to wherever the browser left the textarea's caret.
///
/// The minimal diff ends where the *characters* stopped differing, which is not where
/// the keyboard means the caret to be: correcting "here" to "there" is one inserted
/// `t`, so the edit ends at offset 13 while the user expects the caret after "there".
/// The field already holds the right answer — the browser placed it — so take it from
/// there rather than inferring it.
fn adopt_field_caret(
    handle: &EditorHandle,
    ta: &web_sys::HtmlTextAreaElement,
    textblock_nid: usize,
) {
    let text = ta.value();
    let start = ta.selection_start().ok().flatten().unwrap_or(0);
    let end = ta.selection_end().ok().flatten().unwrap_or(start);
    let (Some(from), Some(to)) = (
        handle.pos_at(textblock_nid, utf16_offset_to_utf8_bytes(&text, start)),
        handle.pos_at(textblock_nid, utf16_offset_to_utf8_bytes(&text, end)),
    ) else {
        return;
    };
    handle.set_selection(Selection::text(from, to));
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
        // Start from an empty field; the `refresh_caret` that ends every focusing
        // `handle_mousedown` fills it from the caret's block. Clear the mirror with it —
        // an emptied field behind a live mirror reads as "the block was deleted".
        clear_mirror(&ta);
    }
}

/// Blur the capture target (when focus leaves the editor for another field).
fn blur_capture_target() {
    if let Some(ta) = capture_target() {
        let _ = ta.blur();
    }
}

/// Attach the clipboard, IME composition, and `beforeinput` listeners to the capture
/// target, once when it is created. These events only fire on the focused editable, so
/// scoping them to our textarea avoids hijacking copy/paste — or a soft keyboard's
/// edits — for any other page input.
fn install_capture_listeners(ta: &web_sys::HtmlTextAreaElement) {
    let target: &web_sys::EventTarget = ta.as_ref();
    add_target_listener(target, "copy", |e: web_sys::ClipboardEvent| on_copy(&e));
    add_target_listener(target, "cut", |e: web_sys::ClipboardEvent| on_cut(&e));
    add_target_listener(target, "paste", |e: web_sys::ClipboardEvent| on_paste(&e));
    add_target_listener(target, "beforeinput", |e: web_sys::InputEvent| {
        on_before_input(&e);
    });
    add_target_listener(target, "input", |_e: web_sys::InputEvent| on_input());
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
    let Some((_, handle)) = focused_handle() else {
        COMPOSING.with(|c| c.set(false));
        return;
    };
    // Drop the overlay first: from here the composed text belongs to the document.
    handle.ime_clear_preedit();
    // The textarea now holds the block with the composition applied — including any
    // text the IME replaced, which is the whole reason the mirror exists. Diff it
    // rather than inserting `event.data()` at the caret: a commit that rewrote the
    // word behind the caret would otherwise be appended to it ("word" + "world"
    // committing as "wordworld").
    let applied = reconcile_mirror(&handle);
    if !applied {
        // No mirror to diff against (composition began before the block was mirrored).
        // Fall back to the plain commit — an append, but better than dropping the text.
        handle.ime_commit(&event.data().unwrap_or_default());
    }
    COMPOSING.with(|c| c.set(false));
    refresh_caret();
}

// ── `beforeinput` (software keyboards, dictation, the edit menu) ──────────────
//
// A physical key never gets here: `handle_keydown` consumes it and `preventDefault`s,
// and a cancelled `keydown` fires no `beforeinput`. What is left is everything a
// `keydown` cannot express — Android's `Unidentified` / 229 keys, swipe typing,
// dictation, the platform edit menu, and the undo/redo gestures — all of which the
// browser describes only by `inputType`.

/// What a `beforeinput` `inputType` asks the model to do.
///
/// Split out from [`on_before_input`] so the mapping is testable without a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditIntent {
    /// Insert the event's `data` (replacing any selection).
    InsertText,
    /// Run a named editor command.
    Command(&'static str),
    /// Extend the selection over `motion`, then delete it — the word/line deletes a
    /// soft keyboard asks for, which the base keymap has no single command for.
    DeleteTo(CursorMotion),
    /// Let the browser make the edit in the mirrored textarea and recover it by diff
    /// on the following `input` — the only honest way to apply an edit whose extent
    /// lives in the keyboard's replacement range rather than in the event itself.
    Reconcile,
    /// Not ours: already applied by a dedicated handler, owned by the composition
    /// events, or not expressible on the model.
    Ignore,
}

/// Map a `beforeinput` `inputType` onto an [`EditIntent`].
///
/// Names come from the Input Events spec; the set here is what browsers actually emit
/// for an editable host. Anything unlisted is [`EditIntent::Ignore`] — silently doing
/// nothing beats guessing at an edit the user did not ask for.
fn edit_intent(input_type: &str) -> EditIntent {
    match input_type {
        // Plain typing, swipe typing, dictation, an emoji picker.
        "insertText" => EditIntent::InsertText,
        // The composition events own the preedit overlay and the commit; applying this
        // as well would type every composed word twice.
        "insertCompositionText" => EditIntent::Ignore,
        // Autocorrect / a suggestion-bar tap: the event says what to insert but not what
        // it replaces (`getTargetRanges()` is empty on a textarea, by spec). Let the
        // browser apply it to the mirror and read the replacement back off the diff.
        "insertReplacementText" => EditIntent::Reconcile,
        "insertParagraph" => EditIntent::Command("enter"),
        "insertLineBreak" => EditIntent::Command("insertHardBreak"),
        // `deleteContent*` is a single char (or the selection, which both commands
        // delete first); `deleteContent` itself only ever means "the selection".
        "deleteContent" => EditIntent::Command("deleteSelection"),
        "deleteContentBackward" => EditIntent::Command("deleteCharBackward"),
        "deleteContentForward" => EditIntent::Command("deleteCharForward"),
        "deleteWordBackward" => EditIntent::DeleteTo(CursorMotion::WordLeft),
        "deleteWordForward" => EditIntent::DeleteTo(CursorMotion::WordRight),
        "deleteSoftLineBackward" | "deleteHardLineBackward" => {
            EditIntent::DeleteTo(CursorMotion::LineStart)
        }
        "deleteSoftLineForward" | "deleteHardLineForward" => {
            EditIntent::DeleteTo(CursorMotion::LineEnd)
        }
        "historyUndo" => EditIntent::Command("undo"),
        "historyRedo" => EditIntent::Command("redo"),
        // The `cut` / `paste` listeners already ran and applied their own edit.
        "deleteByCut" | "insertFromPaste" | "insertFromPasteAsQuotation" => EditIntent::Ignore,
        _ => EditIntent::Ignore,
    }
}

/// Delete from the caret to `motion`'s target. An already non-empty selection is
/// deleted as it stands, matching what the browser would have done.
fn delete_to(handle: &EditorHandle, motion: CursorMotion) -> bool {
    if handle.selection().is_empty() {
        handle.move_cursor(motion, true);
    }
    if handle.selection().is_empty() {
        // The motion had nowhere to go: word and model-line motions are *within* a
        // textblock, so at a block edge they resolve to the caret's own position and
        // leave the selection collapsed. A word/line delete there means what Backspace
        // and Delete mean — join with the adjacent block. Without this the gesture is
        // swallowed silently, because the `beforeinput` was already `preventDefault`ed.
        return handle.command(match motion {
            CursorMotion::WordLeft | CursorMotion::LineStart => "deleteCharBackward",
            _ => "deleteCharForward",
        });
    }
    handle.command("deleteSelection")
}

/// Route a `beforeinput` on the capture textarea into the focused editor's model.
///
/// The textarea is never the document — whatever the browser was about to do to it is
/// wrong — so every event we recognise is `preventDefault`ed and mirrored onto the
/// model instead. An in-flight composition is left entirely alone: cancelling
/// `insertCompositionText` breaks the IME, and the composition events already commit it.
fn on_before_input(event: &web_sys::InputEvent) {
    if event.is_composing() || COMPOSING.with(|c| c.get()) {
        return;
    }
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    let handled = match edit_intent(&event.input_type()) {
        // Let it land in the textarea; `on_input` diffs the mirror and applies it.
        EditIntent::Reconcile => return,
        // Cancel it anyway: an edit we are not applying must not desync the mirror
        // from the document either.
        EditIntent::Ignore => {
            event.prevent_default();
            return;
        }
        EditIntent::InsertText => {
            event.prevent_default();
            match event.data() {
                Some(text) if !text.is_empty() => handle.insert_text(&text),
                _ => false,
            }
        }
        EditIntent::Command(name) => {
            event.prevent_default();
            handle.command(name)
        }
        EditIntent::DeleteTo(motion) => {
            event.prevent_default();
            delete_to(&handle, motion)
        }
    };
    // Typing is a caret move as much as an edit: drop any vertical-motion goal column
    // so a following Up/Down starts from where the text actually landed.
    set_goal_x(None);
    if handled {
        refresh_caret();
    }
}

/// Reconcile the document with whatever the browser just did to the capture textarea.
///
/// Most edits are intercepted at `beforeinput` and never reach the textarea, so this
/// usually finds it already matching the mirror and does nothing. It is the path for
/// the ones we deliberately let through ([`EditIntent::Reconcile`]) and the safety net
/// for a keyboard that edits the field with no `inputType` we know. Composition is
/// exempt — the IME owns the field until `compositionend`, which reconciles it.
fn on_input() {
    if COMPOSING.with(|c| c.get()) {
        return;
    }
    let Some((_, handle)) = focused_handle() else {
        // Nothing to apply it to; just don't leave text for a later `copy` to pick up
        // (and drop the mirror with it, so nothing later diffs against a cleared field).
        if let Some(ta) = capture_target() {
            clear_mirror(&ta);
        }
        return;
    };
    if reconcile_mirror(&handle) {
        refresh_caret();
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
        registry::end_drag(None);
        refresh_caret();
        return true;
    }

    let x = event.client_x() as f32;
    let y = event.client_y() as f32;

    // A click in a task item's checkbox gutter (left of its content, where the CSS
    // `::before` checkbox renders) toggles its `checked` state instead of placing a
    // caret. The checkbox is a pseudo-element, so the hit is geometric: compare the
    // click x against the item's first child (its content block) left edge.
    if let Some(item) = target.closest("[data-pm-type='task_item']").ok().flatten()
        && let Some(content) = item.first_element_child()
        && (x as f64) < content.get_bounding_client_rect().left()
        && let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(clicked) = handle.pos_at(hit.textblock_nid, hit.byte)
        && handle.toggle_task_checked_at(clicked.0)
    {
        registry::end_drag(None);
        refresh_caret();
        return true;
    }

    if let Some(hit) = resolve_editor_point(doc, x, y)
        && hit.container_nid == container_nid
        && let Some(clicked) = handle.pos_at(hit.textblock_nid, hit.byte)
    {
        match event.detail() {
            2 => {
                handle.select_word_at(clicked);
                registry::end_drag(None);
            }
            n if n >= 3 => {
                handle.select_block_at(clicked);
                registry::end_drag(None);
            }
            _ if event.shift_key() => {
                let anchor = handle.selection().anchor();
                handle.set_selection(Selection::text(anchor, clicked));
                registry::begin_drag(None, container_nid, anchor.0);
            }
            _ => {
                handle.set_selection(Selection::cursor(clicked));
                registry::begin_drag(None, container_nid, clicked.0);
            }
        }
    }
    refresh_caret();
    true
}

/// Handle a `mousemove` while a drag-select is active. Returns whether a drag was live.
fn handle_mousemove(event: &web_sys::MouseEvent, doc: &web_sys::Document) -> bool {
    let Some((container_nid, anchor)) = registry::drag_anchor(None) else {
        return false;
    };
    // If the primary button is no longer held (a mouseup was missed — e.g. released
    // outside the window), the drag is stale: end it instead of following the cursor.
    if event.buttons() & 1 == 0 {
        registry::end_drag(None);
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

/// Translate a `keydown` (its logical `key` and physical `code`) + modifier state into
/// an editor-core `KeyBinding` for keymap lookup.
///
/// **Letters resolve LOGICALLY** (`event.key()`), so `Mod-b` is correct on every layout
/// — on Dvorak/AZERTY the key that types `b` fires bold, not the physical QWERTY-B
/// position. **Digits, symbols, and named keys resolve PHYSICALLY** (`event.code()`),
/// because the logical key for e.g. `Shift+8` is `"*"`, not `"8"` — the physical
/// `Digit8` is what the keymap's `Mod-Shift-8` binding needs. `None` for keys with no
/// bindable identity (they fall through to text insertion / native clipboard).
///
/// (Desktop keys everything physically off the platform `KeyCode`; unifying that for
/// letters would mean plumbing winit's logical key through `PlatformEvent::KeyDown`.)
fn editor_key_binding(
    key: &str,
    code: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<rinch_editor_core::KeyBinding> {
    use rinch_editor_core::{Key, KeyBinding, Modifiers};
    // A single ASCII letter from the *logical* key (layout-correct).
    let logical_letter = {
        let mut it = key.chars();
        match (it.next(), it.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
            _ => None,
        }
    };
    let k = if let Some(c) = logical_letter {
        Key::Char(c)
    } else if let Some(d) = code.strip_prefix("Digit").filter(|s| s.len() == 1) {
        Key::Char(d.as_bytes()[0] as char)
    } else {
        match code {
            "Enter" | "NumpadEnter" => Key::Enter,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Space" => Key::Space,
            "ArrowLeft" => Key::ArrowLeft,
            "ArrowRight" => Key::ArrowRight,
            "ArrowUp" => Key::ArrowUp,
            "ArrowDown" => Key::ArrowDown,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "Minus" => Key::Char('-'),
            "Equal" => Key::Char('='),
            _ => return None,
        }
    };
    Some(KeyBinding::new(
        k,
        Modifiers {
            primary: ctrl,
            shift,
            alt,
        },
    ))
}

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
    // The PHYSICAL key, for keymap lookup — `event.key()` is the *resolved* char ("*" for
    // Shift+8, "¡" for Alt+1) which would never match the keymap's physical-key bindings.
    let code = event.code();
    let ctrl = event.ctrl_key() || event.meta_key();
    let shift = event.shift_key();
    let alt = event.alt_key();

    if key != "ArrowUp" && key != "ArrowDown" {
        set_goal_x(None);
    }

    let handled = match key.as_str() {
        // 1. Cursor movement / selection extension — geometry-dependent (browser layout),
        //    so it stays view-owned and never touches the keymap.
        "ArrowUp" => vertical_step(&handle, container_nid, doc, false, shift),
        "ArrowDown" => vertical_step(&handle, container_nid, doc, true, shift),
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
        // 2. Tab: table cell-nav (shared) first; else the keymap resolves
        //    `Tab`→sinkListItem / `Shift-Tab`→liftListItem below. Consumed either way.
        "Tab" if handle.tab_cell(shift) => true,
        _ => {
            // 3. THE KEYMAP — the single source of truth for every command key. Letters
            //    resolve logically (`key`, layout-correct), digits/symbols physically
            //    (`code`). A matched binding is always consumed (never falls through),
            //    even if the command no-op'd here.
            if let Some(binding) = editor_key_binding(&key, &code, ctrl, shift, alt)
                && handle.dispatch_key(binding).is_some()
            {
                true
            }
            // 4. Plain text insertion — a single non-control char, no modifier. Ctrl/Alt
            //    combos (incl. Ctrl+C/X/V) stay UNCONSUMED so the browser fires its native
            //    ClipboardEvent — never bind clipboard keys in any plugin keymap.
            else if ctrl || alt {
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

    // First, the geometry probe at the goal column. Accept it only if the caret
    // genuinely changed visual line: a move to a *different* textblock, or — within a
    // wrapped block — a caret whose new screen y actually crossed the half-line
    // threshold. Otherwise the probe snapped back to the current line (the target line
    // is an empty block with no text to hit-test, or a block atom is in the way).
    let head_tb = handle.caret_address(head).map(|(t, _)| t);
    let geo_head = resolve_editor_point(doc, gx, ty)
        .filter(|hit| hit.container_nid == container_nid)
        .and_then(|hit| handle.pos_at(hit.textblock_nid, hit.byte))
        .filter(|&p| {
            if handle.caret_address(p).map(|(t, _)| t) != head_tb {
                return true; // different textblock — a real line change
            }
            match head_screen_rect(handle, doc, p) {
                Some((_, py, _)) if down => py > hy + hh * 0.5,
                Some((_, py, _)) => py < hy - hh * 0.5,
                None => false,
            }
        });

    // Stuck — step to the adjacent textblock in the model so the caret can still land
    // on a blank line above/below (or past a block atom). Mirrors the desktop fallback.
    let Some(new_head) =
        geo_head.or_else(|| handle.vertical_block_fallback(down).map(|s| s.head()))
    else {
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

/// True when a pointer event lands inside a mounted editor (`[data-pm-editor]`).
/// On `pointerdown` this is what makes the editor consume it in the capture phase
/// before the generic delegation (mirroring `handle_mousedown` returning `true` for an
/// in-editor click); on `pointerup` it gates the touch-tap focus below.
fn pointer_targets_editor(event: &web_sys::PointerEvent) -> bool {
    event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.closest("[data-pm-editor]").ok().flatten())
        .is_some()
}

/// Whether a contact that went down at `start` and came up at `end` was a tap rather
/// than a scroll.
///
/// Defers to the same classifier the generic pointer delegation uses for its deferred
/// touch clicks (`drag_machine::pending_click_is_scroll`, radial within
/// `TOUCH_MOVE_SLOP`), so a gesture cannot be a tap for the editor and a scroll for the
/// rest of the page — which is what a separate per-axis threshold here would produce for
/// a diagonal drift.
fn is_tap(start: (f32, f32), end: (f32, f32)) -> bool {
    !drag_machine::pending_click_is_scroll(end.0 - start.0, end.1 - start.1)
}

/// Record a touch/pen contact that went down inside an editor, so its release can be
/// classified as a tap. Mouse pointers are ignored — they get focus from `mousedown`.
fn note_touch_down(event: &web_sys::PointerEvent) {
    if event.pointer_type() == "mouse" || !event.is_primary() {
        return;
    }
    TOUCH_TAP.with(|c| {
        c.set(Some((
            event.pointer_id(),
            event.client_x() as f32,
            event.client_y() as f32,
        )))
    });
}

/// Focus an editor from a touch/pen **tap**, placing the caret where it landed.
///
/// The mouse path cannot cover this. A tap's compatibility `mousedown` is synthesized
/// late, after `pointerup` — too late to be the *user gesture* iOS requires before it
/// will raise the on-screen keyboard for a programmatic `.focus()` — and on a page that
/// consumes the pointer sequence it may never be synthesized at all, leaving the editor
/// visibly focused but with no model focus, so every keystroke went nowhere. Running
/// the same handler here, from the real touch event, makes the tap self-sufficient; the
/// compatibility `mousedown` that may follow is idempotent (same point, same caret).
///
/// A contact that drifted past the shared touch slop was a scroll, not a tap, and is
/// dropped — panning a manuscript must not pop the keyboard.
fn handle_touch_tap(event: &web_sys::PointerEvent, doc: &web_sys::Document) {
    // Read before clearing, and clear only for the contact we recorded: a second
    // finger's release must not discard the tap the first one is still making.
    let Some((pointer_id, x, y)) = TOUCH_TAP.with(|c| c.get()) else {
        return;
    };
    if event.pointer_id() != pointer_id {
        return;
    }
    TOUCH_TAP.with(|c| c.set(None));
    if !is_tap((x, y), (event.client_x() as f32, event.client_y() as f32)) {
        return;
    }
    if !pointer_targets_editor(event) {
        return;
    }
    // `PointerEvent` *is* a `MouseEvent`, so the click handler takes it as-is. Its
    // `detail` is 0 for touch, which lands on the plain place-the-caret arm; a
    // double-tap's word select still comes from the compatibility `mousedown`.
    handle_mousedown(event.as_ref(), doc);
    // Drag-select follows `mousemove` with a button held — unreachable from touch — so
    // never leave a touch tap's anchor armed behind it.
    registry::end_drag(None);
}

/// Add a capture-phase `document` listener leaked for the page lifetime.
pub(crate) fn add_capture<E: JsCast + 'static>(
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
    // Mirror the mousedown gate onto `pointerdown`. The generic event delegation
    // (`event_delegation.rs`) now delegates on `pointerdown` (bubble phase) — a
    // separate dispatch that the `stop_propagation()` on `mousedown` above cannot
    // stop. Consume an in-editor pointerdown here in the capture phase (document
    // capture fires before the generic bubble handler) so it never reaches the
    // generic click / `data-onmousedown` dispatch — which would otherwise fire an
    // ancestor's `onclick` when an editor is nested inside a clickable card/row.
    // Caret placement still happens from the `mousedown` handler above; we only
    // suppress the redundant generic delegation, never `prevent_default` (which
    // could swallow the compatibility `mousedown` the editor relies on).
    add_capture(
        browser_doc,
        "pointerdown",
        move |e: web_sys::PointerEvent| {
            if pointer_targets_editor(&e) {
                // Remember the contact so `pointerup` can tell a tap from a scroll and
                // raise the on-screen keyboard inside the gesture (see `handle_touch_tap`).
                note_touch_down(&e);
                e.stop_propagation();
            }
        },
    );
    let doc = browser_doc.clone();
    add_capture(browser_doc, "pointerup", move |e: web_sys::PointerEvent| {
        handle_touch_tap(&e, &doc);
    });
    add_capture(
        browser_doc,
        "pointercancel",
        move |e: web_sys::PointerEvent| {
            // Only the recorded contact's own cancellation ends its tap — matching the
            // pointer-id check in `handle_touch_tap`. A second finger's cancel (or any
            // unrelated pointercancel on the page) must not discard a live tap.
            TOUCH_TAP.with(|c| {
                if c.get().is_some_and(|(id, _, _)| id == e.pointer_id()) {
                    c.set(None);
                }
            });
        },
    );
    let doc = browser_doc.clone();
    add_capture(browser_doc, "mousemove", move |e: web_sys::MouseEvent| {
        if handle_mousemove(&e, &doc) {
            e.prevent_default();
        }
    });
    add_capture(browser_doc, "mouseup", move |_e: web_sys::MouseEvent| {
        registry::end_drag(None);
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── `beforeinput` → model intent ──────────────────────────────────────────
    //
    // This mapping is the whole soft-keyboard path: Android reports printable keys as
    // `Unidentified` / 229, so `handle_keydown` never sees them and `inputType` is the
    // only description of the edit we get.

    #[test]
    fn plain_typing_inserts_the_events_data() {
        assert_eq!(edit_intent("insertText"), EditIntent::InsertText);
    }

    #[test]
    fn enter_and_shift_enter_split_and_break() {
        assert_eq!(edit_intent("insertParagraph"), EditIntent::Command("enter"));
        assert_eq!(
            edit_intent("insertLineBreak"),
            EditIntent::Command("insertHardBreak")
        );
    }

    #[test]
    fn the_delete_family_maps_to_commands_or_motions() {
        assert_eq!(
            edit_intent("deleteContent"),
            EditIntent::Command("deleteSelection")
        );
        assert_eq!(
            edit_intent("deleteContentBackward"),
            EditIntent::Command("deleteCharBackward")
        );
        assert_eq!(
            edit_intent("deleteContentForward"),
            EditIntent::Command("deleteCharForward")
        );
        assert_eq!(
            edit_intent("deleteWordBackward"),
            EditIntent::DeleteTo(CursorMotion::WordLeft)
        );
        assert_eq!(
            edit_intent("deleteWordForward"),
            EditIntent::DeleteTo(CursorMotion::WordRight)
        );
        // "soft" and "hard" line deletes differ only in how the browser found the
        // boundary; both mean "to the edge of this line" on the model.
        for t in ["deleteSoftLineBackward", "deleteHardLineBackward"] {
            assert_eq!(
                edit_intent(t),
                EditIntent::DeleteTo(CursorMotion::LineStart)
            );
        }
        for t in ["deleteSoftLineForward", "deleteHardLineForward"] {
            assert_eq!(edit_intent(t), EditIntent::DeleteTo(CursorMotion::LineEnd));
        }
    }

    #[test]
    fn history_gestures_reach_the_history_plugin() {
        assert_eq!(edit_intent("historyUndo"), EditIntent::Command("undo"));
        assert_eq!(edit_intent("historyRedo"), EditIntent::Command("redo"));
    }

    #[test]
    fn edits_another_handler_already_applied_are_ignored() {
        // Composition: the composition events show the preedit and commit it. Applying
        // this too would type every composed word twice.
        assert_eq!(edit_intent("insertCompositionText"), EditIntent::Ignore);
        // Clipboard: `on_cut` / `on_paste` ran on the native ClipboardEvent.
        for t in [
            "deleteByCut",
            "insertFromPaste",
            "insertFromPasteAsQuotation",
        ] {
            assert_eq!(edit_intent(t), EditIntent::Ignore);
        }
    }

    #[test]
    fn autocorrect_is_reconciled_rather_than_inserted() {
        // The event says what to insert but not what it replaces, so it is applied to
        // the mirrored textarea and read back off the diff. Inserting `data` at the
        // caret would leave both the typo and the correction.
        assert_eq!(edit_intent("insertReplacementText"), EditIntent::Reconcile);
    }

    #[test]
    fn an_unknown_input_type_is_a_no_op() {
        assert_eq!(edit_intent("insertFromDrop"), EditIntent::Ignore);
        assert_eq!(edit_intent("formatBold"), EditIntent::Ignore);
        assert_eq!(edit_intent(""), EditIntent::Ignore);
    }

    // ── The mirror diff ───────────────────────────────────────────────────────
    //
    // What the browser did to the mirrored textarea has to be recovered from the text
    // alone: a keyboard's replacement range never reaches us (`getTargetRanges()` is
    // empty on a textarea, and a composition's region is not exposed at all).

    /// `(from, to, inserted)` rendered as a readable expectation.
    fn diff(base: &str, now: &str) -> Option<(usize, usize, String)> {
        text_diff(base, now)
    }

    #[test]
    fn an_unchanged_field_is_no_edit() {
        assert_eq!(diff("hello", "hello"), None);
        assert_eq!(diff("", ""), None);
    }

    #[test]
    fn a_suggestion_narrows_to_the_characters_that_actually_changed() {
        // The reported bug: "word" + a tap on the "world" suggestion. The keyboard
        // replaces the whole word, but the minimal edit is one inserted `l` — which is
        // what keeps the surrounding marks intact and undo granular.
        assert_eq!(diff("word", "world"), Some((3, 3, "l".into())));
        // Mid-block, with text either side that must not be touched.
        assert_eq!(
            diff("hello word here", "hello world here"),
            Some((9, 9, "l".into()))
        );
    }

    #[test]
    fn a_replacement_sharing_no_edges_spans_the_whole_word() {
        assert_eq!(diff("teh", "the"), Some((1, 3, "he".into())));
        // The transposition is the only thing that moved: "ht" → "th", with the
        // sentence either side untouched.
        assert_eq!(
            diff("say hte thing", "say the thing"),
            Some((4, 6, "th".into()))
        );
    }

    #[test]
    fn plain_typing_and_deleting_are_ordinary_diffs() {
        assert_eq!(diff("wor", "word"), Some((3, 3, "d".into())));
        assert_eq!(diff("word", "wor"), Some((3, 4, String::new())));
        assert_eq!(diff("", "a"), Some((0, 0, "a".into())));
        assert_eq!(diff("a", ""), Some((0, 1, String::new())));
    }

    #[test]
    fn a_word_delete_reports_an_empty_insertion() {
        assert_eq!(
            diff("hello world here", "hello  here"),
            Some((6, 11, String::new()))
        );
    }

    #[test]
    fn an_emptied_field_behind_a_live_mirror_reads_as_deleting_the_block() {
        // Why `clear_mirror` empties the field and drops the mirror *together*, and why
        // `reconcile_mirror` re-syncs when the block no longer holds `mirror.text`:
        // a field cleared behind a standing mirror is indistinguishable, at this layer,
        // from the user selecting the paragraph and hitting Backspace. The diff is
        // honest — there is no guard to add here — so the pairing has to hold upstream,
        // in the only two places that write the field.
        assert_eq!(
            diff("a whole paragraph of prose", ""),
            Some((0, 26, String::new()))
        );
    }

    #[test]
    fn the_prefix_and_suffix_scans_never_overlap() {
        // A repeated run is where a naive two-ended scan double-counts and reports a
        // negative-width range. "aaa" → "aa" must be one deletion, not two.
        assert_eq!(diff("aaa", "aa"), Some((2, 3, String::new())));
        assert_eq!(diff("aa", "aaa"), Some((2, 2, "a".into())));
        let (from, to, _) = diff("aaaa", "aa").unwrap();
        assert!(from <= to, "range inverted: {from}..{to}");
    }

    #[test]
    fn offsets_are_chars_so_multibyte_text_maps_correctly() {
        // Char indices, not bytes: the caller turns them into byte offsets against the
        // same string. An accent or an emoji must not shift the range.
        assert_eq!(diff("café", "cafés"), Some((4, 4, "s".into())));
        assert_eq!(diff("naïve", "native"), Some((2, 3, "ti".into())));
        assert_eq!(diff("hi 👋", "hi 👋!"), Some((4, 4, "!".into())));
    }

    #[test]
    fn char_offsets_convert_back_to_byte_offsets() {
        let s = "café!";
        assert_eq!(byte_of_char(s, 0), 0);
        assert_eq!(byte_of_char(s, 3), 3);
        // `é` is two bytes, so everything after it shifts.
        assert_eq!(byte_of_char(s, 4), 5);
        assert_eq!(byte_of_char(s, 5), 6);
        // Past the end clamps to the length, so a stale mirror cannot panic.
        assert_eq!(byte_of_char(s, 99), s.len());
    }

    #[test]
    fn utf16_lengths_are_what_the_textarea_counts_in() {
        assert_eq!(utf16_len_upto("abc", 3), 3);
        assert_eq!(utf16_len_upto("café", "café".len()), 4);
        // Astral characters are surrogate pairs — two units, one char.
        assert_eq!(utf16_len_upto("👋", 4), 2);
        // Partial counts: `é` starts at byte 3 and is two bytes wide.
        assert_eq!(utf16_len_upto("café!", 3), 3);
        assert_eq!(utf16_len_upto("café!", 5), 4);
    }

    #[test]
    fn a_byte_offset_off_a_char_boundary_never_panics() {
        // The model supplies the byte offset and the DOM supplies the text; if they
        // ever disagree, counting must degrade, not blow the page up on a bad slice.
        // Byte 4 is inside the emoji: it counts as the whole character, not a panic.
        assert_eq!(utf16_len_upto("hi 👋!", 4), 5);
        assert_eq!(utf16_len_upto("hi 👋!", 99), 6); // past the end
        assert_eq!(utf16_len_upto("hi 👋!", 0), 0);
    }

    #[test]
    fn a_textarea_caret_offset_converts_back_to_a_byte_offset() {
        // The shared converter from `event_delegation` — the same one the pointer
        // hit-test uses to read a browser selection offset.
        assert_eq!(utf16_offset_to_utf8_bytes("abc", 0), 0);
        assert_eq!(utf16_offset_to_utf8_bytes("abc", 3), 3);
        // `é` is one UTF-16 unit but two bytes.
        assert_eq!(utf16_offset_to_utf8_bytes("café!", 4), 5);
        // `👋` is two UTF-16 units and four bytes: offset 3 is before it.
        assert_eq!(utf16_offset_to_utf8_bytes("hi 👋!", 3), 3);
        assert_eq!(utf16_offset_to_utf8_bytes("hi 👋!", 5), 7);
        // Past the end clamps — a selection read after the field moved on must not
        // panic.
        assert_eq!(utf16_offset_to_utf8_bytes("hi 👋!", 99), "hi 👋!".len());
    }

    #[test]
    fn utf16_offsets_round_trip_through_bytes() {
        for text in ["", "plain", "café", "hi 👋 there", "aaa"] {
            let units = utf16_len_upto(text, text.len());
            assert_eq!(
                utf16_offset_to_utf8_bytes(text, units),
                text.len(),
                "{text:?}"
            );
            assert_eq!(utf16_offset_to_utf8_bytes(text, 0), 0, "{text:?}");
        }
    }

    // ── Tap vs scroll ─────────────────────────────────────────────────────────

    #[test]
    fn a_still_contact_is_a_tap_and_a_swipe_is_not() {
        assert!(is_tap((100.0, 200.0), (100.0, 200.0)));
        // Fingers wobble; a few px is still a tap.
        assert!(is_tap((100.0, 200.0), (104.0, 197.0)));
        // Right up to the slop on one axis is still a tap.
        let slop = drag_machine::TOUCH_MOVE_SLOP;
        assert!(is_tap((100.0, 200.0), (100.0 + slop, 200.0)));
        assert!(is_tap((100.0, 200.0), (100.0, 200.0 - slop)));
        // A scroll must not raise the keyboard, in either direction or axis.
        assert!(!is_tap((100.0, 200.0), (100.0, 260.0)));
        assert!(!is_tap((100.0, 200.0), (100.0, 140.0)));
        assert!(!is_tap((100.0, 200.0), (160.0, 200.0)));
    }
}
