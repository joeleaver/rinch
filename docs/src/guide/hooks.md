# State Management

Rinch uses **fine-grained reactive primitives** for managing state in your components. Components run once to build the DOM, and reactive closures (`{|| expr}`) handle all subsequent updates surgically.

## Core Primitives

| Primitive | Purpose |
|-----------|---------|
| [`Signal::new()`](#signal) | Reactive state that triggers updates |
| [`Memo::new()`](#memo) | Cached computed values |
| [`create_store()`](./stores.md) | Share state across components (recommended) |
| [`use_store::<T>()`](./stores.md) | Access a shared store |
| [`create_context()`](#context) | Low-level shared state (framework internals) |

> **Recommended:** For shared state, use the [store pattern](./stores.md) — a struct with Signal fields and action methods, shared via `create_store()` / `use_store()`.


```rust
use rinch::prelude::*;

#[component]
fn counter() -> NodeHandle {
    let count = Signal::new(0);

    rsx! {
        button { onclick: move || count.update(|n| *n += 1),
            "Count: " {|| count.get().to_string()}
        }
    }
}
```

> **Note:** The `#[component]` attribute macro is the recommended way to define components. It automatically injects the `__scope: &mut RenderScope` parameter needed by `rsx!`.

---

## Signal

The primary state primitive. Creates a reactive value that notifies subscribers when it changes.

```rust
let count = Signal::new(0);

count.get();              // Read value
count.set(5);             // Set new value
count.update(|n| *n += 1); // Update with function
```

`Signal<T>` implements `Copy`, so you can use it in multiple closures without `.clone()`.

### Example: Toggle

```rust
#[component]
fn toggle() -> NodeHandle {
    let enabled = Signal::new(false);

    rsx! {
        button { onclick: move || enabled.update(|b| *b = !*b),
            {|| if enabled.get() { "ON" } else { "OFF" }}
        }
    }
}
```

For more details, see [Signals](./signals.md).

---

## Effect (Advanced)

Most reactive DOM updates use `{|| expr}` closures in rsx. For rare cases like syncing to external systems, use `Effect` — import it explicitly since it's not in the prelude:

```rust
use rinch::reactive::Effect;

let count = Signal::new(0);

// Auto-tracks count — re-runs when count changes
Effect::new(move || {
    println!("Count changed to: {}", count.get());
});
```

No dependency arrays needed — dependencies are discovered automatically at runtime. For more details, see [Effects](./effects.md).

> **Tip:** If you're updating the DOM, use `{|| expr}` in rsx. If you're updating state, put logic in a [store method](./stores.md). Effect is for the rare case where you need to react to signal changes outside of both.

---

## Memo

Create cached computed values that only recompute when dependencies change.

```rust
let first_name = Signal::new("Alice".to_string());
let last_name = Signal::new("Smith".to_string());

// Automatically updates when first_name or last_name change
let full_name = Memo::new(move || {
    format!("{} {}", first_name.get(), last_name.get())
});

println!("Full name: {}", full_name.get());
```

`Memo<T>` implements `Copy` just like `Signal<T>` — use it in multiple closures without `.clone()`.

For more details, see [Memos](./memos.md).

---

## Context

Share state across components without prop drilling. Call `create_context()` in an ancestor component, then `use_context::<T>()` in any descendant.

### Creating Context

```rust
#[derive(Clone)]
struct Theme {
    primary: String,
    background: String,
}

#[component]
fn app() -> NodeHandle {
    create_context(Theme {
        primary: "#007bff".into(),
        background: "#ffffff".into(),
    });

    rsx! {
        div {
            // ... child components can access Theme
        }
    }
}
```

### Consuming Context

`use_context::<T>()` returns `T` directly. It **panics** with a helpful message if the context was not found:

```
Context not found: TypeName
Did you forget to call create_context() in a parent component?
```

```rust
#[component]
fn themed_button() -> NodeHandle {
    let theme = use_context::<Theme>();

    rsx! {
        button { style: {|| format!("background: {}", theme.primary)},
            "Click me"
        }
    }
}
```

### Fallible Access

Use `try_use_context::<T>()` when a context may legitimately be absent — it returns `Option<T>` instead of panicking:

```rust
#[component]
fn themed_button() -> NodeHandle {
    let theme = try_use_context::<Theme>();
    let bg = theme.map(|t| t.primary).unwrap_or("#ccc".into());

    rsx! {
        button { style: {|| format!("background: {}", bg)},
            "Click me"
        }
    }
}
```

For more details, see [Sharing State](./sharing-state.md).

---

## Complete Example

```rust
use rinch::prelude::*;

#[derive(Clone)]
struct AppSettings {
    dark_mode: bool,
}

#[component]
fn app() -> NodeHandle {
    // Create shared settings context
    create_context(AppSettings { dark_mode: false });

    let todos = Signal::new(vec!["Learn Rust".to_string()]);
    let input = Signal::new(String::new());

    // Derived state
    let count = Memo::new(move || todos.get().len());

    // Extract shared handler — both Enter key and button click add a todo.
    // Signal is Copy, so no .clone() needed before closures.
    let add_todo = move || {
        let text = input.get();
        if !text.is_empty() {
            todos.update(|t| t.push(text.clone()));
            input.set(String::new());
        }
    };

    rsx! {
        div {
            h1 { "Todos (" {|| count.get().to_string()} ")" }

            input {
                value: {|| input.get()},
                oninput: move |value: String| input.set(value),
                onsubmit: add_todo,
            }

            button { onclick: add_todo, "Add" }

            for todo in todos.get() {
                li { {todo.clone()} }
            }
        }
    }
}

fn main() {
    run("Todo App", 400, 300, app);
}
```
