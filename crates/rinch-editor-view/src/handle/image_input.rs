//! Inserting images and offering image input (paste, drop) to the app's
//! hook.

use super::*;

impl EditorHandle {
    /// Insert an image node with `src` (e.g. a `data:` URL) and `alt`, replacing
    /// the current selection — the image-paste path. Returns whether the document
    /// changed (the schema rejects an image where inline content isn't allowed).
    pub fn insert_image(&self, src: &str, alt: &str) -> bool {
        let cmd = rinch_editor_core::commands::insert_image(src.to_string(), alt.to_string());
        let mut core = self.core_mut();
        let Some((next, mapping)) = run_stamped(&core.state, &cmd) else {
            return false;
        };
        let prev = core.state.clone();
        let Some(doc_changed) =
            core.commit(prev, next, Some(&mapping), Scroll::IfChanged, Vec::new())
        else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Register the callback that is offered **every picture the person pastes
    /// into or drops on this editor**. Replaces any previously registered
    /// callback.
    ///
    /// This is how an app decides where a picture's bytes live. The editor
    /// hands over the encoded bytes ([`ImageInput`]); the app stores them and
    /// answers with the `src` (and `alt`) the document should carry. Only that
    /// `src` enters the document, so an app that registers this never gets a
    /// `data:` URL from a pasted bitmap or a dropped file.
    ///
    /// The callback may answer in either of two ways:
    ///
    /// - **At once**: return `Some((src, alt))`, and the editor inserts the
    ///   image where the picture was aimed (one transaction, one undo step, as
    ///   [`insert_image`](Self::insert_image)).
    /// - **Later**: keep the input, return `None`, and when the picture is
    ///   stored call [`insert_image_at`](Self::insert_image_at) with
    ///   [`ImageInput::anchor`], which still names the place the person aimed
    ///   at however much they, or a collaborating peer, typed meanwhile. The
    ///   picture lands there and the person's caret stays where they are. If
    ///   that call answers `false` the place is gone (or takes no image any
    ///   more): delete what was stored, nothing references it.
    ///
    /// `None` with nothing later is a refusal: nothing is inserted, and the
    /// editor does **not** fall back to a `data:` URL.
    ///
    /// ```ignore
    /// let editor = handle.clone();
    /// handle.on_image_input(move |input| {
    ///     let editor = editor.clone();
    ///     store_blob_async(input.bytes, input.mime, move |src| {
    ///         editor.insert_image_at(&input.anchor, &src, "");
    ///     });
    ///     None
    /// });
    /// ```
    ///
    /// When it is called: a paste whose clipboard holds a bitmap (a screenshot;
    /// a browser's "Copy image", which also offers an `<img>` as html, counts,
    /// and the bitmap is what is offered), and a drop of image files on the
    /// editor, once per file in the order given, each a PNG, JPEG, GIF or WebP
    /// by its first bytes. Not for a [read-only](Self::set_read_only) editor,
    /// not where an image cannot be inserted (a code block: the app is not
    /// asked to store a picture that would be refused), and not for an html
    /// paste that holds text as well as pictures: that is
    /// an ordinary paste, and its `<img>` elements keep the `src` they came
    /// with ([`Plugin::handle_paste`] is where an app rewrites those).
    ///
    /// **With no callback registered** nothing here applies and the platform
    /// keeps its default: a pasted bitmap is inserted as a `data:` URL, and a
    /// file drop goes to the app's own file-drop handler on desktop and to the
    /// browser on the web.
    ///
    /// The same on both platforms. In the browser a pasted or dropped picture
    /// is a `File`: its bytes are offered untouched (a pasted bitmap arrives
    /// from the browser as a PNG file), `name` is the browser's name for it
    /// for a paste as well, and a dropped file counts as an image when the
    /// browser says so.
    ///
    /// The callback runs with no internal borrow held, so it may re-enter the
    /// handle. Registered while a component renders it belongs to that
    /// component and is not called once it unmounts, like
    /// [`on_change`](Self::on_change).
    pub fn on_image_input(&self, cb: impl Fn(ImageInput) -> Option<(String, String)> + 'static) {
        self.core_mut().on_image_input = Some(Hook::new(Rc::new(cb)));
    }

    /// Whether an [`on_image_input`](Self::on_image_input) callback is
    /// registered and would be called (its component is still mounted). A
    /// platform runtime asks before it does the work of reading a bitmap or a
    /// dropped file.
    pub fn has_image_input_callback(&self) -> bool {
        self.core()
            .on_image_input
            .as_ref()
            .is_some_and(|hook| hook.owner.as_ref().is_none_or(Owner::is_alive))
    }

    /// Offer a picture aimed at the **current selection** to the
    /// [`on_image_input`](Self::on_image_input) callback, and insert the image
    /// there if it answers with one. Returns whether an image was inserted by
    /// this call. [`offer_image_input_at`](Self::offer_image_input_at) at the
    /// selection as it is now.
    pub fn offer_image_input(
        &self,
        source: ImageInputSource,
        bytes: Vec<u8>,
        mime: &str,
        name: Option<String>,
    ) -> bool {
        let at = self.anchor_selection();
        self.offer_image_input_at(&at, source, bytes, mime, name)
    }

    /// Offer a picture aimed at `at` to the
    /// [`on_image_input`](Self::on_image_input) callback, and insert the image
    /// there if it answers with one. The platform runtime calls this with the
    /// place the picture goes (the paste's anchor, the caret at the drop
    /// point), which may no longer be where the person's caret is: the
    /// clipboard or the disk was read in between. Returns whether an image was
    /// inserted by this call.
    ///
    /// Answers `false` **without calling the callback** when none is
    /// registered, when the editor [refuses edits](Self::refuses_edits), when
    /// `at` no longer names a place or belongs to another editor, and when the
    /// place takes no image (a code block): the app is not asked to store a
    /// picture that cannot be inserted. The callback runs with no internal
    /// borrow held.
    pub fn offer_image_input_at(
        &self,
        at: &SelectionAnchor,
        source: ImageInputSource,
        bytes: Vec<u8>,
        mime: &str,
        name: Option<String>,
    ) -> bool {
        if self.refuses_edits() {
            return false;
        }
        let Some(hook) = self.core().on_image_input.clone() else {
            return false;
        };
        let Some(selection) = self.own_anchor_selection(at) else {
            return false;
        };
        if !self.takes_image_at(&selection) {
            return false;
        }
        // The app's own anchor over the same place: `at` is the caller's, and
        // is what a synchronous answer is inserted at, so a callback that moved
        // the selection (opened a dialog, focused elsewhere) does not move the
        // picture.
        let input = ImageInput {
            source,
            bytes,
            mime: mime.to_string(),
            name,
            anchor: self.anchor_at(selection),
        };
        match hook.invoke(|cb| cb(input)).flatten() {
            Some((src, alt)) => self.insert_image_at(at, &src, &alt),
            None => false,
        }
    }

    /// An anchor over `selection`, a selection of the current document.
    fn anchor_at(&self, selection: Selection) -> SelectionAnchor {
        let anchors = self.core().anchors.clone();
        let id = {
            let mut map = anchors.borrow_mut();
            map.next_id += 1;
            let id = map.next_id;
            map.live.insert(id, Some(selection));
            id
        };
        SelectionAnchor {
            anchors: Rc::downgrade(&anchors),
            id,
        }
    }

    /// Where `anchor` points now; `None` when it names no place any more or was
    /// taken from a different editor.
    fn own_anchor_selection(&self, anchor: &SelectionAnchor) -> Option<Selection> {
        let ours = Rc::downgrade(&self.core().anchors);
        if !Weak::ptr_eq(&anchor.anchors, &ours) {
            return None;
        }
        anchor.selection()
    }

    /// Whether an image can be inserted over `selection`: the dry run of
    /// [`insert_image`](Self::insert_image) there. Not whether the editor
    /// accepts edits at all ([`refuses_edits`](Self::refuses_edits)).
    fn takes_image_at(&self, selection: &Selection) -> bool {
        let cmd = rinch_editor_core::commands::insert_image(String::new(), String::new());
        let core = self.core();
        let mut aimed = core.state.clone();
        aimed.selection = selection.clone();
        aimed.command_applies(&cmd)
    }

    /// Insert an image at `anchor`: [`insert_image`](Self::insert_image) at the
    /// place a selection captured earlier has moved to, the completion half of
    /// an [`on_image_input`](Self::on_image_input) callback that could not
    /// answer at once. One transaction, one undo step.
    ///
    /// **The person's caret stays theirs.** If the selection is still the
    /// anchor's (nobody moved since the picture was aimed), it ends up after
    /// the image, as after [`insert_image`](Self::insert_image). If the person
    /// has moved on, their selection is kept where it is in the text (shifted
    /// by the image when that lands before it), and nothing scrolls: the next
    /// letter they type lands where they are, not beside the picture.
    ///
    /// Returns `false`, changing neither the document nor the selection, when
    /// the anchor no longer names a place (its document was replaced, or a
    /// collaborating peer removed or restructured the block it was in; see
    /// [`SelectionAnchor::selection`]), when it was taken from a different
    /// editor, or when the insertion is refused (a read-only editor, a
    /// position that takes no image). An app that stored the picture's bytes
    /// for this insertion should delete them on `false`: nothing references
    /// them.
    pub fn insert_image_at(&self, anchor: &SelectionAnchor, src: &str, alt: &str) -> bool {
        let Some(at) = self.own_anchor_selection(anchor) else {
            return false;
        };
        let cmd = rinch_editor_core::commands::insert_image(src.to_string(), alt.to_string());
        let mut core = self.core_mut();
        let prev = core.state.clone();
        // The command replaces the selection, so run it on the current state
        // with the anchor's selection in place of the live one.
        let moved = prev.selection != at;
        let aimed = if moved {
            // Through a transaction, not by writing the field: an explicit
            // selection ends the history's typing group, so a picture that
            // arrives while the person types is an undo step of its own and
            // not part of the word they are in.
            let mut tr = prev.tr();
            tr.set_selection(at);
            prev.apply(tr)
        } else {
            prev.clone()
        };
        let Some((mut next, mapping)) = run_stamped(&aimed, &cmd) else {
            return false;
        };
        if moved {
            // Put the person's selection back, where it is in the text now.
            // Again a transaction: what they type next is not grouped with the
            // picture either.
            let live = prev.selection.map(&next.doc, &mapping);
            let mut tr = next.tr();
            tr.set_selection(live);
            next = next.apply(tr);
            next.stored_marks = prev.stored_marks.clone();
        }
        // The person's own selection did not change, so there is nothing of
        // theirs to bring into view.
        let scroll = if moved { Scroll::No } else { Scroll::IfChanged };
        let Some(doc_changed) = core.commit(prev, next, Some(&mapping), scroll, Vec::new()) else {
            return false;
        };
        drop(core);
        if doc_changed {
            self.notify_change();
        }
        true
    }

    /// Whether `html` parses, under this editor's schema, to **pictures and
    /// nothing else**: one or more `image` nodes, no text, no other leaf. What
    /// a browser's "Copy image" puts on the clipboard as html, beside the
    /// bitmap itself; a platform runtime asks so that such a paste is offered
    /// to [`on_image_input`](Self::on_image_input) as the bitmap rather than
    /// pasted as an `<img>` pointing wherever it was copied from.
    pub fn html_is_only_images(&self, html: &str) -> bool {
        let schema = self.core().schema.clone();
        let Ok(slice) = slice_from_html(&schema, html) else {
            return false;
        };
        let mut images = 0usize;
        let mut other = false;
        fn walk(node: &Node, images: &mut usize, other: &mut bool) {
            if node.type_name() == "image" {
                *images += 1;
            } else if node.type_name() == "hard_break" {
                // A line break beside the picture (Safari, some mail clients
                // write `<img><br>`) is not words.
            } else if node.is_text() || node.is_atom() {
                *other = true;
            } else {
                for i in 0..node.child_count() {
                    walk(node.child(i), images, other);
                }
            }
        }
        for i in 0..slice.content.child_count() {
            walk(slice.content.child(i), &mut images, &mut other);
        }
        images > 0 && !other
    }
}
