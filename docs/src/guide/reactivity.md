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

When you update multiple signals, effects run after each update. To avoid redundant runs, use `batch()`:

```rust
batch(|| {
    count.set(1);
    name.set("Alice".to_string());
    age.set(30);
    // Effects only run once, after the batch completes
});
```

A top-level `batch()` flushes synchronously: by the time it returns, every effect its writes woke has run. Batches also **nest** — a `batch()` called inside another batch's closure joins the outer transaction, and the single flush happens when the outermost batch exits. (A `batch()` opened from inside an effect *run by a flush* is its own outermost batch — the flag is restored before the flush begins — so it still flushes before returning. An effect body that runs while a batch is still *open* is different: `Effect::new` runs its body immediately, so an effect created inside a batch closure runs inside that batch, and a `batch()` opened there joins the outer transaction instead of flushing.)

Until the flush, nothing has run: inside the closure — including after a nested `batch()` returns — effects have not executed, and `Memo::get` still returns the pre-batch value.

If the closure **panics**, the panic propagates and the batching flag is restored on the way out, so later writes flush normally. Nothing is flushed during the unwind itself — the effects the aborted batch queued stay pending and run at the next flush (the next unbatched write or outermost batch exit; the runtime does not schedule one on its own).

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
