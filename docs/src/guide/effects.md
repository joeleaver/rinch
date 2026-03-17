# Effects

An **Effect** is a side-effect that runs when its dependencies change. It tracks which signals it reads and re-runs when any of them update. No dependency arrays, no manual subscription — just read a signal and you're subscribed.

> **You probably don't need this.** For reactive DOM updates, use `{|| expr}` closures in RSX. For state mutations, use [store methods](./stores.md). Effect is for the rare case where you need to react to signal changes outside of both — logging, syncing to external systems, etc.

Effect is intentionally excluded from the prelude. Import explicitly:

```rust
use rinch::reactive::Effect;
```

## Creating Effects

```rust
let count = Signal::new(0);

// Runs immediately, then re-runs when count changes
Effect::new(move || {
    println!("Count is: {}", count.get());
});

count.set(1); // Prints: "Count is: 1"
count.set(2); // Prints: "Count is: 2"
```

Signal is `Copy`, so you can use `count` in the closure and still use it elsewhere. No `.clone()`.

## When Effects Run

1. **Immediately on creation** — the effect runs once right away
2. **When dependencies change** — whenever a tracked signal is updated

```rust
let a = Signal::new(1);
let b = Signal::new(2);

Effect::new(move || {
    println!("a = {}, b = {}", a.get(), b.get());
});
// Prints: "a = 1, b = 2"

a.set(10); // Prints: "a = 10, b = 2"
b.set(20); // Prints: "a = 10, b = 20"
```

## Conditional Dependencies

Dependencies are discovered at runtime, not declared statically. If a signal is only read in one branch, the subscription only exists when that branch executes:

```rust
let show_details = Signal::new(false);
let name = Signal::new("Alice".to_string());
let age = Signal::new(30);

Effect::new(move || {
    println!("Name: {}", name.get());
    if show_details.get() {
        println!("Age: {}", age.get()); // only tracked when show_details is true
    }
});

age.set(31);           // Nothing happens — age isn't tracked yet
show_details.set(true); // Re-runs, now age IS tracked
age.set(32);           // Re-runs — age is tracked now
```

## Common Patterns

### Logging

```rust
let count = Signal::new(0);
Effect::new(move || {
    println!("[DEBUG] count = {}", count.get());
});
```

### Syncing to External Systems

```rust
let theme = Signal::new("light".to_string());
Effect::new(move || {
    update_css_theme(&theme.get());
});
```

### Derived Side Effects

```rust
let items = Signal::new(vec![1, 2, 3]);
Effect::new(move || {
    let count = items.with(|v| v.len());
    update_badge(count);
});
```

## Disposing Effects

```rust
let effect = Effect::new(move || {
    println!("Count: {}", count.get());
});

effect.dispose(); // Effect stops running
count.set(1);     // Nothing happens
```

## Pitfalls

**Don't create effects inside effects.** The inner effect gets recreated every time the outer one re-runs.

**Don't modify a signal you read in the same effect.** That's an infinite loop. Use `untracked(|| signal.get())` if you need to read without subscribing.

```rust
// BAD: infinite loop
Effect::new(move || {
    let val = count.get();
    count.set(val + 1); // triggers re-run -> reads count -> triggers re-run -> ...
});

// OK: read without subscribing
Effect::new(move || {
    let val = untracked(|| count.get());
    do_something(val);
});
```
