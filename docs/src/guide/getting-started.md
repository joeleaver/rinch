# Getting Started

From zero to window in about ninety seconds.

## Prerequisites

- **Rust nightly** (Rinch uses a few unstable features — `let_chains`, etc.)
- A C/C++ compiler (Stylo has native dependencies)
- On Linux: `libfontconfig-dev` and `pkg-config` (for font discovery)

## Create a Project

```bash
cargo new my-app
cd my-app
```

## Add Rinch

```toml
# Cargo.toml
[dependencies]
rinch = { git = "https://github.com/joeleaver/rinch.git", features = ["desktop", "components", "theme"] }
```

**What the features do:**
- `"desktop"` — windowing, event loop, software renderer. Required for desktop apps.
- `"components"` — the 60+ component library (Button, TextInput, Modal, etc.).
- `"theme"` — CSS variable generation, color palettes, dark mode. Auto-enabled by `"components"`.
- `"gpu"` — GPU rendering via Vello/wgpu instead of the software renderer. Optional.
- `"clipboard"` — clipboard read/write. Optional.
- `"file-dialogs"` — native open/save dialogs. Optional.

## Write Your App

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        Stack { gap: "md", p: "xl",
            Title { order: 1, "Hello, Rinch!" }
            Text { color: "dimmed", "Your first app. It only goes up from here." }

            Group { gap: "sm",
                Button { onclick: move || count.update(|n| *n += 1), "+" }
                Button { variant: "outline", onclick: move || count.set(0), "Reset" }
            }

            Text { size: "xl", {|| format!("Count: {}", count.get())} }
        }
    }
}

fn main() {
    App::new(app)
        .title("My App")
        .size(800, 600)
        .theme(ThemeProviderProps {
            primary_color: Some("cyan".into()),
            ..Default::default()
        })
        .run();
}
```

## Run It

```bash
cargo run --release
```

> **Always use `--release`** for running apps. Debug mode is slow because Stylo and Parley do serious work. Release builds are fast.

You should see a window with a title, description, two buttons, and a count that updates when you click "+".

## What Just Happened

1. `#[component]` injected a `__scope: &mut RenderScope` parameter. You never see it, but `rsx!` needs it.
2. `rsx!` built a DOM tree: created elements, set attributes, wired event handlers.
3. `{|| format!("Count: {}", count.get())}` created an Effect that reads `count` and updates a text node. When `count` changes, that closure re-runs and updates *only that text node*.
4. `App::new(app)…run()` opened a window, loaded theme CSS, mounted the component, and started the event loop. `App` is the one entry point: every startup option — title, size, theme, menu bar, window props, GPU device — is a method on it, and they all compose.

Your `app` function ran exactly once. It will never run again. The closures handle everything from here.

## What's Next

- [RSX Syntax](./rsx-syntax.md) — The full macro syntax: elements, attributes, events, control flow.
- [State Management](./hooks.md) — Signals, Memos, Stores, Context.
- [Components](./components.md) — The full component library.
- [Theming](./theming.md) — Colors, dark mode, CSS variables.
- [Writing Components](./writing-components.md) — Build your own.
- [WASM](./wasm.md) — Run in the browser.
