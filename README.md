# Rinch

A lightweight cross-platform GUI library for Rust with fine-grained reactive rendering.

Rinch provides a declarative UI framework using HTML/CSS for layout with two rendering backends: GPU-accelerated rendering via Vello/wgpu, and a software renderer via tiny-skia for environments without GPU access. Instead of virtual DOM diffing and full re-renders, Rinch uses fine-grained reactivity—signals and effects—to surgically update only the DOM nodes that changed.

## Architecture Highlights

**Fine-Grained Reactivity**

Signals and effects provide surgical DOM updates without full re-renders:

```
Signal.set() → Effect runs → NodeHandle.set_text() → Minimal re-layout
```

Unlike virtual DOM approaches, Rinch tracks which signals each reactive expression depends on. When a signal changes, only the Effects that read that signal re-run and update their target nodes.

**HTML/CSS Rendering**

- **Stylo** CSS engine for style computation
- **Taffy** layout engine for flexbox
- **Parley** for text shaping and layout
- **Vello** for GPU-accelerated 2D rendering via wgpu (GPU mode)
- **tiny-skia** for CPU-based 2D rendering (software mode)

**Platform Abstraction**

- **Desktop (GPU)**: winit for windowing, Vello + wgpu for rendering
- **Desktop (Software)**: winit for windowing, tiny-skia + softbuffer for rendering — no GPU required
- **Web**: WASM target ready (Stylo compiles to wasm32-unknown-unknown)

## Features

- **Fine-Grained Reactivity** — Signals, Effects, Memos. Only changed DOM nodes update.
- **55+ Components** — Mantine-inspired UI components (buttons, inputs, cards, navigation, overlays, etc.)
- **HTML/CSS Rendering** — Stylo CSS engine, Taffy flexbox, Parley text shaping
- **Reactive Primitives** — `Signal::new()`, `Effect::new()`, `Memo::new()`, `create_context()` with automatic dependency tracking
- **RSX Macro** — JSX-like syntax with reactive closures for declarative UIs
- **Theme System** — CSS variables, 20 color palettes, dark mode support, spacing/radius/typography scales
- **5000+ Icons** — Tabler Icons with type-safe enum API
- **Rich-Text Editor** — CRDT-backed editor (Automerge), 22 extensions, markdown shortcuts
- **Image Support** — `<img>` elements and `background-image` CSS, async loading, local files and HTTP(S) via optional `image-network` feature
- **Native Integration** — Menus (muda), file dialogs, clipboard, system tray
- **Transparent Windows** — Borderless frameless windows with custom chrome (Windows)
- **DevTools** — F12 inspector, layout debug overlay, performance stats
- **MCP Debug Server** — Screenshot, DOM inspection, input simulation for AI-assisted development
- **Cross-Platform** — Desktop (Windows, macOS, Linux) and WASM

## Quick Start

### Installation

Add to `Cargo.toml`:

```toml
[dependencies]
# GPU mode (recommended — requires GPU):
rinch = { git = "https://github.com/joeleaver/rinch.git", features = ["desktop", "gpu", "components", "theme"] }

# Software mode (no GPU needed — runs anywhere):
rinch = { git = "https://github.com/joeleaver/rinch.git", features = ["desktop", "components", "theme"] }
```

### Basic Counter Example

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            h1 { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    run("Counter", 400, 300, app);
}
```

**Key API Points:**

- Components use the `#[component]` attribute which auto-injects the render scope:
  ```rust
  #[component]
  fn app() -> NodeHandle {
      // ...
  }
  ```
  This expands to `fn app(__scope: &mut RenderScope) -> NodeHandle`. The manual signature also works.
- Entry point: `run("title", width, height, component_fn)`
- Reactive expressions use closure syntax: `{|| expr}` (without closure = captured once at initial render)
- `Signal` and `Memo` implement `Copy` — no `.clone()` needed before closures

### Component with Multiple Signals

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let name = Signal::new(String::from("World"));
    let count = Signal::new(0);

    rsx! {
        div {
            input {
                oninput: move |value: String| name.set(value),
                placeholder: "Enter your name"
            }
            h1 { "Hello, " {|| name.get()} "!" }
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    run("Hello App", 500, 300, app);
}
```

## Crate Structure

Rinch is organized as a workspace of specialized crates:

| Layer | Crates | Purpose |
|-------|--------|---------|
| **Core** | rinch, rinch-core, rinch-macros, rinch-dom | Foundation types, reactive primitives, rsx! macro, DOM implementation |
| **Platform** | rinch-platform, rinch-web | Platform abstraction traits, WASM backend |
| **UI** | rinch-components, rinch-theme, rinch-tabler-icons | 55+ components, theme system, 5000+ icons |
| **Editor** | rinch-editor, rinch-editor-macros, rinch-editor-components, rinch-editable | Rich-text editor with CRDT backing, editing utilities |
| **Tooling** | rinch-debug, rinch-mcp-server, rinch-clipboard | IPC debug server, Claude MCP integration, clipboard support |
| **Rendering** | rinch-renderer | GPU (Vello/wgpu) and software (tiny-skia/softbuffer) backends |

## Examples

### ui-zoo-desktop

A rich-text editor with formatting toolbar and markdown shortcuts:

```bash
cargo run -p ui-zoo-desktop
```

Features:
- CRDT-backed document (Automerge)
- 22 formatting extensions (bold, italic, strikethrough, code blocks, etc.)
- Markdown input rules (e.g., `# ` → heading)
- Syntax highlighting
- Find & replace

### ui-zoo

Interactive component showcase displaying all 55+ Rinch components.
[**Try the live demo**](https://joeleaver.github.io/rinch/ui-zoo/) (requires WebGPU) or run locally:

```bash
cargo run -p ui-zoo
```

Perfect for exploring the component library and theme customization.

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `F12` | Toggle DevTools window |
| `Alt+D` | Toggle layout debug overlay |
| `Alt+I` | Toggle inspect mode (hover highlight for element info) |
| `Alt+P` | Toggle performance stats console logging |
| `Alt+T` | Print Taffy layout tree to console |
| `Ctrl/Cmd + +/-/0` | Zoom in/out/reset |

## Documentation

- **Getting Started Guide**: https://joeleaver.github.io/rinch/
- **API Documentation**: Run `cargo doc --open`
- **Architecture Overview**: See `CLAUDE.md` in the repository

## Development Setup

```bash
# Clone the repository
git clone git@github.com:joeleaver/rinch.git
cd rinch

# Build all crates
cargo build

# Run the rich-text editor example
cargo run -p ui-zoo-desktop

# Run the component showcase
cargo run -p ui-zoo

# Build and open API docs
cargo doc --open

# Run tests
cargo test

# Lint and format
cargo clippy
cargo fmt
```

## Rendering Backends

Rinch supports two rendering backends, selected at compile time via Cargo features:

| Backend | Feature | Renderer | Presentation | Best For |
|---------|---------|----------|--------------|----------|
| **GPU** | `"gpu"` | Vello + wgpu | GPU compositing | Most apps — smooth animations, high-quality rendering |
| **Software** | (default) | tiny-skia | softbuffer | Headless, CI, servers, environments without GPU |

### Choosing a Backend

Set it in your `Cargo.toml` — no runtime flags needed:

```toml
# GPU mode (recommended for most apps):
rinch = { workspace = true, features = ["desktop", "gpu", "components", "theme"] }

# Software mode (default — no GPU required):
rinch = { workspace = true, features = ["desktop", "components", "theme"] }
```

Both backends use the same `Painter` trait abstraction, so your application code is identical regardless of backend. The software renderer includes dirty region caching — when only a small part of the UI changes (cursor blink, hover feedback, button click), only that region is repainted, making incremental updates fast even without a GPU.

### GPU Backend (Vello)

The GPU backend renders via [Vello](https://github.com/linebender/vello), a GPU-accelerated 2D graphics library built on wgpu. It provides smooth animations, efficient scene-graph rendering, and high-quality anti-aliased path rendering. Requires a GPU supporting Vulkan, Metal, DX12, or WebGPU.

### Software Backend (tiny-skia)

The software backend rasterizes to a CPU pixel buffer using [tiny-skia](https://github.com/nickel-org/tiny-skia) and presents via [softbuffer](https://github.com/rust-windowing/softbuffer). No GPU is needed. Features:

- Full rendering fidelity (same output as GPU mode)
- Dirty region caching for fast incremental updates
- Subtree pruning — unchanged parts of the DOM tree are skipped during paint
- Works in headless environments, CI, SSH sessions, containers

## Transparent Windows (Windows)

Rinch supports true window transparency on Windows via DX12 + DirectComposition, enabling VS Code-style frameless windows with custom chrome:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        BorderlessWindow {
            title: "My App",
            radius: "md",
            // Custom content
            div { "Hello from transparent window!" }
        }
    }
}

fn main() {
    run("Transparent App", 800, 600, app);
}
```

For custom window controls in your titlebar:

```rust
button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }
```

### Requirements

Transparent windows on Windows require a patched wgpu to enable Rgba8Unorm storage textures for Vello rendering with DX12. This is automatically applied via `[patch.crates-io]` in `Cargo.toml`:

- **Repository**: https://github.com/joeleaver/wgpu-fork
- **Branch**: `rinch-patch`
- **Upstream PR**: https://github.com/gfx-rs/wgpu/pull/8908

## AI-Assisted Development with MCP

Rinch integrates with Claude via an MCP (Model Context Protocol) debug server for AI-assisted development. Enable the `debug` feature and use MCP tools to:

- **Screenshot** — Capture and view rendered output inline
- **DOM Inspection** — Query the DOM tree with computed styles
- **Input Simulation** — Click, type, and trigger events
- **Performance Profiling** — Measure frame times and render performance

Configure in `.mcp.json`:

```json
{
  "mcpServers": {
    "rinch": {
      "command": "/path/to/rinch/target/debug/rinch-mcp-server",
      "args": [],
      "cwd": "/path/to/rinch"
    }
  }
}
```

Build the MCP server first:

```bash
cargo build -p rinch-mcp-server
```

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Run `cargo fmt` and `cargo clippy` before committing
4. Write tests for new functionality
5. Update documentation as needed

## License

MIT

## Acknowledgments

Rinch builds on excellent projects:

- [Stylo](https://github.com/servo/stylo) — CSS engine (via rinch-dom)
- [Taffy](https://github.com/DioxusLabs/taffy) — Flexbox layout engine
- [Parley](https://github.com/linebender/parley) — Text shaping and layout
- [Vello](https://github.com/linebender/vello) — GPU rendering
- [tiny-skia](https://github.com/nickel-org/tiny-skia) — Software rendering
- [softbuffer](https://github.com/rust-windowing/softbuffer) — Software presentation
- [winit](https://github.com/rust-windowing/winit) — Cross-platform windowing
- [muda](https://github.com/tauri-apps/muda) — Native menus
- [Automerge](https://automerge.org/) — CRDT for rich-text editor
- [Tabler Icons](https://tabler.io/icons) — Icon library
