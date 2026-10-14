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
    // Window title, size, and root component
    App::new(app).title("My Application").size(800, 600).run();
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

    App::new(app).window_props(props).run();
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
| `non_activating` | `bool` | `false` | Never take keyboard focus or activation, even when clicked (see [Overlay Windows](#overlay-windows)) |
| `click_through` | `bool` | `false` | Let pointer events pass through to the window underneath |
| `skip_taskbar` | `bool` | `false` | No taskbar entry (and on Windows, none in Alt+Tab) |

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

    App::new(app).window_props(props).run();
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
`set_click_through_current_window(bool)` belongs to the same family; see
[Overlay Windows](#overlay-windows).

### Fixed Overlays and Custom Chrome

When you draw your own titlebar (or use `BorderlessWindow`, or the Linux in-app
menu bar), that chrome is an ordinary in-flow element — so normal content sits
below it automatically, but a `position: fixed` overlay does not. Fixed elements
resolve against the real viewport, exactly as in a browser, so a `top: 0` drawer
or modal renders *underneath* your titlebar and covers its close button.

`BorderlessWindow` and the menu bar publish their height as
`--rinch-window-top-inset` for this. Offset full-height overlays by it:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0; left: 0; right: 0;" }
```

Give a fixed overlay an inset on **both** axes. An axis with neither inset
(`left` and `right` both `auto`, or `top` and `bottom`) keeps the box where it
stands in the flow on that axis — its *static position*, as in a browser — so
`top: 0; bottom: 0` alone starts wherever the element's parent put it
horizontally, not at the window's left edge. (Before #633 such a box went to
the window's origin on the desktop backend.)

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

    App::new(app).window_props(props).run();
}
```

### Custom Resize Handles

When creating borderless windows, native resize handles are not available. Rinch provides custom resize handle support through the `resize_inset` property.

The `resize_inset` value defines the distance (in logical pixels) from the window edges where resize handles are active. This is useful for transparent windows where the visible content is inset from the actual window edges for shadow effects.

**How it works:**
- Resize handles are active within `resize_inset` from each edge — the inset is the whole grab zone, with nothing added to it
- A corner is simply where two edge zones meet, a `resize_inset` x `resize_inset` square; it is not enlarged, so raise `resize_inset` if you want bigger diagonal handles
- A visible scrollbar thumb takes an edge press (and the hover cursor) off the zone where the two overlap; the empty track past the thumb still resizes, and a corner never yields to the thumb
- The cursor automatically changes to indicate resize direction when hovering edges
- Only works when both `borderless: true` and `resizable: true` are set (for such windows the shell defaults `resize_inset` to `Some(8.0)`)
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
    App::new(app).window_props(props).run();
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

## Overlay Windows

An overlay — a status pill, a HUD, an indicator — sits on top of another
application while the user keeps typing into that application. It must not
take the keyboard: not when it is created, not when it is shown, and not when
it is clicked. Three `WindowProps` fields ask for that (issue #1519):

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

App::new(pill)
    .window_props(WindowProps {
        width: 360,
        height: 44,
        borderless: true,
        transparent: true,
        always_on_top: true,
        non_activating: true, // never takes focus or activation
        skip_taskbar: true,   // no taskbar / Alt+Tab entry
        click_through: false, // the pill's buttons take clicks
        ..Default::default()
    })
    .prefer_x11(true) // see "Wayland" below
    .run();
```

A non-activating window still takes **pointer** clicks in its DOM, so it can
have buttons; it does not take **keystrokes**, so a text field in it cannot be
typed into. `show_current_window()` shows it without activating it.

`click_through: true` lets pointer events pass to whatever is under the window.
Change it at run time with `set_click_through_current_window(bool)`, from any
thread. A click-through window receives no pointer events at all, so nothing
inside it can turn click-through off again: do it from a hotkey, a tray item, a
timer or another thread. The setting is kept when the window is hidden and
shown again.

| Platform | `non_activating` | `click_through` | `skip_taskbar` |
|---|---|---|---|
| Windows | `WS_EX_NOACTIVATE`; `WM_MOUSEACTIVATE` answers `MA_NOACTIVATE`; first shown with `SW_SHOWNOACTIVATE` | `WS_EX_TRANSPARENT \| WS_EX_LAYERED` | `WS_EX_TOOLWINDOW` (no taskbar button, no Alt+Tab entry) |
| macOS | a non-activating `NSPanel` with `becomesKeyOnlyIfNeeded`, kept on screen while its app is inactive, ordered front without becoming key | `ignoresMouseEvents` | nothing to do: a window has no Dock entry of its own (the app's Dock icon is the activation policy's business) |
| X11 | `WM_HINTS.input = False` | an empty input shape | `_NET_WM_STATE_SKIP_TASKBAR` and `_SKIP_PAGER` |
| Wayland | **no effect** | an empty input region | **no effect** |

On Windows, macOS and X11 such a window is created hidden, adjusted, and only
then shown, so it is never on screen in a state that could take focus.

### Wayland

A plain xdg-toplevel cannot refuse keyboard focus or a taskbar entry: the
compositor decides. KWin decides whether to activate a window when it is
**mapped**, and winit's request to show a window inactive is ignored there.
On native Wayland `non_activating` and `skip_taskbar` therefore do nothing,
and rinch logs one warning saying so. (A `wlr-layer-shell` surface could do
it: #1523.)

Two ways to live with it:

- **`App::prefer_x11(true)`** builds the event loop on X11 whenever an X server
  is reachable (`DISPLAY` is set), which under a Wayland session means
  XWayland, where the X11 column above applies. The backend is chosen once per
  process, so this is an application option, not a window one. It changes the
  whole app's backend (fractional scaling and input go through XWayland).
- **Create the overlay once and keep it mapped.** Since activation is decided
  at map time, create the overlay at startup, keep it on screen (fully
  transparent while idle) and click-through, and change its content rather
  than hiding and showing it per use. What remains is a taskbar entry.

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

Rinch has two rendering backends. Every `desktop` build carries the software one; the `gpu` feature adds the GPU one beside it, and a `gpu` build chooses between them **at run time**, when the window opens.

| Build | Renderers it carries |
|-------|----------------------|
| `features = ["desktop"]` | Software only |
| `features = ["desktop", "gpu"]` | GPU and software, chosen at run time |

### GPU (`features = ["gpu"]`)

Windows are rendered using Vello, a GPU-accelerated 2D graphics library via wgpu. This provides:

- Smooth animations
- High-quality text rendering
- Efficient GPU-accelerated repaints
- Cross-platform consistency (Vulkan, Metal, DX12, WebGPU)

### Software (always available on desktop)

Windows are rendered using tiny-skia (CPU rasterizer) and presented via softbuffer. It is the only renderer of a build without the `gpu` feature, and the fallback of a build with it. This provides:

- No GPU required — works in headless, CI, containers, SSH sessions
- Full rendering fidelity (same visual output as the GPU renderer)
- Dirty region caching — only changed areas are repainted for fast incremental updates
- Subtree pruning — nodes outside the dirty region are skipped during paint

### Choosing the renderer at run time

On a `gpu` build, `App::renderer` picks which renderer presents the window:

```rust
use rinch::prelude::*;

App::new(app)
    .renderer(Renderer::Auto) // the default
    .run();
```

| `Renderer` | What it does |
|------------|--------------|
| `Auto` (default) | Presents on the GPU. When the GPU will not start (no adapter, no surface, no device, or a validation error while setting them up), logs a warning and presents with the software renderer instead |
| `Gpu` | Presents on the GPU, and panics when the GPU will not start |
| `Software` | Presents with the software renderer without touching the GPU |

On a build without `gpu` there is only the software renderer, whatever you choose; asking for `Renderer::Gpu` there logs a warning.

**`RINCH_RENDERER` overrides the app's choice**, so a user can force a renderer without a rebuild:

```bash
RINCH_RENDERER=cpu ./my-app      # software
RINCH_RENDERER=gpu ./my-app      # GPU, panic if it will not start
```

It accepts `auto`, `gpu`, `software`, and `cpu` (an alias of `software`), trimmed and in any case. An empty value is ignored, and any other value is logged as a warning and ignored, so the app's own choice applies.

**An app that configures the GPU device itself always presents on the GPU.** `App::gpu_config` and `App::external_gpu` exist so the app can build its own pipelines on rinch's device (see [Game Engine Integration](game-engine.md)), and that app needs the device `gpu_handle()` hands it. So for such an app:

- there is no fallback: a GPU that will not start panics at startup, even under `Renderer::Auto`;
- a `Software` choice, whether from `App::renderer` or from a user's `RINCH_RENDERER=cpu`, is logged as a warning and ignored.

Everywhere else, `gpu_handle()` returns `None` for the whole session while the software renderer presents, so code that reads it should handle `None`.
