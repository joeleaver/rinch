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
| `.fonts(&[AppFont])` | Typefaces the build carries, `'static` or owned (`AppFont::from_data`; issues #286, #2101) |
| `.menu(Vec<(&str, Menu)>)` | Native menu bar |
| `.window_props(WindowProps)` | Borderless, transparent, icon, `app_id`, `on_close_requested`, … |
| `.renderer(Renderer)` | `Auto` (GPU, falling back to software), `Gpu` or `Software`; `RINCH_RENDERER` overrides it |
| `.gpu_config(RinchGpuConfig)` / `.external_gpu(ExternalGpu)` | GPU device (`gpu` feature) |
| `.vsync(bool)` | Start with vsync on (default) or off (#2095; `rinch::window::set_vsync` at run time) |
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

**`register_font_data` claims no generic** (#2102). It used to claim
`sans-serif` and `system-ui` for every face, so the last face registered won
them. It now fills only an *empty* slot of the face's own class
(`rinch_dom::fonts::fill_empty_generics`; the classification is on the
platform page, "Registered font data fills only empty generic slots"). The
registration is kept as a `PendingFont { font, fill_empty_generics }` so the
mount replays the same policy onto the document's context that the hit-test
context got.

**Font bytes are `FontData`, owned or `'static`** (#2101). `AppFont::data` is a
`FontData` (`&'static [u8]` or an `Arc<dyn AsRef<[u8]> + Send + Sync>`, from
`Vec<u8>`, `Box<[u8]>`, `Arc<[u8]>`, `Arc<Vec<u8>>` or `peniko::Blob<u8>`), so
`AppFont` is `Clone`, not `Copy`; the `const` constructors still take
`&'static [u8]`, and `AppFont::from_data` / `with_generics` cover owned bytes.
Each font context's `Blob` is built over a clone of the one `Arc`, so the
references are exactly: the app's `pending_fonts`, the hit-test context and
the document's context (plus whatever a live layout or scene holds). All go
with the app; `owned_font_bytes_are_released_when_the_app_is_dropped` and the
embed's `owned_font_bytes_are_released_with_the_context` pin the count back to
1 after a mount, layout and paint. **No unregistering:** fontique has
`Collection::unregister_font(family, width, style, weight)`, but the face is
also in the document's shaped layouts, text caches and glyph caches; dropping
the app (or `RinchContext`) is the only release.

Guide: `docs/src/guide/fonts.md`. Fixtures live in `crates/rinch/assets/fonts/`
and `crates/rinch-dom/assets/fonts/` rather than under `examples/`, because
`cargo package` does not carry `examples/` and each crate has to be able to
compile its own test target.

### Window identity on Wayland

A window's `app_id` is how a Wayland compositor decides which `.desktop` entry,
icon and taskbar group a window belongs to. rinch derives it from the
executable's file name unless `WindowProps::app_id` says otherwise
(`resolved_app_id`, `shell/rinch_runtime/app_identity.rs`), at **both** the initial
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
// What to persist and give back as WindowProps x/y, width/height, maximized:
let restore: Option<WindowRestore> = current_window_restore();
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
recorded position out of reach at show is dropped (`begin_appearance`). Wayland
restores the size only; a position set while hidden there is dropped.

**Reachable positions** (`window_geometry::reachable`, #1884). A position —
recorded at hide, or `WindowProps::x`/`y` — is kept only where a *grip* of the
window's top is on the desktop: `GRIP` = 64 x 12 logical pixels (each capped at
the window's width / height), anywhere in its top `GRIP_STRIP` = 24 (a
Windows outer frame a few pixels above a monitor leaves the title bar below
it). "On the desktop" is the union of the monitors (`grip_on_desktop`: x-slabs
between monitor edges, a row band starting at the strip's top or a monitor's
top edge), so a window across two adjacent monitors counts and a gap between
monitors does not. The grip is sized in the scale of a monitor the strip
overlaps. No monitors listed: kept.

**`WindowProps::x`/`y` on a mixed-scale desktop** (`place_requested`). A
logical desktop position is the outer position over the scale of the monitor
the window is on (what the getters publish), but winit converts a
`Position::Logical` with a guessed scale — X11 the monitor under the pointer
(`winit-x11` `window.rs`, `guessed_monitor`), Windows the scale the new window
starts with — so a position next to a scale-2 primary could be created at
twice its coordinates, on no monitor. `place_requested` tries each monitor as
the one the position is on (convert with its scale; the strip must overlap it
and be reachable) and gives winit the resulting `Position::Physical`. Where
two monitors both fit (a logical position is ambiguous on such a desktop) the
first in winit's order wins, unless `WindowProps::monitor` names one: the
monitors of that name are tried first (two of one name: the first on which the
window is reachable), then all of them. The name is what winit's
`MonitorHandle::name()` answers — `connected_monitors` stores it, and the
published `monitor` / `normal_monitor` come from the window's
`current_monitor()` at each reading (`monitor_name`; the monitor whose scale
the window has, so its scale is the one the logical position was computed
with), or from the monitor `place_requested` chose at a first appearance.
X11 names an output (`DP-1`), Windows a device (`\\.\DISPLAY1`), macOS
`Monitor #<model number>` (not unique for two of one model). A position the
app sets while hidden, a layer surface's, and Wayland's absent one carry no
name. macOS keeps
the logical position: `connected_monitors` gives its monitors in points at
scale 1, the unit AppKit positions in.

**Each monitor's rectangle is the platform's own** (`native_monitor_rect`,
#1884). winit gives a monitor's position and its current video mode, and on X11
the mode is the RandR mode, unrotated (`winit-x11` `util/randr.rs`), so a
1080x1920 portrait monitor read as 1920x1080 and a position in its lower half
was refused. On X11 rinch reads the CRTC's rectangle over RandR
(`overlay_window::x11_crtc_rect`; winit's `native_id` there is the CRTC id,
the same CRTC winit builds the monitor from); on Windows `GetMonitorInfoW`'s
`rcMonitor` (physical desktop pixels; winit's `native_id` is the
`HMONITOR`); on macOS `CGDisplayBounds` (points; `native_id` is the
`CGDirectDisplayID`), which follow the display's rotation. Where it cannot be
read (an X error, Wayland) the video mode is used as before. Pinned by
`a_portrait_monitor_takes_its_native_rectangle` and the `#[ignore]`d
`overlay_window::live_crtc` test against a live X server.

**First appearance and `current_window_restore`** (#1884). `begin_appearance`
runs before both `create_window` (visible) and `show_window`: on the first
appearance — a creation, or the first show after a hidden start —
`publish_created` publishes the geometry the window is created with as the
normal (restore) geometry, since a window created maximized has none to read.
`current_window_restore()` answers `normal_position` / `normal_size` and
`maximized` from the published geometry. **X11 maximizes late**: the window
manager applies `with_maximized` after creation, a `SurfaceResized` can come
while `is_maximized` (read off `_NET_WM_STATE`, which raises no winit event)
still answers false, and that reading would make the full-screen size the
normal geometry. `MaximizeWait` (set from the creation attributes; not for a
non-resizable window, which winit gives equal minimum and maximum sizes and
window managers refuse to maximize, nor a layer surface) takes not-maximized
readings as maximized for `MAXIMIZE_GRACE` (2 s) after the creation, while
the window is at its created size or at the first other size it reached (the
maximize). It ends — every reading believed — at the first reading that says
maximized, at the end of the grace, or at a reading at yet another size or
back at the created one after that first other size (an un-maximize, to the
created size or to a smaller one a window manager chose because the created
size does not fit the screen; or a resize by the person). Readings come only
with events (`Moved`, `SurfaceResized`, a scale change, creation, hide), so a
window the window manager never maximizes (tiling, a WM rule, none), moved
inside the grace and then left alone, would keep what the last reading in the
grace said: maximized, at the creation position. So the shell reads the window
once more when the grace ends — `MaximizeWait::deadline` is a timed wake in
`tick_caret_blink`, beside the caret blink, the `title` tooltip and the file
drop's — and at the main window's `CloseRequested`, before the app's
`on_close_requested` reads the restore (`settle_maximize_wait`). That reading
is believed and the wait is `Done`, which has no deadline, so the wake is not
armed again. Residue: a maximize done in two resizes of different sizes before
the property is set ends the wait at the second. Pins:
`window_geometry::tests`, `rinch_runtime::main_window::window_restore_tests`. **On
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
`crates/rinch/tests/hide_show_geometry.rs` (a windowed Xwayland on the
session's own headless KWin, or an Xvfb where one is installed, and for its
`wm_` tests a headless KWin with Xwayland, each inside
`scripts/isolated-session.sh`; the recipes are in the file).

### Window facts on every target (`rinch::window`, #1770)

`crates/rinch/src/window.rs`. `scale_factor`, `window_size` and
`window_focused` read three thread-local signals made `unowned` on first use.
Desktop and Android: `RinchApp::publishes_window_state` (set by
`RinchRuntime::new` for the main window and by Android's `run_component`; off
for `RinchContext`) makes `mount_component`
publish the mount size and scale, `set_device_pixel_ratio` publishes every
scale (so `show_window`'s re-creation and Android's `InitWindow` are
followed, not only `ScaleFactorChanged`), the `Resized` arm the size, and the
`WindowFocus` arm the focus **last**, after the app's own blur handling
(`window::publish`, one `batch`, `set_if_changed`); turning the flag on
publishes the scale and last size at once; `hide_window` publishes not
focused. **A desktop window other than the main one publishes facts of its
own** (#2416; `RinchApp::set_publishes_own_window_state`, set for DevTools):
`window::publish_for_window(doc, …)` writes signals kept per document
(`window::WINDOWS`, each window's owned by a `Scope`, disposed by
`forget_window` when it closes), and a reader resolves
`current_dispatching_doc()`: that document's signals when it has its own,
the thread's (the main window's) otherwise — outside any document, on
another thread, or in the main window's document. The main window's
publishing is unchanged. The size is `Resized`'s logical `u32`s. Web (`cfg(wasm32)`, in
`rinch` itself, no rinch-web code): the first read installs window `resize`,
`focus`/`blur` (bubble phase, target must be the `Window`: a synthetic
bubbling element blur reaches `window` too) and a re-armed
`(resolution: Ndppx)` media query. `set_window_title` stores the
title in `window::STORED_TITLE` and, only while a shell's event loop runs
(`GLOBAL_PROXY` set), wakes it with `RinchNativeEvent::ApplyWindowTitle`;
`create_window` takes a stored title too, and `ShellWindow::title` keeps it
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

### Outputs (`rinch::outputs`, #2414)

Public API: `crates/rinch/src/outputs.rs` — `Output` (name, logical
position and size, scale), `outputs()` and `output_tracking()` reading two
thread-local `unowned` signals made on first use (the `rinch::window`
pattern), `output_changes(before, after)` (pure: removals in `before`'s
order, then additions and changes in `after`'s, paired by name, same-named
ones in order, an unnamed one only while equal), and `publish` (one `batch`,
`set_if_changed`).

Shell: `shell/outputs.rs`. `MonitorReading::of` reads each winit monitor
once (name, position, mode, scale, `native_monitor_rect`, and on Wayland
`xdg_output`'s logical rectangle through the fork's
`MonitorHandleExtWayland`). The placement list `connected_monitors`
(`MonitorReading::rect`, unchanged from #1884: desktop pixels, or points at
scale 1 on macOS) and the public list (`MonitorReading::output`) come from
the same reading, so `Output::name` is the string `WindowProps::monitor` is
matched against. `output` converts to logical pixels: X11/Windows the
rectangle over its scale; macOS the points as they are, with the backing
scale; Wayland the `xdg_output` rectangle, at the scale mode-long-side over
logical-long-side (rotation-proof: winit's mode is unrotated, the logical size
rotated), rounded to 1/120 (the fractional-scale protocol's unit; KWin rounds
logical sizes, 1366 / 1.25 = 1093); with no `xdg_output`, winit's reading over
its integer scale. `tracking_of`: Wayland and X11 `Live`, the rest
`Snapshot`.

The runtime publishes at the top of `can_create_surfaces` (before the window
is created and mounted, so the first render reads the list) and in
`ApplicationHandler::monitors_changed`, which also asks the window for a
frame when a signal changed or layout is pending (`proxy_wake_up`'s rule).
Nothing else publishes: a `Snapshot` list never moves, rather than moving
whenever some unrelated path happened to read the monitors.

Windows and macOS (in issue #2414): winit-win32's thread event-target window is
a hidden top-level, so it receives the `WM_DISPLAYCHANGE` broadcast; a scale
change may need `WM_SETTINGCHANGE` too. winit-appkit would observe
`NSApplicationDidChangeScreenParametersNotification`. Neither is in the fork.

Pins: `outputs::tests` (the diff, publishing wakes once), `shell::outputs::tests`
(units per platform, fractional, rotated and rounded Wayland scales, the
`xdg_output`-less fallback, `an_outputs_name_is_what_window_props_monitor_and_output_accept`,
tracking per backend). Example: `examples/outputs` (prints `OUTPUTS …` and
`CHANGE …` lines).

**Live test** (a nested KWin; one physical output is enough). KWin's virtual
backend makes `--output-count` outputs, and `kscreen-doctor` disables (the
`wl_output` global and the RandR output are withdrawn), enables, rescales and
rotates them while the example runs, once on Wayland and once under Xwayland.
Then `examples/wayland-bar` (`BAR_SCRIPT=outputs`), a layer surface, is moved
from one output to the other with `set_current_window_layer`, and one of its
outputs is disabled under it.

It runs inside `scripts/isolated-session.sh` (#2429), which gives it a
private runtime and XDG directories, none of the desktop's display or bus
variables, a `dbus-daemon` with no service activation, a headless KWin, and
one process group that is ended and checked for leftovers. Never run it any
other way: on 2026-11-10 an earlier, unisolated version of this procedure
started the desktop portals and the secret service with the real session's
runtime directory, Wayland display and home; one of them crashed 391 times,
and orphans were left running. With activation off, KScreen's backend (which
`kscreen-doctor` talks to over the bus) is started by the test itself, inside
the session; its path is the `Exec=` of
`/usr/share/dbus-1/services/org.kde.kscreen.service` (Ubuntu's below).

```bash
#!/bin/bash
# outputs-live.sh OUTPUTS_BINARY BAR_BINARY: the live test of #2414, run as
#   scripts/isolated-session.sh --kwin --xwayland --output-count 2 \
#     --width 1920 --height 1080 -- ./outputs-live.sh \
#     target/release/outputs target/release/wayland-bar
set -u
T=$RINCH_SESSION_DIR BIN=$(realpath "$1") BAR=$(realpath "$2")
export QT_QPA_PLATFORM=wayland

/usr/lib/x86_64-linux-gnu/libexec/kf6/kscreen_backend_launcher > "$T/kscreen.log" 2>&1 &
"$BIN" >> "$T/wayland.log" 2>&1 &
env -u WAYLAND_DISPLAY "$BIN" >> "$T/x11.log" 2>&1 &   # Xwayland
sleep 3

# Each step is marked in every log, then applied, then given time to arrive.
step() {
  echo "== $*" | tee -a "$T/wayland.log" "$T/x11.log" "$T/bar.log" > /dev/null
  kscreen-doctor "$@" >> "$T/steps.log" 2>&1
  sleep 4
}
for s in -o output.Virtual-1.disable output.Virtual-1.enable \
    output.Virtual-0.scale.1.5 output.Virtual-1.rotation.left \
    output.Virtual-0.scale.1 output.Virtual-1.rotation.none \
    output.Virtual-1.scale.2; do
  step "$s"
done

# The bar: on Virtual-0 (1920 wide), moved to the next output (Virtual-1,
# 960 wide at scale 2) every other 6 s step; Virtual-1 disabled under it
# 1.5 s after its first move there, enabled again 9 s later; the tour ends
# after eight steps.
echo "== bar" | tee -a "$T/wayland.log" "$T/x11.log" "$T/bar.log" > /dev/null
BAR_OUTPUT=Virtual-0 BAR_SCRIPT=outputs BAR_STEP_MS=6000 "$BAR" >> "$T/bar.log" 2>&1 &
bar=$!
until grep -q 'BAR move to Virtual-1' "$T/bar.log"; do sleep 0.1; done
sleep 1.5
step output.Virtual-1.disable
sleep 5
step output.Virtual-1.enable
wait "$bar"

for log in wayland x11 bar; do
  echo "### $log"
  grep -E '^(OUTPUTS|CHANGE|BAR|==)|layer surface|no wl_output' "$T/$log.log"
done
```

Before and after a run, `coredumpctl list` should show no new entry and
`~/.config/kwinoutputconfig.json` the same modification time.

Measured on KWin 6.6.6 (both instances at once), last through
`scripts/isolated-session.sh` exactly as above (2026-11-10). In the earlier
written-out session the nested KWin's request for the desktop portal failed
("not provided by any .service files"), so nothing was started on the bus,
and its `kwinoutputconfig.json` was written in the session's own `config`.
After each run nothing of the session was left, and the real
`kwinoutputconfig.json`'s modification time and `coredumpctl list` (for the
session's programs) were as before. Each step printed one
`OUTPUTS` list and the `CHANGE` it implies, and the window's table followed
(checked with the MCP `screenshot`). Wayland, from two 1920x1080 outputs:
disable gives `removed Virtual-1`, enable `added Virtual-1`, scale 1.5
`changed Virtual-0 … -> 1280x720 … scale 1.5`, rotation left
`changed Virtual-1 … -> 1080x1920`. Xwayland reports the same steps in X's
terms: one global scale (`Xft.dpi`, so every output's scale moves together)
and, while KWin rescales Xwayland, an intermediate RandR list in the old
scale followed by the `RESOURCE_MANAGER` change that settles it.

The bar (2026-11-10, with fork PR #2's head carrying #2413 too): on
`Virtual-0` it is 1920 wide; moved with `set_current_window_layer` to
`Virtual-1` (960 x 540 at scale 2) it is 960 wide, and back 1920, at each
step. Disabled while the bar was on it, `Virtual-1` took the surface with it:
rinch logged "the compositor closed the layer surface (its output went away);
making it again", and the bar came back on `Virtual-0`, 1920 wide; after
`Virtual-1` was enabled again the tour moved it there and back as before. A
move asked for an output that had just gone (an earlier run, where the move
raced the disable) is placed by the compositor, with winit-fork's warning "no
wl_output named …".

### Pointer lock and raw motion (#2092)

State and API: `crates/rinch/src/pointer_lock.rs` (private module, re-exported
through `rinch::window`), per thread. `HELD` is the lock asked for or held (its
`PointerLockTarget` — `Window` or `Element(NodeHandle)` —, the
`RenderSurface` id that asked, if one did, and the asking app's `doc`: the
element's `doc_key`, else `current_dispatching_doc`, `None` from outside any
dispatch); `MODE` is the granted mode, mirrored
into a lazily made `unowned` signal that `pointer_lock()` reads; `HANDLERS` are
the `on_pointer_motion` handlers (removed by `on_cleanup` of the ambient owner;
with no owner, by dropping the `#[must_use]` `PointerMotionSubscription` or
`cancel`).
`pointer_lock_changed(Some)` with nothing in `HELD` changes nothing and answers
`false` (it used to adopt the grant as a window lock, which turned a withdrawn
request into a lock). The grab is a `PointerLockHost`'s
(`set_pointer_lock_host`; a swap releases a held lock through the old host
first). `lock` answers `Ok(Some(mode))` (granted now: desktop), `Ok(None)`
(pending: the browser, confirmed by `pointer_lock_changed`) or `Err`; a host is
called out of its cell (`with_host`), so it may call back in.

Release paths, all through `release()` (host `unlock` once, mode `None`):
`exit_pointer_lock`; `RinchApp::handle_event`'s first step — `prune()` (an
`Element` target that is no longer `NodeHandle::is_connected`, checked on every
event, `AboutToWait` included), `WindowFocus(false)` (`window_blurred`), and
a `KeyDown(Escape)` while a lock is granted (`escape(doc_key, fresh)`: released
and the event returned with only `RequestRedraw`, so DnD's Escape, the dismiss
stack and every key handler never see it). Both act only on a lock whose
`doc` is the handling app's (or `None`): several `RinchApp`s share the
thread's state (embedded contexts), and another's blur or Escape used to end
this one's. `ESCAPE_HELD` keeps that Escape's later presses that are not
`KeyRepeat::Fresh` (repeats, and `Unknown`) consumed until its `KeyUp`
(`escape_released`), a fresh press or a blur — a held Escape's repeats used to
reach the dismiss stack and close a modal; `unregister_render_surface` (`surface_gone`, at the surface's unmount,
before any event); `hide_window` (`set_pointer_lock_host(None)`, before the
window is dropped). The host's own loss (`pointer_lock_changed(None)`) clears
without calling `unlock`.

Desktop shell (`shell/pointer_lock.rs`): `WindowPointerLock` holds the main
window as a `Weak<dyn Window>`, installed by `create_window` and `show_window`
(`install_pointer_lock_host`). `grab` refuses an unfocused window
(`PointerLockError::NotFocused`, `Window::has_focus`): X11's `Confined` is an
active `XGrabPointer` the server grants an unfocused window, which then caught
and hid the pointer over a window a click could not focus, with Escape going
elsewhere and no focus loss to end it. Then it tries `CursorGrabMode::Locked`, then
`Confined` (winit-x11 has no lock; it reports `NotSupported`), then hides the
cursor; `ungrab` is `None` + visible. `device_event` sums
`DeviceEvent::PointerMotion` into `pending_motion` (`MotionSum`) only while
`wants_motion()` (a lock granted or a handler registered), and
`flush_pointer_motion` hands it to the app as one `PlatformEvent::PointerMotion`
at the start of each `window_event` and in `about_to_wait`. While a lock is
granted the `PointerMoved` arm keeps the move in `held_pointer_move` instead
of queueing it, so the document's pointer stays where the lock found it
(Confined on X11 still moves the cursor); `restore_held_pointer_move`, in
`about_to_wait`, queues it once the lock has ended (unless a newer move
waits), so hover is not left stale, and the KeyboardInput arm skips `try_menu_shortcut` for Escape.
`RinchApp` dispatches `PointerMotion` through `dispatch_pointer_motion` (the
surface holding a granted lock hears `SurfaceEvent::PointerMotion`, then the
handlers, in one batch, untracked) and resolves layout when a handler dirtied
it. winit delivers device events only while the window is focused
(`DeviceEvents::WhenFocused`, the default; rinch does not change it). Measured
live in a headless KWin with Xwayland in an isolated session
(`scripts/isolated-session.sh --kwin --xwayland -- env POINTER_LOCK_LOG=1
POINTER_LOCK_AUTO=1 timeout 5 target/release/pointer-lock`, and the same with
`env -u WAYLAND_DISPLAY` for Xwayland): Wayland grants `Locked`, Xwayland `Confined`, and
under Xwayland XTest's relative moves arrive as raw motion and Escape releases.
Each raw motion arrives once under the X11 grab since the winit-fork fix for #2114
(see "winit Fork" below); before it, the server's second copy doubled the deltas.

Web (`crates/rinch-web/src/pointer_lock.rs`, installed by
`ensure_event_delegation`): `BrowserPointerLock` resolves the target
(`document.documentElement`, or `web_document::element_of`), answers
`Some(Locked)` at once when `pointerLockElement` already is it, else calls
`requestPointerLock` through `Reflect` (so the promise Chrome and Firefox
return can be caught) and answers pending; either way it records the element
in `REQUESTED` (and whether it is granted), which `unlock` clears.
`pointerlockchange` (`on_change`) marks the request granted when the browser
locked the element asked for; a lock on any other element, or after the
request was withdrawn, is given back (`exitPointerLock`, rinch told `None`);
a lock lost after its grant is reported; an unlock while a request is pending
is waited through. `pointerlockerror` (`on_error`) ends a pending request and
gives back whatever the browser still holds (a refused move from A to B used
to leave A locked and labelled B's). While a lock is granted the canvas's
`pointermove` sends the surface nothing, as on the desktop; a capture-phase
passive `mousemove` on the document feeds `movementX/Y` to
`dispatch_pointer_motion`. The browser ends a lock itself on Escape, blur and
the locked element's removal. Measured (headless Chrome 155, a CDP click): the
click on the canvas locks it, CDP moves arrive as `movementX/Y`, and
`exit_pointer_lock` and the surface's unmount each end it; a CDP Escape does
not (it skips the browser's own Escape handling), so the browser's Escape has
no automated test. Firefox (Playwright's build, through geckodriver): the
synthetic half and the refusal of an ungestured request.

Element liveness is `NodeHandle::is_connected`, whose id carries a slot
generation; the desktop slab reuses slots at generation 0 until #723's second
half, so a handle to a freed node whose slot went to a new attached node still
reads as connected there.

Pins: `pointer_lock::tests` (incl. an unowned subscription dropped, a grant
with no request), `shell::pointer_lock::tests` (lock vs confine, unfocused
refused, cursor), `shell::rinch_runtime::actions::pointer_lock_2092_tests` (device motion summed
and gated, hide), `app/pointer_lock_2092_tests.rs` (motion to the surface and
the handlers, focus loss, Escape consumed ahead of the dismiss stack and an app
key listener, its repeats, another app's blur and Escape, a surface's and an
element's unmount, confine reported), the runtime's held move restored,
`rinch-web/tests/pointer_lock_2092.rs` (Chrome and Firefox),
`rinch-web/tests/pointer_lock_real_2092.rs` (trusted input,
`run_real_keys.sh`, which `real_keys.mjs`'s `RINCH-REAL-MOVE` serves).
Live: `examples/pointer-lock` with `POINTER_LOCK_LOG=1 POINTER_LOCK_AUTO=1`.

### Activation Tokens

`rinch::windows::request_activation_token(callback)` (#2501, `activation.rs`)
asks the compositor for an xdg-activation token for the current window, to
hand to a program the app starts (`XDG_ACTIVATION_TOKEN`, or the
`activation-token` platform data of a D-Bus activation), so that program's
first window gets the focus. Wayland only: winit's X11 backend answers the
same request with a startup-notification ID, a different mechanism this API
does not offer.

- **Only from a user action.** `RinchApp::handle_event` holds
  `app::user_input::enter`'s guard while it dispatches a `KeyDown`, a
  `MouseDown` or a `MouseUp`; a request made with none (a timer, an effect, a
  move, a wheel, a later turn, another thread) is answered
  `ActivationTokenError::NotFromInput`. The release counts, as a browser
  grants user activation on `pointerup`: rinch fires an `onclick` on the
  release for an element with a pending drag whose threshold was never
  crossed (`event_dispatch.rs`'s `MouseUp` arm), and `onmouseup` is a
  deliberate act. The fork takes the seat and serial of the window's latest
  input itself (see "winit Fork"): during a key's dispatch the key press,
  during a button's the button's press, since the fork notes a press and not
  its release (Mutter accepts only the press's serial). Either is a serial
  the compositor sent the client for that click, which is what wlroots and
  KWin check, so the fork needs no change for releases.
- **The callback stays on the UI thread.** It is parked
  (`rinch_core::park_main_callback`) under the ambient owner and the asking
  document, and only its `MainCallbackId` travels in
  `RinchNativeEvent::RequestActivationToken`, whose queue is `Send`. A
  component unmounted before the answer is never called (the parked
  callback's owner is checked at resume, the rule of
  `reactive-rendering.md` #141); otherwise it is called exactly once, untracked
  (`untracked_handler`), and always in a later turn: every answer goes through
  `activation::answer`, which queues the resume on the main-thread queue.
- **The window is the asking document's.** The native event carries the
  sending document (#2416 3b), so `role_for_doc` gives it to that document's
  window, which calls winit's `request_activation_token`.
- **Pending requests are kept by winit's `AsyncRequestSerial`**, in
  `shell::activation_tokens`, with the asking window's `facts_key`; not on
  the `ShellWindow` by `WindowId`. winit delivers
  `WindowEvent::ActivationTokenDone` even after the window that asked was
  dropped, under that window's id, and a window made later can have the same
  id (a Wayland window's id is its surface object's address), so
  `RinchRuntime::window_event` takes the event before it looks the window up,
  and the serial alone finds the request.
- **A window that goes ends its requests.** Whether a compositor answers for
  a surface that is gone is its own choice (it may answer with a token that
  grants no focus, or never), so `ShellWindow::hide_window` and the DevTools
  window's close call `activation_tokens::window_gone`, which answers each of
  the window's requests `WindowClosed`; an answer that comes after is dropped.
  A request whose window is already hidden when the shell handles it is
  answered `WindowClosed` too. Hence the rule the API states: an app that
  hides after starting something hides in the callback.
- **No refusal error.** The protocol always answers with a token; what it is
  worth (focus, or only urgency) is decided when the child uses it.

Tests: `app/user_input.rs` (what counts as a user action),
`shell/activation_tokens.rs` (an answer by serial, once; a window that goes
ends its own requests and drops their late answers),
`shell/rinch_runtime/activation.rs` (refused outside a user action, once and in a
later turn; inside a key press it reaches the shell as the asking document's
request; a drag source's `onclick`, which comes on the release, asks and is
not refused; a move handler is refused; an unmounted component is never
called). Live, on a compositor:
`tests/activation_token_2501/`.

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
  path. A click on the inactive panel acts at once: winit 0.31.0-beta.3
  removed `with_accepts_first_mouse` (which rinch set to `true`) and always
  accepts the first mouse, tagging that press `is_macos_activation_click`,
  which rinch does not filter (#1962).
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
  input are unchanged; see "winit Fork" below). Layer `top`, keyboard none (on-demand when forced and
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
  `rinch_runtime::main_window::layer_drag_tests`. The compositor closes a layer surface whose output went away: the
  shell's `CloseRequested` for a layer surface re-creates it
  (`layer_surface_closed`, output name dropped; with no output left it retries
  every second) instead of quitting. A
  mouse `PointerEntered` now queues a pointer move for every window, so a press
  with no move before it is judged where the pointer is. Where the compositor
  has no layer shell (GNOME) the fork makes an xdg-toplevel and one
  `tracing::warn!` says so (`AfterCreate::CheckLayerShell`);
  `current_window_surface_kind()` answers which. Tested live in a headless
  KWin in an isolated session (`scripts/isolated-session.sh --kwin -- env
  OVERLAY_SCRIPT=1 target/release/wayland-overlay` runs the scripted part;
  no focus taken on map, click, hide/show; move,
  resize, output, hide/show geometry; software and GPU); the GNOME fallback
  is not (no such compositor on the host). An app's own configuration of
  the role (`WindowProps::wayland_layer`, #2413) is the next section.
  `App::prefer_x11(true)`
  builds the event loop on X11 whenever `DISPLAY` is set (XWayland), and
  falls back to the default backend with a warning when that server refuses
  the connection.
- **Click-through** is winit's `set_cursor_hittest(false)` everywhere but X11,
  where winit's XFixes requests are refused (it never negotiates the XFixes
  version), so rinch sets an empty `ShapeInput` region itself over its own
  x11rb connection (`xfixes_query_version` first) and restores `None` to turn
  it off. It is kept by
  the runtime (`ShellWindow::click_through`) across a hide/show re-creation;
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

### Layer surfaces of the app's own (`WindowProps::wayland_layer`, #2413)

The overlay above is one configuration of the `wlr-layer-shell` role, which
rinch picks. `WindowProps::wayland_layer: Option<LayerSurface>`
(`rinch-core/src/element.rs`, with `LayerShellLayer`, `LayerAnchor`,
`LayerMargins`, `LayerExclusiveZone`, `LayerKeyboard`, `LayerAxes`; named
after the protocol, re-exported at the `rinch` root) is any other: a panel,
a dock, a wallpaper, kiosk or signage. Everything goes through
`shell/layer_shell.rs`:

- **Which configuration.** `LayerSpec::for_window` (pure) answers the
  surface a window is created as: `wayland_layer` when it is `Some` — it
  **wins over the shorthands**: a layer surface even with
  `wayland_layer_shell: Some(false)` (`uses_layer_shell`), its own keyboard
  whatever `non_activating` says, its own layer whatever `level` says, no
  `x`/`y` margins, its own output rather than `WindowProps::output` — else
  the overlay, `LayerRequest::surface()`, which spells today's overlay as a
  `LayerSurface` (top-left anchor and margins or none, exclusive zone 0,
  layer `top`/`overlay`, keyboard none/on-demand). An output chosen since
  (`ShellWindow::output_override`: `set_current_window_output`, or none
  after the compositor closed the surface) replaces either's. The shell and
  the tests both go through `LayerSpec`, and `LayerRequest::to_winit` (test
  only) pins that the overlay reaches the fork field for field as before.
- **Compositor-assigned size.** `LayerSurface::compositor_size` asks for the
  protocol's size 0 on an axis, which the compositor then assigns (a bar's
  width: the output's width less the margins, and again on a mode change).
  The protocol allows it only on an axis anchored to both edges
  (`LayerSurface::assigned_axes`); asked elsewhere the window's own size is
  used and `warn_unassignable` logs it. The fork does the protocol side
  (below): it creates the surface with 0 there, sends 0 there on every later
  `set_size` (so a resize of the other axis does not pin the compositor's
  width), keeps the axis at the compositor's size against
  `request_surface_size`, and takes each axis a configure gives. The
  configure goes through the window's ordinary resize path
  (`SurfaceResized`), so the document lays out to the compositor's size with
  nothing layer-specific in the shell. `WindowProps::width`/`height` is the
  first frame's size on an assigned axis, and the toplevel's on GNOME.
- **No gestures, no position.** The app places its own surface by anchors and
  margins, so the shell keeps `LayerState::app_placed()` for it
  (`take_surface_kind`, the window-free half of `note_surface_kind`): no
  position, and `start_layer_gesture` takes a drag or resize request
  (`data-drag-window`, `resize_inset`, `start_window_*`) and does nothing
  (answering `true`, so it does not fall through to the compositor's move,
  which a layer surface has not got); `place_layer` (`set_current_window_position`)
  never re-anchors it. `current_window_position` is `None` for it (Wayland
  readings carry no position and none is published). Re-anchoring a move
  for every other anchor would mean choosing which margins a drag changes on
  a stretched or centred axis, and a panel has no use for it; an app that
  wants a draggable surface changes its margins from a `Drag` itself. The
  overlay keeps its gestures exactly (#1604 is about them, and is untouched).
- **Runtime changes.** `set_current_window_layer(LayerSurface)` posts
  `RinchNativeEvent::SetWindowLayer`; `ShellWindow::set_layer_surface` stores
  it in `WindowProps::wayland_layer` (what a hide/show re-creation and a
  later call start from), clears `output_override`, and on a live layer
  surface takes `layer_update(was, new)` (pure): the changed fields
  (`LayerSet`: layer, anchor, margins, exclusive zone, keyboard,
  compositor-size axes), sent to the fork together as one
  `LayerShellChanges` (`winit_changes`, `set_layer_shell_changes`) and so in
  **one commit** — one setter per field was a commit per field, and a bar
  moved to the top edge with a new margin showed the compositor the top
  anchor with the old margin first — or
  `Recreate` when the output differs — `get_layer_surface` fixes it and the
  protocol has no request to change it, so the surface is made again over
  the same document through `hide_window`/`show_window`, as
  `SetWindowOutput` already did. `was` is computed by `for_window` from the
  props before the change, so an overlay turned into an app's own surface is
  diffed from its overlay configuration. A live window that was not asked to
  be a layer surface is re-created as one; a toplevel given for one (GNOME)
  keeps the configuration for nothing. A change of anchors or axes is not a
  protocol hazard: the fork sends 0 only on an axis anchored to both edges
  in the same commit.
- **A turn is one commit** (#2464). The changes are not sent when the event
  is handled: they are staged on the window (`ShellWindow::pending_layer`),
  and so are an overlay's position (`set_current_window_position`,
  `stage_layer_placement`: a top-left anchor and the position as margins,
  `layer_shell::placement_sets`), in the order asked.
  `RinchRuntime::drain_native_events` sends what the turn staged in one
  `set_layer_shell_changes` when its loop ends (`flush_layer_changes`), and
  `paint` does the same first, for a paint inside the drain (a debug
  screenshot). A size (`set_current_window_size`) needs no staging:
  `request_surface_size` on a layer surface only sends `set_size`, and the
  fork sends the size again with that commit. Before, the order of the
  calls decided the commits: size then layer was one (the size waited for
  the layer change's commit), layer then size two (the layer change
  committed at once, the size with the next frame), and a position was
  already two (`set_layer_shell_anchor`, then `set_layer_shell_margins`,
  each a commit). `hide_window` drops what is staged: the props and the
  saved geometry already hold it for the surface `show_window` makes. A
  move gesture still places the surface at once (`place_layer`), since it
  is not a native event. Pinned live (`#[ignore]`d) by
  `crates/rinch/tests/layer_one_commit_2464.rs`, which counts the commits
  that carry layer state in `WAYLAND_DEBUG=client` output, in both orders,
  for a panel and an overlay, and by `main_window::layer_commit_2464_tests`.
- **GNOME.** No `zwlr_layer_shell_v1`: the fork makes an xdg-toplevel of
  `width` x `height`, `AfterCreate::CheckLayerShell` warns once, and
  `take_surface_kind(Toplevel)` keeps no layer state, so moves and resizes go
  to the compositor as for any window.

Pins: `layer_shell::tests` (precedence, the shorthands' configuration, the
output, the fork's attributes, the exclusive zone's values, assignable axes,
`layer_update`), `rinch_runtime::main_window::layer_drag_tests` (`an_app_placed_…`,
`an_overlay_layer_surface_keeps_its_position`,
`without_layer_shell_an_explicit_surface_is_an_ordinary_toplevel`), and the
fork's `layer_shell::tests`. Live: `examples/wayland-bar` (a bottom bar;
`BAR_SCRIPT=1` changes its exclusive zone at run time, `BAR_SCRIPT=edges`
moves it between the bottom and top edges by its anchors, `BAR_EDGE=top`
starts it on top) and
`examples/wayland-wallpaper` (a `background` surface; a second process,
since rinch shows one window per process), checked in a nested
`kwin_wayland --virtual` and a headless sway (PR for #2413).

### Several windows (`open_window`, #2416 stage 4)

- **Registry** (`crates/rinch/src/window_handle.rs`, thread-local). A `WindowHandle`
  is an id from it, not a document key: a window has no document until it is
  mounted. Each entry has an `unowned` `Signal<bool>` (`is_open`) and, once mounted,
  its document key (what `WindowHandle::current` resolves the running document
  against). `windows()` is a `Signal<Vec<WindowHandle>>`, main first. `open_window`
  queues a `Pending` (props and content) and an `OpenWindow(id)` native event, and
  registers `handle.close()` with the ambient owner (`on_cleanup`).
- **Routing.** A queued native event carries an `EventFor`: `Sender(doc)`, the
  document whose code sent it (the `*_current_window` functions, as in 3b), or
  `Window(id)`, a handle's. `RinchRuntime::role_for` resolves it to a `WindowRole`
  (`Main`, `DevTools`, `Other(id)`), or to none: a handle's event for a closed window,
  and anything addressed to the main window once it has closed, does nothing.
- **Making a window** (`shell/rinch_runtime/open_windows.rs`). On `OpenWindow` the
  runtime builds a `RinchApp` from the content with the app's fonts,
  `set_publishes_own_window_state` and `scope_context_to_doc` (its stores live in its
  own context root, falling back to root 0). It **mounts it before its OS window**
  (`ShellWindow::mount_before_window`) at its props' size and the main window's scale
  (1.0 with no main window on screen). A layer surface whose compositor sizes an axis
  is mounted at its props' size on that axis (0 as 1 px) and gets the real one from
  the compositor's first configure. Then `ShellWindow::create_window`, the main
  window's creation path, makes the OS window; for a document already mounted it
  sends `ScaleFactorChanged` and `Resized` with the window's real scale and size
  before the first frame instead of mounting. Its facts publish under its document
  key (`facts_key`). A popup measuring its content first (#2552) uses this path.
- **What stays the main window's:** taking every title stored before the loop, the
  published vsync pacing (`rinch::window`), handing frames to `on_frame_composited`,
  the native menu bar, and a layer surface re-made on another output when the
  compositor closes it. A second window's layer surface closed by the compositor
  closes the window (`CloseRequested` on `Other` with a layer), with no veto.
- **Closing** is deferred to the end of the handler running
  (`close_soon` → `settle_closes`, as DevTools' close since 3a), so no handler finds
  its window gone. `ShellWindow::close`: what it published goes
  (`forget_published`), then its document (`dispose_document`: component, scopes,
  document, which disposes the windows it owns), then its renderer and OS window, then
  `window_handle::note_closed` turns `is_open` false. A window closed before it was
  made only leaves the pending list.
- **`Window { … }` in `rsx!`** (stage 4b, `rinch-macros/src/dom_codegen/window_codegen.rs`).
  The name had been reserved as a shell element (a compile error); it now has its
  own path. Its props build a `WindowProps` literal (`..Default::default()`): a
  literal goes into its field as written (the field's type decides an integer's),
  everything else through the component props' `.into()` rules, and
  `on_close_requested` into its `Arc`. A closure-valued prop is a compile error
  naming the `WindowHandle` method that changes it after opening (`changed_by`).
  `on_open` and `on_closed` are its own. The children become the content closure
  `generate_children_closure` makes for an `if` body (captures shadowed by
  `site_shadows`), handed to `rinch::__window_component`
  (`window_handle::window_component`), which opens the window under the building
  scope, calls `on_open` with the handle, and answers a comment node. The registry
  calls `on_closed` from `note_closed` unless the owner's cleanup marked the entry
  `owner_gone` first.
- **A windowless app** (`App::windowless`, stage 4c). The builder sets a flag the
  runtime reads at construction (`window_handle::set_windowless`, as `exit_when`) and
  `ExitWhen::Explicitly`. The runtime registers no main handle, starts with
  `main_closed`, and does not let the root publish window facts; on
  `can_create_surfaces` it mounts the root (`mount_windowless_root`) at the props' size
  and scale 1 with no OS window instead of `create_window`. Every path that skips a
  closed main window skips it (`roles`, `role_for`, `ReRender`, `AboutToWait`). A
  `ReRender` looks for painted content in the root (`warn_windowless_root_content`:
  visible text, `img`, `svg`, `canvas`, `video` and the form controls) and warns once.
- **A viewport name in two windows** is warned of once per surface
  (`render_surface::warn_viewports_in_two_windows`, from `note_window_surfaces`): a
  named surface one window's paint shows that another window's last paint showed.
  A surface moved between windows can be warned of once, before the window it left
  paints again.
- **Exit rule** (`exits_after_close`, `App::exit_when`): `MainWindowCloses` exits on
  the main window's close as before; `LastWindowCloses` closes the main window as a
  hide plus a document disposal (`close_main`, `main_closed`) and exits when no other
  is left; `Explicitly` only on `rinch::exit()` (`RinchNativeEvent::Exit`).
- **Each turn,** every open window gets its `AboutToWait` (`about_to_wait_others`),
  so a process-wide change (a theme, a shared signal) restyles every window in the
  turn after it.

### winit Fork

**winit 0.31.0-beta.4 from `joeleaver/winit-fork`, branch
`rinch-patch-beta4`, pinned by rev** (`6f69d8c4c8578f4065ef8a5a90554f1d7ccb4b77`,
#1962, moved for #2114, #2413, #2414, #2501 and #2552). It is a workspace dependency, not a `[patch.crates-io]` entry, so an
app on rinch copies no patch lines (and should delete any old winit, dpi or
wgpu ones). rinch does not re-export winit, and no rinch API takes a winit
type; an app that drives winit itself (`examples/game-embed`, beside the embed
API) declares its own dependency, and naming this git rev keeps one winit in
its build. The rev is pinned so a push to the fork's branch
never changes rinch; moving it is a commit here.

The branch is upstream's `v0.31.0-beta.4` tag plus: seven commits in
`winit-wayland`, all the `wlr-layer-shell` window role (#1523, #1597, #2413;
the last two through joeleaver/winit-fork#3, merged as `bd30ec7c`); one taking
crates.io `dpi` (below); one in `winit-x11` for raw pointer events under a
grab (#2114, joeleaver/winit-fork#1, merged as `d99384ed`); the monitor
changes (#2414, joeleaver/winit-fork#2: the event and a note on its X11
sources, two X11 fixes without which it and winit-x11's own scale-change path
never ran, a merge of the branch after #3, and a rustfmt commit); and the
activation token's input serial (#2501, joeleaver/winit-fork#4); and popups
(#2552, joeleaver/winit-fork#5). The pin is `6f69d8c4`, the merge commit of
joeleaver/winit-fork#5:

- **Popups (#2552, fork PR 5).** Upstream winit#4819 is taken, resolved against
  the layer-shell role: a popup's surface outlives `popup_done` until the app
  drops the window, and a child window holds its parent alive. As a result a
  layer surface given a parent window is refused. A popup's grab is named by
  the newest press of its parent chain; with no such press, creation fails with
  `CreateWindowError::NoGrabCause`. A popup's placement is reported as
  `WindowEvent::Moved`, and `request_popup_reposition` repositions it with a
  token answered by `PopupRepositioned`. One upstream let chain is rewritten
  as nested `if`s, because the branch keeps beta.4's minimum Rust (1.86).

- `WindowAttributesWayland::with_layer_shell(LayerShellAttributes)` makes the
  window a `zwlr_layer_surface_v1` (layer, keyboard interactivity, anchors,
  margins, exclusive zone, output by `wl_output` name, namespace) when the
  compositor has `zwlr_layer_shell_v1`, else an ordinary xdg-toplevel with a
  warning; `WindowExtWayland::wayland_surface_role` says which.
  `set_layer_shell_{margins,anchor,layer,keyboard_interactivity,exclusive_zone}`
  change it at run time and commit at once (a move needs no new frame).
  In beta.4's window state it is a third `WindowType` variant, `Layer`, beside
  upstream's toplevel and popup, with its own configure (`configure_layer`:
  only a size, no states, no decorations). A popup whose parent is a layer
  surface gets it with `get_popup`.
- Requests are sent only within the bound `zwlr_layer_shell_v1` version
  (`set_layer` v2, on-demand keyboard interactivity v4, which falls back to
  none): wayland-client does not check, and the compositor kills the
  connection.
- **Compositor-assigned size** (#2413): `LayerShellAttributes::compositor_size`
  (`LayerShellCompositorSize { width, height }`, changed at run time by
  `set_layer_shell_compositor_size`) asks for the protocol's size 0 on an
  axis, sent only while it is anchored to both edges
  (`LayerShellAttributes::assigned_axes` / `requested_size`; 0 elsewhere is
  `invalid_size`). The surface is created with it, `WindowState::resize`
  sends it on every size request, `request_surface_size` keeps an assigned
  axis at the compositor's size, every attribute change re-sends the size
  (`send_layer_size`: new anchors can start or end an assignment), and
  `configure_layer` takes each axis a configure gives (it took the size only
  when both were given).
- **Several changes in one commit** (#2413):
  `set_layer_shell_changes(LayerShellChanges)` takes any of the properties
  above (`None` keeps one) and sends them under a single commit; the
  single-property setters stay, one commit each (the overlay's moves use
  `set_layer_shell_anchor` and `set_layer_shell_margins`).
- The role's public enums are `#[non_exhaustive]`, as the crate's
  `clippy::exhaustive_enums` asks of upstream's.
- A margin change is confirmed by `WindowEvent::Moved` after a
  `wl_display.sync`, so an app following the pointer with the surface knows
  which origin each pointer position is relative to (#1597;
  `shell::layer_shell`).
- **X11: one `DeviceEvent` per raw pointer event under a grab** (#2114).
  While a window holds `CursorGrabMode::Confined` (an `XGrabPointer` on the
  window with `owner_events`; X11 has no `Locked`, so rinch's pointer lock
  ends up here), the X server delivers each `XI_RawMotion` /
  `XI_RawButtonPress` / `XI_RawButtonRelease` twice to winit's raw selection
  on the root. `dix` `DeliverRawEvent` delivers once through the grab's
  owner-events path, which walks up to the root, and once more in its loop
  over root windows, because `FilterRawEvents` skips the grabbing client only
  when the grab window is the root. The two copies are identical and
  adjacent, so `PointerMotion`, raw `Button` and valuator `MouseWheel` came
  doubled. Mouse-look turned twice as far as on Wayland, Windows or macOS.
  The fork's `XConnection` records which window holds the client's pointer grab
  (cleared by every ungrab, unmap and destroy), and while one is held
  `winit-x11/src/raw_duplicate.rs` passes the first of each identical
  back-to-back pair and drops the second. It pairs rather than dropping every
  repeat, so genuine identical events in one millisecond survive; without a
  grab it filters nothing. Raw keys arrive once (winit never grabs the
  keyboard) and are not filtered. Measured in Xwayland under a nested
  `kwin_wayland --virtual` with XTest, before and after; not measured on a
  real X.Org server with a physical mouse (it shares the `dix` code).
- **`ApplicationHandler::monitors_changed`** (#2414; winit-core, a default
  no-op, forwarded by the `&mut A` / `Box<A>` impls). No detail: the app reads
  `available_monitors`, current by then. Wayland: sctk's `new_output`,
  `update_output` (`wl_output.done`, which follows an `xdg_output` change too)
  and `output_destroyed` set `WinitState::monitors_changed`, and the next
  `single_iteration` calls it after the whole dispatch (sctk drops a removed
  output from its list only after `output_destroyed` returns), before the
  windows' compositor updates; the flag is cleared after the initial
  roundtrip, so start-up outputs are not announced. X11: in
  `process_dpi_change`, after the cached monitor list is dropped and before the
  windows' scale changes (also when nothing had cached a list), so on every
  RandR `RRScreenChangeNotify` and every root `RESOURCE_MANAGER` /
  XSETTINGS change (`Xft.dpi`, winit-x11's scale). Not emitted on Windows,
  macOS or elsewhere.
- **X11: RandR events reach the event processor** (#2414). winit-x11 selects
  `RRScreenChangeNotify` on the root but reads events through Xlib, which drops
  an extension event no library registered a converter for, and winit made its
  RandR requests over x11rb only: `process_dpi_change` never ran from RandR.
  `XConnection::new` calls `XRRQueryExtension` (libXrandr through x11-dl, as
  it calls `XFixesQueryExtension`), so libXrandr is now required by the X11
  backend, like libXfixes and libXi.
- **X11: property changes on the root are selected** (#2414). `property_notify`
  treats a `RESOURCE_MANAGER` change as a scale change, but nothing selected
  `PropertyChange` on the root. The event loop adds it to the client's root
  mask. KWin writes `Xft.dpi` there for Xwayland when an output's scale
  changes; before, winit saw the new scale only at the next RandR event.
- **`MonitorHandleExtWayland::{logical_position, logical_size}`** (#2414):
  `xdg_output`'s logical rectangle. winit's `position()` is the logical
  position times the integer `wl_output.scale` and its video mode is
  unrotated, so neither gives an output's logical rectangle under fractional
  scaling or rotation.
- **An activation token names the input that asked for it** (#2501). Each
  Wayland window notes the seat and serial of the latest input it received
  (`WindowState::note_input`: a key press, repeat or release while it has the
  keyboard focus; a pointer button press, not its release, since Mutter
  accepts only the press's serial; a touch down), and
  `Window::request_activation_token` sets them on the token request
  (`set_serial(serial, seat)`) before its commit. The window's own latest
  input, not the seat's, so the serial belongs to the surface the same
  request names. A window that has received no input requests the token
  without a serial, as upstream does, and the compositor decides what it is
  worth. winit still delivers `WindowEvent::ActivationTokenDone` after the
  window is dropped, under the dropped window's id: rinch matches it by
  serial (see "Activation Tokens").

The beta.2 branch (`rinch-patch`, kept for apps still on it) also carried
Wayland drag and drop (SCTK's `DataDeviceManager`) and its fix to report DnD
positions in physical pixels (#1656). Upstream beta.4 has its own Wayland DnD,
through the data transfer API every platform now uses, and scales its
positions to physical pixels as a pointer's, so neither is carried. rinch
translates the data transfer events in `shell::os_file_drag` (see
`pointer-drag-tooltips.md` "File Drop").

**`dpi`: one crate, crates.io's.** Upstream's workspace takes its in-repo
`dpi` by path (master's, not byte-identical to crates.io 0.1.2), while the
published `winit` 0.31.0-beta.4 depends on crates.io `dpi ^0.1.2`, as muda
does. The fork's commit `34f153bd` makes its workspace take crates.io
`dpi = "0.1.2"` too, so the graph has one `dpi` and an app that names
crates.io `dpi` itself never meets "expected `dpi::PhysicalSize`, found
`dpi::PhysicalSize`". `cargo tree -d` shows no second `dpi`; keep it so on a
bump.

On a winit bump: rebase the branch's commits onto the new tag on a new branch
(apps on the old one keep building), move the rev here, and read upstream's
changelog for API breaks (beta.3 made most public enums `#[non_exhaustive]`;
beta.3/4 replaced the path-carrying `DragEntered`/`DragMoved`/`DragDropped`
with the data transfer API).

### wgpu Fork

**Removed in #1955** (wgpu 27 → 30, Vello 0.7 → 0.11). rinch builds against
crates.io wgpu 30, and (since #1962) `Cargo.toml` has no `[patch.crates-io]` at all.

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
types from `rinch::wgpu`, which is now wgpu 30, and Vello types from
`rinch::vello`. The migration from wgpu 27:
`docs/src/guide/game-engine.md#migrating-from-wgpu-27-to-30`.

### Vello Fork

**Carried since #1955** (Vello 0.11). The workspace depends on `vello` from
https://github.com/joeleaver/vello-fork, pinned by rev
(`b800513d819ab7e7d78f0ac0501893280b6394cc` on branch `rinch-patch`: joeleaver/vello-fork#11's
bilinear taps (#2528) merged with #13's bump reports and #14, over #10, #9 and #8):
the vello `v0.11.0` tag plus the image atlas below, **Bump buffers**, the **Blend
stack**, **Tinted alpha-mask images**, **Subpixel (LCD) mask images**, **Shaders
from build-time naga IR** and **Bind groups reused** after it. The
fork's `vello` takes `vello_encoding` and `vello_shaders` by path, so all three
come from the fork.
It is a dependency, not a `[patch.crates-io]` entry, so an app on rinch gets it
with nothing to copy; apps build Vello types from `rinch::vello` (a second
`vello` from crates.io would be an unpatched copy whose types do not match).
Nothing in the graph may pull crates.io `vello`: `anyrender`/`anyrender_vello`
did and were unused, so #1955 removed them. The rev is pinned so a push to the
fork's branch never changes rinch; moving it is a commit here.

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
render N, including render N's reads of the space being reused. The same
ordering is what stock Vello 0.11 already relies on when it re-uploads a
resident image marked dirty: that write lands on the very region the previous
render read, recorded and submitted the same way. (Vello 0.7 is no precedent:
it created a new atlas texture every render.)

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

**Bump buffers (#2066, #2071, #2466).** Vello's GPU stages allocate tiles,
command lists and the like from bump-allocated buffers, and a frame that needs
more than they hold is not drawn, which on a transparent window is a blank
window. Stock Vello gives them fixed, hand-picked sizes (2^21 tiles, 2^23 ptcl
words, 2^21 lines and segments): about 165 MiB of GPU memory per renderer
whatever it draws, a 1280x40 bar included (#2466). The fork does four things
about it.

1. *First sizes from the frame (#2466, joeleaver/vello-fork#8).*
   `BumpSizes::initial` sizes each buffer from the target's 16x16 tiles `T` and
   256x256 bins `B` and the resolved scene's path tags `S`, paths `P`, draw
   objects `D` and deepest clip/layer nesting `N` (`Encoding::max_open_clips`,
   kept as the scene is encoded), each capped at the device's limit after the
   pool's rounding:

   | buffer | first size | margin over the most measured |
   |---|---|---|
   | tiles | max(min(P, 64) x T, 2^12) | 2.1 (30.7 per target tile, overlapping floating panels) |
   | ptcl | 512 T | 4.6 (110 words per target tile) |
   | lines | max(8 S, 2^14) | 2.2 (3.6 per segment: large circles, wide round strokes) |
   | segments, seg_counts | max(16 S + 4 T, 2^15) | 3.1 (8.2 per segment at 4K, rising with the target) |
   | binning | clamp(D x B, 2^10, 2^18) | an exact bound |
   | blend spill | 2^12 if N <= 4; else clamp(512 x (N - 4) x T, 2^12, 2^20) | exact below the cap: `coarse.wgsl` spills a tile only `if max_blend_depth > BLEND_STACK_SPLIT` (4), 2 x (depth - 4) x 256 words; capped at Vello's 2^20, so a scene that fits today still fits |

   The calibration rendered rinch's examples (ui-zoo's 18 sections,
   markdown-editor, node-graph-demo, tree-demo, todo-app,
   floating-panels-demo) and a deliberately curve-heavy scene at 1280x720,
   1080p and 4K on a Radeon 890M; its table is in joeleaver/vello-fork#8, and
   `vello_encoding`'s `config::initial` tests pin each rule against recorded
   counts. Pimble and other GPUs were not in it. A renderer keeps the largest
   sizes any frame was given or needed (a high-water mark; it never shrinks),
   so a window's partial frames, slices and full frames share one size per
   buffer. `use_cpu` and WebAssembly, which read no counters back and so would
   never see an overflow, keep Vello's sizes raised to the target floors of
   joeleaver/vello-fork#3 and #4 (`BumpSizes::floors`: tiles 32 and ptcl 512
   words per target tile); rinch uses neither. With the readback, 512 ptcl
   words a tile is the floor #2071 set: 8 sequential full-window opacity layers
   overflow 256 at 4K. It is now most of what is left at 4K (64 MiB of about
   90), at 4.6 times what anything measured used.
2. *No wait.* `render_to_texture` submits and returns. Its counters are mapped
   after submit and collected with `PollType::Poll`. From #2066 until #2071 it
   waited for the GPU after every frame, once per split slice, and on a shared
   device for the embedder's work too: a 110 ms GPU frame held the main thread
   110 ms, where it now holds it 1.7 ms (Radeon 890M, 4K). Do not add back
   a blocking poll here; `device.poll(wait_indefinitely)` waits for everything on
   the device.
3. *Recovery.* A frame that overflowed has gone out undrawn by the time its
   counters arrive, along with any frame submitted meanwhile. The buffers grow,
   and `Renderer::check_frames` answers `Lost`. So it does, conservatively, for a
   frame whose counters are never seen: pushed out of the 64-entry queue, or a
   failed map. It is the caller's job to draw again, because the frame's damage
   is long spent:
   - `RinchRuntime::check_vello_frames` runs in `about_to_wait`, where
     `WgpuRenderer::check_frames` tells `UiTextureState`, so `ui_invalid_reason`
     makes the next frame full (`repaint_full_invalidated`). A full frame paints every
     kept split slice again, and repairs a lost partial frame's rects. Then it
     requests the redraw. On `Pending` it arms a `WaitUntil` every 8 ms, so a
     window at rest is back on `Wait` soon after the GPU finishes.
   - `RinchOverlayRenderer::check_frames` sets `fed` to 0, which repaints the
     slices. `render` calls it, which is enough for a host rendering every
     frame; an event-driven host calls it itself (`game-engine.md`). Android's
     loop asks for a redraw and has a timed wake as the
     desktop does.
   - A test or tool calling `render_to_texture` directly gets none of this:
     after it, `check_frames` until not `Pending`, and render again on `Lost`.

   A failed buffer grows to what it asked for plus a quarter. The stages after
   a failed one stop short of saying what they need, so a frame can be lost
   once per buffer; since #2466 a frame that ran out of lines also grows its
   segments and segment counts to 8 per line asked for, so it does not then
   run out of those in turn.
4. *No dead sizes in the pool.* The engine's pool keeps buffers for reuse by
   size class and label. Since #2466, once one of the six bump buffers sized by
   the high-water mark (tiles, segments, ptcl, lines, seg_counts, blend spill)
   is created at a new size, the pool drops the smaller ones of that label,
   which no frame asks for again. `bin_data` is not among them: its size also
   counts the scene's binning info, so it moves both ways.

Pins: `app::gpu_bump_overflow_2071_tests` (overflow found, repaired, never
forgotten; eight full-window layers at 4K draw first time) and
`app::gpu_bump_sizes_2466_tests` (a 1280x40 window holds under 4 MiB of Vello
buffers; the eight-layer page, the calibration's curve scene and a page of
cards draw first time on a new renderer at 1080p and 4K; four full-target
layers nested at 4K fit the blend floor, five are lost once and then draw).
The fork's `vello_research_tests` snapshots, `deep_blend` (7 nested layers)
among them, pass with it; their images come from upstream Vello's LFS store,
since the fork's has none. How deep real apps nest is unmeasured: no
calibrated rinch scene went past 4.

**Blend stack (#2103).** `fine.wgsl` keeps the backdrop of each open clip or
layer while it paints the layer's content, and composites the content over it
at the pop. Stock Vello packs that backdrop to 8 bits a channel
(`pack4x8unorm`). The fork packs it to 16 (`pack2x16unorm`, two words a pixel,
in registers and in the spill buffer past four levels; `coarse.wgsl` allocates
twice the spill). The 8-bit stack rounded the premultiplied backdrop once more.
Over an opaque backdrop it rounds the colour. Over a translucent one it rounds
the alpha as well, and the alpha's error is later multiplied by whatever the
result is composited over. A split surface's above slice
(`docs/design/split-layer-gpu-surfaces.md`) is rendered over transparent and
composited over the surface. Where a glyph's antialiased edge was under a later
box-shadow's clip layer over the surface, the split was 2 levels off the
in-scene frame, past D10's one. The window's scene changes too, by at most a
level in tiles a clip's edge crosses, towards the float result. Pinned by
`shell::split_pages_1865_tests::gpu_split_pages_2103_glyph_edge_shadow_tail`
(red with the 8-bit stack).

**Tinted alpha-mask images (#2286).** An image draw carries one more word, a
premultiplied tint, and a mask mode in bits 16-17 of its sample word
(`DrawTag::IMAGE` is `0x2D0`, info 11 and scene 4 words; upstream `0x28C`, 10
and 3). In mode 1 (`IMAGE_MASK_ALPHA`) the fine shader's `image_paint` reads the
sampled texel's alpha as coverage and paints the tint in it, in every quality
arm; the pixel format is read as one bit, so the mode never reaches it.
`Encoding::encode_image_mask`, `Scene::fill_image_mask` and
`Scene::draw_image_mask` are the API. The glyph-mask atlas draws every glyph
this way (`paint-and-damage.md`, "Glyph-mask atlas"): one mask for every colour.
Pins: the fork's `encoding::tests::image_mask_draw_data` (the draw data and the
tag's sizes) and `gpu_glyph_atlas_2286` (pixels).

**Subpixel (LCD) mask images (#2286 stage 3, joeleaver/vello-fork#7).** Mask
mode 2 (`IMAGE_MASK_LCD`; `Encoding::encode_image_lcd_mask`,
`Scene::draw_image_lcd_mask`): the texel's RGB is per-channel coverage, its
alpha their mean. Only the nearest arm composites it per channel
(`lcd_composite` in `fine.wgsl`; `image_paint` draws any mask mode as grey
coverage in the bilinear and bicubic arms). `lcd_composite` takes the current
layer's pixel and, while it is not opaque, composites it over the next clip's
backdrop down the blend stack (registers, then the spill buffer at depth four
and past); over an opaque result it blends per channel and writes the opaque
pixel into the current layer, else it paints the grey coverage as mode 1. The
fold is exact only for source-over layers, so rinch draws mode 2 only with no
opacity, blend or isolated layer open (`paint-and-damage.md`, "Subpixel (LCD)
text"). Pins: `image_mask_draw_data` (mode 2's words) and
`gpu_subpixel_text_2286` (pixels, including the spill at depth 6).

**Shaders from build-time naga IR (#2468, joeleaver/vello-fork#9).** Stock
Vello hands wgpu each shader's WGSL, so every `Renderer::new` (one per window,
overlay and embed renderer) parsed all 21 shaders with naga: 119 M of its
149 M instructions with a warm driver cache, which no cache removes. The
fork's `vello_shaders` build script already parses each shader to read its
bindings. Its `naga_ir` feature also serializes that module (postcard) and
embeds it as `ComputeShader::naga_ir` (about 106 KB). vello's `naga_ir` feature,
which `crates/rinch/Cargo.toml` turns on for native targets (on WebGPU, wgpu
would write the module back out as WGSL), creates the shader from
`ShaderSource::Naga`, so wgpu still validates the module and translates it for
the backend; only the parse goes. `Renderer::new` falls to 36.5 M instructions,
and 9-16 ms of CPU from 24-42 ms (one init thread, RADV). A module that does not
decode, which would need the build script's naga and wgpu's to differ (cargo
resolves one 30.x for both), falls back to its WGSL with a warning. Pin: the
fork's `vello_shaders/tests/naga_ir.rs` (each shader's IR is byte-for-byte a
fresh parse of its WGSL).

rinch builds every window, overlay and embed renderer, and the Android
shell's, through `gpu_init::new_renderer` (the Android GPU self-test in
`android_runtime.rs` keeps Vello's options).
That function caps the shader-compile threads at 4 (Vello's own default is
`max(parallelism, 4) - 2`) and calls glibc's `malloc_trim` afterwards. With a
cold driver cache, Vello's default 20 threads left 52 MB of freed compiler memory
in their malloc arenas; with the cap and the trim, 12-16 MB is left. The table is
in `gpu_init.rs`, and the knob is `App::gpu_init_threads` /
`RINCH_GPU_INIT_THREADS`.

**Bind groups reused (#2469, joeleaver/vello-fork#10).** Stock Vello made
every dispatch's bind group again on every frame: 15 a frame for a small
scene, a third of `render_to_texture`'s instructions. The fork's engine keeps
them in a `BindGroupCache`, keyed by the layout and the exact buffers (with
offset and size) and texture views they bind (wgpu's identity `Hash`/`Eq`).
A frame binds the pool's same buffers as the frame before it, so a repeated
frame makes at most 2. A group unused for 16 recordings is dropped. Groups that
bind something the caller passed in (rinch's target view, new each frame, and
its texture, new each resize) are never cached, and a group binding an image
the recording destroys (the gradient ramps) goes with it: otherwise each
cached group kept a window-sized texture alive, about 16 MB more per 1440p
resize. A 1280x32 bar's `render_to_texture` went from 780 K to 650 K
instructions a frame. When the pool lets a grown buffer's old sizes go
(joeleaver/vello-fork#8), the cache starts afresh so it does not keep them. Pins: the fork's `bind_group_cache_tests` (ignored, need
an adapter), including `a_target_made_per_frame_is_released`.

An overflow costs a visible frame, or one per buffer that runs out, since a stage
after one that failed stops short of saying what it needs: 400 window-sized
layers at 1024x768 took three frames to draw. A frame that needs more than the
device's largest buffer stays undrawn, logged, and is not reported as lost, or
the shell would repaint it forever. `Limits::default()` caps a buffer at 128 MiB
unless the app asks for more (`GpuInit`). Growth is not undone, and the pool keeps
a buffer of each size class it has used.

A lost device is not reported by any of this. wgpu's `Device::poll` panics on
one rather than returning an error (`handle_error_fatal`; a `DeviceError::Lost`
is no `PollError`), and the check polls, as the desktop's present always has.

Pins: `app::gpu_bump_overflow_2071_tests` (full frame, frames in flight, partial
frame, a frame pushed out of the queue, 8 layers at 4K drawing first time) and
`gpu_split_runtime_an_overflowed_slice_is_painted_again`, all without a blocking
poll before the decision; `shell::desktop` and `a_lost_vello_frame_makes_the_next_frame_full`
for `UiTextureState`, with no adapter; the fork's `bump_sizes_tests` for the
floors. A failed map has no test. The loop's half needs a window, so nothing in
CI runs `check_vello_frames` itself: to check it, run a page of 400 layers in a nested compositor with `RINCH_PERF=1`
and see the frames repeat with no input.

**Downstream apps copy nothing for Vello**, and delete the five wgpu-fork
patch lines they used to copy. On a Vello bump, rebase the commits onto the new
tag (or go back to crates.io if upstream behaves the same), move the pinned rev,
and rerun the pins.

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
