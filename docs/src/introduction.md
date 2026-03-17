# Rinch

A GUI framework for Rust that uses HTML and CSS for layout, renders natively, and won't make you mass a single `.clone()` at a signal.

> **[Live Demo](https://joeleaver.github.io/rinch/ui-zoo/)** — All Rinch components, running in your browser via WASM. Or run locally: `cargo run --release -p ui-zoo-desktop`

## Philosophy

- **HTML/CSS layout** — The layout system that billions of people have already debugged for you, powered by Servo's Stylo and Taffy's flexbox.
- **Fine-grained reactivity** — Signal changes update *one DOM node*, not a component tree. No virtual DOM. No diffing.
- **Components run once** — Your function builds the DOM, closures keep it updated. That's the whole model.
- **Native performance** — GPU rendering via Vello/wgpu, or software rendering via tiny-skia. Pick at compile time.
- **WASM too** — Same components, same signals, browser-native DOM. ~3MB binary, zero JavaScript.

## Quick Example

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}

fn main() {
    run("Counter", 800, 600, app);
}
```

`Signal` is `Copy`. No `.clone()` before closures. The `{|| ...}` closure creates an Effect that tracks its dependencies and surgically updates that text node when `count` changes. The `app` function runs once, builds the DOM, and never runs again.

## What's Included

- **60+ Components** — Buttons, inputs, modals, tabs, accordions, color pickers, trees, a rich text editor, and more.
- **Theme System** — 14 color palettes, dark mode, CSS variables for everything. Mantine-inspired.
- **5,000+ Icons** — Tabler Icons with a type-safe enum. Dead code elimination drops the ones you don't use.
- **Native Platform** — Menus, file dialogs, clipboard, system tray, transparent windows.
- **Developer Tools** — F12 DevTools, inspect mode, MCP server for Claude Code integration.
- **Game Engine Embedding** — Submit GPU textures or CPU pixels into a Rinch UI, or embed Rinch into your own render loop.
