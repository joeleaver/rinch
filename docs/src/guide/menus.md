# Menus

Rinch provides native menu support through the `muda` library. Menus use a unified builder API (`Menu` / `MenuItem`) shared between native window menus and system tray context menus.

The builder API itself needs no windowing at all. Two targets have no native menu bar to attach a menu to — Linux, where muda wants a GTK window, and the browser, where there is no window menu — so both render the same menus out of DOM nodes instead. On the desktop that is automatic; on the web it is `rinch_web::mount_with_menu_bar`, covered in [Running on WASM](./wasm.md#mounting-with-a-menu-bar).

## Native Menus

Add a native menu bar with `App::menu`:

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
        .item(MenuItem::new("Quit").on_click(|| std::process::exit(0)));

    App::new(app)
        .title("My App")
        .size(800, 600)
        .menu(vec![("File", file_menu)])
        .run();
}
```

For apps without menus, just leave `.menu(...)` off. `.menu()` composes with
every other builder method, so a menu bar and custom window props (borderless,
transparent, an icon) go on the same chain — which no `run_*` signature could
express.

## Menu Types

### Menu

A container for menu items, separators, and submenus:

```rust
use rinch::menu::{Menu, MenuItem};

let menu = Menu::new()
    .item(MenuItem::new("Open"))
    .separator()
    .submenu("Recent", Menu::new()
        .item(MenuItem::new("file1.txt"))
        .item(MenuItem::new("file2.txt"))
    );
```

### MenuItem

A clickable menu item with optional shortcut and callback:

```rust
use rinch::menu::MenuItem;

let item = MenuItem::new("Save")
    .shortcut("Ctrl+S")
    .enabled(true)
    .on_click(|| println!("Saving..."));
```

| Method | Type | Description |
|--------|------|-------------|
| `new(label)` | `impl Into<String>` | Create a new item |
| `shortcut(s)` | `impl Into<String>` | Keyboard shortcut |
| `enabled(e)` | `bool` | Whether the item is clickable |
| `on_click(cb)` | `impl Fn() + 'static` | Callback when activated |

## Menu Callbacks

Callbacks are `impl Fn() + 'static` — no `Send`/`Sync` required. They always run on the main thread, so you can safely capture `Signal`s:

```rust
use rinch::menu::MenuItem;

let count = Signal::new(0);

let item = MenuItem::new("Reset Counter")
    .on_click(move || {
        count.set(0);
        println!("Counter reset!");
    });
```

Callbacks fire both when the user clicks the menu item and when the keyboard shortcut is pressed.

### Callback lifetime

A callback belongs to the component that **created** it — the scope that was rendering when you called `on_click`, which is where the closure captured its `Signal`s. When that component unmounts its signals are freed, so the item stops firing rather than reading freed state (reading a freed signal panics). The callback also runs *inside* that component, so a `Signal` it creates belongs there too.

Ownership is per item, not per menu: one `Menu` may collect items contributed by several components, and each item's callback stops on its own component's unmount. This holds however the item is activated — a native menu click, a tray click, the DOM menu bar on Linux or on the web, or the keyboard shortcut.

Build the menu outside any component — from `main`, before `App::run`, which is what all the examples do — and there is no owner to record, so the callback lives for the life of the app:

```rust
fn main() {
    // Created in main(), so menu callbacks can reference them for the whole run.
    let count = Signal::new(0);

    let file_menu = Menu::new()
        .item(MenuItem::new("Reset").on_click(move || count.set(0)));

    App::new(app)
        .title("My App")
        .size(800, 600)
        .menu(vec![("File", file_menu)])
        .run();
}
```

A callback may rebuild the menu it was dispatched from — including registering new items and shortcuts — from inside its own handler.

Menu ids are also released when the menu that registered them goes away: building a new native menu bar releases the previous bar's, and dropping a `TrayIcon` releases that tray's after taking its icon off the panel. Keep the `TrayIcon` for as long as you want the tray. See [System Tray](platform.md#lifetime-and-threads) for why it has to stay on the thread that built it.

A shortcut consumes the keystroke only when a callback actually runs. A chord belonging to a disabled item, to an item given a `shortcut` but no `on_click`, or to a component that has since unmounted falls through to the app instead of being swallowed — and never shadows a live duplicate of the same chord.

**A shortcut with no Ctrl, Cmd or Alt leaves its key to a focused text field.** While an `<input>` that takes text, a `<textarea>`, the rich-text `Editor` (read-only included) or a custom text target (a `register_focus_target` entry with `on_ime`) holds the keyboard, a chord such as `"/"`, `"N"`, `"Shift+/"`, `"Space"` or `"Delete"` matches nothing: the field types or edits with the key, and the item fires once focus is elsewhere. The keys that yield are letters, digits, punctuation, `Space`, and `Enter`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown` and the arrows. `Escape`, `Tab` and `F1`–`F12` do not, so a bare `"F5"` item still fires from inside a field, and neither does any chord holding Ctrl, Cmd or Alt. A checkbox, radio, range or button `<input>` is not a text field. In the browser, "a text field" is the element the key event is dispatched at (`composedPath()[0]`, so a field inside a shadow root counts), and an element with `contenteditable` counts. The rule covers rinch's own chord matching only: on macOS and Windows the native menu also installs the item's accelerator, which the operating system matches before rinch sees the key (issue #1284).

## Submenus

Create nested menus using `Menu::submenu`:

```rust
use rinch::menu::{Menu, MenuItem};

let view_menu = Menu::new()
    .item(MenuItem::new("Zoom In").shortcut("Ctrl+="))
    .item(MenuItem::new("Zoom Out").shortcut("Ctrl+-"))
    .separator()
    .submenu("Appearance", Menu::new()
        .item(MenuItem::new("Light Theme"))
        .item(MenuItem::new("Dark Theme"))
    );
```

## Keyboard Shortcuts

Shortcuts are specified as strings combining modifiers and a key, separated by `+`.

### Modifiers

| Spelling | macOS | Windows / Linux |
|----------|-------|-----------------|
| `Ctrl`, `Cmd`, `Control`, `Meta`, `CmdOrCtrl`, `Command`, `Super`, `CommandOrControl` (also `CommandOrCtrl`, `CmdOrControl`) | one modifier: the native menu shows ⌘; the chord fires with Command **or** Control held | Ctrl; the chord also fires with the Windows/Super key held |
| `Alt`, `Option` | Option | Alt |
| `Shift` | Shift | Shift |

rinch has no way to bind Control and Command separately on macOS: `"Ctrl+K"` and `"Cmd+K"` are the same shortcut.

Modifiers come first and the key last, and a shortcut has exactly one key: `"Ctrl+Shift+C+A"`,
`"Ctrl+N+Shift"` and an unknown modifier such as `"Hyper+N"` are not shortcuts (they warn, below).

### Supported Keys

**Letters:** `A` through `Z`

**Numbers:** `0` through `9`

**Function keys:** `F1` through `F12`

**Special keys:**
- `Enter`, `Return`
- `Escape`, `Esc`
- `Backspace`
- `Tab`
- `Space`
- `Delete`, `Del`

**Navigation:**
- `Home`, `End`
- `PageUp`, `PageDown`
- `Up`, `Down`, `Left`, `Right` (arrow keys)

**Symbols:**
- `=`, `Equal`, `Plus`
- `-`, `Minus`
- `/`, `Slash` · `,`, `Comma` · `.`, `Period` · `;`, `Semicolon` · `'`, `Quote`
- `[`, `BracketLeft` · `]`, `BracketRight` · `\`, `Backslash` · `` ` ``, `Backquote`

**A shortcut names a key, not the character it types.** A character that needs
Shift is spelled as its key plus `Shift`: `"Ctrl+Shift+/"`, not `"Ctrl+?"`.
(Chrome 153 reports that keystroke as `key: "?"`, `code: "Slash"`,
`shiftKey: true`.) The same holds for `Plus`, which is the `=` key *without*
Shift. Punctuation and digit chords are always matched by the physical key and
the modifiers held — the desktop by winit's `KeyCode`, the browser by
`KeyboardEvent.code` — named by where it sits on a US layout; on another
layout such a chord follows the key position, not the printed character.

**A letter chord follows the character, not the physical position (issue
#1170).** `"Ctrl+Z"` fires from whichever key is *labelled* Z, wherever the
active keyboard layout put it — QWERTZ's Y/Z swap, AZERTY's A/Q swap — not
necessarily from the US physical `KeyZ` position. Desktop reads this from
winit's `key_without_modifiers` (ignoring Shift and Caps Lock); the browser
reads `KeyboardEvent.key`, lowercased. When the active layout types no Latin
letter at all for the pressed key — Cyrillic, Thai, Armenian, … — the chord
falls back to the physical key instead, which is the only way such a layout
can reach `Ctrl+C`/`Ctrl+V` at all (the same fix GNOME/GTK have discussed for
their own non-Latin-layout bug). Digit and punctuation chords are never
affected by any of this.

An app (or a user) who wants shortcut *positions* to stay fixed regardless of
the active layout can turn the character matching off:
`App::shortcut_matching(ShortcutMatching::Physical)` on desktop, or
`rinch_web::set_shortcut_matching(ShortcutMatching::Physical)` before mounting
on the web — the same escape hatch VS Code's `keyboard.dispatch` setting and
JetBrains' "use national layout for shortcuts" toggle offer. It affects only
rinch's own chord matching; see the note on macOS/Windows below.

**On macOS and Windows, a window menu bar's item stops going through rinch's
own chord matching once its native accelerator has actually attached to the
window** — the OS then resolves and matches the accelerator muda installs
for it (`NSMenu` on macOS, an accelerator table on Windows) ahead of rinch
ever seeing the key, so matching it a second time would risk firing the
callback twice. That "actually attached" is a recorded fact, not a guess
from which platform the build targets: a window whose handle was not a
Win32 one, whose accelerator table failed to install, or that never attaches
a native bar at all (a borderless window using the DOM bar instead) keeps
matching through rinch's own registry, so its shortcuts are never silently
unreachable. `ShortcutMatching::Physical` has no effect on an item once its
accelerator has attached: the OS, not rinch, decides what counts as "the Z
key" there, following its own per-layout accelerator resolution (character-
based on both platforms, like rinch's default). Linux has no such OS-level
menu-accelerator integration and never attaches one, so its native bar,
every system-tray item, and the DOM menu bar (the browser, and Linux's own
in-app bar) all go through rinch's chord matching — and the override —
exactly as described above. See issue #1284 for a related, unmeasured gap: a
*modifier-less* chord (`chord_yields_to_text_input` above) may still be
taken by a macOS/Windows native accelerator from a focused text field,
because that check runs inside rinch's own matching, which an attached item
skips.

A shortcut string that names no key (`"Ctrl+?"`, a typo) registers no chord and
logs one `tracing` warning per distinct string. The DOM menu bar (Linux, the
browser) still prints the string beside the item; the native menu on macOS and
Windows derives its accelerator from the same parse as the chord, so it shows
one exactly when a chord is armed.

### Examples

```rust
MenuItem::new("New").shortcut("Ctrl+N")
MenuItem::new("Save As").shortcut("Ctrl+Shift+S")
MenuItem::new("Exit").shortcut("Alt+F4")
MenuItem::new("Zoom In").shortcut("Ctrl+=")
MenuItem::new("Find Next").shortcut("F3")
```

Shortcuts work across platforms - `Cmd` and `Ctrl` are automatically mapped to the platform-appropriate modifier.

In a browser a few of them are the browser's own and cannot be claimed — see
[Running on WASM](./wasm.md#mounting-with-a-menu-bar). It changes nothing about the declaration; the desktop build still gets the chord.

## Platform Behavior

### macOS

On macOS, the menu appears in the system menu bar at the top of the screen, following Apple's Human Interface Guidelines.

### Windows

On Windows, the menu appears attached to the window's title bar.

### Linux

On Linux there is no native menu bar (muda needs a GTK window; winit uses raw
X11/Wayland), so rinch renders the menu bar itself as ordinary DOM inside your
window, 28px tall, from the same `Menu`/`MenuItem` API.

Because it lives in the document, it reserves its space with `padding-top` on a
wrapper around your content — **normal flow content clears it automatically**.

A `position: fixed` element does not: fixed resolves against the real viewport,
so a `top: 0` overlay slides underneath the bar. The bar publishes its height as
`--rinch-window-top-inset`, and full-height overlays should offset by it:

```rust
div { style: "position: fixed; top: var(--rinch-window-top-inset, 0px); bottom: 0;" }
```

`Drawer`, `Modal`, and top-anchored `Notification`s already handle this. See
[Theming → Window Chrome Inset](./theming.md#window-chrome-inset).

The bar behaves like a native one: click a top-level label to open its menu,
then move across the bar to switch between menus without clicking again. Hover
or click a submenu row to open its flyout, click an item to run it, and click
anywhere else in the window — or press Escape — to dismiss.

Escape rides the same dismiss stack every other rinch overlay uses, so it is
LIFO against `Modal`, `Drawer`, `Popover` and an open `<select>`: the bar
registers at mount, which puts every later overlay above it, and its handler
declines the key outright while no menu is open.

That last one is a full-window overlay rinch renders under the open menu, at
`z-index: 199` against the bar's `201`. It is `position: fixed`, so it covers
the window whichever of the three menu-bar layouts built it — including the
title bar, so clicking there dismisses too.

It was `position: absolute` between issues #527 and #324. Two `z-index`es in
different stacking contexts are never compared; `BorderlessWindow`'s container
was a stacking context purely because of its `overflow: hidden`; and a fixed box
was hoisted clear out to the body, past everything between. So the overlay's
`199` and the menu's `201` were never compared with each other, and the overlay
covered the very menu it sits beneath. Both halves are fixed: `overflow` no
longer creates a stacking context, and a fixed box now stops at its nearest
ancestor stacking context — so the `199` and the `201` meet in one sequence.

**To put an overlay of your own above the menu bar, render it with a `z-index`
above `201`.** Since #324 that is the whole rule, in either window type: the
bar's `201` and your overlay's number are compared in one sequence whether or
not there is an `overflow: hidden` container between them. It used not to be —
a root-level overlay behind a borderless window's container sat above the bar at
*any* `z-index`, because the two numbers were never compared at all. Code that
relied on that asymmetry needs a real `z-index` now.

## Context Menus

### Rendered Context Menu

Use the `ContextMenu` component for a styled, theme-aware context menu:

```rust
use rinch::prelude::*;

ContextMenu {
    ContextMenuTarget {
        div { "Right-click me" }
    }
    ContextMenuDropdown {
        DropdownMenuItem { onclick: || edit(), "Edit" }
        DropdownMenuItem { onclick: || duplicate(), "Duplicate" }
        DropdownMenuDivider {}
        DropdownMenuItem { color: "red", onclick: || delete(), "Delete" }
    }
}
```

The `ContextMenu` component automatically:
- Wires up the `oncontextmenu` handler on the target
- Positions the dropdown at the click coordinates using `position: fixed`
- Shows an invisible overlay for click-outside-to-close
- Reuses `DropdownMenuItem` and `DropdownMenuDivider` for consistent styling

### oncontextmenu Event

The `oncontextmenu` prop is available on all HTML elements. It fires on right-click and provides mouse coordinates via `get_click_context()`:

```rust
div {
    oncontextmenu: move || {
        let ctx = get_click_context();
        println!("Right-clicked at ({}, {})", ctx.mouse_x, ctx.mouse_y);
    },
    "Right-click target"
}
```

On Android there is no right button, so a **long press** stands in for it: a
finger held still for 500ms — `ViewConfiguration.getLongPressTimeout()`, the
same deadline the platform's own widgets use — synthesises the same event
through the same dispatch. The press must stay within 8dp to count; moving
further makes it a scroll instead, and lifting before the deadline makes it a
tap. A long press that fires the menu does **not** also fire `onclick`, so a
target can safely carry both.

## Complete Example

```rust
use rinch::prelude::*;
use rinch::menu::{Menu, MenuItem};

#[component]
fn app() -> NodeHandle {
    let file_path = Signal::new(None::<String>);
    let show_about = Signal::new(false);
    rsx! {
        div {
            h1 { "Application with Menus" }
            p {
                "Current file: "
                {|| file_path.get().unwrap_or_else(|| "Untitled".into())}
            }
            if show_about.get() {
                div {
                    h2 { "About My App" }
                    p { "Built with Rinch" }
                }
            }
        }
    }
}

fn main() {
    let file_path = Signal::new(None::<String>);
    let show_about = Signal::new(false);

    let file_menu = Menu::new()
        .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(move || {
            file_path.set(None);
        }))
        .item(MenuItem::new("Open...").shortcut("Ctrl+O").on_click(move || {
            file_path.set(Some("example.txt".into()));
        }))
        .separator()
        .item(MenuItem::new("Save").shortcut("Ctrl+S").on_click(|| println!("Saving...")))
        .item(MenuItem::new("Save As...").shortcut("Ctrl+Shift+S"))
        .separator()
        .item(MenuItem::new("Exit").shortcut("Alt+F4"));

    let edit_menu = Menu::new()
        .item(MenuItem::new("Undo").shortcut("Ctrl+Z"))
        .item(MenuItem::new("Redo").shortcut("Ctrl+Shift+Z"))
        .separator()
        .item(MenuItem::new("Cut").shortcut("Ctrl+X"))
        .item(MenuItem::new("Copy").shortcut("Ctrl+C"))
        .item(MenuItem::new("Paste").shortcut("Ctrl+V"))
        .separator()
        .item(MenuItem::new("Select All").shortcut("Ctrl+A"));

    let help_menu = Menu::new()
        .item(MenuItem::new("Documentation"))
        .item(MenuItem::new("About").on_click(move || {
            show_about.update(|v| *v = !*v);
        }));

    App::new(app)
        .title("My App")
        .size(800, 600)
        .menu(vec![
            ("File", file_menu),
            ("Edit", edit_menu),
            ("Help", help_menu),
        ])
        .run();
}
```
