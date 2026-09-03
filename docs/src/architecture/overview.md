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
│   │   ├── src/app/        # Platform-agnostic RinchApp
│   │   └── src/shell/      # Desktop backend (winit + wgpu)
│   ├── rinch-core/         # Core types, reactive primitives, DOM abstractions
│   ├── rinch-macros/       # rsx! proc macro
│   ├── rinch-dom/          # HTML/CSS DOM (Taffy + Parley + Stylo + Vello)
│   ├── rinch-platform/     # Platform abstraction traits
│   ├── rinch-web/          # Browser-native DOM backend (WebDocument over web_sys)
│   ├── rinch-editor-core/  # Rich-text editor model (pure, renderer-agnostic)
│   ├── rinch-editor-view/  # Rich-text editor view (desktop + web projection)
│   ├── rinch-editor-collab/# Optional CRDT collaboration adapter (yrs)
│   ├── rinch-theme/        # Theme system (CSS variables)
│   ├── rinch-components/   # UI component library
│   ├── rinch-editable/     # Single-line <input>/<textarea> editing primitives
│   ├── rinch-clipboard/    # Cross-platform clipboard
│   ├── rinch-tabler-icons/ # 5000+ Tabler Icons
│   ├── rinch-debug/        # Debug IPC server
│   └── rinch-mcp-server/   # MCP server for Claude
└── examples/
    ├── ui-zoo-desktop/     # Desktop component showcase + rich-text editor
    └── ui-zoo-web/         # Web (WASM) component showcase using browser-native DOM
```

These are the principal crates; the workspace `Cargo.toml` has the full member list
(networking, storage, media, testing and platform-specific crates are omitted here).

## Layer Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                     Application Layer                         │
│  (your app, ui-zoo-desktop, ui-zoo, etc.)                         │
├──────────────────────────────────────────────────────────────┤
│                         rinch                                 │
│  ┌──────────────────────────────────────────────────┐        │
│  │          RinchApp (app/)                          │        │
│  │  Platform-agnostic application logic              │        │
│  │  handle_event(PlatformEvent, …) -> Vec<AppAction> │        │
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
│  (HTML/CSS)   │  (CSS vars)   │  (component library)         │
├──────────────────────────────────────────────────────────────┤
│                    External Crates                            │
│  Taffy (layout) │ Parley (text) │ Stylo (CSS) │ Vello/tiny-skia │
└──────────────────────────────────────────────────────────────┘
```

## Cross-Platform Architecture

Rinch uses a platform abstraction pattern to support multiple backends (desktop, web, mobile):

```rust
// Platform-agnostic application logic
impl RinchApp {
    pub fn handle_event(
        &mut self,
        event: PlatformEvent,
        window_size: (u32, u32), // physical pixels (pointer coords are logical)
        scale_factor: f64,
    ) -> Vec<AppAction> {
        match event {
            PlatformEvent::MouseDown { x, y, button } => {
                // Handle click, update DOM
                vec![AppAction::RequestRedraw]
            }
            PlatformEvent::KeyDown { key, modifiers, .. } => {
                // Handle keyboard input
                vec![AppAction::RequestRedraw]
            }
            // ...
        }
    }
}
```

`window_size` is always the **physical** surface size — the one genuinely
physical quantity crossing this boundary. Every shell converts it to the
logical (CSS-pixel) layout viewport with the shared `rinch_platform::to_logical`
— never by dividing inline — so that mount, resize and every other relayout
agree on the same viewport. Inside `RinchApp` this conversion happens once per
`handle_event` call via `RinchApp::layout_viewport`.

The **pointer** coordinates carried by `PlatformEvent` are the opposite: they
are **logical on every host**, and it is each shell's job to convert before
constructing the event, with `rinch_platform::to_logical_point`. The document is
laid out in CSS pixels and `hit_test` probes that layout tree directly, so a
shell that forwards its windowing system's physical pointer position displaces
every click by the scale factor times its distance from the window origin
(issue #299 — the desktop shell did exactly this until it was fixed).
`MouseWheel`'s pixel delta converts the same way.

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

- **Reactive primitives** - `Signal::new()`, `Effect::new()`, `Memo::new()`, plus
  scopes/ownership and `create_context()` / `create_store()`
- **DOM abstractions** - `RenderScope`, `NodeHandle`, `DomDocument` trait
- **Control flow** - `show_dom()`, `for_each_dom_typed()`, `match_dom()` (what rsx
  `if`/`for`/`match` desugar to)
- **Element types** - Minimal enum: `Html`, `Fragment`, `Component` only
- **Event handling** - Input, click, keyboard and drag event dispatch

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

- **RinchApp** (`app/`) - Platform-agnostic application logic (event dispatch, hit
  testing, focus arbitration, text selection)
- **Desktop backend** (`shell/rinch_runtime.rs`) - Event loop, window creation, rendering
- **Menu Manager** - Native menu support via muda
- **Editor wiring** (`editor/`) - Desktop-only editor pieces: block virtualization,
  AccessKit accessibility, the cross-thread collaboration inbound

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
- **Rendering** - Vello (GPU) or tiny-skia (software) via the Painter trait abstraction

### rinch-web

Browser-native DOM backend (the WASM target). Consumed by the `*-web` examples —
`ui-zoo-web`, `paint-web`, `editor-web`, `collab-editor-web`, and others:

- **WebDocument** - Implements `DomDocument` via `web_sys`, creating real browser DOM elements
- **No Taffy/Parley/Vello** - The browser handles layout, text shaping, and painting natively
- **Event delegation** - `setup_event_delegation` installs document-level listeners that dispatch via `data-rid` attributes (plus drag, render-surface routing, and `data-onsubmit`/`data-oninput`)
- **`mount` helper** - One call wires the WebDocument, builds the component tree, installs event delegation, and injects theme CSS
- **Rich-text editor** - Re-exports `Editor` / `EditorHandle` / `create_editor` from
  `rinch-editor-view` and owns the browser input glue, so editor app code is identical
  to desktop
- **Smaller binary** - ~3.2MB WASM (vs 11MB+ with Vello rendering)

### rinch-theme

Theme system with CSS variables:

- Color palettes (10 shades per color, Mantine-inspired)
- Spacing, radius, typography scales
- Dark mode support
- CSS variable generation for components

### rinch-editor-core / rinch-editor-view / rinch-editor-collab

The ProseMirror-style rich-text editor, split by how much each crate may know about a
renderer:

- **rinch-editor-core** — the **model-first** source of truth, and pure: no rinch-dom,
  winit, web_sys, parley, taffy, vello, or CRDT engine, and it compiles to `wasm32`.
  It holds the immutable `Node`/`Mark`/`Fragment`/`Slice` document tree, one char-based
  `Pos` space, a `ContentMatch` schema that is *enforced* at the step boundary,
  invertible `Step`s, `EditorState`/`Transaction`, the command catalogue and keymap,
  markdown input rules, Step-based undo/redo, and total schema-derived serialization
  (`DocNode` JSON, HTML, markdown). It defines the `EditorView` seam and knows nothing
  about any renderer.
- **rinch-editor-view** — the projection: `RinchDomEditorView` renders an
  `EditorState` onto *any* `DomDocument` host, so the desktop (rinch-dom) and browser
  (`web_sys`) editors share one view. Also the `Editor {}` component, the imperative
  `EditorHandle`, the mounted-editor registry and the default stylesheet.
- **rinch-editor-collab** — optional, off by default: projects the model onto a **yrs**
  (Yjs) CRDT for real-time collaboration and rebuilds remote changes back into `Step`s.
  The only crate in the workspace that links a CRDT engine.

**Mutation flows one way:** every edit is a `Transaction` applied by
`EditorState::apply`, after which the view diffs the document and patches the host.
The host tree is never read back for content, and commands query state, never the DOM.

See [Editor Architecture](./editor.md) for technical details.

### rinch-components

UI component library:

- Input components: TextInput, Checkbox, Switch, Select, Slider
- Display components: Button, Badge, Alert, Card, Notification
- Layout components: Stack, Group, Grid, Container, Accordion
- Navigation components: Tabs, NavLink, Breadcrumbs, Pagination
- Overlay components: Modal, Drawer, Tooltip, Popover, Menu
- Typography components: Text, Title, Code, Blockquote
- Data display: Table, List, Timeline, Avatar

### rinch-editable

Editing primitives for single-line `<input>` and `<textarea>` widgets — a **separate
engine**, unrelated to the rich-text editor above (they share no code and no types,
despite both having a `Selection`):

- **EditableDocument** - Trait for text editing operations
- **StringDocument** - Single-line text document
- **EditCommand** - Enum of editing commands (insert, delete, etc.)
- **EditableState** - Cursor position, selection state
- **InputHandler** - Maps keyboard input to commands

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
    │    Vello / tiny-skia         │  GPU or software rendering
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
| **Vello** | GPU-accelerated 2D rendering (GPU mode) |
| **tiny-skia** | CPU-based 2D rendering (software mode) |
| **softbuffer** | Software window presentation (software mode) |
| **wgpu** | Cross-platform GPU abstraction (WebGPU API) |
| **winit** | Cross-platform windowing and input |
| **muda** | Native menu support (macOS/Windows/Linux) |
| **arboard** | Cross-platform clipboard access |
| **yrs** | CRDT for collaborative editing (rinch-editor-collab, opt-in) |

## Design Principles

1. **Fine-grained updates** - Only update what changed, never full re-render
2. **Declarative UI** - RSX syntax describes UI structure
3. **Reactive by default** - Signals and Effects for state management
4. **Web standards** - HTML/CSS for layout via Taffy and Stylo
5. **Platform abstraction** - Write once, run on desktop and web
6. **Native integration** - Native menus, file dialogs, clipboard, system tray
7. **Developer experience** - MCP integration for visual testing and debugging
