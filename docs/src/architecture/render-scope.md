# RenderScope and NodeHandle API

This document specifies the DOM abstraction layer that enables fine-grained reactive rendering.

## Overview

The DOM abstraction consists of three key types:

| Type | Purpose |
|------|---------|
| `RenderScope` | Context for building DOM trees with effect tracking |
| `NodeHandle` | Stable reference to a DOM node for surgical updates |
| `DomDocument` | Trait abstracting DOM mutation operations |

## RenderScope

`RenderScope` is the context passed to component functions. It provides methods for creating DOM nodes and Effects.

### Creating Nodes

The recommended approach uses the `#[component]` macro, which injects `__scope` automatically:

```rust
#[component]
fn my_component() -> NodeHandle {
    rsx! {
        div {
            "Hello, world!"
        }
    }
}
```

For manual DOM construction, use `__scope` directly:

```rust
fn my_component(__scope: &mut RenderScope) -> NodeHandle {
    // Create an element node
    let div = __scope.create_element("div");

    // Create a text node
    let text = __scope.create_text("Hello, world!");

    // Create a comment node (useful for anchors)
    let comment = __scope.create_comment("placeholder");

    // Build the tree
    div.append_child(&text);

    div
}
```

### Creating Effects

```rust
fn counter(__scope: &mut RenderScope) -> NodeHandle {
    let count = Signal::new(0);
    let span = __scope.create_element("span");

    // Create an Effect that updates the span when count changes
    // Signal and NodeHandle are Copy — no .clone() needed
    __scope.create_effect(move || {
        span.set_text(&count.get().to_string());
    });

    span
}
```

### Child Scopes

Child scopes inherit the document but have their own Effect tracking. The `child_scope` method takes a parent `NodeHandle` reference and returns a mutable borrow:

```rust
fn parent(__scope: &mut RenderScope) -> NodeHandle {
    let container = __scope.create_element("div");

    // Create a child scope for a nested component
    let child_scope = __scope.child_scope(&container);
    let child_content = child_component(child_scope);
    container.append_child(&child_content);

    container
}
```

When a child scope is dropped, all its Effects are cleaned up.

### Event Handling

Rinch uses a `register_handler` + `data-rid` pattern for event dispatch. There is no `add_event_listener` on NodeHandle. Instead, handlers are registered on the RenderScope and linked to elements via a `data-rid` attribute:

```rust
fn my_button(__scope: &mut RenderScope) -> NodeHandle {
    let button = __scope.create_element("button");
    button.set_text("Click me");

    // Register a handler and link it to the element
    let handler_id = __scope.register_handler(move || {
        println!("Button clicked!");
    });
    button.set_attribute("data-rid", &handler_id.to_string());

    button
}
```

The `rsx!` macro handles this automatically with `onclick:`:

```rust
#[component]
fn my_button() -> NodeHandle {
    rsx! {
        button { onclick: move || println!("Clicked!"),
            "Click me"
        }
    }
}
```

The runtime uses event delegation (a single document-level listener) that dispatches to the correct handler by looking up the `data-rid` attribute on the clicked element.

### RenderScope API Reference

| Method | Description |
|--------|-------------|
| `create_element(tag: &str) -> NodeHandle` | Create an element node (div, span, etc.) |
| `create_text(content: &str) -> NodeHandle` | Create a text node |
| `create_comment(content: &str) -> NodeHandle` | Create a comment node |
| `create_effect(f: impl FnMut() + 'static)` | Create a reactive Effect |
| `child_scope(&mut self, parent: &NodeHandle) -> &mut RenderScope` | Create a child scope rooted at a parent node |
| `register_handler(callback: impl Fn() + 'static) -> EventHandlerId` | Register an event handler, returns ID for `data-rid` |
| `register_input_handler(callback: impl Fn(String) + 'static) -> EventHandlerId` | Register an input handler for text input events |
| `parent() -> NodeHandle` | Get the parent node for this scope |
| `doc_weak() -> Weak<RefCell<dyn DomDocument>>` | Get a weak reference to the underlying document |
| `body_handle() -> NodeHandle` | Get the body element as a NodeHandle |
| `dispose(self)` | Dispose this scope, cleaning up all Effects |

## NodeHandle

`NodeHandle` is a stable reference to a DOM node. It remains valid even as the document changes around it. Internally it holds a `NodeId` and a `Weak<RefCell<dyn DomDocument>>`.

### Text Content

```rust
let text_node = __scope.create_text("initial");

// Update text content
text_node.set_text("updated");
```

### Attributes

```rust
let button = __scope.create_element("button");

// Set attribute
button.set_attribute("disabled", "true");
button.set_attribute("aria-label", "Submit form");

// Get attribute
let label = button.get_attribute("aria-label"); // Some("Submit form")

// Remove attribute
button.remove_attribute("disabled");
```

**Attribute names are ASCII case-insensitive in HTML content and case-sensitive
in SVG content** (issue #688), as in a browser. `set_attribute("ID", …)` on a
`<div>` stores `id`, so `#up`, `[id]`, `getAttribute("id")` and an uppercase
`STYLE=` (which has to reach the inline-style cache) all agree; `get_attribute`
and `remove_attribute` fold their own name the same way, so either spelling
finds what either spelling wrote. On an element whose tag is an SVG one the
author's spelling is kept, because `viewBox`, `preserveAspectRatio`,
`gradientUnits` and their kind are distinct names in SVG.

The decision is made from the element's **tag**, not from an `<svg>` ancestor,
because `rsx!` and the `Element::Html` parser both write an element's attributes
before appending it to its parent — at the write there is no ancestor to walk.
`crates/rinch-dom/src/attr_name.rs` holds the tag list. The attribute **value**
is never folded: an `id` still matches `#CamelId` case-sensitively.

### Styles

```rust
let div = __scope.create_element("div");

// Set individual styles
div.set_style("color", "blue");
div.set_style("font-size", "16px");
div.set_style("display", "flex");

// Remove a style
div.set_style("color", "");  // Empty string removes
```

### Classes

```rust
let element = __scope.create_element("div");

// Set the class attribute directly
element.set_class("active highlighted");

// Add/remove individual classes
element.add_class("active");
element.add_class("highlighted");
element.remove_class("active");

// Toggle based on condition
element.toggle_class("selected");

// Or conditionally:
if is_selected {
    element.add_class("selected");
} else {
    element.remove_class("selected");
}
```

### Tree Manipulation

```rust
let parent = __scope.create_element("ul");
let item1 = __scope.create_element("li");
let item2 = __scope.create_element("li");
let item3 = __scope.create_element("li");

// Append children
parent.append_child(&item1);
parent.append_child(&item3);

// Insert before a reference node
parent.insert_before(&item2, &item3);  // item1, item2, item3

// Remove a node
item2.remove();  // item1, item3

// Replace a node
let new_item = __scope.create_element("li");
item1.replace_with(&new_item);
```

> **Removal is a detach; `discard()` is the end of a node's life.** `remove()`
> and `replace_with()` take a node out of the tree and leave it **re-insertable**
> on every backend: append the handle again and the whole subtree comes back,
> and you may read, style or restructure it while it is out. That is what a
> reactive branch re-showing a *captured* handle rests on (issue #719).
>
> Because a removed node is still the backend's to keep, tell it when you are
> finished with the subtree for good:
>
> ```rust
> panel.remove();    // hidden for now — I may show it again
> row.discard();     // gone for good — let go of it
> ```
>
> Treat a discarded handle as **dead**: build a fresh node rather than reviving
> one. A backend that retires makes every operation on it a silent no-op, and
> `rinch-web` and the test `MockDomDocument` both do, so the mistake fails
> `cargo test` as well as a browser. A **discarded** id is never handed to a
> different node on either backend, so a stale discard handle can only name
> nothing. (That is a claim about `discard` alone: `rinch-dom` frees slab keys
> through `set_inner_html` and pseudo-element pruning, and those *are* recycled —
> issue #304, live today and independent of this API.)
>
> **Which to use.** If the same handle can be inserted again, `remove()`. If it
> cannot — a pool shrunk, a glyph replaced, a panel rebuilt — `discard()`.
> Reaching for `remove()` where you meant `discard()` costs memory; reaching for
> `discard()` where you meant `remove()` costs the subtree. Neither is reported,
> so choose deliberately.
>
> **The reactive helpers do not choose by hand.** `show_dom`, `match_dom`,
> `reactive_component_dom` and `for`/`virtual_list` rows each run their user
> closure inside a `RenderScope` of their own, and go by **ownership**: a node
> that scope minted is the helper's to discard, a node the closure was handed is
> the caller's and is only detached. `RenderScope::created` is the whole rule,
> and it is the rule #141 gave signals and effects, applied to nodes. It is why
> `if open { p { "hi" } }` reclaims its markup on every hide while
> `if open { {panel} }` keeps yours.
>
> Two edges of that rule. The verb is chosen **before** the scope is disposed, so
> an `on_cleanup` that re-parents a scope-built node cannot rescue it — build it
> outside the closure and hand it in instead. And ownership answers for nodes in
> a subtree: a node attached to nothing is reached by no walk, which is what
> `rinch_core::dom::release_scratch_container` exists for.
>
> **`remove()`'s post-condition is the same on both backends; `discard()`'s is
> not.** `rinch-web` holds a strong `web_sys::Node` in two page-global maps, so a
> `discard()` is what releases the browser node against GC (issue #184).
> `rinch-dom` reclaims nothing: a discarded node there still re-inserts, still
> keeps its subtree and still takes writes, because its slab is per-document and
> freeing the slot would recycle the id (issue #304) — so the desktop slab only
> grows, which is issue #723. The contract is therefore one-sided: a `discard()`
> is *at least* a `remove()` and may be much more, and you may not rely on it
> being less.

### Focus

```rust
let input = __scope.create_element("input");
input.focus();  // Give focus to this element
```

### NodeHandle API Reference

| Method | Description |
|--------|-------------|
| `set_text(content: &str)` | Set text content (for text nodes) |
| `set_attribute(name: &str, value: &str)` | Set an attribute — the name folds to lowercase in HTML content, verbatim in SVG (#688) |
| `get_attribute(name: &str) -> Option<String>` | Get an attribute value (same fold) |
| `remove_attribute(name: &str)` | Remove an attribute (same fold) |
| `set_style(property: &str, value: &str)` | Set a CSS style property |
| `set_class(class: &str)` | Set the class attribute |
| `add_class(name: &str)` | Add a CSS class |
| `remove_class(name: &str)` | Remove a CSS class |
| `toggle_class(name: &str)` | Toggle a CSS class |
| `append_child(child: &NodeHandle)` | Append a child node |
| `insert_before(node: &NodeHandle, reference: &NodeHandle)` | Insert before reference |
| `remove()` | Remove this node from its parent — a **detach**; the handle and its subtree stay re-insertable |
| `replace_with(new_node: &NodeHandle)` | Replace this node with another — also a detach; the displaced handle stays re-insertable |
| `discard()` | Remove this node and release the backend's bookkeeping for its whole subtree — **retires** the ids |
| `focus()` | Give focus to this element |
| `children() -> Vec<NodeHandle>` | Get child nodes as NodeHandles |
| `is_valid() -> bool` | Check if this handle still points to a valid node |
| `node_id() -> NodeId` | Get the internal node ID |
| `clone() -> NodeHandle` | Clone the handle (same underlying node) |

## DomDocument Trait

`DomDocument` is the trait that abstracts DOM operations. The primary desktop implementation is `RinchDocument` which uses Taffy + Parley + Vello. The web implementation is `WebDocument` which uses browser-native DOM via `web_sys`.

```rust
pub trait DomDocument {
    /// Create an element node
    fn create_element(&mut self, tag: &str) -> NodeId;

    /// Create a text node
    fn create_text(&mut self, content: &str) -> NodeId;

    /// Create a comment node
    fn create_comment(&mut self, content: &str) -> NodeId;

    /// Set text content of a node
    fn set_text_content(&mut self, node: NodeId, content: &str);

    /// Set an attribute
    fn set_attribute(&mut self, node: NodeId, name: &str, value: &str);

    /// Remove an attribute
    fn remove_attribute(&mut self, node: NodeId, name: &str);

    /// Append a child to a parent
    fn append_child(&mut self, parent: NodeId, child: NodeId);

    /// Insert a node before a reference node
    fn insert_before(&mut self, parent: NodeId, node: NodeId, reference: NodeId);

    /// Remove a node from its parent
    fn remove_child(&mut self, parent: NodeId, child: NodeId);

    /// Get children of a node
    fn get_children(&self, node: NodeId) -> Vec<NodeId>;

    /// Get the body element
    fn body(&self) -> NodeId;

    /// Mark a node as needing re-layout
    fn mark_dirty(&mut self, node: NodeId);
}
```

## RinchDocument

`RinchDocument` is the desktop implementation of `DomDocument` that uses Taffy for layout, Parley for text, and Vello for rendering.

### Key Features

- **Direct DOM manipulation** - Efficient node creation and mutation
- **Automatic dirty marking** - Calls `mark_ancestors_dirty()` after mutations
- **Event handler storage** - Stores handlers as `data-rid` attributes for dispatch

### Usage

```rust
use rinch_dom::RinchDocument;

// Create document
let doc = Rc::new(RefCell::new(RinchDocument::new()));

// Create RenderScope from shared document
let mut scope = RenderScope::new(Rc::downgrade(&doc) as _, parent_id);

// Build DOM
let root = my_app(&mut scope);
```

### Thread Safety

`RinchDocument` is wrapped in `Rc<RefCell<>>` for interior mutability.

Effects capture clones of the shared document and can mutate the DOM when they run.

RenderScope itself holds a `Weak<RefCell<dyn DomDocument>>` to avoid preventing cleanup.

## Integration with Reactive System

The RenderScope and NodeHandle APIs integrate with the reactive system:

1. **Initial render** - Component function receives `RenderScope` (via `__scope` or `#[component]` macro), builds DOM tree
2. **Effect creation** - `__scope.create_effect()` registers reactive computations
3. **NodeHandle capture** - Effects capture `NodeHandle` clones for later updates
4. **Signal changes** - Effects re-run and use `NodeHandle` methods to update DOM
5. **Cleanup** - When scope is dropped, Effects are disposed

This architecture ensures that:
- Components run once (no re-render overhead)
- Updates are surgical (only affected nodes change)
- Cleanup is automatic (scope disposal cleans up Effects)
- Memory is efficient (NodeHandle is a lightweight ID + weak reference wrapper)
