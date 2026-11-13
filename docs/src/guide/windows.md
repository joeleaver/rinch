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
        maximized: false,       // Open un-maximized
        visible: true,          // Show on creation
        resize_inset: None,     // No custom resize handles
        ..Default::default()
    };

    App::new(app).window_props(props).run();
}
```

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `title` | `String` | "Rinch Window" | Window title bar text |
| `width` | `u32` | 800 | Initial window width in pixels |
| `height` | `u32` | 600 | Initial window height in pixels |
| `x` | `Option<i32>` | `None` | Initial outer x, logical pixels (system-placed if `None`, or if the window would be out of reach there: see [Reachable positions](#reachable-positions)) |
| `y` | `Option<i32>` | `None` | Initial outer y, as `x` |
| `monitor` | `Option<String>` | `None` | The monitor `x`/`y` is on, by the platform's name for it (from `current_window_restore`); settles a position on a desktop whose monitors have different scales |
| `borderless` | `bool` | `false` | Remove native window decorations |
| `resizable` | `bool` | `true` | Allow window resizing |
| `transparent` | `bool` | `false` | Enable window transparency |
| `always_on_top` | `bool` | `false` | Keep window above others |
| `maximized` | `bool` | `false` | Open the first appearance maximized, `width` x `height` at `x`/`y` being what an un-maximize returns to (see [Bringing the window back as it was](#bringing-the-window-back-as-it-was)) |
| `visible` | `bool` | `true` | Whether the window is on screen at startup; `false` starts a tray-only app (see below) |
| `resize_inset` | `Option<f32>` | `None` | Resize handle inset for borderless windows |
| `non_activating` | `bool` | `false` | Never take keyboard focus or activation, even when clicked (see [Overlay Windows](#overlay-windows)) |
| `click_through` | `bool` | `false` | Let pointer events pass through to the window underneath |
| `skip_taskbar` | `bool` | `false` | No taskbar entry (and on Windows, none in Alt+Tab) |
| `level` | `WindowLevel` | `Normal` | How high the window sits: `Normal`, `Floating`, `StatusBar`, `ScreenSaver` (see [Level and workspaces](#level-and-workspaces)) |
| `visible_on_all_workspaces` | `bool` | `false` | Show the window on every workspace / Space, fullscreen ones included |
| `wayland_layer_shell` | `Option<bool>` | `None` | Make a Wayland window a layer-shell overlay (`None`: when `non_activating`; see [Wayland layer-shell overlays](#wayland-layer-shell-overlays)) |
| `wayland_layer` | `Option<LayerSurface>` | `None` | The whole `wlr-layer-shell` role on Wayland: a panel, dock, wallpaper or kiosk surface; wins over `wayland_layer_shell` and `non_activating` (see [Wayland layer surfaces](#wayland-layer-surfaces-panels-docks-wallpapers)) |

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
the first show either, so `gpu_handle()` is `None` while the root mounts. It
stays `None` after a [preparation](#preparing-a-hidden-window-for-its-first-show)
too, until the window has presented: the prepared renderer's device is
published only once it has been shown to present to the window.

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
(-32000, -32000)), and a position [out of reach](#reachable-positions) when the
window is shown (on a monitor unplugged while it was hidden) is dropped, so
the system places the window.

The shell does this by recording the window's outer (frame) position and inner
size at the hide and creating the new window with them, rather than by keeping
an invisible window: Wayland has no invisible toplevel, and on macOS winit's
`set_visible(true)` makes the window key, which would activate a
[non-activating](#overlay-windows) panel. A non-activating window is still
shown without taking focus, and click-through and skip-taskbar are kept.

The first time a window is shown after a [hidden start](#starting-with-no-window-tray-only-apps),
it appears at `WindowProps::x`/`y`, as a window created visible does.

### Preparing a hidden window for its first show

A window's first show pays for everything its first frame needs, because a
hidden window presents nothing and rinch does that work only while painting a
frame it is about to present: on the GPU the device and Vello's renderer and
pipelines (shader compiles, on a cold shader cache), and on both renderers the
style and layout of what changed while it was hidden, the glyphs its text
needs rasterized and its images decoded. A launcher or a menu kept hidden
until summoned is therefore slowest the first time, which is the summon the
user notices most (issue #2633).

`prepare_current_window()` (and `WindowHandle::prepare()` for a window
`open_window` opened) does that work while the window is hidden, at a time the
app chooses — at start-up, or after it has changed the hidden window's content.
Whatever is pending is styled and laid out at the size the window will be shown
at, and a frame is painted there and not presented. The window is not mapped
and nothing reaches the screen. The next show then costs what a show after a
hide does (#2545 keeps the renderer across a hide):

```rust
#[component]
fn launcher() -> NodeHandle {
    // Started with `visible: false`; queued, and run once the root is mounted.
    prepare_current_window();
    rsx! { /* ... */ }
}
```

It is off by default — a browser does not paint what is not shown — because it
costs memory for as long as the window stays hidden: what a shown window holds,
on the GPU its renderer and UI texture (about 16 MiB of device memory measured
for #2577), in software its pixmap and glyph atlas. Content changed after the
call is styled, laid out and painted by the show as usual; call it again to
prepare that too (a call with nothing changed paints nothing).

**The scale.** A hidden window has no display scale of its own before its
first show (a Wayland surface learns its scale only once mapped, and a layer
surface its output), and glyphs rasterized at the wrong scale are rasterized
again by the show. So a preparation paints at the scale the window had when it
was last on screen; before its first show, at the scale of the output it asks
for (`set_current_window_output`, `WindowProps::output`, else
`WindowProps::monitor`), else of the first output [`outputs()`](#outputs-monitors)
lists — the fractional scale on Wayland, `1.5` where the compositor renders at
1.5. An app that knows better passes the scale:
`prepare_current_window_at_scale(1.5)` / `WindowHandle::prepare_at_scale`. A
wrong guess costs only what the preparation would have saved: the show
restyles and rasterizes again at the real scale, and paints the same pixels.

**Which window.** `prepare_current_window()` prepares the window the calling
code is in, as `WindowHandle::current()` names it: in the content of a
`Window { … }` or an `open_window` window (its mount included) that window,
elsewhere the main one.

**Knowing it is done.** The preparation runs when the event loop next handles
its event, and paints in that turn. Under `RINCH_PERF` its frame has a line of
its own, `[PERF] prepared N: … prepared_frames=1`; it is not a `[PERF] frame`
line, it is not counted in the `frames` that the `perf_stats` debug command
reports (that reports `prepared_frames` beside it), and nothing is presented,
so a check that a hidden window draws no frames holds. What the GPU driver
does with the commands after rinch submits them is not observable: a hardware
driver compiles the pipelines when they are created, inside the preparation
(measured on RADV: the first show after a preparation on a cold shader cache
costs what a later show does), but Mesa's lavapipe compiles them when it first
runs them, on its own queue thread, after the preparation's turn has ended,
and nothing tells the app when that is done.

Pictures are the other part that can land late. A `data:` image is decoded
inside the preparation's turn, but one whose bytes come from a file or an app
image scheme is loaded and decoded on a worker thread and arrives in a later
turn. A prepared window that is still hidden is therefore prepared again by
itself, at the scale it used, at the end of the turn a picture lands in
(another `[PERF] prepared` line), so the show lays out and decodes nothing the
preparation missed. Only pictures do this: any other change to a hidden
window's content waits for the app to call `prepare_current_window()` again.
A hide ends the preparation, and so does a show.

It does nothing while the window is shown (its frames are its preparation), in
an embedded `RinchContext` (no window of its own), and on Android, where it is
a no-op stub; the web backend has no such function, the browser painting
itself. On the GPU, with no window yet to ask, the adapter is the one wgpu
prefers with no surface named, and it is not made the shared one until it has
presented to the window: where it cannot (a laptop whose high-performance GPU
does not drive the display), the show builds a renderer for the window as it
would have without the preparation.

### Reading and setting the window's geometry

| Function | Description |
|----------|-------------|
| `current_window_position() -> Option<(f64, f64)>` | Outer top-left in logical pixels; `None` before the window exists and on Wayland (a layer-shell overlay answers its position on its output) |
| `current_window_size() -> Option<(f64, f64)>` | Inner (content) size in logical pixels; `None` before the window exists |
| `current_window_restore() -> Option<WindowRestore>` | What to persist to bring the window back as it is: its normal (un-maximized) position and size, and whether it is maximized; `None` before the window exists |
| `set_current_window_position(x, y)` | Move the outer top-left (ignored on Wayland, except a layer-shell overlay) |
| `current_window_surface_kind() -> Option<WindowSurfaceKind>` | `Toplevel`, or `LayerShell` for a Wayland layer-shell overlay |
| `set_current_window_output(Option<&str>)` | The output a layer-shell overlay is on, by connector name |
| `set_current_window_layer(LayerSurface)` | Change a Wayland layer surface's layer, anchors, margins, exclusive zone, keyboard or compositor-sized axes at run time (a new output makes it again there) |
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

For a window that can be maximized, persist `current_window_restore()`
instead (below): the getters above answer the maximized geometry while it is
maximized.

The position is in logical pixels, like `WindowProps::x`/`y`: the outer
position over the scale of the monitor the window is on. On a desktop whose
monitors have different scale factors, rinch reads a `WindowProps::x`/`y` back
the same way, in the scale of the monitor it falls on, and creates the window
at the desktop pixels that names (winit alone would convert it with a guessed
scale, such as the monitor under the pointer on X11). A logical position can
name a point on two such monitors (a window at desktop pixel 2100 on a scale-2
monitor to the right of a 1920-pixel scale-1 one is at logical 1050, which is
also on the scale-1 one); `WindowProps::monitor`, the name
`current_window_restore` gives with the position, says which. Without it, or
when no connected monitor has that name, the first the platform lists wins.
Two monitors of one name (macOS names a monitor by its model number, so two of
one model share it): the first of them on which the window is reachable. The hide/show round
trip itself does not go through a logical position (it keeps desktop pixels on
X11 and Windows and points on macOS), so a window comes back to the pixel.

### Bringing the window back as it was

`current_window_restore()` answers a `WindowRestore`:

| Field | Type | Description |
|-------|------|-------------|
| `position` | `Option<(f64, f64)>` | Outer top-left, logical pixels, of the window when it is neither maximized nor minimized (what an un-maximize returns to); `None` on Wayland |
| `size` | `(f64, f64)` | Inner (content) size, logical pixels, likewise |
| `maximized` | `bool` | Whether the window is maximized |
| `monitor` | `Option<String>` | The platform's name for the monitor `position` is on; `None` with no position or where the platform names no monitor |

Give them back as `WindowProps::x`/`y`, `width`/`height`, `maximized` and
`monitor`. A
window that opens maximized answers the geometry it was created with until it
is first un-maximized. On X11, where the window manager maximizes it a moment
after creation, rinch keeps that answer for up to two seconds until the
window says it is maximized, so the maximize itself is not mistaken for a
resize; an un-maximize or a resize to another size within them is still
taken. A window manager that refuses the maximize (a tiling one) has its
geometry read again at the end of those two seconds, and at close.
It keeps answering while the window is hidden, and after a
[hidden start](#starting-with-no-window-tray-only-apps) it answers from the
first show.

```rust
// At exit:
if let Some(r) = current_window_restore() {
    save_restore(r.position, r.size, r.maximized, r.monitor);
}

// At startup:
let props = WindowProps {
    x: saved.position.map(|(x, _)| x as i32),
    y: saved.position.map(|(_, y)| y as i32),
    width: saved.size.0 as u32,
    height: saved.size.1 as u32,
    maximized: saved.maximized,
    monitor: saved.monitor.clone(),
    ..Default::default()
};
```

### Reachable positions

A position — `WindowProps::x`/`y`, or the one a hidden window comes back to —
is used only where the person can reach the window to move it: a grip of its
top, **64 x 12 logical pixels** (or all of a narrower or shorter window),
must be on the desktop somewhere in the window's top 24 pixels. The desktop is
all the connected monitors together, so a window across two adjacent monitors
is reachable; one whose title bar is above every monitor, or with only its
bottom edge or a sliver of its side on one, is not, and the system places it.
A position saved on a monitor that has since been unplugged therefore never
opens the window out of reach. When the platform lists no monitors, the
position is used.

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

- `open_url` refuses, on every target, a string that is not a URL — no
  scheme, a one-letter "scheme" (a Windows path such as `C:\…`), a leading
  `-`, a control character: `OpenUrlError::Invalid` — and the schemes that
  run code, `javascript:`, `vbscript:` and `data:`
  (`OpenUrlError::ScriptScheme`; in a browser `window.open("javascript:…")`
  runs with the page's own origin).
- **Everything else goes to the system's handler, which can launch a
  program.** A `file:` URL opens the file with its default application (an
  executable or script runs), and a custom scheme hands the rest of the URL to
  whatever program registered it. Never pass a URL from untrusted input (a
  document, a message, a server response) without checking its scheme —
  usually allow only `https:` (and `http:`, `mailto:`).
- On the desktop `Ok` means the launcher started. In a browser call it from a
  click handler, or the popup blocker answers `OpenUrlError::Blocked`. On
  Android it is `OpenUrlError::Unsupported`.
- `set_window_title` replaces `App::title` and is kept across
  `hide_current_window` / `show_current_window`. On the desktop the title is
  stored and the running window updated; nothing is queued, so an embed host
  (which owns its window's title) is unaffected. A no-op on Android.
- On the desktop the values describe the main window (not the DevTools
  window); a window hidden to the tray reads as not focused; the size is the
  logical size layout uses, in whole pixels. `current_window_size()` answers
  the same size from any thread without subscribing.
- The three getters belong to the UI thread; read on another thread they
  answer that thread's defaults (and log a warning once per thread).
- In a native embed (`RinchContext`) nothing publishes them — the host owns
  the window — so they answer `1.0`, `(0.0, 0.0)` and `false` unless the host
  turns publishing on for its `RinchApp`
  (`set_publishes_window_state(true)`, which publishes the current scale and
  size at once, so it can follow `RinchContext::new`'s mount).

An element's own size is still its `bounds_signal()`, which the runtime
refreshes after every layout pass. In a browser it is refreshed from
`getBoundingClientRect()` in the frame after anything that can move the element:
its own resize (an animated one included), a DOM write anywhere in the page, a
scroll, a window resize, an image or font load, or the end of a transition or
animation. Two things move a box without any of those and are not seen until
the next one: a `:hover` rule that moves but does not resize it, and the middle
frames of a transition that moves it. It starts at the zero rect on both
backends; the first real rect arrives after the next layout (the next frame on
the web). On desktop, a style a subscriber writes from the rect it is handed is
laid out again before that frame is painted (up to three rounds), so a layout
built from a measurement never shows its old geometry for a frame.

Calling `bounds_signal()` again for the same element from the same owner — the
same component render, effect run or handler — returns the same signal, so
asking for it per event costs nothing after the first call. A call from
another component gets a signal of its own, freed with that component.

## Outputs (Monitors)

`outputs()` lists the monitors connected to the desktop, and it is reactive:
read it in a `{|| …}` expression, a `Memo` or an `Effect`, and that code runs
again when a monitor is plugged in or out, moved, rotated or rescaled. There
is nothing to poll.

```rust
use rinch::prelude::*;

#[component]
fn monitor_list() -> NodeHandle {
    rsx! {
        ul {
            for output in outputs() {
                // Keyed by every field, so a changed output's row is rebuilt.
                li {
                    key: format!("{output:?}"),
                    {format!("{}: {} x {} at {:?}, scale {}",
                        output.name.clone().unwrap_or_default(),
                        output.size.0, output.size.1, output.position, output.scale)}
                }
            }
        }
    }
}
```

Each `Output` has:

| Field | |
|---|---|
| `name` | The platform's name for it: the connector on Wayland and X11 (`DP-1`, `HDMI-A-1`), `\\.\DISPLAY1` on Windows, `Monitor #<model>` on macOS. It is exactly what `WindowProps::monitor` and, for a Wayland layer-shell window, `WindowProps::output` / `set_current_window_output` take, so it is how you put a window on that monitor. |
| `position` | Top-left on the desktop, logical pixels. |
| `size` | Width and height, logical pixels (rotated with the monitor). |
| `scale` | Physical pixels per logical pixel: `1.0`, `2.0`, `1.5`, … |

Positions and sizes are in the units window positions use: logical pixels,
each monitor's desktop rectangle over its own scale on X11 and Windows,
points on macOS, the compositor's logical layout on Wayland (a Wayland
layer-shell window's position is relative to its output's top-left).
Outputs need not be contiguous in any of these: under KWin, giving one output
scale 1.5 shrinks its logical size and leaves its neighbour where it was, with
a gap between them. Read each output's own `position`; do not work it out
from its neighbours.

To act on each change rather than redraw a list, keep the last list and ask
`output_changes` what changed: `OutputChange::Added`, `Removed` or
`Changed { before, after }`, matched by name.

```rust
use rinch_core::Effect;

let mut last: Vec<Output> = Vec::new();
Effect::new(move || {
    let now = outputs();
    for change in output_changes(&last, &now) {
        match change {
            OutputChange::Added(o) => println!("plugged in: {:?}", o.name),
            OutputChange::Removed(o) => println!("unplugged: {:?}", o.name),
            OutputChange::Changed { after, .. } => println!("changed: {after:?}"),
            _ => {}
        }
    }
    last = now;
});
```

**Not every platform reports changes.** `output_tracking()` (reactive too)
says which you have:

| | `output_tracking()` | |
|---|---|---|
| Wayland, X11 | `Live` | Every change, as it happens. |
| Windows, macOS | `Snapshot` | Listed at start-up; later changes are not reported yet ([#2414](https://github.com/joeleaver/rinch/issues/2414)). |
| Before `App::run`, a native embed host, the web, Android | `NotListed` | The list is empty. |

The list is published before the window is created, so the first render
already has it. `examples/outputs` shows the list and logs each change.

## Pointer Lock and Raw Mouse Motion

A game's mouse look or an editor's fly camera needs the pointer hidden and
kept in the window, and the mouse's motion as deltas that do not stop at the
edge of the screen (issue #2092). `rinch::window` (and the prelude) has both:

| Function | What it does |
|----------|--------------|
| `request_pointer_lock() -> Result<(), PointerLockError>` | lock the pointer to the window |
| `request_pointer_lock_on(&element) -> Result<(), PointerLockError>` | the same, held by an element: it ends when the element leaves the document |
| `surface.request_pointer_lock()` | the same, held by a `RenderSurface`, which hears the motion as `SurfaceEvent::PointerMotion { dx, dy }` |
| `exit_pointer_lock()` | let it go |
| `pointer_lock() -> Option<PointerLockMode>` | **reactive**: `Some(Locked)`, `Some(Confined)`, or `None` |
| `on_pointer_motion(\|m: PointerMotion\| …)` | hear raw motion (`m.dx`, `m.dy`); removed when the component unmounts |

```rust
#[component]
fn viewport() -> NodeHandle {
    let yaw = Signal::new(0.0_f64);
    let surface = create_render_surface();
    let s = surface.clone();
    surface.set_event_handler(move |event| match event {
        // A click locks (in a browser a lock needs a gesture like this one).
        SurfaceEvent::MouseDown { .. } => { let _ = s.request_pointer_lock(); }
        SurfaceEvent::PointerMotion { dx, .. } => yaw.update(|y| *y += f64::from(dx) * 0.15),
        _ => {}
    });
    rsx! {
        div {
            p { {|| if pointer_lock().is_some() { "Escape to let go" } else { "Click to look around" }} }
            RenderSurface { surface: Some(surface), style: "flex: 1;" }
        }
    }
}
```

`examples/pointer-lock` is this, with a readout.

**What is granted.** The desktop asks winit for a lock (`CursorGrabMode::Locked`:
Wayland, Windows, macOS) and, where the platform refuses it, for a confinement
(`Confined`: X11, whose winit backend has no lock), and hides the cursor for
either. `pointer_lock()` says which it got. The answer is immediate on the
desktop: after `Ok`, `pointer_lock()` already says `Some(…)`. Under either the
document's pointer position is held where the lock found it — hover does not
move, and a press lands there — and the motion arrives only as deltas; when
the lock ends, hover catches up with where the pointer really is. **The
window must have the focus**: an unfocused window's request is refused with
`PointerLockError::NotFocused` (an X11 confinement would otherwise catch the
pointer over a window a click cannot focus). A click in the window gives it
the focus, which is why a click is the usual moment to ask.

**In the browser** the lock is the Pointer Lock API: `requestPointerLock` on the
element (the page's root element for `request_pointer_lock`, the surface's
`<canvas>` for a surface, or the element bound with `bind_element`). **The browser grants it only during a user gesture**
— call it from a click, press or key handler; from a timer or at startup it is
refused. It also answers later: `Ok` means asked, and `pointer_lock()` turns
`Some(Locked)` when the browser has locked (or stays `None` if it refused).
`exit_pointer_lock()` while the answer is pending withdraws the request: a
grant that arrives afterwards is given straight back (a quick press-to-look
click leaves nothing locked), as is any lock rinch did not ask for.

**Release.** A lock ends on `exit_pointer_lock()`; when the window loses focus
(alt-tab, another window, a tab switch); on **Escape**; when the element or
surface that holds it unmounts (for a surface whose element the app bound,
also when it is unbound or re-bound, or the handle's last clone drops: see
[A surface whose element you built](game-engine.md#a-surface-whose-element-you-built));
and when the desktop window is hidden. An
Escape that ends a lock is the lock's alone: it closes no modal, popup or menu
and reaches no key handler, as in a browser, and neither do its auto-repeats
while it is held — the next press is the app's as usual. With several apps on
one thread (embedded contexts), a lock is the requesting app's: only its own
focus loss and Escape end it. In the browser the browser itself handles Escape and a focus loss, and
`pointer_lock()` follows. Read `pointer_lock()` in a `{|| …}` or an effect to
hear any of these.

**Motion units.** `dx`/`dy` are the platform's own, not logical pixels:
unaccelerated device counts on X11 and Windows, unaccelerated surface units on
Wayland, points on macOS, `movementX`/`movementY` in the browser. Multiply by a
sensitivity of your own. On the desktop `on_pointer_motion` also hears motion
while nothing is locked (whenever the window has focus, wherever the cursor
is), which is enough for a drag-to-orbit camera; a burst within one turn of the
event loop arrives summed as one step. Called while a component renders, the
handler goes with the component; called from an event handler, it lives as
long as the returned `PointerMotionSubscription` (dropping it removes the
handler), so keep that value. In the browser it hears `mousemove`s
over the page, and every motion while locked.

**Where there is none.** Android, a WASI guest, and a native embed host that
installed nothing answer `PointerLockError::Unsupported`; see
[Pointer lock in a context](game-engine.md#pointer-lock-in-a-context) for what an
embed host forwards.

## Starting Other Programs (activation tokens)

An app that starts another program (a launcher, a file manager opening a
file, a terminal opening a link) wants that program's first window to get
the focus. On Wayland the compositor gives it only to a window that brings
an xdg-activation token, which the starting app asks for and hands to the
child:

```rust
use rinch::prelude::*;

div {
    onkeydown: move |k| {
        if k.key != "Enter" {
            return false;
        }
        request_activation_token(move |token| {
            let mut child = std::process::Command::new("foot");
            if let Ok(token) = &token {
                child.env("XDG_ACTIVATION_TOKEN", token.as_str());
            }
            let _ = child.spawn();
        });
        true
    },
}
```

- **Ask from the user action that starts the program**: a key press
  (`onkeydown`) or a pointer button's press or release (`onclick`,
  `onmousedown`, `onmouseup`). The token is tied to that action, and the
  compositor checks it. Asked from anywhere else (a timer, an effect, a move,
  another thread), the answer is `ActivationTokenError::NotFromInput`.
- **The callback runs once, in a later turn**, with the token or an
  `ActivationTokenError`: `Unsupported` off Wayland or on a compositor without
  xdg-activation (start the program without a token then), `NotFromInput`, or
  `WindowClosed`. A component unmounted before the answer is not called.
- **Hide after the callback, not when you ask.** A request whose window is
  hidden or closed before the compositor answers ends with `WindowClosed`:
  once the window's surface is gone, the compositor need not answer, or may
  answer with a token that grants nothing. A launcher that hides once it has
  started something hides in the callback, after the start.
- Give the child the token as `XDG_ACTIVATION_TOKEN` in its environment, or
  as the `activation-token` platform data of a D-Bus `Activate` or `Open`.
  One token is for one start.

## Recording the Window (the composited frame)

To record or stream the whole window — its panels and chrome as well as any
render surface — register a handler with `rinch::window::on_frame_composited`
(desktop, `gpu` feature). It is called on the UI thread once for every frame the
window presents, after the frame is composited and before it is presented, with
the finished frame as a GPU texture on the window's own device:

```rust
use rinch::window::{on_frame_composited, CompositedFrame};

#[component]
fn recorder() -> NodeHandle {
    // Removed when this component unmounts.
    let _ = on_frame_composited(move |frame: &mut CompositedFrame<'_>| {
        // frame.device, frame.queue, frame.encoder, frame.texture / frame.view,
        // frame.size, frame.format, frame.alpha.
        // Copy the frame with an encoder of your own, submit it, then map:
        let buffer: wgpu::Buffer = my_next_free_buffer(frame.size);
        let mut encoder = frame.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            frame.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: my_layout },
            wgpu::Extent3d { width: frame.size.0, height: frame.size.1, depth_or_array_layers: 1 },
        );
        frame.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            // Ok once the copy is done: hand the bytes to your encoder.
        });
    });
    rsx! { div {} }
}
```

- **What it is.** The texture the window is presented from: the UI texture
  (`Rgba8Unorm`, straight alpha) when the window has no composite layers, the
  composited texture (the surface's format, `Rgba8Unorm` or `Bgra8Unorm`, with
  premultiplied alpha) while it has. `frame.format` and `frame.alpha` say
  which; for an opaque window the two alphas are the same. Both have
  `TEXTURE_BINDING | COPY_SRC`. After a partial frame, which redraws only what
  changed, it still holds the whole frame. (The composited texture's format is
  the surface's: `Rgba8Unorm` or `Bgra8Unorm` where the surface offers either,
  else the first it offers.)
- **Read it only.** It is the window's own texture and also carries the usages
  rinch draws it with, so a write is possible, and corrupts the frame the next
  partial frame builds on.
- **Reading it back.** A buffer written by a copy you record into
  `frame.encoder` cannot be mapped inside the handler (the copy is not
  submitted yet, and `map_async` fails): record into an encoder of your own,
  submit it on `frame.queue` and map, as above, or record into `frame.encoder`
  and map at the next frame's call. A map's callback runs only when the device
  is polled. rinch polls without waiting once per presented frame, so the last
  frame before the window goes idle arrives only if the app polls the device
  itself (`device.poll`, from a timer or another thread).
- **What it costs.** Nothing is copied for it: the texture is one the window
  already has. While a handler is registered, one command encoder per frame
  carries the handlers' work (`frame.encoder`, submitted right after the
  handlers return and before the present). The frame's own commands are
  submitted before the call, so work you submit yourself on `frame.queue` sees
  the finished frame too. Nothing waits for the GPU.
- **Valid during the call.** The next frame draws into the same texture, and a
  resize replaces it (with `frame.size` following the window): copy what you
  need inside the handler.
- **Which frames.** Only presented ones. rinch draws when something asks (an
  event, an animation, a continuous `RenderSurface`) and sleeps otherwise; a
  recorder that wants a steady rate repeats its last frame across a gap. A
  hidden window, or one whose surface is unavailable for a frame, hands nothing
  over. The DevTools window is not handed over.
- **Lifetime.** Registered while a component renders, the handler goes when the
  component unmounts. Registered elsewhere (or under `rinch_core::unowned`, to
  outlive the component that started it), the returned `FrameSubscription` is
  its lifetime: dropping it removes the handler. `cancel()` removes it early.
- **Software renderer.** Never called: the software renderer has no GPU frame
  (`rinch::window::presentation()` answers `Unsupported` there). Nor on the web,
  Android or in a native embed, where the host owns the frame.

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

This is rinch's overlay configuration of the role; for a panel, dock or
wallpaper of your own see [Wayland layer surfaces](#wayland-layer-surfaces-panels-docks-wallpapers).
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

### Wayland layer surfaces (panels, docks, wallpapers)

The overlay above is one use of the `wlr-layer-shell` role, configured by
rinch. `WindowProps::wayland_layer: Some(LayerSurface)` (issue #2413) gives
the window the role with a configuration of your own: the protocol's fields,
for panels, taskbars, docks, wallpapers, kiosk and signage surfaces.

```rust
use rinch::prelude::*;
use rinch::{LayerAnchor, LayerAxes, LayerExclusiveZone, LayerSurface, WindowProps};

App::new(bar)
    .window_props(WindowProps {
        width: 800, // the compositor's; the first frame's, and GNOME's
        height: 32,
        borderless: true,
        wayland_layer: Some(LayerSurface {
            anchor: LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
            exclusive_zone: LayerExclusiveZone::Reserve(32),
            compositor_size: LayerAxes::WIDTH,
            ..Default::default()
        }),
        ..Default::default()
    })
    .run();
```

| `LayerSurface` field | What it is |
|---|---|
| `layer` | `Background` (wallpaper), `Bottom` (desktop widgets), `Top` (panels; the default), `Overlay` (above fullscreen windows) |
| `anchor` | Any combination of `LayerAnchor::TOP`, `BOTTOM`, `LEFT`, `RIGHT` (`ALL`, `NONE`), joined with `\|`. One edge: against it, centred along it. Two adjacent: in that corner. Both edges of an axis: centred on it, or stretched with `compositor_size`. None: centred on the output |
| `margins` | `LayerMargins { top, right, bottom, left }`, logical pixels from the anchored edges |
| `exclusive_zone` | `KeepClear` (reserve nothing, stay out of others' reserved space; the default), `Ignore` (lie under it too: a wallpaper), `Reserve(px)` (keep ordinary windows, a maximised one included, that far from the anchored edge: a panel's thickness) |
| `keyboard` | `None` (never takes focus; the default), `OnDemand` (when clicked), `Exclusive` (all keyboard input while mapped in `Top` or `Overlay`: a launcher, a lock screen) |
| `output` | The connector name (`"DP-1"`), or `None` for the compositor's choice |
| `compositor_size` | `LayerAxes::WIDTH` / `HEIGHT` / `BOTH`: the compositor assigns that axis (the protocol's size 0) — the output's extent less the margins, given again when it changes. Only on an axis anchored to both of its edges; anywhere else the window's own size is used and a warning is logged |

The window's content lays out to the size the compositor gives, as it would
to a resize; `WindowProps::width`/`height` is the size on the other axes,
and the first frame's on an assigned one.

**It wins over the overlay shorthands.** With `wayland_layer: Some(..)`, the
window is a layer surface on native Wayland whatever `wayland_layer_shell`
says (`Some(false)` included), its keyboard is the surface's whatever
`non_activating` says (which still applies on X11, Windows and macOS),
`level` does not change its layer, `x`/`y` are not its margins and
`WindowProps::output` is not its output. Without it the shorthands make
exactly the overlay they always did.

**You place it, not rinch.** Its anchors and margins are yours, so rinch
runs **no** move or resize gesture on it: a `data-drag-window` region,
`resize_inset` edges, `start_window_drag()` and `start_window_resize()` do
nothing, `set_current_window_position` is ignored, and
`current_window_position()` answers `None`. Move it by changing its margins.
`set_current_window_size` still sets the size on the axes the compositor
does not assign.

**Changing it while it is open.** `set_current_window_layer(surface)`
(any thread) replaces the configuration: layer, anchors, margins, exclusive
zone, keyboard and `compositor_size` change on the live surface at once. A
panel that grows reserves its new height like this:

```rust
set_current_window_size(width, 48.0);
set_current_window_layer(LayerSurface {
    exclusive_zone: LayerExclusiveZone::Reserve(48),
    ..bar_surface
});
```

Changes made in one turn of the event loop (one handler, effect, timer or
`run_on_main_thread` closure on the UI thread) through
`set_current_window_size`, `set_current_window_position` and
`set_current_window_layer` reach the compositor in **one** commit, in
whatever order you call them, so the panel above is arranged once, at its
new height with its new zone.

A different `output` makes the surface again on that output (the protocol
fixes a layer surface's output when it is made), over the same document, so
the UI keeps its state. While the window is hidden the configuration is
what `show_current_window` makes. An axis the compositor stops assigning
keeps the size it had until you set another.

**GNOME** has no layer shell: there the window is an ordinary window of
`width` x `height`, with one warning, as for an overlay, and
`current_window_surface_kind()` answers `Toplevel`.

The examples run a bar and a wallpaper as two separate processes:
`examples/wayland-bar` (a bottom bar with a clock that reserves its height,
and changes its height or moves to the other edge at run time) and
`examples/wayland-wallpaper` (a `Background` surface the size of its output).
One process can also hold both, and a surface per output: a layer surface is
a window like any other (see [More than one window](#more-than-one-window),
and `App::windowless` for an app made only of such windows).

## More than one window

An app can open as many windows as it likes, each with its own content (#2416).
They all run on the UI thread, in one reactive graph: a `Signal` made in one window
can be read and written in any other.

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        button {
            onclick: || {
                // Its own document, its own OS window, built from this closure.
                open_window(
                    WindowProps { title: "Inspector".into(), width: 360, height: 480, ..Default::default() },
                    |__scope| rsx! { Inspector {} },
                );
            },
            "Open the inspector"
        }
    }
}
```

`open_window(props, content)` returns a `WindowHandle` at once; the window appears on
the event loop's next turn. `props` mean what they mean for the main window (title,
size, position, `wayland_layer`, `output`, `on_close_requested`, …), except
`menu_in_titlebar`: the native menu bar is the main window's. The menu's keyboard
shortcuts work in every window.

### The `Window` element

In `rsx!`, a window is an element: it is open while the element is mounted, and its
children are the window's content, built in the window's own document (not the one
the element sits in).

```rust
let settings = Signal::new(false);
rsx! {
    button { onclick: move || settings.set(true), "Settings" }
    if settings.get() {
        Window {
            title: "Settings",
            width: 480,
            height: 360,
            // Called once the window has closed for any cause but this element
            // going: its close button, the compositor, `close_current_window()`
            // from its content, or `handle.close()` from the app's own code.
            on_closed: move || settings.set(false),
            SettingsPage {}
        }
    }
}
```

- **Its props are `WindowProps` fields, read when it opens.** A prop can't follow a
  signal: `title: {|| name.get()}` is a compile error naming what changes it later
  (`WindowHandle::set_title`, `set_size`, `set_position`, `set_output`, `set_layer`,
  `set_click_through`, `show`/`hide`). A literal goes into its field as written, so
  an `Option` field takes `Some(…)` (`x: Some(40)`).
- **`on_open: move |handle: WindowHandle| …`** is called with the window's handle as
  it opens: keep it in a `Signal<Option<WindowHandle>>` to reach the window from
  outside it. Inside its content, `WindowHandle::current()` is the window.
- **`on_closed`** is called once, after the window has closed, for every cause but
  the element itself going (unmounting it calls nothing: the element's state already
  says it is gone). Clear the state that mounted it there, as above, and the next
  mount opens a new window. A window closed under its element does not reopen by
  itself: the element stays mounted with nothing open until its state changes.
- **In a keyed `for`** it is one window per row: a panel on every output is
  `for output in outputs() { Window { key: output.name.clone().unwrap_or_default(), wayland_layer: …, PanelBar { output } } }`.
  An output unplugged closes its panel once, whether the compositor's close of the
  panel's surface or `outputs()` dropping the output comes first.

`open_window(props, content)` is the same outside `rsx!`.

### An app with no main window

A desktop shell, a panel per output, a tray-and-popups app: `App::windowless(root)`
runs `root` in a document that has no window. It draws nothing; its job is to hold the
components that open the windows.

```rust
use rinch::prelude::*;

fn main() {
    App::windowless(Shell).app_id("my-panel").run();
}

#[component]
fn Shell() -> NodeHandle {
    create_store(ShellConfig::load()); // seen in every panel
    rsx! {
        div {
            for output in outputs() {
                Window {
                    key: output.name.clone().unwrap_or_default(),
                    height: 32,
                    wayland_layer: Some(LayerSurface {
                        anchor: LayerAnchor::TOP | LayerAnchor::LEFT | LayerAnchor::RIGHT,
                        exclusive_zone: LayerExclusiveZone::Reserve(32),
                        output: output.name.clone(),
                        ..Default::default()
                    }),
                    PanelBar { output: output.clone() }
                }
            }
        }
    }
}
```

- It runs until `rinch::exit()` (`ExitWhen::Explicitly`): with no output connected it
  has no window and keeps running. `.exit_when(…)` changes that.
- **Content in the root shows nowhere.** Put it in a `Window`'s content; rinch warns
  once if the root holds visible text, an image or a control.
- In the root, `WindowHandle::current()` and `WindowHandle::main()` are `None`, and a
  `*_current_window` call does nothing (there is no window to act on). `windows()`
  lists only the windows it opened. Every other `App` option (theme, fonts, renderer,
  `app_id`, …) applies as for `App::new`.

### What a window's code can do

- **To its own window:** every `*_current_window` function above acts on the window
  whose code calls it (`set_current_window_size` from the inspector's content resizes
  the inspector), and `WindowHandle::current()` names it. That holds from the first
  line of the window's content: a root component can keep
  `let me = WindowHandle::current();` in its body and it is its own window. Code
  outside any window's document (`main`, a worker thread, a menu or tray callback)
  addresses the main window.
- **To another window:** the same operations through its handle: `show`, `hide`,
  `minimize`, `toggle_maximize`, `focus`, `set_title`, `set_position`, `set_size`,
  `set_output`, `set_layer`, `set_click_through`, `close`, and the getters `position`,
  `size`, `restore` and `surface_kind`. A handle is `Copy` (keep it in a `Signal` or a
  store) and stays valid after its window closes: its operations then do nothing and
  `is_open()` answers `false`.
- `windows()` lists the open windows, the main one first, then oldest first.
  `windows()` and `is_open()` are reactive.

### Who owns a window

A window belongs to the owner in scope when it is opened, exactly as a store does,
and closes when that owner goes:

| opened in | closes when |
|---|---|
| a component body | that component goes |
| a `for` row, an `if` or `match` branch | the row or branch goes |
| an event handler | the scope that built the handler goes: its component, or the row or branch its element is in |
| `main`, a menu or tray callback, or `rinch_core::reactive::unowned(\|\| …)` | it is closed |

A row's button that opens a details window closes it when the row is deleted:

```rust
for item in items.get() {
    div {
        key: item.id,
        {item.name.clone()}
        // Owned by this row: deleting the row closes the window.
        button { onclick: move || { open_window(details_props(&item), details(item.id)); }, "Details" }
        // Outlives the row: it stays open until it is closed.
        button { onclick: move || { rinch_core::reactive::unowned(|| open_window(details_props(&item), details(item.id))); }, "Pop out" }
    }
}
```

The same holds for everything a window's code creates. **A timer, an effect or a store
made by a window's code goes with that window's document**: when the window closes,
they stop. Work that must outlive a window is set up outside it, or under `unowned`.

### State shared between windows

- **Signals** are shared: an effect in one window that reads a signal another window
  writes runs in the same flush as the write.
- **Stores:** one created outside any window, or in the main window's content, is seen
  in every window. One created in a second window's content is that window's alone,
  and `use_store` there falls back to the app-wide one. So several panels can each
  `create_store(PanelState::new())` without colliding, and all read one
  `use_store::<ShellConfig>()`.
- **The theme** is the app's: `rinch::update_theme` restyles every open window, and a
  window opened afterwards is made with the theme current when it opens.
- **Viewport names** name one surface in the whole app, not one per window:
  `GameViewport { name: "main" }` in two windows would be one surface drawn by both.
  Give each window's viewports their own names (for example with an output's name in
  them).

### How a window closes, and when the app ends

A window closes when the user or the compositor asks (its `on_close_requested` is asked
first, except for a layer surface the compositor closed because its output went), when
the app calls `handle.close()` or `close_current_window()` from its code (no veto: the
app is deciding), or when its owner goes. Then its document is disposed (its effects,
handlers, stores and the windows it owns), its OS window goes, and `is_open()` turns
`false`.

When the app ends is `App::exit_when`:

- `ExitWhen::MainWindowCloses`, the default: closing the main window ends the app,
  whatever else is open. Every app written before #2416 behaves as it did.
- `ExitWhen::LastWindowCloses`: the other windows outlive the main one; the last close
  ends the app. With the main window gone, `WindowHandle::main()` and, outside any
  window's document, `WindowHandle::current()` answer `None`.
- `ExitWhen::Explicitly`: the app runs, with or without windows, until `rinch::exit()`.

`rinch::exit()` ends the app under any of them.

To keep a window's geometry across runs, persist [`current_window_restore()`](#bringing-the-window-back-as-it-was) (or `handle.restore()`) and give it back as `WindowProps` at the next start.

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
| `Auto` (default) | Presents on the GPU. When the GPU will not start (no adapter, no surface, no device, or a validation error while setting them up), or the only adapter that can present is a software driver, logs a warning and presents with the software renderer instead |
| `Gpu` | Presents on the GPU, on whatever adapter can present (a software driver included), and panics when the GPU will not start |
| `Software` | Presents with the software renderer without touching the GPU |

On a build without `gpu` there is only the software renderer, whatever you choose; asking for `Renderer::Gpu` there logs a warning.

**Under `Auto`, a software GPU driver counts as no GPU.** Where the GPU cannot present (a headless compositor rendering with pixman, a VM without GPU passthrough, Windows with no GPU driver), the only adapter wgpu can offer is a software driver: Mesa's lavapipe (`llvmpipe`), or Windows' WARP. It runs the GPU renderer on the CPU, at many times the cost of rinch's own software renderer: for a small panel under headless sway, about 4.8 s of CPU and 380 MB to its first frames against under 0.5 s and 90 MB. So `Auto` declines such an adapter as soon as wgpu offers it, before it creates a device, and presents with the software renderer, saying so in the log. What follows is what follows any software fallback: GPU frames (`RenderSurface::set_gpu_render_callback`) do not run, the surface's pixel callback runs if it has one, and `gpu_handle()` is `None`. To use the software driver anyway, ask for the GPU: `Renderer::Gpu`, or `RINCH_RENDERER=gpu` — which is what a test or CI job that runs on lavapipe and needs GPU frames must set. An app that configured the device (`App::gpu_config`, `App::external_gpu`) always uses it.

**`RINCH_RENDERER` overrides the app's choice**, so a user can force a renderer without a rebuild:

```bash
RINCH_RENDERER=cpu ./my-app      # software
RINCH_RENDERER=gpu ./my-app      # GPU, panic if it will not start
```

It accepts `auto`, `gpu`, `software`, and `cpu` (an alias of `software`), trimmed and in any case. An empty value is ignored, and any other value is logged as a warning and ignored, so the app's own choice applies.

**The log names the GPU adapter.** When the GPU renderer starts, rinch logs at `info` the adapter's name, driver, driver info, device type and backend. When the device type is `Cpu` (a software Vulkan driver such as lavapipe, which is what is left where the GPU cannot present: a headless compositor, a VM without GPU passthrough), it also logs a warning, because such a driver costs far more than rinch's own software renderer; `RINCH_RENDERER=software` selects that.

**An app that configures the GPU device itself always presents on the GPU.** `App::gpu_config` and `App::external_gpu` exist so the app can build its own pipelines on rinch's device (see [Game Engine Integration](game-engine.md)), and that app needs the device `gpu_handle()` hands it. So for such an app:

- there is no fallback: a GPU that will not start panics at startup, even under `Renderer::Auto`;
- a `Software` choice, whether from `App::renderer` or from a user's `RINCH_RENDERER=cpu`, is logged as a warning and ignored.

Everywhere else, `gpu_handle()` returns `None` for the whole session while the software renderer presents, so code that reads it should handle `None`.

### GPU antialiasing

The GPU renderer antialiases every shape, image edge and glyph with **area coverage** (`GpuAntialiasing::Area`, Vello's `AaConfig::Area`), which gives an edge any of about 240 levels and is the cheapest mode Vello has. Until issue #2286 the desktop window and the embed overlay drew with 16-sample MSAA, which gives an edge at most 15 levels between ground and ink; on lavapipe a full page of 13px text took 37.7 ms with it and 24.8 ms with area coverage, and on a phone GPU MSAA cost 3.5 times as much for the whole frame (Android always drew with area coverage).

MSAA stays available as an escape hatch:

```rust
use rinch::prelude::*;

App::new(app).gpu_antialiasing(GpuAntialiasing::Msaa16).run();
```

```bash
RINCH_GPU_AA=msaa16 ./my-app      # area, msaa8 or msaa16; overrides the app
```

Area coverage adds up a path's signed area in each pixel, so where one path overlaps itself (a self-crossing stroke, overlapping contours in a variable font) an edge pixel is shaded by the sum of the overlaps, where MSAA counts the samples their union covers. That is the case for choosing MSAA. It does not help seams between separate shapes that abut: the GPU renderer blends each shape with its own coverage in either mode, and such seams look about the same under area coverage, MSAA and the software renderer. The mode is fixed when the first GPU renderer is made: an embedding host calls `rinch::set_gpu_antialiasing` before it creates its `RinchOverlayRenderer`, and a later call is logged and ignored. The software renderer and Android ignore the setting.

### GPU shader compile threads

A GPU renderer compiles its shaders when it is created. The compile runs on at most 4 threads (one on macOS). After it, rinch hands the memory the compilers freed back to the system. A start with a cold driver shader cache, the first start after a driver update, then keeps about 14 MB, where Vello's default of one thread per core kept about 50 MB on a 24-core machine. Each start pays about 0.1 s more for this. With a warm cache the thread count makes no measurable difference. To choose the count:

```rust
use rinch::prelude::*;
use std::num::NonZeroUsize;

App::new(app).gpu_init_threads(NonZeroUsize::new(8).unwrap()).run();
```

```bash
RINCH_GPU_INIT_THREADS=1 ./my-app   # overrides the app
```

An embedding host calls `rinch::set_gpu_init_threads` before it creates its overlay renderer.

### Vsync on and off

The GPU renderer presents with vsync on: each frame waits for the display's next refresh, so on a 60 Hz panel frame-to-frame times never read below 16.7 ms, however cheap the frame. To see what a frame costs — profiling, a benchmark, comparing against another engine — or to offer it as a player setting, turn vsync off, at startup or while the app runs:

```rust
use rinch::prelude::*;

App::new(app).vsync(false).run(); // start with vsync off

// Later, from a handler (or any thread):
let honoured: Presentation = set_vsync(true);
```

`set_vsync` answers what the window's surface does with the request, worked out from the present modes the surface offers:

| `Presentation` | Meaning |
|---|---|
| `Vsync` | Paced to the refresh (`Fifo`, or `FifoRelaxed` where offered). Vsync on, or off on a surface with no uncapped mode |
| `Immediate` | Uncapped; a frame goes to the display as soon as it is drawn, and may tear |
| `Mailbox` | Uncapped without tearing: the display shows the newest frame at each refresh. Used when the surface has no `Immediate` |
| `Pending` | No window renderer yet (before the event loop, or while the window is hidden); the setting is stored and the window takes it |
| `Unsupported` | Nothing to switch: the software renderer, the web, Android |

`rinch::window::presentation()` reads the same answer reactively, and `rinch::window::vsync()` whether vsync is asked for (also reactive), so a settings toggle can show both and grey itself out on `Unsupported`. Both are for the UI thread: `presentation()` read on another thread stays `Pending` (keep what `set_vsync` returned there), and a `set_vsync` made on another thread reaches the UI thread's readers when the shell applies it, on the event loop's next turn.

**Vsync off does not make an idle window spin.** rinch draws a frame when something asks for one — a continuous `RenderSurface`, a running animation or transition, a texture source's new frame, input — and with vsync off those frames follow each other as fast as they can be drawn. A window with nothing to draw still sleeps.

The setting applies to the main window's surface and to the DevTools window's, and a window re-created by hide/show takes it. **The software renderer ignores it**: softbuffer has no present mode, so its pacing is the windowing system's (on Wayland a frame waits until the compositor has released the buffer it reuses). The `gpu-device-config` example's `RINCH_GPU_MODE=frames` shows the frame time with a vsync button; `RINCH_MEASURE_FRAMES=300` makes it print a summary of each mode and exit.
