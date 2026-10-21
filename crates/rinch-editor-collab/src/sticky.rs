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
//!   The projection has no "move": joining two blocks deletes the second block's
//!   `Text` and re-inserts its characters into the first, and splitting a block deletes
//!   the tail from one `Text` and inserts it into a new one (see
//!   [`crate::project`]). A sticky index on the moved characters then points into a
//!   deleted `Text` (joined) or at a tombstone at the split point (split, which
//!   resolves to the end of the first half).
//! * **An edit next to its block** can end it or move it even though the block's own
//!   text did not change, at the top level and inside a list, a quote or a cell alike.
//!   The projection finds the unchanged blocks of a level by node *identity* (#1240),
//!   and a split or a join rebuilds both nodes it touches, so the untouched paragraph
//!   is rewritten through its neighbour's `Text` (tracked in #917). The exception is a
//!   level the edit kept **nothing** of — the block was its level's only child, so
//!   both of its new nodes are new: such a level is matched by value, and an Enter at
//!   the start of that block keeps its indexes. Concretely, where the level keeps a
//!   sibling:
//!   - **Enter at the start of a paragraph** (a new empty paragraph above it): every
//!     index into that paragraph resolves into the **new empty paragraph**, a wrong
//!     position and not `None`.
//!   - **Backspace that deletes an empty line above a paragraph**: every index into
//!     that paragraph resolves to `None`.
//!   - **Toggling a bullet list on a paragraph, or wrapping it in a quote** (the
//!     node's kind changes, so the block is replaced whole): `None`. Lifting it out of
//!     a quote or a list is the same replace.

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

use crate::table::{cell_maps, is_table_map};
use yrs::branch::{Branch, BranchID, BranchPtr};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Array, ArrayRef, Assoc, GetString, IndexedSequence, MapRef, Out, ReadTxn, StickyIndex, TextRef,
    Transact,
};

use rinch_editor_core::{Node, Pos, Schema};

use crate::error::Result;
use crate::projection::{
    CollabDoc, block_text, map_is_void, node_content, node_type, read_block, u16_offset,
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
    /// The index's own block is gone (deleted, joined into another block, replaced by
    /// a list toggle, …), so [`Self::pos`] is the nearest place in a surviving
    /// textblock to where that block stood, not the character the index was made for.
    /// `false` when the index resolved to its character (or, for a deleted character,
    /// to where that character was inside its live block).
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
    /// "move", so a block **joined** into the one before it ends it too, and a
    /// **split** before it leaves it at the split point. At the top level, Enter at
    /// the start of its paragraph moves it into the new empty paragraph above, and
    /// deleting an empty line above its paragraph or toggling a list on it ends it
    /// (tracked in #917): the projection finds unchanged top-level blocks by node
    /// identity, and a split or join rebuilds both nodes it touches. Inside a list or
    /// a quote the comparison is structural and the same Enter keeps the index.
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
    /// The bytes start with exactly the plain yrs [`StickyIndex`] (v1) that
    /// [`Self::sticky_index`] documents, with `assoc`'s association, so an app can still
    /// resolve them with yrs alone ([`StickyIndex::decode_v1`] reads the index and
    /// ignores what follows). A trailer follows it, because the CRDT forgets which
    /// block a deleted block's characters were in (yrs garbage-collects them with
    /// it), while the block's own entry in its parent array stays there as a
    /// tombstone: the trailer is `b"RS"`, a version byte (`1`), a count byte, then for
    /// each array on the way from the textblock up to the `content` root, innermost
    /// first, a big-endian `u16` length and a v1 yrs [`StickyIndex`] on that array,
    /// [`Assoc::After`], naming the block's (or its container's) entry there. A table's
    /// rows and cells are not arrays and have no entry; the table itself has one in
    /// the array that holds it.
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
        let model_text = read_block(block).ok()?.text;

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
        let crdt_text = text.get_string(&txn);
        if crdt_text != model_text {
            return None;
        }

        let at = u16_offset(&crdt_text, rp.parent_offset());
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
            bytes.push(u8::try_from(entries.len()).ok()?);
            for entry in entries.iter().rev() {
                let encoded = entry.encode_v1();
                bytes.extend_from_slice(&u16::try_from(encoded.len()).ok()?.to_be_bytes());
                bytes.extend_from_slice(&encoded);
            }
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
        let sticky = StickyIndex::decode_v1(bytes).ok()?;
        let txn = self.doc.transact();
        resolve_text_index(&txn, &self.content, doc, &sticky)
    }

    /// Where the sticky index `bytes` (from [`Self::sticky_index_with`] or
    /// [`Self::sticky_index`], on this replica or any other) points now in `doc`, the
    /// model document this CRDT projects, with the path of the textblock it is in.
    ///
    /// While its block lives this is [`Self::resolve_sticky`]'s answer, with
    /// [`StickyLocation::displaced`] `false`. When its block is gone — deleted,
    /// joined into the block before it (the projection has no move), or replaced (a
    /// list toggled on it) — an index made by [`Self::sticky_index_with`] resolves to
    /// the place that block stood, at the innermost level whose array still holds its
    /// entry, and `displaced` is `true`: the end of the last textblock before that
    /// place, or the start of the first one after it when there is none before. When
    /// the entry still lives (the block's table row went, the table did not), the
    /// start of the first textblock in that block is taken instead. An index from
    /// [`Self::sticky_index`] carries no trailer, so its deleted block gives `None`.
    ///
    /// `None` also when the bytes do not decode, when an item they name is not merged
    /// here yet (a peer's edit), or when `doc` is not the document this CRDT projects.
    pub fn locate_sticky(&self, doc: &Node, bytes: &[u8]) -> Option<StickyLocation> {
        let sticky = StickyIndex::decode_v1(bytes).ok()?;
        let entries = decode_trailer(&bytes[sticky.encode_v1().len().min(bytes.len())..]);
        let txn = self.doc.transact();
        if let Some(pos) = resolve_text_index(&txn, &self.content, doc, &sticky) {
            return Some(location(doc, pos, false));
        }
        // An item this replica has not merged yet is not a deleted one: wait for it.
        let known = |s: &StickyIndex| {
            s.id()
                .is_none_or(|id| txn.state_vector().get(&id.client) > id.clock)
        };
        if !known(&sticky) {
            return None;
        }
        for entry in &entries {
            if !known(entry) {
                return None;
            }
            let Some(offset) = entry.get_offset(&txn) else {
                // Collected with a deleted container: the next level out.
                continue;
            };
            let mut path = Vec::new();
            let Some(Hit::Array(list)) = find_branch(&txn, &self.content, offset.branch, &mut path)
            else {
                continue;
            };
            // The raw index counts void containers too, which the model does not have.
            let mut visible = 0usize;
            let mut alive = false;
            for (i, child) in list.iter(&txn).enumerate() {
                let Out::YMap(map) = child else { continue };
                if i as u32 >= offset.index {
                    alive = entry.id().is_some_and(|id| {
                        AsRef::<Branch>::as_ref(&map).id() == BranchID::Nested(*id)
                    });
                    break;
                }
                if !map_is_void(&txn, &map) {
                    visible += 1;
                }
            }
            let (owner, start) = model_node_at(doc, &path)?;
            let boundary = start
                + (0..visible.min(owner.child_count()))
                    .map(|i| owner.child(i).node_size())
                    .sum::<usize>();
            let pos = snap_to_textblock(doc, boundary, alive)?;
            return Some(location(doc, pos, true));
        }
        None
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
        let doc = self.to_doc(schema)?;
        Ok(self.locate_sticky(&doc, bytes))
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

/// The yrs indexes of a [`CollabDoc::sticky_index_with`] trailer, innermost first;
/// empty for no trailer or one that does not decode.
fn decode_trailer(mut rest: &[u8]) -> Vec<StickyIndex> {
    let Some(([m0, m1, version, count], tail)) = rest.split_first_chunk::<4>() else {
        return Vec::new();
    };
    if [*m0, *m1] != TRAILER_MAGIC || *version != TRAILER_VERSION {
        return Vec::new();
    }
    rest = tail;
    let mut entries = Vec::new();
    for _ in 0..*count {
        let Some((len, tail)) = rest.split_first_chunk::<2>() else {
            return Vec::new();
        };
        let len = u16::from_be_bytes(*len) as usize;
        if tail.len() < len {
            return Vec::new();
        }
        let Ok(entry) = StickyIndex::decode_v1(&tail[..len]) else {
            return Vec::new();
        };
        entries.push(entry);
        rest = &tail[len..];
    }
    entries
}

/// The model position the text index `sticky` names, if it is in the text of a live
/// textblock that `doc` mirrors.
fn resolve_text_index<T: ReadTxn>(
    txn: &T,
    content: &ArrayRef,
    doc: &Node,
    sticky: &StickyIndex,
) -> Option<Pos> {
    let offset = sticky.get_offset(txn)?;
    let mut path: Vec<(usize, String)> = Vec::new();
    let Hit::Text(text) = find_branch(txn, content, offset.branch, &mut path)? else {
        return None;
    };
    let crdt_text = text.get_string(txn);
    let chars = char_offset(&crdt_text, offset.index);
    let (node, start) = model_node_at(doc, &path)?;
    if !node.is_textblock() || read_block(node).ok()?.text != crdt_text {
        return None;
    }
    Some(Pos(start + chars))
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

/// The textblock position nearest `boundary`, a position between nodes: the start
/// of the first textblock at or after it when `forward`, otherwise the end of the
/// last one before it; the other side when there is none on the side asked for.
fn snap_to_textblock(doc: &Node, boundary: usize, forward: bool) -> Option<Pos> {
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
    let before = blocks
        .iter()
        .rev()
        .find(|(_, end)| *end <= boundary)
        .map(|(_, end)| *end);
    let after = blocks
        .iter()
        .find(|(start, _)| *start >= boundary)
        .map(|(start, _)| *start);
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

/// What [`find_branch`] found.
enum Hit {
    /// The `text` of a live textblock.
    Text(TextRef),
    /// A live content array (the `content` root, or a container's or cell's).
    Array(ArrayRef),
}

/// The live textblock `text` or content array whose branch is `target`, searched
/// depth-first under `list` (itself included), recording the `(child index, node
/// type)` of every node on the way to it.
fn find_branch<T: ReadTxn>(
    txn: &T,
    list: &ArrayRef,
    target: BranchPtr,
    path: &mut Vec<(usize, String)>,
) -> Option<Hit> {
    if BranchPtr::from(AsRef::<Branch>::as_ref(list)) == target {
        return Some(Hit::Array(list.clone()));
    }
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
                return Some(Hit::Text(text));
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
                    if let Some(hit) = find_branch(txn, &children, target, path) {
                        return Some(hit);
                    }
                    path.pop();
                }
                path.pop();
            }
        } else if let Some(children) = node_content(txn, &map)
            && let Some(hit) = find_branch(txn, &children, target, path)
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
