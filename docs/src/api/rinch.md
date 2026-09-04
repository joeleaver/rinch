# rinch

The main rinch crate provides the application entry point, shell runtime, and re-exports commonly used types from `rinch-core`, `rinch-macros`, `rinch-theme`, and `rinch-components`.

## Entry Point

### `rinch::App`

`App` is the entry point. Every startup option is one method, and they all
compose — which the old `run_*` functions could not do (a menu bar and window
props needed two different functions, and neither could take anything added
later).

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div {
            h1 { "Hello, rinch!" }
            p { "A lightweight GUI framework for Rust." }
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
| `.title(title)` | Window title |
| `.size(width, height)` | Initial size, in logical pixels |
| `.theme(theme)` | `ThemeProviderProps` — colors, radius, dark mode |
| `.menu(menus)` | Native menu bar, as `Vec<(&str, Menu)>` |
| `.window_props(props)` | Full `WindowProps` — borderless, transparent, icon, `app_id`, … |
| `.gpu_config(cfg)` | Raise the compositor device's features/limits (`gpu` feature) |
| `.external_gpu(gpu)` | Composite onto an embedder-provided device (`gpu` feature) |
| `.run()` | Start on desktop; takes over the thread and does not return |
| `.run_android(android_app)` | Start on Android (`android` feature, Android target) |

`.title()` and `.size()` are applied **over** `.window_props()` whatever the call
order, so a title set explicitly is never silently lost to a later
`window_props`. Every other window field comes from `props`.

Nothing happens until a terminal method is called.

#### With a theme

```rust
use rinch::prelude::*;

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        ..Default::default()
    };
    App::new(app)
        .title("Themed App")
        .size(800, 600)
        .theme(theme)
        .run();
}
```

#### With full window configuration

```rust
use rinch::prelude::*;

fn main() {
    let props = WindowProps {
        title: "My App".into(),
        width: 1024,
        height: 768,
        borderless: true,
        transparent: true,
        ..Default::default()
    };
    App::new(app).window_props(props).run();
}
```

#### Everything at once

This combination is the reason `App` exists — no `run_*` signature could express
it:

```rust
App::new(app)
    .window_props(props)
    .theme(theme)
    .menu(vec![("File", file_menu), ("Edit", edit_menu)])
    .run();
```

### Deprecated `run_*` functions

`run`, `run_with_theme`, `run_with_menu`, `run_with_window_props`,
`run_with_window_props_and_menu`, `run_with_gpu_config` and
`run_with_external_device` still exist as thin shims over `App` and behave
exactly as before, but they are `#[deprecated]`. Each one's deprecation note
names the equivalent builder chain. The two Android entry points
(`run_android`, `run_android_with_theme`) are deprecated the same way, in favour
of `.run_android(android_app)`.

## Prelude

Import commonly used types with the prelude:

```rust
use rinch::prelude::*;
```

This includes:

**Entry points** (desktop feature):
- `App` — the builder; `run`, `run_with_theme` (deprecated shims)

**Element and prop types** (from `rinch_core::element::*`):
- `Element`, `Children`, `WindowProps`, `ThemeProviderProps`
- `Callback`, `SectionRenderer`

**Menu types** (from `rinch::menu`):
- `Menu`, `MenuItem` — unified builder API for native menus and tray menus

**Reactive primitives**:
- `Signal`, `Effect`, `Memo`, `Scope`
- `batch`, `derived`, `untracked`

**Hooks**:
- `Signal::new`, `Memo::new`, `Effect::new`
- `create_context`, `use_context`, `try_use_context`

**DOM construction**:
- `NodeHandle`, `RenderScope`, `with_render_scope`

**Component trait**:
- `Component`

**Control flow**:
- `show_dom`, `FineShowBuilder` (conditional rendering)
- `for_each_dom`, `ForItem`, `FineForBuilder` (list rendering)

**Event handling**:
- `ClickContext`, `InputCallback`, `get_click_context`, `start_drag`

**Icons**:
- `Icon`

**Macros**:
- `rsx!`, `#[component]`

**Window controls** (desktop feature):
- `close_current_window`, `minimize_current_window`, `toggle_maximize_current_window`

**Theme types** (theme feature):
- All types from `rinch_theme` (colors, spacing, radius, etc.)

**Component types** (components feature):
- All component structs from `rinch_components` (Button, TextInput, Stack, Group, etc.)

## Macros

```rust
pub use rinch_macros::rsx;        // RSX macro for DOM construction
pub use rinch_macros::component;  // #[component] attribute macro
```

## Re-exports

### Element Types

```rust
pub use rinch_core::element::{
    Children,
    Element,
    ThemeProviderProps,
    WindowProps,
};
```

### Reactive Primitives

```rust
pub use rinch_core::{
    batch,
    derived,
    untracked,
    Effect,
    Memo,
    Scope,
    Signal,
};
```

### Sub-crates

```rust
pub use rinch_core as core;
pub use rinch_renderer as renderer;  // desktop feature
```

## Modules

### `rinch::shell`

Application runtime and event loop. The entry point is [`rinch::App`](#rinchapp);
everything here is a deprecated shim over it:
- `run()`, `run_with_theme()`, `run_with_menu()` - title/size, plus theme or menu bar
- `run_with_window_props()`, `run_with_window_props_and_menu()` - full window props
- `run_with_gpu_config()`, `run_with_external_device()` - GPU device selection (`gpu` feature)
- `run_rinch()`, `run_rinch_with_window_props()` - lower-level runtime entry points

### `rinch::menu`

Unified menu builder API for native menus and tray context menus:
- `Menu` - Builder with `.item()`, `.separator()`, `.submenu()` methods
- `MenuItem` - Builder with `.shortcut()`, `.enabled()`, `.on_click()` methods

### `rinch::window`

Window utilities (currently minimal, window management is in shell).

### `rinch::windows`

Window control functions for custom window chrome:
- `close_current_window()`
- `minimize_current_window()`
- `toggle_maximize_current_window()`

### `rinch::fine_grained`

Fine-grained rendering types (re-exported from `rinch-core`):
- `NodeHandle`, `RenderScope`

### `rinch::theme` (theme feature)

Theme system types from `rinch-theme`.

### `rinch::components` (components feature)

Component library from `rinch-components`.

### `rinch::dialogs` (file-dialogs feature)

File dialog wrappers via `rfd`.

### `rinch::clipboard` (clipboard feature)

Clipboard operations.

### `rinch::tray` (system-tray feature)

System tray support.
