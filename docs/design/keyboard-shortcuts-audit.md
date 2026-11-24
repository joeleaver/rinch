# Keyboard shortcuts: the line, and the plan (2026-11-24)

**Status:** proposal, agreed in outline by Joe on 2026-11-24 (remove the shortcuts
rinch claims; keep the keys Chrome has, and make every one of them
interceptable). No code has changed yet. The plan comes first. The inventory it
rests on follows as an appendix: every key rinch handles on desktop and on the
web, 59 entries, with file:line.

File:line references are against `main` at `f9d15abf7`. Paths are relative to
`crates/` unless they start with `docs/`.

## Why

Joe thinks rinch has crossed the line between sensible framework defaults and
interfering with shortcuts that apps define, but nobody knew exactly where. The
docs had no list of the keys rinch claims. Two standing rules apply here (see
`docs/design/component-configurability-audit.md`):

1. **A default is what Chrome does.**
2. **Every UX choice is an option with an escape hatch.**

## The line

- **(A) Control behaviour a browser also has** (caret keys in a field, Tab,
  Enter/Space activation, Escape closing a dialog, arrows in an open menu,
  keyboard scrolling): kept, as defaults the app can always take. They are
  rinch's equivalent of a browser's default actions.
- **(B) App-level shortcuts rinch claims: removed.** An app that wants one
  binds it itself, with a keymap plugin on the editor or a menu accelerator.
- **(C) Unclear:** each entry is decided below.

## The plan

### 1. Remove these bindings

| Entry | Keys | Why | What an app does instead |
|---|---|---|---|
| E9 | Mod-Shift-S (strikethrough) | Chrome's `contenteditable` has no such key | Binds it in its own keymap plugin, or calls `command("toggleStrike")` from a toolbar |
| E10 | Mod-E (inline code) | Not in Chrome. On the web it hides Ctrl+E (address bar) and Cmd+E (use selection for Find) | Same |
| E13 | Mod-Shift-7 / 8 / 9 (task, bullet and ordered list) | Not in Chrome | Same |
| E14 | Mod-Shift-B (blockquote) | Not in Chrome. On the web it hides the bookmarks-bar chord | Same |
| E15 | Mod-Shift-0 (paragraph) | Not in Chrome | Same |
| E16 | Mod-Shift-L / E / R / J (alignment) | Not in Chrome. On the web, Ctrl+Shift+R (hard reload) is kept even where the command does nothing | Same |
| E17 | Mod-Alt-1 … 6 (headings) | Not in Chrome | Same |
| R2 | Alt+I (Option+I), inspect mode | An app-level key. In a debug build it beats typing (Option+I is macOS's ˆ dead key) | Inspect mode stays reachable from the DevTools window and the debug tools, with no key |
| R11 | Cmd+Y as redo, on macOS only (fields and editor) | On macOS Chrome, Cmd+Y opens History; redo is Cmd+Shift+Z | Nothing: Cmd+Shift+Z stays. Ctrl+Y stays on Windows and Linux |
| K11 | Tree's Ctrl/Cmd+A (selects the cursor row's siblings) | Not what WAI-ARIA says (select all) | Binds it through the tree's `handle_key` |
| K10, in part | Tree's PageUp / PageDown (±10 rows) | Not in the WAI-ARIA tree pattern | Same. The rest of Tree's keys stay: they follow WAI-ARIA, and `keyboard` is opt-in already |

That is 22 keys and chords: the editor's 17 formatting chords, Alt+I, macOS
Cmd+Y, Tree's Ctrl+A, and Tree's PageUp and PageDown. Each one hides some
other key: a browser chord, a character typed in a field, or the app's own
shortcut. Mod-B, Mod-I and Mod-U stay, because Chrome's `contenteditable` has
them. The commands stay too; only their keys go. Bold, a bullet list or a
heading are still one `command(..)` call away, and the guide shows how to bind
the old chords in a few lines of a keymap plugin.

### 2. Off by default

| Entry | Behaviour | Default today | Change |
|---|---|---|---|
| E12 | Tab / Shift+Tab indents a list item and moves between table cells | on (`TabBehavior::IndentInListsAndTables`), recorded as an exception to the Chrome rule in #2210 | **`TabBehavior::MoveFocus` becomes the default**, as Chrome does. ui-zoo and Pimble turn indenting back on |
| K9 | Sortable's keyboard drag (Space/Enter lifts, arrows move, Escape cancels) | on | Becomes a `keyboard: bool` prop, **off by default**. No browser or WAI-ARIA pattern has it |

### 3. Kept, and why

- **E11, Mod-Enter (`exitCode`).** Nothing else gets the caret out of a code
  block that is the last block in the document. There is no trailing
  paragraph and no gap cursor, and Enter inserts a newline there. It already
  falls through wherever it does not apply, so an app's Ctrl/Cmd+Enter
  (send) still works outside code blocks. Revisit if the editor gains another
  way out.
- **R1, F12.** It opens DevTools in debug builds only. Chrome opens its
  DevTools on F12 too, so this is class A (Chrome parity), not an app
  shortcut. `App::devtools` and `RINCH_DEVTOOLS` turn it off.
- **E1–E8, E18, E20.** The editor's caret, delete, Enter, select-all,
  undo/redo, clipboard and Mod-B/I/U. Chrome's `contenteditable` has all of
  them.
- **R3–R10, R12–R16.** Field keys, Tab between focusable elements,
  activation, `onsubmit`, keyboard scrolling, `<select>` keys, the text
  context menu, and Escape for a drag or a pointer lock. Chrome has all of
  them.
- **K1–K8, and Tree's other keys.** Escape closes overlays, dialogs trap Tab,
  and menus, `Select` and trees have their WAI-ARIA keys.
- **R17, the chord yield.** A focused control keeps its bare keys ahead of
  the app's bare-key accelerators, as native controls do.
  `ChordYield::TextOnly` relaxes it.

### 4. Make every kept key interceptable: the handler order

**Everything rinch does with a key is a browser *default action*.** It runs
only after every app hook has had the key and declined it, and any hook can
take the key from it, as `preventDefault` does in a browser.

Rinch beats the app in four places only, and Chrome beats the page in the
same four. Three of them beat even the app's menu accelerators, as they do
today:

- Escape under a pointer lock (R4). The shell skips the menu matcher for it
  (`rinch/src/shell/rinch_runtime/winit_input.rs:419`).
- An open runtime popup: the `<select>` list (R15) or the text context menu
  (R16). A native popup takes the keyboard before the page sees it, and
  `try_menu_shortcut` runs no chord while one is open (`rinch/src/app/focus.rs:918`).

The fourth runs after the accelerators:

- Escape during a drag (R3). A menu accelerator bound to Escape runs first,
  through `try_menu_shortcut` (`winit_input.rs:419`–`:420`; on the web, the
  `window` capture chord listener).

Apart from R4, R15 and R16, the app's own menu accelerators stay first,
because they are the app's.
Component key handlers count as app code: the components are a library that
runs on top of the app, like a page's JS widget library.

| # | Handler, on both backends | Browser equivalent |
|---|---|---|
| 1 | R4, R15 and R16, then the app's menu accelerators, then R3 (the exceptions above) | the browser chrome |
| 2 | **The interceptor** (`set_keyboard_interceptor`), moved ahead of everything below. On the web it becomes a `window` capture listener | `window` capture listener |
| 3 | `EditorHandle::on_key`, a node view's `on_key`, element `onkeydown` (nearest first) | target phase |
| 4 | Popup key entries and the dismiss stack (components) | `document` listeners |
| 5 | `on_app_key`, **now offered the editor's keys too** | `window` bubble listener |
| 6 | Rinch's defaults: the editor's keymap and caret keys, field keys, Tab, activation, `onsubmit`, scrolling, DevTools | default action |

**On the web, every rinch key listener checks `defaultPrevented` first.** That
covers the menu chord listener, the editor's capture listener and the bubble
delegate, so a page's `preventDefault` beats rinch as it beats Chrome.

Today's departures from this order:

- On both backends, the interceptor runs after element `onkeydown`: desktop
  `event_dispatch.rs:1964` before `:1991`, web `event_delegation.rs:3994`
  (`data-onkeydown`) before `:4013`.
- On the web, the interceptor also runs after the editor's keymap.
- `on_app_key` never sees the editor's keys.
- Nothing on the web reads `defaultPrevented`.

Issue #1678's case still works: a `[[` picker in a field inside a `Modal`
gets Escape, because the picker's handler (step 3) runs before the modal's
dismiss entry (step 4).

### 5. Platform fixes

- **E21:** on the web, Mod becomes Cmd on macOS and Ctrl elsewhere
  (ProseMirror's rule, from `navigator.platform`). That way Ctrl+A and Ctrl+E
  on a Mac stop running editor commands, and Win+key stops reaching the
  keymap.
- **R18, Android:** answer `InputStatus::Unhandled` for keys rinch does not
  map, such as the volume and media keys. Back is delivered as Escape first,
  and goes to the OS when nothing takes it. That half needs the key
  dispatched inside the input callback, so the answer is known before the
  callback returns; today events are queued and run later. Check on a device.
- **E19:** a read-only editor on the web calls `preventDefault` on printable
  keys but stops stopping their propagation, so the page's own listeners see
  them, as they see keys in a `readonly` field.
- **W1:** the web withholds only the keyups of presses a focused surface
  claimed, as desktop does (#1784).

### 6. Proposed, not yet decided: the graph view (K12–K18)

The graph view's canvas (`rinch-graph-view`) was missed by the first pass. Its
keys are below in the inventory. These are proposals for Joe's decision
(tracked on #3062); none is in the agreed list above, and none is in the PRs
below until he decides.

- **K17:** match `+`, `=` and `-` only without Ctrl, Meta or Alt. Today
  `canvas_key` consumes them with any modifier, so on the web Ctrl+= and
  Ctrl+- zoom the graph instead of the page while the canvas has focus.
  Whether the bare zoom and fit keys stay on by default is the open question.
- **K14:** drop Cmd+Y on macOS, as R11 does for fields and the editor.
- **K12:** Delete and Backspace are consumed even with nothing selected.
  Proposed: consume them only when something was deleted.
- **All of K12–K18:** the graph view has no option for its keys. Proposed: a
  `keyboard: bool` option (on by default, since these are the control's own
  editing keys) and an `on_key` hook that runs first.

### The PRs, in order

| PR | Contents | Sections |
|---|---|---|
| 1 | **The handler order** on both backends, `defaultPrevented` on the web, and `on_app_key` offered the editor's keys. Also the web's surface keyups (W1) and read-only editor keys (E19), which touch the same web listeners. Pinned by one cross-backend test file per step, and documented on `focus-and-keyboard.md` | 4, 5 |
| 2 | **Editor keys:** remove the 17 formatting chords (E9, E10, E13–E17) from the base keymap; `TabBehavior::MoveFocus` becomes the default; add `EditorHandle::unbind_key(chord)` / `Keymap::remove`, so an app can drop any kept chord without dropping the whole base keymap; the web's per-platform Mod; Cmd+Y off on macOS. ui-zoo, the markdown-editor example and Pimble bind the chords they want and opt back into indenting. Update `docs/src/guide/contenteditable.md#keyboard-shortcuts` and `docs/internals/editor.md` | 1, 2, 5 |
| 3 | **Runtime:** remove Alt+I; Android `Unhandled` and Back | 1, 5 |
| 4 | **Components:** remove Tree's Ctrl+A and PageUp/PageDown; Sortable's `keyboard` prop, off by default | 1, 2 |
| 5 | **Guide page** `docs/src/guide/keyboard.md`: every key rinch still handles, the handler order, how to take a key, and how to bind the removed chords. Add it to `SUMMARY.md` | all |

PR 1 comes first, because it is the change that makes the kept keys
interceptable. PRs 2–4 are independent of one another and of PR 1, and can run
in parallel. PR 5 goes last.

**Breaking changes** to put in the release notes:

- The 17 formatting chords.
- Tab in editor lists.
- Alt+I.
- Cmd+Y on macOS.
- Tree's Ctrl+A and PageUp/PageDown.
- Sortable's keyboard drag.
- The new handler order: an app whose interceptor relied on running after
  element handlers will see a change.

# Appendix: the inventory

## How a key travels today

### Desktop

All references are to `rinch/src/app/event_dispatch.rs` unless noted. A
`KeyDown` is offered to these handlers in this order. The first one that
consumes the key stops it.

| # | Step | Where | Who it belongs to |
|---|---|---|---|
| 0a | Escape ends a pointer lock. The shell skips the menu matcher for this key (`winit_input.rs:419`), so it runs ahead of the accelerators | `:393` | rinch |
| 0b | **The app's menu accelerators**, unless a focused control keeps the bare key (`chord_yields_to`, `rinch/src/menu/mod.rs:980`), or a `<select>` popup or the text context menu is open (`focus.rs:918`: no chord runs then) | shell, `rinch/src/shell/rinch_runtime/winit_input.rs:420` → `rinch/src/app/focus.rs:908` | app |
| 0c | Escape cancels a DOM drag | `:1847` | rinch |
| 1 | `EditorHandle::on_key`, when an editor has focus | `:1887` | app |
| 2 | A node view's `NodeViewSpec::on_key`, when focus is inside a node view | `:1919` | app |
| 3 | Element `onkeydown`, nearest first. Offered only when focus is on an `Input` or a `Node`: never for the editor, a render surface or an open `<select>` popup | `:1964`, `:19` | app **and components** |
| 4 | **The keyboard interceptor** (`set_keyboard_interceptor`), then the open popups' key entries (`push_key_handler`), then the Escape dismiss stack | `:1991` → `rinch-core/src/events/keyboard.rs:226` | app, then components |
| 5 | The built-in text context menu, while it is open | `:2016` → `text_context_menu.rs:1435` | rinch |
| 6 | Routing by focus target. **Editor:** Tab → `tab_key`, else the keymap (`dispatch_new_editor_key`). **Surface:** `set_key_handler`. **`<select>` popup:** its keys. **Input, Node or nothing:** a registered widget's `on_key`, then `on_app_key` listeners (`:2191`), then the fallback | `:2023`–`:2215` | mixed |
| 7 | The fallback `handle_unclaimed_key_fallback`: F12, Alt+I, keyboard scrolling, Tab, field caret keys, clipboard chords, activation, Enter, Alt+Down, text input | `:2729` | rinch |

Consequences:

- **The app beats every built-in fallback key** (step 7) through `onkeydown`,
  the interceptor or `on_app_key`, as long as something is focused that
  step 3 offers keys to.
- **The interceptor runs after element `onkeydown` (step 3 before step 4).**
  So on desktop an app has no capture-phase hook. A component's own
  `onkeydown` deeper in the tree (`SortableItem`, `DropdownMenuSub`) runs
  before any app handler. Only `EditorHandle::on_key` sits ahead of step 3.
  In a browser, a `document` capture listener beats every element listener.
- **`on_app_key` never sees the editor's keys**, nor the keys of a `<select>`
  popup or the text context menu, on either platform (`offer_app_key`'s
  docs, `:125`).
- **Menu accelerators beat every handler except three:** Escape under a
  pointer lock (R4, step 0a), and an open `<select>` popup or text context
  menu (R15, R16), which suppress them (`focus.rs:918`). They also defer to
  0b's yield check. So an accelerator bound to Escape beats the drag cancel
  (R3, step 0c), the editor's `on_key`, element handlers and the interceptor.
  These are the app's own chords, so this is app against app, not rinch
  against app. But it means an app cannot turn off one of its own chords
  for one focused element, except through `ChordYield`.

### Web (`rinch-web`)

| # | Listener | Where | preventDefault / stopPropagation |
|---|---|---|---|
| 1 | `window` capture: the menu chord matcher, armed when a menu bar mounts | `rinch-web/src/menu_bar.rs:184` | both, but only when a callback ran |
| 2 | `document` capture: the editor (`handle_keydown`): `on_key`, node views, caret keys, Tab, the keymap | `rinch-web/src/editor_input/install.rs:153` → `editor_input/keyboard.rs:177` | both, when consumed |
| 3 | Target and ancestors | page listeners only | — |
| 4 | `document` bubble: the rinch delegate. In order: Escape cancels a drag (`:3973`), element `data-onkeydown` (`:3994`), the interceptor, key entries and dismiss stack (`:4013`), a focused surface (`:4053`), `on_app_key` (`:4071`), the Tab trap (`:4080`), Enter/Space activation (`:4094`), Enter commit and `onsubmit` (`:4101`) | `rinch-web/src/event_delegation.rs:4144` | both when a handler consumes; preventDefault only for the Tab trap, activation, submit and drag cancel |
| 5 | `window` bubble | page listeners only | — |

Consequences:

- **No rinch key handler reads `event.defaultPrevented`.** A page listener
  that calls `preventDefault()` stops nothing in rinch. Only stopping
  propagation earlier in the order does. `default_prevented` is read only for
  clipboard events, file drag and drop and pointer focus
  (`event_delegation.rs:2140`, `:2159`, `:2288`, `:3464`, `:3829`, `:4480`,
  `:4510`). This breaks the contract the (A) class assumes.
- **The editor's keymap runs before the interceptor and before `on_app_key`.**
  On desktop the interceptor beats the keymap (step 4 before step 6). On the
  web it does not (listener 2 before listener 4). On the web, only
  `EditorHandle::on_key`, or a page capture listener installed before rinch
  mounts that calls `stopImmediatePropagation`, gets ahead of Ctrl+B.
- **A page cannot beat a menu chord** unless its `window` capture listener
  was added before the first menu bar mounted and calls
  `stopImmediatePropagation`.
- There is no F12, Alt+I, keyboard scrolling or field caret handling on the
  web. The browser does those.

## Inventory

Columns:

- **Scope:** *field* is a focused text field. *editor* is the focused
  rich-text editor. *control* is a focused control. *popup* is an open popup
  or overlay. *app* is app-wide.
- **Precedence:** which app hooks run before the binding. *el* is element
  `onkeydown`. *int* is the interceptor. *ak* is `on_app_key`. *on_key* is
  `EditorHandle::on_key`. *menu* is a menu accelerator.
- **Off switch:** how an app can disable or override the binding today.

On the web, "*int*" in the precedence column applies only to the bubble
delegate's keys. Read the web consequences above alongside it.

### Runtime (desktop, and the web where noted)

| ID | Key | What rinch does | File:line | Scope | Chrome | Precedence (who beats it) | Off switch | Class |
|---|---|---|---|---|---|---|---|---|
| R1 | F12 | Toggles the DevTools window (desktop only) | `rinch/src/app/event_dispatch.rs:2751` | app | Opens DevTools. A page can `preventDefault` it | menu, el, int, ak | Off by default in release builds (`devtools_keys.rs:27`). `App::devtools(bool)`, `RinchApp::set_devtools`, `RINCH_DEVTOOLS=on\|off` | **A** (Chrome parity: F12 opens Chrome's DevTools), debug-only |
| R2 | Alt+I (Option+I on macOS) | Toggles inspect mode (desktop only) | `event_dispatch.rs:2757` | app, **text fields included** | Nothing on Windows or Linux. On macOS, Option+I is the circumflex dead key (ˆ). Chrome's inspect chord is Ctrl+Shift+C | menu, el, int, ak | Same as R1 | **B**, debug-only. ⚑ It runs before text input, so in a debug build Option+I cannot type ˆ in a field on macOS, and Alt+I cannot type its character on any layout where it types one |
| R3 | Escape during a DOM drag | Cancels the drag | desktop `event_dispatch.rs:1847`, web `event_delegation.rs:3973` | drag | Cancels the drag. During HTML5 DnD the page gets no key events | ⚑ **every DOM handler**: it runs ahead of the editor, element handlers, the interceptor and `on_app_key`. Only a menu accelerator bound to Escape runs first (desktop `try_menu_shortcut`, `winit_input.rs:419`–`:420`; web, the `window` capture chord listener). On the web a page capture listener can also stop it | none | A (Chrome parity). ⚑ beats the app's DOM hooks, as Chrome's does |
| R4 | Escape under a pointer lock | Ends the lock and consumes the key | `event_dispatch.rs:393`, `pointer_lock.rs` | lock | Ends the lock, and the page cannot stop that | nothing | none | A (Chrome parity) |
| R5 | Tab / Shift+Tab | Moves to the next or previous Tab stop; wraps inside `trap_focus` | desktop `event_dispatch.rs:2778` → `app/mod.rs:5610`; web trap `event_delegation.rs:4080` (outside a trap the browser moves focus) | app | Same. A page can `preventDefault` it | menu, el, int, ak | consume the key; `tabindex`/`data-nofocus`; `trap_focus: false` | A |
| R6 | Enter / Space on a focused button-like node | Activates it: Enter on press, Space on release | desktop `:2812`, web `event_delegation.rs:4094` | control | Same | menu, el, int, ak | consume the key | A |
| R7 | Enter, Space, Alt+ArrowDown on a closed `<select>` | Opens the popup | `event_dispatch.rs:2828` | control | Same in Chrome on Linux. **Unverified for Chrome on Windows and macOS**, whose closed `<select>` follows the platform's combo box; which of these keys open it there has not been checked | menu (yields: `KeyOwner::Select`), el, int, ak | consume the key | A |
| R8 | Enter in an `<input>` | Commits `onchange`, then fires `onsubmit` | desktop `:2816` → `app/mod.rs:4841`; web `event_delegation.rs:4101` | field | Implicit form submission | menu (yields), el, int, ak | consume the key; Shift+Enter | A |
| R9 | Arrows, Home/End, Backspace/Delete, word and line variants (Ctrl on Windows and Linux; Option and Cmd on macOS) | Field caret and delete keys, following each platform's keymap | `:2788` → `text_keymap.rs:94` | field | Same (Cocoa bindings on macOS) | menu (yields: `KeyOwner::Text`), el, int, ak | consume the key | A |
| R10 | Ctrl/Cmd + A, C, V, X, Z, Shift+Z | Select all, clipboard, undo/redo in a field | `:2796`–`:2803` | field | Same | **menu first** (a chord holds a modifier, so it never yields), el, int, ak | consume the key | A |
| R11 | Ctrl/Cmd+Y | Redo in a field and in the editor (`plugins/history.rs:967`) | `:2804` | field, editor | Redo on Windows and Linux. **On macOS, Cmd+Y opens History; redo is Cmd+Shift+Z only** | menu, el, int, ak | consume the key | **C**: a non-Chrome chord on macOS |
| R12 | Ctrl/Cmd+A and Ctrl/Cmd+C with no field focused | Selects all read-only text; copies the selection | `:2796`, `app/mod.rs:5323` | app | Same | menu, int, ak (no element is focused) | consume the key | A |
| R13 | Shift + arrows/Home/End, plus the word chord | Extends a read-only text selection | `text_selection_keys.rs:47` | app | Same (Chrome 158) | menu, el, int, ak | consume the key | A |
| R14 | Arrows, PageUp/PageDown, Home/End, Space, Shift+Space | Keyboard scrolling (desktop) | `:2771` → `keyboard_scroll.rs:142` | app | Same | menu, el, int, ak | consume the key; `set_smooth_wheel_scroll` changes only the animation | A |
| R15 | Arrows, Home/End, Enter, Space, Escape, type-ahead in an open `<select>` popup | Navigate, commit, close | `select_widget.rs:633` | popup | Same (a native popup) | int only: no el, no ak, and menu chords are suppressed while it is open | consume the key in the interceptor | A |
| R16 | Arrows, Home/End, Enter, Space, Escape in the built-in text context menu | Navigate, run, close | `text_context_menu.rs:1435` | popup | Same (a native menu) | int only; menu chords are suppressed | `set_text_context_menu_presentation(Shell)` lets the app draw its own menu | A |
| R17 | Bare keys a focused control keeps | The control's own key beats the app's **bare-key** menu accelerator: text fields keep letters, digits, Space, Enter, arrows, Backspace/Delete, Home/End, PageUp/PageDown; `<select>`, buttons and surfaces keep theirs | `rinch/src/menu/mod.rs:980`, web `menu_bar.rs` (`key_target_owner`) | field, control | A native control in a native app keeps its keys. Electron's menu accelerators beat the page | ⚑ **rinch beats the app's menu here, by design** | `App::chord_yield(ChordYield::TextOnly)` releases non-text owners. Text owners always keep theirs | A |
| R18 | Android Back (and every other key) | Back is mapped to Escape (`android_runtime.rs:1166`), and **every key event answers `InputStatus::Handled`** (`:1131`), unmapped keys too | `rinch/src/shell/android_runtime.rs:1083`–`:1131` | app | In Chrome on Android, Back closes the top popup (`CloseWatcher`), then goes back or leaves. Volume keys change the volume | ⚑ rinch beats the system. The app gets Escape but cannot pass Back on to the OS | none | **C**: by android-activity's contract the system's default never runs, so Back never leaves the activity and the volume keys may be swallowed. Not checked on a device |

### Rich-text editor (both platforms unless noted)

Desktop runs these at step 6. The web runs them at listener 2. *Mod* is Ctrl
on Windows and Linux and Cmd on macOS desktop. **On the web, Mod is Ctrl or
Meta on every OS** (`editor_input/keyboard.rs:248`); see E21. The keymap is in
`rinch-editor-core/src/commands/mod.rs:1706`–`:1777` and
`plugins/history.rs:966`. **Every binding except Mod-Enter is consumed even
when its command does nothing** (`docs/internals/editor.md`, "Which keys a
binding consumes").

| ID | Key | What it does | File:line | Chrome `contenteditable` | Precedence | Off switch | Class |
|---|---|---|---|---|---|---|---|
| E1 | Arrows, word, line and document moves, with Shift to extend | Caret motion; Shift+Up/Down selects table cells | desktop `event_dispatch.rs:4876`, `text_keymap.rs`; web `keyboard.rs:302`–`:339` | Same | menu (Mod chords only), on_key; desktop also int | `on_key` only | A |
| E2 | PageUp / PageDown (macOS: Option or Shift to move the caret) | Moves the caret a page | `event_dispatch.rs:5042`, web `motion.rs:20` | Same | as E1 | `on_key` only | A |
| E3 | Backspace / Delete, plus Mod or Alt for a word | Delete. Next to an atom (an embed, an image, a rule), the first press selects it | `commands/mod.rs:1715`–`:1716`, `:1771`–`:1777` | Same, except the select-the-atom-first step (Joe's decision of 2026-10-19) | as E1 | later plugin rebind; `on_key` | A |
| E4 | Enter / Shift+Enter | Split the block / line break (a newline in a code block) | `:1712`–`:1713` | Same | as E1 | as E3 | A |
| E5 | Mod-A | Select all | `:1706` | Same | as E1 | as E3 | A |
| E6 | Mod-Z, Mod-Shift-Z, Mod-Y | Undo / redo (Mod-Y on macOS: see R11) | `plugins/history.rs:966`–`:968` | Same | as E1 | `remove_plugin(HISTORY_KEY)`; as E3 | A |
| E7 | Mod-C, X, V, Shift+V (desktop) | Copy, cut, paste, paste as plain text. The web leaves the key alone and acts on the browser's clipboard events | `event_dispatch.rs:4836`–`:4846`; web `editor_input/clipboard.rs` | Same | **menu first**, on_key, int | `on_key`; `Plugin::handle_paste` | A |
| E8 | Mod-B / Mod-I / Mod-U | Bold / italic / underline | `:1707`–`:1709` | **Same**: Chrome's `contenteditable` runs these | as E1 | as E3 | A |
| E9 | Mod-Shift-S | Strikethrough | `:1710` | Nothing | as E1 | as E3 | **B** |
| E10 | Mod-E | Inline code | `:1711` | Nothing. ⚑ On the web the editor keeps it, so Ctrl+E (focus the address bar, Windows and Linux) and Cmd+E (use the selection for Find, macOS) never reach the browser | as E1 | as E3 | **B** |
| E11 | Mod-Enter | Leave a code block (`exitCode`). Falls through where it does not apply | `:1714`, `:1750` | Nothing | as E1 | as E3 | **C**: no Chrome equivalent, but it is the code block's own control key and is already declared fall-through |
| E12 | Tab / Shift+Tab | Next or previous table cell (the last cell adds a row); indent or outdent a list item; otherwise move focus | `:1717`–`:1718`, `rinch-editor-view/src/handle/selection.rs:227` | Moves focus | on_key, el (desktop), int (desktop) | `set_tab_behavior(TabBehavior::MoveFocus)`. That turns off the table and list behaviour together | **C**: a recorded exception to the Chrome rule (#2210) |
| E13 | Mod-Shift-7 / 8 / 9 | Task / bullet / ordered list | `:1719`–`:1721` | Nothing | as E1 | as E3 | **B** |
| E14 | Mod-Shift-B | Blockquote | `:1722` | Nothing. ⚑ On the web it hides Chrome's toggle for the bookmarks bar | as E1 | as E3 | **B** |
| E15 | Mod-Shift-0 | Paragraph | `:1723` | Nothing | as E1 | as E3 | **B** |
| E16 | Mod-Shift-L / E / R / J | Align left / center / right / justify | `:1725`–`:1728` | Nothing. ⚑ On the web, Ctrl+Shift+R (hard reload) is kept even where the command does nothing | as E1 | as E3 | **B** |
| E17 | Mod-Alt-1 … 6 | Heading 1–6 | `:1729`–`:1734` | Nothing | as E1 | as E3 | **B** |
| E18 | Escape inside a node view | Gives the keyboard back to the editor, caret after the node | `rinch-editor-view/src/handle/node_view_bridge.rs:103` | No equivalent (like leaving a nested widget) | the view's `on_key` | the view's `on_key` | A |
| E19 | Printable keys in a read-only editor | Refused but consumed. On the web its propagation is stopped too | web `keyboard.rs:394`; desktop the same | A `readonly` field types nothing, but the `keydown` still bubbles to the page's listeners | on_key | `on_key` | **C**: ⚑ on the web the page's own letter shortcuts never see these keys. This was intended (`editor.md` line 101) so that Space would not scroll, but it differs from Chrome |
| E20 | ContextMenu / Shift+F10 (web) | Goes to the app's `oncontextmenu`, otherwise the native menu opens at the caret | web `keyboard.rs:272`–`:297` | Native menu | — | `oncontextmenu` | A |
| E21 | Mod on the web = Ctrl **or** Meta on every OS | On macOS, Ctrl+B, Ctrl+E and Ctrl+A run bold, code and select-all. On Windows, Win+key chords that reach the page run editor commands | web `keyboard.rs:248` | On macOS, Ctrl+A/E/B/F/K/N/P are Cocoa's emacs caret keys. Chrome uses only Cmd for formatting | as E1 | none, apart from `on_key` | **C**: wrong per-platform modifier. Desktop has no emacs keys either (a gap, not a claim) |

Not key bindings, so not counted: markdown input rules (typed text), already
opt-in since #2210 (`set_markdown_shortcuts`).

What an app can do about a keymap binding today: rebind the chord in a later
plugin, which replaces it; drop the **whole** base keymap together with its
commands (`remove_plugin(PluginKey("base-commands"))`); or swallow the key in
`on_key`. **There is no way to unbind one chord**, which `editor.md` lists as
"left for later" (`unbind_key`). The only way to let a chord reach the browser
is to rebind it to a command that does not exist, declared in
`fall_through_keys`. Nothing tests that route.

### Components (`rinch-components/src`, both platforms)

"Dismiss stack" Escape is offered **document-wide** while the overlay is open,
from step 4. Focus does not have to be inside the overlay.

| ID | Component and key | What it does | File:line | Scope | WAI-ARIA / Chrome | Precedence | Off switch | Class |
|---|---|---|---|---|---|---|---|---|
| K1 | Modal, Drawer: Escape | `onclose` | `modal.rs:325`, `drawer.rs:315` → `overlay_dismiss.rs:38` | popup, document-wide | Dialog: Escape closes. `<dialog>`'s `cancel` event can be cancelled | el, int | `close_on_escape: false` | A |
| K2 | Modal, Drawer: Tab, plus focus on open and restore on close | Traps Tab in the dialog | `modal.rs:301`, `drawer.rs:291` → `overlay_focus.rs:69` | popup | Modal dialog | el, int, ak (runtime Tab) | `trap_focus: false` | A |
| K3 | Popover: Escape | `onclose`, even with focus outside the popover | `popover.rs:369` | popup, document-wide | `popover=auto` light dismiss (document-wide too) | el, int | `close_on_escape: false` | A |
| K4 | Select (component), open list: arrows, Home/End, Enter, Space, Tab, type-ahead, Escape | Highlight, commit, close | `select.rs:485`–`:557` | popup + focus | Select-only combobox | el, int | **none** | A. ⚑ no option. It does not open on ArrowDown or Alt+ArrowDown, which WAI-ARIA expects |
| K5 | ColorInput: Escape | Closes the dropdown | `color_input.rs:512` | popup | Escape closes | el, int | **none**, deliberately ungated | A. ⚑ no option |
| K6 | DropdownMenu, ContextMenu: arrows, Home/End, letter type-ahead (a single match activates) | Item navigation | `menu_nav.rs:235`–`:259`, listener `:360` | ⚑ **app-wide while open** (an `on_app_key` listener; focus can be anywhere; text fields excluded). ContextMenu never closes on focus leave, so it keeps these keys until closed | Menu: same, with focus in the menu. A native context menu grabs the keys | el, int, and `on_app_key` listeners registered before the menu opened | `keyboard_nav: false`, `typeahead: false`, `loop_focus`, `on_key` | A |
| K7 | DropdownMenu, ContextMenu: Escape | Closes the menu and returns focus | `menu_nav.rs:371` | popup, document-wide | Same | el, int | `close_on_escape: false` | A |
| K7b | The DOM menu bar (`rinch/src/menu/app_menu_bar.rs`): Escape | Closes the open menu. Declines while no menu is open, so Escape falls through | `app_menu_bar.rs:316` (dismiss stack) | popup, document-wide | A native menu bar closes on Escape | el, int | none (only while a menu is open) | A |
| K8 | DropdownMenuSub: ArrowRight/Enter/Space on its row; ArrowLeft/Escape in its flyout; Escape document-wide through the dismiss stack (`push_dismiss_handler`; `_within` would only order nested entries by DOM position, `rinch-core/src/events/dismiss.rs:193`, and is document-wide too) | Opens or closes the submenu | `dropdown_menu_sub.rs:950`, `:961`, `:345` | control, popup | APG submenu; Chrome closes the innermost menu | ⚑ **element `onkeydown`, so it runs before every app hook except the app's own `onkeydown` deeper in the tree** (desktop) | **none** (`disabled` stops it opening, but the key is still consumed) | A. ⚑ no option |
| K9 | SortableItem: Space/Enter lift and drop; arrows move; Escape cancels; Tab cancels without being consumed | Keyboard drag | `sortable.rs:2307`–`:2356` | control | No browser or APG equivalent (the dnd-kit convention) | element `onkeydown`, as K8 | `lift_keys`, `disabled`. The arrows and Escape have **no** option | **C**: no Chrome equivalent. A UX choice that is only partly configurable |
| K10 | Tree: arrows, Home/End, PageUp/PageDown (±10), `*`, Space, Enter, Shift+run, type-ahead | Tree navigation and selection | `tree.rs:2513`–`:3007` | control | APG tree (PageUp/PageDown is an extra) | key entry: el, int | `keyboard: false` is the default, so it is opt-in; `handle_key` | A |
| K11 | Tree: Ctrl/Cmd+A | Selects the cursor row's **siblings** | `tree.rs:2935` | control | APG: Ctrl+A selects all | as K10 | opt-in (`keyboard`, `multiple`, `select_on_click`); `handle_key` | **C**: different from APG, though scoped and opt-in |

Components with no key handling of their own: Tabs (no roving arrows),
Accordion, Slider, NumberInput (no ArrowUp/ArrowDown stepping), HoverCard and
Tooltip (no Escape), Stepper, Pagination. These are gaps against WAI-ARIA, not
claims, and are out of this audit's scope.

Excluded: `rinch::hotkey::HotkeyCapture` (`rinch/src/hotkey/capture.rs:74`,
Escape cancels a capture). It is a state machine the app feeds from its own
key handler while capturing; rinch binds no key there.

### Graph view (`rinch-graph-view/src/canvas.rs`, both platforms)

All of these are `canvas_key` (`:474`–`:520`), an element `data-onkeydown` on
the canvas root (`:187`). They apply only while the canvas has focus. "Mod" is
Ctrl or Meta on every OS (`k.ctrl || k.meta`). The graph view has **no option**
for any of them. Precedence on both backends today: element `onkeydown`, so it
runs before the interceptor (step 3 before step 4 on desktop; `:3994` before
`:4013` on the web) and before `on_app_key`. Decisions on these rows are
proposals; see [section 6](#6-proposed-not-yet-decided-the-graph-view-k12k18).

| ID | Key | What it does | File:line | Chrome / platform | Off switch | Class |
|---|---|---|---|---|---|---|
| K12 | Delete, Backspace | Deletes the selection. Consumed even when nothing is selected | `canvas.rs:477` | No browser equivalent; the convention of canvas editors | none | **C**: always consumed |
| K13 | Mod+Z, Mod+Shift+Z | Undo, redo | `:481`–`:482` | A control's own undo, like a field's | none | A |
| K14 | Mod+Y | Redo, Cmd+Y included on macOS | `:483` | Redo on Windows and Linux; **on macOS, Cmd+Y opens History** (as R11) | none | **C**: as R11 |
| K15 | Mod+A | Selects every node | `:484` | A control's select-all | none | A |
| K16 | Escape | Cancels an active drag, else clears the selection | `:488` | Escape cancels or deselects | none | A |
| K17 | `+`, `=`, `-` (any modifiers), `0` (not with Mod) | Zoom in, zoom out, fit | `:496`–`:507` | No browser equivalent. ⚑ With Ctrl held, `+`/`=`/`-` hide Chrome's page zoom (Ctrl+= / Ctrl+-) on the web | none | **C** |
| K18 | Arrows (Shift: ×10) | Nudges the selected nodes. Falls through when nothing is selected | `:508`–`:520` | Moving a selected item, as drawing apps do | none | A |

### Web-only runtime

| ID | Key | What it does | File:line | Chrome | Precedence | Off switch | Class |
|---|---|---|---|---|---|---|---|
| W1 | Every keyup while a render surface has focus | `preventDefault` + `stopPropagation`, whether or not the surface claimed the press | `event_delegation.rs:4187`–`:4201` | A `<canvas>` lets keyups bubble | page capture listeners only | none | **C**: ⚑ the page's own `window` keyup listeners go deaf. Desktop withholds only a claimed key's release (#1784) |

## Counts

59 bindings in all ("Components" includes the graph view, K12–K18):

| Class | Runtime | Editor | Components | Web-only | Total |
|---|---|---|---|---|---|
| A | 15 (R1, R3–R10, R12–R17) | 10 (E1–E8, E18, E20) | 14 (K1–K8, K7b, K10, K13, K15, K16, K18) | 0 | **39** |
| B | 1 (R2) | 7 (E9, E10, E13–E17) | 0 | 0 | **8** |
| C | 2 (R11, R18) | 4 (E11, E12, E19, E21) | 5 (K9, K11, K12, K14, K17) | 1 (W1) | **12** |

R17 is a precedence rule (a focused control keeps its bare keys over the
app's menu accelerators) rather than one key. It is counted as one row.

## Findings

### Where rinch beats the app today (⚑)

1. **Web: `preventDefault` is ignored.** No rinch keydown handler reads
   `defaultPrevented`, so the browser's own way of saying "mine" does not
   work against any rinch binding. A page has to stop propagation, from a
   capture listener installed early enough. Every (A) binding on the web
   fails the "the app can intercept it" half of the rule in this way.
2. **Web: the editor keymap runs before the interceptor and before
   `on_app_key`.** On desktop the interceptor beats it. On the web only
   `EditorHandle::on_key` does. The same app code therefore behaves
   differently on the two platforms.
3. **No rinch capture-phase hook, on either backend.** The interceptor runs
   after element `onkeydown` on desktop (`:1964` before `:1991`) and after
   element `data-onkeydown` on the web (`event_delegation.rs:3994` before
   `:4013`). So a component's own `onkeydown` (K8 DropdownMenuSub, K9
   Sortable, K12–K18 graph view) cannot be pre-empted through rinch at all.
   On the web a page can still add its own `document` capture listener; on
   desktop there is nothing.
4. **`on_app_key` never sees editor keys** (both platforms), nor those of an
   open `<select>` popup or the text context menu.
5. **Escape ahead of the app:** R4 (pointer lock) runs before every handler,
   menu accelerators included; R3 (drag) before every DOM handler, after the
   accelerators. Both match Chrome, so no change is proposed.
6. **R17:** a focused control keeps its bare keys over the app's own menu
   accelerator. This matches native controls. `ChordYield::TextOnly`
   releases non-text owners only.
7. **R18, Android:** every key answers `Handled`, and Back becomes Escape.
   The system's Back and, it seems, the volume keys never run.
8. **E10, E14, E16 on the web:** browser chords (Ctrl+E, Ctrl+Shift+B,
   Ctrl+Shift+R) are kept even when the command does nothing. K17 does the
   same to Ctrl+= and Ctrl+- (page zoom) while a graph canvas has focus.
9. **E19, W1 on the web:** keys and keyups are stopped before they reach the
   page's own listeners.
10. **R2:** in a debug build, Alt+I (Option+I) beats text input in a focused
    field.

### Bindings with no way to turn them off

These have no setting at all. An app can stop them only by consuming each key
first, where some earlier hook exists, or not at all:

- R3 and R4: by Chrome parity.
- R18: the Android `Handled` answer.
- W1.
- The editor's caret, delete, clipboard and Page keys (E1–E3, E7): `on_key`
  only.
- E21: the web's Ctrl-or-Meta Mod.
- E19.
- K4 (Select's list keys), K5 (ColorInput Escape), K8 (DropdownMenuSub),
  and the arrows and Escape of K9.
- K12–K18: every graph view key.
- **Any single editor keymap chord.** There is no `unbind_key`, and
  `remove_plugin` drops the whole base keymap together with its commands.

### (B): app-level shortcuts that rinch claims

- **R2 (Alt+I, inspect mode):** debug-only, with `App::devtools` and
  `RINCH_DEVTOOLS`, but it beats text input in a field in a debug build
  (finding 10). Removed in the plan. R1 (F12) is class A: Chrome has it.
- **E9, E10, E13–E17:** the editor's formatting chords beyond Chrome's
  Mod-B/I/U: strikethrough, code, three list types, blockquote, paragraph,
  four alignments and six headings, 17 chords in all. A Chrome
  `contenteditable` has none of them. They are consumed even where they do
  nothing, and three of them hide browser chords on the web.

### (C): unclear

- **R11:** Cmd+Y redo on macOS, where Chrome opens History.
- **R18:** Android Back and `Handled`. This needs a device check, and a
  decision on what Back does when nothing takes it.
- **E11:** Mod-Enter `exitCode`. No Chrome equivalent, but it is code-block
  control and already falls through.
- **E12:** Tab indenting in lists and tables. A recorded exception, with a
  switch that couples tables and lists.
- **E19:** a read-only editor stopping letters on the web.
- **E21:** the web's Mod is Ctrl or Meta on every OS.
- **K9:** Sortable's keyboard drag keys. No browser equivalent, and only
  partly configurable.
- **K11:** Tree's Ctrl+A selects siblings rather than all.
- **W1:** keyups swallowed by a focused surface on the web.
- **K12, K14, K17:** the graph view's always-consumed Delete/Backspace, macOS
  Cmd+Y, and zoom keys that take Ctrl+= / Ctrl+-. Proposals in section 6.

What is done about each finding is in [The plan](#the-plan).
