# Rich-Text Editor

Rinch provides a comprehensive rich-text editor with collaborative editing support through Automerge CRDT, a schema-driven document model, and a powerful extension system.

## Quick Start

Create a new editor with the StarterKit (22 default extensions):

```rust
use rinch_editor::prelude::*;

fn editor_component(__scope: &mut RenderScope) -> NodeHandle {
    // Create schema with all standard editing features
    let schema = Schema::starter_kit();
    let config = EditorConfig::default();

    // Create editor instance
    let mut editor = Editor::new(schema, config)?;

    // Render editor content...
    todo!()
}
```

## Editor Configuration

The `Editor` struct holds the document model, schema, selection state, and all editing capabilities:

```rust
pub struct Editor {
    pub doc: EditorDocument,           // The document being edited
    pub schema: Schema,                // Validation rules
    pub selection: SelectionState,     // Current cursor/selection
    pub history: History,              // Undo/redo
    pub commands: CommandDispatcher,   // All mutations
    pub extensions: ExtensionRegistry, // Loaded extensions
    pub input_rules: InputRuleSet,     // Auto-transforms
    pub shortcuts: ShortcutRegistry,   // Keyboard bindings
    pub events: EventDispatcher,       // Event handling
    pub config: EditorConfig,          // Editor settings
}
```

### Configuration Options

```rust
#[derive(Debug, Clone)]
pub struct EditorConfig {
    pub autofocus: AutoFocus,  // Focus behavior on mount
    pub editable: bool,        // Allow editing
}

pub enum AutoFocus {
    Start,  // Focus at document start
    End,    // Focus at document end
    None,   // Don't auto-focus (default)
}
```

## StarterKit Extensions

The `StarterKit` provides 22 pre-configured extensions covering all standard editing operations. These are organized into node and mark extensions.

### Node Extensions (12 types)

| Extension | Tag | Purpose |
|-----------|-----|---------|
| **Document** | `doc` | Root container (content: `block+`) |
| **Paragraph** | `<p>` | Default text block (content: `inline*`) |
| **Text** | - | Inline text content |
| **Heading** | `<h1>-<h6>` | Section headings with level attribute |
| **Blockquote** | `<blockquote>` | Quoted content (content: `block+`) |
| **Bullet List** | `<ul>` | Unordered list (content: `list_item+`) |
| **Ordered List** | `<ol>` | Numbered list (content: `list_item+`) |
| **List Item** | `<li>` | List entry (content: `block+`) |
| **Code Block** | `<pre><code>` | Fenced code with language attr |
| **Horizontal Rule** | `<hr>` | Visual divider (atomic) |
| **Hard Break** | `<br>` | Line break (inline, atomic) |
| **Image** | `<img>` | Embedded image (requires src) |

### Mark Extensions (10 types)

| Extension | HTML | Shortcut | Purpose |
|-----------|------|----------|---------|
| **Bold** | `<strong>` | Mod-B | **Bold text** |
| **Italic** | `<em>` | Mod-I | *Italic text* |
| **Underline** | `<u>` | Mod-U | Underlined text |
| **Strikethrough** | `<s>` | Mod-Shift-X | ~~Struck text~~ |
| **Code** | `<code>` | Mod-E | `Inline code` |
| **Link** | `<a href>` | - | Clickable links |
| **Highlight** | `<mark>` | Mod-Shift-H | Highlighted text |
| **Subscript** | `<sub>` | Mod-, | x₂ subscript |
| **Superscript** | `<sup>` | Mod-. | x² superscript |
| **Text Color** | `<span>` | - | Colored text (requires color attr) |

### Using StarterKit

```rust
use rinch_editor::prelude::*;

// Create schema with all 22 extensions
let schema = Schema::starter_kit();

// Or manually load extensions
let mut editor = Editor::new(schema, EditorConfig::default())?;
for ext in StarterKit::extensions() {
    editor.extensions.register(ext)?;
}
```

## Keyboard Shortcuts

All StarterKit extensions come with keyboard shortcuts. Here's the complete reference:

### Text Formatting

| Shortcut | Action |
|----------|--------|
| Mod-B | Toggle **bold** |
| Mod-I | Toggle *italic* |
| Mod-U | Toggle <u>underline</u> |
| Mod-Shift-X | Toggle ~~strikethrough~~ |
| Mod-E | Toggle `code` |
| Mod-Shift-H | Toggle highlight |
| Mod-, | Toggle subscript |
| Mod-. | Toggle superscript |

**Note:** "Mod" means Ctrl on Windows/Linux and Cmd on macOS.

### Block Structure

| Shortcut | Action |
|----------|--------|
| Mod-Alt-0 | Convert to paragraph |
| Mod-Alt-1 | Convert to heading 1 |
| Mod-Alt-2 | Convert to heading 2 |
| Mod-Alt-3 | Convert to heading 3 |
| Mod-Alt-4 | Convert to heading 4 |
| Mod-Alt-5 | Convert to heading 5 |
| Mod-Alt-6 | Convert to heading 6 |

### Lists and Quotes

| Shortcut | Action |
|----------|--------|
| Mod-Shift-B | Toggle blockquote |
| Mod-Shift-8 | Toggle bullet list |
| Mod-Shift-7 | Toggle ordered list |

### Special

| Shortcut | Action |
|----------|--------|
| Mod-Alt-C | Toggle code block |
| Shift-Enter | Insert hard break |

## Markdown Input Rules

StarterKit includes markdown-style input rules that auto-convert patterns to formatted content:

### Headings

Type any of these at the start of a line, followed by space:

```
# <space>     → Heading 1
## <space>    → Heading 2
### <space>   → Heading 3
#### <space>  → Heading 4
##### <space> → Heading 5
###### <space> → Heading 6
```

### Lists

```
- <space>  → Bullet list (dash)
* <space>  → Bullet list (asterisk)
1. <space> → Ordered list
```

### Quotes and Code

```
> <space>  → Blockquote
```<space> → Code block
---        → Horizontal rule
```

### Inline Formatting

These patterns auto-toggle marks while typing:

```
**text**   → Bold
*text*     → Italic
~~text~~   → Strikethrough
```

## Commands

All mutations go through the command system. Commands are registered by extensions and dispatched by name:

```rust
// Toggle bold on current selection
editor.commands.execute("toggle_bold")?;

// Set heading level
editor.commands.execute("set_heading_1")?;

// Insert elements
editor.commands.execute("insert_table")?;
```

### Command Categories

Commands are organized into three categories:

#### Text Commands
- `insert_text(text)` - Insert characters
- `delete_range(range)` - Delete text range
- `replace_range(range, text)` - Replace with text

#### Formatting Commands
- `toggle_mark(mark)` - Toggle mark on selection
- `remove_mark(mark)` - Remove mark from selection
- `set_mark(mark, attrs)` - Set mark with attributes

#### Structure Commands
- `set_block_type(type)` - Change node type
- `wrap_in(type)` - Wrap in container
- `lift()` - Lift out of container
- `split_block()` - Split at cursor
- `join_blocks()` - Merge adjacent blocks

## Table Editing

The optional `TableExtension` provides full table editing support with 11 commands:

### Table Commands

| Command | Purpose |
|---------|---------|
| `insert_table` | Insert 3x3 table |
| `delete_table` | Remove table |
| `add_row_before` | Insert row before cursor |
| `add_row_after` | Insert row after cursor |
| `delete_row` | Delete current row |
| `add_column_before` | Insert column before cursor |
| `add_column_after` | Insert column after cursor |
| `delete_column` | Delete current column |
| `merge_cells` | Merge selected cells |
| `split_cell` | Split cell (if colspan/rowspan > 1) |
| `toggle_header_row` | Promote/demote first row as header |

### Table Navigation

| Shortcut | Action |
|----------|--------|
| Tab | Move to next cell |
| Shift-Tab | Move to previous cell |

### Table Structure

Tables contain:
- `<table>` - Container (group: block)
- `<table_row>` - Row (content: `table_cell+`) |
- `<table_cell>` - Cell (content: `block+`, attrs: colspan, rowspan)
- `<table_header>` - Header cell (content: `block+`, attrs: colspan, rowspan)

Enable tables:

```rust
let mut editor = Editor::new(Schema::starter_kit(), config)?;
editor.extensions.register(Box::new(TableExtension))?;
```

## Document Model

The editor document is backed by Automerge CRDT, enabling offline editing and automatic conflict resolution:

```rust
pub struct EditorDocument {
    // Automerge-backed CRDT document
}

impl EditorDocument {
    // Insert text at position
    pub fn insert_text(&mut self, pos: Position, text: &str) -> Result<(), EditorError>

    // Delete text range
    pub fn delete_range(&mut self, range: Range) -> Result<(), EditorError>

    // Add mark (formatting) to range
    pub fn add_mark(&mut self, range: Range, mark: &str, attrs: Option<HashMap<String, String>>) -> Result<(), EditorError>

    // Remove mark from range
    pub fn remove_mark(&mut self, range: Range, mark: &str) -> Result<(), EditorError>

    // Split block at position
    pub fn split_block(&mut self, pos: Position) -> Result<(), EditorError>

    // Get block type at position
    pub fn block_type(&self, pos: usize) -> Option<String>

    // Get document length
    pub fn length(&self) -> usize
}
```

### Position and Range

```rust
// Position: byte offset in document (0 = start)
pub struct Position {
    pub offset: usize,
}

// Range: contiguous span from start to end
pub struct Range {
    pub start: Position,
    pub end: Position,
}
```

## Schema System

The schema defines what content is valid. Every extension contributes node and mark specifications:

```rust
// Create a custom schema
let mut schema = Schema::new();

// Add node type
schema.add_node(NodeSpec::builder("paragraph")
    .content("inline*")
    .group("block")
    .build());

// Add mark type
schema.add_mark(MarkSpec::simple("bold"));
```

### Node Specs

```rust
pub struct NodeSpec {
    pub name: String,
    pub content: Option<String>,    // Content model (e.g., "inline*", "block+")
    pub group: Option<String>,      // Grouping (block, inline)
    pub attrs: HashMap<String, AttrSpec>,
    pub inline: bool,               // Inline vs block
    pub atom: bool,                 // Atomic (no content)
    pub isolating: bool,            // Boundary for operations
    pub marks: MarkSet,             // Which marks allowed
    pub parse_html_tags: Vec<String>,
}
```

### Mark Specs

```rust
pub struct MarkSpec {
    pub name: String,
    pub attrs: HashMap<String, AttrSpec>,
    pub parse_html_tags: Vec<String>,
    pub inclusive: bool,            // Extend to typing (default: true)
    pub excludes: Option<String>,   // Conflicts (e.g., "bold italic")
}
```

## Selection and Cursor

The selection tracks the current cursor position or selection range:

```rust
pub struct Selection {
    pub anchor: Position,    // Selection start
    pub head: Position,      // Selection end (same as anchor for cursor)
}

impl Selection {
    // Cursor at position
    pub fn cursor(pos: Position) -> Self

    // Range from start to end
    pub fn range(start: Position, end: Position) -> Self

    // Check if collapsed (cursor, not range)
    pub fn is_collapsed(&self) -> bool
}

// Update selection
editor.set_selection(Selection::cursor(Position::new(10)));
```

## Undo/Redo

Rinch editor includes local undo/redo for collaborative editing. Operations are stored with inverses for reversal:

```rust
pub struct History {
    pub undo_stack: Vec<UndoOperation>,
    pub redo_stack: Vec<UndoOperation>,
}

pub struct UndoOperation {
    pub changes: Vec<DocumentChange>,
    pub inverse: Vec<DocumentChange>,  // Undo reverses these
}

// Undo last operation
editor.history.undo();

// Redo last undone operation
editor.history.redo();
```

Each operation is atomic - undo/redo are single steps regardless of how many DOM mutations happened.

## Extension System

Create custom extensions by implementing the `Extension` trait:

```rust
use rinch_editor::prelude::*;

#[derive(Debug)]
struct MyCustomExtension;

impl Extension for MyCustomExtension {
    fn name(&self) -> &str {
        "my_custom"
    }

    fn nodes(&self) -> Vec<NodeSpec> {
        vec![NodeSpec::builder("custom_node")
            .content("inline*")
            .group("block")
            .build()]
    }

    fn marks(&self) -> Vec<MarkSpec> {
        vec![MarkSpec::simple("custom_mark")]
    }

    fn commands(&self) -> Vec<CommandRegistration> {
        vec![CommandRegistration::new("my_command", |editor| {
            // Command logic here
            Ok(())
        })]
    }

    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> {
        vec![(
            KeyboardShortcut::new("Mod-K", "My command"),
            "my_command".into(),
        )]
    }

    fn input_rules(&self) -> Vec<InputRule> {
        vec![InputRule::new(
            regex::Regex::new(r"^:wave: $").unwrap(),
            "Wave emoji",
            |_editor, _caps| Ok(()),
        )]
    }
}

// Register extension
editor.extensions.register(Box::new(MyCustomExtension))?;
```

### Extension Priorities

Extensions can specify priority (lower = earlier initialization):

```rust
impl Extension for MyExtension {
    fn priority(&self) -> i32 {
        50  // Lower than default (100)
    }
}
```

Use priorities to control initialization order when extensions have dependencies.

## Events

The editor dispatches events for state changes:

```rust
pub struct EventDispatcher {
    // Event routing system
}

// Listen to updates
editor.events.on("update", |event| {
    // Handle update
});
```

## Testing

Rinch provides testing utilities:

```rust
use rinch_editor::prelude::*;

#[test]
fn test_bold_toggle() {
    let schema = Schema::starter_kit();
    let mut editor = Editor::new(schema, EditorConfig::default()).unwrap();

    // Insert text
    editor.doc.insert_text(Position::new(0), "hello").unwrap();

    // Toggle bold
    editor.commands.execute("toggle_bold").unwrap();

    // Verify mark applied
    let marks = editor.doc.marks_at(Position::new(2));
    assert!(marks.iter().any(|m| m.name == "bold"));
}
```

## Advanced: Custom Document Rendering

For embedding the editor in your Rinch app, implement DOM rendering:

```rust
fn render_editor(__scope: &mut RenderScope, editor: &Editor) -> NodeHandle {
    let div = __scope.create_element("div");
    div.set_attribute("class", "editor");

    // Render document
    for node in editor.doc.nodes() {
        let rendered = render_node(__scope, node);
        div.append_child(&rendered);
    }

    // Render cursor/selection
    if let Some(cursor_node) = render_cursor(__scope, &editor.selection) {
        div.append_child(&cursor_node);
    }

    div
}
```

## Architecture

The editor uses a **Hidden Textarea + Virtual DOM** approach:

1. A hidden `<textarea>` captures keyboard input, IME, and clipboard
2. The document model (Automerge CRDT) is the source of truth
3. DOM nodes are rendered from the document using Rinch's NodeHandle API
4. Selection and cursor overlays provide visual feedback

This design enables:
- Full IME support (Chinese, Japanese, etc.)
- Native clipboard handling
- Undo/redo compatible with collaborative editing
- Surgical DOM updates (only changed nodes update)

See [Editor Architecture](../architecture/editor.md) for deep technical details.
