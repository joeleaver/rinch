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
```

```rust
// my-app-web/src/main.rs
use wasm_bindgen::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[wasm_bindgen(start)]
pub fn start() {
    // Logs and panic messages to the browser console (the same call logs to
    // stderr in the desktop build).
    rinch::init_logging();

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

## One entry point for desktop and web

The desktop's `App` builder runs in the browser too. `rinch_web::AppExt` adds
`run()` to it on `wasm32`, so one `main` serves both targets — the only
per-target line is the import:

```rust
use rinch::prelude::*;
#[cfg(target_arch = "wasm32")]
use rinch_web::AppExt as _;

fn main() {
    App::new(my_app::app)
        .title("My App")
        .size(800, 600)
        .theme(ThemeProviderProps { dark_mode: true, ..Default::default() })
        .menu(vec![("File", file_menu())])
        .run();
}
```

With Trunk, a binary crate's `fn main` is the wasm entry point, so no
`#[wasm_bindgen(start)]` is needed. In the browser `run()` mounts a whole-page
app (as `rinch_web::mount` does) and returns:

| Builder setting | In the browser |
|-----------------|----------------|
| `.theme(..)` | Installed, as by `mount` |
| `.menu(..)` | The app is mounted under the DOM menu bar, as by `mount_with_menu_bar` |
| `.title(..)` | Becomes `document.title`; without a title the page keeps its own `<title>` |
| `.image_scheme(..)` | Registered, as on the desktop |
| `.size(..)`, `.window_props(..)` | Ignored: the page sizes the browser window |
| `.fonts(..)` | Accepted and ignored: load web fonts with `@font-face` in the page's CSS |
| GPU and other desktop-only settings (`.gpu_config`, `.renderer`, …) | Not available: the browser composites |

On a web build `.menu(..)` returns an `App<F, WithMenuBar>` rather than the
same type, so an app that never calls it does not link the DOM menu bar (about
93 KB of wasm). A chain reads the same; only code that names the `App` type or
assigns `app = app.menu(..)` back to an `App<F>` variable sees the difference.

**Forgetting the import** gives:

```text
error[E0599]: no method named `run` found for struct `rinch::App` in the current scope
```

`App::run` is the desktop's inherent method, so the web build has none until
`rinch_web::AppExt` is in scope. The compiler suggests the trait only when the
crate already names `rinch_web` somewhere; otherwise it says nothing more. The
fix is the import above, behind `#[cfg(target_arch = "wasm32")]`.

For islands, or a menu bar on an island, use the `mount_*` functions below.

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

## A page with no rinch root

Every `mount*` also installs rinch's runtime for the page: the timer backend
that `set_timeout`, `set_interval` and `sleep` run on (`window.setTimeout`), the
scheduler that polls `spawn_local` tasks (a microtask each), and the drain that
samples `poll_signal`s. A page that runs rinch code but mounts no root — a game
that owns the canvas, or `rinch-ws` / `rinch-rtc` code with no UI of its own —
installs it with one call at startup:

```rust
#[wasm_bindgen(start)]
pub fn start() {
    rinch_web::install_runtime(); // or `rinch::install_web_runtime()` without rinch-web
    // ... set_timeout / spawn_local / rinch-ws from here on
}
```

It is idempotent, page-global and builds no DOM: no root, no event delegation,
no theme `<style>`. A later mount is unaffected. Without it a `set_timeout`
never fires (rinch logs `set_timeout: no timer backend installed`), and a
`spawn_local` task is never polled. Install it before the first timer: a
timeout armed earlier is lost, while a task spawned earlier is picked up.

An app on `rinch::embed::RinchContext` gets the timer and task halves from
`RinchContext::new` (see the [game-engine guide](game-engine.md#rinchcontext)),
so it needs the call only for timers armed before its first context.

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

## Right-click menus

`oncontextmenu` and `ContextMenu` behave as they do on the desktop: a right-click
on an element with a live handler dispatches it, and the browser's own menu does
not open. Everywhere else it does, which is what an island in somebody else's
page must do. A whole-page app that draws its own menus can take the rest of the
page as well:

```rust
rinch_web::set_suppress_native_context_menu(true);
```

`ContextMenu` needs this to look native: its dropdown is portalled out of the
element carrying the handler, so without the flag a right-click inside the open
menu opens the browser's on top of it. The flag is page-global and can be flipped
at any time.

**It leaves text fields alone.** With the flag on, a right-click on a
`<textarea>`, on an `<input>` of type `text`, `search`, `url`, `tel`, `email`,
`password` or `number` (or no `type`), or on `contenteditable` content still
opens the browser's menu, `readonly` and `disabled` fields included. That menu is
the user's cut, copy and paste, spelling suggestions and autofill, and a page
cannot rebuild it. A live handler still wins: to show your own menu in a field,
put `oncontextmenu` on the field or an ancestor of it. Outside `contenteditable` content, selected text, a checkbox, a `<select>` and a
label next to a field are still suppressed, and so is a `contenteditable="false"`
island. Inside editable content the browser reports a checkbox, a `<select>` or a
label as editable too, so they keep its menu there.

The rich-text `Editor` keeps the browser's editing menu too, flag or no flag
(issue #814). Its surface is not `contenteditable`, so a right-click on it is not
an editing context by itself — but a right-button press parks the hidden
`<textarea>` the editor takes its input through under the pointer for the instant
the browser needs to hit-test the point, so the `contextmenu` the browser fires
targets that textarea, which the carve-out covers (measured in Chrome 153: a real
right press dispatches `contextmenu` at the textarea, not prevented). The
keyboard's menu key and Shift+F10 go to the focused element, that same textarea;
the editor parks it at the caret on that key, so the `contextmenu` the browser
places at the focused element's box lands at the caret rather than at the
textarea's resting corner. Under an app's `data-oncontextmenu` the key opens that
handler instead, with its click context at the caret, and the browser's menu is
prevented. A `contextmenu` dispatched at the
surface itself with nothing parked — by script, or by a touch long-press (#821) —
is still suppressed with the flag on. So is a right-click on a link or an image in
the editor outside the selection: the editor parks nothing there, so that the
browser's own link or image menu can open, and a link or an image is not an editing
context, so the flag takes that menu away as it does on any other link or image.
What the menu's items do is described in [the editor guide](contenteditable.md).

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

## Browsers, and running the browser tests

The browser backend is tested in **Chrome and Firefox**: CI runs
`crates/rinch-web`'s browser tests (and `rinch-storage`'s IndexedDB tests) in
both, headless, on every pull request (`test-wasm` in
`.github/workflows/ci.yml`). Safari is not run anywhere.

The tests are ordinary `wasm-bindgen-test` files under `crates/rinch-web/tests/`.
`wasm-bindgen-test-runner` loads each in a real browser through that browser's
WebDriver, and which driver it finds decides the browser:

```bash
# Once: the runner, at the wasm-bindgen version Cargo.lock pins.
cargo install wasm-bindgen-cli --version "$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | grep -o '[0-9.]*')" --locked
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner

# Chrome: a chromedriver of the same version as the installed Chrome.
CHROMEDRIVER=/path/to/chromedriver \
  cargo test -p rinch-web --target wasm32-unknown-unknown

# Firefox: geckodriver (https://github.com/mozilla/geckodriver/releases), with
# `firefox` on PATH.
GECKODRIVER=/path/to/geckodriver \
  cargo test -p rinch-web --target wasm32-unknown-unknown

# One file, and the tests behind the `collaboration` feature:
GECKODRIVER=/path/to/geckodriver \
  cargo test -p rinch-web --target wasm32-unknown-unknown --features collaboration --test editor_read_only

# And the half of the editor's word-segmentation test behind `word-dictionary`:
GECKODRIVER=/path/to/geckodriver \
  cargo test -p rinch-web --target wasm32-unknown-unknown --features word-dictionary --test editor_word_dictionary
```

When both variables are set, `GECKODRIVER` wins. To use a Firefox that is not
on `PATH`, put a `webdriver.json` in `crates/rinch-web/` (do not commit it):

```json
{ "moz:firefoxOptions": { "binary": "/path/to/firefox", "args": ["-headless"] } }
```

Set `WASM_BINDGEN_TEST_TIMEOUT=180` (seconds) as CI does. The test runner
(wasm-bindgen 0.2.127 and later) gives each test binary that long in all, and
each poll of the page waits for its main thread: a binary that keeps the thread
busy past the default 20 s (`review_1449_caret_point` takes about 40 s in
Firefox) is killed with `webdriver POST …/execute/sync timed out`, which reads
like a hang and is not one.

Two things to know when a test passes in one engine only:

- **A synthetic `ClipboardEvent` does not carry its `clipboardData` in
  Firefox.** `new ClipboardEvent("paste", { clipboardData })` hands the
  listeners an empty transfer there. Build clipboard events with
  `tests/support/clipboard.rs`.
- **The engines answer geometry questions differently**, which is often why the
  code under test exists: after a preserved newline Chrome gives a collapsed
  range no rect and Firefox gives it the line above; a `<br>` on an empty line
  is the glyph box in Chrome and the line box in Firefox. A positive control
  that pins the browser's own answer states each engine's
  (`tests/support/engine.rs`, `is_gecko()`), never one engine's for both.

Two tests need input only the browser can make — a real Tab (focus moves) and a
real finger (a pan, and the compatibility mouse events after a tap):
`editor_node_view_real_tab.rs` and `real_touch_1582_1588.rs`. An ordinary run
reports them ignored. `tests/support/run_real_keys.sh` runs them in Chrome
(needs node >= 22), sending trusted keys and touches over the DevTools protocol;
CI runs it after the suite:

```bash
CHROMEDRIVER=/path/to/chromedriver crates/rinch-web/tests/support/run_real_keys.sh
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

**Browsers.** The wasm suites run in Chrome only. A press, drag or hover in the
`Editor` (and the `data-block-index` text hit) finds its text position with
`caretRangeFromPoint` where the browser has it and the standard
`caretPositionFromPoint` otherwise, which is what Firefox has. That second path is
tested by hiding the first in Chrome, where the two give the same positions; it has
not been run in Firefox, and neither has the rest of the editor's input there
(#1461).

**Touch.** A `data-rid` (`onclick`) runs on the press for a mouse, and for a
touch or pen press that cannot start a scroll. One that can — the page scrolls,
or a scroll container around the element does — runs on release, and not at all
if the finger pans (a move past the slop, or the browser's `pointercancel`): a
pan that starts on a button does not press it (#1582). An element with
`touch-action: none` on it or between it and that scroller cannot pan, so its
handler runs on the press, where it can arm a `Drag`: give a control that drags
`touch-action: none`, as `Slider`, `ColorPicker`'s areas and `FloatingPanel`'s
header and resize handle have. A `Drag` armed by a handler that ran on release
is ended at the release.

**Events dispatched by script.** rinch's document listeners (and a
`RenderSurface` canvas's) check an event's interface before reading it, so an
event dispatched by page script, a polyfill or an extension under a name rinch
listens for — `document.dispatchEvent(new Event("pointerup"))` — throws nothing
(#810, #811). A key, pointer or paste event of the wrong interface is ignored: a
plain `Event("pointerup")` does not end a live `Drag`, and a plain `Event("paste")`
does not reach `set_paste_interceptor`. `click` and `contextmenu` are the
exceptions: a plain `Event("click")` still activates the `data-rid` element it is
dispatched at, as untrusted clicks always have, and a plain `Event("contextmenu")`
still runs a live `data-oncontextmenu` handler; both are read as having no
position (the element's centre) and no modifier held.

## What Doesn't (Yet)

- **Custom painting** — Vello and tiny-skia don't run in the browser. The browser paints for you instead.
- **Native menus** — There is no OS menu bar to attach one to. Rinch renders its
  own DOM menu bar instead, from the same `Menu`/`MenuItem` declarations the
  desktop uses — see [Mounting with a menu bar](#mounting-with-a-menu-bar).
- **File dialogs** — Use the browser's `<input type="file">` or the File System Access API.
  The message dialogs (`rinch::dialogs::message(..).confirm()`) do work: they
  are `window.alert` / `window.confirm` (see [Platform Features](platform.md#message-dialogs)).
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

### Word segmentation for CJK and Thai (`word-dictionary`)

The rich-text editor's word motion (Ctrl+Left/Right), word delete and
double-click split text into words (#939). A run of Chinese, Japanese or Thai
text has no spaces between its words, and only ICU's segmentation dictionaries
can split it. The desktop build always has them (parley links them for line
breaking); a web build links them only when asked:

```toml
rinch-web = { version = "...", features = ["word-dictionary"] }
```

It is off by default because of its size. Measured on `examples/editor-web`
(release, `wasm32-unknown-unknown`, before `wasm-opt`): 4,440,944 bytes without
the feature, 8,282,831 with it (+3.84 MB). Without it a CJK or Thai run is one
word, ended by whitespace, punctuation or a change of script; with it
`漢字です|` + Ctrl+Backspace deletes `です`, as on desktop and in Chrome. The
`rinch` facade needs no feature for this: on wasm the editor comes from
`rinch-web`.

## Reference

See `examples/ui-zoo-web` in the repo for a complete, working WASM app with sidebar navigation, theme switching, and all components,
and `examples/menu-bar-web` for the menu bar.
