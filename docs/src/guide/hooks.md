# State Management

There are no hooks. There is no re-rendering. Your component function runs once, builds the DOM, and closures keep it updated forever. Here's how you manage state.

## Core Primitives

| Primitive | What it does |
|-----------|-------------|
| `Signal::new(value)` | Reactive state. Read it, write it, closures that read it re-run when it changes. |
| `Memo::new(closure)` | Cached derived state. Like a Signal you can't write to. |
| `create_store(value)` | Share a store struct across components. |
| `use_store::<T>()` | Access a shared store from any descendant. |
| `create_context(value)` | Low-level shared state (mostly for framework internals). |

> **For shared state, use [stores](./stores.md).** A store is a struct with Signal fields and methods that mutate them. Clean, testable, no prop drilling.

## Signal

The foundational reactive primitive. Create one, read it in closures, and those closures re-run when the value changes.

```rust
let count = Signal::new(0);

count.get();                 // Read
count.set(5);                // Write
count.update(|n| *n += 1);  // Read-modify-write
```

`Signal<T>` is `Copy`. Use it in as many closures as you want — no `.clone()` needed.

```rust
#[component]
fn toggle() -> NodeHandle {
    let on = Signal::new(false);

    rsx! {
        button { onclick: move || on.update(|b| *b = !*b),
            {|| if on.get() { "ON" } else { "OFF" }}
        }
    }
}
```

### Cross-Thread Dispatch

`Signal::set()` and `Signal::update()` must be called from the main thread. From a background thread, use `send()` and `update_send()`:

```rust
let progress = Signal::new(0);

std::thread::spawn(move || {
    for i in 0..100 {
        std::thread::sleep(Duration::from_millis(50));
        progress.send(i);  // Dispatches to main thread automatically
    }
});
```

For more details, see [Signals](./signals.md).

## Memo

Cached derived state that recomputes only when its dependencies change.

```rust
let first = Signal::new("Alice".to_string());
let last = Signal::new("Smith".to_string());

let full = Memo::new(move || format!("{} {}", first.get(), last.get()));
// full.get() recomputes only when first or last change
```

`Memo<T>` is also `Copy`. Dependencies are tracked automatically — no dependency arrays.

For more details, see [Memos](./memos.md).

## Effect (Advanced)

Most reactive DOM updates happen through `{|| expr}` closures in RSX. For the rare case where you need to react to signal changes outside of the DOM (logging, syncing to an external system), use `Effect`:

```rust
use rinch::reactive::Effect;

let count = Signal::new(0);

Effect::new(move || {
    println!("Count is now: {}", count.get());
});
```

Effect is intentionally excluded from the prelude. If you're reaching for it, ask yourself: can this be a closure in RSX, or a method on a store? Usually the answer is yes.

For more details, see [Effects](./effects.md).

## Context

Share state across components without prop drilling. The ancestor creates it, any descendant reads it.

```rust
#[derive(Clone)]
struct AppConfig {
    api_url: String,
}

#[component]
fn app() -> NodeHandle {
    create_context(AppConfig { api_url: "https://api.example.com".into() });
    rsx! { div { ChildComponent {} } }
}

#[component]
fn ChildComponent() -> NodeHandle {
    let config = use_context::<AppConfig>();
    rsx! { Text { {config.api_url.clone()} } }
}
```

`use_context::<T>()` panics if the context is missing (with a helpful message). Use `try_use_context::<T>()` for an `Option<T>` instead.

For reactive shared state, use [stores](./stores.md) — they're contexts with Signal fields and methods.

## Putting It Together

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    let todos = Signal::new(vec!["Learn Rinch".to_string()]);
    let input = Signal::new(String::new());
    let count = Memo::new(move || todos.get().len());

    let add = move || {
        let text = input.get();
        if !text.is_empty() {
            todos.update(|t| t.push(text.clone()));
            input.set(String::new());
        }
    };

    rsx! {
        Stack { gap: "md", p: "xl",
            Title { order: 1, "Todos (" {|| count.get().to_string()} ")" }

            Group { gap: "sm",
                TextInput {
                    placeholder: "What needs doing?",
                    value_fn: move || input.get(),
                    oninput: move |v: String| input.set(v),
                    onsubmit: add,
                }
                Button { onclick: add, "Add" }
            }

            for todo in todos.get() {
                div { key: todo.clone(), {todo.clone()} }
            }
        }
    }
}
```

Signal is `Copy`, so `add` captures `todos` and `input` without ceremony. `count` is a Memo that recomputes when `todos` changes. The `for` loop is reactive — add or remove a todo and only the affected DOM nodes change.
