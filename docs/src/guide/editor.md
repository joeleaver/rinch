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
  `replace` and paste operate on. `replace` wants the open depths to line up with
  the range; `replace_range` fits a slice that does not (see
  [Fitting a slice](#fitting-a-slice-replace_range)).

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
  `text`, `image{src, alt, title, board, width}`, plus the table nodes.
- **Marks:** `bold`, `italic`, `underline`, `strike`, `code`, `link{href}`,
  `highlight{color?}`, `text_color{color}`, `subscript`, `superscript`. Every mark
  is inclusive except `link` (see [inherited marks](#state-selection-stored-marks)).

An image's `board` and `width` are an app's: the editor keeps them through edits,
copy and paste and collaboration, and only shows them. `board` (an id for something
the app draws over the picture) reaches the `<img>` as `data-board` and HTML as
`data-board`; `width` (whole CSS pixels, absent or not positive for the natural
width; read from HTML by HTML's dimension rules, so `320px` is 320, rounded and at
most 65535) is the `<img>`'s `width` hint and HTML's `width`. A GFM image `![alt](src)`
has neither, so it is written without them; an image in a table written as HTML keeps
both. Like every attr of an inline atom they merge per attribute when collaborating:
two peers changing different attrs of one image at once both keep their change, and a
peer's `board` follows its picture through Enter, Backspace, drags and pictures
inserted beside it, and never shows on another picture (one pasted over it with the
same `src` counts as the same picture: identity is type and `src`). A change of an
image's `src` (or a picture with another `src` pasted over it, which the editor cannot
tell from one) makes a new picture, and a concurrent change of the old one is dropped;
so does cutting and pasting a picture, undoing its delete, or dragging it after an
in-editor copy of it. Chosen by Joe (2026-10-09):
a wrong attribution is worse than a lost one — board markup would show on the wrong
picture (`rinch-editor-collab/tests/image_attrs.rs`, `atom_attr_merge.rs`,
`atom_identity_differential.rs`).

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

**This used to reach `EditorHandle` too, and no longer does (#440).** `create_editor`
mints a new `Rc<Schema>` per handle, so `b.load_doc(a.doc())` hands `b` a document built
by `a`'s schema — but `load_doc` (and therefore `load_html`, and the collaboration guest
join) now re-homes the incoming document onto the receiving schema first
(`Node::rebind`), matching every node and mark type by *name* against `b`'s own interned
handles before installing it. `b.is_mark_active("bold")` then answers `true` over the
adopted bold text, and `toggleBold` removes the mark rather than returning `false` (the
pre-#440 guard's refusal) or — on an unfixed build older than #217's guard — duplicating
it. `b.load_doc(a.doc())` is the documented way to move a document between handles; no
manual serialization round-trip is needed for it any more. `Node::rebind` costs one
pointer comparison per node/mark when the document is already on the target schema (the
guest-join case: `projected_doc` already builds through the guest's own schema), and a
rebuild only for a genuinely foreign type.

### Serialization

The durable save/load shape is a recursive, schema-derived structure (under the
`serde` feature). Serialize walks the schema, so every node type and mark type has a
name and there is no string-tag fallthrough; deserialize consults the schema and
**rejects** unknown types rather than dropping them silently. HTML serialization
derives tags from the schema's `parse_html_tags`, so copy-out and paste-in share one
table — the same one `Editor`'s `content:` prop and `load_html` use.

The HTML import is **total about text** (#1397): `slice_from_html` does not fail on
markup, the text of every element it reads is in the slice it returns (it reads no
CSS, and drops a fixed set of elements whole: scripts, styles, embedded content,
`<select>`, `<svg>`, `<math>`), and the slice's content is valid. It is two passes. The tree builder
(`serialize/html_tree.rs`) follows the HTML parsing rules as far as they decide what
is text and what holds what: a tag name runs to the next whitespace, `/` or `>` (so
`<o:p>` is an element — the old tokenizer stopped reading at the colon); comments,
`<![if …]>`, `<style>`, `<script>`, `<title>` and `<head>` give nothing; a `<p>`,
`<li>`, `<td>` or `<tr>` ends where a browser ends it; an end tag closes the nearest
open element of its name unless a table cell (or, for an inline element, a block) is
in between; a formatting element's end tag (`</b>`, `</i>`, `</a>`, …) across an open
block ends the element there and leaves what the block already holds formatted
(#1410: the adoption agency algorithm, done by turning the element's stack entry
into a placeholder and copying the element around what each element open above it
holds, so the tree gets no deeper; skipped with more than 7 blocks or 32 elements
open in between); a `</p>` with no `<p>` to close ends the line; an unclosed `<svg>`
or `<math>` ends at the first HTML element. Character references are the HTML
standard's 2,231 (#1415; `html_entities_table.rs` is generated by
`tools/gen_html_entities.py` from the standard's `entities.json`). The tree
is at most 192 elements deep (`html_tree::MAX_DEPTH`), so the reader's recursion is
bounded (10,000 nested `<div>`s overflowed the stack): an element opened deeper is
still open, so its end tag is its own, but it is flattened into the element at the
limit — a block as a child of it, split around the blocks inside it, an inline
element as its content alone — so past the limit no text is lost and every block
is still a line of its own. Chrome's limit is 512 and it too keeps every element; 192 is what an
unoptimized build reads on a 2 MB stack (about 6.4 KB a level). A tag scans the
innermost 192 open elements for the one it closes and past those asks an index of
where each name is open (`open_at`), so it closes what an unbounded scan would at a
fixed cost: reading stays linear with thousands of elements open, and a block's end
tag ends it however many unclosed elements are inside. The reader (`serialize/html.rs`) then maps the tree onto the schema:

- An element with no node or mark is read through. It is inline unless it holds a
  block or is a block-level HTML element (`div`, `section`, …).
- A block-level element that holds no block ends the line before it (#1413) and is
  a line itself when the text it holds directly is more than ASCII whitespace (a
  non-breaking space); empty, or holding only spaces and line ends directly, it is
  none. (A space inside an inline element in it still makes a paragraph:
  `<div><span> </span></div>`.) An inline element that
  holds a block ends none: its children are read where it stands, so the inline
  content beside the block stays on the line of what is beside the element.
- A run of table parts with no `<table>` is a table; what has no place in a table
  (text in a row, a `<caption>`) is read as blocks in front of it, as a browser's
  foster parenting puts it.
- A textblock element (a heading) that holds blocks is not flattened: its blocks stay
  blocks and the inline content between them takes the element's type.
- A `<div style="white-space: pre">` is a code block, one line per `<div>` or `<br>`
  (what VS Code copies); so is `<pre>`, where a `<br>` is a line end too.
- A mark element around blocks marks what the same element marks around inline
  content: text and images, not hard breaks (#1401). Mark elements nested directly
  in one another mark the blocks once for each mark type; with a block-level
  element between them it is once a level, as before. Neighbouring text with the same
  marks is one text node, and a mark of a type replaces an outer mark of that type.

`tests/html_reader_total_1397.rs` is the property test: a tag-soup generator that
knows what a browser shows, checked for text conservation, validity and reading back
the same, plus a step-count pin that reading is linear;
`tests/html_reader_clipboard_samples.rs` holds what Word, Excel, Sheets, Docs, Notion,
GitHub, VS Code, Apple Notes and Mail, Slack and Gmail write.

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

A loaded document keeps no `colspan` past 1000 either (#1214, Chrome's limit):
`tables::cap_colspans` cuts every larger one to `tables::MAX_COLSPAN`, and
`Schema::node_from_doc`, `EditorHandle::new` and `EditorHandle::load_doc` call it, as
the HTML import (`load_html`, paste, `Editor`'s `content:`) already reads `colspan`.
One `colspan = 3,000,000` cell made a two-row table 2^21 columns wide, and
`addRowAfter` built a cell per column: 2,097,152 cells, 15.6 s and 5 GB through a
mounted view, 3.8 ms after the cap. Edits are not capped — an app's own transaction,
a peer's change (and the shared document a collaboration guest adopts on joining), or
a command (`addColumnAfter` across a 1000-wide cell makes it 1001) — so `node_from_doc` is not a lossless inverse of `to_doc` for a cell that went
past 1000 that way. `rowspan` is not capped: a grid is never taller than its rows. The
row-width limit above is the HTML import's alone: a loaded row of 2,100
`colspan = 1000` cells is still 2,100,000 columns wide.

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
never the attribute's own value ± 1: a `colspan` of `i64::MAX` in a two-row table
(which an app's own transaction can still write; a load caps it at 1000),
which the grid cuts to 2^21 columns, is 2^21 + 1 after `addColumnAfter` across it.

The view lays a table out as that same grid (#1182): `<table>` is a CSS grid of
`tables::column_count` columns and each cell is placed by its rectangle in the map
(`tables::cell_rects`), not by its raw attributes — so a `rowspan` of `i64::MAX` in a
two-row table spans two grid rows. The rectangle is written as definite grid lines,
`grid-column: <left + 1> / <right + 1>` and `grid-row: <top + 1> / <bottom + 1>`
(#1209): CSS auto-placement knows no rows and packed a cell of a row that a rowspan
leaves short into the row before it, where the map has it below. A cell the map has
no slot for is laid out as a band across the whole grid (`grid-column: 1 / -1`) on a
row of its own (the first the grid leaves empty, normally after its last),
so its text stays visible and adds no column.
Written raw, a few cells of huge spans stacked past the 32767 grid lines Taffy 0.12
numbered a grid with, and desktop layout panicked; Taffy 0.14 (#1236) clamps a grid
axis at 10000 tracks instead and overlaps whatever lands past them. Two limits remain, both from the desktop's
Stylo clamping a grid line, a template and every span to 10000: a table wider than
that is drawn 10000 columns wide; and an axis past 9999 tracks, whose lines would be
clamped onto one, is placed by `span <n>` and auto-placement instead. A table past
9999 rows keeps its column lines, so a cell is still placed below the one before it
in its column. On the desktop such a table still meets Taffy's own clamp:
a table of more than 10000 rows overlaps its rows from the 10000th on (they lay out
in the last grid track), where Taffy 0.12 laid out up to 32767 lines correctly. A table past 9999 columns loses its row lines too, because cells
locked to their row with auto-placed columns can grow the grid past what Taffy
numbers (on Taffy 0.12 a row of four `colspan = 20000` cells panicked; 0.14 clamps
the axis at 10000 tracks); there a short row's cells are still lifted into the row
before it, on the web as well as the desktop.

### Markdown

Under the `markdown` feature, `serialize::doc_to_markdown(&doc)` writes a document
as Markdown and `doc_from_markdown(&schema, md)` / `doc_from_markdown_strict(&schema,
md)` read it back. Markdown is for people and language models to read and write; the
durable format is still the `DocNode` shape above.

The dialect is CommonMark with GFM strikethrough and pipe tables (read by
pulldown-cmark), plus a small, exact set of inline HTML for what Markdown has no
syntax for:

| model | written as | also read |
| --- | --- | --- |
| `bold`, `italic`, `strike` | `**…**`, `*…*`, `~~…~~`; `<strong>`, `<em>`, `<s>` where a delimiter would not flank | `<b>`, `<i>`, `<del>` |
| `underline` | `<u>…</u>` | |
| `highlight` | `<mark>…</mark>`, `<mark style="background-color:C">…</mark>` | |
| `text_color` | `<span style="color:C">…</span>` | |
| `subscript`, `superscript` | `<sub>…</sub>`, `<sup>…</sup>` | |
| `link`, `image`, `code`, `hard_break` | Markdown syntax; a hard break is `<br>` at a textblock's end and in a heading | `<br>`, `<br/>` |
| `table` | a GFM pipe table when it has one header row, no merged cells and one inline paragraph per cell; otherwise an HTML `<table>` block | |
| `task_list` > `task_item` | GFM's `- [ ] …` / `- [x] …`; in a table written as HTML, `<ul data-type="taskList">` > `<li data-type="taskItem" data-checked="true">` | `[X]` |

`C` is a colour `is_safe_css_color` accepts (the HTML paste path's check). The writer
writes a colour only when it passes: a `highlight` with any other colour is written as
a bare `<mark>`, and a `text_color` with one is not written at all, so a colour that
arrived unchecked (a `DocNode`, a collaboration peer) never reaches the output. HTML
copy-out (`node_to_html`) applies the same check, so it no longer writes a colour like
`var(--x)` or `oklch(…)`. Tag
names are case-insensitive; the `style` attribute must be lowercase and quoted (either
quote) and hold that one declaration.

**Task lists** (#1365). An item's marker starts its first paragraph, and the item's
other blocks are indented under it like any list item's. An item that starts with
another block (a heading, a quote, a list, …) is written with the marker alone on its
line, `- [ ] ` with a trailing space, and the block on the next line: written after the
marker, pulldown-cmark misreads a quote's or a nested list's later lines. A rule in that
position is written `***`, because `---` under the marker's line is a setext heading to
GitHub. An empty item is the marker and its trailing space; GFM itself has no empty task
item, and GitHub shows one as the text `[ ]`. A bullet or task list that directly
follows another is written with `*` instead of `-` (and back), because CommonMark
continues a list across blank lines when the bullet is the same, which would turn a
bullet list and the task list after it into one list.

On reading, a bullet list whose items all start with a marker is a `task_list`. The
reader finds a marker in the item's source rather than trusting pulldown-cmark's event
for it, which arrives inside whatever paragraph comes first or, before a heading (`- [ ]
# Heading`), not at all. A marker with nothing after it on its line (`- [ ]`, the
writer's empty item once an editor or an LLM strips trailing whitespace) is text to
pulldown-cmark; both readers take it as an empty item's marker when it is the item's
whole first paragraph. A bullet item whose text is `[ ]` is written `- \[ \]`, so it
stays a bullet. A marker on an item of an **ordered** list, or of a list where some
items have none, has no place in the model: the strict reader refuses it
(`Construct::TaskList`) and the lenient one gives it back as text at the start of the
item (`[ ] todo`; before another block, a paragraph of its own).

HTML copy-out (`node_to_html`, `slice_to_html`) writes a task list with the same
`data-type` / `data-checked` markup (TipTap's), and `slice_from_html` parses it, a run
of bare `<li data-type="taskItem">` (a selection inside one list) included, as a
`task_list`. A pasted task list is fitted like any list
([Fitting a slice](#fitting-a-slice-replace_range)): on an empty line or an empty item
it keeps every checkbox; inside text its first item's text continues the line (that
item's checkbox goes with the item) and the rest stays a task list. In a collaborating
editor a pasted task list stalls outbound like any other (A22).

**What round-trips.** A document of the starter kit's marks and nodes written with
`doc_to_markdown` reads back with `doc_from_markdown_strict` as the same document, and
writing it again changes nothing — except for the losses below.
`tests/markdown_round_trip_fuzz.rs` holds the writer to that over seeded random
documents. Text is escaped so it reads back as text: Markdown punctuation, a block
marker at a line's start, `<` (a tag or an autolink), an entity-shaped `&`, a trailing
`#` run in a heading, and a line break inside text (written `&#10;`).

**Strict reading.** `doc_from_markdown` is lenient: what it cannot represent (other raw
HTML, unsafe URLs, footnotes, a task-list marker in an ordered or mixed list) it drops
or keeps as text. `doc_from_markdown_strict` parses the same way but fails with
`MarkdownError::Unsupported { construct, line, source }` on the first construct the
lenient read would drop or degrade, naming it with a `Construct` (`InlineHtml`,
`HtmlBlock`, `UnmatchedTag`, `Footnote`, `TaskList`, `UnsafeLink`, `UnsafeImage`,
`UnsupportedMark`, `Other`; the enum is `#[non_exhaustive]`) and its 1-based line.
`MarkdownError::Invalid` carries a schema validation error. Strict accepts everything
the writer writes (`strict_reads_everything_the_writer_writes` in the fuzz holds it to
that, losses included): a textblock of only hard breaks, written `<br>` alone on a line
— which CommonMark reads as an HTML block — reads back as one. Inside an HTML `<table>`
block strict refuses another tag, stray text, an unsafe `href` or `src` (`UnsafeLink` /
`UnsafeImage`), and any attribute the import does not keep — a `style` other than a safe
colour or `text-align`, a `colspan` above 1000, a `class`, an event handler, a
`data-type` other than `taskList` on a `<ul>`, a `data-checked` other than
`true`/`false` or outside a task list (`HtmlBlock`). What strict accepts, it parses
exactly as the lenient reader does.

**Known losses.**
- Whitespace at the start or end of a textblock or line is stripped (CommonMark;
  not in a table written as HTML, where it round-trips), and
  whitespace at the edge of a bold, italic, strike or link run is written outside the
  run: the text round-trips and that whitespace leaves the mark.
- A code block's language is lost in a table written as HTML; two adjacent ordered
  lists, or two adjacent blockquotes, merge; empty paragraphs are dropped; a link `href`
  containing `\` before punctuation or an entity is decoded on read (#1366).
- A line break inside inline code becomes a space (a code span is literal), and one
  inside a link `href` reads back as text.
- An ordered list numbered past nine digits is not a list to CommonMark.
- A `colspan` above 1000 or a `rowspan` above 65534 is written clamped to those limits,
  which the HTML import applies anyway.
- A paragraph's or heading's `indent` is not written, and a link's `target` and a
  textblock's `text_align` only in a table cell.

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
| Paste | parse to a `Slice`, then `replace_range(from, to, slice)`: one `ReplaceStep`, or a `ReplaceAroundStep` when the text after the caret moves into the pasted content |

### Fitting a slice: `replace_range`

`ReplaceStep` is strict: the slice's open depths must line up with the range, and
every node it rebuilds must be valid. Content that comes from somewhere else rarely
lines up — a list has no place inside the paragraph the caret is in — so paste goes
through `Transform::replace_range(from, to, slice)` (`Transaction::replace_range`, and
`Transaction::replace_selection(slice)`, which also places the caret). It is
ProseMirror's `replaceRange` and `Fitter` (`transform/fit.rs`), and it adds **one**
step or fails with nothing changed.

The rules, with `|` the caret and a pasted `<ul><li>A</li><li>B</li></ul>`:

| Where | Result | Why |
|-------|--------|-----|
| `<p>a|bc</p>` | `<p>aA</p><ul><li>Bbc</li></ul>` | Content open at the slice's start continues the textblock; the rest keeps its structure; the text after the caret joins the textblock the slice ends in |
| `<p>|</p>` (an empty line) | the list | The range covers a whole textblock and the slice starts in a *defining* node (`list_item`), so that node is kept and replaces the textblock |
| `<ul><li>x|y</li></ul>` | `<ul><li>xA</li><li>By</li></ul>` | Open nodes join the nodes of compatible content around the caret: items become sibling items, and the target list keeps its kind |
| an empty item | the pasted items, in its place | as the empty line |
| `<p>a|bc</p>`, pasting `<hr>` or a table | `<p>a</p><hr><p>bc</p>` | A closed block closes the textblock and goes in beside it; at a textblock's edge no empty block is left |
| a table cell | as above, inside the cell | A fit never leaves the isolating node the range starts in |
| `<ul><li>x|y</li></ul>`, pasting a **task** list | `<ul><li>xA<ul data-type="taskList"><li>By</li></ul></li></ul>` | `task_item` and `list_item` are different nodes: nothing joins, so the rest of the list goes in the item. ProseMirror nests there too |

`NodeSpec::defining` (ProseMirror's) marks the nodes that are kept on an empty line:
`list_item`, `task_item`, `heading`, `blockquote` and `code_block` in the starter kit.
A pasted `<h2>` on an empty line is therefore a heading (it was a paragraph with the
heading's text before #1382), and in the middle of a paragraph it is its text.

`slice_from_html` decides the open depths, since markup has none: each edge is open
down to the textblock there, and closed when it reaches none (a rule, a table). So a
list is open through its first and last items, as a selection inside it is.

What it does not do, where ProseMirror does:

- It never **creates** a node to make content valid (`fillBefore`) and never **wraps**
  content in a node it was not in (`findWrapping`): the content matcher cannot be
  asked for either. A fit that would need one fails, and a paste then falls back to
  its plain text.
- A range whose two ends are in different isolating nodes (two table cells) is not
  fitted: only the plain `replace` is tried, as before.
- A **cell selection** is not a range: its `from()..to()` are the positions before
  its two corner cells. `Transaction::replace_selection` refuses one. The paste
  (`EditorHandle`) clears the selected cells (`commands::table_ops::clear_cells`) and
  fits the content in the top-left cell, in one transaction — what ProseMirror's
  `CellSelection.replace` does with content that is not cells. `replace_range` itself
  takes positions and will fit whatever range it is given.
- A slice holding an invalid node (the nodes open at its edges aside) is refused.
  The HTML reader never makes one: a `<td>` or `<tr>` with no table around it is read
  as a table (#1392).

The fitter keeps, per open node, how far its children have got through the content
expression (`ContentMatch::start` / `advance` / `accepts_end`) and steps it once per
node placed, so a paste costs in proportion to its size
(`a_fit_is_linear_in_what_it_places` counts the steps at n and 2n).

`tests/replace_range_fuzz.rs` pastes random slices over random ranges of random
documents: no panic, no invalid document, every step undoes exactly, and the content
the HTML reader makes of the fuzz's markup (every starter-kit block, nested) always
fits at a caret or a selection inside one textblock.

**A delete is fitted the same way.** `Transform::delete(from, to)` is the plain
`replace` when the range's ends are at the same depth. When they are not (from a list
item into the item nested under it, from a paragraph into a list), it fits the empty
slice: what is left of the textblock the range ends in joins the one it starts in, as
ProseMirror's `Transform.delete` does, in one step. `deleteSelection`, Backspace and
Delete over a selection, Enter, `insertHorizontalRule`, the word deletes and typing
over a selection (`EditorHandle::insert_text`, which deletes the selection and then
inserts; `Transaction::insert_text` alone is the plain replace) all go through it, and leave a caret where the range began. Still
refused: a range that leaves or enters a table cell (an isolating node), a range that
holds only the boundary between two textblocks that cannot join (the end of a code
block and the start of marked text), and `insertHardBreak` or an image insert over a
selection whose ends are at different depths (#1460). A selected list item is emptied,
not removed (#1460). `tests/review_1422.rs` checks every range of random nested
documents against those rules.

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
