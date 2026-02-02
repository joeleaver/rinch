# Architecture Overview

Rinch is a lightweight cross-platform GUI library for Rust using fine-grained reactive rendering. The architecture emphasizes surgical DOM updates rather than full re-renders.

## Core Principle: Fine-Grained Reactivity

```
Signal.set()
    → Effect runs
    → NodeHandle.set_text() / set_attribute() / set_style()
    → Direct blitz Document mutation
    → mark_dirty() for re-layout
```

**Key insight:** `app()` runs once to build the DOM. Effects handle all reactive updates surgically. No HTML regeneration for simple updates.

## Crate Structure

```
rinch/
├── crates/
│   ├── rinch/           # Main application crate (shell, runtime)
│   ├── rinch-core/      # Core types, reactive primitives, DOM abstractions
│   ├── rinch-macros/    # rsx! proc macro (DOM construction)
│   ├── rinch-editor/    # Rich-text editor with CRDT collaboration
│   ├── rinch-theme/     # Theme system (CSS variables)
│   ├── rinch-widgets/   # UI widget library
│   └── rinch-renderer/  # Rendering abstraction
└── examples/
    └── smyeditor/       # Rich-text editor application
```

## Layer Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                     Application Layer                        │
│  (your app, smyeditor, etc.)                                │
├─────────────────────────────────────────────────────────────┤
│                        rinch                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │   Shell     │  │   Menu      │  │   Window    │         │
│  │  (runtime)  │  │  (muda)     │  │  (winit)    │         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
│                                                              │
│  ┌─────────────────────────────────────────────────┐        │
│  │             DOM Adapter (BlitzDomAdapter)        │        │
│  │  RenderScope → NodeHandle → blitz Document      │        │
│  └─────────────────────────────────────────────────┘        │
├─────────────────────────────────────────────────────────────┤
│                      rinch-core                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │  Element    │  │  Reactive   │  │    DOM      │         │
│  │ (shell only)│  │  (signals)  │  │ (NodeHandle)│         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
├─────────────────────────────────────────────────────────────┤
│                     External Crates                          │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │   blitz     │  │   vello     │  │   winit     │         │
│  │  (HTML/CSS) │  │  (GPU 2D)   │  │ (windowing) │         │
│  └─────────────┘  └─────────────┘  └─────────────┘         │
└─────────────────────────────────────────────────────────────┘
```

## Key Components

### rinch-core

The foundation layer containing:

- **Reactive primitives** - `Signal<T>`, `Effect`, `Memo<T>` for state management
- **DOM abstractions** - `RenderScope`, `NodeHandle`, `DomDocument` trait
- **Hooks API** - `use_signal`, `use_effect`, `use_memo`, `use_context`
- **Element types** - Shell elements only (Window, AppMenu, Menu, MenuItem, ThemeProvider)
- **Event handling** - Input and click event dispatch

### rinch-macros

The `rsx!` proc macro that generates DOM construction code:

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

- **Shell/Runtime** - Application event loop, window lifecycle
- **DOM Adapter** - `BlitzDomAdapter` implementing `DomDocument` trait
- **Window Manager** - Window creation, blitz rendering integration
- **Menu Manager** - Native menu support via muda

### rinch-theme

Theme system with CSS variables:

- Color palettes (10 shades per color)
- Spacing, radius, typography scales
- Dark mode support
- CSS variable generation for widgets

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

### rinch-widgets

UI component library (~50 widgets):

- Input widgets: TextInput, Checkbox, Switch, Select
- Display widgets: Button, Badge, Alert, Card
- Layout widgets: Stack, Group, Grid, Container
- Overlay widgets: Modal, Drawer, Tooltip

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
    │      blitz Document          │  DOM mutation
    │   (surgical updates via      │
    │    NodeHandle methods)       │
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
| blitz-dom | HTML DOM implementation |
| blitz-html | HTML parsing |
| blitz-paint | CSS painting |
| vello | GPU 2D rendering |
| wgpu | GPU abstraction |
| winit | Cross-platform windowing |
| muda | Native menu support |

## Design Principles

1. **Fine-grained updates** - Only update what changed, never full re-render
2. **Declarative UI** - RSX syntax describes UI structure
3. **Reactive by default** - Signals and Effects for state management
4. **Web standards** - HTML/CSS for layout (via blitz)
5. **Native integration** - Native menus, file dialogs, system tray
