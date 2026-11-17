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

## The window

`.window_props(props)` mounts the component in the window an `App` with those
`WindowProps` would open: its `width`/`height` (`.size(..)` wins over them),
`visible: false` (mounted, but not painted and without the keyboard until it is
shown), `non_activating` (shown without taking the keyboard), `size_to_content`
(the window takes its content's size and follows it) and `transparent`. The
other props are kept and do nothing: there is no compositor.

```rust
let mut app = TestApp::builder(launcher)
    .window_props(WindowProps { visible: false, ..WindowProps::default() })
    .mount();
assert!(!app.is_visible());
```

The window publishes its state as the desktop shell does, so a component that
reads `rinch::window::window_focused()`, `window_size()` or `scale_factor()`
follows `app.window_focus(..)`, `app.resize(..)` and `app.set_scale_factor(..)`.

What the app's code asks of its own window — `hide_current_window()`,
`show_current_window()`, `prepare_current_window()`, a size, a position, a
`WindowHandle::set_title`, `rinch::exit()` — is recorded as a `WindowRequest`
in `app.take_window_requests()`. Showing and hiding are applied too: a hide
makes `app.is_visible()` false and takes the keyboard (a hidden window has
none), a show makes it true, paints it and gives it the keyboard unless it is
non-activating; `window_focus(true)` does nothing while the window is hidden.
A request made through the `WindowHandle` of a window that is not open (closed,
or never made) is recorded as `WindowRequest::OtherWindow { window, request }`
and never applied to the test's window; one for a window the component opened
and that is open is that window's (see
[Windows a component opens](#windows-a-component-opens)). A prepare paints the hidden window; a size request resizes a
window that is not sized by its content. A launcher that hides when it loses
the keyboard:

```rust
let mut app = TestApp::mount(|__scope: &mut RenderScope| {
    // Not before the window first has the keyboard: it is mounted unfocused.
    let had_it = std::cell::Cell::new(false);
    rinch::Effect::new(move || {
        let focused = rinch::window::window_focused();
        if had_it.replace(focused) && !focused {
            rinch::windows::hide_current_window();
        }
    });
    rsx! { input { id: "query" } }
});

app.window_focus(false);
assert_eq!(app.take_window_requests(), [WindowRequest::Hide]);
assert!(!app.is_visible());
```

## Windows a component opens

A `Window { … }` element, or `open_window`, opens a child window, and the same
`TestApp` drives it: it shares the app's thread, reactive runtime and turns.
It is made by the shell's own code up to its OS window, and closed by the
shell's own teardown; it has no OS window and no renderer, and is not painted.

```rust
let mut app = TestApp::mount(|__scope: &mut RenderScope| {
    rsx! {
        div {
            Window { title: "Details".to_owned(), width: 300, height: 200,
                button { id: "ok", onclick: || println!("ok"), "OK" }
            }
        }
    }
});
let details = app.windows()[0];
let mut window = app.window(details);
assert_eq!(window.props().title, "Details");
window.click_on("#ok");
window.resize(400, 250);
```

- **When it opens.** After the turn that built its element, when the window
  requests are served, never inside that turn, as the shell makes it on a later
  wake. `app.windows()` lists the open children, oldest first;
  `app.window(handle)` or `app.window_by_id(id)` borrows one as a `TestWindow`,
  with `props()`, `take_requests()`, `query`, `text`, `attribute`, `bounds`,
  clicks and keys in its own document.
- **Its first facts come in the shell's order.** It is mounted at the main
  window's scale with no surface: a reader made in its mount sees
  `window_physical_size()` `(0, 0)`. Then it gets the configure the shell's
  `create_window` gives it, the surface and a `Resized` at its props' size.
  Then whatever the test plays as the compositor: `resize(w, h)`,
  `set_scale(s)`, and `set_surface_size(w, h)`, which is the surface for the
  next `resize` or `set_scale` only, published with it in one batch; after it
  the stand-in (the logical size times the scale, rounded) returns. There is no
  option to mount a child at another scale than the main window's: the shell
  mounts it at the main window's too (unless its content sizes it), and
  `set_scale` after it gives the reads the shell gives for an output whose
  scale differs.
- **Each turn** gives every open child its `AboutToWait` and, when its layout
  is pending, the style, layout and bounds the shell resolves before a frame,
  short of painting: `bounds()` and `bounds_signal()` readers are current.
- **Its requests are its own.** What its code asks of its window, through its
  `WindowHandle` or with `rinch::windows` in its document, is in its
  `take_requests()`, in the order made, and no longer in the app's
  `take_window_requests()` as `WindowRequest::OtherWindow`. A test that read a
  child's requests there reads them from the child now.
- **Requests are applied as the shell's own code would without an OS.** The
  harness stands in a Wayland compositor with layer shell: a window the shell
  would ask a layer surface for (`wayland_layer` set, or a non-activating
  window) is one, and its handle's `surface_kind()` says so; any other is a
  toplevel. A toplevel's size request is applied at once, as the shell feeds
  back the size winit grants it; a layer surface's size is the compositor's to
  give, so it is recorded only and the test gives it with `resize`. A
  compositor without layer shell (GNOME's toplevel fallback) is not played. The
  handle's `size()` is the logical size the window was last configured at, and
  its `position()` is `None`.
- **What is recorded only.** A child's show, hide, prepare, focus, title,
  position, layer and output requests, and a layer surface's size, are
  recorded in its `take_requests()` and not applied. A child has no visibility
  and no keyboard focus in the harness: it is not painted, and
  `window_focused()` read in a child stays false. The main window does apply
  show and hide; a child does not.
- **Input and modifiers.** Input sent with a `TestWindow` goes to that
  window's app, and what the app answers is taken as the shell takes it (an
  `Exit` closes the window). Each window holds its own modifiers, as the shell
  reports them to the window with the keyboard: `window.set_modifiers(..)` for
  a child, `app.set_modifiers(..)` for the main window. A `TestWindow` names
  its window by id: used after the window closed it panics, naming it, and a
  click, a keystroke or typed text stops at the step after which the window
  closed (a press that closes it sends no release). A `NodeHandle` from one
  window's document used on another panics, since a node id means a node in
  one document only.
- **Closing.** A child closes when its handle closes it (its `on_closed` runs),
  when its owner's scope goes (its `on_closed` does not run), when its app
  agrees to its close control (`close_current_window`), and when
  `close_by_system()` plays the system's close: a layer surface's `closed`,
  with no veto, or a toplevel's title-bar close, which its
  `on_close_requested` may refuse. It is then no longer listed, its handle's
  `is_open()` is false, and its facts are gone.

## Threads

Tests in one binary run in parallel, and so can their apps: each `TestApp`'s
thread is a host of its own, whose turns drain only the main-thread work meant
for it. A `Signal::send` or `update_send` goes to the app whose thread created
the signal and runs there, whether it was sent from the test's own thread
(applied at once), from a worker thread the test spawned, or from another
test's thread. A component fed from another thread is tested as it runs:

```rust
let slot = Rc::new(Cell::new(None));
let s = slot.clone();
let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
    let status = Signal::new(String::from("idle"));
    s.set(Some(status));
    rsx! { p { id: "status", {|| status.get()} } }
});
let status = slot.get().unwrap();

std::thread::spawn(move || status.send(String::from("done")))
    .join()
    .unwrap();
app.settle(); // the worker's write is applied at the app's next turn
assert_eq!(app.text("#status"), "done");
```

No `--test-threads=1` is needed. The one exception is in [Limits](#limits).

## Time

`set_timeout` and `set_interval` run on a virtual clock that starts at zero
when the app is mounted. `app.advance_time(duration)` moves it forward and
fires every timer that comes due on the way, in deadline order (timers due
together in the order they were armed), with a turn of the loop after each
deadline: effects, layout and window requests are current before a later timer
fires. The timers due at one instant fire in one turn, and a timer armed by
another fires too if it comes due inside the advance. A
timeout of 0 fires in the turn after the input that armed it, as on the
desktop. `app.elapsed()` is the clock's time.

```rust
let mut app = TestApp::mount(|__scope: &mut RenderScope| {
    let seconds = Signal::new(0);
    set_interval(1000, move || seconds.update(|s| *s += 1));
    rsx! { p { id: "clock", {move || seconds.get().to_string()} } }
});
app.advance_time(Duration::from_millis(2500));
assert_eq!(app.text("#clock"), "2");
```

The clock is the test thread's, so tests running in parallel keep their own.
Two `TestApp`s alive at once on one thread share one clock: the second starts
at the first's time, and advancing either moves both.

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
| `key_down(key)`, `key_up(key)`, `press_key(key)` | a key, by its `KeyboardEvent.key` name: `"a"`, `"Enter"`, `"Space"`, `"Escape"`, `"ArrowDown"`, `"F1"`–`"F12"`, … With Shift held a letter types its capital |
| `type_text("hi\n")` | one keystroke per character (`\n` is Enter, `\t` Tab) |
| `ime_commit(text)` | an input method committing text to the focused field |
| `focus(target)` | `NodeHandle::focus()`, as app code calls it |
| `window_focus(bool)`, `resize(w, h)`, `set_scale_factor(s)` | the window gaining or losing focus, resizing, or moving to a display with another scale |
| `advance_time(duration)` | the virtual clock moving forward, firing the timers due on the way |

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
| `computed_style(target, name)` | a computed property, in the form the MCP `get_computed_styles` tool reports (`"display"`, `"background-color"`). Unstable: these are rinch's internal names and spellings |
| `bounds(target)`, `center(target)` | the painted border box (`getBoundingClientRect()`), logical pixels |
| `focused()` | the element with keyboard focus |
| `take_actions()` | what the app asked the window for (a cursor, a window drag, exit) |
| `take_window_requests()` | what the app's code asked of its own window (`WindowRequest::Show`, `Hide`, `Prepare`, …) |
| `is_visible()`, `is_window_focused()` | whether the window is shown, and has the keyboard |
| `size()`, `scale_factor()`, `elapsed()` | the window's logical size and scale, and the virtual clock's time |
| `window_props()` | the `WindowProps` the app was mounted with, as given (the layer surface, the app id, …) |

`app.app()` hands out the underlying `RinchApp` for anything the harness does
not wrap.

## Limits

- **No OS windows.** The main window and the windows a component opens are
  driven without one; the native menu bar, the tray and the operating system
  are not there. Menu *shortcuts* are matched. A window a component opens is
  not painted.
- **Transitions and animations run on the wall clock**, as in the app;
  `settle()` runs turns, it does not skip ahead. Only timers are on the
  virtual clock.
- **A timer re-arming itself with no delay** never lets `advance_time` reach
  its end; it panics after 100 000 timers in one call.
- **A worker's `run_on_main_thread` names no app.** Tests run in parallel, a
  `TestApp` on each test thread, and each app drains only its own main-thread
  work: a `Signal::send` or `update_send` reaches the app whose thread created
  the signal, from any thread (see [Threads](#threads)). A closure a worker
  thread queues with `run_on_main_thread` (or `rinch_core::dispatch_main_callback`)
  is the exception: it belongs to no thread, and whichever test's turn drains
  first runs it. A signal it writes (`run_on_main_thread(move || s.set(..))`)
  is then another thread's, and the write is dropped with a warning; write
  from a worker with `s.send(..)`.
- **Render surfaces** (`RenderSurface`, `GameViewport`) are laid out but their
  per-frame render callbacks do not run.
- **Text widths depend on fonts.** A test asserting horizontal text geometry
  should bundle its font with `.fonts(..)`, or its numbers move with the host's
  installed fonts.
- **Punctuation chords** (`Ctrl+/`) do not reach a menu shortcut; letters,
  digits and named keys do. The key still reaches the app.
- **Window actions are recorded, not performed.** A right click in a text field
  shows up as `AppAction::ShowTextContextMenu` in `take_actions()`; no menu
  opens. The same goes for cursor changes, window drags and exit, and for the
  window requests other than show, hide, prepare, focus and size (a position,
  a layer surface, a minimize), which are only in `take_window_requests()`.
- **No compositor.** A layer surface, an output, `always_on_top` and the like
  are kept in the props and have no effect; a content-sized window's largest
  size is its `ContentSizing` maximum or else the props' size, as with no
  output listed.
