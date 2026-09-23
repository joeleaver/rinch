//! Sticky positions: an editor position as a yrs [`StickyIndex`], and back.
//!
//! A model [`Pos`] is an integer into *this* replica's document at *this* moment; any
//! edit before it, local or a peer's, moves what it points at. A deep link (an app
//! pointing at a place inside a document, from outside it) needs an address that
//! follows the text instead. yrs has exactly that: a [`StickyIndex`] names the CRDT
//! *item* next to a position, so it resolves to wherever that character is now, on any
//! replica, after any merge.
//!
//! ## The encoding is plain yrs, on purpose
//!
//! [`CollabDoc::sticky_index`] returns [`StickyIndex::encode_v1`] of an index created on
//! the `text` [`TextRef`] of the textblock holding the position (the wire shape in
//! [`crate::projection`]), and nothing else: no wrapper, no version byte, no block
//! path. So an app can resolve the bytes **without an editor**, with yrs alone, against
//! any replica of the document: [`StickyIndex::decode_v1`], then
//! [`StickyIndex::get_offset`], then check that `offset.branch` is the `text` of a live
//! textblock under the `content` root. The index is in UTF-16 code units, as every
//! index of this projection is (`OffsetKind::Utf16`).
//!
//! The association is [`Assoc::After`] (the index sticks to the character that
//! *starts* at the position), except at the very end of a text, where there is no such
//! character and it is [`Assoc::Before`] (it sticks to the last one). An empty text has
//! neither, and yrs then names the `Text` itself (`IndexScope::Nested`), which resolves
//! to its start.
//!
//! ## What a sticky index survives, and what it does not
//!
//! * Any insert or delete elsewhere, local or merged from a peer: it moves with its
//!   character.
//! * Its own character deleted: it resolves to the place where that character was
//!   (yrs keeps the tombstone), which is the neighbouring position.
//! * Its **whole block deleted**, or its character **moved to another block**: `None`.
//!   The projection has no "move": joining two blocks deletes the second block's
//!   `Text` and re-inserts its characters into the first, and splitting a block deletes
//!   the tail from one `Text` and inserts it into a new one (see
//!   [`crate::project`]). A sticky index on the moved characters then points into a
//!   deleted `Text` (joined) or at a tombstone at the split point (split, which
//!   resolves to the end of the first half).
//!
//! ## Why the model document is a parameter
//!
//! The CRDT does not know model positions: a model position counts open and close
//! tokens of nodes, and how many a node has is the schema's business (a leaf block
//! atom has none). So both directions take the model document the CRDT projects, and
//! use the one fact that makes the mapping cheap: **the projection mirrors the model's
//! tree index for index** (`model ≡ project(model)`), so the child indices on the path
//! from the root to a textblock name the same node in both. Each step of the walk
//! checks the node type, and the textblock's text is compared with the CRDT's, so a
//! model that is not the one this CRDT projects gives `None` rather than a position in
//! the wrong place. Inline atoms (`image`, `hard_break`) are one char in the CRDT's text
//! and one position in the model, so they need nothing of their own.
//!
//! The one block with no CRDT behind it is the starter paragraph of a CRDT holding
//! zero blocks (see [`CollabDoc::to_doc`]): positions in it have no sticky index.

use yrs::branch::{Branch, BranchPtr};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Array, ArrayRef, Assoc, GetString, IndexedSequence, ReadTxn, StickyIndex, TextRef, Transact,
};

use rinch_editor_core::{Node, Pos};

use crate::projection::{
    CollabDoc, block_text, child_map, node_content, node_type, read_block, u16_offset,
};

impl CollabDoc {
    /// The sticky index of model position `pos` of `doc`: an address that follows its
    /// character through every later edit, on this replica or any other, for deep
    /// links. [`Self::resolve_sticky`] turns it back into a position.
    ///
    /// **The bytes are plain yrs**, so an app can resolve them without an editor: a
    /// [`StickyIndex`] in its v1 encoding ([`StickyIndex::encode_v1`]), created with
    /// [`IndexedSequence::sticky_index`] on the `text` [`TextRef`] of the textblock
    /// holding `pos` (the wire shape in [`crate::projection`]), at the position's
    /// UTF-16 offset in that text. The association is [`Assoc::After`] (the character
    /// that starts at `pos`), except at the very end of a text, where it is
    /// [`Assoc::Before`] (the last character); an empty text has neither and yrs then
    /// names the `Text` itself. No wrapper, no version byte. To resolve with yrs
    /// alone: [`StickyIndex::decode_v1`], [`StickyIndex::get_offset`] on a transaction
    /// of any replica, then check that `offset.branch` is the `text` of a block still
    /// under the `content` root (a deleted block's text is not); `offset.index` is a
    /// UTF-16 offset into that text.
    ///
    /// It follows its character through any edit elsewhere; a deleted character
    /// resolves to where it was; it ends with its block. The projection has no
    /// "move", so a block **joined** into the one before it ends it too, and a
    /// **split** before it leaves it at the split point.
    ///
    /// `doc` must be the model document this CRDT projects (the editor's current
    /// document): the projection mirrors the model's tree index for index, which is
    /// how the textblock is found. `None` when:
    ///
    /// * `pos` is out of range, or not inside a textblock (between blocks, say);
    /// * the textblock has no CRDT behind it: the starter paragraph of an empty CRDT,
    ///   or a `doc` that is not the one this CRDT projects (a node type or the
    ///   block's text differs along the way).
    pub fn sticky_index(&self, doc: &Node, pos: Pos) -> Option<Vec<u8>> {
        let rp = doc.resolve(pos).ok()?;
        let block = rp.parent();
        if !block.is_textblock() {
            return None;
        }
        let model_text = read_block(block).ok()?.text;

        let txn = self.doc.transact();
        // Walk the child indices from the root to the textblock, which name the same
        // nodes in the projection.
        let mut list: ArrayRef = self.content.clone();
        let mut text: Option<TextRef> = None;
        for depth in 0..rp.depth() {
            let map = child_map(&txn, &list, u32::try_from(rp.index(depth)).ok()?)?;
            let model_node = rp.node(depth + 1);
            if node_type(&txn, &map)? != model_node.type_name() {
                return None;
            }
            if depth + 1 == rp.depth() {
                text = Some(block_text(&txn, &map)?);
            } else {
                list = node_content(&txn, &map)?;
            }
        }
        let text = text?;
        let crdt_text = text.get_string(&txn);
        if crdt_text != model_text {
            return None;
        }

        let at = u16_offset(&crdt_text, rp.parent_offset());
        let len = u16_offset(&crdt_text, usize::MAX);
        let assoc = if at < len {
            Assoc::After
        } else {
            Assoc::Before
        };
        let sticky = text
            .sticky_index(&txn, at, assoc)
            .or_else(|| text.sticky_index(&txn, at, Assoc::Before))?;
        Some(sticky.encode_v1())
    }

    /// Where the sticky index `bytes` (from [`Self::sticky_index`], on this replica or
    /// any other) points now, as a model position of `doc`, the model document this
    /// CRDT projects.
    ///
    /// `None` when the bytes do not decode, when the item they name is unknown here
    /// (a peer's edit not merged yet) or garbage-collected, when it is not in the text
    /// of a **live** textblock (its block was deleted, or it names something that is
    /// not a textblock's text at all), or when `doc` is not the document this CRDT
    /// projects.
    pub fn resolve_sticky(&self, doc: &Node, bytes: &[u8]) -> Option<Pos> {
        let sticky = StickyIndex::decode_v1(bytes).ok()?;
        let txn = self.doc.transact();
        let offset = sticky.get_offset(&txn)?;

        let mut path: Vec<(usize, String)> = Vec::new();
        let text = find_text(&txn, &self.content, offset.branch, &mut path)?;
        let crdt_text = text.get_string(&txn);
        let chars = char_offset(&crdt_text, offset.index);

        // The same path in the model: a node's content starts one past its open token.
        let mut node = doc;
        let mut start = 0usize;
        for (index, type_name) in &path {
            if *index >= node.child_count() {
                return None;
            }
            start += (0..*index)
                .map(|i| node.child(i).node_size())
                .sum::<usize>();
            node = node.child(*index);
            if node.type_name() != type_name {
                return None;
            }
            start += 1;
        }
        if !node.is_textblock() || read_block(node).ok()?.text != crdt_text {
            return None;
        }
        Some(Pos(start + chars))
    }
}

/// The live textblock `text` whose branch is `target`, searched depth-first under
/// `list`, recording the `(child index, node type)` of every node on the way to it.
fn find_text<T: ReadTxn>(
    txn: &T,
    list: &ArrayRef,
    target: BranchPtr,
    path: &mut Vec<(usize, String)>,
) -> Option<TextRef> {
    for i in 0..list.len(txn) {
        let Some(map) = child_map(txn, list, i) else {
            continue;
        };
        let Some(type_name) = node_type(txn, &map) else {
            continue;
        };
        path.push((i as usize, type_name));
        if let Some(text) = block_text(txn, &map) {
            if BranchPtr::from(AsRef::<Branch>::as_ref(&text)) == target {
                return Some(text);
            }
        } else if let Some(children) = node_content(txn, &map)
            && let Some(text) = find_text(txn, &children, target, path)
        {
            return Some(text);
        }
        path.pop();
    }
    None
}

/// The char offset of UTF-16 offset `u16` inside `s`: the inverse of `u16_offset`.
/// An offset inside a surrogate pair (which yrs does not produce for an item boundary)
/// rounds up to the next char; one past the end clamps to the end.
fn char_offset(s: &str, u16: u32) -> usize {
    let mut units = 0u32;
    for (i, c) in s.chars().enumerate() {
        if units >= u16 {
            return i;
        }
        units += c.len_utf16() as u32;
    }
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::char_offset;
    use crate::projection::u16_offset;

    #[test]
    fn char_and_utf16_offsets_are_inverse() {
        let s = "a😀b😀c";
        for chars in 0..=s.chars().count() {
            assert_eq!(char_offset(s, u16_offset(s, chars)), chars, "at {chars}");
        }
        // Inside the first surrogate pair (units 1..3): rounds up to after the emoji.
        assert_eq!(char_offset(s, 2), 2);
        assert_eq!(char_offset(s, 99), 5);
    }
}
