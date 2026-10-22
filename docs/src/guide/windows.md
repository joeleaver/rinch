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
| `visible` | `bool` | `true` | Whether the window is on screen at startup; `false` starts a tray-only app (see below) |
| `resize_inset` | `Option<f32>` | `None` | Resize handle inset for borderless windows |
| `non_activating` | `bool` | `false` | Never take keyboard focus or activation, even when clicked (see [Overlay Windows](#overlay-windows)) |
| `click_through` | `bool` | `false` | Let pointer events pass through to the window underneath |
| `skip_taskbar` | `bool` | `false` | No taskbar entry (and on Windows, none in Alt+Tab) |
| `level` | `WindowLevel` | `Normal` | How high the window sits: `Normal`, `Floating`, `StatusBar`, `ScreenSaver` (see [Level and workspaces](#level-and-workspaces)) |
| `visible_on_all_workspaces` | `bool` | `false` | Show the window on every workspace / Space, fullscreen ones included |

### Starting with no window (tray-only apps)

`visible: false` starts the app with no window on screen (issue #1520): the
tray is the whole UI until something calls `show_current_window()`, for
example a "Settings…" tray item. The root component is mounted at startup all
the same, at the window's size, so its effects, timers and stores run while
nothing is shown. Showing the window creates the OS window over that same
document, without mounting it again. `hide_current_window()` hides it again,
and so does an `on_close_requested` that calls `hide_current_window()` and
returns `false` (returning `false` alone only cancels the exit). Desktop only; no frame
is presented before the window is first shown (on X11 the window is created
unmapped and destroyed again, measured with `xev`). No renderer exists before
the first show either, so `gpu_handle()` is `None` while the root mounts.

```rust
let props = WindowProps {
    title: "Settings".into(),
    visible: false,
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false
    })),
    ..Default::default()
};
App::new(app).window_props(props).run();
```

`examples/tray-only` is a complete one. Before #1520, `visible` was accepted and
ignored: the window was always shown.

On macOS a hidden window still leaves the app's icon in the Dock, because the
Dock icon belongs to the app, not the window. A tray-only app pairs
`visible: false` with the accessory activation policy (issue #1528), which
removes the Dock icon and the app's menu bar while its window can still be
shown and typed into:

```rust
App::new(app)
    .window_props(props)
    .activation_policy(ActivationPolicy::Accessory)
    .run();
```

`visible: false` does not imply it: an app that only starts hidden and is
otherwise an ordinary app keeps its Dock icon. Without `activation_policy` the
policy is winit's default, unchanged: `Regular` for a bare executable, and the
bundle's `Info.plist` (`LSUIElement`) for a `.app`; setting one overrides the
`Info.plist`. `ActivationPolicy::Prohibited` forbids windows altogether. On
Windows and Linux the call does nothing — an app has a taskbar entry there only
while a window is shown.

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

### Hiding, showing, and where the window comes back

`hide_current_window()` takes the window off screen and
`show_current_window()` puts it back. The window comes back **where it was
and at the size it had** when it was hidden, including a position and size the
user gave it by dragging (issue #1546). On Wayland only the size comes back:
the compositor places windows, and a client cannot learn or choose its
position — except a [layer-shell overlay](#wayland-layer-shell-overlays),
which is placed by rinch and comes back where it was. A maximized window comes back maximized, and un-maximizing it
returns it to the last place and size it had before it was maximized. A
minimized window's position is never recorded (Windows reports it as
(-32000, -32000)), and a position that lies on no connected monitor when the
window is shown (a monitor unplugged while it was hidden) is dropped, so the
system places the window.

The shell does this by recording the window's outer (frame) position and inner
size at the hide and creating the new window with them, rather than by keeping
an invisible window: Wayland has no invisible toplevel, and on macOS winit's
`set_visible(true)` makes the window key, which would activate a
[non-activating](#overlay-windows) panel. A non-activating window is still
shown without taking focus, and click-through and skip-taskbar are kept.

The first time a window is shown after a [hidden start](#starting-with-no-window-tray-only-apps),
it appears at `WindowProps::x`/`y`, as a window created visible does.

### Reading and setting the window's geometry

| Function | Description |
|----------|-------------|
| `current_window_position() -> Option<(f64, f64)>` | Outer top-left in logical pixels; `None` before the window exists and on Wayland (a layer-shell overlay answers its position on its output) |
| `current_window_size() -> Option<(f64, f64)>` | Inner (content) size in logical pixels; `None` before the window exists |
| `set_current_window_position(x, y)` | Move the outer top-left (ignored on Wayland, except a layer-shell overlay) |
| `current_window_surface_kind() -> Option<WindowSurfaceKind>` | `Toplevel`, or `LayerShell` for a Wayland layer-shell overlay |
| `set_current_window_output(Option<&str>)` | The output a layer-shell overlay is on, by connector name |
| `start_window_drag()` / `start_window_resize(edge)` | Move or resize the window with the pointer, from a press handler |
| `set_current_window_size(width, height)` | Resize the content area |

They are in the prelude and callable from any thread. The getters keep
answering while the window is hidden, so an app can read them at exit and
persist them; the setters, called while it is hidden, decide where and how big
it comes back — except for a window hidden while maximized, which comes back
maximized, the set geometry becoming the one it returns to when
un-maximized. On Wayland the position setter does nothing and the position
getter always answers `None`, except for a layer-shell overlay. A setter takes effect on the event loop's next turn, so a getter
called right after it still answers the old value. The window manager may
adjust a requested size or position.

```rust
// At exit (or in on_close_requested): remember the geometry.
if let (Some((x, y)), Some((w, h))) = (current_window_position(), current_window_size()) {
    save_geometry(x, y, w, h);
}

// At startup: pass it back through WindowProps.
let props = WindowProps {
    x: saved.map(|g| g.x as i32),
    y: saved.map(|g| g.y as i32),
    width: saved.map_or(800, |g| g.w as u32),
    height: saved.map_or(600, |g| g.h as u32),
    ..Default::default()
};
```

The position is in logical pixels, like `WindowProps::x`/`y`. On a desktop
whose monitors have different scale factors a logical position is ambiguous;
the hide/show round trip itself does not go through it (it keeps desktop
pixels on X11 and Windows and points on macOS), so a window comes back to the
pixel.

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

Both renderers composite a transparent window with its alpha where the
platform can: the software renderer through softbuffer, the GPU renderer by
configuring its surface with a blending alpha mode and converting Vello's
straight alpha to what that mode reads. A translucent fill looks the same on
either. On Windows the GPU surface offers no blending mode yet, so a
transparent window renders opaque there (issue #89).

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

## Window Facts on Every Target

`rinch::window` (also in the prelude) works the same in a desktop window and in
the browser (issue #1770):

| Function | Desktop | Web |
|----------|---------|-----|
| `open_url(url) -> Result<(), OpenUrlError>` | `xdg-open` / `open` / `ShellExecuteW` | a new tab (`window.open`, `opener` cleared) |
| `set_window_title(title)` | the main window's title bar (any thread) | `document.title` |
| `scale_factor() -> f64` | the main window's display scale | `devicePixelRatio` |
| `window_size() -> (f64, f64)` | the main window's content size, logical px | `innerWidth`, `innerHeight` |
| `window_focused() -> bool` | the main window has the system's focus | the page's window has focus |

The last three are **reactive**: a `{|| …}`, a `Memo` or an `Effect` that reads
one runs again when it changes, and an equal value wakes nothing. That is how an
app hears a resize, a move to a display with another scale, or the window losing
focus:

```rust
#[component]
fn status() -> NodeHandle {
    rsx! {
        p { {|| { let (w, h) = window_size(); format!("{w}x{h} at {}x", scale_factor()) }} }
        p { {|| if window_focused() { "" } else { "Paused while in the background" }} }
        button {
            onclick: || { let _ = open_url("https://example.com/help"); },
            "Help"
        }
    }
}
```

Notes:

- `open_url` refuses a string with no scheme, a leading `-` or a control
  character (`OpenUrlError::Invalid`) and otherwise hands the URL on as given:
  a `file:` URL opens that file, so check the scheme of a URL someone else
  chose. On the desktop `Ok` means the launcher started. In a browser call it
  from a click handler, or the popup blocker answers `OpenUrlError::Blocked`.
  On Android it is `OpenUrlError::Unsupported`.
- `set_window_title` replaces `App::title` and is kept across
  `hide_current_window` / `show_current_window`. A no-op on Android.
- On the desktop the values describe the main window (not the DevTools
  window); a window hidden to the tray reads as not focused; the size is the
  logical size layout uses, in whole pixels. `current_window_size()` answers
  the same size from any thread without subscribing.
- In a native embed (`RinchContext`) nothing publishes them — the host owns
  the window — so they answer `1.0`, `(0.0, 0.0)` and `false` unless the host
  calls `RinchApp::set_publishes_window_state(true)`.

An element's own size is still its `bounds_signal()`, which the runtime
refreshes after every layout pass.

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
| macOS | a non-activating `NSPanel` with `becomesKeyOnlyIfNeeded`, kept on screen while its app is inactive, ordered front at creation without becoming key | `ignoresMouseEvents` | a window has no Dock entry of its own (the app's Dock icon and Cmd-Tab entry are the activation policy's business); the window is kept out of Cmd-` cycling and Mission Control (`ignoresCycle \| stationary`) |
| X11 | `WM_HINTS.input = False` | an empty input shape (set by rinch over its own connection, XFixes version negotiated) | `_NET_WM_STATE_SKIP_TASKBAR` and `_SKIP_PAGER` |
| Wayland | a `wlr-layer-shell` surface with no keyboard ([below](#wayland-layer-shell-overlays)); **no effect** on GNOME | an empty input region | a layer surface has no taskbar entry; alone, **no effect** |

On Windows and X11 such a window is created hidden, adjusted, and only then
shown, so it is never on screen in a state that could take focus. On macOS it is
shown at creation (winit orders a window created inactive to the front without
making it key) and adjusted right after, before any event is handled: showing a
hidden window later would make it key. A window without these fields takes
exactly the path it always did.

### Level and workspaces

Two more fields place an overlay among other windows and workspaces (issue
#1549):

```rust
use rinch_core::element::{WindowLevel, WindowProps};

WindowProps {
    level: WindowLevel::StatusBar,    // above floating windows, the menu bar and the Dock
    visible_on_all_workspaces: true,  // every Space / desktop, fullscreen apps' included
    non_activating: true,
    skip_taskbar: true,
    ..Default::default()
}
```

`level` is `Normal` (the default), `Floating`, `StatusBar` or `ScreenSaver`.
`always_on_top: true` is `Floating`; it never lowers a higher `level`. The
names are macOS's, the one platform with several levels; elsewhere every
raised level is the same "above" state.

| Platform | `Floating` | `StatusBar` | `ScreenSaver` | `visible_on_all_workspaces` |
|---|---|---|---|---|
| macOS | `NSFloatingWindowLevel` (3) | `NSStatusWindowLevel` (25) | `NSScreenSaverWindowLevel` (1000) | `canJoinAllSpaces \| fullScreenAuxiliary` collection behavior |
| Windows | `HWND_TOPMOST` | `HWND_TOPMOST` | `HWND_TOPMOST` | **no effect** (no public API pins a window to every virtual desktop) |
| X11 | `_NET_WM_STATE_ABOVE` | `_NET_WM_STATE_ABOVE` | `_NET_WM_STATE_ABOVE` | `_NET_WM_DESKTOP = 0xFFFFFFFF` and `_NET_WM_STATE_STICKY`, set before the map |
| Wayland | a [layer-shell window](#wayland-layer-shell-overlays): `top` layer; a toplevel: no effect | layer-shell: `overlay` layer | layer-shell: `overlay` layer | no effect |

On macOS a window shows over another app's fullscreen window only with
`visible_on_all_workspaces` (`fullScreenAuxiliary` lets it into that app's
Space) and a raised level. Overlays that do this on macOS are usually
non-activating panels, which is what `non_activating: true` makes; an ordinary
window with the same settings has not been tried on a Mac. Keeping the **app** out of Cmd-Tab and the Dock is
the activation policy (`App::activation_policy(ActivationPolicy::Accessory)`),
not a window setting. On X11 a window with `visible_on_all_workspaces` is
created hidden and shown after the hints are set. All of these come from the
props whenever the window is created, so hiding and showing it again keeps
them.

### Window identity on Linux (`app_id` and `WM_CLASS`)

On Wayland a window's `app_id` is `WindowProps::app_id`, or the executable's
file name when that is not set. On X11 an explicit `WindowProps::app_id` is the
window's `WM_CLASS` (instance and class) since #1519; before, X11 never saw it.
Without one, `WM_CLASS` is unchanged: winit's default, `argv[0]`'s file name,
with `RESOURCE_NAME` as the instance when it is set. The two defaults can
differ (a binary run through a symlink has `argv[0]` naming the link), so a
`.desktop` file whose `StartupWMClass` must match on both should set
`WindowProps::app_id`.

### Wayland layer-shell overlays

A plain xdg-toplevel cannot refuse keyboard focus or a taskbar entry: the
compositor decides (KWin decides whether to activate a window when it is
mapped, and ignores a request to show it inactive). So on native Wayland a
`non_activating` window is not an xdg-toplevel: it is a **`wlr-layer-shell`
surface** (issue #1523) — the kind of surface panels, docks and on-screen
indicators are. It never takes the keyboard (keyboard interactivity *none*),
has no taskbar entry, and sits in the `top` layer, above ordinary windows —
or in the `overlay` layer, above panels and fullscreen windows, when its
`level` is `StatusBar` or `ScreenSaver`.
KWin, wlroots compositors (sway, labwc), Hyprland, niri and COSMIC support it.

`WindowProps::wayland_layer_shell: Option<bool>` decides: `None` (the default)
makes a window a layer surface when it is `non_activating`; `Some(true)` makes
any window one (a focusable overlay, whose keyboard is then *on-demand*: it
takes focus when clicked); `Some(false)` never does. `skip_taskbar` alone does
not: a decorated tool window that only wants no taskbar entry keeps its
decorations and compositor move. Under `App::prefer_x11(true)` the app runs on
XWayland and the X11 column applies instead.

**This changed what a `non_activating` window is on KWin and wlroots**: before
#1523 it was an ordinary window there (with a warning). A layer surface has no
server decorations, title bar, minimize, maximize or fullscreen, and **the
compositor does not move or resize it**. rinch does, so the app's own controls
keep working:

- **Position.** The surface is anchored to its output's top-left corner and
  placed by its margins: `WindowProps::x`/`y`, `set_current_window_position`
  and `current_window_position` are logical pixels **relative to its output**,
  not the desktop. With no position it is not anchored and the compositor
  centres it; `current_window_position` answers `None` until it is moved.
- **Output.** `WindowProps::output` names the output by its connector name
  (`"DP-1"`, `"eDP-1"`, as `wlr-randr` or `kscreen-doctor -o` list them);
  `set_current_window_output(Some("HDMI-A-1"))` moves it there (the surface is
  made again, at the same position and size). An unknown name, or `None`,
  leaves the choice to the compositor.
- **Size.** `set_current_window_size` and `WindowProps::width`/`height` set it.
- **Move and resize by the user.** A `data-drag-window` region and
  `resize_inset` edges work as on any borderless window: rinch follows the
  pointer until the button is released and moves or resizes the surface
  itself. From a handler of your own, call `start_window_drag()` or
  `start_window_resize(edge)` (with `rinch::platform::ResizeDirection`) on the
  press; on an ordinary window they ask the window manager for its
  interactive move or resize. Called with no button held (from a timer, or
  after the press already ended) they do nothing, on every window.
- **Hide and show** keep the position and size, as for any window, and
  showing it takes no focus.
- **Its output going away** (unplugged, or turned off) makes the compositor
  close a layer surface. rinch does not take that for a close: it makes the
  window again on an output the compositor picks (the `output` name is
  dropped until `set_current_window_output` names one again), at the same
  position and size, or, with no output left, as soon as one appears.

`current_window_surface_kind()` answers what the window got:
`Some(WindowSurfaceKind::LayerShell)`, or `Some(WindowSurfaceKind::Toplevel)`
on a compositor without layer shell — **GNOME/Mutter**, where the window is
an ordinary xdg-toplevel as before and rinch logs one warning. There, two
ways to live with it:

- **`App::prefer_x11(true)`** builds the event loop on X11 whenever an X server
  is reachable (`DISPLAY` is set), which under a Wayland session means
  XWayland, where the X11 column above applies. The backend is chosen once per
  process, so this is an application option, not a window one. It changes the
  whole app's backend (fractional scaling and input go through XWayland).
- **Create the overlay once and keep it mapped.** Since activation is decided
  at map time, create the overlay at startup, keep it on screen (fully
  transparent while idle) and click-through, and change its content rather
  than hiding and showing it per use. What remains is a taskbar entry.

The layer role comes from rinch's winit fork
(`WindowAttributesWayland::with_layer_shell`); the renderers, input and
scaling are those of any window. `examples/wayland-overlay` is a bar to try it
with.

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

For the app's main window, use [`current_window_position` / `current_window_size`](#reading-and-setting-the-windows-geometry).

> **Note:** the `WindowHandle` / `WindowState` API below belongs to the
> programmatic window API (`open_window`, `WindowBuilder`), which the desktop
> shell does not process yet: no window is opened from it and no state is
> recorded for it, so `get_window_state` answers `None`.

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
