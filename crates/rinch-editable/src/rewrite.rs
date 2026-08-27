//! Adopting an external rewrite of the whole text (issue #238).
//!
//! A controlled input's owner may rewrite the field's text while the user is
//! editing it — a normalizing `oninput` (uppercase a ticker, strip non-digits),
//! a swatch pick, a timer. The engine must take that text without losing the
//! user's place: the selection is mapped through the rewrite, and the change is
//! recorded on the undo stack like any other edit so undo stays coherent.

use crate::{EditableDocument, EditableState, Position, Range, Selection, TextOperation};

/// The shape of a rewrite `old → new` as a kept prefix, a kept suffix, and a
/// replaced middle — the minimal splice, on UTF-8 byte offsets snapped to char
/// boundaries.
///
/// The caret rule (the same on every backend): an offset inside the kept
/// suffix keeps its distance from the end; an offset inside the kept prefix
/// stays; an offset inside the replaced middle stays in place when the
/// replacement is the same length as what it replaced (a case change, a
/// character substitution) and otherwise lands right after the replacement.
/// The suffix branch is tried first so that an offset exactly at an empty
/// replaced span — a pure insertion at the caret, an empty field being
/// filled — ends up after the inserted text, where a browser's `.value`
/// setter would leave it. Anchor and head are mapped independently so a
/// selection survives. Results are byte offsets of `new`; the same-length
/// branch can land inside a multi-byte char, which [`EditableState::adopt_text`]
/// snaps down to a boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewriteDiff {
    /// Bytes of `old` and `new` that are identical from the start.
    pub prefix: usize,
    /// Bytes identical at the end, beyond the prefix (`prefix + suffix` never
    /// exceeds the shorter string).
    pub suffix: usize,
    old_len: usize,
    new_len: usize,
}

impl RewriteDiff {
    /// Diff `old` against `new`.
    pub fn between(old: &str, new: &str) -> Self {
        let mut prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        // The first differing byte is a lead byte in both strings or a
        // continuation byte in both (they share every byte before it, so they
        // share the sequence it falls inside), hence `old` and `new` agree on
        // whether `prefix` is a char boundary. Snap down to one.
        while !old.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let mut suffix = old[prefix..]
            .bytes()
            .rev()
            .zip(new[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        // Same argument at the suffix's start.
        while !old.is_char_boundary(old.len() - suffix) {
            suffix -= 1;
        }
        Self {
            prefix,
            suffix,
            old_len: old.len(),
            new_len: new.len(),
        }
    }

    /// The range of `old` that was replaced.
    pub fn replaced(&self) -> Range {
        Range::new(self.prefix, self.old_len - self.suffix)
    }

    /// The bytes of `new` that replace [`Self::replaced`].
    pub fn replacement_range(&self) -> std::ops::Range<usize> {
        self.prefix..self.new_len - self.suffix
    }

    /// Map a byte offset in `old` to its logical position in `new`.
    pub fn map(&self, offset: usize) -> usize {
        if offset >= self.old_len - self.suffix {
            self.new_len - (self.old_len - offset)
        } else if offset <= self.prefix {
            offset
        } else if self.old_len == self.new_len {
            // Prefix and suffix are shared, so equal totals mean the replaced
            // middle kept its length: an in-place rewrite.
            offset
        } else {
            self.new_len - self.suffix
        }
    }
}

impl<D: EditableDocument> EditableState<D> {
    /// Replace the whole text with `new`, keeping the selection at its logical
    /// place (see [`RewriteDiff`]) and recording the change on the undo stack.
    /// A no-op when the text already matches. Returns whether anything changed.
    pub fn adopt_text(&mut self, new: &str) -> bool {
        let old = self.document.to_text();
        if old == new {
            return false;
        }
        let diff = RewriteDiff::between(&old, new);
        let replaced = diff.replaced();
        if !replaced.is_empty() {
            let deleted = self.document.text_slice(replaced);
            self.document.delete(replaced);
            self.undo_stack.push(TextOperation::Delete {
                range: replaced,
                deleted_text: deleted,
            });
        }
        let replacement = &new[diff.replacement_range()];
        if !replacement.is_empty() {
            self.document.insert(replaced.start, replacement);
            self.undo_stack.push(TextOperation::Insert {
                pos: replaced.start,
                text: replacement.to_string(),
            });
        }
        let map = |offset: usize| {
            let mut mapped = diff.map(offset);
            while !new.is_char_boundary(mapped) {
                mapped -= 1;
            }
            Position(mapped)
        };
        self.selection = Selection::new(map(self.selection.anchor.0), map(self.selection.head.0));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EditCommand, StringDocument};

    fn map(old: &str, new: &str, caret: usize) -> usize {
        RewriteDiff::between(old, new).map(caret)
    }

    #[test]
    fn a_kept_prefix_keeps_the_caret() {
        // "a|b" with an appended suffix
        assert_eq!(map("ab", "abc!", 1), 1);
        assert_eq!(map("ab", "abc!", 0), 0);
        // whole-text rewrite that shares the prefix
        assert_eq!(map("hello", "help", 3), 3);
        // "a|bcd" → "abcd!" (a trailing insertion far from the caret)
        assert_eq!(map("abcd", "abcd!", 1), 1);
    }

    #[test]
    fn a_kept_suffix_keeps_the_distance_from_the_end() {
        // "abc|" rewritten to "ABC": nothing kept → end
        assert_eq!(map("abc", "ABC", 3), 3);
        // "ab|cd" → "ABcd": suffix "cd" kept → stays 2 from the end
        assert_eq!(map("abcd", "ABcd", 2), 2);
        assert_eq!(map("abcd", "ABcd", 3), 3);
        // a shorter prefix: "xyz|cd" → "Q" + "cd"
        assert_eq!(map("xyzcd", "Qcd", 3), 1);
        assert_eq!(map("xyzcd", "Qcd", 4), 2);
        // replacement longer than what it replaces, caret before the suffix
        assert_eq!(map("a-b", "aXYZb", 2), 4);
    }

    #[test]
    fn a_caret_in_a_same_length_rewrite_stays_in_place() {
        // "ab|c" → "ABC": a case change keeps the caret where it was
        assert_eq!(map("abc", "ABC", 2), 2);
        assert_eq!(map("abc", "ABC", 1), 1);
        // "12-|34" → "12/34": a one-for-one substitution
        assert_eq!(map("12-34", "12/34", 3), 3);
        // "xa|bcz" → "xABCz": middle "abc" → "ABC"
        assert_eq!(map("xabcz", "xABCz", 2), 2);
    }

    #[test]
    fn a_caret_in_a_resized_middle_lands_after_the_replacement() {
        // caret inside a longer replaced middle
        assert_eq!(map("aXYZb", "a-b", 2), 2);
        assert_eq!(map("aXYZb", "a-b", 3), 2);
        // caret inside a middle that grew
        assert_eq!(map("a-Qb", "aXYZb", 2), 4);
        // "1234|5" reformatted with a separator ahead of the caret
        assert_eq!(map("12345", "12,345", 4), 5);
    }

    #[test]
    fn an_insertion_at_the_caret_leaves_the_caret_after_it() {
        // The tie: the caret sits exactly at an empty replaced span. The
        // suffix branch wins, so the inserted text ends up before the caret —
        // where a browser's `.value` setter leaves it.
        assert_eq!(map("", "X", 0), 1);
        assert_eq!(map("ab", "abc!", 2), 4);
        assert_eq!(map("ab", "aXb", 1), 2);
    }

    #[test]
    fn prefix_and_suffix_never_overlap() {
        // "aa" → "aaa": prefix 2 covers all of old, suffix must be 0
        let d = RewriteDiff::between("aa", "aaa");
        assert_eq!((d.prefix, d.suffix), (2, 0));
        assert_eq!(d.map(2), 3);
        assert_eq!(d.map(1), 1);
        // "aaa" → "aa": prefix 2, suffix 0 → the deleted 'a' is the last one
        let d = RewriteDiff::between("aaa", "aa");
        assert_eq!((d.prefix, d.suffix), (2, 0));
        assert_eq!(d.map(3), 2);
        assert_eq!(d.map(1), 1);
    }

    #[test]
    fn offsets_snap_to_char_boundaries() {
        // "é" (C3 A9) vs "è" (C3 A8): the byte prefix is 1 but the char
        // prefix is 0.
        let d = RewriteDiff::between("é", "è");
        assert_eq!((d.prefix, d.suffix), (0, 0));
        assert_eq!(d.map(2), 2);
        // Shared lead byte at the suffix side: "aé" → "bé" keeps "é";
        // "xé" vs "yè" shares only the trailing... nothing (A9 ≠ A8).
        let d = RewriteDiff::between("xé", "yè");
        assert_eq!((d.prefix, d.suffix), (0, 0));
        // "héllo" caret after "é" (3), "é" → "€" (3 bytes): after "€" (4)
        assert_eq!(map("héllo", "h€llo", 3), 4);
        assert_eq!(map("héllo", "h€llo", 1), 1);
        assert_eq!(map("héllo", "h€llo", 6), 7);
        // every mapped offset is a char boundary of `new` (the diff branches)
        for (old, new) in [("héllo", "h€llo"), ("a😀b", "a😀😀b"), ("€", "é")] {
            let d = RewriteDiff::between(old, new);
            for i in (0..=old.len()).filter(|&i| old.is_char_boundary(i)) {
                assert!(new.is_char_boundary(d.map(i)), "{old:?}→{new:?} @{i}");
            }
        }
        // ...and the same-length branch can land mid-char, which adopt_text
        // snaps: "a|é" (3 bytes) → "€" (3 bytes)
        let mut state = EditableState::new(StringDocument::with_text("aé"));
        state.selection = Selection::cursor(1);
        state.adopt_text("€");
        assert!("€".is_char_boundary(state.selection.head.0));
    }

    #[test]
    fn identical_and_empty_texts() {
        let d = RewriteDiff::between("abc", "abc");
        assert_eq!((d.prefix, d.suffix), (3, 0));
        assert_eq!(d.map(1), 1);
        assert_eq!(map("", "abc", 0), 3);
        assert_eq!(map("abc", "", 2), 0);
    }

    #[test]
    fn adopt_text_maps_the_selection_and_keeps_undo_coherent() {
        let mut state = EditableState::new(StringDocument::new());
        state.execute(EditCommand::InsertText("h".into()));
        state.execute(EditCommand::InsertText("i".into()));
        assert!(state.adopt_text("HI!"));
        assert_eq!(state.document.to_text(), "HI!");
        assert_eq!(state.selection, Selection::cursor(3));

        // Undo walks back through the adopted rewrite to the typed text, then
        // through the keystrokes — no stale offsets, no panic.
        state.execute(EditCommand::Undo);
        state.execute(EditCommand::Undo);
        assert_eq!(state.document.to_text(), "hi");
        state.execute(EditCommand::Undo);
        assert_eq!(state.document.to_text(), "h");
        state.execute(EditCommand::Redo);
        state.execute(EditCommand::Redo);
        state.execute(EditCommand::Redo);
        assert_eq!(state.document.to_text(), "HI!");
    }

    #[test]
    fn adopt_text_keeps_a_selection() {
        let mut state = EditableState::new(StringDocument::with_text("abcd"));
        state.selection = Selection::new(3, 2);
        assert!(state.adopt_text("ABcd"));
        assert_eq!(state.selection, Selection::new(3, 2));
        assert!(!state.adopt_text("ABcd"), "a matching text is a no-op");
        assert!(!state.undo_stack.can_redo());
    }
}
