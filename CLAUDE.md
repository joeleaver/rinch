# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on rinch-dom, Taffy, Parley, and Vello. The goal is to provide a reactive GUI framework using HTML/CSS for layout with a Vello-based renderer.

**Key dependencies:**
- **rinch-dom** - HTML/CSS DOM implementation (Taffy for layout, Parley for text)
- **vello** - 2D GPU rendering via wgpu
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
│   ├── src/reactive.rs       # Signal, Effect, Memo primitives
│   ├── src/dom.rs            # NodeHandle, RenderScope, DomDocument trait
│   └── src/icon.rs           # Icon enum for type-safe icons
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
│       ├── icons.rs          # SVG icon rendering (render_icon function)
│       └── styles.rs         # Component CSS generation
├── rinch-tabler-icons/       # 5000+ Tabler Icons (build.rs fetches from tabler.io)
│   ├── build.rs              # Downloads and generates icon data from Tabler
│   └── src/lib.rs            # TablerIcon enum, render_tabler_icon function
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

Rinch has two icon systems:

1. **`Icon` enum** (rinch-core) - A curated set of ~40 common icons, used by components
2. **`TablerIcon` enum** (rinch-tabler-icons) - 5000+ icons from [Tabler Icons](https://tabler.io/icons)

### Tabler Icons (Recommended)

The `rinch-tabler-icons` crate provides 5000+ free SVG icons from [Tabler Icons](https://tabler.io/icons). Icons are downloaded at build time from the Tabler CDN.

**Add to Cargo.toml:**
```toml
rinch-tabler-icons = { workspace = true }
```

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

**Features:**
- **5000+ icons** in both Outline and Filled styles
- **Type-safe** - Use enum variants instead of strings
- **Tree-shaking friendly** - Rust dead code elimination removes unused icons
- **Consistent styling** - All icons render at 24x24 with `currentColor`
- **Build-time download** - Icons fetched from Tabler CDN during `cargo build`

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

### Built-in Icon Enum (Legacy)

The `Icon` enum in rinch-core provides a smaller curated set of ~40 icons. These are used internally by components like `Alert`, `Notification`, etc.

| Category | Icons |
|----------|-------|
| **Navigation** | `ChevronUp`, `ChevronDown`, `ChevronLeft`, `ChevronRight`, `ChevronsLeft`, `ChevronsRight`, `ArrowUp`, `ArrowDown`, `ArrowLeft`, `ArrowRight` |
| **Actions** | `Close`, `Check`, `Plus`, `Minus`, `Search`, `Settings`, `Edit`, `Trash` |
| **Status/Alerts** | `InfoCircle`, `CheckCircle`, `AlertCircle`, `AlertTriangle`, `XCircle` |
| **Content** | `User`, `Mail`, `Phone`, `Calendar`, `Clock`, `File`, `Folder`, `Image`, `Link`, `ExternalLink` |
| **UI** | `Eye`, `EyeOff`, `Menu`, `MoreHorizontal`, `MoreVertical`, `Loader`, `Quote` |

### Components with Icon Support

| Component | Icon Props |
|--------|-----------|
| `Alert` | `icon: Option<Icon>` |
| `Notification` | `icon: Option<Icon>` |
| `AccordionControl` | `icon: Option<Icon>` |
| `Blockquote` | `icon: Option<Icon>` |
| `List`, `ListItem` | `icon: Option<Icon>` |
| `Stepper` | `completed_icon`, `progress_icon: Option<Icon>` |
| `StepperStep` | `icon`, `completed_icon`, `progress_icon: Option<Icon>` |
| `NavLink` | `left_section`, `right_section: Option<Icon>` |
| `DropdownMenuItem` | `left_section`, `right_section: Option<Icon>` |
| `Tab` | `left_section`, `right_section: Option<Icon>` |

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

## Drag and Drop

Rinch has two drag systems: **DOM drag attributes** for element-to-element DnD, and the **`Drag` builder** for pointer capture (sliders, panel dragging, resize handles).

### DOM Drag Attributes

Set these attributes on elements to participate in element-to-element drag-and-drop:

| Attribute | Fires on | When |
|-----------|----------|------|
| `data-ondragstart` | Source | Drag begins |
| `data-ondragmove` | Source | Mouse moves during drag |
| `data-ondragenter` | Target | Drag enters a drop target |
| `data-ondragover` | Target | Mouse moves over drop target (every motion event) |
| `data-ondragleave` | Target | Drag leaves a drop target |
| `data-ondrop` | Target | Drop on target |
| `data-ondragend` | Source | Drag finishes |

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
    .start();

// Percentage 0.0–1.0 (sliders) — reads element bounds from ClickContext automatically
Drag::percent()
    .on_move(move |px, _| slider_value.set(px * 100.0))
    .start();

// Cancel without firing on_end
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

### Image Support

Images render on the desktop (Vello) backend via `<img>` elements and `background-image: url(...)` CSS. Images load asynchronously on background threads.

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

## Transparent Windows (Windows)

Rinch supports true window transparency on Windows via DX12 + DirectComposition.

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

**Requirements for transparency:**
- DX12 backend with DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=DxgiFromVisual`)
- `CompositeAlphaMode::PreMultiplied`
- `WS_EX_NOREDIRECTIONBITMAP` window style (handled automatically)
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
registrar.set_texture_source(wgpu_view, w, h);
registrar.notify_frame_ready();

rsx! { RenderSurface { surface: Some(surface), style: "flex: 1;" } }
```

**Source files:**
- `crates/rinch/src/render_surface.rs` — All RenderSurface types and registry

### Embed API

Your game owns the window and wgpu device. Rinch runs headless — you feed it events, it produces a Vello scene.

**Key types** (all in `rinch::embed`, re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RinchContext` | Main handle — `new()`, `update()`, `scene()` |
| `RinchContextConfig` | Width, height, scale factor, optional theme |
| `RinchOverlayRenderer` | Convenience Vello-to-texture renderer |
| `GameViewport` | Component marking a transparent hole for game rendering |
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
| `icon`, `*_icon` | `icon: Icon::Check` | `Some(Icon::Check)` |
| bool literal | `disabled: true` | `true` (no wrapping) |
| int literal | `size: 42` | `Some(42)` |
| float literal | `value: 30.0` | `Some(30.0)` |
| string literal | `variant: "filled"` | `String::from("filled")` |
| any other expr | `variant: my_var` | `my_var` (pass-through) |

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

// WRONG - double-wraps into Some(Some(Icon::Check))
Alert { icon: Some(Icon::Check) }
// RIGHT - macro adds Some(...) for you
Alert { icon: Icon::Check }

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
- **Elements stacking wrong**: Check `display` property — default is `flex row wrap`
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
