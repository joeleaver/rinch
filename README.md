# Rinch

A lightweight cross-platform GUI library for Rust, built on top of [blitz](https://github.com/DioxusLabs/blitz).

Rinch provides a reactive GUI framework using HTML/CSS for layout with a Vello-based GPU renderer.

## Features

- **Declarative UI** - React-style component model with hooks API
- **HTML/CSS Rendering** - Full HTML/CSS support via Stylo and Taffy
- **GPU Accelerated** - Fast 2D rendering via Vello and wgpu
- **Transparent Windows** - VS Code-style frameless windows with transparency (Windows)
- **Native Menus** - Cross-platform menu support via muda
- **Window Controls** - Programmatic minimize/maximize/close for custom chrome
- **DevTools** - Built-in developer tools for debugging

## Quick Start

```rust
use rinch::prelude::*;

fn app() -> Element {
    let count = use_signal(|| 0);
    let count_inc = count.clone();

    rsx! {
        Window { title: "Counter", width: 400, height: 300,
            div {
                h1 { "Count: " {count.get()} }
                button { onclick: move || count_inc.update(|n| *n += 1),
                    "Increment"
                }
            }
        }
    }
}

fn main() {
    rinch::run(app);
}
```

## Documentation

- [**Getting Started Guide**](https://joeleaver.github.io/wrinch/guide/getting-started.html)
- [**API Reference**](https://joeleaver.github.io/wrinch/api/rinch/)

## Transparent Windows (Windows)

Rinch supports true window transparency on Windows, enabling VS Code-style frameless windows with custom chrome:

```rust
Window {
    title: "My App",
    borderless: true,      // Remove native decorations
    transparent: true,     // Enable transparency
    // ... your custom titlebar and controls
}
```

For custom window controls, use the provided functions:

```rust
use rinch::prelude::*;

button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }
```

### wgpu Fork Requirement

Transparent windows on Windows require a patched version of wgpu to enable Rgba8Unorm storage textures for Vello rendering with DX12/DirectComposition. This is handled automatically via `[patch.crates-io]` in `Cargo.toml`:

```toml
[patch.crates-io]
wgpu = { git = "https://github.com/joeleaver/wgpu-fork", branch = "rinch-patch" }
wgpu-core = { git = "https://github.com/joeleaver/wgpu-fork", branch = "rinch-patch" }
wgpu-hal = { git = "https://github.com/joeleaver/wgpu-fork", branch = "rinch-patch" }
wgpu-types = { git = "https://github.com/joeleaver/wgpu-fork", branch = "rinch-patch" }
naga = { git = "https://github.com/joeleaver/wgpu-fork", branch = "rinch-patch" }
```

A PR has been submitted upstream: [gfx-rs/wgpu#8908](https://github.com/gfx-rs/wgpu/pull/8908)

## Development Setup

```bash
# Clone the repository
git clone git@github.com:joeleaver/rinch.git
cd rinch

# Build
cargo build

# Run the example editor
cargo run -p smyeditor

# Build documentation locally
cargo doc --open
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `F12` | Toggle DevTools window |
| `Alt+D` | Toggle layout debug overlay |
| `Alt+I` | Toggle inspect mode |
| `Alt+T` | Print Taffy layout tree |
| `Ctrl/Cmd + +/-/0` | Zoom in/out/reset |

## License

MIT
