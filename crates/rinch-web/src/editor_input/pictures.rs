//! Pictures pasted or dropped into an editor, offered to the app's image
//! input hook, and other dropped files.

use super::*;

// ── Pictures offered to the app (`EditorHandle::on_image_input`) ───────────────

/// The image files a paste or a drop carries, in order: every `file` item the
/// browser calls an image. Taken while the event is being dispatched, which is
/// the only time a `DataTransfer` can be read; the `File`s stay readable after.
pub(super) fn image_files(dt: &web_sys::DataTransfer) -> Vec<web_sys::File> {
    let items = dt.items();
    (0..items.length())
        .filter_map(|i| items.get(i))
        .filter(|item| item.kind() == "file" && item.type_().starts_with("image/"))
        .filter_map(|item| item.get_as_file().ok().flatten())
        .collect()
}

/// A file read for an `on_image_input` callback.
struct ReadImage {
    bytes: Vec<u8>,
    mime: String,
    name: Option<String>,
}

/// Read `files` and offer each to `handle`'s `on_image_input` callback where
/// the selection is **now** (anchored, so typing while the files are read does
/// not move it, and an inserted picture moves the anchor past itself):
/// pictures answered at once land side by side, in order. A document replaced
/// meanwhile drops the lot.
pub(super) fn offer_image_files(
    handle: &EditorHandle,
    source: ImageInputSource,
    files: Vec<web_sys::File>,
) {
    let handle = handle.clone();
    let anchor = handle.anchor_selection();
    read_images(
        files.into(),
        Vec::new(),
        Box::new(move |images| offer_read_images(&handle, &anchor, source, images)),
    );
}

fn offer_read_images(
    handle: &EditorHandle,
    anchor: &SelectionAnchor,
    source: ImageInputSource,
    images: Vec<ReadImage>,
) {
    // At the anchor, not at the live selection: the person's caret is theirs
    // if they moved while the files were read.
    for image in images {
        handle.offer_image_input_at(anchor, source, image.bytes, &image.mime, image.name);
    }
    refresh_caret();
}

/// Read `pending` one after another with a `FileReader` each (a read is
/// asynchronous in a browser), then hand everything read to `done`. A file
/// that cannot be read is left out. Its type is what its first bytes say, else
/// what the browser called it (an SVG, an AVIF).
fn read_images(
    mut pending: std::collections::VecDeque<web_sys::File>,
    mut read: Vec<ReadImage>,
    done: Box<dyn FnOnce(Vec<ReadImage>)>,
) {
    let Some(file) = pending.pop_front() else {
        done(read);
        return;
    };
    let Ok(reader) = web_sys::FileReader::new() else {
        read_images(pending, read, done);
        return;
    };
    // `loadend` fires once, after a success or a failure. The closure frees
    // itself when it has run.
    let loaded = {
        let reader = reader.clone();
        let file = file.clone();
        Closure::once_into_js(move || {
            if let Ok(result) = reader.result()
                && let Ok(buffer) = result.dyn_into::<js_sys::ArrayBuffer>()
            {
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                let mime = sniff_image_mime(&bytes)
                    .map(str::to_string)
                    .unwrap_or_else(|| file.type_());
                let name = Some(file.name()).filter(|n| !n.is_empty());
                if !bytes.is_empty() {
                    read.push(ReadImage { bytes, mime, name });
                }
            }
            read_images(pending, read, done);
        })
    };
    reader.set_onloadend(Some(loaded.unchecked_ref()));
    let _ = reader.read_as_array_buffer(&file);
}

/// The editor a drag of files is over, when its app takes pictures itself: its
/// container id and handle. An editor with no callback is not involved in
/// drops at all. One that has the callback and refuses edits (read-only, a
/// collaboration freeze) is answered too: it takes nothing, but the drop must
/// still be prevented ([`on_drop`]).
fn file_drop_editor(event: &web_sys::DragEvent) -> Option<(u64, EditorHandle)> {
    let dt = event.data_transfer()?;
    let types = dt.types();
    if !(0..types.length()).any(|i| types.get(i).as_string().as_deref() == Some("Files")) {
        return None;
    }
    let target = event.target()?.dyn_into::<web_sys::Node>().ok()?;
    let element = match target.dyn_ref::<web_sys::Element>() {
        Some(el) => el.clone(),
        None => target.parent_element()?,
    };
    let container = crate::event_delegation::editor_owning(&element)?;
    let id = get_nid(&container.into())?.0;
    let handle = registry::editor_for(id)?;
    handle.has_image_input_callback().then_some((id, handle))
}

/// `dragover` of files over an editor that takes pictures: say the drop is
/// wanted, or the browser never delivers it. Over one that refuses edits the
/// event is prevented all the same, with the "not here" cursor: left to the
/// browser, releasing the file would open it in place of the app.
pub(super) fn on_drag_over(event: &web_sys::DragEvent) {
    if let Some((_, handle)) = file_drop_editor(event) {
        event.prevent_default();
        if let Some(dt) = event.data_transfer() {
            dt.set_drop_effect(if handle.refuses_edits() {
                "none"
            } else {
                "copy"
            });
        }
    }
}

/// Files dropped on an editor that takes pictures: the caret goes to the drop
/// point, the editor takes the keyboard, and the image files are offered to
/// the app (`EditorHandle::on_image_input`), in order. The browser's own
/// handling of the drop (opening the file in place of the page) is prevented
/// whatever the files are. The files the editor does not take — the others of
/// a mixed drop, every file when none is a picture or the editor refuses edits
/// — go to the `ondropfiles` handler under the drop point, if there is one
/// (issue #1771, desktop's rule). An editor with no callback is not involved
/// at all. One that refuses edits (read-only) takes nothing: no offer, no
/// caret move, no focus. A viewer of a read-only note who drops a picture on
/// it must not lose the app to the browser's navigation.
pub(super) fn on_drop(event: &web_sys::DragEvent, doc: &web_sys::Document) {
    let Some((id, handle)) = file_drop_editor(event) else {
        return;
    };
    event.prevent_default();
    let files = event
        .data_transfer()
        .map(|dt| crate::event_delegation::dropped_files(&dt))
        .unwrap_or_default();
    if handle.refuses_edits() {
        crate::event_delegation::dispatch_dropped_files(event, files);
        return;
    }
    let (images, rest): (Vec<_>, Vec<_>) = files
        .into_iter()
        .partition(|file| file.type_().starts_with("image/"));
    if images.is_empty() {
        crate::event_delegation::dispatch_dropped_files(event, rest);
        return;
    }
    if let Some(pos) = resolve_editor_point(doc, event.client_x() as f32, event.client_y() as f32)
        .filter(|hit| hit.container_nid == id)
        .and_then(|hit| hit.pos(&handle))
    {
        handle.set_selection(Selection::cursor(pos));
    }
    handle.focus();
    refresh_caret();
    offer_image_files(&handle, ImageInputSource::Drop, images);
    if !rest.is_empty() {
        crate::event_delegation::dispatch_dropped_files(event, rest);
    }
}
