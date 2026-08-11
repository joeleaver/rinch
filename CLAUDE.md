# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on rinch-dom, Taffy, Parley, and dual rendering backends (Vello for GPU, tiny-skia for software). The goal is to provide a reactive GUI framework using HTML/CSS for layout.

**Key dependencies:**
- **rinch-dom** - HTML/CSS DOM implementation (Taffy for layout, Parley for text, Painter trait for rendering). Taffy is pinned at **0.12** in two places — `crates/rinch-dom/Cargo.toml` and the vendored `crates/stylo-taffy/Cargo.toml` — which must move together. 0.9 could not resolve a percentage `min-height`/`max-height` against a block containing block (its block algorithm hard-coded that basis as indefinite), so `min-height: 100%` silently collapsed to the content height.
- **vello** - 2D GPU rendering via wgpu (GPU mode, enabled with `features = ["gpu"]`)
- **tiny-skia** - 2D software rendering (default mode, no GPU required)
- **softbuffer** - Software window presentation (default mode)
- **winit** - Cross-platform windowing and input
- **muda** - Native menu support

**Design philosophy:** Declarative UI with fine-grained reactive programming. The `rsx!` macro generates DOM construction code with Effects for reactive updates. Only changed DOM nodes are updated - no full re-renders.

## Build Commands

```bash
cargo build                    # Build all crates
cargo build -p ui-zoo-desktop       # Build the editor example
cargo run -p ui-zoo-desktop         # Run the rich-text editor
cargo clippy                   # Lint
cargo fmt                      # Format
```

## Architecture

```
crates/
├── rinch/                     # Main facade crate
│   ├── src/
│   │   ├── shell/            # Window management, event loop
│   │   │   └── rinch_runtime.rs         # Event loop, window creation, rendering
│   │   └── menu/             # Native menu support via muda
│   │       └── mod.rs        # MenuManager, MenuEntry builder API
│   └── ...
├── rinch-core/               # Core types
│   ├── src/element.rs        # Element enum (Html, Fragment, Component only), prop types
│   ├── src/context.rs        # Context API (create_context, use_context)
│   ├── src/reactive/         # Signal, Effect, Memo primitives
│   └── src/dom/              # NodeHandle, RenderScope, DomDocument trait
├── rinch-theme/              # Theme system (optional, enable with `theme` feature)
│   └── src/
│       ├── colors.rs         # Mantine color palettes (10 shades each)
│       ├── spacing.rs        # Spacing scale (xs, sm, md, lg, xl)
│       ├── typography.rs     # Font sizes, line heights, font families
│       ├── radius.rs         # Border radius scale
│       ├── shadows.rs        # Shadow definitions
│       ├── theme.rs          # Theme struct, defaults, builder
│       └── css.rs            # CSS variable generation
├── rinch-components/         # UI components (optional, enable with `components` feature)
│   └── src/
│       ├── button.rs         # Button component
│       ├── text_input.rs     # Text input component
│       ├── text.rs           # Typography component
│       ├── paper.rs          # Card container component
│       ├── stack.rs          # Vertical flex layout
│       ├── group.rs          # Horizontal flex layout
│       ├── badge.rs          # Status indicator
│       ├── icons.rs          # Hand-built chrome glyphs (chevron_up_dom, checkmark_dom, ...)
│       └── styles.rs         # Component CSS generation
├── rinch-tabler-icons/       # ~4,980 Tabler Icons (generated from vendored JSON)
│   ├── build.rs              # Generates the TablerIcon enum from data/ (downloads only as fallback)
│   ├── data/                 # Vendored Tabler JSON, committed — lets the crate build offline
│   └── src/lib.rs            # TablerIcon enum, render_tabler_icon function
├── rinch-editor-core/        # Pure editor model: steps, schema, state, commands, history (wasm-clean)
├── rinch-editor-view/        # Editor view over any DomDocument: EditorHandle, Editor component
├── rinch-editor-collab/      # Optional collab adapter: model <-> yrs (Yjs) CRDT; off by default
├── rinch-editable/           # Single-line <input>/<textarea> engine (not the rich editor)
├── rinch-debug/              # Debug IPC server (optional, enable with `debug` feature)
│   ├── src/lib.rs            # Public API: attach(), CommandSender, CommandReceiver
│   ├── src/protocol.rs       # Wire protocol: length-prefixed JSON, handshake
│   ├── src/server.rs         # TCP listener on background thread (no tokio)
│   └── src/discovery.rs      # ~/.rinch/debug/{pid}.json discovery files
├── rinch-mcp-server/         # Standalone MCP server binary for Claude
│   ├── src/main.rs           # Entry point: MCP on stdio, connects to apps via TCP
│   ├── src/mcp_server.rs     # MCP tools: list_apps, connect, screenshot, dom_tree, etc.
│   ├── src/client.rs         # TCP client for rinch-debug protocol
│   └── src/discovery.rs      # Scans ~/.rinch/debug/*.json, validates PIDs
└── rinch-renderer/           # (placeholder for custom rendering)

examples/
├── ui-zoo/                    # Shared component showcase library
├── ui-zoo-desktop/            # Desktop entry point - primary development target
├── ui-zoo-web/                # WASM browser-native DOM entry point
├── hello_rinch_dom/           # Minimal hello world
├── fine_grained_window/       # Fine-grained rendering demo
└── todo-app/                  # Todo app example
```

## Element Enum

The Element enum is minimal - only used for content that needs to be embedded in the DOM tree:

- `Element::Html(String)` - Raw HTML content rendered by rinch-dom
- `Element::Fragment(Children)` - Groups multiple elements
- `Element::Component(Rc<dyn Component>, Children)` - Custom component implementation

Shell-level constructs (windows, menus, themes) are handled at the runtime level via props, not as Element variants. See the "Application Entry Point" and "Native Menus" sections below.

DOM content is built using `#[component]` functions that return a `NodeHandle`. The `#[component]` macro injects a `RenderScope` (`__scope`) automatically:

```rust
#[component]
fn my_component() -> NodeHandle {
    let div = __scope.create_element("div");
    let text = __scope.create_text("Hello, world!");
    div.append_child(&text);
    div
}
```

## Icon System

There is **one** icon system: the **`TablerIcon` enum** in `rinch-tabler-icons`, which is also what every component's icon prop takes (`Option<TablerIcon>`). There is no `Icon` enum in the workspace — an older curated `rinch-core` `Icon` enum was removed, so treat any doc comment or snippet still showing `Icon::CheckCircle` as stale.

`TablerIcon` is **not** in the `rinch` prelude — the facade doesn't depend on `rinch-tabler-icons`. Any crate that names an icon variant (including one just passing `icon:` to a component) must depend on it directly.

**Add to Cargo.toml:**
```toml
rinch-tabler-icons = { workspace = true }
```

The crate generates the enum at build time from **vendored JSON committed under `crates/rinch-tabler-icons/data/`**, pinned to `@tabler/icons@3.36.1`, so it builds with no network. It only downloads from unpkg if a vendored file is missing or invalid.

**Usage:**
```rust
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

#[component]
fn my_component() -> NodeHandle {
    rsx! {
        div {
            // Render an icon
            {render_tabler_icon(__scope, TablerIcon::Home, TablerIconStyle::Outline)}

            // Filled variant
            {render_tabler_icon(__scope, TablerIcon::Heart, TablerIconStyle::Filled)}
        }
    }
}
```

The `tabler_icon!` macro is a shorter equivalent when the icon is a literal variant name: `tabler_icon!(__scope, Home)` (Outline is the default style) or `tabler_icon!(__scope, Home, Filled)`.

For size, stroke width, or a CSS class, use `render_tabler_icon_with_options(__scope, icon, TablerIconOptions { style, class, size, stroke_width })` — all fields but `style` are `Option`, so `..Default::default()` covers the rest.

**Features:**
- **4,983 icons** (`ICON_COUNT`; iterate `ALL_ICONS`), every one available in Outline
- **Filled is a subset** — about 1,000 icons have real filled artwork. Asking for `TablerIconStyle::Filled` on any other icon silently falls back to its outline paths, so a glyph that still looks like an outline is expected, not a bug
- **Type-safe** - Use enum variants instead of strings
- **Scales with font size** - Icons carry a `0 0 24 24` viewBox and are sized `1em` unless you pass an explicit `size`, so they track the parent's `font-size`
- **Themeable** - Outline icons use `stroke: currentColor` (default `stroke-width: 2`), filled icons `fill: currentColor`
- **Offline build** - Generated from the vendored JSON in `data/`; no network needed

**Using with ActionIcon:**
```rust
// Pass the rendered icon as a child
ActionIcon {
    variant: "subtle",
    onclick: || do_something(),
    {render_tabler_icon(__scope, TablerIcon::Menu2, TablerIconStyle::Outline)}
}
```

**Sample Categories:**
- Navigation: `Home`, `ArrowLeft`, `ArrowRight`, `ChevronUp`, `ChevronDown`, `Menu2`
- Actions: `Plus`, `Minus`, `X`, `Check`, `Edit`, `Trash`, `Search`, `Settings`
- Status: `AlertCircle`, `AlertTriangle`, `CircleCheck`, `InfoCircle`, `CircleX`
- Communication: `Mail`, `Phone`, `Message`, `Bell`, `Send`, `Share`
- Media: `Photo`, `Video`, `Music`, `Microphone`, `Camera`, `PlayerPlay`
- Files: `File`, `Folder`, `Download`, `Upload`, `Copy`, `ClipboardCopy`

### Components with Icon Support

Every one of these props is `Option<TablerIcon>`. The `rsx!` macro adds the `Some(...)` for you, so write `icon: TablerIcon::Check`, never `icon: Some(TablerIcon::Check)`.

| Component | Icon props | Source |
|--------|-----------|--------|
| `ActionIcon` | `icon` | `action_icon.rs:122` |
| `Alert` | `icon` | `alert.rs:161` |
| `Notification` | `icon` | `notification.rs:112` |
| `AccordionControl` | `icon` | `accordion.rs:242` |
| `Blockquote` | `icon` | `blockquote.rs:25` |
| `List`, `ListItem` | `icon` | `list.rs:102`, `list.rs:176` |
| `Stepper` | `completed_icon`, `progress_icon` | `stepper.rs:104`, `:106` |
| `StepperStep` | `icon`, `completed_icon`, `progress_icon` | `stepper.rs:185`, `:187`, `:189` |
| `NavLink` | `left_section`, `right_section` | `navlink.rs:100`, `:102` |
| `DropdownMenuItem` | `left_section`, `right_section` | `dropdown_menu.rs:472`, `:474` |
| `Tab` | `left_section`, `right_section` | `tabs.rs:392`, `:394` |

The `Tree` component takes its icons through data rather than a prop: `TreeNodeData::icon` (`tree.rs:56`), set with the `with_icon(TablerIcon)` builder.

Paths are relative to `crates/rinch-components/src/`. `ActionIcon`'s `icon` prop is a convenience that renders the icon for you as Outline, sized from the component's own `size` prop. It is **mutually exclusive with children** — `loading` wins, then `icon`, and children render only if neither is set — so pass a rendered icon as a child (not via `icon:`) when you need a filled or custom-sized glyph.

## Dependencies and Imports

The `rinch` crate re-exports everything through its prelude. You do NOT need separate dependencies on `rinch-components` or `rinch-theme`:

```toml
# Cargo.toml - this is all you need:
[dependencies]
rinch = { workspace = true, features = ["desktop", "components", "theme"] }
```

**Important:** The workspace dependency uses `default-features = false`, so `"desktop"` must be listed explicitly. Without it, `run()` and other desktop APIs won't be available.

```rust
// In your code - prelude includes all components:
use rinch::prelude::*;

// DO NOT add redundant imports:
// use rinch_components::*;  // Not needed! Already in prelude
```

## Application Entry Point

Use the `run` function to start a rinch application. **`run()` automatically loads theme and component CSS** when those features are enabled, so components work out of the box:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}

fn main() {
    // This works with components - theme CSS is auto-loaded
    run("My App", 800, 600, app);
}
```

To customize the theme (colors, radius, dark mode), use `run_with_theme()`:

```rust
fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };
    run_with_theme("My App", 800, 600, app, theme);
}
```

The `#[component]` macro auto-injects `__scope: &mut RenderScope` as the first parameter, which is required by the `rsx!` macro. Components return a `NodeHandle`. You can also write `fn app(__scope: &mut RenderScope) -> NodeHandle` manually if preferred.

## Component Macro

Use the `#[component]` attribute macro to define component functions without manually writing the `__scope` parameter:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
        }
    }
}

fn main() {
    run("My App", 800, 600, app);
}
```

The macro auto-injects `__scope: &mut RenderScope` as the first parameter. `__scope` is still available inside the function body for use with `rsx!` and direct DOM operations.

Functions with additional parameters get `__scope` prepended:

```rust
#[component]
fn card(title: &str) -> NodeHandle {
    rsx! { div { {title} } }
}
// Expands to: fn card(__scope: &mut RenderScope, title: &str) -> NodeHandle
```

Both patterns are supported -- `#[component]` is preferred for new code, and the manual `__scope` parameter continues to work.

### PascalCase Components (Component Generation)

When a `#[component]` function uses a PascalCase name, the macro generates a struct and `Component` trait implementation:

```rust
#[component]
pub fn MyComponent(
    label: String,
    color: String,
    disabled: bool,
    onclick: Option<Callback>,
    children: &[NodeHandle],
) -> NodeHandle {
    // Parameters are available as local variables
    rsx! {
        div {
            class: "my-component",
            style: {format!("color: {}", color)},
            {label.clone()}
            // children is automatically appended by Component trait impl
        }
    }
}
```

**Key points:**
- **PascalCase name** triggers struct generation
- **Parameters become public struct fields** (must be owned types: `String`, `bool`, `Option<T>`, etc.)
- **`children: &[NodeHandle]` is special** — not a struct field, wired to `Component::render` method
- **Reference types rejected** (`&str`, `&T`) with a helpful error message
- **A manual `Default` impl is generated with per-field defaults for known types (String, bool, Option, Vec, numeric, Callback, InputCallback). Unknown types fall back to `Default::default()`.**
- **Usage:** `MyComponent { label: "Hello", color: "blue", onclick: || {}, "child content" }`

This pattern eliminates boilerplate for creating custom components — just write a PascalCase component function with owned parameters.

## Component Trait

Components implement the `Component` trait to render directly to DOM nodes:

```rust
pub trait Component: std::fmt::Debug + 'static {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle;
}
```

Example custom component:

```rust
#[derive(Debug, Default)]
pub struct MyButton {
    pub label: Option<String>,
    pub onclick: Option<Callback>,
}

impl Component for MyButton {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let btn = scope.create_element("button");
        btn.set_attribute("class", "my-button");

        if let Some(cb) = &self.onclick {
            let handler_id = scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        for child in children {
            btn.append_child(child);
        }
        btn
    }
}
```

## Component Props

For a complete reference of every component's props (fields, types, defaults), see [`docs/src/guide/component-props.md`](docs/src/guide/component-props.md).

**Key points:**
- CSS shorthand props (`w`, `h`, `m`, `p`, `maw`, `px`, `my`, etc.) work on all HTML elements and components. They expand to `set_style()` calls. Spacing scale values (`xs`, `sm`, `md`, `lg`, `xl`) auto-resolve to `var(--rinch-spacing-{value})`. Example: `div { p: "md", maw: "600px" }`. See `docs/src/guide/rsx-syntax.md#style-shorthands` for the full list.
- `Stack` and `Group` both have `align` and `justify` props for flex alignment — no need for `style:` for those.
- **Component text props are `String` (not `Option<String>`)** — empty string means "not set". The `rsx!` macro auto-converts string literals to `String::from(...)`.
- All component props accept reactive closures `{|| expr}` for automatic re-rendering when signals change.
- `_fn` suffix props (e.g., `value_fn`, `checked_fn`, `opened_fn`) provide surgical DOM updates without full component re-render.
- The `rsx!` macro auto-wraps prop values — do NOT manually wrap in `Some(...)`, `Rc::new(...)`, or `Callback::new(...)`.

## Reactive Patterns in RSX

Use closure syntax `{|| expr}` for reactive expressions that update automatically:

```rust
let count = Signal::new(0);

rsx! {
    // Static - captured once at render time
    p { {count.get().to_string()} }

    // Reactive - creates Effect, updates when signal changes
    p { {|| count.get().to_string()} }

    // Reactive attribute
    div { class: {|| if count.get() > 5 { "high" } else { "low" }}, "Value" }
}
```

**Block-expression closures:** The macro now supports block expressions that evaluate to closures, allowing setup code before the closure:

```rust
// ✅ WORKS: closure is the direct expression
div { style: {|| format!("width: {}px", count.get() * 10)} }

// ✅ ALSO WORKS: block with setup + final closure
div { style: { let m = 10; move || format!("width: {}px", count.get() * m) } }

// ✅ BOTH PATTERNS: compute outside or inside the block
let m = 10;
rsx! { div { style: {move || format!("width: {}px", count.get() * m)} } }
```

**Note:** Both `Signal` and `Memo` implement `Copy` — no `.clone()` needed before closures:

```rust
let count = Signal::new(0);
let doubled = Memo::new(move || count.get() * 2);

rsx! {
    // Both count (Signal) and doubled (Memo) can be used in multiple closures without .clone()
    p { {|| count.get().to_string()} }
    p { {|| doubled.get().to_string()} }
    button { onclick: move || count.update(|n| *n += 1), "+" }
}
```

## State Management

Component functions run **once** to build the DOM. Reactive closures (`{|| expr}`) in rsx handle all subsequent DOM updates surgically.

### Core Primitives

| Primitive | Purpose |
|-----------|---------|
| `Signal::new(value)` | Reactive state that triggers updates |
| `Memo::new(closure)` | Cached computed values |
| `create_store(value)` | Share a store across components (recommended for shared state) |
| `use_store::<T>()` | Access a shared store (panics if missing) |
| `try_use_store::<T>()` | Try to access a shared store (returns Option<T>) |
| `create_context(value)` | Low-level shared state (used by framework internals) |
| `use_context::<T>()` | Access shared context (panics if missing) |

> **Note:** `Effect` is intentionally excluded from the prelude. Use `{|| expr}` in rsx for reactive DOM updates, and store methods for side effects. For rare advanced cases (syncing to external systems), import explicitly: `use rinch::reactive::Effect;`

### Basic Example

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Closure syntax {|| ...} creates reactive DOM updates
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    run("My App", 800, 600, app);
}
```

> **Important:** The closure syntax `{|| expr}` is required for fine-grained reactive updates. Without it, values are captured once at initial render and never update. See [RSX Syntax - Reactive Expressions](docs/src/guide/rsx-syntax.md#reactive-expressions). The `#[component]` macro injects `__scope` automatically, so the `rsx!` macro works without manually declaring it.

### Store Pattern (Recommended for Shared State)

A store is a struct with `Signal` fields and methods that encapsulate state mutations. Use `create_store()` / `use_store()` to share it across components:

```rust
#[derive(Clone, Copy)]
struct CounterStore {
    count: Signal<i32>,
}

impl CounterStore {
    fn new() -> Self {
        Self { count: Signal::new(0) }
    }
    fn increment(&self) {
        self.count.update(|n| *n += 1);
    }
}

#[component]
fn app() -> NodeHandle {
    create_store(CounterStore::new());
    rsx! { Counter {} }
}

#[component]
fn counter() -> NodeHandle {
    let store = use_store::<CounterStore>();
    rsx! {
        p { {|| store.count.get().to_string()} }
        button { onclick: move || store.increment(), "+" }
    }
}
```

For component-local state, just use `Signal::new()` directly — no store needed.

### Primitive Reference

**`Signal::new()`** - Reactive state:
```rust
let count = Signal::new(0);
count.get();              // Read value
count.set(5);             // Set new value
count.update(|n| *n += 1); // Update with function

// Cross-thread: send() auto-dispatches to the main thread
std::thread::spawn(move || {
    count.send(10);                       // T: Send required
    count.update_send(|n| *n += 1);       // closure must be Send + 'static
});
```

**`Memo::new()`** - Cached computed values:
```rust
let count = Signal::new(0);
let multiplier = Signal::new(2);

// Automatically tracks count and multiplier
let result = Memo::new(move || count.get() * multiplier.get());
```

**`create_store` / `use_store`** - Shared state across components:
```rust
#[derive(Clone, Copy)]
struct AppStore {
    dark_mode: Signal<bool>,
    user_name: Signal<String>,
}

#[component]
fn app() -> NodeHandle {
    create_store(AppStore {
        dark_mode: Signal::new(false),
        user_name: Signal::new("Guest".into()),
    });
    // ...
}

#[component]
fn child_component() -> NodeHandle {
    let store = use_store::<AppStore>();
    // For optional access: let store = try_use_store::<AppStore>(); // returns Option<T>
    // ...
}
```

**`create_context` / `use_context`** - Low-level shared state (still works, appropriate for framework internals):
```rust
#[derive(Clone)]
struct Theme { color: String }

#[component]
fn app() -> NodeHandle {
    create_context(Theme { color: "#007bff".into() });
    // ...
}

#[component]
fn child_component() -> NodeHandle {
    let theme = use_context::<Theme>();
    // ...
}
```

## Threading Model

Rinch **owns the main thread**. `run()` calls winit's event loop, which takes over and never returns. All UI state (`Signal`, `Effect`, `NodeHandle`, `RenderScope`) is `!Send` — the reactive system is thread-local.

**Async / tokio:** A tokio runtime can coexist but only on a **separate background thread**. You cannot run `#[tokio::main]` and `rinch::run()` on the same thread.

```rust
fn main() {
    // Spawn tokio on a background thread
    let rt = tokio::runtime::Runtime::new().unwrap();
    std::thread::spawn(move || {
        rt.block_on(async {
            // networking, file I/O, etc.
        });
    });

    // Main thread — rinch owns it
    run("My App", 800, 600, app);
}
```

**Sending results back to the UI thread:** Use `Signal::send()` and `Signal::update_send()`, which auto-dispatch to the main thread from any thread. The `T` must be `Send`.

```rust
let data = Signal::new(Vec::new());

std::thread::spawn(move || {
    let result = fetch_data();     // blocking work
    data.send(result);             // dispatches to main thread
    data.update_send(|v| v.sort());// closure runs on main thread
});
```

**Key constraint:** Never call `Signal::set()` or `Signal::update()` from a background thread — they panic. Always use the `_send` variants for cross-thread updates.

## Native Menus

Native menus use a unified `Menu`/`MenuItem` builder API shared between window menu bars and tray context menus. Use `run_with_menu` to add a menu bar:

```rust
use rinch::prelude::*;
use rinch::menu::{Menu, MenuItem};

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { "Application content" }
    }
}

fn main() {
    let file_menu = Menu::new()
        .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(|| println!("New!")))
        .separator()
        .item(MenuItem::new("Exit").on_click(|| std::process::exit(0)));

    let edit_menu = Menu::new()
        .item(MenuItem::new("Undo").shortcut("Ctrl+Z"));

    run_with_menu("My App", 800, 600, app, vec![
        ("File", file_menu),
        ("Edit", edit_menu),
    ]);
}
```

Menu callbacks are `impl Fn() + 'static` — no `Send`/`Sync` required. They always run on the main thread, so Signal is safe to capture (Signal is Copy, no clone needed):

```rust
let count = Signal::new(0);

MenuItem::new("Reset Counter").on_click(move || count.set(0))
```

## Rich-Text Editor

Rinch's rich-text editor is a ProseMirror-style, **model-first** editor. The document lives in `rinch-editor-core` (a pure, wasm-clean crate: `Node`/`Mark`/`Fragment`/`Slice`, one char-based `Pos` space, a real `ContentMatch` schema, invertible `Step`s, `Transaction`/`EditorState`, plugins/commands/keymap/input-rules, a single Step-based history). The view lives in `rinch-editor-view` — **renderer-agnostic**: it projects that model onto any `rinch-core` `DomDocument` host and renders the caret/selection from `Selection` after layout, so desktop (rinch-dom) and web (`web_sys`) share one view. On desktop it arrives with the `desktop` feature and `crates/rinch/src/editor/` re-exports it; on web, `rinch-web` re-exports it. There is no `contenteditable` attribute engine anymore — mount the `Editor {}` component instead.

**Mutation flows one way:** every edit is a `Transaction` applied by `EditorState::apply` → the view diffs old/new doc + decorations and patches the DOM. Commands read **state**, never the DOM.

**Persisting content:** `DocNode` (serde) is the durable wire shape — `Node::to_doc()` / `Schema::node_from_doc()`, plus total HTML/markdown serializers in `rinch-editor-core::serialize`. Enable `serde` on the `rinch` facade (→ `rinch-editor-core/serde`).

**Full guides:** `docs/src/guide/contenteditable.md` (using the editor) and `docs/src/guide/editor.md` (model/schema/steps/plugins/view). Design: `docs/design/editor-rearchitecture.md`.

### Mounting an editor (the public API)

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let editor = create_editor();           // -> EditorHandle (cheap to clone)
    let ed_bold = editor.clone();           // one clone per closure
    rsx! {
        div {
            button { onclick: move || { ed_bold.command("toggleBold"); }, "Bold" }
            Editor {
                editor: editor.clone(),     // optional; omit to self-create a handle
                content: "<h1>Title</h1><p>Hello <strong>world</strong></p>",
            }
        }
    }
}
```

### `EditorHandle` (the app/component API)

| Category | Methods |
|----------|---------|
| **Dispatch** | `command(name) -> bool`, `update(\|state\| -> Option<Transaction>)`, `insert_text(&str)`, `replace_selection_with_html/text(&str)`, `insert_image(src, alt)`, `toggle_link(href)` |
| **Query (read state)** | `can_run(name)`, `is_mark_active(mark)`, `active_link_href() -> Option<String>`, `current_block_type()`, `in_node_type(type)`, `doc() -> Node`, `state()`, `selection()` |
| **Content / selection** | `load_html(&str)`, `load_doc(Node)`, `set_selection(Selection)`, `selection_clipboard()`, `set_dark_mode(bool)` |
| **Notification** | `on_change(impl Fn() + 'static)` — the autosave / dirty-marking hook |

`on_change` fires only for **local, document-changing** edits: `update` (which typing, paste and IME commit all funnel through), `command`, and `insert_image`. It deliberately does **not** fire for selection-only changes, for `load_doc`/`load_html` (a programmatic load isn't a user edit — firing would make an autosave consumer immediately re-save what it just loaded), or for `collab_receive` (already in the shared CRDT). The callback runs with no internal borrow held, so it may re-enter the handle freely — e.g. call `doc()` to serialize for the save.

Command names (dispatch by string): `toggleBold/Italic/Underline/Strike/Code/Highlight/Subscript/Superscript`, `setParagraph`, `setHeading1..6`, `setCodeBlock`, `setTextAlign{Left,Center,Right,Justify}`, `toggleBulletList`, `toggleOrderedList`, `toggleTaskList`, `wrapInBlockquote`, `sinkListItem`/`liftListItem` (indent/outdent), `insertHorizontalRule`, `insertHardBreak`, `insertTable`, `addRow{After,Before}`, `addColumn{After,Before}`, `deleteRow`/`deleteColumn`/`deleteTable`, `mergeCells`/`splitCell`, `removeLink`, `undo`/`redo`. (Adding a link needs an `href` arg, so it is **not** a string command — use `handle.toggle_link(href)`.)

**Markdown shortcuts (input rules).** As you type, the default `MarkdownInputRulesPlugin` rewrites markdown shortcuts (these run inside `EditorHandle::insert_text`, so every text-entry path gets them). Block shortcuts at the line start: `# `…`###### ` → headings, ` ``` ` → code block, `> ` → blockquote, `- `/`* `/`+ ` → bullet list, `1. ` → ordered list, `[ ] `/`[x] ` → task list. Inline mark shortcuts (fire on the closing delimiter): `**bold**`/`__bold__`, `*italic*`/`_italic_`, `~~strike~~`, `==highlight==`, `` `code` ``. The rule set lives in `rinch-editor-core/src/input_rules.rs` (`markdown_input_rules()`); add a `mark_input_rule`/`wrapping_input_rule` there to extend it. Task items render a checkbox from their `checked` attr via the default stylesheet (`[data-pm-type="task_item"]`); `Enter` makes a fresh unchecked item, `Enter` on an empty item exits the list.

The editor ships its own default light/dark stylesheet (`rinch-editor-view/src/styles.rs`, injected once by the view); toggle dark mode with `handle.set_dark_mode(true)` (sets `data-pm-theme="dark"` on the container). Don't hand-roll editor CSS.

### Key Source Files

| File | Purpose |
|------|---------|
| `crates/rinch-editor-core/src/` | Pure model: `model/*`, `pos/*`, `schema/*`, `transform/*` (Steps), `state/*`, `commands/*`, `plugins/*`, `serialize/*`, `tables.rs`, `a11y.rs`, and `view.rs` (the `EditorView` seam) |
| `crates/rinch-editor-view/src/lib.rs` | `create_editor`, `mount_editor`, `caret_blink_tick` |
| `crates/rinch-editor-view/src/handle.rs` | `EditorHandle` — the imperative app/component API |
| `crates/rinch-editor-view/src/view.rs` | `RinchDomEditorView` (the `EditorView` impl: `ViewDesc` diff, caret/selection/decoration overlays) |
| `crates/rinch-editor-view/src/component.rs` | The `Editor {}` rsx component |
| `crates/rinch-editor-view/src/registry.rs` | The mounted-editor registry (keyed by `doc_key` + container id) and `update_all_carets`, the post-layout overlay pass |
| `crates/rinch-editor-view/src/styles.rs` | Default light/dark stylesheet |
| `crates/rinch/src/editor/` | **Desktop-only** wiring; re-exports all of `rinch-editor-view`. Block virtualization (`virtualization.rs` + `virtual_window.rs`), AccessKit (`a11y.rs`), and the `Send`-safe `post_remote_delta` |
| `crates/rinch/src/app/event_dispatch.rs`, `app/focus.rs` | Desktop input glue: key/pointer/IME → `EditorHandle` calls, and the focus arbiter |
| `crates/rinch-web/src/editor_input.rs` | The same glue for the browser (the web editor shares the view crate) |
| `crates/rinch-editable/src/` | The separate single-line `<input>`/`<textarea>` engine (`EditCommand`, `InputHandler`) — unrelated to the rich editor |

### Collaboration (optional, opt-in — M9)

Real-time collaborative editing is a feature-gated adapter, **not** part of the model. The pure `rinch-editor-core` model stays renderer- and CRDT-agnostic; `rinch-editor-collab` projects it onto a **yrs 0.27** (Yjs) CRDT so concurrent edits converge, then translates remote CRDT changes back into editor `Step`s. This crate is the **only** thing in the workspace that links a CRDT engine (yrs replaced Automerge in issue #190).

It is gated behind the optional `collaboration` feature (which implies `desktop`, since the editor wiring lives there), so **default builds — desktop AND web — link zero CRDT code**. The adapter itself is pure model↔CRDT logic with no platform deps and is **wasm-compatible with no shims** — yrs carries its own `fastrand/js` randomness source and builds for `wasm32-unknown-unknown` with no extra features — so a future Rust web editor view can reuse this *same* adapter rather than bridging to a separate JS CRDT.

Enable with `features = ["collaboration"]` on the `rinch` facade (or depend on `rinch-editor-collab` directly).

The design rests on one invariant — **`model ≡ project(model)`**: every local step is projected onto the CRDT, every remote CRDT change is rebuilt into the model. Convergence then follows from yrs's own convergence.

**Staged scope (design A22):** the first milestone covers **flat text-blocks + marks** (`paragraph`/`heading`/`code_block` with text + bold/italic/link/… marks) plus the **list containers** `bullet_list`/`ordered_list`/`list_item`, nested into each other and around text-blocks to any depth. Anything else — `blockquote`, tables, `task_list`/`task_item`, or an inline atom (`image`, `hard_break`, `horizontal_rule`) — is `CollabError::Unsupported`: the adapter **fails loud** rather than silently dropping a change (a silent drop would reintroduce the exact divergence class the editor rewrite killed).

**Collab bytes are opaque.** A snapshot, a broadcast delta, a state vector, and a sync diff are all just `Vec<u8>` in yrs's lib0 v1 encoding — callers move them between calls without decoding them.

```rust
use rinch_editor_collab::CollabSession;

// One session per editor. Peer B joins from peer A's snapshot.
let mut a = CollabSession::new(&state)?;            // project state.doc onto a fresh CRDT
let mut b = CollabSession::from_bytes(&a.snapshot())?;

// After the editor applies a local transaction, project before→after onto the CRDT:
a.record_local(&old_state.doc, &new_state.doc)?;

// Broadcast a delta:
let delta = a.save_incremental()?;
if let Some(next) = b.integrate_incremental(&b_state, &delta)? { b_state = next; }
// `next` applies the remote change as a non-undoable `origin=remote` transaction.

// Reconciliation for a peer that fell behind (offline, a dropped delta):
let to_b = a.sync_diff(&b.state_vector())?;   // "what does b not have yet"
if let Some(next) = b.integrate_incremental(&b_state, &to_b)? { b_state = next; }
```

**The desktop editor wires this in for you** (M9) — you do not drive `CollabSession` directly. Every local edit through an `EditorHandle` projects + broadcasts automatically; a peer's delta integrates + re-projects through `collab_receive`. One peer **hosts** (owns the starting document), the others **join** from its snapshot. The transport is the app's concern — `outbound` carries bytes out, `post_remote_delta` carries them back in from any thread:

```rust
// Host: project the current doc onto a fresh CRDT, hand peers a join snapshot.
let snapshot = host.start_collaboration_host(move |delta| transport.send(delta))?;
// Guest: adopt the host's document and collaborate.
guest.start_collaboration_guest(&snapshot, move |delta| transport.send(delta))?;
// Inbound from a network thread (marshals onto the main thread); from the prelude:
post_remote_delta(container_id, delta_bytes);
```

| `EditorHandle` collab method | Purpose |
|---|---|
| `start_collaboration_host(outbound) -> Result<Vec<u8>, CollabError>` | Host a fresh session; returns the join snapshot |
| `start_collaboration_guest(&snapshot, outbound) -> Result<(), CollabError>` | Join from a host snapshot (adopts its document) |
| `collab_receive(&delta) -> bool` | Integrate a peer's delta or reconciliation diff (main thread, `try_borrow_mut`-soft); re-projects, does **not** re-broadcast |
| `collab_state_vector() -> Option<Vec<u8>>` | This editor's state vector, to hand a peer for `collab_sync_diff` |
| `collab_sync_diff(remote_state_vector) -> Option<Vec<u8>>` | The update a peer at `remote_state_vector` is missing; feed the result to that peer's `collab_receive` |
| `is_collaborating()` / `stop_collaboration()` | Query / detach the session |
| `collab_snapshot() -> Option<Vec<u8>>` | Current shared-doc snapshot for a *late*-joining guest |
| `collab_take_error() -> Option<CollabError>` | Take a fail-loud A22 projection error (the CRDT is left untouched — projection is all-or-nothing) |

Free functions `collab_receive_for(container_id, &delta)` (main thread) and `post_remote_delta(container_id, delta)` (any thread) route an inbound delta to a registered editor. Runnable two-pane in-process loopback: `examples/collab-editor-demo`.

**The transport owns relaying.** `outbound` fires only for an editor's own local edits — a delta integrated via `collab_receive` is never re-broadcast (it's already in the shared CRDT; echoing it back would loop). So the transport must be a **full mesh** (every peer's `outbound` reaches every other peer) or a **hub** that fans each delta it receives out to the others, forwarding the raw bytes unchanged. A chain — A wired to B, B wired to C, nothing joining A and C — silently partitions: C never sees A's edits, and nothing errors. The repair for a peer that fell behind is the reconciliation pair above (`collab_state_vector` → `collab_sync_diff` → `collab_receive`).

**Never use state-vector equality as a convergence test.** A yrs state vector counts *insertions* only — a deletion, or a mark *removal* (yrs un-formats a mark by deleting its format marker), leaves it unchanged, so two replicas can hold different documents behind equal state vectors. That's why `integrate_incremental`/`collab_receive` decide "did anything change" by rebuilding and comparing documents, never by comparing state vectors before/after — and why reconciliation always requests-and-applies a diff (whose reply carries the full delete set) rather than short-circuiting when two state vectors look equal.

`CollabPlugin` (key `"collab"`) folds collab bookkeeping (version + unconfirmed local steps) into `EditorState`; `rebase_steps(steps, &mapping)` is the ProseMirror rebase primitive (`Step::map`). The session integrates by converged rebuild (`CollabDoc::to_doc` → `build_remote_transaction`), which is provably convergent — there's no separate patch→op translation layer. One used to exist (`CollabDoc::patches_to_remote_ops`/`remote_ops_since`, consuming `automerge::Patch`) but it was **deleted**, not ported, with the yrs migration: it was documented as non-convergence-critical and had no session consumer. `remote.rs`'s module doc notes it could be rebuilt on yrs observer deltas (`TextRef::observe` → `TextEvent`) if a future cursor-preserving refinement ever wants it.

| File | Purpose |
|------|---------|
| `crates/rinch-editor-collab/src/projection.rs` | `CollabDoc` — the yrs wire shape (`content: Array<Map{type,attrs,text:Text}>`, marks as native yrs `Text` formatting attributes), `from_doc`/`to_doc`/`load`, the `observe_update_v1` broadcast outbox, fail-loud validation |
| `crates/rinch-editor-collab/src/project.rs` | Local: `project_change` — block-list diff (Rc-identity prefix/suffix, minimal text splice) |
| `crates/rinch-editor-collab/src/remote.rs` | Remote: `build_remote_transaction` — converged rebuild into a minimal block-level `ReplaceStep`; no engine type appears in this file |
| `crates/rinch-editor-collab/src/session.rs` | `CollabSession` — the imperative model↔CRDT lifecycle (`new`/`from_bytes`, `record_local`, `save_incremental`/`state_vector`/`sync_diff`, `integrate_incremental`) |
| `crates/rinch-editor-collab/src/sync.rs` | The yrs bytes transport on `CollabDoc` (`state_vector`, `diff_since`, `apply_update`, the outbox drain) |
| `crates/rinch-editor-collab/src/plugin.rs` | `CollabPlugin` + `CollabState` |
| `crates/rinch-editor-collab/src/rebase.rs` | `rebase_steps` — local steps rebased over a remote mapping |
| `crates/rinch-editor-collab/src/error.rs` | `CollabError` — the crate's one error type (`Engine`/`Unsupported`/`Schema`) |
| `crates/rinch-editor-view/src/handle.rs` | `EditorHandle`'s collab methods (`start_collaboration_host/guest`, `collab_receive`, `collab_state_vector`, `collab_sync_diff`, `collab_snapshot`, `collab_take_error`, `stop_collaboration`, `is_collaborating`) — **not** in `rinch-editor-collab` |
| `crates/rinch-editor-view/src/collab.rs` | `CollabBridge` — the seam driving `CollabSession` from an `EditorHandle` (outbound sink + last error) |
| `crates/rinch-editor-view/src/registry.rs` | `collab_receive_for(container_id, &delta)` — routes an inbound delta to a registered editor |

## Drag and Drop

Rinch has two drag systems: **DOM drag attributes** for element-to-element DnD, and the **`Drag` builder** for pointer capture (sliders, panel dragging, resize handles).

> **Picking the right one:** if you want **continuous per-frame tracking** (slider value, panel position, timeline scrub), use **`Drag::absolute()` / `Drag::percent()`** from inside an `onclick` handler — that's the pointer-capture system. The HTML5-style `draggable: true` + `ondragstart` + `ondragend` attributes only fire at the **endpoints** of the drag; for per-frame events on that path use `data-ondragmove` (source) and `data-ondragover` (target).

### DOM Drag Attributes

Set these attributes on elements to participate in element-to-element drag-and-drop:

| Attribute | Fires on | When |
|-----------|----------|------|
| `data-ondragstart` | Source | Drag begins |
| `data-ondragmove` | Source | Pointer moves during drag |
| `data-ondragenter` | Target | Drag enters a drop target |
| `data-ondragover` | Target | Pointer moves over drop target (every motion event) |
| `data-ondragleave` | Target | Drag leaves a drop target |
| `data-ondrop` | Target | Drop on target |
| `data-ondragend` | Source | Drag finishes |

**Input & activation (mouse, touch, pen).** This suite is driven by Pointer Events on both backends, so it works with a mouse, a finger, or a pen. Activation differs by input so touch doesn't hijack scrolling:
- **Mouse:** activates as soon as the pointer moves past ~5px — snappy, unchanged.
- **Touch / pen:** activates on a short **long-press** hold (~350ms) while the contact stays roughly stationary. Moving before the hold completes is treated as a scroll/pan and the drag is abandoned (the page scrolls normally). This is the standard mobile reorder gesture.

Because there's no built-in drag ghost (the app renders its own from `data-ondragmove`), **set `pointer-events: none` on your ghost element** — on touch the drop target is resolved via `elementFromPoint`, so a ghost under the finger would otherwise intercept the hit and drops would silently fail.

Handlers can read `get_click_context()` for cursor position and element bounds. Use `DragContext<T>` to pass typed data between source and target:

```rust
let drag = DragContext::<MyItem>::new();

// In source's ondragstart:
drag.set(item.clone());

// In target's ondrop:
if let Some(item) = drag.take() {
    target_list.update(|list| list.push(item));
}
```

### Pointer Capture Drag (Drag Builder)

For tracking mouse movement from a click handler until mouseup (sliders, panels, resize):

```rust
// Absolute pixel coordinates (panel dragging)
let ctx = get_click_context();
let offset_x = ctx.mouse_x - panel_x.get();
Drag::absolute()
    .on_move(move |x, y| panel_x.set(x - offset_x))
    .on_end(move |x, y| save_position(x, y))
    .on_cancel(move |_x, _y| restore_position())  // teardown; on_end does NOT fire
    .start();

// Percentage 0.0–1.0 (sliders) — reads element bounds from ClickContext automatically
Drag::percent()
    .on_move(move |px, _| slider_value.set(px * 100.0))
    .start();

// Cancel: fires on_cancel (with the last on_move coords) but NOT on_end —
// a cancelled drag must not commit. The web backend calls this on pointercancel
// (touch-scroll takeover, system gesture, pointer-capture loss).
Drag::cancel();

// Check if active
Drag::is_active();
```

### File Drop (OS → App)

File drops from the OS use `data-onfiledragenter`, `data-onfiledragleave` attributes, and `register_file_drop_handler` for the actual drop. See the File Drop section of UI Zoo for an example.

## Keyboard Shortcuts (built-in)

- `Ctrl/Cmd + +/-/0` - Zoom in/out/reset
- `Alt + D` - Toggle layout debug overlay
- `Alt + I` - Toggle inspect mode (hover highlight for element info)
- `Alt + P` - Toggle performance stats console logging
- `Alt + T` - Print Taffy layout tree (to console)
- `F12` - Toggle DevTools window

## Features

### Rendering Backends

Rinch supports two rendering backends, selected at compile time:

| Backend | Feature | Renderer | Presentation |
|---------|---------|----------|--------------|
| **GPU** | `"gpu"` | Vello + wgpu | GPU compositing |
| **Software** | (default) | tiny-skia | softbuffer |

Set in `Cargo.toml`:
```toml
# GPU mode:
rinch = { workspace = true, features = ["desktop", "gpu"] }

# Software mode (default):
rinch = { workspace = true, features = ["desktop"] }
```

Both use the same `Painter` trait (`crates/rinch-dom/src/paint/painter.rs`):
- `VelloPainter` — records commands into `vello::Scene`
- `TinySkiaPainter` — rasterizes directly to RGBA pixmap

The software renderer includes **dirty region caching**: when only a few nodes change, only the affected rectangular area is cleared and repainted. Subtrees outside the dirty region are skipped during paint traversal.

**Key files:**
- `crates/rinch-dom/src/paint/painter.rs` — Abstract `Painter` trait
- `crates/rinch-dom/src/paint/vello_painter.rs` — GPU backend
- `crates/rinch-dom/src/paint/skia_painter.rs` — Software backend
- `crates/rinch-dom/src/paint/mod.rs` — `paint_document()`, dirty region computation, subtree pruning
- `crates/rinch/src/app/mod.rs` — `build_scene()` (GPU) / `build_pixels()` (software)

### Image Support

Images render on **both** desktop backends — GPU (Vello, `scene.draw_image`) and software (tiny-skia, `draw_pixmap`) — via `<img>` elements and `background-image: url(...)` CSS. Remote/file images load asynchronously on background threads; `data:` URIs (e.g. base64 PNG) are decoded synchronously and inserted straight into the cache (`request_image_load_for_node`).

**Architecture:**
```
rinch-core:  ImageLoader trait + ImageLoadResult enum (no deps)
rinch-dom:   ImageCache + FileImageLoader + decode pipeline (image crate)
rinch:       NetworkImageLoader (ureq, gated behind image-network feature)
```

**Key files:**
- `crates/rinch-core/src/image.rs` — `ImageLoader` trait, `ImageLoadResult` enum
- `crates/rinch-dom/src/image_cache.rs` — `ImageCache`, `DecodedImage`, `FileImageLoader`, async load
- `crates/rinch-dom/src/paint/image.rs` — `paint_image()` with object-fit support (fill/contain/cover)
- `crates/rinch/src/image_loader.rs` — `NetworkImageLoader` (feature-gated)

**How it works:**
1. `set_attribute("src", ...)` on `<img>` or `BackgroundValue::Image` during style resolution triggers a load
2. `ImageCache` checks if already cached; if not, spawns a background thread via `request_image_load()`
3. The background thread calls `ImageLoader::load()` then decodes with the `image` crate to RGBA8
4. Results go to a static `Mutex<Vec<PendingImage>>` queue
5. `drain_pending_images()` at the start of layout picks up decoded images, updates Taffy intrinsic dims
6. `paint_image()` renders via `scene.draw_image()` with proper affine transforms

**Network loading:** Enable `features = ["image-network"]` for HTTP(S) URL support via `ureq`.

**Overflow clipping:** The overflow clip layer uses `RoundedRect` when `border-radius > 0`, enabling circular avatar clipping.

### DevTools Panel

Press F12 to toggle the DevTools panel which shows:
- **Performance**: FPS, frame time, and render time
- **Elements**: DOM tree inspection
- **Styles**: Computed styles for selected elements (enable inspect mode with Alt+I)
- **Styles**: Computed styles for selected elements (enable inspect mode with Alt+I)

### Debug & MCP Server (optional)

Enable with `features = ["debug"]` to expose your app's DOM, screenshots, and input injection to external tools via TCP IPC.

**Architecture:**

```
Claude (stdio) → rinch-mcp-server → TCP:port → rinch-debug (in app) → rinch runtime
```

Two crates work together:

| Crate | Type | Purpose |
|-------|------|---------|
| `rinch-debug` | Library | TCP IPC server embedded in the app, auto-starts on a random localhost port |
| `rinch-mcp-server` | Binary | Standalone MCP server that Claude talks to, discovers and connects to running apps |

**Enabling in your app:**

```toml
# Cargo.toml
[dependencies]
rinch = { workspace = true, features = ["debug"] }
```

The debug server starts automatically when the feature is enabled. Disable at runtime with `RINCH_DEBUG=0`. Force a specific port with `RINCH_DEBUG_PORT=9100`.

**MCP configuration** (`.mcp.json`):

For fastest startup, point to the pre-built binary:

```json
{
  "mcpServers": {
    "rinch": {
      "command": "/path/to/rinch/target/debug/rinch-mcp-server",
      "args": [],
      "cwd": "/path/to/rinch"
    }
  }
}
```

Build first with `cargo build -p rinch-mcp-server`. Using `cargo run` instead works but is slower to start.

**MCP tools available:**

| Tool | Description |
|------|-------------|
| `list_apps` | List all running rinch apps with debug enabled |
| `connect` | Connect to a specific app by name or PID |
| `screenshot` | Capture a PNG screenshot (returns as inline MCP image, directly viewable) |
| `dom_tree` | Get the full DOM tree as JSON with layout bounds and computed styles |
| `query_selector` | Query nodes by tag, `.class`, `[attr]`, or `[attr=value]` |
| `get_node` | Get detailed info for a specific node by ID (includes computed styles, display mode) |
| `get_computed_styles` | Get computed CSS styles for a specific DOM node |
| `get_text_content` | Get text content within a node subtree |
| `click` | Simulate a mouse click at (x, y) coordinates |
| `type_text` | Simulate keyboard text input |
| `wait_frame` | Wait for the next render frame |
| `close_app` | Close the connected app gracefully |
| `disconnect` | Disconnect from the app without closing it |
| `launch_app` | Launch a rinch app via `cargo run -p <package>`, wait for debug registration, auto-connect |

**Discovery mechanism:** Each debug-enabled app writes `~/.rinch/debug/{pid}.json` containing its port, app name, and PID. The MCP server scans this directory to find running apps and auto-connects when only one is running.

**IPC protocol:** Length-prefixed JSON over TCP on localhost. 4-byte big-endian length prefix followed by JSON payload. Handshake exchanges protocol version on connect.

**Key source files:**
- `crates/rinch-debug/src/server.rs` - TCP listener (blocking I/O, no tokio dependency)
- `crates/rinch-debug/src/protocol.rs` - Wire protocol types and framing
- `crates/rinch-mcp-server/src/mcp_server.rs` - MCP tool implementations
- `crates/rinch/src/shell/rinch_runtime.rs` - Runtime integration (`execute_debug_command()`)

### File Dialogs (optional)

Enable with `features = ["file-dialogs"]`:

```rust
use rinch::dialogs::{open_file, save_file, pick_folder, message};

// Open file
if let Some(path) = open_file().add_filter("Text", &["txt"]).pick_file() { }

// Save file
if let Some(path) = save_file().set_file_name("doc.txt").save() { }

// Pick folder
if let Some(path) = pick_folder().pick() { }

// Message dialog
message("Success!").set_title("Info").show();
```

### Clipboard (optional)

Enable with `features = ["clipboard"]`:

```rust
use rinch::clipboard::{copy_text, paste_text, has_text};

copy_text("Hello").unwrap();
if has_text() {
    let text = paste_text().unwrap();
}
```

### System Tray (optional)

Enable with `features = ["system-tray"]`. Uses the same unified `Menu`/`MenuItem` types as native menus:

```rust
use rinch::prelude::*;
use rinch::tray::TrayIconBuilder;
use rinch::menu::{Menu, MenuItem};

let menu = Menu::new()
    .item(MenuItem::new("Show").on_click(show_current_window))
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));

let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_png(include_bytes!("../assets/icon.png"))?
    .with_menu(menu)
    .build()?;
```

**Key features:**
- `with_icon_png(data)` — Load tray icon from PNG bytes (use `include_bytes!`)
- `with_icon_rgba(rgba, w, h)` — Load from raw RGBA pixel data
- `with_icon_path(path)` — Load from a PNG file path
- **Push-based events** — Menu callbacks fire via `MenuEvent::set_event_handler` on the main thread. No polling thread, no wasted CPU.
- **Left-click** — Shows the window automatically via `TrayIconEvent::set_event_handler`.

**Minimize-to-tray pattern:** Combine system tray with `on_close_requested` + `hide_current_window()`:

```rust
let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Don't exit, just hide
    })),
    ..Default::default()
};
```

### Theme System (optional)

Enable with `features = ["theme"]` for the theme system inspired by Mantine.

Theme is configured at the runtime level using `ThemeProviderProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { style: "color: var(--rinch-primary-color);",
            "Uses theme CSS variables"
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    run_with_theme("Themed App", 800, 600, app, theme);
}
```

ThemeProvider generates CSS variables:
- Colors: `--rinch-color-{name}-{0-9}`, `--rinch-primary-color`
- Spacing: `--rinch-spacing-{xs,sm,md,lg,xl}`
- Radius: `--rinch-radius-{xs,sm,md,lg,xl}`, `--rinch-radius-default`
- Shadows: `--rinch-shadow-{xs,sm,md,lg,xl}`
- Typography: `--rinch-font-size-{xs,sm,md,lg,xl}`, `--rinch-font-family`
- Semantic: `--rinch-color-body`, `--rinch-color-text`, `--rinch-color-dimmed`

**Window chrome inset (not ThemeProvider-generated).** `--rinch-window-top-inset`
is published at runtime by whatever chrome rinch draws above your content — the
Linux in-app menu bar (28px) and the `BorderlessWindow` titlebar (36px, +28 when
the menu bar sits below it). That chrome reserves space with in-document padding,
so normal flow content clears it automatically; a `position: fixed` element does
**not**, because fixed resolves against the real viewport (matching browsers and
rinch-web). Full-height overlays must therefore opt in:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0;" }
```

`Drawer`, `Modal`, and the top-anchored `Notification` positions already do this.
Backdrops that intentionally cover the whole window (`DropdownMenu`, `Select`)
deliberately do not. Do **not** "fix" this by insetting the fixed containing
block — that would break CSS semantics and make desktop diverge from rinch-web.

## Transparent Windows

Transparent/borderless windows are configured via `WindowProps { transparent: true, borderless: true }`. The live renderer (`shell::desktop::WgpuRenderer`) uses `CompositeAlphaMode::Auto` + a transparent clear color, which the compositor honors on **Linux (Wayland/X11)**.

> **Windows caveat:** true per-pixel transparency on Windows needs an alpha-capable presentation path (PreMultiplied alpha + DX12 DirectComposition + `WS_EX_NOREDIRECTIONBITMAP`) that is **not currently wired** into `WgpuRenderer` — so `transparent: true` renders opaque on Windows today. Tracked in **issue #89**. (The old `TransparentWindowRenderer` held that path but was never constructed by the runtime and has been removed as dead code.)

Configure via `WindowProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { class: "custom-titlebar",
            // ... your custom titlebar and content
        }
    }
}

fn main() {
    let window_props = WindowProps {
        title: "My App".into(),
        borderless: true,      // Remove native decorations
        transparent: true,     // Enable transparency
        resize_inset: Some(12.0),  // Enable resize handles (matches CSS margin)
        ..Default::default()
    };

    run_with_window_props(app, window_props, None);
}
```

### Resize Handles for Borderless Windows

Borderless windows don't have native resize handles. Use `resize_inset` to enable custom resize handling:

- `resize_inset: Some(f32)` - Enables resize handles within `inset + 8px` from edges
- `resize_inset: None` (default) - Disables custom resize handles
- Only active when `borderless: true` AND `resizable: true`
- The cursor automatically changes to indicate resize direction on hover
- Corner resize areas are larger (`inset + 16px`) for easier diagonal resizing
- On Windows, transparent areas don't receive mouse events, so resize detection relies on the 8px extension into visible content

The `resize_inset` value should match your CSS content margin/padding to align the resize handles with the visible window edge.

**What true Windows transparency would require (see issue #89 — not yet wired):**
- DX12 backend with DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=DxgiFromVisual`)
- `CompositeAlphaMode::PreMultiplied`
- `WS_EX_NOREDIRECTIONBITMAP` window style
- Patched wgpu for Rgba8Unorm storage textures (see wgpu fork below)

### BorderlessWindow Component

The `BorderlessWindow` component provides a complete container for borderless/transparent windows with:
- Rounded corners
- Custom titlebar with drag support
- Window control buttons (minimize, maximize, close)
- Optional left/right custom sections in the titlebar
- Proper theming via CSS variables

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    // For custom left section (e.g., menu button)
    let menu_signal = Signal::new(false);
    let left_section: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            ActionIcon { onclick: move || menu_signal.update(|v| *v = !*v) }
        }
    });

    rsx! {
        BorderlessWindow {
            title: "My App",
            radius: "md",  // none, xs, sm, md, lg, xl
            left_section: Some(left_section),
            on_minimize: || minimize_current_window(),
            on_maximize: || toggle_maximize_current_window(),
            on_close: || close_current_window(),

            // Content goes here
            div { "Hello, world!" }
        }
    }
}
```

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `title` | `Option<String>` | Window title displayed in titlebar |
| `radius` | `Option<String>` | Corner radius: none, xs, sm, md, lg, xl |
| `show_minimize` | `bool` | Show minimize button (default: true) |
| `show_maximize` | `bool` | Show maximize button (default: true) |
| `show_close` | `bool` | Show close button (default: true) |
| `left_section` | `Option<SectionRenderer>` | Custom content for left side of titlebar |
| `right_section` | `Option<SectionRenderer>` | Custom content before window controls |
| `on_minimize` | `Option<Callback>` | Callback for minimize button |
| `on_maximize` | `Option<Callback>` | Callback for maximize button |
| `on_close` | `Option<Callback>` | Callback for close button |

### Window Control Functions

For custom window chrome (minimize/maximize/close buttons):

```rust
use rinch::prelude::*;

// In event handlers:
button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }

// Window visibility (for minimize-to-tray):
button { onclick: || hide_current_window(), "Hide to Tray" }
// From a tray menu callback:
TrayMenuItem::new("Show").on_click(|| show_current_window())
```

These functions are available in the prelude and work from onclick handlers.

**`on_close_requested` callback:** Intercept the window close button to hide instead of exit:

```rust
use std::sync::Arc;

let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Return false to cancel exit, true to proceed
    })),
    ..Default::default()
};
```

### wgpu Fork

Transparent windows require a patched wgpu to enable Rgba8Unorm storage textures for Vello rendering on DX12. The patches are in `[patch.crates-io]` in `Cargo.toml`:

- **Repository**: https://github.com/joeleaver/wgpu-fork
- **Branch**: `rinch-patch`
- **Upstream PR**: https://github.com/gfx-rs/wgpu/pull/8908

The patches:
1. `instance.rs` - Force storage capabilities for Rgba8Unorm/Bgra8Unorm
2. `device/resource.rs` - Use hardware format features instead of WebGPU defaults
3. `present.rs` - Use adapter format features for surface textures

**Downstream projects** must copy the `[patch.crates-io]` section from the workspace `Cargo.toml` into their own `Cargo.toml` for transparent windows to work on Windows. This is required because Cargo patches are not transitive — they only apply to the workspace that declares them.

## Game Engine Integration

Two complementary patterns for integrating with game engines and custom renderers:

### RenderSurface (Recommended)

Rinch owns the window. Your renderer submits frames into a `RenderSurface` component. Rinch handles layout, compositing, and event routing.

**Key types** (all re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RenderSurfaceHandle` | Main handle — `writer()`, `gpu_registrar()`, `set_event_handler()` |
| `RenderSurface` | Component — `RenderSurface { surface: Some(handle) }` |
| `SurfaceWriter` | Thread-safe CPU pixel submission (`Send + Sync + Clone`) |
| `GpuTextureRegistrar` | Thread-safe GPU texture registration (`Send + Sync + Clone`) |
| `SurfaceEvent` | Input events dispatched to surface handler |
| `create_render_surface()` | Factory function |

**Usage:**
```rust
let surface = create_render_surface();
surface.set_event_handler(|event| { /* handle mouse/keyboard */ });

let writer = surface.writer();
std::thread::spawn(move || {
    writer.submit_frame(&pixels, w, h); // CPU pixels
});

// Or for GPU textures:
let registrar = surface.gpu_registrar();
registrar.set_texture_source(wgpu_texture, wgpu_view, w, h);
registrar.notify_frame_ready();

rsx! { RenderSurface { surface: Some(surface), style: "flex: 1;" } }
```

**Sharing a high-capability GPU device (issue #57):** zero-copy compositing needs your texture on the *same* device rinch composites with (`gpu_handle()` → `device`/`queue`/**`adapter`**). By default that device is created with `Features::default()` / `Limits::default()`. To raise it:

| Entry point | Ownership | Use when |
|---|---|---|
| `run_with_gpu_config(component, props, theme, RinchGpuConfig { required_features, required_limits })` | rinch creates the device (surface-compatible adapter) with your extra features/limits | You just need more capability — **recommended**, always presents correctly |
| `run_with_external_device(component, props, theme, ExternalGpu { instance, adapter, device, queue })` | You create the whole stack; rinch makes only the surface, validates present-support, composites onto your device | You must keep your exact `DeviceDescriptor` |

Construct `wgpu` types from **`rinch::wgpu`** (rinch pins a patched fork — a separate `wgpu` dep won't type-match). Both are `#[cfg(feature = "gpu")]`, re-exported in the prelude. Example: `examples/gpu-device-config` (`RINCH_GPU_MODE=external` toggles the two modes).

**Web (canvas viewport, issue #91):** the **same** `RenderSurface` + `create_render_surface()` API works on `rinch-web`, but the model is inverted — the **browser** composites, so rinch only creates and manages a `<canvas>` "viewport hole"; the **app owns the GPU context**. rinch links **no wgpu** on web.

- `handle.canvas_element() -> Option<web_sys::HtmlCanvasElement>` (wasm only) — returns the `<canvas>` after mount (populated in a post-mount microtask; `None` before). Call `wgpu::Instance::create_surface(SurfaceTarget::Canvas(canvas))` on it to render via WebGPU/WebGL. (The lazy 2D-blit CPU path — `SurfaceWriter::submit_frame` — still works and is portable with desktop; it self-disables once you claim a GPU context.)
- **HiDPI out of the box:** `layout_size()` reports **physical** px (`CSS px × devicePixelRatio`, matching desktop), rinch sizes the canvas backing store to match, and `set_resize_callback(|w, h| …)` pushes the new physical size on every resize (ResizeObserver-driven) so you can reconfigure your wgpu surface.
- **Input** (pointer/wheel/keyboard incl. **`KeyUp`**/focus) over the canvas is delivered to `set_event_handler` — rinch does not swallow it. `set_render_callback` drives a `requestAnimationFrame` loop.
- **Teardown** is clean: the ResizeObserver and canvas listeners are removed when the component unmounts (no leaks).
- Example: `examples/webgpu-surface-web` (rinch DOM chrome + a WebGPU triangle; `trunk serve` over localhost so `navigator.gpu` is available). Mirrors `examples/game-embed` on desktop.

**Source files:**
- `crates/rinch/src/render_surface.rs` — All RenderSurface types and registry (incl. the wasm32 canvas path: `canvas_element`, `set_resize_callback`, `setup_canvas_events`, `setup_resize_observer`, `WebSurfaceCleanup`)
- `crates/rinch-web/src/event_delegation.rs` — document-level keyboard/focus routing into the surface (`KeyDown`/`KeyUp`/`TextInput`, focus-clear on outside click)
- `crates/rinch/src/shell/desktop.rs` — `GpuHandle`, `RinchGpuConfig`, `ExternalGpu`, `WgpuRenderer::new` device injection
- `crates/rinch/src/shell/mod.rs` — `run_with_gpu_config` / `run_with_external_device`

### Embed API

Your game owns the window and wgpu device. Rinch runs headless — you feed it events, it produces a Vello scene.

**Key types** (all in `rinch::embed`, re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RinchContext` | Main handle — `new()`, `update()`, `scene()`. Multiple contexts can coexist on one thread (#134): each holds its own `subscribe_signal_change` guard, and bounds signals / editor registrations / focus requests are scoped per document via `DomDocument::doc_key()`. Stores/contexts are namespaced per context with a thread-global fallback (#136): `create_store` inside a context lands in that context's namespace (cleared on drop), its effects/handlers resolve it first, and lookups fall back to stores created outside any context. |
| `RinchContextConfig` | Width, height, scale factor, optional theme |
| `RinchOverlayRenderer` | Convenience Vello-to-texture renderer |
| `GameViewport` | Component marking a transparent hole for game rendering. **Hittable by default** (#207): `wants_mouse` routes input by hit-testing the hole and walking up to `data-viewport`, so an unhittable hole makes the UI claim the mouse *everywhere*. `pointer-events: auto; background: transparent;` come from the UA stylesheet rule for `[data-viewport]` — not an inline style — so restyling the hole can't strip them, and a HUD root's inherited `pointer-events: none` can't reach it. Your own HUD controls under such a root still need `pointer-events: auto`. |
| `LayoutRect` | `{x, y, width, height}` in logical pixels |

**Typical game loop:**
```rust
let mut ctx = RinchContext::new(config, my_ui);
let mut overlay = RinchOverlayRenderer::new(&device, w, h, format);

loop {
    let actions = ctx.update(&events);
    game.render();
    let ui = overlay.render(&device, &queue, ctx.scene());
    composite(game_texture, ui);
}
```

**Source files:**
- `crates/rinch/src/embed.rs` — `RinchContext`, `RinchOverlayRenderer`, `GameViewport`
- `crates/rinch/src/app/mod.rs` — `viewport_rect()`, `has_focused_input()`, `has_focused_contenteditable()`

**Documentation:** `docs/src/guide/game-engine.md`

## Fine-Grained Reactive Rendering

Rinch uses fine-grained reactive rendering for surgical DOM updates. Instead of regenerating HTML on every signal change, reactive expressions become Effects that update specific DOM nodes.

### Architecture

```
Signal.set() → Effect runs → NodeHandle.set_text() → Minimal re-layout
```

**Key principle:** `app()` runs once to build the DOM. Effects handle all reactive updates surgically.

### Key Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `NodeHandle` | `rinch-core/src/dom.rs` | Stable reference to a DOM node for surgical updates |
| `RenderScope` | `rinch-core/src/dom.rs` | Context for building DOM trees with effect tracking |
| `DomDocument` | `rinch-core/src/dom.rs` | Trait abstracting DOM mutation operations |
| `RinchDocument` | `rinch-dom/src/lib.rs` | DOM implementation using Taffy + Parley + Vello |
| `rsx!` | `rinch-macros/src/lib.rs` | Macro generating DOM construction code |

### Usage

Components use `#[component]` and return a `NodeHandle`:

```rust
use rinch::prelude::*;

#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Closure syntax {|| ...} creates a reactive Effect
            p { "Count: " {|| count.get().to_string()} }

            // Reactive styles also use closures
            div {
                style: {|| format!("width: {}px", count.get() * 10)},
                "Progress bar"
            }

            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}
```

The closure syntax `{|| expr}` tells the macro to create an Effect that:
1. Runs the closure and renders the initial value
2. Tracks which signals are read inside the closure
3. Re-runs and updates only that DOM node when those signals change

Without the closure, expressions like `{count.get()}` are captured once at initial render and never update.

### How It Works

1. **Initial Render**: Component runs once, creating DOM nodes via `RenderScope`
2. **Effect Setup**: Dynamic expressions (`{|| expr}`) become Effects with NodeHandles
3. **Signal Changes**: Effects run and surgically update their target nodes
4. **Batched Updates**: Multiple updates are collected for efficient re-layout

**Execution order is a contract** (#154): effects observing the same signal run in **registration order** (the order their `Effect`/`Memo` was created), and the pending queue drains FIFO — so an effect registered *after* an `rsx!` tree sees the post-patch DOM in the same flush ("run me last"), and a signal written from inside an effect queues its observers *behind* the current flush rather than preempting it. Enforced by `BTreeSet<ObserverId>` subscriber sets (ids are monotonic and never reused, so ascending id *is* registration order) plus `pop_front` in `flush_effects`. Don't swap either for a `HashSet`/LIFO. See `docs/src/guide/reactivity.md#execution-order`.

### Native Control Flow (if / for / match)

The `rsx!` macro supports native Rust control flow. All control flow is **always reactive** — conditions, iterators, and scrutinees are automatically wrapped in closures and tracked by Effects.

**`if` / `else` / `if let`:**
```rust
let visible = Signal::new(true);
let user = Signal::new(Some("Alice".to_string()));

rsx! {
    div {
        // if/else
        if visible.get() {
            p { "Visible!" }
        } else {
            p { "Hidden" }
        }

        // if let
        if let Some(name) = user.get() {
            p { "Hello, " {name} "!" }
        }
    }
}
```

**`for` loops with keyed reconciliation:**
```rust
let todos = Signal::new(vec![
    Todo { id: 1, name: "Buy groceries".into() },
    Todo { id: 2, name: "Write code".into() },
]);

rsx! {
    div {
        for todo in todos.get() {
            div { key: todo.id, {todo.name.clone()} }
        }
    }
}
```

The `key:` prop enables efficient keyed reconciliation. Items with matching keys are preserved (not re-rendered). If no `key:` is provided, items are keyed by `Debug` formatting.

**Important:** Items with matching keys are **not** re-rendered when the collection changes. Their existing DOM subtree is preserved as-is. For per-item reactivity, use Signals inside each item.

**`match` with multi-branch rendering:**
```rust
let tab = Signal::new(0);

rsx! {
    div {
        match tab.get() {
            0 => div { "Home" },
            1 => div { "About" },
            _ => div { "Not found" },
        }
    }
}
```

Pattern bindings and guards are supported — each arm re-evaluates the scrutinee to extract bound values.

**Runtime desugaring:** `if` → `show_dom()`, `for` → `for_each_dom_typed()`, `match` → `match_dom()`.

### For Loop Details

The `for` loop variable is **owned** (`T`, not `&T`), so you can capture it directly in `move` closures:

```rust
for todo in todos.get() {
    let id = todo.id;
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: move || todos.update(|t| t.retain(|t| t.id != id)),
            "Delete"
        }
    }
}
```

**Item type requirements:** `Clone + PartialEq + 'static`. The `PartialEq` bound enables selective re-rendering — when the list changes, surviving items (same key) are compared by value. Only items whose data actually changed are re-rendered.

When the list changes, `for` uses keyed reconciliation (LIS algorithm) to compute minimal DOM operations:
- **Insert**: New items are rendered and added at the correct position
- **Remove**: Deleted items have their DOM nodes removed
- **Move**: Reordered items are repositioned without re-rendering
- **Changed**: Surviving items with different data (via `PartialEq`) are re-rendered
- **Unchanged**: Items with matching keys and equal data keep their DOM nodes

**Per-item state in for bodies**: Per-item reactive state works inside `for` loop bodies. Each item gets its own isolated scope:

```rust
for todo in todos.get() {
    let editing = Signal::new(false);  // Per-item state
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: move || editing.update(|v| *v = !*v),
            {|| if editing.get() { "Done" } else { "Edit" }}
        }
    }
}
```

### Programmatic Conditional/List Rendering

For cases requiring explicit control, use the runtime functions directly:

- `show_dom()` — conditional rendering (equivalent to `if`/`else`)
- `for_each_dom_typed()` — keyed list rendering (equivalent to `for`)

### Reactive Component Bindings

Some components support reactive value binding via `_fn` props. The `rsx!` macro auto-wraps `_fn` props — just pass a closure:

| Component | Prop | Type | Purpose |
|--------|------|------|---------|
| `Checkbox` | `checked_fn` | `Option<ReactiveBool>` (`Rc<dyn Fn() -> bool>`) | Reactive checked state |
| `TextInput` | `value_fn` | `Option<ReactiveString>` (`Rc<dyn Fn() -> String>`) | Reactive value binding |

**Controlled Input Pattern:** For controlled inputs, use `value_fn` + `oninput` together. `value_fn` keeps the DOM in sync with your signal; `oninput` updates the signal from user input. Without `value_fn`, programmatic `signal.set("")` won't clear the input visually.

**`onsubmit`:** TextInput supports `onsubmit` which fires when the user presses Enter.

Example - controlled TextInput with submit:
```rust
let input_text = Signal::new(String::new());

rsx! {
    TextInput {
        placeholder: "Type here...",
        value_fn: move || input_text.get(),  // Macro auto-wraps in Some(Rc::new(...))
        oninput: move |value: String| input_text.set(value),
        onsubmit: move || {
            println!("Submitted: {}", input_text.get());
            input_text.set(String::new());  // Clears the input thanks to value_fn
        },
    }
}
```

### RSX Prop Transformation Rules (IMPORTANT - Read Before Using Components)

The `rsx!` macro **automatically wraps** component prop values. You must NOT manually wrap them or you'll get confusing type errors from double-wrapping.

| Prop pattern | What you write | What the macro generates |
|---|---|---|
| `oninput` (closure) | `oninput: move \|val\| do_thing(val)` | `(InputCallback::new(move \|val\| do_thing(val))).into()` |
| `oninput` (value) | `oninput: my_callback` | `(my_callback).into()` |
| `on*` (closure) | `onclick: move \|\| do_thing()` | `(Callback::new(move \|\| do_thing())).into()` |
| `on*` (value) | `onclick: my_callback` | `(my_callback).into()` |
| `*_fn` (reactive) | `value_fn: move \|\| text.get()` | `Some(Rc::new(move \|\| text.get()))` |
| `icon`, `*_icon` | `icon: TablerIcon::Check` | `Some(TablerIcon::Check)` |
| bool literal | `disabled: true` | `true` (no wrapping) |
| int literal | `size: 42` | `Some(42)` |
| float literal | `value: 30.0` | `Some(30.0)` |
| string literal | `variant: "filled"` | `String::from("filled")` |
| `Some(...)` or `None` | `tree: Some(state)` | `Some(state)` (pass-through, preserves unsizing coercion) |
| any other expr | `variant: my_var` | `(my_var).into()` (auto-wraps `T` → `Option<T>` via `From`) |

**Common mistakes (DO NOT do these):**

```rust
// WRONG - don't manually wrap callbacks
TextInput { oninput: Some(InputCallback::new(move |val| ...)) }
// RIGHT - macro wraps closures automatically
TextInput { oninput: move |val: String| input_signal.set(val) }

// WRONG - don't manually wrap callbacks
Button { onclick: Some(Callback::new(|| ...)) }
// RIGHT - just pass the closure
Button { onclick: move || do_something() }

// RIGHT - you can also forward an existing Callback directly
Button { onclick: my_callback }

// WRONG - double-wraps into Some(Some(TablerIcon::Check))
Alert { icon: Some(TablerIcon::Check) }
// RIGHT - macro adds Some(...) for you
Alert { icon: TablerIcon::Check }

// WRONG - component expects String, not Option<String>
Button { variant: Some(String::from("filled")) }
// RIGHT - macro generates String::from("filled")
Button { variant: "filled" }
```

**Additional notes:**
- `on*` props accept both closures and existing `Callback` values. The macro uses `.into()` so the field can be either `Callback` or `Option<Callback>`.
- `Callback` and `InputCallback` have built-in defaults (no-op), so custom components can use `on_toggle: Callback` instead of `Option<Callback>`.
- Component text props (e.g., `variant`, `color`, `size`) are now `String` (not `Option<String>`). Empty string means "not set". The macro auto-converts string literals to `String::from(...)`.
- `_fn` suffix props (e.g., `checked_fn`, `value_fn`) are auto-wrapped — just pass the closure directly, don't wrap in `Some(Rc::new(...))`
- **Note:** ThemeProvider props (`primary_color_fn`, `dark_mode_fn`) use a different codegen path and still require manual `Rc::new()` wrapping

**Component Props vs HTML Attributes:**

- **HTML elements** (`div`, `span`, `p`, etc.) accept any attribute as a string: `style:`, `class:`, `id:`, custom `data-*`, etc. They also support reactive closures `{|| expr}` on any attribute. **`oninput` and `onchange` on `<input>`/`<textarea>` elements** receive the input value as a `String` — use `Fn(String)` closures, not `Fn()`:
  ```rust
  input {
      oninput: move |value: String| name_signal.set(value),
      placeholder: "Type here...",
  }
  ```
- **Components** (`Button`, `TextInput`, `Stack`, etc.) accept their declared struct fields as props. Additionally, all components support these universal props:
  - `style:` — Applied to the component's root DOM element after rendering. Supports static strings and reactive closures.
  - `class:` — Merged with the component's own CSS classes (additive, not replacing). Supports static strings and reactive closures.

```rust
// style: and class: work on all components
Button {
    variant: "filled",
    style: "margin-top: 8px",           // Static style
    class: "my-custom-button",          // Merged with component classes
    onclick: || do_something(),
    "Click me"
}

// Reactive closures also work on component style/class
Text {
    color: "dimmed",
    style: {|| if highlighted.get() { "background: yellow" } else { "" }},
    class: {|| if active.get() { "active" } else { "" }},
    "Dynamic styling"
}
```

**All component props support reactive closures.** Pass `{|| expr}` to any component prop (`variant`, `color`, `size`, `disabled`, etc.) to make it reactive — when signals change, the component re-renders automatically:

```rust
let active = Signal::new(false);
Button {
    variant: {|| if active.get() { "filled" } else { "light" }},
    onclick: move || active.update(|v| *v = !*v),
    "Toggle"
}
```

For surgical updates without full component re-render, use `_fn` props where available (e.g., `checked_fn`, `value_fn`).

| Prop Type | Accepts `{|| ...}` | Update Strategy |
|---|---|---|
| HTML element attributes | Yes | Surgical DOM update |
| Component `style:`/`class:` | Yes | Surgical DOM update |
| Component `_fn` props | Yes (auto-wrapped) | Surgical DOM update |
| Component props (all others) | Yes | Full component re-render |

## Iterative Development with MCP (IMPORTANT)

**Always use the rinch MCP tools to test and iterate on rinch applications.** The MCP server provides direct access to screenshots (viewable inline), DOM inspection with computed styles, and input simulation — no Python scripts or intermediate files needed.

### Workflow: Launch → Test → Close → Edit → Repeat

**Step 1: Launch the app**

Use the MCP `launch_app` tool — it builds, launches, waits for debug registration, and auto-connects:

```
launch_app(package: "ui-zoo-desktop")
```

In headless environments, ensure Xvfb is running: `Xvfb :99 -screen 0 1280x720x24 &`. The `launch_app` tool forwards `DISPLAY` automatically.

**Step 2: Inspect and interact**

Use MCP tools directly — screenshots render inline, DOM queries return computed styles:

```
screenshot()                              → inline PNG image (directly viewable)
dom_tree()                                → full DOM tree with layout + computed styles
query_selector(selector: ".my-class")     → find nodes by tag, .class, [attr], [attr=value]
get_node(id: 42)                          → detailed node info with computed styles + display mode
get_computed_styles(id: 42)               → just the CSS properties for a node
click(x: 100, y: 200)                     → simulate mouse click
wait_frame()                              → wait for next render
type_text(text: "hello")                  → simulate keyboard input
get_text_content(id: 42)                  → get text in subtree
```

**Node geometry — `layout` vs `absolute`.** `dom_tree`/`query_selector`/`get_node` report each
node's box twice: `layout` is **parent-relative** (exactly the node's own `layout`, for checking a
child's offset within its container) and `absolute` is the **on-screen** box. **Pass `absolute.x`/
`absolute.y` (e.g. its center) to `click()`/`mouse_*`** — the `layout` x/y are NOT screen
coordinates. (width/height are identical in both.)

**Step 3: Close the app**

```
close_app()
```

**Step 4: Edit code and repeat**

Make changes, rebuild, launch again. The full cycle:
1. `screenshot()` → view inline → identify issues
2. `dom_tree()` or `get_node()` → check layout and computed styles
3. `click()` → `wait_frame()` → `screenshot()` to verify reactive updates
4. `close_app()` → edit code → rebuild → `launch_app()` → repeat

### What to Check

| Check | How |
|-------|-----|
| Text renders correctly | `screenshot()` — no garbled glyphs, correct wrapping |
| Layout is correct | `dom_tree()` — verify x, y, width, height values |
| Styles applied correctly | `get_computed_styles(id)` — inspect resolved CSS properties |
| Colors/backgrounds | `screenshot()` — visual inspection |
| Click handlers work | `click()` → `wait_frame()` → `screenshot()` to see state change |
| Text content | `get_text_content(id)` to verify reactive values update |
| CSS class matching | `query_selector(selector: ".className")` to find styled elements |

### Common Issues

- **Text wrapping/clipping**: Check that layout measurement and paint use the same font stack
- **Elements stacking wrong**: Check the `display` property. `div` and other block elements default to `display: block` (per the UA stylesheet in `crates/rinch-dom/src/dom_impl/mod.rs`), so flex properties like `align-items` and `justify-content` do nothing until you set `display: flex` explicitly
- **Text not updating**: Verify signal/effect wiring in the component
- **No display (headless)**: Use Xvfb with `DISPLAY=:99` when running without a monitor
- **MCP tools not available**: Ensure `rinch-mcp-server` is built (`cargo build -p rinch-mcp-server`) and `.mcp.json` points to the binary

## Development Notes

- **ui-zoo-desktop** is the primary way to iterate on the framework
- The shell layer handles window management and event loop integration
- Menu callbacks are fully implemented and trigger re-renders automatically
- RSX macro provides helpful error messages with typo suggestions
- Transparent windows use an intermediate render texture (swapchain textures don't support STORAGE_BINDING)

## Documentation Requirements

**Always update user-facing documentation when adding or changing features:**

1. **User Guide** (`docs/src/guide/`): Update relevant guide pages when adding new user-facing features, APIs, or changing behavior
2. **API docs**: Ensure doc comments are added/updated for public APIs
3. **CLAUDE.md**: Update this file when adding new reactive primitives, element types, or architectural changes

Documentation locations:
- `docs/src/guide/hooks.md` - State management guide
- `docs/src/guide/menus.md` - Menu and shortcut guide
- `docs/src/guide/windows.md` - Window management
- `docs/src/guide/reactivity.md` - Signals, effects, memos
- `docs/src/guide/rsx-syntax.md` - RSX macro syntax
- `docs/src/guide/platform.md` - File dialogs, clipboard, system tray
- `docs/src/guide/game-engine.md` - Game engine integration (embed API)
- `docs/src/guide/theming.md` - Theme system and CSS variables
- `docs/src/guide/components.md` - Component library
- `docs/src/guide/contenteditable.md` - Using the rich-text editor (Editor component, EditorHandle, commands)
- `docs/src/guide/editor.md` - Rich-text editor internals (model, schema, steps, plugins, view)
- `docs/src/SUMMARY.md` - Table of contents (update when adding new pages)

Architecture documentation:
- `docs/src/architecture/overview.md` - System architecture and crate structure
- `docs/src/architecture/fine-grained.md` - Fine-grained reactive rendering
- `docs/src/architecture/render-scope.md` - RenderScope and NodeHandle API

Source code documentation:
- `crates/rinch-core/src/dom.rs` - NodeHandle, RenderScope, DomDocument trait
- `crates/rinch-dom/src/lib.rs` - RinchDocument implementation (Taffy + Parley + Vello)
- `crates/rinch-macros/src/dom_codegen.rs` - rsx! macro DOM code generation

## Visual Audit Workflow

Use the rinch MCP tools to systematically compare rinch rendering against expected browser rendering.

**Quick start:**
```
launch_app(package: "ui-zoo-desktop")   # Start app
screenshot()                        # View inline
query_selector(selector: ".class")  # Find elements
get_computed_styles(id: 123)        # Check CSS values
close_app()                         # Done
```

**Full workflow documented at:** `.claude/skills/visual-audit.md`

**Common issues and fixes:**

| Issue | Check | Fix Location |
|-------|-------|--------------|
| Borders appearing unexpectedly | `border_*_width` should be 0 for `border: none` | `computed_style.rs` - check `border-style` |
| SVG icons 0x0 | Missing inline width/height styles | Add `style="width: Xpx; height: Xpx"` |
| currentColor not resolving | Check `is_currentcolor()` handling | `computed_style.rs` |
| Reactive state not updating | Need `{|| expr}` closure syntax | Component render method |
| Menu active state stale | Missing reactive effect | Add `create_effect()` for class updates |
