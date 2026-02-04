# Rinch

Rinch is a lightweight, cross-platform GUI library for Rust that combines the power of web technologies with native performance.

> **[UI Zoo Live Demo](https://joeleaver.github.io/rinch/ui-zoo/)** — Try all Rinch widgets in your browser (requires WebGPU). Or run locally: `cargo run -p ui-zoo`

## Philosophy

- **Declarative UI** - Define your UI as a function of state using RSX syntax
- **Fine-grained Reactivity** - Only update what changed, not the entire UI
- **Web Standards** - Use HTML/CSS for layout, familiar to web developers
- **Native Performance** - GPU-accelerated rendering via Vello, native menus via muda
- **Cross-platform** - Windows, macOS, and Linux from a single codebase

## Quick Example

```rust
use rinch::prelude::*;

fn app(__scope: &mut RenderScope) -> NodeHandle {
    let count = use_signal(|| 0);
    let count_inc = count.clone();

    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count_inc.update(|n| *n += 1), "+" }
        }
    }
}

fn main() {
    run("Counter", 800, 600, app);
}
```

## Features

- **RSX Macro** - JSX-like syntax for building UI
- **Hooks (React-style)** - use_signal, use_effect, use_memo, and more
- **Fine-grained Reactivity** - Surgical DOM updates with signals and effects
- **Theme System (Mantine-inspired)** - CSS variables, color palettes, spacing scales
- **80+ Widgets** - Buttons, inputs, modals, dropdowns, and more
- **Rich-Text Editor** - Full-featured text editing with selections and formatting
- **5000+ Tabler Icons** - Type-safe SVG icons from tabler.io
- **Native Menus** - Platform-native menu bars via muda
- **GPU Rendering** - Fast 2D rendering via Vello and wgpu

## Architecture

Rinch is built on top of several excellent Rust crates:

- [rinch-dom](https://github.com/joeleaver/rinch/tree/main/crates/rinch-dom) - Custom HTML/CSS DOM implementation
- [Stylo](https://github.com/servo/stylo) - CSS parsing and computed styles (from Servo/Firefox)
- [Parley](https://github.com/linebender/parley) - Text layout and shaping
- [Vello](https://github.com/linebender/vello) - GPU 2D rendering
- [winit](https://github.com/rust-windowing/winit) - Cross-platform windowing
- [muda](https://github.com/tauri-apps/muda) - Native menu support
