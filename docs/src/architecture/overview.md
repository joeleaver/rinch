# Architecture Overview

Rinch is a lightweight cross-platform GUI library for Rust using fine-grained reactive rendering. The architecture emphasizes surgical DOM updates rather than full re-renders, with platform abstraction for desktop and web targets.

## Core Principle: Fine-Grained Reactivity

```
Signal.set()
    → Effect runs
    → NodeHandle.set_text() / set_attribute() / set_style()
    → Direct RinchDocument mutation
    → mark_dirty() for re-layout
```

**Key insight:** `app()` runs once to build the DOM. Effects handle all reactive updates surgically. No HTML regeneration for simple updates.

## Crate Structure

```
rinch/
├── crates/
│   ├── rinch/              # Main application crate
│   │   ├── src/app.rs      # Platform-agnostic RinchApp
│   │   └── src/shell/      # Desktop backend (winit + wgpu)
│   ├── rinch-core/         # Core types, reactive primitives, DOM abstractions
│   ├── rinch-macros/       # rsx! proc macro
│   ├── rinch-dom/          # HTML/CSS DOM (Taffy + Parley + Stylo + Vello)
│   ├── rinch-platform/     # Platform abstraction traits
│   ├── rinch-web/          # WASM backend stubs
│   ├── rinch-editor/       # Rich-text editor (CRDT)
│   ├── rinch-theme/        # Theme system (CSS variables)
│   ├── rinch-components/   # UI component library (~55 components)
│   ├── rinch-editable/     # Text editing abstractions
│   ├── rinch-clipboard/    # Cross-platform clipboard
│   ├── rinch-tabler-icons/ # 5000+ Tabler Icons
│   ├── rinch-debug/        # Debug IPC server
│   └── rinch-mcp-server/   # MCP server for Claude
└── examples/
    ├── ui-zoo-desktop/     # Desktop component showcase + rich-text editor
    └── ui-zoo-web/         # Web (WASM) component showcase using browser-native DOM
```

## Layer Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                     Application Layer                         │
│  (your app, ui-zoo-desktop, ui-zoo, etc.)                         │
├──────────────────────────────────────────────────────────────┤
│                         rinch                                 │
│  ┌──────────────────────────────────────────────────┐        │
│  │          RinchApp (app.rs)                        │        │
│  │  Platform-agnostic application logic              │        │
│  │  handle_event(PlatformEvent) -> Vec<AppAction>    │        │
│  └──────────────────────────────────────────────────┘        │
│                          │                                    │
│         ┌────────────────┼────────────────┐                  │
│         ▼                ▼                ▼                   │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│  │  Desktop    │  │   Web       │  │  (future)   │          │
│  │ winit+wgpu  │  │  web-sys    │  │  mobile     │          │
│  └─────────────┘  └─────────────┘  └─────────────┘          │
├──────────────────────────────────────────────────────────────┤
│                    rinch-platform                             │
│  PlatformWindow, PlatformRenderer, PlatformEventLoop         │
│  PlatformEvent, AppAction, KeyCode, Modifiers                │
├──────────────────────────────────────────────────────────────┤
│                      rinch-core                               │
│  Signal, Effect, Memo, RenderScope, NodeHandle, DomDocument  │
├──────────────────────────────────────────────────────────────┤
│  rinch-dom    │  rinch-theme  │  rinch-components            │
│  (HTML/CSS)   │  (CSS vars)   │  (~55 components)            │
├──────────────────────────────────────────────────────────────┤
│                    External Crates                            │
│  Taffy (layout) │ Parley (text) │ Stylo (CSS) │ Vello (GPU) │
└──────────────────────────────────────────────────────────────┘
```

## Cross-Platform Architecture

Rinch uses a platform abstraction pattern to support multiple backends (desktop, web, mobile):

```rust
// Platform-agnostic application logic
impl RinchApp {
    fn handle_event(&mut self, event: PlatformEvent) -> Vec<AppAction> {
        match event {
            PlatformEvent::MouseDown { x, y, button } => {
                // Handle click, update DOM
                vec![AppAction::RequestRedraw]
            }
            PlatformEvent::KeyPress { key, modifiers } => {
                // Handle keyboard input
                vec![AppAction::RequestRedraw]
            }
            // ...
        }
    }
}
```

**Platform backends** implement traits from `rinch-platform`:
- `PlatformWindow` - Window creation, properties, frame buffer access
- `PlatformRenderer` - GPU rendering (wgpu for desktop, browser-native DOM for web)
- `PlatformEventLoop` - Event loop integration
- `PlatformMenu` - Native menu support

**Event flow**: Platform backend → `PlatformEvent` → `RinchApp.handle_event()` → `Vec<AppAction>` → Platform backend

This separation allows:
- Core app logic to be platform-agnostic
- Easy testing with mock platforms
- Adding new platforms by implementing the traits
- Sharing code between desktop and web builds

## Key Components

### rinch-core

The foundation layer containing:

- **Reactive primitives** - `Signal<T>`, `Effect`, `Memo<T>` for state management
- **DOM abstractions** - `RenderScope`, `NodeHandle`, `DomDocument` trait
- **Reactive Primitives** - `Signal::new()`, `Effect::new()`, `Memo::new()`, `create_context()`
- **Element types** - Minimal enum: `Html`, `Fragment`, `Component` only
- **Event handling** - Input and click event dispatch
- **Icon enum** - Curated set of ~40 common icons for components

### rinch-macros

The `#[component]` attribute macro and `rsx!` proc macro that generate DOM construction code:

```rust
// This RSX syntax:
rsx! {
    div { class: "container",
        p { "Count: " {|| count.get().to_string()} }
    }
}

// Generates code that:
// 1. Creates DOM nodes via RenderScope
// 2. Sets up Effects for reactive expressions {|| ...}
// 3. Wires event handlers
```

### rinch

The main crate that ties everything together:

- **RinchApp** (`app.rs`) - Platform-agnostic application logic
- **Desktop backend** (`shell/rinch_runtime.rs`) - Event loop, window creation, rendering
- **Event loop** (`shell/rinch_runtime.rs`) - Desktop runtime: event loop, window creation, rendering
- **Menu Manager** - Native menu support via muda

### rinch-platform

Platform abstraction layer defining cross-platform traits:

- **Traits** - `PlatformWindow`, `PlatformRenderer`, `PlatformEventLoop`, `PlatformMenu`
- **Types** - `PlatformEvent`, `AppAction`, `KeyCode`, `Modifiers`, `MouseButton`

### rinch-dom

HTML/CSS DOM implementation:

- **RinchDocument** - Implements `DomDocument` trait from rinch-core
- **Layout** - Taffy for flexbox/grid layout
- **Text** - Parley for text shaping and line breaking
- **Styling** - Stylo for CSS parsing and computed styles
- **Rendering** - Vello for GPU-accelerated 2D graphics

### rinch-web / ui-zoo-web

WASM backend using browser-native DOM:

- **WebDocument** - Implements `DomDocument` via `web_sys`, creating real browser DOM elements
- **No Taffy/Parley/Vello** - The browser handles layout, text shaping, and painting natively
- **Event delegation** - Document-level listeners dispatch via `data-rid` attributes
- **Smaller binary** - ~3.2MB WASM (vs 11MB+ with Vello rendering)

### rinch-theme

Theme system with CSS variables:

- Color palettes (10 shades per color, Mantine-inspired)
- Spacing, radius, typography scales
- Dark mode support
- CSS variable generation for components

### rinch-editor

Rich-text editor with collaborative editing support:

- **CRDT-backed document** - Automerge for offline editing and conflict resolution
- **Schema system** - Define valid document structure with nodes and marks
- **22 StarterKit extensions** - Complete editing experience out of the box
- **Command system** - All mutations go through named commands
- **Extension system** - Add custom nodes, marks, commands, shortcuts
- **Keyboard shortcuts** - 16+ built-in shortcuts (Mod-B, Mod-I, etc.)
- **Markdown input rules** - Auto-convert markdown patterns (# heading, **bold**, etc.)
- **Table editing** - Full table support with merge/split/navigation
- **Local undo/redo** - Compatible with collaborative editing

See [Editor Architecture](./editor.md) for technical details.

### rinch-components

UI component library (~55 components):

- Input components: TextInput, Checkbox, Switch, Select, Slider
- Display components: Button, Badge, Alert, Card, Notification
- Layout components: Stack, Group, Grid, Container, Accordion
- Navigation components: Tabs, NavLink, Breadcrumbs, Pagination
- Overlay components: Modal, Drawer, Tooltip, Popover, Menu
- Typography components: Text, Title, Code, Blockquote
- Data display: Table, List, Timeline, Avatar

### rinch-editable

Text editing abstractions:

- **EditableDocument** - Trait for text editing operations
- **EditCommand** - Enum of editing commands (insert, delete, etc.)
- **EditableState** - Cursor position, selection state

### rinch-clipboard

Cross-platform clipboard abstraction:

- **arboard** for native platforms
- **web-sys** for WASM
- Unified API: `copy_text()`, `paste_text()`, `has_text()`

### rinch-tabler-icons

5000+ Tabler Icons with type-safe enum:

- **Build-time download** - Icons fetched from Tabler CDN during `cargo build`
- **Type-safe** - `TablerIcon` enum instead of strings
- **Two styles** - Outline and Filled variants
- **Tree-shaking friendly** - Rust dead code elimination removes unused icons
- **render_tabler_icon()** - Helper function for rendering icons

### rinch-debug

Debug IPC server for development tools:

- **TCP listener** - Auto-starts on random localhost port
- **Discovery files** - Writes `~/.rinch/debug/{pid}.json`
- **Protocol** - Length-prefixed JSON over TCP
- **Commands** - DOM inspection, screenshot, input simulation

### rinch-mcp-server

MCP server for Claude integration:

- **Standalone binary** - Connects to running rinch apps via TCP
- **Discovery** - Scans `~/.rinch/debug/*.json` to find apps
- **MCP tools** - `screenshot`, `dom_tree`, `click`, `type_text`, `query_selector`, etc.
- **Auto-connect** - Automatically connects when only one app is running

## Data Flow

```
         User Code (app function)
                   │
                   ▼
    ┌──────────────────────────────┐
    │         rsx! macro           │  Compile time
    │   (generates DOM construction│
    │    code with Effects)        │
    └──────────────────────────────┘
                   │
                   ▼
    ┌──────────────────────────────┐
    │       RenderScope            │  Initial render
    │   (builds DOM tree once)     │  (runs app() once)
    └──────────────────────────────┘
                   │
         ┌────────┴────────┐
         ▼                 ▼
┌─────────────┐    ┌─────────────┐
│  NodeHandle │    │   Effects   │
│   (DOM refs)│    │ (reactive)  │
└─────────────┘    └─────────────┘
         │                 │
         │    Signal.set() │
         │        ↓        │
         │    Effect runs  │
         │        ↓        │
         └────────┬────────┘
                  │
                  ▼
    ┌──────────────────────────────┐
    │     RinchDocument            │  DOM mutation
    │   (surgical updates via      │
    │    NodeHandle methods)       │
    └──────────────────────────────┘
                  │
                  ▼
    ┌──────────────────────────────┐
    │   Taffy Layout Engine        │  Compute layout
    │   (flexbox/grid)             │
    └──────────────────────────────┘
                  │
                  ▼
    ┌──────────────────────────────┐
    │   Parley Text Shaping        │  Shape text
    │   (line breaking, glyphs)    │
    └──────────────────────────────┘
                  │
                  ▼
    ┌──────────────────────────────┐
    │         Vello                │  GPU rendering
    │       (re-paint)             │
    └──────────────────────────────┘
                  │
                  ▼
              Display
```

## Reactive Update Flow

When a signal changes:

1. **Signal.set()** - User code updates a signal
2. **Dependency notification** - Signal notifies subscribed Effects
3. **Effect execution** - Each Effect re-runs its closure
4. **DOM mutation** - Effect uses `NodeHandle` to update specific DOM nodes
5. **Mark dirty** - Changed nodes are marked for re-layout
6. **Re-paint** - Vello re-renders the affected region

This is much more efficient than regenerating HTML and replacing the entire document.

## External Dependencies

| Crate | Purpose |
|-------|---------|
| **Taffy** | Flexbox and grid layout engine |
| **Parley** | Text shaping, line breaking, bidirectional text |
| **Stylo** | CSS parsing and computed style resolution |
| **Vello** | GPU-accelerated 2D rendering |
| **wgpu** | Cross-platform GPU abstraction (WebGPU API) |
| **winit** | Cross-platform windowing and input |
| **muda** | Native menu support (macOS/Windows/Linux) |
| **arboard** | Cross-platform clipboard access |
| **Automerge** | CRDT for collaborative editing (rinch-editor) |

## Design Principles

1. **Fine-grained updates** - Only update what changed, never full re-render
2. **Declarative UI** - RSX syntax describes UI structure
3. **Reactive by default** - Signals and Effects for state management
4. **Web standards** - HTML/CSS for layout via Taffy and Stylo
5. **Platform abstraction** - Write once, run on desktop and web
6. **Native integration** - Native menus, file dialogs, clipboard, system tray
7. **Developer experience** - MCP integration for visual testing and debugging
