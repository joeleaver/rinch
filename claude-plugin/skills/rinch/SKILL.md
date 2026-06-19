---
name: rinch
description: "Best practices for building UIs with the Rinch GUI framework. Proactively guides correct use of rsx! macro, reactive signals, component props, and state management. USE THIS whenever writing or editing Rust code that imports rinch or uses rsx!, Signal, #[component], or rinch components."
---

# Rinch Framework — Correct Usage Guide

You are assisting a developer using **Rinch**, a reactive GUI framework for Rust. Rinch uses an `rsx!` macro (similar to JSX), reactive Signals, and a fine-grained update model. **It is NOT React** — components run once, there are no re-renders. All dynamic updates are surgical DOM mutations via Effects.

Apply these rules whenever you write or review code that uses rinch.

---

## Rule 0: ALWAYS use `rsx!` — NEVER build DOM imperatively

**This is the most common mistake.** Do NOT use `create_element()`, `create_text()`, `set_attribute()`, or `append_child()` to build UI. These are low-level internal APIs. All UI code MUST use the `rsx!` macro.

If you find yourself writing `scope.create_element("div")` or `__scope.create_text("hello")`, **STOP**. You are doing it wrong. Use `rsx!` instead.

| Imperative | rsx! equivalent |
|---|---|
| `scope.create_element("div")` | `div { }` |
| `scope.create_text("hello")` | `"hello"` |
| `node.set_attribute("class", "x")` | `class: "x"` |
| `parent.append_child(&child)` | Nest child inside parent |
| `register_handler(cb)` + `set_attribute("data-rid", ...)` | `onclick: move \|\| cb()` |
| `for_each_dom` / `for_each_dom_typed` | `for item in collection { ... }` in rsx |
| `show_dom` | `if condition { ... }` in rsx |

## Rule 1: NEVER re-render components

Rinch components run **once** to build the DOM. There is no re-render cycle, no virtual DOM diff, no `setState` equivalent that rebuilds a component. **Do not write code that tries to force re-renders.**

Anti-patterns to avoid:
- Calling a component function again to "refresh" it
- Using an Effect to tear down and rebuild a DOM subtree when state changes
- Creating new DOM nodes inside a signal callback to replace existing ones
- Building a "render loop" that reconstructs the UI on each frame

Instead, use `{|| expr}` closures in rsx for reactive updates — they surgically update individual DOM nodes (text, attributes, styles) without touching the rest of the tree. For conditional content, use `if`/`match` in rsx (they're automatically reactive). For lists, use `for` with `key:`.

```rust
// WRONG — rebuilding DOM on every change
Effect::new(move || {
    container.clear_children();
    for item in items.get() {
        let div = scope.create_element("div");
        // ... rebuild everything
    }
});

// CORRECT — declarative, reactive, efficient
rsx! {
    div {
        for item in items.get() {
            div { key: item.id, {item.name.clone()} }
        }
    }
}
```

## Rule 2: Dynamic values MUST use `{|| expr}` closures

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

**Class and style repaint exactly like text.** A reactive `class:` or `style:` closure on a raw element marks the node paint-dirty and triggers a redraw through the same path as reactive text — there is *no* "the attribute updates but the screen doesn't repaint" limitation. So never add manual repaint/refresh workarounds to force a redraw (that violates Rule 1). If a reactive `class:`/`style:` looks frozen, the cause is a missing `{|| }` wrapper (value captured once) or mutating state that isn't a `Signal` — not a missing repaint.

## Rule 3: Don't manually wrap rsx prop values

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

## Rule 4: Signal and Memo are Copy — never clone them

```rust
let count = Signal::new(0);

// WRONG — unnecessary, Signal is Copy
let count_clone = count.clone();
button { onclick: move || count_clone.update(|n| *n += 1) }

// CORRECT — just use it in multiple closures freely
button { onclick: move || count.update(|n| *n += 1) }
p { {|| count.get().to_string()} }
```

## Rule 5: Component text props are String, not Option

Props like `variant`, `color`, `size`, `label` are `String`. Empty string = not set. The macro converts string literals automatically.

```rust
// WRONG
Button { variant: Some("filled".into()) }

// CORRECT
Button { variant: "filled" }
```

## Rule 6: Only use `rinch::prelude::*`

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

## Rule 6b: `rsx!` works in ANY crate that depends on `rinch-core`

The `rsx!` macro generates code using `rinch::core::` paths. In end-user crates, `rinch` is the facade crate. In internal crates, add a shim to `lib.rs`:

```rust
extern crate self as rinch;
#[doc(hidden)]
pub mod core {
    pub use rinch_core::*;
}
```

**Do NOT fall back to imperative code** because you think rsx won't work in a particular crate.

## Rule 7: State architecture

| Scope | Use |
|-------|-----|
| Component-local state | `Signal::new()` directly |
| Shared across components | `create_store()` / `use_store()` |
| Framework internals only | `create_context()` / `use_context()` |

`Effect` is intentionally excluded from the prelude. For reactive DOM updates, use `{|| ...}` in rsx. Only import `Effect` explicitly for syncing with external systems.

## Rule 8: Controlled inputs need `value_fn` + `oninput`

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

## Rule 9: For loops need `key:` props

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

## Rule 10: Raw HTML `oninput` receives a String

On raw `<input>`/`<textarea>` elements (not the `TextInput` component):

```rust
input { oninput: move |value: String| name.set(value) }
```

`onclick` takes no arguments: `button { onclick: move || do_thing() }`

## Rule 11: Cross-thread signals use `send()`, not `set()`

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

## Rule 12: All component props accept reactive closures

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

## Rule 13: Component `style:` and `class:` are additive

They merge with the component's own styles/classes, not replace them:

```rust
Button {
    variant: "filled",
    style: "margin-top: 8px",
    class: "my-custom-class",
    "Click"
}
```

## Rule 14: Native control flow is always reactive

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

## Iterating & Debugging: rinch-debug + MCP server

Rinch ships a debug bridge that lets an AI assistant (or any external tool) **see and drive a running rinch app** — take screenshots, inspect the live DOM with computed styles and layout bounds, and inject input. Use it to visually verify changes instead of guessing.

**Enable it in the app** — add the `debug` feature:

```toml
[dependencies]
rinch = { workspace = true, features = ["desktop", "debug"] }
```

The embedded `rinch-debug` server then auto-starts on a random localhost port and writes a discovery file to `~/.rinch/debug/{pid}.json`. Disable at runtime with `RINCH_DEBUG=0`.

**The MCP server** (`rinch-mcp-server`) is a standalone binary that bridges Claude to running apps. Once configured, these tools become available:

| Tool | Use |
|------|-----|
| `launch_app` | Build, run, and auto-connect to a package |
| `screenshot` | Capture the window as an inline PNG (directly viewable) |
| `dom_tree` / `get_node` | Inspect the live DOM with layout bounds + computed styles |
| `query_selector` | Find nodes by tag, `.class`, `[attr]`, `[attr=value]` |
| `get_computed_styles` | Resolved CSS for a node |
| `click` / `type_text` / `wait_frame` | Drive interaction and observe reactive updates |
| `close_app` | Shut the app down cleanly |

**Recommended loop:** `launch_app` → `screenshot` → inspect (`dom_tree` / `get_computed_styles`) → `click` → `wait_frame` → `screenshot` → `close_app` → edit → repeat. This is the fastest way to confirm layout, styling, and that reactive updates actually repaint.

**If the `rinch__*` MCP tools aren't available, suggest the user install the server.** It is not set up automatically. Point them to:

```bash
cargo build -p rinch-mcp-server   # or: cargo install --path crates/rinch-mcp-server
```

Then add it to `.mcp.json` (pointing at the built binary for fast startup):

```json
{
  "mcpServers": {
    "rinch": {
      "command": "/path/to/rinch/target/debug/rinch-mcp-server",
      "args": [],
      "cwd": "/path/to/rinch"
    }
  }
}
```

(Headless/CI: run under `Xvfb` with `DISPLAY` set; `launch_app` forwards `DISPLAY` automatically.)

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
