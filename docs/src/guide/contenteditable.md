# ContentEditable API

Rinch provides a DOM-level contenteditable system for building rich-text editors. This is the **low-level API** that powers all text editing — the `contenteditable` HTML attribute on a `<div>`, combined with the `ContentEditableApi` trait for programmatic control.

> **Two layers:** The [Rich-Text Editor](./editor.md) guide covers the higher-level `rinch-editor` crate (schemas, extensions, document model). This guide covers the lower-level CE API that it's built on. Most users will use both — CE for the editing surface, and rinch-editor for serialization/collaboration.

## Quick Start

Create a contenteditable element and interact with it:

```rust
use rinch::prelude::*;
use rinch_core::ce::with_active_ce_api;

#[component]
fn my_editor() -> NodeHandle {
    rsx! {
        div {
            // Toolbar
            div {
                button { onclick: move || ce_do(|api| api.toggle_wrap("strong")), "Bold" }
                button { onclick: move || ce_do(|api| api.toggle_wrap("em")), "Italic" }
                button { onclick: move || ce_do(|api| api.set_block_type("h1")), "H1" }
            }
            // Editing surface
            div {
                contenteditable: "true",
                style: "min-height: 200px; padding: 8px; border: 1px solid #ccc;",
            }
        }
    }
}

/// Helper: run a closure on the currently focused CE element.
fn ce_do(f: impl FnOnce(&mut dyn ContentEditableApi) + 'static) {
    with_active_ce_api(|api| f(&mut *api.borrow_mut()));
}
```

Setting `contenteditable: "true"` on a `<div>` activates rinch's built-in editing behavior: cursor rendering, text selection, keyboard input handling, clipboard support, and undo/redo.

## How It Works

```
Keyboard Event
    ↓
InputHandler maps key → EditCommand
    ↓
CeOps mutates EditorDocument (CRDT, source of truth)
    ↓
Affected DOM blocks re-rendered from EditorDocument state
    ↓
CeEvent dispatched to all listeners
    ↓
SelectionChanged + oninput fired
    ↓
Scene marked dirty → repaint
```

1. The rinch runtime captures keyboard events on the focused CE element
2. Keys are mapped to `EditCommand`s (InsertText, DeleteBackward, ToggleBold, etc.)
3. `CeOps` — the runtime's implementation of `ContentEditableApi` — mutates the `EditorDocument` (Automerge CRDT)
4. Only the affected block(s) are re-rendered in the DOM from EditorDocument state
5. Each mutation dispatches a `CeEvent` so observers can react
6. The cursor/selection is updated and the scene is repainted

> **CRDT-first architecture:** Every mutation flows through `EditorDocument` first, then the DOM is updated as a view. This ensures the CRDT and DOM never diverge, and makes collaboration work automatically — remote changes just load into EditorDocument, then re-render.

## ContentEditableApi Trait

The `ContentEditableApi` trait is the single mutation interface for all CE operations. Every method mutates the DOM **and** dispatches a corresponding `CeEvent`.

### Text Operations

```rust
fn insert_text(&mut self, text: &str);     // Insert at cursor
fn delete_backward(&mut self);              // Backspace
fn delete_forward(&mut self);               // Delete key
fn delete_selection(&mut self);             // Delete selected range
```

### Block Structure

```rust
fn split_block(&mut self);                  // Enter key — split block at cursor
fn set_block_type(&mut self, tag: &str);    // Change block to h1, p, blockquote, etc.
```

### Inline Formatting

```rust
fn wrap_selection(&mut self, tag: &str);    // Wrap in <strong>, <em>, etc.
fn unwrap_selection(&mut self, tag: &str);  // Remove formatting wrapper
fn toggle_wrap(&mut self, tag: &str);       // Toggle on/off
```

Supported tags: `"strong"`, `"em"`, `"u"`, `"s"`, `"code"`.

### List Operations

```rust
fn indent(&mut self);    // Convert to list item or increase nesting
fn outdent(&mut self);   // Decrease nesting or convert from list item
```

### Selection

```rust
fn get_selection(&self) -> CeSelection;
fn set_selection(&mut self, sel: CeSelection);
```

### Undo/Redo

```rust
fn undo(&mut self);
fn redo(&mut self);
```

### Query Methods

```rust
fn has_active_mark(&self, tag: &str) -> bool;   // Is cursor inside <strong>, <em>, etc.?
fn cursor_block_tag(&self) -> Option<String>;    // Block tag at cursor ("p", "h1", "ul", etc.)
```

### Content Interchange

```rust
fn extract_content(&self) -> Vec<BlockData>;         // Read content as structured blocks
fn load_content(&mut self, blocks: &[BlockData]);     // Replace content from blocks
fn load_html(&mut self, html: &str);                  // Replace content from HTML string
fn clear_formatting(&mut self);                        // Strip all inline formatting
```

## Accessing the CE API

There are two ways to access the CE API, depending on whether you need to target the focused element or a specific element.

### Active CE API (Focused Element)

Use `with_active_ce_api()` to operate on whichever CE element currently has focus. This is the typical pattern for toolbar buttons:

```rust
use rinch_core::ce::with_active_ce_api;

// Helper function (recommended pattern)
fn ce_do(f: impl FnOnce(&mut dyn ContentEditableApi) + 'static) {
    with_active_ce_api(|api| f(&mut *api.borrow_mut()));
}

// In toolbar buttons:
button { onclick: move || ce_do(|api| api.toggle_wrap("strong")), "Bold" }
button { onclick: move || ce_do(|api| api.set_block_type("h2")), "H2" }
button { onclick: move || ce_do(|api| api.undo()), "Undo" }
```

Returns `None` if no CE element is focused.

### Per-Element CE API (NodeHandle)

Use `NodeHandle::with_ce_api()` to target a specific CE element. This works whether or not the element has focus — useful for loading initial content:

```rust
let editor_div = rsx! {
    div { contenteditable: "true" }
};

// Load content into a specific CE element
editor_div.with_ce_api(|api| {
    api.borrow_mut().load_html("<p>Hello <strong>world</strong></p>");
});
```

### Per-Node Registry

For advanced use cases, access CE APIs by node ID:

```rust
use rinch_core::ce::{with_ce_api_for_node, register_ce_api, unregister_ce_api};

// Access CE API for a known node ID
with_ce_api_for_node(node_id, |api| {
    api.borrow_mut().insert_text("Hello");
});
```

## CeEvent System

Every CE mutation dispatches a `CeEvent` to all subscribed listeners. This is how the editor bridge stays in sync with DOM changes.

### Subscribing to Events

```rust
use rinch_core::ce::{subscribe_ce_events, CeEvent};
use std::rc::Rc;

subscribe_ce_events(Rc::new(move |event| {
    match event {
        CeEvent::TextInserted { node_id, offset, text } => {
            println!("Inserted '{}' at node {} offset {}", text, node_id, offset);
        }
        CeEvent::TextDeleted { node_id, offset, length } => {
            println!("Deleted {} bytes at node {} offset {}", length, node_id, offset);
        }
        CeEvent::SelectionChanged { selection } => {
            // Update toolbar active states, etc.
        }
        _ => {}
    }
}));
```

### Event Reference

#### Text Mutations

| Event | Fields | When |
|-------|--------|------|
| `TextInserted` | `node_id`, `offset`, `text` | Text inserted at cursor |
| `TextDeleted` | `node_id`, `offset`, `length` | Text deleted (backspace, delete, selection) |
| `TextNodeCreated` | `node_id`, `parent_id`, `text` | New text node created (e.g. first char in empty block) |
| `NodeRemoved` | `node_id`, `parent_id` | A DOM node was removed |

#### Selection

| Event | Fields | When |
|-------|--------|------|
| `SelectionChanged` | `selection: CeSelection` | Cursor moved or selection changed |

#### Block Structure

| Event | Fields | When |
|-------|--------|------|
| `BlockSplit` | `original_block_id`, `new_block_id`, `split_offset` | Enter key splits a block |
| `BlockJoined` | `surviving_block_id`, `removed_block_id`, `merge_offset` | Backspace joins two blocks |
| `BlockTypeChanged` | `old_node_id`, `new_node_id`, `old_tag`, `new_tag` | Block type changed (e.g. p → h1) |

#### Inline Formatting

| Event | Fields | When |
|-------|--------|------|
| `SelectionWrapped` | `tag`, `wrapper_node_id`, `wrapped_node_ids` | Selection wrapped in formatting element |
| `SelectionUnwrapped` | `tag`, `unwrapped_node_ids` | Formatting removed from selection |

#### List Structure

| Event | Fields | When |
|-------|--------|------|
| `ListItemOutdented` | `old_li_id`, `new_block_id` | List item outdented |
| `BlockIndented` | `old_block_id`, `new_li_id`, `list_id` | Block indented into list |

#### Tables

| Event | Fields | When |
|-------|--------|------|
| `TableInserted` | `block_node_id`, `rows`, `cols` | Table inserted |
| `TableDeleted` | `block_node_id` | Table removed |

#### History & Clipboard

| Event | Fields | When |
|-------|--------|------|
| `UndoApplied` | *(none)* | Undo operation completed |
| `RedoApplied` | *(none)* | Redo operation completed |
| `HtmlPasted` | `created_node_ids` | HTML pasted from clipboard |

## DomCursor and CeSelection

The CE system uses DOM-level cursor positions, not document-level byte offsets.

```rust
/// A position in the DOM: which text node, and byte offset within it.
pub struct DomCursor {
    pub node_id: usize,   // ID of the text node (or element for empty blocks)
    pub offset: usize,    // Byte offset within the text node
}

/// A selection: anchor (where it started) + head (current caret position).
pub struct CeSelection {
    pub anchor: DomCursor,
    pub head: DomCursor,
}

impl CeSelection {
    fn collapsed(cursor: DomCursor) -> Self  // Cursor (no selection)
    fn range(anchor: DomCursor, head: DomCursor) -> Self
    fn is_collapsed(&self) -> bool
}
```

**Key difference from document positions:** `DomCursor` references specific DOM node IDs, not abstract document offsets. Node IDs can change when the DOM is restructured (e.g. block splits, formatting changes).

## Data Interchange Types

Use `BlockData` for structured content interchange (e.g. syncing with `EditorDocument`):

```rust
pub struct BlockData {
    pub block_type: String,                    // "paragraph", "heading", etc.
    pub attrs: HashMap<String, String>,        // e.g. {"level": "2"} for headings
    pub content: Vec<InlineRunData>,
}

pub struct InlineRunData {
    pub text: String,
    pub marks: Vec<InlineMarkData>,
}

pub struct InlineMarkData {
    pub mark_type: String,                     // "bold", "italic", "code", etc.
    pub attrs: HashMap<String, String>,
}
```

## Keyboard Shortcuts

The CE system provides these built-in keyboard shortcuts:

### Text Formatting

| Shortcut | Action |
|----------|--------|
| Ctrl+B | Toggle bold (`<strong>`) |
| Ctrl+I | Toggle italic (`<em>`) |
| Ctrl+U | Toggle underline (`<u>`) |
| Ctrl+Shift+X | Toggle strikethrough (`<s>`) |
| Ctrl+E | Toggle inline code (`<code>`) |

### Editing

| Shortcut | Action |
|----------|--------|
| Enter | Split block |
| Backspace | Delete backward (joins blocks at boundary) |
| Delete | Delete forward |
| Tab | Indent (increase list nesting or insert tab) |
| Shift+Tab | Outdent (decrease list nesting) |
| Ctrl+Z | Undo |
| Ctrl+Y | Redo |

### Clipboard

| Shortcut | Action |
|----------|--------|
| Ctrl+C | Copy selection |
| Ctrl+X | Cut selection |
| Ctrl+V | Paste (HTML preferred, falls back to plain text) |

### Navigation

All standard cursor movement keys work: arrow keys, Home/End, Ctrl+arrow for word movement, Shift+arrow for selection, Ctrl+A for select all, Page Up/Down.

## Building a Toolbar

A typical pattern for editor toolbars uses reactive signals to track active formatting state:

```rust
use rinch::prelude::*;
use rinch_core::ce::{with_active_ce_api, subscribe_ce_events, CeEvent};

#[component]
fn editor_toolbar() -> NodeHandle {
    let is_bold = Signal::new(false);
    let is_italic = Signal::new(false);
    let block_type = Signal::new(String::from("p"));

    // Subscribe to selection changes to update toolbar state
    subscribe_ce_events(Rc::new(move |event| {
        if let CeEvent::SelectionChanged { .. } = event {
            with_active_ce_api(|api| {
                let api = api.borrow();
                is_bold.set(api.has_active_mark("strong"));
                is_italic.set(api.has_active_mark("em"));
                if let Some(tag) = api.cursor_block_tag() {
                    block_type.set(tag);
                }
            });
        }
    }));

    rsx! {
        div { class: "toolbar",
            button {
                class: {|| if is_bold.get() { "active" } else { "" }},
                onclick: move || ce_do(|api| api.toggle_wrap("strong")),
                "B"
            }
            button {
                class: {|| if is_italic.get() { "active" } else { "" }},
                onclick: move || ce_do(|api| api.toggle_wrap("em")),
                "I"
            }
        }
    }
}

fn ce_do(f: impl FnOnce(&mut dyn ContentEditableApi) + 'static) {
    with_active_ce_api(|api| f(&mut *api.borrow_mut()));
}
```

## Loading Initial Content

Use `load_html` to set the initial content of a CE element:

```rust
#[component]
fn editor_with_content() -> NodeHandle {
    let editor = rsx! {
        div { contenteditable: "true", style: "min-height: 200px;" }
    };

    // Load content (works before the element receives focus)
    editor.with_ce_api(|api| {
        api.borrow_mut().load_html(r#"
            <h1>Welcome</h1>
            <p>This is <strong>rich text</strong> content.</p>
            <ul>
                <li>First item</li>
                <li>Second item</li>
            </ul>
        "#);
    });

    editor
}
```

## Extracting Content

Read the current CE content as structured data:

```rust
// As BlockData (for syncing with EditorDocument or serialization)
let blocks = with_active_ce_api(|api| {
    api.borrow().extract_content()
}).unwrap_or_default();

// Process blocks
for block in &blocks {
    println!("Block type: {}", block.block_type);
    for run in &block.content {
        let marks: Vec<_> = run.marks.iter().map(|m| m.mark_type.as_str()).collect();
        println!("  '{}' marks={:?}", run.text, marks);
    }
}
```

## Architecture Notes

### CeOps

`CeOps` is the runtime's implementation of `ContentEditableApi`. It holds a reference to the `RinchDocument` and the CE element's node ID. Created lazily when a CE element first receives focus.

### Event Flow

```
User types "a"
  → KeyEvent captured by winit
  → RinchApp::handle_contenteditable_key()
  → InputHandler maps to EditCommand::InsertText("a")
  → CeOps::insert_text("a")
    → DOM: set_text_content on text node
    → dispatch_ce_event(TextInserted { ... })
  → dispatch SelectionChanged
  → dispatch oninput
  → mark scene dirty → repaint
```

### Thread-Local Storage

The CE system uses thread-local storage for global access:

- **Event dispatcher:** `dispatch_ce_event()` / `subscribe_ce_events()` — broadcasts events to all listeners
- **Active CE API:** `set_active_ce_api()` / `with_active_ce_api()` — tracks which CE element has focus
- **CE API registry:** `register_ce_api()` / `with_ce_api_for_node()` — per-element API lookup

All are `thread_local!` — safe for single-threaded GUI but not shareable across threads.

## Key Source Files

| File | Purpose |
|------|---------|
| `crates/rinch-core/src/ce.rs` | Core types: `CeEvent`, `ContentEditableApi`, `DomCursor`, `CeSelection`, dispatchers |
| `crates/rinch/src/ce_ops.rs` | `CeOps` — runtime implementation of `ContentEditableApi` (CRDT-first mutations) |
| `crates/rinch/src/ce_render.rs` | Block rendering, `BlockMap`, position conversion (`EditorPosition ↔ DomCursor`) |
| `crates/rinch/src/app/contenteditable/mod.rs` | Keyboard input handler, cursor management |
| `crates/rinch/src/app/contenteditable/ce_selection.rs` | Selection, copy/cut, HTML extraction |
| `crates/rinch/src/app/contenteditable/ce_paste.rs` | HTML paste handling |
| `crates/rinch/src/app/contenteditable/ce_navigation.rs` | Cursor navigation |
| `crates/rinch/src/app/contenteditable/ce_virtualization.rs` | Large document virtualization |
| `crates/rinch-editable/src/` | Generic editing primitives (`EditCommand`, `InputHandler`) |
