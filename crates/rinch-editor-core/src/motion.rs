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

#[cfg(feature = "cost-counters")]
#[doc(hidden)]
pub use rinch_words::segmented_units;
/// The word segmentation itself lives in `rinch-words`, which the desktop text
/// fields share (#1619).
pub use rinch_words::{Unit, word_segments};

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
            let to = grapheme_step(doc, head, false).unwrap_or(head.0.saturating_sub(1));
            let near = Selection::near(doc, Pos(to), -1);
            // A char move onto a selectable block atom (hr/image) becomes a node
            // selection when collapsing (not when extending a text range).
            if !extend && matches!(near, Selection::Node(_)) {
                return Some(near);
            }
            near.head()
        }
        CursorMotion::CharRight => {
            let to = grapheme_step(doc, head, true).unwrap_or(head.0 + 1);
            let near = Selection::near(doc, Pos(to.min(doc.content_size())), 1);
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

/// The position one grapheme cluster before (`forward = false`) or after
/// `head` inside its textblock (#1684): an emoji ZWJ sequence, a flag, a
/// keycap or a base with its marks is one step, as in Chrome. `None` when
/// `head` is not in a textblock or is at the edge it would step across, where
/// the motion leaves the block one position at a time.
pub fn grapheme_step(doc: &Node, head: Pos, forward: bool) -> Option<usize> {
    let r = doc.resolve(head).ok()?;
    let block = r.parent();
    if !block.is_textblock() {
        return None;
    }
    let off = r.parent_offset();
    if (forward && off >= block.content().size()) || (!forward && off == 0) {
        return None;
    }
    let text = grapheme_text(block);
    let i = if forward {
        rinch_words::next_grapheme_boundary_chars(&text, off)
    } else {
        rinch_words::prev_grapheme_boundary_chars(&text, off)
    };
    Some(head.0 - off + i)
}

/// Where a Backspace at `head` starts deleting inside its textblock (#1684):
/// an emoji sequence goes whole, any other cluster one code point at a time
/// ([`rinch_words::backspace_start`], Chrome 155's rule). `None` outside a
/// textblock or at its start.
pub fn backspace_start(doc: &Node, head: Pos) -> Option<usize> {
    let r = doc.resolve(head).ok()?;
    let block = r.parent();
    let off = r.parent_offset();
    if !block.is_textblock() || off == 0 {
        return None;
    }
    let text = grapheme_text(block);
    Some(head.0 - off + rinch_words::backspace_start_chars(&text, off))
}

/// A textblock's content as one string for grapheme segmentation, one char per
/// position: a hard break is `'\n'` (a cluster never crosses it) and any other
/// leaf U+FFFC.
fn grapheme_text(block: &Node) -> String {
    let mut s = String::with_capacity(block.content().size());
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(t) => s.push_str(t),
            None => {
                let c = if child.type_name() == "hard_break" { '\n' } else { '\u{FFFC}' };
                s.extend(std::iter::repeat_n(c, child.node_size()));
            }
        }
    }
    s
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

/// The model position at the previous/next word boundary within `head`'s
/// textblock (cross-block word motion is a later refinement). `None` if `head`
/// is not in a textblock.
///
/// Backward (Ctrl+Left): past any whitespace before `head`, then to the start of
/// the [word segment](word_segments) before it. Forward (Ctrl+Right): past any
/// whitespace after `head`, then to the end of the segment after it — the end
/// of the next word, as on Linux and macOS. An inline atom is a segment of its
/// own, so the motion stops on either side of it (#938), and a run of
/// punctuation is one apart from the letters beside it (#939). The word deletes
/// remove exactly this range (`commands::word_delete_range`).
pub fn word_boundary(doc: &Node, head: Pos, forward: bool) -> Option<Pos> {
    let r = doc.resolve(head).ok()?;
    if !r.parent().is_textblock() {
        return None;
    }
    let units = block_units(r.parent());
    let content_start = head.0 - r.parent_offset();
    let i = rinch_words::word_boundary(&units, r.parent_offset(), forward);
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

/// The positions of a textblock's content, one [`Unit`] per position.
pub fn block_units(block: &Node) -> Vec<Unit> {
    let mut units = Vec::with_capacity(block.content().size());
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(t) => units.extend(t.chars().map(Unit::Char)),
            None => {
                let unit = if child.type_name() == "hard_break" {
                    Unit::Break
                } else {
                    Unit::Atom
                };
                units.extend(std::iter::repeat_n(unit, child.node_size()));
            }
        }
    }
    units
}

/// The model word range `(start, end)` around `pos` — the [word
/// segment](word_segments) under a double-click (a run of whitespace, a word,
/// images, a hard break), chosen by the unit to the right of the gap, or the
/// one to its left at block end and at a word's trailing edge (right unit
/// whitespace, left not). On a punctuation run it is that one char (with its
/// marks), as in Chrome 155 (`foo|...bar` selects one `.`, `foo|——bar` one
/// `—`); a run of emoji stays whole (`😀😀`). Collapsed `(pos, pos)` outside a
/// textblock or in an empty block.
pub fn word_range_at(doc: &Node, pos: Pos) -> (Pos, Pos) {
    let Ok(r) = doc.resolve(pos) else {
        return (pos, pos);
    };
    if !r.parent().is_textblock() {
        return (pos, pos);
    }
    let units = block_units(r.parent());
    let content_start = pos.0 - r.parent_offset();
    let (start, end) = rinch_words::word_range(&units, r.parent_offset());
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
