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
//! index of this projection is (`OffsetKind::Utf16`) — **but only if the resolver's
//! own `yrs::Doc` was built for it**. The offset kind belongs to each `Doc`, not to
//! the bytes, and [`yrs::Doc::new`] defaults to `OffsetKind::Bytes`, under which
//! `get_offset` answers a number in neither unit once the text has been edited (it
//! mixes the clock's UTF-16 units with the byte lengths of the items to the left).
//! Build the resolver's replica as the adapter builds its own:
//!
//! ```
//! use yrs::{Doc, OffsetKind, Options};
//! let doc = Doc::with_options(Options {
//!     offset_kind: OffsetKind::Utf16,
//!     ..Default::default()
//! });
//! # let _ = doc;
//! ```
//!
//! [`CollabDoc::sticky_index`]'s association is [`Assoc::After`] (the index sticks to the character that
//! *starts* at the position), except at the very end of a text, where there is no such
//! character and it is [`Assoc::Before`] (it sticks to the last one). An empty text has
//! neither, and yrs then names the `Text` itself (`IndexScope::Nested`), which resolves
//! to its start.
//! [`CollabDoc::sticky_index_with`] takes the association ([`StickyAssoc`]).
//!
//! ## What a sticky index survives, and what it does not
//!
//! * Any insert or delete elsewhere, local or merged from a peer: it moves with its
//!   character.
//! * Its own character deleted: it resolves to the place where that character was
//!   (yrs keeps the tombstone), which is the neighbouring position.
//! * Its **whole block deleted**, or its character **moved to another block**: `None`
//!   from [`CollabDoc::resolve_sticky`]. An index made by [`CollabDoc::sticky_index_with`]
//!   carries, after the yrs bytes, an index on its block's entry in each parent array
//!   (a deleted block's characters are garbage-collected with it, so nothing else
//!   says where they were, but its entry stays in a live parent array as a
//!   tombstone), and [`CollabDoc::locate_sticky`] resolves it to the place the block
//!   stood, marked displaced.
//!   The projection has no "move": joining two blocks deletes one block's `Text` and
//!   re-inserts its characters into the other (the one holding more of the joined
//!   text keeps its own), and splitting a block deletes its shorter half from the
//!   `Text` and inserts it into a new one (the tail, on a tie; see
//!   `write_child_diff`, #1850). A sticky index on the moved
//!   characters then points into a deleted `Text` (joined) or at a tombstone at the
//!   split point (split, which resolves to the edge of the kept half).
//! * **An edit next to its block** can end it even though the block's own text did
//!   not change, at the top level and inside a list, a quote or a cell alike: toggling
//!   a bullet list on a paragraph, or wrapping it in a quote (the node's kind changes,
//!   so the block is replaced whole), gives `None`; lifting it out of a quote or a list
//!   is the same replace. Enter at the start of a paragraph (a new empty paragraph
//!   above it) and Backspace deleting an empty line above it keep its indexes: the
//!   projection inserts or deletes the empty block and leaves the paragraph's `Text`
//!   in place (#1850, #917).

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
//! A table's rows and cells are not content arrays in the CRDT: the walk reads them
//! off the table's grid (`table::cell_maps`), in the model's order. A **filler** cell
//! (one the read supplies for a slot no cell covers, which the CRDT does not hold) has
//! no text behind it, so a position in one has no sticky index.
//!
//! "Index for index" counts **visible** children: a container emptied by concurrent
//! deletions stays in the CRDT but reads as absent (see `projection::is_void`), so both
//! walks skip it, or every index after it would name the wrong block.
//!
//! The one block with no CRDT behind it is the starter paragraph of a CRDT holding
//! zero blocks (see [`CollabDoc::to_doc`]): positions in it have no sticky index.

use crate::table::{all_cell_maps, cell_maps, is_table_map};
use std::collections::{HashMap, HashSet};

use yrs::branch::{Branch, BranchID, BranchPtr};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Array, ArrayRef, Assoc, ID, IndexedSequence, MapRef, Out, ReadTxn, StickyIndex, TextRef,
    Transact,
};

use rinch_editor_core::{Node, Pos, Schema};

use crate::error::Result;
use crate::projection::{
    CollabDoc, block_text, map_is_void, model_view, node_content, node_type, u16_offset,
};

/// Which character a sticky index holds on to, at the position it was made for.
///
/// For a **range** (a comment's span, a highlight), make two indexes: the start with
/// [`StickyAssoc::After`] and the end with [`StickyAssoc::Before`]. Text typed at
/// either edge then lands outside the range, and each end resolves on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StickyAssoc {
    /// The character that starts at the position (yrs [`Assoc::After`]): text inserted
    /// at the position lands before the index. At the very end of a text, where there
    /// is no such character, the last one is held instead ([`Assoc::Before`]). The
    /// default, and what [`CollabDoc::sticky_index`] uses.
    #[default]
    After,
    /// The character that ends at the position (yrs [`Assoc::Before`]): text inserted
    /// at the position lands after the index. At the very start of a text yrs names the
    /// text itself, which resolves to its start.
    Before,
}

/// Where a sticky index points now: [`CollabDoc::locate_sticky`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StickyLocation {
    /// The model position, always inside a textblock.
    pub pos: Pos,
    /// The child indices from the document root to the textblock holding [`Self::pos`]:
    /// `path[0]` is the top-level block's index.
    pub path: Vec<usize>,
    /// The index's own block is gone from the model (deleted, joined into another
    /// block, replaced by a list toggle or an undone deletion, in a table cell whose
    /// row or column was deleted), so [`Self::pos`] is the nearest place in a surviving
    /// textblock to where that block stood, not the character the index was made for.
    /// `false` when the index resolved to its character (or, for a deleted character,
    /// to where that character was inside its live block).
    /// Never `true` for a block still in the CRDT that the model cannot show (a table
    /// too large to read): that index is not located at all.
    pub displaced: bool,
}

/// The first two bytes of the trailer [`CollabDoc::sticky_index_with`] appends after
/// the yrs bytes, then its version.
const TRAILER_MAGIC: [u8; 2] = *b"RS";
const TRAILER_VERSION: u8 = 1;

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
    /// UTF-16 offset into that text **provided the resolver's `yrs::Doc` was built
    /// with `Options { offset_kind: OffsetKind::Utf16, .. }`**. The offset kind is per
    /// `Doc` and not in the bytes, and a default [`yrs::Doc::new`] (`OffsetKind::Bytes`)
    /// gives an index in neither unit once the text has been edited.
    ///
    /// It follows its character through any edit elsewhere; a deleted character
    /// resolves to where it was; it ends with its block. The projection has no
    /// "move", so a block whose text a **join** writes into the other ends it too,
    /// and a **split** that writes its half anew leaves it at the split point. Enter at the start of its
    /// paragraph and deleting an empty line above it keep it (#1850); toggling a list
    /// on its paragraph ends it.
    ///
    /// An index that must outlive its block is made by [`Self::sticky_index_with`],
    /// which also takes the association.
    ///
    /// `doc` must be the model document this CRDT projects (the editor's current
    /// document, or [`Self::to_doc`] of a loaded replica): the projection mirrors the
    /// model's tree index for index, which is how the textblock is found. `None` when:
    ///
    /// * `pos` is out of range, or not inside a textblock (between blocks, say);
    /// * the textblock has no CRDT behind it: the starter paragraph of an empty CRDT,
    ///   or a `doc` that is not the one this CRDT projects (a node type or the
    ///   block's text differs along the way).
    pub fn sticky_index(&self, doc: &Node, pos: Pos) -> Option<Vec<u8>> {
        self.make_sticky(doc, pos, StickyAssoc::After, false)
    }

    /// A sticky index of model position `pos` of `doc` that holds the character
    /// `assoc` names and **survives the deletion of its block**:
    /// [`Self::locate_sticky`] then resolves it to the nearest surviving place, marked
    /// [`StickyLocation::displaced`].
    ///
    /// **Treat the bytes as opaque**: store them, copy them, compare them for equality,
    /// and hand them back to [`Self::locate_sticky`] (or the session's and the editor
    /// handle's equivalents); do not build or edit them. They start with exactly the
    /// plain yrs [`StickyIndex`] (v1) that [`Self::sticky_index`] documents, with
    /// `assoc`'s association, so an app can still resolve the *character* with yrs
    /// alone: [`StickyIndex::decode_v1`] reads the index and, in the yrs this crate
    /// builds with, ignores what follows (pinned by a test, since it is yrs's behaviour
    /// and not a yrs promise). A trailer follows, because the CRDT forgets which block
    /// a deleted block's characters were in (yrs garbage-collects them with it), while
    /// the block's own entry in its parent array stays there as a tombstone. Version 1
    /// of the trailer is `b"RS"`, a version byte (`1`), an unsigned LEB128 count, then
    /// for each array on the way from the textblock up to the `content` root,
    /// innermost first, an unsigned LEB128 length and a v1 yrs [`StickyIndex`] on that
    /// array, [`Assoc::After`], naming the block's (or its container's) entry there;
    /// and last a big-endian 32-bit FNV-1a hash of every byte before it, the yrs index
    /// included. [`Self::locate_sticky`] refuses (`None`) bytes whose trailer does not
    /// check out: a wrong magic, version or hash, a malformed entry, or entries that do
    /// not form the chain from the block up to the root. The hash is a 32-bit check
    /// against accidental damage, not tamper-proofing: bytes edited on purpose with the
    /// hash recomputed can name another place.
    ///
    /// The bytes name CRDT items, not text: they resolve in any replica grown from the
    /// same CRDT (a snapshot loaded elsewhere included) and in no other, and anything
    /// that writes the document as new items (a load while collaborating, a new host
    /// session from the model, the re-base that ends an outbound stall) loses them. A table's rows and cells are
    /// not arrays and have no entry; the table itself has one in the array that holds
    /// it.
    ///
    /// `None` exactly when [`Self::sticky_index`] is `None`.
    pub fn sticky_index_with(&self, doc: &Node, pos: Pos, assoc: StickyAssoc) -> Option<Vec<u8>> {
        self.make_sticky(doc, pos, assoc, true)
    }

    fn make_sticky(
        &self,
        doc: &Node,
        pos: Pos,
        assoc: StickyAssoc,
        trailer: bool,
    ) -> Option<Vec<u8>> {
        let rp = doc.resolve(pos).ok()?;
        let block = rp.parent();
        if !block.is_textblock() {
            return None;
        }

        let txn = self.doc.transact();
        // Walk the child indices from the root to the textblock, which name the same
        // nodes in the projection.
        //
        // A table is the one node whose children are not a content array: its rows and
        // cells are read off the table's grid (`table::cell_maps`), and a filler cell
        // (one the CRDT does not hold) has no text to stick to.
        enum Level {
            List(ArrayRef),
            Table(Vec<Vec<Option<MapRef>>>),
            Row(Vec<Option<MapRef>>),
        }
        let mut level = Level::List(self.content.clone());
        let mut text: Option<TextRef> = None;
        // The entries in their parent arrays, outermost first.
        let mut entries: Vec<StickyIndex> = Vec::new();
        for depth in 0..rp.depth() {
            let index = rp.index(depth);
            let model_node = rp.node(depth + 1);
            let map = match level {
                Level::List(list) => {
                    let (map, raw) = visible_child_at(&txn, &list, index)?;
                    if trailer {
                        entries.push(list.sticky_index(&txn, raw, Assoc::After)?);
                    }
                    map
                }
                Level::Table(rows) => {
                    if model_node.type_name() != "table_row" {
                        return None;
                    }
                    level = Level::Row(rows.into_iter().nth(index)?);
                    continue;
                }
                Level::Row(cells) => cells.into_iter().nth(index)??,
            };
            if node_type(&txn, &map)? != model_node.type_name() {
                return None;
            }
            if depth + 1 == rp.depth() {
                text = Some(block_text(&txn, &map)?);
                break;
            }
            level = if is_table_map(&txn, &map) {
                Level::Table(cell_maps(&txn, &map).ok()?)
            } else {
                Level::List(node_content(&txn, &map)?)
            };
        }
        let text = text?;
        // The block as the model reads it: a converted break or a dropped atom (#1227)
        // makes a model offset name another stored offset.
        let (crdt_text, view) = model_view(&txn, &text, block)?;
        let at = u16_offset(&crdt_text, view.stored_at(rp.parent_offset()));
        let sticky = match assoc {
            // yrs refuses `After` at the end of a text (there is no character there to
            // stick to), including the only offset of an empty one; `Before` then
            // sticks to the last character, or names the `Text` itself when there is
            // none.
            StickyAssoc::After => text
                .sticky_index(&txn, at, Assoc::After)
                .or_else(|| text.sticky_index(&txn, at, Assoc::Before))?,
            // `Before` at offset 0 names the `Text` itself (its start).
            StickyAssoc::Before => text.sticky_index(&txn, at, Assoc::Before)?,
        };
        let mut bytes = sticky.encode_v1();
        if trailer {
            bytes.extend_from_slice(&TRAILER_MAGIC);
            bytes.push(TRAILER_VERSION);
            write_uvarint(&mut bytes, entries.len() as u64);
            for entry in entries.iter().rev() {
                let encoded = entry.encode_v1();
                write_uvarint(&mut bytes, encoded.len() as u64);
                bytes.extend_from_slice(&encoded);
            }
            let hash = fnv1a(&bytes);
            bytes.extend_from_slice(&hash.to_be_bytes());
        }
        Some(bytes)
    }

    /// Where the sticky index `bytes` (from [`Self::sticky_index`], on this replica or
    /// any other) points now, as a model position of `doc`, the model document this
    /// CRDT projects.
    ///
    /// `None` when the bytes do not decode, when the item they name is unknown here
    /// (a peer's edit not merged yet) or garbage-collected, when it is not in the text
    /// of a **live** textblock (its block was deleted, or it names something that is
    /// not a textblock's text at all), or when `doc` is not the document this CRDT
    /// projects. [`Self::locate_sticky`] answers a deleted block's index too.
    pub fn resolve_sticky(&self, doc: &Node, bytes: &[u8]) -> Option<Pos> {
        let (sticky, _) = decode_yrs_index(bytes)?;
        let txn = self.doc.transact();
        resolve_text_index(&txn, &self.content, doc, &sticky).map(|(pos, _)| pos)
    }

    /// Where the sticky index `bytes` (from [`Self::sticky_index_with`] or
    /// [`Self::sticky_index`], on this replica or any other) points now in `doc`, the
    /// model document this CRDT projects, with the path of the textblock it is in.
    ///
    /// While its character's block is in the CRDT this is [`Self::resolve_sticky`]'s
    /// answer, with [`StickyLocation::displaced`] `false`. When the block is gone from
    /// the CRDT — deleted, joined into the block before it (the projection has no
    /// move), or replaced (a list toggled on it, a deletion undone: both write the
    /// block's text as new characters, so the index stays displaced afterwards) — an
    /// index made by [`Self::sticky_index_with`] resolves to the place that block
    /// stood, at the innermost level whose array still holds its entry, and
    /// `displaced` is `true`: the end of the last textblock before that place, or the
    /// start of the first one after it when there is none before. When the entry
    /// still lives (the block's table row went, the table did not), the start of the
    /// first textblock in that block is taken instead. An index from
    /// [`Self::sticky_index`] carries no trailer, so its deleted block gives `None`.
    ///
    /// `None` also when the bytes do not decode or their trailer does not check out
    /// (see [`Self::sticky_index_with`]), when an item they name is not merged here yet
    /// (a peer's edit), when the block is in the CRDT but the model cannot show it (a
    /// table too large to read, shown as a placeholder), or when `doc` is not the
    /// document this CRDT projects. `displaced` is never `true` for a block that is
    /// still in the CRDT.
    ///
    /// So `None` means "not locatable now", which can change (an unmerged edit, a
    /// placeholder table, a stalled or poisoned session at the session level, a
    /// mismatched `doc`) or not (damaged or foreign bytes); `displaced` means the
    /// block is gone.    ///
    /// One call searches the CRDT until it finds the block, and reads all of it when
    /// the block is gone; [`Self::locate_sticky_many`] locates any number of indexes
    /// in one read.
    pub fn locate_sticky(&self, doc: &Node, bytes: &[u8]) -> Option<StickyLocation> {
        // The common case first, with no index of the whole CRDT: the character is in
        // a block the model shows, found by a search that stops there.
        let (primary, entries) = parse_anchor(bytes)?;
        {
            let txn = self.doc.transact();
            if let Some((pos, owner)) = resolve_text_index(&txn, &self.content, doc, &primary) {
                let chained = entries.first().is_none_or(|e| e.id().copied() == owner);
                return chained.then(|| location(doc, pos, false));
            }
        }
        self.locate_sticky_many(doc, &[bytes]).pop().flatten()
    }

    /// [`Self::locate_sticky`] for every index of `anchors`, in order, in one walk of
    /// the CRDT and the model: for an app that resolves many anchors at once (every
    /// comment of a document, every layout).
    pub fn locate_sticky_many(&self, doc: &Node, anchors: &[&[u8]]) -> Vec<Option<StickyLocation>> {
        let txn = self.doc.transact();
        let index = LiveIndex::build(&txn, &self.content);
        let mut textblocks = None;
        anchors
            .iter()
            .map(|bytes| locate_one(&txn, &index, doc, &mut textblocks, bytes))
            .collect()
    }

    /// [`Self::locate_sticky`] without an editor: reads this replica's model document
    /// ([`Self::to_doc`]) and locates `bytes` in it. For a server or a tool holding
    /// only the CRDT bytes: `CollabDoc::load(&crdt)?.locate_sticky_in(&schema, &sticky)`.
    ///
    /// `Err` when the replica does not read as a document of `schema`.
    pub fn locate_sticky_in(
        &self,
        schema: &Schema,
        bytes: &[u8],
    ) -> Result<Option<StickyLocation>> {
        Ok(self
            .locate_sticky_many_in(schema, &[bytes])?
            .pop()
            .flatten())
    }

    /// [`Self::locate_sticky_many`] without an editor, as [`Self::locate_sticky_in`].
    pub fn locate_sticky_many_in(
        &self,
        schema: &Schema,
        anchors: &[&[u8]],
    ) -> Result<Vec<Option<StickyLocation>>> {
        let doc = self.to_doc(schema)?;
        Ok(self.locate_sticky_many(&doc, anchors))
    }
}

/// The location of `pos`, a position inside a textblock of `doc`.
fn location(doc: &Node, pos: Pos, displaced: bool) -> StickyLocation {
    let path = doc
        .resolve(pos)
        .map(|rp| (0..rp.depth()).map(|d| rp.index(d)).collect())
        .unwrap_or_default();
    StickyLocation {
        pos,
        path,
        displaced,
    }
}

/// Locate one index against the live index of the CRDT. `textblocks` is the model's
/// textblock ranges, built on first need and shared by a batch.
fn locate_one<T: ReadTxn>(
    txn: &T,
    index: &LiveIndex,
    doc: &Node,
    textblocks: &mut Option<Vec<(usize, usize)>>,
    bytes: &[u8],
) -> Option<StickyLocation> {
    let (primary, entries) = parse_anchor(bytes)?;
    let state = txn.state_vector();
    // An item this replica has not merged yet is not a deleted one: wait for it.
    let known = |s: &StickyIndex| s.id().is_none_or(|id| state.get(&id.client) > id.clock);
    if !known(&primary) {
        return None;
    }
    let entry_id = |k: usize| entries.get(k).and_then(|e| e.id()).copied();

    if let Some(offset) = primary.get_offset(txn) {
        if index.gone_texts.contains(&offset.branch) {
            // In a table cell the read no longer shows (its row or column deleted):
            // gone from the model, so the trailer says where it stood.
            if entries.is_empty() {
                return None;
            }
        } else {
            // The character's text is in the CRDT: its live answer or nothing. A text
            // the walk cannot reach (inside a table too large to read) is not a
            // deleted one, so it is never reported displaced.
            let found = index.texts.get(&offset.branch)?;
            if !entries.is_empty() && entry_id(0) != Some(found.owner) {
                return None;
            }
            let Some(ArrayInfo::Live { path, .. }) = index.arrays.get(&found.parent) else {
                return None;
            };
            let (parent, start) = model_node_at(doc, path)?;
            let (node, start) = model_node_at(parent, std::slice::from_ref(&found.step))
                .map(|(n, s)| (n, start + s))?;
            let (crdt_text, view) = model_view(txn, &found.text, node)?;
            let pos = Pos(start + view.before[char_offset(&crdt_text, offset.index)]);
            return Some(location(doc, pos, false));
        }
    }

    // The character's block is gone from the model: yrs garbage-collected both, or it
    // is in a hidden table cell. The trailer's entries say where the block stood.
    //
    // A level whose entry is still *alive* is only possible above a level whose array
    // the model does not show (a hidden cell inside a live table): a live container
    // holds its children's entries until they are deleted, and a deleted entry is a
    // tombstone in it, not collected. So an alive entry right above a collected one
    // (or as the first level, under a collected text) is a chain that does not hold.
    let mut below_gone = false;
    for (k, entry) in entries.iter().enumerate() {
        if !known(entry) {
            return None;
        }
        let Some(offset) = entry.get_offset(txn) else {
            // Collected with a deleted container: the next level out.
            below_gone = false;
            continue;
        };
        // The array this entry is in must belong to the next entry's node (the root
        // `content` for the outermost one): a chain from the block up to the root.
        match index.arrays.get(&offset.branch)? {
            ArrayInfo::Gone { owner } => {
                // A container emptied by concurrent deletions, or a hidden table cell:
                // absent from the model, so the place is a level further out.
                if Some(*owner) != entry_id(k + 1) {
                    return None;
                }
                below_gone = true;
            }
            ArrayInfo::Live { path, owner, items } => {
                if *owner != entry_id(k + 1) {
                    return None;
                }
                let at = (offset.index as usize).min(items.len());
                let alive = items
                    .get(at)
                    .is_some_and(|(id, _)| id.is_some() && id.as_ref() == entry.id());
                if alive && !below_gone {
                    return None;
                }
                // The raw index counts void containers too, which the model does not.
                let visible = items[..at].iter().filter(|(_, void)| !void).count();
                let (owner_node, start) = model_node_at(doc, path)?;
                let boundary = start
                    + (0..visible.min(owner_node.child_count()))
                        .map(|i| owner_node.child(i).node_size())
                        .sum::<usize>();
                let blocks = textblocks.get_or_insert_with(|| textblock_ranges(doc));
                let pos = snap_to_textblock(blocks, boundary, alive)?;
                return Some(location(doc, pos, true));
            }
        }
    }
    None
}

/// The live part of the CRDT, read once for a batch of indexes: every textblock text
/// and every content array the model shows, by branch, with their model paths.
struct LiveIndex {
    texts: HashMap<BranchPtr, TextInfo>,
    /// The texts of blocks in [`ArrayInfo::Gone`] arrays (a hidden table cell's): in the
    /// CRDT, not in the model, so gone as far as the model can tell.
    gone_texts: HashSet<BranchPtr>,
    arrays: HashMap<BranchPtr, ArrayInfo>,
}

struct TextInfo {
    text: TextRef,
    /// The content array the textblock is in, and its `(child index, node type)` there:
    /// its path is that array's path and this step.
    parent: BranchPtr,
    step: (usize, String),
    /// The textblock's map.
    owner: ID,
}

enum ArrayInfo {
    /// An array the model shows: `path` leads to the node owning it, `owner` is that
    /// node's map (`None` for the root `content`; the table's map for a cell's
    /// content), and `items` its entries in order, with whether each is void.
    Live {
        path: Vec<(usize, String)>,
        owner: Option<ID>,
        items: Vec<(Option<ID>, bool)>,
    },
    /// A content array the model does not show and that holds no block it could:
    /// a void container's (or one inside it), a hidden table cell's. `owner` is the
    /// container's map, or the table's for a cell.
    Gone { owner: ID },
}

fn branch_of(r: &impl AsRef<Branch>) -> BranchPtr {
    BranchPtr::from(r.as_ref())
}

fn map_id(map: &MapRef) -> Option<ID> {
    match AsRef::<Branch>::as_ref(map).id() {
        BranchID::Nested(id) => Some(id),
        BranchID::Root(_) => None,
    }
}

impl LiveIndex {
    fn build<T: ReadTxn>(txn: &T, content: &ArrayRef) -> LiveIndex {
        let mut index = LiveIndex {
            texts: HashMap::new(),
            gone_texts: HashSet::new(),
            arrays: HashMap::new(),
        };
        index.walk(txn, content, None, &mut Vec::new());
        index
    }

    /// Index `list` (owned by the node at `path`, whose map is `owner`) and everything
    /// under it. Child indices are *visible* indices, the model's.
    fn walk<T: ReadTxn>(
        &mut self,
        txn: &T,
        list: &ArrayRef,
        owner: Option<ID>,
        path: &mut Vec<(usize, String)>,
    ) {
        let mut items = Vec::new();
        let mut visible = 0usize;
        for child in list.iter(txn) {
            let Out::YMap(map) = child else {
                items.push((None, false));
                continue;
            };
            let id = map_id(&map);
            if map_is_void(txn, &map) {
                items.push((id, true));
                if let (Some(id), Some(children)) = (id, node_content(txn, &map)) {
                    self.mark_gone(txn, &children, id);
                }
                continue;
            }
            items.push((id, false));
            let index = visible;
            visible += 1;
            let (Some(id), Some(type_name)) = (id, node_type(txn, &map)) else {
                continue;
            };
            path.push((index, type_name));
            if let Some(text) = block_text(txn, &map) {
                self.texts.insert(
                    branch_of(&text),
                    TextInfo {
                        text,
                        parent: branch_of(list),
                        step: path.last().cloned().unwrap_or_default(),
                        owner: id,
                    },
                );
            } else if is_table_map(txn, &map) {
                // A table's rows and cells are its grid's, in model order; a filler has
                // no text and is skipped. A table too large to read is not indexed at
                // all: the model shows its placeholder, so nothing in it can be shown,
                // and nothing in it is gone either.
                let Ok(rows) = cell_maps(txn, &map) else {
                    path.pop();
                    continue;
                };
                for (i, row) in rows.into_iter().enumerate() {
                    path.push((i, "table_row".to_string()));
                    for (k, cell) in row.into_iter().enumerate() {
                        let Some(cell) = cell else { continue };
                        let (Some(cell_type), Some(children)) =
                            (node_type(txn, &cell), node_content(txn, &cell))
                        else {
                            continue;
                        };
                        path.push((k, cell_type));
                        self.walk(txn, &children, Some(id), path);
                        path.pop();
                    }
                    path.pop();
                }
                // The cells the read does not show (a deleted row or column, a cell a span
                // covers) hold no block the model has: like a void container's.
                for cell in all_cell_maps(txn, &map) {
                    if let Some(children) = node_content(txn, &cell)
                        && !self.arrays.contains_key(&branch_of(&children))
                    {
                        self.mark_gone(txn, &children, id);
                    }
                }
            } else if let Some(children) = node_content(txn, &map) {
                self.walk(txn, &children, Some(id), path);
            }
            path.pop();
        }
        self.arrays.insert(
            branch_of(list),
            ArrayInfo::Live {
                path: path.clone(),
                owner,
                items,
            },
        );
    }

    /// Record the content array of a void container (`owner`), or of a table cell the
    /// read does not show (`owner` the table), and every container's under it: the
    /// model shows none of them, and none holds a block it could show.
    fn mark_gone<T: ReadTxn>(&mut self, txn: &T, list: &ArrayRef, owner: ID) {
        self.arrays
            .insert(branch_of(list), ArrayInfo::Gone { owner });
        for child in list.iter(txn) {
            let Out::YMap(child) = child else { continue };
            if let Some(text) = block_text(txn, &child) {
                self.gone_texts.insert(branch_of(&text));
            } else if is_table_map(txn, &child) {
                // A table in a gone place: every cell it stores is gone too.
                if let Some(id) = map_id(&child) {
                    for cell in all_cell_maps(txn, &child) {
                        if let Some(children) = node_content(txn, &cell) {
                            self.mark_gone(txn, &children, id);
                        }
                    }
                }
            } else if let (Some(id), Some(children)) = (map_id(&child), node_content(txn, &child)) {
                self.mark_gone(txn, &children, id);
            }
        }
    }
}

// --- the byte format ---------------------------------------------------------------

/// Unsigned LEB128, as lib0 writes it.
fn write_uvarint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Read an unsigned LEB128 at `*at`, at most `max`; `None` past the input or `max`.
fn read_uvarint(bytes: &[u8], at: &mut usize, max: u64) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f).checked_shl(shift)?;
        if value > max {
            return None;
        }
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// 32-bit FNV-1a.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x0100_0193)
    })
}

/// The length of the v1 yrs [`StickyIndex`] at the start of `bytes`, checked by hand
/// before yrs decodes it: a scope this crate writes (an item, tag 0, or a text,
/// tag 2), a client id yrs accepts (53 bits: yrs asserts it in debug builds), a
/// 32-bit clock and a one-byte association. `None` for anything else.
fn yrs_index_len(bytes: &[u8]) -> Option<usize> {
    let mut at = 0;
    let tag = read_uvarint(bytes, &mut at, 2)?;
    if tag == 1 {
        return None;
    }
    read_uvarint(bytes, &mut at, (1u64 << 53) - 1)?;
    read_uvarint(bytes, &mut at, u64::from(u32::MAX))?;
    let assoc = *bytes.get(at)?;
    if assoc & 0x80 != 0 {
        return None;
    }
    Some(at + 1)
}

/// The yrs index at the start of `bytes`, decoded only once [`yrs_index_len`] has
/// checked it, and the length it takes.
fn decode_yrs_index(bytes: &[u8]) -> Option<(StickyIndex, usize)> {
    let len = yrs_index_len(bytes)?;
    let index = StickyIndex::decode_v1(&bytes[..len]).ok()?;
    Some((index, len))
}

/// The yrs index of `bytes` and its trailer's entries, innermost first (none for a
/// plain index); `None` when either does not check out.
fn parse_anchor(bytes: &[u8]) -> Option<(StickyIndex, Vec<StickyIndex>)> {
    let (primary, len) = decode_yrs_index(bytes)?;
    if len == bytes.len() {
        return Some((primary, Vec::new()));
    }
    let (body, hash) = bytes.split_last_chunk::<4>()?;
    if body.len() < len + 3 || fnv1a(body) != u32::from_be_bytes(*hash) {
        return None;
    }
    if body[len..len + 2] != TRAILER_MAGIC || body[len + 2] != TRAILER_VERSION {
        return None;
    }
    let mut at = len + 3;
    let count = read_uvarint(body, &mut at, body.len() as u64)?;
    let mut entries = Vec::new();
    for _ in 0..count {
        let entry_len = read_uvarint(body, &mut at, body.len() as u64)? as usize;
        let entry = body.get(at..at.checked_add(entry_len)?)?;
        // An entry names a block's entry in an array: an item, never a text.
        if entry.first() != Some(&0) {
            return None;
        }
        let (index, used) = decode_yrs_index(entry)?;
        if used != entry_len {
            return None;
        }
        entries.push(index);
        at += entry_len;
    }
    (at == body.len()).then_some((primary, entries))
}

/// The model position the text index `sticky` names, if it is in the text of a live
/// textblock that `doc` mirrors.
fn resolve_text_index<T: ReadTxn>(
    txn: &T,
    content: &ArrayRef,
    doc: &Node,
    sticky: &StickyIndex,
) -> Option<(Pos, Option<ID>)> {
    let offset = sticky.get_offset(txn)?;
    let mut path: Vec<(usize, String)> = Vec::new();
    let (text, owner) = find_text(txn, content, offset.branch, &mut path)?;
    let (node, start) = model_node_at(doc, &path)?;
    let (crdt_text, view) = model_view(txn, &text, node)?;
    let chars = view.before[char_offset(&crdt_text, offset.index)];
    Some((Pos(start + chars), owner))
}

/// The node of `doc` at `path` (child index and node type at each step) and the
/// position its content starts at: `None` when `doc` does not have that path.
fn model_node_at<'a>(doc: &'a Node, path: &[(usize, String)]) -> Option<(&'a Node, usize)> {
    let mut node = doc;
    let mut start = 0usize;
    for (index, type_name) in path {
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
        // A node's content starts one past its open token.
        start += 1;
    }
    Some((node, start))
}

/// The content ranges of every textblock of `doc`, in document order.
fn textblock_ranges(doc: &Node) -> Vec<(usize, usize)> {
    fn collect(node: &Node, start: usize, out: &mut Vec<(usize, usize)>) {
        let mut pos = start;
        for i in 0..node.child_count() {
            let child = node.child(i);
            if child.is_textblock() {
                out.push((pos + 1, pos + 1 + child.content_size()));
            } else if !child.is_leaf() {
                collect(child, pos + 1, out);
            }
            pos += child.node_size();
        }
    }
    let mut blocks = Vec::new();
    collect(doc, 0, &mut blocks);
    blocks
}

/// The textblock position nearest `boundary`, a position between nodes, among
/// `blocks` ([`textblock_ranges`]): the start of the first textblock at or after it
/// when `forward`, otherwise the end of the last one before it; the other side when
/// there is none on the side asked for.
fn snap_to_textblock(blocks: &[(usize, usize)], boundary: usize, forward: bool) -> Option<Pos> {
    // Textblocks do not overlap, so both their starts and their ends ascend.
    let first_after = blocks.partition_point(|(start, _)| *start < boundary);
    let after = blocks.get(first_after).map(|(start, _)| *start);
    let ends_before = blocks.partition_point(|(_, end)| *end <= boundary);
    let before = ends_before
        .checked_sub(1)
        .and_then(|i| blocks.get(i))
        .map(|(_, end)| *end);
    let pos = if forward {
        after.or(before)
    } else {
        before.or(after)
    };
    pos.map(Pos)
}

/// The map of **visible** child `index` of `list` and its raw index in the yrs array
/// (which counts void containers).
fn visible_child_at<T: ReadTxn>(txn: &T, list: &ArrayRef, index: usize) -> Option<(MapRef, u32)> {
    // One sequential pass: `ArrayRef::get` walks the array from its start, so asking it
    // for each child in turn would cost O(index²). Void containers are skipped, as
    // `projection::read_children` skips them.
    list.iter(txn)
        .enumerate()
        .filter_map(|(raw, child)| match child {
            Out::YMap(m) => Some((m, raw as u32)),
            _ => None,
        })
        .filter(|(m, _)| !map_is_void(txn, m))
        .nth(index)
}

/// The live textblock `text` whose branch is `target`, searched depth-first under
/// `list`, recording the `(child index, node
/// type)` of every node on the way to it.
fn find_text<T: ReadTxn>(
    txn: &T,
    list: &ArrayRef,
    target: BranchPtr,
    path: &mut Vec<(usize, String)>,
) -> Option<(TextRef, Option<ID>)> {
    // The index recorded is the child's *visible* index, the one the model uses: a
    // void container is not in the model (see `projection::is_void`), and holds no
    // text to find.
    // One sequential pass: `ArrayRef::get` walks the array from its start, so reading
    // each child by index would cost O(len²) per level.
    let mut visible = 0usize;
    for child in list.iter(txn) {
        let Out::YMap(map) = child else {
            continue;
        };
        if map_is_void(txn, &map) {
            continue;
        }
        let index = visible;
        visible += 1;
        let Some(type_name) = node_type(txn, &map) else {
            continue;
        };
        path.push((index, type_name));
        if let Some(text) = block_text(txn, &map) {
            if BranchPtr::from(AsRef::<Branch>::as_ref(&text)) == target {
                return Some((text, map_id(&map)));
            }
        } else if is_table_map(txn, &map) {
            // A table's rows and cells are its grid's, in model order; a filler has
            // no text and is skipped.
            // A table too large to read holds no text a model position can name (the
            // model sees its placeholder), so the walk steps over it.
            let rows = cell_maps(txn, &map).unwrap_or_default();
            for (i, row) in rows.into_iter().enumerate() {
                path.push((i, "table_row".to_string()));
                for (k, cell) in row.into_iter().enumerate() {
                    let Some(cell) = cell else { continue };
                    let (Some(cell_type), Some(children)) =
                        (node_type(txn, &cell), node_content(txn, &cell))
                    else {
                        continue;
                    };
                    path.push((k, cell_type));
                    if let Some(hit) = find_text(txn, &children, target, path) {
                        return Some(hit);
                    }
                    path.pop();
                }
                path.pop();
            }
        } else if let Some(children) = node_content(txn, &map)
            && let Some(hit) = find_text(txn, &children, target, path)
        {
            return Some(hit);
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
