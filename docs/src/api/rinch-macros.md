# rinch-macros

Procedural macros for rinch: the `rsx!` macro for building UI and the `#[component]` attribute macro for ergonomic component definitions.

## `rsx!`

A JSX-like macro for building UI elements via DOM construction.

### Basic Usage

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { class: "container",
            h1 { "Hello, World!" }
            p { "Welcome to rinch." }
        }
    }
}
```

### Syntax

#### Elements

HTML elements use lowercase names:

```rust
rsx! {
    div { }
    span { }
    button { }
    input { }
}
```

Rinch components and control-flow use PascalCase:

```rust
rsx! {
    Button { label: "Click me" }
    TextInput { placeholder: "Enter text..." }
    Stack { }
    Group { }
    Show { when: {|| visible.get()}, div { "Visible!" } }
    For { each: {|| items.get()}, |item| rsx! { div { } } }
    Fragment { }
}
```

#### Attributes

Attributes are key-value pairs before children:

```rust
rsx! {
    div { class: "container", id: "main",
        // children
    }
}
```

#### Text Content

Text is included directly:

```rust
rsx! {
    p { "Hello, World!" }
    span { "Multiple " "strings " "work" }
}
```

#### Static Expressions

Rust expressions in curly braces are captured once at render time:

```rust
let name = "World";
rsx! {
    p { "Hello, " {name} "!" }
}
```

#### Reactive Expressions

Use closure syntax `{|| expr}` for values that update automatically when signals change:

```rust
let count = use_signal(|| 0);

rsx! {
    // Static - captured once, never updates
    p { {count.get().to_string()} }

    // Reactive - creates an Effect, updates when count changes
    p { {|| count.get().to_string()} }

    // Reactive attribute
    div { class: {|| if count.get() > 5 { "high" } else { "low" }}, "Value" }

    // Reactive style
    div { style: {|| format!("width: {}px", count.get() * 10)}, "Bar" }
}
```

#### Event Handlers

Events use `onevent: handler` syntax with no-argument closures:

```rust
rsx! {
    button {
        onclick: || println!("Clicked!"),
        "Click me"
    }
}
```

### Expansion

The `rsx!` macro expands to DOM construction code using `RenderScope` and `NodeHandle`. It does **not** generate HTML strings or `Element` enum variants.

```rust
// This:
rsx! {
    div { class: "wrapper",
        p { "Hello" }
    }
}

// Expands to approximately:
{
    let __elem0 = __scope.create_element("div");
    __elem0.set_attribute("class", "wrapper");
    let __elem1 = __scope.create_element("p");
    let __text0 = __scope.create_text("Hello");
    __elem1.append_child(&__text0);
    __elem0.append_child(&__elem1);
    __elem0
}
```

Reactive expressions expand to Effects that surgically update DOM nodes:

```rust
// This:
rsx! { p { {|| count.get().to_string()} } }

// Creates an Effect that calls node.set_text() when the signal changes
```

PascalCase components invoke the widget's `render()` method or the component function, passing `__scope` and any children.

### Notes

- The macro requires `__scope: &mut RenderScope` to be in scope (provided automatically by `#[component]`)
- HTML elements become `create_element()` calls, not HTML strings
- Component props use default values where not specified
- The macro is compile-time, so syntax errors appear at build time
- Helpful error messages include typo suggestions for misspelled attributes

## `#[component]` Attribute Macro

Auto-injects `__scope: &mut RenderScope` as the first parameter of a component function. This is the recommended way to define components.

### Usage

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! { div { "Hello!" } }
}
// Expands to: fn app(__scope: &mut RenderScope) -> NodeHandle { ... }

// With extra parameters:
#[component]
fn card(title: &str) -> NodeHandle {
    rsx! { div { {title} } }
}
// Expands to: fn card(__scope: &mut RenderScope, title: &str) -> NodeHandle { ... }
```

### Why Use It

- Eliminates boilerplate: no need to manually write `__scope: &mut RenderScope`
- Makes component signatures cleaner and focused on the component's own props
- Works with any number of additional parameters
- The `rsx!` macro requires `__scope` to be in scope, and `#[component]` provides it automatically
