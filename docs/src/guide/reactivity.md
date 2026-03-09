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

Effects continue running until disposed. Use `Scope` to manage their lifetime:

```rust
let scope = Scope::new();

// Register effects with the scope
let effect = Effect::new(|| { /* ... */ });
scope.add_effect(effect);

// When scope is dropped, all effects are disposed
drop(scope);
```

## Next Steps

- [Signals](./signals.md) - Reactive state containers
- [Memos](./memos.md) - Cached computed values
- [Stores](./stores.md) - Shared state with action methods
- [Effects](./effects.md) - Advanced: side-effects that track dependencies
