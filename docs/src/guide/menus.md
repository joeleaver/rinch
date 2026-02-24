# Menus

Rinch provides native menu support through the `muda` library. Menus use a unified builder API (`Menu` / `MenuItem`) shared between native window menus and system tray context menus.

## Native Menus

Use `run_with_menu` to add a native menu bar to your window:

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
        .item(MenuItem::new("Quit").on_click(|| std::process::exit(0)));

    run_with_menu("My App", 800, 600, app, vec![("File", file_menu)]);
}
```

For apps without menus, use `run("My App", 800, 600, app)`.

## Menu Types

### Menu

A container for menu items, separators, and submenus:

```rust
use rinch::menu::{Menu, MenuItem};

let menu = Menu::new()
    .item(MenuItem::new("Open"))
    .separator()
    .submenu("Recent", Menu::new()
        .item(MenuItem::new("file1.txt"))
        .item(MenuItem::new("file2.txt"))
    );
```

### MenuItem

A clickable menu item with optional shortcut and callback:

```rust
use rinch::menu::MenuItem;

let item = MenuItem::new("Save")
    .shortcut("Ctrl+S")
    .enabled(true)
    .on_click(|| println!("Saving..."));
```

| Method | Type | Description |
|--------|------|-------------|
| `new(label)` | `impl Into<String>` | Create a new item |
| `shortcut(s)` | `impl Into<String>` | Keyboard shortcut |
| `enabled(e)` | `bool` | Whether the item is clickable |
| `on_click(cb)` | `impl Fn() + 'static` | Callback when activated |

## Menu Callbacks

Callbacks are `impl Fn() + 'static` — no `Send`/`Sync` required. They always run on the main thread, so you can safely capture `Signal`s:

```rust
use rinch::menu::MenuItem;

let count = Signal::new(0);

let item = MenuItem::new("Reset Counter")
    .on_click(move || {
        count.set(0);
        println!("Counter reset!");
    });
```

Callbacks fire both when the user clicks the menu item and when the keyboard shortcut is pressed.

## Submenus

Create nested menus using `Menu::submenu`:

```rust
use rinch::menu::{Menu, MenuItem};

let view_menu = Menu::new()
    .item(MenuItem::new("Zoom In").shortcut("Ctrl+="))
    .item(MenuItem::new("Zoom Out").shortcut("Ctrl+-"))
    .separator()
    .submenu("Appearance", Menu::new()
        .item(MenuItem::new("Light Theme"))
        .item(MenuItem::new("Dark Theme"))
    );
```

## Keyboard Shortcuts

Shortcuts are specified as strings combining modifiers and a key, separated by `+`.

### Modifiers

| Modifier | macOS | Windows/Linux |
|----------|-------|---------------|
| `Cmd` | Command | Ctrl |
| `Ctrl` | Control | Ctrl |
| `Alt` | Option | Alt |
| `Shift` | Shift | Shift |

### Supported Keys

**Letters:** `A` through `Z`

**Numbers:** `0` through `9`

**Function keys:** `F1` through `F12`

**Special keys:**
- `Enter`, `Return`
- `Escape`, `Esc`
- `Backspace`
- `Tab`
- `Space`
- `Delete`, `Del`

**Navigation:**
- `Home`, `End`
- `PageUp`, `PageDown`
- `Up`, `Down`, `Left`, `Right` (arrow keys)

**Symbols:**
- `=`, `Equal`, `Plus`
- `-`, `Minus`

### Examples

```rust
MenuItem::new("New").shortcut("Ctrl+N")
MenuItem::new("Save As").shortcut("Ctrl+Shift+S")
MenuItem::new("Exit").shortcut("Alt+F4")
MenuItem::new("Zoom In").shortcut("Ctrl+=")
MenuItem::new("Find Next").shortcut("F3")
```

Shortcuts work across platforms - `Cmd` and `Ctrl` are automatically mapped to the platform-appropriate modifier.

## Platform Behavior

### macOS

On macOS, the menu appears in the system menu bar at the top of the screen, following Apple's Human Interface Guidelines.

### Windows

On Windows, the menu appears attached to the window's title bar.

### Linux

On Linux, the menu appears in the window (similar to Windows) unless a global menu system is available.

## Complete Example

```rust
use rinch::prelude::*;
use rinch::menu::{Menu, MenuItem};

#[component]
fn app() -> NodeHandle {
    let file_path = Signal::new(None::<String>);
    let show_about = Signal::new(false);
    rsx! {
        div {
            h1 { "Application with Menus" }
            p {
                "Current file: "
                {|| file_path.get().unwrap_or_else(|| "Untitled".into())}
            }
            if show_about.get() {
                div {
                    h2 { "About My App" }
                    p { "Built with Rinch" }
                }
            }
        }
    }
}

fn main() {
    let file_path = Signal::new(None::<String>);
    let show_about = Signal::new(false);

    let file_menu = Menu::new()
        .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(move || {
            file_path.set(None);
        }))
        .item(MenuItem::new("Open...").shortcut("Ctrl+O").on_click(move || {
            file_path.set(Some("example.txt".into()));
        }))
        .separator()
        .item(MenuItem::new("Save").shortcut("Ctrl+S").on_click(|| println!("Saving...")))
        .item(MenuItem::new("Save As...").shortcut("Ctrl+Shift+S"))
        .separator()
        .item(MenuItem::new("Exit").shortcut("Alt+F4"));

    let edit_menu = Menu::new()
        .item(MenuItem::new("Undo").shortcut("Ctrl+Z"))
        .item(MenuItem::new("Redo").shortcut("Ctrl+Shift+Z"))
        .separator()
        .item(MenuItem::new("Cut").shortcut("Ctrl+X"))
        .item(MenuItem::new("Copy").shortcut("Ctrl+C"))
        .item(MenuItem::new("Paste").shortcut("Ctrl+V"))
        .separator()
        .item(MenuItem::new("Select All").shortcut("Ctrl+A"));

    let help_menu = Menu::new()
        .item(MenuItem::new("Documentation"))
        .item(MenuItem::new("About").on_click(move || {
            show_about.update(|v| *v = !*v);
        }));

    run_with_menu("My App", 800, 600, app, vec![
        ("File", file_menu),
        ("Edit", edit_menu),
        ("Help", help_menu),
    ]);
}
```
