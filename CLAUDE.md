# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Keep this file short

This file is loaded into every session and every subagent, so its size is paid on every turn. It holds the orientation, the build and CI rules, and an index. **The detail lives in `docs/internals/`**, one page per area, read on demand. When you learn or change how something works, write it on the matching internals page (or a new one, added to the index below). Add to this file only something every task needs. Until 2026-10-22 everything was appended here, and the file reached 695 KB (about 175k tokens).

A code comment or test that cites "CLAUDE.md" and a section name ("Keyboard Focus", "App-bundled fonts", "Stacking contexts and the clip chain", …) means a section that now lives on one of the internals pages: `grep -rn '<name>' docs/internals/`.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on rinch-dom (an HTML/CSS DOM on Stylo, Taffy for layout, Parley for text) with two rendering backends: Vello on wgpu (GPU, `gpu` feature) and tiny-skia (software, always on desktop). Windowing is winit (a pinned fork, below), presentation softbuffer or wgpu, native menus muda. A second backend, `rinch-web`, drives the browser's own DOM through web_sys.

**Design philosophy:** declarative UI with fine-grained reactivity. A component function runs **once**; the `rsx!` macro generates DOM construction code, and `{|| expr}` closures become Effects that update single nodes. There are no re-renders.

Load-bearing dependency facts (detail and history in `docs/internals/dependencies.md`):
- **Taffy 0.14** is pinned in one place, `crates/rinch-dom/Cargo.toml`; its `[dependencies]` and `[dev-dependencies]` lines must agree, features included (the five default features stay off).
- **parley 0.12.0** with `features = ["complex-scripts", "bytemuck"]`; `complex-scripts` must stay on (without it Thai/Lao/Khmer/Burmese text has no line breaks). `parley::Glyph::y` is Y-down.
- **skrifa 0.44**, the version parley uses; move it with any parley bump.
- **wgpu 30** (crates.io) and **Vello 0.11 from `joeleaver/vello-fork` at a pinned rev** (#1955, image-atlas fixes; a dependency, not a patch). Apps use `rinch::wgpu` and `rinch::vello`. See `docs/internals/app-and-windows.md` "Vello Fork".
- **winit 0.31.0-beta.4 from `joeleaver/winit-fork` at a pinned rev** (#1962, the layer-shell role; a dependency, no `[patch]` lines anywhere). See `docs/internals/app-and-windows.md` "winit Fork".

## Where the detail lives

| Page (`docs/internals/`) | Read it when your task touches |
|---|---|
| `dependencies.md` | Taffy/Parley/skrifa pins and traps, line breaking, white space, tabs, NBSP, justify, text-overflow |
| `components.md` | the Tabler icon system, components' icon props, `List`/`Stepper`/`RadioGroup` container defaults, `on_child_inserted`/`on_child_removed` |
| `app-and-windows.md` | the `App` builder, app-bundled fonts, Wayland `app_id`, transparent/borderless windows, resize handles, `BorderlessWindow`, window control, hide/show geometry, overlay windows, window levels, layer shell, the winit fork, the Vello fork (and the removed wgpu fork) |
| `state-and-threading.md` | Signal/Memo/stores/context, memo equality cut-off, event handlers as batches, effect flushing, `note_host_change`, cross-thread `send`, the main-callback queue |
| `menus.md` | `Menu`/`MenuItem`, checkbox/radio items, callback lifetime, shortcut parsing and layout matching, the DOM menu bar, built-in shortcuts |
| `editor.md` | the rich-text editor: model, steps, history, persistence, embeds, node views, paste fitting, the HTML reader, image input, links, popup hooks, margin anchors, read-only, caret affinity, scroll gate, focus/reveal, commands, input rules, collaboration (yrs) |
| `pointer-drag-tooltips.md` | DOM drag attributes, touch activation, drag auto-scroll, the `Drag` builder, missed-release heals, button rules (#1087/#1093), backdrops, file drop, `title` tooltips |
| `focus-and-keyboard.md` | the focus arbiter, element key handlers, text selection API, focusability rules, press focus on web, `<select>` keys, the dismiss stack and key entries, `trap_focus`, Tab order, `data-nofocus`, focus targets, `lock_scroll`, the text context menu, key auto-repeat |
| `paint-and-damage.md` | the rendering backends and renderer choice, named damage, GPU partial frames, dirty regions and painted state, damage clip chains, the software painter's caches, scrollbars and scrollable overflow, viewport holes |
| `layout-and-style-invalidation.md` | layout invalidation paths, typography and atomic-inline re-measure, transitions writing styles, `display: contents` styles, resize restyle, style invalidation, text caches, the scoped structural IFC pass, `<body>` flex basis |
| `positioning.md` | absolute/fixed positioning, containing blocks, the static position, positioned inline spans, shrink-to-fit absolute boxes |
| `clipping-stacking-hit-testing.md` | `0x0` boxes, overflow clipping and the clip predicate, the paint cull, stacking contexts and clip chains, transforms, hit-test pruning and caching, scroll offsets on the device grid, fixed boxes in stacking order |
| `platform.md` | images and app image schemes, DevTools, the debug/MCP server and perf counters, file dialogs, clipboard, system tray, notifications, global hotkeys, the theme system, fonts and generic families, the window top inset |
| `game-engine.md` | `RenderSurface`, pointer capture on surfaces, GPU device sharing, web canvas surfaces, the embed API (`RinchContext`, IME, `wants_keyboard`) |
| `reactive-rendering.md` | `remove` vs `discard`, scope ownership of nodes, cache scopes, execution order, styles and animations across detach/hide, transitions and animations, transform interpolation, easing |
| `rsx-control-flow-and-props.md` | `if`/`for`/`match` in rsx, keyed reconciliation, `_fn` bindings and the echo guard, undo/caret keys in fields, `onsubmit`, rsx prop transformation rules, boolean attributes, merged `style:`/`class:`, root attributes on components |
| `common-issues.md` | a rendering surprise: inline-level flex/grid, UA stylesheet defaults (headings, lists, `<hr>`, form controls, `<select>`), CSS tables, unsupported selectors, sizing keywords, empty blocks |
| `rtc.md` | `rinch-rtc` (WebRTC data channels): the native webrtc-rs worker, the web backend, the main-thread registry rules, exit handling, the str0m interop tests |
| `ci.md` | GitHub Actions workflows, which checks a PR gets, batched merges and the batch procedure, a red batch, the `test` shards, retargets |

User-facing guides are in `docs/src/guide/`, architecture write-ups in `docs/src/architecture/`, design records in `docs/design/`. The rsx usage rules every UI change needs are in `.claude/rules/rinch-best-practices.md` (loaded automatically).

## Build Commands

```bash
cargo build                    # Build all crates
cargo build -p ui-zoo-desktop       # Build the editor example
cargo run -p ui-zoo-desktop         # Run the rich-text editor
cargo clippy                   # Lint
cargo fmt                      # Format

# Sweep the layout-tree invariants after every layout in every test (#584).
# Debug builds only; a violation FAILS the test it happened in. CI sets this.
RINCH_TREE_CHECK=1 cargo test --workspace

# rinch-web's browser tests, in a real browser through its WebDriver. CI runs
# them in Chrome and in Firefox; a web change is tested in both.
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
CHROMEDRIVER=/path/to/chromedriver cargo test -p rinch-web --target wasm32-unknown-unknown
GECKODRIVER=/path/to/geckodriver   cargo test -p rinch-web --target wasm32-unknown-unknown
```

Browser tests that pin what the browser itself answers state each engine's
answer (`crates/rinch-web/tests/support/engine.rs`), and build clipboard events
with `tests/support/clipboard.rs` (Firefox drops a synthetic event's
`clipboardData`). Setup and the known engine differences:
`docs/src/guide/wasm.md#browsers-and-running-the-browser-tests`.

### Rust toolchain

`rust-toolchain.toml` pins **one exact Rust version** (`channel = "1.99.0"`), and
it is the compiler everywhere (#736). `cargo` in the repository uses it with no
`+toolchain`; rustup downloads it (with `rustfmt`, `clippy` and the
`wasm32-unknown-unknown` target) the first time. Every workflow installs the
version that file names through `.github/actions/rust-toolchain`, the only place
a workflow gets a toolchain from: it reads the file, fails on anything that is
not `X.Y.Z`, and checks `rustc --version` afterwards. So `cargo clippy
--workspace --all-targets -- -D warnings` run locally is the lint set CI runs.

It used to say `channel = "stable"` beside `dtolnay/rust-toolchain@stable`: the
newest stable in CI and whatever `rustup update` last fetched locally, under one
name. A new stable brings new clippy lints, so a clean local clippy failed after
the push (#729). Do not write `cargo +stable` or `cargo +<version>` in this
repository to "match CI" any more; a `+toolchain` overrides the pin.

- **Bumping it** is its own pull request: change the `channel` line, run the CI
  clippy invocations (`ci.yml`'s `clippy`, `clippy-wasm`, `clippy-android` and
  `check-android` jobs), fix what the new lints report in the same PR, and
  update the version written in this section and in `rust-toolchain.toml`'s
  comment. Nothing bumps it automatically, so new lints arrive only in that PR.
- **Targets.** rustup adds the file's targets to the pinned toolchain only. The
  Android targets are not in the file (CI's Android jobs ask the action for
  them); add them with `rustup target add aarch64-linux-android
  x86_64-linux-android` from inside the repository, which applies to the pinned
  toolchain. `E0463: can't find crate for core` means a missing target, not a
  broken build.
- **After pulling a bump**, `cargo clean`: cargo keeps the old compiler's
  artifacts beside the new ones, and a gated `target/` is 20 GB or more.
- **CI caches** in `ci.yml` carry the version in their key, so a bump starts
  each job from a cold `target/` once, not from the old compiler's.
- **`perf.yml` compiles base and head with the head's pin** (it exports
  `RUSTUP_TOOLCHAIN`), so a bump PR's table shows no codegen difference and the
  job does not measure what a new compiler costs.

## Architecture

```
crates/
├── rinch/                     # Main facade crate
│   ├── src/
│   │   ├── shell/            # Window management, event loop
│   │   │   └── rinch_runtime.rs         # Event loop, window creation, rendering
│   │   └── menu/             # Native menu support via muda
│   │       └── mod.rs        # Menu, MenuItem builder API
│   └── ...
├── rinch-core/               # Core types
│   ├── src/element.rs        # Element enum (Html, Fragment, Component only), prop types
│   ├── src/context.rs        # Context API (create_context, use_context)
│   ├── src/reactive/         # Signal, Effect, Memo primitives
│   └── src/dom/              # NodeHandle, RenderScope, DomDocument trait
├── rinch-theme/              # Theme system (optional, enable with `theme` feature)
│   └── src/
│       ├── colors.rs         # Mantine color palettes (10 shades each)
│       ├── spacing.rs        # Spacing scale (xs, sm, md, lg, xl)
│       ├── typography.rs     # Font sizes, line heights, font families
│       ├── radius.rs         # Border radius scale
│       ├── shadows.rs        # Shadow definitions
│       ├── theme.rs          # Theme struct, defaults, builder
│       └── css.rs            # CSS variable generation
├── rinch-components/         # UI components (optional, enable with `components` feature)
│   └── src/
│       ├── button.rs         # Button component
│       ├── text_input.rs     # Text input component
│       ├── text.rs           # Typography component
│       ├── paper.rs          # Card container component
│       ├── stack.rs          # Vertical flex layout
│       ├── group.rs          # Horizontal flex layout
│       ├── badge.rs          # Status indicator
│       ├── icons.rs          # Hand-built chrome glyphs (chevron_up_dom, checkmark_dom, ...)
│       └── styles/           # Per-component CSS generation (one file per component + mod.rs)
├── rinch-tabler-icons/       # ~4,980 Tabler Icons (generated from vendored JSON)
│   ├── build.rs              # Generates the TablerIcon enum from data/ (downloads only as fallback)
│   ├── data/                 # Vendored Tabler JSON, committed — lets the crate build offline
│   └── src/lib.rs            # TablerIcon enum, render_tabler_icon function
├── rinch-editor-core/        # Pure editor model: steps, schema, state, commands, history (wasm-clean)
├── rinch-editor-view/        # Editor view over any DomDocument: EditorHandle, Editor component
├── rinch-editor-collab/      # Optional collab adapter: model <-> yrs (Yjs) CRDT; off by default
├── rinch-editable/           # Single-line <input>/<textarea> engine (not the rich editor)
├── rinch-debug/              # Debug IPC server (optional, enable with `debug` feature)
│   ├── src/lib.rs            # Public API: attach(), CommandSender, CommandReceiver
│   ├── src/protocol.rs       # Wire protocol: length-prefixed JSON, handshake
│   ├── src/server.rs         # TCP listener on background thread (no tokio)
│   └── src/discovery.rs      # ~/.rinch/debug/{pid}.json discovery files
├── rinch-mcp-server/         # Standalone MCP server binary for Claude
│   ├── src/main.rs           # Entry point: MCP on stdio, connects to apps via TCP
│   ├── src/mcp_server.rs     # MCP tools: list_apps, connect, screenshot, dom_tree, etc.
│   ├── src/client.rs         # TCP client for rinch-debug protocol
│   └── src/discovery.rs      # Scans ~/.rinch/debug/*.json, validates PIDs
└── rinch-renderer/           # (placeholder for custom rendering)

examples/
├── ui-zoo/                    # Shared component showcase library
├── ui-zoo-desktop/            # Desktop entry point - primary development target
├── ui-zoo-web/                # WASM browser-native DOM entry point
├── hello_rinch_dom/           # Minimal hello world
└── todo-app/                  # Todo app example
```

## Element Enum

The Element enum is minimal - only used for content that needs to be embedded in the DOM tree:

- `Element::Html(String)` - Raw HTML content rendered by rinch-dom
- `Element::Fragment(Children)` - Groups multiple elements
- `Element::Component(Rc<dyn Component>, Children)` - Custom component implementation

Shell-level constructs (windows, menus, themes) are handled at the runtime level via props, not as Element variants. See the "Application Entry Point" and "Native Menus" sections below.

DOM content is built using `#[component]` functions that return a `NodeHandle`. The `#[component]` macro injects a `RenderScope` (`__scope`) automatically:

```rust
#[component]
fn my_component() -> NodeHandle {
    let div = __scope.create_element("div");
    let text = __scope.create_text("Hello, world!");
    div.append_child(&text);
    div
}
```


## Dependencies and Imports

The `rinch` crate re-exports everything through its prelude. You do NOT need separate dependencies on `rinch-components` or `rinch-theme`:

```toml
# Cargo.toml - this is all you need:
[dependencies]
rinch = { workspace = true, features = ["desktop", "components", "theme"] }
```

**Important:** The workspace dependency uses `default-features = false`, so `"desktop"` must be listed explicitly. Without it, `App` and other desktop APIs won't be available.

```rust
// In your code - prelude includes all components:
use rinch::prelude::*;

// DO NOT add redundant imports:
// use rinch_components::*;  // Not needed! Already in prelude
```


## Component Macro

Use the `#[component]` attribute macro to define component functions without manually writing the `__scope` parameter:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
        }
    }
}

fn main() {
    App::new(app).title("My App").size(800, 600).run();
}
```

The macro auto-injects `__scope: &mut RenderScope` as the first parameter. `__scope` is still available inside the function body for use with `rsx!` and direct DOM operations.

Functions with additional parameters get `__scope` prepended:

```rust
#[component]
fn card(title: &str) -> NodeHandle {
    rsx! { div { {title} } }
}
// Expands to: fn card(__scope: &mut RenderScope, title: &str) -> NodeHandle
```

Both patterns are supported -- `#[component]` is preferred for new code, and the manual `__scope` parameter continues to work.

**A lowercase `#[component]` is a plain function, and `rsx!` cannot invoke it as
an element.** `rsx!` decides component-or-tag from the *case* of the name, so
only PascalCase reaches a component; a lowercase name is looked up as an HTML or
SVG tag. Write `{ card(__scope) }` to call one inline, or name it `Card` and
invoke it as `Card { … }`. Getting this wrong used to compile and render an
empty `<card>` element with the function never called — silent, and it looked
like a CSS bug (issue #528). It is now a compile error naming both fixes, and an
unknown *tag* gets a "did you mean" suggestion (`crates/rinch-macros/src/tags.rs`
holds the accepted set).

### PascalCase Components (Component Generation)

When a `#[component]` function uses a PascalCase name, the macro generates a struct and `Component` trait implementation:

```rust
#[component]
pub fn MyComponent(
    label: String,
    color: String,
    disabled: bool,
    onclick: Option<Callback>,
    children: &[NodeHandle],
) -> NodeHandle {
    // Parameters are available as local variables
    rsx! {
        div {
            class: "my-component",
            style: {format!("color: {}", color)},
            {label.clone()}
            // children is automatically appended by Component trait impl
        }
    }
}
```

**Key points:**
- **PascalCase name** triggers struct generation
- **Parameters become public struct fields** (must be owned types: `String`, `bool`, `Option<T>`, etc.)
- **`children: &[NodeHandle]` is special** — not a struct field, wired to `Component::render` method
- **Reference types rejected** (`&str`, `&T`) with a helpful error message
- **A manual `Default` impl is generated with per-field defaults for known types (String, bool, Option, Vec, numeric, Callback, InputCallback). Unknown types fall back to `Default::default()`.**
- **Usage:** `MyComponent { label: "Hello", color: "blue", onclick: || {}, "child content" }`

This pattern eliminates boilerplate for creating custom components — just write a PascalCase component function with owned parameters.

## Component Trait

Components implement the `Component` trait to render directly to DOM nodes:

```rust
pub trait Component: std::fmt::Debug + 'static {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle;
}
```

Example custom component:

```rust
#[derive(Debug, Default)]
pub struct MyButton {
    pub label: Option<String>,
    pub onclick: Option<Callback>,
}

impl Component for MyButton {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let btn = scope.create_element("button");
        btn.set_attribute("class", "my-button");

        if let Some(cb) = &self.onclick {
            let handler_id = scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        for child in children {
            btn.append_child(child);
        }
        btn
    }
}
```

**Every app-callback `invoke` runs untracked** (`Callback`, `ValueCallback`,
`InputCallback`, `FileDropCallback`, `ScrollCallback`; issue #285): called from
inside a component's own effect, the handler's signal reads subscribe no effect
at any nesting depth — it suspends the whole observer stack
(`reactive::untracked_handler`), where `untracked` pops only the top frame
(which leaks to an outer effect only during a run nested inside that outer
effect's run — #932).
Batching and flushing are unchanged. So `onchange.invoke(v)` in a coordinating
effect needs no hand-written `untracked`. **The other synchronously-run app
callbacks go through the same function** (issue #931): the editor's `Hook::invoke`
(`on_change`/`on_selection_change`/`on_caret_moved`/`on_key`) and its
`on_link_click`/`on_link_hover`; the child observers (`dom::late_child::notify`,
reached from inside every `for`/`if`/component-re-render effect); the selection
and selection-sync callbacks; `dispatch_dismiss`; `Drag::cancel`'s `on_cancel`
(which #942's superseding `Drag::start` also reaches); and the keyboard and paste
interceptors (`dispatch_keyboard_event` / `dispatch_paste_event` are public).
`rinch_core::untracked_handler` is public for any new slot that stores an app's
callback — use it there, not `untracked`. Not wrapped, because nothing reaches
them from inside an effect: the focus-registry callbacks and menu callbacks
(both `pub(crate)`, run by the runtime from events or the deferred focus work),
and Drag's `on_move`/`on_end` (pointer events). **Editor plugin code runs
untracked too** (issue #943), by a different route: `EditorHandle`'s core guards
(`CoreGuard`, `CoreMutGuard`) hold a `rinch_core::reactive::suspend_tracking()`
guard — `untracked_handler` as a value — for as long as the core is borrowed, so
`Plugin::apply`, `init_state`, `decorations` (the view's diff), `handle_paste`, a
plugin's command (`command` and `can_run` alike) and an input rule are untracked
wherever the handle itself runs them, including through `add_plugin`, a load and
`collab_receive`; the two constructors wrap `EditorState::create` and the first
projection in `untracked_handler`. A new internal site under the guard needs
nothing. The collaboration `outbound` sink runs under the same guard and wraps
itself in `untracked_handler` as well (#948). Deliberately **not** untracked: the
`EditorHandle::update` `build` closure, the caller's own code — `dispatch_inner`
lifts the suspension for that closure alone, so plugin code *it* calls
(`update(|s| { s.apply(..); … })`) is tracked as the caller's. The guard type
`TrackingSuspended` is `!Send`/`!Sync`: the stack it restores is thread-local.

## Component Props

For a complete reference of every component's props (fields, types, defaults), see [`docs/src/guide/component-props.md`](docs/src/guide/component-props.md).

**Key points:**
- CSS shorthand props (`w`, `h`, `m`, `p`, `maw`, `px`, `my`, etc.) work on all HTML elements and components. They expand to `set_style()` calls. Spacing scale values (`xs`, `sm`, `md`, `lg`, `xl`) auto-resolve to `var(--rinch-spacing-{value})`. Example: `div { p: "md", maw: "600px" }`. See `docs/src/guide/rsx-syntax.md#style-shorthands` for the full list.
- `Stack` and `Group` both have `align` and `justify` props for flex alignment — no need for `style:` for those.
- **Component text props are `String` (not `Option<String>`)** — empty string means "not set". The `rsx!` macro auto-converts string literals to `String::from(...)`.
- All component props accept reactive closures `{|| expr}` for automatic re-rendering when signals change.
- `_fn` suffix props (e.g., `value_fn`, `checked_fn`, `opened_fn`) provide surgical DOM updates without full component re-render.
- The `rsx!` macro auto-wraps prop values — do NOT manually wrap in `Some(...)`, `Rc::new(...)`, or `Callback::new(...)`.

## Reactive Patterns in RSX

Use closure syntax `{|| expr}` for reactive expressions that update automatically:

```rust
let count = Signal::new(0);

rsx! {
    // Static - captured once at render time
    p { {count.get().to_string()} }

    // Reactive - creates Effect, updates when signal changes
    p { {|| count.get().to_string()} }

    // Reactive attribute
    div { class: {|| if count.get() > 5 { "high" } else { "low" }}, "Value" }
}
```

**Block-expression closures:** The macro now supports block expressions that evaluate to closures, allowing setup code before the closure:

```rust
// ✅ WORKS: closure is the direct expression
div { style: {|| format!("width: {}px", count.get() * 10)} }

// ✅ ALSO WORKS: block with setup + final closure
div { style: { let m = 10; move || format!("width: {}px", count.get() * m) } }

// ✅ BOTH PATTERNS: compute outside or inside the block
let m = 10;
rsx! { div { style: {move || format!("width: {}px", count.get() * m)} } }
```

**Note:** Both `Signal` and `Memo` implement `Copy` — no `.clone()` needed before closures:

```rust
let count = Signal::new(0);
let doubled = Memo::new(move || count.get() * 2);

rsx! {
    // Both count (Signal) and doubled (Memo) can be used in multiple closures without .clone()
    p { {|| count.get().to_string()} }
    p { {|| doubled.get().to_string()} }
    button { onclick: move || count.update(|n| *n += 1), "+" }
}
```


## Iterative Development with MCP (IMPORTANT)

**Always use the rinch MCP tools to test and iterate on rinch applications.** The MCP server provides direct access to screenshots (viewable inline), DOM inspection with computed styles, and input simulation — no Python scripts or intermediate files needed.

### Workflow: Launch → Test → Close → Edit → Repeat

**Step 1: Launch the app**

Use the MCP `launch_app` tool — it builds, launches, waits for debug registration, and auto-connects:

```
launch_app(package: "ui-zoo-desktop")
```

In headless environments, ensure Xvfb is running: `Xvfb :99 -screen 0 1280x720x24 &`. The `launch_app` tool forwards `DISPLAY` automatically.

**Step 2: Inspect and interact**

Use MCP tools directly — screenshots render inline, DOM queries return computed styles:

```
screenshot()                              → inline PNG image (directly viewable)
dom_tree()                                → DOM tree with layout (add verbose: true for computed styles)
query_selector(selector: ".my-class")     → find nodes by tag, .class, [attr], [attr=value]
get_node(id: 42)                          → detailed node info with computed styles + display mode
get_computed_styles(id: 42)               → just the CSS properties for a node
click(x: 100, y: 200)                     → simulate mouse click
wait_frame()                              → wait for next render
type_text(text: "hello")                  → simulate keyboard input
get_text_content(id: 42)                  → get text in subtree
```

**Node geometry — `layout` vs `absolute`.** `dom_tree`/`query_selector`/`get_node` report each
node's box twice: `layout` is **parent-relative** (exactly the node's own `layout`, for checking a
child's offset within its container) and `absolute` is the **on-screen** box. **Pass `absolute.x`/
`absolute.y` (e.g. its center) to `click()`/`mouse_*`** — the `layout` x/y are NOT screen
coordinates. `absolute` is the box paint draws *up to the DPI scale*, so a CSS transform on the
node or any ancestor moves *and resizes* it, and a `position: fixed` node reports its viewport
box. That is also why width/height can differ between the two: `layout` is the size Taffy gave
the box, `absolute` is the size it covers on screen (they are equal wherever nothing scales it).

**Units: `absolute` and every input tool are in logical (CSS) pixels; `screenshot()` is in
physical pixels.** They coincide at scale factor 1, which is every ordinary development display.
On a HiDPI display (or under `WINIT_X11_SCALE_FACTOR`) they do not: to find a node in a
screenshot, multiply its `absolute` box by the scale factor. Do **not** feed a coordinate read
off a screenshot back into `click()` there — divide it first. `click()` and the pointer path a
real mouse takes are the same logical space on every host (#299).

**Step 3: Close the app**

```
close_app()
```

**Step 4: Edit code and repeat**

Make changes, rebuild, launch again. The full cycle:
1. `screenshot()` → view inline → identify issues
2. `dom_tree()` or `get_node()` → check layout and computed styles
3. `click()` → `wait_frame()` → `screenshot()` to verify reactive updates
4. `close_app()` → edit code → rebuild → `launch_app()` → repeat

### What to Check

| Check | How |
|-------|-----|
| Text renders correctly | `screenshot()` — no garbled glyphs, correct wrapping |
| Layout is correct | `dom_tree()` — verify x, y, width, height values |
| Styles applied correctly | `get_computed_styles(id)` — inspect resolved CSS properties |
| Colors/backgrounds | `screenshot()` — visual inspection |
| Click handlers work | `click()` → `wait_frame()` → `screenshot()` to see state change |
| Text content | `get_text_content(id)` to verify reactive values update |
| CSS class matching | `query_selector(selector: ".className")` to find styled elements |


## Development Notes

- **ui-zoo-desktop** is the primary way to iterate on the framework
- The shell layer handles window management and event loop integration
- Menu callbacks are fully implemented and trigger re-renders automatically
- RSX macro provides helpful error messages with typo suggestions
- Transparent windows use an intermediate render texture (swapchain textures don't support STORAGE_BINDING)

## Performance regressions

**The exact counter baselines are the performance contract.** Four files assert
**whole frames** (every non-timing counter exact, every unlisted counter `0`):
`crates/rinch-dom/tests/perf_counter_baselines.rs` (#877),
`crates/rinch-dom/tests/perf_regression_scenarios.rs` (one document per counter
increment site, #879), `crates/rinch/src/app/perf_regression_tests.rs` and
`crates/rinch/src/app/perf_regression_editor_tests.rs` (shell scenarios driven
the way the desktop loop drives them, through `app/perf_expect.rs`).
`crates/rinch/src/app/perf_stats_tests.rs` is exact on the counters it names,
not on the whole frame.

- **A PR that changes a baseline says why in its body**, for each moved
  counter, old → new: a fix, or a cost it accepts
  (`.github/pull_request_template.md`). A fall to `0` is a fix or an increment
  that stopped counting — say which. A pinned **finding** names its issue, and
  a fix must *lower* that number.
- **Never loosen an assertion to make it pass.**
- **Fonts:** declared line boxes pin only the vertical axis. Any horizontal
  text geometry needs the bundled Inter (`perf_expect::new_app`, text set in
  `sans-serif`), or the number moves with the host's fonts (CI is DejaVu).
- Updating, reading `RINCH_PERF` / `perf_stats`, and the rest:
  `docs/src/guide/performance.md#the-baselines-are-the-contract`.

**The baselines say which work a frame did; CI's `Perf` workflow says what it
cost** (`.github/workflows/perf.yml`, benchmarks in `crates/rinch-bench`).
It records Callgrind instruction counts (Gungraun) for every benchmark in the crate, on a
**batch** PR's merge commit and on its first parent (the current `main` tip); a
component PR skips it (see CI below). The report
is a table in the job summary and one PR comment. The job fails past +3%
(`vars.PERF_REGRESSION_THRESHOLD`), or when a base that has the benchmarks
cannot run them. The base runs the head's benchmark sources, and its own when
those do not build there — a new bench calling a new API (#1036) — so the rest
are still compared and the new one shows as `new`; the report names what was
not compared, and fails unlabelled if the head's copy also edits an existing
scenario rather than only adding. The `perf-regression-accepted` label (on the batch PR) turns either failure into
a warning, and the PR then explains the cost the same way a baseline change
does. Inside the measured operation the bench binary bump-allocates
(`rinch_bench::alloc`): glibc's malloc cost depends on heap history and moved
one count 7% between identical runs, and with the bump allocator runs agree to
about 0.03%. Counts are comparable only between runs on one machine.
`docs/src/guide/performance.md#ci-regression-job`.

## CI: batched merges

An ordinary PR into `main` (a **component** PR) runs only the fast jobs: `fmt`,
the `clippy` jobs, `check-android` and `check-macos`. The `test (…)` shards, `test-wasm`, Perf
and the full Documentation build run on a PR whose head branch starts with
`batch/`, and on every push to `main`. To land reviewed PRs, the lead merges
their heads (`git merge --no-ff`) into `batch/YYYY-MM-DD-N` cut from `main`,
opens one `batch: …` PR listing them (`Refs #N`), and merges it with a **merge
commit** once green, which marks each component PR merged. A red batch is the
top priority: fix or drop the culprit. Procedure, the test shards, and the
trigger traps (a retarget runs through `base-changed.yml`; retarget a stacked
PR before deleting its base; a conflicting PR gets no run; a re-run does not
re-base): `docs/internals/ci.md`. "No checks" is never a pass.

## Documentation Requirements

**Always update user-facing documentation when adding or changing features:**

1. **User Guide** (`docs/src/guide/`): Update relevant guide pages when adding new user-facing features, APIs, or changing behavior
2. **API docs**: Ensure doc comments are added/updated for public APIs
3. **Internals** (`docs/internals/`): record how a mechanism works, and what changed, on the page for its area (see **Where the detail lives**). `CLAUDE.md` itself changes only for something every task needs, such as a new internals page in the index

Documentation locations:
- `docs/src/guide/hooks.md` - State management guide
- `docs/src/guide/menus.md` - Menu and shortcut guide
- `docs/src/guide/windows.md` - Window management
- `docs/src/guide/reactivity.md` - Signals, effects, memos
- `docs/src/guide/rsx-syntax.md` - RSX macro syntax
- `docs/src/guide/platform.md` - File dialogs, clipboard, system tray, notifications
- `docs/src/guide/game-engine.md` - Game engine integration (embed API)
- `docs/src/guide/theming.md` - Theme system and CSS variables
- `docs/src/guide/components.md` - Component library
- `docs/src/guide/contenteditable.md` - Using the rich-text editor (Editor component, EditorHandle, commands)
- `docs/src/guide/editor.md` - Rich-text editor internals (model, schema, steps, plugins, view)
- `docs/src/guide/performance.md` - Performance counters, `RINCH_PERF`, the `perf_stats` MCP tool
- `docs/src/SUMMARY.md` - Table of contents (update when adding new pages)

Architecture documentation:
- `docs/src/architecture/overview.md` - System architecture and crate structure
- `docs/src/architecture/fine-grained.md` - Fine-grained reactive rendering
- `docs/src/architecture/render-scope.md` - RenderScope and NodeHandle API

Source code documentation:
- `crates/rinch-core/src/dom/` - NodeHandle, RenderScope, DomDocument trait
- `crates/rinch-dom/src/lib.rs` - RinchDocument implementation (Taffy + Parley + Vello)
- `crates/rinch-macros/src/dom_codegen/` - rsx! macro DOM code generation

## Visual Audit Workflow

Use the rinch MCP tools to systematically compare rinch rendering against expected browser rendering.

**Quick start:**
```
launch_app(package: "ui-zoo-desktop")   # Start app
screenshot()                        # View inline
query_selector(selector: ".class")  # Find elements
get_computed_styles(id: 123)        # Check CSS values
close_app()                         # Done
```

**Full workflow documented at:** `.claude/skills/visual-audit.md`

**Common issues and fixes:**

| Issue | Check | Fix Location |
|-------|-------|--------------|
| Borders appearing unexpectedly | `border_*_width` should be 0 for `border: none` | `computed_style/` - check `border-style` |
| SVG icons 0x0 | Missing inline width/height styles | Add `style="width: Xpx; height: Xpx"` |
| SVG `fill`/`stroke` attribute not painting | Must be a CSS `<color>` (parsed by `layout::parse_color_with_current`); `none`/`currentcolor` are case-insensitive; an absent `fill` is black | `paint/svg.rs` `parse_svg_paint` / `SvgPaint` (#464 replaced the old `resolve_svg_color`) |
| currentColor not resolving | Check how `currentcolor` is threaded through `parse_color_with_current` | `computed_style/`, `paint/svg.rs` |
| Reactive state not updating | Need `{|| expr}` closure syntax | Component render method |
| Menu active state stale | Missing reactive effect | Add `create_effect()` for class updates |
