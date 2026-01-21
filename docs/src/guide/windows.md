# Windows

Rinch supports multi-window applications from the ground up. Each window is declared using the `Window` component in your RSX.

## Basic Window

```rust
use rinch::prelude::*;

fn app() -> Element {
    rsx! {
        Window { title: "My Application", width: 800, height: 600,
            html {
                body {
                    h1 { "Window Content" }
                }
            }
        }
    }
}
```

## Window Properties

| Property | Type | Description |
|----------|------|-------------|
| `title` | `&str` | Window title bar text |
| `width` | `u32` | Initial window width in pixels |
| `height` | `u32` | Initial window height in pixels |
| `decorations` | `bool` | Show window decorations (default: true) |

## Multiple Windows

Create multiple windows by including multiple `Window` elements:

```rust
rsx! {
    Fragment {
        Window { title: "Main Window", width: 800, height: 600,
            // Main window content
        }
        Window { title: "Secondary Window", width: 400, height: 300,
            // Secondary window content
        }
    }
}
```

## Frameless Windows (Custom Chrome)

Create frameless windows for custom title bars and window chrome using `borderless: true`:

```rust
rsx! {
    Window { title: "Frameless", width: 800, height: 600, borderless: true,
        html {
            body {
                div { class: "custom-titlebar",
                    "My Custom Title Bar"
                }
                div { class: "content",
                    // Window content
                }
            }
        }
    }
}
```

### Custom Title Bar Example

```rust
use rinch::prelude::*;

fn app() -> Element {
    rsx! {
        Window { title: "Custom Chrome", width: 800, height: 600, borderless: true,
            html {
                head {
                    style {
                        "
                        * { margin: 0; padding: 0; box-sizing: border-box; }
                        body {
                            font-family: system-ui;
                            background: #1e1e1e;
                            color: white;
                        }
                        .titlebar {
                            height: 32px;
                            background: #2d2d2d;
                            display: flex;
                            align-items: center;
                            justify-content: space-between;
                            padding: 0 8px;
                            -webkit-app-region: drag;
                        }
                        .titlebar-buttons {
                            display: flex;
                            gap: 8px;
                            -webkit-app-region: no-drag;
                        }
                        .titlebar-button {
                            width: 12px;
                            height: 12px;
                            border-radius: 50%;
                            border: none;
                            cursor: pointer;
                        }
                        .close { background: #ff5f57; }
                        .minimize { background: #febc2e; }
                        .maximize { background: #28c840; }
                        .content {
                            padding: 16px;
                            height: calc(100vh - 32px);
                            overflow: auto;
                        }
                        "
                    }
                }
                body {
                    div { class: "titlebar",
                        span { "My App" }
                        div { class: "titlebar-buttons",
                            button { class: "titlebar-button close" }
                            button { class: "titlebar-button minimize" }
                            button { class: "titlebar-button maximize" }
                        }
                    }
                    div { class: "content",
                        h1 { "Welcome" }
                        p { "This window has a custom title bar." }
                    }
                }
            }
        }
    }
}
```

### Transparent Windows

For windows with transparency (useful for rounded corners or non-rectangular shapes):

```rust
rsx! {
    Window {
        title: "Transparent",
        width: 400,
        height: 300,
        borderless: true,
        transparent: true,
        html {
            head {
                style {
                    "
                    body {
                        background: transparent;
                    }
                    .window-content {
                        background: rgba(30, 30, 30, 0.95);
                        border-radius: 12px;
                        margin: 8px;
                        padding: 16px;
                        height: calc(100vh - 16px);
                    }
                    "
                }
            }
            body {
                div { class: "window-content",
                    h1 { "Rounded Window" }
                }
            }
        }
    }
}
```

### Frameless Window with WindowBuilder

```rust
use rinch::windows::WindowBuilder;

let handle = WindowBuilder::new()
    .title("Custom Dialog")
    .size(400, 300)
    .borderless(true)
    .transparent(true)
    .always_on_top(true)
    .content(r#"
        <html>
        <head>
            <style>
                body {
                    background: transparent;
                    font-family: system-ui;
                }
                .dialog {
                    background: white;
                    border-radius: 8px;
                    box-shadow: 0 4px 20px rgba(0,0,0,0.3);
                    padding: 24px;
                    margin: 10px;
                }
            </style>
        </head>
        <body>
            <div class="dialog">
                <h2>Custom Dialog</h2>
                <p>A frameless, transparent dialog!</p>
            </div>
        </body>
        </html>
    "#)
    .open();
```

### Window Properties Reference

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `borderless` | `bool` | `false` | Remove native window decorations |
| `transparent` | `bool` | `false` | Enable window transparency |
| `resizable` | `bool` | `true` | Allow window resizing |
| `always_on_top` | `bool` | `false` | Keep window above others |
| `visible` | `bool` | `true` | Initial visibility state |

## Window Content

Windows contain HTML content rendered by the blitz engine. The content is specified using standard HTML elements:

```rust
rsx! {
    Window { title: "Styled Window", width: 800, height: 600,
        html {
            head {
                style {
                    "
                    body {
                        font-family: system-ui;
                        background: #1e1e1e;
                        color: white;
                    }
                    "
                }
            }
            body {
                main {
                    h1 { "Welcome" }
                    p { "This content is styled with CSS." }
                }
            }
        }
    }
}
```

## Programmatic Window Management

Beyond declaring windows in RSX, you can open and close windows programmatically at runtime using the `windows` module.

### Opening Windows

```rust
use rinch::prelude::*;
use rinch::windows::{open_window, WindowHandle};
use rinch_core::element::WindowProps;

fn app() -> Element {
    let settings_handle = use_signal(|| None::<WindowHandle>);
    let handle_clone = settings_handle.clone();

    rsx! {
        Window { title: "Main", width: 800, height: 600,
            button {
                onclick: move || {
                    let handle = open_window(
                        WindowProps {
                            title: "Settings".into(),
                            width: 400,
                            height: 300,
                            ..Default::default()
                        },
                        "<h1>Settings</h1><p>Configure your app here.</p>".into()
                    );
                    handle_clone.set(Some(handle));
                },
                "Open Settings"
            }
        }
    }
}
```

### Closing Windows

```rust
use rinch::windows::close_window;

// Close a window by its handle
if let Some(handle) = settings_handle.get() {
    close_window(handle);
    settings_handle.set(None);
}
```

### Window Builder Pattern

For more ergonomic window creation, use `WindowBuilder`:

```rust
use rinch::windows::WindowBuilder;

let handle = WindowBuilder::new()
    .title("Settings")
    .size(400, 300)
    .position(100, 100)
    .resizable(false)
    .content("<h1>Settings</h1>")
    .open();
```

### Builder Methods

| Method | Description |
|--------|-------------|
| `title(impl Into<String>)` | Set window title |
| `size(u32, u32)` | Set width and height |
| `position(i32, i32)` | Set initial position |
| `resizable(bool)` | Enable/disable resizing |
| `borderless(bool)` | Remove window decorations |
| `transparent(bool)` | Enable transparency |
| `always_on_top(bool)` | Keep window above others |
| `content(impl Into<String>)` | Set HTML content |
| `open()` | Create the window and return handle |

### Complete Example

```rust
use rinch::prelude::*;
use rinch::windows::{open_window, close_window, WindowBuilder, WindowHandle};

fn app() -> Element {
    let dialogs = use_signal(|| Vec::<WindowHandle>::new());
    let dialogs_open = dialogs.clone();
    let dialogs_close = dialogs.clone();

    rsx! {
        Window { title: "Multi-Window Demo", width: 800, height: 600,
            div {
                button {
                    onclick: move || {
                        let handle = WindowBuilder::new()
                            .title(format!("Dialog {}", dialogs_open.get().len() + 1))
                            .size(300, 200)
                            .content("<p>A new dialog window!</p>")
                            .open();
                        dialogs_open.update(|v| v.push(handle));
                    },
                    "Open New Dialog"
                }
                button {
                    onclick: move || {
                        if let Some(handle) = dialogs_close.get().last().copied() {
                            close_window(handle);
                            dialogs_close.update(|v| { v.pop(); });
                        }
                    },
                    "Close Last Dialog"
                }
                p { "Open dialogs: " {dialogs.get().len()} }
            }
        }
    }
}
```

---

## Window State Persistence

For applications that need to save and restore window positions and sizes, use the `WindowState` API.

### Getting Window State

```rust
use rinch::windows::{get_window_state, WindowHandle, WindowState};

fn save_window_positions(handle: WindowHandle) {
    if let Some(state) = get_window_state(handle) {
        // state contains: x, y, width, height, maximized, minimized
        println!("Window at ({}, {}), size {}x{}",
            state.x, state.y, state.width, state.height);

        // Save to config file, database, etc.
        save_to_config("window", state);
    }
}
```

### WindowState Fields

| Field | Type | Description |
|-------|------|-------------|
| `x` | `i32` | X position (outer window position) |
| `y` | `i32` | Y position (outer window position) |
| `width` | `u32` | Content area width |
| `height` | `u32` | Content area height |
| `maximized` | `bool` | Whether window is maximized |
| `minimized` | `bool` | Whether window is minimized |

### Getting All Window States

```rust
use rinch::windows::get_all_window_states;

fn save_all_windows() {
    for (handle, state) in get_all_window_states() {
        // Save each window's state
        save_to_config(&format!("window_{}", handle.id()), state);
    }
}
```

### Restoring Window State

When creating a window, pass the saved position and size to `WindowProps`:

```rust
use rinch::windows::WindowBuilder;

fn restore_window(saved: WindowState) -> WindowHandle {
    WindowBuilder::new()
        .title("Restored Window")
        .size(saved.width, saved.height)
        .position(saved.x, saved.y)
        .content("<p>Window restored!</p>")
        .open()
}
```

> **Note:** Window state is automatically tracked and updated when windows are moved or resized. The state is available immediately after calling `open_window()` or `WindowBuilder::open()`.

---

## GPU-Accelerated Rendering

All windows are rendered using Vello, a GPU-accelerated 2D graphics library. This provides:

- Smooth animations
- High-quality text rendering
- Efficient repaints
- Cross-platform consistency
