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
            // Toolbar — each button runs a named command.
            div {
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
`class:` props, applied to its host element.

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
button { onclick: move || { ed.command("toggleBold"); }, "Bold" }
```

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

> Links need a destination, so applying a link is a builder rather than a bare named
> command. Use `handle.toggle_link(href)` to add (or, over an existing link, remove)
> a link, `handle.command("removeLink")` to clear one unconditionally, and
> `handle.active_link_href()` to read the current link's target for an edit dialog.
> Text typed right after a link is not part of it (`link` is a non-inclusive mark),
> so a link does not grow as the user keeps typing past it. The one exception is a
> link that runs straight into a *different* link: text typed at that seam continues
> the first one (see [inherited marks](./editor.md#state-selection-stored-marks)).

> The word deletes remove from the caret to the boundary the word motion
> (Ctrl+Left / Ctrl+Right) lands on, taking the whitespace between: in `hello wor|ld`
> backward leaves `hello |ld`. A mark boundary is not a word boundary. Where there is
> no word to take they are Backspace / Delete: a non-empty selection is deleted as it
> stands, and at a textblock's start / end they join (or lift), as Backspace and
> Delete do there. An inline atom — an image, a hard break — is a boundary of its
> own: one right beside the caret goes alone, and a word beyond one is not taken
> with it. There are no *line* delete commands (issue #301 is whether a "line" is
> the visual line or the textblock); on the web the browser's
> `deleteSoftLine*` / `deleteHardLine*` still delete to the textblock's edge.

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
| `load_html(&str) -> bool` | Parse schema-whitelisted HTML and replace the document. Returns `false` if it doesn't parse into valid content. |
| `doc() -> Node` | The current document (the save shape; serialize it under the `serde` feature). |
| `insert_image(src, alt)` | Insert an image node (e.g. a `data:` URL), replacing the selection. |
| `toggle_link(href) -> bool` | Add a `link` mark with `href` across the selection, or remove it if the selection is already linked. No-op (returns `false`) for a collapsed cursor. |
| `active_link_href() -> Option<String>` | The `href` of the link the selection is on, for pre-filling an "edit link" dialog: for a range, the first link in it; for a caret, the link text typed there would carry. A link is not inclusive, so a caret inside it answers its `href` and a caret at its start or right after its last character answers `None` — except where it runs straight into a different link, where the caret is in the first. |
| `paste(&PasteContent) -> bool` | Paste `text/plain` and/or `text/html` over the selection the way the user's paste does: your plugins first, then the default. See [Seeing and rewriting a paste](#seeing-and-rewriting-a-paste). |
| `replace_selection_with_html(&str)` | Replace the selection with parsed HTML (the default rich paste; no plugin sees it). |
| `selection_clipboard()` | The current selection serialized as `(html, plain_text)` for the clipboard. |
| `anchor_selection() -> SelectionAnchor` | Capture the selection for a later insertion, kept pointing at the same content as the user keeps editing. See [Pasting is asynchronous](#pasting-is-asynchronous). |

HTML is **schema-whitelisted** on load: known block tags become nodes and known
inline tags become marks; unknown tags and attributes (`<script>`, inline event
handlers, …) are dropped at parse time. The document can only ever hold structure
the schema allows.

### Your own plugins and inline decorations

`add_plugin(Rc<dyn Plugin>) -> bool` installs a plugin of your own — a
spellchecker, a search highlighter. It rebuilds the editor's state over the
current document and selection, which **discards the undo history**, so call it
on a freshly created handle, before content is typed. It is not an edit: a
read-only editor (collaborating or not) accepts it, nothing is sent to peers and
`on_change` does not fire. A plugin whose key is already installed is refused
(`false`), and a refused call changes nothing.

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
lands rather than in each input handler. `can_run(name)` answers `false` for a refused
command, so a toolbar that greys its buttons from it goes inert with the switch; a
toolbar of your own should also read `is_read_only()`. The built-in context menu
offers Copy and Select all only, the OS input method stays off (no IME candidate
window), and on the web the hidden capture field is `readonly`, so no soft keyboard
is raised and the browser's own menu drops Paste and Cut.

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
than smoothly. On the **web** that is the browser's `scrollIntoView`, which scrolls
every scrollable ancestor, the page included. On **desktop** only the caret's
nearest scroll container scrolls (issue #842), so an editor whose scroller is itself
off screen stays off screen.

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

Not covered: the moving end of a text **range** (Shift+arrow) is not revealed, and
moving the caret into a block of a virtualized editor that has never been laid out
(Ctrl+End from the top of a very long document) does not scroll to it until the
block is on screen (issue #845). An app that wants either on screen asks for it with
`scroll_into_view`, below.

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
as well as from an event handler) and moves the nearest scroll container; on the
**web** it happens during the call and `scrollIntoView` moves every scrollable
ancestor, the page included. A virtualized desktop editor lays out the blocks holding
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

It waits, carries and drops exactly as `scroll_into_view` does, and scrolls the same
containers on **desktop** (the nearest scroll container, vertically). On the
**web** it sets the nearest scroll container's `scrollTop` (the page's, when no
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

Both are **no-ops before the editor is mounted**, and nothing is remembered for the
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
handle — load another document, read the selection. A pointer move pays nothing for
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

### Autocomplete popups: keys, selection and caret geometry

Four handle methods are what an app needs to run a popup off the editor, like a
`[[` link picker or an `@` mention list. They are the same on desktop and on the
web.

| Method | What it does |
|--------|--------------|
| `on_key(\|key: &EditorKey\| -> bool)` | Offered every key press **before** the editor moves the caret, runs a binding or types. `true` consumes it: the editor does nothing (on the web the `keydown` is `preventDefault`ed and stopped). `false` leaves it to the editor. |
| `on_selection_change(\|sel: &Selection\|)` | Called after every change that leaves the selection different: typing, deleting, clicks, arrows, commands, app transactions, a load that resets it, a peer's edit that maps it. Not called when the selection is unchanged. |
| `on_caret_moved(\|\|)` | Called once the runtime has placed the focused editor's caret somewhere new, **after layout**. The moment to position the popup. |
| `caret_rect(pos) -> Option<ElementBounds>` | Where a caret at `pos` is on screen (zero width), whether or not the caret is there: logical window pixels on desktop, client pixels on the web. The frame of a `position: fixed` popup. |

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
inside itself. Each is one callback per handle (a second registration replaces the
first), like `on_change`, and costs nothing while none is registered.

**Why `on_caret_moved` and not `on_selection_change` for geometry.** On the web the
browser lays out on demand, so `caret_rect` is always current. The desktop runtime
lays out once per input event, after the editor has handled it, so from
`on_selection_change` or `on_key` a position in the block just edited answers
`None` (it has no layout yet). `on_caret_moved` comes from the caret pass after the
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
| Enter | Split block / new list item |
| Shift+Enter | Insert a hard break (line break within the block) |
| Tab / Shift+Tab | Move between table cells, else indent / outdent a list item |
| Backspace / Delete | Delete backward / forward |
| Ctrl+Backspace / Ctrl+Delete (Alt on macOS) | Delete the word backward / forward |

The four alignment chords are the Google Docs ones, and some of them are also
chords of the browser or the platform, which may take the key before the editor
sees it. Chrome reserves Ctrl+Shift+J (and Ctrl+Shift+I) for its developer
tools; Ctrl+Shift+R is the browser's hard reload; some Linux input methods
(IBus's emoji picker) bind Ctrl+Shift+E. The commands themselves
(`setTextAlignLeft` / `Center` / `Right` / `Justify`) always work from a toolbar.

The word-delete chord is the one binding that is not `Mod`: on macOS Cmd+Backspace
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
reverses one logical edit (typing is merged into a group), because every edit is an
invertible step.

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
`load_html`, or a collaborative re-projection) the anchor reports `None` and the
paste is dropped: the content it was aimed at no longer exists, and reusing the
raw offset would drop it into unrelated text.

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
list containers (bullet/ordered lists and list items, nested to any depth),
horizontal rules, and the inline atoms inside a line — images and hard breaks
(Shift+Enter). Blockquotes, tables and task lists are still outside it: an edit
outside that scope fails loud rather than silently diverging —
`collab_take_error()` surfaces it, and the CRDT is left untouched (the local edit is
not projected). Horizontal rules, images and hard breaks all joined that scope
without a new wire format, so **every peer on a document must be upgraded
together**: an older build accepts a rule, an image or a hard break from a newer
peer and then cannot read it, which poisons its session (see below) in both
directions for as long as that content remains in the document — it heals only
when the last one is deleted. A peer joining from a snapshot that already holds
one fails the join instead.

Two concurrent edits to images can still be lost, and both editors still end up
with the same document when they are. **Two identical images side by side**
(same `src`, same `alt`, …) whose attributes two people change at the same
moment: the CRDT sees them as one formatted run, and one change can overwrite the
other (#860). And **splitting a block right before an image** (Enter) while
someone else changes that image's attributes loses the change — a split moves
content, and this is true of any mark change on moved text, not only images
(#861).

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
    // e.g. "Not syncing — remove the pasted table to resume." The error names the
    // content: "collab does not support this content yet: node `blockquote` …".
    show_banner(&err.to_string());
}
```

The cure is to remove the offending content — undo the paste, unwrap the quote. You do
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
- **Its block deleted:** `collab_resolve_sticky` answers `None`. Joining a block into
  the one before it counts (Backspace at a block's start): the projection writes the
  joined text as a new insert, not a move. Likewise, splitting a block (Enter) before
  the position moves the tail into a new block, and an index on the tail then resolves
  to the split point.
- **An edit next to its paragraph, at the top level of the document.** The projection
  finds the top-level blocks an edit left alone by node *identity*, and a split or a
  join rebuilds both nodes it touches, so the paragraph beside it is rewritten even
  though its text did not change (tracked in #917). In practice:
  - **Enter at the start of a paragraph** (inserting an empty paragraph above it):
    every index into that paragraph now resolves into the **new empty paragraph**.
    That is a wrong position, not `None`.
  - **Backspace that deletes an empty line above a paragraph:** every index into that
    paragraph resolves to `None`.
  - **Toggling a bullet list on a paragraph** replaces the block with one of another
    kind: `None`.

  Blocks inside a list fare better: that level of the diff compares nodes
  structurally, so Enter at the start of a paragraph in a list item keeps its indexes.

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
