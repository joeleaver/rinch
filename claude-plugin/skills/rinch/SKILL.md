---
name: rinch
description: "Best practices for building UIs with the Rinch GUI framework. Proactively guides correct use of rsx! macro, reactive signals, component props, and state management. USE THIS whenever writing or editing Rust code that imports rinch or uses rsx!, Signal, #[component], or rinch components."
---

# Rinch Framework — Correct Usage Guide

You are assisting a developer using **Rinch**, a reactive GUI framework for Rust. Rinch uses an `rsx!` macro (similar to JSX), reactive Signals, and a fine-grained update model. **It is NOT React** — components run once, there are no re-renders. All dynamic updates are surgical DOM mutations via Effects.

Apply these rules whenever you write or review code that uses rinch.

---

## Rule 1: Dynamic values MUST use `{|| expr}` closures

This is the single most important rule. Without the closure wrapper, values are captured once at initial render and **silently never update**. It compiles, it shows the initial value, then it's frozen forever.

```rust
// BUG — captured once, never updates
p { {count.get().to_string()} }

// CORRECT — closure creates a reactive Effect
p { {|| count.get().to_string()} }
```

This applies to **everything dynamic** — text, attributes, styles, classes:

```rust
div { class: {|| if active.get() { "on" } else { "off" }} }
div { style: {|| format!("width: {}px", width.get())} }
span { {|| format!("{} items", items.get().len())} }
```

**Self-check:** Every `.get()` inside `rsx!` should be inside a `{|| ...}`. If it's not, it's almost certainly a bug.

## Rule 2: Don't manually wrap rsx prop values

The `rsx!` macro auto-wraps props. Manual wrapping causes double-wrapping and confusing type errors.

| You write | Macro generates |
|---|---|
| `onclick: move \|\| do_thing()` | `Callback::new(...).into()` |
| `oninput: move \|val\| handle(val)` | `InputCallback::new(...).into()` |
| `value_fn: move \|\| text.get()` | `Some(Rc::new(...))` |
| `icon: Icon::Check` | `Some(Icon::Check)` |
| `variant: "filled"` | `String::from("filled")` |
| `size: 42` | `Some(42)` |
| `disabled: true` | `true` |

```rust
// WRONG — double-wrapped, confusing type errors
Button { onclick: Some(Callback::new(|| ...)) }
Alert { icon: Some(Icon::Check) }
TextInput { value_fn: Some(Rc::new(move || text.get())) }
Button { variant: Some(String::from("filled")) }

// CORRECT — the macro handles wrapping
Button { onclick: move || do_thing() }
Alert { icon: Icon::Check }
TextInput { value_fn: move || text.get() }
Button { variant: "filled" }
```

## Rule 3: Signal and Memo are Copy — never clone them

```rust
let count = Signal::new(0);

// WRONG — unnecessary, Signal is Copy
let count_clone = count.clone();
button { onclick: move || count_clone.update(|n| *n += 1) }

// CORRECT — just use it in multiple closures freely
button { onclick: move || count.update(|n| *n += 1) }
p { {|| count.get().to_string()} }
```

## Rule 4: Component text props are String, not Option

Props like `variant`, `color`, `size`, `label` are `String`. Empty string = not set. The macro converts string literals automatically.

```rust
// WRONG
Button { variant: Some("filled".into()) }

// CORRECT
Button { variant: "filled" }
```

## Rule 5: Only use `rinch::prelude::*`

The prelude re-exports everything from rinch-components and rinch-theme. Don't add separate crate deps.

```toml
# Cargo.toml — all you need
[dependencies]
rinch = { workspace = true, features = ["desktop", "components", "theme"] }
```

**Important:** `"desktop"` must be listed explicitly (workspace uses `default-features = false`).

```rust
use rinch::prelude::*;  // Includes all components, theme, signals, etc.

// DON'T do this — redundant
// use rinch_components::*;
// use rinch_theme::*;
```

## Rule 6: State architecture

| Scope | Use |
|-------|-----|
| Component-local state | `Signal::new()` directly |
| Shared across components | `create_store()` / `use_store()` |
| Framework internals only | `create_context()` / `use_context()` |

`Effect` is intentionally excluded from the prelude. For reactive DOM updates, use `{|| ...}` in rsx. Only import `Effect` explicitly for syncing with external systems.

## Rule 7: Controlled inputs need `value_fn` + `oninput`

Without `value_fn`, programmatic `signal.set("")` won't visually clear the input.

```rust
let text = Signal::new(String::new());

// INCOMPLETE — set("") won't clear the visible input
TextInput {
    oninput: move |val: String| text.set(val),
}

// CORRECT — value_fn keeps DOM in sync both directions
TextInput {
    value_fn: move || text.get(),
    oninput: move |val: String| text.set(val),
    onsubmit: move || {
        process(text.get());
        text.set(String::new());  // Visually clears thanks to value_fn
    },
}
```

## Rule 8: For loops need `key:` props

Items must be `Clone + PartialEq + 'static`. Use `key:` for stable DOM reconciliation.

```rust
for todo in todos.get() {
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: {
                let id = todo.id;
                move || todos.update(|t| t.retain(|t| t.id != id))
            },
            "Delete"
        }
    }
}
```

Items with matching keys are **not** re-rendered when the list changes — their existing DOM is preserved. For per-item reactivity, use per-item Signals.

## Rule 9: Raw HTML `oninput` receives a String

On raw `<input>`/`<textarea>` elements (not the `TextInput` component):

```rust
input { oninput: move |value: String| name.set(value) }
```

`onclick` takes no arguments: `button { onclick: move || do_thing() }`

## Rule 10: Cross-thread signals use `send()`, not `set()`

`set()` and `update()` panic off the main thread.

```rust
std::thread::spawn(move || {
    // WRONG — panics
    // status.set("loading".into());

    // CORRECT
    status.send("loading".into());
    status.update_send(|s| *s = "done".into());
});
```

## Rule 11: All component props accept reactive closures

Pass `{|| expr}` to any component prop to make it reactive:

```rust
let active = Signal::new(false);
Button {
    variant: {|| if active.get() { "filled" } else { "light" }},
    onclick: move || active.update(|v| *v = !*v),
    "Toggle"
}
```

For surgical updates (no full re-render), use `_fn` props where available (`checked_fn`, `value_fn`).

## Rule 12: Component `style:` and `class:` are additive

They merge with the component's own styles/classes, not replace them:

```rust
Button {
    variant: "filled",
    style: "margin-top: 8px",
    class: "my-custom-class",
    "Click"
}
```

## Rule 13: Native control flow is always reactive

`if`, `for`, and `match` inside `rsx!` are automatically tracked by the reactive system:

```rust
rsx! {
    div {
        if visible.get() {
            p { "Shown!" }
        }

        match tab.get() {
            0 => div { "Home" },
            1 => div { "About" },
            _ => div { "404" },
        }
    }
}
```

No special syntax needed — conditions, iterators, and scrutinees are auto-wrapped in Effects.

## Quick Reference: Application Entry Point

```rust
use rinch::prelude::*;

#[component]
fn App() -> NodeHandle {
    let count = Signal::new(0);
    rsx! {
        div {
            p { "Count: " {|| count.get().to_string()} }
            Button {
                variant: "filled",
                onclick: move || count.update(|n| *n += 1),
                "Increment"
            }
        }
    }
}

fn main() {
    run("My App", 800, 600, app);
}
```

## Checklist

Before finishing any rinch code:

- [ ] Every `.get()` in rsx is inside `{|| ...}` (unless intentionally one-shot)
- [ ] No manual `Some()`, `Rc::new()`, `Callback::new()` in rsx props
- [ ] No `.clone()` on Signals or Memos
- [ ] String literals for text props
- [ ] Controlled inputs have both `value_fn` and `oninput`
- [ ] For loops have `key:` on items
- [ ] Cross-thread updates use `send()` / `update_send()`
