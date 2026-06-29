//! Renderer-agnostic caret motion and word/block selection over the model.
//!
//! These are the pure-model primitives behind arrow-key movement and double/triple
//! click selection: they read only the document and a [`Pos`], never any layout or
//! host geometry. Vertical motion (line up/down) and *visual* line Home/End are
//! deliberately **not** here — they require the renderer's laid-out geometry and so
//! live in each platform's input glue (`rinch` for desktop, `rinch-web` for the
//! browser). Everything in this module is shared by both.

use crate::model::Node;
use crate::pos::Pos;
use crate::selection::Selection;

/// A model-resolvable caret motion (the horizontal / document / model-line motions).
/// Vertical motion is intentionally absent — it depends on laid-out geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMotion {
    /// One character/atom left.
    CharLeft,
    /// One character/atom right.
    CharRight,
    /// To the previous word boundary in the textblock.
    WordLeft,
    /// To the next word boundary in the textblock.
    WordRight,
    /// To the start of the current textblock (model line; visual-line Home is a
    /// platform refinement).
    LineStart,
    /// To the end of the current textblock (model line; visual-line End is a
    /// platform refinement).
    LineEnd,
    /// To the start of the document.
    DocStart,
    /// To the end of the document.
    DocEnd,
}

/// Resolve a model cursor `motion` from `head` (with selection `anchor`) into the
/// resulting [`Selection`], or `None` if the motion has no target (e.g. a word
/// motion outside a textblock).
///
/// When `extend` is false a [`CursorMotion::CharLeft`]/[`CursorMotion::CharRight`]
/// that lands on a selectable leaf atom (an image / horizontal rule) returns a
/// [`Selection::Node`] (the caret "selects" the atom); otherwise the result is a
/// collapsed [`Selection::cursor`] (no extend) or an extended [`Selection::text`]
/// from `anchor` (extend).
pub fn resolve_cursor_motion(
    doc: &Node,
    head: Pos,
    anchor: Pos,
    motion: CursorMotion,
    extend: bool,
) -> Option<Selection> {
    let new_head: Pos = match motion {
        CursorMotion::CharLeft => {
            let near = Selection::near(doc, Pos(head.0.saturating_sub(1)), -1);
            // A char move onto a selectable block atom (hr/image) becomes a node
            // selection when collapsing (not when extending a text range).
            if !extend && matches!(near, Selection::Node(_)) {
                return Some(near);
            }
            near.head()
        }
        CursorMotion::CharRight => {
            let near = Selection::near(doc, Pos((head.0 + 1).min(doc.content_size())), 1);
            if !extend && matches!(near, Selection::Node(_)) {
                return Some(near);
            }
            near.head()
        }
        CursorMotion::WordLeft => word_boundary(doc, head, false)?,
        CursorMotion::WordRight => word_boundary(doc, head, true)?,
        CursorMotion::LineStart => line_boundary(doc, head, false)?,
        CursorMotion::LineEnd => line_boundary(doc, head, true)?,
        CursorMotion::DocStart => Selection::at_start(doc).head(),
        CursorMotion::DocEnd => Selection::at_end(doc).head(),
    };
    Some(if extend {
        Selection::text(anchor, new_head)
    } else {
        Selection::cursor(new_head)
    })
}

/// The model position at the start (`forward=false`) or end (`forward=true`) of the
/// textblock `head` is in. (Visual-line Home/End for wrapped lines is a platform
/// refinement; this is block start/end.) `None` if `head` is not in a textblock.
pub fn line_boundary(doc: &Node, head: Pos, forward: bool) -> Option<Pos> {
    let r = doc.resolve(head).ok()?;
    if !r.parent().is_textblock() {
        return None;
    }
    let content_start = head.0 - r.parent_offset();
    Some(Pos(if forward {
        content_start + r.parent().content().size()
    } else {
        content_start
    }))
}

/// The model position at the previous/next word boundary within `head`'s textblock
/// (a simple whitespace/word-character scan; cross-block word motion is a later
/// refinement). `None` if `head` is not in a textblock.
pub fn word_boundary(doc: &Node, head: Pos, forward: bool) -> Option<Pos> {
    let r = doc.resolve(head).ok()?;
    if !r.parent().is_textblock() {
        return None;
    }
    let chars: Vec<char> = block_text(r.parent()).chars().collect();
    let content_start = head.0 - r.parent_offset();
    let mut i = r.parent_offset();
    if forward {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
    } else {
        i = i.saturating_sub(1);
        while i > 0 && chars[i].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
    }
    Some(Pos(content_start + i))
}

/// The concatenated inline text of a textblock, with each non-text leaf counted as
/// one placeholder char (so word/line offsets line up with the position space).
pub fn block_text(block: &Node) -> String {
    let mut s = String::new();
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(t) => s.push_str(t),
            None => s.push(' '),
        }
    }
    s
}

/// The model word range `(start, end)` around `pos` — the contiguous run of
/// like-classed characters (word vs. whitespace) under a double-click, classified
/// by the character to the right of the gap (or the last character at block end).
/// Collapsed `(pos, pos)` outside a textblock or in an empty block.
pub fn word_range_at(doc: &Node, pos: Pos) -> (Pos, Pos) {
    let Ok(r) = doc.resolve(pos) else {
        return (pos, pos);
    };
    if !r.parent().is_textblock() {
        return (pos, pos);
    }
    let chars: Vec<char> = block_text(r.parent()).chars().collect();
    if chars.is_empty() {
        return (pos, pos);
    }
    let content_start = pos.0 - r.parent_offset();
    let off = r.parent_offset().min(chars.len());
    let is_word_char = |c: char| !c.is_whitespace();
    // Classify by the char right of the gap, but prefer the word at a word's
    // trailing edge (right char whitespace, left char a word char).
    let classify_right = off < chars.len()
        && (is_word_char(chars[off]) || off == 0 || !is_word_char(chars[off - 1]));
    let class_idx = if classify_right { off } else { off - 1 };
    let target = is_word_char(chars[class_idx]);
    let mut start = off;
    while start > 0 && is_word_char(chars[start - 1]) == target {
        start -= 1;
    }
    let mut end = off;
    while end < chars.len() && is_word_char(chars[end]) == target {
        end += 1;
    }
    (Pos(content_start + start), Pos(content_start + end))
}

/// The model range `(start, end)` spanning the whole textblock `pos` is in (a
/// triple-click). Collapsed `(pos, pos)` outside a textblock.
pub fn block_range_at(doc: &Node, pos: Pos) -> (Pos, Pos) {
    let Ok(r) = doc.resolve(pos) else {
        return (pos, pos);
    };
    if !r.parent().is_textblock() {
        return (pos, pos);
    }
    let content_start = pos.0 - r.parent_offset();
    let content_end = content_start + r.parent().content().size();
    (Pos(content_start), Pos(content_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Fragment;
    use crate::schema::Schema;

    fn paragraph_doc(text: &str) -> Node {
        let s = Schema::starter_kit();
        let p = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        s.branch("doc", Fragment::from_node(p)).unwrap()
    }

    #[test]
    fn word_range_selects_word_under_pos() {
        let doc = paragraph_doc("hello world");
        // Positions: 0[p 1 'hello world'(11) 12]13.
        assert_eq!(
            word_range_at(&doc, Pos(3)),
            (Pos(1), Pos(6)),
            "inside 'hello'"
        );
        assert_eq!(
            word_range_at(&doc, Pos(9)),
            (Pos(7), Pos(12)),
            "inside 'world'"
        );
    }

    #[test]
    fn word_range_at_block_end_selects_last_word() {
        let doc = paragraph_doc("hello world");
        // The gap at block end classifies by the last char ('d').
        assert_eq!(word_range_at(&doc, Pos(12)), (Pos(7), Pos(12)));
    }

    #[test]
    fn word_range_on_whitespace_selects_whitespace_run() {
        let doc = paragraph_doc("a  b");
        // chars a,_,_,b — a click *between* the two spaces (both neighbours
        // whitespace) selects the run of spaces.
        assert_eq!(word_range_at(&doc, Pos(3)), (Pos(2), Pos(4)));
    }

    #[test]
    fn word_range_at_word_trailing_edge_selects_the_word() {
        let doc = paragraph_doc("a  b");
        // A click in the gap just *after* 'a' (right char whitespace, left char a
        // word char) selects the word 'a', not the following space.
        assert_eq!(word_range_at(&doc, Pos(2)), (Pos(1), Pos(2)));
        // And the gap just before 'b' selects 'b'.
        assert_eq!(word_range_at(&doc, Pos(4)), (Pos(4), Pos(5)));
    }

    #[test]
    fn word_range_empty_block_is_collapsed() {
        let s = Schema::starter_kit();
        let doc = s
            .branch(
                "doc",
                Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
            )
            .unwrap();
        assert_eq!(word_range_at(&doc, Pos(1)), (Pos(1), Pos(1)));
    }

    #[test]
    fn block_range_selects_whole_textblock() {
        let doc = paragraph_doc("hello world");
        assert_eq!(block_range_at(&doc, Pos(3)), (Pos(1), Pos(12)));
        assert_eq!(block_range_at(&doc, Pos(8)), (Pos(1), Pos(12)));
    }

    #[test]
    fn block_range_targets_the_clicked_block() {
        let s = Schema::starter_kit();
        let p1 = s
            .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
            .unwrap();
        let p2 = s
            .branch("paragraph", Fragment::from_node(s.text("cdef").unwrap()))
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![p1, p2]))
            .unwrap();
        // Positions: 0[p 1 ab 3]4[p 5 cdef 9]10 — second block content is [5, 9).
        assert_eq!(block_range_at(&doc, Pos(7)), (Pos(5), Pos(9)));
    }

    #[test]
    fn word_motion_steps_over_words() {
        let doc = paragraph_doc("hello world");
        // From inside 'hello', WordRight lands at the end of 'hello' (Pos 6), then
        // the end of 'world' (Pos 12).
        assert_eq!(word_boundary(&doc, Pos(3), true), Some(Pos(6)));
        assert_eq!(word_boundary(&doc, Pos(6), true), Some(Pos(12)));
        // WordLeft from inside 'world' lands at its start (Pos 7), then 'hello' (1).
        assert_eq!(word_boundary(&doc, Pos(9), false), Some(Pos(7)));
        assert_eq!(word_boundary(&doc, Pos(7), false), Some(Pos(1)));
    }

    #[test]
    fn line_boundary_is_block_start_and_end() {
        let doc = paragraph_doc("hello world");
        assert_eq!(line_boundary(&doc, Pos(5), false), Some(Pos(1)));
        assert_eq!(line_boundary(&doc, Pos(5), true), Some(Pos(12)));
    }

    #[test]
    fn cursor_motion_collapses_and_extends() {
        let doc = paragraph_doc("hello world");
        // Collapse: CharRight from Pos(1) → cursor at Pos(2).
        let s =
            resolve_cursor_motion(&doc, Pos(1), Pos(1), CursorMotion::CharRight, false).unwrap();
        assert_eq!(s, Selection::cursor(Pos(2)));
        // Extend: WordRight from Pos(1) anchored at Pos(1) → text [1,6).
        let s = resolve_cursor_motion(&doc, Pos(1), Pos(1), CursorMotion::WordRight, true).unwrap();
        assert_eq!(s, Selection::text(Pos(1), Pos(6)));
        // DocEnd collapses to the document end.
        let s = resolve_cursor_motion(&doc, Pos(3), Pos(3), CursorMotion::DocEnd, false).unwrap();
        assert_eq!(s, Selection::cursor(Selection::at_end(&doc).head()));
    }
}
