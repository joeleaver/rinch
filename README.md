<p align="center">
  <img src="assets/logo.png" alt="Rinch" width="400">
</p>

<h1 align="center">Rinch</h1>

<p align="center"><strong>A GUI framework for Rust that doesn't hate you.</strong></p>

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
    App::new(app).title("Counter").size(400, 300).run();
}
```

That `{|| ...}` closure is doing all the work. It creates an Effect that tracks which signals it reads, then surgically updates its text node when they change. No virtual DOM. No diffing. No reconciler. Just a function pointer and a node reference.

## Why Not [Other Framework]?

| Complaint | Rinch's answer |
|-----------|---------------|
| "I have to learn a custom layout system" | It's CSS. You already know it. Flexbox via Taffy, style resolution via Servo's Stylo engine. |
| "Reactivity requires re-rendering the whole component" | Signals -> Effects -> surgical node updates. Component runs once. |
| "No component library" | 60+ components. Buttons, inputs, modals, tabs, accordions, color pickers, rich text editors. |
| "Styling is painful" | Theme system with CSS variables, 14 color palettes, dark mode, spacing scales. Write `p: "md"` instead of `padding: var(--rinch-spacing-md)`. |
| "I can't inspect anything" | F12 opens DevTools. Alt+I for inspect mode. There's an MCP server so Claude can screenshot your app and fix your CSS. |
| "Text rendering is bad" | Parley for shaping, HarfBuzz under the hood. Ligatures, BiDi, the works. |
| "No web target" | Compiles to WASM with a browser-native DOM backend — no canvas, just real DOM nodes. 3MB binary. |

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

**Platform Integration.** Native menus via muda. File dialogs. Clipboard. System tray with minimize-to-tray. Transparent borderless windows with custom chrome. Keyboard shortcuts.

**Rich Text Editing.** CRDT-backed editor (yrs), 22 formatting extensions, markdown input rules, syntax highlighting, find & replace. Not a toy.

**Game Engine Embedding.** Two modes: `RenderSurface` (Rinch owns the window, your renderer submits frames) or `RinchContext` (your engine owns the window, Rinch produces a Vello scene you composite). Either way, Rinch handles the UI and gets out of your way.

**Developer Tooling.** F12 DevTools panel. Layout debug overlay. Inspect mode. An MCP debug server that lets Claude Code take screenshots of your running app, inspect the DOM, simulate clicks, and help you fix things. Yes, really.

## Getting Started

```toml
[dependencies]
rinch = { git = "https://github.com/joeleaver/rinch.git", features = ["desktop", "components", "theme"] }
```

Add `"gpu"` for GPU rendering, or leave it off for software rendering (no GPU required — works in CI, containers, SSH sessions, your grandma's laptop).

```bash
# Run the component showcase
cargo run --release -p ui-zoo-desktop

# Run the todo app example
cargo run --release -p todo-app

# Build your own app
cargo run --release
```

> **Use `--release`.** Debug mode is noticeably slow because Stylo and Parley do a lot of work. Release builds are fast.

**[Documentation](https://joeleaver.github.io/rinch/)** — Getting started, RSX syntax, state management, components, theming, WASM, and more. ([API reference](https://joeleaver.github.io/rinch/api/rinch/index.html))

## Writing Your Own Components

There are two ways to make components in Rinch, and both of them are less painful than you'd expect.

### The Easy Way: PascalCase Functions

Write a function with a PascalCase name and `#[component]`, and Rinch generates a struct, a `Default` impl, and a `Component` trait impl for you. Parameters become props. That's it.

```rust
use rinch::prelude::*;

#[component]
pub fn StatusCard(
    title: String,
    count: i32,
    color: String,
    onclick: Option<Callback>,
) -> NodeHandle {
    rsx! {
        div { class: "status-card", style: {format!("border-left: 3px solid {color}")},
            h3 { {title.clone()} }
            span { class: "count", {count.to_string()} }
        }
    }
}
```

Now use it like any built-in component:

```rust
rsx! {
    StatusCard {
        title: "Active Users",
        count: 42,
        color: "var(--rinch-primary-color)",
        onclick: || println!("clicked!"),
    }
}
```

**The rules:**
- PascalCase name -> struct generation. Lowercase name -> just injects `__scope`.
- Props must be owned types (`String`, not `&str`). The macro tells you if you get this wrong.
- `children: &[NodeHandle]` is magic — it's not a struct field, it captures child elements from RSX.
- `onclick` and `oninput` closures are auto-wrapped into `Callback`/`InputCallback`. Don't manually wrap them.
- String literals become `String::from(...)`. Numbers get `Some(...)`. The macro handles the boring conversions.

### The Manual Way: Component Trait

For when you need more control — implement `Component` directly:

```rust
use rinch::prelude::*;

#[derive(Debug, Default)]
pub struct IconButton {
    pub icon: String,
    pub label: String,
    pub onclick: Option<Callback>,
}

impl Component for IconButton {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let btn = scope.create_element("button");
        btn.set_attribute("class", "icon-button");

        if let Some(cb) = &self.onclick {
            let handler_id = scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Create icon element, append children, etc.
        btn
    }
}
```

You probably don't need this. The PascalCase function handles 95% of cases. But it's there when the macro isn't enough.

### Making Components Reactive

Any prop accepts a closure `{|| expr}` to make it reactive:

```rust
let active = Signal::new(false);
rsx! {
    Button {
        variant: {|| if active.get() { "filled" } else { "outline" }},
        onclick: move || active.update(|v| *v = !*v),
        "Toggle Me"
    }
}
```

For surgical updates without re-rendering the whole component, use `_fn` props where available:

```rust
let text = Signal::new(String::new());
rsx! {
    TextInput {
        value_fn: move || text.get(),           // Surgical DOM update
        oninput: move |v: String| text.set(v),  // Update signal from input
    }
}
```

## Theming

Rinch's theme system is CSS-variable-based, Mantine-inspired, and designed to be extended rather than fought against.

### Quick Start

```rust
fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };
    App::new(app)
        .title("My App")
        .size(800, 600)
        .theme(theme)
        .run();
}
```

That's it. Every component automatically picks up your colors, radius, and spacing through CSS variables. Toggle `dark_mode: true` and the whole UI flips.

### What You Get

14 color palettes with 10 shades each, spacing scales (xs through xl), border radius scales, shadow scales, font size scales, and semantic color tokens that flip automatically in dark mode.

```css
/* Use these anywhere in your styles */
var(--rinch-primary-color)     /* Your chosen primary */
var(--rinch-color-blue-5)      /* Any palette, any shade */
var(--rinch-spacing-md)        /* 16px */
var(--rinch-radius-default)    /* Theme default radius */
var(--rinch-color-body)        /* Background (adapts to dark mode) */
var(--rinch-color-dimmed)      /* Secondary text */
```

### Extending the Theme

The theme generates CSS variables, and CSS variables are just strings. Override them with regular inline styles or class-based CSS, and Rinch won't even notice:

```rust
rsx! {
    div { style: "
        --rinch-primary-color: #ff6b6b;
        --rinch-radius-default: 0px;
    ",
        // Everything inside this div now has red primary and sharp corners
        Button { "I'm red and sharp" }
    }
}
```

For global customization, use `ThemeProviderProps`. For scoped overrides, just reassign the CSS variables on a container div.

### Programmatic Access

```rust
use rinch::theme::{Theme, ColorName};

let theme = Theme::builder()
    .primary_color(ColorName::Cyan)
    .dark_mode(true)
    .build();

let css = rinch_theme::generate_theme_css(&theme);
```

## WASM: Running in the Browser

Rinch compiles to WebAssembly with a browser-native DOM backend. Instead of painting pixels to a canvas, the WASM build creates real `<div>`, `<span>`, and `<button>` elements. The browser handles layout, CSS, text rendering, and painting. Your components, signals, and effects work exactly the same way — just pointed at `web_sys` instead of tiny-skia.

The result: ~3MB binary, no JavaScript framework, real DOM elements you can inspect in Chrome DevTools.

### Setup

The WASM entry point is its own crate and depends on `rinch-web`, the browser-native DOM backend:

```toml
# my-app-web/Cargo.toml
[dependencies]
rinch = { git = "...", default-features = false, features = ["components", "theme"] }
rinch-core = { git = "..." }
rinch-web = { git = "..." }   # browser-native DOM backend: WebDocument + mount
wasm-bindgen = "0.2"
console_error_panic_hook = "0.1"
```

```rust
// my-app-web/src/main.rs
use wasm_bindgen::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    // `mount` builds your component tree, wires browser event delegation,
    // and injects the theme CSS. See examples/ui-zoo-web for the full pattern.
    rinch_web::mount(ThemeProviderProps::default(), my_app::app);
}

fn main() {}
```

### Building

```bash
# Install trunk (WASM build tool)
cargo install trunk

# Build and serve
cd my-app-web
trunk serve --release --port 8080
```

Or use `wasm-pack` / `cargo build --target wasm32-unknown-unknown` if you prefer.

### What Works

Everything that goes through `NodeHandle` and `DomDocument` works automatically: signals, effects, memos, stores, contexts, components, the theme system, event handling. The abstraction boundary is clean — your app code doesn't know or care whether it's running on desktop or in Firefox.

### What Doesn't (Yet)

The browser backend doesn't support Rinch's custom painting features (Vello/tiny-skia), game engine embedding, or native menus. It's for standard UI — which is probably what you're building anyway.

### Code Sharing

Structure your app as a library crate with the UI logic, and two thin entry points:

```
my-app/          # Shared components, stores, logic
my-app-desktop/  # cargo run --release
my-app-web/      # trunk serve --release
```

The `ui-zoo` / `ui-zoo-desktop` / `ui-zoo-web` examples in the repo do exactly this.

## Project Status

Rinch is pre-1.0, under active development, and used by its author to build real applications. The API is stabilizing but not yet stable. Things that work well: layout, rendering, reactivity, the component library, text editing. Things that are still evolving: documentation, web target polish, test coverage.

If you want a GUI framework that's been blessed by a foundation and has a 200-page book, this isn't it yet. If you want one where the HTML and CSS knowledge you already have actually transfers, where reactivity doesn't require re-rendering the universe, and where you can ship a single native binary — pull up a chair.

## Examples

| Example | What it is |
|---------|-----------|
| `ui-zoo-desktop` | Component showcase — every widget, interactive |
| `todo-app` | Classic todo app with stores, filtering, reactive lists |
| `markdown-editor` | Rich text editor with CRDT backing |
| `drag-and-drop-demo` | Drag and drop between lists |
| `game-embed` | Game engine integration demo |
| `video-call` | WebRTC video calling |
| `paint-desktop` | Drawing application |

[**Live web demo**](https://joeleaver.github.io/rinch/ui-zoo/) (WASM, works in all modern browsers)

## Who's Using Rinch

**[PlotWeb](https://github.com/lost-conn/plotweb/)** — A gorgeous fiction writing platform that makes Google Docs look like Notepad. Full rich text editor, git-backed version history (every save is a commit!), Google Fonts integration with 1,500+ searchable fonts, per-book typography settings, and a warm dark mode that won't fry your retinas at 2 AM. The entire frontend is Rinch compiled to WASM — zero JavaScript. This is what "bet on the framework" looks like and it paid off beautifully.

**[gitrinching](https://github.com/lost-conn/gitrinching/)** — A commit graph visualizer that turns your repo history into an interactive node graph with lane-based layout and bezier merge curves. Click commits for details, drag to pan, scroll to zoom. Point it at a directory and it tiles every repo it finds into a grid. Uses the software renderer — no GPU needed, just pure CPU-powered git archaeology. The kind of tool you open "just to check something" and then lose an hour exploring your own history.

**[RKIField](https://github.com/joeleaver/rkifield)** — An SDF-based real-time graphics engine that threw out triangle meshes entirely. Every surface is a signed distance field in a sparse voxel brick pool, ray-marched through a 15-stage compute shader pipeline. Real-time GPU sculpting with CSG ops, volumetric fog/clouds/fire using the same SDF infrastructure as solid geometry, and physics queries against actual distance fields instead of proxy meshes. Rinch powers the editor UI. This project is absurdly ambitious and we are here for it.

**[Rorumall](https://github.com/joeleaver/rorumall)** — A native desktop chat client for the OFSCP federated chat protocol. Multi-server connections over WebSocket, Ed25519-signed API requests, role-based access control, clipboard image paste, Markdown rendering, and a presence system — basically Discord if Discord were a single Rust binary that respected your privacy. Seven full-screen views, six reactive stores, all built on Rinch signals. Proof that the framework scales from counter demos to real applications with real networking and real complexity.

Using rinch? [Open an issue](https://github.com/joeleaver/rinch/issues) or send a PR adding your project — we'd love to show it off.

## Architecture, Briefly

```
rinch              <- Facade crate. This is what you depend on.
rinch-core         <- Signal, Effect, Memo, NodeHandle, RenderScope
rinch-macros       <- rsx! macro, #[component] attribute
rinch-dom          <- Stylo + Taffy + Parley. The "browser engine" bits.
rinch-components   <- 60+ UI components
rinch-theme        <- CSS variable generation, color palettes
rinch-tabler-icons <- 5000+ icons, downloaded at build time
rinch-editor-core  <- Rich text editor model (pure, renderer-agnostic)
rinch-editor-view  <- Rich text editor view (shared by desktop and web)
rinch-debug        <- TCP IPC server for tooling
rinch-mcp-server   <- MCP server for Claude Code integration
```

The full layout is in `CLAUDE.md` if you want the gory details.

## Standing On Shoulders

[Stylo](https://github.com/nickel-org/stylo) (CSS resolution) · [Taffy](https://github.com/DioxusLabs/taffy) (flexbox) · [Parley](https://github.com/linebender/parley) (text) · [Vello](https://github.com/linebender/vello) (GPU rendering) · [tiny-skia](https://github.com/nickel-org/tiny-skia) (software rendering) · [winit](https://github.com/rust-windowing/winit) (windowing) · [muda](https://github.com/tauri-apps/muda) (menus) · [yrs](https://github.com/y-crdt/y-crdt) (CRDT)

## License

MIT OR Apache-2.0
