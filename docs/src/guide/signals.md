# Signals

A **Signal** holds a value and tells everyone who cares when it changes. Signals are the foundation of everything reactive in Rinch.

## Creating Signals

```rust
let count = Signal::new(0);
let name = Signal::new(String::from("Alice"));
let items = Signal::new(vec![1, 2, 3]);
```

Signals can hold any `'static` type. They're cheap — just an index into a global slot map.

## Signal is Copy

This is the single most important thing about Rinch's reactive system. `Signal<T>` implements `Copy`. You never need `.clone()` before passing a signal into a closure:

```rust
let count = Signal::new(0);

// Use count in as many closures as you want — it's Copy
Effect::new(move || println!("count = {}", count.get()));
rsx! {
    button { onclick: move || count.update(|n| *n += 1), "+" }
    span { {|| count.get().to_string()} }
}
```

Same for `Memo<T>` — also `Copy`.

## Reading Values

### `.get()` — Clone and Return

```rust
let count = Signal::new(0);
let value = count.get(); // Returns 0 (cloned)
```

**Inside an Effect or Memo, calling `.get()` automatically subscribes to the signal.** When the signal changes, the Effect re-runs. No dependency arrays. No manual tracking.

### `.with()` — Access by Reference

For types that are expensive to clone:

```rust
let items = Signal::new(vec![1, 2, 3, 4, 5]);

let length = items.with(|v| v.len());
let first = items.with(|v| v.first().copied());
```

## Writing Values

### `.set()` — Replace

```rust
count.set(5); // Notifies all subscribers
```

### `.update()` — Modify in Place

```rust
count.update(|n| *n += 1);
items.update(|v| v.push(4));
```

## Automatic Dependency Tracking

Read a signal inside an Effect or Memo and the dependency is tracked automatically:

```rust
let first_name = Signal::new("Alice".to_string());
let last_name = Signal::new("Smith".to_string());

// This effect depends on BOTH signals — discovered at runtime
Effect::new(move || {
    println!("{} {}", first_name.get(), last_name.get());
});

first_name.set("Bob".to_string());   // Effect re-runs
last_name.set("Jones".to_string());  // Effect re-runs
```

Dependencies are dynamic. If a signal is only read conditionally, the subscription only exists when that branch executes.

## Cross-Thread Dispatch

`set()` and `update()` panic if called from a non-main thread (with a helpful message). For background threads, use `send()` and `update_send()`:

```rust
let progress = Signal::new(0);

std::thread::spawn(move || {
    for i in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        progress.send(i);  // Dispatches to main thread
    }
});
```

`send()` requires `T: Send`. `update_send()` requires the closure to be `Send + 'static`.

## Best Practices

**Keep signals focused.** One signal per concern, not one giant state bag:

```rust
// Good
let name = Signal::new(String::new());
let age = Signal::new(0);

// Less good — changing name re-runs everything that reads age too
let state = Signal::new(AppState { name, age });
```

**Read once before loops:**

```rust
// Read once, loop over the snapshot
let val = items.get();
for item in &val {
    // use item
}
```

**Use `{|| expr}` in RSX, not raw Effects.** The RSX closure syntax is the right tool for DOM updates. Reserve `Effect::new()` for side effects outside the DOM (logging, network calls, syncing external state).
