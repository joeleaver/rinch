# Memos

A **Memo** is a cached computed value that only recomputes when its dependencies change. Memos are perfect for derived state that's expensive to calculate.

## Creating Memos

```rust
use rinch::prelude::*;

let count = Signal::new(2);

// Memo computes doubled value
let doubled = Memo::new(move || count.get() * 2);

doubled.get(); // Returns 4
count.set(3);
doubled.get(); // Returns 6 (recomputed)
doubled.get(); // Returns 6 (cached, no recomputation)
```

## How Memos Work

1. **Lazy evaluation** - The computation only runs when you call `.get()`
2. **Caching** - The result is cached until dependencies change
3. **Dependency tracking** - Memos track which signals they read, just like effects
4. **Composable** - Memos can depend on other memos

```
┌─────────────┐         ┌─────────────┐
│   Signal    │ ──────► │    Memo     │ ──────► cached value
│   count     │ depends │   doubled   │
└─────────────┘         └─────────────┘
      │                        │
   .set()                   .get()
      │                        │
      ▼                        ▼
  marks memo             recomputes if
  as "dirty"               dirty
```

## Memos vs Effects

| Aspect | Memo | Effect |
|--------|------|--------|
| Returns a value | Yes | No |
| Runs eagerly | No (lazy) | Yes (immediate) |
| Purpose | Derived state | Side effects |
| Caches result | Yes | N/A |

Use a **Memo** when you need a computed value. Use an **Effect** when you need to perform an action.

```rust
// Memo: computing a derived value
let total = Memo::new(move || {
    items.with(|v| v.iter().sum::<i32>())
});

// Effect: performing a side effect
Effect::new(move || {
    println!("Total changed to: {}", total.get());
});
```

## Chaining Memos

Memos can depend on other memos:

```rust
let count = Signal::new(2);

let doubled = Memo::new({
    let count = count.clone();
    move || count.get() * 2
});

let quadrupled = Memo::new({
    let doubled = doubled.clone();
    move || doubled.get() * 2
});

assert_eq!(quadrupled.get(), 8);

count.set(3);
assert_eq!(quadrupled.get(), 12);
```

## The `derived()` Helper

For simple derivations, use the `derived()` helper function:

```rust
let count = Signal::new(5);

// These are equivalent:
let doubled = Memo::new(move || count.get() * 2);
let doubled = derived(move || count.get() * 2);
```

## Common Patterns

### Filtering Lists

```rust
let items = Signal::new(vec![1, 2, 3, 4, 5, 6]);
let filter = Signal::new(|n: &i32| n % 2 == 0);

let items_clone = items.clone();
let filter_clone = filter.clone();
let filtered = Memo::new(move || {
    let f = filter_clone.with(|f| f.clone());
    items_clone.with(|v| v.iter().filter(|n| f(n)).copied().collect::<Vec<_>>())
});

filtered.get(); // [2, 4, 6]
```

### Computed Strings

```rust
let first_name = Signal::new("Alice".to_string());
let last_name = Signal::new("Smith".to_string());

let full_name = Memo::new({
    let first = first_name.clone();
    let last = last_name.clone();
    move || format!("{} {}", first.get(), last.get())
});

full_name.get(); // "Alice Smith"
```

### Expensive Computations

```rust
let data = Signal::new(large_dataset);

// Only recomputes when data changes
let analysis = Memo::new({
    let data = data.clone();
    move || {
        data.with(|d| {
            // Expensive computation
            perform_analysis(d)
        })
    }
});
```

## Memos in Effects

Reading a memo inside an effect creates a subscription. The effect will re-run when the memo's value changes:

```rust
let count = Signal::new(0);
let doubled = Memo::new({
    let count = count.clone();
    move || count.get() * 2
});

Effect::new({
    let doubled = doubled.clone();
    move || {
        println!("Doubled value: {}", doubled.get());
    }
});

count.set(5);
// Memo recomputes: 10
// Effect re-runs, prints: "Doubled value: 10"
```

## Avoiding Recomputation

Memos won't recompute if you read them multiple times without changing dependencies:

```rust
let count = Signal::new(0);
let computation_count = Signal::new(0);

let expensive = Memo::new({
    let count = count.clone();
    let comp_count = computation_count.clone();
    move || {
        comp_count.update(|n| *n += 1);
        count.get() * 2
    }
});

expensive.get(); // Computes
expensive.get(); // Cached
expensive.get(); // Cached

assert_eq!(computation_count.get(), 1); // Only computed once!
```

## API Reference

```rust
impl<T: Clone + 'static> Memo<T> {
    /// Create a new memo with the given computation function
    pub fn new<F: Fn() -> T + 'static>(f: F) -> Self;

    /// Get the current value, recomputing if necessary
    pub fn get(&self) -> T;
}

/// Convenience function to create a memo
pub fn derived<T: Clone + 'static>(f: impl Fn() -> T + 'static) -> Memo<T>;
```
