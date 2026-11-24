//! Copy, cut and paste through the browser's `ClipboardEvent`.

use super::capture::capture_target;
use super::context_menu::end_context_menu_cycle;
use super::mirror::clear_mirror;
use super::pictures::{image_files, offer_image_files};
use super::*;

// ── Clipboard (copy / cut / paste) ─────────────────────────────────────────────
//
// Mirrors the desktop `editor_copy`/`editor_cut`/`editor_paste`, but reads/writes
// the browser's native `ClipboardEvent.clipboardData` (synchronous, the only path
// to `text/html`) instead of the OS clipboard crate. The model serialization
// (`selection_clipboard` / `replace_selection_with_*`) is the shared handle API.

/// Copy: write the selection's `(text/html, text/plain)` to the clipboard. Always
/// `preventDefault` so the empty hidden textarea is never copied (which would clobber
/// the clipboard); only write data when there is a non-empty selection.
pub(super) fn on_copy(event: &web_sys::ClipboardEvent) {
    // The menu's Copy: the cycle has served its purpose.
    end_context_menu_cycle();
    let Some((_, handle)) = focused_handle() else {
        refuse_orphan_clipboard(event);
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

/// A `copy` or `cut` on the capture textarea with no editor behind it: never the
/// browser's native one. The field only ever holds a mirror of an editor's text, so
/// a native copy of it is a copy of text that is not on the page (issue #1112).
/// [`on_editor_unregistered`] empties and blurs it when the focused editor
/// unmounts; this is the net for whatever still reaches it (the listener is on the
/// textarea itself, so every event here is aimed at it).
///
/// [`on_editor_unregistered`]: super::capture::on_editor_unregistered
fn refuse_orphan_clipboard(event: &web_sys::ClipboardEvent) {
    event.prevent_default();
    if let Some(ta) = capture_target() {
        clear_mirror(&ta);
    }
}

/// Cut: copy the selection, then delete it. Nothing at all in a read-only editor,
/// as in a `readonly` field: the delete would be refused, and a Cut that only
/// copies has changed the clipboard while appearing to do nothing. (The capture
/// field being `readonly` keeps Cut out of the browser's own menu; this is for a
/// `cut` that arrives anyway.)
pub(super) fn on_cut(event: &web_sys::ClipboardEvent) {
    end_context_menu_cycle();
    let Some((_, handle)) = focused_handle() else {
        refuse_orphan_clipboard(event);
        return;
    };
    event.prevent_default();
    if handle.refuses_edits() {
        return;
    }
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
/// (encoded as a `data:` URL via `FileReader`), then `text/plain` — the order
/// desktop's clipboard probe resolves them in.
///
/// An editor whose app takes pictures itself (`EditorHandle::on_image_input`)
/// is offered the clipboard's image files instead, and no `data:` URL is made:
/// when there is no html, or the html is pictures and nothing else (a
/// browser's "Copy image" offers an `<img>` beside the file), as on desktop.
///
/// Text and html go through `EditorHandle::paste`, the entry point the desktop's
/// Ctrl+V shares, so the editor's plugins see both flavours first
/// (`Plugin::handle_paste`) and the default inserts what none claims. A bitmap
/// with no html wrapper goes in as an image and reaches no plugin, as on desktop.
pub(super) fn on_paste(event: &web_sys::ClipboardEvent) {
    end_context_menu_cycle();
    let Some((_, handle)) = focused_handle() else {
        return;
    };
    event.prevent_default();
    let Some(dt) = event.clipboard_data() else {
        return;
    };
    let paste = PasteContent::new(
        dt.get_data("text/plain").ok(),
        dt.get_data("text/html").ok(),
    );
    // 0. Pictures, for an app that stores them itself: the files, read as
    //    their bytes and offered at the selection as it is now, anchored.
    if handle.has_image_input_callback() && !handle.refuses_edits() {
        let files = image_files(&dt);
        let only_pictures = paste
            .html
            .as_deref()
            .is_none_or(|html| html.trim().is_empty() || handle.html_is_only_images(html));
        if !files.is_empty() && only_pictures {
            offer_image_files(&handle, ImageInputSource::Paste, files);
            return;
        }
    }
    // 1. Html (structure, links, marks, URL-referenced images), offered to the
    //    plugins first with the text beside it; its text if it parses to nothing.
    if paste.html.is_some() && handle.paste(&paste) {
        refresh_caret();
        return;
    }
    // 2. A bitmap with no HTML wrapper (a screenshot / "copy image"): read the first
    //    image file as a data URL and insert it asynchronously.
    if paste_image_from(&dt, &handle) {
        return;
    }
    // 3. Plain text, offered to the plugins first.
    if paste.html.is_none() && handle.paste(&paste) {
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
