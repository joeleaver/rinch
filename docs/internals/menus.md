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
and keep matching through the registry as before. A window-bar item whose
chord a focused control could keep gets **no** native accelerator and is
registered with `native_bar: false`, so the registry (and its yield check)
keeps it after the bar attaches (#1284, below).

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

**A keystroke the focused control acts on yields to it** (#1169, #1290,
#934/#1291): `rinch::menu::chord_yields_to(KeyOwner, ctrl, meta, alt, code,
alt_graph_text)`, asked by both backends before matching. `KeyOwner::Text`
keeps letters, digits, punctuation, Space, Enter, Backspace, Delete,
Home/End, PageUp/PageDown and the arrows (Shift does not matter; not Escape,
Tab or F-keys) — `chord_yields_to_text_input` is that set alone;
`KeyOwner::Select` (a closed `<select>`) the arrows, paging keys, letters,
digits, Space and Enter; `KeyOwner::Activator` (a button, checkbox, link,
`tabindex` node) Space and Enter; `KeyOwner::AllKeys` (a `RenderSurface`, an
open select popup; a `<canvas>` on the web) every bare key. A press holding
Ctrl/Cmd/Alt yields only when it is AltGr text into a `Text` owner:
`rinch_platform::is_alt_graph_text` — **only where AltGr arrives as Ctrl+Alt
(Windows; the `alt_graph_is_ctrl_alt` argument, `rinch_platform::alt_graph_is_ctrl_alt()`
at runtime)**: Ctrl+Alt without Cmd, printable text that is not the key's own
unmodified character, case folded (a real Ctrl+Alt+C reports `"c"`). Off
Windows it is always false: Linux xkb and AppKit report a Shift-held Ctrl+Alt
press's *shifted* text against the unshifted key (Ctrl+Alt+Shift+1: `"!"`
against `"1"`), which must stay a chord, as in Chrome on Linux. The setting
`ChordYield::TextOnly` (`set_chord_yield`, `App::chord_yield`) folds every
non-text owner into `None` — the escape hatch to #1169's rule. Desktop
classifies through `RinchApp::key_owner` (the arbiter's `FocusTarget`) in
`RinchApp::try_menu_shortcut`, which the shell calls with winit's `text`; the
web through `menu_bar::key_target_owner` on the keydown's
`composedPath()[0]` — not `activeElement`, the host of a shadow-root field —
and `getModifierState("AltGraph")`. The `<input>` type list is one function,
`rinch::menu::input_type_takes_text`.

**AltGr text reaches the field as text** (#934, Windows): the shell keeps `text` on a
Ctrl+Alt press only when `is_alt_graph_text` says AltGr typed it
(`rinch_platform::alt_graph_filtered_text`; the debug channel does the same),
so the runtime reads "Ctrl+Alt and text" as typing: the input fallback's `_`
arm inserts it and its Ctrl+A/C/V/X/Z/Y arms refuse Alt; the editor inserts it
ahead of its clipboard chords and keymap.

**Native accelerators withheld** (#1284, #1291): `build_muda_item` gives a
window-bar item no muda `Accelerator` when `native_accelerator_allowed` says a
focused control could keep its chord — any bare chord some `KeyOwner` keeps
under the active `ChordYield`, and on Windows a Ctrl+Alt chord on a
letter/digit/punctuation/Space key (AltGr is Ctrl+Alt to `TranslateAccelerator`)
— and registers it `native_bar: false`, so rinch matches it after the bar
attaches. On Windows the label gets `"\t<shortcut>"` (the Win32 accelerator
column); macOS shows no key. Unmeasured on real macOS/Windows: whether AppKit
offers bare key equivalents ahead of `keyDown:` (assumed yes), and the Windows
label rendering. Pins: `app/menu_chord_text_focus_1169_tests.rs`,
`app/menu_chord_owner_tests.rs`, `menu/mod.rs`'s
`each_owner_keeps_its_own_bare_keys_…` and
`native_accelerators_are_withheld_…`, `rinch-web/tests/menu_bar_bare_chord.rs`.

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


## Rendered menus: submenus, keys and Escape (#2214, #2169, #2170)

`DropdownMenuSub` (`rinch-components/src/dropdown_menu_sub.rs`) and the menus'
keyboard (`menu_nav.rs`) are component code only — one implementation for the
desktop and rinch-web. Defaults are Chromium's views menus, read from
`ui/views/controls/menu/menu_config.h` (`show_delay = 400`,
`submenu_horizontal_overlap = 3`, `arrow_key_selection_wraps = true`) and
`menu_controller.cc` (the sibling switch on the show timer in
`CommitPendingSelection`, "keeps the submenu open" when the pointer leaves every
menu in `HandleMouseLocation`, `SelectByChar`, ArrowRight/Left/Return/Escape in
`OnKeyPressed`, no safe triangle).

**The flyout stays in the DOM** under its row (`div.rinch-dropdown-menu__sub` →
row button, offset probe, `div.rinch-dropdown-menu__sub-dropdown`). So the CSS
cascade reaches it per menu, and an item in it closes the whole menu by the
ordinary `find_menu_close_signal` walk: a submenu stamps **no**
`MENU_CLOSE_ID_ATTR`, so the walk passes through it to the root menu (the nested
`DropdownMenu` rule, `nested_menu_closes_the_nearest_one`, is untouched). The
root menu's close cascades down: `DropdownMenu`'s `opened_fn` falling edge and its
item-close watcher, and `ContextMenu`'s hide branch, call
`close_subs_within(root)`.

**Placement.** The flyout is `position: fixed` — no ancestor overflow clips it,
on desktop (a fixed box carries no ancestor clip chain) or in a browser — and is
placed in viewport px on each open by `arm_placement`, the `floating.rs`
pattern: write `left: 0; top: 0`, measure after the layout (desktop: a
`bounds_signal` effect, settled before paint; web: the same task), write the
result. The measurement at 0/0 *is* the containing block's origin, so a
transformed ancestor (which a browser makes a fixed box's containing block) is
subtracted out. The horizontal offset is the CSS custom property
`--rinch-dropdown-menu-sub-offset`, read through a zero-size probe
(`left: var(…)` in the `position: relative` submenu): rinch has no
computed-style read on `NodeHandle`. `decide` (pure, unit-tested) puts it
right of the parent panel, flips it left when it does not fit, shifts it inside
by `EDGE_MARGIN` (4px), and caps one taller than the window. While open it
follows its row (the row's `bounds_signal`). No width is pinned — placed where
it fits, a fixed box's shrink-to-fit width is its content's — and the size it was
placed at is kept: a different size later is its content changing, and it is
measured and placed again (`remeasure`). A window resize is seen through a
viewport-sized, hidden `position: fixed` sentinel shown while open (no viewport
signal on `NodeHandle`); it closes the submenu (Chrome closes menus on a bounds change) or,
with `close_on_resize: false`, re-places it. On desktop the resize is the
runtime's `resize_layout` path, which refreshes the bounds signals. The size compared is the sentinel's own box (`100vw` x `100vh`, the window), not
`viewport_size()`: on the web that is `clientWidth`, which shrinks when a page
scrollbar appears, and a scrollbar closed open submenus (second review of #2216).
A display scale change closes it too, whatever the logical size does (Chrome
closes its menus on any display change), or with `close_on_resize: false`
re-places it: `RinchApp::set_device_pixel_ratio`, every route a new scale takes,
calls `rinch_core::events::notify_display_scale_changed(doc_key)` when the scale
differs, and the placement effect reads `display_scale_changes(doc_key)` (a
per-document counter signal) against the generation it was placed at. rinch-web
has no such event; a browser zoom changes the window's CSS size, which the
sentinel sees.

**Pointer.** Hover-open is a `data-onenter` on the row (arm the open timer); the row's
`data-onleave` clears `on_row`, and a pending open fires only while it is set, so
a row merely crossed — into a flyout whose own panel hears the pointer, or out of
the menu, where no panel hears it — opens nothing (Chrome drops the pending
selection). Desktop's leave-then-enter on every hit-node change inside the row
sets it false and true in one event, so the delay is not restarted. Every
other decision is geometric, from one `data-onmousemove` the first submenu row
the pointer enters (or the first open) installs on its **parent panel**
(`PANEL_MARK`; only panels holding a submenu get one, so no other app's
nearest-handler `data-onmousemove` changes). It records the pointer
(`LAST_POINTER`), cancels a pending open when the pointer leaves the row, and
arms a close when the pointer is on the panel outside an open submenu's region
(row, flyout, open descendants' flyouts). The close timer checks
`LAST_POINTER` against the region again when it fires, because a pointer that
went on into a *deeper* flyout is heard by that flyout's panel handler, not this
one, and cancelled nothing. Leaving every menu produces no move on any panel, so
nothing closes — Chrome's behaviour. Hover enter/leave semantics differ between
the backends (desktop fires leave + enter on every hit-node change, the web only
when the nearest handler changes), which is why nothing here relies on leave.

**Keys.** A submenu root carries an element `onkeydown`: on its row
ArrowRight/Enter/Space open and focus the first row; with the focus in its own
flyout (nearest panel), ArrowLeft/Escape close it and refocus the row. Element
handlers run before everything below, nearest first, so a nested submenu
answers before its parent. Escape with the focus anywhere goes to a dismiss
entry each submenu pushes **at open** (LIFO: innermost first). The menus
themselves (`arm_menu_keys`) push their Escape entry at open too (the
`arm_close_on_escape_while_open` policy, since `opened_fn` has an edge), and hold
an **app key listener** (`on_app_key`) while open for Up/Down/Home/End and
type-ahead — an app listener because a right-click-opened `ContextMenu` has no
focused element inside it, so a focus-scoped `push_key_handler` entry would
never be offered a key. `OPEN_MENUS` makes the most recently opened menu's
listener the only one that answers. Keys are declined while a text-taking
field holds the focus, and with Ctrl/Alt/Meta. The arrows walk the panel the
focus is in (`nearest_panel`), so one listener serves the root panel and every
flyout. On close the focus returns to the element focused at open when it was
left inside the panel. `DropdownMenu` also closes when the focus leaves its
root or the window loses the keyboard (#799: a no-key `push_key_handler` entry
whose `on_focus_leave` closes, `MenuKeyOptions::close_on_focus_leave`), so Tab
past its last item closes it and goes on, as the WAI-ARIA menu button does;
`ContextMenu` does not ask. Item rows are recognised by `tag_name()`, not
`node_type()`, which rinch-web does not implement.

Pins: `rinch/src/app/menu_sub_2214_tests.rs` (desktop, real layout, manual timer
backend), `rinch-web/tests/menu_sub_2214.rs` (Chrome and Firefox),
`dropdown_menu_sub::tests` (`decide`).

## Keyboard Shortcuts (built-in)

- `Alt + I` - Toggle inspect mode (hover highlight for element info)
- `F12` - Toggle DevTools window
