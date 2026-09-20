//! [`CollabDoc`] — the yrs (Yjs) projection of an editor document.
//!
//! The CRDT is **not** the model (design §8). The model lives in `rinch-editor-core`;
//! this is a *projection* of it onto a CRDT so concurrent edits converge. The two are
//! kept byte-for-byte equivalent by the invariant **`model ≡ project(model)`**: every
//! local step is projected onto the CRDT ([`crate::project`]), every remote CRDT change
//! is rebuilt back into the model ([`crate::remote`]). Convergence then follows from
//! yrs's own convergence.
//!
//! ## Wire shape (rich-text projection)
//!
//! Every model node — at any depth — projects onto the **same** yrs `Map` shape;
//! nesting is just recursion on it. A node is *either* a block (carries a `text`) *or* a
//! container (carries a `content` array of child nodes):
//!
//! ```text
//! root Map "meta"                 // the projection-format marker (see `load`)
//!   "format" -> "rinch-editor-collab/yrs-1"
//!
//! root Array "content"            // a named root type, holding the top-level blocks
//!   Node (Map):
//!     "type"  -> string           // "paragraph" | "heading" | "bullet_list" | …
//!     "attrs" -> Map<str, Any>    // e.g. heading {"level": 2}, ordered_list {"start": 3}
//!     and then EXACTLY ONE of:
//!       "text"    -> Text         // a textblock's plain text
//!         (marks are the Text's own *formatting attributes*: the attribute key is
//!          the mark type name; its value is `true` for an attr-less mark or a
//!          `Map` of the mark's attrs. Per the Yjs convention a `null` value
//!          *removes* the format over a range.)
//!       "content" -> Array<Node>  // a container block's child nodes, recursively
//! ```
//!
//! A **leaf block atom** (`horizontal_rule` — a block-level node the schema gives no
//! content at all) is a block whose `text` is simply always **empty**: same `Map`, same
//! `text` key, an empty `Text` object. It needs no wire shape of its own, so a peer at
//! this same wire version reads it back as an ordinary node and the format tag does not
//! move. What keeps that empty text *empty* is [`reconcile_node`]: a node whose type
//! changes into (or out of) a shape with no text is **replaced**, never reconciled in
//! place, so a peer's concurrent typing cannot land in the `Text` of what has become an
//! atom. [`build_block`] refuses an atom carrying text loudly rather than dropping it,
//! which is the only thing left that a foreign writer could produce.
//!
//! An **inline atom** (`image`, `hard_break` — atomic and contentless, but living
//! *inside* a textblock's inline content) is one **U+FFFC** char in that block's `Text`
//! carrying one reserved formatting attribute:
//!
//! ```text
//!       "text" -> Text            // "look: \u{FFFC} and on"
//!         "@atom" -> Map          // over the U+FFFC char only:
//!                                 //   {"@type": "image", "src": "cat.png", …}
//! ```
//!
//! …which is the *same* shape as a mark with attrs, deliberately: the atom is a char
//! that happens to be formatted, so every path that already carries a formatted char
//! carries it — [`read_text_data`] reads it as a [`SpanMark`] with no case of its own,
//! [`splice_min`] moves it as text, [`resync_marks`] diffs it as formatting. Its own
//! marks (a link on an image) are ordinary spans over that same char. **Not** a yrs
//! embed, which is the obvious alternative and is still refused: an embed is opaque to
//! `Text::diff`'s string path, to the char/UTF-16 offset arithmetic and to the minimal
//! splice, so every one of those would need a second, parallel implementation — and the
//! model already gives an inline leaf exactly **one** position, which is exactly one
//! char.
//!
//! The pairing is checked both ways and neither half is an error on its own: a U+FFFC
//! with no attribute is text (a user can paste one), and the attribute over any other
//! char is ignored formatting — see [`is_atom_char`], which is also where the
//! concurrent-edit measurement that forces that second rule is written down.
//!
//! Wire-compatibly this is **additive**: [`FORMAT_TAG`] does not move. An older reader
//! meets the attribute as an unknown *mark name* and fails loud in [`marks_at`]
//! ("unknown mark type `@atom` in CRDT") rather than half-understanding the document —
//! which is the outcome a version bump would have bought, at the price of also locking
//! that reader out of every document with no atom in it.
//!
//! One `Text` per textblock with native formatting attributes over it is the
//! *rich-text* model — text and formatting merge independently, which is exactly the
//! "concurrent insert/format" convergence the milestone requires. A textblock keeps its
//! own `Text` object however deeply it is nested, so concurrent edits to *different*
//! list items are edits to *different* `Text` objects and merge without loss.
//!
//! ## Offsets: char in the model, UTF-16 in the CRDT
//!
//! `rinch-editor-core` positions are **Unicode scalars** (chars); yrs offers only
//! `OffsetKind::Bytes` and `OffsetKind::Utf16`, so the document is built with
//! [`OffsetKind::Utf16`] and every index crossing a `Text` boundary is converted
//! (`u16_offset`). Blocks are small, so the linear walk is cheaper than maintaining a
//! rope mirror. Getting this wrong is silent: yrs snaps an index that lands mid
//! surrogate pair to the nearest boundary rather than erroring, which is why the offset
//! tests use **two** astral characters (one cannot distinguish a correct conversion
//! from a snapped one).
//!
//! ## One transaction per entry point
//!
//! yrs cannot nest transaction acquisitions — a second `transact()`/`transact_mut()`
//! taken while one is live blocks on native and panics on wasm. So the root handle is
//! resolved once at construction (`get_or_insert_array` opens its own transaction) and
//! every public entry point opens **exactly one** transaction and threads it down; the
//! helpers below are free functions taking `&T: ReadTxn` or `&mut TransactionMut`
//! rather than methods that reach for a transaction of their own.
//!
//! ## The broadcast outbox
//!
//! Every committed transaction hands its *own* update to an `observe_update_v1`
//! subscription, which parks it in [`CollabDoc`]'s outbox for
//! [`CollabSession::save_incremental`](crate::CollabSession::save_incremental) to drain.
//! A transaction applying *foreign* bytes is tagged `ENGINE_APPLY_ORIGIN` and skipped:
//! those changes are already shared, so re-broadcasting them would echo. Locally
//! projected writes are therefore exactly the broadcasts.
//!
//! Relaying a peer's content onward is the **transport's** job, not the adapter's: a mesh
//! delivers to everyone directly, and a star hub forwards the raw delta bytes it received.
//!
//! The outbox exists because the obvious alternative — diffing against the state vector
//! of the last broadcast — cannot work here. `encode_diff_v1` writes the *complete*
//! delete set regardless of the target state vector, so once a document has seen any
//! deletion every delta re-carries the whole deletion history (growing without bound) and
//! a "nothing new" diff is no longer empty. A transaction's own update is constant-size.
//!
//! ## Scope (design A22)
//!
//! Supported: **flat text-blocks + marks** (`paragraph`/`heading`/`code_block`), the
//! **inline atoms** inside them (`image`/`hard_break` — one placeholder char each), the
//! **leaf block atoms** (`horizontal_rule` — a block-level node with no content at all),
//! and the **list containers** `bullet_list` / `ordered_list` / `list_item`, nested into
//! each other and around text-blocks to any depth.
//!
//! Everything else still **fails loud** with
//! [`CollabError::Unsupported`](crate::CollabError::Unsupported) — **never a silent
//! drop**: any other nested block (`blockquote`, `table`/`table_row`/cells,
//! `task_list`/`task_item`), and an embedded value in a block's text, which is not how
//! this projection writes an atom.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yrs::types::Attrs as YAttrs;
use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, ClientID, Doc, Map, MapPrelim, MapRef, OffsetKind, Options,
    Origin, Out, ReadTxn, StateVector, Subscription, Text, TextPrelim, TextRef, Transact,
    TransactionMut, Update,
};

use rinch_editor_core::{AttrValue, Attrs, Fragment, Mark, Node, NodeType, Schema};

use crate::error::{CollabError, Result};

/// yrs key/root names used by the projection.
const CONTENT: &str = "content";
const TYPE: &str = "type";
const ATTRS: &str = "attrs";
const TEXT: &str = "text";
/// The root map holding the projection-format marker, and its one key.
const META: &str = "meta";
const FORMAT: &str = "format";

/// The projection-format marker written into the `meta` root at creation and required
/// by [`CollabDoc::load`]. It identifies bytes as *this* crate's projection at *this*
/// wire version, which is what lets a legitimately empty document (zero content
/// blocks — see [`CollabDoc::load`]) be told apart from a foreign CRDT that merely
/// reinterprets as an empty `content` array.
///
/// Bump the trailing version when the wire shape changes incompatibly: an older peer
/// then refuses the bytes loudly instead of half-understanding them. (Playweft's
/// "wipe, don't convert" precedent — tag every blob, refuse an untagged one.)
const FORMAT_TAG: &str = "rinch-editor-collab/yrs-1";

/// The prefix every name the projection **reserves** inside a block's text begins with.
///
/// A yrs `Text`'s formatting attributes are keyed by *mark type name*, so an attribute
/// the projection adds of its own ([`ATOM_MARK`]) must be a name the schema can never
/// mint. Schema mark names and node attribute names are identifiers (`bold`, `link`,
/// `text_color`, `src`); `@` is not an identifier character in any of them, so a single
/// reserved leading `@` separates the two namespaces for good. Both directions are
/// guarded rather than trusted: [`read_block`] refuses an inline atom whose own attrs
/// carry a reserved key, and [`build_block`] refuses a schema that has minted a mark
/// type in the reserved namespace.
const RESERVED_PREFIX: char = '@';

/// The reserved formatting attribute that turns an [`ATOM_PLACEHOLDER`] char in a
/// block's text into an **inline atom** (`image`, `hard_break`).
///
/// Its value is the atom's attrs in [`encode_mark_value`]'s ordinary encoding, plus the
/// node type name under [`ATOM_TYPE`] — so it rides the wire as any other mark does and
/// [`read_text_data`] needs no case of its own for it.
const ATOM_MARK: &str = "@atom";

/// The key inside an [`ATOM_MARK`] value carrying the atom's **node type name**
/// (`"image"`). Reserved (see [`RESERVED_PREFIX`]) so it cannot collide with an attr of
/// the atom itself — an `image`'s `src`/`alt`/`title` sit in the same map.
const ATOM_TYPE: &str = "@type";

/// U+FFFC OBJECT REPLACEMENT CHARACTER — the one char an inline atom occupies in a
/// block's projected text, which is also the one model position it occupies
/// (`Node::node_size` of a leaf is 1). The same stand-in `remote::flat_units` already
/// uses when it measures a caret across a line holding an inline leaf.
const ATOM_PLACEHOLDER: char = '\u{FFFC}';

/// Origin tag for a yrs transaction that applies bytes received from a peer, as opposed to
/// one that projects a local edit. The update observer skips these: they are already
/// shared, so putting them in the outbox would echo them back.
///
/// Not to be confused with [`ORIGIN_REMOTE`](crate::ORIGIN_REMOTE), which is a *model*
/// `Transaction` meta key marking the editor-side transaction as remote-originated. Two
/// different mechanisms at two different layers.
pub(crate) const ENGINE_APPLY_ORIGIN: &str = "collabEngineApply";

/// Updates produced by locally-projected transactions, waiting to be broadcast.
///
/// `Arc<Mutex<_>>` rather than `Rc<RefCell<_>>` even though the adapter is single-threaded:
/// the outbox is captured by the update observer, which is held by [`CollabDoc`], so an
/// `Rc` here would make `CollabDoc` — and therefore `CollabSession` — `!Send`. A server
/// holds a session across an `.await`, so that must not happen; the bound is pinned by a
/// static assertion in `lib.rs`.
pub(crate) type Outbox = Arc<Mutex<Vec<Vec<u8>>>>;

/// Lock the outbox, recovering from a poisoned mutex rather than failing on it.
///
/// The guarded value is a plain queue of encoded updates with no invariant spanning it, so
/// a panic elsewhere while the lock was held cannot have left it inconsistent. Both
/// alternatives are worse: the update observer's signature cannot return an error, so it
/// would have to either drop a broadcast silently — the divergence class this adapter
/// exists to prevent — or panic in the middle of a yrs commit.
pub(crate) fn lock_outbox(outbox: &Outbox) -> std::sync::MutexGuard<'_, Vec<Vec<u8>>> {
    outbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One coalesced run of a single mark over a block's text (char offsets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpanMark {
    pub name: String,
    pub attrs: Attrs,
    pub start: usize,
    pub end: usize,
}

/// The plain text of a flat block plus the marks over it, ready to project. A **leaf
/// block atom** (`horizontal_rule`) is the degenerate case: the same struct with an
/// empty `text` and no marks — see [`is_leaf_block_atom`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockData {
    pub type_name: String,
    pub attrs: Attrs,
    pub text: String,
    pub marks: Vec<SpanMark>,
}

/// One projectable model node: either a flat block ([`BlockData`] — a text-block, or a
/// leaf block atom with empty text) or a container block (a list / list item) holding
/// child nodes recursively. Structural equality is
/// canonical (marks are sorted in [`read_block`] / [`read_text_data`]), so comparing two
/// `NodeData` trees is the same as comparing the model nodes they came from — the
/// child-list diff in [`reconcile_child_list`] relies on that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeData {
    /// A flat text-block (`paragraph`/`heading`/`code_block`): its `text` + marks — or
    /// a leaf block atom (`horizontal_rule`), which is the same thing with no text.
    Block(BlockData),
    /// A container block (`bullet_list`/`ordered_list`/`list_item`): its child nodes.
    Container {
        type_name: String,
        attrs: Attrs,
        children: Vec<NodeData>,
    },
}

impl NodeData {
    /// The node's type name.
    fn type_name(&self) -> &str {
        match self {
            NodeData::Block(b) => &b.type_name,
            NodeData::Container { type_name, .. } => type_name,
        }
    }

    /// The node's attributes.
    fn attrs(&self) -> &Attrs {
        match self {
            NodeData::Block(b) => &b.attrs,
            NodeData::Container { attrs, .. } => attrs,
        }
    }
}

/// The container block types the projection nests (design A22). A container carries a
/// `content` array of child nodes instead of a `text`. Deliberately a whitelist: every
/// other non-text-block node (`blockquote`, tables, `task_list`/`task_item`, atoms)
/// stays [`CollabError::Unsupported`] so an unsupported shape fails loud rather than
/// being silently mangled.
fn is_supported_container(type_name: &str) -> bool {
    matches!(type_name, "bullet_list" | "ordered_list" | "list_item")
}

/// A **leaf block atom** — a block-level node type that holds no content of its own and
/// is one opaque unit in the document (`horizontal_rule`, the scene break). It projects
/// as a [`NodeData::Block`] whose text is empty.
///
/// Deliberately a *predicate on the schema type*, not a name whitelist like
/// [`is_supported_container`]: "block-level, atomic, holds nothing" is exactly the shape
/// the empty-text projection is faithful to, so an app schema's own leaf block atom is
/// in scope for free, while the three conditions each keep something out —
///
/// * `is_block()` — the **inline** atoms (`image`, `hard_break`) are `is_atom() &&
///   is_leaf()` too, and are in scope by the *other* route ([`is_inline_atom`]): they
///   live inside a textblock's inline content, so they project as a char of its text
///   rather than as a block of their own.
/// * `is_atom()` — an opaque unit, which is what makes "no text, no children" its whole
///   content rather than an erasure of something.
/// * `is_leaf()` — "the content match accepts nothing" (the same source of truth as
///   `Node::node_size`), which is what rules out `blockquote`, tables and `task_item`:
///   they hold block content the projection would silently drop.
///
/// A textblock can be none of these (its content match accepts `text`), so the three
/// never overlap with [`Node::is_textblock`].
fn is_leaf_block_atom(typ: &NodeType) -> bool {
    typ.is_block() && typ.is_atom() && typ.is_leaf()
}

/// An **inline atom** — an atomic, contentless node type that lives *inside* a
/// textblock's inline content (`image`, `hard_break`). It projects as one
/// [`ATOM_PLACEHOLDER`] char in the block's text carrying an [`ATOM_MARK`] attribute.
///
/// The exact complement of [`is_leaf_block_atom`] on its first clause, and a predicate
/// on the schema type for the same reason: "inline, atomic, holds nothing" is the shape
/// the one-char projection is faithful to, so an app schema's own inline atom is in
/// scope for free —
///
/// * `!is_block()` — a *block* atom (`horizontal_rule`) is a block of its own and
///   projects as one, with empty text; it must not become a char inside a neighbour.
/// * `is_atom()` — an opaque unit, which is what makes standing for it with a single
///   char faithful rather than an erasure. It is also what excludes `text`, which is
///   inline and a leaf but is the very thing the placeholder is *not*.
/// * `is_leaf()` — "the content match accepts nothing", so there is no content the one
///   char could be silently dropping.
fn is_inline_atom(typ: &NodeType) -> bool {
    !typ.is_block() && typ.is_atom() && typ.is_leaf()
}

/// A yrs document projecting an editor document.
pub struct CollabDoc {
    /// The yrs document. `pub(crate)` so [`crate::sync`] can drive the transport.
    pub(crate) doc: Doc,
    /// The root `content` array handle. Resolved once at construction: obtaining a root
    /// type opens its own transaction, so doing it lazily inside a live one would
    /// deadlock.
    pub(crate) content: ArrayRef,
    /// Updates from locally-projected transactions, awaiting broadcast.
    pub(crate) outbox: Outbox,
    /// Keeps the update observer alive — dropping the subscription unsubscribes it, and
    /// the outbox would silently stop filling.
    _updates: Subscription,
}

// `Subscription` is not `Debug`, and neither is the observer closure behind it, so the
// derive is replaced by a summary of what a reader actually wants to see.
//
// CAUTION: this takes a **read transaction** to report the block count, so formatting a
// `CollabDoc` while a transaction is live (a `dbg!` in the middle of a projection write,
// say) deadlocks on native and panics on wasm — yrs cannot nest transaction acquisitions.
impl std::fmt::Debug for CollabDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollabDoc")
            .field("client_id", &self.doc.client_id())
            .field("blocks", &self.content.len(&self.doc.transact()))
            .field("pending_updates", &lock_outbox(&self.outbox).len())
            .finish()
    }
}

impl CollabDoc {
    /// A fresh, empty CRDT document with its broadcast outbox wired up, and no root type
    /// resolved yet.
    ///
    /// **Never `Doc::new()`** — `Options::default()` is [`OffsetKind::Bytes`], which
    /// would make every text index a byte offset and quietly corrupt any block holding
    /// a non-ASCII character.
    ///
    /// The root is deliberately left unresolved: [`CollabDoc::load`] has to inspect what
    /// arrived *before* declaring a type for it, because declaring one reinterprets
    /// whatever is there (see the shape guard in `load`).
    /// `client_id` is `None` on every production path, which keeps yrs's own
    /// `ClientID::random()`. That randomness is load-bearing: the client id breaks ties
    /// between concurrent inserts at the same position, so two live replicas sharing one
    /// produce colliding block ids and corrupt the shared document. Only the test-only
    /// [`crate::testing`] seam passes `Some`, and only so a fuzz trial replays
    /// bit-for-bit (issue #214).
    fn blank(client_id: Option<ClientID>) -> (Doc, Outbox, Subscription) {
        let mut options = Options {
            offset_kind: OffsetKind::Utf16,
            ..Default::default()
        };
        if let Some(id) = client_id {
            options.client_id = id;
        }
        let doc = Doc::with_options(options);
        let outbox: Outbox = Arc::new(Mutex::new(Vec::new()));
        let sink = outbox.clone();
        let remote = Origin::from(ENGINE_APPLY_ORIGIN);
        // yrs does not fire this for a transaction that changed nothing, so the outbox
        // never collects an empty update and "outbox is empty" really does mean "nothing
        // to send".
        let updates = doc
            .observe_update_v1(move |txn, e| {
                if txn.origin() != Some(&remote) {
                    lock_outbox(&sink).push(e.update.clone());
                }
            })
            .expect("a freshly built document has no live transaction to conflict with");
        (doc, outbox, updates)
    }

    /// Build a fresh projection from a model document. Fails loud
    /// ([`CollabError::Unsupported`]) on any node outside the staged scope.
    pub fn from_doc(doc: &Node) -> Result<CollabDoc> {
        CollabDoc::from_doc_with_client_id(doc, None)
    }

    /// [`CollabDoc::from_doc`] with the yrs client id optionally pinned — see
    /// [`CollabDoc::blank`]. `None` is the production path.
    pub(crate) fn from_doc_with_client_id(
        doc: &Node,
        client_id: Option<ClientID>,
    ) -> Result<CollabDoc> {
        // Validate the whole document before opening the write transaction, so an
        // out-of-scope node leaves no half-built CRDT behind (design A22).
        let mut nodes = Vec::with_capacity(doc.child_count());
        for i in 0..doc.child_count() {
            nodes.push(read_node(doc.child(i))?);
        }

        let (ydoc, outbox, updates) = CollabDoc::blank(client_id);
        // Both roots are resolved before the write transaction opens: resolving one takes
        // exclusive store access and panics if a transaction is already live.
        let content = ydoc.get_or_insert_array(CONTENT);
        let meta = ydoc.get_or_insert_map(META);
        // A plain local transaction, so the initial projection lands in the outbox and is
        // broadcast like any other local change. A peer that joined from a snapshot
        // already has it and applies it as a no-op (updates are idempotent).
        //
        // The format marker rides in the **same** transaction as the content, so a
        // fresh projection is still exactly one broadcast.
        {
            let mut txn = ydoc.transact_mut();
            meta.insert(&mut txn, FORMAT, Any::String(FORMAT_TAG.into()));
            for (i, node) in nodes.iter().enumerate() {
                insert_node(&mut txn, &content, i as u32, node)?;
            }
        }
        Ok(CollabDoc {
            doc: ydoc,
            content,
            outbox,
            _updates: updates,
        })
    }

    /// Load a projection from a peer's saved CRDT bytes (see [`CollabDoc::save`]).
    ///
    /// Fails loud on bytes that decode as a yrs update but are not one of *our*
    /// projections, rather than silently adopting an empty document and collaborating on
    /// content no peer shares.
    ///
    /// Two gates, in this order:
    ///
    /// 1. **The format marker** ([`FORMAT_TAG`] under the `meta` root) must be present
    ///    and exact. This is the discriminator for "are these our bytes at all", and it
    ///    cannot be replaced by a check on the root's *type*, because a root type carries
    ///    no type tag on the wire: a root arriving from a peer reads as
    ///    `Out::UndefinedRef` whatever it really is, and asking for it as an array
    ///    *reinterprets* whatever is there (a foreign `Map` root named `content` then
    ///    reads as a zero-length array, a `Text` root as an array of single characters).
    ///    A marker is an ordinary map entry, so it survives that hazard — it is *content*,
    ///    and content is what the wire carries.
    /// 2. **Every content entry** must read back as a projected node, so a `content`
    ///    array full of junk is refused even if it somehow carried the marker.
    ///
    /// A **zero-block** document passes: that is a legitimate converged state (issue
    /// #192 — two peers deleting different blocks concurrently leaves the content array
    /// empty), and refusing it locked a late joiner out of such a session. Emptiness used
    /// to stand in for gate 1; the marker replaces it, which is what makes admitting zero
    /// blocks safe. [`CollabDoc::to_doc`] then hands the editor the starter paragraph its
    /// schema requires, and the first local edit projects that paragraph into the CRDT
    /// (see [`CollabDoc::project_change`]).
    pub fn load(bytes: &[u8]) -> Result<CollabDoc> {
        CollabDoc::load_with_client_id(bytes, None)
    }

    /// [`CollabDoc::load`] with the yrs client id optionally pinned — see
    /// [`CollabDoc::blank`]. `None` is the production path.
    pub(crate) fn load_with_client_id(
        bytes: &[u8],
        client_id: Option<ClientID>,
    ) -> Result<CollabDoc> {
        let update = Update::decode_v1(bytes)?;
        let (ydoc, outbox, updates) = CollabDoc::blank(client_id);
        {
            let mut txn = ydoc.transact_mut_with(Origin::from(ENGINE_APPLY_ORIGIN));
            txn.apply_update(update)?;
        }
        // Root handles first (each takes exclusive store access), then one read
        // transaction for both gates.
        let content = ydoc.get_or_insert_array(CONTENT);
        let meta = ydoc.get_or_insert_map(META);
        {
            let txn = ydoc.transact();
            match meta.get(&txn, FORMAT) {
                Some(Out::Any(Any::String(tag))) if &*tag == FORMAT_TAG => {}
                other => {
                    return Err(CollabError::schema(format!(
                        "these CRDT bytes are not a rinch editor projection: expected \
                         `{META}.{FORMAT}` = `{FORMAT_TAG}`, found {other:?}"
                    )));
                }
            }
            for i in 0..content.len(&txn) {
                read_node_data(&txn, &content, i)?;
            }
        }
        Ok(CollabDoc {
            doc: ydoc,
            content,
            outbox,
            _updates: updates,
        })
    }

    /// Save the whole projection (for forking a peer): the complete document state as
    /// a v1 update, which [`CollabDoc::load`] reads back.
    pub fn save(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    /// Rebuild the whole model document from the projection (the canonical, total
    /// read-back; both peers reconstruct identically, so equal CRDTs give equal docs).
    ///
    /// # The one exception to `model ≡ project(model)`
    ///
    /// A CRDT holding **zero** content blocks is a legitimate converged state (issue
    /// #192), but the editor schema requires at least one block, so there is no model that
    /// mirrors it. This returns the starter paragraph instead — an **unprojected** block:
    /// the document equality still holds (both sides are `doc(paragraph())`), but that
    /// paragraph has no CRDT content behind it. The exception is scoped to exactly that
    /// state and is cured by the next local edit, which
    /// [`CollabDoc::project_change`] projects wholesale rather than diffing against a
    /// CRDT that does not hold it.
    pub fn to_doc(&self, schema: &Schema) -> Result<Node> {
        let mut blocks = {
            let txn = self.doc.transact();
            let n = self.content.len(&txn);
            let mut blocks = Vec::with_capacity(n as usize);
            for i in 0..n {
                let nd = read_node_data(&txn, &self.content, i)?;
                blocks.push(build_node(schema, &nd)?);
            }
            blocks
        };
        if blocks.is_empty() {
            // An editor doc is never empty; mirror the starter empty paragraph. See the
            // exception documented on this method — this block is not in the CRDT, and
            // `project_change` knows it.
            let para = schema
                .branch("paragraph", Fragment::empty())
                .map_err(CollabError::from)?;
            blocks.push(para);
        }
        schema
            .branch(&schema.top_node, Fragment::from_children(blocks))
            .map_err(CollabError::from)
    }
}

// --- offset conversion ----------------------------------------------------------

/// The UTF-16 code-unit offset of char offset `chars` inside `s`.
///
/// The model counts chars, yrs counts UTF-16 code units ([`OffsetKind::Utf16`]), so
/// every index handed to a `Text` goes through here. A char offset past the end clamps
/// to the end: [`Text::remove_range`] **panics** on an out-of-range index (automerge
/// returned an error), and a panic here would take the whole app down.
fn u16_offset(s: &str, chars: usize) -> u32 {
    s.chars().take(chars).map(|c| c.len_utf16() as u32).sum()
}

/// A char range as a UTF-16 `(index, len)` pair inside `s`.
fn u16_span(s: &str, start: usize, end: usize) -> (u32, u32) {
    let at = u16_offset(s, start);
    let to = u16_offset(s, end.max(start));
    (at, to - at)
}

// --- structural read helpers (any transaction) ----------------------------------

/// The map object of the child at `index` of a content array (the root `content` or a
/// container's nested `content`).
fn child_map<T: ReadTxn>(txn: &T, list: &ArrayRef, index: u32) -> Option<MapRef> {
    match list.get(txn, index) {
        Some(Out::YMap(m)) => Some(m),
        _ => None,
    }
}

/// The `text` Text object of a text-block node.
fn block_text<T: ReadTxn>(txn: &T, node: &MapRef) -> Option<TextRef> {
    match node.get(txn, TEXT) {
        Some(Out::YText(t)) => Some(t),
        _ => None,
    }
}

/// The `content` Array object of a container node.
fn node_content<T: ReadTxn>(txn: &T, node: &MapRef) -> Option<ArrayRef> {
    match node.get(txn, CONTENT) {
        Some(Out::YArray(a)) => Some(a),
        _ => None,
    }
}

/// A node's `type` string.
fn node_type<T: ReadTxn>(txn: &T, node: &MapRef) -> Option<String> {
    match node.get(txn, TYPE) {
        Some(Out::Any(Any::String(s))) => Some(s.to_string()),
        _ => None,
    }
}

/// A node's `attrs` map read back into a model attr set (empty when absent).
fn node_attrs<T: ReadTxn>(txn: &T, node: &MapRef) -> Attrs {
    match node.get(txn, ATTRS) {
        Some(Out::YMap(m)) => read_attrs(txn, &m),
        _ => Attrs::new(),
    }
}

/// Bounds-check a content-array index before a write.
///
/// yrs **panics** on an out-of-range array index where automerge returned `Err`. Every
/// index the projection writes comes from a model/CRDT pair it believes are in step, so
/// a mismatch is a projection bug — and it must surface as a loud [`CollabError`], not
/// as a panic that kills the host application.
pub(crate) fn check_index<T: ReadTxn>(
    txn: &T,
    list: &ArrayRef,
    index: u32,
    allow_end: bool,
) -> Result<()> {
    let len = list.len(txn);
    if if allow_end { index <= len } else { index < len } {
        Ok(())
    } else {
        Err(CollabError::schema(format!(
            "content index {index} out of range (length {len})"
        )))
    }
}

// --- writes --------------------------------------------------------------------

/// Insert a new node (text-block or container) at `index` of `list`.
pub(crate) fn insert_node(
    txn: &mut TransactionMut,
    list: &ArrayRef,
    index: u32,
    nd: &NodeData,
) -> Result<()> {
    check_index(txn, list, index, true)?;
    let node = list.insert(txn, index, MapPrelim::default());
    write_node(txn, &node, nd)
}

/// Write a node's contents into its (already-inserted) map object. Recurses into a
/// container's children.
fn write_node(txn: &mut TransactionMut, node: &MapRef, nd: &NodeData) -> Result<()> {
    node.insert(txn, TYPE, Any::String(nd.type_name().into()));
    let attrs_obj = node.insert(txn, ATTRS, MapPrelim::default());
    write_attrs(txn, &attrs_obj, nd.attrs());
    match nd {
        NodeData::Block(b) => {
            // A leaf block atom takes this same branch with an empty string, so it gets
            // an empty `Text` object: one wire shape for every block, and a peer at this
            // wire version reads it back without knowing atoms exist.
            let text = node.insert(txn, TEXT, TextPrelim::new(b.text.as_str()));
            for m in &b.marks {
                apply_mark(txn, &text, &b.text, m);
            }
        }
        NodeData::Container { children, .. } => {
            let content = node.insert(txn, CONTENT, ArrayPrelim::default());
            for (i, child) in children.iter().enumerate() {
                insert_node(txn, &content, i as u32, child)?;
            }
        }
    }
    Ok(())
}

/// Reconcile the node at `index` of `list` to `target`: update type/attrs if they
/// changed, then reconcile its body — a text-block's text + marks, or a container's
/// child list — recursively. Unchanged descendants keep their CRDT identity, so a
/// concurrent edit to a *different* text-block (even in a different list item) merges.
///
/// Two kinds of change are **replaced** wholesale instead: a node changing kind
/// (text-block ↔ container), and a node retyped into a shape that holds no text (a
/// paragraph becoming a `horizontal_rule`) — see the comments on each below.
pub(crate) fn reconcile_node(
    txn: &mut TransactionMut,
    list: &ArrayRef,
    index: u32,
    target: &NodeData,
) -> Result<()> {
    let node = child_map(txn, list, index)
        .ok_or_else(|| CollabError::schema("reconcile_node: missing node"))?;

    // A node changing *kind* (text-block <-> container — e.g. a paragraph wrapped
    // into a list) can't be reconciled in place; replace it wholesale. Rare, so the
    // coarse-grained replace is fine.
    let crdt_is_block = block_text(txn, &node).is_some();
    let target_is_block = matches!(target, NodeData::Block(_));
    // A node retyped into something that holds no text — a paragraph becoming a
    // `horizontal_rule`, the scene break — is replaced for the same reason, and it is
    // the guard that keeps a leaf block atom's text empty. Reconciled in place it would
    // keep its `Text` object while its `type` flipped to the atom, and a peer typing
    // into that very block concurrently would leave the converged document holding an
    // hr with text in it: a shape no model can express, which `build_block` can then
    // only refuse. Replacing instead makes the conflict structural — the peer's
    // insertion lands in a node that is gone, and its edit is the one thing lost rather
    // than the whole document's projectability.
    //
    // Stated over the target's *text* rather than its atom-ness because this layer has
    // no schema to ask (the type name is all the CRDT carries). The cost is that
    // retyping a block that is *also* empty — an empty paragraph made a heading —
    // replaces rather than reconciles; there is no text in it to preserve, so all that
    // is given up is the merge of a peer's concurrent typing into an empty block.
    let retyped_to_empty = match target {
        NodeData::Block(b) => {
            b.text.is_empty() && node_type(txn, &node).as_deref() != Some(b.type_name.as_str())
        }
        NodeData::Container { .. } => false,
    };
    if crdt_is_block != target_is_block || retyped_to_empty {
        check_index(txn, list, index, false)?;
        list.remove(txn, index);
        return insert_node(txn, list, index, target);
    }

    // type — only write when it changed
    if node_type(txn, &node).as_deref() != Some(target.type_name()) {
        node.insert(txn, TYPE, Any::String(target.type_name().into()));
    }
    // attrs — diff per key when they changed. Replacing the whole attrs object would
    // clobber a concurrent remote edit to a *different* key of the same node (issue
    // #193): yrs merges map conflicts per key, but only within one map object — a
    // freshly-installed object is a conflict on the node's `attrs` entry itself, one
    // object wins wholesale, and the other peer's key is silently gone.
    let current_attrs = node_attrs(txn, &node);
    if current_attrs != *target.attrs() {
        reconcile_attrs(txn, &node, &current_attrs, target.attrs());
    }

    match target {
        NodeData::Block(b) => {
            let text = block_text(txn, &node)
                .ok_or_else(|| CollabError::schema("reconcile_node: missing text"))?;
            reconcile_text(txn, &text, b)
        }
        NodeData::Container { children, .. } => {
            let content = node_content(txn, &node)
                .ok_or_else(|| CollabError::schema("reconcile_node: missing content"))?;
            reconcile_child_list(txn, &content, children)
        }
    }
}

/// Reconcile a text-block's `text` object to `b`: a minimal common-prefix/suffix splice
/// (so the per-char CRDT identity of unchanged text survives) plus a mark resync only
/// when the marks actually changed.
///
/// A **leaf block atom** reaching here is already the same atom type in the CRDT (any
/// other way of becoming one is a replace — see [`reconcile_node`]), so `old` and
/// `b.text` are both empty, the splice and the resync are both skipped, and this is a
/// no-op. It is not a special case that needs guarding: `splice_min` on `"" -> ""`
/// computes a zero-length delete and an empty insert and issues neither, so even a
/// corrupt atom that *did* hold text is healed (spliced back to empty) rather than
/// erroring.
fn reconcile_text(txn: &mut TransactionMut, text: &TextRef, b: &BlockData) -> Result<()> {
    let (old, old_marks) = read_text_data(txn, text)?;
    let spliced = old != b.text;
    if spliced {
        splice_min(txn, text, &old, &b.text);
    }

    let mut target_marks = b.marks.clone();
    target_marks.sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));

    // Marks must be compared *after* the splice: yrs has already shifted existing
    // formatting ranges with the text, and text inserted **inside** a formatted run
    // inherits that run's attributes (which usually matches the editor's own stored-mark
    // behaviour, but not always). Re-reading is what keeps the two in step. Skipping the
    // resync when the marks already agree avoids clobbering a concurrent remote mark on
    // a pure text edit.
    let (current, current_marks) = if spliced {
        read_text_data(txn, text)?
    } else {
        (old, old_marks)
    };
    if current_marks != target_marks {
        resync_marks(txn, text, &current, &current_marks, &target_marks);
    }
    Ok(())
}

/// Reconcile a container's `content` array to `target`. Diffs the CRDT's current
/// children against `target` by *structural* equality (a common leading/trailing run
/// is left untouched, keeping the CRDT identity — and merge behaviour — of every
/// node in it), reconciles the overlapping middle in place, and inserts/deletes the
/// count difference. This is the same block-list diff as
/// [`project_change`](CollabDoc::project_change), one level down; recursion carries it
/// to any depth.
fn reconcile_child_list(
    txn: &mut TransactionMut,
    content: &ArrayRef,
    target: &[NodeData],
) -> Result<()> {
    let cn = content.len(txn) as usize;
    let mut cur = Vec::with_capacity(cn);
    for i in 0..cn {
        cur.push(read_node_data(txn, content, i as u32)?);
    }
    let tn = target.len();

    let mut prefix = 0;
    while prefix < cn && prefix < tn && cur[prefix] == target[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < cn - prefix
        && suffix < tn - prefix
        && cur[cn - 1 - suffix] == target[tn - 1 - suffix]
    {
        suffix += 1;
    }

    let cur_mid = cn - prefix - suffix;
    let tgt_mid = tn - prefix - suffix;
    let common = cur_mid.min(tgt_mid);

    // Reconcile the overlapping changed children in place (keeps identity).
    for k in 0..common {
        reconcile_node(txn, content, (prefix + k) as u32, &target[prefix + k])?;
    }
    // Insert the extra target children.
    for k in common..tgt_mid {
        insert_node(txn, content, (prefix + k) as u32, &target[prefix + k])?;
    }
    // Delete the extra current children (from the end so earlier indices stay valid).
    for idx in (prefix + common..prefix + cur_mid).rev() {
        check_index(txn, content, idx as u32, false)?;
        content.remove(txn, idx as u32);
    }
    Ok(())
}

/// Minimal common-prefix/suffix splice: replace only the changed middle so unchanged
/// characters keep their CRDT identity (and merge across peers). yrs has no
/// `update_text` equivalent, so the diff is computed here and applied as a
/// remove-then-insert pair, converted from char offsets into UTF-16 code units.
pub(crate) fn splice_min(txn: &mut TransactionMut, text: &TextRef, old: &str, new: &str) {
    let o: Vec<char> = old.chars().collect();
    let n: Vec<char> = new.chars().collect();
    let mut prefix = 0;
    while prefix < o.len() && prefix < n.len() && o[prefix] == n[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < o.len() - prefix
        && suffix < n.len() - prefix
        && o[o.len() - 1 - suffix] == n[n.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let del = o.len() - prefix - suffix;
    let ins: String = n[prefix..n.len() - suffix].iter().collect();

    let at: u32 = o[..prefix].iter().map(|c| c.len_utf16() as u32).sum();
    let del_u16: u32 = o[prefix..prefix + del]
        .iter()
        .map(|c| c.len_utf16() as u32)
        .sum();
    if del_u16 > 0 {
        text.remove_range(txn, at, del_u16);
    }
    if !ins.is_empty() {
        text.insert(txn, at, &ins);
    }
}

/// Apply one mark span as a formatting attribute over a `Text` range. A zero-length
/// span is skipped — there is nothing to format, and the model never produces one.
fn apply_mark(txn: &mut TransactionMut, text: &TextRef, s: &str, m: &SpanMark) {
    let (at, len) = u16_span(s, m.start, m.end);
    if len == 0 {
        return;
    }
    let attrs = YAttrs::from([(m.name.as_str().into(), encode_mark_value(&m.attrs))]);
    text.format(txn, at, len, attrs);
}

/// Bring a `Text`'s formatting attributes in step with `target` by **per-span
/// difference**: clear only the spans no longer present (`current \ target`, via the
/// Yjs convention that a `null` attribute value removes that format), apply only the
/// spans that are new (`target \ current`). A span is a `(name, attrs, range)` run —
/// the finest granularity the canonical span lists carry — so a mark the edit did not
/// touch is not written at all.
///
/// It must not be: re-applying an unchanged mark gives it a fresh CRDT write that
/// outlives a peer's *concurrent removal* of that very mark (issue #193). Both peers
/// still converged — on a document that silently resurrected what the peer deleted.
/// A mark the local edit really changed is still cleared/re-added, which is the honest
/// same-mark conflict.
///
/// `current` is the text's present string and `current_marks` the spans read out of it
/// (both **after** any splice), so the ranges being cleared are the live ones. Two
/// spans of one name never overlap (both lists are canonical, coalesced runs), so a
/// clear cannot blank a *kept* span of the same name — and all clears run before all
/// applies, so a span that changed extent (cleared at the old range, re-applied at the
/// new) nets out to exactly the new range.
///
/// An **inline atom**'s [`ATOM_MARK`] span rides through here as an ordinary span, and
/// safely so: a yrs formatting clear is per *attribute key*, so a peer clearing `bold`
/// over the atom's char cannot take `@atom` with it, and the per-span difference above
/// means a local edit that did not touch the atom does not rewrite it either. Writing
/// the attribute *with* the placeholder's insert instead (`insert_with_attributes`)
/// would buy nothing: it is the same format markers around the same char. What it
/// could not prevent either way is the boundary inheritance [`is_atom_char`] describes
/// — an insert landing *inside* those markers — which is why that is handled on the
/// read side rather than guarded against here.
fn resync_marks(
    txn: &mut TransactionMut,
    text: &TextRef,
    current: &str,
    current_marks: &[SpanMark],
    target: &[SpanMark],
) {
    for m in current_marks {
        if target.contains(m) {
            continue;
        }
        let (at, len) = u16_span(current, m.start, m.end);
        if len == 0 {
            continue;
        }
        text.format(
            txn,
            at,
            len,
            YAttrs::from([(m.name.as_str().into(), Any::Null)]),
        );
    }
    for m in target {
        if !current_marks.contains(m) {
            apply_mark(txn, text, current, m);
        }
    }
}

// --- CRDT → NodeData -----------------------------------------------------------

/// Read a `Text` back as its plain string plus the canonical (sorted, coalesced) mark
/// spans over it, in **char** offsets.
///
/// `Text::diff` hands back the text already split into runs at every formatting change,
/// each carrying the full attribute set active over it — so the string and the spans are
/// read in one pass and cannot disagree. A chunk that is not a string is an embedded
/// value, which is outside the staged scope (A22) and fails loud rather than being
/// dropped.
fn read_text_data<T: ReadTxn>(txn: &T, text: &TextRef) -> Result<(String, Vec<SpanMark>)> {
    let mut s = String::new();
    let mut marks: Vec<SpanMark> = Vec::new();
    for chunk in text.diff(txn, YChange::identity) {
        let Out::Any(Any::String(part)) = &chunk.insert else {
            return Err(CollabError::unsupported(
                "an embedded value inside a block's text is not supported; an inline \
                 atom is projected as a placeholder char with an `@atom` attribute, \
                 never as a yrs embed",
            ));
        };
        let start = s.chars().count();
        s.push_str(part);
        let end = s.chars().count();
        if let Some(attrs) = &chunk.attributes {
            for (name, value) in attrs.iter() {
                // A cleared format can linger as an explicit null; it is not a mark.
                if matches!(value, Any::Null | Any::Undefined) {
                    continue;
                }
                push_mark_span(&mut marks, name, decode_mark_value(value)?, start, end);
            }
        }
    }
    marks.sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));
    Ok((s, marks))
}

/// Read the node at `index` of `list` back out of the CRDT as [`NodeData`]. A node
/// carrying a `text` object is a block (a text-block, or a leaf block atom whose text is
/// empty — the schema, consulted in [`build_block`], is what tells those apart, and this
/// layer has none); a node carrying a `content` array is a container, read recursively.
/// Fails loud on a node that is neither (a corrupt projection), rather than
/// materializing a broken shape.
pub(crate) fn read_node_data<T: ReadTxn>(txn: &T, list: &ArrayRef, index: u32) -> Result<NodeData> {
    let node = child_map(txn, list, index)
        .ok_or_else(|| CollabError::schema("read_node_data: missing node"))?;
    let type_name = node_type(txn, &node).ok_or_else(|| CollabError::schema("node has no type"))?;
    let attrs = node_attrs(txn, &node);
    if let Some(text_obj) = block_text(txn, &node) {
        let (text, marks) = read_text_data(txn, &text_obj)?;
        Ok(NodeData::Block(BlockData {
            type_name,
            attrs,
            text,
            marks,
        }))
    } else if let Some(content) = node_content(txn, &node) {
        let len = content.len(txn);
        let mut children = Vec::with_capacity(len as usize);
        for i in 0..len {
            children.push(read_node_data(txn, &content, i)?);
        }
        Ok(NodeData::Container {
            type_name,
            attrs,
            children,
        })
    } else {
        Err(CollabError::schema(format!(
            "node `{type_name}` has neither a `text` nor a `content` object"
        )))
    }
}

// --- model → NodeData ----------------------------------------------------------

/// Validate a model node is projectable and extract its [`NodeData`], recursing into
/// list containers. A flat text-block — or a leaf block atom ([`is_leaf_block_atom`]) —
/// becomes [`NodeData::Block`]; a supported list container ([`is_supported_container`])
/// becomes [`NodeData::Container`] over its recursively-read children. Anything else —
/// an unsupported nested block (`blockquote`, table, task list) or an *inline* atom —
/// fails loud (design A22).
pub(crate) fn read_node(node: &Node) -> Result<NodeData> {
    if node.is_textblock() || is_leaf_block_atom(node.node_type()) {
        return Ok(NodeData::Block(read_block(node)?));
    }
    if is_supported_container(node.type_name()) {
        let mut children = Vec::with_capacity(node.child_count());
        for i in 0..node.child_count() {
            children.push(read_node(node.child(i))?);
        }
        return Ok(NodeData::Container {
            type_name: node.type_name().to_string(),
            attrs: node.attrs().clone(),
            children,
        });
    }
    Err(CollabError::unsupported(format!(
        "node `{}` is not a flat text-block, a leaf block atom, or a supported list \
         container (bullet_list/ordered_list/list_item); other nested blocks and tables \
         are not yet supported",
        node.type_name()
    )))
}

/// Validate a model block is a flat textblock — or a leaf block atom — and extract its
/// projectable data. Marks are returned in canonical `(start, end, name)` order —
/// matching [`read_text_data`] — so a [`NodeData`] read from the model compares equal to
/// the same node read back from the CRDT.
///
/// A textblock's children may be text nodes **or inline atoms** ([`is_inline_atom`]);
/// an atom becomes one [`ATOM_PLACEHOLDER`] char plus an [`ATOM_MARK`] span over it.
pub(crate) fn read_block(block: &Node) -> Result<BlockData> {
    if is_leaf_block_atom(block.node_type()) {
        // An atom holds nothing (`is_leaf` *is* "the content match accepts nothing"), so
        // it projects as a block with empty text and no marks — and reads back from the
        // CRDT as exactly that, which is what makes the round trip an identity. The
        // child check is a cheap guard against a node built past the schema rather than
        // a reachable state.
        if block.child_count() != 0 {
            return Err(CollabError::schema(format!(
                "leaf block atom `{}` holds {} child node(s); an atom has no content",
                block.type_name(),
                block.child_count()
            )));
        }
        return Ok(BlockData {
            type_name: block.type_name().to_string(),
            attrs: block.attrs().clone(),
            text: String::new(),
            marks: Vec::new(),
        });
    }
    if !block.is_textblock() {
        return Err(CollabError::unsupported(format!(
            "block `{}` is not a flat text-block or a leaf block atom (nested blocks are \
             not yet supported)",
            block.type_name()
        )));
    }
    let mut text = String::new();
    let mut marks: Vec<SpanMark> = Vec::new();
    for i in 0..block.child_count() {
        let child = block.child(i);
        let start = text.chars().count();
        match child.text() {
            Some(t) => text.push_str(t),
            // An **inline atom** stands in the text as one placeholder char carrying
            // the reserved [`ATOM_MARK`] attribute, which is where its type and attrs
            // live. Its own marks (a link on an image, if the schema allows one) are
            // pushed over that same char as ordinary spans, exactly as for text — so
            // the atom is a char that happens to be formatted, and every path that
            // already handles a formatted char handles it.
            None => {
                if !is_inline_atom(child.node_type()) {
                    return Err(CollabError::unsupported(format!(
                        "inline node `{}` inside `{}` is neither text nor an inline atom",
                        child.type_name(),
                        block.type_name()
                    )));
                }
                // `is_leaf` says the content match accepts nothing, so children here
                // mean a node built past its own schema. A cheap guard against
                // projecting one char over content that is really there.
                if child.child_count() != 0 {
                    return Err(CollabError::schema(format!(
                        "inline atom `{}` holds {} child node(s); an atom has no content",
                        child.type_name(),
                        child.child_count()
                    )));
                }
                text.push(ATOM_PLACEHOLDER);
                push_mark_span(&mut marks, ATOM_MARK, atom_attrs(child)?, start, start + 1);
            }
        }
        let end = text.chars().count();
        for m in child.marks() {
            push_span(&mut marks, m, start, end);
        }
    }
    marks.sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));
    Ok(BlockData {
        type_name: block.type_name().to_string(),
        attrs: block.attrs().clone(),
        text,
        marks,
    })
}

/// Append a mark over `start..end`, coalescing with an immediately-preceding span of
/// the same (type, attrs).
fn push_span(marks: &mut Vec<SpanMark>, m: &Mark, start: usize, end: usize) {
    push_mark_span(marks, m.type_name(), m.attrs.clone(), start, end);
}

/// The coalescing half of [`push_span`], shared with the CRDT read-back so both sides
/// produce the same canonical span list for the same logical formatting.
fn push_mark_span(marks: &mut Vec<SpanMark>, name: &str, attrs: Attrs, start: usize, end: usize) {
    if let Some(prev) = marks
        .iter_mut()
        .find(|s| s.end == start && s.name == name && s.attrs == attrs)
    {
        prev.end = end;
        return;
    }
    marks.push(SpanMark {
        name: name.to_string(),
        attrs,
        start,
        end,
    });
}

/// The [`ATOM_MARK`] value for an inline atom node: its own attrs plus its node type
/// name under [`ATOM_TYPE`], as one flat attr set.
///
/// Flat, rather than a nested `{type, attrs}` map, so the value encodes and decodes
/// through the *same* [`encode_mark_value`] / [`decode_mark_value`] pair every other
/// mark uses — which is what lets [`read_text_data`] read an atom back without a case
/// of its own (it makes a [`SpanMark`] of every formatting attribute, and this is one).
/// The flattening is only safe because the type key is reserved: an atom that carries
/// an attr in the reserved namespace would be indistinguishable from it, so it is
/// refused here rather than silently overwritten.
fn atom_attrs(atom: &Node) -> Result<Attrs> {
    let mut attrs = Attrs::new();
    for (k, v) in atom.attrs().iter() {
        if k.starts_with(RESERVED_PREFIX) {
            return Err(CollabError::schema(format!(
                "inline atom `{}` carries the reserved attribute `{k}`; `{RESERVED_PREFIX}` \
                 names belong to the projection",
                atom.type_name()
            )));
        }
        attrs = attrs.with(k, v.clone());
    }
    Ok(attrs.with(ATOM_TYPE, AttrValue::from(atom.type_name())))
}

/// The inverse of [`atom_attrs`]: an [`ATOM_MARK`] span's attrs split back into the
/// atom's node type name and its own attrs. Fails loud on a value that could not have
/// come from [`atom_attrs`] — a missing or non-string type, any other reserved key —
/// rather than materializing a guess.
fn atom_span_type(span: &SpanMark) -> Result<(String, Attrs)> {
    let mut type_name: Option<String> = None;
    let mut attrs = Attrs::new();
    for (k, v) in span.attrs.iter() {
        if k == ATOM_TYPE {
            let AttrValue::Str(name) = v else {
                return Err(CollabError::schema(format!(
                    "inline atom's `{ATOM_TYPE}` must be a string, got {v:?}"
                )));
            };
            type_name = Some(name.to_string());
        } else if k.starts_with(RESERVED_PREFIX) {
            return Err(CollabError::schema(format!(
                "unknown reserved key `{k}` in an inline atom's `{ATOM_MARK}` value"
            )));
        } else {
            attrs = attrs.with(k, v.clone());
        }
    }
    let type_name = type_name.ok_or_else(|| {
        CollabError::schema(format!(
            "an `{ATOM_MARK}` span carries no `{ATOM_TYPE}`, so there is no node to build"
        ))
    })?;
    Ok((type_name, attrs))
}

/// The [`ATOM_MARK`] span covering char `i`, if any. Two spans of one name never
/// overlap (both span lists are canonical coalesced runs), so there is at most one.
fn atom_span_at(spans: &[SpanMark], i: usize) -> Option<&SpanMark> {
    spans
        .iter()
        .find(|s| s.name == ATOM_MARK && s.start <= i && i < s.end)
}

// --- NodeData → model ----------------------------------------------------------

/// Rebuild a model node from projected [`NodeData`], recursing into list containers.
/// The inbound scope guard (A22) is shared with the outbound [`read_node`]: a container
/// type must be one [`is_supported_container`] permits, so a peer CRDT can never
/// materialize an out-of-scope shape here even though [`Schema::create_node`] would
/// happily build one.
fn build_node(schema: &Schema, nd: &NodeData) -> Result<Node> {
    match nd {
        NodeData::Block(b) => build_block(schema, b),
        NodeData::Container {
            type_name,
            attrs,
            children,
        } => {
            if !is_supported_container(type_name) {
                return Err(CollabError::unsupported(format!(
                    "container `{type_name}` is not a supported list type in the CRDT"
                )));
            }
            let mut kids = Vec::with_capacity(children.len());
            for c in children {
                kids.push(build_node(schema, c)?);
            }
            schema
                .create_node(type_name, attrs.clone(), Fragment::from_children(kids))
                .map_err(CollabError::from)
        }
    }
}

/// Rebuild a model textblock node from projected block data: split the text into runs
/// at mark boundaries, build a text node per run, assemble the block. A leaf block atom
/// takes the short path — an empty fragment, since it has no content to run-split.
///
/// A char that is an inline atom ([`is_atom_char`]) breaks every run and becomes a node
/// of its own, carrying whatever real marks cover it.
fn build_block(schema: &Schema, b: &BlockData) -> Result<Node> {
    // Inbound scope guard (A22): a peer CRDT must not be able to materialize a
    // non-flat block here — `create_node` would happily build a `blockquote`/`list`,
    // silently breaking the flat-only invariant the outbound `read_block` enforces.
    let typ = schema.node_type(&b.type_name).ok_or_else(|| {
        CollabError::unsupported(format!("unknown block type `{}` in CRDT", b.type_name))
    })?;
    if is_leaf_block_atom(typ) {
        // The atom's whole content is that it has none, so build it from an empty
        // fragment rather than from text runs. Text on an atom means the CRDT holds
        // something this cannot represent: `reconcile_node` replaces a node rather than
        // let a live `Text` survive a type change into an atom, so the only writer that
        // can produce it is a foreign or corrupt one — and dropping it silently is the
        // divergence class A22 exists to kill.
        if !b.text.is_empty() || !b.marks.is_empty() {
            return Err(CollabError::unsupported(format!(
                "leaf block atom `{}` carries text in the CRDT ({} char(s), {} mark \
                 span(s)); an atom has no content",
                b.type_name,
                b.text.chars().count(),
                b.marks.len()
            )));
        }
        return schema
            .create_node(&b.type_name, b.attrs.clone(), Fragment::empty())
            .map_err(CollabError::from);
    }
    if !typ.is_textblock() {
        return Err(CollabError::unsupported(format!(
            "block `{}` is not a flat text-block or a leaf block atom (nested blocks are \
             not yet supported)",
            b.type_name
        )));
    }
    // A schema that has minted a mark type in the reserved namespace would make an
    // atom's attribute indistinguishable from one of its marks, in both directions.
    // Checked here, the one place with a schema in hand, rather than assumed.
    if schema.mark_type(ATOM_MARK).is_some() {
        return Err(CollabError::schema(format!(
            "the schema defines a mark type named `{ATOM_MARK}`, which the projection \
             reserves for inline atoms"
        )));
    }
    let chars: Vec<char> = b.text.chars().collect();
    let mut runs: Vec<Node> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        // An inline atom: one placeholder char carrying the reserved attribute, built
        // as its own node with whatever real marks cover that char. One node **per
        // char**, never per span: `push_mark_span` coalesces two adjacent identical
        // atoms (two hard breaks, the same image twice) into a single two-char span,
        // and those are two nodes.
        if is_atom_char(&b.marks, &chars, i) {
            let span = atom_span_at(&b.marks, i).expect("is_atom_char found one");
            runs.push(build_inline_atom(
                schema,
                span,
                marks_at(schema, &b.marks, i)?,
            )?);
            i += 1;
            continue;
        }
        // Otherwise a run of text: extend while the mark set holds and no atom starts.
        let cur = marks_at(schema, &b.marks, i)?;
        let run_start = i;
        i += 1;
        while i < chars.len()
            && !is_atom_char(&b.marks, &chars, i)
            && same_mark_set(&cur, &marks_at(schema, &b.marks, i)?)
        {
            i += 1;
        }
        let s: String = chars[run_start..i].iter().collect();
        runs.push(schema.text_with_marks(&s, cur).map_err(CollabError::from)?);
    }
    let attrs = b.attrs.clone();
    schema
        .create_node(&b.type_name, attrs, Fragment::from_children(runs))
        .map_err(CollabError::from)
}

/// Whether char `i` is an inline atom: the [`ATOM_PLACEHOLDER`] **and** covered by an
/// [`ATOM_MARK`] span. Both halves are load-bearing, and each mismatch is deliberate:
///
/// * A placeholder with **no** atom attribute is ordinary text. A U+FFFC is a character
///   a user can type or paste, and the projection has no way to tell one that was
///   pasted from one whose attribute a merge dropped, so it round-trips as text.
/// * An atom attribute over a char that is **not** a placeholder is ignored, and the
///   char is text. This is *not* a silent drop — there is no atom node to drop; a char
///   that already exists keeps its text and its real marks, and the stray attribute is
///   cleared by the next [`resync_marks`] on that block.
///
/// The second rule cannot be a fail-loud guard, however much the rest of this crate
/// leans that way (A22), because an ordinary concurrent edit produces it. A yrs insert
/// at the **end boundary** of a formatted range is swallowed into that range — that is
/// rich-text CRDT behaviour, the same rule that continues bold when you type at the end
/// of a bold word — so a peer typing immediately after an image inherits the image's
/// `@atom` attribute on its new chars. Measured: it happens for every client-id order
/// and both integration orders. Refusing it would turn "two authors, one of them typing
/// just after a picture" into a poisoned session (issue #196) over a formatting
/// artifact that carries no content at all.
fn is_atom_char(spans: &[SpanMark], chars: &[char], i: usize) -> bool {
    chars.get(i) == Some(&ATOM_PLACEHOLDER) && atom_span_at(spans, i).is_some()
}

/// Build one inline atom node from its [`ATOM_MARK`] span and the real marks covering
/// its char. Fails loud (never a silent drop: there *is* a node here) on a type the
/// schema does not know, or one that is not an inline atom — a block, a textblock or a
/// text type smuggled into an atom attribute would otherwise be built by
/// [`Schema::create_node`] into a shape no textblock can hold.
fn build_inline_atom(schema: &Schema, span: &SpanMark, marks: Vec<Mark>) -> Result<Node> {
    let (type_name, attrs) = atom_span_type(span)?;
    let typ = schema.node_type(&type_name).ok_or_else(|| {
        CollabError::unsupported(format!("unknown inline atom type `{type_name}` in CRDT"))
    })?;
    if !is_inline_atom(typ) {
        return Err(CollabError::unsupported(format!(
            "`{type_name}` is not an inline atom, so it cannot stand in a block's text"
        )));
    }
    let node = schema
        .create_node(&type_name, attrs, Fragment::empty())
        .map_err(CollabError::from)?;
    Ok(if marks.is_empty() {
        node
    } else {
        node.with_marks(marks)
    })
}

/// The model marks active at char index `i`, resolved against the schema.
///
/// [`ATOM_MARK`] spans are skipped: the attribute names an inline atom, not a mark, and
/// the schema has no mark type of that name to resolve it against — asking for one
/// would fail loud on every atom in the document.
fn marks_at(schema: &Schema, spans: &[SpanMark], i: usize) -> Result<Vec<Mark>> {
    let mut out = Vec::new();
    for s in spans {
        if s.name == ATOM_MARK {
            continue;
        }
        if s.start <= i && i < s.end {
            let mt = schema.mark_type(&s.name).ok_or_else(|| {
                CollabError::schema(format!("unknown mark type `{}` in CRDT", s.name))
            })?;
            let attrs = mt.compute_attrs(&s.attrs).map_err(CollabError::from)?;
            out.push(Mark::new(mt.clone(), attrs));
        }
    }
    // Canonical (mark-type-name) order, matching `Mark::add_to_set`. `spans` is sorted
    // by `(start, end, name)`, so a char covered by two marks with *different* extents
    // would otherwise come out in span-start order, not name order — and the rebuilt
    // node would compare unequal (mark-`Vec` order) to the edited model.
    out.sort_by(|a, b| a.type_name().cmp(b.type_name()));
    Ok(out)
}

/// Order-independent mark-set equality (used to find run boundaries).
fn same_mark_set(a: &[Mark], b: &[Mark]) -> bool {
    a.len() == b.len() && a.iter().all(|m| b.iter().any(|n| n == m))
}

// --- attr / mark-value encoding ------------------------------------------------

/// Write a model attr set into a yrs map. Values stay **typed** — an integer is an
/// integer, not a stringified one.
fn write_attrs(txn: &mut TransactionMut, obj: &MapRef, attrs: &Attrs) {
    for (k, v) in attrs.iter() {
        // `attr_to_any` yields `None` for `AttrValue::Null`, which means "explicitly
        // absent" — storing it would be indistinguishable from a cleared key on
        // read-back.
        if let Some(any) = attr_to_any(v) {
            obj.insert(txn, k.to_string(), any);
        }
    }
}

/// Bring a node's existing `attrs` map in step with `target` by **per-key**
/// insert/remove on the same map object — never by replacing the object (see the
/// caller, [`reconcile_node`], for why replacing loses concurrent peer edits).
///
/// `current` is the attr set already read back from the node (so the caller's
/// changed-at-all check and this diff agree on what the CRDT holds). Keys the target
/// no longer carries — or carries as [`AttrValue::Null`], which [`write_attrs`] never
/// stores — are removed; keys whose value differs are written; equal keys are left
/// untouched, keeping their CRDT history out of the transaction entirely.
fn reconcile_attrs(txn: &mut TransactionMut, node: &MapRef, current: &Attrs, target: &Attrs) {
    let obj = match node.get(txn, ATTRS) {
        Some(Out::YMap(m)) => m,
        // Every projected node is written with an attrs map (`write_node`), but a
        // missing/foreign one is recoverable: install a fresh map, and the loop below
        // fills it (nothing to remove — `current` is empty for a non-map entry).
        _ => node.insert(txn, ATTRS, MapPrelim::default()),
    };
    // Remove stale keys. Collected first: the iteration holds a read borrow of the
    // transaction. Iterating the live map (not `current`) also sweeps keys a foreign
    // writer left in shapes `read_attrs` skips.
    let stale: Vec<String> = obj
        .iter(txn)
        .map(|(k, _)| k.to_string())
        .filter(|k| target.get(k).is_none_or(|v| attr_to_any(v).is_none()))
        .collect();
    for k in &stale {
        obj.remove(txn, k);
    }
    // Write only the keys whose value actually changed.
    for (k, v) in target.iter() {
        if let Some(any) = attr_to_any(v)
            && current.get(k) != Some(v)
        {
            obj.insert(txn, k.to_string(), any);
        }
    }
}

/// Read a yrs map back into a model attr set. Values yrs cannot have come from
/// [`write_attrs`] (a nested shared type, a buffer) are skipped rather than guessed at.
fn read_attrs<T: ReadTxn>(txn: &T, obj: &MapRef) -> Attrs {
    let mut out = Attrs::new();
    for (key, value) in obj.iter(txn) {
        if let Out::Any(any) = value
            && let Some(v) = any_to_attr(&any)
        {
            out = out.with(key, v);
        }
    }
    out
}

/// One model attr value as a yrs [`Any`]. `None` for [`AttrValue::Null`], which is not
/// stored at all.
fn attr_to_any(v: &AttrValue) -> Option<Any> {
    match v {
        AttrValue::Str(s) => Some(Any::String(s.as_ref().into())),
        // Written explicitly as a `BigInt` so the encoding does not depend on the
        // value's magnitude; `any_to_attr` accepts both encodings regardless.
        AttrValue::Int(i) => Some(Any::BigInt(*i)),
        AttrValue::Bool(b) => Some(Any::Bool(*b)),
        AttrValue::Null => None,
    }
}

/// One yrs [`Any`] as a model attr value, or `None` for a shape the projection never
/// writes.
///
/// Integers are accepted in **both** encodings: yrs picks between `BigInt` and
/// `Number` by magnitude, so matching only the arm we write would silently drop
/// attributes that came from another writer (or a future yrs).
fn any_to_attr(v: &Any) -> Option<AttrValue> {
    match v {
        Any::String(s) => Some(AttrValue::from(s.to_string())),
        Any::BigInt(i) => Some(AttrValue::Int(*i)),
        Any::Number(n) if n.fract() == 0.0 => Some(AttrValue::Int(*n as i64)),
        Any::Bool(b) => Some(AttrValue::Bool(*b)),
        _ => None,
    }
}

/// Encode a mark's attrs as its yrs formatting-attribute **value**.
///
/// Two encodings, mirrored by [`decode_mark_value`]: `true` for an attr-less mark
/// (bold, italic) and a `Map` of typed values for an attr-bearing one (a link's
/// `href`). yrs formatting attributes carry structured values, so — unlike automerge,
/// whose marks held a single scalar — no JSON-string indirection is needed.
fn encode_mark_value(attrs: &Attrs) -> Any {
    if attrs.is_empty() {
        return Any::Bool(true);
    }
    let mut map: HashMap<String, Any> = HashMap::new();
    for (k, v) in attrs.iter() {
        map.insert(k.to_string(), attr_to_any(v).unwrap_or(Any::Null));
    }
    Any::Map(Arc::new(map))
}

/// Decode a mark's yrs formatting value back into model attrs. Fails loud on anything
/// [`encode_mark_value`] could not have produced — a wrong value kind, a non-integer
/// number, a nested collection — rather than silently dropping a peer's corrupted mark
/// attrs (A22).
fn decode_mark_value(value: &Any) -> Result<Attrs> {
    let map = match value {
        // The attr-less encoding — no attributes to decode.
        Any::Bool(_) => return Ok(Attrs::new()),
        Any::Map(map) => map,
        other => {
            return Err(CollabError::schema(format!(
                "mark value must be `true` or a map of attributes, got {other:?}"
            )));
        }
    };
    let mut out = Attrs::new();
    for (k, v) in map.iter() {
        let av = match v {
            Any::String(s) => AttrValue::from(s.to_string()),
            Any::Bool(b) => AttrValue::Bool(*b),
            Any::BigInt(i) => AttrValue::Int(*i),
            Any::Number(n) if n.fract() == 0.0 => AttrValue::Int(*n as i64),
            Any::Number(n) => {
                return Err(CollabError::schema(format!(
                    "non-integer number in mark attr `{k}`: {n}"
                )));
            }
            Any::Null | Any::Undefined => AttrValue::Null,
            other => {
                return Err(CollabError::schema(format!(
                    "unsupported value in mark attr `{k}`: {other:?}"
                )));
            }
        };
        out = out.with(k.as_str(), av);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn schema() -> Rc<Schema> {
        Rc::new(Schema::starter_kit())
    }

    #[test]
    fn build_block_rejects_non_textblock_type_inbound() {
        // A22 inbound guard: a peer CRDT carrying a nested block type must NOT be
        // silently materialized.
        let s = schema();
        let bad = BlockData {
            type_name: "blockquote".into(),
            attrs: Attrs::new(),
            text: "x".into(),
            marks: vec![],
        };
        let err = build_block(&s, &bad).unwrap_err();
        assert!(matches!(err, CollabError::Unsupported(_)), "got {err:?}");

        let unknown = BlockData {
            type_name: "not_a_real_type".into(),
            attrs: Attrs::new(),
            text: "x".into(),
            marks: vec![],
        };
        assert!(matches!(
            build_block(&s, &unknown).unwrap_err(),
            CollabError::Unsupported(_)
        ));

        // a flat textblock is accepted
        let good = BlockData {
            type_name: "paragraph".into(),
            attrs: Attrs::new(),
            text: "ok".into(),
            marks: vec![],
        };
        assert!(build_block(&s, &good).is_ok());
    }

    #[test]
    fn the_two_atom_predicates_split_the_starter_kit_between_them() {
        // The pair of predicates that decide what "an atom" means here, pinned against
        // the starter kit. `horizontal_rule` is the block one (a block of its own, with
        // empty text); `image`/`hard_break` are the inline ones (one char inside a
        // block's text). `blockquote`/`table_row` are neither — they hold content — and
        // a textblock is neither, which is what keeps the three paths apart.
        let s = schema();
        let typ = |n: &str| s.node_type(n).expect("starter-kit type").clone();
        assert!(is_leaf_block_atom(&typ("horizontal_rule")));
        assert!(!is_inline_atom(&typ("horizontal_rule")));
        for inline_atom in ["image", "hard_break"] {
            assert!(
                is_inline_atom(&typ(inline_atom)),
                "{inline_atom} is an inline atom and is now in scope"
            );
            assert!(
                !is_leaf_block_atom(&typ(inline_atom)),
                "{inline_atom} is *inline*, so it is not the block kind"
            );
        }
        for container in ["blockquote", "table_row", "task_item", "list_item"] {
            assert!(
                !is_leaf_block_atom(&typ(container)) && !is_inline_atom(&typ(container)),
                "{container} holds content, so it is no kind of atom"
            );
        }
        for textblock in ["paragraph", "heading", "code_block"] {
            assert!(!is_leaf_block_atom(&typ(textblock)) && !is_inline_atom(&typ(textblock)));
            assert!(typ(textblock).is_textblock(), "and is a textblock instead");
        }
        // `text` is inline and a leaf, and is emphatically not an atom: the placeholder
        // stands in for a node that has no text, and text is the thing that has it.
        assert!(!is_inline_atom(&typ("text")));
    }

    #[test]
    fn a_block_atom_reads_and_builds_as_a_block_with_empty_text() {
        // The representation: `read_block`/`build_block` are inverses for an atom, with
        // the empty text standing in for "this node has no content".
        let s = schema();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let data = read_block(&hr).unwrap();
        assert_eq!(
            data,
            BlockData {
                type_name: "horizontal_rule".into(),
                attrs: Attrs::new(),
                text: String::new(),
                marks: vec![],
            }
        );
        assert_eq!(build_block(&s, &data).unwrap(), hr);
        // And `read_node` admits it at the top level, as the same `Block` shape.
        assert_eq!(read_node(&hr).unwrap(), NodeData::Block(data));
    }

    #[test]
    fn build_block_refuses_an_atom_that_carries_text() {
        // Only a foreign or corrupt writer can produce this — `reconcile_node` replaces
        // a node rather than let a live `Text` survive a retype into an atom — and it is
        // a shape no model can express, so it must fail loud rather than be dropped.
        let s = schema();
        let with_text = BlockData {
            type_name: "horizontal_rule".into(),
            attrs: Attrs::new(),
            text: "smuggled".into(),
            marks: vec![],
        };
        let err = build_block(&s, &with_text).unwrap_err();
        assert!(matches!(err, CollabError::Unsupported(_)), "got {err:?}");
    }

    /// `read_block` → `build_block` is an identity for `block`, and the block data it
    /// went through, so a test can assert on the wire shape as well as the round trip.
    fn round_trip(s: &Schema, block: &Node) -> BlockData {
        let data = read_block(block).expect("read_block");
        assert_eq!(
            &build_block(s, &data).expect("build_block"),
            block,
            "the rebuilt block is the identical model tree"
        );
        data
    }

    /// An `image` with the starter kit's three attrs.
    fn image(s: &Schema, src: &str) -> Node {
        s.create_node(
            "image",
            Attrs::new()
                .with("src", AttrValue::from(src))
                .with("alt", AttrValue::from("a cat"))
                .with("title", AttrValue::from("Cat")),
            Fragment::empty(),
        )
        .unwrap()
    }

    #[test]
    fn an_inline_atom_is_one_placeholder_char_carrying_the_reserved_attribute() {
        // The representation, stated once: the atom occupies exactly one char of the
        // block's text — the same single model position a leaf node has — and its type
        // and attrs ride in an `@atom` span over that char, alongside (not instead of)
        // any real marks.
        let s = schema();
        let para = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text("look: ").unwrap(),
                    image(&s, "cat.png"),
                    s.text(" and on").unwrap(),
                ]),
            )
            .unwrap();
        let data = round_trip(&s, &para);
        assert_eq!(data.text, "look: \u{FFFC} and on");
        assert_eq!(
            data.marks,
            vec![SpanMark {
                name: ATOM_MARK.into(),
                attrs: Attrs::new()
                    .with("alt", AttrValue::from("a cat"))
                    .with("src", AttrValue::from("cat.png"))
                    .with("title", AttrValue::from("Cat"))
                    .with(ATOM_TYPE, AttrValue::from("image")),
                start: 6,
                end: 7,
            }],
            "one span, over the placeholder char only"
        );
        // And the value survives the *mark* encoding unchanged, which is what lets
        // `read_text_data` read an atom back with no case of its own.
        assert_eq!(
            decode_mark_value(&encode_mark_value(&data.marks[0].attrs)).unwrap(),
            data.marks[0].attrs
        );
    }

    #[test]
    fn a_hard_break_round_trips_at_the_start_middle_and_end_of_a_paragraph() {
        // Shift+Enter is the everyday inline atom, and the three positions are the
        // three ways the run-splitting in `build_block` can be off by one.
        let s = schema();
        let br = || s.branch("hard_break", Fragment::empty()).unwrap();
        for children in [
            vec![br(), s.text("after").unwrap()],
            vec![s.text("a").unwrap(), br(), s.text("b").unwrap()],
            vec![s.text("before").unwrap(), br()],
        ] {
            let para = s
                .branch("paragraph", Fragment::from_children(children))
                .unwrap();
            round_trip(&s, &para);
        }
    }

    #[test]
    fn two_adjacent_atoms_are_two_nodes_even_though_they_coalesce_into_one_span() {
        // Two identical hard breaks are one coalesced `@atom` span over two chars (the
        // canonical span lists coalesce by (name, attrs), and these agree in both) —
        // and they must still rebuild as TWO nodes, which is why `build_block` makes a
        // node per placeholder char and not per span.
        let s = schema();
        let br = || s.branch("hard_break", Fragment::empty()).unwrap();
        let para = s
            .branch("paragraph", Fragment::from_children(vec![br(), br()]))
            .unwrap();
        let data = round_trip(&s, &para);
        assert_eq!(data.text, "\u{FFFC}\u{FFFC}");
        assert_eq!(data.marks.len(), 1, "coalesced into one span: {data:?}");
        assert_eq!((data.marks[0].start, data.marks[0].end), (0, 2));
        assert_eq!(build_block(&s, &data).unwrap().child_count(), 2);
    }

    #[test]
    fn an_atom_inside_a_mark_that_also_covers_the_text_on_both_sides() {
        // The interleaving case: a bold run spanning text-atom-text is ONE bold span
        // over all of it, and the atom breaks the text runs without breaking the mark.
        let s = schema();
        let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
        let para = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text_with_marks("a", vec![bold.clone()]).unwrap(),
                    image(&s, "cat.png").with_marks(vec![bold.clone()]),
                    s.text_with_marks("b", vec![bold]).unwrap(),
                ]),
            )
            .unwrap();
        let data = round_trip(&s, &para);
        let bold_spans: Vec<&SpanMark> = data.marks.iter().filter(|m| m.name == "bold").collect();
        assert_eq!(bold_spans.len(), 1, "one coalesced bold span: {data:?}");
        assert_eq!((bold_spans[0].start, bold_spans[0].end), (0, 3));
    }

    #[test]
    fn an_atom_carries_its_own_mark() {
        // A linked image: the mark is an ordinary span over the atom's char, and the
        // rebuilt node carries it — which is what `Node::with_marks` is for (a
        // non-text node cannot be built through `Schema::text_with_marks`).
        let s = schema();
        let link = Mark::new(
            s.mark_type("link").unwrap().clone(),
            Attrs::new().with("href", AttrValue::from("https://example.test/")),
        );
        let para = s
            .branch(
                "paragraph",
                Fragment::from_node(image(&s, "cat.png").with_marks(vec![link])),
            )
            .unwrap();
        let data = round_trip(&s, &para);
        let names: Vec<&str> = data.marks.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec![ATOM_MARK, "link"], "both spans, over one char");
        assert_eq!(
            build_block(&s, &data).unwrap().child(0).marks().len(),
            1,
            "the rebuilt atom keeps its link"
        );
    }

    #[test]
    fn a_literal_placeholder_char_with_no_atom_attribute_is_text() {
        // A user can paste a U+FFFC. Nothing marks it as an atom, so it is a character
        // like any other and must round-trip as one, not vanish and not error.
        let s = schema();
        let para = s
            .branch(
                "paragraph",
                Fragment::from_node(s.text("a\u{FFFC}b").unwrap()),
            )
            .unwrap();
        let data = round_trip(&s, &para);
        assert_eq!(data.text, "a\u{FFFC}b");
        assert!(data.marks.is_empty(), "no atom span: {data:?}");
    }

    #[test]
    fn an_atom_attribute_over_an_ordinary_char_is_ignored_rather_than_fatal() {
        // The one corruption that is NOT fail-loud, and the reason is in
        // `is_atom_char`: an ordinary concurrent edit produces it. yrs swallows an
        // insert at the end boundary of a formatted range into that range, so a peer
        // typing right after an image inherits its `@atom` attribute — and there is no
        // atom node to lose, only a formatting artifact over a char that is already
        // text. Erroring would poison a live session for it.
        let s = schema();
        let stray = BlockData {
            type_name: "paragraph".into(),
            attrs: Attrs::new(),
            text: "xy".into(),
            marks: vec![SpanMark {
                name: ATOM_MARK.into(),
                attrs: Attrs::new().with(ATOM_TYPE, AttrValue::from("image")),
                start: 0,
                end: 2,
            }],
        };
        let built = build_block(&s, &stray).expect("a stray atom attribute is not fatal");
        assert_eq!(built.child_count(), 1);
        assert_eq!(built.child(0).text(), Some("xy"), "still plain text");
    }

    #[test]
    fn a_corrupt_atom_attribute_with_a_real_placeholder_fails_loud() {
        // Where the placeholder IS there, the attribute is the only thing saying what
        // node to build, so every way of it being wrong is a node that would otherwise
        // be silently dropped or silently invented (A22).
        let s = schema();
        let block = |attrs: Attrs| BlockData {
            type_name: "paragraph".into(),
            attrs: Attrs::new(),
            text: "\u{FFFC}".into(),
            marks: vec![SpanMark {
                name: ATOM_MARK.into(),
                attrs,
                start: 0,
                end: 1,
            }],
        };
        // A type the schema does not know.
        let err =
            build_block(&s, &block(Attrs::new().with(ATOM_TYPE, "no_such_node"))).unwrap_err();
        assert!(matches!(err, CollabError::Unsupported(_)), "got {err:?}");
        // A type that is a block, or a textblock, or text — none can stand in a line.
        for not_inline in ["horizontal_rule", "paragraph", "blockquote", "text"] {
            let err =
                build_block(&s, &block(Attrs::new().with(ATOM_TYPE, not_inline))).unwrap_err();
            assert!(
                matches!(err, CollabError::Unsupported(_)),
                "{not_inline}: got {err:?}"
            );
        }
        // No type at all, or one that is not a string.
        let err = build_block(&s, &block(Attrs::new().with("src", "cat.png"))).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
        let err = build_block(&s, &block(Attrs::new().with(ATOM_TYPE, 7i64))).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
        // An unknown key in the reserved namespace: a shape a future wire version
        // might mean something by, refused rather than half-read.
        let err = build_block(
            &s,
            &block(Attrs::new().with(ATOM_TYPE, "image").with("@extra", true)),
        )
        .unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
    }

    #[test]
    fn read_block_refuses_an_inline_atom_carrying_content() {
        // Outbound guard: a node built past its own schema. One char must not stand
        // for content that is really there.
        let s = schema();
        let stuffed = s
            .create_node(
                "image",
                Attrs::new().with("src", AttrValue::from("cat.png")),
                Fragment::from_node(s.text("smuggled").unwrap()),
            )
            .unwrap();
        let para = s.branch("paragraph", Fragment::from_node(stuffed)).unwrap();
        let err = read_block(&para).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
    }

    #[test]
    fn read_block_refuses_an_atom_whose_own_attrs_invade_the_reserved_namespace() {
        // The flat `@atom` value works only because `@type` cannot collide with one of
        // the atom's own attrs. An atom that carries a reserved key is refused rather
        // than having it silently overwritten.
        let s = schema();
        let odd = s
            .create_node(
                "image",
                Attrs::new()
                    .with("src", AttrValue::from("cat.png"))
                    .with(ATOM_TYPE, AttrValue::from("hard_break")),
                Fragment::empty(),
            )
            .unwrap();
        let para = s.branch("paragraph", Fragment::from_node(odd)).unwrap();
        let err = read_block(&para).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
    }

    #[test]
    fn a_schema_mark_in_the_reserved_namespace_fails_loud() {
        // The other half of the same guarantee, from the schema's side: a mark type
        // named `@atom` would make a mark and an atom indistinguishable on the wire, in
        // both directions, so the projection refuses to read into such a schema at all.
        let mut builder = rinch_editor_core::SchemaBuilder::new();
        builder = builder.node(
            "doc",
            rinch_editor_core::NodeSpec::builder("doc")
                .content("block+")
                .build(),
        );
        builder = builder.node(
            "paragraph",
            rinch_editor_core::NodeSpec::builder("paragraph")
                .content("inline*")
                .group("block")
                .build(),
        );
        builder = builder.node(
            "text",
            rinch_editor_core::NodeSpec::builder("text")
                .group("inline")
                .inline()
                .build(),
        );
        builder = builder.mark(ATOM_MARK, rinch_editor_core::MarkSpec::simple(ATOM_MARK));
        let hostile = builder.build();
        let err = build_block(
            &hostile,
            &BlockData {
                type_name: "paragraph".into(),
                attrs: Attrs::new(),
                text: "hi".into(),
                marks: vec![],
            },
        )
        .unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
    }

    #[test]
    fn an_inline_atom_is_still_not_a_block_on_its_own() {
        // The one boundary this does NOT move: an `image` is in scope *inside* a
        // textblock's text, never as a top-level node of its own.
        let s = schema();
        let err = read_node(&image(&s, "cat.png")).unwrap_err();
        assert!(matches!(err, CollabError::Unsupported(_)), "got {err:?}");
    }

    #[test]
    fn to_doc_fails_loud_on_a_non_flat_block_in_the_crdt() {
        // End-to-end inbound path: hand-corrupt a block's type in the CRDT and confirm
        // `to_doc` errors rather than producing a non-flat node.
        let s = schema();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        let cdoc = CollabDoc::from_doc(&doc).unwrap();
        {
            let mut txn = cdoc.doc.transact_mut();
            let block = child_map(&txn, &cdoc.content, 0).unwrap();
            block.insert(&mut txn, TYPE, Any::String("blockquote".into()));
        }
        assert!(matches!(
            cdoc.to_doc(&s).unwrap_err(),
            CollabError::Unsupported(_)
        ));
    }

    #[test]
    fn decode_mark_value_fails_loud_on_corruption() {
        // attr-less encoding decodes to empty attrs
        assert!(decode_mark_value(&Any::Bool(true)).unwrap().is_empty());
        // valid attr map decodes
        let attrs = decode_mark_value(&encode_mark_value(
            &Attrs::new().with("href", AttrValue::from("x")),
        ))
        .unwrap();
        assert_eq!(attrs.get_str("href"), Some("x"));
        // malformed values fail loud rather than silently dropping the attrs
        assert!(decode_mark_value(&Any::String("not a map".into())).is_err()); // wrong kind
        assert!(decode_mark_value(&Any::Array(Arc::from([Any::Bool(true)]))).is_err()); // not a map
        assert!(
            decode_mark_value(&Any::Map(Arc::new(HashMap::from([(
                "n".to_string(),
                Any::Number(1.5)
            )]))))
            .is_err()
        ); // non-integer
        assert!(decode_mark_value(&Any::BigInt(5)).is_err()); // wrong kind
    }

    /// Encode a whole foreign document as an update, for the `load` guard cases.
    fn foreign_update(build: impl FnOnce(&Doc)) -> Vec<u8> {
        let (doc, _outbox, _sub) = CollabDoc::blank(None);
        build(&doc);
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default())
    }

    #[test]
    fn load_fails_loud_on_bytes_that_are_not_a_projection() {
        // A decodable yrs update that is not one of our projections must not be silently
        // adopted as an empty document — a guest joining on it would then collaborate on
        // content no peer shares, which is the silent-divergence class A22 exists to kill.
        //
        // The guard cannot key off the root's *type*: a root arriving from a peer reads as
        // `UndefinedRef` whatever it is, and asking for it as an array reinterprets
        // whatever is there. Each case below is a different way that reinterpretation can
        // succeed, which is why the guard validates the blocks instead.

        // (a) a differently-named root: `content` is absent, so it reinterprets as empty.
        let elsewhere = foreign_update(|d| {
            let m = d.get_or_insert_map("something_else");
            m.insert(&mut d.transact_mut(), "k", Any::Bool(true));
        });
        assert!(matches!(
            CollabDoc::load(&elsewhere).unwrap_err(),
            CollabError::Schema(_)
        ));

        // (b) a MAP root that happens to be named `content`: reinterprets as a
        // zero-length array, so a type-blind `get_array(..).is_some()` check would let it
        // through and `to_doc` would invent a paragraph.
        let map_root = foreign_update(|d| {
            let m = d.get_or_insert_map(CONTENT);
            m.insert(&mut d.transact_mut(), "k", Any::Bool(true));
        });
        assert!(matches!(
            CollabDoc::load(&map_root).unwrap_err(),
            CollabError::Schema(_)
        ));

        // (c) a TEXT root named `content`: reinterprets as a *non-empty* array of single
        // characters, so an emptiness check alone would let it through too.
        let text_root = foreign_update(|d| {
            let t = d.get_or_insert_text(CONTENT);
            t.insert(&mut d.transact_mut(), 0, "hello");
        });
        assert!(matches!(
            CollabDoc::load(&text_root).unwrap_err(),
            CollabError::Schema(_)
        ));

        // (d) an array root named `content` holding junk rather than block maps.
        let junk_array = foreign_update(|d| {
            let a = d.get_or_insert_array(CONTENT);
            a.insert(&mut d.transact_mut(), 0, Any::String("junk".into()));
        });
        assert!(matches!(
            CollabDoc::load(&junk_array).unwrap_err(),
            CollabError::Schema(_)
        ));

        // Undecodable bytes are an engine error, not a schema one.
        assert!(matches!(
            CollabDoc::load(&[0xff, 0xff, 0xff, 0xff]).unwrap_err(),
            CollabError::Engine(_)
        ));

        // A real projection loads and rebuilds identically.
        let s = schema();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        let cdoc = CollabDoc::from_doc(&doc).unwrap();
        let loaded = CollabDoc::load(&cdoc.save()).unwrap();
        assert_eq!(loaded.to_doc(&s).unwrap(), doc);
    }

    /// One projectable paragraph, for building foreign documents that *look* like a
    /// projection.
    fn plausible_block() -> NodeData {
        NodeData::Block(BlockData {
            type_name: "paragraph".into(),
            attrs: Attrs::new(),
            text: "hi".into(),
            marks: vec![],
        })
    }

    #[test]
    fn load_requires_the_projection_format_marker() {
        // The marker is the discriminator for "are these our bytes at all", which is what
        // lets `load` admit a legitimately empty document (see the test below) without
        // also admitting a foreign CRDT that merely reinterprets as an empty `content`
        // array. Neither case below is caught by the content check: both carry a content
        // array whose entries read back as perfectly valid projected nodes.

        // A foreign document that would sail through a content-only guard: the right root
        // name, holding a real projected block — but no format marker.
        let markerless = foreign_update(|d| {
            let a = d.get_or_insert_array(CONTENT);
            let mut txn = d.transact_mut();
            insert_node(&mut txn, &a, 0, &plausible_block()).unwrap();
        });
        let err = CollabDoc::load(&markerless).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");

        // A *wrong* marker — e.g. a peer still speaking an older wire shape — must fail
        // just as loudly as a missing one, rather than being half-understood.
        let wrong_version = foreign_update(|d| {
            let a = d.get_or_insert_array(CONTENT);
            let m = d.get_or_insert_map(META);
            let mut txn = d.transact_mut();
            m.insert(
                &mut txn,
                FORMAT,
                Any::String("rinch-editor-collab/yrs-0".into()),
            );
            insert_node(&mut txn, &a, 0, &plausible_block()).unwrap();
        });
        let err = CollabDoc::load(&wrong_version).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");

        // A marker of the wrong *kind* (not a string) is not a marker either.
        let wrong_kind = foreign_update(|d| {
            let a = d.get_or_insert_array(CONTENT);
            let m = d.get_or_insert_map(META);
            let mut txn = d.transact_mut();
            m.insert(&mut txn, FORMAT, Any::Bool(true));
            insert_node(&mut txn, &a, 0, &plausible_block()).unwrap();
        });
        let err = CollabDoc::load(&wrong_kind).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");

        // The `meta` root is subject to the same reinterpretation hazard as `content` — a
        // foreign root of ANY type arrives untagged and is reinterpreted as the map we ask
        // for. Reading a missing key out of a reinterpreted sequence must fail loud, not
        // panic. Both sequence kinds, since they reinterpret differently.
        for (label, build) in [
            (
                "text root named meta",
                Box::new(|d: &Doc| {
                    let t = d.get_or_insert_text(META);
                    t.insert(&mut d.transact_mut(), 0, "format");
                }) as Box<dyn FnOnce(&Doc)>,
            ),
            (
                "array root named meta",
                Box::new(|d: &Doc| {
                    let a = d.get_or_insert_array(META);
                    a.insert(&mut d.transact_mut(), 0, Any::String(FORMAT_TAG.into()));
                }),
            ),
        ] {
            let bytes = foreign_update(build);
            let err = CollabDoc::load(&bytes).unwrap_err();
            assert!(
                matches!(err, CollabError::Schema(_)),
                "{label}: got {err:?}"
            );
        }

        // And the marker alone is not enough: the content entries are still validated.
        let marked_junk = foreign_update(|d| {
            let a = d.get_or_insert_array(CONTENT);
            let m = d.get_or_insert_map(META);
            let mut txn = d.transact_mut();
            m.insert(&mut txn, FORMAT, Any::String(FORMAT_TAG.into()));
            a.insert(&mut txn, 0, Any::String("junk".into()));
        });
        let err = CollabDoc::load(&marked_junk).unwrap_err();
        assert!(matches!(err, CollabError::Schema(_)), "got {err:?}");
    }

    #[test]
    fn load_accepts_a_projection_that_has_no_blocks_left() {
        // Zero blocks is a legitimate converged state (#192: two peers deleting different
        // blocks concurrently), and refusing it locked a late joiner out of such a
        // session. The marker — not emptiness — is what tells our bytes from a stranger's,
        // so the empty document loads and projects to the starter paragraph the editor
        // schema requires.
        let s = schema();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
        let cdoc = CollabDoc::from_doc(&doc).unwrap();

        // Empty the content array the way a converged double-delete does.
        {
            let mut txn = cdoc.doc.transact_mut();
            cdoc.content.remove(&mut txn, 0);
        }
        assert_eq!(cdoc.content.len(&cdoc.doc.transact()), 0);

        let starter = s
            .branch(
                "doc",
                Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
            )
            .unwrap();
        assert_eq!(
            cdoc.to_doc(&s).unwrap(),
            starter,
            "an empty projection reads back as the starter paragraph"
        );

        let loaded = CollabDoc::load(&cdoc.save()).expect("a zero-block snapshot is joinable");
        assert_eq!(
            loaded.to_doc(&s).unwrap(),
            starter,
            "and a late joiner adopts that same starter paragraph"
        );
    }

    #[test]
    fn two_independently_created_projections_merge_and_keep_one_readable_marker() {
        // Every other path creates the second peer by *joining* the first
        // (`load`/`from_bytes`), so both share one lineage and one marker write. Two peers
        // that each ran `from_doc` instead both wrote `meta.format` under their own client
        // id, and merging them is a map-key conflict. yrs resolves it to one writer; both
        // wrote the same value, so the marker must still read back — and the merged
        // document must still be joinable from its own snapshot.
        let s = schema();
        let doc_of = |t: &str| {
            let p = s
                .branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap();
            s.branch("doc", Fragment::from_node(p)).unwrap()
        };
        let mut a = CollabDoc::from_doc(&doc_of("alpha")).unwrap();
        let b = CollabDoc::from_doc(&doc_of("beta")).unwrap();
        a.merge_from(&b).unwrap();

        let merged = a.to_doc(&s).unwrap();
        assert_eq!(merged.child_count(), 2, "both peers' blocks survived");
        let meta = a.doc.get_or_insert_map(META);
        assert!(
            matches!(
                meta.get(&a.doc.transact(), FORMAT),
                Some(Out::Any(Any::String(tag))) if &*tag == FORMAT_TAG
            ),
            "the marker survives the conflicting write"
        );
        assert_eq!(
            CollabDoc::load(&a.save()).unwrap().to_doc(&s).unwrap(),
            merged,
            "and the merged document is joinable from its own snapshot"
        );
    }

    #[test]
    fn the_outbox_collects_local_writes_and_skips_applied_bytes() {
        // The broadcast contract: a locally-projected transaction is a broadcast, an
        // applied peer update is not (re-broadcasting it would echo).
        let s = schema();
        let para = s
            .branch("paragraph", Fragment::from_node(s.text("hi").unwrap()))
            .unwrap();
        let doc = s.branch("doc", Fragment::from_node(para)).unwrap();

        let mut a = CollabDoc::from_doc(&doc).unwrap();
        assert_eq!(
            lock_outbox(&a.outbox).len(),
            1,
            "the initial projection is a local write and is broadcast"
        );

        let mut b = CollabDoc::load(&a.save()).unwrap();
        assert!(
            lock_outbox(&b.outbox).is_empty(),
            "joining from a snapshot must not queue the host's document for re-broadcast"
        );

        // A local edit on A fills A's outbox; applying it on B leaves B's empty.
        let edited = {
            let p = s
                .branch("paragraph", Fragment::from_node(s.text("hi!").unwrap()))
                .unwrap();
            s.branch("doc", Fragment::from_node(p)).unwrap()
        };
        a.project_change(&doc, &edited).unwrap();
        let delta = a.take_outbox().unwrap();
        assert!(!delta.is_empty(), "a local edit produces a delta");
        assert!(
            a.take_outbox().unwrap().is_empty(),
            "a drained outbox is empty, so a second save sends nothing"
        );

        b.apply_update(&delta).unwrap();
        assert!(
            lock_outbox(&b.outbox).is_empty(),
            "an applied peer delta must not be queued for re-broadcast"
        );
    }

    #[test]
    fn several_parked_updates_merge_into_one_delta_that_converges_a_peer() {
        // The multi-update branch of `take_outbox`. More than one local transaction can
        // pile up before a single save (the initial projection plus a first edit is the
        // everyday case), and those updates must be *merged* into one valid update —
        // concatenating them would produce bytes that are not an update at all — which a
        // peer then applies in one go.
        let s = schema();
        let doc = |texts: &[&str]| {
            let blocks: Vec<Node> = texts
                .iter()
                .map(|t| {
                    let content = if t.is_empty() {
                        Fragment::empty()
                    } else {
                        Fragment::from_node(s.text(t).unwrap())
                    };
                    s.branch("paragraph", content).unwrap()
                })
                .collect();
            s.branch("doc", Fragment::from_children(blocks)).unwrap()
        };

        let d0 = doc(&["one", "two"]);
        let mut a = CollabDoc::from_doc(&d0).unwrap();
        let mut b = CollabDoc::load(&a.save()).unwrap();
        // Drop the initial projection: the peer already has it from the snapshot, so what
        // follows is a merge of edits only.
        let _ = a.take_outbox().unwrap();

        // Three separate transactions, touching different blocks and adding a mark, so the
        // merge has to carry insertions in two text objects plus formatting.
        let d1 = doc(&["oneA", "two"]);
        let d2 = doc(&["oneA", "twoB"]);
        let d3 = doc(&["oneA", "twoB!"]);
        a.project_change(&d0, &d1).unwrap();
        a.project_change(&d1, &d2).unwrap();
        a.project_change(&d2, &d3).unwrap();
        assert_eq!(
            lock_outbox(&a.outbox).len(),
            3,
            "each local transaction parks its own update, so the merge branch is what runs"
        );

        let merged = a.take_outbox().unwrap();
        b.apply_update(&merged).unwrap();
        assert_eq!(
            b.to_doc(&s).unwrap(),
            a.to_doc(&s).unwrap(),
            "one apply of the merged delta converges the peer"
        );
        assert_eq!(
            b.to_doc(&s).unwrap(),
            d3,
            "and lands exactly on the document the three edits produced"
        );
        assert!(
            a.take_outbox().unwrap().is_empty(),
            "the merge drained the outbox"
        );
    }

    #[test]
    fn a_broadcast_delta_does_not_regrow_with_deletion_history() {
        // The regression the outbox exists to prevent. `encode_diff_v1` writes the whole
        // delete set whatever state vector it is given, so a diff-based delta re-carries
        // every past deletion on every keystroke and a "nothing new" diff is never empty.
        // A transaction's own update carries only its own change.
        let s = schema();
        let text = |t: &str| {
            let p = s
                .branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
                .unwrap();
            s.branch("doc", Fragment::from_node(p)).unwrap()
        };

        let long: String = std::iter::repeat_n("abcdefghij", 20).collect();
        let start = text(&long);
        let mut cdoc = CollabDoc::from_doc(&start).unwrap();
        let _ = cdoc.take_outbox().unwrap(); // drop the initial projection

        // Delete a lot, in many separate transactions, to build up deletion history.
        let mut current = start;
        for _ in 0..20 {
            let shorter: String = current
                .child(0)
                .child(0)
                .text()
                .unwrap()
                .chars()
                .skip(5)
                .collect();
            let next = text(&shorter);
            cdoc.project_change(&current, &next).unwrap();
            let _ = cdoc.take_outbox().unwrap();
            current = next;
        }

        // With history accumulated: nothing new must mean literally nothing.
        assert!(
            cdoc.take_outbox().unwrap().is_empty(),
            "a save with no local edit since the last one must be empty even after deletions"
        );
        // And a one-character insert must cost about one character, not the history.
        let with_char: String =
            format!("{}Z", current.child(0).child(0).text().unwrap_or_default());
        let next = text(&with_char);
        cdoc.project_change(&current, &next).unwrap();
        let one_char = cdoc.take_outbox().unwrap();
        let history_diff = cdoc.diff_since(&cdoc.state_vector());
        assert!(
            one_char.len() < 40,
            "a 1-char edit's delta should be small, got {} bytes",
            one_char.len()
        );
        assert!(
            !history_diff.is_empty(),
            "precondition: the state-vector diff is NOT empty even with nothing new — \
             which is exactly why it cannot be the broadcast mechanism"
        );
    }

    #[test]
    fn char_offsets_convert_to_utf16_across_astral_pairs() {
        // Two astral characters, deliberately: yrs snaps an index that lands mid
        // surrogate pair to the nearest boundary and silently corrects the off-by-one,
        // so a single-astral case cannot tell a correct conversion from a broken one.
        let s = "a🐱b🐱c"; // chars 0..5, UTF-16 units 0..7
        assert_eq!(u16_offset(s, 0), 0);
        assert_eq!(u16_offset(s, 1), 1); // after 'a'
        assert_eq!(u16_offset(s, 2), 3); // after the first 🐱 (2 units)
        assert_eq!(u16_offset(s, 3), 4); // after 'b'
        assert_eq!(u16_offset(s, 4), 6); // after the second 🐱
        assert_eq!(u16_offset(s, 5), 7); // end
        // Past the end clamps rather than running off (remove_range would panic).
        assert_eq!(u16_offset(s, 99), 7);
        assert_eq!(u16_span(s, 1, 4), (1, 5)); // "🐱b🐱" is 5 UTF-16 units
    }
}
