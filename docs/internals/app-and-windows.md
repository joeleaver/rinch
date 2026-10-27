# App entry, fonts, windows and overlays

Read this page when your task touches: App builder, app fonts, Wayland app_id, transparent/borderless windows, overlay windows, hide/show, window control.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

## Application Entry Point

**`App` is the entry point** (issue #493). Every startup option is one method and
they all compose. Theme and component CSS load automatically when those features
are enabled, so components work out of the box even with no `.theme(...)`:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}

fn main() {
    App::new(app).title("My App").size(800, 600).run();
}
```

| Method | Purpose |
|--------|---------|
| `App::new(component)` | Start configuring around the root component |
| `.title(title)` / `.size(w, h)` | Window title and initial logical size |
| `.theme(ThemeProviderProps)` | Colors, radius, dark mode |
| `.fonts(&[AppFont])` | Typefaces the build carries in its own binary (issue #286) |
| `.menu(Vec<(&str, Menu)>)` | Native menu bar |
| `.window_props(WindowProps)` | Borderless, transparent, icon, `app_id`, `on_close_requested`, … |
| `.renderer(Renderer)` | `Auto` (GPU, falling back to software), `Gpu` or `Software`; `RINCH_RENDERER` overrides it |
| `.gpu_config(RinchGpuConfig)` / `.external_gpu(ExternalGpu)` | GPU device (`gpu` feature) |
| `.run()` | Start on desktop; runs until the event loop exits |
| `.run_android(android_app)` | Start on Android (`android` feature, Android target) |
| `rinch_web::AppExt::run()` | Start in the browser (`wasm32`, #1769) |

`.title()` / `.size()` are applied **over** `.window_props()` in either call
order, so an explicit title is never silently lost to a later `window_props`.
Everything else comes from `props`. Nothing runs until a terminal method.

```rust
fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };
    App::new(app)
        .title("My App")
        .size(800, 600)
        .theme(theme)
        .run();
}
```

**`App` also builds for `wasm32`** (#1769): the `app_builder` module and its
re-exports are gated `any(desktop, android on Android, target_arch = "wasm32")`,
and `rinch_web::AppExt::run` runs a builder chain in the browser, so a
cross-platform app writes one chain and imports
`#[cfg(target_arch = "wasm32")] use rinch_web::AppExt as _;`. rinch cannot call
rinch-web (rinch-web depends on rinch), so the web terminal method lives in
rinch-web and reads the builder through `App::into_web_parts` (doc-hidden,
pure; pinned by `app_builder::tests::web_parts_*`): the theme (installed as by
`mount`), the menus (`mount_with_menu_bar` when set, else `mount`) and the title
(`.title()`, else a `window_props` title that is not `WindowProps::default()`'s,
written to `document.title`; with neither the page keeps its own `<title>`).
`.menu()` is therefore no longer `desktop`-gated (an Android build has it too,
and `run_android` warns). The menu bar is a **compile-time** choice on the web:
`App<F, M = NoMenuBar>`, and on `wasm32` only `.menu()` returns
`App<F, WithMenuBar>` (elsewhere it returns `Self`, so desktop code is
unchanged); `AppExt` is implemented once per state, so an app with no menu does
not link `mount_with_menu_bar` (paint-web release wasm, pre-wasm-opt: a runtime
branch cost +93 KB over main). `.image_scheme()` registers when called, on both targets; `.size()` and the
rest of `.window_props()` have no web effect. `.fonts()` exists on every target and a web mount ignores the faces
(`WebParts::fonts`): `rinch::font` is ungated because `GenericFamily` is named
from `parlance` directly — the very type fontique re-exports, so nothing
changes for an `AppFont` user — and parlance builds for wasm with no
dependencies. The GPU and desktop-only methods do not exist there.
Pins: `rinch-web/tests/app_ext_1769.rs`, `app_ext_menu_1769.rs` (Chrome 155).

The seven `run_*` functions (`run`, `run_with_theme`, `run_with_menu`,
`run_with_window_props`, `run_with_window_props_and_menu`, `run_with_gpu_config`,
`run_with_external_device`) and the two Android ones (`run_android`,
`run_android_with_theme`) are now **`#[deprecated]` shims over `App`**, and each
deprecation note names its builder chain. Behaviour is unchanged with one
deliberate exception: unifying the startup paths also unified the Linux
**Wayland `app_id`**, which is now derived per application from the executable
name instead of the shared constant `"rinch-app"` (an explicit
`WindowProps::app_id` still wins). Every non-Linux platform is untouched. See
**Window identity on Wayland** below.
They exist because none of them could express its own combinations:
`run_with_menu` took no window props, `run_with_window_props` took no menu
without a second function, and neither could take whatever landed next.

### App-bundled fonts (`.fonts`)

`App::fonts(&[AppFont])` registers typefaces the build carries, before the
first layout pass — which is the whole reason it is a builder method and not a
function you call beforehand: a face that arrives after the first measurement
cannot un-measure it, so the first frame renders in a fallback and reflows.
Applies on Android too (the platform that needs it most), and it is the one
configuration method besides `.theme()` that does. A second `.fonts()` call
**appends**, unlike `.title()` / `.menu()`, which replace.

**A face is reachable by its own family name, and by nothing else, unless it
asks.** Registering `Newsreader.ttf` answers `font-family: Newsreader`
immediately. A *generic* — `serif`, `sans-serif`, `monospace`, `system-ui` — is
not the name of anything; it is a slot the platform fills, and a bundled face is
in one only by declaring it: `AppFont::serif/sans_serif/monospace(bytes)`, or a
struct literal over `generics` for the rest. Since essentially every real stack
ends in a generic, a face that claims none will usually not be picked at all.

**A claim is a prepend, not an append** (`rinch_dom::fonts::claim_generic_families`).
The #322 Android repair fills an empty generic slot by *appending* a platform
family into the collection's own list, at context construction, before any app
font can register — so an appended claim would sit behind it and silently lose,
on Android only. What was in the slot stays behind the claim, so a character the
declared face lacks still falls through to the platform's entry.

**`AppFont::script_fallback` is off by default and should stay off wherever the
platform has fonts.** fontique resolves a script's fallback from the app's own
list first and the platform's second — as an alternative, not a chain — so an
entry there *replaces* that script's platform fallback. Measured, not argued:
with it set on a Latin face, `漢字` in a stack headed by that face resolves to
the bundled face at **glyph 0** (`.notdef`), where without it the platform's CJK
face answers with real glyphs. It is on for `RinchApp::register_font_data`, the
wasm/embed front door, because wasm has no system fonts and a last-resort face
is the only thing that renders a character the author did not name a font for.

Guide: `docs/src/guide/fonts.md`. Fixtures live in `crates/rinch/assets/fonts/`
and `crates/rinch-dom/assets/fonts/` rather than under `examples/`, because
`cargo package` does not carry `examples/` and each crate has to be able to
compile its own test target.

### Window identity on Wayland

A window's `app_id` is how a Wayland compositor decides which `.desktop` entry,
icon and taskbar group a window belongs to. rinch derives it from the
executable's file name unless `WindowProps::app_id` says otherwise
(`resolved_app_id`, `shell/rinch_runtime.rs`), at **both** the initial
`create_window` and the `show_window` re-creation path that minimize-to-tray
restores through. On X11 an **explicit** `app_id` is the window's `WM_CLASS`
(instance and class) since #1519: a window has one platform-attributes box,
read by the backend the event loop runs on, and rinch used to hand X11 the
Wayland one, which X11 discards, so an explicit `app_id` never reached
`WM_CLASS`. Without one, X11 keeps winit's default (`argv[0]`'s file name,
`RESOURCE_NAME` for the instance), which can differ from the executable name
the Wayland `app_id` is derived from — deliberately unchanged.

It used to default to the constant `"rinch-app"` for every app that did not set
one. That is not merely a label: `install_wayland_icon` writes
`~/.local/share/applications/{app_id}.desktop` carrying `Name={app_id}`, so
under one shared constant the first rinch app to ship an icon supplied the dock
icon *and* the displayed name for every other rinch app on the machine, and
they grouped together as a single application. The whole name is used, not the
`file_stem` — a stem truncates at the last dot, so a reverse-DNS binary name
like `com.example.notes` would register as `com.example` and collide with its
own siblings.

The `#[component]` macro auto-injects `__scope: &mut RenderScope` as the first parameter, which is required by the `rsx!` macro. Components return a `NodeHandle`. You can also write `fn app(__scope: &mut RenderScope) -> NodeHandle` manually if preferred.


## Transparent Windows

Transparent/borderless windows are configured via `WindowProps { transparent: true, borderless: true }`. The software renderer presents premultiplied pixels through softbuffer. The GPU renderer (`shell::desktop::WgpuRenderer::try_new_transparent`, PR #1597) renders Vello over a transparent base and configures the surface with `surface_alpha_mode`: `Auto` for an opaque window (unchanged), and for a transparent one the first of `PreMultiplied`, `PostMultiplied`, `Inherit` the surface offers, else `Auto` (wgpu's `Auto` is `Opaque` on Vulkan, which drew every unpainted pixel black). **Vello 0.7 stores straight alpha** (`fine.wgsl` divides by alpha at its final store), so `present_step` converts on the way to the surface: a `PreMultiplied` (or `Inherit`) surface gets Vello's frame through the compositor's `fs_present_premultiply` pass, a straight copy of which showed 50% white over green as opaque white; a `PostMultiplied` one gets it as it is; an opaque window is copied as before (a Bgra8 surface goes through a `REPLACE` pass that writes every pixel, where it blended over an uncleared image). The layer compositor's `fs_ui` premultiplies Vello's colour before its `One, OneMinusSrcAlpha` blend, so translucent UI over a video or render-surface layer is no longer too bright, in opaque windows too, and the composited texture holds premultiplied pixels (copied to a premultiplied surface, made straight by `fs_present_unpremultiply` for a `PostMultiplied` one). Measured in a nested KWin on RADV (`Opaque | PreMultiplied` only; Mesa's Wayland and X11 WSI never offer `PostMultiplied`): `rgba(255,255,255,0.5)` over green reads `(128,228,128)` on GPU and software alike; pinned by the `#[ignore]`d `compositor::gpu_alpha_tests` (run with `--ignored gpu_alpha` on a host with a GPU) and `desktop::alpha_mode_tests`. **macOS** (not tested): Metal offers `Opaque | PostMultiplied`, and wgpu's `PostMultiplied` there is a `CAMetalLayer` with `opaque = false`, whose contents Core Animation composites as premultiplied, so `present_step` premultiplies for it on macOS only; a transparent window there used to be opaque and now blends. A layer's own pixels (a `RenderSurface` frame) are blitted as they are and read as premultiplied.

> **Windows caveat:** true per-pixel transparency on Windows needs an alpha-capable presentation path (PreMultiplied alpha + DX12 DirectComposition + `WS_EX_NOREDIRECTIONBITMAP`) that is **not currently wired** into `WgpuRenderer` — so `transparent: true` renders opaque on Windows today. Tracked in **issue #89**. (The old `TransparentWindowRenderer` held that path but was never constructed by the runtime and has been removed as dead code.)

Configure via `WindowProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
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

    App::new(app).window_props(window_props).run();
}
```

### Resize Handles for Borderless Windows

Borderless windows don't have native resize handles. Use `resize_inset` to enable custom resize handling:

- `resize_inset: Some(f32)` - Enables resize handles within **`inset`** px of the edges. The shell defaults it to `Some(8.0)` for any borderless resizable window.
- `resize_inset: None` (default before that fill-in) - Disables custom resize handles
- Only active when `borderless: true` AND `resizable: true`
- The cursor automatically changes to indicate resize direction on hover
- A **corner** is simply where two edge zones meet — an `inset` x `inset` square. It is *not* enlarged. `detect_resize_edge` used to carry a second `inset * 2` radius, but every guard reading it was implied by the `near_*` its arm already required, so the enlargement never took effect in any form the code has had; it has been removed rather than documented (#423). An app that wants bigger handles raises `resize_inset`.
- On Windows, transparent areas don't receive mouse events, so resize detection relies on the zone reaching into visible content

The `resize_inset` value should match your CSS content margin/padding to align the resize handles with the visible window edge. `BorderlessWindow` does **not** do this — its root is `100vw` x `100vh` with no margin — so on that component the zone genuinely overlaps your content, and the rule below is what keeps it usable.

**A visible scrollbar thumb wins an edge press (#399, #420).** Because the zone
overlaps content, the overlay scrollbar of a container flush with the window
edge is painted *entirely inside* it: at the default 8px inset the East zone is
`x > width - 8` and the thumb is drawn in `[width - 8, width - 2)`, so every
pixel of the thumb you could see used to be a resize handle. The rule now is
**what you can see, you can grab**, applied per axis. **Along** the track, a
press (and the hover cursor) at the thumb's extent goes to the scrollbar, while
the empty track past the thumb — and every edge with no bar on it — still
resizes. **Across** the bar the grab is edge-forgiving: the whole 16px hit
strip at thumb height goes to the scrollbar, *including* the 2px margin between
the thumb and the window edge — the way a browser's bar in a maximised window
is grabbable at the very last pixel. A **corner** never yields, so a diagonal
resize stays reachable however tall the thumb grows. The predicate is
`hit_testing::pointer_on_scrollbar_thumb`, which reads the thumb's along-axis
extent from the shared `rinch_dom::paint::scrollbar` geometry — the same
numbers paint draws with — over `find_scrollbar_hit`'s across-axis strip.

A browser has no equivalent conflict: its resize border lives in the window
frame *outside* the client area, so its scrollbars are grabbable right up to the
edge. A borderless window has no frame to put it in, so something has to give,
and the visible target is the one the user is aiming at.

**What true Windows transparency would require (see issue #89 — not yet wired):**
- DX12 backend with DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=DxgiFromVisual`)
- `CompositeAlphaMode::PreMultiplied`
- `WS_EX_NOREDIRECTIONBITMAP` window style
- Rgba8Unorm storage textures for Vello: no longer a patched wgpu (see "wgpu Fork" below, removed in #1955); Vello writes rinch's Rgba8Unorm intermediate, never the surface

### BorderlessWindow Component

The `BorderlessWindow` component provides a complete container for borderless/transparent windows with:
- Rounded corners
- Custom titlebar with drag support
- Window control buttons (minimize, maximize, close)
- Optional left/right custom sections in the titlebar
- Proper theming via CSS variables

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    // For custom left section (e.g., menu button)
    let menu_signal = Signal::new(false);
    let left_section: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            ActionIcon { onclick: move || menu_signal.update(|v| *v = !*v) }
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
| `title` | `String` | Window title displayed in titlebar (empty = not set) |
| `radius` | `String` | Corner radius: none, xs, sm, md, lg, xl (empty = default `md`) |
| `show_minimize` | `bool` | Show minimize button (default: true) |
| `show_maximize` | `bool` | Show maximize button (default: true) |
| `show_close` | `bool` | Show close button (default: true) |
| `left_section` | `Option<SectionRenderer>` | Custom content for left side of titlebar |
| `right_section` | `Option<SectionRenderer>` | Custom content before window controls |
| `on_minimize` | `Option<Callback>` | Callback for minimize button |
| `on_maximize` | `Option<Callback>` | Callback for maximize button |
| `on_close` | `Option<Callback>` | Callback for close button |

**With `WindowProps::menu_in_titlebar` (Linux in-app menu bar)** the menu row —
`left_section`, the branded `title` and the top-level labels — is an absolutely
positioned layer drawn over the titlebar, and an empty
`.rinch-borderlesswindow__menu-spacer` in the titlebar's flex row holds that
space open for the right section and the controls. The spacer is sized from the
**row's measured box** (the row's right edge minus the spacer's left edge), not
from a count of the labels (#529): the shell's
`MenuBarContext::spacer_width` is only the estimate the first layout uses, made
before any document exists; the spacer's write from the row's measured box is
laid out again before that frame is painted (`RinchApp::settle_bounds_writes`,
#1764 — it took one more frame until then). The subtraction is
made in the window root's **layout** frame — `NodeHandle::get_layout_bounds`
summed over root > layer > row and root > titlebar > spacer — because the
result is written as a CSS width (#1375); the two `bounds_signal`s are painted
boxes, scaled by any transform on the window or above it, and are read only to
re-run the effect. No shell builds the inline layout on `rinch-web`, so this is
a desktop path; a web one could not reuse
`get_layout_bounds`, which is `getBoundingClientRect`-based (painted) on that
backend. Pins: `app/inline_menu_spacer_529_tests.rs`.

### Window Control Functions

For custom window chrome (minimize/maximize/close buttons):

```rust
use rinch::prelude::*;

// In event handlers:
button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }

// Window visibility (for minimize-to-tray):
button { onclick: || hide_current_window(), "Hide to Tray" }
// From a tray menu callback:
MenuItem::new("Show").on_click(|| show_current_window())

// Pointer events pass through the window, or stop doing so (any thread):
set_click_through_current_window(true);

// Geometry, logical px, any thread (#1546); kept while hidden:
let pos: Option<(f64, f64)> = current_window_position(); // None on Wayland
let size: Option<(f64, f64)> = current_window_size();
set_current_window_position(300.0, 200.0); // while hidden: where it comes back
set_current_window_size(700.0, 200.0);
```

These functions are available in the prelude and work from onclick handlers.

**Hide / show keeps the geometry** (#1546, `shell/window_geometry.rs`).
`hide_window` still destroys the OS window (Wayland has no invisible toplevel;
macOS `set_visible(true)` is `makeKeyAndOrderFront`, which would activate a
non-activating panel). The normal (restore) geometry — outer position and
logical inner size — is noted while the window is live (`SavedGeometry::note`,
on creation, `Moved`, `SurfaceResized`, `ScaleFactorChanged`; not while it is
maximized or minimized — Windows reports a minimized window at -32000 — and
not for the never-shown sizing window of a hidden start), `record` at hide adds
whether it is maximized, and `main_window_attributes` creates the next window
with them
(`SavedGeometry::apply`: saved geometry, else `WindowProps::x`/`y` on a first
appearance only). The position is kept in desktop pixels (X11, Windows) or
points (macOS), the size in logical pixels; a maximized window comes back
maximized with its earlier normal geometry as the restore geometry. A
recorded position on no connected monitor at show (`on_some_monitor`) is
dropped. Wayland restores the size only; a position set while hidden there is
dropped. **On
X11 the frame position is read by rinch** (`overlay_window::outer_position`:
the client's root coordinates less `_NET_FRAME_EXTENTS`), because winit's
`outer_position` subtracts extents cached before the window manager wrote
them, which under KWin moved the window down by its title bar on every
hide/show; with no `_NET_FRAME_EXTENTS` published it falls back to winit's
answer. On Windows the content size is requested again after the menu bar is
re-attached (`SetMenu` takes it out of the client area). The published geometry the getters read is a `Mutex` updated on
creation, `Moved`, `SurfaceResized`, `ScaleFactorChanged` and hide. The native
menu bar is detached at hide (Windows: `remove_for_hwnd`, which also clears
`NATIVE_ACCELERATOR_ATTACHED`) and attached to the re-created window (#1533).
Pins: `shell::window_geometry::tests`, and the `#[ignore]`d live
`crates/rinch/tests/hide_show_geometry.rs` (a private X server; its `wm_`
tests inside a private headless KWin, `kwin_wayland --virtual --xwayland`).

### Window facts on every target (`rinch::window`, #1770)

`crates/rinch/src/window.rs`. `scale_factor`, `window_size` and
`window_focused` read three thread-local signals made `unowned` on first use.
Desktop and Android: `RinchApp::publishes_window_state` (set by
`RinchRuntime::new` for the main window and by Android's `run_component`; off
for the DevTools `RinchApp` and for `RinchContext`) makes `mount_component`
publish the mount size and scale, `set_device_pixel_ratio` publishes every
scale (so `show_window`'s re-creation and Android's `InitWindow` are
followed, not only `ScaleFactorChanged`), the `Resized` arm the size, and the
`WindowFocus` arm the focus **last**, after the app's own blur handling
(`window::publish`, one `batch`, `set_if_changed`); turning the flag on
publishes the scale and last size at once; `hide_window` publishes not
focused. The size is `Resized`'s logical `u32`s. Web (`cfg(wasm32)`, in
`rinch` itself, no rinch-web code): the first read installs window `resize`,
`focus`/`blur` (bubble phase, target must be the `Window`: a synthetic
bubbling element blur reaches `window` too) and a re-armed
`(resolution: Ndppx)` media query. `set_window_title` stores the
title in `window::STORED_TITLE` and, only while a shell's event loop runs
(`GLOBAL_PROXY` set), wakes it with `RinchNativeEvent::ApplyWindowTitle`;
`create_window` takes a stored title too, and `RinchRuntime::title` keeps it
for re-creation. Web: `document.title`. `open_url` first refuses non-URLs
(one-letter schemes included: `C:\…` would be run by `ShellExecuteW`) and
`javascript:`/`vbscript:`/`data:` (`ScriptScheme`; Chrome runs
`window.open("javascript:…")` with the opener's origin even after
`opener = null`). Then `launch` spawns `xdg-open`/`open`
with null stdio and reaps it on a thread; Windows `ShellExecuteW`
(`Win32_UI_Shell`); web `window.open(url, "_blank")` then `opener = null`
(`noopener` would make it answer null and hide a block). Pins:
`window::tests`, `app/window_facts_1770_tests.rs`,
`rinch-web/tests/window_facts_1770.rs` (Chrome 155, Firefox). The browser's
`resize` and resolution listeners have no fixture (a test cannot resize the
page or change its scale).

### Overlay windows (non-activating, click-through, skip-taskbar)

`WindowProps::non_activating`, `click_through` and `skip_taskbar` (#1519,
`shell/overlay_window.rs`) make a window that never takes keyboard focus or
activation — on creation, on `show_current_window`, or on a click — while its
DOM still takes pointer clicks. The decision is pure (`WindowSteps::plan`, per
`Backend`), executed as an ordered action list (`after_create_actions`, also
pure). A window without the three fields gets **no** action — no hit-test call
— and a tray re-show is not moved to `WindowProps::x/y` (only the first
appearance is placed; a re-show comes back where it was hidden, #1546). On Windows and X11 a window that needs a tweak
winit cannot make at creation is created hidden, adjusted, and then shown:

- **Windows**: the window procedure is replaced (`SetWindowLongPtrW`) so
  `WM_MOUSEACTIVATE` answers `MA_NOACTIVATE` and every `WM_STYLECHANGING` of
  `GWL_EXSTYLE` re-adds `WS_EX_NOACTIVATE` / `WS_EX_TOOLWINDOW` (minus
  `WS_EX_APPWINDOW`) — winit rewrites the whole extended style on every flag
  change (`set_cursor_hittest`, `set_visible`), so setting them once is not
  enough; `with_active(false)` gives the first show `SW_SHOWNOACTIVATE`.
- **macOS**: winit's `with_panel(true)` (a `NonactivatingPanel` `NSPanel`),
  then `setBecomesKeyOnlyIfNeeded:YES` and `setHidesOnDeactivate:NO` through
  `objc2`. **Not** created hidden: in winit-appkit `set_visible(true)` is
  `makeKeyAndOrderFront` whatever `active` says, which would make the panel
  key; only the creation-time show honours `with_active(false)`
  (`orderFront`). A tray re-show re-creates the window, so it takes the same
  path.
- **X11**: `WM_HINTS.input = False` and `_NET_WM_STATE_SKIP_TASKBAR` /
  `_SKIP_PAGER` written over a separate `x11rb` connection before the map,
  then a round trip; `always_on_top` is asked again after the map, since a
  level set on an unmapped window is a client message the WM ignores.
- **Wayland** (#1523, `shell/layer_shell.rs`): xdg-toplevel cannot refuse
  focus or a taskbar entry (KWin decides activation at map time and ignores
  winit's `with_active(false)`), so a `non_activating` window (or
  `WindowProps::wayland_layer_shell: Some(true)`; `Some(false)` opts out,
  `skip_taskbar` alone does not opt in) is a `wlr-layer-shell` surface instead,
  through the winit fork's `WindowAttributesWayland::with_layer_shell`
  (`zwlr_layer_surface_v1` on the window's `wl_surface`; its configure goes
  through the window state's ordinary resize path, so both renderers and
  input are unchanged). Layer `top`, keyboard none (on-demand when forced and
  not non-activating), anchored top-left with the #1546 position as margins
  (output-relative logical px; no position = unanchored, compositor-centred),
  `WindowProps::output` / `set_current_window_output` by connector name. The
  compositor has no move or resize for it, so `AppAction::DragWindow` /
  `DragResizeWindow` (`data-drag-window`, `resize_inset`,
  `start_window_drag` / `start_window_resize`) start a `layer_shell::Gesture`
  that the shell's `PointerMoved` intercept follows (consuming the moves) and
  any press or release ends, and that starts only while the primary button is
  held (so does a `start_window_*` call on any window); the fork commits a
  margin change at once and sends only requests the bound layer-shell version
  has. **A move is confirmed in the event stream** (PR #1597): after a
  margin commit the fork sends a `wl_display.sync` and, when its callback is
  done, pushes `WindowEvent::Moved` (the margins times the scale) in order
  with the pointer's events; `LayerState` (`layer_shell.rs`) reads each
  pointer position against the origin the last `Moved` reported
  (`LayerState::applied`), never against the position last set, and drops
  none. Reading against the last set position added each move again for
  every position already on its way (one burst read in one batch ran the
  window to 18380,11040); dropping positions until the compositor caught up
  put fast drags 30–590 px off. Measured in a nested KWin with fake input:
  60 moves at 0–16 ms apart all land exactly under the pointer, and nothing
  moves after the release. Pins: `layer_shell::tests::a_drag_lands_under_the_pointer_at_every_lag`,
  `rinch_runtime::layer_drag_tests`. The compositor closes a layer surface whose output went away: the
  shell's `CloseRequested` for a layer surface re-creates it
  (`layer_surface_closed`, output name dropped; with no output left it retries
  every second) instead of quitting. A
  mouse `PointerEntered` now queues a pointer move for every window, so a press
  with no move before it is judged where the pointer is. Where the compositor
  has no layer shell (GNOME) the fork makes an xdg-toplevel and one
  `tracing::warn!` says so (`AfterCreate::CheckLayerShell`);
  `current_window_surface_kind()` answers which. Tested live in a private
  `kwin_wayland --virtual` (no focus taken on map, click, hide/show; move,
  resize, output, hide/show geometry; software and GPU); the GNOME fallback
  is not (no such compositor on the host). `App::prefer_x11(true)`
  builds the event loop on X11 whenever `DISPLAY` is set (XWayland), and
  falls back to the default backend with a warning when that server refuses
  the connection.
- **Click-through** is winit's `set_cursor_hittest(false)` everywhere but X11,
  where winit's XFixes requests are refused (it never negotiates the XFixes
  version), so rinch sets an empty `ShapeInput` region itself over its own
  x11rb connection (`xfixes_query_version` first) and restores `None` to turn
  it off. It is kept by
  the runtime (`RinchRuntime::click_through`) across a hide/show re-creation;
  `set_click_through_current_window` posts `RinchNativeEvent::SetClickThrough`,
  which the shell applies without reaching the app.

**Level and workspaces** (#1549): `WindowProps::level: WindowLevel`
(`Normal` / `Floating` / `StatusBar` / `ScreenSaver`, macOS's names;
`always_on_top` is `Floating`, read through `WindowProps::effective_level()`)
and `visible_on_all_workspaces`. Every raised level is winit's `AlwaysOnTop`
at creation (`HWND_TOPMOST`, `_NET_WM_STATE_ABOVE`, `NSFloatingWindowLevel`),
and is re-asserted after an X11 map; macOS then `setLevel:`s 25 or 1000 for
`StatusBar` / `ScreenSaver`. All workspaces: macOS `canJoinAllSpaces |
fullScreenAuxiliary`; X11 `_NET_WM_DESKTOP = 0xFFFFFFFF` +
`_NET_WM_STATE_STICKY` before the map (so the window is created hidden);
**Windows and Wayland nothing**. On macOS `skip_taskbar` now adds `ignoresCycle
| stationary` (out of Cmd-` and Mission Control; Cmd-Tab is the app's
activation policy, #1528). Collection bits are merged into the window's own,
dropping the ones AppKit says exclude them (`overlay_window::mac::merge`). The
steps come from the props on every creation, so a hide/show re-creation (#1546)
keeps them. On native Wayland a level places a layer-shell window (#1523):
`Normal`/`Floating` in the `top` layer, `StatusBar`/`ScreenSaver` in
`overlay`; it does nothing for an xdg-toplevel.

Only the Linux paths have run; Windows and macOS are type-checked. Examples:
`examples/overlay-pill`, `examples/wayland-overlay`. Guide:
`docs/src/guide/windows.md#overlay-windows`.

**`on_close_requested` callback:** Intercept the window close button to hide instead of exit:

```rust
use std::sync::Arc;

let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Return false to cancel exit, true to proceed
    })),
    ..Default::default()
};
```

### wgpu Fork

**Removed in #1955** (wgpu 27 → 30, Vello 0.7 → 0.11). rinch builds against
crates.io wgpu 30; `[patch.crates-io]` in `Cargo.toml` patches only winit.

The fork (`joeleaver/wgpu-fork`, branch `rinch-patch`, one commit `54b7ce08`
touching `wgpu-core/src/instance.rs`) forced the `STORAGE` flags for
Rgba8Unorm/Rgba8Snorm/Bgra8Unorm in `Adapter::get_texture_format_features`,
for transparent Vello windows on DX12 + DirectComposition. Its upstream
counterpart, gfx-rs/wgpu#8908, was closed unmerged on 2026-01-28. It was dead
code for rinch: wgpu-core consults that function only for a device with
`TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` or a downlevel one, and rinch asks
for `Features::default()` (`shell::desktop`) on DX12, which is not downlevel.
Otherwise wgpu applies the format's guaranteed features, and Rgba8Unorm always
includes `STORAGE_BINDING` (WebGPU guarantees it). The only storage texture
rinch binds is its own Rgba8Unorm intermediate (`WgpuRenderer::create_render_texture`);
the window surface is configured `RENDER_ATTACHMENT | COPY_DST` and is only
copied or drawn into. (wgpu-core 30's `present.rs` still gives a surface
texture `RENDER_ATTACHMENT` alone, so Vello could not write the surface
directly with or without the patch.)

So downstream projects copy no wgpu patch any more; a wgpu upgrade is a
version bump. The DX12 transparent-window case still wants a check on a
Windows host after each upgrade.

**Apps that share rinch's `Device`** (`App::gpu_config`, `App::external_gpu`,
`gpu_handle`, `GpuTextureRegistrar`, the embed API) must build their wgpu
types from `rinch::wgpu`, which is now wgpu 30. The migration from wgpu 27:
`docs/src/guide/game-engine.md#migrating-from-wgpu-27-to-30`.

### Vello Fork

**Carried since #1955** (Vello 0.11). `[patch.crates-io]` in `Cargo.toml`
patches `vello_encoding` from https://github.com/joeleaver/vello-fork, branch
`rinch-patch`: one commit (`c6b2e61`) on the vello `v0.11.0` tag, touching only
`research/vello_encoding/src/{image_cache,resolve}.rs`. `vello` and
`vello_shaders` stay crates.io.

Vello 0.9 made the image atlas persistent: an image stays resident across
renders, keyed on its blob's id, and is uploaded again only when new or marked
dirty. Two parts of that broke rinch, and neither has a cheap rinch-side fix:

1. **A render of a scene with no late-bound resources wiped the atlas.**
   `Resolver::resolve` returned `Images::default()` (size 0) for a scene with
   no image, gradient or glyph run, so the renderer replaced the persistent
   atlas texture with a 1x1 one, and the next render with images replaced
   that with an empty full-size one, while the cache still held its images as
   uploaded: every resident image drew transparent until marked dirty. A
   solid-colour partial frame (a hover) was enough. The patch reports the
   current atlas size with nothing to upload.
2. **An image stayed resident for two renders after its last use**
   (`EVICT_AFTER_GENERATIONS = 2`), with no call to free it, so the 8192x8192
   atlas had to hold three scenes' images. Large or many images re-encoded per
   paint, live frames, and a resized `RenderSurface`'s old texture beside its
   new one (#1861 item 2) went undrawn. The patch sets it to 0: when an image
   does not fit, any image the current render does not draw gives up its space.

**Why 0 is safe.** A render's atlas writes are `Queue::write_texture` for CPU
images and `copy_texture_to_texture` for override textures, recorded in that
render's own command encoder ahead of its dispatches (`wgpu_engine.rs`,
`Command::WriteImage`). wgpu documents that `write_texture`/`write_buffer`
transfers "begin GPU execution only on the next call to `Queue::submit()`,
just before the explicitly submitted commands" (`Queue::write_buffer` docs,
wgpu 30), and a queue executes its submissions in order (WebGPU's queue
timeline), with wgpu-core's resource tracking placing the barriers between a
texture's uses. So the write for render N+1 runs after every command of
render N, including render N's reads of the space being reused. So reusing
the space of an image no earlier render will read again is the same hazard
Vello 0.7 already took when it rebuilt the whole atlas every render.

Pins: `compositor::gpu_alpha_tests::gpu_alpha_vello_image_survives_a_solid_only_render`
(1), `gpu_alpha_vello_repaints_keep_every_large_image` and
`render_surface::compositor_routing_tests::gpu_render_callback_atlas_share_holds_for_idle_surfaces`
(2); each fails with stock `vello_encoding`. The fork carries two unit tests in
`image_cache.rs`.

rinch also hands Vello one blob per image (`DecodedImage::blob`, and
`vello_painter::frame_blob` for a live frame's shared buffer), so an image
repainted unchanged stays resident and is not uploaded again, and keeps a
`RenderSurface` texture it draws into in place fresh with
`mark_override_image_dirty` (`SurfaceImages::sync`).

**Downstream apps copy the `vello_encoding` patch line**, as they used to copy
the wgpu one: Cargo patches are not transitive. On a Vello bump, rebase the
commit onto the new tag (or drop it if upstream behaves the same) and rerun the
three pins.

### Bounds signals on the web (#1781)

`NodeHandle::bounds_signal()` registers in `rinch_core::reactive::bounds`
and, for a new entry, calls `DomDocument::watch_bounds` (default no-op: desktop
refreshes every registered node after each layout, `RinchApp::refresh_bounds_signals`).
`WebDocument::watch_bounds` hands the element to `rinch-web/src/bounds.rs`, which
while anything is watched holds one `ResizeObserver` over the watched elements,
a `MutationObserver` on the document (subtree, child list, attributes, text),
a window `resize` listener, capturing document `scroll`, `load`,
`transitionend` and `animationend` listeners and a `document.fonts`
`loadingdone` listener. Each only schedules one `requestAnimationFrame`
refresh, which measures the connected watched elements with
`getBoundingClientRect()` under its own borrow and then publishes through
`update_bounds_signals` with no borrow held. The refresh drops elements whose
signals are all gone (`registered_bounds_nodes`) and, once none is left,
disconnects the observers and removes the listeners, so a page with no bounds
signal pays nothing; `WebDocument`'s `Drop` forgets its document's elements.
A disconnected element is not measured and its signal keeps its value.
`bounds_signal()` asks for the watch on **every** call (a repeat is one lookup):
a first call made while the document is borrowed cannot reach the backend, and
the next call starts it. Not seen: a `:hover` rule that moves a box without
resizing it, the middle frames of a transition that moves one, and a DOM write
or scroll inside a shadow root (an island mounted in one) — the
`MutationObserver` and the `scroll` listener are on `document`; the element's
own resize is still seen. A change reaches the signal one frame after it
happens (the observers report after layout; the refresh is the next frame),
where desktop's refresh lands in the frame that paints it. While anything is
watched, every DOM write in the page makes a mutation record and asks for a
refresh (at most one per frame). Pins:
`rinch-web/tests/bounds_signal_1781.rs` (Chrome 155, Firefox 155).

**On desktop a subscriber's write is laid out in the frame that woke it**
(#1764). `RinchApp::resolve_and_repaint` and `resize_layout` follow
`refresh_bounds_signals` with `settle_bounds_writes`: while the document has a
change its layout does not show (`dirty_nodes`, `styles_dirty`,
`DomDocument::layout_pending`), it lays out again and publishes the bounds
again, at most `BOUNDS_SETTLE_PASSES` (3) times (a `debug!` line says when the
cap is hit); a subscriber that never settles carries on next frame, as every
one did before. Each pass runs the frame's own post-layout work
(`RinchApp::after_layout`: owed scrolls, the editor's virtualization and
caret/selection/margin overlay pass, clamped-scroll events), so a subscriber's
write that re-wraps an editor leaves no overlay measured against the layout
before it. So a placement written
from a measurement (a dropdown kept inside the viewport, the #529 spacer) is
never painted at its old geometry. The cost is a layout per pass, only in a
frame where a subscriber wrote. Pins:
`rinch/src/app/dropdown_viewport_1764_tests.rs`,
`app/inline_menu_spacer_529_tests.rs`.

The registry is a `BTreeMap` keyed by `(doc_key, node_id)` (one document is
a range); the web state and both refreshes look rects up in hash maps, so no
step is quadratic in the number of watched nodes. Entries are shared per
`(doc_key, node_id, owner)` (`current_owner()`, `None`
for app lifetime): a repeated call from one owner returns the same signal and
registers nothing; another owner gets its own, because the signal belongs to
the scope that made it and is freed with it. `registered_bounds_nodes` lists
each node once. Pins: `rinch-core` `reactive::bounds::dedupe_tests_1781`,
`rinch` `app::layout_notification_tests::repeated_bounds_signal_calls_share_one_signal`.
