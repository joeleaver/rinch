# Rinch Editor — Definitive Rip-and-Replace Design

**Status:** Approved direction; hardened by adversarial validation. **M0–M4 (model/positions/transform/serialize/state) are implementation-ready as written; M5–M9 + the public handle carry amendments — see Appendix A.**
**Scope:** Full replacement of the contenteditable / rich-text editing subsystem
**Date:** 2026-06-20
**Audit basis:** `docs/audits/contenteditable-audit-2026-06-20.md`
**Validation:** 4 adversarial critics (coherence / feasibility-vs-real-APIs / rip-completeness / scope-closure) + amendment synthesis. Verdict: *ready-with-amendments*. All 22 amendments recorded in **Appendix A**; the body below is the original design — read it together with the appendix.

---

## 1. Summary & goals

We are replacing the entire rinch editing stack with a **ProseMirror-faithful architecture in idiomatic Rust**, split into a pure renderer-agnostic core plus pluggable platform views. The current system has three overlapping editing engines, two competing undo interpreters, two incompatible position spaces, Automerge welded in as the authoritative model, a documented-but-dead `Editor`/`StarterKit` framework, no IME, no accessibility, inert tables, and a lossy serializer that drops marks (`#59`). The audit's root cause is precise: **multiple mutation paths write to different sources of truth with no reconciliation.** This design eliminates that class structurally, not by patching.

### Non-negotiable principles

1. **Single source of truth.** `EditorState` is a value: `{ doc, selection, stored_marks, plugin_state }`. There is exactly one document, one selection, one position space. No DOM node, no Automerge document, no second cursor model is ever authoritative.
2. **All changes are Steps.** Every edit is a `Transaction` of invertible, mappable `Step`s. `state.apply(tx) -> EditorState`. Undo/redo, collab rebase, and decoration tracking all ride on `Step::invert` + `StepMap`.
3. **View is a pure function of state.** `EditorView::update(prev, next)` diffs `prev.doc`/`next.doc` and patches the host tree. The host tree is **never** read back as content. Caret and selection are rendered **from `state.selection`**. There is no DOM-direct mutation path anywhere.
4. **Input produces Transactions, not DOM edits.** Keys, IME composition, paste, pointer selection, and drag are translated by the view into commands/transactions.
5. **Schema is authoritative and enforced.** Transactions are validated against the schema; invalid steps are rejected, never silently written. Serialization is **total and attr-aware**, derived from the schema. No mark or node type can be dropped or rendered to garbage. This structurally closes `#59` and its whole class.
6. **Renderer-agnostic core.** Model, transform, state, selection, commands, plugins, schema, history are **pure Rust** with zero dependency on rinch-dom, winit, web_sys, parley, taffy, vello, **or automerge**. Rendering + input live behind a `View` seam.

### How this kills the root cause

The audit's failure mode is "convention-based dual writes" — `CeOps` writing the DOM *and* an Automerge `EditorDocument`, `paste_html_into_ce` writing the DOM but never the model, `move_dom_cursor` maintaining a second cursor in `ContentEditableFocus`. In the new design there is **one** function that can change content — `EditorState::apply` — and **one** function that can change the host — `EditorView::update`, which is a pure projection of state. A bug can no longer be "the DOM and the model disagree" because the DOM is *derived* from the model on every transaction and is never consulted for truth. Collaboration becomes "remote Steps are just more Transactions"; IME preedit becomes "a decoration that is not in the document"; paste becomes "parse to a Slice, then `replaceSelection`." Every previously-divergent path collapses into the single state→view pipeline.

---

## 2. Crate & module layout

### New crate graph

```
rinch-editor-core   (NEW, pure Rust, wasm-clean)   ← model/transform/state/commands/plugins/schema/history
rinch-editor-collab (NEW, optional, NON-wasm)      ← the ONLY home of automerge; Step↔CRDT adapter
rinch  (desktop View)                              ← the ONLY rinch-dom touchpoint; winit IME + AccessKit
rinch-web (web View)                               ← web_sys EditorView delegating to native contentEditable (closes #51)
rinch-clipboard                                    ← UNCHANGED, reused as-is
rinch-editable                                     ← KEPT, demoted to single-line <input>/<textarea> ONLY
```

**Deleted crates:** `rinch-editor`, `rinch-editor-macros`, `rinch-editor-components` (all three have zero non-test source consumers).

### `rinch-editor-core` module layout

```
rinch-editor-core/
  src/
    lib.rs                  // prelude, re-exports
    error.rs                // EditorError (NO Automerge variant)
    model/
      mod.rs                // Node, Mark, Fragment, Slice, Text
      node.rs               // Node value type (Rc-shared, persistent)
      mark.rs               // Mark value type
      fragment.rs           // Fragment (ordered child list)
      slice.rs              // Slice { content: Fragment, open_start, open_end }
      attrs.rs              // AttrValue (typed), Attrs
    schema/
      mod.rs                // Schema (lifted verbatim from rinch-editor/src/schema/mod.rs)
      node_spec.rs          // NodeSpec, MarkSet, AttrSpec  (lifted)
      mark_spec.rs          // MarkSpec                     (lifted)
      content_match.rs      // ContentMatch NFA (REWRITE — replaces naive matches_content)
      validation.rs         // validate_content/can_mark/can_insert (lifted, wired into Step::apply)
      starter_kit.rs        // node/mark catalogue (names preserved)
    pos/
      mod.rs                // Pos(usize) — ProseMirror depth-aware integer position
      resolved.rs           // ResolvedPos { pos, path, parent, depth, ... }
    selection.rs            // Selection enum (TextSelection, NodeSelection, CellSelection)
    transform/
      mod.rs                // Transform builder, replace helpers
      step.rs               // trait Step, StepResult
      step_map.rs           // StepMap, Mapping
      steps/
        replace.rs          // ReplaceStep, ReplaceAroundStep
        mark.rs             // AddMarkStep, RemoveMarkStep
        attr.rs             // SetNodeAttrStep, SetDocAttrStep
        // (set-block-type, split, join, indent are EXPRESSED as Replace/ReplaceAround)
    state/
      mod.rs                // EditorState, Transaction, apply
      transaction.rs        // Transaction (Transform + selection + stored_marks + meta)
    command.rs              // Command = fn(&EditorState, Option<&mut dyn FnMut(Transaction)>) -> bool
    commands/               // built-in commands (split_block, set_block_type, toggle_mark, lift/sink, ...)
    keymap.rs               // platform-agnostic KeyBinding + normalize (winit-agnostic vocabulary)
    input_rules.rs          // InputRule { pattern: Regex, handler -> Option<Transaction> }
    plugin.rs               // trait Plugin, PluginKey, PluginState, view-hooks (decorations/node-views)
    plugins/
      history.rs            // History plugin (Step-based, grouped, typing-merge)
      // tables.rs, links.rs contributed here or by feature
    serialize/
      mod.rs                // schema-derived TOTAL serializer
      html.rs               // model ↔ HTML (paste/copy), whitelist-driven
      markdown.rs           // model ↔ markdown (pulldown-cmark front-end)
      doc_json.rs           // the durable serde wire shape (DocNode)
    decoration.rs           // Decoration, DecorationSet (preedit, collab cursors, search highlight)
    view.rs                 // trait EditorView, ViewEvent, ime/a11y abstract requests
```

### Feature flags

`rinch-editor-core/Cargo.toml`:
```toml
[features]
default = []
serde = ["dep:serde"]   # schema-derived total serialization (DocNode wire shape)
markdown = ["dep:pulldown-cmark"]   # benign, renderer-agnostic; wasm-clean
[dependencies]
regex = "1"             # input rules; wasm-clean
serde = { version = "1", optional = true, features = ["derive"] }
pulldown-cmark = { version = "0.10", optional = true }
# NO automerge, NO rinch-dom, NO winit, NO web_sys, NO parley/taffy/vello.
```

`rinch-editor-collab/Cargo.toml`:
```toml
[dependencies]
rinch-editor-core = { workspace = true }
automerge = "0.5"       # the ONLY first-party automerge dependency in the workspace
```

`rinch/Cargo.toml` changes:
- **Drop** `rinch-editor = { workspace = true }` (line 48, non-optional — currently drags Automerge + the dead framework + regex + pulldown-cmark into *every* build including wasm).
- **Add** `rinch-editor-core = { workspace = true }` (non-optional).
- **Add** `rinch-editor-collab = { workspace = true, optional = true }`, gated `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
- `collaboration = ["dep:rinch-editor-collab"]` (was `["rinch-editor/collaboration"]`, line 118).
- Keep `rinch-editable` (lines 17/85/93/98) — still required for `<input>`/`<textarea>`.
- `serde = ["rinch-editor-core/serde"]` (was `["rinch-core/serde"]` for the deleted `BlockData`).

**Dependency-move outcome:** non-editor desktop apps and **all wasm builds** stop linking Automerge, regex, and pulldown-cmark transitively. This was a hard requirement (the audit notes Automerge is currently a fixed cost on every build).

**Workspace `Cargo.toml`:** remove `rinch-editor`, `rinch-editor-macros`, `rinch-editor-components` from `[workspace.dependencies]` (lines 31/32/33); add `rinch-editor-core` and `rinch-editor-collab`. The `crates/*` glob auto-discovers the new dirs; delete the three old crate dirs. Web members (`ui-zoo-web`, etc., already excluded lines 7–13) must **not** reach `rinch-editor-collab` — guaranteed because collab is behind both an optional feature and a non-wasm target-cfg.

---

## 3. The document model

### Node / Mark / Fragment / Slice

The model is a **persistent (structurally-shared) immutable tree**. Cloning a `Node` is cheap (`Rc` bump). Every edit produces a new tree sharing unchanged subtrees with the old — this makes `EditorState` cheap to keep in history and to diff in the view.

```rust
// model/node.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node(Rc<NodeInner>);

#[derive(Debug, PartialEq, Eq)]
struct NodeInner {
    type_name: NodeType,        // interned schema node-type handle (cheap eq)
    attrs: Attrs,               // typed, schema-defaulted
    content: Fragment,          // ordered children (empty for leaves)
    marks: MarkSet,             // marks on THIS node (only meaningful for inline/text leaves)
    text: Option<Box<str>>,     // Some only for the schema "text" node
}

impl Node {
    pub fn node_type(&self) -> NodeType { ... }
    pub fn is_text(&self) -> bool { self.0.text.is_some() }
    pub fn is_inline(&self) -> bool { self.node_type().spec().inline }
    pub fn is_leaf(&self) -> bool { self.0.content.is_empty() && !self.is_text() }
    pub fn is_atom(&self) -> bool { self.node_type().spec().atom }  // hr, image
    pub fn child_count(&self) -> usize { self.0.content.len() }
    pub fn child(&self, i: usize) -> &Node { self.0.content.child(i) }
    /// Size in the position space (see below).
    pub fn node_size(&self) -> usize { ... }
    pub fn content_size(&self) -> usize { self.0.content.size() }
}
```

```rust
// model/mark.rs — descended from the PLAIN MarkData (model/mod.rs:20-44), zero Automerge.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Mark {
    pub mark_type: MarkType,   // interned schema mark-type handle
    pub attrs: Attrs,          // e.g. link.href, text_color.color
}
```

```rust
// model/attrs.rs — typed, fixing the audit's "Stringly-typed HashMap" attr hazard.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AttrValue { Null, Bool(bool), Int(i64), Str(Box<str>) }

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(Option<Rc<BTreeMap<Box<str>, AttrValue>>>);  // None == empty, ordered for stable serde
```

`AttrSpec` upgrades `default` to `AttrValue` and keeps `required`. Schema-validation applies defaults and enforces required attrs (link.href, image.src, heading.level) at the Step boundary — so attr-aware round-trip is structurally guaranteed, not best-effort.

`Fragment` is an ordered, sized, `Rc`-shared child list with the cut/append/replace primitives ProseMirror needs. `Slice` carries open depths for paste/replace:

```rust
// model/slice.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice { pub content: Fragment, pub open_start: usize, pub open_end: usize }
```

`Slice` replaces today's `Fragment`/`FragmentBlock`/`FragmentInline` clipboard tree (`document/fragment.rs`). Its markdown I/O (pulldown-cmark `from_markdown`/`to_markdown`) is **salvaged** into `serialize/markdown.rs`; its hand-rolled `from_html` becomes the *starting structure* for `serialize/html.rs`, rewritten schema/whitelist-driven.

### The position model — exactly ONE depth-aware integer space

We adopt ProseMirror's single integer space verbatim. There is **one** `Pos(usize)`; the old flat `ResolvedPosition{block_index, inline_index, text_offset}` and `rinch_editable::Position` byte-space are both **deleted**.

- Each node contributes **1** to the position count for its opening boundary and **1** for its closing boundary (the "token" at depth changes).
- Text contributes **one position per Unicode scalar value (char)**, not per byte. **We choose chars deliberately** (newtype'd) and never bytes inside text nodes — this designs out the audit's S3 byte/char mix-up class. (Byte offsets exist only transiently at platform seams; see §7.)
- The doc root's start is `0`; `doc.content_size()` is the last valid position.

```rust
// pos/mod.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos(pub usize);

// pos/resolved.rs
pub struct ResolvedPos {
    pub pos: Pos,
    pub path: Vec<(Node, usize, Pos)>, // (parent, index-in-parent, parent-start-pos) per depth
    pub depth: usize,
    pub parent_offset: usize,          // CHAR offset within the parent text-block content
}
impl Node {
    pub fn resolve(&self, pos: Pos) -> Result<ResolvedPos, EditorError>;
}
```

`ResolvedPos` answers `parent()`, `node(depth)`, `before(depth)`, `after(depth)`, `index(depth)`, `marks()`, replacing all of `queries.rs`'s flat math.

### Schema — lifted, with a real ContentMatch

`NodeSpec`/`MarkSpec`/`MarkSet`/`AttrSpec`, `excludes`, `parse_html_tags`, and `starter_kit()`'s node/mark catalogue move **verbatim** from `crates/rinch-editor/src/schema/{mod,node,mark,validation}.rs` into `rinch-editor-core/src/schema/`. The dead `Node`/`Mark` **dyn traits** (`node.rs:6-22`, `mark.rs:7-15`) are dropped — we have a concrete `Node` value type.

**One required upgrade:** the existing `matches_content` (`schema/mod.rs:413-526`) is a naive string-split matcher (single-part + "paragraph block*"). Real nesting (`blockquote > block+`, `list_item > block+`, ordered/bullet lists, tables) needs a proper **ContentMatch NFA** compiled from the content-expression. We implement `content_match.rs` (compile a regex-like content expr to a DFA over node types; `matchType`, `matchFragment`, `fillBefore`). This is the one piece the salvaged schema crate genuinely lacks and it is load-bearing for Step validation.

Starter-kit catalogue (names preserved for wire stability): `doc, paragraph, heading{level}, blockquote, code_block, bullet_list, ordered_list, list_item, task_list, task_item, horizontal_rule, hard_break, text, image{src,alt}`; marks `bold, italic, underline, strike, code, link{href}, highlight{color?}, text_color{color}, subscript, superscript`. `hard_break`, `horizontal_rule`, `image` are **first-class nodes** (atoms / inline leaves) — so the audit's #59-class loss (hard_break-as-string, no hr/image) is structurally impossible.

### Total, attr-aware serialization (the durable wire shape)

The save/load contract is a **recursive, schema-derived** shape, replacing the flat `BlockData/InlineRunData/InlineMarkData` (which cannot express lists/tables/atoms). We keep the snake_case key conventions (`type`, `attrs`, `content`, `marks`) for familiarity, but the shape is now recursive and total.

```rust
// serialize/doc_json.rs  (feature = "serde")
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DocNode {
    #[serde(rename = "type")]
    pub node_type: String,                       // schema node name; ALWAYS present
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub attrs: BTreeMap<String, JsonAttr>,       // typed, defaults applied
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<DocNode>,                   // recursive — nesting works
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,                    // text nodes only
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<DocMark>,                      // on text/inline leaves
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DocMark {
    #[serde(rename = "type")] pub mark_type: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")] pub attrs: BTreeMap<String, JsonAttr>,
}
```

**Totality is enforced two ways:**
1. **Serialize:** `Node → DocNode` walks the schema. Every node type and mark type has a name from `NodeType::name()`/`MarkType::name()` — there is no `_ => content` fallthrough because there is no string-tag bridge; the node *is* its schema type. Attr defaults are applied; required attrs are present by construction (validated at Step time).
2. **Deserialize:** `DocNode → Node` consults `Schema::node()`/`Schema::mark()`. Unknown type ⇒ **`Err(SchemaValidation)`**, never a silent drop. Required-attr-missing ⇒ `Err`. This is the structural fix for #59: the boundary either round-trips a thing or rejects it.

HTML serialization (`serialize/html.rs`) derives tags from `NodeSpec.parse_html_tags`/`MarkSpec.parse_html_tags` (the schema already carries these), unifying the three divergent hardcoded tag maps (`wrap_mark`, `block_type_to_tag`, `mark_type_to_tag`) into one schema-driven table used for both copy-out and paste-in. The `from_block_data` roundtrip tests (`serialization.rs`, `roundtrip_tests.rs`) become acceptance fixtures for `DocNode` round-trip.

---

## 4. The transform engine

This is the core. Every editing operation is a `Transaction` carrying one or more `Step`s. Steps are **invertible** (for undo) and **mappable** (for redo, collab rebase, decoration tracking).

### Step trait

```rust
// transform/step.rs
pub trait Step: std::fmt::Debug {
    /// Apply to a doc; returns the new doc or a failure (schema/position invalid).
    fn apply(&self, doc: &Node) -> Result<Node, StepError>;
    /// The position map this step induces (for mapping later steps/selections).
    fn get_map(&self) -> StepMap;
    /// Invert against the doc this step was applied to (for undo).
    fn invert(&self, doc: &Node) -> Box<dyn Step>;
    /// Rebase this step over a mapping (for collab); None if it no longer applies.
    fn map(&self, mapping: &Mapping) -> Option<Box<dyn Step>>;
    /// Try to merge with a following step (typing coalescing).
    fn merge(&self, other: &dyn Step) -> Option<Box<dyn Step>>;
}
```

`apply` is where **schema enforcement (decision G)** lives: a `ReplaceStep` whose slice would violate the parent's `ContentMatch` returns `Err` and the whole transaction is rejected. No invalid content is ever written.

### The Step set (deliberately minimal — ProseMirror's)

We do **not** add a Step per editing gesture. A small primitive set expresses everything; gestures map onto it. This is the key insight that collapses the old engine's dozen bespoke mutation methods.

| Step | Purpose |
|---|---|
| `ReplaceStep { from, to, slice }` | Replace a range with a Slice. **This is the workhorse** — text insert, delete, split, join, paste all reduce to this. |
| `ReplaceAroundStep { from, to, gap_from, gap_to, slice, insert }` | Replace around a preserved gap — used for wrapping (blockquote/list), lifting, set-block-type that re-parents. |
| `AddMarkStep { from, to, mark }` | Add a mark across an inline range. |
| `RemoveMarkStep { from, to, mark }` | Remove a mark across an inline range. |
| `SetNodeAttrStep { pos, attr, value }` | Change one node attr (heading level, image alt, list start). |
| `SetDocAttrStep { attr, value }` | Doc-level attr (rare; e.g. doc-wide settings). |

Tables add **no new Step kinds** — table edits are `ReplaceStep`/`ReplaceAroundStep`/`SetNodeAttrStep` over table/row/cell nodes (see §8). Collab adds none either.

### Transaction & Transform

```rust
// state/transaction.rs
pub struct Transaction {
    pub before: Node,                 // doc before (for invert/grouping)
    steps: Vec<Box<dyn Step>>,
    docs: Vec<Node>,                  // doc snapshot before each step (for invert)
    mapping: Mapping,                 // accumulated StepMaps
    pub doc: Node,                    // current doc after applied steps
    selection: Selection,            // mapped forward as steps are added
    stored_marks: Option<Vec<Mark>>,
    meta: HashMap<TypeId, Box<dyn Any>>,  // plugin metadata (e.g. "addToHistory=false", collab origin)
    time: u64,
}
impl Transaction {
    pub fn step(&mut self, step: Box<dyn Step>) -> Result<(), StepError>;  // applies + maps selection
    pub fn replace(&mut self, from: Pos, to: Pos, slice: Slice) -> &mut Self;
    pub fn insert_text(&mut self, pos: Pos, text: &str) -> &mut Self;
    pub fn delete(&mut self, from: Pos, to: Pos) -> &mut Self;
    pub fn split(&mut self, pos: Pos, depth: usize, types: Option<Vec<(NodeType, Attrs)>>) -> &mut Self;
    pub fn set_block_type(&mut self, from: Pos, to: Pos, ty: NodeType, attrs: Attrs) -> &mut Self;
    pub fn add_mark(&mut self, from: Pos, to: Pos, mark: Mark) -> &mut Self;
    pub fn remove_mark(&mut self, from: Pos, to: Pos, mark: Mark) -> &mut Self;
    pub fn wrap(&mut self, range: NodeRange, wrappers: &[(NodeType, Attrs)]) -> &mut Self;  // ReplaceAround
    pub fn lift(&mut self, range: NodeRange, target: usize) -> &mut Self;
    pub fn set_node_attr(&mut self, pos: Pos, attr: &str, value: AttrValue) -> &mut Self;
    pub fn set_selection(&mut self, sel: Selection) -> &mut Self;
    pub fn set_stored_marks(&mut self, marks: Option<Vec<Mark>>) -> &mut Self;
    pub fn set_meta<K: 'static>(&mut self, key: K, val: impl Any) -> &mut Self;
}
```

### Editing operation → Steps (the concrete table)

| Editing gesture | Step(s) produced |
|---|---|
| Type a character | `ReplaceStep(sel.from, sel.to, Slice::text(ch, stored_marks))` |
| Backspace (collapsed, mid-text) | `ReplaceStep(pos-1, pos, empty)` |
| Backspace at block start (join) | `ReplaceStep(blockStart-1, blockStart+1?, empty)` — joins via boundary removal, validated by ContentMatch |
| Delete (forward) | `ReplaceStep(pos, pos+1, empty)` |
| Delete selection | `ReplaceStep(sel.from, sel.to, empty)` |
| Enter / split block | `ReplaceStep` splitting the textblock: slice with `open_start=open_end=1` and the new block type |
| Shift+Enter / hard break | `ReplaceStep(pos, pos, Slice::node(hard_break))` |
| Toggle bold/italic/etc. (range) | `AddMarkStep` or `RemoveMarkStep` over `sel.from..sel.to` (toggle decided by `has_mark` query) |
| Toggle mark (collapsed cursor) | **No step** — set `stored_marks` (see §5) |
| Set heading level | `SetNodeAttrStep(blockPos, "level", N)` (or `ReplaceAroundStep` if changing node type) |
| Paragraph ↔ heading/code_block | `set_block_type` → `ReplaceAroundStep` re-parenting the content |
| Wrap in blockquote | `wrap(range, [(blockquote, {})])` → `ReplaceAroundStep` |
| Toggle bullet/ordered list | `wrap` into `list_item`+`list` (or `lift` if already in a list) |
| Indent list item (sink) | `ReplaceAroundStep` nesting the `list_item` deeper, ContentMatch-validated |
| Outdent list item (lift) | `lift(range, targetDepth)` → `ReplaceAroundStep` |
| Insert hr | `ReplaceStep(pos, pos, Slice::node(horizontal_rule))` |
| Insert link | `AddMarkStep(from, to, Mark{link, {href}})` |
| Insert image | `ReplaceStep(pos, pos, Slice::node(image, {src, alt}))` |
| Paste (HTML/text) | parse → `Slice` → `ReplaceStep(sel.from, sel.to, slice)` with open depths |
| Insert table | `ReplaceStep(pos, pos, Slice::node(table{rows×cols of empty cells}))` |
| Add/remove row/column | `ReplaceStep`/`ReplaceAroundStep` over the table node |
| Remote edit (collab) | adapter emits Steps, applied as a Transaction with `meta(origin=remote)` |

**Invert** is mechanical: `ReplaceStep.invert(doc)` produces `ReplaceStep(from, from+slice.size, doc.slice(from, to))`. **Map** rebases positions through `Mapping`. **Merge** coalesces consecutive single-char `ReplaceStep`s (typing) so history groups naturally.

---

## 5. State, selection, stored marks, plugins, commands, keymap, input rules, history — an edit end-to-end

### EditorState

```rust
// state/mod.rs
pub struct EditorState {
    pub doc: Node,
    pub selection: Selection,
    pub stored_marks: Option<Vec<Mark>>,
    pub schema: Schema,
    plugins: Vec<Rc<dyn Plugin>>,
    plugin_state: HashMap<PluginKey, Box<dyn Any>>,  // history stack, collab unconfirmed steps, decoration sets
}
impl EditorState {
    pub fn tr(&self) -> Transaction;  // fresh transaction from current state
    pub fn apply(&self, tr: Transaction) -> EditorState;  // pure: returns NEW state
}
```

`apply` runs the transaction's steps (already applied into `tr.doc`), then calls each plugin's `apply(tr, old_state, new_state)` to fold plugin state forward (history pushes the inverted steps; collab records unconfirmed steps; decoration sets map through `tr.mapping`). **`apply` is total and side-effect-free** — no DOM, no window, no thread-locals.

### Selection

```rust
// selection.rs
pub enum Selection {
    Text(TextSelection),       // { anchor: Pos, head: Pos }  (anchor fixed, head moving — shape from CeSelection)
    Node(NodeSelection),       // { pos: Pos } — whole atom/node selected (image, hr)
    Cell(CellSelection),       // { anchor_cell: Pos, head_cell: Pos } — table rectangle (replaces stringly DOM hints)
}
```

Selection is part of state and is mapped forward by every transaction. The caret is **rendered from `Selection`** by the view — there is no `DomCursor`, no second cursor model, no `data-ce-cursor` attribute.

### Stored marks (fixes the "click Bold then type" bug, audit S2)

We carry ProseMirror's tri-state, salvaged in concept from `editor.rs:109-147`:

- `stored_marks: None` → inherit marks from the position context on next text insert.
- `stored_marks: Some(vec![])` → explicitly no marks.
- `stored_marks: Some([bold])` → next inserted text gets bold.

Toggling a mark **with a collapsed cursor** sets `stored_marks`; toggling with a range emits an `AddMark`/`RemoveMark` Step. `insert_text` consumes `stored_marks` (or inherited marks) and clears it after a non-typing transaction. This is the structural fix for the dead engine's cursor-formatting bug.

### Plugins

```rust
// plugin.rs
pub trait Plugin {
    fn key(&self) -> PluginKey;
    fn schema_contributions(&self) -> SchemaContrib { SchemaContrib::default() } // nodes/marks
    fn commands(&self) -> Vec<(&'static str, Command)> { vec![] }
    fn keymap(&self) -> Vec<(KeyBinding, &'static str)> { vec![] }   // key → command name
    fn input_rules(&self) -> Vec<InputRule> { vec![] }
    fn init_state(&self, doc: &Node) -> Option<Box<dyn Any>> { None }
    fn apply(&self, tr: &Transaction, old: &EditorState, new_doc: &Node, prev: Option<&dyn Any>) -> Option<Box<dyn Any>> { None }
    /// View-facing: decorations to render (preedit, collab cursors, search), and node-views.
    fn decorations(&self, state: &EditorState) -> DecorationSet { DecorationSet::empty() }
    fn node_views(&self) -> Vec<(NodeType, NodeViewCtor)> { vec![] }
}
```

History, tables, links, collaboration, accessibility, and IME-preedit-decoration are **all plugins** — none is special-cased in core. This is the `Extension`-trait shape (salvaged in concept from `extensions/mod.rs:56`) but rebound from `&mut Editor` to `(state, dispatch)`.

### Commands & keymap

```rust
// command.rs
pub type Dispatch<'a> = &'a mut dyn FnMut(Transaction);
pub type Command = Rc<dyn Fn(&EditorState, Option<Dispatch>) -> bool>;
```

A command **queries state and, if it applies, builds a transaction and dispatches it**, returning `true`. Called with `dispatch = None`, it only reports applicability (drives toolbar enabled/disabled state). All toolbar buttons, keymap entries, and input rules resolve to commands by name through a `CommandRegistry` aggregated from plugins (salvaged concept: `CommandRegistration`, `registry.rs:63-95`).

**Toolbar queries read STATE, never the DOM** (replacing `with_active_ce_api(...).has_active_mark`): `is_mark_active(&state, bold) -> bool`, `current_block_type(&state) -> NodeType`, `can_indent(&state) -> bool`. The `markdown-editor` example's `ce_do(|api| api.toggle_wrap("strong"))` becomes `editor.command("toggleBold")`.

The keymap normalizes `winit` logical keys + modifiers into `KeyBinding` and looks up command names. We salvage the `normalize_key` *logic* (`shortcuts.rs:18-71`, Mod→platform-primary, case-folding) but consume `winit::KeyEvent` directly rather than pre-normalized strings.

### Input rules

```rust
// input_rules.rs
pub struct InputRule {
    pub pattern: Regex,
    pub handler: Rc<dyn Fn(&EditorState, &Captures, Pos /*from*/, Pos /*to*/) -> Option<Transaction>>,
}
```

Markdown-shortcut rules (`## ` → heading, `- ` → bullet list, `> ` → blockquote, `` `code` `` → code mark) ship as plugin data. Salvaged shape from `rules.rs`, rebound to return `Option<Transaction>`.

### History plugin (single, Step-based, grouped, typing-merged)

One history, a plugin, storing **inverted Steps** (not byte-positions). It groups by transaction boundary and **merges consecutive typing transactions** within a time/affinity window (salvaged semantics from `UndoOperation::can_merge`/`merge` + `CompoundOperation` + `LocalUndoStack`, but over Steps/StepMap):

```rust
struct HistoryState { done: Branch, undone: Branch }   // Branch = Vec<HistoryGroup>; group = Vec<inverted Step> + Mapping + selection-before
```

On undo: pop a group, **rebase its inverted steps over any intervening (collab) mappings**, apply as a transaction with `meta(addToHistory=false)`, restore the recorded selection. The two dead undo interpreters (`Editor::apply_inverse`, `CommandDispatcher`, and `CeOps`'s `UndoOp`/`UndoGroup`) are **all deleted** — there is exactly one.

### End-to-end edit flow

```
winit/web event
  → View translates to a command call OR a direct Transaction
  → command(state, dispatch): queries state; builds tr; dispatch(tr)
  → runtime: new_state = state.apply(tr)
      • steps applied into tr.doc (schema-validated; reject ⇒ no-op)
      • selection mapped forward
      • plugins fold state (history pushes inverse, collab queues steps, decorations remap)
  → runtime swaps state; calls EditorView::update(old_state, new_state)
      • view diffs old_state.doc vs new_state.doc → minimal host patches
      • view renders caret/selection FROM new_state.selection
      • view renders plugin decorations (preedit, collab cursors)
      • desktop view updates IME cursor area + pushes AccessKit TreeUpdate
```

No step in this pipeline reads the host tree for content. That is the whole game.

---

## 6. The view seam & desktop rinch-dom view

### The seam

```rust
// rinch-editor-core/src/view.rs  (pure; no rinch-dom/winit/web_sys)
pub trait EditorView {
    /// Project state change onto the host. MUST be pure wrt content (host is never read back).
    fn update(&mut self, prev: &EditorState, next: &EditorState);
    /// Abstract platform requests the runtime fulfills (desktop: winit; web: browser).
    fn request(&mut self, req: ViewRequest);
}
pub enum ViewRequest {
    EnableIme(bool),
    SetImeCaretRect(Rect),          // logical px; desktop → set_ime_cursor_area
    ScrollSelectionIntoView,
    AccessibilityRefresh,
}
/// Events the platform feeds INTO the editor (translated to commands/transactions by the runtime glue,
/// NOT by core). The view owns translation; core only owns apply().
pub enum ViewEvent { /* see §7 */ }
```

The core defines the trait and the request/event vocabulary. The **desktop view implements it in the `rinch` crate** (the only rinch-dom touchpoint). The **web view implements it in `rinch-web`**. Nothing else knows the renderer. The old thread-local `set_ce_api_factory`/`with_active_ce_api`/`register_ce_api` registry is **gone** — the runtime *owns* an `EditorView` value per focused editor.

### Desktop view: driving rinch-dom

The desktop `RinchDomEditorView` owns the CE root `NodeHandle` and a **`ViewDesc` tree** — the conceptual successor to `BlockMap` (`ce_render.rs:409`) — mapping model positions ↔ rinch-dom node ids. `update(prev, next)`:

1. **Diff** `prev.doc` vs `next.doc`. Because the model is persistent (`Rc`), subtree identity (`Rc::ptr_eq`) makes the diff cheap: unchanged children are skipped entirely.
2. For changed regions, **patch** rinch-dom via the *preserved* primitives (these stay; only the CE logic above them dies): `create_element`/`create_text`/`append_child`/`insert_before`/`remove_node`/`set_text_content`/`set_attribute`/`set_style`. The schema-driven HTML serializer (`serialize/html.rs`) decides tags from `parse_html_tags`, so the node→DOM mapping is total and shared with copy/paste.
3. **Render caret/selection from `next.selection`**, NOT from attributes. We compute caret geometry with the preserved Parley plumbing — `node.text_layout: Option<InlineLayout>`, `cached_text_parley`, `parley::Cursor::from_byte_index`, and `rinch_dom::text_query` (`byte_offset_from_position`, `caret_position_for_offset_layout`) — converting the model's **char `Pos`** to a **byte offset** for Parley only at this render edge. The caret and selection rectangles are emitted as ordinary scene primitives by the view (or as a small decoration overlay node it owns), not via `data-ce-cursor`/`data-ce-selection-start`.
4. Map the selection's caret to a logical `Rect` and `request(SetImeCaretRect(rect))` so the runtime keeps the IME candidate box positioned.

**Deleted in this move:** the entire attribute-driven paint contract — `paint/contenteditable.rs` CE functions, `paint/mod.rs:1030-1343` `data-ce-focused` overlay branch, and the global-offset producer (`dom_cursor_to_global_offset`/`walk_for_global_offset`). The fragile byte-agreement (ZWS stripping, `<br>`=`\n`, block separators duplicated across three files) is eliminated because there is no producer/consumer pair anymore — caret comes straight from `Selection`. `paint_input_value` (for `<input>`/`<textarea>`) is **preserved untouched**.

### Node-views

For nodes needing custom host rendering/behavior (images, horizontal rules, table cells, and — later — embeds), a plugin registers a `NodeViewCtor`. A node-view owns its DOM subtree and is told when its node/decorations change; the diff in (1) stops at the node-view boundary and delegates. This is how tables and atoms render without bloating the core diff.

### Block virtualization

`CeVirtualWindow` (`ce_virtualization.rs`) is **salvaged logic, re-homed**: it moves from under the deleted CE module to the desktop view, keyed on the CE root's block children. It keeps the two-phase contract with rinch-dom (`pre_layout_update` before `resolve_layout`, `post_layout_cache` after) using Taffy `estimated_height` + previous-frame `layout.y`. It is desktop-view-local; the core and web view never see it.

---

## 7. Desktop input — keyboard, IME, paste/copy/cut, pointer (model-first, no DOM-direct path)

All input lands as a `ViewEvent`; the desktop view translates it to a command call or a transaction. **There is no DOM-direct mutation anywhere.**

### Keyboard

The runtime's two `WindowEvent::KeyboardInput` arms (`rinch_runtime.rs:1601`/`:1629`) route to the focused view when CE has focus (replacing `handle_contenteditable_key`). The desktop view maps `winit::KeyEvent` → keymap lookup → command. Printable text not bound to a command becomes `insert_text` (consuming `stored_marks`). The `rinch_editable::EditCommand`/`InputHandler` rich-text borrowing is **deleted**; the view owns its own winit→command mapping.

### IME (net-new desktop work; winit fork already exposes the surface)

On CE focus the runtime calls `window.set_ime_allowed(true)` (the view requests `EnableIme(true)`; the runtime owns `self.window`). A **new `WindowEvent::Ime(Ime)` arm** is added beside the keyboard arms and forwards to the view:

- `Ime::Enabled` → mark composition-capable.
- `Ime::Preedit(text, cursor)` → **render a composition decoration, do NOT touch the document.** The preedit string is a `DecorationSet` widget at the caret; `cursor` is **byte-indexed UTF-8** (winit fork `event.rs:941`) and positions the candidate box. An empty `Preedit("", None)` clears the decoration. (winit always sends an empty preedit before a commit — we clear the decoration, then apply the commit transaction.)
- `Ime::Commit(text)` → `replaceSelection(Slice::text(text, stored_marks))` transaction.
- `Ime::DeleteSurrounding{before_bytes, after_bytes}` → a delete transaction; byte counts are converted to char `Pos` deltas at this seam.
- `Ime::Disabled` → clear composition state.

The view recomputes the caret `Rect` on **every** selection-changing transaction and `request(SetImeCaretRect(rect))` so the runtime calls `set_ime_cursor_area` — required for candidate-box placement.

**Android** mirrors this commit-only path: `android_runtime.rs:208-261`'s drain loop is rewired from synthetic `KeyDown` to **emit transactions directly** — `drain_committed_text()` → `Commit` transaction, `drain_deletions()` → delete transaction. `has_focused_contenteditable()` is answered by "a CE view is focused." The portable IME contract is **commit + delete-surrounding**; preedit is a desktop/web enrichment, not a core requirement.

### Paste / copy / cut (model-first, schema-whitelisted)

- **Paste:** read `paste_html()` (fall back to `paste_text()` — arboard returns `ContentNotAvailable` when no HTML, `native.rs:110`). HTML is tokenized by the existing zero-dep `parse_html_fragment` (`app/html_parser.rs`, salvaged as the front-end), then **run through the schema whitelist** in `serialize/html.rs` to produce a `Slice` — unknown tags/marks/attrs are dropped at parse time (`<script>`/`<iframe>` never materialize), `<a href>`/`<span style>` map to schema marks. The Slice is applied as one `replaceSelection` transaction. This kills the audit's critical S1 (raw-DOM paste that never touched the model) and the no-whitelist hole.
- **Copy/cut:** serialize the selected `Slice` from the **model** to HTML + plaintext via `serialize/html.rs` + `to_text`, and `copy_html(html, plaintext)`. Cut additionally dispatches a delete transaction. This replaces the DOM-walking `extract_selection_html`/`serialize_html_range` (which dropped `href`, audit C2).

Clipboard entry points (`rinch-clipboard`) are unchanged.

### Pointer

Pointer-down/drag translate to a model `Pos` via the preserved geometry helpers. We reuse `text_query` (`dom_cursor_to_ifc_offset`, `caret_position_for_offset_layout`) and `parley::Cursor::from_point`, plus `TextHitInfo` (`events/mod.rs:29-39`, retained) as the hit-test result — converting the layout byte offset to a model char `Pos` at this edge. `compute_dom_cursor_from_click` is **reference, deleted**; the new pointer→`Pos` path lives in the view and sets a `TextSelection` via a `set_selection` transaction. Drag-select extends `head`; double/triple-click set word/block ranges via model queries.

**Focus contract:** the RenderSurface/GameViewport keyboard interceptor (audit 4d) still runs first; CE input is only consumed when a CE view holds focus and no surface has grabbed keys.

---

## 8. Tables, Accessibility, Collaboration, Web view

### Tables (a real plugin, first-class cell positions)

Schema nodes `table > table_row+`, `table_row > table_cell+ | table_header_cell+`, cells contain `block+`. **No new Step kinds** — all table edits are `Replace`/`ReplaceAround`/`SetNodeAttr`:

- Insert table: `ReplaceStep` inserting a table node with `rows×cols` empty-paragraph cells.
- Add/remove row/column: `ReplaceStep`/`ReplaceAroundStep` over the table or rows.
- Cell merge/split: `SetNodeAttr` on `colspan`/`rowspan` + `ReplaceAround` to move content.

**Cell selection** is a first-class `Selection::Cell { anchor_cell, head_cell }` (real positions, replacing the dead `closest_table_cell: "{id}:{row}:{col}"` stringly hint). The plugin contributes keymap (Tab/Shift+Tab move between cells, arrow-at-edge crosses cells), commands (`addRowAfter`, `deleteColumn`, `mergeCells`, …), and a `table_cell` **node-view** for the desktop view to render/resize. Cell selection renders as a rectangle decoration from `Selection`. Tables ship in a milestone but are wired from day one — never a permanent stub.

### Accessibility (AccessKit plugin — fully greenfield)

Repo grep confirms **zero** AccessKit today; the desktop Vello/tiny-skia surface exports no a11y tree (the scattered `aria-*` attributes are inert). The a11y plugin builds the entire pipeline:

- **New dep** `accesskit` + `accesskit_winit` in the `rinch` crate (desktop view only).
- The plugin derives an `accesskit::TreeUpdate` from `EditorState`: doc structure → nodes with roles (paragraph→`Paragraph`, heading→`Heading`+level, list→`List`, cell→`Cell`, etc.), text content per textblock, and `state.selection` → a `TextSelection` on the focused text node.
- On **every transaction**, the desktop view pushes the `TreeUpdate` through the `accesskit_winit` adapter, wired into the runtime event loop next to the IME arm.
- **Web gets a11y for free** — the browser's native contentEditable exposes the accessibility tree; the web view contributes nothing here. This is another reason web delegates rather than ports.

The `accesskit_winit` version vs the forked winit `0.31-beta.2` is an open compatibility risk (§11).

### Collaboration (`rinch-editor-collab`, optional, non-wasm, end-to-end tested)

Automerge is **not** the model. Collaboration is a feature-gated plugin + adapter living **only** in `rinch-editor-collab`:

- The plugin records **unconfirmed local Steps** in its plugin-state (the ProseMirror collab protocol). It maps local Steps → Automerge operations on a CRDT *projection* of the doc.
- Remote Automerge **patches** are diffed from heads and translated to **Steps** (salvaging the *shape* of `patches_to_ce_ops`, `remote_ops.rs:74`: `Insert/Delete/Split/SetBlockType/Add|RemoveMark`), then applied as a remote Transaction with `meta(origin=remote, addToHistory=false)`. Local unconfirmed steps are **rebased over the remote mapping via `StepMap`** (this is why Steps must be `map`-able).
- The Automerge **transport** is salvaged: `generate_sync_message`/`receive_sync_message`/`save_incremental`/`load_incremental`/`merge`/`get_heads` (`sync.rs`) move into the collab crate.
- **Deleted:** `CeDocBridge` (`bridge.rs` — its CE subscriber is a no-op `let _ = remote_flag.get()` and `flush()` does extract-diff-rebuild, defeating CRDT merge), the Automerge-as-authoritative `EditorDocument`, the flat-position `text_obj_offset_to_flat_pos`/`block_start_flat_pos` (rewritten against the depth-aware `Pos`), and `ce_crdt_sync.rs` (tested the dead `CeOps` dual-write).
- **New tests** drive `Steps → adapter → Automerge → adapter → Steps` end-to-end (two states converge after concurrent edits) and **run in CI under the `collaboration` feature**. The plain model + desktop view compile and pass with collaboration **off** (no zero-test gate).
- `ContentEditableApi::as_any`/`as_any_mut` (which existed only to downcast to `CeOps` for collab) is **gone** — collab is a plugin over the public Step API, no downcasting.

### Web view (`rinch-web`, delegating to native contentEditable — closes #51)

A `WebEditorView` implements the same `EditorView` seam over `web_sys`, delegating caret/selection/IME/a11y to the browser:

- `update(prev, next)` patches the native DOM via the existing `WebDocument` (`web_document.rs`) using the **same schema-driven serializer**.
- It listens to **`beforeinput`/`input`/`compositionstart|update|end`** on the editable host (NOT the global `keydown` delegation in `event_delegation.rs:663`) and calls **`event.preventDefault()`** so the browser never mutates content — the model stays authoritative. Each event becomes a transaction.
- Caret/selection sync uses the existing **UTF-8↔UTF-16 conversion** helpers (`web_document.rs:274`, `event_delegation.rs:310`) because the model is char/byte-based and DOM ranges are UTF-16 code units. Pointer→`Pos` reuses `caretRangeFromPoint`→`data-block-index`→byte-offset (`resolve_text_hit`, `event_delegation.rs:321-371`).
- IME and a11y are **free** (browser native).
- `rinch-web/Cargo.toml` adds web-sys features it currently lacks: `CompositionEvent`, `Selection`, `InputEvent`, `HtmlElement` contentEditable. The crate stays wasm32-only, excluded from the workspace, and **must not** pull rinch-dom/parley/taffy/vello/automerge — `rinch-editor-core` is wasm-clean, so this holds.

---

## 9. The clean rip-and-replace — leave no dead code

### Crates deleted wholesale

| Crate | Reason |
|---|---|
| `crates/rinch-editor/` | Sole home of Automerge-as-model `EditorDocument` + the dead `Editor`/`StarterKit`/24-extensions/`CommandDispatcher`/`ExtensionRegistry`/two undo interpreters. Schema + selection salvaged into core first. |
| `crates/rinch-editor-macros/` | `extension!` proc-macro, zero source usages. |
| `crates/rinch-editor-components/` | `RichTextEditorConfig`, zero consumers. |

Remove all three from `[workspace.dependencies]` (Cargo.toml lines 31/32/33).

### Salvage-before-delete (move into `rinch-editor-core`)

- `rinch-editor/src/schema/{mod,node,mark,validation}.rs` → `core/src/schema/*` (verbatim; drop the dyn `Node`/`Mark` traits; add `content_match.rs` NFA).
- `rinch-editor/src/selection/{mod,state}.rs` → `core/src/selection.rs` (rebind to `Pos`; cut the `rinch_editable::Selection` re-export).
- `document/fragment.rs` markdown I/O (pulldown-cmark) → `core/src/serialize/markdown.rs`.
- `document/fragment.rs` HTML parser structure → starting point for `core/src/serialize/html.rs` (rewritten whitelist-driven).
- `rinch-editor/src/input/{rules,shortcuts}.rs` content + `normalize_key` logic → `core/src/{input_rules,keymap}.rs` (rebound to `(state)→Option<Transaction>` / winit keys).
- `history/operations.rs` invert/merge/group **semantics** → `core/src/plugins/history.rs` (over Steps).
- `MarkData` (plain) → `core/src/model/mark.rs`.

### Files deleted in `rinch` (the live CE engine — ~5,400 lines + paint + wiring)

- `crates/rinch/src/ce_ops.rs` (entire — `CeOps`, the second undo interpreter, `set_pending_editor_doc`, `apply_remote_changes`).
- `crates/rinch/src/ce_render.rs` (entire — tag tables, `extract_*`, `load_blocks`, `render_block_*`, `BlockMap`, `editor_pos↔dom_cursor`, ZWS math).
- `crates/rinch/src/app/contenteditable/` (entire dir — `mod.rs`, `ce_blocks.rs`, `ce_cursor.rs`, `ce_helpers.rs`, `ce_navigation.rs`, `ce_paste.rs`, `ce_selection.rs`, `ce_virtualization.rs`). `ce_virtualization.rs` logic is salvaged into the desktop view first.
- `crates/rinch-dom/src/paint/contenteditable.rs` CE functions: `paint_contenteditable_cursor`, `paint_ce_sub_blocks`, `is_block_tag`, `get_flat_text_len`, `collect_text_len_recursive`, `line_height_at_y`. **Keep `paint_input_value`, `parse_px`, `get_style_property`.**
- `crates/rinch-dom/src/paint/mod.rs:1030-1343` — the `data-ce-focused` overlay branch.

### Symbols deleted in `rinch-core`

- `crates/rinch-core/src/ce.rs` — **entire file** after extracting the serialization-shape concept (which is reborn as `DocNode` in core): `ContentEditableApi` trait + no-op defaults, `DomCursor`, `CeSelection`, `CeEvent` + `CeEventDispatcher` + thread-local dispatch (`subscribe_ce_events`/`dispatch_ce_event`/…), the active-API + factory registry (`set_active_ce_api`/`with_active_ce_api`/`set_ce_api_factory`/`register_ce_api`/`with_ce_api_for_node`/`CE_API_REGISTRY`/`CE_API_FACTORY`), `BlockData`/`InlineRunData`/`InlineMarkData`.
- `crates/rinch-core/src/dom/mod.rs:555-560` — `NodeHandle::with_ce_api`.
- `crates/rinch-core/src/events/contenteditable.rs` — **entire file** (100% dead: `ContentEditableClickData`, `ContentEditableDragData`, `CeClickInterceptor`/`CeDragInterceptor`, `set/clear/dispatch_ce_click`+`_drag`).
- `crates/rinch-core/src/events/mod.rs` — remove only `mod contenteditable;` (line 6) and `pub use contenteditable::*;` (line 15). **Preserve** `ClickContext`, `AncestorBounds`, `MouseButton`, `ModifierState`, `html_escape_string`, and `TextHitInfo` (retained for desktop hit-testing).
- `crates/rinch-core/src/lib.rs` — remove the `ce::{…}` re-export block (77-82) and the CE entries in the `events::` block (line 52 `ContentEditableClickData`/`ContentEditableDragData`; 55 `clear_ce_*_interceptor`; 57 `dispatch_ce_click`/`dispatch_ce_drag`; 63 `set_ce_*_interceptor`).

### Module decls & wiring to remove

- `crates/rinch/src/lib.rs:48` `pub mod ce_ops;`, `:50` `pub(crate) mod ce_render;`.
- `crates/rinch/src/app/mod.rs:10` `pub(crate) mod contenteditable;`.
- `app/mod.rs`: `set_ce_api_factory` closure (441-471), `register_ce_ops` (1844-1900), `sync_ce_ops_cursor` (1905), CE virtualization pre/post hooks (522-543, 570-590), fields `focused_contenteditable`/`ce_ops`/`ce_scroll_pending` (193/195/201, init 256-259), `focus_element` CE branch (1457-1491), and the `rinch_editor::EditorDocument::new()` construction (1884). **Replace** with `EditorView` instantiation owned by the app/runtime.

### Call-sites to rewire (must be updated, not orphaned)

- `app/event_dispatch.rs` ~262-275 (pointer set-cursor), ~915-917 (key routing) → route to focused `EditorView`.
- `app/click_handling.rs` ~107-292 (CE focus/cursor), ~463-470 (ce_ops dispatch) → `EditorView` focus + pointer→transaction.
- `app/debug_commands.rs` ~309-321, ~423-433, ~606-615, ~718-766 (MCP key/click routing) → route to focused `EditorView` so MCP `type_text`/`click` still drive the editor.
- `rinch_runtime.rs` — add `WindowEvent::Ime` arm; add `accesskit_winit` adapter; call `set_ime_allowed`/`set_ime_cursor_area` on CE focus/selection-change.

### What stays (do NOT break)

- **`rinch-editable` for single-line input.** `EditableState<StringDocument>`, `StringDocument`, `InputHandler`/`Key`/`Modifiers`, the text/cursor/selection/clipboard/undo `EditCommand` variants. The live `<input>`/`<textarea>` engine (`app/mod.rs:189` `focused_input_state`, `handle_input_edit_command` 1128, `try_focus_input` 1501, `click_handling.rs:399`) is **preserved verbatim**. Sever only the CE-side consumption (`contenteditable/*` imports). **Prune** the rich-text `EditCommand` variants (`command.rs:50-67`: Indent/Outdent/ToggleBold/…/ToggleBlockquote) and their no-op arms (`state.rs:171-184`); **remove** `MultilineDocument` (never instantiated — textarea uses `EditableState<StringDocument>`). Net: rinch-editable stays in the workspace and on the `rinch→rinch-editable` dependency edge.
- `rinch-clipboard` (unchanged), `paint_input_value` and the input data-attribute path, all shared `events/mod.rs` infra.

### Tests to delete/rewrite

- `crates/rinch/tests/ce_crdt_sync.rs` — delete (tested dead `CeOps`). Reborn as collab end-to-end tests in `rinch-editor-collab`.
- `crates/rinch/tests/ce_link_roundtrip.rs` — rewrite against `DocNode`/HTML round-trip.
- `model/mod.rs:677-834` behavioral specs (no per-char fragmentation, plain-after-bold not inherited, mark round-trip) — **port the assertions** to the new Step/Node tests.
- `serialization.rs`/`roundtrip_tests.rs` `from_block_data` cases → `DocNode` acceptance fixtures.

### Docs & examples to rewrite

- `docs/src/guide/contenteditable.md`, `docs/src/guide/editor.md` — rewrite around `EditorState`/`Transaction`/`Command`/`EditorView`; delete the "CRDT-first / single mutation path / StarterKit / hidden-textarea" narrative.
- `CLAUDE.md` ContentEditable section — replace with the new model.
- `examples/markdown-editor/src/main.rs`, `examples/ui-zoo/src/sections/editor.rs` — the `ce_do(|api| api.toggle_wrap(...))` helper becomes `editor.command("toggleBold")`; keep these as smoke-test/reference apps driving the new Command API.
- Delete stale docstrings: `rinch-editor/src/lib.rs:1-20`, `ce_ops.rs:8-11`, `bridge.rs` dual-write comments, all `skip_next_sync`/`sync_editor_doc_from_dom` references.

**Verification gate for "no dead code":** after the rip, `cargo build --workspace`, `cargo build -p rinch --no-default-features --features components,theme` (wasm-shaped), `cargo build --target wasm32-unknown-unknown` for web members, and `cargo clippy --workspace -- -D warnings` (catches dead-code) must all pass, plus `grep -r` for `ContentEditableApi|CeOps|EditorDocument|with_active_ce_api|data-ce-` returns only the new code/docs.

---

## 10. Sequenced implementation plan

Each milestone is independently compilable and testable. Earlier milestones do **not** delete the old engine; the rip lands in M8 once the new path is at parity, so `main` never has a broken editor. (Alternative: land the new crate behind a `new-editor` feature and flip in M8 — recommended to keep `main` green throughout.)

### M0 — Scaffold the pure core (no behavior change) — ✅ DONE (2026-06-20)
Create `rinch-editor-core`; lift `schema/*`; add it to the workspace. Old engine untouched. *(Selection lift deferred to M1 — it depends on `Pos`, which M1 introduces; lifting it "verbatim" in M0 would drag in `rinch_editable::Selection`.)*
- **Done:** new crate `crates/rinch-editor-core` (pure Rust); schema (`mod`/`node`/`mark`/`validation`) lifted; dyn `Node`/`Mark` traits dropped; `EditorError` lifted minus the `Automerge` variant; registered in workspace `[workspace.dependencies]`.
- **Tests:** **50/50** lifted schema tests pass; native + `wasm32-unknown-unknown` build clean; `clippy -D warnings` clean; `cargo tree` confirms zero automerge/rinch-dom/winit/parley/taffy/vello/web-sys.
- **Closes:** sets up the structural fix for #59; the wasm Automerge cost is gone from this crate now and from `rinch`/wasm once M8 lands.

### M1 — Model + positions + ContentMatch — ✅ DONE (2026-06-20)
`Node`/`Mark`/`Fragment`/`Slice`/`Attrs`; `Pos`/`ResolvedPos`; `ContentMatch`; `NodeType`/`MarkType` interned handles; typed `AttrValue`/`Attrs`.
- **Done:** persistent (`Rc`-shared) value model; single char-based integer position space (faithful ProseMirror `resolve`/`find_index`/`ResolvedPos`); `ContentMatch` DP replaces the old naive matcher (no duplicate logic); `ptr_eq` fast-path equality (A12); typed attrs (A11).
- **Verified:** adversarial pass (3 lenses + synthesis) — verdict *"sound enough to build M2 on; position math PM-faithful, no off-by-one; value model has no interior mutability so ptr_eq is sound; Eq/Hash complete; DP correct."* All 3 must-fix items applied (single `is_leaf` source of truth; ContentMatch compile hardening; fallible `Schema::text`).
- **Tests:** 79 pass (model sizes incl. multibyte, nested-doc resolution, two-run `text_node` + seam rule, ContentMatch edge cases, empty-content leaf); clippy `-D warnings` + wasm32 clean.
- **Closes:** the audit's "flat 2-level position space" and "no Node/Fragment value type" findings.

### M2 — Transform engine + schema enforcement
`Step` trait, `ReplaceStep`/`ReplaceAroundStep`/`AddMark`/`RemoveMark`/`SetNodeAttr`/`SetDocAttr`, `StepMap`/`Mapping`, `Transaction`, the gesture→steps helpers (`replace`/`split`/`set_block_type`/`wrap`/`lift`).
- **Tests (unit + property):** `apply∘invert == identity` for every step over random docs; `map` round-trips positions; rejected steps (schema-invalid slice) leave doc unchanged; the editing-operation→steps table each has a test.
- **Closes:** "no invertible/mappable model"; schema enforcement (decision G); structurally closes #59 (serialize totality lands in M3).

### M3 — Total serialization (the durable wire shape)
`DocNode` serde shape; schema-driven HTML serializer/parser (whitelist); markdown I/O salvaged.
- **Tests (round-trip):** `Node → DocNode → Node` identity for every starter-kit node/mark incl. attrs (link.href, image.src, heading.level, text_color.color, highlight.color); unknown type ⇒ `Err`; the old `from_block_data` cases ported as fixtures; HTML paste of `<script>`/`<iframe>`/`<span style>` is sanitized.
- **Closes:** **#59 and its whole class** (no mark/node dropped; no garbage tag).

### M4 — State, plugins, commands, keymap, input rules, history
`EditorState::apply`; `Plugin`/`Command`/`CommandRegistry`; `KeyBinding` + winit-agnostic normalize; `InputRule`; the History plugin (Step-based, grouped, typing-merge); stored-marks.
- **Tests (unit):** end-to-end "type, bold a range, undo, redo" over headless `EditorState` (no view); stored-marks "cursor bold then type" (audit S2); typing coalesces into one undo group; markdown input rules; toolbar queries (`is_mark_active`, `current_block_type`) read state.
- **Closes:** the two dead undo interpreters; the cursor-formatting bug; toolbar-reads-DOM anti-pattern.

### M5 — Desktop view + caret-from-state (parity, behind `new-editor` feature)
`EditorView` trait; `RinchDomEditorView` with `ViewDesc` diff/patch over preserved rinch-dom primitives; caret/selection rendered from `Selection` via Parley geometry; re-home `CeVirtualWindow`; node-views for image/hr; pointer→`Pos`. Keyboard input → commands.
- **Tests (headless-integration via MCP):** `launch_app` the markdown-editor example on the new path; `type_text`, `click`, toolbar buttons; assert `dom_tree`/`get_text_content`/`get_caret_position` match expected; screenshot caret. Verify no `data-ce-*` attributes exist.
- **Closes:** the DOM-direct mutation paths; the attribute-driven paint contract; the second cursor model.

### M6 — Desktop IME + paste/copy/cut
`WindowEvent::Ime` arm; `set_ime_allowed`/`set_ime_cursor_area` on focus/selection; preedit decoration; commit/delete-surrounding transactions; model-first paste (whitelisted Slice), copy/cut from model. Rewire Android drain loop to transactions.
- **Tests:** headless IME simulation (feed `Ime::Preedit`/`Commit` to the view; assert preedit renders as decoration not document, commit lands as a transaction); paste a `<b><a href>` snippet → assert marks survive; copy → clipboard HTML has `href`; CJK char round-trips (byte↔char seam).
- **Closes:** "no IME"; audit S1/C2 (raw-DOM paste; copy drops href).

### M7 — Tables + Accessibility
Tables plugin (schema, cell selection, commands, keymap, `table_cell` node-view); a11y plugin + `accesskit`/`accesskit_winit` wired into the runtime; `TreeUpdate` from state.
- **Tests:** table insert/add-row/add-column/merge as transactions (invert round-trips); Tab/arrow cell navigation; cell-selection rectangle renders; **a11y test** asserts the `TreeUpdate` contains correct roles + a text selection for a sample doc (AccessKit `TreeUpdate` snapshot test, no GUI needed).
- **Closes:** "inert tables"; "no accessibility."

### M8 — The clean rip + docs + flip default
Delete everything in §9; rewire all call-sites; remove `rinch-editor`/`-macros`/`-components`; drop Automerge from non-collab/wasm; flip `new-editor` to default (or remove the gate). Rewrite docs/examples/CLAUDE.md.
- **Tests/CI:** full `cargo build --workspace` + `clippy -D warnings` (dead-code gate); wasm member builds; the grep gate (no `CeOps`/`data-ce-`/`with_active_ce_api`); ui-zoo + markdown-editor MCP smoke tests on the new engine.
- **Closes:** "three overlapping engines"; "dead Editor framework"; "no dead/duplicate code"; removes the per-build Automerge cost (CI: confirm wasm bundle no longer links automerge/regex/pulldown-cmark).

### M9 — Collaboration (optional, non-wasm, CI-gated) — ✅ DONE (staged scope)
`rinch-editor-collab`: Step↔Automerge adapter (rebase via `StepMap`), salvaged sync transport; collab plugin; delete `bridge.rs`/`remote_ops.rs` flat-pos code.
- **Tests (collab):** two `EditorState`s + two adapters; concurrent insert/format; assert convergence after sync; rebase of local unconfirmed steps over remote; **runs in CI under `--features collaboration`** (no zero-test gate). `cargo test --workspace` (default) must still pass with collab off.
- **Closes:** "gated/dead collaboration"; binding decision #1 (CRDT optional, out of core/wasm).
- **Built (staged, A22 scope = flat text-blocks + marks):** `crates/rinch-editor-collab` — `CollabDoc` rich-text projection (`content: List<Block{type,attrs,text:Text}>`, marks over the Text; Automerge `Text` is codepoint-indexed = editor-core's char positions, no UTF conversion); **local** = `project_change` block-list diff (Rc-identity prefix/suffix, minimal common-prefix/suffix text splice — preserves CRDT identity so concurrent typing merges); **remote** = converged rebuild (`to_doc` → `build_remote_transaction` block-level `ReplaceStep`, provably convergent) plus the salvaged surgical `patches_to_remote_ops` translator; `CollabSession` lifecycle; salvaged sync transport (sync protocol + incremental broadcast); `CollabPlugin`/`CollabState`; `rebase_steps` primitive. Invariant `model ≡ project(model)`; **fail-loud `Unsupported`** on any non-flat node (A22). `collaboration = ["dep:rinch-editor-collab"]` is a **pure optional-feature gate** (not a `cfg(not(wasm32))` target gate) → default builds (desktop **and** web) link **zero** automerge (verified via `cargo tree`), while the adapter stays **wasm-compatible** so a future Rust web editor view reuses this *same* crate (a wasm app supplies a randomness source for the transitive `automerge → uuid`; verified the crate compiles for `wasm32-unknown-unknown` modulo that one downstream feature). The original design's `cfg(not(wasm32))` target gate was deliberately relaxed: it over-constrained — blocking opt-in web collab and forcing an awkward JS-CRDT bridge — while the optional feature alone already keeps automerge out of every default/web bundle. 8 convergence/round-trip/rebase/fail-loud integration tests + 2 plugin unit tests; clippy `-D warnings` clean; editor-core stays wasm-clean. **Deferred to a follow-up:** inline atoms (image/hard_break) collab, nested-block/table collab convergence, surgical patch→Step remote application (the converged-rebuild path is coarse but correct).

### M10 (follow-up) — Web view (closes #51)
`WebEditorView` in `rinch-web` over native contentEditable; `beforeinput`/`compositionstart` listeners + `preventDefault`; UTF-8↔UTF-16 seam; pointer via `caretRangeFromPoint`. Add web-sys features (`CompositionEvent`/`Selection`/`InputEvent`).
- **Tests:** Playwright-driven (per repo MEMORY: web validated via Playwright) — type, format, paste, IME composition; assert model-derived DOM matches; confirm `rinch-web` pulls no rinch-dom/parley/automerge.
- **Closes:** #51; proves the seam is genuinely renderer-agnostic.

**CI changes:** add `cargo test -p rinch-editor-core --target wasm32-unknown-unknown` (wasm-clean gate), a default `cargo test --workspace` job (collab off, must pass), a `--features collaboration` job, and the dead-code grep gate to the pipeline.

---

## 11. Open questions / risks (author input wanted)

1. **`accesskit_winit` vs forked winit `0.31-beta.2`.** AccessKit's winit adapter tracks upstream winit versions; the vendored fork may need an `accesskit_winit` compatible with the fork's `0.31-beta` window/event-loop API, or a thin custom adapter. **Decision needed:** accept a custom AccessKit adapter against the fork, or pin `accesskit_winit` and patch the fork to match? (M7 blocker if mishandled.)

2. **Position unit at text level: char vs UTF-16.** We chose **char (Unicode scalar)** positions inside text nodes (clean for winit byte-preedit and Automerge). The web seam is UTF-16 and converts at the boundary; desktop Parley is byte and converts at the boundary. This is correct but means **two conversions exist**. Confirm char (not UTF-16) is the core unit — it's the right call but it makes the web seam do slightly more work.

3. **Landing strategy: `new-editor` feature flag vs branch.** The plan keeps `main` green by building the new path behind a feature and flipping in M8. This doubles some app-wiring temporarily. **Confirm** you prefer feature-gated coexistence over a long-lived branch.

4. **History grouping affinity.** ProseMirror groups by time + selection adjacency. We'll port `LocalUndoStack`'s typing-merge window (default ~500ms). **Confirm** the desired undo granularity (per-keystroke vs per-word vs time-window) — it's a UX call, easy to tune, but worth fixing early so tests assert the right behavior.

5. **Collab projection fidelity.** Mapping the depth-aware `Pos` tree onto Automerge's text/list objects (and back) for arbitrary nesting (nested lists, tables) is the hardest part of M9. The salvaged `patches_to_ce_ops` only handled flat text. **Risk:** the first collab milestone may scope to text + flat blocks + marks, deferring table/deep-nest collab convergence to a follow-up. Confirm that staged scope is acceptable.

---

## Appendix A — Post-Validation Amendments

Four adversarial critics (coherence, feasibility-vs-real-APIs, rip-completeness, scope-closure) reviewed the body above against the real code; a synthesis pass verified every issue and produced the concrete changes below. **Verdict: `ready-with-amendments`.** Nothing structural was wrong — these are unspecified joints in an otherwise coherent system, every one verified against a real file:line. **M0–M4 are unaffected (build as written); the amendments land in M1 (attrs/Step), M4 (command type/catalogue), M5–M9, and the rip checklist.** Each amendment is authoritative over the body where they conflict.

### Blockers (must land before the milestone that depends on them)

**A1 · winit IME API is deprecated (affects §7, §6 `ViewRequest`, §9 wiring; milestone M6).**
The fork marks `set_ime_allowed` / `set_ime_cursor_area` / `set_ime_position` `#[deprecated → use Window::request_ime_update]` (`winit-core/src/window.rs:1103/1134/1161`), and `set_ime_cursor_area` no-ops unless `cursor_area` was negotiated. M6-as-written would fail the design's own `clippy -D warnings` gate. **Use** `request_ime_update(ImeRequest::Enable(ImeEnableRequest::new(ImeCapabilities::new().with_cursor_area().with_hint_and_purpose(), …)))` on focus, `ImeRequest::Update(…with_cursor_area(pos,size))` on caret change, `ImeRequest::Disable` on blur. Redefine `ViewRequest::EnableIme(bool)` → `EnableIme(ImeConfig { hint, purpose })`; map `SetImeCaretRect` onto the Update path. Treat `Ime::Preedit`'s second field as `Option<(begin_byte, end_byte)>` — a byte **range**, not one cursor (`event.rs:956`).

**A2 · AccessKit has no off-the-shelf adapter for the forked winit (affects §8, §11 risk 1; milestone M7).**
Repo grep = zero accesskit (greenfield, as stated), and the vendored fork is the winit-core 0.31-beta multi-crate split exposing trait-object `Window`/`ActiveEventLoop`; every published `accesskit_winit` binds monolithic winit 0.30 and will not compile. **Decision (no longer open):** hand-write a thin AccessKit adapter directly on `accesskit_windows`/`accesskit_macos`/`accesskit_unix` against the fork's `Arc<dyn Window>` / `&dyn ActiveEventLoop`, plus focus/activation plumbing. Split **M7 → M7a Tables / M7b A11y**; desktop-feature-gate the `accesskit_*` crates so they never reach `rinch-editor-core` or any wasm member (`accesskit_unix` pulls zbus/async). Ship the free web-native a11y path first.

### Majors

**A3 · Two-phase view contract (§6; M5).** Split `EditorView` projection to mirror the verified DOM-mutate → `resolve_layout` → measure pipeline (the same seam `CeVirtualWindow` already straddles with `pre_layout_update`/`post_layout_cache`): `update_dom(prev, next, prev_decos, next_decos)` **before** `resolve_layout` (DOM patches + decoration node create/remove + IME-enable requests), and `update_caret(next)` **after** `resolve_layout` (caret/selection/decoration geometry from fresh `cached_text_parley` + `SetImeCaretRect`). Caret geometry MUST NOT be computed in the call that mutates the DOM.

**A4 · Decorations are a first-class view input (§6 + §5/§7; M5/M6).** `update_dom` diffs `prev` vs `next` `DecorationSet` (from `state.decorations()` aggregated across plugins) and patches decoration/overlay nodes (preedit widget, collab cursor, selection/cell rect) **independently of the doc diff**. A transaction that changes only decorations (IME preedit) still triggers a view update handled entirely by the decoration-diff step (zero doc diff, zero selection change). Add it as an explicit numbered step.

**A5 · Preedit geometry seam (§6/§7 new subsection; M6).** The IME preedit decoration carries its **own throwaway Parley layout** laid out fresh at the caret anchor (the preedit text has no model `Pos`); the candidate box `Rect` derives from that local layout using the `(begin_byte, end_byte)` range. Commit must consume-and-clear the preedit decoration **and** apply the `Ime::Commit` transaction atomically in one `state.apply` (the fork suppresses `KeyboardInput` during preedit and always sends an empty `Preedit` before `Commit`).

**A6 · Command catalogue + example mapping (§5/§9; M4/M5).** Add a built-in command table to §5: `toggleBold/Italic/Underline/Strike/Code/Highlight/Subscript/Superscript` (AddMark/RemoveMark or stored_marks), `setParagraph/setHeading{level}/setCodeBlock` (set_block_type → ReplaceAround/SetNodeAttr), **`toggleBulletList`/`toggleOrderedList` (WRAP/lift, NOT set_block_type)**, `wrapInBlockquote` (wrap), `sinkListItem`/`indent` + `liftListItem`/`outdent` (ReplaceAround nesting), `toggleLink`, `insertHr`, `insertImage`, `undo`/`redo`. Map every old `ce_do`/`set_block_type('ul'|'ol'|'blockquote')`/`indent()`/`outdent()` call in `markdown-editor/src/main.rs` (187-195) and `ui-zoo/.../editor.rs` (79-102) to its new command. Add ancestor-aware toolbar queries `in_node_type(&state, list|blockquote)`/`can_lift`/`can_sink` (from `ResolvedPos.node(depth)`) — `current_block_type` alone cannot drive the List/Blockquote button active-states (the textblock stays `paragraph` inside a `list_item`).

**A7 · Public editor handle (§5/§6 new subsection; M5 — blocks every consumer).** §9 deletes the entire app-facing surface (`with_active_ce_api`, `NodeHandle::with_ce_api`, `load_html`, `extract_content`) but never defines the replacement. Specify: how `rsx!` mounts an editor (`Editor {}` → `EditorHandle`), how to set initial content **before focus** (`handle.load_doc(DocNode)` / `handle.load_html(&str)` operating on the owned `EditorState` even unfocused — successor to the pre-focus `with_ce_api(...).load_html(...)` pattern in CLAUDE.md and `ui-zoo editor.rs:139`), dispatch (`handle.command("toggleBold")`), query (`handle.is_mark_active`), change subscription, and save (`handle.doc() → DocNode`).

**A8 · Placeholder / empty-state (§6; M5).** Confirmed audit gap (§5.7) addressed nowhere, and the rip deletes the whole CE paint path → a parity regression vs the preserved `<input>` path. Render an empty-state decoration from `EditorState` (when `doc` == one empty textblock, show a dimmed placeholder prop) through the same decoration-diff step. Add an M5 test for show/hide.

**A9 · Preserve `has_focused_contenteditable()` + Android caller (§9/§7).** Add `crates/rinch/src/embed.rs:270` (`has_focused_input() || has_focused_contenteditable()`, a live non-CE caller) to the rewire list; **keep and re-point** `has_focused_contenteditable()` to "a CE `EditorView` holds focus" rather than deleting it with the `focused_contenteditable` field (`android_runtime.rs:209` also depends on it).

**A10 · Focus arbiter, not a restated contract (§7; closes audit 4d).** Replace the single sentence with an enforced `enum FocusTarget { Surface(id), Editor(id), Input(id), None }` owned by the runtime (focus mutually exclusive by construction). Route `KeyDown`/`Ime` by matching on `FocusTarget` instead of interceptor-then-fallback; last-focused wins; emit a debug diagnostic when a surface swallows a key while a CE Editor is the focus target; IME-enable follows `FocusTarget::Editor`. The current wording verbatim describes the *broken* `event_dispatch.rs` fallback behavior audit 4d flagged.

**A11 · Schema is "lift structure, then re-type attrs" — not verbatim (§2/§3/§9; M1).** Today `AttrSpec.default: Option<String>` (`node.rs:155`), attrs are stringly (`optional("1")`, `optional("false")`), specs live in a non-deterministic `HashMap` (`mod.rs:25/27`). Reword to: migrate `default → Option<AttrValue>`; per-attr parse rules (`heading.level "1"→Int(1)`, `task.checked "false"→Bool(false)`); `HashMap → BTreeMap`/`IndexMap` for deterministic iteration (matches §3's ordered `Attrs(BTreeMap)`); and wire required-attr validation into `Step::apply`. Reserve "verbatim" for content-expression strings and `parse_html_tags` only.

**A12 · `Step` trait needs `clone_box`; Node diff needs an explicit ptr-eq contract (§4/§6; M2/M5).** Add `fn clone_box(&self) -> Box<dyn Step>;` (trait objects can't derive `Clone`; `Transaction`'s `Vec<Box<dyn Step>>`, history's inverted steps, collab rebase, and decoration mapping all need to clone). State that §6's ViewDesc diff uses `Rc::ptr_eq` as the fast skip and structural `PartialEq` only as fallback, and that `Fragment` shares an `Rc` per child so `ptr_eq` actually skips unchanged siblings.

### Minors

**A13 · §7/§9:** there are **three** `KeyboardInput` arms in `rinch_runtime.rs` (1601 Pressed / 1629 Released / **1963 F12 devtools**). Reroute 1601/1629 to the focused `EditorView`; leave the F12 arm **untouched**; add the new `WindowEvent::Ime` arm beside them.

**A14 · §6 primitive names:** the real rinch-dom API is `RenderScope::create_element`/`create_text` (`render_scope.rs:58/66`), `NodeHandle::set_text` (not `set_text_content`), `remove`/`remove_child` (not `remove_node`), plus `append_child`/`insert_before`/`set_attribute`/`set_style`. `text_layout` is `Option<Box<InlineLayout>>` (boxed).

**A15 · §5/M5 ViewDesc:** add an explicit responsibility — maintain per-textblock char-length prefix sums + an IFC byte map so a model char `Pos` resolves to `(rinch-dom node id, byte offset)` before calling the Parley helpers. This char→(node,byte) mapping across a multi-text-node IFC is **net-new** M5 work, not free reuse.

**A16 · §9 rinch-core Cargo cleanup:** after `ce.rs` is deleted, remove `serde = ["dep:serde"]`, the optional `serde` dep, and the (non-optional) `serde_json` dep from `rinch-core/Cargo.toml` (only `ce.rs` used them); delete the `block_data_serde_round_trip_snake_case` test; repoint `rinch/Cargo.toml:121` `serde = ["rinch-core/serde"]` → `["rinch-editor-core/serde"]`. (Feature dead-config isn't caught by clippy's dead-code lint.)

**A17 · §9 paint range:** delete the full `data-ce-focused` block (`paint/mod.rs` ~1031–1359, through the closing braces) — stop just before the IFC block at ~1362; deleting only 1030–1343 leaves orphaned braces.

**A18 · §9 `paint/contenteditable.rs` is retained-but-pruned**, not wholesale-deleted: keep `paint_input_value`/`parse_px`/`get_style_property`; delete `paint_contenteditable_cursor`/`paint_ce_sub_blocks`/`is_block_tag`/`get_flat_text_len`/`collect_text_len_recursive`/`line_height_at_y` + CE tests.

**A19 · §9 stale imports + grep gate:** also remove dead `use` lines (`app/mod.rs:25` `use crate::ce_ops::CeOps;`, `:26` `use rinch_core::ce::{…}`, and CE imports in `click_handling.rs`/`event_dispatch.rs`/`debug_commands.rs`). Broaden the grep gate to also assert zero hits for `set_ce_api_factory`, `with_ce_api_for_node`, `register_ce_api`/`unregister_ce_api`/`clear_active_ce_api`, `CeEvent`/`subscribe_ce_events`/`dispatch_ce_event`, `BlockData`/`InlineRunData`, and the CE paint fn names.

**A20 · §2 vs §5 command type:** delete the bare-`fn` signature in the §2 `command.rs` comment; the load-bearing type is §5's `Command = Rc<dyn Fn(&EditorState, Option<Dispatch>) -> bool>` (a bare fn can't close over `level`/`href`).

**A21 · §2/§9 `rinch-editable` status:** it stays **optional, desktop-gated** (`Cargo.toml:17` `optional = true`, enabled at features 84/93) — not unconditional. Wasm never gets it; the web view delegates `<input>` to the browser. Correct the §2 line refs to "17 (dep), 84/93 (feature lists)."

**A22 · §8/§11 collab — fail loud + name the bound:** keep the staged scope (text + flat blocks + marks first) but make the adapter **return `Err`** (not silently drop) on any Step it can't project, so deep-nest collab is a clean "unsupported" error, not silent non-convergence (which would reintroduce the exact divergence class this rip kills). Name the flat-`CeRemoteOp` ↔ depth-aware-`ReplaceStep` gap as a known bounded seam limitation. Add an explicit post-M10 deferral line for spellcheck + find/replace (search is naturally a `DecorationSet` plugin — a clean defer).

### Carried-forward risks (into implementation)

- **AccessKit/fork adapter** (A2): per-platform plumbing effort still unestimated — committed M7b, not an open question.
- **Deep-nested collaboration** projected onto Automerge remains unproven; staged + fail-loud (A22), full convergence deferred post-M9.
- **Two char↔byte/UTF-16 conversion edges** (desktop Parley byte seam + web UTF-16): correct by design, but the char→(node,byte) IFC mapping (A15) is a likely source of off-by-one caret bugs at CJK/emoji boundaries until well-tested.
- **History grouping affinity** (§11 risk 4): pin a chosen granularity (per-word vs ~500ms) early so tests don't churn.
- **IME preedit/commit atomicity on slow frames** (A5): needs an integration test with the focus arbiter (A10) + surface interceptor, not just a unit test.