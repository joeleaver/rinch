# Reactive System Architecture

Rinch uses a **fine-grained reactivity** model inspired by Solid.js and Leptos. This document describes the architecture and design decisions.

## Core Concepts

### Signals

A **Signal** is a reactive container that holds a value and notifies subscribers when it changes.

```rust
let count = Signal::new(0);

// Read the value
let value = count.get();

// Update the value (triggers subscribers)
count.set(5);

// Update based on current value
count.update(|n| *n += 1);
```

**Key properties:**
- Reading a signal inside an effect automatically subscribes to it
- Setting a signal schedules dependent effects to re-run
- Signals are `Clone` and can be shared across closures

### Effects

An **Effect** is a side-effect that re-runs when its dependencies change.

```rust
let count = Signal::new(0);

Effect::new(move || {
    println!("Count is now: {}", count.get());
});

count.set(1); // Prints: "Count is now: 1"
count.set(2); // Prints: "Count is now: 2"
```

**Key properties:**
- Dependencies are tracked automatically (no dependency arrays)
- Effects run immediately when created, then re-run when dependencies change
- Effects are cleaned up when their scope is disposed

### Memos

A **Memo** is a cached computed value that only recomputes when dependencies change.

```rust
let count = Signal::new(2);
let doubled = Memo::new(move || count.get() * 2);

doubled.get(); // Returns 4
count.set(3);
doubled.get(); // Returns 6 (recomputed)
doubled.get(); // Returns 6 (cached)
```

**Key properties:**
- Lazily evaluated (only computes when read)
- Caches the result until dependencies change
- Can be read inside effects (creates a subscription)

## Dependency Tracking

Rinch uses **automatic dependency tracking** at runtime:

1. When an effect runs, it registers itself as the "current observer"
2. When a signal is read, it checks for a current observer and subscribes it
3. When a signal changes, it notifies all subscribers to re-run

```
┌─────────────┐     read      ┌─────────────┐
│   Effect    │ ───────────── │   Signal    │
│             │               │             │
│  observer   │ ◄──subscribe──│ subscribers │
└─────────────┘               └─────────────┘
                                    │
                                    │ set()
                                    ▼
                              notify observers
```

## Runtime Architecture

```
┌────────────────────────────────────────────────────┐
│                    Runtime                          │
├────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐               │
│  │ Observer     │  │ Pending      │               │
│  │ Stack        │  │ Effects      │               │
│  └──────────────┘  └──────────────┘               │
│                                                    │
│  ┌──────────────────────────────────────────────┐ │
│  │              Signal Storage                   │ │
│  │  ┌────────┐ ┌────────┐ ┌────────┐           │ │
│  │  │Signal 1│ │Signal 2│ │Signal 3│  ...      │ │
│  │  └────────┘ └────────┘ └────────┘           │ │
│  └──────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────┘
```

### Observer Stack

The runtime maintains a stack of "current observers":
- When an effect starts, it pushes itself onto the stack
- When a signal is read, it subscribes the top of the stack
- When an effect ends, it pops itself

This allows nested effects to work correctly.

### Batching

Multiple signal updates can be batched to avoid redundant effect runs:

```rust
batch(|| {
    count.set(1);
    name.set("Alice");
    // Effects only run once, after the batch
});
```

### Scheduling

Effects are scheduled to run after the current synchronous code completes:
1. Signal is set → effect is marked as "dirty"
2. Dirty effects are queued for execution
3. After current execution, queued effects run
4. This prevents infinite loops and ensures consistent state

## Memory Management

### Scopes

A **Scope** manages the lifetime of reactive primitives. In practice, `RenderScope` serves as the scope for component rendering:

```rust
// RenderScope is the scope used during rendering.
// Effects created via __scope.create_effect() are tracked
// and disposed when the scope is dropped.

fn my_component(__scope: &mut RenderScope) -> NodeHandle {
    let signal = Signal::new(0);
    __scope.create_effect(|| { /* tracked by this scope */ });
    // signal and effect belong to this scope
    rsx! { div { } }
}
// When __scope is disposed, all its effects are cleaned up
```

> **Note:** `RenderScope` is what application code sees — it tracks effects and child scopes for automatic cleanup. Underneath, each `RenderScope` owns a reactive `Scope`, and `Scope::run(f)` makes that scope the **ambient owner** for `f`: signals, memos, effects and event handlers created inside are attributed to it. Attribution is currently recorded but not acted on — see [issue #141](https://github.com/joeleaver/rinch/issues/141), which turns these records into actual reclamation.
>
> Two independent stacks are in play, and they are easy to confuse: `untracked` suspends the *observer* stack (who subscribes to a read), while `unowned` suspends the *owner* stack (who owns an allocation). **With no ambient owner, a resource has app lifetime** — which is why signals created in `main()` or in startup code keep working untouched.

### Ownership

- Signals are reference-counted (`Rc<RefCell<T>>`)
- Effects hold strong references to their closures
- Disposing a scope drops all its primitives

## Integration with UI

The reactive system integrates with the rendering pipeline:

1. **Component functions** run inside a `RenderScope`
2. **RSX expressions** use closure syntax for reactive reads: `{|| count.get().to_string()}`
3. **When signals change**, Effects re-run and surgically update affected DOM nodes
4. **Fine-grained updates** - only the specific nodes bound to changed signals are updated

```rust
#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        div {
            // Reactive text - closure syntax {|| ...} creates an Effect
            "Count: " {|| count.get().to_string()}

            button { onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}
```

> **Note:** The closure syntax `{|| expr}` is required for reactive updates — without it, values are captured once at initial render and never update.

## Thread Safety

The current implementation uses `Rc<RefCell<T>>` for single-threaded use:

- Thread-local runtime by default
- All reactive primitives (`Signal`, `Effect`, `Memo`) are `!Send` and `!Sync`
- This is intentional: GUI frameworks are inherently single-threaded (main thread)
- The observer stack and effect scheduling are thread-local

## Comparison with Other Systems

| Feature | Rinch | React | Solid.js | Leptos |
|---------|-------|-------|----------|--------|
| Reactivity | Fine-grained | Coarse (VDOM) | Fine-grained | Fine-grained |
| Tracking | Automatic | Manual (deps array) | Automatic | Automatic |
| Scheduling | Batched | Batched | Synchronous | Batched |
| Memory | Scoped | GC | Scoped | Scoped |
