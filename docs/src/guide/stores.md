# State Management with Stores

Stores are the recommended way to manage shared state in rinch. A store is a plain struct with `Signal` fields and methods that encapsulate state mutations.

## The Store Pattern

```rust
use rinch::prelude::*;

#[derive(Clone, Copy)]
struct CounterStore {
    count: Signal<i32>,
}

impl CounterStore {
    fn new() -> Self {
        Self { count: Signal::new(0) }
    }

    fn increment(&self) {
        self.count.update(|n| *n += 1);
    }

    fn decrement(&self) {
        self.count.update(|n| *n -= 1);
    }

    fn reset(&self) {
        self.count.set(0);
    }
}
```

**Why this works well:**
- State and mutations live together — no scattered signal updates across components
- `Signal<T>` is `Copy`, so the store struct is `Copy` when all fields are signals
- Methods are the single source of truth for how state changes
- Easy to test, easy to reason about

## Providing and Consuming Stores

Call `create_store()` in a parent component to make the store available to all descendants. Call `use_store::<T>()` in any descendant to retrieve it.

```rust
#[component]
fn app() -> NodeHandle {
    create_store(CounterStore::new());

    rsx! {
        div {
            CounterDisplay {}
            CounterControls {}
        }
    }
}

#[component]
fn counter_display() -> NodeHandle {
    let store = use_store::<CounterStore>();

    rsx! {
        p { "Count: " {|| store.count.get().to_string()} }
    }
}

#[component]
fn counter_controls() -> NodeHandle {
    let store = use_store::<CounterStore>();

    rsx! {
        div {
            button { onclick: move || store.decrement(), "-" }
            button { onclick: move || store.reset(), "Reset" }
            button { onclick: move || store.increment(), "+" }
        }
    }
}
```

`use_store::<T>()` panics with a helpful message if no store of that type exists:

```
Store not found: CounterStore
Did you forget to call create_store() in a parent component?
```

Use `try_use_store::<T>()` when a store may legitimately be absent — it returns `Option<T>`.

## A Larger Example: Todo Store

```rust
use rinch::prelude::*;

#[derive(Clone, PartialEq)]
struct Todo {
    id: u32,
    text: String,
    done: bool,
}

#[derive(Clone, Copy)]
struct TodoStore {
    todos: Signal<Vec<Todo>>,
    next_id: Signal<u32>,
    filter: Signal<Filter>,
}

#[derive(Clone, Copy, PartialEq)]
enum Filter { All, Active, Completed }

impl TodoStore {
    fn new() -> Self {
        Self {
            todos: Signal::new(Vec::new()),
            next_id: Signal::new(1),
            filter: Signal::new(Filter::All),
        }
    }

    fn add(&self, text: String) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.todos.update(|t| t.push(Todo { id, text, done: false }));
    }

    fn toggle(&self, id: u32) {
        self.todos.update(|todos| {
            if let Some(todo) = todos.iter_mut().find(|t| t.id == id) {
                todo.done = !todo.done;
            }
        });
    }

    fn remove(&self, id: u32) {
        self.todos.update(|t| t.retain(|todo| todo.id != id));
    }

    fn visible_todos(&self) -> Vec<Todo> {
        let todos = self.todos.get();
        match self.filter.get() {
            Filter::All => todos,
            Filter::Active => todos.into_iter().filter(|t| !t.done).collect(),
            Filter::Completed => todos.into_iter().filter(|t| t.done).collect(),
        }
    }

    fn remaining_count(&self) -> usize {
        self.todos.with(|t| t.iter().filter(|t| !t.done).count())
    }
}
```

Components consume the store and call its methods:

```rust
#[component]
fn todo_app() -> NodeHandle {
    create_store(TodoStore::new());
    let store = use_store::<TodoStore>();
    let input = Signal::new(String::new());

    rsx! {
        div {
            TextInput {
                placeholder: "What needs to be done?",
                value_fn: move || input.get(),
                oninput: move |val: String| input.set(val),
                onsubmit: move || {
                    let text = input.get();
                    if !text.is_empty() {
                        store.add(text);
                        input.set(String::new());
                    }
                },
            }

            p { {|| format!("{} items left", store.remaining_count())} }

            for todo in store.visible_todos() {
                div { key: todo.id,
                    Checkbox {
                        label: todo.text.clone(),
                        checked_fn: {
                            let done = todo.done;
                            move || done
                        },
                        on_change: {
                            let id = todo.id;
                            move || store.toggle(id)
                        },
                    }
                    button {
                        onclick: { let id = todo.id; move || store.remove(id) },
                        "Delete"
                    }
                }
            }
        }
    }
}
```

## Side Effects Belong in Store Methods

Keep side effects (logging, persistence, validation) inside store methods rather than scattered across components:

```rust
impl TodoStore {
    fn add(&self, text: String) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.todos.update(|t| t.push(Todo { id, text: text.clone(), done: false }));

        // Side effect lives here, not in 5 different components
        println!("Added todo #{id}: {text}");
    }
}
```

This keeps components focused on rendering and event handling.

## When You Don't Need a Store

For **component-local state** that no other component needs, just use `Signal::new()` directly:

```rust
#[component]
fn toggle_button() -> NodeHandle {
    let expanded = Signal::new(false);

    rsx! {
        button {
            onclick: move || expanded.update(|v| *v = !*v),
            {|| if expanded.get() { "Collapse" } else { "Expand" }}
        }
    }
}
```

No store needed — `expanded` is only used by this one component.

**Use a store when:**
- Multiple components need to read or write the same state
- You want to centralize mutation logic (validation, side effects)
- State has complex update rules that benefit from named methods

**Use a plain Signal when:**
- State is local to a single component (toggles, input text, hover state)
- No other component cares about this value

## Advanced: Explicit Effects

Most reactive DOM updates use `{|| expr}` closures in rsx — you rarely need `Effect` directly. For advanced cases like syncing state to an external system, import it explicitly:

```rust
use rinch::reactive::Effect;

let count = Signal::new(0);

// Sync to an external system whenever count changes
Effect::new(move || {
    update_external_dashboard(count.get());
});
```

**You probably don't need Effect.** If you're updating the DOM, use `{|| expr}` in rsx. If you're updating state, put the logic in a store method. `Effect` is for the rare case where you need to react to signal changes outside of both rendering and user actions.
