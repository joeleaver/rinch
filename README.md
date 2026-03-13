# Rinch

**A GUI framework for Rust that doesn't hate you.**

Every few months, someone on r/rust asks "what's the state of GUI in Rust?" and the thread fills with the same apologetic answers: "it's getting better," "try [framework du jour]," "have you considered a web view?" Rinch exists because "getting better" has been the answer for seven years and we got tired of waiting.

## The Pitch

Rinch gives you HTML and CSS — the layout system that billions of people have already debugged for you — but renders it natively with Vello (GPU) or tiny-skia (CPU). No Electron. No web view. No 200MB runtime. Your app is a single binary that starts in milliseconds.

Reactivity is fine-grained. When a signal changes, Rinch updates *that one DOM node*. Not the component. Not the subtree. Not "the whole thing but we diff it so it's fine." The node. Your component function runs exactly once.

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            h1 { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}

fn main() {
    run("Counter", 400, 300, app);
}
```

That `{|| ...}` closure is doing all the work. It creates an Effect that tracks which signals it reads, then surgically updates its text node when they change. No virtual DOM. No diffing. No reconciler. Just a function pointer and a node reference.

## Why Not [Other Framework]?

| Complaint | Rinch's answer |
|-----------|---------------|
| "I have to learn a custom layout system" | It's CSS. You already mass it. Flexbox via Taffy, style resolution via Servo's Stylo engine. |
| "Reactivity requires re-rendering the whole component" | Signals → Effects → surgical node updates. Component runs once. |
| "No component library" | 60+ components. Buttons, inputs, modals, tabs, accordions, color pickers, rich text editors. |
| "Styling is painful" | Theme system with CSS variables, 20 color palettes, dark mode, spacing scales. Write `p: "md"` instead of `padding: var(--rinch-spacing-md)`. |
| "I can't inspect anything" | F12 opens DevTools. Alt+I for inspect mode. There's an MCP server so Claude can screenshot your app and fix your CSS. |
| "Text rendering is bad" | Parley for shaping, HarfBuzz under the hood. Ligatures, BiDi, the works. |
| "No web target" | Compiles to WASM. Browser-native DOM backend — no canvas, just real DOM nodes. 3MB binary. |

## What's In The Box

**Rendering.** Dual backend — GPU via [Vello](https://github.com/linebender/vello)/wgpu, or software via [tiny-skia](https://github.com/nickel-org/tiny-skia). Same code, same output, pick at compile time. The software renderer does dirty-region tracking, so even without a GPU, incremental updates are fast.

**60+ Components.** A Mantine-inspired component library that actually works: `Button`, `TextInput`, `Modal`, `Tabs`, `Accordion`, `Select`, `ColorPicker`, `Stepper`, `RichTextEditor`, and about fifty more. Plus 5,000+ [Tabler Icons](https://tabler.io/icons) with a type-safe enum API.

**Fine-Grained Reactivity.** `Signal`, `Memo`, `Effect` — all `Copy`, no `.clone()` ceremony. Cross-thread dispatch built in (`signal.send(value)` from any thread). Stores for shared state. Context for dependency injection.

**Native Rust Control Flow.** `if`, `for`, `match` in RSX — all automatically reactive. Keyed list reconciliation with the LIS algorithm. No `.map()` gymnastics.

```rust
let tab = Signal::new("home");
let todos = Signal::new(vec![Todo { id: 1, name: "Ship it".into() }]);
let user = Signal::new(Some("Alice".to_string()));

rsx! {
    div {
        // if — shows/hides reactively when the signal changes
        if let Some(name) = user.get() {
            p { "Welcome back, " {name} }
        }

        // match — switches between views
        match tab.get().as_str() {
            "home" => div { "Home sweet home" },
            "settings" => div { "Tweak away" },
            _ => div { "404, probably" },
        }

        // for — keyed list with minimal DOM ops on change
        for todo in todos.get() {
            div { key: todo.id, {todo.name.clone()} }
        }
    }
}
```

**Platform Integration.** Native menus via muda. File dialogs. Clipboard. System tray with minimize-to-tray. Transparent borderless windows with custom chrome (Windows). Keyboard shortcuts.

**Rich Text Editing.** CRDT-backed editor (Automerge), 22 formatting extensions, markdown input rules, syntax highlighting, find & replace. Not a toy.

**Game Engine Embedding.** Two modes: `RenderSurface` (Rinch owns the window, your renderer submits frames) or `RinchContext` (your engine owns the window, Rinch produces a Vello scene you composite). Either way, Rinch handles the UI and gets out of your way.

**Developer Tooling.** F12 DevTools panel. Layout debug overlay. Inspect mode. An MCP debug server that lets Claude Code take screenshots of your running app, inspect the DOM, simulate clicks, and help you fix things. Yes, really.

## Getting Started

```toml
[dependencies]
rinch = { git = "https://github.com/joeleaver/rinch.git", features = ["desktop", "gpu", "components", "theme"] }
```

Drop the `"gpu"` feature for software rendering (no GPU required — works in CI, containers, SSH sessions, your grandma's laptop).

```bash
# Run the component showcase
cargo run --release -p ui-zoo-desktop

# Run the todo app example
cargo run --release -p todo-app

# Build your own app
cargo run --release
```

> **Use `--release`.** Debug mode is noticeably slow because Stylo and Parley do a lot of work. Release builds are fast.

## Project Status

Rinch is pre-1.0, under active development, and used by its author to build real applications. The API is stabilizing but not yet stable. Things that work well: layout, rendering, reactivity, the component library, text editing. Things that are still evolving: documentation, web target polish, test coverage.

If you want a GUI framework that's been blessed by a foundation and has a 200-page book, this isn't it yet. If you want one where the HTML and CSS knowledge you already have actually transfers, where reactivity doesn't require re-rendering the universe, and where you can ship a single native binary — pull up a chair.

## Examples

| Example | What it is |
|---------|-----------|
| `ui-zoo-desktop` | Component showcase — every widget, interactive |
| `todo-app` | Classic todo app |
| `markdown-editor` | Rich text editor with CRDT backing |
| `drag-and-drop-demo` | Drag and drop between lists |
| `game-embed` | Game engine integration demo |
| `video-call` | WebRTC video calling |
| `paint-desktop` | Drawing application |

[**Live web demo**](https://joeleaver.github.io/rinch/ui-zoo/) (requires WebGPU)

## Architecture, Briefly

```
rinch              ← Facade crate. This is what you depend on.
rinch-core         ← Signal, Effect, Memo, NodeHandle, RenderScope
rinch-macros       ← rsx! macro, #[component] attribute
rinch-dom          ← Stylo + Taffy + Parley. The "browser engine" bits.
rinch-components   ← 60+ UI components
rinch-theme        ← CSS variable generation, color palettes
rinch-tabler-icons ← 5000+ icons, downloaded at build time
rinch-editor       ← Rich text editor core
rinch-debug        ← TCP IPC server for tooling
rinch-mcp-server   ← MCP server for Claude Code integration
```

The full layout is in `CLAUDE.md` if you want the gory details.

## Standing On Shoulders

[Stylo](https://github.com/nickel-org/stylo) (CSS resolution) · [Taffy](https://github.com/DioxusLabs/taffy) (flexbox) · [Parley](https://github.com/linebender/parley) (text) · [Vello](https://github.com/linebender/vello) (GPU rendering) · [tiny-skia](https://github.com/nickel-org/tiny-skia) (software rendering) · [winit](https://github.com/rust-windowing/winit) (windowing) · [muda](https://github.com/tauri-apps/muda) (menus) · [Automerge](https://automerge.org/) (CRDT)

## License

MIT OR Apache-2.0
