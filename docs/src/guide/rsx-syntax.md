# RSX Syntax

RSX is a JSX-like syntax for building UI in Rust. It lets you write HTML-like markup directly in your Rust code.

## How RSX Works

The `rsx!` macro generates DOM construction code that:
1. Creates DOM nodes via `RenderScope`
2. Sets up Effects for reactive expressions `{|| ...}`
3. Wires event handlers

This enables fine-grained reactive updates - when a signal changes, only the affected DOM nodes are updated, not the entire tree.

## Component Signature

Components use the `#[component]` attribute macro and return a `NodeHandle`:

```rust
use rinch::prelude::*;

#[component]
fn my_component() -> NodeHandle {
    rsx! {
        div { "Hello from my component!" }
    }
}
```

The `#[component]` macro injects the `__scope: &mut RenderScope` parameter automatically, which is used by `rsx!` to create DOM nodes and Effects.

## Basic Syntax

```rust
use rinch::prelude::*;

#[component]
fn example() -> NodeHandle {
    rsx! {
        div {
            h1 { "Hello, World!" }
            p { "This is a paragraph." }
        }
    }
}
```

## HTML Elements

Standard HTML elements are written in lowercase:

```rust
rsx! {
    div {
        span { "Text content" }
        button { "Click me" }
        input { type: "text", placeholder: "Enter text..." }
    }
}
```

## Attributes

Attributes are specified after the element name:

```rust
rsx! {
    div { class: "container", id: "main",
        a { href: "https://example.com", "Link text" }
        img { src: "image.png", alt: "Description" }
    }
}
```

## Components

Components are custom UI elements written in PascalCase. They implement the `Component` trait and render directly to DOM nodes:

```rust
rsx! {
    Button { variant: "filled", color: "blue", onclick: || println!("clicked"),
        "Click me"
    }

    Alert { icon: Icon::InfoCircle, color: "blue", title: "Info",
        "This is an informational message."
    }
}
```

Note: Shell-level constructs like windows, menus, and themes are configured at the runtime level via props, not in RSX.

## Fragments

Use empty braces to group multiple elements without a wrapper:

```rust
rsx! {
    div {
        // Multiple children without extra wrapper
        span { "First" }
        span { "Second" }
    }
}
```

## Text Content

Text can be included directly in elements:

```rust
rsx! {
    p { "This is text content" }
    span { "Multiple " "strings " "concatenated" }
}
```

## Expressions

Rust expressions can be embedded in curly braces:

```rust
let name = "World";
let count = 42;

rsx! {
    p { "Hello, " {name} "!" }
    p { "Count: " {count} }
}
```

## Reactive Expressions

For fine-grained reactivity, use **closure syntax** `{|| expr}` to create expressions that automatically update when signals change. Without the closure, values are captured once at initial render and never update.

### Static vs Reactive

```rust
let count = use_signal(|| 0);

rsx! {
    // ❌ STATIC: Captured once, never updates
    p { "Count: " {count.get()} }

    // ✅ REACTIVE: Creates an effect, updates when count changes
    p { "Count: " {|| count.get().to_string()} }
}
```

### Reactive Text

Wrap dynamic text in a closure to make it reactive:

```rust
let name = use_signal(|| "World".to_string());
let count = use_signal(|| 0);

rsx! {
    // Simple reactive text
    h1 { "Hello, " {|| name.get()} "!" }

    // Reactive with formatting
    p { {|| format!("You clicked {} times", count.get())} }

    // Conditional text
    p { {|| if count.get() > 10 { "Many clicks!" } else { "Keep clicking" }} }
}
```

### Reactive Styles

Style attributes can also be reactive:

```rust
let progress = use_signal(|| 50);
let is_active = use_signal(|| false);

rsx! {
    // Reactive width
    div {
        class: "progress-bar",
        style: {|| format!("width: {}%", progress.get())}
    }

    // Multiple reactive style properties
    div {
        style: {|| format!(
            "background: {}; opacity: {}",
            if is_active.get() { "blue" } else { "gray" },
            if is_active.get() { "1" } else { "0.5" }
        )}
    }
}
```

### Reactive Classes

Dynamically change CSS classes based on state:

```rust
let is_selected = use_signal(|| false);
let variant = use_signal(|| "primary");

rsx! {
    // Conditional class
    div {
        class: {|| if is_selected.get() { "item selected" } else { "item" }}
    }

    // Dynamic class from state
    button {
        class: {|| format!("btn btn-{}", variant.get())}
    }
}
```

### Important: Closure Must Be the Direct Expression

The `{|| ...}` pattern requires the closure to be the **direct expression** inside the braces. Block expressions with setup code now work:

```rust
let count = use_signal(|| 0);

rsx! {
    // ✅ CORRECT: Closure is the direct expression
    div { style: {|| format!("width: {}px", count.get() * 10)} }

    // ✅ CORRECT: Block with setup + final closure — works!
    div { style: {
        let multiplier = 10;
        move || format!("width: {}px", count.get() * multiplier)
    }}

    // ✅ CORRECT: Compute inside the closure
    div { style: {move || {
        let multiplier = 10;
        format!("width: {}px", count.get() * multiplier)
    }}}
}
```

You can compute intermediate values **outside** the RSX, **inside** the closure body, or in a setup block before the closure.

### Why Closures?

Rust macros operate on syntax, not types. The macro cannot distinguish between:
- `signal.get()` - reading reactive state
- `hashmap.get("key")` - reading a regular value

The closure `{|| ...}` explicitly marks the expression as reactive, telling Rinch to:
1. Create an Effect that wraps this expression
2. Track which signals are read inside the closure
3. Re-run and update the DOM when those signals change

This approach has benefits:
- **Zero runtime overhead** for static content
- **Clear intent** - you know exactly what's reactive
- **Works with Rust's ownership** - closures capture values correctly

### What Supports Reactive Expressions?

| Context | Fine-Grained? | Notes |
|---------|---------------|-------|
| Text content | ✅ Yes | `{|| expr}` creates reactive span |
| `style` attribute | ✅ Yes | Updates specific element's style |
| `class` attribute | ✅ Yes | Updates specific element's class |
| Portal content | ✅ Yes | Content inside portals is reactive |
| `if`/`else` blocks | ✅ Yes | Native reactive conditional rendering |
| `for` loops | ✅ Yes | Native reactive list rendering with keyed reconciliation |
| `match` blocks | ✅ Yes | Native reactive multi-branch rendering |
| `.iter().map()` | ✅ Yes | Creates a `display:contents` wrapper for the Vec |
| Window/Menu | N/A | Native OS elements, not DOM |

## Control Flow

Rinch supports native Rust control flow directly in RSX. All control flow is **always reactive** — the conditions, iterators, and scrutinees are automatically wrapped in closures and tracked by Effects. When the underlying signals change, only the affected branches are updated.

### `if` / `else`

Write standard Rust `if`/`else` directly in RSX:

```rust
let visible = use_signal(|| true);
let count = use_signal(|| 5);

rsx! {
    div {
        // Simple if
        if visible.get() {
            p { "I'm visible!" }
        }

        // if/else
        if count.get() > 10 {
            p { "Big number!" }
        } else {
            p { "Small number" }
        }

        // if/else if/else
        if count.get() > 100 {
            p { "Huge" }
        } else if count.get() > 10 {
            p { "Big" }
        } else {
            p { "Small" }
        }
    }
}
```

#### `if let`

Pattern matching with `if let` is also supported:

```rust
let current_user = use_signal(|| Some("Alice".to_string()));

rsx! {
    div {
        if let Some(name) = current_user.get() {
            p { "Welcome, " {name} "!" }
        } else {
            p { "Please log in" }
        }
    }
}
```

#### How `if` works internally

The `if` block desugars to `show_dom()` — the same runtime function used by the `Show` component. The condition is auto-wrapped in a `move || { ... }` closure, creating an Effect that watches the signals read inside it. When the condition changes:

1. The old branch's scope is disposed (cleaning up nested effects)
2. Old DOM nodes are removed
3. New branch content is rendered with a fresh scope

### `for` loops

Write standard `for..in` loops directly in RSX:

```rust
let todos = use_signal(|| vec![
    Todo { id: 1, name: "Buy groceries".into() },
    Todo { id: 2, name: "Write code".into() },
    Todo { id: 3, name: "Take a walk".into() },
]);

rsx! {
    div {
        for todo in todos.get() {
            div { key: todo.id, {todo.name.clone()} }
        }
    }
}
```

#### Keys

Use `key:` on the first child element or component to enable efficient keyed reconciliation. When the list changes, items with the same key are preserved (not re-rendered), and only new/removed items trigger DOM operations:

```rust
for item in items.get() {
    div { key: item.id,
        span { {item.name.clone()} }
    }
}
```

`key:` also works on components — place it on the component tag directly:

```rust
for item in items.get() {
    TodoRow { key: item.id, label: item.name.clone() }
}
```

If no `key:` prop is provided, items are keyed by their `Debug` representation (fallback).

#### How `for` works internally

The `for` loop desugars to `for_each_dom_typed()`, which:

1. Wraps the iterator in a `move || { ... }` closure (making it reactive)
2. Creates a `<!-- for -->` comment marker in the DOM
3. Evaluates the collection and renders initial items
4. Creates an Effect that watches the collection
5. When the collection changes, uses LIS-based keyed reconciliation (`diff_keyed()`) to compute minimal DOM operations: insert new items, remove deleted items, and reposition moved items
6. For surviving items (same key), compares data via `PartialEq` — only re-renders items whose data actually changed

The loop variable is **owned** (`T`, not `&T`), so you can capture it directly in `move` closures. `let` bindings before the element are available in the `key:` expression too:

```rust
for todo in todos.get() {
    let id = todo.id;
    div { key: id,
        {todo.name.clone()}
        button {
            onclick: move || todos.update(|t| t.retain(|t| t.id != id)),
            "Delete"
        }
    }
}
```

**Note:** The item type must implement `Clone + PartialEq + 'static` for `for` loops to work. This enables efficient data comparison for selective re-rendering.

### `match`

Write standard Rust `match` directly in RSX:

```rust
let tab = use_signal(|| 0);

rsx! {
    div {
        match tab.get() {
            0 => div { "Home page" },
            1 => div { "About page" },
            2 => div { "Settings page" },
            _ => div { "Page not found" },
        }
    }
}
```

#### Match with pattern bindings

Patterns that bind variables work — each arm re-evaluates the scrutinee to extract the bound values:

```rust
let result = use_signal(|| Ok::<String, String>("Hello".into()));

rsx! {
    div {
        match result.get() {
            Ok(value) => p { "Success: " {value} },
            Err(msg) => p { class: "error", "Error: " {msg} },
        }
    }
}
```

#### Match with guards

Guard expressions are supported:

```rust
match score.get() {
    n if n >= 90 => div { "A" },
    n if n >= 80 => div { "B" },
    n if n >= 70 => div { "C" },
    _ => div { "F" },
}
```

#### How `match` works internally

The `match` block desugars to `match_dom()`, which generalizes `show_dom()` to N branches. A discriminant closure returns the index of the active branch (0, 1, 2, ...). When the discriminant changes, the old branch is disposed and the new branch is rendered.

### Programmatic Conditional Rendering (show_dom)

For cases requiring explicit control (e.g., lazy evaluation of children that contain hooks), use `show_dom()` directly:

```rust
show_dom(
    __scope,
    &parent,
    move || visible.get(),           // Condition closure
    |scope| {                        // Then branch
        let div = scope.create_element("div");
        div.set_text("Visible!");
        div
    },
    Some(|scope| {                   // Else branch (optional)
        let div = scope.create_element("div");
        div.set_text("Hidden");
        div
    }),
)
```

### Programmatic List Rendering (for_each_dom_typed)

For cases requiring the raw list API, use `for_each_dom_typed()` directly:

```rust
for_each_dom_typed(
    __scope,
    &parent,
    move || todos.get().into_iter().collect::<Vec<_>>(),
    |todo| todo.id.to_string(),
    |todo, __child_scope| {
        let __scope = __child_scope;
        rsx! { div { {todo.name.clone()} } }
    },
)
```

### How Keyed Reconciliation Works

When the list changes, `for` uses the LIS (Longest Increasing Subsequence) algorithm to find the minimum DOM operations:

```rust
// Before: ["a", "b", "c", "d"]
// After:  ["b", "e", "c", "a"]

// Operations:
// - Remove "d"
// - Insert "e" at index 1
// - Move "a" to index 3
// - Items "b" and "c" stay in place (part of LIS)
```

This means:
- **Unchanged items** keep their DOM nodes (no re-render)
- **Moved items** are repositioned in DOM (no re-create)
- **Changed items** are re-rendered if their data differs (via `PartialEq`)
- **Internal state** (signals, effects) is preserved for unchanged items
- **Only new items** trigger component initialization

> **Note:** The loop variable is owned (`T`, not `&T`), so you can capture it directly in `move` closures without manual field extraction.

## Event Handlers

Events use the `onevent: handler` syntax:

```rust
// Inside a #[component] function:
let count = use_signal(|| 0);

rsx! {
    button {
        onclick: move || count.update(|n| *n += 1),
        "Increment"
    }
}
```

## Sharing Event Handlers

You can extract event handlers into variables and reuse them across multiple elements. This is especially useful when a button and a text input should trigger the same action:

```rust
let input_text = use_signal(|| String::new());
let todos = use_signal(|| Vec::<String>::new());

// Extract shared logic into a closure
let add_todo = move || {
    let text = input_text.get();
    if !text.trim().is_empty() {
        todos.update(|t| t.push(text.trim().to_string()));
        input_text.set(String::new());
    }
};

rsx! {
    // Both TextInput (on Enter) and Button (on click) use the same handler
    TextInput {
        value_fn: move || input_text.get(),
        oninput: move |value: String| input_text.set(value),
        onsubmit: add_todo,
    }
    Button { onclick: add_todo, "Add" }
}
```

This works because `Signal` implements `Copy`, so closures capturing signals can be used in multiple places without `.clone()`.

## Style Shorthands

The `rsx!` macro supports CSS shorthand props on both HTML elements and components. Shorthands expand to `set_style()` calls that merge with existing styles.

Spacing scale values (`xs`, `sm`, `md`, `lg`, `xl`) auto-resolve to `var(--rinch-spacing-{value})`.

### Available Shorthands

| Shorthand | CSS Property | Shorthand | CSS Property |
|-----------|-------------|-----------|-------------|
| `w` | `width` | `m` | `margin` |
| `h` | `height` | `mt` | `margin-top` |
| `miw` | `min-width` | `mb` | `margin-bottom` |
| `maw` | `max-width` | `ml` | `margin-left` |
| `mih` | `min-height` | `mr` | `margin-right` |
| `mah` | `max-height` | `mx` | `margin-left` + `margin-right` |
| `p` | `padding` | `my` | `margin-top` + `margin-bottom` |
| `pt` | `padding-top` | `display` | `display` |
| `pb` | `padding-bottom` | `pos` | `position` |
| `pl` | `padding-left` | `top` | `top` |
| `pr` | `padding-right` | `bottom` | `bottom` |
| `px` | `padding-left` + `padding-right` | `left` | `left` |
| `py` | `padding-top` + `padding-bottom` | `right` | `right` |

### Usage

```rust
// On HTML elements
div { p: "md", m: "lg", w: "200px", "Padded and margined" }

// On components
Stack { gap: "md", p: "xl", maw: "600px",
    Text { "Constrained content" }
}

// Reactive shorthands
let big = use_signal(|| false);
div { p: {|| if big.get() { "xl" } else { "sm" }}, "Dynamic padding" }

// Spacing scale values auto-resolve
div { p: "md" }    // becomes: padding: var(--rinch-spacing-md)
div { p: "20px" }  // becomes: padding: 20px (passed through)
```

### Application Order

Shorthands are applied via `set_style()` after component rendering and after the `style:` prop. This means shorthands win over conflicting properties in `style:`.

## Styling

Inline styles and CSS classes work like regular HTML:

```rust
rsx! {
    html {
        head {
            style {
                "
                .container { padding: 20px; }
                .highlight { color: red; }
                "
            }
        }
        body {
            div { class: "container",
                span { class: "highlight", "Styled text" }
            }
        }
    }
}
```
