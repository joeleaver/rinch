# Rich-Text Editor Architecture

The rinch-editor is a comprehensive rich-text editor built for the Rinch GUI framework with full collaborative editing support through Automerge CRDT.

## Design Overview

The editor uses a **Hidden Textarea + Custom Virtual DOM** approach to balance:
- Full IME and clipboard support
- Native browser input handling (without being in a browser)
- Surgical DOM updates (only changed nodes update)
- Collaboration-friendly undo/redo

### Key Principle

```
Hidden Textarea → Key Events → Editor Commands → Document Model → Virtual DOM Render
                  (Clipboard)      (Dispatch)      (Automerge)      (NodeHandle)
```

The document model (Automerge CRDT) is the **single source of truth**. All mutations go through commands, which update the document, which then updates the DOM via Effects.

## Core Components

### Editor Instance

```rust
pub struct Editor {
    pub doc: EditorDocument,           // Automerge CRDT document
    pub schema: Schema,                // Validation rules
    pub selection: SelectionState,     // Cursor/selection
    pub history: History,              // Local undo/redo
    pub commands: CommandDispatcher,   // Command execution
    pub extensions: ExtensionRegistry, // Loaded plugins
    pub input_rules: InputRuleSet,     // Auto-transforms
    pub shortcuts: ShortcutRegistry,   // Keyboard bindings
    pub events: EventDispatcher,       // Event routing
    pub config: EditorConfig,          // Settings
}
```

### Document Model

The `EditorDocument` wraps an Automerge document:

```rust
pub struct EditorDocument {
    // Contains:
    // - Block nodes (paragraph, heading, list, code_block, etc.)
    // - Inline content (text, marks)
    // - Mark data (bold, italic, link, etc.)
    //
    // Operations:
    // - insert_text(pos, text)
    // - delete_range(range)
    // - add_mark(range, mark, attrs)
    // - remove_mark(range, mark)
    // - split_block(pos)
}
```

**Key properties:**
- **CRDT-backed**: Changes can be merged with remote edits
- **Immutable snapshots**: Document state at any point can be accessed
- **Collaborative**: Multiple users can edit simultaneously
- **Local undo**: Operations track inverses for reversal

### Schema System

The schema defines valid document structure:

```rust
pub struct Schema {
    pub nodes: HashMap<String, NodeSpec>,
    pub marks: HashMap<String, MarkSpec>,
    pub top_node: String,  // Usually "doc"
}

pub struct NodeSpec {
    pub name: String,
    pub content: Option<String>,      // "block+", "inline*", etc.
    pub group: Option<String>,        // block, inline grouping
    pub attrs: HashMap<String, AttrSpec>,
    pub inline: bool,
    pub atom: bool,                   // No content (leaf node)
    pub isolating: bool,              // Boundary for operations
    pub marks: MarkSet,               // Which marks allowed
}

pub struct MarkSpec {
    pub name: String,
    pub attrs: HashMap<String, AttrSpec>,
    pub inclusive: bool,              // Extend to new typing
    pub excludes: Option<String>,     // Conflicting marks
}
```

### Extension System

Extensions are plugins that contribute to the editor:

```rust
pub trait Extension: Debug {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 { 100 }
    fn nodes(&self) -> Vec<NodeSpec> { vec![] }
    fn marks(&self) -> Vec<MarkSpec> { vec![] }
    fn commands(&self) -> Vec<CommandRegistration> { vec![] }
    fn keyboard_shortcuts(&self) -> Vec<(KeyboardShortcut, String)> { vec![] }
    fn input_rules(&self) -> Vec<InputRule> { vec![] }
    fn on_init(&self, editor: &mut Editor) -> Result<(), EditorError> { Ok(()) }
}
```

**Extension lifecycle:**
1. Extensions register nodes, marks, commands
2. Schema is built from all extensions
3. Shortcuts and input rules are collected
4. Extensions are initialized in priority order
5. Editor is ready for editing

## Command System

All mutations are commands dispatched through `CommandDispatcher`:

```rust
pub type Command = fn(&mut Editor) -> Result<(), EditorError>;

pub struct CommandDispatcher {
    commands: HashMap<String, Command>,
}
```

### Command Categories

**Text Commands:**
- Direct text insertion/deletion
- Operations on the text content level

**Formatting Commands:**
- Toggle marks (bold, italic, etc.)
- Add/remove marks on ranges
- Set mark attributes (e.g., link href)

**Structure Commands:**
- Change block type (paragraph to heading)
- Wrap in container (create blockquote)
- Lift out of container (remove list wrapper)
- Split/join blocks

### Command Execution Flow

```
User Input (keyboard/UI)
    ↓
Resolve to command name (shortcut or direct call)
    ↓
CommandDispatcher.execute(name)
    ↓
Call command function with &mut Editor
    ↓
Command updates EditorDocument
    ↓
Document mutations trigger Effects
    ↓
DOM nodes update (NodeHandle calls)
```

## Input System

### Keyboard Shortcuts

```rust
pub struct KeyboardShortcut {
    pub key: String,              // "Mod-B", "Ctrl+I", etc.
    normalized: String,           // Normalized for matching
    pub description: String,
}

pub struct ShortcutRegistry {
    shortcuts: HashMap<String, String>,  // normalized → command name
}
```

**Key parsing:**
- Mod = Ctrl (Windows/Linux) or Cmd (Mac)
- Separators: `-` or `+` (normalized to `+`)
- Case-insensitive (normalized to lowercase)

### Input Rules

```rust
pub struct InputRule {
    pub pattern: Regex,
    pub description: String,
    pub handler: fn(&mut Editor, &Captures) -> Result<(), EditorError>,
}

pub struct InputRuleSet {
    rules: Vec<InputRule>,
}
```

**Examples:**
- Pattern: `^# $` → Convert to Heading 1
- Pattern: `**(.+)\*\*$` → Apply bold mark
- Pattern: `^---$` → Insert horizontal rule

Input rules run AFTER text insertion, checking if the last line matches a pattern. If matched, the handler modifies the document.

## History and Undo/Redo

The `History` system is designed for collaborative editing:

```rust
pub struct History {
    pub undo_stack: Vec<UndoOperation>,
    pub redo_stack: Vec<UndoOperation>,
}

pub struct UndoOperation {
    pub changes: Vec<DocumentChange>,
    pub inverse: Vec<DocumentChange>,  // Reverses these changes
}
```

**Key design:**
- Operations are atomic (multiple DOM changes = one undo step)
- Inverses are automatically computed
- Compatible with CRDT merging (local undo doesn't break collaboration)
- Redo pushes undone operations back to undo stack

## StarterKit: 22 Default Extensions

The `StarterKit` bundles 22 extensions for full-featured editing:

### Nodes (12)

| Name | Group | Content | Purpose |
|------|-------|---------|---------|
| doc | - | block+ | Root node |
| paragraph | block | inline* | Default text block |
| text | inline | - | Inline text (atomic) |
| heading | block | inline* | h1-h6 with level attr |
| blockquote | block | block+ | Quote wrapper |
| bullet_list | block | list_item+ | Unordered list |
| ordered_list | block | list_item+ | Numbered list |
| list_item | - | block+ | List entry |
| code_block | block | text* | Pre-formatted code |
| horizontal_rule | block | - | Visual divider (atomic) |
| hard_break | inline | - | Line break (atomic) |
| image | inline | - | Image embed (atomic) |

### Marks (10)

| Name | Shortcut | HTML | Attributes |
|------|----------|------|------------|
| bold | Mod-B | `<strong>` | - |
| italic | Mod-I | `<em>` | - |
| underline | Mod-U | `<u>` | - |
| strike | Mod-Shift-X | `<s>` | - |
| code | Mod-E | `<code>` | excludes: bold, italic, underline, strike |
| link | - | `<a>` | href (required), title, target |
| highlight | Mod-Shift-H | `<mark>` | color (optional) |
| subscript | Mod-, | `<sub>` | excludes: superscript |
| superscript | Mod-. | `<sup>` | excludes: subscript |
| text_color | - | `<span>` | color (required) |

## Table Extension

The optional `TableExtension` adds table editing:

```rust
pub struct TableExtension;

impl Extension for TableExtension {
    fn nodes(&self) -> Vec<NodeSpec> {
        vec![
            NodeSpec::builder("table")      // block container
                .content("table_row+")
                .isolating(true)            // Operations don't cross boundary
                .build(),
            NodeSpec::builder("table_row")
                .content("table_cell+")
                .build(),
            NodeSpec::builder("table_cell")
                .content("block+")
                .attr("colspan", AttrSpec::optional("1"))
                .attr("rowspan", AttrSpec::optional("1"))
                .build(),
            NodeSpec::builder("table_header")
                .content("block+")
                .attr("colspan", AttrSpec::optional("1"))
                .attr("rowspan", AttrSpec::optional("1"))
                .build(),
        ]
    }
}
```

**Commands (11 total):**
- `insert_table` - Create 3x3 table
- `delete_table` - Remove table
- `add_row_before`, `add_row_after`, `delete_row`
- `add_column_before`, `add_column_after`, `delete_column`
- `merge_cells` - Combine selected cells
- `split_cell` - Undo colspan/rowspan
- `toggle_header_row` - Promote first row

**Navigation:**
- Tab = Next cell
- Shift-Tab = Previous cell

## Selection and Positions

### Position

```rust
pub struct Position {
    pub offset: usize,  // Byte offset in document
}

impl Position {
    pub fn new(offset: usize) -> Self
}
```

Positions are 0-indexed from document start.

### Range

```rust
pub struct Range {
    pub start: Position,
    pub end: Position,
}
```

### Selection

```rust
pub struct Selection {
    pub anchor: Position,   // Selection start (fixed)
    pub head: Position,     // Selection end (can move)
}

impl Selection {
    pub fn cursor(pos: Position) -> Self      // anchor == head
    pub fn range(start: Position, end: Position) -> Self
    pub fn is_collapsed(&self) -> bool        // Is it just a cursor?
}
```

**Key insight:** Even when selecting, both anchor and head point to positions in the document. Cursor operations check `is_collapsed()`.

## Rendering Integration

### Hidden Textarea Approach

```html
<!-- Editor container -->
<div class="editor">
    <!-- Hidden textarea for input -->
    <textarea style="position: absolute; opacity: 0;"></textarea>

    <!-- Rendered document (from NodeHandle API) -->
    <div class="document">
        <!-- Rendered nodes from editor.doc -->
    </div>

    <!-- Selection overlay -->
    <div class="selection-overlay">
        <!-- Cursor and selection highlights -->
    </div>
</div>
```

**Why hidden textarea?**
- Captures all keyboard input (arrow keys, meta keys, etc.)
- Full IME support (for CJK input)
- Native clipboard events (Ctrl-C/V, Cmd-C/V)
- Works without browser context

### Document Rendering

The editor must render its document to Rinch NodeHandles:

```rust
fn render_editor_doc(__scope: &mut RenderScope, editor: &Editor) -> NodeHandle {
    let container = __scope.create_element("div");
    container.set_attribute("class", "editor-doc");

    // Render document nodes
    for node in &editor.doc.nodes {
        let rendered = render_node(__scope, node);
        container.append_child(&rendered);
    }

    container
}

fn render_node(__scope: &mut RenderScope, node: &DocumentNode) -> NodeHandle {
    match node.type_name.as_str() {
        "paragraph" => {
            let p = __scope.create_element("p");
            for child in &node.children {
                let rendered = render_node(__scope, child);
                p.append_child(&rendered);
            }
            p
        }
        "text" => {
            let text = __scope.create_text(&node.content);
            text
        }
        // ... other node types
        _ => __scope.create_text(""),
    }
}
```

### Marks Rendering

Marks are applied after text nodes are created:

```rust
// For each mark span in the document:
let span = __scope.create_element("strong");  // For bold
for text in &mark.content {
    span.append_child(&__scope.create_text(text));
}
```

## Event Flow

```
Keyboard Event
    ↓
Hidden Textarea captures (e.g., "a" key)
    ↓
Check InputRules (does "a" complete a pattern?)
    ↓
Insert text in document
    ↓
Check shortcuts (is Mod-B pressed?)
    ↓
Execute command if matched
    ↓
EditorDocument updated
    ↓
Effects trigger DOM updates
    ↓
Render marks and styles
```

## CRDT Collaboration

The Automerge document enables offline editing and collaboration:

```
Local Editor          Remote Editor
    │                      │
    ├─ Edit locally ────→ Receive update
    │                      ├─ Merge into document
    ├─ Create change  ←─── Send change
    │                      └─ Re-render
    └─ Merge remote
```

Each change is recorded with metadata (actor, timestamp) allowing automatic merging without conflicts.

## Error Handling

```rust
pub enum EditorError {
    InvalidNodeType(String),
    InvalidMarkType(String),
    InvalidSelection,
    DocumentError(String),
    CommandNotFound(String),
    // ...
}
```

All document mutations return `Result<(), EditorError>`, allowing graceful error handling:

```rust
editor.doc.insert_text(pos, "hello")?;  // Propagate errors
match editor.commands.execute("toggle_bold") {
    Ok(()) => { /* success */ }
    Err(e) => eprintln!("Command failed: {}", e),
}
```

## Performance Characteristics

### Time Complexity

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| Insert text | O(n) | n = document length |
| Delete range | O(n) | Proportional to range size |
| Toggle mark | O(n) | Scans range for existing marks |
| Apply input rule | O(1) | Regex on last line only |
| Command dispatch | O(log k) | k = number of commands |
| Schema lookup | O(1) | HashMap |

### Memory

- Document size proportional to content
- CRDT overhead: ~2x text size (metadata)
- Undo stack: One entry per atomic operation
- Selection state: Minimal (two positions)

## Testing Utilities

Rinch provides test helpers:

```rust
#[cfg(test)]
fn test_example() {
    let schema = Schema::starter_kit();
    let config = EditorConfig::default();
    let mut editor = Editor::new(schema, config).unwrap();

    // Insert and verify
    editor.doc.insert_text(Position::new(0), "hello").unwrap();
    assert_eq!(editor.doc.length(), 5);

    // Toggle formatting
    editor.commands.execute("toggle_bold").unwrap();

    // Check result
    let marks = editor.doc.marks_at(Position::new(2));
    assert!(marks.iter().any(|m| m.name == "bold"));
}
```

## Extensibility Points

### Custom Extensions

Implement `Extension` to add:
- New node types (with content rules)
- New mark types (with attributes)
- Commands (via `CommandRegistration`)
- Shortcuts (via `KeyboardShortcut`)
- Input rules (via `InputRule`)

### Custom Schema

Build your own schema without StarterKit:

```rust
let mut schema = Schema::new();
schema.add_node(my_custom_node);
schema.add_mark(my_custom_mark);
```

### Custom Commands

Register commands directly:

```rust
editor.commands.register("my_command", |editor| {
    // Command implementation
    Ok(())
})?;
```

### Custom Renderers

Implement rendering for your document structure using Rinch's NodeHandle API (set_text, set_attribute, append_child, etc.).

## Related Documentation

- [User Guide: Rich-Text Editor](../guide/editor.md)
- [RenderScope API](./render-scope.md)
- [Fine-Grained Reactivity](./fine-grained.md)
