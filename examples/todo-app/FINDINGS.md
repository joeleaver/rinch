# Todo App - Fresh Eyes DX Findings (Complete Record)

## Phase 1: Reading Documentation

1. **Getting Started guide uses `path` dep, examples use `workspace`** — confusing for newcomers building in-workspace
2. **Feature relationship unclear** — README lists `desktop`, `components`, `theme` separately; components.md says components enables theme. Which do I need?
3. **hello_rinch_dom doesn't use components/theme** — unclear when to use raw HTML vs components

## Phase 2: Writing Code (First Attempt)

4. **No docs on passing callbacks to custom components** — docs only show String/bool props on PascalCase components
5. **Guessed `Callback` as prop type** — docs never show what type to use for callback props on custom components
6. **Unsure if `for` works with Memo from `use_derived`** — docs only show Signal<Vec<T>>
7. **Unsure about `key:` on custom components** — docs show it on HTML elements and mention components, but no full example

## Phase 3: Compilation Errors

8. **ERROR: `Todo` needs `Default`** — `#[component]` macro generates struct with Default. All prop types must impl Default. NOT DOCUMENTED.
9. **ERROR: `Callback` has no `Default`** — Must use `Option<Callback>` instead. NOT DOCUMENTED.
10. **ERROR: RSX macro wraps `on_*` with `Some(Callback::new(...))` but struct field was `Callback`** — Had to change to `Option<Callback>`

## Phase 4: Compilation Fix

11. **Added `#[derive(Default)]` to `Todo`** — Semantically wrong (what's a "default" todo?) but required
12. **Changed `Callback` → `Option<Callback>`** — Now have to manually unwrap when forwarding to child components
13. **Wrote ugly callback forwarding boilerplate** — `let cb = on_toggle.clone(); move || if let Some(ref cb) = cb { cb.invoke() }`

## Phase 5: Interactive Testing

14. **Everything renders correctly on first launch** — Layout, components, theme all work great
15. **Add button works** — Todo added, input cleared, count updated
16. **Enter/onsubmit works** — Todo added via Enter key
17. **Checkbox toggle works** — Strikethrough + color change applied correctly
18. **Filter switching works** — All/Active/Completed correctly filter the list
19. **Delete (X) works** — Item removed, count updated
20. **Clear completed works** — All completed items removed
21. **BUG: value_fn 1-frame delay** — After onsubmit clears signal, typing immediately concatenates with old text. The DOM input element's internal value isn't cleared synchronously.
22. **Docs show `cb.call()` but API is `cb.invoke()`** — inconsistency in components.md
23. **Hooks.md has stale `For` component syntax** — Should use native `for` loop syntax instead

## Summary: Errors Per Phase

| Phase | Count | Severity |
|-------|-------|----------|
| Doc reading confusion | 3 | Low |
| Code writing uncertainty | 4 | Medium |
| Compilation errors | 3 | **High** |
| Runtime/visual bugs | 1 | Medium |
| Doc inconsistencies | 2 | Low |
| **Total friction points** | **13** | |
| Things that worked great | **9** | Positive |
