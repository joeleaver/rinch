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

```rust
fn my_component(scope: &mut RenderScope) -> NodeHandle {
    // Create an element node
    let div = scope.create_element("div");

    // Create a text node
    let text = scope.create_text("Hello, world!");

    // Create a comment node (useful for anchors)
    let comment = scope.create_comment("placeholder");

    // Build the tree
    div.append_child(&text);

    div
}
```

### Creating Effects

```rust
fn counter(scope: &mut RenderScope) -> NodeHandle {
    let count = use_signal(|| 0);
    let span = scope.create_element("span");

    // Create an Effect that updates the span when count changes
    scope.create_effect({
        let count = count.clone();
        let span = span.clone();
        move || {
            span.set_text(&count.get().to_string());
        }
    });

    span
}
```

### Child Scopes

Child scopes inherit the document but have their own Effect tracking:

```rust
fn parent(scope: &mut RenderScope) -> NodeHandle {
    let container = scope.create_element("div");

    // Create a child scope for a nested component
    let mut child_scope = scope.child_scope();
    let child_content = child_component(&mut child_scope);
    container.append_child(&child_content);

    container
}
```

When a child scope is dropped, all its Effects are cleaned up.

### RenderScope API Reference

| Method | Description |
|--------|-------------|
| `create_element(tag: &str) -> NodeHandle` | Create an element node (div, span, etc.) |
| `create_text(content: &str) -> NodeHandle` | Create a text node |
| `create_comment(content: &str) -> NodeHandle` | Create a comment node |
| `create_effect(f: impl FnMut() + 'static)` | Create a reactive Effect |
| `child_scope() -> RenderScope` | Create a child scope |
| `document() -> &dyn DomDocument` | Access the underlying document |

## NodeHandle

`NodeHandle` is a stable reference to a DOM node. It remains valid even as the document changes around it.

### Text Content

```rust
let text_node = scope.create_text("initial");

// Update text content
text_node.set_text("updated");
```

### Attributes

```rust
let button = scope.create_element("button");

// Set attribute
button.set_attribute("disabled", "true");
button.set_attribute("aria-label", "Submit form");

// Remove attribute
button.remove_attribute("disabled");
```

### Styles

```rust
let div = scope.create_element("div");

// Set individual styles
div.set_style("color", "blue");
div.set_style("font-size", "16px");
div.set_style("display", "flex");

// Remove a style
div.set_style("color", "");  // Empty string removes
```

### Classes

```rust
let element = scope.create_element("div");

// Add/remove classes
element.add_class("active");
element.add_class("highlighted");
element.remove_class("active");

// Toggle based on condition
if is_selected {
    element.add_class("selected");
} else {
    element.remove_class("selected");
}
```

### Tree Manipulation

```rust
let parent = scope.create_element("ul");
let item1 = scope.create_element("li");
let item2 = scope.create_element("li");
let item3 = scope.create_element("li");

// Append children
parent.append_child(&item1);
parent.append_child(&item3);

// Insert before a reference node
parent.insert_before(&item2, &item3);  // item1, item2, item3

// Remove a node
item2.remove();  // item1, item3

// Replace a node
let new_item = scope.create_element("li");
item1.replace_with(&new_item);
```

### Event Listeners

```rust
let button = scope.create_element("button");

button.add_event_listener("click", |event| {
    println!("Button clicked!");
});

button.add_event_listener("mouseenter", |event| {
    println!("Mouse entered!");
});
```

### NodeHandle API Reference

| Method | Description |
|--------|-------------|
| `set_text(content: &str)` | Set text content (for text nodes) |
| `set_attribute(name: &str, value: &str)` | Set an attribute |
| `remove_attribute(name: &str)` | Remove an attribute |
| `set_style(property: &str, value: &str)` | Set a CSS style property |
| `add_class(name: &str)` | Add a CSS class |
| `remove_class(name: &str)` | Remove a CSS class |
| `append_child(child: &NodeHandle)` | Append a child node |
| `insert_before(node: &NodeHandle, reference: &NodeHandle)` | Insert before reference |
| `remove()` | Remove this node from its parent |
| `replace_with(new_node: &NodeHandle)` | Replace this node with another |
| `add_event_listener(event: &str, handler: impl Fn(Event))` | Add event listener |
| `node_id() -> NodeId` | Get the internal node ID |
| `clone() -> NodeHandle` | Clone the handle (same underlying node) |

## DomDocument Trait

`DomDocument` is the trait that abstracts DOM operations. The primary implementation is `BlitzDomAdapter` which wraps blitz-dom.

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

## BlitzDomAdapter

`BlitzDomAdapter` is the production implementation of `DomDocument` that wraps blitz-dom's `Document`.

### Key Features

- **Direct DOM manipulation** - Uses blitz-dom's `DocumentMutator` for efficient updates
- **Automatic dirty marking** - Calls `mark_ancestors_dirty()` after mutations
- **Event handler storage** - Stores handlers as `data-rid` attributes for dispatch

### Usage

```rust
use rinch::shell::dom_adapter::{BlitzDomAdapter, SharedDomAdapter};

// Create adapter from blitz Document
let adapter = BlitzDomAdapter::new(blitz_document);
let shared = SharedDomAdapter::new(adapter);

// Create RenderScope from adapter
let mut scope = RenderScope::new(shared.clone());

// Build DOM
let root = my_app(&mut scope);
```

### Thread Safety

`SharedDomAdapter` wraps the adapter in `Rc<RefCell<>>` for interior mutability:

```rust
pub type SharedDomAdapter = Rc<RefCell<BlitzDomAdapter>>;
```

Effects capture clones of the shared adapter and can mutate the DOM when they run.

## Integration with Reactive System

The RenderScope and NodeHandle APIs integrate with the reactive system:

1. **Initial render** - Component function receives `RenderScope`, builds DOM tree
2. **Effect creation** - `scope.create_effect()` registers reactive computations
3. **NodeHandle capture** - Effects capture `NodeHandle` clones for later updates
4. **Signal changes** - Effects re-run and use `NodeHandle` methods to update DOM
5. **Cleanup** - When scope is dropped, Effects are disposed

This architecture ensures that:
- Components run once (no re-render overhead)
- Updates are surgical (only affected nodes change)
- Cleanup is automatic (scope disposal cleans up Effects)
- Memory is efficient (NodeHandle is a lightweight ID wrapper)
