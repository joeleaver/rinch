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

## Event Handlers

Event handlers are `on*` attributes on HTML elements. Pointer/click handlers are
`Fn()` closures; read the cursor position, button, and modifier keys from
`get_click_context()`:

```rust
use rinch::prelude::*;

rsx! {
    div {
        onclick: || println!("clicked"),
        onmousedown: || {
            let ctx = get_click_context();
            match ctx.button {
                MouseButton::Right => println!("right press"),
                _ if ctx.modifiers.shift => println!("shift+press"),
                _ => println!("press at {}, {}", ctx.mouse_x, ctx.mouse_y),
            }
        },
        onmouseup: || println!("released"),
        onmousemove: || { /* fires on every move over this element */ },
        onmouseenter: || println!("hover in"),
        onmouseleave: || println!("hover out"),
        oncontextmenu: || println!("right-click menu"),
        "Interactive"
    }
}
```

Supported HTML-element event attributes:

| Attribute | Fires | Closure |
|---|---|---|
| `onclick` | Primary press (dispatched on pointerdown), or Enter/Space on the focused element — see [Keyboard activation](#keyboard-activation) | `Fn()` |
| `onmousedown` / `onmouseup` | Pointer press / release (any button) | `Fn()` |
| `onmousemove` | Pointer moves over the element | `Fn()` |
| `onmouseenter` / `onmouseleave` | Pointer enters / leaves the element | `Fn()` |
| `oncontextmenu` | Right-click, or a 500ms long press on Android (suppresses the native menu when handled) | `Fn()` |
| `oninput` | `<input>`/`<textarea>` value change, per keystroke | `Fn(String)` |
| `onchange` | Commit boundary: the gesture ends (blur after a modification, Enter, a `<select>` pick) — fires with the final value | `Fn(String)` |
| `onscroll` | Scroll container scrolls, on either axis | `Fn(ScrollEvent)` — `ev.scroll_top` and `ev.scroll_left` |
| `ondragstart` … `ondrop`, `ondragend` | Element drag-and-drop | `Fn()` |
| `onfiledrop`, `onfiledragenter`/`onfiledragleave` | OS → app file drop | `Fn(Vec<PathBuf>)` / `Fn()` |

`onscroll` fires once per container that moved, whichever axis moved it, and
its [`ScrollEvent`] payload carries **both** offsets — so a horizontal-only
scroller reports its position rather than an unchanging `scroll_top`
(issue #177). `ScrollEvent` is `#[non_exhaustive]`: read its fields by name,
and construct one (in a test, say) with `ScrollEvent::new(top, left)` rather
than a struct literal.

```rust
div {
    style: "overflow: auto; width: 200px; height: 100px",
    onscroll: move |ev: ScrollEvent| {
        column.set((ev.scroll_left / char_width) as usize);
    },
    // …wide content…
}
```

[`ScrollEvent`]: https://docs.rs/rinch/latest/rinch/prelude/struct.ScrollEvent.html

`oninput` and `onchange` follow HTML semantics and are **not** aliases (they
were before issue #226): `oninput` fires on every keystroke with the live
value, while `onchange` fires once when the typed gesture *ends* — focus
leaves the control after a modification, Enter commits explicitly, or a
`<select>` commits a pick — and only if the value actually changed since the
control was focused. Use `oninput` for live previews and controlled inputs,
`onchange` for validate-on-commit, autosave-on-leave, and undo bracketing.
On Enter, `onchange` fires before `onsubmit`, and the eventual blur does not
re-fire an already-committed value. Enter commits **single-line inputs
only** — in a `<textarea>` (as in HTML) the gesture runs until blur, where
Enter inserts a line break instead (see [Enter in a
`<textarea>`](#enter-in-a-textarea)). Like the
browser's `change`, the event bubbles: `data-onchange` on an ancestor of the
control receives the commit too.

Mouse and click handlers read per-event data from `get_click_context()`:
`mouse_x`/`mouse_y`, element bounds (`relative_x()`, `percent_x()`, …),
`button` (`MouseButton::{Left, Middle, Right}`), and `modifiers`
(`shift`/`ctrl`/`alt`/`meta`). These behave identically on the desktop and web
(WASM) backends.

### Keyboard activation

**Enter** and **Space** on a keyboard-focused element run its `onclick` — the
same handler as a pointer press, on both backends. What Tab can reach differs
by backend: on web the browser makes `<button>`, `<a href>` and the like Tab
stops on its own; on desktop only an element with `tabindex` (or a text input)
is a Tab stop today, so a bare `<button>` needs `tabindex="0"` there (issue
#252). The handler receives a `ClickContext` whose cursor sits at the
element's centre (`mouse_x`/`mouse_y` = the middle of `element_x`/`element_y`
+ size), with `button: Left` and no text hit, so placement logic that reads
`get_click_context()` (a `Select` flipping to fit the viewport, say) keeps
working for a keyboard user. Ctrl/Meta chords are left alone; Shift and Alt
ride along in `modifiers`.

On desktop, activation latches once per physical press (a held key does not
repeat). On web, two routes cover the two kinds of element:

- **Natively activatable elements** — `<button>` and `<summary>` on Enter or
  Space, `<a href>` on Enter, checkbox/radio inputs on Space — use the
  browser's own activation: the `click` the browser synthesises is what
  dispatches the handler. The browser's semantics apply, so `<a href>` also
  **navigates** on Enter (exactly as a mouse click on it already does), a
  checkbox still toggles, and a held Enter repeats at the browser's key-repeat
  rate. Where the browser has no activation for a key — **Space on a link**
  — rinch dispatches it from `keydown` instead (the handler runs, the page
  does not scroll, and nothing navigates), matching desktop.
- **`tabindex` elements** (a `<div tabindex="0">` such as a `Tree` node) get no
  browser click, so rinch dispatches from `keydown`: once per press, and the
  key is consumed for the whole press — including its auto-repeats — so a held
  Space neither re-activates nor scrolls the page. Elements the browser
  focuses on its own without `tabindex` (a keyboard-focusable scroll
  container, `<video controls>`) keep their keys: Space there scrolls or plays,
  it does not activate the clickable around them.

Neither route double-fires with the mouse: a pointer press dispatches from
`pointerdown`, and the trailing `click` of that same gesture — including the
extra click a `<label>` fires at its control — is suppressed, even if a key
was pressed while the button was held. Clicks with no pointer gesture behind
them — assistive technology (whose clicks are *trusted*, with no pointer or
key event of their own on Firefox/WebKit), `element.click()` — are honoured
once, and a click a handler raises itself (`hidden_input.click()`) does not
re-enter that handler. Enter inside an `<input>`/`<textarea>`, an editable
region or the rich-text editor is never an activation of a surrounding
clickable; it is the `onsubmit` gesture described above (or, in a
`<textarea>` with no `onsubmit`, a line break). An element with no
live handler in its ancestry is a quiet no-op — the key falls through to the
browser, so Tab and scrolling keep their usual meaning.

A **mouse press focuses the nearest focusable ancestor** of what it hits, like a
browser — so a clicked `tabindex` element owns Enter/Space straight away,
without having to be reached by Tab first. A component that wants the keys for
itself (a code editor, a keyboard-driven grid) registers for them; see
[Keyboard Focus](./focus.md).

### Enter in a `<textarea>`

Enter has two meanings in a multi-line field, and which one it takes is the
author's declaration:

| Field | Key | What happens |
|---|---|---|
| `<textarea>` | Enter, no `onsubmit` | inserts a line break |
| `<textarea>` | Enter, with `onsubmit` | runs `onsubmit` — nothing is inserted |
| `<textarea>` | **Shift+Enter** | always inserts a line break |
| `<input>` | Enter / Shift+Enter | commits (`onchange`, then `onsubmit`); never inserts |

So a notes field takes line breaks with no ceremony, a chat composer sends on
Enter and takes a break on Shift+Enter, and a single-line `<input>` is
unchanged — a line break is not representable in its value.

`onsubmit` is read the way `onchange` is: from the control, then up its
ancestors, so an `onsubmit` on the wrapper around a field submits that field.
One backend difference to know about: on web, **Shift+Enter in an `<input>`
does nothing** (the browser's own Enter path is what desktop's commit stands
in for), while on desktop it commits like a plain Enter.

The break is an ordinary edit: it moves the caret, fires `oninput` with the new
value (`\n` included) and is one Backspace away from gone. On Android the soft
keyboard is told per focused field which kind it is serving, so a `<textarea>`
gets a keyboard whose Enter types a newline instead of an action key that ends
the input session.

### Sizing a `<textarea>`

A `<textarea>` holds its value in an attribute rather than as child text, so it
has no content to size against. Its height comes from `rows`, which reserves
that many lines plus padding and border — 2 rows when unset, matching HTML:

```rust
rsx! {
    textarea { rows: "6" }                        // six lines tall
    textarea { style: "min-height: 200px;" }      // or size it with CSS
}
```

`min-height` and an explicit `height` both work as usual: `height` overrides the
`rows` height outright, and `min-height` acts as a floor, so the taller of the
two wins. The `Textarea` component exposes the same thing as `min_rows`.

## Components

Components are custom UI elements written in PascalCase. They implement the `Component` trait and render directly to DOM nodes:

```rust
rsx! {
    Button { variant: "filled", color: "blue", onclick: || println!("clicked"),
        "Click me"
    }

    Alert { icon: TablerIcon::InfoCircle, color: "blue", title: "Info",
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
let count = Signal::new(0);

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
let name = Signal::new("World".to_string());
let count = Signal::new(0);

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
let progress = Signal::new(50);
let is_active = Signal::new(false);

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
let is_selected = Signal::new(false);
let variant = Signal::new("primary");

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
let count = Signal::new(0);

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
let visible = Signal::new(true);
let count = Signal::new(5);

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
let current_user = Signal::new(Some("Alice".to_string()));

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

#### Braces around control flow

A brace in child position is transparent to control flow: `{ match x { … } }`
renders the same reactive `match_dom` as `match x { … }`, and the same goes for
`if` and `for`, at any depth — including a braced `match` arm body,
`_ => { if … }`.

That is worth stating because the two used to differ, silently. A braced
construct was parsed as a plain Rust expression and evaluated **once**, so it
kept whichever branch was true when it mounted and never changed again — one
brace apart from a fully reactive twin, with no diagnostic (issue #221).

Braced control flow whose bodies are *not* rsx cannot be made reactive — there is
no markup for the runtime to swap — so it is a compile error now instead of a
silent freeze:

```rust
// error: `match` wrapped in braces renders once and never updates
{ match selected.get() {
    Some(p) => rsx! { div { {p.name.clone()} } },
    None => rsx! { div { "—" } },
} }
```

The message names both ways out, because which one is right depends on what you
meant. For reactive **markup**, drop the braces and write each arm as rsx rather
than as a nested `rsx!` invocation:

```rust
match selected.get() {
    Some(p) => div { {p.name.clone()} },
    None => div { "—" },
}
```

For a reactive **value**, wrap the whole thing in a closure — the reactive
expression form from earlier in this guide:

```rust
{|| match selected.get() {
    Some(p) => p.name.clone(),
    None => "—".to_string(),
}}
```

Everything else in braces is untouched: an expression (`{ count.to_string() }`),
a reactive closure (`{|| count.get()}`) and a call (`{ section(__scope) }`) never
begin with a control-flow keyword, so they are the expressions they always were.

#### Capturing the same value in more than one branch

Every closure RSX builds is a `move` closure — a branch, a `match` arm, a
per-item view, a reactive binding's effect, an event handler — and `show_dom`
takes the then branch **and** the else branch, so both are *constructed*
whichever way the condition comes out. A non-`Copy` value named by two of them
would be moved twice.

The macro handles that for you. At each construction site it emits a shadow
clone of what that site shares with a sibling site, or reaches for from outside
a body that re-runs. So this compiles, and `label` is still yours afterwards:

```rust
let label = String::from("hello");

rsx! {
    div {
        if label.is_empty() {
            p { "empty" }
        } else {
            p { {label.clone()} }
        }
        Text { {label.clone()} }
    }
}
```

**What is not cloned.** A value only one site names is moved, exactly as before,
so a type that isn't `Clone` keeps working where it always did. A value bound
*inside* a repeating body — a `let` at the top of a branch, the `for` item
itself — is a fresh local on every run and is never cloned. `Signal` and `Memo`
are `Copy`, so where a shadow does fall on one it is a copy, not an allocation.

**What still needs a manual `.clone()`.** The analysis is lexical: it cannot see
a value hidden inside a macro (`format!("{}", label)` hides `label` from it), and
it compares sites within one construct — two separate `if` blocks that both name
one `String` still need one of them to clone. When rustc does report an E0382 or
E0507 from inside `rsx!`, the fix is the ordinary one: give the site that
complains its own clone.

### `for` loops

Write standard `for..in` loops directly in RSX:

```rust
let todos = Signal::new(vec![
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

**Keys must be unique within one list.** The whole reconcile rests on one key
naming one item and one DOM node, so a repeat used to leave a row that rendered,
swallowed clicks and never updated again (issue #185). What a repeat *means*
depends on who chose the key:

| | Rule |
|---|---|
| **You wrote `key:`** | A repeat is a mistake in your key. The repeat is **not rendered** and a warning is logged — the first occurrence wins, the same rule React applies to duplicate `key` props. |
| **No `key:`** | The framework fabricated the key from `format!("{:?}", item)`, so a repeated *value* is not your mistake. Rinch makes the fabricated key unique by its occurrence ordinal instead: **every row renders**. |

So `for tag in ["rust", "rust", "gui"]` renders three rows, and
`for n in vec![1, 1, 2]` renders three rows. Reordering such a list still moves
rows rather than rebuilding them, because the ordinal follows the *value*, not
the position.

The one thing the fallback cannot give you is stable identity across an *edit*:
if the first `"rust"` is deleted, the second one inherits its key and its DOM
node. Where rows carry per-row state, give them a real `key:`:

```rust
for (i, n) in numbers.get().into_iter().enumerate() {
    div { key: i, {n.to_string()} }
}
```

Prefer a stable per-row id where the data has one. An index key makes identity
follow *position*, so inserting or removing anywhere but the end re-renders every
row after the change — correct, but it gives up the reconciliation the keys are
there for. Reach for the index only when the rows genuinely have nothing else to
distinguish them.

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

#### Reactivity in for Loops

The `for` loop expression itself is reactive — it reads a Signal, which creates an Effect. When that Signal changes, the loop re-evaluates and reconciles the list. Items whose data changed (per `PartialEq`) get their component **re-created** with fresh props. This means the component function runs again with the new values.

Because of this, **closures on plain props inside for-loop components are unnecessary** — the whole function runs again with new values when the item changes. Closures *are* needed for per-item Signals created with `Signal::new()` inside the loop body, since those change independently of the list data.

```rust
// Props are plain values from the for loop — no closure needed
#[component]
pub fn TodoItem(label: String, completed: bool) -> NodeHandle {
    // ❌ Unnecessary: `completed` is a plain bool, not a Signal.
    // The closure captures it once, but the component is re-created
    // with a fresh `completed` value when the item data changes anyway.
    rsx! { Text { style: {|| if completed { "text-decoration: line-through" } else { "" }} } }

    // ✅ Simpler: just use the value directly
    rsx! { Text { style: if completed { "text-decoration: line-through" } else { "" } } }
}

// Per-item Signal — closure IS needed
for todo in todos.get() {
    let editing = Signal::new(false);  // Per-item state
    div { key: todo.id,
        // ✅ Closure needed: `editing` is a Signal that changes independently
        span { style: {|| if editing.get() { "outline: 1px solid blue" } else { "" }} }
    }
}
```

**Rule of thumb:** If the value comes from a Signal (`.get()`), use a closure. If it comes from the `for` loop variable (a plain value), just use it directly.

#### Filtering and Transforming Collections

You can filter, map, and transform the collection inline in the `for` expression. The entire expression is wrapped in a reactive closure by the macro, so all Signals read inside it are tracked:

```rust
let todos = Signal::new(vec![/* ... */]);
let filter = Signal::new(Filter::All);

rsx! {
    div {
        // Filter the collection inline — .filter(), .map(), .collect() all work
        for todo in todos.get().into_iter().filter(|t| {
            match filter.get() {
                Filter::All => true,
                Filter::Active => !t.completed,
                Filter::Completed => t.completed,
            }
        }).collect::<Vec<_>>() {
            TodoItem { key: todo.id, label: todo.text.clone() }
        }
    }
}
```

Both `todos` and `filter` are tracked — the loop re-evaluates when either Signal changes. Any iterator chain that produces a `Vec<T>` (where `T: Clone + PartialEq + 'static`) works.

### `match`

Write standard Rust `match` directly in RSX:

```rust
let tab = Signal::new(0);

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
let result = Signal::new(Ok::<String, String>("Hello".into()));

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
let count = Signal::new(0);

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
let input_text = Signal::new(String::new());
let todos = Signal::new(Vec::<String>::new());

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
On a positioned element the `top`/`right`/`bottom`/`left` shorthands accept any CSS length — `px`, `%`, `em`, `var()`, `calc()` — not just the spacing scale. (A `calc()` that mixes `%` with a length is not resolved yet on desktop; keep a `calc()` to one unit.)

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
let big = Signal::new(false);
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
