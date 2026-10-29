# Writing Your Own Components

You have two options. The first one is almost always what you want.

## Option 1: PascalCase Component Functions

Write a function with a PascalCase name and `#[component]`. The macro generates a struct, a `Default` impl, and a `Component` trait impl. Parameters become props. Done.

> **The name's case is what makes it a component.** `rsx!` reads it to tell
> markup from a component, so `MyThing { … }` calls your function while
> `my_thing { … }` is looked up as an HTML tag. A lowercase `#[component]`
> function is still perfectly good Rust — it just has to be called as one:
> `{ my_thing(__scope) }`. Writing `my_thing { … }` is a compile error that says
> so (issue #528); before that it rendered an empty `<my_thing>` element and
> never called the function.

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
| `icon: TablerIcon::Check` | `Some(TablerIcon::Check)` |
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

## Node Ids (`NodeId`)

Most components never see a node id: they hold `NodeHandle`s. Code that does — a
custom `DomDocument`, an editor integration, a debug client, a hand-written
`Component` that keeps ids in a side table — should know what a `NodeId` is.

A `NodeId` is a `u64` on every target, wasm32 included: a 32-bit **slot index**
in the low half and a 32-bit **generation** in the high half. The generation is
what lets a backend that reuses a freed slot tell an old id from the slot's new
occupant: an operation through an id that names no live node does nothing, and a
read answers `None` or empty. The desktop backend reuses the slots of discarded
nodes this way (a slot is retired after 2^21 − 1 generations, so an id stays
below 2^53 and survives a JSON number exactly); a `NodeHandle` kept past a
`discard()` names nothing, never the node minted into its slot.

```rust
let id = NodeId::from_index(4);      // slot 4, generation 0
assert_eq!(id.index(), 4);           // the slot, as a usize
assert_eq!(id.generation(), 0);
let again = NodeId::new(4, 1);       // the same slot, one generation later
assert_ne!(id, again);
```

### Migrating from `usize` ids

Before issue #723, `NodeId` wrapped a `usize`, and about 57 public signatures
carried a bare `usize` node id. They are `u64` now:

- **`rinch-core`:** `NodeId(pub u64)`; `TextHitInfo::inline_root_node_id`;
  `InputContext::node_id`; the selection callbacks and
  `dispatch_selection` / `get_saved_selection` / `query_selection_ranges`;
  `request_focus`, `post_release_request`, `cancel_pending_text_selection_request`.
- **`rinch-editor-view`:** `container_id`, `editor_for` / `all_editors`,
  `begin_drag` / `drag_anchor`, `collab_receive_for`, `caret_address`,
  `caret_inline_box`, `caret_blink_tick`, and the focus, blur and node-view-host
  handler types.
- **`rinch-debug`:** the protocol's node id fields (the MCP server's too).

What to change:

- `NodeId(i)` with `i: usize` becomes `NodeId::from_index(i)`. A literal
  `NodeId(4)` still compiles and still means slot 4 at generation 0.
- Never use `.0` as an index: it holds the packed bits. Use `id.index()` for the
  slot. Keep a whole `NodeId` (or its `.0` as a `u64`) as a map key, not the
  slot, so an old id and the slot's next occupant stay different keys.
- A `usize` id you stored or passed to one of the APIs above becomes a `u64`
  (`id.0`), or better, the `NodeId` itself.

Every id is still issued at generation 0, so a generation-0 id prints exactly as
before: `Debug` gives `NodeId(4)`, and the debug server, the MCP tools and
DevTools show `4`. A non-zero generation prints as `NodeId(4@g1)`.
