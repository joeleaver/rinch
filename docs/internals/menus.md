# Native menus and shortcuts

Read this page when your task touches: Menu/MenuItem, chords, DOM menu bar, built-in shortcuts.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

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

**Checkbox and radio items (#1520).** `MenuItem::checkbox(label, Signal<bool>)`
and `MenuItem::radio(label, Signal<T>, value)`: the signal is the state both
ways. Activation (`MenuItem::activation`, used by every backend and by chords)
flips/selects it *before* `on_click`; a write from anywhere moves the mark
through a **watcher** — an `Effect` in a `Scope` owned by the build's
`MenuRegistration` (`MenuRegistration::watch`), so it dies with the menu, not
with whatever was rendering. muda builds a `CheckMenuItem` (no radio item; the
watcher re-asserts the mark muda flips itself on click), ksni a `Checkmark` or a
`RadioGroup` per run of adjacent options of one group (the ksni thread never
touches a `Signal`: clicks dispatch to the main thread, marks arrive through
`KsniUpdater`), the DOM bar a `rinch-app-menu-entry__check` mark with
`aria-checked`. **Any `Scope` dropped from a `thread_local!` at thread exit**
after the reactive runtime's own locals (a `TrayIcon` in one, holding its
watchers) skips disposal: `ScopeInner`'s `Drop` asks `disposal_available()` and
marks itself disposed, because disposing then panicked inside a TLS destructor
and aborted the process (`rinch-core/tests/review_1527_scope_tls.rs`).

**Callback lifetime (#183).** A callback belongs to the component that *created*
it — the scope rendering when `on_click` was called, where the closure captured
its signals — and stops firing once that component unmounts, rather than reading
freed state. Ownership is per **item**, not per menu build, so one `Menu` may
collect items from several components and each stops on its own. A live callback
runs *inside* its owner, so a `Signal` it creates belongs to the menu's
component; an ownerless one runs `unowned`. Either way it runs under the
document and in the context root current where the item was made (#1032),
not where it is fired — so an item built in `main` and clicked in an embedded
context's DOM bar resolves the thread-global store namespace, not that
context's. Built outside any render — from
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

**A shortcut string names a key, never a character** (#1160). Chords match the
physical key — winit's `KeyCode` on the desktop, `KeyboardEvent.code` in the
browser — plus modifiers, so punctuation is spelled by its unshifted US-layout
character or code name (`"Ctrl+/"`, `"Ctrl+Comma"`) and a shifted character by its
key and Shift (`"Ctrl+Shift+/"`; Chrome 153 reports that keystroke as `key: "?"`,
`code: "Slash"`). `"Ctrl+?"` names no key: it arms nothing and
`parse_shortcut_or_warn` logs one `tracing::warn!` per distinct string. The muda
accelerator is built from the same `ParsedShortcut`, not from muda's own string
parser, so a native label appears exactly when a chord is armed. Every Ctrl/Cmd
spelling muda and Electron accept (`Command`, `Super`, `CommandOrControl`, …) is the
one `ctrl_or_cmd` modifier, and anything after the key — a second key, a late
modifier, an unknown modifier read as a key (`Hyper+N`) — makes the string
unparseable (`Hyper+N` used to arm a bare N). The forward
table (`parse_shortcut_for_matching`) and `key_code_name` must name the same codes;
`the_two_key_tables_name_the_same_codes` reads both out of the source.

**A letter chord matches the layout's character, not only the physical key
(#1170).** `"Ctrl+Z"` fires from whichever key is *labelled* Z, wherever a
QWERTZ or AZERTY layout put it, not just the US physical `KeyZ`
(`chord_key_matches`): desktop reads winit's `key_without_modifiers` (ignores
Shift/Caps), the web reads `KeyboardEvent.key` lowercased
(`web_typed_letter`/`menu_bar.rs`), and either feeds `match_shortcut_code`'s
new `typed_letter: Option<char>` parameter. A chord falls back to the
physical key — the pre-#1170 rule — in two cases: the layout types no single
Latin letter for the pressed key at all (Cyrillic, Thai, Armenian — the
GTK/GNOME/Firefox bug class the issue's recon found, where `Ctrl+C` is
otherwise unreachable), or `ShortcutMatching::Physical` is set
(`App::shortcut_matching` / `rinch_web::set_shortcut_matching`, off by
default — the escape hatch VS Code's `keyboard.dispatch` and JetBrains'
national-layout toggle both ship). Digit and punctuation chords are untouched
by any of this; `letter_from_code` is the one gate that decides whether a
chord's key counts as a letter at all. **On macOS and Windows, a window
menu bar's item stops going through rinch's own chord matching once its
native accelerator has actually attached — "B".** `attach_menu_to_window`
wires the bar into the OS there (`NSMenu`/an accelerator table), which then
resolves and matches the `Accelerator` muda built for the item ahead of rinch
ever seeing the key; matching it a second time would risk firing the callback
twice. `build_muda_item`'s `native_bar` parameter (`true` only from
`build_native_menu_bar`, never from `build_muda_menu`'s tray items, whose
accelerator is shown only while the menu is open rather than OS-wide) tags
every window-bar `ShortcutEntry` and is **always registered** — the skip is
decided at *match* time, not registration time, by
`skips_rinch_chord(entry.native_bar, native_accelerator_attached())`. The
second fact is a recorded success, not a `target_os` guess (review of PR
#1304): `attach_menu_to_window` calls `mark_native_accelerator_attached()`
only on its own success path — Windows inside the `Ok(Win32(..))` branch,
after `init_for_hwnd` itself returns `Ok`; macOS unconditionally, since
`init_for_nsapp` reports no failure. A window whose handle was never
`Win32`, whose `init_for_hwnd` failed, or that never calls
`attach_menu_to_window` at all (a headless host, a borderless window using
the DOM bar instead) never marks it, so that window's chord keeps firing
through rinch's own registry rather than going unreachable by *either* path.
`skips_rinch_chord`'s own two-bool rule is what this crate's test suite can
exercise on a Linux host; the attach call's actual success path still needs
a human check on a real macOS or Windows host that an item's native
accelerator fires its callback exactly once. `ShortcutMatching::Physical` has
no effect on such an item once attached: the OS decides what counts as "the
Z key" for it, by its own (already character-based) per-layout resolution.
Linux's window bar (never attaches), every tray item (`native_bar: false` —
never attached to a window at all), and the DOM menu bar are unaffected by B
and keep matching through the registry as before. Related, unmeasured: issue
#1284, where a *modifier-less* chord (`chord_yields_to_text_input`) may still
be taken by a macOS/Windows native accelerator from a focused text field,
because that check is inside the matching an attached item now skips.

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

**A chord with no Ctrl/Cmd/Alt yields its key to a focused text target** (#1169):
`rinch::menu::chord_yields_to_text_input` (letters, digits, punctuation, Space,
Enter, Backspace, Delete, Home/End, PageUp/PageDown, arrows; Shift does not
matter; not Escape, Tab or F-keys), asked by both backends before matching.
Desktop asks the arbiter (`RinchApp::text_target_holds_keyboard`: a text-taking
`Input`, any `Editor`, a `Node` registered with `on_ime`) through
`RinchApp::try_menu_shortcut`, which is what the shell calls; the web asks
the keydown's `composedPath()[0]` — not `activeElement`, the host of a shadow-root field — (`<textarea>`, a text-taking `<input>`,
`contenteditable`). The `<input>` type list is one function,
`rinch::menu::input_type_takes_text`. A native muda accelerator on macOS/Windows
is matched by the OS first and is not covered (#1284, unmeasured). Pins:
`app/menu_chord_text_focus_1169_tests.rs`, `rinch-web/tests/menu_bar_bare_chord.rs`.

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
never even overwrite). Keep the `TrayIcon` alive for as long as you want the tray:
dropping it also removes the icon (#377, below). A callback may also rebuild the menu it was dispatched from — that used
to be a `BorrowMutError`.


## Keyboard Shortcuts (built-in)

- `Alt + I` - Toggle inspect mode (hover highlight for element info)
- `F12` - Toggle DevTools window
