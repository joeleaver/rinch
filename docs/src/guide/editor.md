# Rich-Text Editor

Rinch's rich-text editor is a **ProseMirror-faithful architecture in idiomatic
Rust**: a pure, renderer-agnostic core (`rinch-editor-core`) plus an equally
renderer-agnostic view (`rinch-editor-view`) that projects it onto whichever host tree
the platform provides — rinch-dom on desktop, the browser DOM on web. This page is the
conceptual guide to that core — the document model, schema, steps and transactions,
state, commands, history, and the view seam.

> **Just adding an editor to a screen?** Start with the
> [Rich-text editing](./contenteditable.md) guide — `create_editor()`, the
> `Editor {}` component, and the command API. This page is the layer underneath it.

## The one big idea

There is exactly one source of truth and exactly one way to change it:

1. **`EditorState` is a value** — `{ doc, selection, stored_marks, plugin_state }`.
   No DOM node is ever authoritative.
2. **Every edit is a `Transaction` of invertible `Step`s**. `state.apply(tr)`
   returns a *new* `EditorState`; it is pure and side-effect-free.
3. **The view is a pure function of state.** The view diffs the old and new document
   and patches the host tree; it renders the caret and selection from
   `state.selection`. The host tree is never read back for content.
4. **Input produces transactions, not DOM edits.** Keys, IME, paste, and pointer
   selection are translated into commands/transactions.
5. **The schema is authoritative and enforced.** Transactions are validated; invalid
   steps are rejected, never silently written. Serialization is total and
   attr-aware — no mark or node type can be dropped.

Because the host is *derived* from the model on every transaction, "the DOM and the
model disagree" is structurally impossible.

## The document model

The model is a **persistent (structurally-shared) immutable tree**. Cloning a `Node`
is a cheap `Rc` bump; every edit produces a new tree that shares unchanged subtrees
with the old one. That is what makes states cheap to keep in history and cheap to
diff in the view.

### Node, Mark, Fragment, Slice

- **`Node`** — a value in the tree. It has a schema node type, typed attrs, a
  `Fragment` of children (empty for leaves), marks (on inline/text leaves), and
  optional text (for the `text` node). `hard_break`, `horizontal_rule`, and `image`
  are first-class nodes, not strings.
- **`Mark`** — an inline annotation (`bold`, `italic`, `link{href}`, …) carried on
  text and inline leaves, with typed attrs.
- **`Fragment`** — an ordered, sized, `Rc`-shared child list with the cut / append /
  replace primitives the transform engine needs.
- **`Slice`** — `{ content: Fragment, open_start, open_end }`. The open depths let a
  copied or pasted range merge into the surrounding structure; it is the unit that
  `replace` and paste operate on.

```rust
use rinch::prelude::*;            // Node, Mark, Slice, Schema, Selection, Pos, …

let doc = editor.doc();           // the current document Node (the save shape)
assert!(doc.child_count() >= 1);
```

### One position space

The editor uses ProseMirror's **single depth-aware integer position space**, `Pos`:

- Each node contributes **1** for its opening boundary and **1** for its closing
  boundary.
- Text contributes **one position per Unicode scalar value (char)**, not per byte —
  byte offsets exist only transiently at platform seams (Parley layout, IME).
- The document root starts at `0`; `doc.content_size()` is the last valid position.

`Pos` resolves to a `ResolvedPos` that answers `parent()`, `node(depth)`,
`before(depth)`/`after(depth)`, `index(depth)`, and `marks()` — replacing flat
block/inline/offset math with one consistent model.

### Schema and ContentMatch

The **schema** defines which nodes and marks exist and what content each node may
contain. The starter-kit catalogue:

- **Nodes:** `doc`, `paragraph`, `heading{level}`, `blockquote`, `code_block`,
  `bullet_list`, `ordered_list`, `list_item`, `horizontal_rule`, `hard_break`,
  `text`, `image{src, alt}`, plus the table nodes.
- **Marks:** `bold`, `italic`, `underline`, `strike`, `code`, `link{href}`,
  `highlight{color?}`, `text_color{color}`, `subscript`, `superscript`. Every mark
  is inclusive except `link` (see [inherited marks](#state-selection-stored-marks)).

Each node spec carries a **content expression** (e.g. `blockquote > block+`,
`list_item > block+`, `bullet_list > list_item+`). These compile to a **ContentMatch
NFA** that the transform engine consults to decide whether a step's result is valid —
`matchType`, `matchFragment`, and `fillBefore` drive both validation and the
automatic insertion of required nodes. Required attrs (`link.href`, `image.src`,
`heading.level`) are applied/enforced at the step boundary, so attr-aware round-trip
is structural, not best-effort.

**Node and mark types are interned per `Schema` instance, and compared by pointer.**
`NodeType`/`MarkType` equality is `Rc::ptr_eq`, so two `Schema::starter_kit()` values
mint two `bold` handles that are never equal — and a `Mark` carries its `MarkType`, so
marks from different schemas never match either. One document, one `Schema`: build the
document, the transform, and every mark you hand it from the *same* `Rc<Schema>`.

Getting this wrong used to be silent. `Transform::remove_mark` matched nothing and
returned `Ok`, so the removal quietly did nothing; `add_mark` added the foreign mark
*beside* the document's real one. Both now fail loud with a `StepError` naming the cause
(issue #217) — but only for a genuine type-identity mismatch. Removing a mark the range
simply does not carry, or a `link[href=a]` where the text has `link[href=b]`, is still an
ordinary no-op. The safest way to name "whatever this document calls bold" is to read the
handle off the document (or off `state.schema()`, which is the same instance).

The same pointer identity is why comparing `Node`s across two editor handles fails even
for structurally identical documents — compare their serialized HTML instead.

**This reaches `EditorHandle` too.** `create_editor` mints a new `Rc<Schema>` per handle,
and `load_doc` installs the `Node` you give it as-is, so `b.load_doc(a.doc())` hands `b` a
document built by `a`'s schema. `b` then reports `is_mark_active("bold") == false` over
text that is bold, and its formatting commands return `false` rather than editing it —
before the guard above they returned `true` and left the run carrying *two* `bold` marks,
one per schema. Move a document between handles through a serialization instead:

```rust
b.load_html(&to_html(&a.doc()));        // or
b.load_doc(b_schema.node_from_doc(&a.doc().to_doc())?);
```

### Serialization

The durable save/load shape is a recursive, schema-derived structure (under the
`serde` feature). Serialize walks the schema, so every node type and mark type has a
name and there is no string-tag fallthrough; deserialize consults the schema and
**rejects** unknown types rather than dropping them silently. HTML serialization
derives tags from the schema's `parse_html_tags`, so copy-out and paste-in share one
table — the same one `Editor`'s `content:` prop and `load_html` use.

The HTML import reads its three integer attributes as Chrome 153 does (#1164):
`<ol start>` by HTML's rules for parsing integers (`" 3"`, `"3abc"` and `"2.5"`
are 3, 3 and 2; a value past `i32` is the default 1), and `colspan` / `rowspan`
clamped to 1..=1000 and 0..=65534, a value too large to parse being the maximum,
and a negative or unparsable value 1.
The model has no "to the end" span, so `rowspan="0"` is imported as the number of
rows left in the cell's row group (`<thead>`, `<tbody>`, `<tfoot>`, or a run of
bare `<tr>`s), counting its own — rows the import keeps: a `<tr>` with no cells is
dropped and not counted, where Chrome counts it. Any other `rowspan` is cut at the
same place, the end of its row group, which is where Chrome 153 cuts it when it
lays the table out (#1176).

A pasted row's spans also reach at most 1000 grid columns
(`tables::MAX_IMPORTED_ROW_WIDTH`), counting the columns that rowspans from the rows
above carry into it: a `colspan` is cut to what is left, never below 1, and a cell
that starts in a row already 1000 columns wide spans that row alone, so it carries
nothing down (#1176). Each such cell adds one column, so a row is at most 1000 columns
plus one per cell that found it full. That limit is rinch's, not Chrome's (Chrome lays
out a row of 3001 columns); without it a row's width grew with every rowspan above it,
and a 57 KB paste made a 1,000,000-column table.

Whatever route a table takes into the model — paste, `load_doc`, an app's own
transaction, the table commands — the grid `TableMap` builds over it is bounded:
`tables::column_count` caps `width × rows` at `tables::grid_slot_budget`, twice the
table's cells and never less than 2^22 slots (32 MB of map). A rectangular table with
no spans fills exactly `width × rows` slots with as many cells, so it is never cut;
what can be cut is a grid of more than 2^22 slots and more than twice its cells, as
spans or ragged rows make one. Past the cap a cell is cut at the grid's right edge,
and one that starts past it is in no slot, so the table commands do nothing there.

A slot no cell covers — the tail of a row shorter than the grid, or what the cap cut —
is a hole, and the table commands treat it as no cell (#1184): `deleteRow` and
`deleteColumn` pass over it, `addColumnBefore`/`addColumnAfter` give a row with a
hole at that column a new cell at its end, a row added by `addRowBefore`/`addRowAfter`
gets a cell in a hole's column, and `mergeCells` grows the top-left cell over the holes
in its rectangle. Every span a command writes is the cell's extent in the grid ± 1,
never the attribute's own value ± 1: a `colspan` of `i64::MAX` in a two-row table,
which the grid cuts to 2^21 columns, is 2^21 + 1 after `addColumnAfter` across it.

The view lays a table out as that same grid (#1182): `<table>` is a CSS grid of
`tables::column_count` columns and each cell is placed by its rectangle in the map
(`tables::cell_rects`), `grid-column: span <width>` / `grid-row: span <height>`, not
by its raw attributes — so a `rowspan` of `i64::MAX` in a two-row table spans two
grid rows. A cell the map has no slot for is laid out as a band across the whole grid
(`grid-column: 1 / -1`) on a row of its own, so its text stays visible and adds no
column. Written raw, a few cells of huge spans stacked past the 32767 grid lines Taffy
numbers a grid with, and desktop layout panicked. Two limits remain: the desktop's
Stylo clamps a grid template and every span to 10000 tracks, so a table wider than
that is drawn 10000 columns wide; and the cells are *auto-placed*, which can put a
cell in a different row than the map does when a rowspan leaves a row short (#1209).

## The transform engine

Every editing operation is a `Transaction` carrying one or more `Step`s. Steps are
**invertible** (for undo) and **mappable** (for redo, collaboration rebase, and
decoration tracking).

### A deliberately minimal Step set

There is *not* a step per gesture. A small primitive set expresses everything; the
gestures map onto it:

| Step | Purpose |
|------|---------|
| `ReplaceStep { from, to, slice }` | Replace a range with a `Slice`. **The workhorse** — text insert, delete, split, join, and paste all reduce to this. |
| `ReplaceAroundStep { … gap …, slice }` | Replace around a preserved gap — wrapping (blockquote/list), lifting, and re-parenting block-type changes. |
| `AddMarkStep { from, to, mark }` | Add a mark across an inline range. |
| `RemoveMarkStep { from, to, mark }` | Remove a mark across an inline range. |
| `SetNodeAttrStep { pos, attr, value }` | Change one node attr (heading level, image alt, list start). |
| `SetDocAttrStep { attr, value }` | A document-level attr. |
| `BatchStep` (`Transaction::batch(Vec<BatchEdit>)`) | Many disjoint replaces and node-attr changes, stated in the coordinates of the document it applies to, as **one** step. |

`BatchStep` is the one step kind that exists for cost rather than expressiveness. It
means its edits applied one at a time from the last position to the first (two
changes of one node's attribute: the last given wins): it builds the document they
build, and its step map maps every position as those `ReplaceStep`s did. It differs
in three places. Each rebuilt node's content is checked once, on the result, so it
accepts a batch whose sequence would pass through an invalid intermediate state.
Mapped over another change (`Step::map`, a rebase), each *range* maps as one
`ReplaceStep` of it would — and inserts at one point, and deletions that meet, are one
range from construction, so a concurrent insert between two such deletions is deleted
with them where two separate deletions would keep it. Two kept ranges the change brings
to meet end to end become one replace of both — the same document — unless one
carries an open slice: then the later edit is dropped and **its content is lost**,
where separate steps would apply it. An attribute change the change puts inside a kept
range is dropped as well. No table command builds an open slice. And a selection a command
does not set is mapped once through the step and resolved in its result, where a
step per row mapped it after every step: in about 0.6% of the commands the table
differential compares, the selection lands somewhere else (the old one was placed in an intermediate document). It rebuilds
each node on the way to an edit once and keeps one document in the transaction
rather than one per edit. Every table command is one `BatchStep`
(#1200): as a step per row, a column into 16,000 rows rebuilt and kept the row list
16,000 times (6.4 s, 2.1 GB; 29 ms as one step). The one exception is `deleteRow` /
`deleteColumn` on a table whose cells overlap, which still removes one row or
column per step on a recomputed map. Collaboration adds no step kinds.

A few gesture → step mappings:

| Gesture | Step(s) |
|---------|---------|
| Type a character | `ReplaceStep(from, to, Slice::text(ch, stored_marks))` |
| Backspace mid-text | `ReplaceStep(pos-1, pos, empty)` |
| Enter / split block | a `ReplaceStep` that splits the textblock (open slice + new block) |
| Toggle bold over a range | `AddMarkStep` / `RemoveMarkStep` over `from..to` |
| Toggle bold at a cursor | **no step** — set `stored_marks` (applied to the next typed text) |
| Wrap in blockquote | `wrap(range, [(blockquote, {})])` → `ReplaceAroundStep` |
| Paste | parse to a `Slice`, then `ReplaceStep(from, to, slice)` |

`Step::apply` is where **schema enforcement** lives: a `ReplaceStep` whose slice
would violate the parent's ContentMatch returns an error and the whole transaction is
rejected. Invert is mechanical; map rebases positions through a `Mapping`; merge
coalesces consecutive single-char replaces so typing groups naturally.

### Transactions

A `Transaction` accumulates steps (each applied into a running `doc`), maps the
selection forward as steps are added, and carries `stored_marks` and plugin meta. You
rarely build one by hand — commands do — but the shape is:

```rust
// Inside a command, or via EditorHandle::update:
let mut tr = state.tr();
tr.insert_text("hello").ok()?;     // a ReplaceStep
tr.set_selection(Selection::cursor(Pos(/* … */)));
Some(tr)                           // dispatched and applied by state.apply
```

## State, selection, stored marks

```text
EditorState { doc, selection, stored_marks, schema, plugins, plugin_state }
```

`state.apply(tr)` runs the transaction's steps, then folds each plugin's state
forward (history pushes inverted steps). Decorations are **not** stored and
remapped by the core: `state.decorations()` asks every plugin afresh for the
state being rendered, so a plugin that caches decoration ranges maps them through
`tr.mapping()` in its own `apply`.
It returns a brand-new state; nothing is mutated in place, no DOM is touched.

**Selection** is part of state and is mapped forward by every transaction. It is one
of:

- `Text { anchor, head }` — a text caret/range (anchor fixed, head moving).
- `Node { pos }` — a whole atom selected (an image, a horizontal rule).
- `Cell { anchor_cell, head_cell }` — a table-cell rectangle.

The caret is **rendered from the selection** by the view — there is no second cursor
model.

**Stored marks** carry a tri-state for the "click Bold then type" case:

- `None` → inherit marks from the position context on the next insert.
- `Some(vec![])` → explicitly no marks.
- `Some([bold])` → the next inserted text gets bold.

Toggling a mark at a *collapsed cursor* sets `stored_marks`; toggling over a *range*
emits an `AddMark`/`RemoveMark` step.

**Inherited marks and `inclusive`.** With no stored marks, typed text takes the marks
of its position (`ResolvedPos::marks`, ProseMirror's `$pos.marks()`):

- inside a text run: that run's marks;
- at a boundary between two runs: the marks of the run *before* it;
- at the start of a textblock: the marks of the run after it.

A mark spec's `inclusive` flag (`MarkSpec::inclusive`, default `true`, builder
`.inclusive(false)`) decides what happens at a mark's **end**. An inclusive mark
(`bold`) carries on: type right after a bold word and the new text is bold. A
non-inclusive mark does not: at a boundary it is dropped unless the run after the
position carries a mark of the same type too, and at a textblock's start it is
dropped. The starter kit's `link` is the one non-inclusive mark, as in ProseMirror's
example schema and tiptap, so for a link:

| Caret | Typed text | `active_link_href()` |
|---|---|---|
| inside the link | linked | the link's `href` |
| at the link's start | not linked (the text before decides) | `None` |
| right after its last character | not linked | `None` |
| right after its last character, with a *different* link right after | in the first link | the first link's `href` |

That last row departs from ProseMirror, where the typed text is plain. With
collaboration on, plain text between two links cannot be written to the CRDT
without a formatting marker that brings a link back onto text nobody linked when a
peer removes the second link at the same moment; text that continues the first link
needs no marker at all.

`is_mark_active`, `marks_at` and `Transaction::add_stored_mark` read the same rule.
With collaboration on, the typed character is written to the CRDT outside the link
as well, in a way that leaves the link's own formatting untouched, so a peer's
concurrent change to the link — a new `href`, removing it, extending it over the
text after it — survives. What such a concurrent change *can* do is take the typed
character with it: when the peer re-writes the link or links the text after it at
the same moment, the character may end up inside the peer's link (#923). Both editors
still end up with the same document.

## Commands, keymap, input rules

A **command** queries the state and, if it applies, builds a transaction and
dispatches it — returning `true`. Called for applicability only, it reports whether
it *would* apply (driving toolbar enabled/disabled state). Toolbar queries read
**state**, never the DOM:

```rust
editor.command("toggleBold");          // dispatch by name
editor.can_run("liftListItem");        // would it apply? (enablement)
editor.is_mark_active("bold");         // toolbar "on" state
editor.current_block_type();           // e.g. Some("heading")
editor.in_node_type("bullet_list");    // ancestor-aware (lists, blockquote)
```

The built-in command catalogue is listed in the
[Rich-text editing](./contenteditable.md#driving-the-editor-command) guide:
mark toggles, block-type setters, list/blockquote wrapping, indent/outdent, inserts,
the full table command set, and `undo`/`redo`.

The **keymap** is the single source of truth for command keys. Each platform view
translates its native event into a platform-agnostic `KeyBinding` and routes it through
one entry point, `EditorHandle::dispatch_key`, which looks up the aggregated `Keymap` and
runs the bound command (so `Mod-b` → `toggleBold` everywhere). **Letters** resolve by the
*logical* key (winit's layout-mapped `logical_key` on desktop, `event.key()` on web), so
`Mod-b` follows the keycap on Dvorak/AZERTY; **digits and symbols** resolve by the
*physical* key (`KeyCode` / `event.code()`), so `Mod-Shift-8` matches the `8` key
regardless of the shifted glyph. Only keys that can't be pure editor-core commands stay
view-owned: cursor movement (needs laid-out geometry), clipboard (needs the platform
clipboard), and plain text insertion. **Input rules** are regex-driven
transforms — block shortcuts like `## ` → heading, `- ` → bullet list, `[ ] ` → task
list, and inline mark shortcuts like `**bold**` / `==highlight==` / `` `code` `` —
each returning an optional transaction. The view runs `apply_input_rules` inside
`EditorHandle::insert_text` (before the plain insert) on every text-entry path, so a
just-typed character can complete a shortcut and rewrite the text instead of being
inserted verbatim (ProseMirror's `inputRules` plugin). They only fire at a collapsed
cursor; paste and IME preedit don't reach this path (an IME *commit* does).

## History

There is **one** history, implemented as a plugin. It stores **inverted steps** (not
byte positions), groups them by transaction boundary, and **merges consecutive typing
transactions** within a time/affinity window, so a burst of typing undoes as one
step. On undo it pops a group, rebases its inverted steps over any intervening
mappings, applies them as a transaction that doesn't re-enter history, and restores
the recorded selection. `undo` / `redo` are the only history entry points.

## Plugins

History, tables, links, input rules, and (later) collaboration and accessibility are
**all plugins** — none is special-cased in the core. A plugin can contribute schema
nodes/marks, commands, keymap bindings, input rules, per-document state, decorations
(a widget such as the placeholder, or an inline class over a document range — a
spellcheck squiggle, a search highlight), node-views, and a claim on a paste
(`handle_paste`: the transaction to apply instead of the default paste, asked in
plugin order through `EditorState::handle_paste`). An app adds its own with
`EditorHandle::add_plugin`. This is how
features compose without bloating the core.

## The view seam

The core defines an `EditorView` trait and a small request/event vocabulary; it knows
nothing about any renderer. The view that implements it — `RinchDomEditorView` — lives
in **`rinch-editor-view`** and is itself renderer-agnostic: it projects onto any
`rinch-core` `DomDocument`, so the desktop (rinch-dom) and browser (`web_sys`) editors
run the *same* view code. Only the thin input glue and the layout-coupled extras are
per-platform (`rinch` on desktop, `rinch-web` in the browser).

The view, on each transaction:

1. **Diffs** the new document against the descriptor tree it retained from the previous
   one. Because the model is persistent, `Node::same_ref` (an `Rc::ptr_eq`) makes the
   diff cheap — unchanged subtrees are skipped entirely. Siblings unchanged at the start
   and the end of a child list are matched first, so a new or removed block leaves the
   blocks after it on their own host nodes; the stretch between is diffed positionally,
   and a node whose tag or mark set changed is rebuilt rather than patched.
2. **Patches** the host for the changed regions via the standard `DomDocument`
   primitives (`create_element` / `create_text` / `append_child` / `insert_before` /
   `remove` / `set_text` / `set_attribute` / `set_style`), choosing tags from the
   schema-driven serializer. Decorations (placeholder, IME preedit) diff separately, so
   a decoration-only transaction still produces a visible update.
3. **Renders the caret and selection from `state.selection`** (after layout, in the
   second phase), converting the model's char `Pos` to a byte offset for the platform's
   text layout only at this render edge.

Around that, the platform crates own block virtualization (skipping layout for
off-screen blocks while keeping their real height), IME positioning, and — under the
opt-in `a11y` feature — pushing an accessibility tree derived from the state.

## End-to-end edit flow

```text
key / IME / paste / pointer event
  → the view translates it to a command call or a transaction
  → command(state, dispatch): queries state, builds tr, dispatches it
  → new_state = state.apply(tr)         // schema-validated; reject ⇒ no-op
       • steps applied, selection mapped forward
       • plugins fold state (history pushes inverse, decorations remap)
  → view.update(old_state, new_state)   // diff doc → minimal host patches
       • caret/selection rendered from new_state.selection
```

No step in this pipeline reads the host tree for content. That is the whole design.

## Persisting content

`handle.doc()` returns the current document `Node` — the canonical save shape. Enable
the `serde` feature (`rinch-editor-core/serde`, or `serde` on the `rinch` facade) to
serialize it to the recursive, schema-derived wire shape and load it back; unknown
types are rejected on load, never silently dropped.

## Where to go next

- [Rich-text editing](./contenteditable.md) — the practical component + command API.
- `examples/markdown-editor` and `examples/ui-zoo/src/sections/editor.rs` — working
  editors driving the command API.
- [Editor Architecture](../architecture/editor.md) — the crate boundaries, data flow,
  and the invariants each boundary protects.
