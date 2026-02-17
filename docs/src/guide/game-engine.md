# Game Engine Integration

Rinch can be embedded into an existing game engine or custom render loop. Instead of rinch owning the window and GPU resources, your game creates them, and rinch provides UI as a Vello scene that you composite on top of your game content.

Enable with: `features = ["desktop"]` (included by default)

## Quick Start

```rust
use rinch::prelude::*;
use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};

#[component]
fn game_hud() -> NodeHandle {
    let health = use_signal(|| 100);
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
            theme: None, // uses default theme
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

        // Handle actions
        for action in &actions {
            match action {
                AppAction::SetCursor(cursor) => { /* set cursor */ },
                AppAction::Exit => return,
                _ => {}
            }
        }

        // Render game, then UI overlay
        game.render(&device, &queue);
        let ui_texture = overlay.render(&device, &queue, ctx.scene());
        composite(&device, &queue, &surface, game_texture, ui_texture);
    }
}
```

## Core Concepts

### RinchContext

`RinchContext` is the main handle to an embedded rinch UI. You create it once during initialization and interact with it each frame.

```rust
use rinch::embed::{RinchContext, RinchContextConfig};

let mut ctx = RinchContext::new(
    RinchContextConfig {
        width: 1280,
        height: 720,
        scale_factor: 1.0,
        theme: Some(ThemeProviderProps {
            primary_color: Some("cyan".into()),
            dark_mode: true,
            ..Default::default()
        }),
    },
    my_ui_component,
);
```

### Per-Frame Update

Call `update()` once per frame with the platform events that occurred since the last frame:

```rust
let actions = ctx.update(&platform_events);
```

This processes input (mouse, keyboard), updates reactive state, resolves layout, and returns `AppAction`s your game should handle (cursor changes, exit requests, etc.).

### Reading the Scene

After `update()`, call `scene()` to get the Vello scene:

```rust
let scene: &vello::Scene = ctx.scene();
```

The scene is lazily rebuilt only when something changed. You can render it with `RinchOverlayRenderer` or your own Vello pipeline.

## Two Integration Patterns

### Full Overlay (HUD)

Rinch covers the entire window as a transparent overlay. Use input routing to decide what goes to the game vs the UI:

```rust
// Check where mouse input should go
if ctx.wants_mouse(mouse_x, mouse_y) {
    // Mouse is over a UI element -- route to rinch
    ctx.update(&[mouse_event]);
} else {
    // Mouse is over game content -- route to game
    game.handle_mouse(mouse_x, mouse_y);
}

// Check keyboard routing
if ctx.wants_keyboard() {
    // A text input is focused -- route keyboard to rinch
    ctx.update(&[key_event]);
} else {
    // No text input focused -- route to game
    game.handle_key(key);
}
```

### Split Layout (Viewport Hole)

Rinch renders toolbars and panels around a `GameViewport` component that marks where the game should render:

```rust
use rinch::embed::GameViewport;

#[component]
fn editor_ui() -> NodeHandle {
    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100%;",
            div { class: "toolbar",
                Button { onclick: || save(), "Save" }
                Button { onclick: || undo(), "Undo" }
            }
            div { style: "display: flex; flex: 1;",
                div { style: "width: 200px;",
                    // Side panel with tools
                }
                GameViewport { name: "main", style: "flex: 1;" }
            }
            div { class: "status-bar", "Ready" }
        }
    }
}
```

Then query the viewport's computed rect to know where to render your game:

```rust
if let Some(rect) = ctx.viewport_rect("main") {
    // rect.x, rect.y, rect.width, rect.height in logical pixels
    game.set_viewport(rect.x, rect.y, rect.width, rect.height);
}
```

## Input Routing

### wants_mouse

`wants_mouse(x, y)` hit-tests the rinch DOM at the given point. It returns `true` if the point hits a UI element, and `false` if it hits a `GameViewport` hole or empty space.

```rust
if ctx.wants_mouse(x, y) {
    // Route click/hover to rinch
} else {
    // Route to game (camera, selection, etc.)
}
```

### wants_keyboard

`wants_keyboard()` returns `true` when a text input or contenteditable element is focused. Use this to prevent game shortcuts from firing while the user is typing in a UI field.

```rust
if ctx.wants_keyboard() {
    // Let rinch handle keyboard (user is typing)
} else {
    // Handle game shortcuts (WASD, etc.)
}
```

## RinchOverlayRenderer

A convenience helper that renders a Vello scene to a GPU texture. If you already have your own Vello setup, you can skip this and render `ctx.scene()` directly.

```rust
use rinch::embed::RinchOverlayRenderer;

// Create from your game's device
let mut overlay = RinchOverlayRenderer::new(
    &device, width, height, TextureFormat::Rgba8Unorm,
);

// Each frame: render UI to texture
let ui_view = overlay.render(&device, &queue, ctx.scene());

// Composite the TextureView over your game scene
// (your compositor shader samples this with alpha blending)

// On resize:
overlay.resize(&device, new_width, new_height);
```

The overlay renders with a transparent background, so you can alpha-blend it on top of your game.

## Resize and Scale Factor

Notify rinch when the window size or DPI changes:

```rust
// Physical pixel dimensions
ctx.resize(new_width, new_height);
overlay.resize(&device, new_width, new_height);

// DPI scale factor
ctx.set_scale_factor(window.scale_factor());
```

## Fonts

In environments without system fonts (WASM, embedded), register fonts explicitly:

```rust
static FONT_DATA: &[u8] = include_bytes!("../assets/Inter-Regular.ttf");
ctx.register_font(FONT_DATA);
```

## Dirty Checking

For games that want to skip UI rendering on unchanged frames:

```rust
if ctx.needs_update() {
    let actions = ctx.update(&events);
    let scene = ctx.scene();
    overlay.render(&device, &queue, scene);
}
```

## Platform Events

Rinch uses `rinch_platform::PlatformEvent` for input. You need to translate your engine's native events to these:

```rust
use rinch_platform::{PlatformEvent, MouseButton, KeyCode, Modifiers};

// Mouse move
PlatformEvent::MouseMove { x: 100.0, y: 200.0 }

// Mouse click
PlatformEvent::MouseDown { x: 100.0, y: 200.0, button: MouseButton::Left }
PlatformEvent::MouseUp { x: 100.0, y: 200.0, button: MouseButton::Left }

// Mouse wheel
PlatformEvent::MouseWheel { x: 100.0, y: 200.0, delta_x: 0.0, delta_y: -30.0 }

// Key press
PlatformEvent::KeyDown {
    key: KeyCode::KeyA,
    text: Some("a".into()),
    modifiers: Modifiers::default(),
}

// Window resize
PlatformEvent::Resized { width: 1920, height: 1080 }
```

## API Reference

### RinchContext

| Method | Description |
|--------|-------------|
| `new(config, component)` | Create and mount a rinch UI |
| `update(&events) -> Vec<AppAction>` | Process events, update layout, return actions |
| `scene() -> &Scene` | Get the Vello scene (lazy rebuild) |
| `resize(w, h)` | Notify of window resize (physical pixels) |
| `set_scale_factor(scale)` | Update DPI scale factor |
| `viewport_rect(name) -> Option<LayoutRect>` | Query a GameViewport's computed rect |
| `wants_mouse(x, y) -> bool` | True if point hits UI (not viewport hole) |
| `wants_keyboard() -> bool` | True if a text input is focused |
| `needs_update() -> bool` | True if UI needs repaint |
| `register_font(data)` | Register font data for text rendering |
| `app() / app_mut()` | Access the underlying RinchApp |

### RinchOverlayRenderer

| Method | Description |
|--------|-------------|
| `new(device, w, h, format)` | Create from game's wgpu device |
| `render(device, queue, scene) -> TextureView` | Render scene to texture |
| `resize(device, w, h)` | Resize render target |
| `texture()` | Get the underlying wgpu Texture |

### RinchContextConfig

| Field | Type | Description |
|-------|------|-------------|
| `width` | `u32` | Initial viewport width (physical pixels) |
| `height` | `u32` | Initial viewport height (physical pixels) |
| `scale_factor` | `f64` | Display scale factor |
| `theme` | `Option<ThemeProviderProps>` | Theme configuration |

### LayoutRect

| Field | Type | Description |
|-------|------|-------------|
| `x` | `f32` | Absolute X position (logical pixels) |
| `y` | `f32` | Absolute Y position (logical pixels) |
| `width` | `f32` | Width (logical pixels) |
| `height` | `f32` | Height (logical pixels) |
