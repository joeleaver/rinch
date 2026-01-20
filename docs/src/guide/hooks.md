# Hooks

Rinch provides a **React-style hooks API** for managing state and side effects in your components. Hooks offer a clean, ergonomic alternative to manually managing `Signal`, `Effect`, and `Memo` instances.

## Why Hooks?

Hooks provide:
- **Automatic lifecycle management** - State persists across re-renders without manual setup
- **Familiar API** - If you've used React, you'll feel right at home
- **Cleaner code** - No need for `thread_local!` or manual `Rc` cloning

```rust
use rinch::prelude::*;

fn counter() -> Element {
    let count = use_signal(|| 0);
    let count_inc = count.clone();

    rsx! {
        Window { title: "Counter",
            button { onclick: move || count_inc.update(|n| *n += 1),
                "Count: " {count.get()}
            }
        }
    }
}
```

## Available Hooks

| Hook | Purpose |
|------|---------|
| [`use_signal`](#use_signal) | Reactive state that triggers re-renders |
| [`use_state`](#use_state) | State with `(value, setter)` tuple API |
| [`use_ref`](#use_ref) | Mutable reference (no re-renders) |
| [`use_effect`](#use_effect) | Side effects when deps change |
| [`use_effect_cleanup`](#use_effect_cleanup) | Effects with cleanup functions |
| [`use_mount`](#use_mount) | One-time effect on first render |
| [`use_memo`](#use_memo) | Memoized computations |
| [`use_callback`](#use_callback) | Memoized callbacks |
| [`use_context`](#use_context) | Access shared state |
| [`use_derived`](#use_derived) | Computed state from signals |

---

## use_signal

The primary state hook. Returns a `Signal<T>` that persists across re-renders.

```rust
let count = use_signal(|| 0);

count.get();              // Read value
count.set(5);             // Set new value
count.update(|n| *n += 1); // Update with function
```

The initialization function only runs on the first render. Subsequent renders return the existing signal.

### Example: Toggle

```rust
fn toggle() -> Element {
    let enabled = use_signal(|| false);
    let enabled_toggle = enabled.clone();

    rsx! {
        Window { title: "Toggle",
            button { onclick: move || enabled_toggle.update(|b| *b = !*b),
                {if enabled.get() { "ON" } else { "OFF" }}
            }
        }
    }
}
```

---

## use_state

React-style tuple API. Returns `(value, setter)` for simpler cases.

```rust
let (count, set_count) = use_state(|| 0);

// Read directly
println!("Count: {}", count);

// Set with closure
set_count(count + 1);
```

### When to Use

- `use_signal` when you need `.update()` or `.with()` methods
- `use_state` for simple read/write patterns

---

## use_ref

Mutable reference that does **not** trigger re-renders. Useful for:
- Storing values that shouldn't cause updates
- Tracking render counts
- Caching expensive computations

```rust
let render_count = use_ref(|| 0);
*render_count.borrow_mut() += 1;

let cached_data = use_ref(|| load_expensive_data());
```

---

## use_effect

Run side effects when dependencies change.

```rust
let count = use_signal(|| 0);

// Re-runs when count changes
use_effect(|| {
    println!("Count changed to: {}", count.get());
}, count.get());
```

The second argument is the dependency. When it changes (compared by equality), the effect re-runs.

### Multiple Dependencies

Use a tuple for multiple dependencies:

```rust
let a = use_signal(|| 0);
let b = use_signal(|| 0);

use_effect(|| {
    println!("a={}, b={}", a.get(), b.get());
}, (a.get(), b.get()));
```

---

## use_effect_cleanup

Effects that need cleanup (like subscriptions or timers).

```rust
let id = use_signal(|| 1);

use_effect_cleanup(|| {
    let current_id = id.get();
    subscribe_to_updates(current_id);

    // Return cleanup function
    move || {
        unsubscribe_from_updates(current_id);
    }
}, id.get());
```

The cleanup function runs:
1. Before the effect re-runs (when deps change)
2. When the component unmounts

---

## use_mount

Run an effect only once, on first render.

```rust
use_mount(|| {
    println!("Component mounted!");

    // Optional: return cleanup function
    || println!("Component unmounted!")
});
```

Equivalent to `useEffect(() => { ... }, [])` in React.

---

## use_memo

Memoize expensive computations. Only recomputes when dependencies change.

```rust
let items = use_signal(|| vec![1, 2, 3, 4, 5]);

// Only recomputes when items change
let sum = use_memo(|| {
    items.get().iter().sum::<i32>()
}, items.get());

let total = sum.get(); // Cached value
```

### When to Use

- Expensive calculations (sorting, filtering, aggregating)
- Derived data that multiple components need
- Avoiding redundant computation on every render

---

## use_callback

Memoize callback functions. Useful for passing stable callbacks to child components.

```rust
let count = use_signal(|| 0);

let increment = use_callback(|| {
    count.update(|n| *n += 1);
}, count.get());

// `increment` has a stable identity when `count` hasn't changed
```

---

## use_context

Access shared state across components without prop drilling.

### Creating Context

At the top of your component tree:

```rust
#[derive(Clone)]
struct Theme {
    primary: String,
    background: String,
}

fn app() -> Element {
    // Create context available to all descendants
    create_context(Theme {
        primary: "#007bff".into(),
        background: "#ffffff".into(),
    });

    rsx! {
        Window { title: "App",
            // ... child components can access Theme
        }
    }
}
```

### Consuming Context

In any descendant component:

```rust
fn themed_button() -> Element {
    let theme: Option<Theme> = use_context();

    let bg = theme.map(|t| t.primary).unwrap_or("#ccc".into());

    rsx! {
        button { style: format!("background: {bg}"),
            "Click me"
        }
    }
}
```

---

## use_derived

Create computed state that automatically tracks signal dependencies.

```rust
let first_name = use_signal(|| "Alice".to_string());
let last_name = use_signal(|| "Smith".to_string());

// Automatically updates when first_name or last_name change
let full_name = use_derived(|| {
    format!("{} {}", first_name.get(), last_name.get())
});

println!("Full name: {}", full_name.get());
```

Unlike `use_memo`, `use_derived` doesn't require explicit dependencies - it automatically tracks any signals read inside the closure.

---

## Rules of Hooks

Hooks must be called **in the same order** every render. This is how rinch tracks which hook corresponds to which state.

### Do: Call at Top Level

```rust
fn app() -> Element {
    let count = use_signal(|| 0);      // Always first
    let name = use_signal(|| "".into()); // Always second

    rsx! { /* ... */ }
}
```

### Don't: Call Conditionally

```rust
fn app() -> Element {
    let show = use_signal(|| false);

    // BAD: Hook order changes based on condition
    if show.get() {
        let extra = use_signal(|| 0);  // Sometimes first, sometimes not!
    }

    rsx! { /* ... */ }
}
```

### Don't: Call in Loops

```rust
fn app() -> Element {
    // BAD: Number of hooks depends on items length
    for i in 0..items.len() {
        let state = use_signal(|| i);  // Wrong!
    }

    rsx! { /* ... */ }
}
```

### Don't: Call in Event Handlers

```rust
fn app() -> Element {
    rsx! {
        button { onclick: || {
            let x = use_signal(|| 0);  // Wrong! Not during render
        }}
    }
}
```

---

## Complete Example

```rust
use rinch::prelude::*;

#[derive(Clone)]
struct AppSettings {
    dark_mode: bool,
}

fn app() -> Element {
    // Create shared settings context
    create_context(AppSettings { dark_mode: false });

    let todos = use_signal(|| vec!["Learn Rust".to_string()]);
    let input = use_signal(|| String::new());

    // Derived state
    let count = use_derived(|| todos.get().len());

    // Log changes
    use_effect(|| {
        println!("Todo count: {}", count.get());
    }, count.get());

    // Setup on mount
    use_mount(|| {
        println!("App started!");
        || println!("App closing...")
    });

    let todos_add = todos.clone();
    let input_submit = input.clone();
    let input_change = input.clone();

    rsx! {
        Window { title: "Todo App", width: 400, height: 300,
            div {
                h1 { "Todos (" {count.get()} ")" }

                input {
                    value: {input.get()},
                    oninput: move |e| input_change.set(e.value())
                }

                button { onclick: move || {
                    let text = input_submit.get();
                    if !text.is_empty() {
                        todos_add.update(|t| t.push(text.clone()));
                        input_submit.set(String::new());
                    }
                }, "Add" }

                ul {
                    {todos.get().iter().map(|t| rsx! {
                        li { {t.clone()} }
                    }).collect::<Vec<_>>()}
                }
            }
        }
    }
}

fn main() {
    rinch::run(app);
}
```
