//! Remote → model: turn a converged CRDT change back into editor steps.
//!
//! When a peer's change arrives, yrs merges it (convergence is *its* job). This module
//! closes the loop back to the model: [`build_remote_transaction`] rebuilds the model
//! from the converged CRDT ([`CollabDoc::to_doc`](crate::CollabDoc::to_doc)) and emits a
//! minimal **block-level** `ReplaceStep` (common-prefix/suffix on blocks, so untouched
//! blocks keep their identity). Because the model is rebuilt from the *same* CRDT both
//! peers converge to, `model ≡ project(model)` is restored exactly — no position-math
//! risk. Every remote change is therefore a block replace and never a text splice, which
//! is what makes a block changing *kind* — a paragraph becoming a `horizontal_rule`, or
//! back — nothing special here; only the caret carry below has to notice.
//!
//! There is deliberately no engine type in this file. The convergence-critical path only
//! ever needs "give me the converged document as a `Node`", which is why swapping the
//! CRDT engine (#190) left it untouched.
//!
//! A surgical, cursor-preserving translation (a remote change described as
//! insert/delete/mark ops, so a caret could be re-anchored instead of re-derived) used
//! to sit alongside this and was **dropped with automerge**: it consumed
//! `automerge::Patch`, was documented as not convergence-critical, and never fed the
//! session. It can be rebuilt on yrs observer deltas (`TextRef::observe` → `TextEvent`)
//! if a future refinement wants it.

use rinch_editor_core::{EditorState, Fragment, Node, Pos, Selection, Slice, Transaction};

use crate::error::Result;

/// Transaction meta key marking a transaction as a remote (collab) application, so the
/// history plugin and any origin-sensitive logic can tell it apart from local typing.
pub const ORIGIN_REMOTE: &str = "collabOriginRemote";

/// Build the transaction that brings `state`'s model up to the converged CRDT
/// document `target`, as a minimal block-level replace. Returns `None` if nothing
/// changed. The transaction is tagged remote + non-undoable.
pub fn build_remote_transaction(state: &EditorState, target: &Node) -> Result<Option<Transaction>> {
    let old = &state.doc;
    let on = old.child_count();
    let nn = target.child_count();

    // Common leading/trailing blocks by structural equality (different trees, so no
    // Rc fast-path here).
    let mut prefix = 0;
    while prefix < on && prefix < nn && old.child(prefix) == target.child(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < on - prefix
        && suffix < nn - prefix
        && old.child(on - 1 - suffix) == target.child(nn - 1 - suffix)
    {
        suffix += 1;
    }
    if prefix == on && nn == on {
        return Ok(None); // identical
    }

    // Model position before the first changed block, and after the last changed block.
    let start: usize = (0..prefix).map(|j| old.child(j).node_size()).sum();
    let end: usize = (0..on - suffix).map(|j| old.child(j).node_size()).sum();

    // The replacement blocks (the changed middle of the new doc).
    let mid: Vec<Node> = (prefix..nn - suffix)
        .map(|j| target.child(j).clone())
        .collect();
    let slice = Slice::new(Fragment::from_children(mid), 0, 0);

    // Where the selection's ends belong afterwards, worked out against the old
    // document before it is replaced (see `carried_position`).
    let old_selection = &state.selection;
    let carried = (
        carried_position(old, target, prefix, suffix, start, old_selection.anchor().0),
        carried_position(old, target, prefix, suffix, start, old_selection.head().0),
    );

    let mut tr = state.tr();
    tr.replace(start, end, slice)?;
    let new_doc = tr.doc().clone();
    match carried {
        // Both ends sat in changed textblocks that are still there: put them back
        // where they were in the text.
        (Some(anchor), Some(head)) if anchor == head => {
            tr.set_selection(Selection::near(&new_doc, Pos(head), 1));
        }
        (Some(anchor), Some(head)) => {
            tr.set_selection(Selection::text(Pos(anchor), Pos(head)));
        }
        // Otherwise re-anchor to a safe spot from where the mapping left the head
        // (`near` guarantees a valid text position even if the old block is gone).
        _ => {
            let head = tr.selection().head();
            tr.set_selection(Selection::near(&new_doc, head, 1));
        }
    }
    tr.set_add_to_history(false);
    tr.set_meta(ORIGIN_REMOTE, true);
    Ok(Some(tr))
}

/// Where model position `pos` of the old document belongs in the new one, when
/// it sits inside one of the changed blocks; `None` when it does not, or when
/// the change is not one this can follow, and the step mapping decides.
///
/// A remote change arrives as a block-level replace, and a position inside a
/// replaced range maps to the *end* of the replacement. For a caret in the very
/// paragraph a peer is typing in, that is the start of the next paragraph: every
/// keystroke of theirs threw it out of the line it was on, so two people could
/// not write in one paragraph. The text says where the caret belongs. With the
/// same number of changed blocks before and after, each old block corresponds
/// to the new block at the same index; inside a textblock the caret keeps its
/// place relative to the text the two versions share (before the change:
/// unmoved; after it: shifted by the difference in length; inside it: just
/// after what replaced it).
fn carried_position(
    old: &Node,
    target: &Node,
    prefix: usize,
    suffix: usize,
    start: usize,
    pos: usize,
) -> Option<usize> {
    let old_changed = old.child_count() - prefix - suffix;
    let new_changed = target.child_count() - prefix - suffix;
    if old_changed == 0 || old_changed != new_changed {
        return None; // blocks were added, removed, split or joined
    }
    // Both walks start at `start`: the blocks before it are identical.
    let (mut old_at, mut new_at) = (start, start);
    for index in prefix..prefix + old_changed {
        let (old_block, new_block) = (old.child(index), target.child(index));
        // Content positions of a block at `at`: `at + 1 ..= at + 1 + content_size`.
        let inside = pos > old_at && pos < old_at + old_block.node_size();
        if inside {
            // Both sides must be textblocks for an offset in one to mean anything in
            // the other. A **block atom** (`horizontal_rule`) is where the two can
            // differ: a peer replacing a paragraph with a scene break, or a scene break
            // with a paragraph, changes the block's kind, and there is no text offset to
            // carry across. `None` hands the caret to the step mapping, which re-anchors
            // it from the block-level replace the caller already emits — an atom is
            // never spliced as text. (An atom on the *old* side cannot even reach here:
            // a leaf is one position wide, so nothing sits `inside` it.)
            if !(old_block.is_textblock() && new_block.is_textblock()) {
                return None;
            }
            let (old_units, new_units) = (flat_units(old_block)?, flat_units(new_block)?);
            let offset = carried_offset(&old_units, &new_units, pos - old_at - 1);
            return Some(new_at + 1 + offset);
        }
        old_at += old_block.node_size();
        new_at += new_block.node_size();
    }
    None
}

/// A textblock's inline content, one entry per model position: a text node's
/// characters, and a placeholder for an inline leaf (which is one position
/// wide). `None` for inline content with positions inside it.
fn flat_units(block: &Node) -> Option<Vec<char>> {
    let mut units = Vec::with_capacity(block.content_size());
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(text) => units.extend(text.chars()),
            None if child.node_size() == 1 => units.push('\u{FFFC}'),
            None => return None,
        }
    }
    Some(units)
}

/// `offset` into `old` carried over to `new`, by the text the two share at
/// their start and at their end.
fn carried_offset(old: &[char], new: &[char], offset: usize) -> usize {
    let shortest = old.len().min(new.len());
    let mut shared_start = 0;
    while shared_start < shortest && old[shared_start] == new[shared_start] {
        shared_start += 1;
    }
    let mut shared_end = 0;
    while shared_end < shortest - shared_start
        && old[old.len() - 1 - shared_end] == new[new.len() - 1 - shared_end]
    {
        shared_end += 1;
    }
    if offset <= shared_start {
        offset
    } else if offset >= old.len() - shared_end {
        new.len() - (old.len() - offset)
    } else {
        new.len() - shared_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_editor_core::{Pos, Schema};
    use std::rc::Rc;

    fn doc(schema: &Rc<Schema>, paragraphs: &[&str]) -> Node {
        let blocks: Vec<Node> = paragraphs
            .iter()
            .map(|text| {
                let content = if text.is_empty() {
                    Fragment::empty()
                } else {
                    Fragment::from_node(schema.text(text).unwrap())
                };
                schema.branch("paragraph", content).unwrap()
            })
            .collect();
        schema
            .branch("doc", Fragment::from_children(blocks))
            .unwrap()
    }

    fn state_with_cursor(schema: &Rc<Schema>, paragraphs: &[&str], cursor: usize) -> EditorState {
        let state = EditorState::create(schema.clone(), doc(schema, paragraphs), vec![]);
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(cursor)));
        state.apply(tr)
    }

    fn head_after(state: &EditorState, target: &Node) -> usize {
        let tr = build_remote_transaction(state, target)
            .unwrap()
            .expect("the document changed");
        state.apply(tr).selection.head().0
    }

    /// A peer backspaces at the end of the line this caret is at the end of: the
    /// caret stays at the end of that line. It used to be carried to the end of the
    /// replaced range and re-anchored from there, which is the start of the *next*
    /// paragraph whenever there is one.
    #[test]
    fn a_peer_deleting_before_the_caret_in_its_own_paragraph_keeps_it_there() {
        let schema = Rc::new(Schema::starter_kit());
        // "Once in a while" is 15 long: content 1..=16, the caret at its end.
        let state = state_with_cursor(&schema, &["Once in a while", "next"], 16);
        let target = doc(&schema, &["Once in a whil", "next"]);
        assert_eq!(head_after(&state, &target), 15);
    }

    /// A peer types earlier in the same paragraph: the caret keeps its place in the
    /// text, which is one further along.
    #[test]
    fn a_peer_typing_before_the_caret_in_its_own_paragraph_shifts_it() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world", "next"], 7); // before "world"
        let target = doc(&schema, &["oh hello world", "next"]);
        assert_eq!(head_after(&state, &target), 10);
    }

    /// A peer types after the caret in the same paragraph: the caret does not move.
    #[test]
    fn a_peer_typing_after_the_caret_in_its_own_paragraph_leaves_it() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world", "next"], 3);
        let target = doc(&schema, &["hello world!", "next"]);
        assert_eq!(head_after(&state, &target), 3);
    }

    /// The peer rewrites the very text the caret is inside: it lands just after
    /// what replaced that text.
    #[test]
    fn a_caret_inside_the_text_a_peer_replaced_lands_after_the_replacement() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["one two three", "next"], 7); // "one tw|o three"
        let target = doc(&schema, &["one 2 three", "next"]);
        assert_eq!(head_after(&state, &target), 6); // "one 2| three"
    }

    /// A range selection keeps both ends.
    #[test]
    fn a_selection_in_the_edited_paragraph_keeps_both_ends() {
        let schema = Rc::new(Schema::starter_kit());
        let state = EditorState::create(
            schema.clone(),
            doc(&schema, &["hello world", "next"]),
            vec![],
        );
        let mut tr = state.tr();
        tr.set_selection(Selection::text(Pos(7), Pos(12))); // "world"
        let state = state.apply(tr);
        let target = doc(&schema, &["oh hello world", "next"]);
        let tr = build_remote_transaction(&state, &target).unwrap().unwrap();
        let after = state.apply(tr);
        assert_eq!(
            (after.selection.anchor().0, after.selection.head().0),
            (10, 15)
        );
    }

    /// The peer splits the caret's paragraph: not a change this follows, and the
    /// caret still ends up somewhere valid.
    #[test]
    fn a_split_paragraph_still_leaves_a_valid_caret() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world"], 3);
        let target = doc(&schema, &["hello", " world"]);
        let head = head_after(&state, &target);
        assert!(head >= 1 && head <= target.content_size());
    }

    /// A caret in a paragraph the peer did not touch is carried by the mapping, as
    /// before.
    #[test]
    fn a_caret_in_an_untouched_later_paragraph_shifts_with_the_document() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello", "next"], 9); // "n|ext"
        let target = doc(&schema, &["hello there", "next"]);
        assert_eq!(head_after(&state, &target), 15);
    }

    /// A peer removes a whole block. The old and new changed ranges then no longer
    /// correspond index for index, so the walk that follows them in step must not
    /// start: `Fragment::child` indexes a `Vec`, and reading past the end of the
    /// shorter range is a panic — which on wasm takes the page down. The
    /// block-count guard is the whole of what stands in front of that, and this is
    /// its only pin; `a_split_paragraph_still_leaves_a_valid_caret` covers the
    /// other direction, where there are *more* new blocks and no walk runs off
    /// anything.
    #[test]
    fn a_peer_deleting_a_whole_paragraph_is_left_to_the_mapping() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["keep", "doomed"], 3); // "ke|ep"
        let target = doc(&schema, &["keep"]);
        assert_eq!(head_after(&state, &target), 3);
    }

    /// Two blocks change in one delta and the caret is in the **second** of them.
    /// Its new home is measured against what the earlier changed blocks have
    /// *become*, not against what they were — every other fixture puts the caret in
    /// the first changed block, where the two are the same number and a walk that
    /// accumulated the old sizes would still answer correctly.
    #[test]
    fn a_caret_in_a_later_changed_block_is_measured_against_the_new_earlier_block() {
        let schema = Rc::new(Schema::starter_kit());
        // "alpha" is 5 long, so its block is 7 wide and "beta" content is 8..=12;
        // the caret is at the end of "beta".
        let state = state_with_cursor(&schema, &["alpha", "beta", "tail"], 12);
        let target = doc(&schema, &["alpha rewritten", "beta!", "tail"]);
        // "alpha rewritten" is 15 long, so its block is 17 wide, and the caret keeps
        // its place at the end of "beta": 17 + 1 + 4.
        assert_eq!(head_after(&state, &target), 22);
    }

    /// A peer types before the caret in a line that holds an **inline atom**, which
    /// the caret has to be measured *across*. `flat_units` gives the atom one unit —
    /// the same single model position it occupies, and the same U+FFFC the collab
    /// projection stands it up with — so the offsets on either side of it still line
    /// up. Counting it as zero (or as its rendered width) would slide the caret by one
    /// per picture on every keystroke a peer makes.
    #[test]
    fn a_caret_after_an_inline_atom_is_carried_across_it() {
        let schema = Rc::new(Schema::starter_kit());
        let line = |lead: &str| {
            let image = schema
                .create_node(
                    "image",
                    rinch_editor_core::Attrs::new()
                        .with("src", rinch_editor_core::AttrValue::from("cat.png")),
                    Fragment::empty(),
                )
                .unwrap();
            let para = schema
                .branch(
                    "paragraph",
                    Fragment::from_children(vec![
                        schema.text(lead).unwrap(),
                        image,
                        schema.text("tail").unwrap(),
                    ]),
                )
                .unwrap();
            schema.branch("doc", Fragment::from_node(para)).unwrap()
        };
        // doc(paragraph("ab", image, "tail")): content 1..=8, the caret after "ta".
        let state = EditorState::create(schema.clone(), line("ab"), vec![]);
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(6)));
        let state = state.apply(tr);
        // The peer types one char before the atom: everything after it shifts by one.
        assert_eq!(head_after(&state, &line("abX")), 7);
    }

    /// The caret sits exactly where the two versions stop agreeing. It belongs to
    /// the text before it, which both versions share, so it does not move. Reading
    /// that boundary as *inside* the change instead would carry it to the end of
    /// whatever replaced the rest of the line — a caret in the middle of a line
    /// jumping to its end on a peer's keystroke.
    #[test]
    fn a_caret_at_the_point_the_two_versions_diverge_stays_put() {
        let schema = Rc::new(Schema::starter_kit());
        let state = state_with_cursor(&schema, &["hello world"], 7); // "hello |world"
        let target = doc(&schema, &["hello there"]);
        assert_eq!(head_after(&state, &target), 7);
    }
}
