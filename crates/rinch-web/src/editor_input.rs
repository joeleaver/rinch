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
//! it. There is no `contenteditable`, so this v1 handles physical keys (all Latin text,
//! shortcuts, navigation, selection, paste/copy/cut). **IME composition is a documented
//! follow-up** — it needs a focused editable capture target (a hidden textarea), a
//! separate piece of work.

use std::cell::Cell;

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
/// `preventDefault`s + stops it). Mirrors `dispatch_new_editor_key`. Clipboard keys
/// (Ctrl+C/X/V) are intentionally *not* consumed here — the browser fires dedicated
/// `copy`/`cut`/`paste` events that carry the clipboard payload.
fn handle_keydown(event: &web_sys::KeyboardEvent, doc: &web_sys::Document) -> bool {
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

/// The viewport `(x, y, height)` of the caret at model `pos`, via a collapsed `Range`
/// at the host text node. `None` for an empty block (no text node).
fn head_screen_rect(
    handle: &EditorHandle,
    doc: &web_sys::Document,
    pos: Pos,
) -> Option<(f32, f32, f32)> {
    let (tb_nid, byte) = handle.caret_address(pos)?;
    let block = node_by_nid(tb_nid)?;
    let (text_node, off) = find_text_node_at_byte_offset(&block, byte)?;
    let range = doc.create_range().ok()?;
    range.set_start(&text_node, off).ok()?;
    range.set_end(&text_node, off).ok()?;
    let r = range.get_bounding_client_rect();
    let h = if r.height() > 0.0 {
        r.height() as f32
    } else {
        18.0
    };
    Some((r.x() as f32, r.y() as f32, h))
}

// ── Clipboard ────────────────────────────────────────────────────────────────

/// Handle a `paste`: prefer rich HTML, else plain text. Returns whether the editor is
/// focused (so the listener `preventDefault`s the browser's own paste).
fn handle_paste(event: &web_sys::ClipboardEvent) -> bool {
    let Some((_, handle)) = focused_handle() else {
        return false;
    };
    if let Some(data) = event.clipboard_data() {
        let html = data.get_data("text/html").unwrap_or_default();
        let changed = if !html.trim().is_empty() {
            handle.replace_selection_with_html(&html)
        } else {
            let text = data.get_data("text/plain").unwrap_or_default();
            !text.is_empty() && handle.replace_selection_with_text(&text)
        };
        if changed {
            refresh_caret();
        }
    }
    true
}

/// Handle `copy`/`cut`: write the selection as HTML + text, and (cut) delete it. Returns
/// whether the editor is focused (so the listener `preventDefault`s the default copy).
fn handle_copy(event: &web_sys::ClipboardEvent, cut: bool) -> bool {
    let Some((_, handle)) = focused_handle() else {
        return false;
    };
    if let Some((html, text)) = handle.selection_clipboard()
        && let Some(data) = event.clipboard_data()
    {
        let _ = data.set_data("text/html", &html);
        let _ = data.set_data("text/plain", &text);
        if cut {
            handle.command("deleteSelection");
            refresh_caret();
        }
    }
    true
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

/// Install the editor input listeners once. Called from `mount_tree` alongside
/// `ensure_event_delegation`. Idempotent.
pub(crate) fn install(browser_doc: &web_sys::Document) {
    if INSTALLED.with(|c| c.replace(true)) {
        return;
    }

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
    add_capture(browser_doc, "paste", move |e: web_sys::ClipboardEvent| {
        if handle_paste(&e) {
            e.prevent_default();
        }
    });
    add_capture(browser_doc, "copy", move |e: web_sys::ClipboardEvent| {
        if handle_copy(&e, false) {
            e.prevent_default();
        }
    });
    add_capture(browser_doc, "cut", move |e: web_sys::ClipboardEvent| {
        if handle_copy(&e, true) {
            e.prevent_default();
        }
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
