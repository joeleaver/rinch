# Testing Your App's Views

`rinch::testing` mounts a component in a headless app — no window, no display,
no GPU — and lets a plain `#[test]` send it input and read the document back.
The input goes through the same dispatch the desktop runtime uses, so a test
answers what real input does: which element a press lands on, which handler
runs, where focus goes, what a key does in a field.

## Setup

The harness is behind the `testing` feature, which needs `desktop` (the
default). Turn it on for your tests only:

```toml
[dev-dependencies]
rinch = { workspace = true, features = ["desktop", "testing"] }
```

It adds no dependencies. A web build that turns it on gets no `rinch::testing`
module and compiles as before.

## A first test

```rust
use rinch::prelude::*;
use rinch::testing::TestApp;

#[test]
fn the_counter_counts() {
    let mut app = TestApp::mount(|__scope: &mut RenderScope| {
        let count = Signal::new(0);
        rsx! {
            div {
                button { id: "inc", onclick: move || count.update(|n| *n += 1), "+" }
                p { id: "out", {move || format!("Count: {}", count.get())} }
            }
        }
    });

    app.click_on("#inc");
    app.click_on("#inc");
    assert_eq!(app.text("#out"), "Count: 2");
}
```

`TestApp::mount` mounts in an 800x600 window at scale 1. For anything else, use
the builder:

```rust
let mut app = TestApp::builder(my_view)
    .size(400, 300)
    .scale_factor(2.0)
    .fonts(&[AppFont::sans_serif(include_bytes!("../assets/Inter.ttf"))])
    .theme(ThemeProviderProps::default()) // with the `theme` feature
    .mount();
```

## Which handler runs

The case the harness was written for (issue #2553): a list whose rows have
`onmousedown`/`onmouseup`, inside a container that has them too. A press on a
row runs the row's handlers and not the container's: the nearest element with
the handler takes the press. Only a test that goes through rinch's dispatch can
show this; calling the closures yourself cannot.

```rust
let log = Rc::new(RefCell::new(Vec::<&str>::new()));
let l = log.clone();
let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
    let (a, b, c, d) = (l.clone(), l.clone(), l.clone(), l.clone());
    rsx! {
        div {
            id: "list",
            style: "padding: 20px; width: 300px",
            onmousedown: move || a.borrow_mut().push("list down"),
            onmouseup: move || b.borrow_mut().push("list up"),
            div {
                id: "row",
                style: "height: 30px",
                onmousedown: move || c.borrow_mut().push("row down"),
                onmouseup: move || d.borrow_mut().push("row up"),
                "First result"
            }
        }
    }
});

app.click_on("#row");
assert_eq!(*log.borrow(), ["row down", "row up"]);

log.borrow_mut().clear();
let list = app.bounds("#list");
app.click(list.x + 5.0, list.y + 5.0); // the list's own padding
assert_eq!(*log.borrow(), ["list down", "list up"]);
```

## Naming what to act on

Every method that acts on an element takes a **target**: a CSS selector
(`"#row"`, `".item:nth-child(2)"`, the first match) or a `NodeHandle` — one
the component kept, or one from `app.query(..)` / `app.query_all(..)`. A
selector that matches nothing panics with the selector in the message.

## Sending input

All positions are logical (CSS) pixels, at any scale factor — the space
`bounds` answers in, so read a box and press inside it without converting.

| Method | What the window receives |
|---|---|
| `mouse_move(x, y)`, `hover(target)` | a pointer move |
| `mouse_down(x, y, button)`, `mouse_up(..)` | a press or release there (the pointer moves there first, as a real one would have) |
| `press(target, button)`, `release(target, button)` | the same, at the centre of `target`'s box |
| `click(x, y)`, `click_on(target)`, `right_click(x, y)` | press and release |
| `wheel(x, y, dx, dy)`, `wheel_on(target, dx, dy)` | a trackpad scroll of that many pixels; positive `dy` scrolls the content down (the DOM's `deltaY`) |
| `wheel_lines(x, y, lx, ly)` | a mouse wheel's notches, which glide |
| `set_modifiers(ModifierState { .. })` | modifier keys held from now on, for keys, clicks and wheels |
| `key_down(key)`, `key_up(key)`, `press_key(key)` | a key, by its `KeyboardEvent.key` name: `"a"`, `"Enter"`, `"Escape"`, `"ArrowDown"`, … |
| `type_text("hi\n")` | one keystroke per character (`\n` is Enter, `\t` Tab) |
| `ime_commit(text)` | an input method committing text to the focused field |
| `focus(target)` | `NodeHandle::focus()`, as app code calls it |
| `window_focus(bool)`, `resize(w, h)` | the window gaining or losing focus, or resizing |

A press "on" an element is a press at the centre of its box, and goes through
hit testing like any press: if something else is painted over that point, that
is what gets it. That is the point — but it means a target covered by an
overlay is not reached, and a test that wants a different point uses `bounds`
and `click(x, y)`.

A key a menu shortcut owns goes to the menu first and the app sees nothing, as
with a real keyboard.

After each input the harness runs one turn of the event loop: the queue of
main-thread callbacks, `AboutToWait` (transitions, animations, layout), and a
frame painted with the software renderer when one was asked
for. When the call returns, effects have run and layout is current.
`app.settle()` runs further turns until the app stops asking for frames — for a
change made outside input, or a short transition.

## Reading the document

| Method | Answer |
|---|---|
| `query(selector)`, `query_all(selector)` | `NodeHandle`s, in document order |
| `handle(target)` | a `NodeHandle`, for the rest of its API (`scroll_top`, `children`, `live_value`, …) |
| `text(target)` | `textContent` |
| `attribute(target, name)` | an attribute |
| `value(target)` | an `<input>`/`<textarea>`/`<select>` value as the user left it |
| `computed_style(target, name)` | a computed property, in the form the MCP `get_computed_styles` tool reports (`"display"`, `"background-color"`) |
| `bounds(target)`, `center(target)` | the painted border box (`getBoundingClientRect()`), logical pixels |
| `focused()` | the element with keyboard focus |
| `take_actions()` | what the app asked the window for (a cursor, a window drag, exit) |

`app.app()` hands out the underlying `RinchApp` for anything the harness does
not wrap.

## Limits

- **One window.** The native menu bar, the tray, other windows, and the
  operating system are not there. Menu *shortcuts* are matched.
- **Real time.** Transitions and animations run on the wall clock, as in the
  app; `settle()` runs turns, it does not skip ahead.
- **No timers.** `set_timeout` and `set_interval` do not fire: their native
  scheduler hands the callback back to the main thread the runtime registers,
  and a test has none.
- **The main-thread queue is process-wide.** A `Signal::send` from a worker
  thread hops through one queue that every test thread drains, so another
  test's turn can run it on the wrong thread. A test that sends from a worker
  should run alone in its process, or under `cargo test -- --test-threads=1`.
- **Render surfaces** (`RenderSurface`, `GameViewport`) are laid out but their
  per-frame render callbacks do not run.
- **Text widths depend on fonts.** A test asserting horizontal text geometry
  should bundle its font with `.fonts(..)`, or its numbers move with the host's
  installed fonts.
