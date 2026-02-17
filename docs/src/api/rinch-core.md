# rinch-core

Core types and traits for rinch, including elements, reactive primitives, the DOM abstraction layer, hooks, and the Component trait.

## Element Types

### `Element`

The fundamental building block enum. Note that it is minimal - shell-level constructs (windows, menus, themes) are handled at the runtime level, not as Element variants.

```rust
pub enum Element {
    /// Raw HTML content rendered by the DOM backend.
    Html(String),
    /// A fragment containing multiple children.
    Fragment(Children),
    /// A custom component implementing the Component trait.
    Component(Rc<dyn Component>, Children),
}
```

### `WindowProps`

Configuration for a window (used at the runtime level via `run_with_window_props`):

```rust
pub struct WindowProps {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub borderless: bool,
    pub resizable: bool,
    pub transparent: bool,
    pub always_on_top: bool,
    pub visible: bool,
    pub resize_inset: Option<f32>,
}
```

### `MenuProps`

Configuration for a menu:

```rust
pub struct MenuProps {
    pub label: String,
}
```

### `MenuItemProps`

Configuration for a menu item:

```rust
pub struct MenuItemProps {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub onclick: Option<MenuItemCallback>,
}
```

## DOM Abstraction Layer

The fine-grained reactive rendering system is built on three core types that abstract DOM operations away from any specific backend (desktop via Taffy/Parley/Vello, or browser-native via web_sys).

### `DomDocument` Trait

The backend abstraction. All DOM operations go through this trait:

```rust
pub trait DomDocument {
    fn create_element(&self, tag: &str) -> NodeId;
    fn create_text(&self, content: &str) -> NodeId;
    fn set_attribute(&self, node: NodeId, name: &str, value: &str);
    fn remove_attribute(&self, node: NodeId, name: &str);
    fn set_text(&self, node: NodeId, content: &str);
    fn append_child(&self, parent: NodeId, child: NodeId);
    fn insert_before(&self, parent: NodeId, child: NodeId, reference: NodeId);
    fn remove_child(&self, parent: NodeId, child: NodeId);
    fn register_handler(&self, handler: Box<dyn Fn()>) -> HandlerId;
    // ... and more
}
```

### `RenderScope`

Context for building DOM trees. Wraps a `DomDocument` and provides the API that the `rsx!` macro calls:

```rust
impl RenderScope {
    pub fn create_element(&mut self, tag: &str) -> NodeHandle;
    pub fn create_text(&mut self, content: &str) -> NodeHandle;
    pub fn register_handler(&mut self, handler: impl Fn() + 'static) -> HandlerId;
}
```

Component functions receive a `RenderScope` (injected automatically by `#[component]`):

```rust
#[component]
fn my_component() -> NodeHandle {
    rsx! { div { "Hello" } }
}
// Expands to: fn my_component(__scope: &mut RenderScope) -> NodeHandle { ... }
```

### `NodeHandle`

A stable reference to a DOM node. Delegates all operations via `Weak<RefCell<dyn DomDocument>>`:

```rust
impl NodeHandle {
    pub fn set_attribute(&self, name: &str, value: &str);
    pub fn remove_attribute(&self, name: &str);
    pub fn set_text(&self, content: &str);
    pub fn append_child(&self, child: &NodeHandle);
    pub fn insert_before(&self, child: &NodeHandle, reference: &NodeHandle);
    pub fn remove_child(&self, child: &NodeHandle);
}
```

NodeHandles are used by Effects for surgical DOM updates:

```rust
// Signal change -> Effect runs -> NodeHandle.set_text() -> Minimal re-layout
```

## Component Trait

Components implement the `Component` trait to render directly to DOM nodes:

```rust
pub trait Component: std::fmt::Debug + 'static {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle;
}
```

## Reactive Module

### `Signal<T>`

A reactive container for mutable state. In components, prefer `use_signal()` to create signals tied to the component lifecycle.

```rust
impl<T> Signal<T> {
    pub fn new(value: T) -> Self;
    pub fn set(&self, value: T);
    pub fn update(&self, f: impl FnOnce(&mut T));
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R;
}

impl<T: Clone> Signal<T> {
    pub fn get(&self) -> T;
}
```

**Preferred usage via hooks:**

```rust
#[component]
fn counter() -> NodeHandle {
    let count = use_signal(|| 0);
    rsx! {
        p { {|| count.get().to_string()} }
        button { onclick: move || count.update(|n| *n += 1), "+" }
    }
}
```

### `Effect`

A side-effect that tracks signal dependencies and re-runs when they change.

```rust
impl Effect {
    pub fn new<F: FnMut() + 'static>(f: F) -> Self;
    pub fn new_deferred<F: FnMut() + 'static>(f: F) -> Self;
    pub fn run(&self);
    pub fn dispose(&self);
}
```

In practice, Effects are created automatically by the `rsx!` macro for reactive expressions (`{|| expr}`), and by `use_effect()` for explicit side effects.

### `Memo<T>`

A cached computed value that automatically re-computes when its dependencies change.

```rust
impl<T: Clone + 'static> Memo<T> {
    pub fn new<F: Fn() -> T + 'static>(f: F) -> Self;
    pub fn get(&self) -> T;
}
```

### `Scope`

Manages the lifetime of reactive primitives.

```rust
impl Scope {
    pub fn new() -> Self;
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R;
    pub fn add_effect(&self, effect: Effect);
    pub fn dispose(&self);
}
```

## Hooks API

React-style hooks for managing state in components. All hooks must be called at the top level of a component, in the same order every render.

| Hook | Purpose |
|------|---------|
| `use_signal(init)` | Reactive state that triggers re-renders |
| `use_state(init)` | Simple state with `(value, setter)` tuple |
| `use_ref(init)` | Mutable reference (no re-renders) |
| `use_effect(f, deps)` | Side effects when deps change |
| `use_effect_cleanup(f, deps)` | Effects with cleanup functions |
| `use_mount(f)` | One-time effect on first render |
| `use_memo(f, deps)` | Memoized computations |
| `use_callback(f, deps)` | Memoized callbacks |
| `use_derived(f)` | Auto-tracking computed values (uses Memo) |
| `create_context(value)` | Create shared context |
| `use_context::<T>()` | Access shared context |

## Utility Functions

### `batch`

Batch multiple signal updates to defer effect execution:

```rust
pub fn batch<R>(f: impl FnOnce() -> R) -> R;
```

### `derived`

Create a memo (convenience function):

```rust
pub fn derived<T: Clone + 'static>(f: impl Fn() -> T + 'static) -> Memo<T>;
```

### `untracked`

Read signals without tracking dependencies:

```rust
pub fn untracked<R>(f: impl FnOnce() -> R) -> R;
```

## Control Flow

### `show_dom` / `Show`

Reactive conditional rendering. Swaps DOM content when the condition changes:

```rust
#[component]
fn example() -> NodeHandle {
    let visible = use_signal(|| true);
    rsx! {
        Show {
            when: {move || visible.get()},
            div { "Visible!" }
        }
    }
}
```

### `for_each_dom` / `For`

Keyed list rendering with minimal DOM operations via LIS-based reconciliation:

```rust
#[component]
fn example() -> NodeHandle {
    let items = use_signal(|| vec!["a", "b", "c"]);
    rsx! {
        For {
            each: {move || items.get().into_iter().map(|s| ForItem::new(s, s)).collect()},
            |item: &ForItem| rsx! { div { {item.downcast::<&str>().unwrap()} } }
        }
    }
}
```
