//! Pictures arriving from outside the document: what an app hears when the
//! person pastes an image or drops an image file on an editor.
//!
//! The editor does not decide where a picture's bytes live. An app that keeps
//! them itself (a blob store, an upload) registers
//! [`EditorHandle::on_image_input`](crate::EditorHandle::on_image_input), is
//! handed the bytes, stores them, and answers with the `src` the document should
//! carry. Only that `src` enters the document.

use crate::SelectionAnchor;

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
}
