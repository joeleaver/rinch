# Todo App - Newcomer DX Findings

## Methodology
- Pretended to be a newcomer with only CLAUDE.md and docs/src/guide/ as reference
- Built a todo app with: text input, add button, checkboxes, delete, filter, clear completed
- Documented every friction point, surprise, and unintuitive behavior
- Tested interactively using rinch MCP tools (screenshot, click, type, etc.)

---

## Summary: What Went Right (Positives)

### P1: First-try compilation
The code compiled on the very first `cargo build` attempt with zero errors. This is a strong signal that the documentation is accurate and the API is well-designed. The prelude re-exports everything needed.

### P2: Single dependency line
`rinch = { workspace = true, features = ["desktop", "components", "theme"] }` was all that was needed. No separate `rinch-core`, `rinch-components`, or `rinch-theme` dependencies required.

### P3: Component macro worked seamlessly
Both `#[component] fn app()` (lowercase, auto-injects `__scope`) and `#[component] pub fn TodoItem(...)` (PascalCase, generates struct + Component impl) worked exactly as documented.

### P4: Reactive patterns worked as documented
- `{|| expr}` closure syntax for reactive text and styles worked perfectly
- `use_signal` + reactive closures on component props (filter button variants) worked
- Signal `Copy` semantics eliminated the need for `.clone()` before closures
- Shared handler (`add_todo` closure) used by both `onsubmit` and `onclick` worked

### P5: For loop with keyed reconciliation
`for todo in vec { TodoItem { key: id, ... } }` worked correctly including:
- Adding items (DOM nodes created)
- Removing items (DOM nodes removed)
- Toggling checked state (PartialEq detected change, re-rendered affected items)
- Filtering (collection changes reactively)

### P6: Conditional rendering
`if todos.get().is_empty() { ... }` and `if todos.get().iter().any(|t| t.completed) { ... }` both worked reactively — showing/hiding the empty state and "Clear completed" button.

### P7: TextInput controlled input pattern
`value_fn` + `oninput` + `onsubmit` worked exactly as documented. The input cleared when `input_text.set(String::new())` was called thanks to `value_fn`.

### P8: Component prop auto-wrapping
String literals, closures, and callbacks were all auto-wrapped correctly by the `rsx!` macro. No manual `Some()`, `Callback::new()`, or `String::from()` needed.

---

## Findings: Friction Points & Issues

### ~~F1: Layout doesn't fill the window~~ [RESOLVED - NOT A BUG]
**Severity: N/A (was HIGH)**

**Update:** After thorough DOM inspection, the layout IS correct. The Container (id 42) has `width: 500px` (full window). The Stack (id 41) has `width: 468px` (500 - 16px*2 container padding). Content area is 428px (468 - 20px*2 stack padding). The body node correctly gets `width: 100%` and `flex_grow: 1.0` via a special case in `apply_stylo_styles_to_taffy`. The visual impression of "not filling" was due to the combined padding (36px each side = 72px total on a 500px window), which is actually correct and intentional.

### ~~F2: TextInput `style: "flex: 1"` doesn't make it grow~~ [RESOLVED - WORKS CORRECTLY]
**Severity: N/A (was MEDIUM)**

**Update:** DOM inspection confirmed `flex: 1` on TextInput IS working. The TextInput wrapper (id 13) has `flex_grow: 1.0` and `flex_basis: 0%`. It takes 340px while the Add button takes 76px, plus 12px gap = 428px, perfectly filling the Group. The `style:` prop correctly applies to the component's root element and participates in flex layout.

### F3: No "getting started" tutorial in docs [DOCUMENTATION]
**Severity: MEDIUM**

The docs have excellent API reference and syntax guide, but there's no end-to-end "build your first app" tutorial. I had to piece together knowledge from:
- CLAUDE.md "Application Entry Point" section
- docs/src/guide/rsx-syntax.md for RSX syntax
- docs/src/guide/components.md for component usage
- docs/src/guide/component-props.md for prop types

A newcomer would benefit from a single "Build a Todo App" tutorial that walks through the whole process.

**Suggestion:** Add a `docs/src/guide/tutorial.md` that builds a small app step by step, showing the common patterns (state, input, lists, conditionals).

### F4: For loop with inline `.filter().collect()` — unclear from docs [DOCUMENTATION]
**Severity: LOW**

The docs show `for todo in todos.get() { ... }` but don't show filtering inline. I wasn't sure if `for todo in todos.get().into_iter().filter(...).collect::<Vec<_>>()` would work. It did, but I had to guess.

**Suggestion:** Add a docs example showing filtered/transformed collections in for loops.

### F5: Strikethrough style in TodoItem — confusing mental model [DX / CONCEPTUAL]
**Severity: LOW**

In my `TodoItem` component, I wrote:
```rust
style: { let completed = completed; move || if completed { "line-through..." } else { "" } }
```

This works because when the todo data changes (PartialEq detects it), the entire TodoItem component re-renders with fresh props. So the closure captures the new `completed` value. But from a newcomer's perspective, this is confusing: "I'm wrapping in a closure for reactivity, but `completed` is a plain `bool`, not a Signal — how does this update?"

The answer is: it doesn't update reactively within a single render. It updates because the `for` loop re-renders the item. A closure isn't needed here at all — a plain string would work the same way.

**Suggestion:** Document the distinction between "reactive within a component render" (Signals + closures) vs "reactive across re-renders" (component re-creation from `for` loop reconciliation). Clarify that closures on plain values inside components don't add reactivity.

### F6: `Stack { gap: "0" }` — is "0" a valid spacing value? [DOCUMENTATION]
**Severity: VERY LOW**

I wanted zero gap. I wrote `gap: "0"`. It worked, but I wasn't sure if it would. The docs show spacing scale values (xs, sm, md, lg, xl) and CSS values like "20px", but don't explicitly mention "0".

**Suggestion:** Mention in docs that raw CSS values (including "0") work for gap/spacing props.

### F7: `ThemeProviderProps` location unclear [DOCUMENTATION]
**Severity: VERY LOW**

The components.md docs show `use rinch_core::element::ThemeProviderProps;` as a separate import. But it's actually available from the prelude. A newcomer following the docs might add the explicit import unnecessarily.

**Suggestion:** Update docs examples to just use `use rinch::prelude::*;` without the extra import line, since the prelude already includes ThemeProviderProps.

### F8: No example project in the repo [ONBOARDING]
**Severity: MEDIUM**

There's no simple "app" example in the examples/ directory. The existing examples are either minimal (`hello_rinch_dom` — just text and a counter) or complex (`ui-zoo-desktop` — full component showcase). A todo app or similar small-but-real application would help newcomers see patterns.

**Suggestion:** Ship a `todo-app` example in the repo as a reference implementation.

---

## Visual / Runtime Issues Observed

### ~~V1: Content width doesn't fill window~~ [RESOLVED]
Layout is correct. Container fills 500px window, content area is 428px (after Container 16px + Stack 20px padding on each side). This is expected behavior.

### ~~V2: TextInput placeholder wraps to two lines~~ [RESOLVED]
Placeholder fits on one line at the correct layout width. This was a misperception tied to the now-resolved F1.

### V3: All interactive features worked correctly
- Text input + Enter submission: WORKS
- Text input + Add button click: WORKS
- Checkbox toggle: WORKS (visual checked state + strikethrough)
- Delete button: WORKS (item removed, list re-rendered)
- Filter buttons (All/Active/Completed): WORKS (reactive variant switching + list filtering)
- Clear completed: WORKS (bulk removal)
- Empty state show/hide: WORKS (reactive conditional)
- Counter updates: WORKS (reactive text with formatting)

---

## Lessons Learned — Proposed Changes to Rinch

### Priority 1: Add a "Build Your First App" tutorial [F3, F8]
**Impact: Dramatically improves onboarding**
- Walk through building a todo app step-by-step
- Cover: project setup, state management, components, lists, conditionals, styling
- Ship the todo-app as an example in the repo
- This is the single most impactful change for newcomers

### Priority 2: Clarify component re-render vs signal reactivity [F5]
**Impact: Corrects mental model misunderstandings**
- Document that components in `for` loops re-render when data changes (via PartialEq)
- Clarify that closures on plain values inside components don't add reactivity — the component just re-creates them
- Help newcomers understand WHEN to use `{|| ...}` vs plain values
- This is the most likely source of confusion in real-world usage

### Priority 3: Document filtered/transformed collections in for loops [F4]
**Impact: Removes guesswork for common patterns**
- Show `.filter()`, `.map()`, `.collect()` usage in for loops
- Show how to derive a filtered view from state

### Priority 4: Document that raw CSS values work in spacing/gap props [F6]
**Impact: Small but removes uncertainty**
- Mention "0", "20px", "1rem" etc. explicitly alongside the scale values

### Priority 5: Fix docs import examples [F7]
**Impact: Minor cleanup**
- Update docs examples to use `use rinch::prelude::*;` without redundant import lines

---

## Overall Assessment

**Rinch's DX is remarkably good for a newcomer.** The fact that a complete todo app with
state management, controlled inputs, keyed lists, conditional rendering, and reactive UI
compiled on the first try with zero errors is exceptional. The `#[component]` macro,
`rsx!` auto-wrapping, Signal `Copy` semantics, and comprehensive prelude all contribute to a
smooth experience. The remaining friction points are primarily documentation gaps, not API issues.

**No layout or rendering bugs were found.** The initial suspicion of a layout issue (F1/F2)
was investigated thoroughly through DOM inspection and proved to be correct behavior — the
padding hierarchy (Container + Stack) creates appropriate margins, and `flex: 1` works as
expected on components via the `style:` prop.
