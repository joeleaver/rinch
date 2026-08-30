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

### Callback lifetime

A callback belongs to the component that **created** it — the scope that was rendering when you called `on_click`, which is where the closure captured its `Signal`s. When that component unmounts its signals are freed, so the item stops firing rather than reading freed state (reading a freed signal panics). The callback also runs *inside* that component, so a `Signal` it creates belongs there too.

Ownership is per item, not per menu: one `Menu` may collect items contributed by several components, and each item's callback stops on its own component's unmount. This holds however the item is activated — a native menu click, a tray click, the Linux in-app menu bar, or the keyboard shortcut.

Build the menu outside any component — from `main`, before `run_with_menu`, which is what all the examples do — and there is no owner to record, so the callback lives for the life of the app:

```rust
fn main() {
    // Created in main(), so menu callbacks can reference them for the whole run.
    let count = Signal::new(0);

    let file_menu = Menu::new()
        .item(MenuItem::new("Reset").on_click(move || count.set(0)));

    run_with_menu("My App", 800, 600, app, vec![("File", file_menu)]);
}
```

A callback may rebuild the menu it was dispatched from — including registering new items and shortcuts — from inside its own handler.

Menu ids are also released when the menu that registered them goes away: building a new native menu bar releases the previous bar's, and dropping a `TrayIcon` releases that tray's. Keep the `TrayIcon` for as long as you want its menu to work.

A shortcut consumes the keystroke only when a callback actually runs. A chord belonging to a disabled item, to an item given a `shortcut` but no `on_click`, or to a component that has since unmounted falls through to the app instead of being swallowed — and never shadows a live duplicate of the same chord.

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

On Linux there is no native menu bar (muda needs a GTK window; winit uses raw
X11/Wayland), so rinch renders the menu bar itself as ordinary DOM inside your
window, 28px tall, from the same `Menu`/`MenuItem` API.

Because it lives in the document, it reserves its space with `padding-top` on a
wrapper around your content — **normal flow content clears it automatically**.

A `position: fixed` element does not: fixed resolves against the real viewport,
so a `top: 0` overlay slides underneath the bar. The bar publishes its height as
`--rinch-window-top-inset`, and full-height overlays should offset by it:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0;" }
```

`Drawer`, `Modal`, and top-anchored `Notification`s already handle this. See
[Theming → Window Chrome Inset](./theming.md#window-chrome-inset).

## Context Menus

### Rendered Context Menu

Use the `ContextMenu` component for a styled, theme-aware context menu:

```rust
use rinch::prelude::*;

ContextMenu {
    ContextMenuTarget {
        div { "Right-click me" }
    }
    ContextMenuDropdown {
        DropdownMenuItem { onclick: || edit(), "Edit" }
        DropdownMenuItem { onclick: || duplicate(), "Duplicate" }
        DropdownMenuDivider {}
        DropdownMenuItem { color: "red", onclick: || delete(), "Delete" }
    }
}
```

The `ContextMenu` component automatically:
- Wires up the `oncontextmenu` handler on the target
- Positions the dropdown at the click coordinates using `position: fixed`
- Shows an invisible overlay for click-outside-to-close
- Reuses `DropdownMenuItem` and `DropdownMenuDivider` for consistent styling

### oncontextmenu Event

The `oncontextmenu` prop is available on all HTML elements. It fires on right-click and provides mouse coordinates via `get_click_context()`:

```rust
div {
    oncontextmenu: move || {
        let ctx = get_click_context();
        println!("Right-clicked at ({}, {})", ctx.mouse_x, ctx.mouse_y);
    },
    "Right-click target"
}
```

On Android there is no right button, so a **long press** stands in for it: a
finger held still for 500ms — `ViewConfiguration.getLongPressTimeout()`, the
same deadline the platform's own widgets use — synthesises the same event
through the same dispatch. The press must stay within 8dp to count; moving
further makes it a scroll instead, and lifting before the deadline makes it a
tap. A long press that fires the menu does **not** also fire `onclick`, so a
target can safely carry both.

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
