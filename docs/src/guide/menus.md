# Menus

Rinch provides native menu support through the `muda` library. Menus are configured at the runtime level using `MenuEntry` and `MenuProps`.

## Native Menus

Menus are built using the `MenuEntry` enum and passed to the runtime:

```rust
use rinch::prelude::*;
use rinch::menu::MenuEntry;
use rinch_core::element::{MenuProps, MenuItemProps, MenuItemCallback};

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { "Application content" }
    }
}

fn main() {
    // Menus are configured via the FineGrainedApp builder when you need
    // native menu bars. For a simple app without menus:
    run("My App", 800, 600, app);

    // To add menus, use the FineGrainedApp builder API which accepts
    // a Vec<(MenuProps, Vec<MenuEntry>)> for menu configuration.
}
```

## Menu Types

### MenuEntry

The `MenuEntry` enum represents items in a menu:

```rust
pub enum MenuEntry {
    /// A clickable menu item
    Item(MenuItemProps),
    /// A separator line
    Separator,
    /// A submenu with nested entries
    Submenu(MenuProps, Vec<MenuEntry>),
}
```

### MenuProps

Properties for a menu or submenu:

```rust
pub struct MenuProps {
    pub label: String,
}
```

### MenuItemProps

Properties for a menu item:

```rust
pub struct MenuItemProps {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub onclick: Option<MenuItemCallback>,
}
```

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `label` | `String` | Required | The menu item text |
| `shortcut` | `Option<String>` | `None` | Keyboard shortcut |
| `enabled` | `bool` | `true` | Whether the item is clickable |
| `checked` | `Option<bool>` | `None` | Shows a checkmark next to the item |
| `onclick` | `Option<MenuItemCallback>` | `None` | Callback when clicked or shortcut pressed |

## Menu Callbacks

Use `MenuItemCallback` to handle menu item activation:

```rust
use rinch_core::element::{MenuItemProps, MenuItemCallback};

// Signal created inside a component or with Signal::new() outside render context
let count = Signal::new(0);

let menu_item = MenuItemProps {
    label: "Reset Counter".into(),
    onclick: Some(MenuItemCallback::new(move || {
        count.set(0);
        println!("Counter reset!");
    })),
    ..Default::default()
};
```

Callbacks are triggered both when:
- The user clicks the menu item
- The user presses the keyboard shortcut

## Submenus

Create nested menus using `MenuEntry::Submenu`:

```rust
let menus = vec![
    (MenuProps { label: "View".into() }, vec![
        MenuEntry::Item(MenuItemProps {
            label: "Zoom In".into(),
            shortcut: Some("Cmd+=".into()),
            ..Default::default()
        }),
        MenuEntry::Item(MenuItemProps {
            label: "Zoom Out".into(),
            shortcut: Some("Cmd+-".into()),
            ..Default::default()
        }),
        MenuEntry::Separator,
        MenuEntry::Submenu(
            MenuProps { label: "Appearance".into() },
            vec![
                MenuEntry::Item(MenuItemProps {
                    label: "Light Theme".into(),
                    ..Default::default()
                }),
                MenuEntry::Item(MenuItemProps {
                    label: "Dark Theme".into(),
                    ..Default::default()
                }),
            ]
        ),
    ]),
];
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
MenuItemProps { label: "New".into(), shortcut: Some("Cmd+N".into()), ..Default::default() }
MenuItemProps { label: "Save As".into(), shortcut: Some("Cmd+Shift+S".into()), ..Default::default() }
MenuItemProps { label: "Exit".into(), shortcut: Some("Alt+F4".into()), ..Default::default() }
MenuItemProps { label: "Zoom In".into(), shortcut: Some("Cmd+=".into()), ..Default::default() }
MenuItemProps { label: "Find Next".into(), shortcut: Some("F3".into()), ..Default::default() }
```

Shortcuts work across platforms - `Cmd` is automatically mapped to `Ctrl` on Windows and Linux.

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
use rinch::menu::MenuEntry;
use rinch_core::element::{MenuProps, MenuItemProps, MenuItemCallback};

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
            Show {
                when: {move || show_about.get()},
                div {
                    h2 { "About My App" }
                    p { "Built with Rinch" }
                }
            }
        }
    }
}

fn main() {
    // State signals for menu callbacks (use Signal::new outside render context)
    let file_path = Signal::new(None::<String>);
    let show_about = Signal::new(false);

    let menus = vec![
        (MenuProps { label: "File".into() }, vec![
            MenuEntry::Item(MenuItemProps {
                label: "New".into(),
                shortcut: Some("Cmd+N".into()),
                onclick: Some(MenuItemCallback::new(move || {
                    file_path.set(None);
                    println!("New file created");
                })),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Open...".into(),
                shortcut: Some("Cmd+O".into()),
                onclick: Some(MenuItemCallback::new(move || {
                    file_path.set(Some("example.txt".into()));
                    println!("Opening file...");
                })),
                ..Default::default()
            }),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItemProps {
                label: "Save".into(),
                shortcut: Some("Cmd+S".into()),
                onclick: Some(MenuItemCallback::new(|| println!("Saving..."))),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Save As...".into(),
                shortcut: Some("Cmd+Shift+S".into()),
                ..Default::default()
            }),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItemProps {
                label: "Exit".into(),
                shortcut: Some("Alt+F4".into()),
                ..Default::default()
            }),
        ]),
        (MenuProps { label: "Edit".into() }, vec![
            MenuEntry::Item(MenuItemProps {
                label: "Undo".into(),
                shortcut: Some("Cmd+Z".into()),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Redo".into(),
                shortcut: Some("Cmd+Shift+Z".into()),
                ..Default::default()
            }),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItemProps {
                label: "Cut".into(),
                shortcut: Some("Cmd+X".into()),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Copy".into(),
                shortcut: Some("Cmd+C".into()),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Paste".into(),
                shortcut: Some("Cmd+V".into()),
                ..Default::default()
            }),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItemProps {
                label: "Select All".into(),
                shortcut: Some("Cmd+A".into()),
                ..Default::default()
            }),
        ]),
        (MenuProps { label: "View".into() }, vec![
            MenuEntry::Item(MenuItemProps {
                label: "Zoom In".into(),
                shortcut: Some("Cmd+=".into()),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Zoom Out".into(),
                shortcut: Some("Cmd+-".into()),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "Reset Zoom".into(),
                shortcut: Some("Cmd+0".into()),
                ..Default::default()
            }),
        ]),
        (MenuProps { label: "Help".into() }, vec![
            MenuEntry::Item(MenuItemProps {
                label: "Documentation".into(),
                ..Default::default()
            }),
            MenuEntry::Item(MenuItemProps {
                label: "About".into(),
                onclick: Some(MenuItemCallback::new(move || {
                    show_about.update(|v| *v = !*v);
                })),
                ..Default::default()
            }),
        ]),
    ];

    // Menus are configured via the FineGrainedApp builder.
    // For a simple app without menus, use: run("My App", 800, 600, app);
    run("My App", 800, 600, app);
}
```
