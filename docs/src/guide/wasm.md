# Running on WASM

Rinch compiles to WebAssembly with a browser-native DOM backend. Instead of painting pixels to a canvas, the WASM build creates real `<div>`, `<span>`, and `<button>` elements. The browser handles layout, CSS, text rendering, and painting natively.

The result: ~3MB binary, no JavaScript framework, real DOM elements you can inspect in Chrome DevTools.

## How It Works

The magic is in the `DomDocument` trait. On desktop, `NodeHandle` talks to `RinchDocument` (Stylo + Taffy + Parley + Vello/tiny-skia). On WASM, it talks to `WebDocument` (web_sys). Your components, signals, effects, and stores don't know or care which one they're using.

```
Desktop: Signal -> Effect -> NodeHandle -> RinchDocument -> tiny-skia pixels
WASM:    Signal -> Effect -> NodeHandle -> WebDocument   -> real browser DOM
```

Same `rsx!`. Same `Signal::new()`. Same `Button { "Click me" }`. Different backend.

## Project Setup

WASM targets live outside the main workspace because Parley's fontconfig dependency doesn't cross-compile to wasm32. Structure your project like this:

```
my-app/              # Shared library: components, stores, logic
my-app-desktop/      # Desktop entry point
my-app-web/          # WASM entry point (separate workspace)
```

### The Shared Library

```toml
# my-app/Cargo.toml
[package]
name = "my-app"

[dependencies]
rinch = { git = "...", default-features = false, features = ["components", "theme"] }
```

```rust
// my-app/src/lib.rs
use rinch::prelude::*;

#[component]
pub fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        Stack { gap: "md", p: "xl",
            Title { order: 1, "My App" }
            Button { onclick: move || count.update(|n| *n += 1),
                {|| format!("Clicked {} times", count.get())}
            }
        }
    }
}
```

### The Desktop Entry Point

```toml
# my-app-desktop/Cargo.toml
[dependencies]
rinch = { git = "...", features = ["desktop", "components", "theme"] }
my-app = { path = "../my-app" }
```

```rust
// my-app-desktop/src/main.rs
use rinch::prelude::*;

fn main() {
    run("My App", 800, 600, my_app::app);
}
```

### The WASM Entry Point

```toml
# my-app-web/Cargo.toml
[package]
name = "my-app-web"

[dependencies]
rinch = { git = "...", default-features = false, features = ["web", "components", "theme"] }
my-app = { path = "../my-app" }
wasm-bindgen = "0.2"
console_error_panic_hook = "0.1"
```

```rust
// my-app-web/src/main.rs
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    // Mount to browser DOM — see examples/ui-zoo-web for the full pattern
}
```

## Building

### With Trunk (Recommended)

[Trunk](https://trunkrs.dev/) handles WASM compilation, asset bundling, and dev server:

```bash
cargo install trunk

cd my-app-web
trunk serve --release --port 8080
```

Add an `index.html`:

```html
<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body></body>
</html>
```

Trunk does the rest.

### With wasm-pack

```bash
cd my-app-web
wasm-pack build --target web --release
```

### Manual

```bash
cd my-app-web
cargo build --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/my_app_web.wasm --out-dir pkg --target web
```

## What Works

Everything that goes through `NodeHandle` works:

- Signals, Effects, Memos, derived state
- Stores and Context
- All 60+ components
- The theme system and CSS variables
- Event handling (onclick, oninput, etc.)
- Reactive control flow (if, for, match)
- CSS shorthand props

The abstraction is clean. If your component code doesn't import anything from `rinch-dom` or `winit` directly, it'll work on WASM without changes.

## What Doesn't (Yet)

- **Custom painting** — Vello and tiny-skia don't run in the browser. The browser paints for you instead.
- **Game engine embedding** — `RenderSurface` and `RinchContext` are desktop-only.
- **Native menus** — Browsers have their own menu system. Use Rinch components instead.
- **File dialogs** — Use the browser's `<input type="file">` or the File System Access API.
- **System tray** — Not a thing in browsers.

## Theme in WASM

The theme system works by injecting a `<style>` tag into `<head>`. Dynamic theme changes (primary color, dark mode) work via reactive props:

```rust
let dark = Signal::new(false);

// ThemeProviderProps with reactive dark_mode
let theme = ThemeProviderProps {
    dark_mode_fn: Some(Rc::new(move || dark.get())),
    ..Default::default()
};
```

## Binary Size

A full UI Zoo demo (12 sections, 60+ components) compiles to ~3.3MB after `wasm-opt`. A minimal app is well under 500KB. The Rust compiler's dead code elimination is doing real work here — unused components, icons, and CSS don't make it into the binary.

## Reference

See `examples/ui-zoo-web` in the repo for a complete, working WASM app with sidebar navigation, theme switching, and all components.
