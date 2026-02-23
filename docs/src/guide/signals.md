# Signals

A **Signal** is a reactive container that holds a value and notifies subscribers when that value changes. Signals are the foundation of rinch's reactivity system.

## Creating Signals

```rust
use rinch::prelude::*;

// Create a signal with an initial value
let count = Signal::new(0);
let name = Signal::new(String::from("Alice"));
let items = Signal::new(vec![1, 2, 3]);
```

Signals can hold any type. They use `Rc<RefCell<T>>` internally, making them cheap to clone and share across closures.

## Reading Values

### `.get()` - Clone and Return

For `Clone` types, use `.get()` to get a copy of the value:

```rust
let count = Signal::new(0);
let value = count.get(); // Returns 0 (cloned)
```

**Important:** Reading a signal inside an Effect or Memo automatically subscribes that observer to the signal.

### `.with()` - Access by Reference

For types that are expensive to clone or when you just need to inspect the value:

```rust
let items = Signal::new(vec![1, 2, 3, 4, 5]);

// Access without cloning
let length = items.with(|v| v.len());
let first = items.with(|v| v.first().copied());
```

## Writing Values

### `.set()` - Replace the Value

```rust
let count = Signal::new(0);
count.set(5); // Replaces value with 5, notifies subscribers
```

### `.update()` - Modify in Place

For updating based on the current value:

```rust
let count = Signal::new(0);
count.update(|n| *n += 1); // Increment by 1

let items = Signal::new(vec![1, 2, 3]);
items.update(|v| v.push(4)); // Add item to vec
```

## Cloning Signals

Signals are reference-counted. Cloning a signal creates another handle to the same underlying data:

```rust
let count = Signal::new(0);
let count_clone = count.clone();

count.set(5);
assert_eq!(count_clone.get(), 5); // Same underlying value
```

This makes it easy to share signals across multiple closures:

```rust
let count = Signal::new(0);

let count_for_effect = count.clone();
Effect::new(move || {
    println!("Count is: {}", count_for_effect.get());
});

let count_for_button = count.clone();
// In button onclick handler:
// count_for_button.update(|n| *n += 1);
```

## Automatic Dependency Tracking

When you read a signal inside an Effect or Memo, that dependency is tracked automatically:

```rust
let first_name = Signal::new("Alice".to_string());
let last_name = Signal::new("Smith".to_string());

// This effect depends on BOTH signals
Effect::new(move || {
    let full = format!("{} {}", first_name.get(), last_name.get());
    println!("Full name: {full}");
});

first_name.set("Bob".to_string()); // Effect re-runs
last_name.set("Jones".to_string()); // Effect re-runs
```

## Display and Debug

Signals implement `Display` and `Debug` for easy printing:

```rust
let count = Signal::new(42);

println!("{}", count);      // Prints: 42
println!("{:?}", count);    // Prints: Signal { value: 42 }
```

## Best Practices

### Do: Keep Signals Focused

```rust
// Good: Separate signals for separate concerns
let name = Signal::new(String::new());
let age = Signal::new(0);

// Avoid: One big signal for everything
let state = Signal::new(AppState { name, age, ... });
```

### Do: Clone Before Moving into Closures

```rust
let count = Signal::new(0);

// Clone before the closure captures it
let count_for_closure = count.clone();
Effect::new(move || {
    let _ = count_for_closure.get();
});

// Original `count` still available here
count.set(1);
```

### Avoid: Reading Signals in Loops (when possible)

```rust
// Avoid: Creates subscription on every iteration
for _ in 0..100 {
    let val = signal.get(); // Tracks 100 times!
}

// Better: Read once before the loop
let val = signal.get();
for _ in 0..100 {
    // use val
}
```

## API Reference

```rust
impl<T> Signal<T> {
    /// Create a new signal with the given initial value
    pub fn new(value: T) -> Self;

    /// Set the signal to a new value (notifies subscribers)
    pub fn set(&self, value: T);

    /// Update the value using a function (notifies subscribers)
    pub fn update(&self, f: impl FnOnce(&mut T));

    /// Access the value by reference without cloning
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R;
}

impl<T: Clone> Signal<T> {
    /// Get a clone of the current value
    pub fn get(&self) -> T;
}
```
