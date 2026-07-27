# Windows

Rinch supports multi-window applications with window configuration at the runtime level. Windows are created using `WindowProps` when launching the application.

## Basic Window

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div {
            h1 { "Window Content" }
        }
    }
}

fn main() {
    // Window title, width, height, and app function
    run("My Application", 800, 600, app);
}
```

## Window Properties

Configure windows using `WindowProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { "Content" }
    }
}

fn main() {
    let props = WindowProps {
        title: "My Application".into(),
        width: 800,
        height: 600,
        x: Some(100),           // Initial x position
        y: Some(100),           // Initial y position
        borderless: false,      // Show window decorations
        resizable: true,        // Allow resizing
        transparent: false,     // No transparency
        always_on_top: false,   // Normal z-order
        visible: true,          // Show on creation
        resize_inset: None,     // No custom resize handles
    };

    run_with_window_props(app, props, None);
}
```

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `title` | `String` | "Rinch Window" | Window title bar text |
| `width` | `u32` | 800 | Initial window width in pixels |
| `height` | `u32` | 600 | Initial window height in pixels |
| `x` | `Option<i32>` | `None` | Initial x position (centered if None) |
| `y` | `Option<i32>` | `None` | Initial y position (centered if None) |
| `borderless` | `bool` | `false` | Remove native window decorations |
| `resizable` | `bool` | `true` | Allow window resizing |
| `transparent` | `bool` | `false` | Enable window transparency |
| `always_on_top` | `bool` | `false` | Keep window above others |
| `visible` | `bool` | `true` | Initial visibility state |
| `resize_inset` | `Option<f32>` | `None` | Resize handle inset for borderless windows |

## Frameless Windows (Custom Chrome)

Create frameless windows for custom title bars and window chrome using `borderless: true`:

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { class: "custom-titlebar",
            "My Custom Title Bar"
        }
        div { class: "content",
            "Window content"
        }
    }
}

fn main() {
    let props = WindowProps {
        title: "Frameless".into(),
        width: 800,
        height: 600,
        borderless: true,
        ..Default::default()
    };

    run_with_window_props(app, props, None);
}
```

### Custom Title Bar with Window Controls

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
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
        div { class: "titlebar",
            span { "My App" }
            div { class: "titlebar-buttons",
                button {
                    class: "titlebar-button minimize",
                    onclick: || minimize_current_window()
                }
                button {
                    class: "titlebar-button maximize",
                    onclick: || toggle_maximize_current_window()
                }
                button {
                    class: "titlebar-button close",
                    onclick: || close_current_window()
                }
            }
        }
        div { class: "content",
            h1 { "Welcome" }
            p { "This window has a custom title bar." }
        }
    }
}
```

### Window Control Functions

For custom window chrome, use the window control functions from the prelude:

```rust
use rinch::prelude::*;

// In event handlers:
button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }
```

These functions work from onclick handlers and affect the window containing the element.

### Fixed Overlays and Custom Chrome

When you draw your own titlebar (or use `BorderlessWindow`, or the Linux in-app
menu bar), that chrome is an ordinary in-flow element — so normal content sits
below it automatically, but a `position: fixed` overlay does not. Fixed elements
resolve against the real viewport, exactly as in a browser, so a `top: 0` drawer
or modal renders *underneath* your titlebar and covers its close button.

`BorderlessWindow` and the menu bar publish their height as
`--rinch-window-top-inset` for this. Offset full-height overlays by it:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0;" }
```

If you hand-roll the titlebar shown above, publish the variable yourself on a
wrapper around your content so overlays inside it inherit the value:

```rust
div { style: "--rinch-window-top-inset: 36px;",
    // titlebar + content + any fixed overlays
}
```

`Drawer`, `Modal`, and top-anchored `Notification`s already consume it. See
[Theming → Window Chrome Inset](./theming.md#window-chrome-inset).

## Transparent Windows

For windows with transparency (useful for rounded corners or non-rectangular shapes):

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        style {
            "
            body { background: transparent; }
            .window-content {
                background: rgba(30, 30, 30, 0.95);
                border-radius: 12px;
                margin: 8px;
                padding: 16px;
                height: calc(100vh - 16px);
            }
            "
        }
        div { class: "window-content",
            h1 { "Rounded Window" }
        }
    }
}

fn main() {
    let props = WindowProps {
        title: "Transparent".into(),
        width: 400,
        height: 300,
        borderless: true,
        transparent: true,
        resize_inset: Some(8.0),  // Enable resize handles
        ..Default::default()
    };

    run_with_window_props(app, props, None);
}
```

### Custom Resize Handles

When creating borderless windows, native resize handles are not available. Rinch provides custom resize handle support through the `resize_inset` property.

The `resize_inset` value defines the distance (in logical pixels) from the window edges where resize handles are active. This is useful for transparent windows where the visible content is inset from the actual window edges for shadow effects.

**How it works:**
- Resize handles are active within `resize_inset + 8px` from each edge
- Corner resize areas are larger (`resize_inset + 16px`) for easier diagonal resizing
- The cursor automatically changes to indicate resize direction when hovering edges
- Only works when both `borderless: true` and `resizable: true` are set
- On Windows, transparent areas don't receive mouse events, so the resize detection only works on visible content

**Example with shadow:**
```rust
#[component]
fn app() -> NodeHandle {
    rsx! {
        style {
            "
            body { background: transparent; }
            .window {
                margin: 12px;  /* Same as resize_inset */
                background: #1e1e1e;
                border-radius: 8px;
                box-shadow: 0 4px 20px rgba(0,0,0,0.3);
                height: calc(100vh - 24px);
            }
            "
        }
        div { class: "window",
            "Your content here"
        }
    }
}

fn main() {
    let props = WindowProps {
        borderless: true,
        transparent: true,
        resize_inset: Some(12.0),  // Match CSS margin
        ..Default::default()
    };
    run_with_window_props(app, props, None);
}
```

## Observing Window Resizes

There is no separate resize event. The supported way to observe viewport size
changes is a root element's `bounds_signal()`, which the runtime refreshes after
every layout pass — including pure window resizes:

```rust
#[component]
fn app() -> NodeHandle {
    let root = __scope.create_element("div");
    root.set_attribute("style", "width: 100%; height: 100%");
    let bounds = root.bounds_signal(); // Signal<ElementBounds>

    // bounds.get().width / bounds.get().height track the viewport reactively.
    root
}
```

## Programmatic Window Management

Open and close windows programmatically at runtime using the `windows` module.

### Opening Windows

```rust
use rinch::prelude::*;
use rinch::windows::{open_window, WindowHandle};
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    let settings_handle = Signal::new(None::<WindowHandle>);
    let handle_clone = settings_handle.clone();

    rsx! {
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

#[component]
fn app() -> NodeHandle {
    let dialogs = Signal::new(Vec::<WindowHandle>::new());
    let dialogs_open = dialogs.clone();
    let dialogs_close = dialogs.clone();
    let dialogs_display = dialogs.clone();

    rsx! {
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
            p { "Open dialogs: " {|| dialogs_display.get().len().to_string()} }
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

## Rendering Backends

Rinch supports two rendering backends, selected at compile time:

### GPU Mode (`features = ["gpu"]`)

Windows are rendered using Vello, a GPU-accelerated 2D graphics library via wgpu. This provides:

- Smooth animations
- High-quality text rendering
- Efficient GPU-accelerated repaints
- Cross-platform consistency (Vulkan, Metal, DX12, WebGPU)

### Software Mode (default)

Without the `gpu` feature, windows are rendered using tiny-skia (CPU rasterizer) and presented via softbuffer. This provides:

- No GPU required — works in headless, CI, containers, SSH sessions
- Full rendering fidelity (same visual output as GPU mode)
- Dirty region caching — only changed areas are repainted for fast incremental updates
- Subtree pruning — nodes outside the dirty region are skipped during paint
