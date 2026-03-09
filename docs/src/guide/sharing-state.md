# Sharing State

There are three ways to share state between components in Rinch: **stores** (recommended), **lifting state up** (passing signals as props), and **context** (low-level implicit sharing).

> **Recommended:** For most shared state, use the [store pattern](./stores.md) — a struct with Signal fields shared via `create_store()` / `use_store()`.

## Stores (Recommended)

See the full [Stores guide](./stores.md). Quick summary:

```rust
#[derive(Clone, Copy)]
struct AppStore {
    count: Signal<i32>,
}

impl AppStore {
    fn new() -> Self { Self { count: Signal::new(0) } }
    fn increment(&self) { self.count.update(|n| *n += 1); }
}

#[component]
fn app() -> NodeHandle {
    create_store(AppStore::new());
    rsx! { div { Counter {} } }
}

#[component]
fn counter() -> NodeHandle {
    let store = use_store::<AppStore>();
    rsx! {
        p { {|| store.count.get().to_string()} }
        button { onclick: move || store.increment(), "+" }
    }
}
```

---

## Lifting State Up

The simplest approach — move state to the nearest common ancestor and pass it down as props.

```rust
use rinch::prelude::*;

#[component]
pub fn Counter(count: Signal<i32>, children: &[NodeHandle]) -> NodeHandle {
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            {children}
        }
    }
}

#[component]
fn app() -> NodeHandle {
    // State lives here, shared with both children
    let count = Signal::new(0);

    rsx! {
        div {
            Counter { count,
                button { onclick: move || count.update(|n| *n += 1), "+" }
                button { onclick: move || count.update(|n| *n -= 1), "-" }
            }
        }
    }
}
```

`Signal<T>` implements `Copy`, so you can pass it to multiple children without `.clone()`.

**When to use:** When only a small number of nearby components need the same state.

---

## Context

Context lets you share state without explicitly threading it through props. Call `create_context()` in an ancestor component, then `use_context::<T>()` in any descendant.

### Creating Context

```rust
#[derive(Clone)]
struct Theme {
    primary: String,
    dark_mode: bool,
}

#[component]
fn app() -> NodeHandle {
    create_context(Theme {
        primary: "#007bff".into(),
        dark_mode: false,
    });

    rsx! {
        div {
            // All descendants can access Theme
            Toolbar {}
            MainContent {}
        }
    }
}
```

### Consuming Context

`use_context::<T>()` returns `T` directly. It **panics** with a helpful message if the context is absent:

```
Context not found: Theme
Did you forget to call create_context() in a parent component?
```

```rust
#[component]
fn toolbar() -> NodeHandle {
    let theme = use_context::<Theme>();

    rsx! {
        nav { style: {|| format!("background: {}", theme.primary)},
            "Toolbar"
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

**When to use:** When a component can render sensibly with or without the context (e.g., a component used both inside and outside a theme provider).

---

## Reactive Context with Signals

For context that changes over time, store a `Signal<T>` in context so descendants can both read and update it:

```rust
#[derive(Clone, Copy)]
struct AppState {
    dark_mode: Signal<bool>,
    user_name: Signal<String>,
}

#[component]
fn app() -> NodeHandle {
    create_context(AppState {
        dark_mode: Signal::new(false),
        user_name: Signal::new("Guest".into()),
    });

    rsx! {
        div {
            Header {}
            MainContent {}
        }
    }
}

#[component]
fn header() -> NodeHandle {
    let state = use_context::<AppState>();

    rsx! {
        header {
            span { {|| state.user_name.get()} }
            button {
                onclick: move || state.dark_mode.update(|v| *v = !*v),
                {|| if state.dark_mode.get() { "Light mode" } else { "Dark mode" }}
            }
        }
    }
}
```

Because `Signal<T>` is `Copy`, the `AppState` struct itself is `Copy` when all its fields are signals, making it cheap to access from any descendant.

---

## Which Approach to Use?

| Situation | Approach |
|-----------|----------|
| Shared state with action methods | [Store](./stores.md) (`create_store` / `use_store`) |
| 1--2 nearby components | Lift state up (pass signals as props) |
| Framework-internal shared state | Context (`create_context` / `use_context`) |
| Optional / may not always be provided | `try_use_store` or `try_use_context` |
| Static configuration (theme colors, locale) | Plain value in context |
