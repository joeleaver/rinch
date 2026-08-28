# Rendering Pipeline

Rinch uses a multi-stage rendering pipeline that transforms component code into pixels on the desktop backend. Two rendering backends are available: GPU (Vello/wgpu) and software (tiny-skia/softbuffer). The web backend uses browser-native DOM instead (see note at the end).

## Pipeline Stages (Desktop)

```
┌───────────────────────────────────────────────────────────────┐
│                   1. Component Input                            │
│  #[component] functions + rsx! macro generate DOM construction │
│  code via __scope.create_element(), create_text(),             │
│  create_effect()                                               │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   2. DOM Construction                           │
│  DomDocument creates nodes programmatically via RenderScope   │
│  (RinchDocument uses Taffy + Parley on desktop)               │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   3. Style Resolution                           │
│  Stylo (Firefox's CSS engine) computes styles for each node   │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   4. Layout                                     │
│  Taffy computes the position and size of each element         │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   5. Painting (via Painter trait)                │
│  paint_document() walks the DOM tree and emits drawing        │
│  commands through the abstract Painter interface               │
└───────────────────────────────────────────────────────────────┘
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
┌──────────────────────────┐  ┌──────────────────────────┐
│  6a. GPU (Vello)         │  │  6b. Software (tiny-skia) │
│  Scene graph → wgpu →    │  │  Rasterize to RGBA pixmap │
│  GPU compositing         │  │  → softbuffer → display   │
└──────────────────────────┘  └──────────────────────────┘
                    │                   │
                    └─────────┬─────────┘
                              ▼
                          Display
```

## Input Stage

User code defines components using the `#[component]` macro and `rsx!` macro:

```rust
#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}
```

The `#[component]` macro injects a `__scope: &mut RenderScope` parameter. The `rsx!` macro generates calls to `__scope.create_element()`, `__scope.create_text()`, and `__scope.create_effect()` to build the DOM tree programmatically. No HTML strings are generated or parsed at runtime.

## Key Technologies

### rinch-dom

The custom DOM and layout engine built specifically for Rinch:

- **rinch-dom** - DOM implementation with Taffy layout, Parley text shaping, and a `Painter` trait for backend-agnostic rendering
- **VelloPainter** - GPU backend: records drawing commands into a Vello scene graph
- **TinySkiaPainter** - Software backend: rasterizes directly to an RGBA pixel buffer

### Stylo

Mozilla's CSS engine (from Firefox) provides:

- Full CSS specification support
- Efficient style computation
- Media query handling
- CSS custom properties

### Taffy

A flexbox/grid layout engine that computes:

- Element positions (x, y)
- Element sizes (width, height)
- Flexbox alignment and distribution
- CSS Grid support

### Painter Trait

All rendering goes through the `Painter` trait, which abstracts over both backends:

```rust
pub trait Painter {
    fn fill(&mut self, fill: Fill, transform: Affine, brush: &Brush, shape: &PaintShape);
    fn stroke(&mut self, stroke: &Stroke, transform: Affine, brush: &Brush, shape: &PaintShape);
    fn draw_glyphs(&mut self, font: &FontData, font_size: f32, ...);
    fn draw_image(&mut self, image: &PaintImage, transform: Affine);
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape);
    fn push_layer(&mut self, blend: BlendMode, opacity: f32, ...);
    fn pop_layer(&mut self);
    // ...
}
```

Application code never interacts with the Painter directly — the backend is selected at compile time via Cargo features.

### Vello (GPU Backend)

A GPU-accelerated 2D graphics library (enabled with `features = ["gpu"]`):

- Scene graph-based rendering
- Efficient batching
- High-quality anti-aliasing
- Path rendering (beziers, fills, strokes)
- Text rendering with proper shaping
- Requires wgpu (Vulkan, Metal, DX12, or WebGPU)

### tiny-skia (Software Backend)

A CPU-based rasterizer (the default when `gpu` is not enabled):

- Direct pixel rendering to an RGBA buffer
- Presented via softbuffer (no GPU required)
- Dirty region caching — only changed areas are repainted
- Subtree pruning — nodes outside the dirty region are skipped
- Works in headless environments, CI, containers, SSH sessions

## Rendering Backends

### Choosing a Backend

Set it in your `Cargo.toml`:

```toml
# GPU mode (recommended for most apps):
rinch = { workspace = true, features = ["desktop", "gpu"] }

# Software mode (default — no GPU required):
rinch = { workspace = true, features = ["desktop"] }
```

### GPU Rendering Flow

```rust
// Simplified GPU rendering flow
fn paint_gpu(&mut self) {
    let scene = self.app.build_scene(scale, size);
    self.renderer.render_to_surface(&scene, &params, &surface);
}
```

### Software Rendering Flow

```rust
// Simplified software rendering flow
fn paint_software(&mut self) {
    let (pixels, w, h) = self.app.build_pixels(scale, size, &layers);
    // pixels are blitted to the window via softbuffer
}
```

The software renderer includes **dirty region caching**: when only a small part of the UI changes (e.g., cursor blink, hover feedback), only the affected rectangular region is cleared and repainted. Nodes outside the dirty region are skipped entirely during the paint traversal.

The region is the union of the changed *nodes'* rects, so anything painted **outside** the node tree has to contribute its own. The drag ghost is the one such overlay: it is blitted into the framebuffer after the document paint, so `RinchApp` remembers the rect it covered and folds that into the next frame's dirty region — otherwise the frame that stops drawing the ghost would never clear where it had been, leaving it stuck on screen (issue #173). The GPU backend rebuilds the whole Vello scene every dirty frame and so has no equivalent case.

## Incremental Updates

When content changes, the pipeline can skip unchanged stages:

1. **Style cache** - Styles are cached per element selector
2. **Layout cache** - Layout is only recomputed for affected subtrees
3. **Scene diffing** - Only changed primitives are re-rendered

## Performance Characteristics

| Stage | Complexity | Caching |
|-------|------------|---------|
| DOM Build | O(n) | Incremental (surgical updates) |
| Style Resolve | O(n x rules) | Selector cache |
| Layout | O(n) | Subtree cache |
| Paint | O(visible) | Dirty region caching (software) |
| GPU Render | O(primitives) | GPU buffers (GPU mode) |

## Web Backend

The pipeline above is **desktop-only**. The web backend (`ui-zoo-web`) takes a completely different path:

```
#[component] + rsx! → DOM construction code → WebDocument (web_sys) → Browser-native DOM
```

On the web, `WebDocument` implements `DomDocument` using `web_sys` to create real browser DOM elements. The browser handles style resolution, layout, painting, and compositing natively. No Taffy, Parley, Stylo, Vello, or wgpu are needed for the web backend, resulting in a much smaller WASM binary.

## The frame clock

Every shell must send `PlatformEvent::AboutToWait` once per iteration of its
event loop. It is not an optimisation and it is not optional: it is the only
place `RinchApp` advances CSS transitions and CSS animations, marks the scene
dirty when either moved, resolves the dirty state the input handlers
deliberately leave for it to batch, and drains the focus requests effects
raise. The winit shell sends it from `ActiveEventLoop::about_to_wait`; the
Android shell sends it from `android_frame::pump_frame`, once per 16ms poll;
`embed::RinchContext` sends it from `update`.

A shell that omits it does not look broken. Every screen still paints and every
un-animated control still works — but a transitioned property is sampled once,
at the instant the transition starts, which is its *old* value, and stays there
for ever, while un-transitioned properties on the same element apply
immediately. A bottom sheet then answers a tap by becoming pointer-active
without moving: open, invisible, and covering the screen.

Turning the clock is only half of it. A transition on a paint-only property —
`opacity`, `transform`, a colour — marks its node `PAINT`-dirty and nothing
else, so `has_pending_layout()` stays false and no `RequestRedraw` is raised. A
shell decides whether to present from `RinchApp::scene_dirty`, which
`AboutToWait` sets whenever a transition or an animation moved; a shell that
does not consult it turns the clock in private, and the surface keeps the frame
from before the change until something unrelated forces a present.

And `scene_dirty` is set for every tick that had something to tick, not only
for ticks after which something is still running. A transition that *finishes*
on a tick applies its end value and then reports nothing active — and on a
shell whose first paint after the change is slower than the transition is long
(Android's is around 300ms against a typical 220ms), that is the only tick the
transition ever gets.

## Optimizations

Current and planned improvements to the rendering pipeline:

- **Dirty region caching** (software) - Only repaint the rectangular area covering changed nodes
- **Subtree pruning** (software) - Skip paint traversal for nodes outside the dirty region
- **Sensitivity flags** - Hover/active/focus only trigger repaints for nodes with matching CSS selectors
- **Batched redraws** - Multiple state changes are batched into a single repaint via the frame clock (`AboutToWait`, above)
- **Layer compositing** - GPU layers for transformed content (planned)
- **Text caching** - Glyph atlas for repeated text (planned)
- **Viewport culling** - Skip off-screen content (planned)
