//! Pictures arriving from outside the document: what an app hears when the
//! person pastes an image or drops an image file on an editor.
//!
//! The editor does not decide where a picture's bytes live. An app that keeps
//! them itself (a blob store, an upload) registers
//! [`EditorHandle::on_image_input`](crate::EditorHandle::on_image_input), is
//! handed the bytes, stores them, and answers with the `src` the document should
//! carry. Only that `src` enters the document.
//!
//! And pictures already in it: which source an image's `<img>` asks the
//! loader for ([`set_image_source`]), and the image the pointer is over
//! ([`EditorHandle::on_image_hover`](crate::EditorHandle::on_image_hover)),
//! so an app can float controls over a picture.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::reactive::ElementBounds;
use rinch_editor_core::{Attrs, Pos};

use crate::SelectionAnchor;

/// What [`set_image_source`] installs.
type ImageSourceFn = Rc<dyn Fn(&Attrs) -> Option<String>>;

thread_local! {
    static IMAGE_SOURCE: RefCell<Option<ImageSourceFn>> = const { RefCell::new(None) };
}

/// Decide which source the `<img>` of an `image` node asks for, from the
/// node's attributes, for every editor on this thread. `None` keeps the
/// node's own `src`, which is also what happens with nothing installed.
///
/// The document is untouched: the node keeps its `src`, and copy, paste,
/// export and collaboration see it. Only the picture shown is chosen here.
/// What the `<img>` is given is what the image loader is asked for
/// (`rinch::image::register_image_scheme`, rinch-web's
/// `register_image_url_scheme`) and what `rinch::image::reload_image` names,
/// so two images with one `src` and different attrs can be two pictures,
/// loaded and reloaded on their own. An app that draws over a picture says,
/// for example,
///
/// ```ignore
/// rinch::editor::set_image_source(|attrs| {
///     let board = attrs.get_str("board").filter(|b| !b.is_empty())?;
///     Some(format!("{}#board={board}", attrs.get_str("src")?))
/// });
/// // … and when that board changes:
/// rinch::image::reload_image(&format!("{src}#board={board}"));
/// ```
///
/// and its loader answers `…#board=…` with the picture drawn over.
///
/// It is asked whenever an image's `<img>` is built or its attributes change,
/// with the editor's state borrowed: it must be a function of `attrs` alone
/// (answer the same source for the same attributes) and must not touch an
/// editor or the DOM. Installing it again replaces the earlier one; images
/// already shown keep the source they were given until they are rebuilt or
/// their attributes change, so install it before the first editor shows a
/// picture. To make images already shown ask again, reload their documents
/// (`handle.load_doc(handle.doc())`).
///
/// **Its lifetime is the registering component's.** Installed while a
/// component renders, it is removed when that component unmounts (unless a
/// later install has replaced it — an earlier unmount never clobbers a later
/// one); installed outside any render (from `main`), it keeps app lifetime.
/// It is **per thread, not per document**: two documents on one thread (two
/// embedded contexts, a window and its DevTools) share one source function,
/// and the last install wins for both.
pub fn set_image_source(source: impl Fn(&Attrs) -> Option<String> + 'static) {
    let source: ImageSourceFn = Rc::new(source);
    rinch_core::reactive::install_scoped_slot(&IMAGE_SOURCE, source);
}

/// Remove what [`set_image_source`] installed: an image's `<img>` asks for
/// its node's `src` again (from its next rebuild or attribute change).
pub fn clear_image_source() {
    rinch_core::reactive::clear_scoped_slot(&IMAGE_SOURCE);
}

/// The source an `image` node with `attrs` shows: [`set_image_source`]'s
/// answer, else its `src`.
pub(crate) fn image_source(attrs: &Attrs) -> String {
    // Cloned out, so a source function that installs another is not inside
    // the borrow.
    let source = rinch_core::reactive::read_scoped_slot(&IMAGE_SOURCE);
    source
        .and_then(|f| f(attrs))
        .unwrap_or_else(|| attrs.get_str("src").unwrap_or("").to_string())
}

/// The image the pointer is over — see
/// [`EditorHandle::on_image_hover`](crate::EditorHandle::on_image_hover).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageHover {
    /// The position before the image: where a
    /// `SetNodeAttrStep` for it goes, and what
    /// [`Selection::node_at`](rinch_editor_core::Selection::node_at) selects.
    pub pos: Pos,
    /// The image's attributes (`src`, `alt`, and whatever else its node
    /// carries).
    pub attrs: Attrs,
    /// The box the `<img>` is painted in, measured on the pointer move that
    /// reported it, in the frame an app positions an overlay in: logical
    /// window pixels on desktop (the frame of
    /// [`NodeHandle::bounds_signal`](rinch_core::dom::NodeHandle::bounds_signal)
    /// and of a `position: fixed` overlay at the root), viewport client pixels
    /// in the browser (`getBoundingClientRect`).
    pub rect: ElementBounds,
}

/// How a picture reached the editor.
///
/// `#[non_exhaustive]`: another route (a pasted file list, say) can be added
/// without breaking an app, so a `match` on it outside this crate needs a
/// wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageInputSource {
    /// Pasted: the clipboard held a bitmap.
    Paste,
    /// Dropped: an image file was dropped on the editor.
    Drop,
}

/// A picture the person pasted into or dropped on an editor, offered to the
/// app — see [`EditorHandle::on_image_input`](crate::EditorHandle::on_image_input).
///
/// `#[non_exhaustive]`: a field can be added (the picture's pixel size, say)
/// without breaking an app. Outside this crate it is read by field or
/// destructured with `..`, and cannot be built with a struct literal; the
/// editor is what makes one.
#[derive(Debug)]
#[non_exhaustive]
pub struct ImageInput {
    /// How it arrived.
    pub source: ImageInputSource,
    /// The picture's **encoded** bytes: a file's contents, as they would sit on
    /// disk. On desktop a pasted bitmap is encoded as PNG first; a dropped
    /// file's bytes, and in the browser a pasted file's, are the file's,
    /// untouched.
    pub bytes: Vec<u8>,
    /// The media type of `bytes`: what their first bytes say they are
    /// (`image/png`, `image/jpeg`, `image/gif`, `image/webp`), else, in the
    /// browser, what the browser called the file (`image/svg+xml`, …).
    pub mime: String,
    /// The file's name (`holiday.jpg`), without its directory. On desktop
    /// `None` for a paste: a clipboard bitmap has no name. The browser names
    /// every pasted file (`image.png` for a bitmap).
    pub name: Option<String>,
    /// Where the picture goes: the selection at the paste, or the caret at the
    /// drop point, **anchored**, so it still names the same place after the
    /// person types on. An app that cannot answer at once keeps this and calls
    /// [`EditorHandle::insert_image_at`](crate::EditorHandle::insert_image_at)
    /// with it when the picture is stored.
    pub anchor: SelectionAnchor,
}

/// The media type of an encoded image, from its first bytes: the four formats
/// the built-in image pipeline decodes. `None` for anything else, whatever its
/// file name says.
pub fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::sniff_image_mime;

    #[test]
    fn the_four_decodable_formats_are_told_apart_by_their_first_bytes() {
        assert_eq!(
            sniff_image_mime(b"\x89PNG\r\n\x1a\n...."),
            Some("image/png")
        );
        assert_eq!(
            sniff_image_mime(b"\xff\xd8\xff\xe0...."),
            Some("image/jpeg")
        );
        assert_eq!(sniff_image_mime(b"GIF89a...."), Some("image/gif"));
        assert_eq!(
            sniff_image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_image_mime(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(sniff_image_mime(b"<svg xmlns="), None);
        assert_eq!(sniff_image_mime(b""), None);
    }

    use super::ImageHover;
    use crate::create_editor;
    use crate::registry::{image_hover_wanted, set_image_hover};
    use rinch_core::reactive::ElementBounds;
    use rinch_editor_core::{AttrValue, Attrs, Pos};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hover(pos: usize, alt: &str, x: f32) -> ImageHover {
        ImageHover {
            pos: Pos(pos),
            attrs: Attrs::new()
                .with("src", AttrValue::from("x.png"))
                .with("alt", AttrValue::from(alt)),
            rect: ElementBounds {
                x,
                y: 10.0,
                width: 40.0,
                height: 30.0,
            },
        }
    }

    type Seen = Rc<RefCell<Vec<Option<(usize, String, f32)>>>>;

    fn record(h: &crate::EditorHandle) -> Seen {
        let seen: Seen = Rc::default();
        let seen_in = seen.clone();
        h.on_image_hover(move |hover| {
            seen_in.borrow_mut().push(hover.map(|h| {
                (
                    h.pos.0,
                    h.attrs.get_str("alt").unwrap_or("").to_string(),
                    h.rect.x,
                )
            }))
        });
        seen
    }

    #[test]
    fn image_hover_fires_on_enter_change_and_leave_only() {
        let h = create_editor();
        assert!(!image_hover_wanted(), "no callback yet");
        let seen = record(&h);
        assert!(image_hover_wanted());
        let doc = Some(7);
        set_image_hover(doc, Some((h.clone(), hover(3, "a", 100.0))));
        set_image_hover(doc, Some((h.clone(), hover(3, "a", 100.0))));
        assert_eq!(seen.borrow().len(), 1, "resting on one image fires once");
        // The same image, edited or moved (a scroll): reported again.
        set_image_hover(doc, Some((h.clone(), hover(3, "b", 100.0))));
        set_image_hover(doc, Some((h.clone(), hover(3, "b", 80.0))));
        // Another image of the same editor: only the new `Some`.
        set_image_hover(doc, Some((h.clone(), hover(9, "c", 300.0))));
        set_image_hover(doc, None);
        set_image_hover(doc, None);
        assert_eq!(
            *seen.borrow(),
            vec![
                Some((3, "a".into(), 100.0)),
                Some((3, "b".into(), 100.0)),
                Some((3, "b".into(), 80.0)),
                Some((9, "c".into(), 300.0)),
                None,
            ]
        );
        drop(h);
        assert!(!image_hover_wanted(), "gone with the editor");
    }

    #[test]
    fn moving_between_editors_leaves_one_images_and_enters_the_others() {
        let (a, b) = (create_editor(), create_editor());
        let (seen_a, seen_b) = (record(&a), record(&b));
        set_image_hover(None, Some((a.clone(), hover(3, "a", 0.0))));
        set_image_hover(None, Some((b.clone(), hover(3, "a", 0.0))));
        assert_eq!(*seen_a.borrow(), vec![Some((3, "a".into(), 0.0)), None]);
        assert_eq!(*seen_b.borrow(), vec![Some((3, "a".into(), 0.0))]);
        set_image_hover(None, None);
    }

    /// Unmounting an editor while one of its images is hovered forgets the
    /// hover without calling the editor back, as for links.
    #[test]
    fn unregistering_a_hovered_editor_forgets_its_image_hover_without_a_call() {
        use crate::registry::{register_editor, unregister_editor};
        let h = create_editor();
        let seen = record(&h);
        register_editor(41, 9, h.clone());
        set_image_hover(Some(41), Some((h.clone(), hover(3, "a", 0.0))));
        unregister_editor(41, 9);
        drop(h);
        assert!(!image_hover_wanted());
        set_image_hover(Some(41), None);
        assert_eq!(seen.borrow().len(), 1, "no `None` for the unmounted editor");
    }

    #[test]
    fn the_image_source_is_the_apps_answer_else_the_src() {
        use super::{clear_image_source, image_source, set_image_source};
        let plain = Attrs::new().with("src", AttrValue::from("pimble-blob:s/b"));
        let titled = plain.clone().with("title", AttrValue::from("t1"));
        assert_eq!(
            image_source(&titled),
            "pimble-blob:s/b",
            "nothing installed"
        );
        set_image_source(|attrs| {
            let title = attrs.get_str("title").filter(|t| !t.is_empty())?;
            Some(format!("{}#{title}", attrs.get_str("src")?))
        });
        assert_eq!(image_source(&titled), "pimble-blob:s/b#t1");
        assert_eq!(
            image_source(&plain),
            "pimble-blob:s/b",
            "`None` keeps the src"
        );
        // A source function that installs another is not inside a borrow.
        set_image_source(|_| {
            set_image_source(|_| Some("second".into()));
            Some("first".into())
        });
        assert_eq!(image_source(&plain), "first");
        assert_eq!(image_source(&plain), "second");
        clear_image_source();
        assert_eq!(image_source(&titled), "pimble-blob:s/b");
    }
}
