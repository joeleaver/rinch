//! Plain-text clipboard interchange for [`Slice`]s.
//!
//! Plain text is the lossy fall-back the rich [`super::html`] path complements:
//! [`slice_to_text`] is the `text/plain` alternative placed alongside `text/html`
//! on copy, and [`slice_from_text`] turns an OS plain-text paste (which has no
//! markup) into paragraph-per-line content for insertion. Block boundaries map to
//! newlines and vice-versa; inline marks are dropped (there is nowhere to put them
//! in plain text — lossy by design, like the markdown path).

use crate::EditorError;
use crate::model::{Fragment, Node, Slice};
use crate::schema::Schema;

/// Serialize a [`Slice`]'s content to plain text — the `text/plain` clipboard
/// alternative. Block boundaries become newlines; `hard_break`s become newlines;
/// inline runs (a within-block copy) concatenate with no separator; other leaves
/// contribute nothing.
pub fn slice_to_text(slice: &Slice) -> String {
    let mut out = String::new();
    write_fragment(slice.content.children(), &mut out);
    out
}

/// Write a fragment's children, inserting a newline only across a block boundary
/// (so inline runs concatenate but adjacent blocks are line-separated).
fn write_fragment(children: &[Node], out: &mut String) {
    let mut prev_block = false;
    for (i, child) in children.iter().enumerate() {
        let is_block = child.is_block();
        if i > 0 && (is_block || prev_block) {
            out.push('\n');
        }
        write_text(child, out);
        prev_block = is_block;
    }
}

/// Append `node`'s plain-text content to `out`. Textblocks concatenate their
/// inline text; block containers newline-separate their block children; a
/// `hard_break` is a newline; other leaves are empty.
fn write_text(node: &Node, out: &mut String) {
    if let Some(text) = node.text() {
        out.push_str(text);
        return;
    }
    if node.is_leaf() {
        if node.type_name() == "hard_break" {
            out.push('\n');
        }
        return;
    }
    write_fragment(node.content().children(), out);
}

/// Parse plain text into a [`Slice`] of one paragraph per line (an empty line →
/// an empty paragraph) — used for an OS plain-text paste. The first and last
/// blocks are "open" (`open_start == open_end == 1`), matching [`super::html`]'s
/// heuristic, so a single-line paste merges inline at the cursor and a multi-line
/// paste splits the surrounding block.
pub fn slice_from_text(schema: &Schema, text: &str) -> Result<Slice, EditorError> {
    if text.is_empty() {
        return Ok(Slice::empty());
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    // Drop a single trailing newline: copying a whole line usually carries its
    // terminating '\n', and pasting that should not append an empty paragraph.
    // Interior and leading blank lines are preserved.
    let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let mut blocks = Vec::new();
    for line in body.split('\n') {
        let content = if line.is_empty() {
            Fragment::empty()
        } else {
            Fragment::from_node(schema.text(line)?)
        };
        blocks.push(schema.branch("paragraph", content)?);
    }
    Ok(Slice::new(Fragment::from_children(blocks), 1, 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fragment, Mark};
    use crate::schema::Schema;

    fn schema() -> Schema {
        Schema::starter_kit()
    }

    fn para(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }

    #[test]
    fn within_block_slice_is_bare_text() {
        // A cursor-range copy inside one paragraph yields bare inline runs.
        let s = schema();
        let doc = s
            .branch("doc", Fragment::from_node(para(&s, "hello world")))
            .unwrap();
        let slice = doc.slice(2, 7).unwrap(); // "ello "
        assert_eq!(slice_to_text(&slice), "ello ");
    }

    #[test]
    fn multi_block_slice_is_newline_separated() {
        let s = schema();
        let doc = s
            .branch(
                "doc",
                Fragment::from_children(vec![para(&s, "one"), para(&s, "two")]),
            )
            .unwrap();
        // Whole document content: 0[p 1 one 4]5[p 6 two 9]10.
        let slice = doc.slice(1, 9).unwrap();
        assert_eq!(slice_to_text(&slice), "one\ntwo");
    }

    #[test]
    fn hard_break_becomes_newline() {
        let s = schema();
        let hb = s
            .create_node("hard_break", Default::default(), Fragment::empty())
            .unwrap();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![s.text("a").unwrap(), hb, s.text("b").unwrap()]),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(p)).unwrap();
        let slice = doc.slice(1, 4).unwrap(); // a, br, b
        assert_eq!(slice_to_text(&slice), "a\nb");
    }

    #[test]
    fn from_text_single_line_open_paragraph() {
        let s = schema();
        let slice = slice_from_text(&s, "hello").unwrap();
        assert_eq!(slice.content.child_count(), 1);
        assert_eq!(slice.open_start, 1);
        assert_eq!(slice.open_end, 1);
        assert_eq!(slice.content.child(0).type_name(), "paragraph");
        assert_eq!(slice_to_text(&slice), "hello");
    }

    #[test]
    fn from_text_multiline_splits_paragraphs_crlf() {
        let s = schema();
        let slice = slice_from_text(&s, "a\r\nb\r\nc").unwrap();
        assert_eq!(slice.content.child_count(), 3);
        assert_eq!(slice_to_text(&slice), "a\nb\nc");
    }

    #[test]
    fn from_text_blank_line_is_empty_paragraph() {
        let s = schema();
        let slice = slice_from_text(&s, "a\n\nb").unwrap();
        assert_eq!(slice.content.child_count(), 3);
        assert_eq!(
            slice.content.child(1).child_count(),
            0,
            "middle paragraph is empty"
        );
    }

    #[test]
    fn from_text_strips_single_trailing_newline() {
        let s = schema();
        // Copying a whole line carries its terminating newline — don't append an
        // empty paragraph for it.
        let one = slice_from_text(&s, "line\n").unwrap();
        assert_eq!(
            one.content.child_count(),
            1,
            "single trailing newline trimmed"
        );
        assert_eq!(slice_to_text(&one), "line");
        // Only ONE trailing newline is dropped — a deliberate blank line survives.
        let two = slice_from_text(&s, "line\n\n").unwrap();
        assert_eq!(two.content.child_count(), 2);
        assert_eq!(
            two.content.child(1).child_count(),
            0,
            "trailing blank line kept"
        );
    }

    #[test]
    fn empty_text_is_empty_slice() {
        let s = schema();
        let slice = slice_from_text(&s, "").unwrap();
        assert_eq!(slice, Slice::empty());
    }

    #[test]
    fn round_trip_through_text_preserves_lines() {
        let s = schema();
        let original = "first line\nsecond line";
        let slice = slice_from_text(&s, original).unwrap();
        assert_eq!(slice_to_text(&slice), original);
    }

    #[test]
    fn marks_are_dropped_in_plain_text() {
        let s = schema();
        let bold = Mark::new(s.mark_type("bold").unwrap().clone(), Default::default());
        let p = s
            .branch(
                "paragraph",
                Fragment::from_node(s.text_with_marks("bold", vec![bold]).unwrap()),
            )
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(p)).unwrap();
        let slice = doc.slice(1, 5).unwrap();
        assert_eq!(slice_to_text(&slice), "bold");
    }
}
