# Writing Your Own Components

You have two options. The first one is almost always what you want.

## Option 1: PascalCase Component Functions

Write a function with a PascalCase name and `#[component]`. The macro generates a struct, a `Default` impl, and a `Component` trait impl. Parameters become props. Done.

```rust
use rinch::prelude::*;

#[component]
pub fn UserCard(
    name: String,
    email: String,
    avatar_url: String,
    online: bool,
    onclick: Option<Callback>,
) -> NodeHandle {
    rsx! {
        Paper { shadow: "sm", p: "md",
            Group { gap: "md",
                Avatar { src: avatar_url.clone(), size: "lg" }
                Stack { gap: "xs",
                    Text { fw: "bold", {name.clone()} }
                    Text { size: "sm", color: "dimmed", {email.clone()} }
                }
                if online {
                    Badge { color: "green", variant: "dot", "Online" }
                }
            }
        }
    }
}
```

Use it:

```rust
rsx! {
    UserCard {
        name: "Alice",
        email: "alice@example.com",
        avatar_url: "/avatars/alice.png",
        online: true,
        onclick: || println!("clicked!"),
    }
}
```

### The Rules

**Prop types must be owned.** `String`, not `&str`. `Vec<T>`, not `&[T]`. The macro will give you a clear error if you use a reference type.

**The macro wraps things for you.** Don't fight it:

| What you write | What the macro generates |
|---|---|
| `onclick: \|\| do_thing()` | `Callback::new(\|\| do_thing()).into()` |
| `oninput: move \|v: String\| ...` | `InputCallback::new(move \|v\| ...).into()` |
| `icon: Icon::Check` | `Some(Icon::Check)` |
| `variant: "filled"` | `String::from("filled")` |
| `disabled: true` | `true` (no wrapping) |
| `size: 42` | `Some(42)` |
| `value_fn: move \|\| text.get()` | `Some(Rc::new(move \|\| text.get()))` |

**Don't manually wrap.** Writing `onclick: Some(Callback::new(|| ...))` double-wraps and you'll get confusing type errors.

**`children: &[NodeHandle]` is special.** It's not a struct field — it captures child elements passed in RSX:

```rust
#[component]
pub fn Card(title: String, children: &[NodeHandle]) -> NodeHandle {
    rsx! {
        Paper { shadow: "sm", p: "md",
            Title { order: 3, {title.clone()} }
            // children are automatically appended
        }
    }
}

// Usage:
rsx! {
    Card { title: "My Card",
        Text { "This is a child" }
        Button { "So is this" }
    }
}
```

## Option 2: Manual Component Trait

For full control, implement `Component` directly:

```rust
use rinch::prelude::*;

#[derive(Debug, Default)]
pub struct Countdown {
    pub from: i32,
}

impl Component for Countdown {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let remaining = Signal::new(self.from);

        // rsx! works inside Component::render too — __scope is just `scope` here
        let __scope = scope;
        rsx! {
            div {
                Text { size: "xl", {|| remaining.get().to_string()} }
                Button {
                    disabled: {|| remaining.get() <= 0},
                    onclick: move || remaining.update(|n| *n -= 1),
                    "Count down"
                }
            }
        }
    }
}
```

You probably don't need this. The PascalCase function covers 95% of use cases. But it's there for when you need custom `Debug`, or want to store state on the struct itself, or are doing something the macro can't express.

## Lowercase Component Functions

Not everything needs to be a reusable component with props. For private helper functions, use a lowercase name:

```rust
#[component]
fn sidebar_link(label: &str, active: bool) -> NodeHandle {
    rsx! {
        div { class: {|| if active { "nav-link active" } else { "nav-link" }},
            {label}
        }
    }
}
```

Lowercase functions get `__scope` injected but don't generate a struct. Call them directly:

```rust
rsx! {
    div {
        {sidebar_link(__scope, "Home", true)}
        {sidebar_link(__scope, "Settings", false)}
    }
}
```

## Making Props Reactive

Any component prop accepts a closure `{|| expr}` for reactive updates:

```rust
let active = Signal::new(false);
rsx! {
    Button {
        variant: {|| if active.get() { "filled" } else { "outline" }},
        onclick: move || active.update(|v| *v = !*v),
        "Toggle"
    }
}
```

When the signal changes, the component re-renders with the new prop value. For the built-in components, `style:` and `class:` props get **surgical** DOM updates without re-rendering the component.

### `_fn` Props for Surgical Updates

Some components offer `_fn` suffix props that bypass full re-render:

```rust
let text = Signal::new(String::new());
rsx! {
    TextInput {
        value_fn: move || text.get(),           // Updates the DOM input value directly
        oninput: move |v: String| text.set(v),  // Feeds changes back to the signal
    }
}
```

`value_fn` updates the input's value attribute without re-rendering the entire TextInput component. Use this for controlled inputs — without it, programmatic `signal.set("")` won't clear the input visually.

## Style and Class on Components

All components support `style:` and `class:` props:

```rust
Button {
    variant: "filled",
    style: "margin-top: 8px",
    class: "my-custom-button",
    onclick: || do_something(),
    "Click me"
}
```

`class:` is additive — it merges with the component's own CSS classes, it doesn't replace them. Both accept reactive closures:

```rust
Text {
    style: {|| if highlighted.get() { "background: yellow" } else { "" }},
    "Dynamic styling"
}
```

## CSS Shorthand Props

All HTML elements and components support CSS shorthand props. These expand to `set_style()` calls:

```rust
rsx! {
    div { p: "md", m: "sm", maw: "600px",    // padding, margin, max-width
        Stack { w: "100%", h: "auto",          // width, height
            Text { px: "lg", my: "xs", "Spaced text" }  // padding-x, margin-y
        }
    }
}
```

Spacing scale values (`xs`, `sm`, `md`, `lg`, `xl`) auto-resolve to `var(--rinch-spacing-{value})`.
