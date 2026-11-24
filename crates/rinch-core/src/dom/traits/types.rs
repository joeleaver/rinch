//! The value types and constants the [`DomDocument`](super::DomDocument)
//! methods take and return, and [`next_doc_key`].

use crate::dom::NodeId;

/// A stretch of a text-bearing element's flat byte offsets that holds CSS
/// generated content, `start..end`
/// ([`DomLayout::generated_flat_ranges`](super::DomLayout::generated_flat_ranges), #891). `after` when it is an
/// `::after`'s text, which belongs after the content before it: a caret at
/// `start` is drawn before it. A `::before`'s text belongs before the content
/// after it, so a caret at its `start` is drawn past it, at `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeneratedFlatRange {
    /// The first byte of the generated text.
    pub start: usize,
    /// The byte after it.
    pub end: usize,
    /// An `::after`'s text (else a `::before`'s, or a list marker's).
    pub after: bool,
}

/// Which side of a soft line wrap a text caret belongs to (#301).
///
/// The end of one visual line and the start of the next are one text position,
/// so a caret there could be drawn in either place. `Downstream` — the default —
/// draws it at the start of the lower line; `Upstream` at the end of the upper
/// one. Anywhere but a soft wrap the two are the same caret. The rich-text
/// editor keeps the hint beside its selection
/// (`EditorHandle::caret_affinity`); Parley calls the same thing
/// `parley::Affinity`, CodeMirror `SelectionRange.assoc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum CaretAffinity {
    /// At a soft wrap, the start of the lower line.
    #[default]
    Downstream,
    /// At a soft wrap, the end of the upper line.
    Upstream,
}

/// The width, in CSS px, of the text caret the rich-text editor draws (its
/// overlay). The editor keeps that whole width inside the textblock's border
/// box when spaces hang past a line's end (#1041), as a browser keeps its own
/// 1px caret.
pub const CARET_WIDTH: f32 = 2.0;

/// The text a highlight box covers, for [`DomContent::set_highlight_text`](super::DomContent::set_highlight_text)
/// (#2667): the flat UTF-8 byte range `[start, end)` of the textblock host
/// `block`, plus the zero-byte inline leaves (images) the range holds, as
/// [`DomLayout::query_selection_rects_with_boxes`](super::DomLayout::query_selection_rects_with_boxes) takes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightText<'a> {
    /// The textblock's host node.
    pub block: NodeId,
    /// The range's first byte in the block's flat text.
    pub start: usize,
    /// One past the range's last byte.
    pub end: usize,
    /// The host ids of the selected zero-byte leaves.
    pub boxes: &'a [u64],
}

/// Bounding box for a glyph cluster.
#[derive(Debug, Clone, Copy)]
pub struct GlyphBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The resolved font of a node, as CSS-ready strings — used by the IME preedit
/// overlay so the composing text matches the text it composes into (a large
/// heading vs. body text). Each field is a CSS value (e.g. `size = "32px"`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeFont {
    pub family: String,
    pub size: String,
    pub weight: String,
    pub style: String,
}

/// What [`DomFocus::focus_into`](super::DomFocus::focus_into) should do when the subtree has no
/// `autofocus` descendant (issue #695).
///
/// The two values are the two things browsers actually do, and rinch's three
/// overlays are split between them: a `<dialog>` opened with `showModal()`
/// focuses its first focusable whether or not anything asked for it, while an
/// `auto` popover moves focus **only** for an `autofocus` element and otherwise
/// leaves the keyboard where it is. `Modal` and `Drawer` take the first;
/// `Popover` takes the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusIntoPolicy {
    /// Fall back to the first focusable descendant — a modal dialog.
    FirstFocusable,
    /// Move nothing unless something inside asked for focus — a popover.
    AutofocusOnly,
}

/// The direction a text selection extends in — [`DomFocus::set_selection_range`](super::DomFocus::set_selection_range)'s
/// fourth argument, matching the DOM's `HTMLInputElement.setSelectionRange(start,
/// end, direction)` exactly (issue #552): `"forward"`, `"backward"` and
/// `"none"` are literally these three variants, lowercased.
///
/// `Forward` and `Backward` decide which end is the *anchor* (fixed) and which
/// is the *head* (what Shift+Arrow would move from) — not merely how the
/// range paints, since a control's own keyboard handling continues from the
/// head. `None` — "unspecified", a browser's own default when the third
/// argument is omitted — is applied the same way [`Forward`](Self::Forward)
/// is: it selects the same text, and the two differ only in what
/// `selectionDirection` would read back, which rinch does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionDirection {
    /// The anchor is `start`, the head is `end`.
    #[default]
    Forward,
    /// The anchor is `end`, the head is `start`.
    Backward,
    /// Unspecified, as when a browser's `setSelectionRange` is called with no
    /// third argument. Applied the same way [`Forward`](Self::Forward) is.
    None,
}

/// Allocate a fresh process-unique document key for [`DomTree::doc_key`](super::DomTree::doc_key).
///
/// Call once per document at construction and store the result. Monotonic and
/// never reused, so it is immune to allocator address reuse (unlike keying by
/// `Rc` pointer).
pub fn next_doc_key() -> u64 {
    static NEXT_DOC_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_DOC_KEY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
