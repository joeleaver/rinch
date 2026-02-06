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
| Show component | ✅ Yes | Conditional with fine-grained updates |
| For component | ✅ Yes | Lists with keyed reconciliation |
| `if`/`else` blocks | ❌ No | Structural changes need re-render |
| `.iter().map()` | ❌ No | Use For component instead |
| Window/Menu | N/A | Native OS elements, not DOM |

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
        when: {|| visible.get()},
        then: |__scope| rsx! { div { "This is shown when visible is true!" } },
        fallback: || rsx! { div { "Hidden" } },
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
        when: {|| visible.get()},
        fallback: || rsx! { div { "Hidden" } },

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
| `fallback` | `\|\| Element` (optional) | Content to show when false |
| Children | Elements | Content to show when true (eager evaluation, not used with `then:`) |

### Show with Nested Reactivity

Show preserves reactive content within its children:

```rust
let visible = use_signal(|| true);
let count = use_signal(|| 0);

rsx! {
    Show {
        when: {|| visible.get()},
        then: |__scope| rsx! {
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
    when: {|| active.get()},
    then: |__scope| {
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
            when: {|| current_section.get() == 1},
            // Use lazy evaluation to prevent hook panic
            then: |__scope| section_with_hooks(__scope),
        }
    }
}
```

## List Rendering with For

The `For` component enables efficient list rendering with keyed reconciliation. When the list changes, only affected items are added, removed, or moved - unchanged items keep their DOM nodes and internal state.

```rust
let items = use_signal(|| vec![
    Item { id: "1", name: "Alice" },
    Item { id: "2", name: "Bob" },
    Item { id: "3", name: "Carol" },
]);

rsx! {
    For {
        each: {|| items.get().into_iter().map(|item| {
            ForItem::new(item.id.clone(), item)
        }).collect()},

        |item| {
            let data = item.downcast::<Item>().unwrap();
            rsx! {
                div { class: "list-item",
                    {|| data.name.clone()}
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
| Children | `|item| Element` | Render function for each item |

### ForItem

Each item needs a unique key for reconciliation:

```rust
// Create from any data type
let item = ForItem::new("unique-key", my_data);

// Access the key
let key = item.key; // "unique-key"

// Downcast to original type
if let Some(data) = item.downcast::<MyData>() {
    // use data
}
```

### Helper: to_for_items

Convert any iterator to ForItems:

```rust
use rinch::prelude::*;

let items = to_for_items(
    my_vec.into_iter(),
    |item| item.id.clone(), // key function
);
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
    each: {|| ...},
    |item| {
        let data = item.downcast::<Item>().unwrap();

        // This effect is scoped to this item
        // When the item is removed, the effect is disposed
        rsx! {
            div {
                // Reactive text within the item
                span { {|| data.count.get().to_string()} }
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
