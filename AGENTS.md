# AGENTS.md

Guidance for AI agents working on the rinch codebase. `CLAUDE.md` is the detailed API
and usage reference — read it for *how to write rinch code*. This file is about
*working on rinch itself*: where things live, which invariants you must not break, and
how to verify a change.

## Project Overview

Rinch is a Rust-native GUI framework that uses HTML/CSS for layout. The core design is
**fine-grained reactivity** (SolidJS-style): components run **once**, closures become
Effects that surgically update individual DOM nodes. No virtual DOM, no diffing, no
re-renders.

**Desktop pipeline:** component → `RenderScope`/`NodeHandle` → `RinchDocument`
(Stylo CSS + Taffy layout + Parley text) → `Painter` → window.
The `Painter` is **tiny-skia + softbuffer by default**; the `gpu` feature swaps in
Vello + wgpu.

**Web pipeline:** the same component code → `WebDocument` (`rinch-web`) → real browser
DOM. No Taffy/Parley/Vello — the browser lays out, shapes and paints.

Both are the *same* `DomDocument` trait from `rinch-core`. That trait is the seam that
makes the reactive layer renderer-agnostic; keep it that way.

## Workspace Structure

`members = ["crates/*", "examples/*"]`, so **every** directory under `crates/` is a
member unless listed in the root `exclude` (currently `crates/rinch-visual-test` plus
the `*-web` examples). Verify against the root `Cargo.toml` rather than trusting this
list if something looks off.

**Framework core**

| Crate | Role |
|---|---|
| `rinch` | Facade + desktop runtime. `app/` (platform-agnostic app logic), `shell/` (winit + renderer), `menu/`, `editor/` (desktop editor wiring), `embed.rs`, `render_surface.rs`. |
| `rinch-core` | The reactive kernel and the DOM abstraction. `reactive/` (Signal, Effect, Memo, Scope), `dom/` (NodeHandle, RenderScope, `DomDocument`), `events/`, control flow (`show.rs`, `for_loop.rs`, `match_dom.rs`), `context.rs`, `timer.rs`. Almost no external deps. |
| `rinch-macros` | `rsx!` and `#[component]`. |
| `rinch-platform` | Platform abstraction traits: `PlatformWindow`, `PlatformRenderer`, `PlatformEventLoop`, `PlatformMenu`, plus `PlatformEvent` / `AppAction`. |
| `rinch-dom` | CSS + layout + paint engine (Stylo, Taffy, Parley, Vello/tiny-skia). The heaviest crate. |
| `stylo-taffy` | Vendored Stylo↔Taffy interop. Pins Taffy alongside `rinch-dom`. |
| `rinch-renderer` | Near-empty stub (two empty modules). Optional dep of `rinch` under `gpu`; nothing meaningful lives here yet. |

**UI**

| Crate | Role |
|---|---|
| `rinch-components` | The Mantine-inspired component library. |
| `rinch-theme` | Theme system, colour palettes, CSS variable generation. |
| `rinch-tabler-icons` | Tabler Icons; `build.rs` downloads them and generates `TablerIcon` / `TablerIconStyle` / `render_tabler_icon`. |

**Rich-text editor** (see `docs/src/architecture/editor.md` for the full picture)

| Crate | Role |
|---|---|
| `rinch-editor-core` | The pure model: document tree, `Pos`, schema/`ContentMatch`, `Step`s, `EditorState`/`Transaction`, commands, plugins, serialization. **wasm-clean; no renderer, no CRDT.** |
| `rinch-editor-view` | Renderer-agnostic view: `RinchDomEditorView`, `EditorHandle`, the `Editor {}` component, the mounted-editor registry. Shared by desktop and web. |
| `rinch-editor-collab` | Optional (`collaboration` feature) CRDT adapter over **yrs**. The only crate in the workspace that links a CRDT engine. |
| `rinch-editable` | **Unrelated** engine for single-line `<input>`/`<textarea>`. Shares no code or types with the rich editor despite overlapping names (`Selection`, `Position`). |

**Web**

| Crate | Role |
|---|---|
| `rinch-web` | Browser-native backend: `WebDocument` over `web_sys`, document-level event delegation, `mount`, the browser editor glue. Depends on `rinch` with `default-features = false`, so it links no desktop stack. |

**Platform services**

`rinch-clipboard` (arboard / web-sys) · `rinch-storage` (filesystem / IndexedDB) ·
`rinch-http` (ureq / fetch) · `rinch-ws` (tungstenite / WebSocket) · `rinch-android`
(the JNI bridge to Android platform services: clipboard, IME, camera, permissions, …).

**Media**

`rinch-video` · `rinch-av` · `rinch-webrtc` (str0m / web_sys) · `rinch-signaling` ·
`rinch-signaling-server` · `rinch-screen-capture`.

**Tooling**

| Crate | Role |
|---|---|
| `rinch-debug` | TCP IPC server embedded in an app (`debug` feature); writes `~/.rinch/debug/{pid}.json`. |
| `rinch-mcp-server` | Standalone MCP server Claude talks to; discovers and drives running apps. |
| `rinch-test` / `rinch-test-cli` | MCP-based test harness ("Playwright for rinch") and its CLI (binary name `rinch-test`). |
| `rinch-visual-test` | Visual regression testing. **Excluded from the workspace.** |

**Examples.** `ui-zoo-desktop` (+ the shared `ui-zoo` library) is the primary
development target. The `*-web` examples (`ui-zoo-web`, `paint-web`, `editor-web`,
`collab-editor-web`, `events-web`, `islands-web`, `webgpu-surface-web`) are **excluded
from the workspace** — build them with `--target wasm32-unknown-unknown` or from their
own directory. Others of note: `markdown-editor`, `collab-editor-demo`, `game-embed`,
`gpu-device-config`, `todo-app`, `hello-android`.

## Crate Dependency Graph

```
rinch (facade)
  ├── rinch-core, rinch-macros, rinch-platform   (always)
  ├── rinch-dom, rinch-editable, rinch-editor-core, rinch-editor-view,
  │   winit, muda, parley, softbuffer            (desktop feature)
  ├── vello, wgpu, rinch-renderer                (gpu feature → implies desktop)
  ├── rinch-theme                                (theme feature)
  ├── rinch-components                           (components feature → implies theme)
  ├── rinch-clipboard / rfd / tray-icon / rinch-debug / rinch-video / rinch-http
  │                                              (clipboard / file-dialogs /
  │                                               system-tray / debug / video /
  │                                               image-network)
  └── rinch-editor-view/collaboration            (collaboration → implies desktop)

rinch-dom            → rinch-core + stylo, stylo_taffy, taffy, parley, vello,
                       tiny-skia (software-renderer), cssparser
rinch-components     → rinch-core, rinch-theme, rinch-macros, rinch-tabler-icons
rinch-web            → rinch-core, rinch (no default features), rinch-editor-core,
                       rinch-editor-view, wasm-bindgen/js-sys/web-sys
rinch-editor-core    → thiserror, regex (+ optional serde, pulldown-cmark, accesskit)
rinch-editor-view    → rinch-core, rinch-editor-core (+ optional rinch-editor-collab)
rinch-editor-collab  → rinch-editor-core, yrs
```

**Every heavy dependency in `rinch` is optional**, including `rinch-dom`, `parley` and
`vello`. A build with `default-features = false` links none of them — which is exactly
how `rinch-web` consumes the facade. Don't make a dep non-optional without checking the
wasm build.

## Key Conventions

### Reactivity model

- `Signal<T>` and `Memo<T>` are `Copy` — never `.clone()` one before a closure.
- `{|| expr}` in `rsx!` creates an Effect for surgical DOM updates. `{expr}` without
  the closure captures once and **never** updates. This is the #1 source of "the UI
  doesn't update" bugs.
- Components run **once**. Never write code that rebuilds a DOM subtree to "refresh"
  it — that is a re-render, and it is always the wrong fix.

### Reactive resource ownership (#141)

A scope **owns** every `Signal`/`Memo`/`Effect`/event handler created while it was the
ambient owner, and disposing the scope **frees** them: reads of a freed handle panic,
writes are warn-once no-ops. Check with `is_alive()`. No ambient owner (`main()`, a
timer callback, a detached thread) means app lifetime.

Opt out inside a render with `unowned(|| …)` or `.leak()`. A process-lifetime registry
that holds scope-owned state **must** call `rinch_core::reactive::on_cleanup` to let go.

### Effect execution order is a contract (#154)

Effects observing the same signal run in **registration order**, and the pending queue
drains FIFO. This is enforced by `BTreeSet<ObserverId>` subscriber sets (ids are
monotonic and never reused, so ascending id *is* registration order) plus `pop_front`
in the flush. **Do not** swap either for a `HashSet` or a LIFO stack — an effect
registered after an `rsx!` tree relies on seeing the post-patch DOM in the same flush.

### Threading

Rinch owns the main thread and all UI state is `!Send` (thread-local reactive system).
`Signal::set()` / `update()` **panic** off the main thread; use `send()` /
`update_send()`, which marshal onto the main thread. The runtime installs the
dispatcher at startup via `register_main_thread()` + `set_cross_thread_dispatcher()`.

### RSX macro auto-wrapping

The `rsx!` macro auto-wraps component prop values. **Never** manually wrap in
`Some(...)`, `Rc::new(...)`, or `Callback::new(...)`:

```rust
// CORRECT — macro wraps for you
Button { variant: "filled", onclick: move || do_thing() }

// WRONG — double-wrapping
Button { variant: Some(String::from("filled")), onclick: Some(Callback::new(|| do_thing())) }
```

The fallback codegen is `.into()`, so a bare `T` auto-wraps into an `Option<T>` field.
The one exception: `Option<Rc<dyn Fn(...)>>` props still need `Some(Rc::new(...))`,
because `.into()` cannot trigger Rust's unsizing coercion.

### Native control flow in RSX

`rsx!` supports native Rust `if`/`for`/`match`, and all of it is **always reactive** —
conditions, iterators and scrutinees are auto-wrapped in closures and tracked by
Effects.

```rust
rsx! {
    div {
        // if/else — desugars to show_dom()
        if visible.get() { p { "Shown" } } else { p { "Hidden" } }

        // for — desugars to for_each_dom_typed(), keyed reconciliation (LIS)
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

`if let`, `else if` chains, match guards and pattern bindings are all supported. For
loop items need `Clone + PartialEq + 'static`, and a `key:` prop for stable identity.

### Component pattern

```rust
#[component]
fn my_component(title: &str) -> NodeHandle {
    // __scope is auto-injected by the macro
    let count = Signal::new(0);
    rsx! {
        div {
            h1 { {title} }
            p { {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "+" }
        }
    }
}
```

A **PascalCase** `#[component]` name generates a struct + `Component` impl instead;
its params become public owned fields, and `children: &[NodeHandle]` is special-cased
to the trait's `render`.

There are **no hooks**. `use_signal`/`use_effect`/`use_memo`/`HookRegistry` were
removed — call `Signal::new()` / `Effect::new()` / `Memo::new()` directly. There is no
"hook ordering" rule to obey, and no `Show`/`For` components (only the runtime
functions `show_dom()` / `for_each_dom_typed()` / `match_dom()`).

## Per-Crate Agent Guidance

### rinch-core (reactive kernel) — highest risk

`reactive/scope.rs` (ownership, disposal, the dispose fixpoint) and `reactive/mod.rs`
(the runtime, subscriber sets, flush) are the most dangerous files in the workspace: a
bug there breaks every app. `reactive/signal.rs`, `effect.rs`, `memo.rs` sit on top.
`dom/traits.rs` (`DomDocument`), `dom/mod.rs` (`NodeHandle`) and `dom/render_scope.rs`
(`RenderScope`) are the contract between the reactive layer and every renderer, so a
signature change there ripples into `rinch-dom`, `rinch-web`, `rinch-editor-view` and
`rinch` at once.

`dom/mock.rs` (`MockDomDocument`, behind the `test-util` feature) is how downstream
crates test against the seam without a renderer. Prefer it over reaching for
`rinch-dom` in a test.

### rinch-dom (CSS + layout + paint)

`ifc.rs` (inline formatting / Parley), `paint/` (the `Painter` trait plus the Vello and
tiny-skia backends), `layout_engine.rs` + `layout.rs` (Taffy), `computed_style/` (Stylo
→ computed values), `style_resolution/`, `stylesheet/`, `dom_impl/` (`RinchDocument`),
`stylo_impl.rs` (the Stylo DOM trait impls).

- Stylo integration has many trait impls with subtle requirements — read before editing.
- Paint order matters; layout measurement and paint must use the same font stack or
  text clips.
- Taffy is pinned at **0.12** in two places (`rinch-dom` and the vendored
  `stylo-taffy`) which must move together.

### rinch-macros (proc macros)

`dom_codegen/` (`control_flow.rs` is the biggest and trickiest), `node.rs`,
`element.rs`, `prop.rs`. Changes here affect every component in the project, and prop
auto-wrapping is the subtlest part — a wrong wrap surfaces as a confusing type error in
user code, far from the macro. Codegen is mostly permissive rather than validating (a
prop the macro doesn't recognise falls through to `.into()`), so a typo in `rsx!`
usually shows up as a type error at the component's field, not as a macro diagnostic.

### rinch (desktop runtime / facade)

`app/event_dispatch.rs` and `app/mod.rs` are the highest-coupling files — event
processing, hit testing, focus, the render loop. `shell/rinch_runtime.rs` owns the
winit event loop and window creation. Also here: `shell/desktop.rs` (wgpu / GPU device
injection), `shell/softbuffer_renderer.rs`, `shell/devtools_panel.rs`,
`render_surface.rs`, `embed.rs`, `menu/`, `editor/`.

The `prelude` re-exports everything downstream users need; feature flags gate the heavy
dependencies.

### rinch-components

Each component is self-contained and low-risk to modify individually. They implement
`Component::render(&self, scope, children) -> NodeHandle`. Follow existing patterns —
`badge.rs` for something simple, `tabs.rs` or `tree.rs` for something complex. `_fn`
suffix props (`value_fn`, `checked_fn`) exist for surgical updates without a full
component re-render. There are no unit tests here; verify visually through ui-zoo.

### The editor crates

Read `docs/src/architecture/editor.md` before touching any of them. The invariant that
matters: **the model is the only source of truth and the host tree is derived from it.**
Never read the host for content, never add a second mutation path, and keep
`rinch-editor-core` free of renderer and CRDT dependencies (its manifest states the ban
list; CI lints it for `wasm32`).

## Build & Test

```bash
cargo build                          # Build all workspace crates
cargo build -p ui-zoo-desktop        # Build the primary example
cargo run --release -p ui-zoo-desktop  # Run it — ALWAYS use --release, debug is too slow
cargo test --workspace               # Tests
cargo clippy --workspace --all-targets -- -D warnings   # Lint (matches CI)
cargo fmt --all -- --check           # Format check
```

WASM: `cargo check --target wasm32-unknown-unknown`, or
`cd examples/ui-zoo-web && trunk serve --release`. Android:
`./examples/hello-android/build-apk.sh`.

**The pre-commit hook is the correctness gate.** `./scripts/setup-hooks.sh` installs a
hook that runs fmt → clippy (`-D warnings`) → the `ui-zoo-web` wasm check → the full
workspace test suite. It mirrors CI, so a clean hook run means a clean CI run.

**CI jobs** (`.github/workflows/ci.yml`): `test` (workspace tests **plus** an explicit
feature-gated set), `clippy`, `clippy-wasm` (per-crate `wasm32` lint for
`rinch-core`, `rinch-editor-core`, `rinch-editor-collab`, `rinch-editor-view`,
`rinch-http`, `rinch-ws`, `rinch-web`), `test-wasm` (headless Chrome, for
`rinch-storage` and `rinch-web`), and `fmt`.

**Feature-gated tests are a trap.** A test file whose `cfg` is false compiles to an
*empty* binary that still prints `Running tests/x.rs` — only the test **count** reveals
it. And `cargo test --workspace` unifies features across members, so a gated test can
be alive purely because some unrelated example turns its feature on. If you add a
gated test, add its feature combination to the "Run feature-gated tests" step; don't
assume the workspace run covers it. (`--all-features` is not an option — it pulls a
transitive `ashpd` that fails to build.)

**System deps (Linux):** GTK3, GLib, ATK, Cairo, Pango, gdk-pixbuf, libxdo, libmpv
development packages.

**wgpu fork:** the workspace patches wgpu crates from `joeleaver/wgpu-fork` (branch
`rinch-patch`) for transparent-window support. Cargo patches are not transitive, so
downstream consumers must copy the `[patch.crates-io]` section into their own
workspace.

## Common Pitfalls

1. **Non-reactive expressions.** `{count.get()}` captures once; `{|| count.get()}`
   creates an Effect. The most common bug by a wide margin.
2. **Double-wrapping props.** The `rsx!` macro auto-wraps; `Some(...)` or
   `Callback::new(...)` inside `rsx!` produces confusing type errors.
3. **Missing `desktop` feature.** The workspace dependency sets
   `default-features = false`, so `run()` and the rendering APIs are unavailable
   without `features = ["desktop"]`.
4. **Cross-thread signal writes.** `set()`/`update()` panic off the main thread; use
   `send()`/`update_send()`.
5. **Using a disposed reactive handle.** Since #141, a freed `Signal`/`Memo` panics on
   read. If a resource can outlive its component (a timer, a global registry, a parked
   callback), cancel it on unmount or explicitly `.leak()` it.
6. **Thread-local registries keyed by bare node id.** Node ids are per-document slab
   indices and collide across documents on one thread (two windows, two embedded
   contexts). Always key by `DomDocument::doc_key()` too.
7. **`app/event_dispatch.rs` coupling.** Event handling, hit testing, focus and
   rendering interact; changes there have non-obvious reach.
8. **Stylo complexity.** The Firefox CSS engine integration has many trait impls with
   subtle requirements. Read existing code carefully first.
9. **Concurrent cargo processes** block on the target-dir file lock. Kill stale ones;
   don't run cargo in parallel across agents.
10. **`cargo check` inside `examples/ui-zoo-web`** fails on fontconfig — Parley pulls
    it in on native Linux only. Always pass `--target wasm32-unknown-unknown` there.

## Testing with MCP

The `rinch-mcp-server` binary lets an agent see and drive a running app — screenshots,
live DOM with layout bounds and computed styles, and input injection. Use it to
*verify* a visual change instead of guessing.

```
launch_app(package: "ui-zoo-desktop")   # Build, launch, auto-connect
screenshot()                            # Inline PNG — directly viewable
dom_tree()                              # Full DOM with layout + computed styles
query_selector(selector: ".my-class")   # Find nodes
get_computed_styles(id: 42)             # Resolved CSS for a node
click(x: 100, y: 200) / wait_frame()    # Simulate input, then re-screenshot
close_app()                             # Clean shutdown
```

The repo's `.mcp.json` invokes `rinch-mcp-server` **by name**, so it must be on `PATH`:
install it with `cargo install --path crates/rinch-mcp-server`, and **re-install after
changing its source** — a `cargo build` alone won't update the installed binary. (An
alternative `.mcp.json` can point `command` at `target/debug/rinch-mcp-server` instead.)

Node geometry is reported twice: `layout` is **parent-relative**, `absolute` is
on-screen — pass `absolute` coordinates to `click()`. Headless: run under Xvfb with
`DISPLAY` set (`launch_app` forwards it).

The app under test needs `features = ["debug"]`; the server auto-starts and can be
disabled at runtime with `RINCH_DEBUG=0`.

## Documentation Expectations

User-facing changes must update the docs in the same change: the relevant page under
`docs/src/guide/`, `docs/src/architecture/` for structural changes, doc comments on new
public APIs, and `CLAUDE.md` when adding a reactive primitive, an element type, or an
architectural boundary. Add new pages to `docs/src/SUMMARY.md`.
