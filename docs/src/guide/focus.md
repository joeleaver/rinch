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
| `disabled` / `data-disabled` | Takes no focus at all, and accepts no keyboard edit. Both spellings count — the component library writes the HTML one, the runtime's own widgets write the `data-` one |
| `readonly` | Focuses, moves its caret, selects and copies like any other field — and refuses every command that would change its text (typing, delete, cut, paste, undo/redo) |
| `data-nofocus` | A press here takes the **click** but not the keyboard: whatever is focused stays focused. Read anywhere on the pressed element's ancestor chain, so a toolbar carries it once |
| `data-trap-focus` | Tab and Shift+Tab cycle **inside** this element instead of walking the page. What `Modal`, `Drawer` and `Popover` write while they are open — see [Containing Tab inside an overlay](#containing-tab-inside-an-overlay) |

All five are **boolean attributes**: their presence is their value, so
`disabled`, `disabled=""` and `disabled="disabled"` say the same thing. To say
*enabled*, remove the attribute — a `bool` in `rsx!` does that for you
(`button { disabled: {move || busy.get()} }`).

The two HTML attributes, `disabled` and `readonly`, are read by **presence
alone**, exactly as a browser reads them: `disabled="false"` disables and
`readonly="false"` is read-only, on desktop and on the web alike (issue #612).
That applies to every tag the attribute reaches, so a `<textarea readonly="false">`
is read-only too.

rinch's **own** `data-disabled`, `data-nofocus` and `data-trap-focus` are the
exception, and the only one: there the literal `"false"` turns the attribute off.
`data-nofocus` and `data-trap-focus` are read that way on both backends;
`data-disabled` is a desktop attribute with no web reader, because the browser
does not know it. Only `"false"` is excused — `"0"` is on, matching the web's
`[data-nofocus="false" i]` selector. Reach for the escape
when you are writing the attribute **by hand with `set_attribute`** and would
otherwise have to branch between writing and removing it; a `bool` in `rsx!`
already removes it for you.

A **disabled `<fieldset>`** disables every control below it, which is what the
element is for. HTML's carve-out is honoured: controls inside the fieldset's
first `<legend>` stay enabled, so the control that re-enables the section can
live there. No other tag's `disabled` reaches its subtree — a disabled
`<button>` does not disable a `<span>` inside it.

A press on a disabled control also paints no DOM `:focus`, so a focus ring
never appears on something that owns no keyboard.

The same rule drives the CSS **`:disabled`** and **`:enabled`** pseudo-classes,
so a control that refuses input also *looks* disabled — `input:disabled { ... }`
matches, and a control below a disabled `<fieldset>` matches with it (the first
`<legend>` exemption included). Styling is deliberately **narrower** than focus
in one way: HTML defines both pseudo-classes over form elements only, so a
`<div data-disabled>` is still removed from the Tab order but does **not** match
`:disabled` — matching it would style on desktop what a browser, and therefore
rinch-web, leaves alone.

A native **`<select>`** refuses the whole popup interaction, not just the last
step: a disabled one opens no popup, and one disabled while its popup is
already up dismisses it and commits nothing rather than firing `oninput` /
`onchange`.

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
> of DOM order — the collector is a plain pre-order walk (issue #435).
> Arrow/Enter/Escape navigation of the `Select` component's open option list is
> issue #434.

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
  the value, and the explicit `"false"` opts out — one of rinch's own three
  attributes where that escape exists.
- It works on **both backends**. On the web it becomes `preventDefault()` on
  the `pointerdown`.

### Containing Tab inside an overlay

A dialog that lets Tab walk out into the page behind it is not really a dialog:
the keyboard user ends up editing a form they cannot see. `Modal`, `Drawer` and
`Popover` all take a **`trap_focus`** prop for this — `true` by default on the
first two, `false` on `Popover`, which is not modal.

```rust
rsx! {
    Modal {
        opened_fn: move || open.get(),
        onclose: move || open.set(false),
        trap_focus: true,          // the default; `false` lets Tab out
        TextInput { placeholder: "Name" }
        Button { onclick: move || save(), "Save" }
    }
}
```

While the overlay is open its root carries **`data-trap-focus`**, and both
backends read it: Tab and Shift+Tab cycle the focusable elements inside that
element and wrap at its ends, instead of walking the whole document. When it
closes the attribute is removed and Tab goes back to the page.

You can put the attribute on anything, not just these three components — it is a
plain contract, the way `data-nofocus` is:

```rust
div { data-trap-focus: "", class: "command-palette",
    input { }
    button { "Run" }
}
```

The rules:

- **Which trap wins.** Whichever one currently holds the keyboard; if none does,
  the **last** one in document order. Nesting therefore resolves inside-out — a
  modal opened from a modal is rendered deeper, so later — and when the inner one
  closes the outer takes over with focus unmoved. `z-index` is not consulted, so
  a raised-but-earlier overlay loses to a later one; that matches the [dismiss
  stack](#the-dismiss-stack), which answers Escape in the same order.
- **A trap with no box is not a trap.** A closed `Modal`'s root is
  `display: none`, so it is skipped even if the attribute were left on it. Both
  guards are real, and the component relies on the first: it *removes* the
  attribute rather than writing `"false"` into it.
- **Only Tab is contained.** A **click** outside the overlay still moves focus
  out of it, and so does a scripted `focus()`. That is what a *non-modal*
  `<dialog>` does; a browser's `showModal()` goes further and marks the rest of
  the page `inert`, so even a scripted focus behind it is refused — measured in
  Chrome 150. rinch has no `inert` and no `showModal` semantics, so `trap_focus`
  is a Tab rule, not a modality barrier. The backdrop's `close_on_click_outside`
  is what an outside click is for; full modality would need `inert`, which rinch
  does not have.
- **A trap with nothing focusable inside it swallows Tab.** That is what
  containment means when there is nowhere to go — and on the web the same is
  true of a trap whose every control the *browser* refuses to focus (all of them
  inside a `<fieldset disabled>`, say): the key is consumed and focus stays put,
  rather than the stepping-on above running forever.
- Boolean attribute, same rule as `data-nofocus`: present means on whatever the
  value, and the explicit `"false"` opts out.
- **Both backends, different code.** Desktop starts its own focusable walk at
  the trap; `rinch-web` collects the trap's focusable descendants with a CSS
  selector and calls `focus()` itself. The two sets are therefore computed
  separately and agree only as far as each is written to — desktop's notion of
  focusable is already documented as broader than HTML's in places. Trapping
  inherits that difference rather than creating it. On the web the **browser**
  is the authority, not rinch's selector: a `focus()` it declines (a control
  inside a `<fieldset disabled>`, a `tabindex` it parsed differently) is stepped
  over rather than trusted, so an element rinch listed and the browser will not
  focus cannot stall the cycle.
- **Some browser Tab stops are not rinch Tab stops**, on either backend:
  `contenteditable` elements, `<iframe>`, `<summary>`, `<audio controls>` and
  `<area href>` are reachable by Tab in a browser and are in neither backend's
  focusable set. An overlay containing one loses it while trapped — and outside
  a trap those elements are not desktop Tab stops either, so this is the
  focusable set's shape rather than something trapping introduces. Give such an
  element an explicit `tabindex="0"` if it has to be reachable.

### Moving focus in, and giving it back

Containment is half of what a dialog does. The other half is that opening one
*takes* the keyboard and closing it *gives it back* (issue #695), and both are
tied to the same `trap_focus` prop — `trap_focus` is rinch's spelling of "this
overlay is modal", and a browser moves focus for `showModal()` and not for
`show()`. An overlay that wants one without the other cannot ask for it.

**On open**, the overlay remembers whatever holds the keyboard — usually the
button that was clicked — and then focuses inside itself:

| Component | What it focuses |
|---|---|
| `Modal`, `Drawer` | the `autofocus` descendant, else the **first** focusable |
| `Popover` | the `autofocus` descendant, and **nothing** without one |

`Popover`'s rule is the HTML popover API's, not the dialog's: an `auto` popover
runs its focusing steps only for an element that asked. A popover that grabbed
the keyboard on every open would interrupt whatever the user was typing in.

**On close** — and on unmount while still open, which is what
`if show { Modal { … } }` does — the keyboard goes back to the remembered
element, provided it can still *take* it (see below). If it cannot, the claim is
released rather than left inside the overlay that has just gone. Focus the user
moved out of the overlay before it closed is left alone entirely: neither
released nor restored over, which is HTML's own dialog rule.

Nesting needs no special case: an inner overlay remembers whatever the outer one
focused, so closing the inner restores into the outer and closing the outer
restores to the page.

Three things to know about the machinery:

- **Desktop decides both a turn later.** An overlay opening or closing is a class
  change in the same effect flush, so at that instant its children still carry
  the zero-size boxes their `display: none` ancestor gave them — and on a close,
  whether the remembered opener can still take the keyboard is the same kind of
  question about boxes. The desktop backend therefore parks the request and
  resolves it after the next layout, which is the turn a signal write already
  triggers. `rinch-web` answers on the spot, because the browser lays out on
  demand and refuses a `focus()` it should refuse.
- **"Still there" means it can still take focus**, not merely that it is
  attached. An opener that went `disabled` while the dialog worked, or one that
  lives inside an *outer* overlay closed before this one, is connected and
  unreachable; the keyboard is released instead. What is *not* checked is
  identity: a node id freed and handed to a different, attached node would still
  be focused — the recycled-slot hazard of issue #304, which desktop has
  independently of this.
- **A `Popover` that declines the move is still a Tab trap** when `trap_focus` is
  on. Focus stays where it was, and the next Tab enters the popover and is
  contained there. That is containment's own rule (the last visible trap wins),
  not a consequence of the move policy.

The portable API this rests on is three `DomDocument` methods, reachable on any
`NodeHandle`: `active_element()`, `focus_into(policy)` and `restore_focus(opener)`.
A component that builds its own overlay can use them directly. There is
deliberately no bare `blur()`: releasing the keyboard is never the whole answer,
only the fallback half of a restore, and a caller that could only blur would have
to make the decision `restore_focus` exists to make.

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

## The dismiss stack

Escape is the one key an overlay has to answer without holding the keyboard: a
`Modal` is dismissed by Escape whether the focus is on a field inside it, on the
modal itself, or nowhere at all. That is a different job from both APIs above,
and it has its own registry (issue #474).

```rust
use rinch::prelude::*;

let opened = Signal::new(true);
let handle = push_dismiss_handler(root.doc_key(), move || {
    if opened.get() {
        opened.set(false);
        true            // consumed: the key stops here
    } else {
        false           // not open: pass it to the overlay below, then the app
    }
});
__scope.on_cleanup(move || drop(handle));
```

- **It is a stack, last in first out.** The most recently registered handler is
  asked first, so two nested modals close innermost-first. The first handler
  that returns `true` consumes the key and the ones beneath it are never asked.
- **Answer `false` while you are closed.** An overlay usually stays *mounted*
  when it closes — `opened_fn` only toggles a class — so the check belongs
  inside the handler, at dispatch time, not at registration. A handler that
  always returns `true` swallows Escape for the rest of the session.
- **It is released on unmount**, twice over: the [`DismissHandle`] releases it
  when it drops, and an entry whose owning scope has been disposed is dropped
  as the scan passes it rather than run (issue #183 — its captured signals are
  already freed).
- **It is scoped per document, on desktop and embed.** Two `RinchContext`s on
  one thread do not answer each other's Escape (issues #134, #139). On
  `rinch-web` there is nothing to scope and nothing doing the scoping: a page
  dispatches with no document marked, and the rule is deliberately permissive
  when either side is unmarked, so every handler is reached. The explicit key
  you pass at registration is what carries the distinction where one exists.
- **Both backends, no branch.** Dispatch lives inside
  `dispatch_keyboard_event`, which desktop calls ahead of the focus arbiter and
  `rinch-web` calls from its document `keydown` listener.

Some things are dispatched **before** the stack and will take Escape from it: a
`set_keyboard_interceptor` that consumes the key, and an in-progress drag, which
Escape cancels. Both apply on either backend. `rinch-web` has one more — a
focused `RenderSurface` swallows every key there, where on desktop a surface is
routed by the arbiter, i.e. *after* the stack. That asymmetry is older than the
stack (the interceptor already had it) and is not something this API changed.

### Register at mount, or at open

The snippet above registers once, when the overlay mounts, and answers `false`
while it is closed. That is what `Modal`, `Drawer` and `Popover` do, and it is
right for an overlay whose open state arrives as a *prop*: `render` runs once,
so there is no later moment to hang a registration on.

It is the wrong choice for an overlay **statically nested inside another one**.
A component renders *after* its children, so `Modal { ColorInput { … } }` pushes
the input's entry first and the modal's on top of it — and the stack is LIFO, so
the modal answers Escape while the colour picker is the thing on screen.

An overlay that owns its open state avoids that by pushing when it **opens** and
releasing when it closes. Opening happens after every ancestor has mounted, so
LIFO puts the entry where the user expects it with no precedence rule anywhere.
This is what the native `<select>` popup does (issue #671) and what
`ColorInput`'s dropdown does (issue #465); in a component,
`rinch_components::overlay_dismiss::arm_close_on_escape_while_open` is that
policy written out. Such a handler consumes unconditionally — it exists only
while the overlay is open, so it has no closed state to decline in — and the
release must cover **unmount-while-open** as well as close.

`Modal`, `Drawer` and `Popover` already do all of this for you through their
`close_on_escape` prop — reach for `push_dismiss_handler` when you are building
an overlay of your own. **Prefer it over `set_keyboard_interceptor` for
Escape**: the interceptor is a single slot per document, so a second overlay
registering there silently disables the first, and when it unmounts it clears
the slot rather than restoring what it displaced.

[`DismissHandle`]: https://docs.rs/rinch/latest/rinch/struct.DismissHandle.html

## Locking the page behind an overlay

The other thing an open dialog owes the page behind it is stillness. `Modal` and
`Drawer` do it for you through their `lock_scroll` prop (default on, issue
#474); a custom overlay takes the same lock on its own root:

```rust
// On open — the node matters: it names the subtree that stays scrollable.
root.set_scroll_locked(true);
// On close, and again on unmount, or the page never scrolls again.
root.set_scroll_locked(false);
```

- **Locks are counted.** Two overlays open and the inner one closing leaves the
  page locked. Every `true` must be matched by exactly one `false`.
- **Release it on unmount as well as on close.** An overlay can be taken out of
  the tree while still open, which is what `if show { Modal { .. } }` does. The
  effect that would have unlocked it never runs again, so the release belongs in
  an `on_cleanup` too. `Modal` and `Drawer` already do both.
- **The two backends do different things**, and this is the one overlay
  behaviour where that is visible. Desktop refuses the *gesture*: a wheel (which
  is what a touch scroll arrives as too) and a scrollbar-thumb press are
  rejected for any container that is not inside a locking overlay, so the
  dialog's own `overflow: auto` body still scrolls. `rinch-web` sets
  `overflow: hidden` on the real `<html>`, because rinch cannot gate the
  browser's own wheel.
- **So web also removes the page's scrollbar and desktop does not.** A web page
  with a classic (non-overlay) scrollbar shifts sideways when an overlay opens.
  Desktop's bars are overlays with no gutter, and a locked page's bar stays
  painted — visible, and inert.
- **Input only.** Programmatic scrolling (`set_scroll_top`, a list the app
  scrolls itself) is untouched on both.
- **A drag already in flight ends when a lock arrives.** A scrollbar thumb the
  user is holding when something else opens a dialog is released rather than
  left to go on scrolling the page.
- **In island mode the lock is still the whole page.** There is one `<html>`, so
  a rinch `Modal` inside an island freezes the host page, not just rinch's own
  region — one lock, counted across every island on the page. That is what every
  web overlay library does, but an embedder should know it before shipping a
  dialog into someone else's page.
- **On iOS Safari, `overflow: hidden` on `<html>` is not always enough** to stop
  rubber-band scrolling; libraries there add a `position: fixed` body trick. We
  have not tested it and rinch does not do it, so treat the web half as "the
  page stops scrolling in every browser we have measured" rather than as
  universal.

Do **not** reach for `RenderScope::body_handle()` and set `overflow: hidden`
there: on the web that handle is `<div id="rinch-body">`, a descendant of the
real `<body>`, and styling it does not stop the page scrolling.

**A scroll container the runtime portals to `<body>` is exempt**, because it is
not a descendant of any overlay's root: the native `<select>` popup registers
itself with `NodeTree::push_scroll_lock_exempt` while it is open, so a long
option list inside a dialog still scrolls. Anything else that portals a
*scrollable* element to the body needs the same, with the same
push-on-open / release-on-close lifetime.

The exemption has no portable spelling yet, so it covers the runtime's own
portals and nothing else. A scroll container of yours that sits **outside** the
locking overlay in the tree — window chrome, a panel anchored to the frame — is
refused while the lock is held; the Linux in-app menu bar's own dropdown is the
known instance, tracked in
[issue #701](https://github.com/joeleaver/rinch/issues/701).

## Where this does *not* apply

- **The browser backend (`rinch-web`).** There is no arbiter there because the
  browser is one: `register_focus_target` is a desktop / Android / embed API.
  On web, give the element a real `tabindex` and use the DOM's own `focus`,
  `blur` and `keydown` events. `data-trap-focus` is the exception — it works on
  both backends, because it is a contract the backend reads rather than a
  registration against the arbiter.
- **The document-level keyboard hook.** `set_keyboard_interceptor` is a
  capture-phase hook for the whole document, dispatched *before* the arbiter and
  regardless of focus. It is for global shortcuts; `on_key` is for a focused
  widget. They are different jobs and both still exist. It routes per document
  (issue #340): a hook registered while a document's events are being
  dispatched intercepts only that document's keys, and one registered from
  `main` or at mount is the thread-global fallback that intercepts for every
  document without its own — so two windows that each register from inside
  their own event handling no longer clobber each other. Registrations made
  outside any dispatch still share the single fallback slot, last-wins. Its *lifetime* does match the arbiter's, though:
  registering it during a render releases it when that component unmounts,
  exactly as a `FocusEntry` is deregistered (issue #183). Registering it from
  `main` keeps app lifetime. For **Escape**, use
  [the dismiss stack](#the-dismiss-stack) instead — one slot cannot nest.
- **IME on the browser backend.** `on_ime` is desktop / Android / embed only,
  like the rest of this API. On web, attach `compositionstart` /
  `compositionupdate` / `compositionend` to your element yourself — the browser
  delivers composition to whatever it considers focused.
- **The Android soft keyboard.** A registered target participates in desktop
  IME, but does not yet raise Android's on-screen keyboard: the shell still
  watches for a focused `<input>` or the rich-text editor.
- **Modality.** Tab **is** contained by an open `Modal` or `Drawer` — see
  [Containing Tab inside an overlay](#containing-tab-inside-an-overlay) — but a
  *click* still reaches controls behind the backdrop wherever the backdrop
  itself does not cover them, and a click or a scripted `focus()` outside an
  overlay moves focus out of it. A browser's `showModal()` refuses both, because
  it marks the rest of the page `inert`; rinch has no `inert` and models no
  `showModal`. `DropdownMenu` has no `trap_focus` prop and contains nothing.
