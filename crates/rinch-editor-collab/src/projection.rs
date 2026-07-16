//! [`CollabDoc`] — the Automerge projection of a flat editor document.
//!
//! Automerge is **not** the model (design §8). The model lives in `rinch-editor-core`;
//! this is a *projection* of it onto a CRDT so concurrent edits converge. The two are
//! kept byte-for-byte equivalent by the invariant **`model ≡ project(model)`**: every
//! local step is projected onto the CRDT ([`crate::project`]), every remote CRDT change
//! is translated back into steps ([`crate::remote`]). Convergence then follows from
//! Automerge's own convergence.
//!
//! ## Wire shape (rich-text projection)
//!
//! Every model node — at any depth — projects onto the **same** Automerge `Map` shape;
//! nesting is just recursion on it. A node is *either* a textblock (carries a `text`)
//! *or* a container (carries a `content` list of child nodes):
//!
//! ```text
//! ROOT (Map)
//!   "content" -> List<Node>
//!     Node (Map):
//!       "type"  -> Str             // "paragraph" | "heading" | "bullet_list" | …
//!       "attrs" -> Map<Str,scalar> // e.g. heading {"level": 2}, ordered_list {"start": 3}
//!       and then EXACTLY ONE of:
//!         "text"    -> Text        // a textblock's plain text, codepoint-indexed
//!           (marks applied to "text" via doc.mark(): name = mark type;
//!            value = Bool(true) for an attr-less mark, or Str(json) carrying its attrs)
//!         "content" -> List<Node>  // a container block's child nodes, recursively
//! ```
//!
//! One `Text` per textblock with Automerge marks over it is the *rich-text* model — text
//! and formatting merge independently, which is exactly the "concurrent insert/format"
//! convergence the milestone requires. Automerge `Text` is **codepoint-indexed**, which
//! matches `rinch-editor-core`'s char-based positions exactly — no UTF conversion. A
//! textblock keeps its own `Text` object however deeply it is nested, so concurrent edits
//! to *different* list items are edits to *different* `Text` objects and merge without
//! loss.
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

use automerge::marks::{ExpandMark, Mark as AmMark};
use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, Value};

use rinch_editor_core::{AttrValue, Attrs, Fragment, Mark, Node, Schema};

use crate::error::{CollabError, Result};

/// Automerge key/value names used by the projection.
const CONTENT: &str = "content";
const TYPE: &str = "type";
const ATTRS: &str = "attrs";
const TEXT: &str = "text";

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
/// canonical (marks are sorted in [`read_block`] / [`CollabDoc::read_text_marks`]), so
/// comparing two `NodeData` trees is the same as comparing the model nodes they came
/// from — the child-list diff in [`CollabDoc::reconcile_child_list`] relies on that.
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
/// `content` list of child nodes instead of a `text`. Deliberately a whitelist: every
/// other non-text-block node (`blockquote`, tables, `task_list`/`task_item`, atoms)
/// stays [`CollabError::Unsupported`] so an unsupported shape fails loud rather than
/// being silently mangled.
fn is_supported_container(type_name: &str) -> bool {
    matches!(type_name, "bullet_list" | "ordered_list" | "list_item")
}

/// An Automerge document projecting a flat editor document.
#[derive(Debug)]
pub struct CollabDoc {
    /// The Automerge document. `pub(crate)` so [`crate::sync`] can drive the transport.
    pub(crate) doc: AutoCommit,
    /// The root `content` list object id.
    pub(crate) content: ObjId,
}

impl CollabDoc {
    /// Build a fresh projection from a model document. Fails loud
    /// ([`CollabError::Unsupported`]) on any node outside the staged scope.
    pub fn from_doc(doc: &Node) -> Result<CollabDoc> {
        let mut am = AutoCommit::new();
        let content = am.put_object(automerge::ROOT, CONTENT, ObjType::List)?;
        let mut cdoc = CollabDoc {
            doc: am,
            content: content.clone(),
        };
        for i in 0..doc.child_count() {
            let node = read_node(doc.child(i))?;
            cdoc.insert_node(&content, i, &node)?;
        }
        Ok(cdoc)
    }

    /// Load a projection from saved Automerge bytes (a peer's document).
    pub fn load(bytes: &[u8]) -> Result<CollabDoc> {
        let am = AutoCommit::load(bytes)?;
        let content = match am.get(automerge::ROOT, CONTENT)? {
            Some((Value::Object(ObjType::List), id)) => id,
            _ => {
                return Err(CollabError::schema(
                    "loaded automerge doc has no `content` list",
                ));
            }
        };
        Ok(CollabDoc { doc: am, content })
    }

    /// Save the whole projection (for forking a peer).
    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    /// The number of blocks.
    pub(crate) fn block_count(&self) -> usize {
        self.doc.length(&self.content)
    }

    /// The map object of the child at `index` of a content list (top-level `content` or
    /// a container's nested `content`).
    pub(crate) fn child_map(&self, list: &ObjId, index: usize) -> Option<ObjId> {
        match self.doc.get(list, index) {
            Ok(Some((Value::Object(ObjType::Map), id))) => Some(id),
            _ => None,
        }
    }

    /// The object id of the top-level block at `index`.
    pub(crate) fn block_obj(&self, index: usize) -> Option<ObjId> {
        self.child_map(&self.content, index)
    }

    /// The `text` Text object of a text-block node.
    pub(crate) fn block_text_obj(&self, block: &ObjId) -> Option<ObjId> {
        match self.doc.get(block, TEXT) {
            Ok(Some((Value::Object(ObjType::Text), id))) => Some(id),
            _ => None,
        }
    }

    /// The `content` List object of a container node.
    pub(crate) fn node_content_obj(&self, node: &ObjId) -> Option<ObjId> {
        match self.doc.get(node, CONTENT) {
            Ok(Some((Value::Object(ObjType::List), id))) => Some(id),
            _ => None,
        }
    }

    /// Insert a new node (text-block or container) at `index` of `list`.
    pub(crate) fn insert_node(&mut self, list: &ObjId, index: usize, nd: &NodeData) -> Result<()> {
        let node = self.doc.insert_object(list, index, ObjType::Map)?;
        self.write_node(&node, nd)
    }

    /// Write a node's contents into its (already-inserted) map object. Recurses into a
    /// container's children.
    fn write_node(&mut self, node: &ObjId, nd: &NodeData) -> Result<()> {
        self.doc.put(node, TYPE, nd.type_name())?;
        let attrs_obj = self.doc.put_object(node, ATTRS, ObjType::Map)?;
        write_attrs(&mut self.doc, &attrs_obj, nd.attrs())?;
        match nd {
            NodeData::Block(b) => {
                let text = self.doc.put_object(node, TEXT, ObjType::Text)?;
                if !b.text.is_empty() {
                    self.doc.splice_text(&text, 0, 0, &b.text)?;
                }
                for m in &b.marks {
                    self.apply_mark(&text, m)?;
                }
            }
            NodeData::Container { children, .. } => {
                let content = self.doc.put_object(node, CONTENT, ObjType::List)?;
                for (i, child) in children.iter().enumerate() {
                    self.insert_node(&content, i, child)?;
                }
            }
        }
        Ok(())
    }

    /// Delete the top-level block at `index`.
    pub(crate) fn delete_block(&mut self, index: usize) -> Result<()> {
        self.doc.delete(&self.content, index)?;
        Ok(())
    }

    /// Reconcile the node at `index` of `list` to `target`: update type/attrs if they
    /// changed, then reconcile its body — a text-block's text + marks, or a container's
    /// child list — recursively. Unchanged descendants keep their CRDT identity, so a
    /// concurrent edit to a *different* text-block (even in a different list item) merges.
    pub(crate) fn reconcile_node(
        &mut self,
        list: &ObjId,
        index: usize,
        target: &NodeData,
    ) -> Result<()> {
        let node = self
            .child_map(list, index)
            .ok_or_else(|| CollabError::schema("reconcile_node: missing node"))?;

        // A node changing *kind* (text-block <-> container — e.g. a paragraph wrapped
        // into a list) can't be reconciled in place; replace it wholesale. Rare, so the
        // coarse-grained replace is fine.
        let crdt_is_block = self.block_text_obj(&node).is_some();
        let target_is_block = matches!(target, NodeData::Block(_));
        if crdt_is_block != target_is_block {
            self.doc.delete(list, index)?;
            self.insert_node(list, index, target)?;
            return Ok(());
        }

        // type — only write when it changed
        if self.scalar_str(&node, TYPE).as_deref() != Some(target.type_name()) {
            self.doc.put(&node, TYPE, target.type_name())?;
        }
        // attrs — only rebuild when they changed. Replacing the attrs object on every
        // keystroke would churn the CRDT and clobber a concurrent remote attr edit on
        // the same node.
        let current_attrs = match self.doc.get(&node, ATTRS) {
            Ok(Some((Value::Object(ObjType::Map), id))) => read_attrs(&self.doc, &id),
            _ => Attrs::new(),
        };
        if current_attrs != *target.attrs() {
            let attrs_obj = self.doc.put_object(&node, ATTRS, ObjType::Map)?;
            write_attrs(&mut self.doc, &attrs_obj, target.attrs())?;
        }

        match target {
            NodeData::Block(b) => {
                let text = self
                    .block_text_obj(&node)
                    .ok_or_else(|| CollabError::schema("reconcile_node: missing text"))?;
                self.reconcile_text(&text, b)?;
            }
            NodeData::Container { children, .. } => {
                let content = self
                    .node_content_obj(&node)
                    .ok_or_else(|| CollabError::schema("reconcile_node: missing content"))?;
                self.reconcile_child_list(&content, children)?;
            }
        }
        Ok(())
    }

    /// Reconcile a text-block's `text` Text object to `b`: a minimal common-prefix/suffix
    /// splice (so the per-char CRDT identity of unchanged text survives) plus a mark
    /// resync only when the marks actually changed.
    fn reconcile_text(&mut self, text: &ObjId, b: &BlockData) -> Result<()> {
        let old = self.doc.text(text)?;
        if old != b.text {
            splice_min(&mut self.doc, text, &old, &b.text)?;
        }
        // marks — only clear-and-reapply when they actually changed (after the splice,
        // Automerge has already shifted existing mark ranges with the text). Skipping
        // the resync on a pure text edit avoids clobbering a concurrent remote mark.
        let mut target_marks = b.marks.clone();
        target_marks.sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));
        if self.read_text_marks(text)? != target_marks {
            self.resync_marks(text, &b.marks)?;
        }
        Ok(())
    }

    /// Reconcile a container's `content` list to `target`. Diffs the CRDT's current
    /// children against `target` by *structural* equality (a common leading/trailing run
    /// is left untouched, keeping the CRDT identity — and merge behaviour — of every
    /// node in it), reconciles the overlapping middle in place, and inserts/deletes the
    /// count difference. This is the same block-list diff as [`Self::project_change`],
    /// one level down; recursion carries it to any depth.
    fn reconcile_child_list(&mut self, content: &ObjId, target: &[NodeData]) -> Result<()> {
        let cn = self.doc.length(content);
        let mut cur = Vec::with_capacity(cn);
        for i in 0..cn {
            cur.push(self.read_node_data(content, i)?);
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
            self.reconcile_node(content, prefix + k, &target[prefix + k])?;
        }
        // Insert the extra target children.
        for k in common..tgt_mid {
            self.insert_node(content, prefix + k, &target[prefix + k])?;
        }
        // Delete the extra current children (from the end so earlier indices stay valid).
        for idx in (prefix + common..prefix + cur_mid).rev() {
            self.doc.delete(content, idx)?;
        }
        Ok(())
    }

    /// Read a Text object's active marks back as canonical (sorted) [`SpanMark`]s.
    fn read_text_marks(&self, text: &ObjId) -> Result<Vec<SpanMark>> {
        let mut marks: Vec<SpanMark> = self
            .doc
            .marks(text)?
            .into_iter()
            .filter(|m| m.value() != &ScalarValue::Null)
            .map(|m| {
                Ok(SpanMark {
                    name: m.name().to_string(),
                    attrs: decode_mark_value(m.value())?,
                    start: m.start,
                    end: m.end,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        marks.sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));
        Ok(marks)
    }

    /// Clear and reapply a Text object's marks to match `target` exactly.
    pub(crate) fn resync_marks(&mut self, text: &ObjId, target: &[SpanMark]) -> Result<()> {
        // Clear every currently-active mark (set its name to Null over its range).
        // Collect owned data first so the read borrow ends before we mutate.
        let existing: Vec<(String, usize, usize)> = self
            .doc
            .marks(text)?
            .iter()
            .map(|m| (m.name().to_string(), m.start, m.end))
            .collect();
        for (name, start, end) in existing {
            self.doc.mark(
                text,
                AmMark::new(name, ScalarValue::Null, start, end),
                ExpandMark::None,
            )?;
        }
        for m in target {
            self.apply_mark(text, m)?;
        }
        Ok(())
    }

    /// Apply one mark span to a Text object, encoding its attrs into the mark value.
    fn apply_mark(&mut self, text: &ObjId, m: &SpanMark) -> Result<()> {
        let value = if m.attrs.is_empty() {
            ScalarValue::Boolean(true)
        } else {
            ScalarValue::Str(encode_attrs(&m.attrs).into())
        };
        self.doc.mark(
            text,
            AmMark::new(m.name.clone(), value, m.start, m.end),
            ExpandMark::None,
        )?;
        Ok(())
    }

    /// Read a scalar string attribute of an object.
    fn scalar_str(&self, obj: &ObjId, key: &str) -> Option<String> {
        match self.doc.get(obj, key) {
            Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
                ScalarValue::Str(smol) => Some(smol.to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Read the node at `index` of `list` back out of the CRDT as [`NodeData`]. A node
    /// carrying a `text` Text is a text-block; a node carrying a `content` List is a
    /// container, read recursively. Fails loud on a node that is neither (a corrupt
    /// projection), rather than materializing a broken shape.
    pub(crate) fn read_node_data(&self, list: &ObjId, index: usize) -> Result<NodeData> {
        let node = self
            .child_map(list, index)
            .ok_or_else(|| CollabError::schema("read_node_data: missing node"))?;
        let type_name = self
            .scalar_str(&node, TYPE)
            .ok_or_else(|| CollabError::schema("node has no type"))?;
        let attrs = match self.doc.get(&node, ATTRS) {
            Ok(Some((Value::Object(ObjType::Map), id))) => read_attrs(&self.doc, &id),
            _ => Attrs::new(),
        };
        if let Some(text_obj) = self.block_text_obj(&node) {
            let text = self.doc.text(&text_obj)?;
            let marks = self.read_text_marks(&text_obj)?;
            Ok(NodeData::Block(BlockData {
                type_name,
                attrs,
                text,
                marks,
            }))
        } else if let Some(content) = self.node_content_obj(&node) {
            let len = self.doc.length(&content);
            let mut children = Vec::with_capacity(len);
            for i in 0..len {
                children.push(self.read_node_data(&content, i)?);
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

    /// Rebuild the whole model document from the projection (the canonical, total
    /// read-back; both peers reconstruct identically, so equal CRDTs give equal docs).
    pub fn to_doc(&self, schema: &Schema) -> Result<Node> {
        let n = self.block_count();
        let mut blocks = Vec::with_capacity(n);
        for i in 0..n {
            let nd = self.read_node_data(&self.content, i)?;
            blocks.push(build_node(schema, &nd)?);
        }
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
/// are returned in canonical `(start, end, name)` order — matching
/// [`CollabDoc::read_text_marks`] — so a [`NodeData`] read from the model compares equal
/// to the same node read back from the CRDT.
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
    let name = m.type_name().to_string();
    if let Some(prev) = marks
        .iter_mut()
        .find(|s| s.end == start && s.name == name && s.attrs == m.attrs)
    {
        prev.end = end;
        return;
    }
    marks.push(SpanMark {
        name,
        attrs: m.attrs.clone(),
        start,
        end,
    });
}

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

/// Write a model attr set into an Automerge map (scalar values only).
fn write_attrs(doc: &mut AutoCommit, obj: &ObjId, attrs: &Attrs) -> Result<()> {
    for (k, v) in attrs.iter() {
        match v {
            AttrValue::Str(s) => doc.put(obj, k, s.as_ref())?,
            AttrValue::Int(i) => doc.put(obj, k, *i)?,
            AttrValue::Bool(b) => doc.put(obj, k, *b)?,
            AttrValue::Null => {}
        }
    }
    Ok(())
}

/// Read an Automerge map back into a model attr set.
fn read_attrs(doc: &AutoCommit, obj: &ObjId) -> Attrs {
    let mut out = Attrs::new();
    for key in doc.keys(obj) {
        if let Ok(Some((Value::Scalar(s), _))) = doc.get(obj, &key) {
            let v = match s.as_ref() {
                ScalarValue::Str(smol) => AttrValue::from(smol.to_string()),
                ScalarValue::Int(i) => AttrValue::Int(*i),
                // Clamp rather than wrap a foreign-impl Uint past i64::MAX.
                ScalarValue::Uint(u) => AttrValue::Int(i64::try_from(*u).unwrap_or(i64::MAX)),
                ScalarValue::Boolean(b) => AttrValue::Bool(*b),
                _ => continue,
            };
            out = out.with(key, v);
        }
    }
    out
}

/// Encode mark attrs as a compact deterministic JSON object string.
fn encode_attrs(attrs: &Attrs) -> String {
    let mut map = serde_json::Map::new();
    for (k, v) in attrs.iter() {
        let jv = match v {
            AttrValue::Str(s) => serde_json::Value::String(s.to_string()),
            AttrValue::Int(i) => serde_json::Value::from(*i),
            AttrValue::Bool(b) => serde_json::Value::Bool(*b),
            AttrValue::Null => serde_json::Value::Null,
        };
        map.insert(k.to_string(), jv);
    }
    serde_json::Value::Object(map).to_string()
}

/// Decode a mark's Automerge scalar value back into model attrs. The two valid
/// encodings ([`CollabDoc::apply_mark`]) are `Boolean(true)` for an attr-less mark
/// (→ empty attrs) and `Str(json-object)` for an attr-bearing mark. Fails loud on
/// anything else — a wrong scalar kind, invalid JSON, a non-object, or a non-integer
/// number — rather than silently dropping a peer's corrupted mark attrs (A22).
fn decode_mark_value(value: &ScalarValue) -> Result<Attrs> {
    let smol = match value {
        // The attr-less encoding — no attributes to decode.
        ScalarValue::Boolean(_) => return Ok(Attrs::new()),
        ScalarValue::Str(smol) => smol,
        other => {
            return Err(CollabError::schema(format!(
                "mark value must be Boolean(true) or a JSON-object string, got {other:?}"
            )));
        }
    };
    let serde_json::Value::Object(map) = serde_json::from_str(smol)
        .map_err(|e| CollabError::schema(format!("mark attr json invalid: {e}")))?
    else {
        return Err(CollabError::schema("mark attr json must be an object"));
    };
    let mut out = Attrs::new();
    for (k, v) in map {
        let av = match v {
            serde_json::Value::String(s) => AttrValue::from(s),
            serde_json::Value::Bool(b) => AttrValue::Bool(b),
            serde_json::Value::Number(n) if n.is_i64() => AttrValue::Int(n.as_i64().unwrap()),
            serde_json::Value::Number(n) => {
                return Err(CollabError::schema(format!(
                    "non-integer JSON number in mark attr `{k}`: {n}"
                )));
            }
            serde_json::Value::Null => AttrValue::Null,
            _ => continue,
        };
        out = out.with(k, av);
    }
    Ok(out)
}

/// Minimal common-prefix/suffix splice: replace only the changed middle so unchanged
/// characters keep their CRDT identity (and merge across peers).
pub(crate) fn splice_min(doc: &mut AutoCommit, text: &ObjId, old: &str, new: &str) -> Result<()> {
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
    doc.splice_text(text, prefix, del as isize, &ins)?;
    Ok(())
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
        let mut cdoc = CollabDoc::from_doc(&doc).unwrap();
        let block = cdoc.block_obj(0).unwrap();
        cdoc.doc.put(&block, TYPE, "blockquote").unwrap();
        assert!(matches!(
            cdoc.to_doc(&s).unwrap_err(),
            CollabError::Unsupported(_)
        ));
    }

    #[test]
    fn decode_mark_value_fails_loud_on_corruption() {
        // attr-less encoding decodes to empty attrs
        assert!(
            decode_mark_value(&ScalarValue::Boolean(true))
                .unwrap()
                .is_empty()
        );
        // valid attr json decodes
        let attrs = decode_mark_value(&ScalarValue::Str(r#"{"href":"x"}"#.into())).unwrap();
        assert_eq!(attrs.get_str("href"), Some("x"));
        // malformed values fail loud rather than silently dropping the attrs
        assert!(decode_mark_value(&ScalarValue::Str("not json".into())).is_err());
        assert!(decode_mark_value(&ScalarValue::Str(r#"["a"]"#.into())).is_err()); // not an object
        assert!(decode_mark_value(&ScalarValue::Str(r#"{"n":1.5}"#.into())).is_err()); // non-integer
        assert!(decode_mark_value(&ScalarValue::Int(5)).is_err()); // wrong scalar kind
    }
}
