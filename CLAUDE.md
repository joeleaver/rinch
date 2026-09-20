# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Rinch is a lightweight cross-platform GUI library for Rust, built on rinch-dom, Taffy, Parley, and dual rendering backends (Vello for GPU, tiny-skia for software). The goal is to provide a reactive GUI framework using HTML/CSS for layout.

**Key dependencies:**
- **rinch-dom** - HTML/CSS DOM implementation (Taffy for layout, Parley for text, Painter trait for rendering). Taffy is pinned at **0.12** in two places — `crates/rinch-dom/Cargo.toml` and the vendored `crates/stylo-taffy/Cargo.toml` — which must move together. 0.9 could not resolve a percentage `min-height`/`max-height` against a block containing block (its block algorithm hard-coded that basis as indefinite), so `min-height: 100%` silently collapsed to the content height.
- **parley** - Text shaping and line breaking, at the **published `0.11.1`**. It was a git `rev`
  on an unmerged 22-commit "Floats WIP" branch off `v0.7.0` until #659; upstream landed that work
  in 0.9.0, so nothing was lost by moving to the release line. Two things about the dependency line
  are load-bearing and neither is enforced by the compiler:
  - **`features = ["complex-scripts"]` is on, and must stay on.** It is not in parley's `default`.
    Without it icu4x has no Thai/Lao/Khmer/Burmese segmentation dictionary and a Thai paragraph
    finds **no** line-break opportunity at all — measured, 4 lines become 1 and the text runs off
    the container — plus an `ICU4X data error: No segmentation model for language: th` on stderr
    **twice per layout**. CJK is *not* affected (its breaking comes from Unicode line-break
    classes), so a CJK fixture cannot stand in for a Thai one.
    `crates/rinch-dom/tests/complex_scripts_tests.rs` is the pin.
    It is not free: the dictionaries cost about **+3.80 MB of stripped release binary**
    (measured on this host by #659's review, on a one-crate probe differing only in the parley
    dependency line — no `icu_*` crate is *added*, they were already in stylo's graph). It stays
    on by default because CJK and Thai line breaking should be correct out of the box; **#660**
    is where exposing it as a rinch cargo feature for size-sensitive embed/wasm builds is being
    considered.
  - **`parley::Glyph::y` is Y-down** since parley #528 (0.8.0). The struct did not change, only the
    sign of what parley puts in it, so a consumer that subtracts it still compiles and still looks
    right in English: `y` is 0 for ordinary Latin shaping and non-zero only for mark positioning.
    `crates/rinch-dom/src/paint/text.rs` **adds** it, in the main pass and the shadow pass alike;
    `crates/rinch-dom/tests/glyph_y_ydown_tests.rs` is the pin, one fixture per site.
- **vello** - 2D GPU rendering via wgpu (GPU mode, enabled with `features = ["gpu"]`)
- **tiny-skia** - 2D software rendering (default mode, no GPU required)
- **softbuffer** - Software window presentation (default mode)
- **winit** - Cross-platform windowing and input
- **muda** - Native menu support

**Design philosophy:** Declarative UI with fine-grained reactive programming. The `rsx!` macro generates DOM construction code with Effects for reactive updates. Only changed DOM nodes are updated - no full re-renders.

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
```

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

## Icon System

There is **one** icon system: the **`TablerIcon` enum** in `rinch-tabler-icons`, which is also what every component's icon prop takes (`Option<TablerIcon>`). There is no `Icon` enum in the workspace — an older curated `rinch-core` `Icon` enum was removed, so treat any doc comment or snippet still showing `Icon::CheckCircle` as stale.

`TablerIcon` is **not** in the `rinch` prelude — the facade doesn't depend on `rinch-tabler-icons`. Any crate that names an icon variant (including one just passing `icon:` to a component) must depend on it directly.

**Add to Cargo.toml:**
```toml
rinch-tabler-icons = { workspace = true }
```

The crate generates the enum at build time from **vendored JSON committed under `crates/rinch-tabler-icons/data/`**, pinned to `@tabler/icons@3.36.1`, so it builds with no network. It only downloads from unpkg if a vendored file is missing or invalid.

**Usage:**
```rust
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

#[component]
fn my_component() -> NodeHandle {
    rsx! {
        div {
            // Render an icon
            {render_tabler_icon(__scope, TablerIcon::Home, TablerIconStyle::Outline)}

            // Filled variant
            {render_tabler_icon(__scope, TablerIcon::Heart, TablerIconStyle::Filled)}
        }
    }
}
```

The `tabler_icon!` macro is a shorter equivalent when the icon is a literal variant name: `tabler_icon!(__scope, Home)` (Outline is the default style) or `tabler_icon!(__scope, Home, Filled)`.

For size, stroke width, or a CSS class, use `render_tabler_icon_with_options(__scope, icon, TablerIconOptions { style, class, size, stroke_width })` — all fields but `style` are `Option`, so `..Default::default()` covers the rest.

**Features:**
- **4,985 icons** (`ICON_COUNT`; iterate `ALL_ICONS`), every one available in Outline
- **Filled is a subset** — about 1,000 icons have real filled artwork. Asking for `TablerIconStyle::Filled` on any other icon silently falls back to its outline paths, so a glyph that still looks like an outline is expected, not a bug
- **Type-safe** - Use enum variants instead of strings
- **Scales with font size** - Icons carry a `0 0 24 24` viewBox and are sized `1em` unless you pass an explicit `size`, so they track the parent's `font-size`
- **Themeable** - Outline icons use `stroke: currentColor` (default `stroke-width: 2`), filled icons `fill: currentColor`
- **Offline build** - Generated from the vendored JSON in `data/`; no network needed

**Using with ActionIcon:**
```rust
// Pass the rendered icon as a child
ActionIcon {
    variant: "subtle",
    onclick: || do_something(),
    {render_tabler_icon(__scope, TablerIcon::Menu2, TablerIconStyle::Outline)}
}
```

**Sample Categories:**
- Navigation: `Home`, `ArrowLeft`, `ArrowRight`, `ChevronUp`, `ChevronDown`, `Menu2`
- Actions: `Plus`, `Minus`, `X`, `Check`, `Edit`, `Trash`, `Search`, `Settings`
- Status: `AlertCircle`, `AlertTriangle`, `CircleCheck`, `InfoCircle`, `CircleX`
- Communication: `Mail`, `Phone`, `Message`, `Bell`, `Send`, `Share`
- Media: `Photo`, `Video`, `Music`, `Microphone`, `Camera`, `PlayerPlay`
- Files: `File`, `Folder`, `Download`, `Upload`, `Copy`, `ClipboardCopy`

### Components with Icon Support

Every one of these props is `Option<TablerIcon>`. The `rsx!` macro adds the `Some(...)` for you, so write `icon: TablerIcon::Check`, never `icon: Some(TablerIcon::Check)`.

| Component | Icon props | Source |
|--------|-----------|--------|
| `ActionIcon` | `icon` | `action_icon.rs:122` |
| `Alert` | `icon` | `alert.rs:161` |
| `Notification` | `icon` | `notification.rs:113` |
| `AccordionControl` | `icon` | `accordion.rs:251` |
| `Blockquote` | `icon` | `blockquote.rs:25` |
| `List`, `ListItem` | `icon` | `list.rs:109`, `list.rs:233` |
| `Stepper` | `completed_icon`, `progress_icon` | `stepper.rs:143`, `:150` |
| `StepperStep` | `icon`, `completed_icon`, `progress_icon` | `stepper.rs:544`, `:546`, `:548` |
| `NavLink` | `left_section`, `right_section` | `navlink.rs:100`, `:102` |
| `DropdownMenuItem` | `left_section`, `right_section` | `dropdown_menu.rs:483`, `:485` |
| `Tab` | `left_section`, `right_section` | `tabs.rs:397`, `:399` |

The `Tree` component takes its icons through data rather than a prop: `TreeNodeData::icon` (`tree.rs:56`), set with the `with_icon(TablerIcon)` builder.

**`List::icon` and the two `Stepper` icons are container *defaults*, and they
work by patching the rendered children** (#707). A parent component renders
*after* its children — the rsx macro builds the children into a `<template>` and
hands the finished nodes to `Component::render` — so nothing a container knows
can reach an item as a prop. `List` therefore finds each `.rinch-list__item`
that carries no icon of its own and rebuilds it into the layout `ListItem` would
have built; `Stepper` replaces the glyph in each step icon box that its
`StepperStep` marked `data-icon-fallback`. **The item's own icon always wins**,
and `Stepper::progress_icon` stands in for the *`progress_icon`* a step did not
set, which therefore outranks that step's plain `icon` exactly as its own
`progress_icon` would have. All three were declared and read by nothing until
#707; the doc example on `Stepper` (`completed_icon: TablerIcon::CircleCheck`)
is one of the things that now does what it says.

**`Stepper::active` travels the same way** (#709): one pass gives each step the
index of its position and the state that position has against `active` — before
it completed, at it in progress, after it inactive. A step that named a `state`
or a `step` of its own keeps it, and `data-state` is what carries that ask,
because `state: String` renders an explicit `"inactive"` and an unset field into
the same class. A closure prop (`active: {|| sig.get()}`) re-renders the whole
component, children included, so the derivation simply runs again. The delicate
half is the icon: a step draws its icon *before* its parent exists, and the icon
for a state it was moved into is a `TablerIcon` in its props rather than anything
in its DOM. So a step with no `state` of its own renders those icons and leaves
them in its icon box as **hidden alternates**
(`.rinch-stepper__step-icon-alt`, `display: none` inline *and* from the sheet);
the parent promotes the one it needs. Promotion re-parents
**before** clearing the box, and a glyph that is gone for good leaves by
**`discard`** (#719) because `rinch-web` compiles this crate. Those
two facts are one fact: a `discard` retires the whole subtree, so discarding a
wrapper that still held the promoted glyph would retire the glyph with it. The
selection family stays decorative: no step carries a `data-rid` and `Stepper`
takes no callback (issue #737).

**A container default reaches a child that arrives after the container rendered**
(#716), **and a container that counts positions is re-derived when one leaves**
(#745). The parent patch is still there and still runs first — a parent
component renders *after* its children, so nothing a container knows can travel
as a prop — but the container also registers
`rinch_core::dom::on_child_inserted` against its own root, and the four
`NodeHandle` verbs that put a node into a tree (`append_child`, `insert_before`,
`insert_after`, `replace_with`) tell **every** registered ancestor, nearest
first, synchronously. So a `for` reconcile, a `show_dom` branch and a hand-rolled
`append_child` all land with the default in place before the frame is laid out,
with no deferred queue and no drain site in any host — which is what the
alternative would have needed, since `queue_main_callback` takes a `Send`
closure (a `NodeHandle` is `!Send`) and `rinch-web` drains it nowhere.

`rinch_core::dom::on_child_removed` is the other half, fired by the four verbs
that take a node **out** of a tree (`remove_child`, `remove`, `discard`, and
`replace_with` for the node it displaces) plus the implicit detach an insertion
verb performs when handed a node that already has a parent — which is the only
thing that tells a container a child *moved away*. The two halves are separate
registrations: `Stepper` takes both, `List` and `RadioGroup` only the first,
since neither of their defaults can be changed by a row going away.

**The removal half is handed the node the subtree LEFT — its former parent —
not the node that went**, and that asymmetry is forced rather than chosen. A
removed node is detached, so it has no ancestor chain for an observer's boundary
test to walk, and after a `discard` the backend may have retired it (`remove`
and `discard` are one verb to a `for` reconcile). Every removal verb therefore
reads that parent *before* it mutates the document; a `Cell` counted separately
from the insertion one keeps an app whose containers only care about arrivals
from paying that read. A reorder **inside** one parent fires the insertion half
only: the child set is unchanged, and firing both would make an idempotent
container re-derive twice per moved row.

The pieces that follow from it:
- **The boundary is the observer's own question.** Every registered ancestor is
  told, because two containers of *different* kinds can nest; each declines a
  node whose chain up to it crosses one of its own items, which is its
  render-time walk's rule read upwards. A `List` inside another list's item owns
  its own rows.
- **A callback's own edits do not call it back.** Dispatch is suppressed for the
  duration of a callback; without that, a container that patches the subtree it
  watches recurses without bound (measured: stack overflow).
- **An observer is released by the ambient scope's `on_cleanup`**, the #147
  discipline, and by `NodeHandle::discard` on the container — a discarded id may
  be reissued.
- **A child moved between containers re-resolves**, because a container marks
  what it supplied (`data-list-icon`) and never touches what the child asked for
  itself.
- **`Stepper` re-runs its whole pass, not just the newcomer**: an insertion
  renumbers the steps after it and can restate them. That pass is idempotent by
  construction — `data-step-derived` says who wrote an index, `data-icon-has`
  records the step's icon *props* rather than what it drew, `data-icon-live`
  names the content key showing — and a glyph the step's props supplied is
  **parked** hidden rather than discarded when it stops being drawn, since a
  later insertion — or a keyed `for` **reorder**, which repositions a live node
  with `insert_before` and can move it *backwards* — can want it back. Every
  alternate is kept while the stepper owns the step's state; one that named its
  own `state` keeps none. A **removal** runs the same whole pass for the same
  reason, through `on_child_removed` (issue #745): a step that goes moves every
  step behind it backwards, which renumbers it and can restate it.
- **A `StepperCompleted` is not a position, and neither is anything inside it**
  (issue #741). The step walk skips that block: it is what a stepper shows
  *instead of* its steps once they are all done. It used to walk straight into
  it, re-deriving a nested stepper's steps at the outer stepper's indices.
  **A skip, not a stop** — Mantine writes `Stepper.Completed` last by convention
  but does not require it and never stops counting at it, so the count runs
  straight across the block and a step after it is a position like any other.
  Reading it as terminal makes a stepper whose block is written *first* derive
  nothing at all: no step numbered, every step inactive, two steps both drawing
  the number `1`.
- **Cost:** one `Cell` read per insertion (and one per removal) while **nothing
  on the thread** is registered for that half — the counts are thread-local, not
  per document, so one live container anywhere makes every insertion in every
  document on that thread pay an ancestor walk. That walk is **0.12 µs** per insertion at depth 8 (0.16 µs at
  depth 1, 0.56 µs at depth 32 — roughly 0.013 µs per level), measured on 5000
  appends, software build, best of 40. A registered **removal** observer costs
  about the same per removal (+0.13 µs at depth 8, against a 0.54 µs
  `MockDomDocument` detach) and adds **+0.02 µs to every insertion** on the
  thread, for the parent read an insertion verb makes to find out whether it is
  moving a node out of somewhere. `Stepper` is the one container whose own patch
  is O(n) per change, so growing *or shrinking* one step at a time is quadratic:
  10.6 ms for 100 steps, against 0.11 ms for the ten a real stepper has (#748).

`RadioGroup::size` and the `Stepper` props are the same shape.

Paths are relative to `crates/rinch-components/src/`. `ActionIcon`'s `icon` prop is a convenience that renders the icon for you as Outline, sized from the component's own `size` prop. It is **mutually exclusive with children** — `loading` wins, then `icon`, and children render only if neither is set — so pass a rendered icon as a child (not via `icon:`) when you need a filled or custom-sized glyph.

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

## Application Entry Point

**`App` is the entry point** (issue #493). Every startup option is one method and
they all compose. Theme and component CSS load automatically when those features
are enabled, so components work out of the box even with no `.theme(...)`:

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
    App::new(app).title("My App").size(800, 600).run();
}
```

| Method | Purpose |
|--------|---------|
| `App::new(component)` | Start configuring around the root component |
| `.title(title)` / `.size(w, h)` | Window title and initial logical size |
| `.theme(ThemeProviderProps)` | Colors, radius, dark mode |
| `.fonts(&[AppFont])` | Typefaces the build carries in its own binary (issue #286) |
| `.menu(Vec<(&str, Menu)>)` | Native menu bar |
| `.window_props(WindowProps)` | Borderless, transparent, icon, `app_id`, `on_close_requested`, … |
| `.gpu_config(RinchGpuConfig)` / `.external_gpu(ExternalGpu)` | GPU device (`gpu` feature) |
| `.run()` | Start on desktop; runs until the event loop exits |
| `.run_android(android_app)` | Start on Android (`android` feature, Android target) |

`.title()` / `.size()` are applied **over** `.window_props()` in either call
order, so an explicit title is never silently lost to a later `window_props`.
Everything else comes from `props`. Nothing runs until a terminal method.

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

The seven `run_*` functions (`run`, `run_with_theme`, `run_with_menu`,
`run_with_window_props`, `run_with_window_props_and_menu`, `run_with_gpu_config`,
`run_with_external_device`) and the two Android ones (`run_android`,
`run_android_with_theme`) are now **`#[deprecated]` shims over `App`**, and each
deprecation note names its builder chain. Behaviour is unchanged with one
deliberate exception: unifying the startup paths also unified the Linux
**Wayland `app_id`**, which is now derived per application from the executable
name instead of the shared constant `"rinch-app"` (an explicit
`WindowProps::app_id` still wins). X11 `WM_CLASS` and every non-Linux platform
are untouched. See **Window identity on Wayland** below.
They exist because none of them could express its own combinations:
`run_with_menu` took no window props, `run_with_window_props` took no menu
without a second function, and neither could take whatever landed next.

### App-bundled fonts (`.fonts`)

`App::fonts(&[AppFont])` registers typefaces the build carries, before the
first layout pass — which is the whole reason it is a builder method and not a
function you call beforehand: a face that arrives after the first measurement
cannot un-measure it, so the first frame renders in a fallback and reflows.
Applies on Android too (the platform that needs it most), and it is the one
configuration method besides `.theme()` that does. A second `.fonts()` call
**appends**, unlike `.title()` / `.menu()`, which replace.

**A face is reachable by its own family name, and by nothing else, unless it
asks.** Registering `Newsreader.ttf` answers `font-family: Newsreader`
immediately. A *generic* — `serif`, `sans-serif`, `monospace`, `system-ui` — is
not the name of anything; it is a slot the platform fills, and a bundled face is
in one only by declaring it: `AppFont::serif/sans_serif/monospace(bytes)`, or a
struct literal over `generics` for the rest. Since essentially every real stack
ends in a generic, a face that claims none will usually not be picked at all.

**A claim is a prepend, not an append** (`rinch_dom::fonts::claim_generic_families`).
The #322 Android repair fills an empty generic slot by *appending* a platform
family into the collection's own list, at context construction, before any app
font can register — so an appended claim would sit behind it and silently lose,
on Android only. What was in the slot stays behind the claim, so a character the
declared face lacks still falls through to the platform's entry.

**`AppFont::script_fallback` is off by default and should stay off wherever the
platform has fonts.** fontique resolves a script's fallback from the app's own
list first and the platform's second — as an alternative, not a chain — so an
entry there *replaces* that script's platform fallback. Measured, not argued:
with it set on a Latin face, `漢字` in a stack headed by that face resolves to
the bundled face at **glyph 0** (`.notdef`), where without it the platform's CJK
face answers with real glyphs. It is on for `RinchApp::register_font_data`, the
wasm/embed front door, because wasm has no system fonts and a last-resort face
is the only thing that renders a character the author did not name a font for.

Guide: `docs/src/guide/fonts.md`. Fixtures live in `crates/rinch/assets/fonts/`
and `crates/rinch-dom/assets/fonts/` rather than under `examples/`, because
`cargo package` does not carry `examples/` and each crate has to be able to
compile its own test target.

### Window identity on Wayland

A window's `app_id` is how a Wayland compositor decides which `.desktop` entry,
icon and taskbar group a window belongs to. rinch derives it from the
executable's file name unless `WindowProps::app_id` says otherwise
(`resolved_app_id`, `shell/rinch_runtime.rs`), at **both** the initial
`create_window` and the `show_window` re-creation path that minimize-to-tray
restores through.

It used to default to the constant `"rinch-app"` for every app that did not set
one. That is not merely a label: `install_wayland_icon` writes
`~/.local/share/applications/{app_id}.desktop` carrying `Name={app_id}`, so
under one shared constant the first rinch app to ship an icon supplied the dock
icon *and* the displayed name for every other rinch app on the machine, and
they grouped together as a single application. The whole name is used, not the
`file_stem` — a stem truncates at the last dot, so a reverse-DNS binary name
like `com.example.notes` would register as `com.example` and collide with its
own siblings.

The `#[component]` macro auto-injects `__scope: &mut RenderScope` as the first parameter, which is required by the `rsx!` macro. Components return a `NodeHandle`. You can also write `fn app(__scope: &mut RenderScope) -> NodeHandle` manually if preferred.

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

## State Management

Component functions run **once** to build the DOM. Reactive closures (`{|| expr}`) in rsx handle all subsequent DOM updates surgically.

### Core Primitives

| Primitive | Purpose |
|-----------|---------|
| `Signal::new(value)` | Reactive state that triggers updates |
| `Memo::new(closure)` | Cached computed values |
| `create_store(value)` | Share a store across components (recommended for shared state) |
| `use_store::<T>()` | Access a shared store (panics if missing) |
| `try_use_store::<T>()` | Try to access a shared store (returns Option<T>) |
| `create_context(value)` | Low-level shared state (used by framework internals) |
| `use_context::<T>()` | Access shared context (panics if missing) |

> **Note:** `Effect` is intentionally excluded from the prelude. Use `{|| expr}` in rsx for reactive DOM updates, and store methods for side effects. For rare advanced cases (syncing to external systems), import explicitly: `use rinch::reactive::Effect;`

### Basic Example

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Closure syntax {|| ...} creates reactive DOM updates
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    App::new(app).title("My App").size(800, 600).run();
}
```

> **Important:** The closure syntax `{|| expr}` is required for fine-grained reactive updates. Without it, values are captured once at initial render and never update. See [RSX Syntax - Reactive Expressions](docs/src/guide/rsx-syntax.md#reactive-expressions). The `#[component]` macro injects `__scope` automatically, so the `rsx!` macro works without manually declaring it.

### Store Pattern (Recommended for Shared State)

A store is a struct with `Signal` fields and methods that encapsulate state mutations. Use `create_store()` / `use_store()` to share it across components:

```rust
#[derive(Clone, Copy)]
struct CounterStore {
    count: Signal<i32>,
}

impl CounterStore {
    fn new() -> Self {
        Self { count: Signal::new(0) }
    }
    fn increment(&self) {
        self.count.update(|n| *n += 1);
    }
}

#[component]
fn app() -> NodeHandle {
    create_store(CounterStore::new());
    rsx! { Counter {} }
}

#[component]
fn counter() -> NodeHandle {
    let store = use_store::<CounterStore>();
    rsx! {
        p { {|| store.count.get().to_string()} }
        button { onclick: move || store.increment(), "+" }
    }
}
```

For component-local state, just use `Signal::new()` directly — no store needed.

### Primitive Reference

**`Signal::new()`** - Reactive state:
```rust
let count = Signal::new(0);
count.get();              // Read value
count.set(5);             // Set new value
count.update(|n| *n += 1); // Update with function

// Cross-thread: send() auto-dispatches to the main thread
std::thread::spawn(move || {
    count.send(10);                       // T: Send required
    count.update_send(|n| *n += 1);       // closure must be Send + 'static
});
```

**`Memo::new()`** - Cached computed values:
```rust
let count = Signal::new(0);
let multiplier = Signal::new(2);

// Automatically tracks count and multiplier
let result = Memo::new(move || count.get() * multiplier.get());
```

**`create_store` / `use_store`** - Shared state across components:
```rust
#[derive(Clone, Copy)]
struct AppStore {
    dark_mode: Signal<bool>,
    user_name: Signal<String>,
}

#[component]
fn app() -> NodeHandle {
    create_store(AppStore {
        dark_mode: Signal::new(false),
        user_name: Signal::new("Guest".into()),
    });
    // ...
}

#[component]
fn child_component() -> NodeHandle {
    let store = use_store::<AppStore>();
    // For optional access: let store = try_use_store::<AppStore>(); // returns Option<T>
    // ...
}
```

**`create_context` / `use_context`** - Low-level shared state (still works, appropriate for framework internals):
```rust
#[derive(Clone)]
struct Theme { color: String }

#[component]
fn app() -> NodeHandle {
    create_context(Theme { color: "#007bff".into() });
    // ...
}

#[component]
fn child_component() -> NodeHandle {
    let theme = use_context::<Theme>();
    // ...
}
```

## Threading Model

Rinch **owns the main thread**. `App::run()` calls winit's event loop, which takes over the thread and returns only once the application exits it (`close_current_window`, or an `on_close_requested` answering `true`). All UI state (`Signal`, `Effect`, `NodeHandle`, `RenderScope`) is `!Send` — the reactive system is thread-local.

**Async / tokio:** A tokio runtime can coexist but only on a **separate background thread**. You cannot run `#[tokio::main]` and `App::run()` on the same thread.

```rust
fn main() {
    // Spawn tokio on a background thread
    let rt = tokio::runtime::Runtime::new().unwrap();
    std::thread::spawn(move || {
        rt.block_on(async {
            // networking, file I/O, etc.
        });
    });

    // Main thread — rinch owns it
    App::new(app).title("My App").size(800, 600).run();
}
```

**Sending results back to the UI thread:** Use `Signal::send()` and `Signal::update_send()`, which auto-dispatch to the main thread from any thread. The `T` must be `Send`.

```rust
let data = Signal::new(Vec::new());

std::thread::spawn(move || {
    let result = fetch_data();     // blocking work
    data.send(result);             // dispatches to main thread
    data.update_send(|v| v.sort());// closure runs on main thread
});
```

**Key constraint:** Never call `Signal::set()` or `Signal::update()` from a background thread — they panic. Always use the `_send` variants for cross-thread updates.

**Embed works the same way** (issue #172). A `RinchContext` arms cross-thread
dispatch too, and drains the queued writes at the top of each `update()` —
before that frame's events, so a queued write and an event handler land in one
layout pass. `run_on_main_thread` and everything riding it (`set_timeout`,
`rinch-http`, `rinch-ws`) work in embed for the same reason. The queue itself
lives in `rinch-core` (`queue_main_callback` / `drain_main_callbacks`) so the
desktop shell, the Android loop and embed all share one; the shell's dispatcher
adds the "wake the event loop" side effect that embed has no use for.

## Native Menus

Native menus use a unified `Menu`/`MenuItem` builder API shared between window menu bars and tray context menus. Add a menu bar with `App::menu`:

```rust
use rinch::prelude::*;
use rinch::menu::{Menu, MenuItem};

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { "Application content" }
    }
}

fn main() {
    let file_menu = Menu::new()
        .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(|| println!("New!")))
        .separator()
        .item(MenuItem::new("Exit").on_click(|| std::process::exit(0)));

    let edit_menu = Menu::new()
        .item(MenuItem::new("Undo").shortcut("Ctrl+Z"));

    App::new(app)
        .title("My App")
        .size(800, 600)
        .menu(vec![("File", file_menu), ("Edit", edit_menu)])
        .run();
}
```

Menu callbacks are `impl Fn() + 'static` — no `Send`/`Sync` required. They always run on the main thread, so Signal is safe to capture (Signal is Copy, no clone needed):

```rust
let count = Signal::new(0);

MenuItem::new("Reset Counter").on_click(move || count.set(0))
```

**Callback lifetime (#183).** A callback belongs to the component that *created*
it — the scope rendering when `on_click` was called, where the closure captured
its signals — and stops firing once that component unmounts, rather than reading
freed state. Ownership is per **item**, not per menu build, so one `Menu` may
collect items from several components and each stops on its own. A live callback
runs *inside* its owner, so a `Signal` it creates belongs to the menu's
component; an ownerless one runs `unowned`. Built outside any render — from
`main`, before the event loop, which is what every example does — there is no
owner and the callback keeps **app lifetime**, unchanged. Every activation path
goes through one `invoke_menu_callback`, so the rule holds for a muda click, a
tray click, the DOM menu bar — on Linux *and* in the browser, which fires the
`Rc` straight out of the `Menu`, not through the registry — and the keyboard
shortcut alike.

**`rinch::menu` needs no windowing at all.** The declaration types and the DOM
bar (`render_with_menu_bar`) build with `default-features = false`; only the
`muda` and `winit` halves are `#[cfg(feature = "desktop")]`. That is what lets
`rinch_web::mount_with_menu_bar` render the *same* bar from the *same* `Menu`
values a desktop build hands `App::menu` — see `docs/src/guide/wasm.md`. CI's
`cargo test -p rinch --no-default-features --features components,theme` is the
decoupling gate; the tests it runs also run under `--workspace`.

A **shortcut consumes the keystroke only when a callback actually runs.** A chord
whose item is disabled, has no `on_click`, or belongs to an unmounted component
falls through to the app rather than being swallowed, and every chord matching
the key is tried in registration order — so a dead duplicate cannot shadow a live
one. On the desktop the shell `return`s on a match, so no input target sees the
key; the web says the same thing with a **`window`**-capture `keydown` listener
whose `preventDefault` + `stopPropagation` stop the event before it reaches
`document` — where `editor_input`'s keymap and the bubble delegate sit. `document`
capture was not enough: `stopPropagation` aborts the walk to the *next* node and
leaves same-node listeners running, so a focused editor acted on `Ctrl+Z` twice
(issue #806). The pointer-gesture observer is on `window` for the other half of
that reason — it must go on seeing a consumed chord.

**The chords a web bar arms come back down when its root unmounts** (issue #805).
`register_menu_shortcuts` returns a `MenuBarChords` token that
`rinch-web`'s `menu_bar::wrap` holds through `scope.on_cleanup`; dropping it
releases the build **only while `MENU_BAR_REGISTRATION` still holds that build**,
because two islands on one page share one chord registry and the later one to arm
wins. Nothing else ever took a build out — only a *replacement* did — which is
fine for a desktop bar with the app's lifetime and was a page-global leak for an
island: measured in Chrome 153, an unmounted root's `Ctrl+K` still ran its item
and still came back `defaultPrevented`.

**The DOM bar's dismiss overlay is `position: fixed`**, a full-window
box at `z-index: 199` under the bar's `201`. Fixed is what makes it independent
of its parent: the three menu-bar layouts put it in three different containing
blocks, one of which (`render_menu_bar_standalone`'s container) starts
`top_offset` px down the window. **`build_overlay` therefore takes no offset,
and must not grow one back** — an offset on a viewport-anchored box lands it off
the top of the window, which
`the_below_titlebar_overlay_covers_the_whole_window` fails on. In a browser the
same `100vw`/`100vh` box covers the whole **page**, not an app window, which is
one more reason an island's bar is the page's business and not only its own.

That deviation is **gone** (#324 stage B): `overflow` creates no stacking
context. Two things had to be true for the `fixed` spelling to order correctly
again, and stage B was only the first. The second is #545: the overlay is a
child of `.rinch-app-menu-bar__inline-layer`, which is `position: absolute;
z-index: 200` and therefore a stacking context of its own — so a `fixed` overlay
hoisted all the way to the **body** would sit at 199 in the body's sequence while
the row sat at 201 inside the layer, and the two still would not meet. (What
would have ordered them is 199 against the *layer's* 200 — same outcome, so the
mistake was invisible.) With #545 the overlay is hoisted no further than the
layer, and 199 and 201 land in one sequence for real — measured, not reasoned.

It was `position: absolute` between #527 and #324's stage C, which is worth
knowing because the reason was not geometry: `z-index` orders boxes only
*within* one stacking context, `BorderlessWindow`'s container carries
`overflow: hidden`, and rinch used to make a stacking context of every clipping
box. The menus stayed trapped inside it at that container's own `z == 0` while a
fixed overlay escaped to `199` above them, so it covered its own menus and every
item click merely dismissed the menu — and every *hover* with it, so
hover-to-switch and submenu flyouts were dead too. Stage B removed the cause,
#545 made the ordering mean what the comments say, and stage C took the
workaround out; `menu::app_menu_bar`'s tests cover all three layouts and were
green at every step — which is why the mistake #545 corrected could hide in them.

The non-borderless layout (`render_with_menu_bar`) was never broken — its
wrapper carries no `overflow`, so nothing between the bar and the body formed a
context.

The registry also shrinks now. It used to only ever grow: building a new native
menu bar releases the previous bar's ids, and dropping a `TrayIcon` releases that
tray's (the ksni path mints a fresh `ksni-{N}` id per item per build, so it could
never even overwrite). Keep the `TrayIcon` alive for as long as you want its menu
to work. A callback may also rebuild the menu it was dispatched from — that used
to be a `BorrowMutError`.

## Rich-Text Editor

Rinch's rich-text editor is a ProseMirror-style, **model-first** editor. The document lives in `rinch-editor-core` (a pure, wasm-clean crate: `Node`/`Mark`/`Fragment`/`Slice`, one char-based `Pos` space, a real `ContentMatch` schema, invertible `Step`s, `Transaction`/`EditorState`, plugins/commands/keymap/input-rules, a single Step-based history). The view lives in `rinch-editor-view` — **renderer-agnostic**: it projects that model onto any `rinch-core` `DomDocument` host and renders the caret/selection from `Selection` after layout, so desktop (rinch-dom) and web (`web_sys`) share one view. On desktop it arrives with the `desktop` feature and `crates/rinch/src/editor/` re-exports it; on web, `rinch-web` re-exports it. There is no `contenteditable` attribute engine anymore — mount the `Editor {}` component instead.

**Mutation flows one way:** every edit is a `Transaction` applied by `EditorState::apply` → the view diffs old/new doc + decorations and patches the DOM. Commands read **state**, never the DOM.

**Persisting content:** `DocNode` (serde) is the durable wire shape — `Node::to_doc()` / `Schema::node_from_doc()`, plus total HTML/markdown serializers in `rinch-editor-core::serialize`. Enable `serde` on the `rinch` facade (→ `rinch-editor-core/serde`).

**Full guides:** `docs/src/guide/contenteditable.md` (using the editor) and `docs/src/guide/editor.md` (model/schema/steps/plugins/view). Design: `docs/design/editor-rearchitecture.md`.

### Mounting an editor (the public API)

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let editor = create_editor();           // -> EditorHandle (cheap to clone)
    let ed_bold = editor.clone();           // one clone per closure
    rsx! {
        div {
            button { onclick: move || { ed_bold.command("toggleBold"); }, "Bold" }
            Editor {
                editor: editor.clone(),     // optional; omit to self-create a handle
                content: "<h1>Title</h1><p>Hello <strong>world</strong></p>",
            }
        }
    }
}
```

### `EditorHandle` (the app/component API)

| Category | Methods |
|----------|---------|
| **Dispatch** | `command(name) -> bool`, `update(\|state\| -> Option<Transaction>)`, `insert_text(&str)`, `replace_selection_with_html/text(&str)`, `insert_image(src, alt)`, `toggle_link(href)` |
| **Query (read state)** | `can_run(name)`, `is_mark_active(mark)`, `active_link_href() -> Option<String>`, `current_block_type()`, `in_node_type(type)`, `doc() -> Node`, `state()`, `selection()` |
| **Content / selection** | `load_html(&str)`, `load_doc(Node)`, `set_selection(Selection)`, `selection_clipboard()`, `anchor_selection()`, `set_dark_mode(bool)` |
| **Notification** | `on_change(impl Fn() + 'static)` — the autosave / dirty-marking hook |
| **Access** | `set_read_only(bool)`, `is_read_only() -> bool` — a runtime switch; `Editor { read_only: true }` sets it at mount |

`anchor_selection() -> SelectionAnchor` captures the selection for an insertion that
completes **later** (an async paste, an image upload, a model completion). Every
document-changing transaction maps the anchor forward through its steps, so
`anchor.selection()` still points at the content the user aimed at however much they
typed meanwhile; it answers `None` once the document is *replaced* (`load_doc`/
`load_html`, a collab re-projection), and the anchor releases itself on drop. This is
what makes the asynchronous Ctrl+V (#149) land in the right place — desktop's
`dispatch_editor_paste` anchors, reads the clipboard off-thread, then inserts at the
anchor.

`on_change` fires only for **local, document-changing** edits: `update` (which typing, paste and IME commit all funnel through), `command`, `insert_image`, and `toggle_link`. Those are the four `notify_change()` call sites in `rinch-editor-view/src/handle.rs`; if you add a fifth mutation path, it needs one too. It deliberately does **not** fire for selection-only changes, for `load_doc`/`load_html` (a programmatic load isn't a user edit — firing would make an autosave consumer immediately re-save what it just loaded), or for `collab_receive` (already in the shared CRDT). The callback runs with no internal borrow held, so it may re-enter the handle freely — e.g. call `doc()` to serialize for the save.

**Read-only refuses edits in `EditorCore::commit`, and nowhere else.** `set_read_only(true)` makes the editor what `readonly` makes an `<input>`: caret, selection, `selectAll`, copy and every query work; every local change to the document does not — typing, IME commit, paste, cut, every document-changing `command` (so every key bound to one, and `undo`/`redo`), `toggle_link`, `insert_image`, a task-checkbox click, any `update` transaction that changes the document or sets stored marks. Each answers `false`; `can_run` answers `false` for a refused command. The rule (`EditorCore::refuses`) reads the `prev`/`next` states rather than the caller, and `commit` is the single landing spot for a local change **that arrives as a transaction**, so **a new input path is read-only by default — do not add an `is_read_only()` check to an input handler to refuse an edit it already refuses.** (The one thing `commit` cannot judge from the states is a whole-document `load_doc`, which it is told about by `mapping: None`; a future mapping-less local change that is *not* a load would need an explicit origin instead.)

The flag is read in seven other places, and every one is there for something a refusal **cannot express** — none of them refuses an edit:
`ime_set_preedit` (handle.rs) shows no composition, since its commit is refused; `attach` re-stamps the container on a re-mount. Desktop turns the **OS input method off** (`focus.rs`), greys **Cut and Paste** in the built-in context menu (`text_context_menu.rs`), keeps **Cut from becoming a quiet Copy** (`editor_cut`) and starts **no clipboard read** for a paste that cannot land (`dispatch_editor_paste` — a read is a 4s worst case and a permission prompt on some platforms). The web makes the capture `<textarea>` **`readonly`** so no soft keyboard rises (`sync_capture_read_only`), makes `cut` a no-op, and **keeps ownership of a printable key** (`editor_input.rs`, `handle.insert_text(&key) || handle.is_read_only()`) — that last one looks exactly like the check the rule forbids and is not: without it Space scrolls the page and a letter reaches the page's own shortcuts. `collab_receive` never goes through `commit` — a remote change must not be recorded back onto the CRDT — so a read-only editor keeps integrating peers' deltas **by construction**; never gate `integrate_remote`, and never toggle the flag around it. While read-only nothing is recorded or broadcast, which is why `load_doc`/`load_html` are refused on a **collaborating** read-only editor (a load with a session attached is a write to the shared document) and allowed otherwise (the app showing a document is not the user editing one). The flag lives on the handle, not the view: it works before mount and survives a re-mount; the container carries `data-pm-readonly="true"` while it is on.

Command names (dispatch by string): `toggleBold/Italic/Underline/Strike/Code/Highlight/Subscript/Superscript`, `setParagraph`, `setHeading1..6`, `setCodeBlock`, `setTextAlign{Left,Center,Right,Justify}`, `toggleBulletList`, `toggleOrderedList`, `toggleTaskList`, `wrapInBlockquote`, `sinkListItem`/`liftListItem` (indent/outdent), `insertHorizontalRule`, `insertHardBreak`, `insertTable`, `addRow{After,Before}`, `addColumn{After,Before}`, `deleteRow`/`deleteColumn`/`deleteTable`, `mergeCells`/`splitCell`, `removeLink`, `undo`/`redo`. (Adding a link needs an `href` arg, so it is **not** a string command — use `handle.toggle_link(href)`.)

**Markdown shortcuts (input rules).** As you type, the default `MarkdownInputRulesPlugin` rewrites markdown shortcuts (these run inside `EditorHandle::insert_text`, so every text-entry path gets them). Block shortcuts at the line start: `# `…`###### ` → headings, ` ``` ` → code block, `> ` → blockquote, `- `/`* `/`+ ` → bullet list, `1. ` → ordered list, `[ ] `/`[x] ` → task list. Inline mark shortcuts (fire on the closing delimiter): `**bold**`/`__bold__`, `*italic*`/`_italic_`, `~~strike~~`, `==highlight==`, `` `code` ``. The rule set lives in `rinch-editor-core/src/input_rules.rs` (`markdown_input_rules()`); add a `mark_input_rule`/`wrapping_input_rule` there to extend it. Task items render a checkbox from their `checked` attr via the default stylesheet (`[data-pm-type="task_item"]`); `Enter` makes a fresh unchecked item, `Enter` on an empty item exits the list.

The editor ships its own default light/dark stylesheet (`rinch-editor-view/src/styles.rs`, injected once by the view); toggle dark mode with `handle.set_dark_mode(true)` (sets `data-pm-theme="dark"` on the container). Don't hand-roll editor CSS.

### Key Source Files

| File | Purpose |
|------|---------|
| `crates/rinch-editor-core/src/` | Pure model: `model/*`, `pos/*`, `schema/*`, `transform/*` (Steps), `state/*`, `commands/*`, `plugins/*`, `serialize/*`, `tables.rs`, `a11y.rs`, and `view.rs` (the `EditorView` seam) |
| `crates/rinch-editor-view/src/lib.rs` | `create_editor`, `mount_editor`, `caret_blink_tick` |
| `crates/rinch-editor-view/src/handle.rs` | `EditorHandle` — the imperative app/component API |
| `crates/rinch-editor-view/src/view.rs` | `RinchDomEditorView` (the `EditorView` impl: `ViewDesc` diff, caret/selection/decoration overlays) |
| `crates/rinch-editor-view/src/component.rs` | The `Editor {}` rsx component |
| `crates/rinch-editor-view/src/registry.rs` | The mounted-editor registry (keyed by `doc_key` + container id) and `update_all_carets`, the post-layout overlay pass |
| `crates/rinch-editor-view/src/styles.rs` | Default light/dark stylesheet |
| `crates/rinch/src/editor/` | **Desktop-only** wiring; re-exports all of `rinch-editor-view`. Block virtualization (`virtualization.rs` + `virtual_window.rs`), AccessKit (`a11y.rs`), and the `Send`-safe `post_remote_delta` |
| `crates/rinch/src/app/event_dispatch.rs`, `app/focus.rs` | Desktop input glue: key/pointer/IME → `EditorHandle` calls, and the focus arbiter |
| `crates/rinch-web/src/editor_input.rs` | The same glue for the browser (the web editor shares the view crate) |
| `crates/rinch-editable/src/` | The separate single-line `<input>`/`<textarea>` engine (`EditCommand`, `InputHandler`) — unrelated to the rich editor |

### Collaboration (optional, opt-in — M9)

Real-time collaborative editing is a feature-gated adapter, **not** part of the model. The pure `rinch-editor-core` model stays renderer- and CRDT-agnostic; `rinch-editor-collab` projects it onto a **yrs 0.27** (Yjs) CRDT so concurrent edits converge, then translates remote CRDT changes back into editor `Step`s. This crate is the **only** thing in the workspace that links a CRDT engine (yrs replaced Automerge in issue #190).

It is gated behind the optional `collaboration` feature (which implies `desktop`, since the editor wiring lives there), so **default builds — desktop AND web — link zero CRDT code**. The adapter itself is pure model↔CRDT logic with no platform deps and is **wasm-compatible with no shims** — yrs carries its own `fastrand/js` randomness source and builds for `wasm32-unknown-unknown` with no extra features — so a future Rust web editor view can reuse this *same* adapter rather than bridging to a separate JS CRDT.

Enable with `features = ["collaboration"]` on the `rinch` facade (or depend on `rinch-editor-collab` directly).

The design rests on one invariant — **`model ≡ project(model)`**: every local step is projected onto the CRDT, every remote CRDT change is rebuilt into the model. Convergence then follows from yrs's own convergence.

**Staged scope (design A22):** the first milestone covers **flat text-blocks + marks** (`paragraph`/`heading`/`code_block` with text + bold/italic/link/… marks), the **list containers** `bullet_list`/`ordered_list`/`list_item` (nested into each other and around text-blocks to any depth), the **leaf block atoms** (`horizontal_rule` — a block whose projected text is simply empty) and the **inline atoms** (`image`/`hard_break` — one U+FFFC char of the block's text carrying a reserved `@atom` formatting attribute that names the node type and its attrs). Anything else — `blockquote`, tables, `task_list`/`task_item` — is `CollabError::Unsupported`: the adapter **fails loud** rather than silently dropping a change (a silent drop would reintroduce the exact divergence class the editor rewrite killed).

**An A22-refused local edit stalls outbound, it does not wedge it (#220).** The
model applies the edit even though the CRDT refuses it, so from that moment the
caller's `before` no longer describes the CRDT. `record_local` therefore treats
`before` as a *hint*: if diffing against it fails, the change is re-projected
against the CRDT's own read-back, which is authoritative. That is what makes the
recovery real — before, the block-count gate refused **the very deletion that was
the cure**, and once the counts realigned by coincidence the diff skipped the
blocks it believed unchanged, so an edit made during the stall stayed local
forever while `record_local` answered `Ok`. Silent divergence. Now removing the
offending content resumes outbound on the spot and ships the whole backlog in one
delta. `record_local` takes the `&Schema` for the read-back (a session cannot hold
an `Rc<Schema>` — it must stay `Send`); `collab_outbound_stall()` reports the
state while it holds.

**Collab bytes are opaque.** A snapshot, a broadcast delta, a state vector, and a sync diff are all just `Vec<u8>` in yrs's lib0 v1 encoding — callers move them between calls without decoding them.

```rust
use rinch_editor_collab::CollabSession;

// One session per editor. Peer B joins from peer A's snapshot.
let mut a = CollabSession::new(&state)?;            // project state.doc onto a fresh CRDT
let mut b = CollabSession::from_bytes(&a.snapshot())?;

// After the editor applies a local transaction, project before→after onto the CRDT:
a.record_local(new_state.schema(), &old_state.doc, &new_state.doc)?;

// Broadcast a delta:
let delta = a.save_incremental()?;
if let Some(next) = b.integrate_incremental(&b_state, &delta)? { b_state = next; }
// `next` applies the remote change as a non-undoable `origin=remote` transaction.

// Reconciliation for a peer that fell behind (offline, a dropped delta):
let to_b = a.sync_diff(&b.state_vector())?;   // "what does b not have yet"
if let Some(next) = b.integrate_incremental(&b_state, &to_b)? { b_state = next; }
```

**The desktop editor wires this in for you** (M9) — you do not drive `CollabSession` directly. Every local edit through an `EditorHandle` projects + broadcasts automatically; a peer's delta integrates + re-projects through `collab_receive`. One peer **hosts** (owns the starting document), the others **join** from its snapshot. The transport is the app's concern — `outbound` carries bytes out, `post_remote_delta` carries them back in from any thread:

```rust
// Host: project the current doc onto a fresh CRDT, hand peers a join snapshot.
let snapshot = host.start_collaboration_host(move |delta| transport.send(delta))?;
// Guest: adopt the host's document and collaborate.
guest.start_collaboration_guest(&snapshot, move |delta| transport.send(delta))?;
// Inbound from a network thread (marshals onto the main thread); from the prelude:
post_remote_delta(container_id, delta_bytes);
```

| `EditorHandle` collab method | Purpose |
|---|---|
| `start_collaboration_host(outbound) -> Result<Vec<u8>, CollabError>` | Host a fresh session; returns the join snapshot |
| `start_collaboration_guest(&snapshot, outbound) -> Result<(), CollabError>` | Join from a host snapshot (adopts its document) |
| `collab_receive(&delta) -> bool` | Integrate a peer's delta or reconciliation diff (main thread, `try_borrow_mut`-soft); re-projects, does **not** re-broadcast |
| `collab_state_vector() -> Option<Vec<u8>>` | This editor's state vector, to hand a peer for `collab_sync_diff` |
| `collab_sync_diff(remote_state_vector) -> Option<Vec<u8>>` | The update a peer at `remote_state_vector` is missing; feed the result to that peer's `collab_receive` |
| `is_collaborating()` / `stop_collaboration()` | Query / detach the session |
| `is_collaboration_poisoned()` | Whether the session is **poisoned** (#196): an integrate left the shared CRDT unprojectable with nothing pending that could cure it (yrs has no rollback; a rebuild failure with updates parked on missing dependencies stays transient), so every convergence call — inbound AND outbound — fails sticky with `CollabError::SessionPoisoned` instead of one-way partitioning. Inbound is still attempted, and an update that makes the doc rebuildable again clears the poison; recovery in practice is `stop_collaboration()` + rejoin from a healthy peer's snapshot |
| `collab_snapshot() -> Option<Vec<u8>>` | Current shared-doc snapshot for a *late*-joining guest |
| `collab_outbound_stall() -> Option<CollabError>` | Why **outbound** is currently refusing (#220): a local edit outside A22 scope (a pasted table, a blockquote) that the model applied but the CRDT would not. That edit and every one after it stays local; inbound keeps working and the shared doc is healthy. Unlike `collab_take_error` this does **not** clear — it is the *state*, so drive a persistent "not syncing — remove the table" banner from it. It clears itself on the next projectable edit, which broadcasts everything that accumulated. Not poison, no rejoin |
| `collab_take_error() -> Option<CollabError>` | Take a fail-loud collab error. A22 projection errors are transient (the CRDT is left untouched — projection is all-or-nothing); a `SessionPoisoned` is not a one-off — taking it does not un-poison, every affected call re-fails with it |

Free functions `collab_receive_for(container_id, &delta)` (main thread) and `post_remote_delta(container_id, delta)` (any thread) route an inbound delta to a registered editor. Runnable two-pane in-process loopback: `examples/collab-editor-demo`.

**The transport owns relaying.** `outbound` fires only for an editor's own local edits — a delta integrated via `collab_receive` is never re-broadcast (it's already in the shared CRDT; echoing it back would loop). So the transport must be a **full mesh** (every peer's `outbound` reaches every other peer) or a **hub** that fans each delta it receives out to the others, forwarding the raw bytes unchanged. A chain — A wired to B, B wired to C, nothing joining A and C — silently partitions: C never sees A's edits, and nothing errors. The repair for a peer that fell behind is the reconciliation pair above (`collab_state_vector` → `collab_sync_diff` → `collab_receive`).

**Never use state-vector equality as a convergence test.** A yrs state vector counts *insertions* only — a deletion, or a mark *removal* (yrs un-formats a mark by deleting its format marker), leaves it unchanged, so two replicas can hold different documents behind equal state vectors. That's why `integrate_incremental`/`collab_receive` decide "did anything change" by rebuilding and comparing documents, never by comparing state vectors before/after — and why reconciliation always requests-and-applies a diff (whose reply carries the full delete set) rather than short-circuiting when two state vectors look equal.

`CollabPlugin` (key `"collab"`) folds collab bookkeeping (version + unconfirmed local steps) into `EditorState`; `rebase_steps(steps, &mapping)` is the ProseMirror rebase primitive (`Step::map`). The session integrates by converged rebuild (`CollabDoc::to_doc` → `build_remote_transaction`), which is provably convergent — there's no separate patch→op translation layer. One used to exist (`CollabDoc::patches_to_remote_ops`/`remote_ops_since`, consuming `automerge::Patch`) but it was **deleted**, not ported, with the yrs migration: it was documented as non-convergence-critical and had no session consumer. `remote.rs`'s module doc notes it could be rebuilt on yrs observer deltas (`TextRef::observe` → `TextEvent`) if a future cursor-preserving refinement ever wants it.

| File | Purpose |
|------|---------|
| `crates/rinch-editor-collab/src/projection.rs` | `CollabDoc` — the yrs wire shape (`content: Array<Map{type,attrs,text:Text}>`, marks as native yrs `Text` formatting attributes, plus a `meta` root map carrying `format = "rinch-editor-collab/yrs-1"` — the marker `load` requires to tell our bytes from a foreign CRDT), `from_doc`/`to_doc`/`load`, the `observe_update_v1` broadcast outbox, fail-loud validation |
| `crates/rinch-editor-collab/src/project.rs` | Local: `project_change` — block-list diff (Rc-identity prefix/suffix, minimal text splice) |
| `crates/rinch-editor-collab/src/remote.rs` | Remote: `build_remote_transaction` — converged rebuild into a minimal block-level `ReplaceStep`; no engine type appears in this file |
| `crates/rinch-editor-collab/src/session.rs` | `CollabSession` — the imperative model↔CRDT lifecycle (`new`/`from_bytes`, `record_local`, `save_incremental`/`state_vector`/`sync_diff`, `integrate_incremental`) |
| `crates/rinch-editor-collab/src/sync.rs` | The yrs bytes transport on `CollabDoc` (`state_vector`, `diff_since`, `apply_update`, the outbox drain) |
| `crates/rinch-editor-collab/src/plugin.rs` | `CollabPlugin` + `CollabState` |
| `crates/rinch-editor-collab/src/rebase.rs` | `rebase_steps` — local steps rebased over a remote mapping |
| `crates/rinch-editor-collab/src/error.rs` | `CollabError` — the crate's one error type (`Engine`/`Unsupported`/`Schema`, plus the sticky `SessionPoisoned` a session fails with in both directions once an integrate has left the CRDT unprojectable with nothing pending to cure it — #196; cleared by an inbound update that makes the doc rebuildable again) |
| `crates/rinch-editor-view/src/handle.rs` | `EditorHandle`'s collab methods (`start_collaboration_host/guest`, `collab_receive`, `collab_state_vector`, `collab_sync_diff`, `collab_snapshot`, `collab_take_error`, `collab_outbound_stall`, `stop_collaboration`, `is_collaborating`, `is_collaboration_poisoned`) — **not** in `rinch-editor-collab` |
| `crates/rinch-editor-view/src/collab.rs` | `CollabBridge` — the seam driving `CollabSession` from an `EditorHandle` (outbound sink + last error) |
| `crates/rinch-editor-view/src/registry.rs` | `collab_receive_for(container_id, &delta)` — routes an inbound delta to a registered editor |

## Drag and Drop

Rinch has two drag systems: **DOM drag attributes** for element-to-element DnD, and the **`Drag` builder** for pointer capture (sliders, panel dragging, resize handles).

> **Picking the right one:** if you want **continuous per-frame tracking** (slider value, panel position, timeline scrub), use **`Drag::absolute()` / `Drag::percent()`** from inside an `onclick` handler — that's the pointer-capture system. The HTML5-style `draggable: true` + `ondragstart` + `ondragend` attributes only fire at the **endpoints** of the drag; for per-frame events on that path use `data-ondragmove` (source) and `data-ondragover` (target).

### DOM Drag Attributes

Set these attributes on elements to participate in element-to-element drag-and-drop:

| Attribute | Fires on | When |
|-----------|----------|------|
| `data-ondragstart` | Source | Drag begins |
| `data-ondragmove` | Source | Pointer moves during drag |
| `data-ondragenter` | Target | Drag enters a drop target |
| `data-ondragover` | Target | Pointer moves over drop target (every motion event) |
| `data-ondragleave` | Target | Drag leaves a drop target |
| `data-ondrop` | Target | Drop on target |
| `data-ondragend` | Source | Drag finishes |

**Input & activation — and the two backends do NOT agree.**
- **rinch-web** drives this suite from Pointer Events and branches on the pointer kind, so activation differs by input and touch does not hijack scrolling. **Mouse:** past `WEB_DRAG_THRESHOLD`, 5 CSS px. **Touch / pen:** a short **long-press** hold, `TOUCH_LONG_PRESS_MS` = 350ms, while the contact stays within `TOUCH_MOVE_SLOP`; moving before the hold completes is a scroll/pan and the drag is abandoned. That is the standard mobile reorder gesture, and it lives in `rinch-web/src/event_delegation.rs`.
- **Desktop has no such branch.** `event_dispatch.rs` applies one flat `DRAG_THRESHOLD` (5px, `app/mod.rs`) to every pointer alike — the shell folds touch into a plain left-click and discards winit's device kind. Desktop's own touch translation is a *different* machine (`shell/touch_gesture.rs`): a finger that moves past `SCROLL_THRESHOLD` becomes a **scroll** and emits `PointerCancel` first, a finger that lifts while still is a **tap**, and one held past `LONG_PRESS_TIMEOUT` (**500ms**, not 350) becomes a **context menu** — a right-button press/release. So the mobile reorder gesture described above is a web behaviour; do not assume a touch drag of this suite behaves the same way on desktop, and treat its reachability there as unestablished rather than as working.

Because there's no built-in drag ghost (the app renders its own from `data-ondragmove`), **set `pointer-events: none` on your ghost element** — on touch the drop target is resolved via `elementFromPoint`, so a ghost under the finger would otherwise intercept the hit and drops would silently fail.

Handlers can read `get_click_context()` for cursor position and element bounds. Use `DragContext<T>` to pass typed data between source and target:

```rust
let drag = DragContext::<MyItem>::new();

// In source's ondragstart:
drag.set(item.clone());

// In target's ondrop:
if let Some(item) = drag.take() {
    target_list.update(|list| list.push(item));
}
```

### Pointer Capture Drag (Drag Builder)

For tracking mouse movement from a click handler until mouseup (sliders, panels, resize):

```rust
// Absolute pixel coordinates (panel dragging)
let ctx = get_click_context();
let offset_x = ctx.mouse_x - panel_x.get();
Drag::absolute()
    .on_move(move |x, y| panel_x.set(x - offset_x))
    .on_end(move |x, y| save_position(x, y))
    .on_cancel(move |_x, _y| restore_position())  // teardown; on_end does NOT fire
    .start();

// Percentage 0.0–1.0 (sliders) — reads element bounds from ClickContext automatically
Drag::percent()
    .on_move(move |px, _| slider_value.set(px * 100.0))
    .start();

// Cancel: fires on_cancel (with the last on_move coords) but NOT on_end —
// a cancelled drag must not commit. The web backend calls this on pointercancel
// (touch-scroll takeover, system gesture, pointer-capture loss).
Drag::cancel();

// Check if active
Drag::is_active();
```

**A drag belongs to the document that armed it (issue #139).** `Drag` state is a
single thread-local, but a thread can pump several documents' pointer streams
through it — a desktop window and its DevTools panel are two `RinchApp`s on one
thread, and so are two embedded `RinchContext`s. Only the document whose events
armed the drag drives it: another document's `MouseMove` does not reach
`on_move`, its `MouseUp` does not fire `on_end`, and `Drag::is_active()` answers
`false` there (so a drag in one window never freezes hover in the other). A drag
armed **outside** any event dispatch — from a timer, a menu callback, or on
rinch-web, which has one page-wide pointer stream — belongs to no document in
particular and stays drivable by anybody. Nothing changes for a single-window
app. (Two desktop *windows* do not cross-feed a plain mouse drag on their own —
the pointer is grabbed to the pressing window while a button is held — so this
matters for an embed host pumping several contexts from one event stream, and
for a drag left live past a missed `MouseUp`.)

**A drag whose release was swallowed heals itself (issue #189).** A native
context menu, a modal dialog, a window-manager grab, or the pointer leaving a
non-capturing surface can eat the `pointerup` that should have ended a drag,
leaving it armed for the rest of the session and following the cursor with no
button held. On a move that reports the primary button/contact released, the
drag ends through **`on_cancel`** — not `on_end`: a release nobody saw has no
trustworthy commit position, so the honest ending is the teardown one, with the
last coordinates actually delivered to `on_move`.

The backend says which by calling `update_drag_with_button(x, y,
PrimaryButton::{Down,Up,Unknown})`; `update_drag(x, y)` is exactly the `Unknown`
form. Three states rather than a bool because a backend that cannot see the
button state is a real case: **rinch-web reports `Down`/`Up` from `buttons & 1`
and heals; desktop reports `Unknown` and does not.** `PlatformEvent::MouseMove`
carries no button mask, and neither does winit's `PointerMoved` behind it, so
desktop has no independent source of truth — and a flag the runtime kept itself
would be no help, since the missed `MouseUp` that strands the drag is the same
event that would have cleared the flag. Tracked in **issue #294**. Nothing is
ever ended on a guess: `Unknown` behaves exactly like `Down`. The heal is
document-scoped like the rest of the drag — another document's idle pointer
cannot tear down this one's live drag.

### File Drop (OS → App)

File drops from the OS use `data-onfiledragenter`, `data-onfiledragleave` attributes, and `register_file_drop_handler` for the actual drop. See the File Drop section of UI Zoo for an example.

## Keyboard Shortcuts (built-in)

- `Alt + I` - Toggle inspect mode (hover highlight for element info)
- `F12` - Toggle DevTools window

## Keyboard Focus

Exactly one widget owns the keyboard at a time — the **focus arbiter**
(`FocusTarget` in `crates/rinch/src/app/mod.rs`, design A10): an `<input>`, an
open `<select>`, the rich-text editor, a render surface, or a generic focusable
DOM node (`FocusTarget::Node`). Every transition goes through
`RinchApp::set_focus_target`, which tears the previous owner down first.

Focusability on desktop comes from the **tag** or an explicit `tabindex` (or a
`data-oninput` on a custom control), and an explicit `tabindex` always wins —
the browser rule (issue #252). Focusable by tag: `<button>`, `<select>`,
`<textarea>`, `<input>`, and `<a>` with a non-empty `href`. **Not** `<summary>`
(rinch has no `<details>` behaviour) and **not** `data-rid` (the DropdownMenu
backdrop carries one). `tabindex="-1"` is focusable by click and
programmatically but not tabbable;
`disabled` and `data-disabled` are both honoured (issue #315) as **boolean
attributes**, take no focus by any route, and are re-checked at edit time: a
field that goes disabled *while focused* stops accepting keys **and releases the
keyboard** — with its `data-onchange` commit suppressed, since going disabled is
not the user committing an edit (`release_focus_for_disabled`, the only
transition that suppresses it). `readonly` focuses and selects but refuses every
text-changing command, on a `<textarea>` as well as an `<input>`.

**Presence is the whole value, and `"false"` is not an escape for the HTML
pair** (issue #612). `disabled` and `readonly` are read by presence alone, the
way a browser reads them — `<button disabled="false">` is disabled and
`<input readonly="false">` is read-only, measured in Chrome 150 as both the IDL
property and a `:disabled` / `:read-only` match — so **the value rule** is now
one rule across desktop and `rinch-web`. Only that clause: desktop is still
broader than a browser elsewhere in the family (`node_is_disabled` is
tag-agnostic where HTML ignores `disabled` on a `<div>`, and rinch has no
`:read-only` pseudo-class at all). Desktop used to honour a `"false"` escape the
browser has no notion of, which meant one markup and opposite behaviour. To say
*enabled*, **remove** the attribute, which is what a falsey reactive `bool` does
for you (`NodeHandle::write_attribute`, #551).
rinch's **own** `data-disabled`, `data-nofocus` and `data-trap-focus` keep the
escape, and are the only three that have it — a rinch convention rather than a
desktop quirk, which the latter two are what show: the web reads them the same
way, through `[data-nofocus]:not([data-nofocus="false" i])` and
`[data-trap-focus]:not([data-trap-focus="false" i])`. (`data-disabled` has no web
reader; the browser does not know the attribute.) The rules are one function
each — `rinch_core::dom::data_attr_is_on` for the `data-` family, a bare
`contains_key` for the HTML pair — and `"0"` is the only value that can tell
which one a reader uses, so all three readers pin it.

A disabled `<fieldset>` disables its
subtree (except its first `<legend>`); every other tag's `disabled` removes
only the node from the Tab order, not its subtree. A **mouse press claims
the nearest focusable ancestor** of the hit node, browser-style, so a clicked
`tabindex` div owns Enter/Space immediately.

A focused **`<select>` is closed**, like a browser's — Enter/Space/Alt+Down
opens its popup, which then owns the keyboard (issue #314). It is never handed
to the text engine, whatever handlers it carries: branching on `data-oninput`
without a tag guard used to install an `EditableState` over a select's `value`
and make it a typable text field (issue #424). `Select`'s trigger `<div>`
carries `tabindex="0"` + combobox ARIA (issue #251); arrow/Enter/Escape
navigation of its **open** option list is issue #434.

**Tab is contained by an open overlay** (`trap_focus`, #474). `Modal`, `Drawer`
and `Popover` stamp **`data-trap-focus`** on their root while open and **remove**
it when closed, and both backends read it: desktop's
`RinchApp::tab_trap_root` starts `collect_focusable_nodes_from` at the trap so
`handle_tab`'s existing wrap becomes a wrap *inside* it, and `rinch-web`'s
keydown listener collects the trap's focusable descendants and calls `focus()`
itself. Which trap: the nearest one the current claim sits inside, else the
**last** visible one in DOM pre-order — so nesting resolves inside-out and no
`z-index` is read, matching the dismiss stack's LIFO. Two guards, and a fixture
each, because a closed `Modal` satisfies both: the attribute is removed (hence
`is_boolean_attribute`, so `write_attribute` removes rather than writing
`"false"`), **and** a trap with no box is skipped. **Only Tab is contained** — a
click or a scripted `focus()` outside still moves focus out, which is a
*non-modal* `<dialog>`'s behaviour; `showModal()` inerts the page and refuses
both, and rinch models neither. On the web the **browser** is the
authority on focusability, not `trap_focusables`' selector: `handle_trapped_tab`
checks `activeElement` after each `focus()` and steps on when the browser
declines, because a filter cannot be closed over `<fieldset disabled>`, a
`tabindex` the browser parsed differently, or `inert`. A trap whose every
control the browser refuses therefore takes the **empty-trap** path — key
consumed, focus unmoved — not an endless retry. `register_focus_target`'s
`on_key` is the obvious-looking route and silently does nothing: the arbiter
offers a key to a registered target only while it holds `FocusTarget::Node`, and
the focused element inside a dialog is normally an `<input>`.
`crates/rinch/src/app/trap_focus_tests.rs` and
`crates/rinch-web/tests/trap_focus.rs` are the pins — twins, not shared code,
since the focusable set is a tree walk on one backend and a CSS selector on the
other.

**`trap_focus` also moves focus in and gives it back** (issue #695) — the half
#474 deferred, and the same prop, because `trap_focus` is rinch's spelling of
"this overlay is modal". Opening remembers whatever holds the keyboard and
focuses inside: an `autofocus` descendant wherever it sits, else the first
focusable (`Modal`/`Drawer`) or nothing at all (`Popover`, matching the HTML
popover API rather than the dialog). Closing — and unmounting while still open,
which `if show { Modal { … } }` does — hands it back if that element is **still
connected**, and otherwise releases the claim rather than focusing into a
detached subtree; focus the user moved outside the overlay is left alone. The
memory is per-overlay, which is the whole of nesting: the inner remembers what
the outer focused. `rinch-components`' `overlay_focus::arm_overlay_focus` is the
policy and it is the *only* copy; what it rests on is three `DomDocument`
methods — `active_element`, `focus_into`, `restore_focus` — reachable on any
`NodeHandle` and defaulted so `MockDomDocument` keeps compiling. There is no
bare `blur()`: releasing the keyboard is only ever the fallback half of a
restore, and the decision belongs where the facts are. **Desktop resolves both
overlay calls a layout later**, parked as a `rinch_core::FocusRequest` in the
same single slot `request_focus` has always used, because an overlay opening or
closing is a class change in the same effect flush and every node inside it
still carries a zero-size box. `FocusRequest::needs_layout` says which kinds
that applies to, and **a consumer that has not run a layout re-parks them** —
the two synchronous drains after `dispatch_event` (`click_handling` and
`activate_focused_node`) otherwise consumed the slot and lost the move for good,
which is every overlay opened by a click or by Enter. `rinch-web` answers on the
spot and lets the browser refuse what it should refuse. "Still there" means
**can still take focus**, not merely attached: a `disabled` opener, or one
inside an outer overlay closed first, releases the keyboard instead. Identity is
not checked, so a recycled node id (issue #304, live on desktop through
`NodeTree::remove_subtree`) would still be focused.
`crates/rinch/src/app/overlay_focus_tests.rs` and
`crates/rinch-web/tests/overlay_focus.rs` are the pins, twins like the
containment pair.

Still unmatched to the web: a positive `tabindex` does not order ahead of DOM
order (issue #435).

**`data-nofocus` takes the click without the keyboard** (issue #312) — the
`preventDefault()`-on-mousedown mechanism browsers converged on, which an editor
toolbar needs so Bold does not blur the editor it acts on. Same boolean rule as
`data-disabled` — including the `"false"` escape, which the three rinch-owned
attributes keep and the HTML pair does not — read **anywhere on the pressed
node's ancestor chain** so a toolbar carries it once; it protects whatever holds
the keyboard (editor, input, surface, node), the `data-rid` click still fires,
and a text field *inside* the
region still focuses normally. Both backends — on web it becomes
`preventDefault()` on the `pointerdown`.

A custom component that takes keyboard input registers for the lifecycle
(issue #147, `crates/rinch/src/focus_registry.rs`, in the prelude):

```rust
register_focus_target(
    &node,
    FocusEntry::new()
        .on_focus_gained(move || focused.set(true))
        .on_focus_lost(move || focused.set(false))
        .on_key(move |k| k.key == "ArrowDown"), // true = consumed
);
```

- Keyed `(doc_key, node_id)` like the editor registry (#134); **deregistered by
  the ambient scope's `on_cleanup`**, so unmounting is **silent** —
  `on_focus_lost` never fires after disposal (that would read freed signals and
  panic, #141 PR4). The registry is the arbiter's liveness authority for
  registered nodes, which closes the recycled-slot window (#304) for them.
- Both focus callbacks run **after** the transition completes (deferred through
  the same `PendingFocusWork` mechanism as a blurred input's `data-onchange`),
  so they may re-enter the runtime freely.
- `on_key` is offered before the runtime's own handling; `true` consumes. It
  sees **releases too** (#337): `k.kind`/`k.is_up()` tell the phases apart
  (auto-repeat is a `Down`, and `KeyEventData` still does not tell it from a
  fresh press — the *platform* event does, see **Key auto-repeat** below).
  A press and its release are spelled
  by the same rule from the same fields, so pairing them by `k.key` works by
  construction — a release carries no text and resolves through `logical_key`,
  which `PlatformEvent::KeyUp` now carries for exactly that reason.
  `logical_key` is `Option<String>` holding the full **case-accurate**
  `KeyboardEvent.key` value (`"A"` under Shift, `"!"`, `"Enter"`, `"Dead"`;
  it was a lowercased `Option<char>` letter, which made a shifted release
  disagree with its own press) — lowercase at the comparison site, as
  `editor_key_binding` does, never at the source. A release's
  return value is ignored (nothing downstream to suppress, and the activation
  latch must clear regardless). `KeyEventData` is `#[non_exhaustive]` — build
  one with `KeyEventData::new(key, code)` plus `with_modifiers`/`with_kind`.
  `set_keyboard_interceptor` is unrelated — a document-level capture-phase hook
  dispatched *before* the arbiter. **It is the wrong registry for Escape**
  (#474): it is one slot per document, so a second overlay registering there
  disables the first and its unmount clears the slot rather than restoring what
  it displaced. Escape goes through the **dismiss stack** instead —
  `rinch_core::push_dismiss_handler(doc_key, || bool) -> DismissHandle`,
  LIFO, per document (`doc_matches`: only two differing `Some` keys are
  refused, so a backend that marks none — rinch-web — reaches every handler),
  owner-checked at dispatch, dispatched from inside
  `dispatch_keyboard_event` for an Escape *press* after the interceptor (so
  both backends get it with no edit). `Modal`/`Drawer`/`Popover`'s
  `close_on_escape` rides it, and so does the **DOM menu bar**, which is the
  stack's first non-component member and registers on both backends from one
  `build_overlay`; a custom overlay should too — **and there are two
  registration policies, not one** (#465). Those three register at *mount* and
  answer `opened_fn` at dispatch, because `render` runs once and a closed
  overlay stays mounted. That is wrong for an overlay **statically nested inside
  another one**: a component renders *after* its children, so
  `Modal { ColorInput { … } }` pushes the input's entry first and the modal's on
  top, and the modal answers Escape while the picker is what is on screen. An
  overlay that owns its open state pushes at *open* time instead and releases on
  close — the `<select>` popup's shape (#671), spelled for components as
  `rinch_components::overlay_dismiss::arm_close_on_escape_while_open` and used by
  `ColorInput`'s dropdown (whose outside-click backdrop is `DropdownMenu`'s,
  `position: fixed` and all). Such a handler consumes unconditionally, since it
  exists only while the overlay is open, and its release has to cover
  unmount-while-open as well as close. It shares the
  *lifetime* rule though (#183):
  registering it during a render releases it on unmount, ownerless registration
  keeps app lifetime, and an earlier unmount never clobbers a later
  registration. Same for `set_paste_interceptor`, `set_selection_callback` and
  `set_selection_sync_callback`; the discipline is
  `rinch_core::reactive::install_doc_scoped_slot` / `clear_doc_scoped_slot`, and
  any new **document-level** callback registry should go through those rather
  than paraphrase them — all four of the callbacks named above do. The
  thread-scoped pair `install_scoped_slot` / `clear_scoped_slot` is the same
  discipline over a plain `RefCell<Option<Rc<T>>>`, for a registry that is
  genuinely per-thread rather than per-document; reaching for it when you wanted
  the doc-scoped one gives two `RinchContext`s on one thread a single shared
  slot, last-registration-wins for both, which is the #134 class of bug.
  A registry written *repeatedly* from a live component (or one that wants the
  callback attributed to its component when it runs) takes the other template
  instead — owner beside the callback, `is_alive()` at dispatch, invoked inside
  `owner.run(...)` — as `main_thread::park_main_callback`, `rinch-ws`'s
  `HANDLERS`, the menu registry and `rinch-android`'s sensor / location /
  lifecycle / activity-result registries all do.
- **Window blur notifies but retains**: `PlatformEvent::WindowFocus(false)`
  fires `on_focus_lost` and keeps the claim (releasing would fire
  `data-onchange` on every alt-tab, #226); refocus re-fires `on_focus_gained`.
  `ime_state()` reports disabled while blurred.
- **IME rides the same claim** (#176): adding `.on_ime(|e: &ImeEvent| …)`
  declares the target a *text* target, so `ime_state()` switches the platform
  input method on for it and every `ImeEvent` is routed to it, exactly like the
  editor and `<input>`. Without `on_ime` a focusable node drives no IME (a card
  must not pop a candidate window). `.caret_rect(|| Some((x, y, w, h)))` places
  the OS candidate box in **logical window pixels** (not physical — the shell
  passes it to winit as a `LogicalPosition`), and is re-polled every event-loop
  iteration so it follows the caret. The runtime fabricates no events: a focus
  change is *not* an `ImeEvent::Disabled`, so clear your preedit in
  `on_focus_lost`. `ImeEvent::DeleteSurrounding` stays inert on desktop
  (`sync_ime` requests only `with_cursor_area()`).
- Not yet: the Android soft keyboard for a registered target (the shell still
  watches for a focused `<input>`/editor). **Tab containment does work** — see
  `data-trap-focus` above — as do *dismissal* (the dismiss stack) and the
  focus move/restore (#695). What remains unmatched is **modality**: a click
  still reaches a control the backdrop does not cover, and a click — or a
  scripted `focus()` — outside an overlay moves focus out of it. That matches a *non-modal* `<dialog>`; a browser's
  `showModal()` inerts the rest of the page and refuses even a scripted focus
  behind it (measured, Chrome 150), which rinch does not model.
- **`lock_scroll` gates the gesture on desktop and the style on web** (#474).
  `Modal`/`Drawer`'s prop reaches
  `DomDocument::set_scroll_locked(locked, root)` through
  `NodeHandle::set_scroll_locked`; it takes the overlay's **root node**, not
  just a bool, because desktop has to know which subtree may still scroll.
  Desktop records the locking roots on `NodeTree` and refuses a container
  outside every one of them (`NodeTree::scroll_locked_out`) at two input sites:
  the **wheel arm** in `event_dispatch.rs` (both axes, both the ancestor walk
  and the geometric fallback — an open overlay is exactly the shape that
  fallback exists for) and **`find_scrollbar_hit`**, so the page's #178 thumb
  cannot be dragged either. Nothing is restyled: the page does not reflow and
  keeps its `scroll_offset`. Web sets `overflow: hidden` on the real `<html>`
  (page-global and counted in `web_document.rs`, previous inline value saved and
  restored), because rinch cannot gate the browser's own wheel. **The
  divergence to know:** web removes the page's *scrollbar* too, so a classic
  (non-overlay) scrollbar there shifts the layout on open; on desktop a locked
  page's bar stays painted and is simply inert. Locks are **counted** in both
  backends, so an inner modal closing over an outer one leaves the page locked,
  and `overlay_scroll_lock::arm_lock_scroll` releases on close *and* in
  `on_cleanup` — an unmount while open would otherwise wedge the page for the
  session. A scrollbar drag already **in flight** when the lock arrives is
  ended rather than left scrolling (the #189 shape, re-checked in `MouseMove`),
  and the native `<select>` popup is **exempt** through a second list,
  `NodeTree::scroll_lock_exempt` — its option panel is a `<body>` portal, so it
  is inside no overlay's root and a long list in a dialog was unscrollable.
  Exempt is a separate list because `scroll_lock_roots` is also the count:
  putting the popup there would freeze the page whenever a `<select>` was open.
  `ContextMenu` is the other body portal and needs none *today* — its dropdown
  declares no `overflow`/`max-height`, so it is not a scroll container; give it
  either and it inherits the trap silently. The exemption has **no portable
  spelling**: `push_scroll_lock_exempt` is a `NodeTree` method, reachable from
  the runtime and not through `NodeHandle`, so a scroll container built by a
  *component* and sitting outside the locking overlay stays refused — the Linux
  in-app menu bar's own dropdown is the known instance (#701). What the lock does **not** gate:
  programmatic scrolling (`set_scroll_top`), and keyboard page-scrolling, which
  desktop does not have at all. A touch scroll and the MCP `scroll` tool both arrive as
  `PlatformEvent::MouseWheel`, so they are gated. `RenderScope::body_handle()`
  is the trap the design avoids — on web it is `<div id="rinch-body">`, not the
  page.
- **An open `<select>` popup joins the dismiss stack** (#671). It is handled by
  the arbiter, which is step 2, while the stack is inside step 1 — so once
  `close_on_escape` started working, a `<select>` inside a `Modal` lost Escape
  to the modal. It now pushes its own entry when it opens (`open_select_popup`,
  released at `remove_select_popup_nodes`, the one place `open_select` becomes
  `None`), and since the popup opens *after* the modal mounted, LIFO puts it on
  top with no precedence special case anywhere. A dismiss handler is an
  `Fn() -> bool` and closing the popup needs `&mut RinchApp`, so the handler
  only sets a flag and consumes; `handle_event` drains it the moment
  `dispatch_keyboard_event` returns — the `PendingFocusWork` shape. On that path
  the flag cannot be left set: only a handler that returns `true` sets it, and
  `true` is exactly when the drain site runs. That holds for the Escape path,
  not for the flag as such — `dispatch_dismiss` is public and a shell calling it
  for another gesture (Android Back) would set the flag with nothing to drain
  it, so a new caller has to drain it the way `handle_event` does.
- **Web has no arbiter** — `register_focus_target` is desktop/Android/embed
  only; use a real `tabindex` and the DOM's own `focus`/`blur` there.

**A right press on a text target opens a built-in Cut / Copy / Paste / Select
all menu** (#813) — an `<input>` of a text-like type, a `<textarea>`, or the
rich-text `Editor`; not a checkbox, a plain focusable node or a render surface.
It is a runtime-built DOM overlay in the `<select>` popup's shape
(`app/text_context_menu.rs`): body-portal nodes, a dismiss-stack entry pushed at
*open* and released on close, closed by any outside press (swallowed), a wheel,
a window blur, a focus move, or the field leaving the document (checked on
every event). A live `data-oncontextmenu` on the field or an ancestor wins and
the menu stays shut. Each item runs **the chord's code** — `handle_cut` /
`handle_copy` / `handle_paste` / `handle_select_all` for the editable fields,
`editor_cut` / `editor_copy` / `dispatch_editor_paste` / `selectAll` for the
editor — and `text_context_menu_tests` pins item and chord to identical value,
selection and clipboard. Enabled states: Cut = selection && writable &&
not `password`; Copy = selection && not `password`; Paste = writable, **never
decided by reading the clipboard** (#149 — so it stays enabled over an empty
clipboard); Select all = content. **All three clipboard rows — Cut, Copy and
Paste — are greyed when `rinch` is built without the `clipboard` feature**:
there is no clipboard for Copy or Paste to reach, and an enabled Cut would
delete text it never copied. The press applies the native caret rule:
outside the selection it moves the caret, inside it keeps the selection. The
field keeps the keyboard throughout. The platform-neutral half is a seam for a
shell with its own toolbar: `RinchApp::text_edit_state()`,
`perform_text_edit(TextEditAction)`, `prepare_text_context_target(..)`, and
`set_text_context_menu_presentation(Shell)`, under which the gesture emits
`AppAction::ShowTextContextMenu` instead of opening the DOM menu — what the
Android half of #813 builds on.

**Key auto-repeat: the press says so** (#463). Enter/Space on a focused
`FocusTarget::Node` activates **once per physical press**, and the OS delivers a
held key as a stream of `KeyDown`s indistinguishable from real ones — so
`PlatformEvent::KeyDown` carries `repeat: KeyRepeat`, three-state
(`Fresh` / `Repeat` / `Unknown`) for the same reason `PrimaryButton` is
(#189): *a backend that cannot see is a real case and must not be made to
guess.* winit fills it on desktop, `KeyEvent::repeat_count()` on Android, the
debug/MCP channel and the `game-embed` host fill it themselves;
`Unknown` is the `Default` and falls back to `RinchApp::node_activation_held`,
the press/release latch rinch has always used.

**The latch alone was the #189 shape**: armed on the way down, cleared only by
the matching `KeyUp` — and a release that never reaches us (alt-tab while held,
a WM grab, a native menu or modal taking the keyboard, an embed host or the MCP
channel that sends no releases at all) stranded it, killing that key on that
node **for the rest of the session**, silently. A `WindowFocus(false)` clear
(landed with #147, pinned by nothing until #463) bounds it, and on Android it was
the *only* clear, since that backend translates no `KeyAction::Up` at all
(#479 — whose activation-latch half this closes, leaving it about release
*visibility*). But a second event can be swallowed too; a fact carried **by the
press being judged** cannot. `RinchApp::press_is_fresh` is the one place that
decides, and `Fresh` is authoritative *over* the latch — that is the repair, not
a tie-break.

Not carried to `KeyEventData`, so a registered node's `on_key` still cannot tell
a repeat from a press: the honest field is the same three-state one, and
`rinch-core` depends on neither `rinch-platform` nor the browser, so it needs a
home for the type first (**#797**).

Guide: `docs/src/guide/focus.md`.

## Features

### Rendering Backends

Rinch supports two rendering backends, selected at compile time:

| Backend | Feature | Renderer | Presentation |
|---------|---------|----------|--------------|
| **GPU** | `"gpu"` | Vello + wgpu | GPU compositing |
| **Software** | (default) | tiny-skia | softbuffer |

Set in `Cargo.toml`:
```toml
# GPU mode:
rinch = { workspace = true, features = ["desktop", "gpu"] }

# Software mode (default):
rinch = { workspace = true, features = ["desktop"] }
```

Both use the same `Painter` trait (`crates/rinch-dom/src/paint/painter.rs`):
- `VelloPainter` — records commands into `vello::Scene`
- `TinySkiaPainter` — rasterizes directly to RGBA pixmap

The software renderer includes **dirty region caching**: when only a few nodes change, only the affected rectangular area is cleared and repainted. Subtrees outside the dirty region are skipped during paint traversal.

**Scrollbars.** A scroll container paints an overlay thumb on each axis that is scrollable (`overflow-{x,y}: scroll | auto`) *and* overflowing — 6px thick, 2px margin, 20px minimum thumb, fully rounded, and a neutral 40% that follows the container's palette (see **Styling the bar** below). Both bars are hit-tested over a wider 16px strip along their edge and can be dragged (issue #178). Where both are present each track gives up the other bar's footprint at its far end, so the bottom-right **corner belongs to neither**: nothing paints there and clicking it falls through to the container. Desktop only — on the web the browser draws its own.

That geometry lives in **one place**, `crates/rinch-dom/src/paint/scrollbar.rs`
(`scrollbars(tree, node_id, scale)` → a `Scrollbars` holding an `Option<ScrollbarTrack>` per axis — `None` where that axis has no bar): paint draws
the thumb from it, and `find_scrollbar_hit` plus the `MouseDown`/`MouseMove`
arms in `crates/rinch/src/app/` press and drag it by it. They used to derive it
separately and had drifted (#400) — paint measured the track across the
container's **border** box and input across its **content** box, and input never
knew about the 20px minimum thumb — so a drag did not move the thumb the
distance the pointer moved. A drag now converts pointer distance to scroll
distance through `ScrollbarTrack::scroll_for_drag`, whose denominator is the
thumb's **travel** (`track_len - thumb_len`), not the track length. Anything new
that needs to know where a bar is should ask that module rather than re-derive
it.

**What counts as scrollable content (#765).** `content_extents` measures the
scroll range from the container's **direct children**, and only from those the
container is the **containing block** of — `out_of_flow::contributes_to_scrollable_overflow`,
which mirrors `out_of_flow_kind`'s walk. The two **must not drift apart**, and
nothing enforces it — `scrollable_overflow_tests` pins this predicate against
Chrome, but no test compares it with `out_of_flow_kind`. They share
`Node::establishes_abs_containing_block`, but each spells its own `match` on
`position` and its own initial-containing-block test, so a change to either —
a transformed ancestor containing a fixed box (#386/#415), say — has to be made
in both. A `position:
fixed` child resolves against the viewport, so it is no part of any scroll
range below it; an `absolute` child counts only where the container is
positioned or transformed (`Node::establishes_abs_containing_block`) or *is*
the initial containing block, which in rinch is the `<html>` box. Taffy lays
every out-of-flow box out against its direct parent whatever CSS says, so
without that filter a closed `Drawer` — `position: fixed`, and still rendered
since #751/#761 — made an `overflow: auto` ancestor report 800x600 of content
and paint **both** bars over a div holding 100x50. `visibility: hidden` is not
the rule and must not become it: a hidden box still has a box and still counts,
in rinch as in Chrome. `display: none` generates none and its zeroed rect
contributes nothing without a special case.

Limits, all pre-existing and none introduced by that filter. The walk
is one level deep, so an absolute whose containing block is a **non-parent**
ancestor contributes to no box's range at all — Chrome gives it to that
ancestor (measured: a 700x1500 absolute under a static `overflow: auto` div
lands on `documentElement.scrollHeight`), and rinch used to give it to the
wrong box, which is what grew the phantom bar (**#770**). And
`find_vertical_scroll_container` compares the same extent against the
container's **border**-box height where `scrollbars` compares it against the
content box, so a padded container can paint and drag a thumb the wheel routes
straight past (**#769**). A **positioned** child — `absolute`, or `relative`
with an offset — is measured differently from Chrome. The paddings of rinch's
content-box frame and Chrome's padding-box frame cancel for a **non-positioned**
child only. An `absolute` child's overflow is over-reported by **up to** the
container's end padding: a `position: absolute; inset: 0` child (a
`LoadingOverlay`) in a `padding: 20px; overflow: auto; position: relative` panel
paints two bars with 20px of travel where Chrome paints none. A `relative` child
is read only at its offset position, where Chrome also counts its static one. An
offset toward the end over-reports by up to the end padding; an offset toward
the **start** under-reports by up to the offset, and needs no padding to do it —
a 240x50 child at `left: -40px` in a plain 200x100 scroller gives Chrome 40px of
travel and rinch no bar. `content_extents`' doc has the rule and the
measurements. And the extent is each child's untransformed **border** box, so a
child's `transform`, its end margins, and the children of a `display: contents`
wrapper (whose own box is 0x0) add nothing, where Chrome counts all three.

**Styling the bar (#416).** Two inherited custom properties:
`--rinch-scrollbar-color: <thumb> [<track>]` and `--rinch-scrollbar-width: auto
| thin | none`. One declaration on `:root` restyles every scroll region.
`thin` is a 4px thumb; **`none` removes the bar from paint *and* from hit
testing**, so an app drawing its own can switch rinch's off rather than cover
it up. A track is painted only when a second colour is given.

They are `--rinch-` custom properties rather than the real `scrollbar-color` /
`scrollbar-width` because both real properties are **gecko-only in Stylo** — a
codegen-time filter, not a `#[cfg]`, so the servo build rinch uses emits no
parser entry and drops the declaration (grep this repo's generated
`properties.rs` for `scrollbar_color`: nothing). Custom properties cascade and
inherit normally in that build, which is what makes the root declaration work.

The `auto` default is **not a fixed colour**: the thumb is 40% black or 40%
white, chosen by the luminance of the container's computed `color`. A light
theme's text is dark, so the thumb is the 40% black it has always been; a dark
theme's text is light, so it flips to white and becomes visible — which is the
whole of #416, with no opt-in. Only the polarity follows the palette, so a
`color: red` container does not get a red thumb; `color` is read rather than
`background-color` because backgrounds are transparent by default.

**Viewport holes.** A `data-viewport` node is a compositing hole: `find_viewport_rects`
(`crates/rinch-dom/src/paint/mod.rs`) cuts its rect out of every clipping ancestor's
background fill, `<body>` included (the UA sheet makes it `overflow-y: auto`), so the
layer underneath shows through. On a **transparent** window a hole nothing fills is
see-through to the desktop, so a viewport whose content can be absent opts out by
stamping **`data-viewport-ready="false"`** — paint then leaves the backgrounds alone and
the node paints its own `background` (a placeholder, poster, or error affordance). The
attribute is an **opt-out and absence means ready**: `GameViewport` stamps nothing and
punches unconditionally, unchanged. `VideoViewport` opts in — it stays `"false"` until
mpv hands a real frame to the compositor (`VideoPlayer::has_frame`, reset by
`set_source`) and returns to `"false"` on a `PlaybackState::Error`, which is issue #186.
A node that carries the attribute must say exactly `"true"` to punch, so a mis-stamped
value fails safe.

**Layout invalidation: three paths, two flags.** `resolve_layout` early-returns
when `tree.layout_dirty` is false (styles resolve, dirty Parley layouts rebuild,
**no Taffy compute**), and runs the inline-formatting-context setup passes only
when `tree.ifc_dirty` is true. Both gates are load-bearing for frame cost and
both used to be closed on changes that move a box:

- **A typography change is a layout change (#678).** `font-family`,
  `font-weight`, `font-style`, `line-height`, `letter-spacing`, `word-spacing`,
  `text-transform`, `white-space` and `overflow-wrap` are not Taffy properties —
  and neither is **`font-size`**, for a box whose own sizes are in `px`, since it
  reaches the Taffy style only through a value that *uses* it. Any of them
  re-wraps the text, so `ComputedStyle::same_measured_text_inputs` sets
  `layout_dirty` for them. That predicate is deliberately **narrower** than
  `same_text_layout_inputs` (#654), which decides whether the *glyphs* must be
  re-shaped: `color` is baked into the glyphs and moves no box, so a
  `:hover { color }` still takes the cheap path — pinned by
  `frozen_box_remeasure_tests::a_colour_only_restyle_still_skips_taffy`, which
  reads `tree.taffy_computes`, the counter that exists because nothing else
  distinguishes "took the cheap path" from "recomputed and got the same answer".
  Cost, measured on 500 rows: a whole-document typography swap goes 7.4 → 16.1ms,
  a one-row hover 0.60 → 0.70ms.
  **Being in that list is necessary and was not sufficient** (#698):
  `letter-spacing` and `word-spacing` sat in both predicates while reaching no
  Parley producer but `build_parley_layout`, whose only callers are two MCP
  debug tools — so they invalidated correctly and then re-shaped the text
  without themselves. They now reach the IFC root, the per-span properties, the
  `TextMeasure` context and both of its consumers, both `text-overflow:
  ellipsis` rebuilds and paint's on-demand fallback. The **form-control** text
  path (`<input>`, `<textarea>`, `<select>`) is deliberately not among them:
  paint and the two hit-test builders there have to move as one piece, which is
  #320. A percentage spacing is still dropped where Chrome resolves it against
  the font-size (#743).
- **An atomic inline is sized by three passes and no compute (#661).**
  `inline-block`, `inline-flex` and `inline-grid` boxes are detached from their
  parent's Taffy child list so the enclosing IFC can measure them as Parley
  `InlineBox`es, so the **root compute never reaches one**. All three sizers go
  through `measure_inline_blocks`: `compute_inline_block_layouts` on an
  `ifc_dirty` pass, `resolve_percentage_inline_blocks` after the root compute
  for a percentage inline size (which is also what makes such a box track a
  viewport resize, on a pass with no `ifc_dirty` and an empty dirty set), and —
  since #661 —
  `remeasure_dirty_atomic_inlines`. `ifc_dirty` is left false by a style-only
  restyle and by a `set_text_content` alike, so before that third pass the box
  was measured once and frozen at `225x20` while paint drew six lines. It
  re-measures the ones a change actually reached, off a **dirty set**
  (`tree.dirty_atomic_inlines`) rather than a flag: measured, re-measuring the
  whole document instead costs
  +47% on a one-row text edit in a 500-row document carrying 500 chips.
- **A transition or animation writes `computed_style` directly**, so it reaches
  none of the cascade's invalidation; `tick_transitions` and `tick_animations`
  each invalidate the text measure of the nodes they are interpolating
  (`TransitionProperty::changes_text_measure`). `font-size` is the only
  animatable property in that set today, and the `All` arm of that predicate is
  unreachable — a transition map is always keyed by the concrete property — so
  `transition: all`, which `Checkbox` and `Radio` both declare, pays nothing per
  frame. **Their Taffy re-sync marks atomic inlines separately**, because that
  pre-pass fires only for `font-size` while a `transition: width` on a box
  *inside* an `inline-block` is #661's own symptom reached without the cascade
  (found by the review of #694).

`RinchDocument::invalidate_text_measure_for_node` is the one place that knows
what a typography change owes: the IFC's Parley layout, the box of any atomic
inline above it, and the `NodeContext::Text` a text child is measured through
when it is a flex or grid item — plus the Taffy `mark_dirty` beside that last
one, since Taffy caches a leaf measure per available space and serves the stale
one back otherwise.

**Absolute positioning.** Taffy resolves an out-of-flow box against its **direct
parent**, always. CSS resolves an absolute box against its nearest *positioned*
ancestor — or, when it has none, against the initial containing block. rinch
corrects the second case (#204): an absolutely positioned box with **no**
positioned ancestor (nothing non-`static`, no transform, up to `<html>`) resolves
against the viewport, so `inset: 0` inside an unpositioned 300x200 div gives an
800x600 box, matching the browser and therefore `rinch-web`. The correction is
`crates/rinch-dom/src/out_of_flow.rs` — a pre-layout **size** bake into the Taffy
style (so the box's own children lay out inside the right box) plus a post-layout
**position** patch in `read_layout_results`. That patch writes a
*parent-relative delta*, so `LayoutResult` keeps its meaning and no coordinate
consumer — paint, stacking, hit testing, `ClickContext`, the MCP `absolute`
contract — needs an exception. An axis with both insets `auto` keeps Taffy's
static position, which is what CSS asks for.

**Not covered (#386):** an absolute whose nearest positioned ancestor is not its
direct parent is still parent-resolved (its used size isn't known until a first
compute pass); percentage `padding`/`margin` and percentage `min-`/`max-` sizes on the
box; the shrink-to-fit available width of an auto-sized absolute. `position:
fixed` is unchanged and now shares the same helper — which is what stops
`tick_transitions`/`tick_animations` from dropping its viewport size on a
transition frame. **A component whose overlay must cover its parent (e.g.
`LoadingOverlay`) needs that parent to declare `position: relative`** — without
it the overlay now covers the window, as it always has on the web.

**Overflow clipping.** One predicate — `Node::clips_overflow()`, "either axis is
not `visible`, and the box is not a non-atomic `display: inline` element" — and
one shape, `paint::clip_shape` (the rounded border box).
Everything that needs either asks those: paint's clip bracket, its dirty-region
subtree prune, the layer-bounds walk, a hoisted entry's clip chain, hit
testing's `check_children` gate, and `RinchApp`'s two viewport clip walks.
(`creates_stacking_context` was on that list until stage B and is deliberately
not any more — see **Stacking contexts and the clip chain** below.) Those seven
sites held **four** different predicates before #324 stage A, and the
disagreement was real rather than cosmetic: paint, `layer_bounds` and
`creates_stacking_context` matched `overflow_y` against `Hidden | Scroll | Auto`
and so **missed `overflow: clip` entirely**, while hit testing clipped it —
content drawn and not clickable.

**But "this node clips, therefore a bracket is open" is no longer true** (card
K43). A Vello clip layer is two extra passes over the clipped area, paid whether
or not it removes a pixel, so `paint_node` may *decline* to push one for a box
that answers `clips_overflow` — when the clip covers the render target, or when
`layer_bounds::clip_cuts_nothing` says nothing inside reaches past it. Both cases
require **square corners**: a rounded clip cuts the corners of its own box, so a
subtree fitting the box does not mean the shape cuts nothing.

This weakens no answer above, and consumers must not assume it does — the
predicate and the shape are unchanged, and everything reasoning about *where
content ends up* (the clip chain, hit testing, `layer_bounds`' own intersection)
still applies the clip, because the elision only ever drops a restriction that
was already vacuous. What it breaks is the **reverse inference**, which is why
`paint_children_with_stacking` is *told* whether a bracket is open
(`clip_elided`) rather than deducing it. Anything new that needs to know must be
told too, not infer it from the predicate.

`clip` is the only value that ever reached that gap, and that is measured rather
than reasoned: css-overflow-3 §3 makes a `visible` compute to `auto` when the
other axis is neither `visible` nor `clip`, Stylo's style adjuster implements
it, so `overflow-x: hidden; overflow-y: visible` is **unreachable** — pinned in
`crates/rinch-dom/tests/clip_predicate_tests.rs`, which is also what fails if a
Stylo bump ever changes that. `clip` beside `visible` is the pair the spec
allows, so it stays asymmetric.

Two deviations from CSS remain **in the predicate and the shape** — this is not
an inventory of everything rinch gets wrong about `overflow`, which also covers
scrolling, `text-overflow` and scrollbars. rinch clips both axes with one rect,
so `overflow-x: clip; overflow-y: visible` clips vertically too (#535). And the
rect is the **border** box where CSS clips to the padding box, so a clipping
container with a non-zero `border-width` lets its content paint over its own
border (#536); with no border the two coincide, which is every clipping box in
the component library.

A third one is gone: **clipping no longer forms a stacking context** — see
**Stacking contexts and the clip chain** below.

**A non-atomic `display: inline` element never clips** (#591 PR 1), whatever its
`overflow` computes to — `overflow` applies to block, flex and grid containers
(css-overflow-3 §3), and an inline *box* is none of those; an `inline-block` is a
block container and still clips. The predicate says so, not `clip_shape`, so the
bracket, the chain, hit testing's gate and the dirty-region prune all agree. The
rinch-specific reason it had to be said: a *flowed* inline element owns no box
(`Node::is_flowed_inline_element` — its `layout` is zeroed and `E ghost box`
enforces that), so a clip derived from it was a `0x0` rect that the stacking
collector pushed onto the chain of a positioned box hoisted out from under it
whose entry carries the live chain — a `relative` box, or an `absolute` whose
containing block is the span itself; an `absolute` truncated at a containing
block above the span (#591's own child) and a `fixed` box never carried it. An
`inline-block` button inside an `overflow: hidden` span could not be tapped. **The
guard is deliberately wider than the boxless set**: a split inline and an
*unmarked* inline element — one no IFC has claimed, which therefore keeps a real
Taffy box — are inline boxes too, and `overflow` applies to neither — narrowing
the guard to the flowed predicate silently restores the clip on both. The
unmarked case used to be produced by the inner `<span>` of a re-measured
`inline-block` (#630); since #592 an `inline-block` is a block container and that
span is its IFC content with a `0x0` box, so the state is now reached only by
construction in a fixture. A future component that puts `overflow: hidden` on an inline-level
element loses its clip silently, exactly as in a browser; there is no warning.
An inline element with `overflow: auto` is still found as a *scroll container*
(those walks read `overflow` against `Scroll | Auto` directly) while clipping
nothing — pre-existing, and no ink is at stake on a `0x0` box.

Anything new that asks "does this clip" must call the predicate, not re-derive
it. Code looking for the nearest *scroll* container (sticky positioning, wheel
routing, the scrollbar overlays) wants a different question and must not borrow
this one: `visible` and `clip` are the two non-scrollable values and only one of
them clips.

**Stacking contexts and the clip chain.** `Node::creates_stacking_context()`
answers: a positioned box with an explicit `z-index`, `position: fixed` or
`sticky` whatever the `z-index`, `opacity < 1`, a non-identity `transform`.

That is **not** the whole CSS list, and the shortfall is not only about what
`ComputedStyle` can hold. `clip-path`, `mask`, `isolation`, `mix-blend-mode`,
`contain: paint` and `will-change` are absent from `ComputedStyle` altogether
and need new style plumbing per property. But **two creators are representable
today and simply missing** (measured, not argued):

- a non-`none` **`filter`** (CSS Filter Effects §2.1) — `filter: brightness(0.5)`
  reaches `ComputedStyle::filter_brightness` and paint consumes it, and the
  predicate still answers `false`. (`blur()` really is unexpressed; only the four
  scalars survive `from_stylo`.)
- a **flex or grid item with a `z-index`** other than `auto`, even at
  `position: static` (css-flexbox-1 §5.4, css-grid-1 §6) — both the `z_index`
  and the parent's `display` are already in `ComputedStyle`.

Neither was folded into stage B, because adding a creator changes which boxes
hoist — the axis stage B is re-founding — and landing both at once would make a
regression impossible to attribute. **Tracked as #542**; the six properties
`ComputedStyle` does not carry at all (plus `blur()`, which really is
unexpressed) need per-property plumbing and are separate work again. **#415** is
the related one, in this same function and the same class: a `transform` that
composes to the identity creates no stacking context here, where CSS keys on
`not none`.

Stage B slightly **widens** that exposure rather than leaving it untouched: a box
declaring **both** a filter and a clipping `overflow` used to get a stacking
context by accident through the `overflow` arm, and no longer does. Its clipping
survives — the chain carries that — its ordering does not. A filter box with no
`overflow` was already wrong before.

**`overflow` is not on that list** (#324 stage B). It used to be, so that a
descendant hoisted to an ancestor's paint sequence stayed inside the bracket
paint opened around it — clipping was made a stacking question because the clip
was implemented as a per-sequence bracket. That cost the same user-visible bug
twice, because it made rinch compare two `z-index` values from different
stacking contexts, which CSS never does: #317's `DropdownMenu`/`Select`
backdrops and #534's menu-bar overlay, each worked around by respelling a
`fixed` overlay as `absolute`, and each reverted in stage C.

Clipping is decoupled from stacking now. Every hoisted
`stacking::PaintEntry` carries a **clip chain** — the clipping ancestors between
it and the collecting root, as a `ClipSpan` into `PaintOrder::clips` — and its
consumer re-applies them: `paint_children_with_stacking` pushes them around the
entry, `hit_test_node` rejects a probe point outside them. Four rules make it
correct, and each has a fixture in
`crates/rinch-dom/tests/clip_chain_tests.rs`:

- **A link's rect is the clipping box's own `clip_shape`, at its PRE-scroll
  painted origin.** A container's box does not move when its content scrolls.
  Invisible at scroll offset 0, which is why every fixture is scrolled.
- **`position: absolute` truncates its chain at its containing block.** CSS does
  not clip an absolute by an `overflow` ancestor below its containing block, so
  `descend` tracks `live.len()` as of the nearest
  `establishes_abs_containing_block()` and an absolute entry takes only that
  prefix. This is also what keeps #204's initial-containing-block correction
  correct. `position: fixed` takes an **empty** chain; everything else takes the
  whole one — an in-flow or `relative` box is clipped by every clipping
  ancestor, containing block or not.
- **The collecting root's own clip is in no chain.** Paint opens that bracket
  before it walks the sequence, and hit testing gates the whole walk on it —
  **except around a `position: fixed` entry** (#545), which the root does not
  contain: paint lifts the bracket for the length of that entry and puts the very
  same shape back, and hit testing exempts it from the bounds gate per entry.
  That was stage B's documented "Known gap", and #545 is what made it live.
  **Only that one clip is lifted (#549).** The chain on the *root's own* entry,
  in whatever sequence hoisted the root, is still pushed around the root's whole
  subtree — so under `plain clipper > stacking context > fixed` the fixed box is
  clipped away, where a browser paints it. Paint and hit testing agree on it, so
  it is a consistent deviation and not drift; the honest fix moves the collecting
  root's clip off the bracket and into every entry's chain, which would close
  #386's shape too. A third consumer has to know the same thing from the other
  side: `paint::layer_bounds` walks the tree rather than the sequence, so it sees
  clippers a fixed descendant escapes, and its `Extent::Escapes` case stops one
  narrowing a translucent layer to less than it paints — which tiny-skia ignores
  and Vello enforces, i.e. a software/GPU divergence no pixel test can see.
  **`position: absolute` has the same hole and it is NOT handled for the two
  *bounds* callers** (#550), but it is a *different shape*. `Collector::span`
  truncates an absolute's chain at its containing block (`Absolute =>
  self.cb_depth`, set at **any** `establishes_abs_containing_block()` ancestor —
  any non-`static` position or a transform), so it escapes the clippers *below*
  that block while staying clipped by the ones above. `opacity_layer_bounds` and
  `clip_cuts_nothing` narrow it at **every** clipping ancestor regardless, so a
  layer holding one can come back too small with the same GPU-only symptom. It
  needs a **partial** escape, which `Escapes` cannot express — that is why only
  the `fixed` case is covered there, and it is pre-existing rather than something
  #204 introduced. The module's **third** caller,
  `subtree_is_entirely_outside` (the off-window cull, #562), does **not** accept
  that hole and does not close it either: it answers `Escapes` for an absolute
  whenever a clipping ancestor **below the walk root** sits below the containing
  block, which is whole-escape where partial would be exact — conservative in the
  direction that costs an unpruned subtree rather than a deleted box. *Below the
  walk root* is the whole qualification and not pedantry: the root's own clip is
  one no absolute inside escapes, because paint opens that bracket around the
  entire sequence (#549), so a clipping root that is not itself a containing
  block answers `Within` and prunes —
  `offscreen_cull_tests::a_clipping_non_containing_block_root_still_prunes`. The asymmetry is
  deliberate: a bounds under-measure costs content on the Vello path only, a
  cull under-measure costs it on every path. `layer_bounds.rs`'s module doc names
  all of this explicitly; read it before touching this.
- **No transform composition is needed.** A transform creates a stacking
  context, so `descend` never crosses one and every link lives in the collecting
  root's own untransformed space — the same space the entries' offsets are in.

Hit testing tests a link's **rect only**, ignoring its radii, matching the
`check_children` gate it stands in for; paint pushes the rounded shape. That is
the pre-existing rounded-corner divergence, not a new one.

**Two behaviour changes that follow from the rules, are CSS-correct, and will
still surprise someone.**

- **An `absolute` with no positioned ancestor now escapes an intervening
  *static* `overflow: hidden` entirely.** It resolves against the initial
  containing block (#204), so no `overflow` box between it and the root is in
  its containing-block chain and its chain is empty. Before, that box was a
  stacking context and its bracket caught everything hoisted into its sequence,
  so the absolute was clipped. **Measured against Chromium**, not reasoned:
  `elementFromPoint` 100px outside a static `overflow: hidden` 200x200 box
  answers the absolute, and its `offsetParent` is `BODY`. On a transparent
  borderless window the visible form is an absolute painting into the rounded
  corners; the remedy is the browser's — put `position: relative` on the wrapper
  you meant to clip with.
- **A `z-index: -1` descendant of a scroller now sorts into the *body's* step 1**
  rather than the scroller's, so it paints behind the scroller's own background
  instead of merely behind its content. Still clipped by the chain. This one
  follows from the same rule rather than being separately measured — the
  scroller is not a stacking context, so it cannot hold a negative-`z` layer of
  its own.

**Cost.** `TinySkiaPainter::push_clip` allocates a full-surface `Mask`, so a
push is not free — but consecutive entries that share a `ClipSpan` share one
push, which is exact rather than a heuristic (identical spans name identical
clips), and it collapses the shape that motivated the worry: a scroller with 200
positioned rows is **one** push, not 200. Measured at 1200x800, software
painter, best of 40: 200 positioned rows in one scroller 1.82 → 1.90ms (+4.5%);
an adversarial 50 scrollers x 4 rows — one push per scroller, so **50**, since
each scroller's own entry carries no chain and interrupts the run — 2.73 →
3.08ms (+13%). Building the body's sequence went 1 → 3us.

`clip_chain_tests::consecutive_entries_that_share_a_chain_share_one_clip_push`
is the pin, and **the 50-scroller assertion is the load-bearing half of it**.
The headline "200 rows = 1 push" is a *fixed point*: one scroller means the clip
table only ever holds one chain, so a collector that reused far less still
answers 1. Reusing the table's head instead of its tail leaves every pixel and
every tap identical — pushing the same geometry twice is idempotent — and takes
the 50-scroller case to **197** pushes and a 197-entry table. Do not simplify
that fixture down to one scroller: the reuse would then have no test at all, and
a refactor could quietly 4x the mask cost in the exact workload benchmarked
above. The
containment skip the scoping proposed (don't push a clip the entry is entirely
inside) is **not** implemented: a sound version needs a subtree extent per entry,
which is a new per-frame walk for every positioned box, and the run reuse already
took the realistic case.

**A `position: fixed` box is hoisted to its NEAREST ancestor stacking context**,
not to the body (#545). It is viewport-*positioned*, not viewport-*stacked*: its
entry keeps zeroed offsets, an empty clip chain and the body's transform, but it
is entered in the sequence CSS says owns it. **"Empty clip chain" is not "escapes
every clip"** — see the paragraph on #549 below. It used to be pulled out to the body
whatever lay between, which compared two `z-index` values across two stacking
contexts — the same fault `overflow` caused before stage B. A `z-index: 99`
dismiss backdrop escaped a wrapper its `z-index: 100` panel could not and covered
it; and under a wrapper whose own `z` was *above* 99 (`Modal` and `Drawer` at 201,
`Notification` at 300) the backdrop sank beneath the whole wrapper, so a popup
inside a modal could not be dismissed at all. Chromium does neither. There is no
`is_body` flag in `stacking.rs` any more: the body is simply the outermost
stacking context.

rinch still models no **containment** — CSS makes a *transformed* ancestor the
containing block of a fixed descendant (and an `opacity` one not), while
`out_of_flow::out_of_flow_kind` answers "the viewport" for every fixed box. That
was already wrong before #545 and is exactly as wrong after; what #545 preserves
is that it is wrong *consistently*, since paint keeps handing a fixed entry the
body's transform. Tracked with #386 and #415.

**Stage C reverted both workarounds.** `.rinch-dropdown-menu__backdrop`,
`.rinch-select__backdrop` and `.rinch-app-menu-bar__overlay` are `position:
fixed` again, and `build_overlay`'s compensating offset is gone with them. That
buys back two behaviours, each pinned by a pair of tests in
`popup_backdrop_hit_tests` — one asserting the behaviour, one asserting the
`absolute` spelling did not have it: a tap **outside the popup's clipping
ancestor** dismisses, and a tap on the **app's own fixed chrome** dismisses
rather than being taken by the chrome.

The overlay's `position` and its offset had to move **together**, and the test
that says so fails from *both* sides: flipping only the `position` fails
`the_below_titlebar_overlay_covers_the_whole_window` at the window's bottom edge,
flipping only the offset fails the same test at the title bar. Stage C also
needed **#545** as much as stage B — the reverted overlay lands in
`.rinch-app-menu-bar__inline-layer`'s own sequence at 199 beside the row's 201,
measured, so the "199 and 201 in one sequence" its comments describe is literally
what happens. Hoisted to the body it would have sat at 199 against the *layer's*
200, which orders the same way: every test would have stayed green with the
comments' mechanism wrong.

**Key files:**
- `crates/rinch-dom/src/stacking.rs` — the paint sequence and the clip chain
- `crates/rinch-dom/src/paint/clip.rs` — the clip predicate and shape
- `crates/rinch-dom/src/paint/painter.rs` — Abstract `Painter` trait
- `crates/rinch-dom/src/paint/vello_painter.rs` — GPU backend
- `crates/rinch-dom/src/paint/skia_painter.rs` — Software backend
- `crates/rinch-dom/src/paint/mod.rs` — `paint_document()`, dirty region computation, subtree pruning
- `crates/rinch/src/app/mod.rs` — `build_scene()` (GPU) / `build_pixels()` (software)

### Image Support

Images render on **both** desktop backends — GPU (Vello, `scene.draw_image`) and software (tiny-skia, `draw_pixmap`) — via `<img>` elements and `background-image: url(...)` CSS. Remote/file images load asynchronously on background threads; `data:` URIs (e.g. base64 PNG) are decoded synchronously and inserted straight into the cache (`request_image_load_for_node`).

**Architecture:**
```
rinch-core:  ImageLoader trait + ImageLoadResult enum (no deps)
rinch-dom:   ImageCache + FileImageLoader + decode pipeline (image crate)
rinch:       NetworkImageLoader (rinch-http, gated behind image-network feature)
```

**Key files:**
- `crates/rinch-core/src/image.rs` — `ImageLoader` trait, `ImageLoadResult` enum
- `crates/rinch-dom/src/image_cache.rs` — `ImageCache`, `DecodedImage`, `FileImageLoader`, async load
- `crates/rinch-dom/src/paint/image.rs` — `paint_image()` with object-fit support: all five of `fill`/`contain`/`cover`/`none`/`scale-down` (`ObjectFitValue`, `computed_style/values.rs`)
- `crates/rinch/src/image_loader.rs` — `NetworkImageLoader` (feature-gated)

**How it works:**
1. `set_attribute("src", ...)` on `<img>` or `BackgroundValue::Image` during style resolution triggers a load
2. `ImageCache` checks if already cached; if not, spawns a background thread via `request_image_load()`
3. The background thread calls `ImageLoader::load()` then decodes with the `image` crate to RGBA8
4. Results go to a static `Mutex<Vec<PendingImage>>` queue
5. `drain_pending_images()` at the start of layout picks up decoded images, updates Taffy intrinsic dims
6. `paint_image()` renders via `scene.draw_image()` with proper affine transforms

**Network loading:** Enable `features = ["image-network"]` for HTTP(S) URL support. It goes through `rinch_http::fetch_blocking`, **not** a private `ureq` call, so image loads share the app's one HTTP agent — its cookie jar, proxy and TLS config (`image-network = ["dep:rinch-http"]`).

**Circular avatars:** a clipping ancestor with `border-radius` clips to a
`RoundedRect` rather than a plain rect, which is what crops an `<img>` to a
circle. The shape comes from `paint::clip_shape` — see **Overflow clipping**
under Rendering Backends for the predicate and its deviations.

### DevTools Panel

Press F12 to toggle the DevTools panel which shows:
- **Performance**: FPS, frame time, and render time
- **Elements**: DOM tree inspection
- **Styles**: Computed styles for selected elements (enable inspect mode with Alt+I)

### Debug & MCP Server (optional)

Enable with `features = ["debug"]` to expose your app's DOM, screenshots, and input injection to external tools via TCP IPC.

**Architecture:**

```
Claude (stdio) → rinch-mcp-server → TCP:port → rinch-debug (in app) → rinch runtime
```

Two crates work together:

| Crate | Type | Purpose |
|-------|------|---------|
| `rinch-debug` | Library | TCP IPC server embedded in the app, auto-starts on a random localhost port |
| `rinch-mcp-server` | Binary | Standalone MCP server that Claude talks to, discovers and connects to running apps |

**Enabling in your app:**

```toml
# Cargo.toml
[dependencies]
rinch = { workspace = true, features = ["debug"] }
```

The debug server starts automatically when the feature is enabled. Disable at runtime with `RINCH_DEBUG=0`. Force a specific port with `RINCH_DEBUG_PORT=9100`.

**MCP configuration** (`.mcp.json`):

For fastest startup, point to the pre-built binary:

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

Build first with `cargo build -p rinch-mcp-server`. Using `cargo run` instead works but is slower to start.

**MCP tools available:**

| Tool | Description |
|------|-------------|
| `list_apps` | List all running rinch apps with debug enabled |
| `connect` | Connect to a specific app by name or PID |
| `screenshot` | Capture a PNG screenshot (returns as inline MCP image, directly viewable) |
| `dom_tree` | Get the DOM tree as JSON with layout bounds (depth 3 by default — `max_depth` goes deeper, `root_id` scopes to a subtree, `verbose: true` adds each node's computed styles) |
| `query_selector` | Query nodes by tag, `.class`, `[attr]`, or `[attr=value]` |
| `get_node` | Get detailed info for a specific node by ID (includes computed styles, display mode) |
| `get_computed_styles` | Get computed CSS styles for a specific DOM node |
| `get_text_content` | Get text content within a node subtree |
| `click` | Simulate a mouse click at (x, y) coordinates |
| `type_text` | Simulate keyboard text input |
| `wait_frame` | Wait for the next render frame |
| `close_app` | Close the connected app gracefully |
| `right_click` | Simulate a right-click at (x, y) |
| `mouse_down` / `mouse_move` / `mouse_up` | The pointer primitives. **This trio is the only way to drive a drag** — `click` cannot, so any test of the DnD suite or a scrollbar thumb needs these |
| `scroll` | Scroll a container at (x, y) |
| `key_press` | Press a single key (with modifiers), as distinct from `type_text`'s literal text |
| `get_caret_position` | The text caret's rect — note it mixes logical and physical px at scale != 1 (#421) |
| `get_glyph_bounds` | The box of the **one** glyph cluster at a given `byte_offset` in a text node — not every glyph; same #421 caveat |
| `disconnect` | Disconnect from the app without closing it |
| `launch_app` | Launch a rinch app via `cargo run -p <package>`, wait for debug registration, auto-connect |

**Discovery mechanism:** Each debug-enabled app writes `~/.rinch/debug/{pid}.json` containing its port, app name, and PID. The MCP server scans this directory to find running apps and auto-connects when only one is running.

**IPC protocol:** Length-prefixed JSON over TCP on localhost. 4-byte big-endian length prefix followed by JSON payload. Handshake exchanges protocol version on connect.

**Key source files:**
- `crates/rinch-debug/src/server.rs` - TCP listener (blocking I/O, no tokio dependency)
- `crates/rinch-debug/src/protocol.rs` - Wire protocol types and framing
- `crates/rinch-mcp-server/src/mcp_server.rs` - MCP tool implementations
- `crates/rinch/src/app/debug_commands.rs` - `execute_debug_command()`, the debug-command dispatch; `shell/rinch_runtime.rs` only calls it

### File Dialogs (optional)

Enable with `features = ["file-dialogs"]`:

```rust
use rinch::dialogs::{open_file, save_file, pick_folder, message};

// Open file
if let Some(path) = open_file().add_filter("Text", &["txt"]).pick_file() { }

// Save file
if let Some(path) = save_file().set_file_name("doc.txt").save() { }

// Pick folder
if let Some(path) = pick_folder().pick() { }

// Message dialog
message("Success!").set_title("Info").show();
```

### Clipboard (optional)

Enable with `features = ["clipboard"]`:

```rust
use rinch::clipboard::{copy_text, paste_text, has_text};

copy_text("Hello").unwrap();
if has_text() {
    let text = paste_text().unwrap();
}
```

**Reading is slow and may block (#149).** A read is a request to whichever app owns
the clipboard; on X11 arboard waits up to **4s** for a hung owner, and the browser
can't be read synchronously at all. Every read comes in three shapes on every
platform: `paste_text()` (blocks, unbounded), `paste_text_timeout(Duration)` (blocks,
`Err(ClipboardError::TimedOut)` after that), `paste_text_async(cb)` (never blocks).
Same three for `paste_html`/`paste_image`. `paste_rich`/`paste_rich_async` resolve
`text/html` → bitmap → `text/plain` in **one** read (→ `RichPaste`), so a rich paste
never stacks three worst-case waits. `copy_text_async`/`copy_html_async` queue a write
without waiting.

On native all of them are served by **one clipboard worker thread** owning the
`arboard::Clipboard` (`crates/rinch-clipboard/src/native.rs`). That's what makes the
timeout useful: giving up doesn't cancel the request, so an abandoned read finishes on
the worker rather than wedging later callers behind a held lock. The worker talks to a
private `Backend` trait, so the queue/timeout/probe logic is unit-tested against a fake
clipboard with no display.

**Async callbacks are `Send` and do NOT run on the UI thread** (native: the worker;
web: the browser thread). Marshal before touching UI state — `Signal::send` for a
`Send` payload, or `park_main_callback` + `run_on_main_thread` + `resume_main_callback`
for a `!Send` continuation (an `EditorHandle`, an `Rc`). A *blocking* call made from
inside an async callback is reported as an error rather than deadlocking.

**Web (#150):** `paste_text()` answers from a buffer that rinch-web fills from the
document's `paste` ClipboardEvent — the only synchronous channel to content copied in
another app or tab — so a web app can paste from outside itself.
`paste_text_async` additionally tries `navigator.clipboard.readText()`. Because the
browser's `paste` event arrives *after* the keydown, web paste logic should hang off
`rinch_core::set_paste_interceptor(|data: &PasteEventData| -> bool)` (dispatched
**after** the buffers are filled, so `paste_text()` works inside it) rather than off an
intercepted Ctrl+V, which is still `prevent_default()`ed. One slot per document plus a
thread-global fallback, like `set_keyboard_interceptor` (#340/#478); desktop never
dispatches it (no OS paste event).

**Android: a long press on a text field shows the platform's floating toolbar**
(issue #813) — `ActionMode.TYPE_FLOATING`, started by `RinchActivity` on the
window's decor view (the 1x1 `RinchInputView` cannot host one: the framework
hides the toolbar unless the content rect intersects the originating view).
The shell sets `TextContextMenuPresentation::Shell`, so the runtime prepares
the caret and selection and hands back `AppAction::ShowTextContextMenu`; the
shell selects the word under the finger (`RinchApp::select_word_at_caret`, a
press inside a selection keeps it), shows the items `text_edit_state()` allows,
and performs a tapped item through `perform_text_edit` — the chord's own code.
Item taps, `RinchInputConnection.performContextMenuAction` and every dismissal
arrive through one queue in `rinch_android::text_action`, and
`ToolbarMirror` there is the loop's belief about the toolbar, with a count of
the finishes it asked for so a late report for a finished mode cannot take
down the one that replaced it. Two things that were silently wrong before and
are worth knowing: **a still finger never fired the long press** on an idle
loop, because K37's looper sleep has no deadline and nothing woke it
(`TouchGesture::long_press_due` now feeds `poll_timeout`); and **the `android`
feature now implies `clipboard`** — without it `handle_paste` reads an empty
string, so a hardware Ctrl+V inserted nothing and an IME's
`performContextMenuAction(paste)` returned `true` having pasted nothing (the
toolbar hides its Paste). Paste is also hidden while `hasPrimaryClip()` is
false, which reads no clip and raises no clipboard-access notice (measured),
and is asked **once per long press**, not per refresh: it is a binder round
trip (0.56 ms median, 10.7 ms p99 on the emulator) and the refresh runs every
turn the toolbar is up. **An item that finishes the toolbar is finished on
the Java side before it is reported** (PR #819 review, F1): both calls queue
an event and wake the loop, and reported first, a lone `Perform` turn
re-prepared a toolbar the loop believed was up and the UI thread started a
fresh one — an orphan whose items acted on nothing. **And a refresh never
starts a toolbar** (final review, N1): a refresh posted *before* the tap
reaches the UI thread after the item finished the mode, so
`ToolbarMirror::request` answers `Start | Update | Nothing`, the per-turn
refresh (run only while the mirror is shown) is always an `Update`, and
`showTextActionMode`'s `start` flag returns rather than starting a mode for
one — measured 11 of 11 orphans under an anchor moving every frame before
it. **A `Perform` carries its
source**, because Cut / Copy / Paste end the toolbar from either one (as in an
`EditText`, measured) but only a toolbar item has already finished it:
`ToolbarMirror::performing` takes the mirror down uncounted for a toolbar item
and asks for a counted finish for an IME request — treating the IME's like the
item's left an orphan no tap could take down, measured. Measured on an API 34
emulator with one Gboard build: its clipboard chip and clipboard panel both
paste as `commitText`, and its Text Editing panel's Paste and Select all
arrive as `performContextMenuAction` (its Cut and Copy stay disabled over a
rinch selection, which the input connection does not report to the IME).
The toolbar floats over the selection itself: `TextEditState.anchor` is the
selection's rect (a 1px caret rect when collapsed), scaled to physical px. The
rich-text `Editor` is
`desktop`-only and is not part of an Android build (#818).

**The built-in editor's Ctrl+V is asynchronous** — see the `anchor_selection` row in the
`EditorHandle` table. The plain `<input>`/`<textarea>` paste path
(`RinchApp::handle_paste`) is still synchronous: its completion would need `&mut
RinchApp`, and there is no deferred-work queue that hands one back.

### System Tray (optional)

Enable with `features = ["system-tray"]`. Uses the same unified `Menu`/`MenuItem` types as native menus:

```rust
use rinch::prelude::*;
use rinch::tray::TrayIconBuilder;
use rinch::menu::{Menu, MenuItem};

let menu = Menu::new()
    .item(MenuItem::new("Show").on_click(show_current_window))
    .separator()
    .item(MenuItem::new("Quit").on_click(close_current_window));

let tray = TrayIconBuilder::new()
    .with_tooltip("My App")
    .with_icon_png(include_bytes!("../assets/icon.png"))?
    .with_menu(menu)
    .build()?;
```

**Key features:**
- `with_icon_png(data)` — Load tray icon from PNG bytes (use `include_bytes!`)
- `with_icon_rgba(rgba, w, h)` — Load from raw RGBA pixel data
- `with_icon_path(path)` — Load from a PNG file path
- **Push-based events** — Menu callbacks fire via `MenuEvent::set_event_handler` on the main thread. No polling thread, no wasted CPU.
- **Left-click** — Shows the window automatically via `TrayIconEvent::set_event_handler`.

**Minimize-to-tray pattern:** Combine system tray with `on_close_requested` + `hide_current_window()`:

```rust
let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Don't exit, just hide
    })),
    ..Default::default()
};
```

### Theme System (optional)

Enable with `features = ["theme"]` for the theme system inspired by Mantine.

Theme is configured at the runtime level using `ThemeProviderProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { style: "color: var(--rinch-primary-color);",
            "Uses theme CSS variables"
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    App::new(app)
        .title("Themed App")
        .size(800, 600)
        .theme(theme)
        .run();
}
```

ThemeProvider generates CSS variables:
- Colors: `--rinch-color-{name}-{0-9}`, `--rinch-primary-color`
- Spacing: `--rinch-spacing-{xs,sm,md,lg,xl}`
- Radius: `--rinch-radius-{xs,sm,md,lg,xl}`, `--rinch-radius-default`
- Shadows: `--rinch-shadow-{xs,sm,md,lg,xl}`
- Typography: `--rinch-font-size-{xs,sm,md,lg,xl}`, `--rinch-font-family`
- Semantic: `--rinch-color-body`, `--rinch-color-text`, `--rinch-color-dimmed`

**Generic font families are repaired on Android** (`crates/rinch-dom/src/fonts.rs`,
issue #322). fontique maps the `monospace` slot to the literal name `"monospace"`,
which `/system/fonts` is not indexed under, so the slot is empty and `<code>`,
`<pre>` and the whole default `font_family_monospace` stack render proportional.
Build font contexts with `rinch_dom::fonts::new_font_context()` — never
`parley::FontContext::new()` — so every context resolves a stack identically;
the document's, the hit-test one, and the input hit-test one must agree glyph for
glyph or a tap lands on the wrong character. The repair fills `monospace`,
`ui-monospace`, `ui-sans-serif` and `ui-serif` only where the platform left them
empty; `ui-rounded` and `fangsong` are deliberately left alone (Android has no
face for either).

**Window chrome inset (not ThemeProvider-generated).** `--rinch-window-top-inset`
is published at runtime by whatever chrome rinch draws above your content — the
DOM menu bar (28px, on Linux and in the browser alike: `render_with_menu_bar`
publishes it on its own wrapper *and* on `scope.body_handle()`) and the
`BorderlessWindow` titlebar (36px, +28 when
the menu bar sits below it). That chrome reserves space with in-document padding,
so normal flow content clears it automatically; a `position: fixed` element does
**not**, because fixed resolves against the real viewport (matching browsers and
rinch-web). Full-height overlays must therefore opt in:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0;" }
```

`Drawer`, `Modal`, and the top-anchored `Notification` positions already do this.
`DropdownMenu`'s and `Select`'s click-catching backdrops, and the DOM menu
bar's dismiss overlay (`.rinch-app-menu-bar__overlay`), are fixed at
`top: 0` and deliberately do **not** take the inset: a dismiss region has to
cover the chrome, or clicking the title bar leaves the menu open. Do **not**
"fix" an overlay of your own by insetting the fixed containing block — that
would break CSS semantics and make desktop diverge from rinch-web.

## Transparent Windows

Transparent/borderless windows are configured via `WindowProps { transparent: true, borderless: true }`. The live renderer (`shell::desktop::WgpuRenderer`) uses `CompositeAlphaMode::Auto` + a transparent clear color, which the compositor honors on **Linux (Wayland/X11)**.

> **Windows caveat:** true per-pixel transparency on Windows needs an alpha-capable presentation path (PreMultiplied alpha + DX12 DirectComposition + `WS_EX_NOREDIRECTIONBITMAP`) that is **not currently wired** into `WgpuRenderer` — so `transparent: true` renders opaque on Windows today. Tracked in **issue #89**. (The old `TransparentWindowRenderer` held that path but was never constructed by the runtime and has been removed as dead code.)

Configure via `WindowProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::WindowProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { class: "custom-titlebar",
            // ... your custom titlebar and content
        }
    }
}

fn main() {
    let window_props = WindowProps {
        title: "My App".into(),
        borderless: true,      // Remove native decorations
        transparent: true,     // Enable transparency
        resize_inset: Some(12.0),  // Enable resize handles (matches CSS margin)
        ..Default::default()
    };

    App::new(app).window_props(window_props).run();
}
```

### Resize Handles for Borderless Windows

Borderless windows don't have native resize handles. Use `resize_inset` to enable custom resize handling:

- `resize_inset: Some(f32)` - Enables resize handles within **`inset`** px of the edges. The shell defaults it to `Some(8.0)` for any borderless resizable window.
- `resize_inset: None` (default before that fill-in) - Disables custom resize handles
- Only active when `borderless: true` AND `resizable: true`
- The cursor automatically changes to indicate resize direction on hover
- A **corner** is simply where two edge zones meet — an `inset` x `inset` square. It is *not* enlarged. `detect_resize_edge` used to carry a second `inset * 2` radius, but every guard reading it was implied by the `near_*` its arm already required, so the enlargement never took effect in any form the code has had; it has been removed rather than documented (#423). An app that wants bigger handles raises `resize_inset`.
- On Windows, transparent areas don't receive mouse events, so resize detection relies on the zone reaching into visible content

The `resize_inset` value should match your CSS content margin/padding to align the resize handles with the visible window edge. `BorderlessWindow` does **not** do this — its root is `100vw` x `100vh` with no margin — so on that component the zone genuinely overlaps your content, and the rule below is what keeps it usable.

**A visible scrollbar thumb wins an edge press (#399, #420).** Because the zone
overlaps content, the overlay scrollbar of a container flush with the window
edge is painted *entirely inside* it: at the default 8px inset the East zone is
`x > width - 8` and the thumb is drawn in `[width - 8, width - 2)`, so every
pixel of the thumb you could see used to be a resize handle. The rule now is
**what you can see, you can grab**, applied per axis. **Along** the track, a
press (and the hover cursor) at the thumb's extent goes to the scrollbar, while
the empty track past the thumb — and every edge with no bar on it — still
resizes. **Across** the bar the grab is edge-forgiving: the whole 16px hit
strip at thumb height goes to the scrollbar, *including* the 2px margin between
the thumb and the window edge — the way a browser's bar in a maximised window
is grabbable at the very last pixel. A **corner** never yields, so a diagonal
resize stays reachable however tall the thumb grows. The predicate is
`hit_testing::pointer_on_scrollbar_thumb`, which reads the thumb's along-axis
extent from the shared `rinch_dom::paint::scrollbar` geometry — the same
numbers paint draws with — over `find_scrollbar_hit`'s across-axis strip.

A browser has no equivalent conflict: its resize border lives in the window
frame *outside* the client area, so its scrollbars are grabbable right up to the
edge. A borderless window has no frame to put it in, so something has to give,
and the visible target is the one the user is aiming at.

**What true Windows transparency would require (see issue #89 — not yet wired):**
- DX12 backend with DirectComposition (`WGPU_DX12_PRESENTATION_SYSTEM=DxgiFromVisual`)
- `CompositeAlphaMode::PreMultiplied`
- `WS_EX_NOREDIRECTIONBITMAP` window style
- Patched wgpu for Rgba8Unorm storage textures (see wgpu fork below)

### BorderlessWindow Component

The `BorderlessWindow` component provides a complete container for borderless/transparent windows with:
- Rounded corners
- Custom titlebar with drag support
- Window control buttons (minimize, maximize, close)
- Optional left/right custom sections in the titlebar
- Proper theming via CSS variables

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    // For custom left section (e.g., menu button)
    let menu_signal = Signal::new(false);
    let left_section: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            ActionIcon { onclick: move || menu_signal.update(|v| *v = !*v) }
        }
    });

    rsx! {
        BorderlessWindow {
            title: "My App",
            radius: "md",  // none, xs, sm, md, lg, xl
            left_section: Some(left_section),
            on_minimize: || minimize_current_window(),
            on_maximize: || toggle_maximize_current_window(),
            on_close: || close_current_window(),

            // Content goes here
            div { "Hello, world!" }
        }
    }
}
```

**Props:**
| Prop | Type | Description |
|------|------|-------------|
| `title` | `String` | Window title displayed in titlebar (empty = not set) |
| `radius` | `String` | Corner radius: none, xs, sm, md, lg, xl (empty = default `md`) |
| `show_minimize` | `bool` | Show minimize button (default: true) |
| `show_maximize` | `bool` | Show maximize button (default: true) |
| `show_close` | `bool` | Show close button (default: true) |
| `left_section` | `Option<SectionRenderer>` | Custom content for left side of titlebar |
| `right_section` | `Option<SectionRenderer>` | Custom content before window controls |
| `on_minimize` | `Option<Callback>` | Callback for minimize button |
| `on_maximize` | `Option<Callback>` | Callback for maximize button |
| `on_close` | `Option<Callback>` | Callback for close button |

### Window Control Functions

For custom window chrome (minimize/maximize/close buttons):

```rust
use rinch::prelude::*;

// In event handlers:
button { onclick: || minimize_current_window(), "−" }
button { onclick: || toggle_maximize_current_window(), "□" }
button { onclick: || close_current_window(), "×" }

// Window visibility (for minimize-to-tray):
button { onclick: || hide_current_window(), "Hide to Tray" }
// From a tray menu callback:
MenuItem::new("Show").on_click(|| show_current_window())
```

These functions are available in the prelude and work from onclick handlers.

**`on_close_requested` callback:** Intercept the window close button to hide instead of exit:

```rust
use std::sync::Arc;

let window_props = WindowProps {
    on_close_requested: Some(Arc::new(|| {
        hide_current_window();
        false // Return false to cancel exit, true to proceed
    })),
    ..Default::default()
};
```

### wgpu Fork

Transparent windows require a patched wgpu to enable Rgba8Unorm storage textures for Vello rendering on DX12. The patches are in `[patch.crates-io]` in `Cargo.toml`:

- **Repository**: https://github.com/joeleaver/wgpu-fork
- **Branch**: `rinch-patch`
- **Upstream PR**: https://github.com/gfx-rs/wgpu/pull/8908

**The patch is one commit touching one file**: `wgpu-core/src/instance.rs`, forcing storage
capabilities for Rgba8Unorm/Bgra8Unorm (16 added lines). Checked against the revision `Cargo.lock`
actually pins — `54b7ce083ac9575b27884054ae74eb652cb541b3` — and against upstream PR #8908, which
carries the same single file. This list used to name `device/resource.rs` and `present.rs` too;
**neither is patched, in this fork or upstream**, so do not add them back.

**Downstream projects** must copy the `[patch.crates-io]` section from the workspace `Cargo.toml` into their own `Cargo.toml` for transparent windows to work on Windows. This is required because Cargo patches are not transitive — they only apply to the workspace that declares them.

## Game Engine Integration

Two complementary patterns for integrating with game engines and custom renderers:

### RenderSurface (Recommended)

Rinch owns the window. Your renderer submits frames into a `RenderSurface` component. Rinch handles layout, compositing, and event routing.

**Key types** (all re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RenderSurfaceHandle` | Main handle — `writer()`, `gpu_registrar()`, `set_event_handler()` |
| `RenderSurface` | Component — `RenderSurface { surface: Some(handle) }` |
| `SurfaceWriter` | Thread-safe CPU pixel submission (`Send + Sync + Clone`) |
| `GpuTextureRegistrar` | Thread-safe GPU texture registration (`Send + Sync + Clone`) |
| `SurfaceEvent` | Input events dispatched to surface handler |
| `create_render_surface()` | Factory function |

**Usage:**
```rust
let surface = create_render_surface();
surface.set_event_handler(|event| { /* handle mouse/keyboard */ });

let writer = surface.writer();
std::thread::spawn(move || {
    writer.submit_frame(&pixels, w, h); // CPU pixels
});

// Or for GPU textures:
let registrar = surface.gpu_registrar();
registrar.set_texture_source(wgpu_texture, wgpu_view, w, h);
registrar.notify_frame_ready();

rsx! { RenderSurface { surface: Some(surface), style: "flex: 1;" } }
```

**Sharing a high-capability GPU device (issue #57):** zero-copy compositing needs your texture on the *same* device rinch composites with (`gpu_handle()` → `device`/`queue`/**`adapter`**). By default that device is created with `Features::default()` / `Limits::default()`. To raise it:

| Entry point | Ownership | Use when |
|---|---|---|
| `App::new(component).gpu_config(RinchGpuConfig { required_features, required_limits })` | rinch creates the device (surface-compatible adapter) with your extra features/limits | You just need more capability — **recommended**, always presents correctly |
| `App::new(component).external_gpu(ExternalGpu { instance, adapter, device, queue })` | You create the whole stack; rinch makes only the surface, validates present-support, composites onto your device | You must keep your exact `DeviceDescriptor` |

Construct `wgpu` types from **`rinch::wgpu`** (rinch pins a patched fork — a separate `wgpu` dep won't type-match). Both are `#[cfg(feature = "gpu")]`, re-exported in the prelude. Example: `examples/gpu-device-config` (`RINCH_GPU_MODE=external` toggles the two modes).

**Web (canvas viewport, issue #91):** the **same** `RenderSurface` + `create_render_surface()` API works on `rinch-web`, but the model is inverted — the **browser** composites, so rinch only creates and manages a `<canvas>` "viewport hole"; the **app owns the GPU context**. rinch links **no wgpu** on web.

- `handle.canvas_element() -> Option<web_sys::HtmlCanvasElement>` (wasm only) — returns the `<canvas>` after mount (populated in a post-mount microtask; `None` before). Call `wgpu::Instance::create_surface(SurfaceTarget::Canvas(canvas))` on it to render via WebGPU/WebGL. (The lazy 2D-blit CPU path — `SurfaceWriter::submit_frame` — still works and is portable with desktop; it self-disables once you claim a GPU context.)
- **HiDPI out of the box:** `layout_size()` reports **physical** px (`CSS px × devicePixelRatio`, matching desktop), rinch sizes the canvas backing store to match, and `set_resize_callback(|w, h| …)` pushes the new physical size on every resize (ResizeObserver-driven) so you can reconfigure your wgpu surface.
- **Input** (pointer/wheel/keyboard incl. **`KeyUp`**/focus) over the canvas is delivered to `set_event_handler` — rinch does not swallow it. `set_render_callback` drives a `requestAnimationFrame` loop.
- **Teardown** is clean: the ResizeObserver and canvas listeners are removed when the component unmounts (no leaks).
- Example: `examples/webgpu-surface-web` (rinch DOM chrome + a WebGPU triangle; `trunk serve` over localhost so `navigator.gpu` is available). Mirrors `examples/game-embed` on desktop.

**Source files:**
- `crates/rinch/src/render_surface.rs` — All RenderSurface types and registry (incl. the wasm32 canvas path: `canvas_element`, `set_resize_callback`, `setup_canvas_events`, `setup_resize_observer`, `WebSurfaceCleanup`)
- `crates/rinch-web/src/event_delegation.rs` — document-level keyboard/focus routing into the surface (`KeyDown`/`KeyUp`/`TextInput`, focus-clear on outside click)
- `crates/rinch/src/shell/desktop.rs` — `GpuHandle`, `RinchGpuConfig`, `ExternalGpu`, `WgpuRenderer::new` device injection
- `crates/rinch/src/app_builder.rs` — the `App` builder (incl. `gpu_config` / `external_gpu`); `crates/rinch/src/shell/mod.rs` holds the deprecated shims

### Embed API

Your game owns the window and wgpu device. Rinch runs headless — you feed it events, it produces a Vello scene.

**Enable it — the module is feature-gated and the gate is not `desktop`.** `rinch::embed` is
`#[cfg(any(feature = "gpu", feature = "embed"))]` (`crates/rinch/src/lib.rs`), so on default
features every type below is a compile error rather than the documented behaviour. Use
`features = ["embed"]` for the headless case this section describes — it pulls in `rinch-dom`,
`parley`, `peniko`, `vello` and `wgpu` without the desktop shell — or nothing extra if you already
build with `"gpu"`.

**Key types** (all in `rinch::embed`, re-exported in prelude):

| Type | Purpose |
|------|---------|
| `RinchContext` | Main handle — `new()`, `update()`, `scene()`. Multiple contexts can coexist on one thread (#134): each holds its own `subscribe_signal_change` guard, and bounds signals / editor registrations / focus requests are scoped per document via `DomDocument::doc_key()`. Stores/contexts are namespaced per context with a thread-global fallback (#136): `create_store` inside a context lands in that context's namespace (cleared on drop), its effects/handlers resolve it first, and lookups fall back to stores created outside any context. |
| `RinchContextConfig` | Width, height, scale factor, optional theme |
| `RinchOverlayRenderer` | Convenience Vello-to-texture renderer |
| `GameViewport` | Component marking a transparent hole for game rendering. **Hittable by default** (#207): `wants_mouse` routes input by hit-testing the hole and walking up to `data-viewport`, so an unhittable hole makes the UI claim the mouse *everywhere*. `pointer-events: auto; background: transparent;` come from the UA stylesheet rule for `[data-viewport]` — not an inline style — so restyling the hole can't strip them, and a HUD root's inherited `pointer-events: none` can't reach it. Your own HUD controls under such a root still need `pointer-events: auto`. It also stamps **no** `data-viewport-ready`, and absence means ready, so its hole is unconditional (#186) — see the **Viewport holes** note under Rendering Backends. |
| `LayoutRect` | `{x, y, width, height}` in logical pixels |

**Typical game loop:**
```rust
let mut ctx = RinchContext::new(config, my_ui);
let mut overlay = RinchOverlayRenderer::new(&device, w, h, format);

loop {
    let actions = ctx.update(&events);
    game.render();
    let ui = overlay.render(&device, &queue, ctx.scene());
    composite(game_texture, ui);
}
```

**Source files:**
- `crates/rinch/src/embed.rs` — `RinchContext`, `RinchOverlayRenderer`, `GameViewport`
- `crates/rinch/src/app/mod.rs` — `viewport_rect()`; `app/focus.rs` — `has_focused_input()`, `has_focused_contenteditable()`

**Documentation:** `docs/src/guide/game-engine.md`

## Fine-Grained Reactive Rendering

Rinch uses fine-grained reactive rendering for surgical DOM updates. Instead of regenerating HTML on every signal change, reactive expressions become Effects that update specific DOM nodes.

### Architecture

```
Signal.set() → Effect runs → NodeHandle.set_text() → Minimal re-layout
```

**Key principle:** `app()` runs once to build the DOM. Effects handle all reactive updates surgically.

### Key Components

| Component | Location | Purpose |
|-----------|----------|---------|
| `NodeHandle` | `rinch-core/src/dom/` | Stable reference to a DOM node for surgical updates |
| `RenderScope` | `rinch-core/src/dom/` | Context for building DOM trees with effect tracking |
| `DomDocument` | `rinch-core/src/dom/` | Trait abstracting DOM mutation operations |
| `RinchDocument` | `rinch-dom/src/lib.rs` | DOM implementation using Taffy + Parley + Vello |
| `rsx!` | `rinch-macros/src/lib.rs` | Macro generating DOM construction code |

### Taking a node out: `remove()` vs `discard()`

**`NodeHandle::remove()` is a detach, on every backend** (issue #719). The node
and its whole subtree keep their identity: append the handle again and the
subtree comes back exactly as it was, and you may read, style or restructure it
while it is out. `replace_with()` leaves the node it displaced in the same
state. That post-condition is what a reactive branch re-showing a **captured**
handle rests on — `rsx!`'s `if cond { {panel} }` desugars to `show_dom` with a
branch closure returning that same `NodeHandle` every toggle, and `match` arms
and a memoised `for` row are the same shape.

**`NodeHandle::discard()` says the caller is finished with the subtree for
good.** Treat a discarded handle as dead — build a fresh node rather than
re-attaching one — because a backend is then free to drop its bookkeeping and
make every operation on it a silent no-op. `rinch-web` and the test
`MockDomDocument` both do, so re-attaching a discarded node fails `cargo test`
as well as a browser.

**The contract is one-sided on purpose.** A `discard()` is *at least* a
`remove()` and may be much more; you may not rely on it being less. On
`rinch-dom` today it is exactly a `remove()` — a discarded node still
re-inserts, still keeps its subtree and still takes writes — and that is #723,
not a promise. Writing code that depends on desktop's inertness is the #719
mistake one verb along: right on desktop, dead on web, with nothing in the app's
own tests to say so. The mock is what closes that.

**A discarded id is never re-issued** on either backend, so a stale discard
handle names nothing rather than somebody else: `rinch-web`'s counter is a
monotonic `fetch_add` with no free list, and `rinch-dom` frees nothing on this
route. That is a claim about **`discard` alone** — `rinch-dom` does free slab
keys through `set_inner_html` and pseudo-element pruning on restyle, and
`slab::Slab` recycles them (measured: `NodeId(4)` handed to a second node), so
#304's recycled-slot hazard is live on desktop today, independently of this API.

### Who picks the verb: **scope ownership**, not a judgement call

The four marker-based reactive helpers — `show_dom`, `match_dom`,
`reactive_component_dom`, and `for_each_dom_typed` / `virtual_list` rows —
cannot know whether a subtree will be wanted again, so they do not guess. Each
runs its user closure inside its own `RenderScope`, and **a node that scope
minted is the helper's to discard; a node the closure was handed is the
caller's, and is only detached**. That is the rule #141 PR4 gave signals and
effects (`RenderScope::created`), applied to nodes.

It falls out of that, with no special cases:

| shape | verb | why |
|---|---|---|
| `if open { p { "hi" } }` — fresh markup | `discard` | the branch built it; nothing can show it again |
| `if open { {panel} }` — a captured handle (the #654 shape) | `remove` | the closure was handed it; the next show puts it back |
| a `render_fn`, branch closure or `for` view that **memoises a subtree built outside it** | `remove` | same reason: the closure was handed the node, so it is the caller's |
| a nested `for`'s rows inside a discarded branch | reclaimed | the discard is recursive, and nothing outside minted them either |

Ownership is asked of the **content root only**. That is what makes the nested
case above right, and it costs one shape: a captured handle *inside*
branch-built markup (`if open { div { {panel} } }`) is inside the recursion and
goes with the wrapper. That is **#732**, it behaved the same way before #719,
and `reinsertion_tests::a_captured_handle_nested_inside_fresh_markup_is_still_lost`
pins it.

**One more shape is lost on web, and only `for` can reach it: #733.** A `view`
closure that builds *lazily through the row's own scope* and caches afterwards
owns its row by this rule, so the first removal discards it. A branch closure or
a `render_fn` can build its cached subtree outside itself and capture it; a `for`
view is only ever handed the row's scope, so lazy-build-then-cache is the only
way to write it there. Both were equally lost before #719 —
`rinch-web` pruned every removed subtree — and `branch_helper_transition_tests`
passes on both because `rinch-dom` reclaims nothing (#723), which is a fixed
point worth remembering when reading that file.

Every other release site says the verb outright, because it knows: the editor's
`ViewDesc` diff (popped children, kind-changed blocks, placeholder, selection
rects), `virtual_list`'s drained spacers, `Stepper`'s replaced default glyph,
and DevTools' rebuilt panels all `discard`.

**"Takes its whole subtree with it" is true of everything *in* a subtree, and a
node in none is never reached.** An `rsx!` component site mints a scratch
`<template>`, builds the site's children into it, and hands them to
`Component::render`, which re-parents the ones it adopts; the container is then
dead and attached to nothing, so no walk from a branch's content root can find
it. That is `rinch_core::dom::release_scratch_container`, called at the three
codegen sites and at `element.rs`'s `Element::Component` arm — one orphan per
component render otherwise, measured at **+99 over 198 toggles** of
`if open { Card {} }` in Chrome and on the mock alike. Anything still under the
container was not adopted and leaves with it, by the same ownership rule one
level down.

Two ordering facts that fall out and are easy to get wrong: the verb is chosen
**before** the branch scope is disposed, so a cleanup that re-parents a
scope-built node during disposal cannot rescue it; and the scratch container is
released **after** `Component::render`, so the children it adopted have been
re-parented out by then.

**Getting it wrong is silent either way**: `remove` where `discard` was meant
costs memory on web, `discard` where `remove` was meant costs the subtree. The
suite pins **both** directions for every helper — a re-show fixture and a
node-count-over-200-toggles growth fixture, on the host through the mock and in
real Chrome through `NODE_REGISTRY`. A round of PR #728 had only the re-show
half for `show_dom`/`match_dom` and leaked one subtree per toggle in a browser
with the board green.

This divergence is what #719 was. `rinch-web`'s `remove_node` used to prune both
maps, so on that backend alone the first hide retired the id and every later show
inserted nothing — reproduced in Chrome 150, silent, no error anywhere.
`crates/rinch-core/src/dom/mock.rs` is the host-runnable oracle for both verbs
(it retires a discard and keeps a remove), which is why a `remove`/`discard`
mistake now fails `cargo test` rather than only a browser.

### Usage

Components use `#[component]` and return a `NodeHandle`:

```rust
use rinch::prelude::*;

#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Closure syntax {|| ...} creates a reactive Effect
            p { "Count: " {|| count.get().to_string()} }

            // Reactive styles also use closures
            div {
                style: {|| format!("width: {}px", count.get() * 10)},
                "Progress bar"
            }

            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}
```

The closure syntax `{|| expr}` tells the macro to create an Effect that:
1. Runs the closure and renders the initial value
2. Tracks which signals are read inside the closure
3. Re-runs and updates only that DOM node when those signals change

Without the closure, expressions like `{count.get()}` are captured once at initial render and never update.

### How It Works

1. **Initial Render**: Component runs once, creating DOM nodes via `RenderScope`
2. **Effect Setup**: Dynamic expressions (`{|| expr}`) become Effects with NodeHandles
3. **Signal Changes**: Effects run and surgically update their target nodes
4. **Batched Updates**: Multiple updates are collected for efficient re-layout

**Execution order is a contract** (#154): effects observing the same signal run in **registration order** (the order their `Effect`/`Memo` was created), and the pending queue drains FIFO — so an effect registered *after* an `rsx!` tree sees the post-patch DOM in the same flush ("run me last"), and a signal written from inside an effect queues its observers *behind* the current flush rather than preempting it. Enforced by `BTreeSet<ObserverId>` subscriber sets (ids are monotonic and never reused, so ascending id *is* registration order) plus `pop_front` in `flush_effects`. Don't swap either for a `HashSet`/LIFO. See `docs/src/guide/reactivity.md#execution-order`.

**A node outside the document is not styled** (#651, #668). `set_attribute` /
`remove_attribute` / `set_style` record the node in `tree.style_roots`, and
`resolve_styles` **drops** an entry whose node is not connected to the document
before cascading anything — "what is this element's style?" is a question CSS
answers only for elements in a document. It used to answer anyway, against no
parent at all, which is a cascade in which every inherited property lands on its
*initial* value and no descendant selector can match:

- **Unmount.** A removed subtree recascaded to `font-family: serif` / `color:
  black`, and `build_ifc_layouts` — which collects its roots from the whole node
  slab rather than by walking the document (#628) — reshaped its text in serif
  from there. That is where #654's serif came from.
- **Mount.** A component classes a child before splicing it in, so anything that
  resolves in that window cascaded the child parentless *and* set its
  `has_been_styled` flag. Its first real resolution then read as a **change** on
  an already-styled node, which is exactly what `transition` waits for: every
  mount of a component sized by a modifier class on its wrapper (`Checkbox`,
  `Switch`, `Select` all are) animated to its own size. A browser never animates
  there, because an element enters the document already carrying its final style.

Two consequences worth knowing. **A detached node's `computed_style` is now
whatever it last resolved to *in* the document** — a never-attached node's is the
default `ComputedStyle` — rather than an invented parentless cascade. Every
reader keeps working. Paint and hit testing never see one, since both walk from
`tree.body_id`, and so do `query_selector` and an unscoped `dom_tree`; but
`dom_tree` takes a `root_id` and hands it straight to the serializer, so
**`dom_tree(root_id: <a detached id>)` does reach one** — and what it reports is
that last in-document style, where before this change it reported the parentless
cascade's `serif`. And connectivity is asked at **resolve** time, not where the
entry was pushed, so a node classed while detached and spliced in before the next
layout is still styled by that same entry. Entering the document is what styles a
node, through `recompute_node_styles_recursive`, which every insertion route ends
in (`append_child`, `insert_before`, `insert_child`, `replace_node`).

**A subtree that leaves the document loses its before-change style** (#699) —
the same rule as above, read from the other end. A node styled while it was
*connected*, then detached, used to keep `has_been_styled` **and** the
`computed_style` it had in the document; so if an ancestor's class changed while
it was out, its re-insertion resolved to a different value, the cascade read
old ≠ new on an already-styled node, and the box **animated in from a style the
user never saw**. `RinchDocument::detach_subtree_styles` now clears the flag and
cancels any running transition **and animation** for the **whole removed
subtree**. Cancelling the transition is not optional — `tick_transitions` walks
`tree.active_transitions`, not the document, so one left behind would go on
writing interpolated values straight through the re-insertion. Cancelling the
*animation* is a separate repair riding along: an animation has no declared
duration to expire, and the desktop shell keeps asking for frames while
`tree.active_animations` holds a running (not paused) animation, so a removed
`Loader` kept an app rendering forever.

**Five places in `dom_impl/dom_document_impl.rs` write `parent = None`; four
call the helper.** `remove_node` (every reactive removal funnels through
`NodeHandle::remove`), `remove_child` (plus `RenderScope`'s batched
`DomUpdate::RemoveChild`), `replace_node`'s displaced `old`, and —
least obviously — **`set_text_content` on an element with children**, which
orphans every one of them without freeing the slab, so a handle the app still
holds stays alive and styled. That fourth one was missed on the first pass, when
this paragraph said "the three routes"; it is the count to re-check against
`grep -n '\.parent = None'` if a sixth ever appears, because an unhooked one is
silent. The fifth is `set_inner_html`, safe by **destruction** rather than
reset — it calls `NodeTree::remove_subtree`, which frees the entries and drops
both animation maps with them.

Three things it deliberately does not do.

- **It does not clear `computed_style` or `text_layout`.** A detached node still
  reads back as it last did in the document (above), and the re-insertion's own
  staleness gates compare against that same style to decide whether to re-shape
  the text (#654, #661, #678). The flag is what the transition reads; the value
  is what everything else reads.
- **`display: none` is not a detach**, and does not need to be (**#703**). §3
  asks whether an element is being *rendered*, not whether it is in the
  document, so the cascade answers that half itself: before starting a
  transition it checks the node's display **before** the change as well as
  after, then walks its ancestors — `display` does not inherit, so a box under a
  hidden wrapper computes `display: block` and its own style says nothing. A
  change made while an element is hidden lands outright, so it is already at its
  new value when shown; an element that stops being rendered has its
  transitions cancelled, and so does everything under it. `visibility: hidden`
  is **rendered** and still transitions.
  **The rule this puts on the component library**: *an overlay that animates
  must stay rendered — animate `opacity`, `visibility` or `transform`, never
  toggle `display`* (**#751**). A `display` flip cannot transition on **either**
  backend, because a browser refuses it for the same reason, which is what
  `@starting-style` and `transition-behavior: allow-discrete` exist for.
  `Drawer` was the one component that had it backwards: its root toggled
  `display: none` while its panel transitioned `transform`, so the slide-in ran
  on desktop only until #703 and had never run on `rinch-web` at all. Its closed
  state is `visibility: hidden` now, like `Popover`'s — hidden, still rendered,
  and still out of paint, hit testing and the Tab order on both backends.
  `rinch/src/app/overlay_animation_audit_tests.rs` holds one fixture per overlay
  and fails the moment a transitioned property starts changing on a reveal pass;
  the *close* is deliberately instant on both backends, because animating it
  would need a transition on `visibility` and `TransitionProperty` has no
  variant for one (**#759**).
- **A `@keyframes` animation on a hidden element does not run either**
  (**#747**), and that one is not only a paint question: the desktop frame clock
  schedules another frame whenever `tree.active_animations` holds a running
  (not paused) animation (`app/event_dispatch.rs`), so a `Loader` in a
  `display: none` panel kept an app rendering at full rate with nothing on
  screen moving. css-animations-1 §3 is
  stricter than the transition rule it sits beside — an element that is not
  being rendered has no animation *effect* at all, and is shown again with a
  **new** animation from t=0 rather than the one it had. So the entries are
  dropped rather than parked (parking them would leave the frame clock running,
  which is the whole complaint), and three things happen in the cascade:
  nothing **starts** on a node that is not rendered, everything under a subtree
  that **stops** being rendered is dropped, and everything under one that
  **starts** being rendered again is started afresh. The last is not
  symmetry for its own sake — a node shown by an *ancestor* need not be
  re-cascaded at all, because `set_style` invalidates the one node it was
  written to, so without the walk a panel un-hidden that way would come back
  with its spinner permanently still. None of the three reads
  `transitions_enabled` (**#762**, below): the walk ran behind that flag when
  #747 landed, so a panel shown by an inline write on a cascade before the first
  layout completed kept its spinner still until something re-cascaded it.
- **`visibility: hidden` runs an animation, and that costs a frame clock.** It
  is the same answer the transition rule gives — such a box is rendered — and it
  is what a browser does, measured in Chrome. But an animation has no duration to
  expire, so the cost is not a one-off: after #751 made the closed `Drawer`
  `visibility: hidden`, a `Loader` inside a **closed** drawer keeps animating and
  keeps the app rendering. Measured on the software backend at 804x600, closed
  drawer, 20 idle frames: **20/20 asked for a redraw at 2.53ms per tick+paint**,
  where the `display: none` spelling asked for 0. That is accepted, not
  overlooked — the two rules cannot disagree without desktop diverging from the
  web — and the cure belongs to the component: `animation-play-state: paused`
  on a closed overlay's subtree. **That cure works since #763.** A paused
  animation keeps its `ActiveAnimation` entry (its frozen sample is still
  written into `computed_style` on every cascade), but `tick_animations`
  neither counts it nor marks its node dirty, and the `AboutToWait` guard asks
  `NodeTree::has_running_animations()` rather than whether the map is empty —
  so it schedules no frame, on desktop or through the Android loop. Resuming
  continues from the frozen time. A paused *typography* animation is measured
  **by each cascade that writes its sample, and never per tick** — and that is
  not free, because a cascade of such a node now always re-measures and re-runs
  Taffy, whether or not the sample moved. Measured, release, 500 rows: a
  one-row colour-only hover goes **0.143 → 0.679ms** when that row carries a
  paused `font-size` animation (one extra Taffy compute per hover), and a
  whole-document colour-only restyle **16.1 → 45.1ms** with all 500 rows
  animated. It is bounded by "nodes carrying a text-measure animation", which
  is rare, and `main` paid a compute *every frame* for the same node. The
  narrowing is available and **not done**: compare the node's old
  `computed_style` with the post-animation style instead of asking only whether
  an animation has a `font-size` stop. A finished
  `forwards`/`both` animation is the same shape (**#782**): the tick that
  finishes it writes the fill and dirties the node once
  (`ActiveAnimation::fill_settled`), and after that it is kept, re-applied and
  not counted. `Drawer`'s own closed rule does **not** declare the pause yet, so
  a `Loader` in a closed `Drawer` still keeps the app rendering unless the app
  pauses it. The three `Loader` variants animate three different elements, so
  the rule has to name all of them —
  `.rinch-drawer__root--hidden .rinch-loader__oval, .rinch-drawer__root--hidden .rinch-loader__bar, .rinch-drawer__root--hidden .rinch-loader__dot { animation-play-state: paused; }`
  (`app/paused_animation_frames_tests.rs` installs exactly that list and
  mounts the default oval).
- **A move is not a detach.** `append_child`, `insert_before` and `insert_child`
  unlink a node from its old parent with the same lines `remove_child` uses, but
  it is back in the document before the call returns — so a row that was
  mid-transition when a keyed `for` reordered the list goes on transitioning
  (`insert_after` is how a reorder moves rows). **Unless the destination is
  itself detached** (**#702**): a mounted node moved into a parent not connected
  to `tree.root_id` has left the document while keeping a parent, so it fails
  the `parent = None` test — and connectivity, not the parent field, is the
  question (#696). `detach_subtree_styles_if_moved_out` answers it at **four**
  sites: the three move verbs plus `replace_node`, which splices its incoming
  `new` into `old`'s parent. Two guards decide: the destination must differ from
  the old parent (a move within one container cannot change connectivity), and
  the child must already have a parent (a node created moments ago is not a
  move). So a **keyed reorder never walks** — counted on 500 rows, it enters the
  helper 499 times and walks 0 — and a reparenting move walks once each.
  **An `rsx!` component site does walk**, which is the non-obvious part and is
  not free of the reset either: the macro builds a site's children into a
  detached `<template>` and `Component::render` then adopts them into a root
  that is *also* still detached (#719), so each adoption is a move into a
  detached parent. Counted on 500 sites of 20 nodes: 500 entries, 500 walks, 500
  resets over 10,000 nodes. The reset is semantically a no-op there (a fresh node
  is already unstyled with empty transition maps) and the cost does not show
  above noise, but "building a tree pays nothing" is true only of nodes appended
  straight into their final parent.
  One narrow behaviour change comes with it: a **mounted** node round-tripped
  out through a detached parent and back in the same pass loses its running
  transitions and restarts its animations, where a browser — whose style recalc
  is batched to the end of the task — never observes the intermediate state. The
  component *re-render* path does not reach it (`reactive_component_dom` removes
  the old output first, so #699 has already reset it); handing a component a
  handle that is mounted elsewhere and still connected does.

**The reactive helpers reach all of that, and `NodeHandle::clear_animations` is
gone** (**#704**). It used to be called before `remove()` by `show_dom`,
`match_dom`, `for_each_dom_typed`'s `Remove` arm and `reclaim_displaced`, and
the component re-render effect, and it stamped a literal inline
`transition: none; animation: none` over the whole subtree and **never took it
off again** — so a branch hidden once could never transition again, for any
reason, which is what a reactive `if` returning a captured `NodeHandle` (the
#654 shape) does on its first hide. It also masked #699 completely: through one
of those helpers, removing that whole fix changed nothing, which is why its
fixtures drive the `DomDocument` API directly.

Deleting the method was safe because every one of the five call sites was
`clear_animations(); remove();`, and `NodeHandle::remove` is
`DomDocument::remove_node` — the first of the five routes listed above. The cheapest
evidence was already in the tree: `for_each_dom_typed`'s **`Changed` arm** never
called it, and had no defect. On `rinch-web` the browser cancels a removed
element's transitions itself and treats a re-insertion as a first style, so the
inline write there was redundant at best. (A CSS *animation* does restart on
re-insertion in a browser — that is spec behaviour, not something the stamp was
guarding.) **Do not add a `set_style` of any kind to a removal path**: the node
survives the removal, so anything written there is permanent.
`crates/rinch-dom/tests/branch_helper_transition_tests.rs` is the pin: seven
fixtures over four of the five sites, three of them on `show_dom`.
`reclaim_displaced` is the fifth and has none — a
`debug_assert!(clobbered.is_none())` sits one line above the call, so a debug
test panics on the assertion before it can get there.

It was not free either, which is why a deprecated no-op shim would have been the
wrong shape too: `set_style` re-merges the node's whole inline `style` string,
re-parses it into a Stylo declaration block and invalidates the node's inline
style, and `clear_animations` called it **twice per node** over the subtree.
Measured on a 500-row list unmounted one row at a time, best of 200 alternated
rounds in one release binary: 340us without the stamp, 5541us with it —
**16x**, about 3.5us per node against the ~11ns per node #699's reset costs
(`crates/rinch-dom/tests/detach_reset_bench.rs`, both harnesses `#[ignore]`d).

**Nothing transitions on load, and everything animates on load** (**#762**).
`NodeTree::transitions_enabled` is set at the *end* of the first
`resolve_layout`, so a freshly mounted tree cannot transition into existence —
and `recompute_all_styles_full` forces it off again for its own re-cascade, so a
theme change applies instantly instead of every element sliding from the old
palette to the new one. Both are rules about **transitions**. A `@keyframes`
animation has no before-change style to be wrong about: it plays its own
timeline, and a browser runs one on the very first frame the element exists. The
two questions used to be asked with one `if`, and the cost was two bugs — an
animation present in the first frame never started at all (an embedded
`RinchContext` at a fixed size, where no later event re-cascades the tree, kept
a dead spinner for good), and a theme change stopped **every** animation in the
document permanently, so toggling dark mode killed every `Loader`, `Skeleton`
and `Progress` stripe in the app. The animation half of
`apply_stylo_styles_to_taffy` now reads no flag at all, and
`recompute_all_styles_full` no longer clears `tree.active_animations`: the
re-cascade matches each running animation by name and **keeps its clock**
(`start_time_ms`, `paused_elapsed_ms`, `play_state`). Measured in Chrome, replacing
a `<style>` element's text under a running animation leaves `currentTime`
untouched — so preserving is right and restarting would be the smaller wrong
answer. **The clock is all it keeps on that pass.** While
`recompute_all_styles_full` runs, `NodeTree::refreshing_animations` (a flag of
its own, not `!transitions_enabled`) makes a kept animation look its
`@keyframes` rule up again — **dropping it if the new sheet no longer defines
it**, which Chrome does and which keeping the entry whole got wrong, since a
theme sheet can carry `@keyframes` — re-extract its stops from the new base
style, and take the new duration and delay. A paused animation keeps its frozen
elapsed *time* across a re-timing, as Chrome keeps `currentTime`. That is
`recompute_all_styles_full` **only**, and it is not the only pass that
re-cascades the whole document: a `<style>` append (`maybe_load_style_css`) and
a viewport change in `resolve_layout` do too, and set no flag — measured, a
`<style>` appended after the first layout with a redefined `@keyframes` body
leaves the running animation on its old one (tracked on **#781**). On those two
passes and on every targeted restyle, an edited `@keyframes` body (**#766**), a
changed duration or delay (**#780**) and a stop derived from the base style
(**#781**) still stay stale, because a refresh per animated node per cascade is
a cost a hover should not pay. And one divergence the theme path now reaches,
not fixed: **a finished one-shot animation replays on a theme toggle** — its
entry was dropped when it completed, so the full restyle mints it again from
t=0. Chrome does not, and neither did `main` (its map was cleared with the
animation block gated off, right by accident). That is **#783**'s mechanism.
#747's restart walk — the one that gives a subtree shown by an inline `display`
write its animations back — reads no flag either; it shipped behind this one, so
a panel shown by such a write on a cascade before the first layout completed
kept a still spinner until something unrelated re-cascaded it. The full restyle
reaches the walk as well, and that half is **not** harmless without the refresh:
the walk runs on a shown panel's cascade before its descendants' and mints their
entries from their pre-restyle styles, so an `em`-sized spinner under a theme
that also changed the font-size came out on the old basis. Each descendant's own
cascade runs after the walk on that pass, and the refresh there re-extracts the
stops.
`crates/rinch-dom/tests/animation_start_gating_tests.rs` and
`full_restyle_animation_refresh_tests.rs` are the pins, with the Chrome
measurements and mutant-by-fixture tables in their module docs.

### Native Control Flow (if / for / match)

The `rsx!` macro supports native Rust control flow. All control flow is **always reactive** — conditions, iterators, and scrutinees are automatically wrapped in closures and tracked by Effects.

**`if` / `else` / `if let`:**
```rust
let visible = Signal::new(true);
let user = Signal::new(Some("Alice".to_string()));

rsx! {
    div {
        // if/else
        if visible.get() {
            p { "Visible!" }
        } else {
            p { "Hidden" }
        }

        // if let
        if let Some(name) = user.get() {
            p { "Hello, " {name} "!" }
        }
    }
}
```

**`for` loops with keyed reconciliation:**
```rust
let todos = Signal::new(vec![
    Todo { id: 1, name: "Buy groceries".into() },
    Todo { id: 2, name: "Write code".into() },
]);

rsx! {
    div {
        for todo in todos.get() {
            div { key: todo.id, {todo.name.clone()} }
        }
    }
}
```

The `key:` prop enables efficient keyed reconciliation. Items with matching keys are preserved (not re-rendered). If no `key:` is provided, items are keyed by `Debug` formatting.

**Keys must be unique within one list**, and what a repeat means depends on who chose the key (issue #185):

- **You wrote `key:`** — a repeat is a mistake in your key. The repeat is **not rendered** and a warning is logged; the first occurrence wins, as in React.
- **No `key:`** — the framework fabricated the key from `format!("{:?}", item)`, so a repeated *value* is not your mistake. The fabricated key is made unique by its occurrence ordinal instead, and **every row renders**: `for tag in ["rust", "rust", "gui"]` renders three. Reordering still moves rows rather than rebuilding them, because the ordinal follows the value, not the position.

An index key (`key: i`) is a last resort: it makes identity follow *position*, so inserting anywhere but the end re-renders every row after the insertion point and loses per-row state — the exact failure `key:` exists to prevent.

**Important:** Items with matching keys are **not** re-rendered when the collection changes. Their existing DOM subtree is preserved as-is. For per-item reactivity, use Signals inside each item.

**`match` with multi-branch rendering:**
```rust
let tab = Signal::new(0);

rsx! {
    div {
        match tab.get() {
            0 => div { "Home" },
            1 => div { "About" },
            _ => div { "Not found" },
        }
    }
}
```

Pattern bindings and guards are supported — each arm re-evaluates the scrutinee to extract bound values.

**Runtime desugaring:** `if` → `show_dom()`, `for` → `for_each_dom_typed()`, `match` → `match_dom()`.

**A brace around control flow is transparent** (issue #221). `div { { match x { … } } }`
renders the same reactive `match_dom` as `div { match x { … } }`, and likewise for
`if` and `for`, at any depth — a braced arm body (`_ => { match y { … } }`) included.
It used to parse as a plain Rust expression and go through `IntoNode::into_node`,
which evaluates **once**: the branch that rendered on mount was the branch you kept,
with nothing at the call site to distinguish it from its unbraced twin.

Braced control flow whose bodies are **not** rsx cannot be made reactive — arms
written as nested `rsx! { … }`, or bodies that are plain calls — so it is now a
compile error naming both rewrites rather than a silent freeze: drop the braces
and write each body as rsx for reactive *markup*, or wrap the whole construct in
`{|| … }` for a reactive *value*. Everything else in braces is unchanged: an
expression (`{ count.to_string() }`), a reactive closure (`{|| count.get()}`) and
a call (`{ section(__scope) }`) never start with a control-flow keyword and are
never touched.

### For Loop Details

The `for` loop variable is **owned** (`T`, not `&T`), so you can capture it directly in `move` closures:

```rust
for todo in todos.get() {
    let id = todo.id;
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: move || todos.update(|t| t.retain(|t| t.id != id)),
            "Delete"
        }
    }
}
```

**Item type requirements:** `Clone + PartialEq + 'static`. The `PartialEq` bound enables selective re-rendering — when the list changes, surviving items (same key) are compared by value. Only items whose data actually changed are re-rendered.

**Keys must be unique** (issue #185). An item whose key repeats one already seen in the same pass is **not rendered** and a warning is logged — first occurrence wins, as in React. The reconcile rests on one key naming one item state and one mounted sibling; a repeat used to leave a row that rendered, swallowed clicks and never updated again. Note the no-`key:` fallback keys by `format!("{:?}", item)`, so two `Debug`-equal items collide: `for n in vec![1, 1, 2]` renders **two** rows, not three. `virtual_list` applies the same rule within a visible range.

When the list changes, `for` uses keyed reconciliation (LIS algorithm) to compute minimal DOM operations:
- **Insert**: New items are rendered and added at the correct position
- **Remove**: Deleted items have their DOM nodes removed
- **Move**: Reordered items are repositioned without re-rendering
- **Changed**: Surviving items with different data (via `PartialEq`) are re-rendered
- **Unchanged**: Items with matching keys and equal data keep their DOM nodes

**Per-item state in for bodies**: Per-item reactive state works inside `for` loop bodies. Each item gets its own isolated scope:

```rust
for todo in todos.get() {
    let editing = Signal::new(false);  // Per-item state
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: move || editing.update(|v| *v = !*v),
            {|| if editing.get() { "Done" } else { "Edit" }}
        }
    }
}
```

### Programmatic Conditional/List Rendering

For cases requiring explicit control, use the runtime functions directly:

- `show_dom()` — conditional rendering (equivalent to `if`/`else`)
- `for_each_dom_typed()` — keyed list rendering (equivalent to `for`)

### Reactive Component Bindings

Some components support reactive value binding via `_fn` props. The `rsx!` macro auto-wraps `_fn` props — just pass a closure:

| Component | Prop | Type | Purpose |
|--------|------|------|---------|
| `Checkbox` | `checked_fn` | `Option<ReactiveBool>` (`Rc<dyn Fn() -> bool>`) | Reactive checked state |
| `TextInput` | `value_fn` | `Option<ReactiveString>` (`Rc<dyn Fn() -> String>`) | Reactive value binding |

**Controlled Input Pattern:** For controlled inputs, use `value_fn` + `oninput` together. `value_fn` keeps the DOM in sync with your signal; `oninput` updates the signal from user input. Without `value_fn`, programmatic `signal.set("")` won't clear the input visually.

A `value_fn` write (any `set_attribute("value")`) to the **focused** field is adopted by the field on both backends (issue #238): it becomes the text the next keystroke edits, the caret keeps its logical position (kept prefix/suffix keeps it, a same-length rewrite leaves it in place, a resized rewrite puts it after the new text), a selection survives, the write is deferred during an IME composition, and it never commits `onchange` by itself. A normalizing or rejecting `oninput` (uppercase, digits-only, max-length) that writes back on every keystroke therefore just works — on desktop, where it used to snap back to the pre-rewrite text on the next key, as well as on the web, where it used to throw the caret to the end.

**`onsubmit`:** TextInput supports `onsubmit` which fires when the user presses Enter.

Example - controlled TextInput with submit:
```rust
let input_text = Signal::new(String::new());

rsx! {
    TextInput {
        placeholder: "Type here...",
        value_fn: move || input_text.get(),  // Macro auto-wraps in Some(Rc::new(...))
        oninput: move |value: String| input_text.set(value),
        onsubmit: move || {
            println!("Submitted: {}", input_text.get());
            input_text.set(String::new());  // Clears the input thanks to value_fn
        },
    }
}
```

### RSX Prop Transformation Rules (IMPORTANT - Read Before Using Components)

The `rsx!` macro **automatically wraps** component prop values. You must NOT manually wrap them or you'll get confusing type errors from double-wrapping.

| Prop pattern | What you write | What the macro generates |
|---|---|---|
| `oninput` (closure) | `oninput: move \|val\| do_thing(val)` | `(InputCallback::new(move \|val\| do_thing(val))).into()` |
| `oninput` (value) | `oninput: my_callback` | `(my_callback).into()` |
| `on*` (closure) | `onclick: move \|\| do_thing()` | `(Callback::new(move \|\| do_thing())).into()` |
| `on*` (value) | `onclick: my_callback` | `(my_callback).into()` |
| `*_fn` (reactive) | `value_fn: move \|\| text.get()` | `Some(Rc::new(move \|\| text.get()))` |
| `icon`, `*_icon` | `icon: TablerIcon::Check` | `Some(TablerIcon::Check)` |
| bool literal | `disabled: true` | `true` (no wrapping) |
| int literal | `size: 42` | `Some(42)` |
| float literal | `value: 30.0` | `Some(30.0)` |
| string literal | `variant: "filled"` | `String::from("filled")` |
| `Some(...)` or `None` | `tree: Some(state)` | `Some(state)` (pass-through, preserves unsizing coercion) |
| any other expr | `variant: my_var` | `(my_var).into()` (auto-wraps `T` → `Option<T>` via `From`) |

**Common mistakes (DO NOT do these):**

```rust
// WRONG - don't manually wrap callbacks
TextInput { oninput: Some(InputCallback::new(move |val| ...)) }
// RIGHT - macro wraps closures automatically
TextInput { oninput: move |val: String| input_signal.set(val) }

// WRONG - don't manually wrap callbacks
Button { onclick: Some(Callback::new(|| ...)) }
// RIGHT - just pass the closure
Button { onclick: move || do_something() }

// RIGHT - you can also forward an existing Callback directly
Button { onclick: my_callback }

// WRONG - double-wraps into Some(Some(TablerIcon::Check))
Alert { icon: Some(TablerIcon::Check) }
// RIGHT - macro adds Some(...) for you
Alert { icon: TablerIcon::Check }

// WRONG - component expects String, not Option<String>
Button { variant: Some(String::from("filled")) }
// RIGHT - macro generates String::from("filled")
Button { variant: "filled" }
```

**Additional notes:**
- `on*` props accept both closures and existing `Callback` values. The macro uses `.into()` so the field can be either `Callback` or `Option<Callback>`.
- `Callback` and `InputCallback` have built-in defaults (no-op), so custom components can use `on_toggle: Callback` instead of `Option<Callback>`.
- Component text props (e.g., `variant`, `color`, `size`) are now `String` (not `Option<String>`). Empty string means "not set". The macro auto-converts string literals to `String::from(...)`.
- `_fn` suffix props (e.g., `checked_fn`, `value_fn`) are auto-wrapped — just pass the closure directly, don't wrap in `Some(Rc::new(...))`
- **Note:** ThemeProvider props (`primary_color_fn`, `dark_mode_fn`) use a different codegen path and still require manual `Rc::new()` wrapping

**Component Props vs HTML Attributes:**

- **HTML elements** (`div`, `span`, `p`, etc.) accept any attribute as a string: `style:`, `class:`, `id:`, custom `data-*`, etc. They also support reactive closures `{|| expr}` on any attribute. **An attribute *name* is ASCII case-insensitive in HTML content and case-SENSITIVE in SVG content** (issue #688), so `RinchDocument::set_attribute` stores it lowercased unless the element's tag is an SVG one: `<div ID="up">` is `id`, and `#up`, `[id]`, `.x` via `CLASS=` and `STYLE=`'s inline style all work, while `viewBox`, `preserveAspectRatio`, `gradientUnits`, `stdDeviation`, `markerWidth` and `startOffset` keep the author's spelling. The decision is keyed on the **tag**, not on an `<svg>` ancestor, and that is forced rather than chosen: `rsx!` and the `Element::Html` parser both write an element's attributes *before* appending it to its parent, so at the write there is no ancestor to walk and a walk would lowercase every SVG child's name. `crates/rinch-dom/src/attr_name.rs` holds the list and the four SVG names it deliberately omits — `a`, `script`, `style`, `title` are HTML element names too, and the HTML reading is the likely one. That is free for three of them, which carry no camelCase SVG attribute, and **not** free for `a`: SVG's `requiredExtensions` / `systemLanguage` apply to it, so those two fold on an `<a>` inside an `<svg>`. Inert today rather than harmless — nothing in the workspace reads either name and `paint/svg.rs` drops `<a>` through its `_ => {}` arm — but it is the one place the list trades correctness for the ambiguity, and worth weighing again if conditional processing lands. `get_attribute` and `remove_attribute` fold the same way (a fold on the write and not the remove could never turn a boolean attribute off again), and so does `is_boolean_attribute`, or a `CHECKED: {|| false}` would fall through to the literal writer as `checked="false"` — #551 under a different spelling. The **value** is never folded, so an `id` still matches case-sensitively outside quirks mode. `rinch-web` needs none of it: the browser folds the name itself, per namespace, because `web_document.rs` creates SVG elements with `createElementNS`. An **HTML boolean attribute** is the exception to "as a string": its *presence* is its value, so `rsx!` writes the bare presence form for a truthy value and **removes** the attribute for a falsey one, through `NodeHandle::write_attribute` rather than `set_attribute` (issue #551). Writing `disabled="false"` would leave the attribute present, which HTML — and therefore the browser, measured — reads as *disabled*; a reactive `disabled: {|| busy.get()}` was disabled from the first render and never recovered, on **both** backends. The set is `rinch_core::dom::is_boolean_attribute`: the 30 rows the WHATWG attributes index marks "Boolean attribute", plus `hidden` (enumerated, but its invalid-value default is the hidden state, so `hidden="false"` hides) and rinch's own `data-disabled` / `data-nofocus` / `data-trap-focus`. **The rule keys on the attribute name, not the value's type**, and that is load-bearing: `draggable` is enumerated (`"true"`/`"false"`, invalid → `auto`) and desktop's drag dispatch matches the literal `"true"`, the ARIA states are tri-valued, and `data-viewport-ready`'s *absence* means ready — presence-mapping any of them loses or inverts it. A *string* yielded into a boolean attribute follows `attr_is_truthy` (on unless `"false"` in any case, or `"0"`), which is the **writer's** rule and nobody's reader: every desktop reader of the HTML set is presence-only, correctly, because a browser is (`:checked` matches on `checked="false"`, `:disabled` on `disabled="false"`, both measured) — which is why #551 reproduced on desktop and why the cure is a writer that removes rather than a reader that learns a falsey string. **`set_attribute` is not that writer.** It is the literal primitive on both backends — including for the `checked` family, which is issue #612's divergence over again and what **#622** closed: the web arm used to presence-map `checked` / `selected` through `attr_is_truthy`, so `set_attribute("checked", "false")` unchecked the box on web and checked it on desktop. It now writes the string, leaving the attribute *present*, which checks the box on both. Writing a `bool` is `write_attribute`'s job; meaning "off" is `remove_attribute`'s. On web the live IDL property (`.checked`, `<option>.selected`) is mirrored from that **presence** and not from the string, because a browser stops mirroring the attribute onto the property once the user has toggled the control (the dirty-checkedness flag) and rinch has no such flag — desktop's reader sees the attribute and nothing else, so the app's write has to win on both. `indeterminate` is the exception and stays truthiness-mapped: HTML has no such content attribute at all, so it has no presence for anyone to read. **The same two names are the reason `write_attribute` cannot skip a falsey write** (issue #687): its removal is otherwise guarded on the attribute being present, since "already absent" reads as "already off" and costs no style invalidation — but a user's toggle leaves the property on and the attribute absent, so the guard skipped the one write that could have corrected it and the control stayed checked against a binding saying `false`. `checked` and `selected` (`rinch_core::dom::is_presence_reflected_attribute`) therefore go to the backend unconditionally; desktop pays nothing for the extra call, because `RinchDocument::remove_attribute` returns before invalidating anything when the node does not carry the attribute — what `removeAttribute` does in a browser. A `"false"` escape survives for exactly three attributes, rinch's own `data-disabled` / `data-nofocus` / `data-trap-focus`, spelled once as `rinch_core::dom::data_attr_is_on`; issue #612 retired it from `disabled` / `readonly`, where it had been desktop-only and so a pure divergence. It is a deliberate convention rather than a leftover, and `data-nofocus` / `data-trap-focus` are what show that — the web reads them the same way, through `[data-nofocus]:not([data-nofocus="false" i])` and `[data-trap-focus]:not([data-trap-focus="false" i])`, while `data-disabled` has no web reader at all. Note `data_attr_is_on` is deliberately *narrower* than `attr_is_truthy`: `"0"` is on, matching those selectors, and `"0"` is therefore the only value that distinguishes the two rules — which is why all three desktop readers pin it (`computed_style_tests::the_data_escape_excuses_only_false_at_the_reader`, `nofocus_tests::only_false_opts_out_not_zero`, `trap_focus_tests::the_false_escape_opts_out_and_zero_does_not`). **`oninput` and `onchange` on `<input>`/`<textarea>` elements** receive the input value as a `String` — use `Fn(String)` closures, not `Fn()`. They are **not aliases** (issue #226): `oninput` fires per keystroke with the live value; `onchange` fires once at the commit boundary — focus leaves the control after a modification, Enter (single-line inputs only; a `<textarea>` commits at blur), or a `<select>` pick — and only if the value actually changed since focus. On Enter, `onchange` fires before `onsubmit`:
  ```rust
  input {
      oninput: move |value: String| name_signal.set(value),
      onchange: move |value: String| autosave(value),
      placeholder: "Type here...",
  }
  ```
  **`onscroll`** takes `Fn(ScrollEvent)` (issue #177). The payload is a
  `#[non_exhaustive]` struct carrying **both** axes — `ev.scroll_top` and
  `ev.scroll_left` — so a horizontal-only scroller reports where it is instead
  of an unchanging zero. It fires once per container that moved, whichever axis
  moved it. Construct one with `ScrollEvent::new(top, left)`; a struct literal
  will not compile downstream. `ScrollEvent` is in the prelude.
- **Components** (`Button`, `TextInput`, `Stack`, etc.) accept their declared struct fields as props. Additionally, all components support these universal props:
  - `style:` — Applied to the component's root DOM element after rendering. Supports static strings and reactive closures. **Merged, not assigned** (issue #647): the declarations are laid over whatever is already on the root, last-wins per property, so every inline declaration the component's own `render` wrote survives — which is how `Modal`, `Drawer`, `Popover`, `DropdownMenu`, `Notification` and `LoadingOverlay` publish `z_index`, `offset`, `overlay_blur` and the rest as custom properties. A write of the whole attribute silently returned all five overlays to their stylesheet defaults. The same is true of `style:` on an HTML element, where the second author is usually a style shorthand: `div { style: {|| …}, p: "md" }` keeps its padding. An element with no second author keeps the author's string verbatim, on every fire and not just the first.

    A **reactive** `style:` takes its own previous declarations back on each re-run: one it no longer makes is removed, or restored to the value it had displaced. Two rules keep that from reaching past the caller's own work, and each is a bug that was there before they were written. It **overwrites a property it still declares where that property stands**, never removing and re-appending it — a declaration that moves to the end of the block changes which of two colliding declarations wins, and `div { style: {|| …}, mt: "8px" }` silently lost its top margin on the first signal change that way. And it **leaves alone a property whose current value is not the one it wrote**, because somebody else has written it since and reverting it would undo a value they never saw.

    What is **not** repaired, deliberately: a genuine same-property collision between `style:` and a style shorthand. `div { style: {|| …}, p: "12px" }` whose closure also names `padding` gets the shorthand at mount, because shorthands are applied last, and the closure's value from its first re-fire onward, because nothing re-asserts a shorthand after the fact. Declaring one property from two props on one element is the mistake; making shorthands reactive is separate work.

    **Every author splits and re-joins the attribute with one parser** — `rinch_core::dom::split_declarations`/`serialize_declarations` (issue #670) — so a `;` or `:` inside `url(…)` or a quoted string is part of that value for all of them, and a property declared twice collapses at the **last** declaration's position, as CSSOM does. Both are behaviour rather than spelling. **Property names are ASCII case-insensitive** and come out lowercased (issue #711): `COLOR` and `color` are one property, a `set_style("COLOR", …)` overwrites an existing `color` rather than declaring it twice, and the rule applies to the name a caller passes as well as the names the parser reads — `rinch_core::dom::normalize_property_name` is the one place it lives, and `rinch-web` needs none of it because the browser's own CSSOM lowercases. A **custom property** is compared exactly, so `--Foo` and `--foo` stay two variables. The collapse is also priority-aware: the surviving **value** is the `!important` one where the two disagree (`color: red !important; color: blue` stays red), while the **slot** is still the last declaration's — a pre-existing #670 residue that the case fold could not be correct without, since two spellings used to reach Stylo as two declarations and its own cascade picked the important one. The collapse is **syntactic** where a browser's is **post-validity** (issue #722), which is the one place it stops being CSSOM's rule: Chrome drops an invalid declaration as it parses, so the duplicate never competes, while rinch keeps it for Stylo to reject — `color: notacolor !important; color: blue` computes blue in Chrome and black here. Older than the priority arm (`color: blue; color: notacolor` always diverged), widened by it, and pinned by `inline_style_case_tests::a_duplicate_collapses_before_validity_not_after`, which flips when #722 is closed. What the attribute reads back as after a merge is the CSSOM-touched form, lowercased and collapsed; a browser reaches the same string one write later, when something first touches its CSSOM. `rinch-dom` kept its own `parse_style_string` until then: it split on a bare `;`, so a `background-image: url(data:image/png;base64,…)` already on a node was destroyed by the next unrelated `set_style` on it, and it collapsed at the *first* position, which computes `left: 25px` for `inset: 0px; left: 25px; inset: 4px` where Chrome computes `4px`. `MockDomDocument::set_style` merges the same way now (issue #666), so a component test rendered onto the mock is an oracle for inline-style composition. Two read-only scanners in crates that cannot reach `rinch-core` are deliberately left out and tracked in #705.
  - `class:` — Merged with the component's own CSS classes (additive, not replacing). Supports static strings and reactive closures. A **reactive** `class:` has the same memory the reactive `style:` above does: it takes off the words it last wrote before adding the new ones, so it never accumulates its own old values. It does not reach a class it has **never named** — but naming one is enough to own it: a word the closure emits is the closure's to remove, and on the next re-run that stops naming it the word comes off and nothing puts it back. That includes a class the component itself wrote, **base class included** — `class: {|| if p.get() { "rinch-checkbox marker" } else { "marker" }}` strips `rinch-checkbox` off the element for good (measured). Pre-existing #647 behaviour and the same rule the reactive `style:` follows, not something components can defend against.

    **The merge holds only if the component keeps its side of it** (issue #717). `class:` is applied to the handle `Component::render` *returned*, i.e. after the component has built its tree and registered its effects — so **a component effect must add and remove the one class it is responsible for** (`NodeHandle::add_class` / `remove_class`) and must never write `class` wholesale. Thirteen effects across ten components did, rebuilding the attribute from a base string captured during `render`, and so discarded the caller's class on their first run with no way to get it back short of a re-render: `Checkbox { class: "compact", checked_fn: … }` styled correctly until the user clicked it once. That also means an effect must not be the thing that establishes a node's *base* class — `Tree`'s row was written only by its selection effect, which is the same coupling seen from the other end. `add_class` is **idempotent**, as `DOMTokenList.add` is, which matters because an effect re-runs whenever anything it read changes and `Signal::set` notifies on every write, equal value or not (`set_if_changed` is the other method); without it three `set(true)` calls left three copies of the modifier.

```rust
// style: and class: work on all components
Button {
    variant: "filled",
    style: "margin-top: 8px",           // Static style
    class: "my-custom-button",          // Merged with component classes
    onclick: || do_something(),
    "Click me"
}

// Reactive closures also work on component style/class
Text {
    color: "dimmed",
    style: {|| if highlighted.get() { "background: yellow" } else { "" }},
    class: {|| if active.get() { "active" } else { "" }},
    "Dynamic styling"
}
```

**All component props support reactive closures** — except the `icon` / `*_icon` family, whose values the macro wraps as `Some(expr)` before the arm that would invoke it, so a reactive icon is a **compile error** naming `TablerIcon` (issue #718). Pass `{|| expr}` to any other component prop (`variant`, `color`, `size`, `disabled`, etc.) to make it reactive — when signals change, the component re-renders automatically:

```rust
let active = Signal::new(false);
Button {
    variant: {|| if active.get() { "filled" } else { "light" }},
    onclick: move || active.update(|v| *v = !*v),
    "Toggle"
}
```

For surgical updates without full component re-render, use `_fn` props where available (e.g., `checked_fn`, `value_fn`).

| Prop Type | Accepts `{|| ...}` | Update Strategy |
|---|---|---|
| HTML element attributes | Yes | Surgical DOM update |
| Component `style:`/`class:` | Yes | Surgical DOM update |
| Component `_fn` props | Yes (auto-wrapped) | Surgical DOM update |
| Component props (all others) | Yes | Full component re-render |

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

### Common Issues

- **Text wrapping/clipping**: Check that layout measurement and paint use the same font stack
- **Elements stacking wrong**: Check the `display` property. `div` and other block elements default to `display: block` (per the UA stylesheet in `crates/rinch-dom/src/dom_impl/mod.rs`), so flex properties like `align-items` and `justify-content` do nothing until you set `display: flex` explicitly
- **Components sitting side by side where you expected a column**: `display: inline-flex` is **inline-level** (#595) — an *atomic inline*, like `inline-block`: it joins the line around it and shrink-wraps, and only its inside is a flex container. `Button`, `Badge`, `Checkbox`, `Switch`, `Avatar`, `ActionIcon`, `CloseButton`, `Loader`, `Pagination` and `Center` all declare it, so two of them in a plain `<div>` share a line, exactly as in a browser and as on rinch-web. Put them in a `Stack` (or any `display: flex` parent) to stack them — CSS blockifies a flex item, so the `inline-flex` is `flex` there and nothing about this applies. `display: inline-grid` is inline-level too, and has been since #607 — an atomic inline whose *inside* is a grid container rather than a flex one. `display: inline-block` is the third, and its **inside is a block container** (#592): its children stack, its inline runs get anonymous block boxes, and its own `font-size`/`font-weight`/`line-height` size its box. It laid its children out in a row until #592, because the Taffy container it built was a flex one
- **A heading suddenly got bigger and bold**: since #627 the UA stylesheet gives `<h1>`–`<h6>` the browser's typography (`2em`…`0.67em`, `font-weight: bold`, `em` block margins) and `<th>` `font-weight: bold` plus a centring, so a bare heading renders as a heading on desktop the way it always did on rinch-web. They are cascade rules, so any author `font-size`/`font-weight`/`margin` still wins — which is why `Title`, `Modal`, `Drawer` and the editor's own stylesheet are unmoved. `<th>` gets no `display`: rinch has no table formatting context, so `table` stays `display: block` and the cells stay `inline`. **The `<th>` centring is conditional and is spelled `text-align: -moz-center-or-inherit`, not `center`** — the HTML Standard's rule matches only a `th` "whose parent node's computed `text-align` is its initial value", so a header cell under an alignment its parent declares inherits that instead (measured in Chrome 150: `right` under a `text-align: right` ancestor). Stylo carries that value for exactly this rule and parses it for any **non-author** origin, so an app stylesheet cannot use it — the declaration is dropped there
- **A `<p>`, list or `<pre>` suddenly has space around it, or an `<hr>` appeared**: #674 finished the UA-stylesheet audit #627 started. `p`, `blockquote`, `figure`, `ul`, `ol`, `menu`, `dir` and `pre` now carry the browser's `margin-block: 1em`; `blockquote`/`figure` also `margin-inline: 40px` and `dd` `margin-inline-start: 40px`; a list nested in a list carries **none**, spelled `:is(ul, ol, menu, dir) :is(…)` as Chrome spells it (a *descendant* combinator — a `<ul>` under an `<li>` counts; `:is()` verified to match in this Stylo build, unlike `:has()`). `menu` and `dir` were in **no** rinch UA rule at all and so were `display: inline`; Chrome gives both exactly `ul`'s treatment, `padding-left: 40px` included. `<pre>` gets `white-space: pre`, so it finally preserves its newlines. `code`/`kbd`/`samp`/`pre` get `font-family: monospace` from the **UA sheet** rather than only from the theme. `small`/`sub`/`sup` get `font-size: smaller` (Chrome's 1.2 divisor — 13.3333px from a 16px parent, and it compounds). And `<hr>`, which rendered **nothing** before because the sheet's own `* { border-width: 0 }` reset applied to it, now carries `color: gray; border: 1px inset; margin-block: 0.5em; margin-inline: auto; height: 0; overflow: hidden` — a 2px grey rule. Every value measured in Chrome 150; all cascade rules, so any author declaration wins, which is why `Divider`, `List`, `Blockquote`, `Breadcrumbs`, `Tree` and `Image` are unmoved (they all declare `margin: 0`) — `Code` had to be given one. **The editor has one exception**: its stylesheet declares `margin`, `font-family` and `white-space` for every tag it renders but no `font-size` for `sub`/`sup`, so editor subscripts and superscripts now take `smaller` — correct, and what rinch-web always did, pinned by `the_editors_sub_and_sup_do_take_the_new_smaller_rule`. **Three consequences worth knowing.** The `<hr>` border is `currentcolor` over a UA `color: gray`, so `<hr style="color: red">` gives a red rule. A bare `<hr>` in a `Stack` collapses to a 2px dot and centres, because auto cross-axis margins suppress a flex item's stretch — that is what a browser does too (measured); use `Divider`, or `width: 100%`. And rinch does **not** reproduce Chrome's monospace font-*size* quirk (13px for a `medium` monospace element), so `<code>` keeps the inherited size. Still not done, and tracked separately: `vertical-align: sub`/`super` on `<sub>`/`<sup>` (#724 — `ComputedStyle` has no `vertical_align` field) and `display: list-item` markers on `<li>` (#725 — `DisplayValue` has no `ListItem`; desktop draws **no** list marker anywhere, `List`'s ignored `list-style-type` and the editor's bullet lists included — the editor's only marker is its task-list checkbox)
- **A stylesheet rule that matches nothing**: desktop's **selector** surface is narrower than a browser's and every gap is silent — the rule parses, then matches nothing, with no warning. `#id` works as of **#675**: `TElement::id()` now hands Stylo a stored, interned `Atom` (`Node::id_atom`, written by `Node::write_attribute` / `erase_attribute`), where it returned a hard `None` before, so `SelectorMap::get_all_matching_rules` never consulted the id bucket and `has_id` — correct all along — never ran **for a rule whose rightmost compound carries the id**. An ancestor-side id (`#a > p`, bucketed by `p`) always worked, which is the asymmetry that hid it. An UPPERCASE attribute name — `<div ID="up">`, `[DATA-X]` — works as of **#688**: the store folds the name in HTML content and Stylo already hands `attr_matches` a lowercased selector name, so both ends meet. Still silently dropped, measured in the same sweep: `:has()`, the `[attr=v i]` case-insensitive flag, camelCase SVG type selectors (`linearGradient`), presentational attributes (`<img width=100>`), and the whole `:required` / `:optional` / `:read-only` / `:read-write` / `:placeholder-shown` / `:indeterminate` / `:valid` / `:default` / `:defined` / `:target` / `:focus-within` / `:fullscreen` / `:lang()` family, which falls through a catch-all `_ => false` in `match_non_ts_pseudo_class`. `docs/src/guide/theming.md` has the measured table. All of them work on `rinch-web` (the browser matches), so a rule that works in the browser and not on desktop is probably one of these; a class selector is the spelling with no gap on either backend
- **`width: max-content` filled the container instead of shrink-wrapping**: rinch implements **no** intrinsic sizing keyword on a box's own size (#626). `max-content`, `min-content`, `fit-content`, `fit-content(<length-percentage>)`, `stretch` and `-webkit-fill-available` all parse — stylo's `static_prefs::pref!` is a *compile-time* macro in `stylo_static_prefs` that hard-codes those gates to `true`, and is unrelated to the runtime `stylo_config` store `RinchDocument::new` pokes — and are then laid out as `auto`, on `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height` and `flex-basis` alike. This is **not** a missing match arm. `taffy::Dimension` is a newtype over `CompactLength`, and while that type carries `MIN_CONTENT_TAG`/`MAX_CONTENT_TAG`/`FIT_CONTENT_*_TAG`, only the **grid track sizing** functions read them — `Dimension` implements `TaffyAuto` but not `TaffyMaxContent`, so there is no safe constructor; its resolver ends `_ => unreachable!()`, so a `size`/`min_size`/`max_size` carrying one **panics** in layout. So **`grid-template-columns: max-content` works** and a box's own `width: max-content` cannot, and implementing the latter needs a rinch-side measurement pass. What the substitution costs depends on the box, and the two halves are **mirror images** (measured, Chrome 150): the three intrinsic keywords are already correct wherever `auto` is content-sized — a block's `height`, an `inline-block`'s or a flex-row item's `width` — and wrong wherever `auto` fills — a block's `width`, any `min-width`/`max-width`, a flex-column or grid item's `width`; `stretch` is correct exactly where `auto` fills and wrong where `auto` is content-sized. The declaration is no longer *discarded*, only unimplemented: `DimensionValue::Intrinsic` carries it, so `get_computed_styles` reports what the author wrote, and style conversion prints one line per property and keyword per process instead of dropping it in silence. The measured table and the Taffy pin live in `crates/rinch-dom/tests/intrinsic_sizing_tests.rs`
- **Text not updating**: Verify signal/effect wiring in the component
- **No display (headless)**: Use Xvfb with `DISPLAY=:99` when running without a monitor
- **MCP tools not available**: Ensure `rinch-mcp-server` is built (`cargo build -p rinch-mcp-server`) and `.mcp.json` points to the binary

## Development Notes

- **ui-zoo-desktop** is the primary way to iterate on the framework
- The shell layer handles window management and event loop integration
- Menu callbacks are fully implemented and trigger re-renders automatically
- RSX macro provides helpful error messages with typo suggestions
- Transparent windows use an intermediate render texture (swapchain textures don't support STORAGE_BINDING)

## Documentation Requirements

**Always update user-facing documentation when adding or changing features:**

1. **User Guide** (`docs/src/guide/`): Update relevant guide pages when adding new user-facing features, APIs, or changing behavior
2. **API docs**: Ensure doc comments are added/updated for public APIs
3. **CLAUDE.md**: Update this file when adding new reactive primitives, element types, or architectural changes

Documentation locations:
- `docs/src/guide/hooks.md` - State management guide
- `docs/src/guide/menus.md` - Menu and shortcut guide
- `docs/src/guide/windows.md` - Window management
- `docs/src/guide/reactivity.md` - Signals, effects, memos
- `docs/src/guide/rsx-syntax.md` - RSX macro syntax
- `docs/src/guide/platform.md` - File dialogs, clipboard, system tray
- `docs/src/guide/game-engine.md` - Game engine integration (embed API)
- `docs/src/guide/theming.md` - Theme system and CSS variables
- `docs/src/guide/components.md` - Component library
- `docs/src/guide/contenteditable.md` - Using the rich-text editor (Editor component, EditorHandle, commands)
- `docs/src/guide/editor.md` - Rich-text editor internals (model, schema, steps, plugins, view)
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
