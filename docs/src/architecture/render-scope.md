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

> **Removal retires a handle.** `remove()` and `replace_with()` end the node's
> life, along with its whole subtree: the backend is free to drop its bookkeeping
> for those ids, so re-attaching or writing to the handle afterwards is not
> guaranteed to do anything. On the browser backend it silently no-ops — that is
> what lets the backend let go of the DOM node it was pinning (issue #184). Build
> a fresh node instead of reviving a removed one.

### Focus

```rust
let input = __scope.create_element("input");
input.focus();  // Give focus to this element
```

### NodeHandle API Reference

| Method | Description |
|--------|-------------|
| `set_text(content: &str)` | Set text content (for text nodes) |
| `set_attribute(name: &str, value: &str)` | Set an attribute |
| `get_attribute(name: &str) -> Option<String>` | Get an attribute value |
| `remove_attribute(name: &str)` | Remove an attribute |
| `set_style(property: &str, value: &str)` | Set a CSS style property |
| `set_class(class: &str)` | Set the class attribute |
| `add_class(name: &str)` | Add a CSS class |
| `remove_class(name: &str)` | Remove a CSS class |
| `toggle_class(name: &str)` | Toggle a CSS class |
| `append_child(child: &NodeHandle)` | Append a child node |
| `insert_before(node: &NodeHandle, reference: &NodeHandle)` | Insert before reference |
| `remove()` | Remove this node from its parent — **retires** the handle and its subtree |
| `replace_with(new_node: &NodeHandle)` | Replace this node with another — **retires** this handle and its subtree |
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
