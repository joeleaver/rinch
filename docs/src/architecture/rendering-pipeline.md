# Rendering Pipeline

Rinch uses a multi-stage rendering pipeline that transforms component code into GPU-rendered pixels on the desktop backend. The web backend uses browser-native DOM instead (see note at the end).

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
│  (BlitzDocumentAdapter wraps blitz-dom on desktop)            │
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
│                   5. Painting                                   │
│  blitz-paint generates paint commands for the layout          │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   6. Scene Construction                         │
│  Commands are converted to a Vello scene graph                │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────────────────────────────────────────────┐
│                   7. GPU Rendering                              │
│  Vello renders the scene using wgpu                           │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                          Display
```

## Input Stage

User code defines components using the `#[component]` macro and `rsx!` macro:

```rust
#[component]
fn counter() -> NodeHandle {
    let count = use_signal(|| 0);
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

### Blitz (Internal Dependencies)

Blitz is a modular HTML/CSS rendering engine used internally by the desktop backend. These are not user-facing crates:

- **blitz-dom** - DOM implementation with Stylo integration (wrapped by `BlitzDocumentAdapter`)
- **blitz-traits** - Shared traits for rendering backends
- **blitz-paint** - Converts styled DOM to paint commands
- **blitz-html** - HTML parser (used for initial document setup, not for reactive updates)

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

### Vello

A GPU-accelerated 2D graphics library:

- Scene graph-based rendering
- Efficient batching
- High-quality anti-aliasing
- Path rendering (beziers, fills, strokes)
- Text rendering with proper shaping

### wgpu

Cross-platform GPU abstraction:

- Works on Vulkan, Metal, DX12, WebGPU
- Handles surface creation and management
- Provides compute shaders for Vello

## Window Rendering Flow

```rust
// Simplified rendering flow in window_manager.rs

impl ManagedWindow {
    fn paint_scene(&mut self) {
        // 1. Get the document's scene from blitz
        let scene = self.doc.render();

        // 2. Set up render parameters
        let params = RenderParams {
            width: self.size.width,
            height: self.size.height,
            base_color: Color::WHITE,
            antialiasing: AaConfig::default(),
        };

        // 3. Submit to Vello renderer
        self.renderer.render_to_surface(&scene, &params, &self.surface);
    }
}
```

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
| Paint | O(visible) | Command cache |
| GPU Render | O(primitives) | GPU buffers |

## Web Backend

The pipeline above is **desktop-only**. The web backend (`ui-zoo-web`) takes a completely different path:

```
#[component] + rsx! → DOM construction code → WebDocument (web_sys) → Browser-native DOM
```

On the web, `WebDocument` implements `DomDocument` using `web_sys` to create real browser DOM elements. The browser handles style resolution, layout, painting, and compositing natively. No Taffy, Parley, Stylo, Vello, or wgpu are needed for the web backend, resulting in a much smaller WASM binary.

## Future Optimizations

Planned improvements to the rendering pipeline:

- **Dirty tracking** - Only re-style/re-layout changed subtrees
- **Layer compositing** - GPU layers for transformed content
- **Text caching** - Glyph atlas for repeated text
- **Viewport culling** - Skip off-screen content
