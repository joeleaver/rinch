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
//! ```text
//! ROOT (Map)
//!   "content" -> List<Block>
//!     Block (Map):
//!       "type"  -> Str            // "paragraph" | "heading" | "code_block"
//!       "attrs" -> Map<Str,scalar>// e.g. heading {"level": 2}
//!       "text"  -> Text           // the block's plain text, codepoint-indexed
//!         (marks applied to "text" via doc.mark(): name = mark type;
//!          value = Bool(true) for an attr-less mark, or Str(json) carrying its attrs)
//! ```
//!
//! One `Text` per block with Automerge marks over it is the *rich-text* model — text
//! and formatting merge independently, which is exactly the "concurrent insert/format"
//! convergence the milestone requires. Automerge `Text` is **codepoint-indexed**, which
//! matches `rinch-editor-core`'s char-based positions exactly — no UTF conversion.
//!
//! ## Staged scope (design A22)
//!
//! The first milestone covers **flat text-blocks + marks**: a `doc` whose children are
//! all [textblocks](rinch_editor_core::Node::is_textblock) (`paragraph`/`heading`/
//! `code_block`) whose own children are all text nodes. Anything else — a nested block
//! (blockquote, list, table), an inline atom (`hard_break`, `image`) — is
//! [`CollabError::Unsupported`](crate::CollabError::Unsupported): **fail loud, never a
//! silent drop**.

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
        let mut cdoc = CollabDoc { doc: am, content };
        for i in 0..doc.child_count() {
            let block = read_block(doc.child(i))?;
            cdoc.insert_block(i, &block)?;
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

    /// The object id of the block at `index`.
    pub(crate) fn block_obj(&self, index: usize) -> Option<ObjId> {
        match self.doc.get(&self.content, index) {
            Ok(Some((Value::Object(ObjType::Map), id))) => Some(id),
            _ => None,
        }
    }

    /// The `text` Text object of a block.
    pub(crate) fn block_text_obj(&self, block: &ObjId) -> Option<ObjId> {
        match self.doc.get(block, TEXT) {
            Ok(Some((Value::Object(ObjType::Text), id))) => Some(id),
            _ => None,
        }
    }

    /// Insert a new block (type + attrs + text + marks) at `index`.
    pub(crate) fn insert_block(&mut self, index: usize, b: &BlockData) -> Result<()> {
        let block = self.doc.insert_object(&self.content, index, ObjType::Map)?;
        self.doc.put(&block, TYPE, b.type_name.as_str())?;
        let attrs_obj = self.doc.put_object(&block, ATTRS, ObjType::Map)?;
        write_attrs(&mut self.doc, &attrs_obj, &b.attrs)?;
        let text = self.doc.put_object(&block, TEXT, ObjType::Text)?;
        if !b.text.is_empty() {
            self.doc.splice_text(&text, 0, 0, &b.text)?;
        }
        for m in &b.marks {
            self.apply_mark(&text, m)?;
        }
        Ok(())
    }

    /// Delete the block at `index`.
    pub(crate) fn delete_block(&mut self, index: usize) -> Result<()> {
        self.doc.delete(&self.content, index)?;
        Ok(())
    }

    /// Reconcile the block at `index` to `target`: update type/attrs if they changed,
    /// splice the text to match (minimal common-prefix/suffix splice so the per-char
    /// CRDT identity of unchanged text survives), and resync its marks.
    pub(crate) fn reconcile_block(&mut self, index: usize, target: &BlockData) -> Result<()> {
        let block = self
            .block_obj(index)
            .ok_or_else(|| CollabError::schema("reconcile_block: missing block"))?;

        // type — only write when it changed
        if self.scalar_str(&block, TYPE).as_deref() != Some(target.type_name.as_str()) {
            self.doc.put(&block, TYPE, target.type_name.as_str())?;
        }
        // attrs — only rebuild when they changed. Replacing the attrs object on every
        // keystroke would churn the CRDT and clobber a concurrent remote attr edit on
        // the same block.
        let current_attrs = match self.doc.get(&block, ATTRS) {
            Ok(Some((Value::Object(ObjType::Map), id))) => read_attrs(&self.doc, &id),
            _ => Attrs::new(),
        };
        if current_attrs != target.attrs {
            let attrs_obj = self.doc.put_object(&block, ATTRS, ObjType::Map)?;
            write_attrs(&mut self.doc, &attrs_obj, &target.attrs)?;
        }

        // text — minimal splice
        let text = self
            .block_text_obj(&block)
            .ok_or_else(|| CollabError::schema("reconcile_block: missing text"))?;
        let old = self.doc.text(&text)?;
        if old != target.text {
            splice_min(&mut self.doc, &text, &old, &target.text)?;
        }

        // marks — only clear-and-reapply when they actually changed (after the splice,
        // Automerge has already shifted existing mark ranges with the text). Skipping
        // the resync on a pure text edit avoids clobbering a concurrent remote mark.
        let mut target_marks = target.marks.clone();
        target_marks.sort_by(|a, b| (a.start, a.end, &a.name).cmp(&(b.start, b.end, &b.name)));
        if self.read_text_marks(&text)? != target_marks {
            self.resync_marks(&text, &target.marks)?;
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

    /// Read the block at `index` back out of the CRDT as [`BlockData`].
    pub(crate) fn read_block_data(&self, index: usize) -> Result<BlockData> {
        let block = self
            .block_obj(index)
            .ok_or_else(|| CollabError::schema("read_block_data: missing block"))?;
        let type_name = self
            .scalar_str(&block, TYPE)
            .ok_or_else(|| CollabError::schema("block has no type"))?;
        let attrs = match self.doc.get(&block, ATTRS) {
            Ok(Some((Value::Object(ObjType::Map), id))) => read_attrs(&self.doc, &id),
            _ => Attrs::new(),
        };
        let text_obj = self
            .block_text_obj(&block)
            .ok_or_else(|| CollabError::schema("block has no text"))?;
        let text = self.doc.text(&text_obj)?;
        let marks = self.read_text_marks(&text_obj)?;
        Ok(BlockData {
            type_name,
            attrs,
            text,
            marks,
        })
    }

    /// Rebuild the whole model document from the projection (the canonical, total
    /// read-back; both peers reconstruct identically, so equal CRDTs give equal docs).
    pub fn to_doc(&self, schema: &Schema) -> Result<Node> {
        let n = self.block_count();
        let mut blocks = Vec::with_capacity(n);
        for i in 0..n {
            let b = self.read_block_data(i)?;
            blocks.push(build_block(schema, &b)?);
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

/// Validate a model block is a flat textblock and extract its projectable data.
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
