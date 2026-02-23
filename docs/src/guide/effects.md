# Effects

An **Effect** is a side-effect that runs when its dependencies change. Effects automatically track which signals they read and re-run when any of those signals update.

## Creating Effects

```rust
use rinch::prelude::*;

let count = Signal::new(0);

// Effect runs immediately, then re-runs when dependencies change
Effect::new(move || {
    println!("Count is: {}", count.get());
});

count.set(1); // Prints: "Count is: 1"
count.set(2); // Prints: "Count is: 2"
```

## When Effects Run

1. **Immediately on creation** - The effect runs once right away
2. **When dependencies change** - Whenever a signal that was read during the effect is updated

```rust
let a = Signal::new(1);
let b = Signal::new(2);

Effect::new(move || {
    println!("a = {}, b = {}", a.get(), b.get());
});
// Prints immediately: "a = 1, b = 2"

a.set(10);
// Prints: "a = 10, b = 2"

b.set(20);
// Prints: "a = 10, b = 20"
```

## Deferred Effects

Sometimes you don't want the effect to run immediately. Use `new_deferred()`:

```rust
let count = Signal::new(0);

let effect = Effect::new_deferred(move || {
    println!("Count is: {}", count.get());
});

// Nothing printed yet

effect.run(); // Manually trigger: prints "Count is: 0"
count.set(1); // Automatically triggers: prints "Count is: 1"
```

## Conditional Dependencies

Effects only track signals read during their most recent execution:

```rust
let show_details = Signal::new(false);
let name = Signal::new("Alice".to_string());
let age = Signal::new(30);

Effect::new(move || {
    println!("Name: {}", name.get());

    if show_details.get() {
        // `age` is only tracked when show_details is true
        println!("Age: {}", age.get());
    }
});

age.set(31);
// No effect re-run! (show_details is false, so age wasn't tracked)

show_details.set(true);
// Effect re-runs, now age IS tracked

age.set(32);
// Effect re-runs because age is now tracked
```

## Disposing Effects

Effects continue running until disposed. To stop an effect:

```rust
let count = Signal::new(0);

let effect = Effect::new(move || {
    println!("Count: {}", count.get());
});

effect.dispose(); // Effect will no longer run

count.set(1); // Nothing happens
```

## Using Scopes for Cleanup

For managing multiple effects, use a `Scope`:

```rust
let scope = Scope::new();

let effect1 = Effect::new(|| { /* ... */ });
let effect2 = Effect::new(|| { /* ... */ });

scope.add_effect(effect1);
scope.add_effect(effect2);

// Later: dispose all effects at once
scope.dispose();
```

Scopes also clean up when dropped:

```rust
{
    let scope = Scope::new();
    let effect = Effect::new(|| { /* ... */ });
    scope.add_effect(effect);
} // scope dropped here, effect disposed
```

## Common Patterns

### Logging State Changes

```rust
let count = Signal::new(0);

let count_clone = count.clone();
Effect::new(move || {
    println!("[DEBUG] count = {}", count_clone.get());
});
```

### Syncing to External Systems

```rust
let theme = Signal::new("light".to_string());

let theme_clone = theme.clone();
Effect::new(move || {
    let current_theme = theme_clone.get();
    // Sync to localStorage, CSS variables, etc.
    update_css_theme(&current_theme);
});
```

### Derived Side Effects

```rust
let items = Signal::new(vec![1, 2, 3]);

let items_clone = items.clone();
Effect::new(move || {
    let count = items_clone.with(|v| v.len());
    update_item_count_badge(count);
});
```

## Avoiding Common Pitfalls

### Don't Create Effects Inside Effects

```rust
// BAD: Creates new effects every time count changes
Effect::new(move || {
    let val = count.get();
    Effect::new(move || {  // This is created every time!
        println!("Nested: {val}");
    });
});

// GOOD: Single effect that tracks count
Effect::new(move || {
    let val = count.get();
    println!("Count: {val}");
});
```

### Don't Modify Signals You Read in the Same Effect

```rust
// BAD: Can cause infinite loops
Effect::new(move || {
    let val = count.get();
    count.set(val + 1); // Triggers this effect again!
});

// GOOD: Use untracked if you need to read without subscribing
Effect::new(move || {
    let val = untracked(|| count.get());
    // ... do something with val
});
```

## API Reference

```rust
impl Effect {
    /// Create an effect that runs immediately and re-runs when dependencies change
    pub fn new<F: FnMut() + 'static>(f: F) -> Self;

    /// Create an effect that doesn't run until manually triggered or dependencies change
    pub fn new_deferred<F: FnMut() + 'static>(f: F) -> Self;

    /// Manually trigger the effect to run
    pub fn run(&self);

    /// Dispose the effect, preventing it from running again
    pub fn dispose(&self);
}
```
