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
    App::new(app).title("My App").size(1280, 720).run();
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

### Drawing with the GPU (`set_gpu_render_callback`)

The simplest GPU path: rinch hands your callback a device and a target at the surface's
size, and draws what you render **as the surface element's content**, in the scene, the way
an `<img>` is drawn.

```rust
use rinch::render_surface::GpuSurfaceFrame;
use rinch::wgpu;

let surface = create_render_surface();
surface.set_continuous(true); // a game: every frame. `false` + request_frame() for an editor.
surface.set_gpu_render_callback(move |f: &GpuSurfaceFrame| {
    // f.device / f.queue: rinch's device. f.target: this frame's view,
    // f.size: physical px, f.format: build your pipelines for it.
    // Rebuild device resources when f.generation changes.
    let mut encoder = f.device.create_command_encoder(&Default::default());
    // ... a render pass into f.target ...
    f.queue.submit(Some(encoder.finish()));
});
// Optional fallback when no GPU frame can be given (software renderer, embed):
surface.set_render_callback(move |writer, w, h| { /* CPU pixels */ });

rsx! { div { style: "flex: 1;", RenderSurface { surface: Some(surface) } } }
```

- **It is an element.** Ancestor backgrounds are beneath it; an element painted after it, or
  with a higher `z-index`, covers it; `border-radius` and `overflow` clipping on an ancestor
  clip it; an ancestor's `opacity` fades it; a transform moves, scales and rotates it; it
  scrolls with its scroller. Its own background, border, padding, outline and shadow are
  painted as an `<img>`'s are, with the frame in the **content box**, rounded by the inner
  radius. There is no hole and nothing to opt into.
- **Pacing.** The target is reallocated when the surface's size changes, and the callback is
  paced by `set_continuous` / `request_frame` (see below) — **on demand until you call
  `set_continuous`**: a frame at the first size, at each new size, and for each
  `request_frame()`. (An unset CPU render callback runs at every paint, but only damages
  when it submits a frame; a GPU frame always damages, so it is not run for a caret blink.)
  A frame drawn is the next paint's damage; a quiet surface costs nothing.
- Draw with `queue.submit` before the callback returns. The texture is drawn as
  premultiplied alpha: clear an opaque viewport to an opaque colour. Its bytes are
  sRGB-encoded (`f.format` is `Rgba8Unorm` on desktop); for linear-light blending, create a
  view of `f.texture` with format `Rgba8UnormSrgb`, which it allows.
- **Size and transforms.** The target is the surface's content box at the window's scale,
  times the transform's scale along each axis, as a CPU frame's is: under a `scale(2)` it is
  twice the layout size, so it stays sharp; under a `rotate()` it keeps the box's shape and
  turns with it. The surface's own `border-radius` rounds the frame, as an `<img>`'s does.
- **Large and many surfaces.** A frame drawn in the scene lives in Vello's image atlas for the paint: one
  texture of at most 8192×8192 shared by every image in the scene. A surface past the
  device's largest texture or 8192 px a side gets a target scaled down to fit (aspect kept;
  `f.size` says how big; `layout_size()` stays the real size), and when the GPU frames of a
  paint together pass half the atlas, all of them — idle ones included, which are redrawn —
  are drawn at one smaller scale, and at full size again once there is room. Either way the
  frame is drawn — scaled up into the box, so softer — and rinch logs a warning once. A
  surface laid out off screen counts toward the share; one under `display: none` does not.
- **Cost.** On the desktop GPU window the frame is composited outside the scene when it
  can be (#1865, the split path): a new frame under a still UI renders no Vello at all, about
  what a compositor layer cost, and the pixels are the scene's. A surface inside an opacity
  group, a blend or a mask, under a rotating or skewing transform, overlapping another split
  surface, or past the window's split memory cap is drawn in the scene instead, where each
  frame costs a Vello render of (at least) its region. Measured in the scene on an
  integrated GPU (Radeon 890M): a 640×360 surface 3–7 ms a frame, a full-window 1080p one
  about 12 ms, a full-window 4K one about 15 ms. Nothing in the app chooses between the two.
- A GPU frame needs a GPU renderer presenting the window (`gpu_frames_available()`). Under
  the software renderer and in an embedded context the ordinary render callback runs
  instead, if you set one.
- The device is never replaced on desktop: every window — including one re-created by
  hiding and showing — presents with the device rinch made first (or yours, with
  `App::external_gpu`), so `f.generation` stays the same there. A lost device stays lost
  (#1788).
- **Mixing with `set_texture_source`.** The GPU render callback draws into a target of its
  own; a texture registered with `set_texture_source` is not drawn into and, while the GPU
  callback draws, not shown. Use one or the other per surface.
- **Off screen still draws.** Pacing does not look at visibility: a continuous surface
  scrolled out of view (or under `visibility: hidden`) keeps drawing every frame. Switch it to
  `set_continuous(false)` while it is out of view if that matters.

Example: `examples/gpu-surface` (a rotating triangle, Pause / One frame).

### GPU Texture Compositing

> **For new code, use the GPU render callback above.** It draws on the same shared device
> with no readback, and its frame is an element: in paint order, clipped, rounded and
> faded like an `<img>`, and composited outside the scene when it can be. A texture
> registered with `set_texture_source` is the older route: the compositor draws it as a
> layer, which neither takes the element's place in paint order nor splits. The ui-zoo cube
> and `examples/gpu-device-config`'s `zerocopy` mode moved to the callback (#1865).

To hand rinch a `wgpu::TextureView` you render yourself, use `GpuTextureRegistrar`:

```rust
let surface = create_render_surface();
let registrar = surface.gpu_registrar();

// Get the shared wgpu Device via gpu_handle(). It is None while the window
// presents with the software renderer (RINCH_RENDERER=cpu, or the fallback
// when the GPU would not start), so handle that case.
let Some(gpu) = gpu_handle() else {
    // No GPU device: submit CPU pixels through surface.writer() instead.
    return;
};
let device = &gpu.device;
let queue = &gpu.queue;

// Create your texture and render to it
let texture = device.create_texture(&wgpu::TextureDescriptor { /* ... */ });
// ... render into texture ...

// Register the texture for compositing — pass BOTH the texture and its view.
// rinch needs the texture for the GPU→CPU readback path (inline-painted
// RenderSurface) and the view for compositing it directly.
let view = texture.create_view(&Default::default());
registrar.set_texture_source(texture, view, width, height);
registrar.notify_frame_ready();
```

`GpuTextureRegistrar` is also `Send + Sync + Clone`. The texture must be created on the same `wgpu::Device` (available via `gpu_handle()`).

`gpu_handle()` returns `None` until the window's GPU renderer has started, and for the whole session when the window presents with the software renderer: `Renderer::Software` was chosen, a user set `RINCH_RENDERER=cpu`, or the GPU would not start and `Renderer::Auto` fell back (see [Rendering Backends](windows.md#rendering-backends)). An app that configures the device with `App::gpu_config` or `App::external_gpu` (below) is exempt: it always presents on the GPU, or panics at startup if the GPU will not start, so once its window is up the handle is there.

> **Use `rinch::wgpu` and `rinch::vello`.** rinch re-exports the exact crates it links: `rinch::wgpu` (**wgpu 30**, crates.io) and `rinch::vello` (**Vello 0.11** from `joeleaver/vello-fork` at a pinned rev, with `rinch::vello::peniko` and `rinch::vello::kurbo`). Construct every `wgpu` or Vello type you hand to rinch (`TextureView`, config features/limits, a `Scene`, colours, shapes, …) from these, instead of declaring your own `wgpu`/`vello` dependencies, so the versions can never drift. If you need an extra wgpu feature (e.g. `webgpu`), keep `wgpu = "30"` in your `Cargo.toml` with just that feature; Cargo unifies it with rinch's.

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
App::new(app).gpu_config(gpu).run();
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
App::new(app).external_gpu(gpu).run();
```

The provided device is published through `gpu_handle()`. On a single-GPU machine a headless adapter (`compatible_surface: None`) usually presents fine; on multi-GPU systems create the adapter from a surface-compatible one, or prefer `App::gpu_config` (which guarantees compatibility). See the `gpu-device-config` example for both modes.

### Migrating from wgpu 27 to 30

rinch moved from wgpu 27 / Vello 0.7 to **wgpu 30 / Vello 0.11** (#1955), and
dropped its patched wgpu fork: there is no wgpu `[patch.crates-io]` to copy any
more. An app that shares rinch's `Device` (`App::gpu_config`,
`App::external_gpu`, `gpu_handle()`, `GpuTextureRegistrar`, a GPU render
callback, the embed API) uses the same wgpu, so its own wgpu code moves with
it. The changes rinch's own code and examples needed:

| wgpu 27 | wgpu 30 |
|---|---|
| `Instance::new(&InstanceDescriptor { .. })` | `Instance::new(InstanceDescriptor { .., display: None })` (by value) |
| `InstanceDescriptor::default()` / `from_env_or_default()` | `new_without_display_handle()` / `new_without_display_handle_from_env()` (or the `with_display_handle` forms) |
| `SurfaceTargetUnsafe::RawHandle { raw_display_handle: h, .. }` | `raw_display_handle: Some(h)` |
| `RequestAdapterOptions { .. }` | add `apply_limit_buckets: false` (or `..Default::default()`) |
| `SurfaceConfiguration { .. }` | add `color_space: SurfaceColorSpace::Auto` (the old behaviour) |
| `surface.get_current_texture()` → `Result<SurfaceTexture, SurfaceError>` | → `CurrentSurfaceTexture::{Success, Suboptimal, Timeout, Occluded, Outdated, Lost, Validation}` |
| `frame.present()` | `queue.present(frame)` |
| `slice.get_mapped_range()` → `BufferView` | → `Result<BufferView, MapRangeError>` |
| `device.push_error_scope(f); … device.pop_error_scope()` | `let g = device.push_error_scope(f); … g.pop()` |
| `PipelineLayoutDescriptor { bind_group_layouts: &[&l], push_constant_ranges: &[] }` | `bind_group_layouts: &[Some(&l)], immediate_size: 0` |
| `RenderPipelineDescriptor { multiview: None, .. }` | `multiview_mask: None` |
| `RenderPassDescriptor { .. }` | add `multiview_mask: None` |
| `VertexState { buffers: &[layout] }` | `buffers: &[Some(layout)]` |
| `DepthStencilState { depth_write_enabled: true, depth_compare: Less, .. }` | `Some(true)`, `Some(CompareFunction::Less)` |

Each row was checked against the wgpu 27.0.1 and 30.0.1 sources; wgpu's own
changelog has the rest (v28.0.0, v29.0.0, v30.0.0 sections). A `RenderSurface`
frame, a registered texture and a Vello scene from the embed API composite as
before; Vello 0.9 began sampling images at pixel centres, and rinch's painter
adjusted, so its images are drawn at the same pixels.

**Use `rinch::vello`; no patch line is needed.** rinch depends on Vello 0.11
from `joeleaver/vello-fork` at a pinned commit (#1955): stock Vello 0.11 makes
every image already in its atlas draw transparent after any render of a scene
with no image, gradient or text, and keeps an image in its atlas for two renders
after its last use, so large or many images can go undrawn; the fork fixes both.
An app on rinch gets it as an ordinary dependency. Build Vello types (a
`Scene`, `peniko` colours, `kurbo` shapes, a `vello::Renderer` for the embed
API's `scene()`) from `rinch::vello`, `rinch::vello::peniko` and
`rinch::vello::kurbo`, not from a `vello` dependency of your own, which would
be a second, unpatched copy whose types do not match rinch's. If you register your own textures with
`Renderer::register_texture` and draw into them in place, call
`Renderer::mark_override_image_dirty` before each render that should show the
new contents (Vello 0.9+ copies an override texture into its atlas only when it
is new or marked).

**Migrating an app from the wgpu-27 rinch:**

- Delete your wgpu-fork `[patch.crates-io]` lines (`wgpu`, `wgpu-core`,
  `wgpu-hal`, `wgpu-types`, `naga`); rinch no longer uses a wgpu fork. Delete
  the winit and dpi ones too (rinch takes its winit fork as a pinned
  dependency since #1962), and add nothing for Vello.
- If you share rinch's GPU device or build Vello types, use `rinch::wgpu` and
  `rinch::vello` instead of declaring your own `wgpu`/`vello` dependencies, so
  the versions can never drift.
- If you need an extra wgpu feature (e.g. `webgpu`), keep `wgpu = "30"` with
  just that feature; Cargo unifies it with rinch's.

**An app that drives winit itself** (an engine's own window and event loop
beside the embed API, as `examples/game-embed` does) declares its own winit;
no rinch API takes a winit type. rinch is on winit 0.31.0-beta.4 (#1962);
naming the same dependency keeps one winit in the build:

```toml
winit = { git = "https://github.com/joeleaver/winit-fork", rev = "34f153bd6e523de53f9da67c6db58bd1f8732542" }
```

From beta.2, winit's public enums are `#[non_exhaustive]` (a `match` on
`MouseScrollDelta`, `PointerSource`, `ButtonSource` and the rest needs a `_`
arm), and an OS drag is a data transfer: `DragEntered { id, position }`,
`DragPosition`, `DragDropped { id, .. }` and `DragLeft`, with the dragged
data fetched by `ActiveEventLoop::fetch_data_transfer` and accepted with
`set_valid_dnd_actions` (winit's changelog, beta.3 and beta.4).

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

**A press keeps the pointer.** From a press on the surface until the release of the button
that started it, every move and release of that pointer go to the surface wherever it is
(coordinates then fall outside the surface's size), and it hears no `MouseLeave` until
then; a `MouseLeave` follows a release off the surface. A press or release of another button
meanwhile is a chord: the surface hears it and keeps the pointer. The browser backend does
this with `setPointerCapture`. Desktop routes the *surface's* events the same way, but
not the document's: DOM hover (`:hover`, `onmouseenter` / `onmouseleave`) and the cursor
still follow the element under the pointer during the press, where a browser routes them
to the capturing canvas.

A press whose release never arrives is ended without one, and the surface hears it ended —
`PointerCancel` with pointer events, else a `MouseUp` where the cursor is: on desktop when
the window loses focus, when the same pointer presses the left button again (another
finger's press proves nothing), on a platform `PointerCancel` (Android's touch-scroll
takeover), and silently when the surface unmounts; in the browser on `pointercancel`.

**Pen, touch and pinch: pointer events (opt-in).** A drawing surface wants to know which
device pressed and how hard. `surface.set_pointer_events(true)` replaces `MouseDown` /
`MouseMove` / `MouseUp` with these (every other surface hears exactly what it did before):

| Event | Fields | Description |
|-------|--------|-------------|
| `PointerDown` | `x, y, button, pointer` | A button, pen tip or finger went down |
| `PointerMove` | `x, y, pointer` | It moved (or its pressure changed) |
| `PointerUp` | `x, y, button, pointer` | The press ended |
| `PointerCancel` | `pointer` | The platform took the pointer away mid-press; no `PointerUp` follows |
| `Pinch` | `x, y, scale` | Two touches spreading or closing, a trackpad pinch, or Ctrl+wheel; `scale` multiplies the zoom (above 1 is in), relative to the last `Pinch` |

`SurfacePointer` is `{ id, kind, pressure, primary }`: `kind` is `Mouse`, `Pen`, `Eraser`,
`Touch` or `Unknown`; `pressure` is `0.0..=1.0`, and a device that cannot measure it
reports `0.5` while down and `0` otherwise (the Pointer Events rule); `id` tells pointers
that are down at once apart (two fingers) and stays the same from a press to its release.
Compare ids for equality only: their values are the platform's. On desktop the mouse is
`1`, a tablet tool `2` and each finger an id of its own above both; in the browser `id` is
the event's `pointerId`, whose value for a mouse differs between engines. What each host
reports:

- **Browser**: Pointer Events (`pointerType`, `pressure`, `pointerId`, `isPrimary`, the
  eraser button); two touches pinch; Ctrl+wheel (a trackpad pinch) pinches.
- **Desktop**: winit's pointer source (a tablet tool's force, a touch's force where
  measured); two touches pinch; a trackpad pinch (macOS, iOS, Wayland) and Ctrl+wheel
  pinch. One move per pointer per frame, as the browser does without
  `getCoalescedEvents`.
- **Android**: every finger is folded into one mouse pointer (id `1`, kind `Mouse`) by the
  touch translation, so there is no `Touch` kind and no two-finger `Pinch`.
- **Embed** (`RinchContext`): the host feeds plain mouse events, so every pointer is the
  mouse (`Mouse`, id `1`); Ctrl+wheel still pinches.

`SurfaceKeyData` contains `key`, `code`, `ctrl`, `shift`, `alt`, `meta`. `key` is spelled like the browser's `KeyboardEvent.key` on both backends, so the space bar is `key == " "` and `code == "Space"`.

**A focused surface only swallows the keys it claims (issue #482).** `set_event_handler`
keeps receiving every `KeyDown`/`KeyUp` exactly as above — that delivery is unaffected.
`set_key_handler(handler: impl Fn(&SurfaceKeyData) -> bool)` is a separate, additive
registration asked *after*: should this key stop at the surface? Leave it unset — the
default — and the surface claims nothing, so a host's own `window`-level keybindings (and,
on the web, the browser's own reload/find/… shortcuts) keep working the moment the canvas
is focused, and on desktop so do DevTools (F12), inspect mode (Alt+I) and Tab:

```rust
surface.set_key_handler(|key| matches!(key.code.as_str(), "KeyW" | "KeyA" | "KeyS" | "KeyD" | "Space"));
```

The document-level keyboard interceptor (`set_keyboard_interceptor`) is unaffected either
way — it already sees every key before a focused surface does, on both backends.

**On the web, an unclaimed key also keeps the browser's default action** — no longer
`preventDefault()`ed just because the surface is focused, matching an unfocused `<canvas>`.
Space, PageUp/Down and the arrow keys can scroll the page, and Tab leaves the canvas. A web
game steering with any of those must claim them via `set_key_handler`, or the page scrolls
under it.

### How often the render callback runs

`set_render_callback` keeps each backend's original pacing until you choose one: on desktop
it runs whenever the window paints (for any reason), on the web on every animation frame.
`set_continuous` picks the same behaviour on both:

```rust
// A game in play: every frame, on desktop and in the browser.
surface.set_continuous(true);

// An editor viewport: only when something changed.
surface.set_continuous(false);
// …and wherever the scene changes (an event handler, a signal effect):
surface.request_frame();
```

- `set_continuous(true)` — every frame while the surface is mounted. On desktop each run asks
  the window for the next paint; on the web the `requestAnimationFrame` loop keeps going.
- `set_continuous(false)` — on demand: once when the surface first has a size, once more
  whenever its size changes, and once for each `request_frame()` (several requests before the
  next frame are one frame). Nothing else runs it, and the web's loop stops in between.
- `request_frame()` works in every mode, including from inside the render callback — an
  on-demand surface that is mid-animation calls it each frame until it is done.
- A surface with no size is not run, in any mode: one under `display: none`, one whose node
  has left the document (a handle hidden by an `if`), or one not laid out yet. A
  `request_frame()` made meanwhile waits, and draws once the surface has a size; it does not
  keep the app, the browser's animation-frame loop or an embedding host busy in between.
- A surface under `visibility: hidden` **is** run: it is still rendered, with its size, the
  way a `Loader` in a closed `Drawer` keeps animating. A continuous surface in an overlay that
  closes that way goes on drawing every frame; call `set_continuous(false)` when you close it
  (and `true` when you open it again).
- `RenderSurfaceHandle` is not `Send`, so `set_continuous` and `request_frame` are
  main-thread calls. From another thread, deliver a frame with the `SurfaceWriter` from
  `writer()` (`submit_frame` is `Send + Sync` and asks for the paint itself).

`layout_rect()` returns `(x, y, width, height)`: the top-left of the surface's painted box
(its border box after any transform) in logical (CSS) pixels relative to the window — on the
web, the canvas's `getBoundingClientRect`, read when you call it — and its frame size
(`layout_size()`, the content box on desktop) in physical pixels. Use it to place a menu or tooltip at a
surface-local point on both targets.

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
| `set_key_handler(handler)` | `Fn(&SurfaceKeyData) -> bool`; whether a key already delivered to `set_event_handler` should stop here instead of continuing to a window-level host keybinding (web) or DevTools/inspect/Tab (desktop). Unset: claims nothing (issue #482) |
| `set_render_callback(cb)` | Per-frame `FnMut(&SurfaceWriter, w, h)` — drives `requestAnimationFrame` on web |
| `set_continuous(on)` | `true`: run the render callback every frame; `false`: on demand (first size, each resize, each `request_frame`). Same on desktop and web (#1768) |
| `request_frame()` | Run the render callback once more at the next frame; callable from inside it |
| `layout_rect()` | `(x, y, w, h)`: the painted box's top-left in logical window px (web: the canvas's client rect), and the frame size (as `layout_size()`) in physical px |
| `set_resize_callback(cb)` | `FnMut(w, h)` fired on backing-size change (physical px) — reconfigure a GPU surface |
| `layout_size()` | Get the current frame size in physical px: desktop, the content box (inside border and padding) × transform scale × display scale; web, the canvas's CSS px × devicePixelRatio |
| `canvas_element()` | **(web only)** the `<canvas>` after mount — create a WebGPU/WebGL context on it |
| `set_gpu_render_callback(cb)` | **(gpu)** `FnMut(&GpuSurfaceFrame)`: draw the surface with rinch's device into a target drawn in the scene as the element's content; paced like `set_render_callback`, which runs instead when no GPU frame can be given (#1768) |
| `gpu_frames_available()` | **(gpu)** whether a GPU frame can be given now (a GPU renderer presents the window) |
| `set_texture_source(texture, view, w, h)` | **(desktop, gpu)** Set GPU texture directly (main thread only) |
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
        let ui_texture = overlay.render(&device, &queue, &mut ctx);
        composite(&device, &queue, &surface, game_texture, ui_texture);
    }
}
```

**Replacing the overlay renderer.** `RinchOverlayRenderer` is bound to the
device it was created on, and the `RinchContext` is not. After a device loss,
drop the renderer, create a new one on the new device and keep the context:
the new renderer's first `render` draws the whole UI, the HUD over a
`GameViewport` included, with nothing changed in the document and nothing to
mark dirty (#1976). The same holds for two renderers taking turns on one
context, and for one renderer drawing several contexts in turn: a renderer
that does not hold the context's last painted frame has that frame painted
again for it. Steady state (one renderer, one context) paints only when the
document changes.

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
namespace first — and so do a timer, an `rinch-http` completion, a
`run_on_main_thread` closure, a `rinch-ws` event and a menu, tray, hotkey,
notification or focus-target callback made inside it, wherever it is fired
(#1032; a menu item built in `main` is no context's, even clicked in a
context's DOM bar, and an `unowned` timer that outlives its context runs in
that dead namespace, where `create_store` leaves an entry nothing clears;
the dismiss stack and the interceptors still run at the thread-global root,
#1665) — and lookups **fall back to the thread-global namespace** for
stores created outside any context (e.g. before mounting). Two contexts can
therefore create the *same* store type without overwriting each other, and
dropping a context clears its namespace. One caveat remains: each context
still processes the input events *you* feed it — route each window's events
only to its own context.

**Background threads.** The documented cross-thread pattern works in embed:
`Signal::send()` / `Signal::update_send()` from a worker, and
`run_on_main_thread()` for anything else, marshal the write onto the thread that
created the context and apply it at the top of the **next `update()`** — before
the events of that frame, so a queued write and an event handler are absorbed by
the same layout pass. Everything that rides that transport comes with it:
`set_timeout`, `rinch-http` completions, `rinch-ws` events. `Signal::set()` and
`Signal::update()` still panic off the main thread — only the `_send` variants
marshal.

The queue is process-global, so a `RinchContext` you drive drains work queued
against *any* context on the thread. That is fine: the payload writes its own
signals, signals are thread-local rather than per-document, and the document it
touches is still repainted by its own `update()`.

> Before issue #172 an embedded context registered the main thread but no
> dispatcher, and a background-thread `send()` panicked on the calling thread.
> If you pinned an older rinch and wrapped your worker writes to avoid it, you
> can drop the workaround.

**In a browser (wasm32).** A page whose only rinch host is a `RinchContext` —
a game that owns the canvas and composites rinch's scene, with no `rinch-web`
root — gets the browser's scheduling from the context: `RinchContext::new`
installs the timer backend (`window.setTimeout`) and the task scheduler
(microtasks) on wasm32, so `set_timeout`, `set_interval`, `sleep`,
`spawn_local` and the timers inside `rinch-ws` / `rinch-rtc` run with no
`update()` needed to start them (issue #1956). `update()` still drains polled
signals once a frame. Work that arms a timer **before** the first context is
created needs the runtime first: call `rinch::install_web_runtime()` at
startup (idempotent; it also installs a between-frames poll drain). It is a
no-op on native targets, so a cross-platform app needs no
`#[cfg(target_arch = "wasm32")]` around the call. See
[Running on WASM](wasm.md#a-page-with-no-rinch-root).

**Timed work: the caret blink.** A focused rich-text `Editor`'s caret toggles
every 530 ms, and `update()` ticks that clock just as the desktop runtime does
after each event-loop turn (issue #331). A host that calls `update()` every
frame needs nothing more. An **event-driven** host — one that renders only when
input arrives — asks `ctx.next_wake()` after each `update()`: it is the instant
the caret next toggles (what desktop arms its `ControlFlow::WaitUntil` with), or
`None` when nothing is blinking. Schedule a wake for it, or the caret toggles
only when some unrelated event happens to arrive. `needs_update()` answers
`true` once that instant has passed. A blurred host window
(`PlatformEvent::WindowFocus(false)`) stops the blink with the caret solid and
arms no wake. Each context has its own blink clock (issue #1149): two contexts
on one thread that each have a focused editor both blink, each on its own
phase, and blurring or typing in one leaves the other's blink alone.

**RenderSurface in an embedded context.** A `RenderSurface` works inside a
`RinchContext` the way it does on desktop: `scene()` tells each surface in
that context's document its physical layout size, runs its render callback, and
paints its latest frame inline at the surface's box (issue #331). Call `scene()`
once per frame — every call runs the render callbacks again. A frame submitted
from another thread makes `needs_update()` true and the next `update()` answer
`RequestRedraw`. Several contexts on one thread each drive only their own
surfaces, and a thread with no `RenderSurface` at all pays nothing for this in
`scene()`.
Three things are **not** composited by rinch in an embedded context, because
the host owns compositing there: a surface registered under a `data-viewport`
name (`create_render_surface_with_name`), a `<video>` / `VideoViewport` (whose
frame sink is installed only by the desktop runtime), and a
`GpuTextureRegistrar` texture on a `RenderSurface` (read back only through the
desktop runtime's own device, so in an embedded context it draws nothing, and
its `notify_frame_ready` asks for no redraw). Treat them like a [`GameViewport`](#split-layout-viewport-hole)
hole: render into your own target and composite it under rinch's overlay.

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

> **A mouse-clicked plain button does not claim `wants_keyboard()` —
> a Tab-focused one still does** (issue #548). A `<button>`/`<a href>` is
> focusable with no `tabindex` needed, so a single click on one (no Tab
> involved) used to make `wants_keyboard()` answer `true` until the next
> click landed somewhere else — a host following the pattern above then
> stopped seeing its own Esc/hotkey presses the moment the user clicked
> *any* button in a HUD. The fix follows the browser's own `:focus-visible`
> split rather than "is a node focused" or "did it register anything":
>
> - A **mouse click** on a plain button/link only ever consumes Enter/Space
>   through rinch's own activation path, so it does not need the host's
>   keyboard — `wants_keyboard()` is `false`.
> - **Tab** (or a programmatic `NodeHandle::focus()`) onto that same plain
>   button sets its keyboard-focus ring (`is_focus_visible`), and
>   `wants_keyboard()` is `true` — so Tab navigation and Enter/Space
>   activation of ordinary controls keep reaching rinch under the pattern
>   above, exactly as before #548's fix.
> - A **custom widget** that registered its own
>   [`FocusEntry::on_key`](rinch::focus_registry::FocusEntry::on_key)
>   (`register_focus_target` — arrow-key navigation, a shortcut of its own)
>   is `true` whichever way it was focused: the runtime can't know which
>   keys it wants without seeing them.
> - An open native **`<select>`** popup is always `true` too — its own
>   arrow/Enter/Escape handling needs every key (a separate, pre-existing
>   gap folded into the same fix).

> **A viewport hole must stay hittable — that is what routes the mouse.**
> `wants_mouse` only reports "the game wants this" when the hit-tested node has a
> `data-viewport` ancestor. A hole that is *not* hittable inverts the routing: the
> hit falls through to `body` behind it, the walk up from `body` finds no
> `data-viewport`, and `wants_mouse` answers `true` — **everywhere**. The game
> silently receives no mouse input at all, with nothing in the layout or the
> render looking wrong.
>
> Since #207 `GameViewport` needs no defending: every `data-viewport` node gets
> `pointer-events: auto; background: transparent;` from the UA stylesheet, so both
> `GameViewport { name: "main" }` and `GameViewport { name: "main", style: "flex: 1;" }`
> route correctly. Because those defaults are a UA rule rather than an inline
> style, restyling the hole cannot strip them. (A component's universal `style:`
> prop is merged over the component's inline style per property since #647, so
> it would only displace an inline default by declaring that same property.)
> Being a UA declaration, it also survives `pointer-events: none`
> inherited from a HUD root, and still loses to any `pointer-events` you declare on
> the viewport yourself (which is how you deliberately hand a region back to the
> UI).
>
> **The inheritance trap is still real for everything you build yourself.**
> `pointer-events` is inherited, so `pointer-events: none` on a HUD root — the
> natural way to let clicks fall through to the game — reaches every descendant.
> Your own HUD controls, and any hand-rolled hole that isn't a `data-viewport`
> node, need an explicit opt-in:
>
> ```rust
> div { style: "pointer-events: none;",          // HUD root: clicks fall through
>     GameViewport { name: "main", style: "flex: 1;" }   // hittable by default
>     div { style: "pointer-events: auto;", /* real HUD controls */ }
> }
> ```
>
> That inheritance is spec-correct, not a bug — but the overlay idiom that triggers
> it is the obvious one to reach for, so it is worth designing around up front.
> (`visibility: hidden` works the same way: it is inherited, so a hidden HUD root
> makes its whole subtree unhittable, and a descendant that sets
> `visibility: visible` is painted and takes pointer events again, on desktop as
> in a browser — issue #843.)

#### `title` tooltips in a context

An element's `title` shows as a tooltip after the pointer rests over it, as on
desktop (#1646). The rest is timed, so an event-driven host must call `update`
at [`next_wake()`](#rinchcontext), which includes the tooltip's deadline. When
the pointer leaves the UI's surface — the window, or the part of it you route
to rinch — call `ctx.pointer_left()`; there is no `PlatformEvent` for it, and a
tooltip showing at the time otherwise stays until the next press, key or move.
It answers whether a frame is owed. `ctx.app_mut().set_title_tooltips(false)`
turns tooltips off for a HUD that does not want them.

### Input methods (IME)

Your engine owns the window, so it owns the platform input method (IBus, a
Wayland `text-input`, Windows TSF, macOS's input sources). rinch tells you what
the focused target wants; you switch the input method on and off and place its
candidate window, and feed the composition back in:

```rust
// The caret area is an `ElementBounds` in the context's logical px; your
// platform probably wants physical px.
let to_physical = |b: ElementBounds, s: f32| (b.x * s, b.y * s, b.width * s, b.height * s);

let actions = ctx.update(&events);
match ctx.take_ime_change() {
    Some(ImeChange::Enable { cursor_area, .. }) => {
        // Switch the platform IME on, its candidate box at the caret — or
        // wherever the platform likes while the target has no caret yet.
        platform_ime_enable(cursor_area.map(|b| to_physical(b, scale)));
    }
    Some(ImeChange::Update { cursor_area, .. }) => {
        platform_ime_move_candidate_box(to_physical(cursor_area, scale));
    }
    Some(ImeChange::Disable) => platform_ime_disable(),
    // `ImeChange` is `#[non_exhaustive]`: a later request kind lands here.
    Some(_) | None => {}
}
// The platform's composition events go back in on the next update:
// PlatformEvent::Ime(ImeEvent::Enabled / Preedit { .. } / Commit(..) / Disabled)
```

`take_ime_change()` answers the change from what it last reported, so call it
once per frame and apply every answer: it is the same decision the desktop
shell makes for its window after every event-loop turn. `ime_state()` reads the
wanted state itself (`ImeState { enabled, cursor_area, .. }`) without consuming
anything. The IME is on for a focused `<input>`/`<textarea>`, an editor that is
not read-only, and a node registered with `FocusEntry::on_ime`; it is off for
everything else and while you have reported the window blurred
(`PlatformEvent::WindowFocus(false)`). That is not `wants_keyboard()`: a node
that reads keys but takes no text wants the keyboard and no input method. The
caret area is where the caret is painted, so it follows transforms and a text
field's horizontal scroll.

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

#### The viewport is the base layer

**Nothing behind a `GameViewport` is ever visible.** Its box shows the game's
pixels: everything painted beneath the viewport — ancestor backgrounds
(whether or not they clip), earlier siblings, the window's base colour — is
replaced there by the game (or, in an embedded context, left transparent for
yours). What
is painted *after* it draws over the game as usual: its own border, its
children, later siblings, an element with a higher `z-index`, popups, your
HUD. So a full-window panel with a background around the viewport needs no
special treatment.

The clear is the viewport's own shape — a `border-radius` on it rounds the
game's corners, the UI covering them — and it is clipped by the viewport's
clipping ancestors and moved by its transform, like any paint of the element.
It holds on the desktop GPU window, which composites the game under the UI and
replaces what is beneath the viewport there; in an embedded context, where
`RinchOverlayRenderer::render(&device, &queue, &mut ctx)` leaves the texture
transparent inside the viewport for your host to composite the game under;
and on the software window, which clears the box and draws the frame in the
scene. A clipping ancestor with a background — an `overflow: hidden` panel,
square or rounded, or a scroller the viewport is partly scrolled out of — is
replaced under the viewport's visible part, edges included, on all three.

> **Rendering `ctx.scene()` yourself.** A host that renders the scene with
> its own Vello renderer, instead of `RinchOverlayRenderer`, gets the
> viewport cleared *in the scene*, which is approximate inside a clip: along a
> clipping ancestor's edges near the viewport (within one 16px tile), Vello
> keeps what the clip saved, so a band of that ancestor's background can show
> over the game. `RinchOverlayRenderer::render` cuts the viewport out after
> Vello and has no band. The desktop GPU window falls back to the same
> in-scene clear for a frame in which the viewport sits in a group effect or
> under a rotation (below), which the debug warnings name.

**The game fills the viewport's border box.** The viewport is a base layer,
not a framed element: the game's layer (or, on the software window, its
frame, fitted `contain`) covers the whole box, and the viewport's own border
and its children are painted over it, so a border hides the game's edge and
the padding shows the game. That differs from a `RenderSurface`, whose frame
sits in its content box, inside its padding. `RinchContext::viewport_rect`
reports that border box, so render the game at its size.

Two things break the rule, and a **debug build warns once per viewport**,
naming it and the element responsible:

- **A group effect** on the viewport or an ancestor — `opacity` below 1,
  `mask-image`, or `filter`. The viewport is cleared inside the group, and the
  group is then composited over what is beneath it, so that shows through over
  the game. Put the effect on the viewport's siblings instead, or move the
  viewport out of the element.
- **A transform other than a translation** (a rotation, scale or skew) on the
  viewport or an ancestor: the clear turns with the element, but the game's
  layer is composited axis-aligned at the viewport's box. Keep the viewport's
  transforms to translations. (The software window draws the frame under the
  same transform, so there only the group warning applies; the desktop GPU
  window composites a translated or scaled viewport exactly, and warns only
  for a rotation or skew.)

> **A hole with nothing behind it: `data-viewport-ready`.** The clear removes
> everything beneath the viewport, the window's base included, so on a
> transparent window a hole nothing fills is see-through to the desktop, not
> to your app ([#186]). A viewport whose
> content can be *absent* opts out by stamping `data-viewport-ready="false"` on
> the hole; paint then clears nothing and the node paints its own
> `background`, which is the natural seat for a poster or an error affordance.
> Set it back to `"true"` once there is something to show.
>
> The attribute is an **opt-out, and absence means ready**: a `data-viewport`
> node that never stamps it punches unconditionally, exactly as before.
> `GameViewport` is therefore unaffected — it stamps nothing, keeps its hole
> from the first frame, and needs no change. `VideoViewport` (rinch-video) is
> the one built-in that opts in: it stays `"false"` until the decoder has
> handed a real frame to the compositor, and flips back to `"false"` on a
> `PlaybackState::Error`. A node that *does* carry the attribute must say
> exactly `"true"` to punch, so a mis-stamped value fails to the safe side — an
> opaque placeholder, never a see-through window.
>
> **Not ready also means no frame**, on both backends. A surface keeps the last
> buffer it was given, so a video that errors mid-stream still has one; while
> its node says `"false"` that frame is not drawn — the node paints as an
> ordinary element (on both backends since #1768; the GPU backend used to hand
> the compositor no layer for it) — and the node's own `background` is what
> shows. Say `"true"`
> again and the frame the surface holds is back, with no new frame needed.
>
> Since [#358] software cuts no hole for a video at all (see below), so there
> the gate decides only whether the frame or the node's `#000` placeholder is
> drawn.

> **A hole bigger than what fills it: black bars (GPU `GameViewport`).** The
> complementary case ([#354]). Since #1768 this is the `GameViewport` CPU-frame
> path only: video is drawn in the scene on the GPU too (below), with the same
> black bars the software backend paints. The GPU compositor **aspect-fits** a frame inside its viewport,
> so a source whose aspect ratio differs from the box leaves letterbox or
> pillarbox bars — inside the cleared box, but covered by no layer. That was
> see-through to the desktop for a video playing perfectly. The compositor now
> pushes an opaque black layer over the **whole** viewport box under the frame's
> own layer, so the bars come out black exactly as a browser paints them for
> `<video>`, and the hole is always fully covered.
>
> The black is painted by the **compositor**, not by the viewport element. That
> distinction is the whole design: the compositor draws *under* the Vello UI, so
> giving the element an opaque `background` would hide the video entirely.
> `GameViewport` is unaffected — its surface fills its box, so the fit is exact
> and the backdrop is entirely overdrawn.
>
> On the **software** backend the same bars are black for a different reason —
> see the next note.

> **Software paints every viewport inline.** The two backends route viewport
> frames differently, and the split is worth knowing if you are reading the
> compositing code ([#358], [#361]).
>
> | | `RenderSurface` | video | `GameViewport` |
> |---|---|---|---|
> | **software** | inline | **inline** | **inline** |
> | **GPU** | split, or inline when it can't be (#1768, #1865) | **split**, or inline (#1768, #1865) | compositor |
>
> Software used to blit its compositor frames onto the **finished** pixel
> buffer, after the whole UI had been painted, clipped only by the viewport's
> overflow-clipping ancestors. That write has no notion of occlusion, so every
> overlay above a playing video or a game — the nav drawer, `Modal`,
> `DropdownMenu`, `Select`'s popup, tooltips, a HUD, DevTools — was overwritten.
> GPU has no such problem: its layers go down *first* and the Vello UI
> alpha-blends on top, so an opaque drawer already covers the layer there. But a
> layer under the UI showed only through the backgrounds of *clipping* ancestors
> (where paint then cut its hole; since #1866 the viewport clears its own box),
> and took no ancestor's opacity or transform, so
> since #1768 video is drawn in the scene on the GPU as on software: an element
> like any other. Cost, measured on an integrated GPU (Radeon 890M): a 4K
> full-window video is about 25 ms a frame in the scene against 6 ms as a layer;
> a 640×360 one 3–6 ms against 1–3.
>
> Since #1865 the desktop GPU window gets the layer's cost back without its
> limits: a video or a GPU `RenderSurface` is composited outside the scene
> with the UI above it cut out around it, so a new frame under a still UI is
> an upload and a composite, about what a layer cost. Its pixels are the
> scene's except on the edge classes the design record's D10 bounds
> (`docs/design/split-layer-gpu-surfaces.md`). A surface inside an opacity group, a blend or a mask, under a
> rotating or skewing transform, or overlapping another split surface is
> drawn in the scene instead. So is one past the window's split memory cap:
> what splitting adds over the scene (slice textures and a scratch target,
> not the video frame, which the scene holds too) may be four times the
> window's own texture bytes (33 MB at 1080p, 133 MB at 4K). The surfaces
> kept split from earlier frames are counted first, then this frame's in
> paint order.
>
> So on software a frame goes through **paint** instead, on the same inline
> path a `RenderSurface` component has always used: the `data-viewport` node
> draws the frame `object-fit: contain` inside its box, at its own z-order.
> Overlays occlude it by ordinary paint order, with no occlusion tracking
> anywhere, and a new frame's damage is the viewport's box — not a full repaint.
> A paint with no new frame (a hover elsewhere over a paused game) does not
> damage the viewport; it is repainted only where something else's damage
> reaches it — a box moving across it, an overlay above it closing — and then
> from the frame it already holds.
>
> It is not slower than the blit was. `submit_frame` checks, on the thread that
> submits, whether every pixel is opaque; an opaque frame is never
> premultiplied, and one that lands on whole pixels — at its natural size, or
> scaled to a box whose edges are whole pixels — is copied into the surface row
> by row, byte-identical to the sampled draw. Measured in release on one host
> (`game_viewport_inline_bench`), per frame on the UI thread, inline vs the old
> blit: a 1920×1080 frame in a 1920×1080 viewport 1.5 vs 4.8 ms, the same frame
> in a 960×540 viewport 2.3 vs 2.3 ms, a 320×180 frame in 640×360 0.7 vs 0.8 ms.
> A frame with any translucent pixel is premultiplied and sampled per draw, as
> any image is.
>
> What still tells video and `GameViewport` apart is the **hole**:
>
> - **Video** punches none: it is no compositor frame, so its name is absent from
>   the active-viewport set and #186's filter reads "absent ⇒ do not punch". The
>   node fills opaque black under the frame, and the `contain` fit plus that fill
>   *are* #354's letterbox bars on this backend.
> - **`GameViewport`** keeps its hole while it has a frame: it is the base layer
>   (above), so its box is cleared of everything painted beneath it and no black
>   is drawn, and where the frame does not reach — a letterbox, or pixels the
>   game left transparent — is the hole: see-through on a transparent window,
>   black on an opaque one.
>
> The two are told apart by an explicit flag set at video's registration site,
> **not** by the viewport's name — they share `create_render_surface_with_name`,
> so a naming rule would treat the game viewport as video. A surface carrying a
> GPU texture source never takes the inline path: it has no CPU pixels to paint,
> and is the GPU compositor's.

[#186]: https://github.com/joeleaver/rinch/issues/186
[#354]: https://github.com/joeleaver/rinch/issues/354
[#358]: https://github.com/joeleaver/rinch/issues/358
[#361]: https://github.com/joeleaver/rinch/issues/361

> **Overlays: which ancestor sizes them.** A `position: absolute` overlay is
> sized and placed against its **containing block**, as in a browser and under
> `rinch-web`: the padding box of its nearest positioned (or transformed)
> ancestor, however many unpositioned elements sit between them ([#386]), or
> the window when it has none ([#204]). So `position: absolute; inset: 0`
> inside `position: relative` chrome fills the chrome even when the element
> directly enclosing it has auto height, and dropped into unpositioned
> containers it fills the window.
>
> Two things to know. An overlay meant to cover one panel needs that panel to
> be `position: relative`, or it covers the next positioned ancestor up. And a
> `position: relative` **inline** element (a `<span>`) is a containing block
> too ([#631]): an overlay inside one resolves against the span's text, not
> the block that holds the line. On one line that is the text's box. For a
> span that **wraps**, the block runs from the first line's start of the text
> to the last line's end of it, so `inset: 0` is usually **zero wide** there
> (the last line ends left of where the first began), as in a browser. Make
> the wrapper `display: inline-block` (or a block) when the overlay should
> cover a box.

[#204]: https://github.com/joeleaver/rinch/issues/204
[#386]: https://github.com/joeleaver/rinch/issues/386
[#631]: https://github.com/joeleaver/rinch/issues/631

### Resize and Scale Factor

```rust
ctx.resize(new_width, new_height);
overlay.resize(&device, new_width, new_height);
ctx.set_scale_factor(window.scale_factor());
```

`set_scale_factor` also feeds Stylo's `device_pixel_ratio` (issue #211): CSS
`resolution` media queries (`@media (min-resolution: 2dppx)`), `image-set()`
candidate selection, and border-width device-pixel snapping all follow the
value you pass. A changed scale restyles the document on the next `update()`,
which also re-lays it out at the new logical size. The initial value comes
from `RinchContextConfig::scale_factor` and is live from the first layout.

Hit testing is transform-aware, so the usual `position: absolute; left: 50%;
top: 50%; transform: translate(-50%, -50%)` centering idiom is fully clickable —
a transformed subtree is hit where it is *painted*.

> **Caveat at HiDPI on the embed path.** The embed path resolves layout at logical
> size, so `set_scale_factor` genuinely matters here — and paint currently applies
> a CSS `translate` without multiplying it by that scale ([#202]). At any scale
> other than `1.0`, a **translated** element therefore paints slightly off from
> where it hit-tests. Only `translate` is affected (`rotate`/`scale`/`skew` are
> unit-invariant), and only on the embed path. Centering with flexbox instead of a
> `translate` sidesteps it entirely until that fix lands.

[#202]: https://github.com/joeleaver/rinch/issues/202

### Platform Events

Translate your engine's events to `rinch_platform::PlatformEvent`:

```rust
use rinch_platform::{PlatformEvent, MouseButton, KeyCode, KeyRepeat, Modifiers};

PlatformEvent::MouseMove { x: 100.0, y: 200.0 }
PlatformEvent::MouseDown { x: 100.0, y: 200.0, button: MouseButton::Left }
PlatformEvent::MouseUp { x: 100.0, y: 200.0, button: MouseButton::Left }
PlatformEvent::MouseWheel { x: 100.0, y: 200.0, delta_x: 0.0, delta_y: -30.0 }
PlatformEvent::KeyDown {
    key: KeyCode::KeyA,
    // The layout-produced `KeyboardEvent.key` value, case-accurate ("A" under
    // Shift). `None` = unknown; the app then falls back to the physical `key`.
    logical_key: Some("a".into()),
    text: Some("a".into()),
    modifiers: Modifiers::default(),
    // Is this a fresh physical press, or the OS repeating a key still held
    // (issue #463)? Say `KeyRepeat::Repeat` for a re-sent held key, and
    // `KeyRepeat::Unknown` if your engine cannot tell — the runtime then
    // infers it from the press/release pair, as it always did. Answering
    // where you can is what keeps a swallowed release from stranding the
    // Enter/Space activation of a focused node.
    repeat: KeyRepeat::Fresh,
}
PlatformEvent::KeyUp {
    key: KeyCode::KeyA,
    // Spell it like the press, or a consumer pairing press with release by
    // key string ("is A still held") never matches.
    logical_key: Some("a".into()),
    modifiers: Modifiers::default(),
}
PlatformEvent::Resized { width: 1920, height: 1080 }
```

> **Pointer coordinates are logical (CSS) pixels, not physical ones** (issue
> #299). `MouseMove`/`MouseDown`/`MouseUp`/`MouseWheel`, `MouseWheel`'s
> `delta_x`/`delta_y`, and the `File*` positions are all in the space the
> document is laid out in and `hit_test` probes. A windowing system that reports
> physical pixels — winit does — must divide first, and
> `rinch_platform::to_logical_point(point, scale)` is the shared conversion:
>
> ```rust
> let (lx, ly) = rinch_platform::to_logical_point((position.x, position.y), window.scale_factor());
> events.push(PlatformEvent::MouseMove { x: lx as f32, y: ly as f32 });
> ```
>
> Forwarding the physical position instead displaces every click, hover, drag
> and scroll by the scale factor times its distance from the window origin — it
> is invisible at scale 1.0 and wrong everywhere else. Note this is the opposite
> of `resize(w, h)` and `RinchContextConfig`'s `width`/`height`, which stay
> **physical**; `scale_factor` is how rinch reconciles the two. `wants_mouse(x,
> y)` takes the same logical pair, so ask it with exactly what you put on the
> event. `examples/game-embed` does this.

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
| `wants_keyboard() -> bool` | True for a text input, the editor, an open `<select>` popup, a generic node that registered `on_key`, or a *keyboard-focused* (Tab/`NodeHandle::focus()`) generic node — **not** a plain button/link a mouse click merely focused (issue #548; the click-vs-Tab split follows `:focus-visible`) |
| `ime_state() -> ImeState` | Whether the focused target wants the platform IME, and its caret area (logical px) — see [Input methods](#input-methods-ime) |
| `take_ime_change() -> Option<ImeChange>` | The `Enable` / `Update` / `Disable` that takes the host's IME from what it last reported to `ime_state()` |
| `needs_update() -> bool` | True if UI needs repaint (including a due `next_wake` or a new `RenderSurface` frame) |
| `next_wake() -> Option<Instant>` | When to call `update()` again with no input (the caret blink), or `None` |
| `register_font(data)` | Register font data for text rendering |
| `app() / app_mut()` | Access the underlying RinchApp |

**RinchOverlayRenderer:**

| Method | Description |
|--------|-------------|
| `new(device, w, h, format)` | Create from game's wgpu device |
| `render(device, queue, &mut ctx) -> TextureView` | Render the context's UI to a texture, transparent inside each `GameViewport` (straight alpha). Call it in place of `ctx.scene()`. Before #1866 it took `ctx.scene()`. A new renderer on an existing context (after a device loss) draws the whole UI on its first call (#1976) |
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
- `examples/gpu-surface/` — A RenderSurface drawn with wgpu through `set_gpu_render_callback`, as an element in the page
- `examples/render-surface-demo/` — Painting app with canvas and navigator (RenderSurface)
- `examples/webgpu-surface-web/` — **(web)** WebGPU triangle in a rinch DOM UI — the app owns a wgpu `<canvas>` viewport (RenderSurface on `rinch-web`)
