# Rich-text editing

Rinch ships a built-in rich-text editor — a ProseMirror-style, model-first editor
with schema-enforced structure, inline marks, lists, tables, and exact undo/redo.
You add it with the `Editor {}` component and drive it through an `EditorHandle`.

> **This page is the practical "how do I add an editor" guide.** For the deeper
> model — the document tree, steps and transactions, the schema, commands, and the
> view seam — see the [Rich-Text Editor](./editor.md) guide.

> **Desktop today.** The editor view is a **desktop** feature (gated behind the
> `desktop` cargo feature). The editor *core* is renderer-agnostic; a web view (over
> the browser's native contentEditable) is a planned follow-up. There is no
> `contenteditable` HTML attribute and no DOM-level editing API to wire up — the
> `Editor` component is the whole surface.

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
| **Containers** | `toggleBulletList`, `toggleOrderedList`, `wrapInBlockquote` |
| **Lists** | `sinkListItem` (indent), `liftListItem` (outdent) |
| **Inserts** | `insertHorizontalRule`, `insertHardBreak`, `insertTable` |
| **Tables** | `addRowAfter`, `addRowBefore`, `addColumnAfter`, `addColumnBefore`, `deleteRow`, `deleteColumn`, `deleteTable`, `mergeCells`, `splitCell` |
| **Links** | `removeLink` |
| **History** | `undo`, `redo` |

> Links need a destination, so applying a link is a builder rather than a bare named
> command — use `handle.command("removeLink")` to clear links and the lower-level
> link API to add one with an `href`.

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
| `replace_selection_with_html(&str)` | Replace the selection with parsed HTML (the rich-paste path). |
| `selection_clipboard()` | The current selection serialized as `(html, plain_text)` for the clipboard. |

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

The editor handles its own keyboard input. Built-in shortcuts include:

| Shortcut | Action |
|----------|--------|
| Ctrl/Cmd+B | Toggle **bold** |
| Ctrl/Cmd+I | Toggle *italic* |
| Ctrl/Cmd+U | Toggle underline |
| Ctrl/Cmd+Z | Undo |
| Ctrl/Cmd+Shift+Z / Ctrl+Y | Redo |
| Enter | Split block / new list item |
| Tab / Shift+Tab | Move between table cells |
| Backspace / Delete | Delete backward / forward |

Undo/redo is a single, exact history: each undo reverses one logical edit (typing
is merged into a group), because every edit is an invertible step.

## A complete toolbar

A full example pairing a command toolbar with the editor lives at
`examples/markdown-editor/src/main.rs`, and `examples/ui-zoo/src/sections/editor.rs`
shows the same pattern inside the component showcase. The shape is always:
`create_editor()` once, clone the handle into each button's `onclick`, and place a
single `Editor {}` for the surface.

## Where to go next

- [Rich-Text Editor](./editor.md) — the document model, schema, transactions,
  commands, history, and the view seam in depth.
- `examples/markdown-editor` — a standalone editor app (great for MCP-driven
  iteration; built with the `debug` feature).
