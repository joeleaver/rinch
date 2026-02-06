# rinch

The main rinch crate provides the application entry point, shell runtime, and re-exports commonly used types from `rinch-core`, `rinch-macros`, `rinch-theme`, and `rinch-widgets`.

## Entry Point

### `rinch::run`

Runs a rinch application with the given root component:

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
    run("My App", 800, 600, app);
}
```

### `rinch::run_with_theme`

Runs with a theme configuration:

```rust
use rinch::prelude::*;

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        ..Default::default()
    };
    run_with_theme("Themed App", 800, 600, app, theme);
}
```

### `rinch::run_with_window_props`

Runs with full window configuration:

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
    run_with_window_props(app, props, None);
}
```

## Prelude

Import commonly used types with the prelude:

```rust
use rinch::prelude::*;
```

This includes:

**Entry points** (desktop feature):
- `run`, `run_with_theme`

**Element and prop types** (from `rinch_core::element::*`):
- `Element`, `Children`, `WindowProps`, `MenuProps`, `MenuItemProps`, `ThemeProviderProps`
- `WidgetCallback`, `MenuItemCallback`, `SectionRenderer`

**Reactive primitives**:
- `Signal`, `Effect`, `Memo`, `Scope`
- `batch`, `derived`, `untracked`

**Hooks**:
- `use_signal`, `use_state`, `use_ref`
- `use_effect`, `use_effect_cleanup`, `use_mount`
- `use_memo`, `use_callback`, `use_derived`
- `use_context`, `create_context`
- `RefHandle`

**DOM construction**:
- `NodeHandle`, `RenderScope`, `with_render_scope`

**Widget trait**:
- `Widget`

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

**Widget types** (widgets feature):
- All widget structs from `rinch_widgets` (Button, TextInput, Stack, Group, etc.)

## Macros

```rust
pub use rinch_macros::rsx;        // RSX macro for DOM construction
pub use rinch_macros::component;  // #[component] attribute macro
```

## Re-exports

### Element Types

```rust
pub use rinch_core::element::{
    AppMenuProps,
    Children,
    Element,
    MenuItemProps,
    MenuProps,
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

Application runtime and event loop:
- `run()` - Entry point function
- `run_with_theme()` - Entry point with theme configuration
- `run_with_window_props()` - Entry point with full window props
- `run_rinch()`, `run_rinch_with_window_props()` - Lower-level runtime entry points

### `rinch::menu`

Menu management:
- `MenuManager` - Builds native menus from `MenuEntry` structs
- `MenuEntry` - Enum with `Item(MenuItemProps)` and `Separator` variants

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

### `rinch::widgets` (widgets feature)

Widget library from `rinch-widgets`.

### `rinch::dialogs` (file-dialogs feature)

File dialog wrappers via `rfd`.

### `rinch::clipboard` (clipboard feature)

Clipboard operations.

### `rinch::tray` (system-tray feature)

System tray support.
