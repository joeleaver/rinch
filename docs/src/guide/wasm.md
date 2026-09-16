# Running on WASM

Rinch compiles to WebAssembly with a browser-native DOM backend. Instead of painting pixels to a canvas, the WASM build creates real `<div>`, `<span>`, and `<button>` elements. The browser handles layout, CSS, text rendering, and painting natively.

The result: ~3MB binary, no JavaScript framework, real DOM elements you can inspect in Chrome DevTools.

## How It Works

The magic is in the `DomDocument` trait. On desktop, `NodeHandle` talks to `RinchDocument` (Stylo + Taffy + Parley + Vello/tiny-skia). On WASM, it talks to `WebDocument` (web_sys). Your components, signals, effects, and stores don't know or care which one they're using.

```
Desktop: Signal -> Effect -> NodeHandle -> RinchDocument -> tiny-skia pixels
WASM:    Signal -> Effect -> NodeHandle -> WebDocument   -> real browser DOM
```

Same `rsx!`. Same `Signal::new()`. Same `Button { "Click me" }`. Different backend.

## Project Setup

The WASM entry point is its own crate (separate from the desktop workspace) so it can carry wasm-tuned build profiles and a Trunk setup. It depends on `rinch-web` — the browser-native DOM backend that implements `DomDocument` over `web_sys`. Structure your project like this:

```
my-app/              # Shared library: components, stores, logic
my-app-desktop/      # Desktop entry point
my-app-web/          # WASM entry point (separate workspace)
```

### The Shared Library

```toml
# my-app/Cargo.toml
[package]
name = "my-app"

[dependencies]
rinch = { git = "...", default-features = false, features = ["components", "theme"] }
```

```rust
// my-app/src/lib.rs
use rinch::prelude::*;

#[component]
pub fn app() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        Stack { gap: "md", p: "xl",
            Title { order: 1, "My App" }
            Button { onclick: move || count.update(|n| *n += 1),
                {|| format!("Clicked {} times", count.get())}
            }
        }
    }
}
```

### The Desktop Entry Point

```toml
# my-app-desktop/Cargo.toml
[dependencies]
rinch = { git = "...", features = ["desktop", "components", "theme"] }
my-app = { path = "../my-app" }
```

```rust
// my-app-desktop/src/main.rs
use rinch::prelude::*;

fn main() {
    App::new(my_app::app).title("My App").size(800, 600).run();
}
```

### The WASM Entry Point

```toml
# my-app-web/Cargo.toml
[package]
name = "my-app-web"

[dependencies]
rinch = { git = "...", default-features = false, features = ["components", "theme"] }
rinch-core = { git = "..." }
rinch-web = { git = "..." }   # browser-native DOM backend: WebDocument + mount
my-app = { path = "../my-app" }
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

    // `rinch_web::mount` creates the WebDocument, builds your component tree,
    // wires browser event delegation, and installs the theme CSS.
    let theme = ThemeProviderProps {
        dark_mode: false,
        ..Default::default()
    };
    rinch_web::mount(theme, my_app::app);
}

fn main() {}
```

## Islands (multiple roots per page)

`mount` boots a single whole-page app. For a **static-first** page (e.g.
server-rendered HTML) you can instead hydrate independent rinch widgets into
specific placeholder elements — the "islands" pattern. Each island is its own
reactive root; they coexist on one page and can be unmounted individually.

```html
<article>… static server-rendered content …</article>
<div id="comments"></div>
<div id="search"></div>
```

```rust
#[wasm_bindgen(start)]
pub fn start() {
    let theme = ThemeProviderProps::default();

    // Mount into an element selected by CSS selector …
    rinch_web::mount_selector("#comments", theme.clone(), comments_app);

    // … or into a `web_sys::Element` you already hold:
    let el = web_sys::window().unwrap().document().unwrap()
        .get_element_by_id("search").unwrap();
    let handle = rinch_web::mount_into(&el, theme, search_app);

    // Later, tear that root down (removes its DOM + handlers; the host stays):
    // handle.unmount();
}
```

| API | Purpose |
|-----|---------|
| `mount_into(&Element, theme, build) -> RootHandle` | Mount a root into an existing element |
| `mount_selector("#id", theme, build) -> Option<RootHandle>` | Same, by CSS selector (`None` if no match) |
| `RootHandle::unmount(self)` | Remove that root's DOM subtree and its event handlers |

Islands adopt their host element directly (no `#rinch-root`/`#rinch-body`
wrappers), share one page-global theme `<style>`, and install document event
listeners once — so any number can coexist without interfering. Dropping a
`RootHandle` keeps the root mounted; only `unmount()` tears it down. See
`examples/islands-web` for a runnable demo.

## Mounting with a menu bar

Browsers have no window menu to attach a native menu bar to. Rinch renders one
out of DOM nodes instead — the same bar the Linux desktop uses, built from the
same `Menu` / `MenuItem` values you would hand to `App::menu`. So the menus are
declared once and both targets render them.

```rust
use rinch_web::{Menu, MenuItem};

#[wasm_bindgen(start)]
pub fn start() {
    let file = Menu::new()
        .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(|| new_doc()))
        .separator()
        .submenu("Open Recent", Menu::new()
            .item(MenuItem::new("notes.md").on_click(|| open("notes.md"))))
        .item(MenuItem::new("Save").shortcut("Ctrl+S").enabled(false));

    let view = Menu::new()
        .item(MenuItem::new("Focus Search").shortcut("Ctrl+K").on_click(|| focus_search()));

    rinch_web::mount_with_menu_bar(
        ThemeProviderProps::default(),
        vec![("File", file), ("View", view)],
        my_app::app,
    );
}
```

| API | Purpose |
|-----|---------|
| `mount_with_menu_bar(theme, menus, build)` | Whole-page app under a menu bar |
| `mount_into_with_menu_bar(&Element, theme, menus, build) -> RootHandle` | An island under its own menu bar |
| `mount_selector_with_menu_bar("#id", theme, menus, build) -> Option<RootHandle>` | Same, by CSS selector |

Each `(label, menu)` pair becomes one top-level menu, in order. Clicking a label
opens it; moving the pointer across the bar switches to the next menu without a
second click; clicking outside or pressing Escape dismisses it. Separators and
submenus render as flyouts.

The bar is laid out *inside* whatever it is mounted into, above the content, and
the wrapper it builds is **viewport-tall** — `height: 100vh`, from the component
stylesheet. A whole-page app needs nothing for that; an island gets no say in it
through the API, so a host shorter than the viewport is overflowed by the bar and
giving that host a height changes nothing. Measured in Chrome 153: with no
height on `html`, `body` or the host, everything lands at the viewport's 437px;
with the host at `height: 120px`, the wrapper is still 437px. Tracked
separately. An author stylesheet can still override it — the component sheet's
rule is a bare class selector, so `div.rinch-app-menu-bar-wrapper { height: 100% }`
wins on specificity (measured: a 120px host then gets a 120px wrapper).

**Shortcuts are armed against the page.** Every item's `shortcut` string is
matched on a capture-phase `keydown` on `window`, before the app sees the key,
and a chord the menus claim is consumed: `preventDefault`, so the browser does
not also act on it, and the event is stopped before it reaches anything else in
the app — the same thing the desktop does by returning before a matched chord
becomes an event. `Ctrl` and `Cmd` are interchangeable in the string, as on the
desktop, so `"Ctrl+K"` is Cmd+K on a Mac. A chord nothing is listening to falls
through to the page: an item with a `shortcut` but no `on_click`, or a disabled
one, arms nothing.

**A few chords are the browser's own and cannot be taken.** The ones that open
and close windows and tabs — `Ctrl+N`, `Ctrl+T`, `Ctrl+W` and their `Shift`
variants in Chrome and Firefox — are handled by the browser's chrome ahead of the
page, so `preventDefault` does not reach them and the page may not be sent the
keystroke at all. Exactly which chords those are depends on the browser and the
platform, and rinch's `Ctrl`↔`Cmd` mapping shifts the question again on a Mac,
where `"Ctrl+Q"` means `Cmd+Q` and belongs to the OS. Declaring one is still
right when the same `Menu` drives a desktop build — `Ctrl+N` is what "New"
should be there — just do not rely on the chord in the browser, and check any
chord you care about in the browsers you support.

**Unmounting gives the chords back.** They are page-global, so an island mounted
into somebody else's page arms them against the whole document;
`RootHandle::unmount` releases exactly the ones that island armed. Two islands
that each declare menus still share one registry — the later one to arm wins, and
the earlier one unmounting leaves the winner alone.

Nothing about this needs the `desktop` feature. `rinch::menu`'s declaration types
and the DOM renderer build with `default-features = false`; only the `muda`
builders behind them are desktop-gated. See `examples/menu-bar-web` for a
runnable demo.

## Building

### With Trunk (Recommended)

[Trunk](https://trunkrs.dev/) handles WASM compilation, asset bundling, and dev server:

```bash
cargo install trunk

cd my-app-web
trunk serve --release --port 8080
```

Add an `index.html`:

```html
<!DOCTYPE html>
<html>
<head><meta charset="utf-8"></head>
<body></body>
</html>
```

Trunk does the rest.

### With wasm-pack

```bash
cd my-app-web
wasm-pack build --target web --release
```

### Manual

```bash
cd my-app-web
cargo build --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/my_app_web.wasm --out-dir pkg --target web
```

## What Works

Everything that goes through `NodeHandle` works:

- Signals, Effects, Memos, derived state
- Stores and Context
- All 60+ components
- The theme system and CSS variables
- Event handling (onclick, oninput, etc.)
- Reactive control flow (if, for, match)
- CSS shorthand props

The abstraction is clean. If your component code doesn't import anything from `rinch-dom` or `winit` directly, it'll work on WASM without changes.

## What Doesn't (Yet)

- **Custom painting** — Vello and tiny-skia don't run in the browser. The browser paints for you instead.
- **Game engine embedding** — `RenderSurface` and `RinchContext` are desktop-only.
- **Native menus** — There is no OS menu bar to attach one to. Rinch renders its
  own DOM menu bar instead, from the same `Menu`/`MenuItem` declarations the
  desktop uses — see [Mounting with a menu bar](#mounting-with-a-menu-bar).
- **File dialogs** — Use the browser's `<input type="file">` or the File System Access API.
- **System tray** — Not a thing in browsers.

## Theme in WASM

The theme system works by injecting a `<style>` tag into `<head>`. Dynamic theme changes (primary color, dark mode) work via reactive props:

```rust
let dark = Signal::new(false);

// ThemeProviderProps with reactive dark_mode
let theme = ThemeProviderProps {
    dark_mode_fn: Some(Rc::new(move || dark.get())),
    ..Default::default()
};
```

## Binary Size

A full UI Zoo demo (12 sections, 60+ components) compiles to ~3.3MB after `wasm-opt`. A minimal app is well under 500KB. The Rust compiler's dead code elimination is doing real work here — unused components, icons, and CSS don't make it into the binary.

## Reference

See `examples/ui-zoo-web` in the repo for a complete, working WASM app with sidebar navigation, theme switching, and all components,
and `examples/menu-bar-web` for the menu bar.
