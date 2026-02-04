# Fine-Grained Reactive Rendering

Rinch uses fine-grained reactive rendering to achieve efficient UI updates. Instead of regenerating the entire DOM on every state change, Rinch surgically updates only the specific nodes that depend on changed signals.

## Core Concepts

### The Problem with Full Re-rendering

Traditional approaches regenerate HTML on every state change:

```
Signal.set() → re-run app() → generate full HTML → replace entire Document
```

This is inefficient because:
- Small changes cause full tree reconstruction
- Scroll positions are lost
- Focus state is lost
- Animation state is reset
- Layout is recalculated for the entire document

### The Fine-Grained Solution

Rinch's approach runs `app()` once and uses Effects for updates:

```
Signal.set() → Effect runs → NodeHandle.set_text() → Minimal re-layout
```

Benefits:
- Only changed nodes are updated
- Scroll and focus preserved
- Sub-millisecond updates for text changes
- No HTML parsing overhead

## Reactive Primitives

### Signal<T>

A reactive state container that notifies subscribers when its value changes.

```rust
let count = Signal::new(0);

// Read (subscribes the current Effect)
let value = count.get();

// Write (notifies all subscribers)
count.set(5);

// Update (read-modify-write)
count.update(|n| *n += 1);
```

**Tracking:** When `get()` is called inside an Effect, that Effect becomes a subscriber. When `set()` is called, all subscribers are notified and re-run.

### Effect

A side-effect that re-runs when its dependencies change.

```rust
let count = Signal::new(0);
let node = scope.create_element("span");

// This Effect will re-run whenever count changes
scope.create_effect({
    let count = count.clone();
    let node = node.clone();
    move || {
        node.set_text(&count.get().to_string());
    }
});
```

**Lifecycle:**
1. **Creation** - Effect runs immediately, tracking any signals accessed
2. **Dependency change** - When a tracked signal changes, Effect is queued
3. **Re-execution** - Effect runs again, updating its output
4. **Cleanup** - When the scope is disposed, Effects are cleaned up

### Memo<T>

A cached computed value that only recomputes when dependencies change.

```rust
let items = Signal::new(vec![1, 2, 3, 4, 5]);

// Only recomputes when items changes
let sum = Memo::new({
    let items = items.clone();
    move || items.get().iter().sum::<i32>()
});

// Reading sum.get() returns cached value if items hasn't changed
```

**Laziness:** Memos are lazy - they only compute when first accessed and recompute only when dependencies change AND the value is accessed again.

## Dependency Tracking

Rinch uses automatic dependency tracking. You don't need to declare dependencies - they are discovered at runtime.

### How It Works

1. When an Effect runs, it sets itself as the "current observer"
2. Any `signal.get()` call checks for a current observer
3. If present, the signal adds the observer to its subscriber list
4. When the signal changes, it notifies all subscribers

```rust
// Automatic tracking example
let a = Signal::new(1);
let b = Signal::new(2);

scope.create_effect(move || {
    // Both a and b are automatically tracked
    let sum = a.get() + b.get();
    println!("Sum: {}", sum);
});

a.set(10);  // Effect re-runs, prints "Sum: 12"
b.set(20);  // Effect re-runs, prints "Sum: 30"
```

### Conditional Tracking

Dependencies are tracked dynamically. If a branch isn't taken, those signals aren't tracked:

```rust
let show_a = Signal::new(true);
let a = Signal::new("A");
let b = Signal::new("B");

scope.create_effect(move || {
    if show_a.get() {
        println!("{}", a.get());  // Only tracked when show_a is true
    } else {
        println!("{}", b.get());  // Only tracked when show_a is false
    }
});
```

## DOM Integration

### NodeHandle

A stable reference to a DOM node that enables surgical updates:

```rust
let node = scope.create_element("div");

// Text content
node.set_text("Hello");

// Attributes
node.set_attribute("class", "active");
node.remove_attribute("disabled");

// Styles
node.set_style("color", "red");
node.set_style("display", "none");

// Classes
node.add_class("highlighted");
node.remove_class("dimmed");

// Tree manipulation
node.append_child(&child_node);
node.insert_before(&new_node, &reference_node);
node.remove();
```

### Reactive DOM Updates

The rsx! macro creates Effects for reactive expressions:

```rust
rsx! {
    div {
        // Static text - rendered once
        "Hello, "

        // Reactive expression - creates an Effect
        {|| name.get()}

        // Reactive style - creates an Effect
        style: {|| format!("color: {}", color.get())},
    }
}
```

Under the hood, this generates:

```rust
let div = scope.create_element("div");

// Static text
let text1 = scope.create_text("Hello, ");
div.append_child(&text1);

// Reactive text - creates Effect
let text2 = scope.create_text(&name.get().to_string());
div.append_child(&text2);
scope.create_effect({
    let name = name.clone();
    let text2 = text2.clone();
    move || {
        text2.set_text(&name.get().to_string());
    }
});

// Reactive style - creates Effect
scope.create_effect({
    let color = color.clone();
    let div = div.clone();
    move || {
        div.set_style("color", &color.get());
    }
});
```

## Batch Updates

Multiple signal changes can be batched to avoid redundant Effect executions:

```rust
// Without batching - Effect runs 3 times
a.set(1);
b.set(2);
c.set(3);

// With batching - Effect runs once
batch(|| {
    a.set(1);
    b.set(2);
    c.set(3);
});
```

## Conditional Rendering (Show)

The `show()` function handles conditional rendering with fine-grained updates:

```rust
show(
    scope,
    || condition.get(),           // Condition closure
    |s| rsx_content!(s, div { "Visible" }),  // Then branch
    Some(|s| rsx_content!(s, span { "Hidden" })),  // Else branch
)
```

When the condition changes:
1. The Effect runs
2. Old content is removed from DOM
3. New content is rendered into a fresh scope
4. New content is inserted at the anchor point

## List Rendering (For)

The `for_each()` function handles keyed list rendering:

```rust
for_each(
    scope,
    || items.get(),                    // Items closure
    |item| item.id.clone(),            // Key function
    |item, s| rsx_content!(s, li { {item.name.clone()} }),  // Item renderer
)
```

When items change:
1. Diff algorithm compares old and new key lists
2. Items with unchanged keys are preserved (not re-rendered)
3. New items are inserted at correct positions
4. Removed items are cleaned up
5. Moved items are repositioned

## Comparison with Other Approaches

| Approach | Update Granularity | Performance | Complexity |
|----------|-------------------|-------------|------------|
| Full re-render | Entire DOM | O(n) | Simple |
| Virtual DOM | Subtree patches | O(log n) | Medium |
| Fine-grained | Single nodes | O(1) | Complex |

Rinch's fine-grained approach provides the best performance for reactive updates, at the cost of more sophisticated compilation and runtime machinery.

## Performance Characteristics

- **Text updates:** < 1ms (single node mutation)
- **Style changes:** < 1ms (single node mutation)
- **List item add/remove:** ~5ms (DOM operations + layout)
- **Full conditional swap:** ~10ms (subtree rebuild)

These are typical measurements; actual performance depends on document complexity and hardware.
