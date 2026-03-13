---
paths:
  - "examples/**/*.rs"
  - "crates/rinch-components/**/*.rs"
  - "crates/rinch/src/**/*.rs"
  - "crates/rinch-core/src/**/*.rs"
  - "crates/rinch-dom/src/**/*.rs"
  - "crates/rinch-macros/src/**/*.rs"
  - "crates/rinch-theme/**/*.rs"
  - "crates/rinch-tabler-icons/**/*.rs"
---

# Rinch Best Practices — Automatically Loaded

When writing or editing rinch UI code, follow these rules. Violations compile fine but cause silent runtime bugs.

## The Core Mental Model

**Rinch is NOT React.** The component function runs **once** to build the DOM. There are no re-renders. All subsequent updates are surgical DOM mutations driven by reactive Effects.

## Rules (Ordered by Impact)

### 1. Dynamic values MUST use `{|| expr}` closures

Without the closure, the value is captured once and **silently never updates**.

```rust
// BUG — silently static
p { {count.get().to_string()} }

// CORRECT — reactive
p { {|| count.get().to_string()} }
```

Applies to text, attributes, styles, classes — anything with `.get()` in rsx needs `{|| ...}`.

### 2. Don't manually wrap rsx props

The `rsx!` macro auto-wraps. Manual wrapping double-wraps and causes confusing type errors.

```rust
// WRONG                                    // CORRECT
Button { onclick: Some(Callback::new(..)) } // Button { onclick: move || do_thing() }
Alert { icon: Some(Icon::Check) }           // Alert { icon: Icon::Check }
TextInput { value_fn: Some(Rc::new(..)) }   // TextInput { value_fn: move || text.get() }
Button { variant: Some("filled".into()) }   // Button { variant: "filled" }
```

### 3. Signal and Memo are Copy — don't clone

```rust
// WRONG — unnecessary
let c = count.clone();
button { onclick: move || c.update(|n| *n += 1) }

// CORRECT — Signal is Copy
button { onclick: move || count.update(|n| *n += 1) }
```

### 4. Props are String, not Option\<String\>

Component text props use `String` (empty = not set). The macro converts string literals.

### 5. Only `use rinch::prelude::*`

Don't add separate `rinch-components` or `rinch-theme` deps. Prelude re-exports everything. `features = ["desktop"]` must be explicit.

### 6. State architecture

- Local: `Signal::new()` directly
- Shared: `create_store()` / `use_store()`
- `Effect` is intentionally not in prelude — use `{|| ...}` in rsx instead

### 7. Controlled inputs need `value_fn` + `oninput` together

Without `value_fn`, programmatic `signal.set("")` won't visually clear the input.

### 8. For loops need `key:` and items need `Clone + PartialEq + 'static`

Items with matching keys are NOT re-rendered. Use per-item Signals for per-item reactivity.

### 9. Raw `<input>` oninput receives `String`

```rust
input { oninput: move |value: String| name.set(value) }
```

### 10. Cross-thread: use `send()` / `update_send()`, not `set()` / `update()`

`set()`/`update()` panic off the main thread.

## Quick Checklist

Before finishing any rinch code change, verify:
- Every `.get()` in rsx is inside `{|| ...}` (unless intentionally static)
- No `Some()`, `Rc::new()`, `Callback::new()` wrapping in rsx props
- No `.clone()` on Signals/Memos
- String literals for text props, not `Some(String::from(...))`
- Controlled inputs have both `value_fn` and `oninput`
- For loops have `key:` props
