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
//! nesting is just recursion on it. A node is *either* a textblock (carries a `text`)
//! *or* a container (carries a `content` array of child nodes):
//!
//! ```text
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
//! Supported: **flat text-blocks + marks** (`paragraph`/`heading`/`code_block` whose own
//! children are text nodes), and the **list containers** `bullet_list` / `ordered_list` /
//! `list_item`, nested into each other and around text-blocks to any depth.
//!
//! Everything else still **fails loud** with
//! [`CollabError::Unsupported`](crate::CollabError::Unsupported) — **never a silent
//! drop**: any other nested block (`blockquote`, `table`/`table_row`/cells,
//! `task_list`/`task_item`) and every inline atom (`hard_break`, `image`,
//! `horizontal_rule`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yrs::types::Attrs as YAttrs;
use yrs::types::text::YChange;
use yrs::updates::decoder::Decode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Doc, Map, MapPrelim, MapRef, OffsetKind, Options, Origin,
    Out, ReadTxn, StateVector, Subscription, Text, TextPrelim, TextRef, Transact, TransactionMut,
    Update,
};

use rinch_editor_core::{AttrValue, Attrs, Fragment, Mark, Node, Schema};

use crate::error::{CollabError, Result};

/// yrs key/root names used by the projection.
const CONTENT: &str = "content";
const TYPE: &str = "type";
const ATTRS: &str = "attrs";
const TEXT: &str = "text";

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

/// The plain text of a flat block plus the marks over it, ready to project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlockData {
    pub type_name: String,
    pub attrs: Attrs,
    pub text: String,
    pub marks: Vec<SpanMark>,
}

/// One projectable model node: either a flat text-block ([`BlockData`]) or a container
/// block (a list / list item) holding child nodes recursively. Structural equality is
/// canonical (marks are sorted in [`read_block`] / [`read_text_data`]), so comparing two
/// `NodeData` trees is the same as comparing the model nodes they came from — the
/// child-list diff in [`reconcile_child_list`] relies on that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeData {
    /// A flat text-block (`paragraph`/`heading`/`code_block`): its `text` + marks.
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
    fn blank() -> (Doc, Outbox, Subscription) {
        let doc = Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Default::default()
        });
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
        // Validate the whole document before opening the write transaction, so an
        // out-of-scope node leaves no half-built CRDT behind (design A22).
        let mut nodes = Vec::with_capacity(doc.child_count());
        for i in 0..doc.child_count() {
            nodes.push(read_node(doc.child(i))?);
        }

        let (ydoc, outbox, updates) = CollabDoc::blank();
        let content = ydoc.get_or_insert_array(CONTENT);
        // A plain local transaction, so the initial projection lands in the outbox and is
        // broadcast like any other local change. A peer that joined from a snapshot
        // already has it and applies it as a no-op (updates are idempotent).
        {
            let mut txn = ydoc.transact_mut();
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
    /// The guard validates **content, not the root's type**, because a root type carries
    /// no type tag on the wire: a root arriving from a peer reads as `Out::UndefinedRef`
    /// whatever it really is, and asking for it as an array *reinterprets* whatever is
    /// there (a foreign `Map` root named `content` then reads as a zero-length array, a
    /// `Text` root as an array of single characters). So the check is that the array is
    /// non-empty and that every entry reads back as a projected node — which is strictly
    /// stronger than the type check this replaced, since a `content` list full of junk
    /// would have passed that.
    pub fn load(bytes: &[u8]) -> Result<CollabDoc> {
        let update = Update::decode_v1(bytes)?;
        let (ydoc, outbox, updates) = CollabDoc::blank();
        {
            let mut txn = ydoc.transact_mut_with(Origin::from(ENGINE_APPLY_ORIGIN));
            txn.apply_update(update)?;
        }
        let content = ydoc.get_or_insert_array(CONTENT);
        {
            let txn = ydoc.transact();
            let n = content.len(&txn);
            if n == 0 {
                return Err(CollabError::schema(
                    "these CRDT bytes carry no `content` blocks — not a rinch editor projection",
                ));
            }
            for i in 0..n {
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
            // An editor doc is never empty; mirror the starter empty paragraph.
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
    if crdt_is_block != target_is_block {
        check_index(txn, list, index, false)?;
        list.remove(txn, index);
        return insert_node(txn, list, index, target);
    }

    // type — only write when it changed
    if node_type(txn, &node).as_deref() != Some(target.type_name()) {
        node.insert(txn, TYPE, Any::String(target.type_name().into()));
    }
    // attrs — only rebuild when they changed. Replacing the attrs object on every
    // keystroke would churn the CRDT and clobber a concurrent remote attr edit on
    // the same node.
    if node_attrs(txn, &node) != *target.attrs() {
        let attrs_obj = node.insert(txn, ATTRS, MapPrelim::default());
        write_attrs(txn, &attrs_obj, target.attrs());
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

/// Clear and reapply a `Text`'s formatting attributes to match `target` exactly.
/// `current` is the text's present string and `current_marks` the spans read out of it
/// (both **after** any splice), so the ranges being cleared are the live ones.
fn resync_marks(
    txn: &mut TransactionMut,
    text: &TextRef,
    current: &str,
    current_marks: &[SpanMark],
    target: &[SpanMark],
) {
    // Clear every currently-active mark over its range. Per the Yjs convention a
    // `null` attribute value removes that format.
    for m in current_marks {
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
        apply_mark(txn, text, current, m);
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
                "an embedded value inside a block's text is not supported (inline atoms \
                 such as image/hard_break are not yet projected)",
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
/// carrying a `text` object is a text-block; a node carrying a `content` array is a
/// container, read recursively. Fails loud on a node that is neither (a corrupt
/// projection), rather than materializing a broken shape.
fn read_node_data<T: ReadTxn>(txn: &T, list: &ArrayRef, index: u32) -> Result<NodeData> {
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
/// list containers. A flat text-block becomes [`NodeData::Block`]; a supported list
/// container ([`is_supported_container`]) becomes [`NodeData::Container`] over its
/// recursively-read children. Anything else — an unsupported nested block (`blockquote`,
/// table, task list) or an inline atom — fails loud (design A22).
pub(crate) fn read_node(node: &Node) -> Result<NodeData> {
    if node.is_textblock() {
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
        "node `{}` is not a flat text-block or a supported list container \
         (bullet_list/ordered_list/list_item); other nested blocks, tables and inline \
         atoms are not yet supported",
        node.type_name()
    )))
}

/// Validate a model block is a flat textblock and extract its projectable data. Marks
/// are returned in canonical `(start, end, name)` order — matching [`read_text_data`] —
/// so a [`NodeData`] read from the model compares equal to the same node read back from
/// the CRDT.
pub(crate) fn read_block(block: &Node) -> Result<BlockData> {
    if !block.is_textblock() {
        return Err(CollabError::unsupported(format!(
            "block `{}` is not a flat text-block (nested blocks/atoms are not yet supported)",
            block.type_name()
        )));
    }
    let mut text = String::new();
    let mut marks: Vec<SpanMark> = Vec::new();
    for i in 0..block.child_count() {
        let child = block.child(i);
        let Some(t) = child.text() else {
            return Err(CollabError::unsupported(format!(
                "inline node `{}` (an atom such as image/hard_break) is not yet supported",
                child.type_name()
            )));
        };
        let start = text.chars().count();
        text.push_str(t);
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
/// at mark boundaries, build a text node per run, assemble the block.
fn build_block(schema: &Schema, b: &BlockData) -> Result<Node> {
    // Inbound scope guard (A22): a peer CRDT must not be able to materialize a
    // non-flat block here — `create_node` would happily build a `blockquote`/`list`,
    // silently breaking the flat-only invariant the outbound `read_block` enforces.
    let typ = schema.node_type(&b.type_name).ok_or_else(|| {
        CollabError::unsupported(format!("unknown block type `{}` in CRDT", b.type_name))
    })?;
    if !typ.is_textblock() {
        return Err(CollabError::unsupported(format!(
            "block `{}` is not a flat text-block (nested blocks/atoms are not yet supported)",
            b.type_name
        )));
    }
    let chars: Vec<char> = b.text.chars().collect();
    let mut runs: Vec<Node> = Vec::new();
    if !chars.is_empty() {
        let mut run_start = 0usize;
        let mut cur = marks_at(schema, &b.marks, 0)?;
        for i in 1..=chars.len() {
            let here = if i < chars.len() {
                marks_at(schema, &b.marks, i)?
            } else {
                Vec::new()
            };
            let boundary = i == chars.len() || !same_mark_set(&cur, &here);
            if boundary {
                let s: String = chars[run_start..i].iter().collect();
                let node = schema
                    .text_with_marks(&s, cur.clone())
                    .map_err(CollabError::from)?;
                runs.push(node);
                run_start = i;
                cur = here;
            }
        }
    }
    let attrs = b.attrs.clone();
    schema
        .create_node(&b.type_name, attrs, Fragment::from_children(runs))
        .map_err(CollabError::from)
}

/// The model marks active at char index `i`, resolved against the schema.
fn marks_at(schema: &Schema, spans: &[SpanMark], i: usize) -> Result<Vec<Mark>> {
    let mut out = Vec::new();
    for s in spans {
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
        let (doc, _outbox, _sub) = CollabDoc::blank();
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
