# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on top of [blitz](https://github.com/DioxusLabs/blitz). The goal is to provide a reactive GUI framework using HTML/CSS for layout with a Vello-based renderer.

**Key dependencies:**
- **blitz-dom/blitz-html** - HTML/CSS rendering engine (Stylo for CSS, Taffy for layout, Parley for text)
- **blitz-paint** - Converts DOM to drawing commands
- **vello** - 2D GPU rendering via wgpu
- **winit** - Cross-platform windowing and input
- **muda** - Native menu support

**Design philosophy:** Declarative UI with reactive programming. Windows and menus are elements in the tree. UI content is HTML rendered by blitz.

## Build Commands

```bash
cargo build                    # Build all crates
cargo build -p smyeditor       # Build the editor example
cargo run -p smyeditor         # Run the rich-text editor
cargo clippy                   # Lint
cargo fmt                      # Format
```

## Architecture

```
crates/
├── rinch/                     # Main facade crate
│   ├── src/
│   │   ├── shell/            # Window management, event loop
│   │   │   ├── runtime.rs    # Event loop, processes Element tree
│   │   │   └── window_manager.rs  # ManagedWindow with blitz rendering
│   │   └── menu/             # Native menu support via muda
│   │       └── mod.rs        # MenuManager builds muda menus from Elements
│   └── ...
├── rinch-core/               # Core types
│   ├── src/element.rs        # Element enum, WindowProps, MenuProps, etc.
│   ├── src/hooks.rs          # React-style hooks API (use_signal, use_effect, etc.)
│   └── src/reactive.rs       # Signal, Effect, Memo primitives
└── rinch-renderer/           # (placeholder for custom rendering)

examples/
└── smyeditor/                # Rich-text editor - primary development target
```

## Element Types

- `Element::Window(WindowProps, Children)` - Creates a native OS window
- `Element::AppMenu(AppMenuProps, Children)` - Application menu (native via muda when `native: true`)
- `Element::Menu(MenuProps, Children)` - Submenu within AppMenu
- `Element::MenuItem(MenuItemProps)` - Clickable menu item with optional shortcut
- `Element::MenuSeparator` - Separator line in menus
- `Element::Html(String)` - Raw HTML content rendered by blitz
- `Element::Fragment(Children)` - Groups multiple elements

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

fn app() -> Element {
    // Persistent state - survives across re-renders
    let count = use_signal(|| 0);
    let name = use_signal(|| String::from("World"));

    // Clone for event handlers
    let count_inc = count.clone();

    rsx! {
        Window { title: "Hooks Demo", width: 800, height: 600,
            div {
                h1 { "Hello, " {name.get()} "!" }
                p { "Count: " {count.get()} }
                button { onclick: move || count_inc.update(|n| *n += 1),
                    "Increment"
                }
            }
        }
    }
}
```

### Rules of Hooks

**Hooks must be called in the same order every render:**

```rust
// ✅ DO: Call hooks at the top level
fn app() -> Element {
    let count = use_signal(|| 0);
    let name = use_signal(|| String::new());
    rsx! { /* ... */ }
}

// ❌ DON'T: Call hooks conditionally
fn app() -> Element {
    let show = use_signal(|| false);
    if show.get() {
        let extra = use_signal(|| 0);  // WRONG!
    }
    rsx! { /* ... */ }
}

// ❌ DON'T: Call hooks in event handlers
fn app() -> Element {
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

fn app() -> Element {
    // Create context at top level
    create_context(Theme { color: "#007bff".into() });
    // ...
}

fn child_component() -> Element {
    // Access context anywhere in tree
    let theme = use_context::<Theme>().unwrap();
    // ...
}
```

## Menu Item Callbacks

Menu items support `onclick` callbacks that can modify state:

```rust
let count = use_signal(|| 0);
let count_reset = count.clone();

rsx! {
    AppMenu { native: true,
        Menu { label: "Edit",
            MenuItem {
                label: "Reset Counter",
                onclick: move || count_reset.set(0)
            }
        }
    }
}
```

## Keyboard Shortcuts (built-in)

- `Ctrl/Cmd + +/-/0` - Zoom in/out/reset
- `Alt + D` - Toggle layout debug overlay
- `Alt + I` - Toggle inspect mode (hover highlight for element info)
- `Alt + T` - Print Taffy layout tree (to console)
- `F12` - Toggle DevTools window

## Features

### Hot Reload (optional)

Enable the `hot-reload` feature to watch files and auto-refresh:

```toml
[dependencies]
rinch = { path = "...", features = ["hot-reload"] }
```

### DevTools Overlay

Press F12 to toggle the DevTools panel which shows:
- **Elements**: DOM tree inspection
- **Styles**: Computed styles for selected elements
- **Hooks**: Current hook state for debugging

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

## Transparent Windows (Windows)

Rinch supports true window transparency on Windows via DX12 + DirectComposition:

```rust
Window {
    title: "My App",
    borderless: true,      // Remove native decorations
    transparent: true,     // Enable transparency
    // ... your custom titlebar and content
}
```

**Requirements for transparency:**
- DX12 backend with DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=DxgiFromVisual`)
- `CompositeAlphaMode::PreMultiplied`
- `WS_EX_NOREDIRECTIONBITMAP` window style (handled automatically)
- Patched wgpu for Rgba8Unorm storage textures (see wgpu fork below)

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

Transparent windows require a patched wgpu to enable Rgba8Unorm storage textures for Vello's compute shaders on DX12. The patches are in `[patch.crates-io]` in `Cargo.toml`:

- **Repository**: https://github.com/joeleaver/wgpu-fork
- **Branch**: `rinch-patch`
- **Upstream PR**: https://github.com/gfx-rs/wgpu/pull/8908

The patches:
1. `instance.rs` - Force storage capabilities for Rgba8Unorm/Bgra8Unorm
2. `device/resource.rs` - Use hardware format features instead of WebGPU defaults
3. `present.rs` - Use adapter format features for surface textures

## Development Notes

- **smyeditor** is the primary way to iterate on the framework
- We implement our own shell layer (not blitz-shell) for more control
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
- `docs/src/guide/platform.md` - File dialogs, clipboard, system tray
- `docs/src/SUMMARY.md` - Table of contents (update when adding new pages)
