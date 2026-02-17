# Rinch Known Layout Issues

Discovered while building a real application (rorumall - federated chat client).

---

## 1. ~~`display: contents` doesn't participate in layout~~ FIXED

**Status:** Fixed. `sync_display_contents()` now rebuilds taffy children from DOM
structure each time (idempotent), and recursively flattens nested `display:contents`
nodes. Block parents, incremental relayout, adjacent siblings, and nested wrappers
all work correctly.

---

## 2. ~~`if/else if/else` chains create nested broken wrappers~~ FIXED

**Status:** Fixed as a consequence of #1. The nested `display:contents` wrappers
from `else if` chains are now recursively flattened by the layout engine, so they
layout correctly. A future optimization could flatten the chain at the macro level
(`show_dom_multi`), but it's no longer necessary for correctness.

---

## 3. ~~Bare `{ expr }` inside `if` blocks attaches to wrong parent~~ FIXED

**Status:** Fixed. The macro codegen now wraps bare `{expr}` children inside
`if`/`for`/`match` branches in a `display:contents` div, so `show_dom` and
`for_each_dom` can reliably track and remove them. Both of these now work:

```rust
if condition {
    {some_component(__scope)}
}

for item in items {
    {component_fn(__scope, item)}
}
```

---

## 4. `min-height: 0` requires `overflow: hidden` on intermediate containers

**Status:** Expected CSS behavior (not a bug).

In standard CSS Flexbox (section 4.5), `min-height: 0` on a flex item correctly
bypasses the automatic minimum size. However, for a scrollable area to actually
constrain its size, **all intermediate flex containers in the chain** must also
have `overflow: hidden` (or `overflow: auto`). This is because each flex container
independently computes its minimum size from its content unless `overflow` creates
a new block formatting context.

**Pattern for scrollable flex layouts:**

```rust
// All intermediate containers need overflow: hidden
div {
    style: "display: flex; flex-direction: column; height: 100vh; overflow: hidden;",

    // Fixed header
    div { style: "height: 50px;", ... }

    // Scrollable content area
    div {
        style: "flex: 1; overflow-y: auto; min-height: 0;",
        // content scrolls properly
    }
}
```

**Key rule:** `overflow: hidden` must be on ALL intermediate flex containers between
the root (with a fixed height) and the scrollable child. Missing it on any
intermediate container will cause the content to expand instead of scrolling.
