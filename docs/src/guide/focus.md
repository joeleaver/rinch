# Keyboard Focus

Exactly one thing in a rinch window owns the keyboard at a time. The runtime
calls that the **focus arbiter**: an `<input>`, a `<select>` with its popup
open, a rich-text editor, a render surface, or a generic focusable DOM node.
Moving focus tears the previous owner down before installing the next, so two
widgets can never both believe they are typing.

Most of the time you never touch this — put `tabindex="0"` on an element and it
joins the Tab order, paints `:focus`, and activates on Enter/Space. This page is
about the other case: a **custom component that takes keyboard input of its
own** (a code editor, a canvas grid, a keyboard-driven list) and needs to know
when it gained the keyboard, when it lost it, and what keys arrived meanwhile.

## Making an element focusable

```rust
rsx! {
    div { tabindex: "0", "I can be tabbed to" }
}
```

Focusability comes from the **tag** or from an explicit **`tabindex`**, and an
explicit one always wins — the browser rule.

| Focusable by tag | |
|---|---|
| `<button>`, `<select>`, `<textarea>`, `<input>` | always |
| `<a>` | only with a non-empty `href` — a bare `<a>` is not a link |

`<summary>` is deliberately **not** in the set: rinch has no `<details>`
disclosure behaviour, so a focusable `<summary>` would be a Tab stop that does
nothing.

A focusable element that is **visually hidden but laid out** — `opacity: 0`, or
`position: absolute` off-screen — *is* in the Tab order, and that is correct:
`.sr-only` text and skip links depend on exactly that, as they do in a browser.
What leaves the order is a zero-sized box, `display: none`, and
`visibility: hidden`/`collapse`. Neither is `data-rid` a focusability signal — the `DropdownMenu`'s
full-screen dismissal backdrop carries one, and so do clickable cards, table
rows and list items; none of those should be Tab stops.

| Attribute | Effect |
|---|---|
| `tabindex="0"` | Reachable by Tab, focusable by click and by `NodeHandle::focus()`. Needed on anything that is not focusable by tag — a `div` you are driving yourself |
| `tabindex="-1"` | **Not** in the Tab order, but still focusable by click and programmatically — the standard "focus this dialog when it opens" idiom, and the way to take a `<button>` *out* of the Tab order |
| `disabled` / `data-disabled` | Takes no focus at all, and accepts no keyboard edit. Both spellings count — the component library writes the HTML one, the runtime's own widgets write the `data-` one. A boolean attribute: present means disabled whatever the value; only the explicit `"false"` opts out |
| `readonly` | Focuses, moves its caret, selects and copies like any other field — and refuses every command that would change its text (typing, delete, cut, paste, undo/redo). Same boolean rule |
| `data-nofocus` | A press here takes the **click** but not the keyboard: whatever is focused stays focused. Same boolean rule. Read anywhere on the pressed element's ancestor chain, so a toolbar carries it once |

A **disabled `<fieldset>`** disables every control below it, which is what the
element is for. HTML's carve-out is honoured: controls inside the fieldset's
first `<legend>` stay enabled, so the control that re-enables the section can
live there. No other tag's `disabled` reaches its subtree — a disabled
`<button>` does not disable a `<span>` inside it.

A press on a disabled control also paints no DOM `:focus`, so a focus ring
never appears on something that owns no keyboard.

Disabled is re-checked at **edit** time, not only at focus time, so a field
that goes disabled *while focused* — a reactive `disabled` prop re-rendering
under a live caret — stops accepting keys immediately, and **releases the
keyboard**, the way a browser moves focus to the body. The one thing that
release does *not* do is fire the field's `data-onchange` commit: everywhere
else that commit is load-bearing (a window blur deliberately keeps the claim so
alt-tabbing cannot fire it), but a control going disabled is not the user
committing an edit, and browsers dispatch no `change` for it either.

Focus arrives three ways, and all three go through the same arbiter:

- **Tab / Shift+Tab** — walks the focusable elements in DOM order, and paints
  the `:focus-visible` keyboard ring.
- **A mouse press** — claims the *nearest focusable ancestor-or-self* of
  whatever was hit, exactly as a browser does. Pointer focus does **not** paint
  the `:focus-visible` ring. A press that resolves to something *other* than
  the current claim holder takes the keyboard away from it, and a press that
  resolves to nothing releases it — a nested focusable inside a focused node
  counts as "somewhere else", for every mouse button alike.

  This applies to `tabindex="-1"` too — it keeps an element out of the Tab
  order, but it does not keep a click from focusing it, in rinch or in a
  browser. A control that must **not** steal focus from the field it sits
  beside — a toolbar button over a rich-text editor, a spinner next to a number
  input — needs [`data-nofocus`](#taking-the-click-without-the-keyboard).
- **`node.focus()` / `request_focus(node_id)`** — programmatic, also no ring.

A focused `<select>` is **closed**, like a browser's: Enter, Space or Alt+Down
opens its popup, and the popup then owns the keyboard until it commits or is
dismissed — at which point focus returns to the closed control, so Tab carries
on from there rather than restarting. (A click *outside* the popup is the
exception: it belongs to whatever it landed on.) Everything else focusable activates the nearest ancestor-or-self
`data-rid` on Enter/Space, which is what makes `div { tabindex: "0", onclick: … }`
behave like a button — and what makes Space on a `Checkbox`'s visually hidden
`<input>` toggle the `<label>` that wraps it.

> **Still not matched to the web.** A positive `tabindex` does not order ahead
> of DOM order — the collector is a plain pre-order walk (issue #435) — and a
> Modal's or Drawer's backdrop does not contain Tab, so controls behind it stay
> reachable. Arrow/Enter/Escape navigation of the `Select` component's open
> option list is issue #434.

### Taking the click without the keyboard

An editor toolbar has a problem every GUI toolkit has to answer: pressing
**Bold** must run the command *without* blurring the editor, or the command
reads a selection that is no longer there. Browsers answer it with
`preventDefault()` on `mousedown`, which suppresses the focus change while
still delivering the click.

`data-nofocus` is that mechanism:

```rust
rsx! {
    // The whole toolbar opts out at once — every control inside it takes its
    // click without taking the keyboard.
    div { data-nofocus: "", class: "toolbar",
        button { tabindex: "0", onclick: move || ed.command("toggleBold"), "B" }
        button { tabindex: "0", onclick: move || ed.command("toggleItalic"), "I" }
    }
    Editor { editor: ed.clone() }
}
```

The rules:

- It is read **anywhere on the pressed element's ancestor chain**, so a toolbar
  carries it once instead of every button in it. Put it on the toolbar, not on
  a big content region — a press inside it suppresses the browser's default,
  which includes starting a text selection.
- It protects **whatever holds the keyboard** — the rich-text editor, an
  `<input>`, a render surface, another focusable node — not just the editor.
- The **click still fires**. `data-rid` dispatch is untouched.
- A **text field inside** a `data-nofocus` region still focuses normally. A
  link-URL field in a toolbar has to be usable, so the field's own claim wins
  over the region's opt-out.
- Boolean attribute, same rule as `data-disabled`: present means on whatever
  the value, only the explicit `"false"` opts out.
- It works on **both backends**. On the web it becomes `preventDefault()` on
  the `pointerdown`.

## Registering a focus target

`register_focus_target` attaches callbacks to a focusable node. It does not
change *who* can be focused — it changes what your component is told about it.

```rust
use rinch::prelude::*;

#[component]
fn key_grid() -> NodeHandle {
    let focused = Signal::new(false);
    let cursor = Signal::new(0usize);

    let grid = rsx! {
        div {
            tabindex: "0",
            class: {move || if focused.get() { "grid focused" } else { "grid" }},
            "cell "
            {move || cursor.get().to_string()}
        }
    };

    register_focus_target(
        &grid,
        FocusEntry::new()
            .on_focus_gained(move || focused.set(true))
            .on_focus_lost(move || focused.set(false))
            .on_key(move |k| match k.key.as_str() {
                "ArrowRight" => {
                    cursor.update(|c| *c += 1);
                    true // consumed
                }
                "ArrowLeft" => {
                    cursor.update(|c| *c = c.saturating_sub(1));
                    true
                }
                _ => false, // let the runtime have it
            }),
    );

    grid
}
```

Every callback is optional; `FocusEntry::new()` with none of them is legal and
registers the node without asking for anything back.

### What fires, and when

| Callback | Fires |
|---|---|
| `on_focus_gained` | Tab onto the node, a press on it (or on any of its children), `focus()` / `request_focus`, and when the **window** regains OS focus while this target still holds the claim |
| `on_focus_lost` | Anything else takes the keyboard — another registered widget, an `<input>`, a `<select>`, the rich-text editor, a render surface — a press landing outside, and when the **window** loses OS focus |
| `on_key` | Every `KeyDown` **and `KeyUp`** while this target holds focus, **before** the runtime's own handling — read `k.kind` (or `k.is_up()`) to tell them apart |
| `on_ime` | Every IME composition event while this target holds focus (see [IME](#ime) below) |

### Presses and releases

`k.kind` is `KeyEventKind::Down` or `Up`; `k.is_down()` / `k.is_up()` are the
shorthands. **OS auto-repeat arrives as `Down`**, and nothing yet distinguishes
it from a fresh press — the browser supplies a flag but the desktop platform
event does not carry winit's, so exposing one would be truthful on web and
silently wrong on desktop. It arrives with that plumbing.

A press and its release are spelled by the same rule, from the same fields, so
**pairing them by `k.key` works by construction** — which is what "is W still
held" needs. Concretely: the platform event's `logical_key` carries the full
layout-produced `KeyboardEvent.key` value on the press and the release alike,
case included. On AZERTY, the key labelled A is `"a"` on the way down *and* on
the way up, not `"a"` down and `"q"` up; `Shift+A` is `"A"` both ways, and
`Shift+1` is `"!"` both ways — the same strings a browser reports, so the same
consumer code works against `rinch-web`. Case is identity, so a Shift pressed
*mid-hold* changes what the eventual release spells (`"w"` down, `"W"` up) —
exactly as in a browser; track held keys by the physical `k.code`, or fold
case at the comparison, if that matters to you.

Two things to know:

- A release is delivered to whoever holds the claim **at release time**. A
  focus change mid-chord can therefore hand a target a release it never saw
  pressed — treat `on_focus_lost` as "everything is up" if you track held keys.
- **A release's return value is ignored.** There is nothing downstream of it to
  suppress, and the runtime's own release work (clearing the Enter/Space
  activation latch) must happen whatever a handler thinks — otherwise a
  consumed release would strand the latch and swallow the next press.

`on_key` returns `true` to **consume** the key. A consumed key stops there: no
Tab navigation, no Enter/Space activation, no DevTools shortcut. Returning
`false` (or not setting `on_key` at all) leaves every one of those working
exactly as it would for an unregistered node, so registering costs you nothing
you did not ask for.

`k.key` is spelled the way the browser spells `KeyboardEvent.key` — with one
long-standing exception, the spacebar, which rinch names `"Space"` where a
browser reports `" "` (so `rinch-web`, which forwards `event.key()`
unchanged, reports `" "` there). It is resolved in four steps:

1. **A named key wins over the text it would insert** — `"ArrowLeft"`,
   `"Enter"`, `"Escape"`, `"Tab"`, `"PageUp"`, `"F1"`…`"F12"`, `"Shift"`, and
   `"Space"` (not `" "`).
2. **Otherwise the inserted text wins**, so a non-QWERTY layout reports the
   letter actually typed rather than the physical QWERTY position: the AZERTY
   key at the QWERTY-Q position is `k.key == "a"`, and Shift+A is `"A"`.
3. **Otherwise the layout's key value.** A modifier suppresses the inserted
   text, but the layout-produced `KeyboardEvent.key` value survives it — so a
   chord keeps step 2's promise: on AZERTY, `Ctrl` plus the key labelled A is
   `k.key == "a"` (and `Ctrl+Shift` makes it `"A"` — the value is
   case-accurate), the same letter the editor's own keymap acts on. It also
   names shifted punctuation (`Ctrl+Shift+1` is `"!"` where the layout puts
   one), a dead key (`"Dead"`), and keys rinch has no `KeyCode` of its own for
   but the platform names — CapsLock, media keys.
4. **Otherwise the physical key's US-layout character**, for events that carry
   no layout value at all (the debug channel, injected and embedded events):
   `Ctrl+S` is `"s"`, `Cmd+1` is `"1"`, `Ctrl+-` is `"-"`.

`k.code` is always the physical key (`"KeyS"`, `"Digit1"`).

> Two things to know. `k.key` is **case-accurate**, browser-style —
> `Ctrl+S` is `"s"`, `Ctrl+Shift+S` is `"S"` — and a press and its release
> spell alike (both read the modifier state of their own moment), which is
> what pairing them by `k.key` relies on. And a key bound to a **native menu
> accelerator** is consumed by the menu before the document sees it — but only
> the *press*: the release still arrives (the menu consumes nothing on the way
> up), so it is one more source of a release with no visible press.

The one key that still reports nothing is one rinch has no `KeyCode` for
(`k.code == "Other"`) arriving with **no layout value and no inserted text**.
A real keystroke carries the layout value (unless the platform itself cannot
identify the key), so this is mostly the injected regime: the debug channel names only single characters and the named keys, so
an injected `Ctrl+/` has no spelling to fall back to and never reaches the
hook. From the keyboard those keys are fine: `Ctrl+/` reports `"/"` through
step 3, and unmodified the character arrives as the inserted text.

Both focus callbacks run **after** the transition is complete: the arbiter and
the DOM `:focus` state are already installed, so a callback may re-enter the
runtime freely — move focus again, mutate the DOM, save a document.

### Enter and Space

A focused node that does **not** consume Enter/Space gets the runtime's default:
they dispatch the `onclick` handler of the nearest ancestor-or-self that has
one, once per physical press. That is what makes `div { tabindex: "0", onclick:
… }` behave like a button. If your widget wants Enter for itself, consume it in
`on_key`.

### Unmounting

Unmounting the component **deregisters silently**. `on_focus_lost` does *not*
fire, even if the node held the keyboard at the time — by then the component's
scope has been disposed and its signals freed, and calling back into it would
panic. Do your teardown in the component's own cleanup path, not in
`on_focus_lost`.

The arbiter notices the vanished target on its next key dispatch and drops the
claim, so keys fall back to the global handlers rather than disappearing.

### Window focus

When the window loses OS focus, the focused widget **keeps** its claim — it is
only *notified*, and notified again when the window comes back. This is browser
behaviour, and it is deliberate: releasing focus on every alt-tab would fire
`onchange` on whatever field the user was typing in each time they switched
windows.

So the pair to expect is `on_focus_lost` on window blur, `on_focus_gained` on
window refocus, with no key routing in between. Use it to hide a caret and idle
a blink timer. While the window is blurred, rinch reports the OS IME disabled,
so a candidate box follows the window that actually has the keyboard.

The runtime does the same for its own caret: the rich-text editor's caret stops
blinking while the window is blurred and shows **solid**, resuming from the
solid phase on refocus. The blink is the event loop's only timed wake, so a
backgrounded rinch app now idles completely instead of waking twice a second to
animate a caret nobody can type into. The selection highlight is unaffected —
the claim is still held, so a blurred window still shows what is selected.

### IME

A widget with its own text model can take **IME composition** — CJK conversion,
autocorrect, dead keys, a swipe keyboard — by registering `on_ime`. That is what
declares the target a *text* target: the runtime then switches the platform
input method on while the widget holds the keyboard, and routes every one of the
five portable `ImeEvent` variants to it — the same contract the rich-text editor
and a built-in `<input>` consume. A registration without `on_ime` turns nothing on, so a focusable card
or a custom checkbox never pops a candidate window.

```rust
use rinch::prelude::*;

register_focus_target(
    &node,
    FocusEntry::new()
        // Where the OS puts its candidate box, in logical window pixels.
        .caret_rect(move || Some(caret.get()))
        .on_ime(move |e| match e {
            // A transient overlay you render and discard — never document text.
            ImeEvent::Preedit { text, cursor } => model.set_preedit(text, *cursor),
            // The conversion the user chose. Insert it as one edit.
            ImeEvent::Commit(text) => model.insert(text),
            // Composition ended with nothing committed.
            ImeEvent::Disabled => model.clear_preedit(),
            _ => {}
        })
        .on_focus_lost(move || model.clear_preedit()),
);
```

`caret_rect` is `(x, y, w, h)` in **logical window space** — CSS pixels from the
window's top-left, the same space layout bounds are reported in. Do not
pre-multiply by the display scale factor; the shell hands the rect to the
platform as a logical size and the platform scales it. Return `None` when there
is no caret; placement then falls back to the platform's default. The provider
is polled every time rinch reconciles the window's IME state (once per
event-loop iteration), so the candidate box follows the caret with nothing to
notify — keep it cheap, and do not mutate the DOM from it.

Two things the runtime deliberately does **not** do:

- **It never fabricates an event.** It holds no preedit on your behalf, so a
  focus change is not an `ImeEvent::Disabled`. When another target claims the
  keyboard the window's input method may stay enabled throughout and nothing
  ends your composition — clear the preedit in `on_focus_lost`, as above.
- **`ImeEvent::DeleteSurrounding` is inert on desktop today.** Rinch asks winit
  only for cursor-area support, so no desktop backend advertises
  surrounding-text and the variant never arrives. Android's `InputConnection`
  does send it.

Everything under [Window focus](#window-focus) applies: while the window is
blurred, IME reports disabled and no composition is routed, but the claim — and
`on_ime` with it — comes back on refocus.

## Where this does *not* apply

- **The browser backend (`rinch-web`).** There is no arbiter there because the
  browser is one: `register_focus_target` is a desktop / Android / embed API.
  On web, give the element a real `tabindex` and use the DOM's own `focus`,
  `blur` and `keydown` events.
- **The document-level keyboard hook.** `set_keyboard_interceptor` is a
  capture-phase hook for the whole document, dispatched *before* the arbiter and
  regardless of focus. It is for global shortcuts; `on_key` is for a focused
  widget. They are different jobs and both still exist. It routes per document
  (issue #340): a hook registered while a document's events are being
  dispatched intercepts only that document's keys, and one registered from
  `main` or at mount is the thread-global fallback that intercepts for every
  document without its own — so two windows on one thread no longer clobber
  each other's registration. Its *lifetime* does match the arbiter's, though:
  registering it during a render releases it when that component unmounts,
  exactly as a `FocusEntry` is deregistered (issue #183). Registering it from
  `main` keeps app lifetime.
- **IME on the browser backend.** `on_ime` is desktop / Android / embed only,
  like the rest of this API. On web, attach `compositionstart` /
  `compositionupdate` / `compositionend` to your element yourself — the browser
  delivers composition to whatever it considers focused.
- **The Android soft keyboard.** A registered target participates in desktop
  IME, but does not yet raise Android's on-screen keyboard: the shell still
  watches for a focused `<input>` or the rich-text editor.
- **Modal containment.** Tab still reaches controls behind a `Modal`, `Drawer`
  or `DropdownMenu` backdrop; the backdrop blocks pointer hits only. Tracked
  separately.
