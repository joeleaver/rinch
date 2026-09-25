# Reactivity

Rinch uses a **fine-grained reactivity** model inspired by [Solid.js](https://www.solidjs.com/) and [Leptos](https://leptos.dev/). This means that when state changes, only the parts of your UI that depend on that state are updated—not the entire component tree.

## Why Fine-Grained Reactivity?

Traditional virtual DOM approaches (like React) re-render entire component subtrees when state changes, then diff the virtual DOM to find what changed. This works well but has overhead.

Fine-grained reactivity tracks dependencies at a granular level. When a signal changes, only the specific effects subscribed to that signal re-run. There's no diffing step—updates are direct.

**Benefits:**
- Minimal re-computation
- Predictable performance
- No dependency arrays to maintain
- Automatic subscription management

## Core Primitives

Rinch provides three core reactive primitives, plus a store pattern for shared state:

| Primitive | Purpose | When to Use |
|-----------|---------|-------------|
| [Signal](./signals.md) | Holds reactive state | For any mutable state |
| [Memo](./memos.md) | Caches computed values | For derived/computed state |
| [Stores](./stores.md) | Shared state with actions | For state used by multiple components |
| [Effect](./effects.md) | Runs side-effects | Advanced: syncing to external systems |

> **Note:** For DOM updates, use `{|| expr}` closures in rsx rather than `Effect`. For shared state, use [stores](./stores.md). `Effect` is a power-user escape hatch for rare cases like syncing to external systems.

## Quick Example

```rust
use rinch::prelude::*;

#[component]
fn counter() -> NodeHandle {
    // Create reactive state
    let count = Signal::new(0);

    // Create a derived value (Memo is Copy, just like Signal)
    let doubled = Memo::new(move || count.get() * 2);

    rsx! {
        div {
            // Use closure syntax {|| ...} for reactive updates
            // Both Signal and Memo are Copy — no .clone() needed
            p { "Count: " {|| count.get().to_string()} }
            p { "Doubled: " {|| doubled.get().to_string()} }

            button {
                onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}
```

> **Note:** The closure syntax `{|| expr}` is required for fine-grained reactive updates. Without it, values are captured once and never update. See [RSX Syntax - Reactive Expressions](./rsx-syntax.md#reactive-expressions) for details.

> **When are closures unnecessary?** Inside `for` loop bodies, components are re-created when the item data changes (via keyed reconciliation). Plain props from the loop variable don't need closures — only per-item Signals do. See [RSX Syntax - Reactivity in for Loops](./rsx-syntax.md#reactivity-in-for-loops) for details.

## How Dependency Tracking Works

1. When an **Effect** or **Memo** runs, it registers itself as the "current observer"
2. When a **Signal** is read (via `.get()`), it checks for a current observer
3. If there's an observer, the signal subscribes it
4. When the signal's value changes (via `.set()` or `.update()`), all subscribers are notified

This happens automatically—you never manually specify dependencies.

```
┌─────────────────┐        .get()         ┌─────────────────┐
│     Effect      │ ────────────────────► │     Signal      │
│                 │                        │                 │
│  (observer)     │ ◄──── subscribes ──── │  (subscribers)  │
└─────────────────┘                        └─────────────────┘
                                                   │
                                              .set() / .update()
                                                   │
                                                   ▼
                                           notify all subscribers
                                                   │
                                                   ▼
                                           effects re-run
```

### Dependencies are per-run

The dependency set is rebuilt on every run, not accumulated. An effect ends each
run subscribed to exactly the signals and memos it read *that* time, so a
dependency it stops reading stops waking it:

```rust
let show_details = Signal::new(true);
let details = Signal::new(String::new());

Effect::new(move || {
    if show_details.get() {
        render(details.get()); // subscribed only while the branch is taken
    }
});

show_details.set(false); // re-runs; `details` is no longer a dependency
details.set("...".into()); // does not re-run the effect
show_details.set(true);   // re-runs, and picks `details` back up
```

Disposal releases the subscriptions too — an effect (or a scope full of them)
that is disposed leaves nothing behind in the signals it read, so a long-lived
signal does not accumulate dead observers as components mount and unmount.

## Execution Order

When several effects observe the **same** signal, they run in **registration order** — the order the `Effect`s (or `Memo`s) were created. This is a guaranteed contract, not an implementation detail.

That makes the "run me last" idiom well-defined: an effect registered *after* a tree of rendering effects observes their writes in the same synchronous flush, so a measuring effect can read the post-patch DOM rather than the previous frame's.

```rust
let width = Signal::new(100);

// Registered first — writes.
Effect::new(move || resize_panel(width.get()));

// Registered second — reads what the first one just wrote.
Effect::new(move || {
    width.get();
    record_measurement(measure_panel());
});

width.set(250); // resize_panel runs, then record_measurement sees the new size
```

Two related guarantees follow from the same queue:

- **Effects run in the order they were queued.** Notifying signal A then signal B runs A's effects before B's.
- **A signal written from inside an effect queues its observers *behind* the current flush**, not ahead of it. Cascading updates therefore run breadth-first, and an effect already scheduled for this flush is never preempted.

Effects are still de-duplicated per flush: an effect observing two signals that both change in one `batch()` runs once, at the position of its first enqueue.

## Batching Updates

Outside a batch, effects run after each update. To avoid redundant runs, use `batch()`:

```rust
batch(|| {
    count.set(1);
    name.set("Alice".to_string());
    age.set(30);
    // Effects only run once, after the batch completes
});
```

A top-level `batch()` flushes synchronously: by the time it returns, every effect its writes woke has run. Batches also **nest** — a `batch()` called inside another batch's closure joins the outer transaction, and the single flush happens when the outermost batch exits. (A `batch()` opened from inside an effect *run by a flush* is its own outermost batch — the flag is restored before the flush begins — so it still flushes before returning. An effect body that runs while a batch is still *open* is different: `Effect::new` runs its body immediately, so an effect created inside a batch closure runs inside that batch, and a `batch()` opened there joins the outer transaction instead of flushing.)

Until the flush, no effect has run: inside the closure — including after a nested `batch()` returns — effects have not executed, so a signal an effect writes still holds its old value. **Memos are different: `Memo::get` inside the batch returns the new value.** A write marks every memo that reads it stale synchronously, and the read recomputes it (see [Memos](./memos.md#the-equality-cut-off)). It used to return the pre-batch value, which made a handler that wrote a signal and then read a memo of it act on stale data. **The DOM is different too:** any `NodeHandle` call made in the batch first runs the effects queued so far (see [below](#touching-the-dom-inside-a-handler)).

If the closure **panics**, the panic propagates and the batching flag is restored on the way out, so later writes flush normally. Nothing is flushed during the unwind itself — the effects the aborted batch queued stay pending and run at the next flush (the next unbatched write or outermost batch exit; the runtime does not schedule one on its own).

### Event handlers run as batches

You rarely need to call `batch()` yourself: **rinch runs each of these as one**, on desktop, on the web (the delegated listeners go through the same `rinch_core::events::dispatch_*` functions) and in embed:

- `onclick` and every other `data-rid` handler (the element drag attributes included), `oninput`/`onchange`, `onscroll`, file drops;
- the keyboard interceptor and the Escape dismiss stack (one transaction for both), the paste interceptor;
- `Drag` `on_move`, `on_end` and `on_cancel`;
- menu and tray callbacks, and their shortcuts;
- a registered focus target's `on_focus_gained`, `on_focus_lost`, `on_key` and `on_ime`;
- each callback drained from the main-thread queue — `Signal::send`/`update_send` and `run_on_main_thread` called from another thread, and on native `set_timeout`, `rinch-http` and `rinch-ws` completions — **one transaction per callback**, not one for the whole drain, because they were queued independently and a later one may rely on an earlier one's effects;
- on the web, a `set_timeout` callback, a `rinch-http` completion and a `rinch-ws` event, which arrive straight from the browser.

**Everything else that calls into your code is not batched by rinch**, and its writes flush one by one as they always did. That includes, for example: `run_on_main_thread` called *on* the main thread (it runs its closure there and then, inside whatever transaction is open), `set_selection_callback` / `set_selection_sync_callback`, the configuration-change handler, `on_child_inserted` / `on_child_removed` observers, a `RenderSurface`'s event handler, the editor's `EditorHandle::on_change`, `on_link_click`, `on_link_hover`, `on_key`, `on_selection_change` and `on_caret_moved` callbacks, and effects and memo computations themselves. Wrap any of those in `batch()` yourself if it writes several signals.

So a handler that writes five signals flushes effects **once**, when it returns, and the host is told once (on desktop, one `ReRender`, not five). What that means inside the handler:

- **Memos are current.** Write, then read a memo of what you wrote: you get the new value.
- **Effects run when the handler returns — or earlier, the moment the handler touches the DOM** (next section). Until then a signal an effect derives still holds its old value; read the source signal (or a memo) instead.
- **Glitch-free.** An effect that reads several of the signals the handler wrote runs once, and sees all of the writes, never a mix of old and new.

#### Touching the DOM inside a handler

A handler that writes a signal and then works on the DOM itself was written expecting the write to have reached the DOM already — it always had:

```rust
button { onclick: move || {
    dialog_open.set(true);   // a Modal's effect shows the dialog and focuses its first field
    email_field.focus();     // …but this handler wants the email field
}, "Sign in" }

button { onclick: move || {
    messages.update(|m| m.push(new_message()));  // a `for` effect appends the row
    log.scroll_to_bottom();                      // …and this should reach it
}, "Send" }
```

So **every `NodeHandle` operation made from handler code first runs the effects the handler has queued so far** — `focus()`, `scroll_into_view()`, `set_scroll_top()`, `scroll_to_bottom()`, reads such as `get_attribute`, `children`, `scroll_height` or `get_layout_bounds`, and writes such as `set_attribute` alike. It is the analogue of a browser flushing pending style when script asks for layout. Program order is kept: the `Modal`'s own focus request is made first and the handler's `focus()` overrides it; the new message row exists before the scroll; a `set_attribute` made after a signal write wins over the effect bound to that signal, instead of being overwritten when the handler returns. A handler that only writes signals still flushes exactly once. `rinch_core::flush_pending_effects()` does the same thing explicitly, for code that reaches the DOM some other way.

The rule applies to handler code, not to effects: a `NodeHandle` call inside an effect body or a memo computation never drains the queue, since that code runs *inside* a flush and must not run effects queued behind it ahead of their turn. A handler dispatched *from* an effect (an effect that calls `dispatch_event`, or a web `focus()` that fires a listener synchronously) is handler code again: its batch is a flush context of its own.

**One way to trip over it in your own code.** If a handler writes a signal and then makes a `NodeHandle` call while holding a borrow of a `RefCell` that an effect of that signal also borrows, the effect runs inside that call — and panics with `BorrowError`/`BorrowMutError`. Before handlers were batched the same effect ran at the write itself, before your borrow was taken; the conflict is the same, it just moved from the write to the DOM call. Take the borrow after the DOM call, or drop it before. (Editor *queries* such as `is_mark_active` are not DOM calls and run no effects.)

**Writing a library that keeps its own `RefCell` state?** A `NodeHandle` call inside a handler can now run *user* effects, and a user effect is free to call back into your API. If your code touches the DOM while it holds a borrow of its own state, that is a `BorrowMutError` waiting to happen. Hold `rinch_core::reactive::suppress_effect_flush()` for as long as the borrow lives — no effect runs inside it; the pending ones run at the next DOM access outside it, or when the batch ends — and call `rinch_core::flush_pending_effects()` just *before* taking the borrow, so the caller's earlier writes have reached the DOM you are about to work on. The rich-text editor does exactly this around every borrow of its core, so `tick.set(1); editor.command("toggleBold")` in one handler, or two commands in a row with an `on_change` that bumps a toolbar signal, behave as they did before handlers were batched.

#### Two consequences to know about

**An effect sees only where the handler ended up, never the steps.** `show.set(false); show.set(true)` in one handler does not remount the `if` branch; removing a `for` item and putting one with the same key back keeps that row and its state; closing and re-opening an overlay in one handler neither restores nor re-captures focus. (A `NodeHandle` call in between flushes, so it *does* make the step visible — that is what the rule above is for.)

**A handler that blocks keeps its transaction open.** If a handler shows a native modal dialog (a file picker, a message box) that runs a nested platform event loop — which macOS and Windows dialogs can — its batch stays open until the dialog returns, and anything rinch runs meanwhile, such as a drained `Signal::send` or a timer, joins that batch: its effects run when the dialog closes, not when it arrived.

## Reading Without Tracking

Sometimes you want to read a signal without creating a subscription. Use `untracked()`:

```rust
Effect::new(move || {
    // This creates a subscription
    let count = count.get();

    // This does NOT create a subscription
    let name = untracked(|| name.get());

    println!("Count: {count}, Name: {name}");
});
// This effect only re-runs when `count` changes, not when `name` changes
```

`untracked` hides only the observer directly above it. That matters only while
an effect's run is nested inside another effect's run — the first run of an
`Effect::new` written inside another effect's body, for instance. A read inside
`untracked` there is recorded on the outer effect. Later re-runs of the inner
effect normally come from a flush with no effect beneath them and are unaffected (issue
#932).

### App handlers never subscribe the effect that calls them

A component often calls its app's handler from inside one of its own effects — a
coordinating effect that ends in `onchange`, a `value_fn` binding that reports
back. The handler is app code, and it routinely reads signals: the controlled
idiom `onchange: move |v| if v != store.get() { store.set(v) }` reads the store
it writes. If that read counted, the component's internal effect would be
subscribed to the app's store, and a write to the store from anywhere else would
re-run it out of turn.

So every app-callback type's `invoke` — `Callback`, `ValueCallback<T>`,
`InputCallback`, `FileDropCallback` and `ScrollCallback` — runs the handler with
**no** observer in view, however many effects deep the call is (issue #285).
Handlers respond to events; effects respond to signals. A component author does
not need to wrap `cb.invoke(..)` in `untracked` any more.

Only tracking changes: batching is untouched, so a handler's writes inside an
event handler still flush once, when the batch ends. An effect the handler
creates, or one its writes run, tracks normally.

The same holds for the other app callbacks rinch runs synchronously from a call
you can make inside an effect (issue #931):

- the rich-text editor's `EditorHandle` hooks — `on_change`,
  `on_selection_change`, `on_caret_moved`, `on_key`, `on_link_click` and
  `on_link_hover` — so an app effect that calls `set_selection`, runs a
  `command` or offers a key is not subscribed to a popup hook's own `open`
  signal;
- the child observers (`rinch_core::dom::on_child_inserted` /
  `on_child_removed`), which every `for` reconcile, `if` branch swap and
  component re-render reaches from inside its effect;
- the selection and selection-sync callbacks (`query_selection_ranges`,
  `fire_selection_sync`), the dismiss stack (`dispatch_dismiss`) and
  `Drag::cancel`'s `on_cancel`.

A closure you pass *as the call itself* is not one of these: the `build` closure
of `EditorHandle::update(|state| …)` runs as part of your own code, and a signal
it reads is your effect's dependency like any other.

If you write a component or library that stores an app's callback and calls it
later, call it through `rinch_core::untracked_handler(|| cb(..))` for the same
reason. The five callback types do this in their `invoke`; a bare `Rc<dyn Fn()>`
does not. A closure a component stores and calls itself — an `Rc<dyn Fn()>`, a
`render_fn` — is an ordinary call and is tracked like any other code in the
effect; that is what a `_fn` prop such as `value_fn` relies on, and why the
choice is yours.

## Memory Management with Scopes

Reactive resources continue to exist until something disposes them. Usually that
something is implicit: a `Scope::run(f)` makes that scope the **ambient owner**
for `f`, so every `Signal`, `Memo`, `Effect` and event handler created inside is
attributed to it and freed when it is disposed. Rendering runs under a scope, so
component state is cleaned up without you asking.

```rust
let scope = Scope::new();

scope.run(|| {
    let count = Signal::new(0);          // owned by `scope`
    Effect::new(move || { count.get(); }); // owned by `scope`
});

scope.dispose(); // both are freed
```

`scope.add_effect(effect)` is the manual path, for an effect built outside the
scope that should still die with it.

**With no ambient owner, a resource has app lifetime** — which is why signals
created in `main()` or in startup code keep working untouched. Because handles
are `Copy` and can outlive their values, reads of a freed handle panic while
writes are warn-once no-ops; use `try_get()` / `is_alive()` when a handle may
legitimately be gone. See
[Lifetimes](./hooks.md#lifetimes-what-owns-your-state) for the full story.

## Next Steps

- [Signals](./signals.md) - Reactive state containers
- [Memos](./memos.md) - Cached computed values
- [Stores](./stores.md) - Shared state with action methods
- [Effects](./effects.md) - Advanced: side-effects that track dependencies
