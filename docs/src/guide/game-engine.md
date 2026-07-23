# Game Engine Integration

Rinch supports two complementary integration patterns for game engines and custom renderers:

1. **Embed API** — Your game owns the window and GPU. Rinch provides UI as a Vello scene you composite on top.
2. **RenderSurface** — Rinch owns the window. Your game/renderer submits frames (CPU pixels or GPU textures) into a DOM component.

## RenderSurface (Recommended)

`RenderSurface` is a component that embeds external pixel content into rinch's layout. Your renderer submits frames via a thread-safe `SurfaceWriter` (CPU pixels) or `GpuTextureRegistrar` (GPU textures), and rinch composites them during paint. Input events (mouse, keyboard) are routed back to your event handler.

This is the simpler pattern — rinch handles windowing, layout, and event dispatch. You just provide pixels and handle surface-local events.

### Quick Start

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let surface = create_render_surface();

    // Handle input events from the surface
    surface.set_event_handler(|event| {
        match event {
            SurfaceEvent::MouseDown { x, y, button } => { /* handle click */ },
            SurfaceEvent::MouseMove { x, y } => { /* handle hover */ },
            SurfaceEvent::MouseWheel { delta_y, .. } => { /* handle zoom */ },
            SurfaceEvent::KeyDown(key) => { /* handle keyboard */ },
            _ => {}
        }
    });

    // Submit frames from a worker thread
    let writer = surface.writer();
    std::thread::spawn(move || {
        loop {
            let pixels = render_frame(); // your renderer
            writer.submit_frame(&pixels, width, height);
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    });

    rsx! {
        div { style: "display: flex; height: 100%;",
            div { style: "width: 200px;",
                // Sidebar with rinch UI controls
                Button { onclick: || do_something(), "Tool" }
            }
            RenderSurface { surface: Some(surface), style: "flex: 1;" }
        }
    }
}

fn main() {
    run("My App", 1280, 720, app);
}
```

### CPU Pixel Submission

`SurfaceWriter` is `Send + Sync + Clone` — safe to use from any thread:

```rust
let surface = create_render_surface();
let writer = surface.writer();

// From any thread:
let pixels: Vec<u8> = render_rgba(width, height);
writer.submit_frame(&pixels, width, height);
```

Pixels are RGBA8, row-major. The surface redraws automatically after `submit_frame()`.

### GPU Texture Compositing

For zero-copy compositing, use `GpuTextureRegistrar` to provide a `wgpu::TextureView` directly:

```rust
let surface = create_render_surface();
let registrar = surface.gpu_registrar();

// Get the shared wgpu Device via gpu_handle()
let gpu = gpu_handle().unwrap();
let device = &gpu.device;
let queue = &gpu.queue;

// Create your texture and render to it
let texture = device.create_texture(&wgpu::TextureDescriptor { /* ... */ });
// ... render into texture ...

// Register the texture view for compositing
let view = texture.create_view(&Default::default());
registrar.set_texture_source(view, width, height);
registrar.notify_frame_ready();
```

`GpuTextureRegistrar` is also `Send + Sync + Clone`. The texture must be created on the same `wgpu::Device` (available via `gpu_handle()`).

> **Use `rinch::wgpu`.** rinch pins a patched `wgpu` fork. Construct every `wgpu` type you hand back to rinch (`TextureView`, config features/limits, …) from `rinch::wgpu::…`, not a separately-pinned `wgpu` dependency, or the types won't match.

### Sharing a High-Capability GPU Device

Zero-copy compositing requires your texture to live on **the same device rinch composites with**. By default rinch requests that device with `Features::default()` / `Limits::default()`. If your renderer needs more (extra features, larger storage buffers, more bind groups, …), raise the device's requirements at startup so the shared device — the one published via `gpu_handle()` — can host your pipelines too.

**Let rinch own the device (recommended).** rinch still creates the instance, picks a *surface-compatible* adapter, and creates the device, so presentation is always correct — you only add the capabilities you need:

```rust
use rinch::prelude::*;
use rinch::wgpu;

let gpu = RinchGpuConfig {
    required_features: wgpu::Features::FLOAT32_FILTERABLE,
    required_limits: wgpu::Limits {
        max_storage_buffers_per_shader_stage: 32, // wgpu default is 8
        ..Default::default()
    },
};
run_with_gpu_config(app, WindowProps::default(), None, gpu);
```

After startup, `gpu_handle()` returns the shared `device`, `queue`, **and `adapter`** (use `adapter.limits()` to clamp, e.g. `max_buffer_size = min(adapter, 2 GiB)`). The requested features/limits are passed through verbatim — if the adapter can't satisfy them, device creation fails loudly rather than silently dropping a capability.

**Bring your own device.** If you must keep your renderer's exact `DeviceDescriptor`, build the whole GPU stack and hand it to rinch. rinch creates only the window surface (from your `instance`), validates that your adapter can present to it, and composites directly onto your device — no `request_device`, no CPU readback:

```rust
use std::sync::Arc;
use rinch::prelude::*;

let gpu = ExternalGpu {
    instance,                    // rinch creates the window surface from this
    adapter: Arc::new(adapter),  // must be able to present to the window
    device: Arc::new(device),
    queue: Arc::new(queue),
};
run_with_external_device(app, WindowProps::default(), None, gpu);
```

The provided device is published through `gpu_handle()`. On a single-GPU machine a headless adapter (`compatible_surface: None`) usually presents fine; on multi-GPU systems create the adapter from a surface-compatible one, or prefer `run_with_gpu_config` (which guarantees compatibility). See the `gpu-device-config` example for both modes.

### Layout Size

Query the surface's current layout dimensions (set by CSS/Taffy) to match your render resolution:

```rust
let (w, h) = surface.layout_size();
// or from the registrar:
let (w, h) = registrar.layout_size();
```

### Surface Events

Events are dispatched to the handler set via `set_event_handler()`. Coordinates are in logical pixels relative to the surface's top-left corner.

| Event | Fields | Description |
|-------|--------|-------------|
| `MouseDown` | `x, y, button` | Mouse button pressed |
| `MouseUp` | `x, y, button` | Mouse button released |
| `MouseMove` | `x, y` | Mouse moved over surface |
| `MouseWheel` | `x, y, delta_x, delta_y` | Scroll wheel |
| `MouseEnter` | `x, y` | Cursor entered surface |
| `MouseLeave` | — | Cursor left surface |
| `KeyDown` | `SurfaceKeyData` | Key pressed (when focused) |
| `KeyUp` | `SurfaceKeyData` | Key released (when focused) |
| `TextInput` | `String` | Text input (when focused) |
| `FocusGained` | — | Surface received keyboard focus |
| `FocusLost` | — | Surface lost keyboard focus |

`SurfaceKeyData` contains `key`, `code`, `ctrl`, `shift`, `alt`, `meta`.

### Web (canvas viewport)

The **same** `RenderSurface` + `create_render_surface()` API works on `rinch-web`, but the model is inverted. On the web the **browser** composites, so rinch only creates and manages a real `<canvas>` "viewport hole" sized by layout; the **app owns the GPU context** (rinch links no wgpu on web). This mirrors desktop symmetrically: **desktop** = rinch owns the window and you submit frames; **web** = you own the canvas surface.

```rust
use rinch::prelude::*;
use rinch::render_surface::{create_render_surface, RenderSurface};

let surface = create_render_surface();

// After mount, grab the <canvas> and create your own wgpu WebGPU surface on it.
// (canvas_element() returns None until the component has mounted.)
if let Some(canvas) = surface.canvas_element() {
    let wgpu_surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))?;
    // request adapter/device (async on web), configure, and render each frame.
}

// Resize notification (ResizeObserver-driven). Size is PHYSICAL px
// (CSS px × devicePixelRatio), matching desktop — HiDPI-correct out of the box.
surface.set_resize_callback(move |w, h| reconfigure_wgpu_surface(w, h));

// Per-frame tick (drives requestAnimationFrame on web).
surface.set_render_callback(move |_writer, w, h| render_triangle(w, h));

rsx! { div { style: "flex:1;", RenderSurface { surface: Some(surface) } } }
```

Notes:

- **HiDPI out of the box.** `layout_size()` reports **physical** pixels (`CSS px × devicePixelRatio`), rinch sizes the canvas backing store to match, and `set_resize_callback` pushes the new physical size on every resize so you can reconfigure your wgpu/WebGL surface.
- **Input** (pointer / wheel / keyboard incl. `KeyUp` / focus) over the canvas is delivered to `set_event_handler` — rinch does not swallow it. Click the canvas to focus it for keyboard events.
- **CPU path still works & is portable.** `SurfaceWriter::submit_frame` blits via a lazily-created 2D context; it self-disables once you claim a WebGPU/WebGL context. So the same app can run on desktop and web.
- **Clean teardown.** The ResizeObserver and canvas listeners are removed when the component unmounts — no leaks.
- See `examples/webgpu-surface-web` (a WebGPU triangle in a rinch DOM UI) — the web counterpart of `examples/game-embed`. Build with `trunk serve` over `localhost` (so `navigator.gpu` is available; the `webgl` cargo feature is a fallback).

### API Reference: RenderSurface

| Function / Type | Description |
|----------------|-------------|
| `create_render_surface()` | Create a new surface handle |
| `RenderSurfaceHandle` | Main handle — set event handler, get writer/registrar |
| `SurfaceWriter` | Thread-safe CPU pixel submission (`Send + Sync + Clone`) |
| `GpuTextureRegistrar` | Thread-safe GPU texture registration (`Send + Sync + Clone`) |
| `RenderSurface` | Component — use in RSX with `surface: Some(handle)` |
| `SurfaceEvent` | Input event enum dispatched to handler |

**RenderSurfaceHandle methods:**

| Method | Description |
|--------|-------------|
| `writer()` | Get a `SurfaceWriter` for CPU pixel submission |
| `gpu_registrar()` | Get a `GpuTextureRegistrar` for GPU texture compositing |
| `set_event_handler(handler)` | Set input event callback (main thread closure) |
| `set_render_callback(cb)` | Per-frame `FnMut(&SurfaceWriter, w, h)` — drives `requestAnimationFrame` on web |
| `set_resize_callback(cb)` | `FnMut(w, h)` fired on backing-size change (physical px) — reconfigure a GPU surface |
| `layout_size()` | Get current physical `(width, height)` (web: CSS px × devicePixelRatio) |
| `canvas_element()` | **(web only)** the `<canvas>` after mount — create a WebGPU/WebGL context on it |
| `set_texture_source(view, w, h)` | **(desktop, gpu)** Set GPU texture directly (main thread only) |
| `has_texture_source()` | Check if a GPU texture is registered |
| `id()` | Surface ID |
| `viewport_name()` | Internal viewport name |

---

## Embed API

The embed API is for when **your game owns the window and wgpu device**. Rinch runs headless — you feed it platform events, it produces a Vello scene, and you render/composite it yourself.

Enable with: `features = ["desktop"]`

### Quick Start

```rust
use rinch::prelude::*;
use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};

#[component]
fn game_hud() -> NodeHandle {
    let health = Signal::new(100);
    rsx! {
        div { style: "position: absolute; top: 10px; left: 10px;",
            Text { size: "lg", color: "white", {|| format!("HP: {}", health.get())} }
        }
    }
}

fn main() {
    // Your game creates the window and wgpu device
    let (device, queue, surface, window) = my_engine::init();

    // Create rinch UI context
    let mut ctx = RinchContext::new(
        RinchContextConfig {
            width: 1280,
            height: 720,
            scale_factor: window.scale_factor(),
            theme: None,
        },
        game_hud,
    );

    // Create overlay renderer from your device
    let mut overlay = RinchOverlayRenderer::new(
        &device, 1280, 720, wgpu::TextureFormat::Rgba8Unorm,
    );

    // Game loop
    loop {
        let events = collect_platform_events(&window);
        let actions = ctx.update(&events);

        for action in &actions {
            match action {
                AppAction::SetCursor(cursor) => { /* set cursor */ },
                AppAction::Exit => return,
                _ => {}
            }
        }

        game.render(&device, &queue);
        let ui_texture = overlay.render(&device, &queue, ctx.scene());
        composite(&device, &queue, &surface, game_texture, ui_texture);
    }
}
```

### RinchContext

`RinchContext` is the main handle. Create it during initialization, call
`update()` each frame.

**Multiple contexts.** Several `RinchContext`s can coexist on one thread — a
screen HUD plus N render-to-texture panels, say. Each context tracks signal
changes through its own subscription (so creating or dropping one never
silences another) and keeps its own document-scoped element-bounds signals,
editor registrations, and focus requests. Stores/contexts are **namespaced
per context** (#136): a `create_store` inside a context's component lands in
that context's own namespace, its effects and event handlers resolve that
namespace first, and lookups **fall back to the thread-global namespace** for
stores created outside any context (e.g. before mounting). Two contexts can
therefore create the *same* store type without overwriting each other, and
dropping a context clears its namespace. One caveat remains: each context
still processes the input events *you* feed it — route each window's events
only to its own context.

### Input Routing

For HUD overlays, use `wants_mouse` and `wants_keyboard` to decide whether input goes to the UI or the game:

```rust
if ctx.wants_mouse(mouse_x, mouse_y) {
    ctx.update(&[mouse_event]); // UI element under cursor
} else {
    game.handle_mouse(mouse_x, mouse_y); // game content
}

if ctx.wants_keyboard() {
    ctx.update(&[key_event]); // text input focused
} else {
    game.handle_key(key); // game shortcuts
}
```

### Split Layout (Viewport Hole)

Use `GameViewport` to mark a transparent region where the game renders:

```rust
use rinch::embed::GameViewport;

#[component]
fn editor_ui() -> NodeHandle {
    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100%;",
            div { class: "toolbar",
                Button { onclick: || save(), "Save" }
            }
            div { style: "display: flex; flex: 1;",
                div { style: "width: 200px;", /* side panel */ }
                GameViewport { name: "main", style: "flex: 1;" }
            }
        }
    }
}
```

Query the viewport rect to set your game's render region:

```rust
if let Some(rect) = ctx.viewport_rect("main") {
    game.set_viewport(rect.x, rect.y, rect.width, rect.height);
}
```

### Resize and Scale Factor

```rust
ctx.resize(new_width, new_height);
overlay.resize(&device, new_width, new_height);
ctx.set_scale_factor(window.scale_factor());
```

### Platform Events

Translate your engine's events to `rinch_platform::PlatformEvent`:

```rust
use rinch_platform::{PlatformEvent, MouseButton, KeyCode, Modifiers};

PlatformEvent::MouseMove { x: 100.0, y: 200.0 }
PlatformEvent::MouseDown { x: 100.0, y: 200.0, button: MouseButton::Left }
PlatformEvent::MouseUp { x: 100.0, y: 200.0, button: MouseButton::Left }
PlatformEvent::MouseWheel { x: 100.0, y: 200.0, delta_x: 0.0, delta_y: -30.0 }
PlatformEvent::KeyDown {
    key: KeyCode::KeyA,
    text: Some("a".into()),
    modifiers: Modifiers::default(),
}
PlatformEvent::Resized { width: 1920, height: 1080 }
```

### API Reference: Embed

**RinchContext:**

| Method | Description |
|--------|-------------|
| `new(config, component)` | Create and mount a rinch UI |
| `update(&events) -> Vec<AppAction>` | Process events, update layout, return actions |
| `scene() -> &Scene` | Get the Vello scene (lazy rebuild) |
| `resize(w, h)` | Notify of window resize (physical pixels) |
| `set_scale_factor(scale)` | Update DPI scale factor |
| `set_theme(&props)` | Replace this context's theme (restyles on the next `update()`) |
| `viewport_rect(name) -> Option<LayoutRect>` | Query a GameViewport's computed rect |
| `wants_mouse(x, y) -> bool` | True if point hits UI (not viewport hole) |
| `wants_keyboard() -> bool` | True if a text input is focused |
| `needs_update() -> bool` | True if UI needs repaint |
| `register_font(data)` | Register font data for text rendering |
| `app() / app_mut()` | Access the underlying RinchApp |

**RinchOverlayRenderer:**

| Method | Description |
|--------|-------------|
| `new(device, w, h, format)` | Create from game's wgpu device |
| `render(device, queue, scene) -> TextureView` | Render scene to texture |
| `resize(device, w, h)` | Resize render target |
| `texture()` | Get the underlying wgpu Texture |

**RinchContextConfig:**

| Field | Type | Description |
|-------|------|-------------|
| `width` | `u32` | Initial viewport width (physical pixels) |
| `height` | `u32` | Initial viewport height (physical pixels) |
| `scale_factor` | `f64` | Display scale factor |
| `theme` | `Option<ThemeProviderProps>` | Theme configuration |

**Theming is per-context.** Each `RinchContext` owns the theme CSS generated
from its `config.theme` (or the default theme when `None`) — creating a second
context, or a shell app changing the thread-global theme, never restyles an
existing context (issue #138). To change an embedded context's theme at
runtime, call `ctx.set_theme(&props)`; the document restyles on its next
`update()`.

> **Caveat:** an embedded `ThemeProvider` component with a reactive
> `dark_mode_fn`/`primary_color_fn` writes the *thread-global* theme slot,
> which embed contexts deliberately ignore — `RinchContext::set_theme` is the
> supported path for runtime theme changes in the embed API.

**LayoutRect:**

| Field | Type | Description |
|-------|------|-------------|
| `x` | `f32` | Absolute X position (logical pixels) |
| `y` | `f32` | Absolute Y position (logical pixels) |
| `width` | `f32` | Width (logical pixels) |
| `height` | `f32` | Height (logical pixels) |

## Which Pattern to Use?

| Scenario | Use |
|----------|-----|
| Adding UI overlay to an existing game engine | **Embed API** — game keeps its window/GPU ownership |
| Building a tool with embedded viewports (e.g., level editor, paint app) | **RenderSurface** — rinch handles the window, you embed content |
| Terminal emulator, video player, or custom canvas inside a rinch app | **RenderSurface** — component-level integration |
| WASM game with HTML-based UI | Neither — use the browser-native DOM backend |

## Examples

- `examples/game-embed/` — Spinning cube with rinch HUD overlay (embed API)
- `examples/render-surface-demo/` — Painting app with canvas and navigator (RenderSurface)
- `examples/webgpu-surface-web/` — **(web)** WebGPU triangle in a rinch DOM UI — the app owns a wgpu `<canvas>` viewport (RenderSurface on `rinch-web`)
