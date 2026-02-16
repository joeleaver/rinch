# AGENTS.md

Guidance for AI agents working on the rinch codebase. See also `CLAUDE.md` for detailed API usage and examples.

## Project Overview

Rinch is a Rust-native GUI framework that uses HTML/CSS for layout with GPU rendering. The core design is **fine-grained reactivity** (SolidJS-style) — components run once, closures become Effects that surgically update individual DOM nodes. No virtual DOM, no diffing.

**Rendering pipeline:** Component → RenderScope/NodeHandle → RinchDocument (Stylo CSS + Taffy layout + Parley text) → Vello scene → wgpu

## Workspace Structure

```
crates/
├── rinch-core/          # Reactive primitives, DOM abstraction, hooks (7.6K lines)
├── rinch-macros/        # rsx! macro, #[component] attribute (2.5K lines)
├── rinch-dom/           # CSS + layout + paint engine (18.4K lines) — heaviest crate
├── rinch-platform/      # Platform abstraction traits (500 lines)
├── rinch/               # Desktop runtime, facade crate (15.8K lines)
├── rinch-widgets/       # 56 Mantine-inspired widgets (17.8K lines, 113 files)
├── rinch-theme/         # Theme system, CSS variables (1.5K lines)
├── rinch-editor/        # ProseMirror-style rich-text editor (11.4K lines)
├── rinch-editable/      # Text editing primitives (1.5K lines)
├── rinch-editor-widgets/# Editor toolbar and controls (1.5K lines)
├── rinch-editor-macros/ # Editor proc macros (231 lines)
├── rinch-clipboard/     # Clipboard via arboard (373 lines)
├── rinch-tabler-icons/  # 5000+ icons, fetched at build time (721 lines)
├── rinch-debug/         # TCP IPC server for inspection (424 lines)
├── rinch-mcp-server/    # MCP server for Claude integration (862 lines)
├── rinch-visual-test/   # Visual regression test framework (2.1K lines)
├── rinch-renderer/      # PLACEHOLDER — 9 lines, unused
└── rinch-web/           # STUB — 217 lines, all TODOs

examples/
├── ui-zoo/              # Shared widget showcase (primary dev target)
├── ui-zoo-desktop/      # Desktop entry point
├── ui-zoo-web/          # WASM entry (excluded from workspace, not functional via rinch-web)
├── hello_rinch_dom/     # Minimal hello world
├── fine_grained_window/ # Fine-grained rendering demo
├── contenteditable_spike/
├── contenteditable_test/
└── hidden_textarea_spike/
```

## Crate Dependency Graph

```
rinch (facade)
  ├── rinch-core (always)
  ├── rinch-macros (always)
  ├── rinch-platform (always)
  ├── rinch-dom (desktop feature)
  ├── rinch-theme (theme feature)
  ├── rinch-widgets (widgets feature → pulls theme)
  ├── rinch-debug (debug feature)
  └── winit, wgpu, vello, muda (desktop feature)

rinch-dom
  ├── rinch-core
  └── stylo, stylo_taffy, taffy, parley, vello, cssparser

rinch-widgets → rinch-core, rinch-theme, rinch-macros
rinch-editor → rinch-core, rinch-editable, rinch-clipboard, automerge
```

## Key Conventions

### Reactivity Model

- `Signal<T>` and `Memo<T>` are `Copy` — no `.clone()` needed before closures
- `{|| expr}` in rsx! creates an Effect for surgical DOM updates
- `{expr}` without closure captures the value once at render time and never updates
- Components run **once** to build the DOM. All reactive updates go through Effects.

### RSX Macro Auto-Wrapping

The `rsx!` macro auto-wraps widget prop values. **Never** manually wrap in `Some(...)`, `Rc::new(...)`, or `WidgetCallback::new(...)`:

```rust
// CORRECT — macro wraps for you
Button { variant: "filled", onclick: move || do_thing() }

// WRONG — double-wrapping
Button { variant: Some(String::from("filled")), onclick: Some(WidgetCallback::new(|| do_thing())) }
```

### Native Control Flow in RSX

The `rsx!` macro supports native Rust `if`/`for`/`match`. All control flow is **always reactive** — conditions and iterators are auto-wrapped in closures and tracked by Effects.

```rust
rsx! {
    div {
        // if/else — desugars to show_dom()
        if visible.get() { p { "Shown" } } else { p { "Hidden" } }

        // for — desugars to for_each_dom_typed(), uses keyed reconciliation
        for todo in todos.get() {
            div { key: todo.id, {todo.name.clone()} }
        }

        // match — desugars to match_dom()
        match tab.get() {
            0 => div { "Home" },
            1 => div { "About" },
            _ => div { "404" },
        }
    }
}
```

`if let`, `else if` chains, match guards, and pattern bindings are all supported. The `Show` and `For` components remain available but native syntax is preferred for new code.

### Component Pattern

```rust
#[component]
fn my_widget(title: &str) -> NodeHandle {
    // __scope is auto-injected by the macro
    let count = use_signal(|| 0);
    rsx! {
        div {
            h1 { {title} }
            p { {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}
```

### Hooks Rules

Hooks must be called in the same order every render — no conditional hooks, no hooks inside event handlers or loops. Always call at the top of the component.

## Per-Crate Agent Guidance

### rinch-core (reactive kernel)

**Key files:**
- `reactive.rs` (1,384 lines) — Signal, Effect, Memo, reactive graph. **Highest risk** — bugs here break everything.
- `dom.rs` (1,534 lines) — NodeHandle, RenderScope, DomDocument trait. This is the contract between the reactive layer and rendering.
- `hooks.rs` (1,342 lines) — use_signal, use_effect, use_memo, use_derived, use_context, etc.
- `for_loop.rs` — Keyed list reconciliation (LIS algorithm), `for_each_dom()` and `for_each_dom_typed()`
- `show.rs` — Conditional rendering (`show_dom()`)
- `match_dom.rs` — Multi-branch conditional rendering (`match_dom()`)
- `element.rs` (593 lines) — Widget trait, Element enum, callback types
- `events.rs` (940 lines) — Event types, event handler registration

**When working here:**
- Changes to `DomDocument` trait signatures ripple through rinch-dom and rinch
- Changes to `Signal`/`Effect`/`Memo` affect the entire framework
- 65 tests cover reactivity, hooks, and reconciliation

### rinch-dom (CSS + layout + paint)

**Key files:**
- `computed_style.rs` (3,066 lines) — CSS computed style extraction from Stylo
- `paint.rs` (2,353 lines) — Vello scene construction (backgrounds, borders, text, shadows, transforms)
- `stylesheet.rs` (1,567 lines) — CSS parsing and stylesheet management
- `style_resolution.rs` (1,040 lines) — Stylo style resolution bridge
- `transition.rs` (1,003 lines) — CSS transitions engine
- `dom_impl.rs` (972 lines) — `RinchDocument` implementing `DomDocument`
- `layout.rs` (879 lines) — Taffy layout integration
- `ifc.rs` (859 lines) — Inline formatting context (text layout)
- `node.rs` (636 lines) — DOM node types, dirty flags, layout results

**When working here:**
- 207 tests across 7 dedicated test files in `tests/`
- Stylo integration (`stylo_impl.rs`, `style_resolution.rs`) is complex — understand the Stylo DOM trait requirements
- Paint order matters: backgrounds → borders → content → outlines → shadows
- Layout uses Taffy for flexbox; inline text uses Parley for shaping

### rinch-macros (proc macros)

**Key files:**
- `dom_codegen/mod.rs` — RSX macro code generation (dispatches to submodules)
- `dom_codegen/control_flow.rs` — Codegen for Show, For, and native `if`/`for`/`match`
- `dom_codegen/html.rs` — HTML element codegen
- `dom_codegen/widget.rs` — Widget codegen
- `element.rs` — Element parsing
- `node.rs` — Node parsing (includes `RsxIfBlock`, `RsxForLoop`, `RsxMatchBlock`)

**When working here:**
- Changes affect every component in the project
- 79 tests cover RSX parsing and codegen
- Be careful with the auto-wrapping logic for widget props (Some, Rc, callbacks)
- Native control flow (`if`/`for`/`match`) desugars to `show_dom()`/`for_each_dom_typed()`/`match_dom()`

### rinch (desktop runtime / facade)

**Key files:**
- `app.rs` (7,211 lines) — **Monolith.** Entire application lifecycle, event processing, rendering loop. Highest-coupling file in the project.
- `shell/window_manager.rs` (2,491 lines) — Multi-window management
- `shell/fine_grained_runtime.rs` (779 lines) — Fine-grained rendering runtime
- `shell/rinch_runtime.rs` (745 lines) — Event loop, window creation
- `shell/devtools.rs` — DevTools panel (F12)
- `shell/transparent_renderer.rs` — Transparent window rendering
- `menu/mod.rs` — Native menu system via muda

**When working here:**
- `app.rs` is the most dangerous file to modify — side effects are likely
- The `prelude` module re-exports everything from sub-crates
- Feature flags gate heavy dependencies (desktop, widgets, theme, debug, etc.)

### rinch-widgets (56 widgets)

**Key files:**
- `styles/` directory — CSS generation for each widget
- Individual widget files (e.g., `button.rs`, `text_input.rs`, `modal.rs`, `tabs.rs`)
- `icons.rs` (855 lines) — SVG icon rendering
- `tree.rs` (703 lines) — Most complex widget

**When working here:**
- Each widget is self-contained — low risk to modify individually
- Widgets implement `Widget::render(&self, scope, children) -> NodeHandle`
- Follow existing patterns: look at `badge.rs` (simple) or `tabs.rs` (complex) as templates
- **No unit tests** on individual widgets currently — test via ui-zoo visually
- `_fn` suffix props (e.g., `value_fn`, `checked_fn`) enable surgical DOM updates

### rinch-editor (rich-text editor)

**Key files:**
- `document/model.rs` (1,697 lines) — Document tree model
- `schema/mod.rs` (807 lines) — Document schema definition
- `extensions/starter_kit.rs` (1,457 lines) — Default extension pack
- `view/input_bridge.rs` (878 lines) — Input handling bridge
- `view/render.rs` (685 lines) — Editor rendering
- `extensions/table_model.rs` (667 lines) — Table support

**When working here:**
- 294 tests — best-covered crate in the workspace
- Schema-based: understand the node/mark type system before making changes
- Uses Automerge CRDT for collaboration support
- Extensions are modular — safe to add/modify individually

## Build & Test

```bash
cargo build                         # Build all crates
cargo build -p ui-zoo-desktop       # Build the main example
cargo run -p ui-zoo-desktop         # Run it
cargo test --workspace              # Run all 700 tests
cargo clippy --workspace            # Lint (CI uses -D warnings)
cargo fmt --check                   # Format check
```

**System deps (Linux):** GTK3, Pango, Cairo development libraries.

**wgpu fork:** The workspace patches 5 wgpu crates from `joeleaver/wgpu-fork` (branch `rinch-patch`) for transparent window support. Downstream consumers must copy the `[patch.crates-io]` section.

## Common Pitfalls

1. **Double-wrapping props** — The rsx! macro auto-wraps. Writing `Some(...)` or `WidgetCallback::new(...)` in rsx! causes confusing type errors.
2. **Non-reactive expressions** — `{count.get()}` captures once; `{|| count.get()}` creates an Effect that updates. This is the #1 source of "UI doesn't update" bugs.
3. **Conditional hooks** — Hooks must be called unconditionally, in the same order. Use `if`/`else` or `Show` for conditional rendering instead.
4. **Missing `desktop` feature** — The workspace default-features is false. Without `features = ["desktop"]`, `run()` and rendering APIs are unavailable.
5. **app.rs coupling** — The 7K-line monolith means changes to event handling, rendering, or window management can have unexpected interactions.
6. **Stylo complexity** — The Firefox CSS engine integration has many trait implementations with subtle requirements. Read existing code carefully before modifying.

## Testing with MCP

The project includes a dedicated MCP server (`rinch-mcp-server`) for AI-driven visual testing:

```
launch_app(package: "ui-zoo-desktop")  # Build, launch, auto-connect
screenshot()                           # Inline PNG — directly viewable
dom_tree()                             # Full DOM with layout + computed styles
click(x: 100, y: 200)                 # Simulate input
close_app()                            # Clean shutdown
```

Build the MCP server first: `cargo build -p rinch-mcp-server`

## Test Coverage Map

| Crate | Tests | Notes |
|-------|-------|-------|
| rinch-editor | 294 | Best covered — model, schema, commands, history, tables |
| rinch-dom | 207 | Computed styles, DOM ops, IFC, layout, paint, transitions |
| rinch-macros | 79 | RSX parsing, codegen, native control flow |
| rinch-core | 65 | Reactivity, hooks, reconciliation, show/for/match |
| rinch-editor-widgets | 32 | Toolbar, controls |
| rinch-visual-test | 22 | Capture, comparison, CSS export |
| rinch-editable | 10 | Input, state |
| rinch | 5 | HTML parser only |
| rinch-widgets | 0 | No unit tests — tested visually via ui-zoo |
| rinch-debug | 0 | No tests |
| rinch-mcp-server | 0 | No tests |
