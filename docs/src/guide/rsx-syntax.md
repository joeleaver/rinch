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

## Widgets

Widgets are custom components written in PascalCase. They implement the `Widget` trait and render directly to DOM nodes:

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

The `{|| ...}` pattern requires the closure to be the **direct expression** inside the braces. Block expressions that evaluate to a closure are **not** treated as reactive:

```rust
let count = use_signal(|| 0);

rsx! {
    // ✅ CORRECT: Closure is the direct expression
    div { style: {|| format!("width: {}px", count.get() * 10)} }

    // ✅ CORRECT: Block with a single closure statement
    div { style: {move || format!("width: {}px", count.get() * 10)} }

    // ❌ WRONG: Block expression with multiple statements — NOT reactive
    // The macro sees a multi-statement block, not a closure
    div { style: {
        let multiplier = 10;
        move || format!("width: {}px", count.get() * multiplier)
    }}
}
```

If you need intermediate variables, compute them **outside** the RSX or **inside** the closure:

```rust
// Option 1: Compute outside RSX
let multiplier = 10;
rsx! {
    div { style: {move || format!("width: {}px", count.get() * multiplier)} }
}

// Option 2: Compute inside the closure
rsx! {
    div { style: {move || {
        let multiplier = 10;
        format!("width: {}px", count.get() * multiplier)
    }}}
}
```

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
| Show component | ✅ Yes | Explicit conditional (still supported) |
| For component | ✅ Yes | Explicit lists (still supported) |
| `.iter().map()` | ❌ No | Use `for` loop or `For` component instead |
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

Use `key:` on the first child element to enable efficient keyed reconciliation. When the list changes, items with the same key are preserved (not re-rendered), and only new/removed items trigger DOM operations:

```rust
for item in items.get() {
    div { key: item.id,
        span { {item.name.clone()} }
    }
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

**Important:** Items with matching keys are **not** re-rendered. Their existing DOM subtree is preserved as-is. If you need per-item reactivity (e.g., updating a todo's name), use Signals inside each item.

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

### Native control flow vs Show/For components

| Feature | Native syntax | Component syntax |
|---------|--------------|-----------------|
| Conditional | `if cond { ... }` | `Show { when: ... }` |
| List | `for x in items { ... }` | `For { each: ... }` |
| Multi-branch | `match expr { ... }` | Chain multiple `Show` |
| Lazy evaluation | Always lazy | `then:` prop for lazy |
| Reactivity | Always reactive | Always reactive |
| Verbosity | Minimal | More boilerplate |

Both approaches are fully supported. Native syntax is recommended for new code. The `Show` and `For` components remain available for backward compatibility and for cases where you want explicit control (e.g., the `then:` prop for lazy evaluation).

## Conditional Rendering with Show

The `Show` component enables fine-grained conditional rendering. When the condition changes, only the affected DOM nodes are updated - the rest of the tree stays stable.

### Show Syntax Modes

Show supports two modes for conditional rendering:

#### Lazy Evaluation (Recommended when children contain hooks)

Use the `then:` prop for lazy evaluation. The closure body only executes when the condition becomes true, preventing hooks from running when hidden:

```rust
let visible = use_signal(|| true);

rsx! {
    Show {
        when: {move || visible.get()},
        then: |__scope: &mut RenderScope| rsx! { div { "This is shown when visible is true!" } },
        fallback: |__scope: &mut RenderScope| rsx! { div { "Hidden" } },
    }
}
```

**When to use lazy evaluation:**
- When children call hooks (`use_signal`, `use_effect`, etc.)
- When children render expensive components
- When you want to defer initialization until the condition is true

#### Eager Evaluation (Simpler syntax)

Omit the `then:` prop for eager evaluation. Children are evaluated immediately at render time:

```rust
let visible = use_signal(|| true);

rsx! {
    Show {
        when: {move || visible.get()},
        fallback: |__scope: &mut RenderScope| rsx! { div { "Hidden" } },

        div { "This is shown when visible is true!" }
    }
}
```

**Note:** With eager evaluation, hooks in children still run even when Show is hidden. This can cause panic if hooks are called when the condition is false.

### Show Props

| Prop | Type | Description |
|------|------|-------------|
| `when` | `{|| bool}` | Reactive condition closure |
| `then` | `\|__scope\| Element` (optional) | Lazy-evaluated content to show when true |
| `fallback` | `\|__scope\| NodeHandle` (optional) | Content to show when false |
| Children | Elements | Content to show when true (eager evaluation, not used with `then:`) |

### Show with Nested Reactivity

Show preserves reactive content within its children:

```rust
let visible = use_signal(|| true);
let count = use_signal(|| 0);

rsx! {
    Show {
        when: {move || visible.get()},
        then: |__scope: &mut RenderScope| rsx! {
            // This reactive text updates independently of Show
            p { "Count: " {|| count.get().to_string()} }
            button { onclick: move || count.update(|n| *n += 1), "Increment" }
        },
    }
}
```

### Show Cleanup

When Show toggles, nested effects are automatically cleaned up:

```rust
Show {
    when: {move || active.get()},
    then: |__scope: &mut RenderScope| {
        // When Show toggles to false:
        // - This component's effects are disposed
        // - Cleanup functions run
        // - DOM nodes are removed
        MyComponent {}
    },
}
```

### Example: Component with Hooks

This example shows why lazy evaluation is important:

```rust
#[component]
fn section_with_hooks() -> NodeHandle {
    // This would panic if evaluated when section is hidden
    let local_state = use_signal(|| 0);
    rsx! { div { {|| local_state.get().to_string()} } }
}

#[component]
fn app() -> NodeHandle {
    let current_section = use_signal(|| 1);

    rsx! {
        Show {
            when: {move || current_section.get() == 1},
            // Use lazy evaluation to prevent hook panic
            then: |__scope| section_with_hooks(__scope),
        }
    }
}
```

## List Rendering with For

The `For` component enables efficient list rendering with keyed reconciliation. When the list changes, only affected items are added, removed, or moved - unchanged items keep their DOM nodes and internal state.

### Auto-Downcast Mode (Recommended)

Use a typed parameter to automatically downcast from `ForItem`:

```rust
let items = use_signal(|| vec![
    Item { id: "1", name: "Alice" },
    Item { id: "2", name: "Bob" },
    Item { id: "3", name: "Carol" },
]);

rsx! {
    For {
        each: {move || items.get().into_iter().map(|item| {
            ForItem::new(item.id.clone(), item)
        }).collect()},

        |item: &Item| {
            // Auto-downcast: no manual downcast_ref needed!
            rsx! {
                div { class: "list-item",
                    {item.name.clone()}
                }
            }
        }
    }
}
```

### Manual Mode

You can still use `&ForItem` and manually downcast:

```rust
rsx! {
    For {
        each: {move || items.get().into_iter().map(|item| {
            ForItem::new(item.id.clone(), item)
        }).collect()},

        |item: &ForItem| {
            let data = item.data.downcast_ref::<Item>().unwrap();
            rsx! {
                div { class: "list-item",
                    {data.name.clone()}
                }
            }
        }
    }
}
```

### For Props

| Prop | Type | Description |
|------|------|-------------|
| `each` | `{|| Vec<ForItem>}` | Reactive closure returning items |
| Children | `|item: &T| Element` | Render function for each item (auto-downcast) |
| Children | `|item: &ForItem| Element` | Render function (manual downcast) |

### ForItem

Each item needs a unique key for reconciliation:

```rust
// Create from any data type
let item = ForItem::new("unique-key", my_data);

// Access the key
let key = item.key; // "unique-key"

// Manual downcast to original type (when using |item: &ForItem|)
if let Some(data) = item.data.downcast_ref::<MyData>() {
    // use data
}
```

### Helper: ForItem::from_iter

Convert any iterator to ForItems:

```rust
use rinch::prelude::*;

let items = ForItem::from_iter(my_vec, |item| item.id.clone());
```

The legacy `to_for_items()` function also works but `ForItem::from_iter` is preferred.

### Hooks in For Bodies

Hooks work inside For view closures. Each item gets its own isolated hook scope:

```rust
For {
    each: {move || ForItem::from_iter(todos.get(), |t| t.id.to_string())},
    |item: &Todo| {
        let editing = use_signal(|| false);
        rsx! {
            div {
                span { {item.name.clone()} }
                button {
                    onclick: move || editing.update(|v| *v = !*v),
                    {|| if editing.get() { "Done" } else { "Edit" }}
                }
            }
        }
    }
}
```

### How Keyed Reconciliation Works

When the list changes, For uses the LIS (Longest Increasing Subsequence) algorithm to find the minimum DOM operations:

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
- **Internal state** (signals, effects) is preserved for unchanged items
- **Only new items** trigger component initialization

### Nested Reactivity in For

Items can contain their own reactive content:

```rust
For {
    each: {move || ...},
    |item: &Item| {
        // Auto-downcast: item is already &Item

        // This effect is scoped to this item
        // When the item is removed, the effect is disposed
        rsx! {
            div {
                // Reactive text within the item
                span { {|| item.count.get().to_string()} }
            }
        }
    }
}
```

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

## Style Shorthands

The `rsx!` macro supports CSS shorthand props on both HTML elements and widgets. Shorthands expand to `set_style()` calls that merge with existing styles.

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

// On widgets
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

Shorthands are applied via `set_style()` after widget rendering and after the `style:` prop. This means shorthands win over conflicting properties in `style:`.

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
