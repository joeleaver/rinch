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

The value type must be `Clone + PartialEq`. `PartialEq` is what the equality
cut-off below compares with; a type that cannot implement it meaningfully can
implement it as "never equal" (`fn eq(&self, _: &Self) -> bool { false }`),
which gives back the old behaviour of waking every dependent on every recompute.

## How Memos Work

1. **Lazy** — only computes when you call `.get()`
2. **Cached** — returns the cached result if dependencies haven't changed
3. **Tracked** — automatically discovers which signals it reads
4. **Composable** — memos can depend on other memos
5. **Cut off on equality** — a recompute that produces a value *equal* to the
   previous one wakes none of the memo's dependents
6. **Always current** — a read straight after a write to one of its sources sees
   the new value, even inside a `batch()` or an event handler that has not
   returned yet

```
Signal(count) ──► Memo(doubled) ──► cached value
    │                   │
  .set(3)            .get()
    │                   │
    ▼                   ▼
 marks memo       recomputes if
 as "dirty"          dirty
```

### The equality cut-off

The pattern it exists for is per-row derived state:

```rust
let selected = Signal::new(3);

for row in rows.get() {
    let id = row.id;
    let is_selected = Memo::new(move || selected.get() == id);
    div { key: id, class: {move || if is_selected.get() { "row selected" } else { "row" }} }
}
```

Every row's memo reads `selected`, so a selection change recomputes all of
them — a comparison each. But only two answers change (the row that lost the
selection and the row that gained it), so only those two rows' `class` effects
run. Without the cut-off every row's effect ran on every click.

How it works: a write marks the memos that read it stale **at once** (so a read
straight after the write is current), and a memo's dependents are woken as
*maybes*. Before running a maybe, the flush brings the memos it read up to date
and compares each one's **version** — a counter that moves only when a recompute
produces an unequal value — with the version the dependent saw last time. None
moved: it is skipped. An effect that also reads a *signal* that changed is not a
maybe, and runs as usual; signals have no cut-off (`set` notifies whether or not
the value changed — use `set_if_changed` for that).

The cut-off chains: a memo that reads a memo which recomputed to an equal value
is re-validated without running its own computation. A source memo that has
been **freed** (its scope disposed) counts as changed, so a reader that falls
back with `try_get()` recomputes once and sees `None` rather than keeping a value
computed from a memo that no longer exists.

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
