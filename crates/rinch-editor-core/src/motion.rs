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
    let segments = word_segments(&units);
    let content_start = head.0 - r.parent_offset();
    let mut i = r.parent_offset().min(units.len());
    if forward {
        while i < units.len() && units[i].is_space() {
            i += 1;
        }
        if i < units.len() {
            i = segment_at(&segments, i).1;
        }
    } else {
        while i > 0 && units[i - 1].is_space() {
            i -= 1;
        }
        if i > 0 {
            i = segment_at(&segments, i - 1).0;
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

/// One position of a textblock's content: a char of its text, or an inline
/// atom (an image, a hard break).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A character of a text node.
    Char(char),
    /// A non-text leaf: one position, and a word of its own.
    Atom,
}

impl Unit {
    fn is_space(self) -> bool {
        matches!(self, Unit::Char(c) if c.is_whitespace())
    }
}

/// The positions of a textblock's content, one [`Unit`] per position.
pub fn block_units(block: &Node) -> Vec<Unit> {
    let mut units = Vec::with_capacity(block.content().size());
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(t) => units.extend(t.chars().map(Unit::Char)),
            None => units.extend(std::iter::repeat_n(Unit::Atom, child.node_size())),
        }
    }
    units
}

/// What a unit is to word segmentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Space,
    /// Letters, digits, marks and `_` of every script but the CJK ones.
    Word,
    /// Han, Hiragana and Katakana.
    Cjk,
    /// Everything else that is not whitespace: punctuation, symbols, emoji.
    Punct,
    Atom,
}

/// Han, Hiragana, Katakana (and their iteration and prolonged-sound marks).
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3005..=0x3007 | 0x3021..=0x3029 | 0x3031..=0x3035 | 0x303B..=0x303C
        | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF | 0xFF66..=0xFF9F | 0x20000..=0x3134F)
}

/// A char that extends the one before it rather than starting anything: a
/// combining mark of any script (General_Category M), a format char (Cf: the
/// joiners, variation-selector-like controls) or an emoji skin-tone modifier.
/// std has no General_Category, so this asks the `regex` crate's Unicode
/// tables, which this crate links already for its input rules.
fn is_extend(c: char) -> bool {
    use std::sync::OnceLock;
    static EXTEND: OnceLock<regex::Regex> = OnceLock::new();
    if c.is_ascii() {
        return false;
    }
    let re = EXTEND.get_or_init(|| {
        regex::Regex::new(r"^[\p{M}\p{Cf}\x{1F3FB}-\x{1F3FF}]$").expect("a valid class")
    });
    let mut buf = [0u8; 4];
    re.is_match(c.encode_utf8(&mut buf))
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_cjk(c) {
        Class::Cjk
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// The word segments of a textblock's content, as `(start, end)` unit offsets
/// that tile it: maximal runs of one class — whitespace, word characters, CJK,
/// punctuation — with every inline atom a segment of its own (#938, #939).
/// A combining mark or joiner joins the unit before it; an apostrophe between
/// two word characters stays in the word (`don't`). With the `word-dictionary`
/// feature a word or CJK run is split further where ICU's dictionary segmenter
/// breaks it (`漢字です` is `漢字` `です`); without it such a run is one segment.
///
/// Measured against Chrome 153 (contenteditable, Ctrl+Backspace): `foo.bar` is
/// `foo` `.` `bar`, `foo...` is `foo` `...`, `don't` and an emoji ZWJ sequence
/// are one word each. Not Chrome's ICU word rules, which keep `foo.bar` and
/// `3.14` whole: Chrome's editing does not either.
pub fn word_segments(units: &[Unit]) -> Vec<(usize, usize)> {
    let class = |i: usize| match units[i] {
        Unit::Atom => Class::Atom,
        Unit::Char(c) => class_of(c),
    };
    let mut classes: Vec<Class> = Vec::with_capacity(units.len());
    for i in 0..units.len() {
        let mut k = class(i);
        if let (Unit::Char(c), Some(&prev)) = (units[i], classes.last())
            && is_extend(c)
            && prev != Class::Atom
            && prev != Class::Space
        {
            k = prev;
        }
        classes.push(k);
    }
    // An apostrophe between two word characters is part of the word.
    for i in 1..units.len().saturating_sub(1) {
        if matches!(units[i], Unit::Char('\'' | '\u{2019}'))
            && classes[i - 1] == Class::Word
            && classes[i + 1] == Class::Word
        {
            classes[i] = Class::Word;
        }
    }
    let mut segments = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let k = classes[i];
        let mut j = i + 1;
        if k != Class::Atom {
            while j < units.len() && classes[j] == k {
                j += 1;
            }
        }
        if matches!(k, Class::Word | Class::Cjk) {
            split_by_dictionary(units, i, j, &mut segments);
        } else {
            segments.push((i, j));
        }
        i = j;
    }
    segments
}

/// Push the segments of the run `units[start..end]`: ICU's dictionary word
/// breaks inside it with `word-dictionary`, the whole run without.
#[cfg(feature = "word-dictionary")]
fn split_by_dictionary(units: &[Unit], start: usize, end: usize, out: &mut Vec<(usize, usize)>) {
    use icu_segmenter::WordSegmenter;
    use icu_segmenter::options::WordBreakInvariantOptions;
    let text: String = units[start..end]
        .iter()
        .map(|u| match *u {
            Unit::Char(c) => c,
            Unit::Atom => ' ',
        })
        .collect();
    // Byte offsets of ICU's breaks, mapped to unit offsets.
    let mut byte_to_unit = Vec::with_capacity(text.len() + 1);
    for (n, c) in text.chars().enumerate() {
        byte_to_unit.extend(std::iter::repeat_n(start + n, c.len_utf8()));
    }
    byte_to_unit.push(end);
    let mut prev = start;
    for b in WordSegmenter::new_dictionary(WordBreakInvariantOptions::default()).segment_str(&text)
    {
        let at = byte_to_unit[b];
        if at > prev {
            out.push((prev, at));
            prev = at;
        }
    }
    if prev < end {
        out.push((prev, end));
    }
}

#[cfg(not(feature = "word-dictionary"))]
fn split_by_dictionary(_: &[Unit], start: usize, end: usize, out: &mut Vec<(usize, usize)>) {
    out.push((start, end));
}

/// The segment holding unit `i` (which must be inside the content).
fn segment_at(segments: &[(usize, usize)], i: usize) -> (usize, usize) {
    let k = segments.partition_point(|&(_, end)| end <= i);
    segments[k]
}

/// The model word range `(start, end)` around `pos` — the [word
/// segment](word_segments) under a double-click (a run of whitespace, a word, a
/// run of punctuation, an atom), chosen by the unit to the right of the gap, or
/// the one to its left at block end and at a word's trailing edge (right unit
/// whitespace, left not). Collapsed `(pos, pos)` outside a textblock or in an
/// empty block.
pub fn word_range_at(doc: &Node, pos: Pos) -> (Pos, Pos) {
    let Ok(r) = doc.resolve(pos) else {
        return (pos, pos);
    };
    if !r.parent().is_textblock() {
        return (pos, pos);
    }
    let units = block_units(r.parent());
    if units.is_empty() {
        return (pos, pos);
    }
    let content_start = pos.0 - r.parent_offset();
    let off = r.parent_offset().min(units.len());
    let classify_right =
        off < units.len() && (!units[off].is_space() || off == 0 || units[off - 1].is_space());
    let idx = if classify_right { off } else { off - 1 };
    let (start, end) = segment_at(&word_segments(&units), idx);
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
