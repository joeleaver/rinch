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
| **History** | `undo`, `redo` |

> Links need a destination, so applying a link is a builder rather than a bare named
> command. Use `handle.toggle_link(href)` to add (or, over an existing link, remove)
> a link, `handle.command("removeLink")` to clear one unconditionally, and
> `handle.active_link_href()` to read the current link's target for an edit dialog.

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
| `active_link_href() -> Option<String>` | The `href` of the link at the selection head, for pre-filling an "edit link" dialog. `None` when not inside a link. |
| `replace_selection_with_html(&str)` | Replace the selection with parsed HTML (the rich-paste path). |
| `selection_clipboard()` | The current selection serialized as `(html, plain_text)` for the clipboard. |
| `anchor_selection() -> SelectionAnchor` | Capture the selection for a later insertion, kept pointing at the same content as the user keeps editing. See [Pasting is asynchronous](#pasting-is-asynchronous). |

HTML is **schema-whitelisted** on load: known block tags become nodes and known
inline tags become marks; unknown tags and attributes (`<script>`, inline event
handlers, …) are dropped at parse time. The document can only ever hold structure
the schema allows.

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
| Mod+Z / Mod+Shift+Z / Mod+Y | Undo / redo |
| Enter | Split block / new list item |
| Shift+Enter | Insert a hard break (line break within the block) |
| Tab / Shift+Tab | Move between table cells, else indent / outdent a list item |
| Backspace / Delete | Delete backward / forward |

(Copy/cut/paste — Mod+C/X/V, and Mod+Shift+V for paste-as-plain — are handled by the
platform clipboard, not the keymap.) Undo/redo is a single, exact history: each undo
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
| `> ` | Blockquote |
| `- ` / `* ` / `+ ` | Bullet list |
| `1. ` | Ordered list |
| `[ ] ` / `[x] ` | Task list (unchecked / checked) |
| `**bold**` / `__bold__` | **bold** |
| `*italic*` / `_italic_` | *italic* |
| `~~strike~~` | ~~strike~~ |
| `==highlight==` | highlighted |
| `` `code` `` | inline `code` |

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
text-blocks + marks** (paragraphs, headings, code blocks, bold/italic/link/…) plus
list containers (bullet/ordered lists and list items, nested to any depth); an edit
outside that scope fails loud rather than silently diverging —
`collab_take_error()` surfaces it, and the CRDT is left untouched (the local edit is
not projected). A runnable two-pane loopback (both editors in one window, no network)
lives at `examples/collab-editor-demo/src/main.rs`.

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
