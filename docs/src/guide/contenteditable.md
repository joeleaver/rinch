# Rich-text editing

Rinch ships a built-in rich-text editor — a ProseMirror-style, model-first editor
with schema-enforced structure, inline marks, lists, tables, and exact undo/redo.
You add it with the `Editor {}` component and drive it through an `EditorHandle`.

> **This page is the practical "how do I add an editor" guide.** For the deeper
> model — the document tree, steps and transactions, the schema, commands, and the
> view seam — see the [Rich-Text Editor](./editor.md) guide.

> **Desktop and web.** On desktop the editor arrives with the `desktop` cargo feature;
> in the browser `rinch-web` re-exports it (see [On the web](#on-the-web-rinch-web)
> below) — the app code is identical, because both share the renderer-agnostic view in
> `rinch-editor-view`. There is no `contenteditable` HTML attribute and no DOM-level
> editing API to wire up — the `Editor` component is the whole surface.

## Quick start

Create a handle with `create_editor()`, mount it with `Editor {}`, and dispatch
named commands from toolbar buttons. Everything you need is in the prelude:

```rust
use rinch::prelude::*;

#[component]
fn my_editor() -> NodeHandle {
    let editor = create_editor();

    // One cheap handle clone per closure that needs it (EditorHandle is an Rc).
    let ed_bold = editor.clone();
    let ed_italic = editor.clone();
    let ed_h1 = editor.clone();

    rsx! {
        div {
            // Toolbar — each button runs a named command. `data-nofocus`
            // keeps the keyboard (and the selection) in the editor when a
            // button is pressed; without it the button takes the focus.
            div {
                data-nofocus: "",
                button { onclick: move || { ed_bold.command("toggleBold"); }, "Bold" }
                button { onclick: move || { ed_italic.command("toggleItalic"); }, "Italic" }
                button { onclick: move || { ed_h1.command("setHeading1"); }, "H1" }
            }
            // The editing surface — mounts the handle and renders its content.
            Editor {
                editor: editor.clone(),
                content: "<h1>Hello</h1><p>Type here…</p>",
            }
        }
    }
}
```

The `Editor` component renders the document, the caret, and the selection straight
from the editor's state. It ships its **own default light/dark stylesheet**, so the
content looks right out of the box — you don't hand-roll editor CSS.

## The `Editor` component

| Prop | Type | Purpose |
|------|------|---------|
| `editor` | `Option<EditorHandle>` | A handle from `create_editor()`. Omit it and the component creates its own self-contained editor. |
| `content` | `String` | Initial content as schema-whitelisted HTML, parsed into the document once on mount. |
| `read_only` | `bool` | Mount the editor [read-only](#read-only). `false` (the default) leaves the handle's switch as it is, so a handle locked before mount stays locked. Change it afterwards with `handle.set_read_only(..)`. |

Like every rinch component, `Editor {}` also accepts the universal `style:` and
`class:` props, applied to its host element. Its **children** are an overlay slot that
scrolls with the text — see [Margin anchors](#margin-anchors-drawing-beside-a-line).

```rust
// Self-contained: no handle needed if you don't drive it programmatically.
rsx! { Editor { content: "<p>Just some editable text.</p>" } }
```

## The `EditorHandle`

`create_editor()` returns an `EditorHandle` — a cheap, cloneable (`Rc`) handle to a
single editor. Clone one per closure that captures it; all clones share the same
editor. It works **before** the editor is mounted or focused: state edits mutate the
owned state and render when the view attaches.

### Driving the editor: `command`

`handle.command(name)` dispatches a built-in command by name and returns whether it
applied. This is the single entry point for toolbar buttons and menu items:

```rust
let ed = editor.clone();
button { data-nofocus: "", onclick: move || { ed.command("toggleBold"); }, "Bold" }
```

A toolbar button needs `data-nofocus` (on itself or on the toolbar around it):
a press on a `<button>` focuses it, on desktop and on the web alike, and the
editor it acts on loses the keyboard.

Command names are case-sensitive. The full catalogue:

| Category | Commands |
|----------|----------|
| **Inline marks** | `toggleBold`, `toggleItalic`, `toggleUnderline`, `toggleStrike`, `toggleCode`, `toggleHighlight`, `toggleSubscript`, `toggleSuperscript` |
| **Block types** | `setParagraph`, `setHeading1`…`setHeading6`, `setCodeBlock` |
| **Alignment** | `setTextAlignLeft`, `setTextAlignCenter`, `setTextAlignRight`, `setTextAlignJustify` |
| **Containers** | `toggleBulletList`, `toggleOrderedList`, `toggleTaskList`, `wrapInBlockquote` |
| **Lists** | `sinkListItem` (indent), `liftListItem` (outdent) |
| **Inserts** | `insertHorizontalRule`, `insertHardBreak`, `insertTable` |
| **Tables** | `addRowAfter`, `addRowBefore`, `addColumnAfter`, `addColumnBefore`, `deleteRow`, `deleteColumn`, `deleteTable`, `mergeCells`, `splitCell` |
| **Links** | `removeLink` |
| **Deleting** | `deleteSelection`, `deleteCharBackward`, `deleteCharForward`, `deleteWordBackward`, `deleteWordForward` |
| **History** | `undo`, `redo` |

`addRowBefore` and `addRowAfter` shape the new row like the row beside it (the row
below for `addRowBefore`, the row above for `addRowAfter`): under a cell two columns
wide the new row has a cell two columns wide, and a cell that spans across the new
row's line (a `rowspan` reaching past it) grows by one row and keeps those columns.
The new cells are empty, plain `table_cell`s, even beside a header row. In a table
with no merged cells that is one cell per column, the row these commands have always
added. Before #1244 a new row was always one cell per grid column, so a row added
under a row of 2,100 cells each `colspan = 1000` wide built 2 million cells; it now
builds about 2,100. Where the row beside it has no cell (a short row in a ragged
table), the new row gets a cell per empty column, unless there are more than 1000 such
columns and more of them than the whole table has cells; then it gets cells up to 1000
columns wide.

**Selections in a table are held as cell selections** (#1847), as prosemirror-tables
holds them. A selection that lands (`set_selection`, a drag, a Shift+click, a peer's
edit mapping it) is normalized first: a node selection of a cell, or of a row, becomes
a cell selection of it; a text range whose ends are in two cells of one table becomes
the cell selection of the rectangle between them, on the web as on the desktop; an end
between cells or rows belongs to the cell after it (the one before it, at a row's
end), so a Shift+click from a cell selection keeps its anchor cell; and a range that
only crosses the boundary between two cells becomes a caret at its start. Two
selections are left as they are: a node selection of a whole table (Backspace after a
table selects it, and the next Backspace deletes it), and the whole-document selection
`selectAll` makes (ProseMirror's `AllSelection`, never a cell selection).
`selection()` reads back the normalized selection, and copying a cell selection copies
the selected cells, whole, as a table.

**Shift+arrow at a cell's edge selects cells**, as prosemirror-tables' `shiftArrow`
does, in front of the normalization: at the end of a cell's last block (Shift+Right,
or Ctrl+Shift+Right, whose word motion stops there) or the start of its first
(Shift+Left), or on its last or first line (Shift+Down / Shift+Up), the key selects
the cell and its neighbour; over a cell selection every Shift+arrow moves the head
cell, and stops at the table's edge. In any other block of the cell (a paragraph above
a nested table, say) the selection extends as text, and a nested table counts as inside
its cell. Past a row's end Shift+Right reaches the next row's first cell; out of the
table (the last cell's end, the bottom row's last line) the selection extends as text,
and at the document's start or end the key does nothing. Up or Down into a table from
outside it lands in the cell under the caret, so Shift+Down out of the bottom row and
Shift+Up back is a text selection again.
`EditorHandle::move_cursor` does this for the horizontal motions;
`EditorHandle::shift_arrow_cells` is the hook a platform's Shift+Up/Down calls with its
own vertical step, as the desktop and web input paths do.

Over a cell selection, the toolbar commands apply to every selected cell: marks
(`toggleBold` …, links), block types (`setHeading2` …, `setParagraph`) and alignment
walk `Selection::ranges`, one content range per cell. Backspace, Delete, typing and
paste clear the cells first, and so do Shift+Enter (`insertHardBreak`),
`insertHorizontalRule`, `insertTable` and `insert_image`, which then put their node in
the first cell. Enter (`enter`, `splitBlock`) does nothing over a cell selection.

A text range with its ends in two different tables, or with one end outside every
table, has no cell selection to stand for it. An edit over it that takes cells out of
a row, or rows out from under a `rowspan`, has the table put back in the same
transaction (one undo step): a span past the last row or into another cell is cut, and
each short row gets an empty cell for each missing column (one cell spanning each gap,
when that would be an unreasonable number of cells: a pasted `colspan=1000` cell over
bare rows), until the table tiles. A pasted table whose rows are ragged is
padded the same way, at a caret as over a range. Only a table the edit made ragged, or
pasted, is repaired; a table that was already ragged (loaded that way from HTML) is
left as it is. An edit inside one cell is not checked against the tables around it, so
it costs nothing in their size. A range from text inside a cell to text outside the
table (a drag out of the table) is refused by Backspace, typing and paste, as before
(#1930). The repair runs in the `Transaction` methods that replace the selection
(`delete_selection`, `replace_selection`, `insert_text`, `replace_selection_with`,
`insert_block`), which every built-in command and the handle's typing and paste use.
**A transaction an app builds from the position methods (`delete`, `replace`,
`replace_with`, `step`, ...) is not repaired**: give it whole cells or rows, or it can
leave a ragged table.

A block-type command clears what the new type cannot hold, as ProseMirror's `setBlockType` does: `setCodeBlock` removes the marks (a code block takes none), turns a hard break into a line break (`\n`) and deletes an image.

> Links need a destination, so applying a link is a builder rather than a bare named
> command. Use `handle.toggle_link(href)` to add (or, over an existing link, remove)
> a link, `handle.command("removeLink")` to clear one unconditionally, and
> `handle.active_link_href()` to read the current link's target for an edit dialog.
> Text typed right after a link is not part of it (`link` is a non-inclusive mark),
> so a link does not grow as the user keeps typing past it. The one exception is a
> link that runs straight into a *different* link: text typed at that seam continues
> the first one (see [inherited marks](./editor.md#state-selection-stored-marks)).

> Arrows, Delete and Backspace step over a **grapheme cluster**, as a browser does:
> an emoji ZWJ sequence (`👨‍👩‍👧`), a flag, a keycap, `❤️` or a skin-toned emoji is one
> step, and a click never puts the caret inside one. Backspace follows Chrome: it
> deletes an emoji sequence whole but takes one code point off any other cluster, so
> after `e` + a combining accent it removes only the accent (Delete removes both).
> The same rule holds in desktop `<input>`/`<textarea>` fields.

> The word deletes remove from the caret to the boundary the word motion
> (Ctrl+Left / Ctrl+Right) lands on, taking the whitespace between: in `hello wor|ld`
> backward leaves `hello |ld`. A mark boundary is not a word boundary. Where there is
> no word to take they are Backspace / Delete: a non-empty selection is deleted as it
> stands, and at a textblock's start / end they join (or lift), as Backspace and
> Delete do there. The word motion, the word deletes and double-click share one
> word segmentation (`motion::word_segments`, #938, #939), Chrome's where it was
> measured: a run of punctuation is a word apart from the letters beside it
> (`foo.bar|` deletes `bar`, `foo...|` deletes `...`), an apostrophe between letters
> stays in the word (`don't`), a `.` `,` `;` or apostrophe between decimal digits stays in
> the number (`3.14`, `1,000`), and an inline atom is a word of its own — adjacent
> images together, each hard break alone: the motion stops on either side of it,
> one beside the caret goes alone (with any whitespace between them:
> `foo<img>  |` leaves `foo`), and a word beyond one is not taken with it.
> Double-click on a run of punctuation (not emoji) selects the one character, as Chrome
> does. A CJK or Thai run is split into words by ICU's
> dictionary on desktop (`漢字です|` deletes `です`). A web build does the same with
> `rinch-web`'s opt-in `word-dictionary` feature (+3.8 MB of wasm, see
> [Binary Size](./wasm.md#word-segmentation-for-cjk-and-thai-word-dictionary));
> without it such a run is one word, ended by whitespace, punctuation or a change
> of script. Desktop text fields (`<input>`, `<textarea>`) take their words from the same
> segmentation (the `rinch-words` crate, #1619), so a field's Ctrl+Backspace after
> `foo-bar` leaves `foo-` as the editor's does. There are no *line* delete commands. On the web the browser's
> `deleteSoftLine*` (a soft keyboard's line delete, Cmd+Backspace / Cmd+Delete)
> deletes to the edge of the **visual** line the caret is drawn on — on a wrapped
> paragraph, normally only the current line's prefix or rest — and
> `deleteHardLine*` to the textblock's edge (issue #301). Home / End go to the
> visual line's edges on both backends. The web finds the edge from caret
> geometry: the first and last positions whose caret is drawn on the caret's own
> line, which are the line's *logical* edges — a right-to-left line starts at its
> right edge. Desktop reads the line from the textblock's text layout — the line
> holding the caret, the upper one for a caret drawn at the end of a
> soft-wrapped line — and takes its logical start and end: after a hanging space
> at a soft wrap, before a hard break (Shift+Enter).
> ArrowUp / ArrowDown inside a wrapped paragraph read the line above or below
> from the same layout on desktop, at the column the caret had, so they move
> within the paragraph even when its lines are scrolled out of view, and the
> caret is then scrolled back into view, as in Chrome (#1121).
>
> **Caret affinity at a soft wrap.** The end of one visual line and the start of
> the next are one model position, so a caret there could be drawn in either
> place. The editor keeps a hint beside the selection, as CodeMirror and a
> browser do: End (landing on the wrap point itself, after any hanging space), a
> press past a wrapped line's end, and an Up / Down move whose column lies past
> the target line's end draw it at the end of the upper line; Home and anything
> else draw it at the start of the lower one. So Home twice stays on its line,
> Home then End reaches that line's end, End on a word broken by `overflow-wrap`
> lands after the line's last letter (typing appends to the upper line), and a
> soft-line delete backward right after Home deletes one character, as Chrome
> does. The hint is view state, not part of the `Selection`: it applies only
> while the selection is the one it came with, so typing, an edit before the caret
> in its own paragraph, undo, a load or a plain `set_selection` returns the caret
> to the lower line's start. An edit that leaves the caret's paragraph unchanged
> up to the caret — in an earlier paragraph, a collaborator typing above or after
> the caret — carries the hint along, so the caret stays at the end of its line.
> A caret right before a hard break (Shift+Enter) is always drawn at the end of
> its own line. A paragraph that *ends* in a hard break shows the empty line
> after it, where the caret after the break sits: a browser lays `<p>a<br></p>`
> out as one line (a line after the last forced break needs something after
> it), and so does rinch-dom, so the view renders a second `<br
> data-pm-trailing-break>` after the break, as ProseMirror does — and after a
> code block's text that ends in a newline, for the same reason. It is not part
> of the document and never reaches `doc()`, the HTML or the markdown. Set it yourself with `EditorHandle::set_selection_with_affinity`;
> read it with `caret_affinity()`, and `caret_rect` at the head draws with it.
>
> **Spaces hanging past a line's end.** Spaces typed at the end of a line in a
> `pre-wrap` textblock (every editor block) hang past its edge rather than
> wrapping, as CSS says. The caret after them is drawn at the textblock's
> border-box edge, its whole width inside the block, as Chrome draws its own
> (issue #1041), whatever the textblock's `overflow`. Only the drawn caret is
> kept in: the selection
> highlight over the spaces, `caret_rect` and the vertical motions keep the
> spaces' own x.
>
> Neither backend hit-tests points on the line (the web uses collapsed `Range`
> rects, desktop the text layout), so a caret line scrolled out of the editor's
> scroller or off the page behaves as a visible one (#1026, #1107), and a hard
> break (Shift+Enter) always ends the line: Home after it lands after it, End
> before it stops before it.
>
> An image at a line's edge is on that line: Home on a line that starts with
> one lands before it, End on a line that ends with one after it, and at a soft
> wrap between two images End on the upper line lands after the first and Home
> on the lower line before the second, as in Chrome 155 (#1116); Home and End
> pressed again stay there. The caret beside an image is drawn on its side, and
> a drag or Up/Down onto an image lands on the half it reaches (#1104). The
> selection highlight covers an image exactly when the selection holds it, on
> both platforms: a selection from right after an image starts at its right
> edge, one ending right after it takes it in, and an image selected alone
> (Shift+Right from before it) is highlighted over its box (#1120); a hard
> break is resolved on both sides on both platforms (#1099, #1025), and the web
> resolves both sides of an image too — a click, Home, a soft-line delete, and
> an IME commit or autocorrect right after one.
>
> Where this still differs from a browser field:
> - **Desktop, an image on a line holding right-to-left text**: desktop reads
>   which side of an image is the line's edge from the line's items, which are
>   in visual order, so on such a line it keeps the flat byte's answer, which
>   can differ from Chrome's: in `abc שלום <img>שלום` wrapped before the image,
>   End on the second line lands at the paragraph's end, where Chrome 155
>   lands right after the image (#1116).
> - On a line that *mixes* directions the answer is the line's logical edge,
>   which can differ from a browser field's: at the end of a line whose last
>   run is right-to-left, Chrome's End stops before the trailing space where
>   rinch's lands after it, on the wrap point.

> Alignment applies to the textblocks (`paragraph` / `heading`) overlapping the
> selection, including ones nested in lists, blockquotes, and table cells.
> Re-applying the alignment a block already has is a no-op, so a toolbar button
> bound to the current alignment stays inert. `setParagraph` ("reset to normal
> text") clears alignment back to `left` along with the rest of the formatting.

### Querying state

Toolbar "active" states and enablement read the **editor state**, never the DOM:

```rust
handle.is_mark_active("bold")          // -> bool: is bold active at the selection?
handle.current_block_type()            // -> Option<String>: e.g. Some("heading")
handle.in_node_type("bullet_list")     // -> bool: is the cursor inside a bullet list?
handle.can_run("liftListItem")         // -> bool: would this command apply right now?
```

`can_run` is a query: it asks the command whether it *would* apply (the
command's `dispatch = None` branch) and never runs the edit, so asking for every
toolbar button on every caret move builds no transactions, and a command of your
own whose dispatch branch does something besides dispatching never sees it fire
from a query.

A reactive toolbar button can read these inside a `{|| ... }` closure so it
re-renders when the selection moves:

```rust
let ed = editor.clone();
Button {
    variant: {|| if ed.is_mark_active("bold") { "filled" } else { "subtle" }},
    onclick: move || { ed.command("toggleBold"); },
    "B"
}
```

### Setting content

Pass HTML through the `content:` prop, or load it imperatively:

```rust
let editor = create_editor();

// Before or after mount — both work; the view renders the result either way.
editor.load_html("<h1>Loaded</h1><p>Programmatically set content.</p>");

rsx! { Editor { editor: editor.clone() } }
```

| Method | Purpose |
|--------|---------|
| `load_html(&str) -> bool` | Parse schema-whitelisted HTML and replace the document. No markup is refused, however broken: its text is read, into valid content (see [What the HTML reader keeps](#what-the-html-reader-keeps)). Returns `false` only when the load itself is refused (a read-only collaborating editor). |
| `doc() -> Node` | The current document (the save shape; serialize it under the `serde` feature). |
| `insert_image(src, alt)` | Insert an image node (e.g. a `data:` URL), replacing the selection. |
| `on_image_input(cb)` / `insert_image_at(&anchor, src, alt)` | Be offered pasted and dropped pictures and answer with the `src` to insert, at once or later. See [Pasted and dropped pictures](#pasted-and-dropped-pictures). |
| `toggle_link(href) -> bool` | Add a `link` mark with `href` across the selection, or remove it if the selection is already linked. No-op (returns `false`) for a collapsed cursor. |
| `active_link_href() -> Option<String>` | The `href` of the link the selection is on, for pre-filling an "edit link" dialog: for a range, the first link in it; for a caret, the link text typed there would carry. A link is not inclusive, so a caret inside it answers its `href` and a caret at its start or right after its last character answers `None` — except where it runs straight into a different link, where the caret is in the first. |
| `paste(&PasteContent) -> bool` | Paste `text/plain` and/or `text/html` over the selection the way the user's paste does: your plugins first, then the default. See [Seeing and rewriting a paste](#seeing-and-rewriting-a-paste). |
| `replace_selection_with_html(&str)` | Replace the selection with parsed HTML (the default rich paste; no plugin sees it), fitted to where it lands. See [What a paste does](#what-a-paste-does). |
| `selection_clipboard()` | The current selection serialized as `(html, plain_text)` for the clipboard. A selection inside a list is copied with its list, one across table cells with its table. A selection across the cells of one table is a cell selection, and copies those cells whole (#1847). |
| `anchor_selection() -> SelectionAnchor` | Capture the selection for a later insertion, kept pointing at the same content as the user keeps editing. See [Pasting is asynchronous](#pasting-is-asynchronous). |

HTML is **schema-whitelisted** on load: known block tags become nodes and known
inline tags become marks; an element the schema has nothing for is read through for
its content, and unknown attributes, `<script>`, inline event handlers and the like
are dropped at parse time. The document can only ever hold structure the schema
allows, and it holds all of the text: see
[What the HTML reader keeps](#what-the-html-reader-keeps).

### Your own plugins and inline decorations

`add_plugin(Rc<dyn Plugin>) -> bool` installs a plugin of your own — a
spellchecker, a search highlighter. It rebuilds the editor's state over the
current document and selection, which **discards the undo history**, so call it
on a freshly created handle, before content is typed. It is not an edit: a
read-only editor (collaborating or not) accepts it, nothing is sent to peers and
`on_change` does not fire. A plugin whose key is already installed is refused
(`false`), and a refused call changes nothing.

A command that only moves the selection (never edits the document) should be
named in the plugin's `selection_commands()`. On a read-only editor `can_run`
answers `false` for every command not named there, so a toolbar button for an
undeclared selection-only command greys out; running it still works. The
built-in `selectAll` is declared by the base commands plugin.

A plugin marks ranges of text through its `decorations()`, without touching the
document:

```rust
use rinch_editor_core::decoration::{Decoration, DecorationSet};

impl Plugin for Spellcheck {
    fn key(&self) -> PluginKey { PluginKey("my-app.spellcheck") }
    fn decorations(&self, state: &EditorState) -> DecorationSet {
        DecorationSet::new(
            self.misspelled(&state.doc)
                .map(|(from, to)| {
                    Decoration::inline(from, to, Attrs::new().with("class", "pm-spell-error"))
                })
                .collect(),
        )
    }
}
```

The view wraps each decorated stretch in a `<span data-pm-deco class="…">`.
The built-in stylesheet draws `pm-spell-error` (red) and `pm-grammar-error`
(green) as wavy underlines, on desktop and on the web; any other class is yours
to style. Decorations are recomputed from every new state, and the editor does
**not** move a range for you when text is inserted before it — a plugin that
caches ranges maps them through `tr.mapping()` in its `apply`.

`decorations()` is called on every transaction, a caret move included, so build
its set from ranges the plugin already holds rather than re-scanning the
document there. The view's own pass is about linear in the decorations (a sort, then one walk): it walks the
top-level blocks once beside the decorations sorted by position and descends
only into a block a decoration reaches (or one that still shows a segment to
take off), and a run whose decorations did not change is not rewritten (#848).

Plugin code runs **untracked** (issue #943). `decorations`, `apply`,
`init_state`, `handle_paste` and a plugin's commands run synchronously inside
whatever app effect called the handle (a `set_selection`, a `command`, an
`insert_text`), and a signal they read does not become that effect's
dependency. The flip side: a plugin is not re-asked when a signal it reads
changes. A spellchecker with an on/off switch re-runs its decorations by
dispatching something when the switch moves, from an effect of its own:

```rust
let ed = editor.clone();
Effect::new(move || {
    let _ = spellcheck_on.get();
    // Any transaction re-asks every plugin's decorations; this one changes nothing.
    ed.update(|state| Some(state.tr()));
});
```

The `build` closure you pass to `update` is the exception: it is your own code,
and what it reads tracks as usual — including what plugin code *it* calls reads
(`update(|s| { s.apply(..); … })` runs the plugins' `apply` as your code).

A right press over the editor places the caret before an app's
`data-oncontextmenu` handler runs, on both backends, so a handler that draws its
own suggestions menu reads the pressed word from `selection()`.

### Seeing and rewriting a paste

A plugin can claim a paste before the editor inserts it, through
`Plugin::handle_paste`. It is handed the editor's state at the paste and the
clipboard's `text/plain` and `text/html` flavours (`PasteContent`), and answers
the transaction to apply instead, or `None` to leave the paste alone. Every
paste the platform reports as a paste event reaches it: Ctrl+V / Cmd+V,
Ctrl+Shift+V and the context menu's Paste on desktop, and the browser's `paste`
event on the web, all go through `EditorHandle::paste`, which asks the plugins
first. A mobile keyboard's clipboard chip is the exception: it inserts text
directly, as if typed, and does not reach the hook.

A URL pasted over selected words that links them instead of replacing them:

```rust
use rinch_editor_core::{
    AttrValue, Attrs, EditorState, Mark, PasteContent, Plugin, PluginKey, Transaction,
};

struct LinkOnPaste;

impl Plugin for LinkOnPaste {
    fn key(&self) -> PluginKey { PluginKey("my-app.link-on-paste") }

    fn handle_paste(&self, state: &EditorState, paste: &PasteContent) -> Option<Transaction> {
        let url = paste.text.as_deref()?.trim();
        if !url.starts_with("https://") || url.contains(char::is_whitespace) {
            return None; // not ours: the default paste
        }
        let (from, to) = (state.selection.from().0, state.selection.to().0);
        if from == to {
            return None; // nothing selected: paste the URL as text
        }
        let link = state.schema().mark_type("link")?.clone();
        let mut tr = state.tr();
        tr.add_mark(from, to, Mark::new(link, Attrs::from_iter([("href", AttrValue::from(url))])))
            .ok()?;
        Some(tr)
    }
}

editor.add_plugin(Rc::new(LinkOnPaste));
```

- **Order.** Plugins are asked in the order they were installed (the built-ins
  first, then `add_plugin`'s, which claim no paste); the first `Some` wins and
  later plugins are not asked. With no claim the default runs: `text/html`
  parsed into structure, else `text/plain` one paragraph per line.
- **Flavours.** Either may be `None`. A browser's copy usually carries both, so
  read the text when you want to know whether the paste *is* a URL: a copied
  link's html is an `<a>` whose text may be its title. Ctrl+Shift+V offers the
  text alone. An image-only paste (a screenshot) never reaches the hook. On
  desktop the text beside the html comes from the same clipboard read
  (`paste_rich_with_text_async`).
- **Where.** `state.selection` is where the paste lands; on desktop that is
  where Ctrl+V was pressed, mapped through anything typed while the clipboard
  was read (see [Pasting is asynchronous](#pasting-is-asynchronous)).
- **What a claim is.** One transaction: one undo step, `on_change` if the
  document changed, the caret scrolled into view, and on a collaborating editor
  recorded and broadcast like any local edit. `Some(state.tr())` swallows the
  paste. A read-only editor refuses the claim as it refuses every edit (and on
  desktop starts no clipboard read at all).
- **Borrowing.** `handle_paste` runs while the editor is borrowed, like a
  command, so it must not call the `EditorHandle`. Anything it needs (a note's
  title for a link, say) has to be at hand already.

`EditorHandle::paste(&PasteContent)` is the same entry point for an app of its
own: a "Paste link" menu item that read the clipboard itself goes through the
plugins exactly as Ctrl+V does.

### Pasted and dropped pictures

By default a bitmap pasted into an editor becomes an image whose `src` is a
`data:` URL, and image files dropped on a desktop editor go to the app's own
`onfiledrop` handler. An app that keeps pictures somewhere of its own (a blob
store, an upload) asks to be offered them instead:

```rust
let editor = handle.clone();
handle.on_image_input(move |input: ImageInput| {
    // input.bytes: the encoded file (a pasted bitmap arrives as PNG)
    // input.mime:  "image/png", "image/jpeg", "image/gif" or "image/webp"
    // input.name:  the dropped file's name; None for a paste
    let src = store_blob(&input.bytes)?;        // answer at once...
    Some((src, String::new()))                  // ...with (src, alt)
});
```

The editor inserts an image with that `src` where the picture was aimed: the
selection at the paste, or the caret at the drop point. When storing takes a
round trip, keep the input, return `None`, and finish later:

```rust
handle.on_image_input(move |input| {
    let editor = editor.clone();
    store_blob_async(input.bytes, move |src| {
        editor.insert_image_at(&input.anchor, &src, "");
    });
    None
});
```

`input.anchor` is the place the picture was aimed at, kept pointed at the same
content while the person, or a collaborating peer, types on. When the answer
arrives the picture goes there and **the person's caret stays theirs**: if they
have moved on, their selection is kept (shifted by the picture when it lands
before it), nothing scrolls, and the picture is an undo step of its own. If
they have not moved, the caret ends up after the picture, as after any insert.

`insert_image_at` answers `false`, changing nothing, when the place is gone
(the document was replaced, or a peer removed or restructured the block it was
in), when the editor has become read-only, or when the place no longer takes an
image. **Delete what you stored for it then**: nothing references it. Returning
`None` and never inserting is a refusal: nothing is inserted, and no `data:`
URL is made.

`ImageInput` and `ImageInputSource` are `#[non_exhaustive]`: read the fields
you need (or destructure with `..`), and give a `match` on the source a
wildcard arm.

### Pictures drawn over, and controls over a picture

The `<img>` of an `image` node asks the image loader for its node's `src`. An app
that shows something else for some pictures (a picture with marks drawn over it)
chooses the source from the node's attributes, once, for every editor on the
thread:

```rust
rinch::editor::set_image_source(|attrs| {
    let id = attrs.get_str("data-annotation-id").filter(|i| !i.is_empty())?;
    Some(format!("{}#annotation={id}", attrs.get_str("src")?))
});
```

`None` keeps the `src`. The document is untouched (copy, export and collaboration
still see the `src`); only the picture shown changes. The answer is what the
loader is asked for and what `rinch::image::reload_image` names, so two images with
one `src` and different attributes are two pictures, each loaded and reloaded on
its own: `reload_image("…#annotation=a1")` asks again for that one alone. It is asked
whenever an `<img>` is built or its node's attributes change, with the editor
borrowed, so it must be a function of `attrs` alone. `clear_image_source()` takes
it away. In the browser it is `rinch_web::set_image_source`.

Installed while a component renders, the source function is removed when that
component unmounts (a later install is never clobbered by an earlier unmount);
installed from `main`, it keeps app lifetime. It is per thread, not per document:
two documents on one thread (two embedded contexts) share it, the last install
winning for both. Pictures already shown keep the source they were given until
their `<img>` is rebuilt or their attributes change; `handle.load_doc(handle.doc())`
makes an editor's pictures ask again.

To float controls over a picture (a button in its corner), an app hears where the
pointer is:

```rust
let eye: Signal<Option<(Pos, rinch::reactive::ElementBounds)>> = Signal::new(None);
editor.on_image_hover(move |hover| eye.set(hover.map(|h| (h.pos, h.rect))));
```

| Method | Purpose |
|--------|---------|
| `on_image_hover(impl Fn(Option<&ImageHover>))` | The pointer came onto an image (`Some`), moved straight onto another (`Some`), or left this editor's images (`None`). Called only when that answer changes; an image that was edited, or that a scroll or relayout moved, is reported again on the next move. |
| `image_at_host(host_id) -> Option<(Pos, Attrs)>` | The image whose `<img>` is that host element (what the runtimes ask). |

`ImageHover` carries `pos` (before the image: where a `SetNodeAttrStep` for it
goes), `attrs`, and `rect`, the box the `<img>` is painted in, in the same frames
as `LinkHover::rect`. The controls are the app's own elements outside the editor,
so moving onto them leaves the image and reports `None`; keep them shown while the
pointer is over them with their own `onmouseenter` / `onmouseleave`. Not reported
during a drag-select or a drag-and-drop; free while no editor has the callback.

Hover is measured on pointer moves only: a wheel scroll moves the picture under a
resting pointer and reports nothing until the pointer moves, so **hide the controls
when the content scrolls** (an `onscroll` on the scroller). On desktop the pointer
leaving the window reports nothing either (the browser reports `None`). Like
`on_change`, the callback belongs to the component that registered it: once that
component unmounts it is not called again, and one registered outside any render
keeps app lifetime.

With the callback registered:

- A paste whose clipboard holds a bitmap is offered. That includes a browser's
  "Copy image", which also puts an `<img>` on the clipboard as html: when the
  html is pictures and nothing else, the bitmap is what the app gets.
- Image files dropped on the editor are offered one by one, in order, each as
  soon as it is read, and the editor takes the keyboard. A file is a picture
  when its extension says so (`png`, `jpg`, `jpeg`, `gif`, `webp`), its first
  bytes agree, and it is at most **64 MiB** (the file is read whole into
  memory and handed over as one `Vec`). The app's `onfiledrop` handler gets
  **every other file of the drop**: the PDF dropped with a photo, a text file
  that is only named `.png`, a picture over the limit. A drop with no picture
  in it, or on a read-only editor, is the handler's whole, as before.
- While a file drag from outside the window hovers over the editor, a **drop
  caret** (a caret bar of its own, `data-pm-drop-caret`) shows where a picture
  would land and follows the pointer; the drop lands exactly there. It shows
  when the editor is not read-only and one of the hovered files is *named*
  like a picture (its bytes cannot be read before the drop). The names arrive
  a moment after the drag enters the window; until they do (or if the
  dragging app does not give them), it shows for any file drag, and a drop
  that holds no picture inserts nothing (the caret just goes away). The selection does not move and
  nothing scrolls to the caret until the drop. Held in the top or bottom band
  of the editor's scroll container, the drag scrolls it, as an in-app drag does,
  so a long note can be targeted. The caret and the scrolling end with the drag:
  its cancel, its drop, or the first press, wheel, key or window blur the
  window hears, since none reaches it while an OS drag is in flight. A pointer
  move hides the caret and stops the scrolling but keeps the shown position for
  the drop: on X11 the window hears the pointer enter at the release, before the
  drop arrives.
- A pasted bitmap has no size limit of its own: it is whatever the clipboard
  held, encoded as PNG on the UI thread (about 50 to 90 ms for a 4K
  screenshot).
- Where an image cannot be inserted (the caret is in a code block), nothing is
  offered: the app is not asked to store a picture that would be refused.
- An html paste that holds text as well as pictures is an ordinary paste: its
  `<img>` elements keep the `src` they came with. `Plugin::handle_paste` is
  where an app rewrites or strips those.

The list above is the desktop's. The browser build offers pictures through
the same callback, with these differences. There a picture is a `File`: a
pasted image file is offered as its own bytes with the browser's name for it,
and image files dragged onto the editor from outside the page are offered at
the drop point, the editor taking the drop (and the keyboard) so the browser
does not open the file in place of the page. A file counts as a picture when
the browser calls it `image/*` (so an SVG or an AVIF is offered, typed as the
browser typed it), and there is no size limit. The browser build does not
dispatch `onfiledrop` (a browser drop has no paths); the files of a drop the
editor does not take — the others of a mixed drop, every file of a drop with
no picture or on a read-only editor — go to an `ondropfiles` handler around
it, as they do on the desktop.

On a **read-only** editor with the callback, a drop is prevented and nothing
is offered: the caret and the focus stay, and the files go to an
`ondropfiles` handler around the editor if there is one. It has to be
prevented, or the browser would navigate the tab to the dropped file and the
person would lose the app. With no callback a pasted image is a `data:` URL,
as before, and a file drop is the browser's (which opens the file; that is
[#1458](https://github.com/joeleaver/rinch/issues/1458)).

### Embeds: blocks an app draws

The starter kit has an `embed` node: a block of its own, holding no text, that
stands for something the app draws — a sheet, a canvas, a counter. The document
holds only the block and the app's own `data-*` attributes (the same rules as an
image's: valid custom data attribute names, no name rinch reserves, at most 32 of
at most 1024 bytes each); those attributes are what the app keys its own state to.

```rust
use rinch::prelude::*;

let sheet = AttrValue::from("sheet-42");
handle.update(|state| {
    let embed = state
        .schema()
        .create_node("embed", Attrs::from_iter([("data-sheet", sheet)]), Fragment::empty())
        .ok()?;
    let mut tr = state.tr();
    tr.insert_block(embed).ok()?;
    Some(tr)
});
```

`handle.insert_embed(attrs)` does the same in one call. It is edited as a
horizontal rule is: a click or an arrow selects it whole, Backspace at the start of
the paragraph after it (or Delete at the end of the one before) selects it, and
Backspace or Delete on it removes it. Removing an atom always takes two presses,
which is a deliberate difference from ProseMirror: there, when the paragraph is not
empty, one Backspace deletes the rule or embed before it (and one Delete the one
after it). An embed can hold an app's component and its state, so rinch selects it
first and lets the next press delete it — the same for a horizontal rule. An empty
paragraph beside the atom goes in the first press. Its host element is
`<div data-pm-type="embed" data-sheet="sheet-42">`; with nothing drawn in it the
default stylesheet gives it a line of height and a dashed outline. What is drawn in
it is a [node view](#node-views-your-component-in-the-document).

**HTML.** An embed is written as that `<div>` and read back from it, wherever a
block can stand. The default paste reads the element as an embed only from rinch's
own copy-out, which marks it (`data-rinch-clip`): another application's
`<div data-pm-type="embed">` — say, a browser copy of a page showing an app's
export — is read as an ordinary `<div>`, so its content arrives as content.
Another application has nothing to show for an embed, so an
app can give exported HTML markup to put inside the element:

```rust
use rinch_editor_core::serialize::set_html_fallback;

let _guard = set_html_fallback(|node| {
    let sheet = node.attrs().get_str("data-sheet")?;
    Some(format!("<table><tr><td>Sheet {sheet}</td></tr></table>"))
});
```

`node_to_html` and the clipboard copy-out then write
`<div data-pm-type="embed" data-sheet="…"><table>…</table></div>`; rinch reads that
element back as the same embed (a load always, a paste when it is rinch's copy) and
ignores what is inside it. The markup is written as given, and is the app's to make
safe. It must be valid inside a `<div>`: well-formed, and with no bare `<li>`,
`<td>` or `<tr>` outside its own list or table — in an embed inside a list item or
a table cell those would close the outer item or cell when read back, and become a
new one. A whole `<table>` or `<ul>` is fine. The callback is per thread and
is unregistered when the guard is dropped; registrations stack, so dropping a later
one brings an earlier one back. **Markdown** writes the bare element as an HTML
block, which both of its readers take back, and a `DocNode` keeps the attributes
like any others.

### Node views: your component in the document

A node view draws one of your components in place of an embed: a live sheet, a
canvas, a counter. Register it on the handle:

```rust
use rinch::prelude::*;
use rinch_editor_core::AttrValue;

let view = NodeViewSpec::new("embed", |__scope, ctx: NodeViewCtx| {
    let clicks = Signal::new(0);
    let sheet = ctx.attrs;
    let ctx_for_button = ctx.clone();
    rsx! {
        div { class: "sheet",
            p { {move || format!("Sheet {}", sheet.get().get_str("data-sheet").unwrap_or("?"))} }
            input { placeholder: "a cell" }
            button {
                onclick: move || {
                    clicks.update(|n| *n += 1);
                    ctx_for_button.set_attrs(|a| *a = a.with("data-rev", AttrValue::from(clicks.get().to_string())));
                },
                "Save"
            }
        }
    }
})
.matches(|attrs| attrs.get_str("data-sheet").is_some());
handle.register_node_view(view).keep();
```

- **Which nodes.** Every node of the spec's type (a **leaf** type: `embed`, or an
  app schema's own) whose attributes `matches` accepts; the first registered spec
  that matches wins. Releasing a registration (dropping it, or the component that
  registered it unmounting, after `.keep()`) unmounts its views, and the next spec
  that matches takes over.
- **Lifetime.** A view is mounted when its node's host is built and disposed when the
  node leaves the document — its scope, with every signal, effect and handler it
  made, goes with it. Typing anywhere else never touches it, and its own state
  (`clicks` above) survives. It is also disposed when the editor unmounts.
- **One node, for good.** A view is never handed a different node. Its context
  follows its node's attributes — `ctx.set_attrs` (one undo step, sent to
  collaborators key by key), its undo and redo, and a collaborator's change all
  arrive in `ctx.attrs` without a remount — also when the same transaction inserts
  or removes blocks around it. Anything else that puts a node where its
  node stood — a replace, an undo of a delete, a move, a paste of a copy, even an
  identical twin — is another node and gets a fresh mount. Key your app's state to
  the node's attributes, not to its position. A render may set its own node's
  attributes (a default): the view is mounted once and `ctx.attrs` follows. If
  that write makes `matches` refuse the node, the view is unmounted (or another
  matching spec's mounted) right after, as for any other change of the node —
  up to 8 such re-matches inside one another. Renders that keep changing which
  spec matches (spec A's render writes what only B matches, and B's what only
  A matches) stop there: the last view stays mounted, with its `ctx.attrs`
  refreshed, although its spec no longer matches the node, until the node next
  changes.
- **Context.** `ctx.attrs`, `ctx.selected` (node-selected), `ctx.read_only` and
  `ctx.dark_mode` are memos; `ctx.pos()`, `ctx.set_attrs(..)`, `ctx.delete()`,
  `ctx.select()`, `ctx.return_focus()` and `ctx.editor()` act on the node. A
  read-only editor refuses `set_attrs` and `delete`.
- **Events.** A press or a key inside the component is the component's: an `<input>`
  in it takes the keyboard (the editor's caret goes), a button in it clicks. A press
  on the host's frame — the padding the default stylesheet gives a host with a view
  — node-selects the node, as an arrow onto it does.
- **Keys.** While the keyboard is inside the view, `.on_key(|key| bool)` sees every
  key first; `true` consumes it. An Escape it does not consume gives the keyboard
  back to the editor with the caret right after the node (`ctx.return_focus()` does
  the same from code). An arrow that lands on the node calls
  `.on_caret_enter(|from: CaretFrom| bool)` with the side it came from; answer
  `true` when the view took the keyboard (it focused something of its own),
  `false` to leave the node selected. Up and Down node-select a node with a view
  (they step over a horizontal rule), from the last (first) line of a list, a list
  item or a quote too when the view is right after (before) it.
- **Layout.** The host's height follows the component's, and the caret and
  selection below it follow when it grows or shrinks.
- **Collaboration.** A collaborator's attribute change keeps the view mounted —
  alone, beside their typing in the same update (a reconnect catching up), on an
  embed that had no attributes yet, and in a document holding a table. Which embed
  is which is the shared document's: an embed a collaborator moved, or two embeds
  they swapped — equal ones included — arrive as other embeds, and the views on
  them are remounted. While this editor's own changes are not reaching the
  others (`collab_outbound_stall()` is `Some`, e.g. it holds a task list), the
  same holds: a collaborator's attribute change keeps the view mounted, and a
  view whose embed the editor cannot prove is the same one is remounted rather
  than kept — and when the stall ends, an embed this editor reordered or copied
  meanwhile reaches the others as a new embed, so their views of it remount. A
  collaborator's change that arrives during a stall brings every block that
  differs from the shared document back to it, so the unsent edits in those
  blocks are dropped — a view's own attribute change included. Every peer must run a build with node views: a peer built
  before them writes an embed it moved into the place of the node that stood there,
  which a newer peer cannot tell from an attribute change, and the newer peer's
  view then shows the other embed's attributes.

Your code (`matches`, the render, `on_key`, `on_caret_enter`) runs with no editor
state borrowed, after the edit that caused it has been applied: it may read the
editor, dispatch to it and write signals.

### Drawing an image

A spec for `"image"` draws the images it matches with your component, on the
line, in place of the picture: a status chip, a live preview, a mention.

```rust
let badge = NodeViewSpec::new("image", |__scope, ctx: NodeViewCtx| {
    let attrs = ctx.attrs;
    rsx! {
        span { class: "badge", {move || attrs.get().get_str("data-badge").unwrap_or("").to_string()} }
    }
})
.matches(|attrs| attrs.get_str("data-badge").is_some());
handle.register_node_view(badge).keep();
```

- **Only the images it matches.** Every other image keeps its `<img>`, and
  `set_image_source`, `on_image_hover` and `on_image_input` work for it as before.
  An image that stops matching (its `data-badge` removed) gets its `<img>` back.
- **One character of the line.** A drawn image's host is a
  `<span data-pm-type="image" data-…>` laid out as an inline-block, as wide and
  tall as your component, with no frame. The caret treats it exactly as a picture:
  one position, drawn at its left edge before it and its right edge after it; an
  arrow steps over it, Home and End go past it, a press on the text beside it
  places the caret there, and a press on your component is your component's.
  `ctx.select()` node-selects it, and `ctx.selected` says when it is.
- **Identity** is an `embed`'s: `ctx.set_attrs`, its undo and a collaborator's
  change of the picture's attributes reach `ctx.attrs` with no remount; a picture
  moved, or another (even an equal one) put in its place, is a fresh mount — on
  every collaborator too: a picture pasted over another, **with the same `src`
  included**, is a new picture in the shared document, so each peer's view of the
  old one is remounted, never handed the pasted one. Two pictures swapped in one
  edit are each the same picture moved, and no view is handed the other one.

### Node views, step by step

`examples/editor-node-views` (desktop) and `examples/editor-node-views-web` (the
same `views.rs`, in the browser) draw three views. Each one keeps everything it
needs in its node's `data-*` attributes, so a change is an edit like any other:
undone with Ctrl+Z, saved with the document, sent to collaborators.

**A counter.** The count is `data-count`; the buttons write it.

```rust
NodeViewSpec::new("embed", |__scope, ctx: NodeViewCtx| {
    let attrs = ctx.attrs; // a Memo: Copy, so every closure can have it
    let count = move || attrs.get().get_str("data-count").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let add = ctx.clone();
    rsx! {
        div {
            span { {move || count().to_string()} }
            button {
                onclick: move || {
                    let n = count() + 1;
                    add.set_attrs(|a| *a = a.with("data-count", AttrValue::from(n.to_string())));
                },
                "+"
            }
        }
    }
})
.matches(|a| a.get_str("data-kind") == Some("counter"))
```

`handle.insert_embed(Attrs::from_iter([("data-kind", AttrValue::from("counter"))]))`
puts one at the caret.

**An editable table.** A grid of `<input>`s, each cell's text in
`data-c-<row>-<col>`:

- rinch has no HTML table layout (`<table>`/`<tr>`/`<td>` are laid out as plain
  boxes), so the grid is a CSS grid, `grid-template-columns: repeat(N, …)`,
  whose rows are `display: contents` (each row's cells become the grid's items).
- A cell shows its attribute with a reactive `value: {|| …}` and writes it back in
  `onchange` (Enter, or leaving the cell): one `set_attrs`, one undo step, the one
  key sent to collaborators.
- On a raw `<input>`, write `value: {|| …}` — `value_fn:` is a component prop
  (`TextInput`), and on a raw input it becomes a stray attribute (#1591) — and
  handle Enter in `onchange` or `on_key`: `onsubmit:` on a raw input is wired as a
  click (#1590).
- **Keys.** While a cell has the keyboard, `.on_key(|key| bool)` hears each key
  **press** first (never a release), before the input and before Tab moves the
  keyboard. The example claims Up and Down to move between rows and leaves every
  other key to the input: Left and Right move its caret, Enter commits it, **Tab**
  and Shift+Tab go to the next (previous) tabbable element in document order — the
  next cell, and from the last cell on to the next focusable element of the page —
  and an Escape it leaves gives the keyboard back to the editor, the caret right
  after the table. To keep Tab inside the view, claim it in `on_key` and focus the
  next cell yourself. A page-level `set_keyboard_interceptor`, and a registered
  node's `on_key`, also see key **releases**: there, return early on
  `k.is_up()`, or an arrow moves twice.

**A badge** is the image view above, its click cycling `data-badge`.

**Exporting.** Another application has nothing to draw a view with, so the
example gives exported HTML a static stand-in for each
(`set_html_fallback`): a `<table>` of the cells inside the table's
`<div data-pm-type="embed">`, and for a badged image a
`<span data-pm-type="image" src=… data-badge=…><b>[hot]</b></span>` written
**in place of** its `<img>` (an `<img>` cannot hold anything). rinch reads both
back as the same atoms. An image's markup must be inline content (it is inside a
`<span>`); answer `None` for an image to keep its plain `<img>`.

**Copy and paste.** A copied embed or drawn image pastes back as the same kind of
atom with its attributes (rinch's copy-out marks it, and the default paste keeps
the attributes only beside that mark), and the copy gets a view of its own. If
your views key app state to an attribute (a sheet id), give the copy a new one in
`Plugin::handle_paste`: read the html with `slice_from_pasted_html`, rewrite the
attribute on the atoms, and `tr.replace_selection(slice)`
(`rinch-editor-view/tests/node_view_paste.rs` does exactly this).

### Dark mode

The editor's built-in stylesheet has light and dark color schemes; toggle between
them with `set_dark_mode`:

```rust
let dark = Signal::new(false);
let ed = editor.clone();
button {
    onclick: move || {
        dark.update(|d| *d = !*d);
        ed.set_dark_mode(dark.get());
    },
    {move || if dark.get() { "Light mode" } else { "Dark mode" }}
}
```

### Read-only

`set_read_only(true)` turns the editor into a document people can read, select and
copy, and cannot change — what `readonly` is to an `<input>`. It is a runtime switch:
flip it whenever access changes, mounted or not, and ask with `is_read_only()`.

```rust
let editor = create_editor();
editor.load_html(&note_html);
editor.set_read_only(!can_edit);          // at open…

// …and again whenever the answer changes (a role was granted or revoked):
editor.set_read_only(!now_can_edit);

rsx! { Editor { editor: editor.clone() } }   // or: Editor { read_only: true, content: … }
```

| Refused (each answers `false`; document, undo history and `on_change` untouched) | Still works |
|---|---|
| Typing, IME commit, Backspace/Delete, Enter | Placing and moving the caret, pointer and keyboard selection, `selectAll` |
| Paste, cut, pasted images | Copy (`selection_clipboard`, Ctrl+C, the context menu's Copy) |
| Every document-changing `command(..)` — marks, block types, lists, indent/outdent, tables — and the keys bound to them | Every query: `doc()`, `is_mark_active`, `current_block_type`, … |
| `undo` / `redo` | `load_html` / `load_doc` — the app showing a document is not the user editing one |
| `toggle_link`, `insert_image`, a task checkbox click, any `update(..)` transaction that changes the document or sets stored marks | **Remote collaboration updates** (`collab_receive`), exactly as in an editable editor |

It makes no difference who asks: a toolbar button calling `command("toggleBold")` and
a keystroke go through the same gate, which sits at the one place every local change
lands rather than in each input handler. `can_run(name)` answers `false` for every
command except those that change only the selection, so a toolbar that greys its
buttons from it goes inert with the switch; a toolbar of your own should also read
`is_read_only()`. It answers without running the command: a command counts as
selection-only when its plugin lists it in `Plugin::selection_commands()`
(`selectAll` is the built-in one), and an unlisted command is greyed. The list only
steers `can_run`; `command(..)` on a read-only editor still refuses whatever would
change the document or the stored marks, listed or not. The built-in context menu
offers Copy and Select all only, the OS input method stays off (no IME candidate
window), and on the web the hidden capture field is `readonly`, so no soft keyboard
is raised and the browser's own menu drops Paste and Cut.

Tab and Shift+Tab move the focus on, as from a `readonly` field: to the next and the
previous Tab stop from the editor's place in the page, on desktop and on the web (inside
a focus trap, within the trap). Past the last or the first Tab stop, desktop wraps round,
as every Tab there does; the web behaves as a native field: Tab is left to the browser,
and Shift+Tab sends the focus to the document. An editable editor keeps them for table
cells and list indentation, so a reader who comes in by keyboard can always leave by keyboard. An
`on_key` callback still sees Tab first and may claim it.

The caret stays visible — a reader selects and copies with it. The mounted container
carries `data-pm-readonly="true"` while the switch is on, for your own styling; the
built-in stylesheet uses it to hide the empty-editor placeholder.

Switching it on drops pending typing state (a clicked "Bold" waiting for text, an IME
preedit). Switching it off gives everything back, undo history included.

### Keeping the caret in view

A focused editor scrolls its caret into view after the **user** moves it or edits
at it: typing, Enter, Backspace/Delete, every command and the keys bound to them,
arrow keys and clicks, `set_selection`, paste, IME commit, `insert_image` and
`toggle_link`. For a node selection the selected
node's outline is what is revealed, and for a cell selection the cell under its
moving (head) corner.

The scroll is the minimal one (`nearest`): a caret already in view moves nothing,
and one out of view is brought just inside the edge it left by, instantly rather
than smoothly. Every scroll container the caret is in moves, innermost first and the
page last, on both axes, so an editor whose scroller is itself off screen is brought
on screen too. On the **web** that is the browser's `scrollIntoView`; **desktop**
does the same (since issue #842; before it only the nearest scroll container moved).

It deliberately does **not** scroll when the caret's position on screen changes
without the user moving it — a user who scrolled away to read something is left
where they are:

- a collaborator's edit arriving (`collab_receive`), even one above the caret;
- `load_html` / `load_doc`, and `add_plugin` (neither is an edit);
- a window resize that reflows the text, and the user's own scrolling (including a
  long, virtualized editor measuring the blocks it scrolls past);
- an app transaction through `update(..)` that edits the document and lets the
  selection be *mapped* rather than setting it. A transaction that calls
  `tr.set_selection(..)` — as `tr.insert_text` does — scrolls like typing; to reveal
  the caret after an edit of your own, set the selection explicitly, even to where
  it already is (`let sel = tr.selection(); tr.set_selection(sel);`);
- an edit a [read-only](#read-only) editor refuses. A read-only editor's caret
  moves still scroll: they are real selection changes;
- focus on its own. A click that places the caret scrolls (it moved the
  selection); a click that focuses the editor without moving the caret — a task
  checkbox, a right-click on an image — does not jump to where the caret was, and
  neither does [`focus()`](#focus-and-scrolling-from-code) when nothing is owed;
- an editor that is not focused: its caret is not drawn, so a programmatic
  `set_selection` on it scrolls when it is next focused, not before.

Moving the caret into a part of a long, virtualized desktop editor that has not
been laid out yet (Ctrl+End from the top, `set_selection` far below) lays out the
caret's paragraph and scrolls to it; as the paragraphs around it are measured,
within that frame, the scroll follows the caret, so it ends in view. A
`scroll_into_view` asked for together with the caret move still decides where the
view goes: the caret is not followed past it.

Not covered: the moving end of a text **range** (Shift+arrow) is not revealed. An
app that wants it on screen asks for it with `scroll_into_view`, below.

### Setting the selection from code

`set_selection`, `command("selectAll")` and any other change to the selection made
by app code — from a toolbar button, a menu, a timer or an effect — draws the caret
and the selection highlight without an input event in the editor. On **desktop**
(and in an embedded `RinchContext`) the change wakes the runtime for one layout and
overlay pass, like any other change to the page; on the **web** the overlays are
redrawn in a microtask, once the code that made the change has returned. Only a
focused editor draws a caret or a highlight, on both backends: set on an editor
that is not focused, the selection is shown when it is focused.

### Focus and scrolling from code

Opening a note at a link — select the words the link quotes, bring them on screen,
put the keyboard there — takes three calls, and no click:

```rust
handle.set_selection(Selection::text(from, to));
handle.scroll_into_view(from, to);
handle.focus();
```

**`scroll_into_view(from, to)`** scrolls so that the range is on screen: its start,
and as much of the rest as fits, 16px inside the scroller's edge (where the
document extends that far). `from == to` reveals a caret position. A range already
in view moves nothing. It needs no focus and touches neither the selection nor
focus, and it is the only way to reveal a **range**: `set_selection` brings a caret
into view on its own, but never a range. The ends may be given in either order; a
position between blocks reveals the nearest text.

It uses the scroll the caret uses, on two hidden boxes the view places over the
range's end and then its start, so the rules above apply: on **desktop** it happens
after the next layout (the call wakes the runtime for one, from an effect or a timer
as well as from an event handler); on the **web** it happens during the call. Both
move every scroll container the range is in, the page included. A virtualized desktop editor lays out the blocks holding
`from` and `to` for it. The request waits on the handle until there is geometry for
its start (a block edited since the last layout has none on desktop); if the end has
none by then, the start alone is revealed. A later call replaces it, local edits
carry it along (text typed exactly at either edge stays outside the range), and
`load_html` / `load_doc` drop it.

A request that cannot be fulfilled does not wait for good. In an editor that is not
rendered (`display: none`, an inactive tab) the boxes are laid out at zero size on
both backends, so the request is used up at once and scrolls nothing; one whose start
has no geometry at all for `REVEAL_PATIENCE` (8) overlay passes in a row is dropped.
Either way nothing scrolls when the editor is shown later: ask again then.

**`scroll_into_view_aligned(from, to, align)`** chooses where the range lands.
`ScrollAlign::Nearest` is `scroll_into_view` itself. `ScrollAlign::Fraction(f)` puts
the start's line `f` of the way down the scroll container's visible height — its
padding box, inside the border, as a browser measures it (`0.0` the top edge, `1.0 / 3.0` a third down; clamped to `0.0..=1.0`), never nearer than
16px to either edge, and it scrolls even when the range is already in view. The
scroll is clamped to what the content allows, so a range near the top of the
document stays near the top of the view and one near the end sits lower. A range
taller than the space below `f` still puts its start at `f`. To open a note at a link
with some context above the words:

```rust
handle.set_selection(Selection::text(from, to));
handle.scroll_into_view_aligned(from, to, ScrollAlign::Fraction(1.0 / 3.0));
handle.focus();
```

It waits, carries and drops exactly as `scroll_into_view` does. On both backends it
sets the `scrollTop` of the nearest vertical scroll container (the page's, when no
ancestor scrolls) and then brings the start into the view of the scrollers further
out, the page included, the "nearest" way. One limit: in a virtualized desktop
editor the blocks the scroll brings into the window were laid out at their
estimated heights when the scroll was computed and settle to their measured ones
after it, so the start can land a fraction of a line off `f`. Both are built on
`NodeHandle::scroll_to_fraction(fraction, margin)`
(`DomDocument::request_scroll_to_fraction`), which any element can use.

**`focus()`** gives the editor the keyboard as a press in it would — the previous
owner loses it the same way (an `<input>` commits its change, another editor hides
its caret) — without moving the selection. It never asks for a scroll of its own,
so giving the keyboard back after a dialog closes does not jump to a caret the user
scrolled away from. It does **perform a caret scroll already owed**: a
`set_selection` of a caret (or an edit through `update` that sets the selection) on
an editor that is not focused asks for its caret to be shown, and only a focused
editor draws a caret, so that scroll waits and happens when `focus()` lands. That is
what makes `set_selection(Selection::cursor(pos)); focus()` show the caret. A
*range* selection owes nothing (a range is never revealed on its own), which is why
the deep-link example above asks with `scroll_into_view`. The caret or selection
highlight is drawn where the selection already is. On **desktop** it posts the same
focus request `NodeHandle::focus` posts (which now focuses an editor container too),
and the runtime applies it through the focus arbiter after the current event or
effect; on the **web** it focuses the editor's hidden capture textarea during the
call (`preventScroll`), as a mousedown does.

**`blur()`** is the other half: if this editor holds the keyboard, it lets go and
nothing holds it — what a press on page content that takes no focus does. Keys stop
reaching it, and its caret and selection highlight are hidden as on any blur; the
selection is kept, so a later `focus()` shows it again. If the editor does not hold
the keyboard, nothing happens: another editor or a field that has it keeps it. An app
with several editors calls it when a pane stops holding a document:

```rust
// The person focused an empty pane: no editor takes keys now.
for pane_editor in &editors {
    pane_editor.blur();
}
```

On **desktop** it posts a release that the runtime applies at the same point as
`focus()`'s request, and **the two are applied in the order they were called**:
`focus(); blur()` in one handler or effect leaves nothing focused and
`blur(); focus()` leaves the editor focused. A release never displaces a focus
request posted by anything else (an overlay opening still moves focus into itself),
and several releases in one turn are all applied. Both are applied right after a
click or Enter/Space handler, otherwise before the next key press, pointer press or
IME event is routed — a key already queued behind the Escape whose `on_key` called
`blur()` does not reach the editor — and at the latest on the next turn of the event
loop. On the **web** it blurs the hidden
capture textarea during the call if this editor is the focused one; focus goes to
the page body, as a browser's own `blur()` does.

Effects queued by a signal write earlier in the same handler run before the blur, as
they do before any `NodeHandle` call, so `open.set(false); editor.blur()` leaves
nothing focused even when the closing overlay hands focus back to the editor. A
composition in progress ends with the blur and leaves no preedit behind: the web
commits it (the browser's `compositionend` reaches the editor before it lets go),
desktop drops it.

All three are **no-ops before the editor is mounted**, and nothing is remembered for the
mount: call them after the `Editor {}` has rendered.

### Links: clicking and hovering

The editor never follows a link by itself: what a link *means* — a URL to open in a
browser, another document in your app — is the app's business. Two callbacks tell
the app what the pointer does with links:

```rust
let ed = editor.clone();
editor.on_link_click(move |click| {
    if !click.primary {
        return false; // a plain click puts the caret in the link, as always
    }
    open_link(&click.link.href);
    true // claimed: the caret does not move
});

let tooltip: Signal<Option<(String, rinch::reactive::ElementBounds)>> = Signal::new(None);
editor.on_link_hover(move |hover| {
    tooltip.set(hover.map(|h| (h.link.href.clone(), h.rect)));
});
```

| Method | Purpose |
|--------|---------|
| `on_link_click(impl Fn(&LinkClick) -> bool)` | A single primary-button press on a linked character, **before** the caret is placed. Return `true` to claim it: no caret move, no drag-select, no selection change (the editor still takes focus). `false` and the press is an ordinary one. Called in a read-only editor too. |
| `on_link_hover(impl Fn(Option<&LinkHover>))` | The pointer came onto a link (`Some`), moved straight onto a different link (`Some`), or left this editor's links (`None`). Called only when that answer changes, never once per move. |
| `link_at(pos) -> Option<LinkSpan>` | The link carrying the character that starts at `pos`, as the whole run. |

A `LinkSpan` is the whole link as the reader sees it: `href`, `title` (`None` when
empty) and its `from`..`to` range, taken across other formatting, so a link with a
bold word in the middle is one span. `LinkClick` carries the span and the modifiers
(`ctrl`, `meta`, `shift`, `alt`) plus `primary`, the platform accelerator — Cmd on
macOS, Ctrl elsewhere — which is the usual "open the link" chord. `LinkHover`
carries the span and `rect`, the box around the link's painted run (a wrapped link
is the union of its lines), measured when the pointer came onto it: logical window
pixels on desktop, the frame `bounds_signal()` and root-level absolutely positioned
popups use; viewport client pixels (`getBoundingClientRect`) in the browser.

Both are decided by **the character under the pointer**, not by the nearest caret
position. The two differ at a link's edges: a caret at either edge of a link is not
in it (a link is non-inclusive, so `active_link_href()` answers `None` there and
typing there is plain), but the pointer over the right half of the link's last
letter, or over its first letter, is on it — and the pointer over the space after
the link is not.

Not reported: a double or triple press (they select a word or a block), the
secondary button (the context menu keeps its own link handling), a press on an image
inside a link (it selects the image), and hover during a drag-select or a
drag-and-drop. A callback runs with no internal borrow held, so it may re-enter the
handle — load another document, read the selection. It also runs untracked: a
signal it reads never becomes a dependency of an effect that happened to call
`dispatch_link_click` (issue #931). Like `on_change`, each belongs to the component
rendering when it was registered: once that component unmounts the callback is not
called any more (a press is not claimed, and the editor counts as having no click
callback), and one registered outside any render keeps app lifetime. A pointer move pays nothing for
hover while no editor on the thread has an `on_link_hover` callback; with one it
reuses the move's own hit test on desktop.

The callbacks are not run inside a `batch()` (like `on_change`): a callback that
writes several signals flushes effects after each write. Wrap its body in
`batch(|| …)` if that matters.

**In the browser** an editor link is a real `<a href>`, and the click that follows
a press would navigate even though the editor cancels the press. Whether the
browser follows it is decided per editor:

| Editor | A click, middle click or Ctrl/Cmd+click on a link |
|--------|---------------------------------------------------|
| **Editable** | Never navigates. A click in an editable editor is an edit gesture (it placed the caret), so following the link is always wrong. |
| **Read-only, with an `on_link_click` callback** | Never navigates. The callback is where the app says what a link does, even when it declines the press. |
| **Read-only, no `on_link_click` callback** | The browser's own behaviour: a click follows the link, and a middle click or Ctrl/Cmd+click opens it in a new tab. |

The same rule covers an `<a href>` inside the editor that is not a link mark, but
such an anchor is never offered to `on_link_click`. The right-click menu is never
touched: it is the browser's own link menu, whose "Open link" still works (see
[On the web](#on-the-web-rinch-web)).

**Keyboard activation.** The editor container is not `contenteditable`, so an
editor link is an ordinary Tab stop, and the browser turns Enter on a focused link
into a click with no press before it. That click is offered to `on_link_click` with
the whole link and the modifiers held; claiming it prevents the navigation. With no
claim, the table above decides.

> **Migration note (web).** Before link events, a click on a link in an **editable**
> web editor followed the link as it placed the caret, which was a bug; it no longer
> does. A **read-only** editor used as a document viewer keeps native link behaviour
> as long as it registers no `on_link_click`. Registering one makes the app
> responsible for every link in that editor, middle click and Ctrl/Cmd+click
> included. To open links in an editable editor, register `on_link_click` and open
> `click.link.href` yourself (for example on `click.primary`, the Ctrl/Cmd+click
> convention). Desktop never navigates: there is no browser to follow the link.

**Decorating links with CSS generated content.** A stylesheet rule may add text
around a mark, for example a small "opens outside" arrow after web links:

```css
[data-pm-editor] a[href^="http"]::after { content: "\2197"; font-size: 0.7em; }
```

That text is not in the document, and the caret and a press step over it. The
caret right after the link is drawn before the arrow, as a browser draws it, and
a press on the arrow puts the caret there; a character typed there still goes
after the link (a link does not continue past its end), so it appears after the
arrow. When the arrow wraps onto the next line, the caret after the link stays
at the end of the link's line. A padded link is the exception on desktop: the
caret after it is drawn past its padding, and so past the arrow too, unless the
arrow wrapped. `::before` works the mirror way, on a mark or on a block: the
caret before a link is drawn before its `::before` text, and a caret inside an
inclusive mark (such as bold) at its start after it. On desktop the selection
highlight leaves the generated text out; in the browser the highlight is drawn
from the browser's own `Range` rectangles.

### Autocomplete popups: keys, selection and caret geometry

Four handle methods are what an app needs to run a popup off the editor, like a
`[[` link picker or an `@` mention list. They are the same on desktop and on the
web.

| Method | What it does |
|--------|--------------|
| `on_key(\|key: &EditorKey\| -> bool)` | Offered every key press **before** the editor moves the caret, runs a binding or types. `true` consumes it: the editor does nothing (on the web the `keydown` is `preventDefault`ed and stopped). `false` leaves it to the editor. |
| `on_selection_change(\|sel: &Selection\|)` | Called after every change that leaves the selection different: typing, deleting, clicks, arrows, commands, app transactions, a load that resets it, a peer's edit that maps it. Not called when the selection is unchanged. |
| `on_caret_moved(\|\|)` | Called once the runtime has placed the focused editor's caret somewhere new, **after layout**. The moment to position the popup. |
| `caret_rect(pos) -> Option<ElementBounds>` | Where a caret at `pos` is on screen (zero width), whether or not the caret is there: logical window pixels on desktop, client pixels on the web. The frame of a `position: fixed` popup. It is the text position, as a collapsed `Range`'s rect is in a browser: after spaces hanging past a line's end it is out past the block, where the drawn caret is kept at the block's edge (below). |

`EditorKey` speaks the browser's `KeyboardEvent`: `key` is `"ArrowDown"`,
`"Enter"`, `"Escape"`, `"Tab"` for named keys and the typed text for printable
ones (`"["`, `"A"` with Shift, `" "` for the space bar); `primary` is Cmd on macOS
and Ctrl elsewhere, beside `ctrl`, `meta`, `shift`, `alt` and `repeat`.

```rust
use rinch_editor_core::Pos;

let picker: Signal<Option<Pos>> = Signal::new(None);  // where `[[` began
let popup_at: Signal<(f32, f32)> = Signal::new((0.0, 0.0));

// Open and close from the text before the caret.
editor.on_selection_change({
    let editor = editor.clone();
    move |sel| picker.set(link_trigger_before(&editor.doc(), sel.head()))
});
// Position once the text is laid out.
editor.on_caret_moved({
    let editor = editor.clone();
    move || {
        if let Some(anchor) = picker.get()
            && let Some(r) = editor.caret_rect(anchor)
        {
            popup_at.set((r.x, r.y + r.height));
        }
    }
});
// Drive the list while it is open; every other key stays the editor's.
editor.on_key(move |key| {
    if picker.get().is_none() {
        return false;
    }
    match key.key {
        "ArrowDown" => list.next(),
        "ArrowUp" => list.previous(),
        "Enter" | "Tab" => list.accept(),   // e.g. replace `[[query` with a link via `update`
        "Escape" => picker.set(None),
        _ => return false,                  // typing narrows the list
    }
    true
});
```

Every callback runs with no internal borrow held, so each may call back into the
handle: read the document, run `update(..)` or `toggle_link(..)`, move the
selection. A selection move made from `on_selection_change` calls it again, from
inside itself. Every callback also runs **untracked** (issue #931): an app effect
that calls `set_selection`, runs a `command` or offers a key calls these
synchronously, and the popup's own signals a callback reads do not become that
effect's dependencies. Each is one callback per handle (a second registration replaces the
first), like `on_change`, and costs nothing while none is registered.

**Why `on_caret_moved` and not `on_selection_change` for geometry.** On the web the
browser lays out on demand, so `caret_rect` is always current. The desktop runtime
lays out once per input event, after the editor has handled it, so from
`on_selection_change` or `on_key` a position in the block just edited answers
`None` (it has no layout yet), and so does the new empty paragraph Enter makes. `on_caret_moved` comes from the caret pass after the
layout. The desktop may run a caret pass before an event's layout as well, so a
call can come with `None` for the edited block; the one after the layout follows
in the same event. It is not called for a scroll: the caret moves with the editor
then, and a `position: fixed` popup does not.

**What `on_key` is not offered.** A key while an IME composition is in progress (the
input method owns it; the composed text arrives as a commit, which is typing); on
the web, a soft keyboard's input, which arrives as `beforeinput` rather than keys
(`"Unidentified"` and `"Process"` keys are not offered); key releases. A modifier
pressed alone is offered (`"Shift"`, `"Control"`); ignore it.

**With an input method on** (on Linux, IBus or Wayland `text-input`, which the editor
switches on while it has focus), plain printable characters can arrive as input-method
*commits* rather than as keys, so `on_key` may never see the `[` of a `[[` trigger.
Named keys (arrows, Enter, Tab, Escape) still arrive as keys. Detect a typed trigger
from `on_selection_change` and the document around the caret, which sees the result
however the text came in.

**What runs before `on_key`.** Only a menu shortcut (a chord registered by an app menu
or the DOM menu bar). `on_key` comes **before** the document-wide
`set_keyboard_interceptor` and before the dismiss stack — `Modal`, `Drawer` and
`Popover`'s `close_on_escape`, the DOM menu bar, a `<select>` popup — on desktop and
in the browser alike. So a popup that claims Escape closes itself and leaves a `Modal`
around the editor open; an Escape it does not claim goes on and closes the modal.

**Callbacks follow their component.** `on_key`, `on_selection_change`, `on_caret_moved`
and `on_change` remember the component that was rendering when they were registered,
and stop being called once it unmounts, even if the `EditorHandle` lives on (an
app-level handle handed to a view that comes and goes). A live callback runs inside
that component, so a signal it creates belongs to it. A callback registered outside
any render (from `main`, a timer) keeps app lifetime.

### Margin anchors: drawing beside a line

An app that draws something beside a place in the text — a comment's marker, a
bookmark flag, a change bar — needs to know where that place's line is drawn, and to
follow it through edits, reflows and scrolling. Give the editor the places as **margin
anchors**, listen for where their lines are, and draw your markers in the `Editor`'s
**overlay slot** (its children), which scrolls with the text:

```rust
let editor = create_editor();
let tops = Signal::new(Vec::<(String, f32)>::new());
let right = Signal::new(0.0f32);
let ed = editor.clone();
editor.on_margin_layout(move |lines: &[MarginLine]| {
    tops.set(lines.iter().map(|l| (l.key.clone(), l.top)).collect());
    right.set(ed.margin_text_right().unwrap_or(0.0));
});
editor.set_margin_anchors(vec![
    MarginAnchor::at("c1", Pos(12)),     // a model position, carried through edits
    MarginAnchor::at("c2", Pos(40)),
]);

rsx! {
    // A right padding for the markers: the gutter they sit in is inside the
    // editor's padding box, so a self-scrolling editor (`overflow: auto`)
    // does not clip them. (A marker outside the padding box — a negative
    // `left` — shows only while nothing between it and the page clips.)
    Editor { editor: editor.clone(), style: "padding-right: 32px",
        // Children are the overlay slot: placed in the editor's own scroller,
        // at the frame `MarginLine` reports in.
        for (key, top) in tops.get() {
            div { key: key.clone(), class: "marker",
                  style: {format!("position: absolute; left: {}px; top: {top}px", right.get() + 8.0)} }
        }
    }
}
```

- **`set_margin_anchors(Vec<MarginAnchor>)`** replaces the whole set. `MarginAnchor::at(key,
  pos)` is a model `Pos`, carried through every change the editor applies — local edits by
  their mapping (`at_with(key, pos, MarginAssoc::Before)` keeps it before text inserted right
  at it), a peer's change the way a `SelectionAnchor` is carried. It is lost when the
  document is replaced (`load_doc`/`load_html`) or a peer removes, splits, joins or retypes its
  block. While collaborating, `MarginAnchor::sticky(key, bytes)` takes a sticky index
  (`collab_sticky_index_with`, from this editor or a peer's), resolved afresh on every pass —
  the same character on every replica, and the place its block stood once that block is gone.
- **`on_margin_layout(|lines: &[MarginLine]|)`** hears one `MarginLine { key, top, height }`
  per anchor that has geometry, in the order given: the **line box** of the line it sits in
  (leading included), in the editor container's content coordinates — its padding box, the
  content unscrolled, which is exactly where an overlay-slot child at `top` lands. Anchors on
  one line report the same box. It is called after a layout or overlay pass that changed any
  reported line, not every frame, and once when anchors and a callback are both set on a
  mounted editor (and again after a re-mount); a scroll moves no line and
  reports nothing. An anchor whose block is not laid out (a virtualized block far out of view)
  is left out until it is. In a table cell it is that cell's line; right before a leaf block (a
  rule, an embed) it is that block's box. The callback runs with no borrow held, belongs to
  the component that registered it, and may set the anchors again.
- **`margin_text_right()`** is the right edge of the text column (the first laid-out block's
  border box) in the same coordinates, as of the last report.

Anchors never change the document, its undo history or what collaborators receive, and they
work on a read-only editor. With none set the pass is one check per editor; `N` anchors are
one pass of `N` measures for a layout that could have moved one of their lines, and none for
one that could not: the pass compares, for every block an anchor is in, where it is, its box
and (on desktop) whether its text was laid out again — so a spinner, a hover, a label's text
elsewhere or a keystroke in another editor measures nothing. On the web, which keeps no record
of text layouts, a rewrap that moves no anchored block's box is reported at the next change that
does. On the web the line box is rebuilt from what is on the line (each text run's inline box at
its own `line-height`, each picture's box), so a line a tall picture makes taller is reported
whole; under `line-height: normal` it is within a pixel. On the web a resize that rewraps the text reports too
(the container is watched with a `ResizeObserver` while anchors are set).

The overlay slot is a 0×0 layer at the container's padding-box origin
(`[data-pm-overlay]`). Its children are never part of the document, the editor's diff, its
caret map or its text; a press or a key in them is theirs (a button there runs its handler
and places no caret). The layer sets no `pointer-events` — style your markers as you need.

## Keyboard shortcuts

The editor handles its own keyboard input. Every shortcut below comes from the
**editor-core keymap** (`BaseCommandsPlugin`/`HistoryPlugin`), which both the desktop
and web views consult through one path — add a binding there and it works on every
platform. `Mod` = Ctrl on Windows/Linux, Cmd on macOS.

| Shortcut | Action |
|----------|--------|
| Mod+B / Mod+I / Mod+U | Toggle **bold** / *italic* / underline |
| Mod+E | Toggle inline `code` |
| Mod+Shift+S | Toggle ~~strikethrough~~ |
| Mod+A | Select all |
| Mod+Alt+1…6 | Heading 1–6 |
| Mod+Shift+0 | Paragraph |
| Mod+Shift+7 / 8 / 9 | Task / bullet / ordered list |
| Mod+Shift+B | Blockquote |
| Mod+Shift+L / E / R / J | Align left / center / right / justify |
| Mod+Z / Mod+Shift+Z / Mod+Y | Undo / redo |
| Enter | Split block / new list item (between two blocks: a new empty paragraph there) |
| Shift+Enter | Insert a hard break (line break within the block) |
| Tab / Shift+Tab | Move between table cells, else indent / outdent a list item. Consumed even where neither applies, so Tab never leaves an editable editor; an app that wants it to claims it in `on_key`. In a [read-only](#read-only) editor: move the focus to the next / previous Tab stop |
| Backspace / Delete | Delete backward / forward |
| Ctrl+Backspace / Ctrl+Delete (Alt on macOS) | Delete the word backward / forward |
| Page Up / Page Down (Option+ on macOS) | Move the caret about a page (0.875 of the scroller's height) up / down at its column, scrolling the same amount; Shift extends. At the first / last line: the document's start / end. Read-only: scroll only. Plain Page Up / Down on macOS: scroll only |

The four alignment chords are the Google Docs ones, and some of them are also
chords of the browser or the platform, which may take the key before the editor
sees it. Chrome reserves Ctrl+Shift+J (and Ctrl+Shift+I) for its developer
tools; Ctrl+Shift+R is the browser's hard reload; some Linux input methods
(IBus's emoji picker) bind Ctrl+Shift+E. The commands themselves
(`setTextAlignLeft` / `Center` / `Right` / `Justify`) always work from a toolbar.

The caret keys are not keymap bindings; they are the platform's. On desktop
Ctrl+Left / Ctrl+Right move by word, Home / End go to the visual line's edges and
Ctrl+Home / Ctrl+End to the document's on Windows and Linux; on macOS Option+Left /
Option+Right move by word, Cmd+Left / Cmd+Right go to the visual line's edges,
Cmd+Up / Cmd+Down to the document's, and Home / End leave the caret where it is
(Shift+Home / Shift+End extend to the document's ends) — the same keys a desktop
`<input>` or `<textarea>` takes (issue #951). On the web the word modifier is Ctrl
or Cmd on every platform.

Page Up and Page Down (issue #2020) follow Chrome's editing keys. The page is
0.875 of the editor's nearest scroll container's height — the editor's own when it
scrolls, at most the window's, the window's when nothing scrolls. The scroller
moves a page and the caret moves to the point a page below (above) where it is,
at the column Up and Down keep, so a caret on screen stays where it was there.
The page is measured from the caret, not from the view: after scrolling away
from the caret with the wheel, Page Down still moves it a page below where it
is, and the view follows it (as in Chrome and Firefox). A scroller a
`lock_scroll` overlay holds still is not paged, and nothing outside a
`position: fixed` box (a modal) is: an editor in a modal pages only scrollers
inside the modal. A page that reaches the document's last (first) line, or runs
past the editor, puts the caret at the document's end (start), Shift extends
the selection, and a [read-only](#read-only) editor
scrolls a page and leaves the caret, as a read-only `<textarea>` does in Chrome.
On macOS (desktop, and the web on a Mac) plain Page Up / Page Down scroll and
leave the caret, Option+Page Up / Page Down move it and Shift+Page Up / Page Down
extend. Chrome measures the page on the editable element rather than its
scroller, so an editor inside a taller scroller pages further there than here,
where the page is what is on screen (as in Firefox); and Chrome scrolls the
caret's line to the top, where here the caret keeps its place on screen (as in
Firefox). A virtualized editor pages into blocks it has not laid out yet: the
page scrolls first, and the caret lands once they are.

The word-delete chord is the one binding whose modifier differs by platform: on macOS Cmd+Backspace
is the line delete, so the word deletes are Alt(Option)+Backspace / Alt+Delete
there. The desktop binding is chosen at compile time. **On the web the keymap binds
no word-delete chord**: the browser already turns the platform's chord into a
`beforeinput` `deleteWordBackward` / `deleteWordForward`, which the web view runs
as the commands of the same name, and leaves Cmd+Backspace to its own line delete.

(Copy/cut/paste — Mod+C/X/V, and Mod+Shift+V for paste-as-plain — are handled by the
platform clipboard, not the keymap.) On desktop the same four operations — cut, copy,
paste and select all — are also on the built-in right-click menu of the editor
(issue #813), and the menu items run exactly the code the chords run; see
[Right-clicking a text field](focus.md#right-clicking-a-text-field). On the web the
browser's own menu covers the editor surface. Undo/redo is a single, exact history: each undo
reverses one logical edit (typing — and an inline picture or plain-text paste landing in
it — is merged into a group until a pause of 500 ms, a caret move, an undo or redo, or a
structural edit such as a split, a mark or a block change), because every edit is an
invertible step.

### What a paste does

A paste is the clipboard's `text/html` parsed against the schema (unknown tags and
attributes dropped), else its `text/plain`, one paragraph per line. The content is
then **fitted** to the caret, as ProseMirror and TipTap do it:

- **Text and inline content** go in at the caret. Several paragraphs: the first
  continues the line, the last takes the text after the caret.
- **A list** pasted on an empty line is that list — bullet, ordered (with its
  `start`) or task (with its checkboxes). Pasted inside a line of text, its first
  item's text continues the line, the other items are a list below it, and the text
  after the caret joins the last item. Pasted in a list item, its items become sibling
  items of the list the caret is in; on an empty item they take its place. That is
  between bullet and ordered lists, whose items are the same node (the target list
  keeps its kind). A task list and a plain list do not mix: one pasted in an item of
  the other is a list nested in that item.
- **A heading, a quote or a code block** pasted on an empty line is that block;
  inside text a heading is its text, and a code block is a paragraph for each of its
  lines (the first continues the line; one copied line of code is just text in the
  line), never one paragraph holding line ends — wherever the caret is in a line
  that has content, its start included. A line selected whole counts as empty: the
  code block takes its place. A selection that runs into another line does not: the
  code is a line each, and what is left of the last selected line joins the last.
- **A rule or a table** splits the line: the text before the caret, the block, the
  text after. At the start or end of a line nothing is split and no empty line is
  left.
- **In a table cell** all of this happens inside the cell.
- **Over selected table cells** the selected cells are emptied and the content goes
  into the top-left one, as typing over them does. The table's rows and columns are
  never changed by a paste.
- **In a code block** the paste is the clipboard's plain text, exactly, line breaks
  included.
- **Over a selection** the selected content is removed first; a selection of a whole
  line counts as an empty line.

Copying from inside a list copies the list around the selected items, so the same
kind of list arrives wherever it is pasted, in rinch or elsewhere. A selection inside
one line copies as its text and marks only.

Lists copied from a browser, GitHub or Google Docs paste as lists, nested ones
included: Docs writes a nested list beside the item it belongs to
(`<ul><li>a</li><ul><li>b</li></ul></ul>`), and it is read as that item's. Nothing
in a list is dropped for not being an `<li>`. A link or other mark around whole
blocks (a card that is one `<a>` around a heading and a paragraph) is kept on the
text of those blocks. A `<b style="font-weight:normal">`, which Docs wraps its
copies in, is not bold. All of this is the HTML reader, so `load_html` and
`content:` read the same way. (Before #1382 HTML
that began with a `<meta>` tag, as what Chrome and Firefox copy does, was read as
empty, and the paste fell back to its plain text.)

A paste the document has no valid place for falls back to the plain text; if that has
none either, nothing happens. One paste is one undo step.

### What the HTML reader keeps

`load_html`, `content:` and a paste read HTML the same way, and the reader is
**total about text**: no markup is refused, the text of every element is in the
document — the elements listed under "Hidden text" below aside — and the result is
always valid content. Structure it cannot keep is approximated; text is not dropped.
It is not a browser: it reads no CSS, so text hidden by `display: none`, `hidden`
or a closed `<details>` is read, and whitespace is kept as written.

- **Lines are where a browser puts them.** A block-level element ends the line
  before it even when it is empty (`a<div></div>b` is two paragraphs), and is a
  line itself when it holds something: a `<div>` holding only spaces is none, one
  that holds a non-breaking space is a blank line (and so, unlike in a browser, is
  one whose space is inside an inline element: `<div><span> </span></div>`). An inline element that holds a block
  ends no line by itself: `x<b>y<div>z</div>v</b>w` is `xy`, `z`, `vw`.

- **An element the reader does not know** — Word's `<o:p>` and `<st1:place>`, a
  custom element, `<button>`, `<label>` — is read through. It is inline unless it
  holds a block, so `a <my-chip>b</my-chip> c` is one line and
  `<my-card><h2>…</h2><p>…</p></my-card>` is its heading and paragraph. (Before
  #1397 the reader stopped at the first `<o:p>`, and a paste from Word kept its first
  paragraph only.)
- **Hidden text is not read:** comments, conditional comments
  (`<!--[if gte mso 9]>…<![endif]-->`), `<style>`, `<script>`, `<head>`, `<title>`,
  `<template>`. Not read either, though a browser may show something for them: an
  `<input>`'s value, the options of a `<select>`, the fallback content of
  `<object>` / `<video>` / `<canvas>`, and `<svg>` and `<math>` (on a clipboard those
  are icons and the hidden copy of a rendered formula). An `<svg>` or `<math>` that
  is never closed ends at the first HTML element after it (`<p>`, `<div>`,
  `<span>`, …), as in a browser, so it does not take the rest of the paste with it;
  an unclosed `<select>`, `<object>` or `<video>` does, where Chrome shows what
  follows. An `<img>` with an empty `src` is no image.
- **Broken markup is read as a browser reads it:** a `<p>` ends at the next block, an
  `<li>` at the next `<li>`, a cell at the next cell; an end tag with nothing to close
  is skipped, except a `</p>`, which ends the line as it does in a browser
  (`<p>a<ul>…</ul>c</p>d` reads `c` and `d` on two lines); a `<` that starts no tag
  (`a < b`) is text.
- **A bold, italic, link or code end tag written across a block**
  (`<b>a<div>b</b>c</div>d`) ends the formatting where it is written, and what the
  block already held keeps it: `a` and `b` are bold, `c` and `d` are not (#1410; the
  bold used to run to the end of the document). As in a browser, such an end tag
  is ignored when it would reach out of a table cell or across more than seven open
  blocks. One browser rule is not followed: formatting that a block's end cut short
  is not carried on after it, so `<p><b>a</p>b` has a bold `a` and a plain `b`
  where a browser bolds both (#1445).
- **Character references** are decoded as a browser decodes them (#1415): numeric
  ones (`&#233;`, Word's `&#146;`), every name of the HTML standard (`&eacute;`,
  `&plus;`, `&check;`, `&AMP;`), and the older names with no `;` (`&copy 2024` is
  `© 2024`, `&notit;` is `¬it;`). In a link's address a name with no `;` is left
  alone before `=`, a letter or a digit, so `?a=1&copy=2` stays a query string.
  Anything else after an `&` is text (`R&D`, `&nosuch;`).
- **Table parts with no `<table>`** are a table: a run of `<td>` is one row, a run of
  `<tr>` the rows of one table (#1392; they used to load as an invalid document and
  paste as plain text). What has no place in a table — text between its rows, a
  `<caption>` — becomes paragraphs in front of the table. That is where a browser
  shows it for a written `<table>`; for parts with no `<table>` a browser has no
  table to put it in front of, and the order can differ from both the source and the
  browser (`<td>w1<tbody>w2` reads `w2`, then `w1`). No text is lost.
  Parts that do not make full rows (`<td>A</td><td>B</td><tr><td>C</td></tr>`) are
  padded with empty cells to a rectangle, as a browser renders them — up to eight
  empty cells for each cell written (sixteen at least). Parts whose spans ask for
  more (`<td colspan=1000>` over a few short rows) are left as written, short rows
  and all. A `<table>` is always read as it is written, short rows included.
- **A heading around blocks** (`<h2><div>…</div><ul>…</ul></h2>`) keeps the blocks;
  the text between them is headings.
- **A mark around blocks** marks their text and images, not their line breaks, as it
  does around a line (#1401). Of two links or two colours, one inside the other, the
  text has the inner one.
- **Nesting** is kept 192 elements deep (a list level costs two or three, a quote
  one). Deeper elements are flattened into the 192nd: every block stays a line of
  its own with its text, in order; what is lost is the nesting, the marks of inline
  elements that deep, and the attributes of a block's later parts when it is split
  around the blocks inside it (the first part keeps them). Nothing overflows, and
  reading stays linear in the size of the input. A document nested deeper than that does
  not read back as itself from its own HTML — the same lines, less nesting.

What the applications people paste from write, and how it reads:

| From | Reads as |
|---|---|
| Word | Headings, paragraphs, tables, links, bold. A Word list is paragraphs that start with their bullet or number (`·`, `1.`): that is how Word writes it, and it is kept that way rather than rebuilt into a list. |
| Excel, Google Sheets | A table (Sheets' `<google-sheets-html-origin>` wrapper and Excel's `<col>` are read through); one Sheets cell is a line of text. |
| Google Docs | Headings, paragraphs, links, nested lists. Bold, italic and underline are not read: Docs says them with CSS on a `<span>` (#1407). Text keeps the colour Docs gives it. |
| Notion, GitHub, Slack, Apple Notes and Mail, Gmail | Their headings, lists, quotes, code blocks, tables and links; a `<div>` per line is a paragraph per line. |
| VS Code | A **code block**: VS Code copies a `<div style="white-space: pre">` holding one `<div>` per line, and a code block is what keeps the lines and their indentation. The token colours are not kept. Pasted inside a line of text, one copied line is its text, and several lines are a line of text each: the first continues the line, the text after the caret joins the last. |

Whitespace is kept as written, not collapsed as a browser collapses it, so a source
that wraps its lines inside a paragraph (Word does) reads with those line ends in the
text (#1406).

The rules are `Transaction::replace_selection`'s; see
[Fitting a slice](editor.md#fitting-a-slice-replace_range).

### Pasting is asynchronous

Reading the clipboard is a request to another application, and that application
can be slow or hung — on X11 the read waits up to four seconds. Ctrl+V therefore
does **not** block: the key is consumed immediately, the clipboard is read on a
worker thread, and the content is inserted when it arrives (issue #149). The
editor stays live throughout, which means the user can keep typing while a slow
paste is in flight.

Where does the content land, then? At the position the paste was **asked for**,
carried across whatever was typed in the meantime. Ctrl+V captures a
`SelectionAnchor`, every intervening transaction maps that anchor forward through
its steps, and the insertion happens there. Type ahead of it and the paste still
splits the text where you originally pointed; type after it and the paste is
unaffected; move the caret about and nothing happens to it at all — a
selection-only change is not a document change.

If the document is *replaced* while the read is in flight (`load_doc` /
`load_html`) the anchor reports `None` and the paste is dropped: the content it
was aimed at no longer exists, and reusing the raw offset would drop it into
unrelated text.

A collaborating peer's edit is not a replacement. The anchor is carried across
it as across a local edit: text a peer types elsewhere shifts it, and text a
peer types in the anchor's own paragraph leaves it at the same place in the
words. It reports `None` only when the peer removed, split, joined or changed
the kind of the block it sat in (a paragraph deleted, Enter pressed inside it,
a paragraph made a list item): there the place can no longer be named.

The same anchor is available to your own asynchronous insertions — an uploaded
image, a completion from a model:

```rust
let anchor = editor.anchor_selection();
let editor = editor.clone();
fetch_something(move |content| {
    // ... back on the UI thread ...
    if let Some(sel) = anchor.selection() {
        editor.set_selection(sel);
        editor.replace_selection_with_text(&content);
    }
});
```

## Markdown shortcuts

As you type, the editor rewrites markdown shortcuts in place (the default
`MarkdownInputRulesPlugin`, on by default). Block shortcuts fire on a space at the
start of a line; inline mark shortcuts fire when you type the closing delimiter:

| Type | Becomes |
|------|---------|
| `# ` … `###### ` | Heading 1–6 |
| `` ``` `` | Code block |
| `---` / `***` | Horizontal rule (fires on the third character, no space) |
| `> ` | Blockquote |
| `- ` / `* ` / `+ ` | Bullet list |
| `1. ` | Ordered list |
| `[ ] ` / `[x] ` | Task list (unchecked / checked) |
| `**bold**` / `__bold__` | **bold** |
| `*italic*` / `_italic_` | *italic* |
| `~~strike~~` | ~~strike~~ |
| `==highlight==` | highlighted |
| `` `code` `` | inline `code` |

The horizontal rule fires only when the marker is the paragraph's **whole**
content: `---` typed in front of existing text, or in a heading, stays text. The
caret lands in the block after the rule, or in a fresh empty paragraph appended
after it (inside the same blockquote or list item) when nothing follows.

Inside a task list, **Enter** adds a new (unchecked) item, and **Enter** on an empty
item exits the list — just like bullet/ordered lists. **Click a task's checkbox** to
toggle it done (works on desktop and web). To add your own shortcut, append
a `mark_input_rule` / `wrapping_input_rule` / `textblock_type_input_rule` to
`markdown_input_rules()` (or contribute an `input_rules()` set from your own plugin).

## A complete toolbar

A full example pairing a command toolbar with the editor lives at
`examples/markdown-editor/src/main.rs`, and `examples/ui-zoo/src/sections/editor.rs`
shows the same pattern inside the component showcase. The shape is always:
`create_editor()` once, clone the handle into each button's `onclick`, and place a
single `Editor {}` for the surface.

> **Put `data-nofocus` on the toolbar.** A focusable toolbar button takes the
> keyboard away from the editor when it is pressed, so Bold would apply to a
> selection that no longer has focus. `data-nofocus` on the toolbar container
> makes every control inside it take its click without taking the keyboard —
> the `preventDefault()`-on-mousedown answer browsers converged on, and it works
> on both backends. See
> [Taking the click without the keyboard](focus.md#taking-the-click-without-the-keyboard).
>
> ```rust
> div { data-nofocus: "", class: "toolbar",
>     button { tabindex: "0", onclick: move || ed.command("toggleBold"), "B" }
> }
> ```

## On the web (rinch-web)

The editor runs in the browser too — the **same** `Editor {}` / `EditorHandle` /
`create_editor()`, with **identical app code**. The renderer-agnostic view lives in
`rinch-editor-view` and projects onto `rinch-web`'s `web_sys` DOM (the model is the
single source of truth; the container is deliberately **not** `contenteditable`).
`rinch-web` re-exports the editor, so a web app just imports it:

```rust
use rinch_web::{Editor, create_editor};

#[component]
fn app() -> NodeHandle {
    let editor = create_editor();
    let ed_bold = editor.clone();
    rsx! {
        div {
            button { onclick: move || { ed_bold.command("toggleBold"); }, "Bold" }
            Editor { editor: editor.clone(), content: "<p>Edit me in the browser.</p>" }
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    rinch_web::mount(ThemeProviderProps::default(), app);
}
```

A runnable demo is `examples/editor-web` (built with `trunk serve`). The browser
build links **no** `rinch-dom`/Parley/CRDT engine — the browser handles layout, text,
and painting.

**Supported today:** typing, the full command/toolbar surface, keyboard shortcuts,
caret + selection rendering (pixel-accurate overlays), click / double-click (word) /
triple-click (block) / shift-click / drag selection, arrow / word / Home-End /
vertical navigation, **clipboard (copy / cut / paste — rich `text/html`, image, or
plain text)**, and **IME composition** (the preedit overlay matches the composing
block's font).

Clipboard and IME ride a focused, off-screen **hidden `<textarea>`** capture target
(created on first editor focus): a plain non-`contenteditable` `<div>` receives no
`paste`/`cut`/`compositionstart` events, so the editor focuses the textarea to make
the browser route those native events to it — which also makes focus browser-native
so keys can't reach the wrong control. Typed characters are still consumed by the
editor's key handler (and never reach the textarea); only IME composition flows
through it. This mirrors the CodeMirror / ProseMirror hidden-input technique.

**The editor owns a key only while that textarea holds focus** (issue #271). Tab to a
button, press a focusable control (a `<button>`, a link, a `tabindex` element, a text
field), or press blank page, and the editor lets go: the next key — Enter and Space
included — belongs to whatever has focus now, and the editor stops painting its
caret, as a blurred editor does on desktop. Press inside the editor to give it the
keyboard back. A switch to another window and back is meant to leave it in place: the
textarea stays the page's focused element, and the release waits for focus to move
within the page.

A press on a **non-focusable** clickable — a DOM menu-bar item, a `div { onclick }`
toolbar button — leaves the keyboard with the editor, as desktop does for such a
press. A focusable one takes it, `DropdownMenu`'s `<button>` items included, as on
desktop. A toolbar of real `<button>`s
that should leave the keyboard with the editor carries `data-nofocus` (see the
toolbar note above); the editor-web example's does. A command run while the editor
keeps the keyboard with no pointer event at all — assistive technology or
`element.click()` on a toolbar button — still moves the caret with its edit.

**Right-click gets the browser's own editing menu** (issue #814) — Paste, Cut, Copy,
Select All, and whatever else the browser puts there (emoji, extensions; the hidden
field has spellcheck off, so there are no spelling suggestions). The editor draws no
context menu of its own on the web: a page's paste needs
`navigator.clipboard.readText()`, which prompts. Instead, the same hidden textarea
is what the browser's menu is built for. The editor's surface is not editable as
far as the browser is concerned, so a right-click there used to open the menu for a
plain element, with no Paste; now a right-button press parks the capture textarea
under the pointer — invisible, but hittable — for the instant the browser needs to
fire `contextmenu` and hit-test the point for its menu (CodeMirror 5's technique),
then sends it back off-screen 100 ms later, whether or not a menu came, so a click
on the page after that reaches the page. (A click inside the textarea's 30 px box
within those 100 ms goes to the textarea instead.) The menu's items fire the ordinary `paste` /
`cut` / `copy` events on the focused textarea and are answered from the editor's
model: a paste lands at the editor's selection, Cut and Copy act on the editor's
selection even when it spans blocks, and Select All — which the browser can only
apply to the textarea — is detected and becomes the editor's `selectAll`, however
long the menu stayed open before it was chosen.

Where the right press lands decides the rest:

- **Inside a non-empty selection** — on its text, a link or an image in it — the
  selection is kept and the menu is the editing one, so Cut, Copy and Paste act on
  the selection.
- **On a link or an image outside the selection** (or with only a caret) nothing is
  parked, so the browser's own link or image menu opens — Open link, Copy link
  address, Save image, Copy image — and there is no Paste at that spot. The caret
  moves onto the link, and over an image the selection stays where it was: that is
  what a native `contenteditable` does in Chrome.
- **On a horizontal rule** outside the selection, the rule is selected, and the
  menu's Copy and Cut act on it.
- **Anywhere else** outside the selection, the caret moves to the press point first.

**Undo and Redo in the menu act on the editor**, but Chrome enables them only when
its own page-wide undo stack has a step — after an IME commit in the editor, or
after typing into another field on the page — so they are usually greyed out, and
Redo stays greyed after a menu Undo. Use Ctrl+Z and Ctrl+Y. (Chrome aims the menu's
Undo at whichever field owns that step; the editor takes it back while it has
focus, so the other field is not undone.)

The keyboard's menu key and Shift+F10 open the same menu at the caret: the browser
sends their `contextmenu` to the focused element, which is the textarea, and the
editor parks it with its left edge at the caret first — at a selected horizontal
rule, which has no caret, it parks at the rule, and with neither a caret nor a
selected node — Select All in a document that ends with a rule — at the editor's
own box. On macOS a Control-click opens the menu like a right press, and the word a
Mac right-click selects under the pointer is not mistaken for Select All; neither is
verified on a Mac. An element up the chain carrying a live `data-oncontextmenu`
still wins, as it does over any other element it wraps: over the whole editor,
links and images included, and for the menu key and Shift+F10, which dispatch that
handler with its click context at the caret and open no browser menu. What was measured, in Chrome 153: a real right press makes Chrome's own
`contextmenu` target the parked textarea, unprevented, where before it targeted the
paragraph (outside a selection it still targets the `<a>` or `<img>`) — Chrome
builds its menu for the element its hit test finds there, so the editing menu
follows from that. Firefox and Safari are unverified.

## Collaboration (optional, `collaboration` feature)

Two editors can share one live document. Enable the `collaboration` feature and the
editor projects every local edit onto a [yrs](https://github.com/y-crdt/y-crdt) (Yjs)
CRDT, broadcasts the resulting delta, and rebuilds the model from a peer's delta when
one arrives — so concurrent edits converge. The CRDT adapter
([`rinch-editor-collab`](https://docs.rs/rinch-editor-collab)) is the only thing in a
rinch app that links a CRDT engine; default builds link none of it.

```toml
rinch = { workspace = true, features = ["desktop", "collaboration"] }
```

One peer **hosts** (it owns the starting document); the others **join** from a
snapshot of it. Each side supplies an `outbound` closure — where to send a delta a
local edit produced — and feeds a peer's delta back in with `collab_receive`:

```rust
// Host: project the current document onto a fresh CRDT and hand peers a snapshot.
let snapshot = host.start_collaboration_host(move |delta| transport.send(delta))?;

// Guest: adopt the host's document and start collaborating.
guest.start_collaboration_guest(&snapshot, move |delta| transport.send(delta))?;

// When a delta arrives from the network, apply it on the main thread:
guest.collab_receive(&delta_bytes);
```

On desktop, network callbacks usually arrive on a background socket/data-channel
thread — use the `Send`-safe entry point, which marshals the delta onto the main
thread for you:

```rust
use rinch::prelude::*;
post_remote_delta(editor_container_id, delta_bytes); // any thread → main
```

**The transport owns relaying.** `outbound` fires only for an editor's own local
edits — a delta that arrives through `collab_receive` is never re-broadcast (it's
already in the shared CRDT; echoing it back would loop). So the transport must be a
**full mesh** (every peer's `outbound` reaches every other peer directly) or a **hub**
that fans each delta it receives out to the other peers, forwarding the raw bytes
unchanged. A chain — A wired to B, B wired to C, with nothing joining A and C —
silently partitions: C never sees A's edits, and nothing errors.

**Undo reverts your own edits.** A peer's edit never enters your undo history, and
your undo steps follow the text through peers' changes: Ctrl+Z after a peer typed
before, after or inside your edit, bolded or retyped your paragraph, wrapped it in a
quote or moved a block past it, reverts your edit where it now is (#1249). A step
whose text a peer deleted is skipped, and so is one that no longer fits (re-making a
code block of a paragraph a peer has since made bold); the rest of that undo still
applies, and the undo is used up. One limit: the editor tells your text from a peer's
by comparing the document before and after their change, so where the two hold
identical copies (the same word pasted beside yours, two empty paragraphs made at
once, line breaks side by side) an undo can land on the peer's copy (#1895).

If a peer might have missed deltas — offline, reconnecting, or polling over HTTP with
no persistent connection at all — reconcile with a **state vector + diff** exchange
instead of relying on delta delivery being perfect:

```rust
// The peer that might be behind sends its state vector...
let my_sv = editor.collab_state_vector().unwrap();
transport.send(my_sv);

// ...the other side answers with what it's missing, plus its own state vector...
let diff = peer.collab_sync_diff(&my_sv).unwrap();
let peer_sv = peer.collab_state_vector().unwrap();
transport.send((diff, peer_sv));

// ...and the first peer applies it (and can answer the second peer's state vector
// the same way, to reconcile in the other direction too):
editor.collab_receive(&diff);
```

`collab_receive` is the same entry point for a broadcast delta and a reconciliation
diff — they're the same wire format, so there's no separate sync protocol and no
per-peer state to keep on either side; a stateless server can answer each state
vector it's handed with nothing held between requests.

**Never use state-vector equality as a convergence check.** A state vector counts
*insertions* only, so a deletion — or a mark *removal* (yrs un-formats a mark by
deleting its format marker) — leaves it unchanged, and two editors can hold different
documents behind equal state vectors. Always request and apply a diff instead; the
diff reply always carries the full delete set, which is what actually converges the
peer.

`is_collaborating()`, `stop_collaboration()`, `collab_snapshot()` (a fresh snapshot
for a *late*-joining peer to `start_collaboration_guest` from), and
`collab_take_error()` round out the API. The first milestone covers **flat
text-blocks + marks** (paragraphs, headings, code blocks, bold/italic/link/…),
list containers (bullet/ordered lists and list items) and block quotes, nested
into each other to any depth, tables (merged cells included, anything above inside a
cell), horizontal rules, embeds ([an app's own block](#embeds-blocks-an-app-draws)), and the inline atoms inside a
line — images and hard breaks (Shift+Enter). Task lists, and a ragged table an app's own
transaction builds, are still outside it (a pasted table with ragged rows is padded in
the paste, #1847): an edit
outside that scope fails loud rather than silently diverging —
`collab_take_error()` surfaces it, and the CRDT is left untouched (the local edit is
not projected). Horizontal rules, embeds, images, hard breaks, block quotes and tables all
joined that scope without a new format tag, so **every peer on a document must be upgraded
together**: an older build accepts a rule, an embed, an image, a hard break, a quote or a table
from a newer peer and then cannot read it, which poisons its session (see below) in both
directions for as long as that content remains in the document — for a rule, an embed, an
image or a hard break it heals when the last one is deleted. (An embed is new in a build
that has node views: before any peer inserts one, every peer must run such a build.) A quote may never
leave: one emptied by two people deleting its contents at the same time is no
longer shown, but it stays in the shared document for good (deleting it could race
a third person typing into it), so once that has happened an older build stays
locked out of the document for its whole life. A peer joining from a snapshot that
already holds one fails the join instead.

Wrapping a paragraph in a quote or a list, or lifting it out of one, replaces that
paragraph in the shared document, so if someone else is typing in it at that moment
their typing is lost; both editors still end up with the same document.

Tables keep a full grid however their structure is edited at once: a row and a column
added by two people meet at an empty cell, two columns added at once are both there in
every row, and a row deleted while someone types in it is gone with that typing. What
can still be lost is typing in a row, column or cell someone else deletes or merges
away, and, when two people type at once into the same empty cell that such a
concurrent row-and-column insert created, one of the two. Rows, columns and paragraphs that look
alike are told apart by which one was actually edited, not by their content: deleting
the first of three empty rows while someone types in the last keeps their typing.

A table read from someone else that would be absurdly large for what the shared
document actually holds (rows and columns cost a few bytes each to send, the empty
cells between them nothing) is not built: it shows up as a single empty cell instead,
and while the document holds one, **the editor is frozen**: it refuses every edit, as
a read-only editor does (typing, commands, paste, undo and `load_doc`/`load_html` do
nothing and answer `false`; the caret, selection and copying still work), while other
people's edits keep arriving. `collab_outbound_stall()` reports
`CollabError::OversizedTable` naming the table; show it — "a collaborator added a
table too large to load; delete it to keep editing". No edit is allowed near the
table — not even deleting it with the editor — because the empty cell is not the
table, and a change to it (or a document reloaded from an export, which cannot tell
it from an ordinary empty table) could delete the real one for everybody; and none is
kept to send later, because the next change from anyone else would wipe it. Two
things end it: `collab_oversized_tables()` lists the tables and
`collab_delete_oversized_table(id)` deletes one from the shared document for every
peer (it works while frozen); or someone else deletes or shrinks it. Nothing typed is
lost: nothing was accepted. One person's edits never make such a table;
two people adding hundreds of rows and hundreds of columns at the same moment could.

A table you make yourself whose merged cells span far more rows and columns than it
has cells (thousands of them) is refused before it is shared, and the session reports
it as not syncing until you remove it.

An embed's `data-*` attributes merge one by one as well: two people changing different
attributes of one embed at the same moment both keep their change, two changing the same
one end up with one of the two values, and one removed while someone else changes another
stays removed. Deleting an embed wins over a change to it made at the same moment.

An image's attributes merge one by one: two people changing different attributes of
one image at the same moment (one its `alt`, the other its `title` or an app's
`data-*` attribute) both keep their change, and two changing the same attribute end up with one
of the two values. An image is known by its type and its `src`: changing its `src`
makes it a new image, and a change someone else makes to the old one at that moment is
dropped. A change to an image's attributes is kept while someone else types beside it,
presses Enter or Backspace anywhere in its line, drags it, inserts or deletes another
image next to it (an identical one included), copies it elsewhere, makes the line bold
or turns it into a heading; if they delete the image, it is deleted. Such a change
never shows on another image. Some cases lose it, and both editors still end up with
the same document when they do: **changing the image's `src`**, or **pasting any
picture over it** — one with the same `src` included — at that moment (a pasted
picture is a new one, and a change dropped is better than a change shown on the wrong
picture); **moving the image into or out of a table cell**; **cutting the
image and pasting it back**, or **deleting it and undoing the delete** (the image comes
back as a new one: only a move made in one step, such as a drag, keeps it);
**dragging an image after copying it within the editor**, until either copy is changed
(the two are one image to the editor, so which one moved cannot be told); and
**loading a document** over it. Two people moving one image at the same moment (both
pressing Enter before it) end up with two copies of it.

Every person sharing a document must run a version with per-attribute image merging
before any of them changes an image's attributes or moves an image: an older version
refuses such an image and stops syncing until it rejoins on a newer one.

Typing right after a link while someone else changes that link at the same moment
keeps their change — a new `href`, removing the link, or extending it over the text
after it — but the character you typed may end up **inside** their link rather than
plain, because it sits exactly where their change starts or ends. The same is true when they link the text right after yours to
something else. Nothing is lost and both editors converge; the typed character is
just formatted where you did not mean it (#923). A link is never brought back over text
someone unlinked, and never spreads over text nobody linked.

A runnable two-pane loopback (both editors in one window, no network)
lives at `examples/collab-editor-demo/src/main.rs`.

**Outbound stalls: an out-of-scope edit, and how it un-sticks.** The local edit that
was refused is still in *your* document — only the CRDT declined it — so from that
moment your editor holds content collaboration cannot express. Outbound is
**stalled**: that edit and every one after it stays local, while inbound keeps
working normally and the shared document stays healthy.

`collab_outbound_stall()` reports it. Unlike `collab_take_error()`, which is a
one-shot event you take and clear, this is the *state* — `Some(err)` for as long as
the condition holds — so it is what to drive a persistent indicator from:

```rust
if let Some(err) = editor.collab_outbound_stall() {
    // e.g. "Not syncing — remove the task list to resume." The error names the
    // content: "collab does not support this content yet: node `task_list` …".
    show_banner(&err.to_string());
}
```

The cure is to remove the offending content — undo the paste, remove the task list. You do
not have to re-send anything: the next edit that projects re-bases on the CRDT and
broadcasts **everything** that accumulated during the stall, in one delta. Nothing typed
while stalled is lost (issue #220).

This is not `SessionPoisoned` and needs no rejoin — see the next paragraph for that.

**Errors: transient vs poisoned.** Most collaboration errors are *transient*: an
undecodable blob from the transport, a local edit outside the staged scope, or a
rebuild still waiting on an out-of-order delta's missing dependency — the session
keeps collaborating, and `collab_take_error()` tells you what was refused. One
class is not: inbound bytes that, once integrated, leave the shared CRDT document
**unprojectable with nothing pending that could cure it** (e.g. bytes from a
foreign, non-rinch yrs document, or a peer delta carrying content this build
cannot read back). yrs has no rollback, so such a session cannot receive — and,
before issue #196, it would keep *broadcasting* while receiving nothing, silently
partitioning the peers. The session now **poisons** itself instead: sticky
`CollabError::SessionPoisoned` on every affected call, in **both** directions
(local edits stop being projected/broadcast, and receives keep failing — though
they are still *attempted*, so an inbound update that makes the document
rebuildable again clears the poison on its own). A heal re-syncs the editor to
the converged shared document, discarding any local edits made during the
poison window — each was already refused loudly when it happened — the same
semantics as stopping and rejoining.
`is_collaboration_poisoned()` queries the state; the recovery in practice is
`stop_collaboration()` followed by rejoining from a healthy peer's snapshot
(`collab_snapshot()` → `start_collaboration_guest`).

**A read-only collaborator.** [`set_read_only(true)`](#read-only) on a collaborating
editor makes it a live view of a document other people are writing: `collab_receive`
(and `post_remote_delta`, and a reconciliation diff) integrates and re-projects as
ever — remote integration does not pass through the gate local edits pass through, so
there is nothing to switch off and on around it — while no local change is recorded
onto the CRDT and `outbound` never fires. Set it before or after joining, and flip it
mid-session when a role changes. Two consequences worth knowing:

- `load_html` / `load_doc` on a **collaborating** read-only editor are refused
  (`load_html` answers `false`): with a session attached a load is a write to the
  shared document. `stop_collaboration()` first, or join the next document with
  `start_collaboration_guest`, which adopts it without writing anything.
- `collab_sync_diff` is a pure read and still answers. A read-only client that was
  editable earlier in the session can hold edits its server lacks; whether to send
  that diff is the app's call (a server that enforces the role will refuse it).

### Sticky positions (deep links)

A `Pos` is a number into *this* editor's document *now*: the next edit before it, yours
or a peer's, makes it point somewhere else. To remember a place in a shared document (a
link to a paragraph, a bookmark, a comment anchor), ask for a **sticky index** instead.
It follows its character through every later edit, on every replica:

```rust
// Where the caret is, as an address that survives edits.
let anchor: Vec<u8> = editor.collab_sticky_index(editor.state().selection.head()).unwrap();

// Later, on this editor or on any peer's:
if let Some(pos) = editor.collab_resolve_sticky(&anchor) {
    editor.set_selection(Selection::cursor(pos));
}
```

What it survives and what it does not:

- **Any edit elsewhere**, local or merged from a peer: it moves with its character.
- **Its own character deleted:** it resolves to where that character was.
- **Its block deleted:** `collab_resolve_sticky` answers `None` (an index from
  `collab_sticky_index_with` is still located, displaced: see below).
- **A split or a join that moves its character.** The projection has no move: a split
  (Enter) keeps the longer half of the block in place and writes the shorter half
  into a new block, and a join (Backspace at a block's start, Delete at its end) keeps
  the block holding more of the joined text and writes the other block's text into
  it. An index on the characters that were written anew resolves to the split point,
  or to `None` for a join. An index on the characters that stayed follows them.
- **Enter at the start of a paragraph, or Backspace deleting the empty line above
  it:** the empty paragraph is the block inserted or deleted, so the paragraph keeps
  its text and every index into it (#1850).
- **Toggling a bullet list on a paragraph** replaces the block with one of another
  kind: `None`, at the top level or inside a list, a quote or a table cell.

Choosing which half moves trades one concurrent edit against another, since a
peer's edit in the half that moved lands on the old copy:

- A peer typing at the start of a line that another peer splits in the middle, head
  shorter than tail, sees the typed character follow the tail.
- **If A types into a line while B empties it and joins the next line into it** (B
  selects from that line's start to the next line's start and deletes), B's join
  keeps the next line's block and deletes the emptied one, so **A's character is
  lost**. Before #1850 the emptied line's block was kept and the next line's text
  written into it, so A's character survived (and the next line's text was the copy
  a concurrent edit of it missed).
- Two peers moving the same text at once (both pressing Enter at one place, both
  joining the same two lines) each write their own copy, and the text appears twice
  (#1857).

`collab_sticky_index` answers `None` when not collaborating, for a position that is
not inside a textblock (between two blocks), for the empty starter paragraph of a
shared document that has no blocks, and while the session is stalled or poisoned.

**The bytes are plain yrs.** They are a yrs `StickyIndex` in its v1 encoding
(`StickyIndex::encode_v1`), created on the `text` of the textblock holding the position,
with `Assoc::After` (or `Assoc::Before` at the very end of a text; an empty text names
the text itself). Nothing wraps them. So an app that keeps the shared document outside
the editor (a server, an index) can resolve one with yrs alone:
`StickyIndex::decode_v1`, then `get_offset` on a transaction of the document, then
check that `offset.branch` is the `text` of a block still under the `content` root.
`offset.index` counts UTF-16 code units, like every index of this document, **but only
if you built your `yrs::Doc` for it.** The offset unit belongs to each `Doc` and is not
carried in the bytes, and `Doc::new()` defaults to `OffsetKind::Bytes`, under which
`get_offset` gives a number in neither unit once the text has been edited (7 where the
UTF-16 answer is 4, after typing `😀ö ` before the position). Build the replica the way
the adapter builds its own:

```rust
use yrs::{Doc, OffsetKind, Options};

let doc = Doc::with_options(Options {
    offset_kind: OffsetKind::Utf16,
    ..Default::default()
});
```

The
[`CollabDoc::sticky_index` docs](https://docs.rs/rinch-editor-collab) say the same at
the source. The adapter itself exposes the pair as `CollabSession::sticky_index(doc,
pos)` / `resolve_sticky(doc, bytes)`, taking the model document the session projects.

#### Choosing the side, outliving the block, and resolving without an editor

`collab_sticky_index_with(pos, assoc)` takes the side the index holds on to:
`StickyAssoc::After` (the default, the character that starts at `pos`: text typed at
`pos` goes before the index) or `StickyAssoc::Before` (the character that ends at
`pos`: text typed there goes after it). A **range** — a comment's span, a highlight —
is two indexes, the start `After` and the end `Before`, so text typed at either edge
stays outside it.

Its bytes also **outlive their block**. yrs garbage-collects a deleted block's
characters together with the block, so nothing in the shared document says which
block they were in any more; but the block's own entry in its parent array stays there
as a tombstone. So these bytes are the plain yrs `StickyIndex` described above, with
`assoc`'s association, followed by a versioned trailer: one more yrs `StickyIndex` per
array on the way from the block up to the document root, naming the block's entry (or
its container's) in it, and a 32-bit hash of everything before it. **Treat the bytes
as opaque**: store them, compare them, hand them back; do not build or edit them. A
trailer whose hash, version or chain (each entry in the array of the next one's node)
does not check out is refused (`None`). That check is a 32-bit hash against
accidental damage, not protection against tampering: bytes edited on purpose, with
the hash recomputed, can name another place.
yrs alone still reads the leading index, because yrs's decoder ignores what follows
it; that is yrs's behaviour, not a promise, and a test pins it.
`collab_locate_sticky(&bytes)` answers a `StickyLocation`:

```rust
let anchor = editor.collab_sticky_index_with(pos, StickyAssoc::After).unwrap();
// …any number of local and remote edits later:
if let Some(at) = editor.collab_locate_sticky(&anchor) {
    // at.pos: a position inside a textblock
    // at.path: child indices to that textblock; at.path[0] is the top-level block
    // at.displaced: its block is gone, and at.pos is only the nearest place
}
```

While the block is in the shared document it is the same answer as
`collab_resolve_sticky`, with `displaced: false`, or `None` when the model cannot show
it (a table too large to read, shown as a placeholder): `displaced` is never `true`
for a block that is still there. Once the block is deleted, joined into the one
before it or replaced, it is the place the block stood, at the innermost level that
still has its entry: the end of the last textblock before that place, or the start of
the first one after it when there is none. Two edits that keep the text still
**replace** the block and leave the index displaced for good: toggling a list on its
paragraph, and undoing the deletion of its block (the undo writes the text as new
characters). A table's rows and cells have no entry, so an index whose row was
deleted lands at the start of the first textblock of the table. An index from plain
`collab_sticky_index` has no trailer, and its deleted block is still `None`. An index
naming a peer's edit this replica has not merged yet is `None`, not displaced.

**`None` means "not locatable now", `displaced` means "its block is gone".** `None`
covers cases that can change: an edit not merged yet, a stalled or poisoned session,
the block inside a table too large to read (its placeholder), a `doc` that is not the
one the shared document projects; and cases that will not: damaged bytes, an anchor
from another document. An app that keeps anchors should not drop one on a single
`None`; it can keep it and ask again after the next change.

**An anchor is tied to the shared document's CRDT items, not to its text.** It
resolves in any replica that grew from the same CRDT (a peer, a server copy, a
"duplicate" made by loading a snapshot), and in no other: the same text in a
document projected afresh has new items. So anything that writes the document as new
items loses every anchor into it (`None`, or displaced for those with a trailer
whose outer levels survive): `load_doc` / `load_html` while collaborating, starting a
new host session from the model, the re-base that ends an outbound stall for the
blocks it rewrites, and a future change of the wire format tag. Keep anchors keyed
by document, and treat a re-projection as invalidating them.

**Many at once.** `collab_locate_sticky_many(&[&anchor, …])` answers every anchor in
one read of the shared document, in order. One `collab_locate_sticky` stops at its
block when the block is there but reads the whole document when it is gone, so an app
resolving every comment on each layout should batch: measured on a 4,000-paragraph
document (debug build), 200 single calls took 0.37 s with every block present and
1.6 s with every block deleted; one batch of the same 200 took 24 ms and 26 ms.

**Without an editor** (a server, a tool holding only the CRDT bytes), load a replica
and ask it. Making an index works the same way, on the model the replica reads:

```rust
use rinch_editor_collab::{CollabDoc, StickyAssoc};

let replica = CollabDoc::load(&crdt_bytes)?;
let doc = replica.to_doc(&schema)?;
let anchor = replica.sticky_index_with(&doc, pos, StickyAssoc::After); // Option<Vec<u8>>
// …and later, on a replica loaded from newer bytes:
let found = replica.locate_sticky_in(&schema, &anchor_bytes)?; // Option<StickyLocation>
let all = replica.locate_sticky_many_in(&schema, &[&a1, &a2])?; // Vec<Option<…>>
```

`CollabSession::sticky_index_with` / `locate_sticky` / `locate_sticky_many` are the
session's.

### On the web

The **same** adapter runs in the browser — yrs compiled to wasm. Enable the
`collaboration` feature on `rinch-web`:

```toml
rinch-web = { path = "...", features = ["collaboration"] }
```

The `EditorHandle` collab API is identical to desktop, with two web specifics:

- **Inbound is a direct call.** Wasm is single-threaded, so a transport callback
  (e.g. a `WebSocket` `onmessage`) already runs on the main thread — call
  `handle.collab_receive(&bytes)` (or `collab_receive_for(container_id, &bytes)`)
  directly. There is no `post_remote_delta` on web (that is the desktop runtime's
  off-thread marshaller).
- **No randomness shim needed.** yrs carries its own `fastrand/js` source and builds
  for `wasm32-unknown-unknown` with no extra features — unlike Automerge, which
  needed the app to add `uuid = { version = "1", features = ["js"] }` just to make the
  wasm build compile. `rinch-web`'s `collaboration` feature now needs nothing else
  configured by the app.

A runnable two-pane web loopback is `examples/collab-editor-web` (built with
`trunk serve`), the browser counterpart of `collab-editor-demo`.

## Where to go next

- [Rich-Text Editor](./editor.md) — the document model, schema, transactions,
  commands, history, and the view seam in depth.
- `examples/markdown-editor` — a standalone editor app (great for MCP-driven
  iteration; built with the `debug` feature).
- `examples/collab-editor-demo` — two editors sharing one CRDT, live.
