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
│   ├── src/element.rs        # Element enum (Html, Fragment, Widget only), prop types
│   ├── src/hooks.rs          # React-style hooks API (use_signal, use_effect, etc.)
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
├── rinch-widgets/            # UI widgets (optional, enable with `widgets` feature)
│   └── src/
│       ├── button.rs         # Button widget
│       ├── text_input.rs     # Text input widget
│       ├── text.rs           # Typography widget
│       ├── paper.rs          # Card container widget
│       ├── stack.rs          # Vertical flex layout
│       ├── group.rs          # Horizontal flex layout
│       ├── badge.rs          # Status indicator
│       ├── icons.rs          # SVG icon rendering (render_icon function)
│       └── styles.rs         # Widget CSS generation
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
└── ui-zoo-desktop/                # Rich-text editor - primary development target
```

## Element Enum

The Element enum is minimal - only used for content that needs to be embedded in the DOM tree:

- `Element::Html(String)` - Raw HTML content rendered by rinch-dom
- `Element::Fragment(Children)` - Groups multiple elements
- `Element::Widget(Rc<dyn Widget>, Children)` - Custom widget implementation

Shell-level constructs (windows, menus, themes) are handled at the runtime level via props, not as Element variants. See the "Application Entry Point" and "Native Menus" sections below.

DOM content is built using `RenderScope` and `NodeHandle`:

```rust
fn my_component(__scope: &mut RenderScope) -> NodeHandle {
    let div = __scope.create_element("div");
    let text = __scope.create_text("Hello, world!");
    div.append_child(&text);
    div
}
```

## Icon System

Rinch has two icon systems:

1. **`Icon` enum** (rinch-core) - A curated set of ~40 common icons, used by widgets
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

fn my_component(__scope: &mut RenderScope) -> NodeHandle {
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

The `Icon` enum in rinch-core provides a smaller curated set of ~40 icons. These are used internally by widgets like `Alert`, `Notification`, etc.

| Category | Icons |
|----------|-------|
| **Navigation** | `ChevronUp`, `ChevronDown`, `ChevronLeft`, `ChevronRight`, `ChevronsLeft`, `ChevronsRight`, `ArrowUp`, `ArrowDown`, `ArrowLeft`, `ArrowRight` |
| **Actions** | `Close`, `Check`, `Plus`, `Minus`, `Search`, `Settings`, `Edit`, `Trash` |
| **Status/Alerts** | `InfoCircle`, `CheckCircle`, `AlertCircle`, `AlertTriangle`, `XCircle` |
| **Content** | `User`, `Mail`, `Phone`, `Calendar`, `Clock`, `File`, `Folder`, `Image`, `Link`, `ExternalLink` |
| **UI** | `Eye`, `EyeOff`, `Menu`, `MoreHorizontal`, `MoreVertical`, `Loader`, `Quote` |

### Widgets with Icon Support

| Widget | Icon Props |
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

## Application Entry Point

Use the `run` function to start a rinch application:

```rust
use rinch::prelude::*;

fn app(__scope: &mut RenderScope) -> NodeHandle {
    let count = use_signal(|| 0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}

fn main() {
    run("My App", 800, 600, app);
}
```

The component function receives a `RenderScope` and returns a `NodeHandle`. The `rsx!` macro requires `__scope` to be in scope.

## Widget Trait

Widgets implement the `Widget` trait to render directly to DOM nodes:

```rust
pub trait Widget: std::fmt::Debug + 'static {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle;
}
```

Example custom widget:

```rust
#[derive(Debug, Default)]
pub struct MyButton {
    pub label: Option<String>,
    pub onclick: Option<WidgetCallback>,
}

impl Widget for MyButton {
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

## Reactive Patterns in RSX

Use closure syntax `{|| expr}` for reactive expressions that update automatically:

```rust
let count = use_signal(|| 0);

rsx! {
    // Static - captured once at render time
    p { {count.get().to_string()} }

    // Reactive - creates Effect, updates when signal changes
    p { {|| count.get().to_string()} }

    // Reactive attribute
    div { class: {|| if count.get() > 5 { "high" } else { "low" }}, "Value" }
}
```

**Important:** Clone signals before using in multiple closures:

```rust
let count = use_signal(|| 0);
let count_display = count.clone();  // Clone for reactive display
let count_click = count.clone();    // Clone for click handler

rsx! {
    p { {|| count_display.get().to_string()} }
    button { onclick: move || count_click.update(|n| *n += 1), "+" }
}
```

## Hooks API

Rinch provides a React-style hooks API for managing state. Hooks replace the verbose `thread_local!` pattern with a clean, ergonomic API.

### Available Hooks

| Hook | Purpose |
|------|---------|
| `use_signal` | Reactive state that triggers re-renders |
| `use_state` | Simple state with `(value, setter)` tuple |
| `use_ref` | Mutable reference (no re-renders) |
| `use_effect` | Side effects when deps change |
| `use_effect_cleanup` | Effects with cleanup functions |
| `use_mount` | One-time effect on first render |
| `use_memo` | Memoized computations |
| `use_callback` | Memoized callbacks |
| `use_derived` | Auto-tracking computed values (uses reactive Memo) |
| `use_context` | Access shared context values |
| `create_context` | Create shared context values |

### Basic Example

```rust
use rinch::prelude::*;

fn app(__scope: &mut RenderScope) -> NodeHandle {
    // Persistent state - survives across re-renders
    let count = use_signal(|| 0);
    let name = use_signal(|| String::from("World"));

    // Clone for event handlers
    let count_inc = count.clone();

    rsx! {
        div {
            // Use closure syntax {|| ...} for reactive text updates
            h1 { "Hello, " {|| name.get()} "!" }
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count_inc.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    run("Hooks Demo", 800, 600, app);
}
```

> **Important:** The closure syntax `{|| expr}` is required for fine-grained reactive updates. Without it, values are captured once at initial render and never update. See [RSX Syntax - Reactive Expressions](docs/src/guide/rsx-syntax.md#reactive-expressions).

### Rules of Hooks

**Hooks must be called in the same order every render:**

```rust
// ✅ DO: Call hooks at the top level
fn app(__scope: &mut RenderScope) -> NodeHandle {
    let count = use_signal(|| 0);
    let name = use_signal(|| String::new());
    rsx! { /* ... */ }
}

// ❌ DON'T: Call hooks conditionally
fn app(__scope: &mut RenderScope) -> NodeHandle {
    let show = use_signal(|| false);
    if show.get() {
        let extra = use_signal(|| 0);  // WRONG!
    }
    rsx! { /* ... */ }
}

// ❌ DON'T: Call hooks in event handlers
fn app(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        button { onclick: || {
            let x = use_signal(|| 0);  // WRONG!
        }}
    }
}
```

### Hook Reference

**`use_signal`** - Primary state hook:
```rust
let count = use_signal(|| 0);
count.get();              // Read value
count.set(5);             // Set new value
count.update(|n| *n += 1); // Update with function
```

**`use_state`** - React-style tuple API:
```rust
let (count, set_count) = use_state(|| 0);
set_count(count + 1);
```

**`use_ref`** - Mutable reference (no re-renders):
```rust
let render_count = use_ref(|| 0);
*render_count.borrow_mut() += 1;
```

**`use_effect`** - Side effects:
```rust
let count = use_signal(|| 0);
use_effect(|| {
    println!("Count changed to: {}", count.get());
}, count.get());  // Re-runs when count changes
```

**`use_memo`** - Memoized computation:
```rust
let items = use_signal(|| vec![1, 2, 3, 4, 5]);
let sum = use_memo(|| {
    items.get().iter().sum::<i32>()
}, items.get());  // Only recomputes when items change
```

**`use_mount`** - One-time setup:
```rust
use_mount(|| {
    println!("Component mounted!");
    || println!("Component unmounted!")  // Cleanup function
});
```

**`use_derived`** - Auto-tracking computed values:
```rust
let count = use_signal(|| 0);
let multiplier = use_signal(|| 2);

// Automatically tracks count and multiplier - no deps needed!
let result = use_derived(move || count.get() * multiplier.get());
```

**`create_context` / `use_context`** - Shared state:
```rust
#[derive(Clone)]
struct Theme { color: String }

fn app(__scope: &mut RenderScope) -> NodeHandle {
    // Create context at top level
    create_context(Theme { color: "#007bff".into() });
    // ...
}

fn child_component(__scope: &mut RenderScope) -> NodeHandle {
    // Access context anywhere in tree
    let theme = use_context::<Theme>().unwrap();
    // ...
}
```

## Native Menus

Native menus are configured at the runtime level using `FineGrainedApp` and `MenuEntry`:

```rust
use rinch::prelude::*;
use rinch::menu::{MenuEntry, MenuManager};
use rinch_core::element::{MenuProps, MenuItemProps, MenuItemCallback};

fn app(__scope: &mut RenderScope) -> NodeHandle {
    rsx! {
        div { "Application content" }
    }
}

fn main() {
    // Build menu structure
    let menus = vec![
        (MenuProps { label: "File".into() }, vec![
            MenuEntry::Item(MenuItemProps {
                label: "New".into(),
                shortcut: Some("Ctrl+N".into()),
                onclick: Some(MenuItemCallback::new(|| println!("New!"))),
                ..Default::default()
            }),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItemProps {
                label: "Exit".into(),
                onclick: Some(MenuItemCallback::new(|| std::process::exit(0))),
                ..Default::default()
            }),
        ]),
        (MenuProps { label: "Edit".into() }, vec![
            MenuEntry::Item(MenuItemProps {
                label: "Undo".into(),
                shortcut: Some("Ctrl+Z".into()),
                ..Default::default()
            }),
        ]),
    ];

    // Run with menu
    run_with_menu("My App", 800, 600, app, menus);
}
```

Menu callbacks can modify signals:

```rust
let count = use_signal(|| 0);
let count_reset = count.clone();

// In menu construction:
MenuEntry::Item(MenuItemProps {
    label: "Reset Counter".into(),
    onclick: Some(MenuItemCallback::new(move || count_reset.set(0))),
    ..Default::default()
})
```

## Keyboard Shortcuts (built-in)

- `Ctrl/Cmd + +/-/0` - Zoom in/out/reset
- `Alt + D` - Toggle layout debug overlay
- `Alt + I` - Toggle inspect mode (hover highlight for element info)
- `Alt + P` - Toggle performance stats console logging
- `Alt + T` - Print Taffy layout tree (to console)
- `F12` - Toggle DevTools window

## Features

### DevTools Panel

Press F12 to toggle the DevTools panel which shows:
- **Performance**: FPS, frame time, and render time
- **Elements**: DOM tree inspection
- **Styles**: Computed styles for selected elements (enable inspect mode with Alt+I)
- **Hooks**: Current hook state for debugging

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

Enable with `features = ["system-tray"]`:

```rust
use rinch::tray::{TrayIconBuilder, TrayMenu, TrayMenuItem};

let menu = TrayMenu::new()
    .add_item(TrayMenuItem::new("Show").on_click(|| println!("clicked")))
    .add_separator()
    .add_item(TrayMenuItem::new("Quit"));

let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_menu(menu)
    .build()?;
```

### Theme System (optional)

Enable with `features = ["theme"]` for the theme system inspired by Mantine.

Theme is configured at the runtime level using `ThemeProviderProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

fn app(__scope: &mut RenderScope) -> NodeHandle {
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

fn app(__scope: &mut RenderScope) -> NodeHandle {
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

    run_with_window_props(app, window_props);
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

### BorderlessWindow Widget

The `BorderlessWindow` widget provides a complete container for borderless/transparent windows with:
- Rounded corners
- Custom titlebar with drag support
- Window control buttons (minimize, maximize, close)
- Optional left/right custom sections in the titlebar
- Proper theming via CSS variables

```rust
use rinch::prelude::*;

fn app(__scope: &mut RenderScope) -> NodeHandle {
    // For custom left section (e.g., menu button)
    let menu_signal = use_signal(|| false);
    let menu_toggle = menu_signal.clone();
    let left_section: SectionRenderer = Rc::new(move |__scope| {
        let mt = menu_toggle.clone();
        rsx! {
            ActionIcon { onclick: move || mt.update(|v| *v = !*v) }
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
| `on_minimize` | `Option<WidgetCallback>` | Callback for minimize button |
| `on_maximize` | `Option<WidgetCallback>` | Callback for maximize button |
| `on_close` | `Option<WidgetCallback>` | Callback for close button |

### Window Control Functions

For custom window chrome (minimize/maximize/close buttons):

```rust
use rinch::prelude::*;

// In event handlers:
button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }
```

These functions are available in the prelude and work from onclick handlers.

### wgpu Fork

Transparent windows require a patched wgpu to enable Rgba8Unorm storage textures for Vello rendering on DX12. The patches are in `[patch.crates-io]` in `Cargo.toml`:

- **Repository**: https://github.com/joeleaver/wgpu-fork
- **Branch**: `rinch-patch`
- **Upstream PR**: https://github.com/gfx-rs/wgpu/pull/8908

The patches:
1. `instance.rs` - Force storage capabilities for Rgba8Unorm/Bgra8Unorm
2. `device/resource.rs` - Use hardware format features instead of WebGPU defaults
3. `present.rs` - Use adapter format features for surface textures

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

Components receive a `RenderScope` and return a `NodeHandle`:

```rust
use rinch::prelude::*;

fn counter(__scope: &mut RenderScope) -> NodeHandle {
    let count = use_signal(|| 0);
    let count_inc = count.clone();

    rsx! {
        div {
            // Closure syntax {|| ...} creates a reactive Effect
            p { "Count: " {|| count.get().to_string()} }

            // Reactive styles also use closures
            div {
                style: {|| format!("width: {}px", count.get() * 10)},
                "Progress bar"
            }

            button { onclick: move || count_inc.update(|n| *n += 1),
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

### Conditional Rendering (Show)

Use `Show` in RSX for reactive conditional rendering with fine-grained updates:

**Lazy evaluation (recommended when children contain hooks):**
```rust
let visible = use_signal(|| true);

rsx! {
    Show {
        when: {|| visible.get()},
        then: |__scope| rsx! { div { "Visible!" } },
        fallback: || rsx! { div { "Hidden" } },
    }
}
```

**Eager evaluation (simpler syntax, but hooks run even when hidden):**
```rust
let visible = use_signal(|| true);

rsx! {
    Show {
        when: {|| visible.get()},
        fallback: || rsx! { div { "Hidden" } },
        div { "Visible!" }
    }
}
```

**When to use lazy evaluation:**
- When children call hooks (use_signal, use_effect, etc.)
- When children render expensive components
- When you want to defer initialization until the condition is true

**Key difference:**
- With `then:`, the closure body only executes when the condition becomes true
- Without `then:`, children are evaluated immediately at render time

When the condition changes, only the affected nodes are updated - the Effect swaps content surgically.

**Example with component that uses hooks:**
```rust
// Use lazy evaluation to prevent hook panic when section is hidden
Show {
    when: move || current_section.get() == 1,
    then: |__scope| my_section_with_hooks(__scope),
}
```

For programmatic usage, use `show_dom()` directly:

```rust
show_dom(
    __scope,
    move || visible.get(),              // Condition closure
    |scope| {                            // Then branch
        let div = scope.create_element("div");
        div.set_text("Visible!");
        div
    },
    Some(|scope| {                       // Else branch (optional)
        let div = scope.create_element("div");
        div.set_text("Hidden");
        div
    }),
)
```

### List Rendering (For)

Use `For` in RSX for keyed list rendering with fine-grained updates:

```rust
let items = use_signal(|| vec![
    Item { id: "1", name: "Alice" },
    Item { id: "2", name: "Bob" },
]);

rsx! {
    For {
        each: {|| items.get().into_iter().map(|item| {
            ForItem::new(item.id.clone(), item)
        }).collect()},
        |item| {
            let data = item.downcast::<Item>().unwrap();
            rsx! { div { {data.name.clone()} } }
        }
    }
}
```

When the list changes, `For` uses keyed reconciliation (LIS algorithm) to compute minimal DOM operations:
- **Insert**: New items are rendered and added at the correct position
- **Remove**: Deleted items have their DOM nodes removed
- **Move**: Reordered items are repositioned without re-rendering

For programmatic usage, use `for_each_dom()` directly:

```rust
for_each_dom(
    __scope,
    move || items.get(),                    // Items closure
    |item, scope| {                          // View function
        let data = item.downcast::<Item>().unwrap();
        let div = scope.create_element("div");
        div.set_text(&data.name);
        div
    },
)
```

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
3. **CLAUDE.md**: Update this file when adding new hooks, element types, or architectural changes

Documentation locations:
- `docs/src/guide/hooks.md` - Hooks API guide
- `docs/src/guide/menus.md` - Menu and shortcut guide
- `docs/src/guide/windows.md` - Window management
- `docs/src/guide/reactivity.md` - Signals, effects, memos
- `docs/src/guide/rsx-syntax.md` - RSX macro syntax
- `docs/src/guide/platform.md` - File dialogs, clipboard, system tray
- `docs/src/guide/theming.md` - Theme system and CSS variables
- `docs/src/guide/widgets.md` - Widget library components
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
| Reactive state not updating | Need `{|| expr}` closure syntax | Widget render method |
| Menu active state stale | Missing reactive effect | Add `create_effect()` for class updates |
