# Memos

A **Memo** is a cached computed value that only recomputes when its dependencies change. Think of it as a signal you can't write to — it derives its value from other signals.

## Creating Memos

```rust
let count = Signal::new(2);

let doubled = Memo::new(move || count.get() * 2);

doubled.get(); // 4
count.set(3);
doubled.get(); // 6 (recomputed)
doubled.get(); // 6 (cached — no recomputation)
```

`Memo<T>` is `Copy`, just like `Signal<T>`. Use it in multiple closures without `.clone()`.

## How Memos Work

1. **Lazy** — only computes when you call `.get()`
2. **Cached** — returns the cached result if dependencies haven't changed
3. **Tracked** — automatically discovers which signals it reads
4. **Composable** — memos can depend on other memos

```
Signal(count) ──► Memo(doubled) ──► cached value
    │                   │
  .set(3)            .get()
    │                   │
    ▼                   ▼
 marks memo       recomputes if
 as "dirty"          dirty
```

## Memos vs Effects

| | Memo | Effect |
|---|---|---|
| Returns a value | Yes | No |
| Runs eagerly | No (lazy) | Yes (immediate) |
| Purpose | Derived state | Side effects |
| Caches result | Yes | N/A |

Use a **Memo** when you need a computed value. Use an **Effect** when you need to perform an action.

## Chaining Memos

Memos can depend on other memos:

```rust
let count = Signal::new(2);
let doubled = Memo::new(move || count.get() * 2);
let quadrupled = Memo::new(move || doubled.get() * 2);

assert_eq!(quadrupled.get(), 8);
count.set(3);
assert_eq!(quadrupled.get(), 12);
```

## The `derived()` Helper

Syntactic sugar for `Memo::new()`:

```rust
let count = Signal::new(5);

// These are equivalent:
let doubled = Memo::new(move || count.get() * 2);
let doubled = derived(move || count.get() * 2);
```

## Common Patterns

### Computed Strings

```rust
let first = Signal::new("Alice".to_string());
let last = Signal::new("Smith".to_string());

let full = Memo::new(move || format!("{} {}", first.get(), last.get()));
```

### Filtering Lists

```rust
let items = Signal::new(vec![1, 2, 3, 4, 5, 6]);
let show_even = Signal::new(true);

let filtered = Memo::new(move || {
    let all = items.get();
    if show_even.get() {
        all.into_iter().filter(|n| n % 2 == 0).collect()
    } else {
        all
    }
});
```

### Expensive Computations

```rust
let data = Signal::new(large_dataset);

// Only recomputes when data changes, no matter how many times you .get()
let analysis = Memo::new(move || {
    data.with(|d| perform_analysis(d))
});
```

## Memos in RSX

Memos work in reactive closures exactly like signals:

```rust
let count = Signal::new(0);
let doubled = Memo::new(move || count.get() * 2);

rsx! {
    p { "Count: " {|| count.get().to_string()} }
    p { "Doubled: " {|| doubled.get().to_string()} }
}
```

Both `count` and `doubled` are `Copy`. Both create subscriptions when read in `{|| ...}` closures. The doubled text node updates only when the memo's value actually changes.
