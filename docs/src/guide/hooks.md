# State Management

There are no hooks. There is no re-rendering. Your component function runs once, builds the DOM, and closures keep it updated forever. Here's how you manage state.

## Core Primitives

| Primitive | What it does |
|-----------|-------------|
| `Signal::new(value)` | Reactive state. Read it, write it, closures that read it re-run when it changes. |
| `Memo::new(closure)` | Cached derived state. Like a Signal you can't write to. |
| `create_store(value)` | Share a store struct across components. |
| `use_store::<T>()` | Access a shared store from any descendant. |
| `create_context(value)` | Low-level shared state (mostly for framework internals). |

> **For shared state, use [stores](./stores.md).** A store is a struct with Signal fields and methods that mutate them. Clean, testable, no prop drilling.

## Signal

The foundational reactive primitive. Create one, read it in closures, and those closures re-run when the value changes.

```rust
let count = Signal::new(0);

count.get();                 // Read
count.set(5);                // Write
count.update(|n| *n += 1);  // Read-modify-write
```

`Signal<T>` is `Copy`. Use it in as many closures as you want — no `.clone()` needed.

```rust
#[component]
fn toggle() -> NodeHandle {
    let on = Signal::new(false);

    rsx! {
        button { onclick: move || on.update(|b| *b = !*b),
            {|| if on.get() { "ON" } else { "OFF" }}
        }
    }
}
```

### Cross-Thread Dispatch

`Signal::set()` and `Signal::update()` must be called from the main thread. From a background thread, use `send()` and `update_send()`:

```rust
let progress = Signal::new(0);

std::thread::spawn(move || {
    for i in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        progress.send(i);  // Dispatches to main thread automatically
    }
});
```

For more details, see [Signals](./signals.md).

## Memo

Cached derived state that recomputes only when its dependencies change.

```rust
let first = Signal::new("Alice".to_string());
let last = Signal::new("Smith".to_string());

let full = Memo::new(move || format!("{} {}", first.get(), last.get()));
// full.get() recomputes only when first or last change
```

`Memo<T>` is also `Copy`. Dependencies are tracked automatically — no dependency arrays.

For more details, see [Memos](./memos.md).

## Effect (Advanced)

Most reactive DOM updates happen through `{|| expr}` closures in RSX. For the rare case where you need to react to signal changes outside of the DOM (logging, syncing to an external system), use `Effect`:

```rust
use rinch::reactive::Effect;

let count = Signal::new(0);

Effect::new(move || {
    println!("Count is now: {}", count.get());
});
```

Effect is intentionally excluded from the prelude. If you're reaching for it, ask yourself: can this be a closure in RSX, or a method on a store? Usually the answer is yes.

For more details, see [Effects](./effects.md).

## Lifetimes: What Owns Your State

State created during a render belongs to the thing being rendered, and dies with
it. You rarely have to think about this — it is what stops a list that churns
10,000 rows from leaking 10,000 signals — but it becomes visible the moment a
handle outlives its component.

**The rule:** a `Signal`, `Memo`, `Effect` or event handler created while
something is rendering is *owned* by that render. When the owner goes away — a
`for` row is reconciled out, an `if` branch flips, a component unmounts — every
resource it owned is freed.

**No ambient owner means app lifetime.** A signal created in `main()`, in startup
code, or in a detached callback has no owner and lives as long as the thread.
That is why hoisted, app-wide state keeps working with no ceremony.

### Reading a freed handle panics; writing to one does not

`Signal<T>` is `Copy`, so nothing stops a handle from outliving what created it —
a background thread, a timer, a global registry. Reads and writes deliberately
behave differently:

```rust
count.get();       // panics if the signal was freed
count.set(5);      // no-op plus a warning if the signal was freed
```

> **You may always write to a handle; you may only read a live one.**

The asymmetry is not an oversight. A background thread that calls `send()` every
50ms cannot be asked to notice that the UI moved on, so a write to a dead signal
must be survivable. A read has nothing to return — `T` need not implement
`Default` — so it asserts.

When a handle may legitimately be gone, ask instead of assuming:

```rust
if let Some(n) = count.try_get() {       // None if freed
    println!("still here: {n}");
}
let len = items.try_with(|v| v.len());   // Option<usize> — None if freed
if count.is_alive() {                    // no subscription is created
    schedule_more_work();
}
```

`Memo<T>` has `try_get()` and `is_alive()` too. `is_alive()` never subscribes the
current observer — liveness is not reactive, and tracking it would resurrect the
dependency you are trying to drop.

### Global callback registries are released too

A callback handed to a process-wide registry is the classic way for a component's
state to outlive the component — the registry keeps the closure, the closure keeps
a `Signal`, and the next event reads a handle whose storage is gone. Registering
one **from inside a render** therefore ties it to that render:

```rust
// `set_keyboard_interceptor` is not in the prelude — it is a document-level
// capture-phase hook, not everyday component API. `set_paste_interceptor` is.
use rinch::core::set_keyboard_interceptor;

#[component]
fn shortcuts() -> NodeHandle {
    let count = Signal::new(0);
    // Released when this component unmounts — the interceptor cannot outlive
    // `count` and read it after it is freed.
    set_keyboard_interceptor(move |k| {
        if k.key == "j" { count.update(|n| *n += 1); true } else { false }
    });
    rsx! { div { {|| count.get().to_string()} } }
}
```

This applies to [`set_keyboard_interceptor`], [`set_paste_interceptor`],
`set_selection_callback` and `set_selection_sync_callback` (issue #183). Two
consequences are worth knowing:

- **Registering with no ambient owner still means app lifetime.** A hook installed
  from `main()`, from startup code or from a detached callback has no owner, so
  nothing removes it — unchanged, and what app-wide shortcuts rely on.
- **An earlier component unmounting never clears a later one's registration.**
  These are single, last-wins slots; the release only reclaims the slot if it is
  still holding the callback that registered it.
- **Register once per component, not once per event.** Each call from inside a
  live component queues its own release, and those accumulate until the component
  unmounts. Installing a hook from a render (as above) is the intended shape;
  re-installing one on every click or on every run of a reactive closure is not.

`rinch-ws` callbacks work the same way from the outside — an `on_message`
registered inside a component stops firing once that component unmounts, even if
the `WsHandle` was parked somewhere longer-lived:

```rust
// `WsHandle` is neither `Clone` nor `Copy`, so a store holds it behind an `Rc`:
//     #[derive(Clone)] struct AppStore { socket: Rc<WsHandle> }
#[component]
fn feed() -> NodeHandle {
    let lines = Signal::new(Vec::<String>::new());
    // The socket outlives this component: the store holds the handle open.
    let ws = use_store::<AppStore>().socket;
    // Holds `lines`, which this component owns, so it must stop firing when
    // this component goes away.
    ws.on_message(move |m| {
        if let Some(t) = m.as_text() {
            lines.update(|v| v.push(t.to_string()));
        }
    });
    rsx! { div { {|| lines.get().join("\n")} } }
}
```

The mechanism underneath differs, and the difference is visible in one place.
The interceptor slots above *release* the callback when the scope disposes; a
WebSocket callback is checked at **dispatch** and dropped by the first event that
arrives after the component is gone. That is deliberate: a socket's callbacks can
be re-registered as often as the app likes, and queueing a release per
registration would grow without bound. The practical consequence is that the
"register once per component" advice above does **not** apply to `rinch-ws` — and
that a callback belonging to an unmounted component is released on the next
event rather than at unmount.

Each of a connection's four callbacks carries its own owner, so one handle may be
shared by two components — a message list registering `on_message`, a
connection-status badge registering `on_close` — and each stops on its own
component's unmount rather than on the other's. A slot that will never be
dispatched again (`on_open` after the handshake) is released by the first event
that finds *any* of the connection's callbacks dead.

### Android platform callbacks

The `rinch-android` services follow the `rinch-ws` rule, for the same reason —
they are registered from live components, often more than once:

```rust
use rinch_android::sensors::{DELAY_UI, SensorType};

#[component]
fn compass() -> NodeHandle {
    let heading = Signal::new([0.0f32; 3]);
    // Stops firing, and is released, when this component unmounts. Before, only
    // an explicit `stop()` removed it, and the callback kept reading `heading`
    // after it had been freed.
    rinch_android::sensors::start(SensorType::MagneticField, DELAY_UI, move |d| {
        heading.set([d.values[0], d.values[1], d.values[2]]);
    });
    rsx! { div { {|| format!("{:?}", heading.get())} } }
}
```

This covers `sensors::start`, `location::start`, `lifecycle::on_pause` /
`on_resume`, and the one-shot results behind `camera::pick_image`,
`camera::take_photo`, `file_picker::pick_file` / `save_file` and
`permissions::request_permission`. Four things follow:

- **The one-shots are covered too.** They are removed on delivery, which bounds
  the leak but not the lifetime: Android delivers a result whatever the user does
  — a cancelled picker still reports `RESULT_CANCELED` — so a picker opened by a
  component that has since unmounted used to be delivered exactly once, into that
  component's freed state. It is now discarded instead.
- **Release does not wait for an event.** Every one of these registries is
  drained once a frame, and each drain also releases what unmounted components
  left behind. A sensor that has fallen silent, an activity result that never
  comes back and an app that never backgrounds would otherwise pin their
  callbacks — and everything captured — for the life of the process. Each
  release is logged at `debug`, because the symptom is otherwise silent: the
  callback simply stops firing.
- **Releasing a sensor or location callback powers the hardware down**, because
  nothing else can. Once the entry is gone, `sensors::stop` has no
  `SensorType` to be called with and the component that knew it is disposed —
  so the release calls `stopSensor` / `stopLocationUpdates` itself, rather than
  freeing a `Box` and leaving a radio on. Only what it actually released: a
  sensor another component is still using stays armed, including one
  re-registered from inside the released callback's own `Drop`.
- **Registration from `android_main` keeps app lifetime**, unchanged. That is
  what an app-wide `on_pause` autosave relies on.
- **Stopping from inside the callback works.** "Stop the sensor once the reading
  crosses a threshold", or "stop location once the fix is accurate enough", used
  to panic with a `BorrowMutError`, because the registry was borrowed across the
  call. So did swapping a lifecycle handler from inside one.

`sensors::stop` and `location::stop` remain the way to stop early — the moment
you have the fix you wanted, rather than at unmount. What they no longer have to
be is a leak-preventing ritual.

[`set_keyboard_interceptor`]: ./focus.md#where-this-does-not-apply
[`set_paste_interceptor`]: ./platform.md#clipboard

### Opting out

Sometimes a resource is *meant* to outlive the component that created it — state
handed to a longer-lived store, a registration something else owns:

```rust
let s = Signal::new(0).leak();       // this one signal gets app lifetime
unowned(|| { /* nothing in here is owned by the current render */ });
```

`leak()` searches the owner stack *as it stands now*, so call it in the same
render that created the signal; from a later callback the stack is empty and it
does nothing. `unowned` is a free function at the crate root rather than in the
prelude, deliberately: it is an attractive nuisance that converts a lifetime bug
into a permanent leak. Reach for `try_get()` / `is_alive()` first.

### Effects are the exception

Dropping an `Effect` handle does **not** stop the effect. Disposal is explicit
(`effect.dispose()`) or comes from the owning scope. See
[Effects](./effects.md#disposing-effects).

## Context

Share state across components without prop drilling. The ancestor creates it, any descendant reads it.

```rust
#[derive(Clone)]
struct AppConfig {
    api_url: String,
}

#[component]
fn app() -> NodeHandle {
    create_context(AppConfig { api_url: "https://api.example.com".into() });
    rsx! { div { ChildComponent {} } }
}

#[component]
fn ChildComponent() -> NodeHandle {
    let config = use_context::<AppConfig>();
    rsx! { Text { {config.api_url.clone()} } }
}
```

`use_context::<T>()` panics if the context is missing (with a helpful message). Use `try_use_context::<T>()` for an `Option<T>` instead.

For reactive shared state, use [stores](./stores.md) — they're contexts with Signal fields and methods.

## Putting It Together

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let todos = Signal::new(vec!["Learn Rinch".to_string()]);
    let input = Signal::new(String::new());
    let count = Memo::new(move || todos.get().len());

    let add = move || {
        let text = input.get();
        if !text.is_empty() {
            todos.update(|t| t.push(text.clone()));
            input.set(String::new());
        }
    };

    rsx! {
        Stack { gap: "md", p: "xl",
            Title { order: 1, "Todos (" {|| count.get().to_string()} ")" }

            Group { gap: "sm",
                TextInput {
                    placeholder: "What needs doing?",
                    value_fn: move || input.get(),
                    oninput: move |v: String| input.set(v),
                    onsubmit: add,
                }
                Button { onclick: add, "Add" }
            }

            for todo in todos.get() {
                div { key: todo.clone(), {todo.clone()} }
            }
        }
    }
}
```

Signal is `Copy`, so `add` captures `todos` and `input` without ceremony. `count` is a Memo that recomputes when `todos` changes. The `for` loop is reactive — add or remove a todo and only the affected DOM nodes change.
