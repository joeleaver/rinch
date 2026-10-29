# rsx control flow, for loops, reactive bindings, prop rules

Read this page when your task touches: if/for/match in rsx, keyed reconciliation, value_fn/oninput, undo/caret keys in fields, rsx prop and attribute rules.

Moved verbatim from `CLAUDE.md` on 2026-10-22 (the index there points here). New mechanism notes for this area go on this page, not in `CLAUDE.md`. A code comment that cites a `CLAUDE.md` section by name means a section now on one of these pages: `grep -rn '<name>' docs/internals/`.

### Native Control Flow (if / for / match)

The `rsx!` macro supports native Rust control flow. All control flow is **always reactive** — conditions, iterators, and scrutinees are automatically wrapped in closures and tracked by Effects.

**`if` / `else` / `if let`:**
```rust
let visible = Signal::new(true);
let user = Signal::new(Some("Alice".to_string()));

rsx! {
    div {
        // if/else
        if visible.get() {
            p { "Visible!" }
        } else {
            p { "Hidden" }
        }

        // if let
        if let Some(name) = user.get() {
            p { "Hello, " {name} "!" }
        }
    }
}
```

**`for` loops with keyed reconciliation:**
```rust
let todos = Signal::new(vec![
    Todo { id: 1, name: "Buy groceries".into() },
    Todo { id: 2, name: "Write code".into() },
]);

rsx! {
    div {
        for todo in todos.get() {
            div { key: todo.id, {todo.name.clone()} }
        }
    }
}
```

The `key:` prop enables efficient keyed reconciliation. Items with matching keys are preserved (not re-rendered). If no `key:` is provided, items are keyed by `Debug` formatting.

**Keys must be unique within one list**, and what a repeat means depends on who chose the key (issue #185):

- **You wrote `key:`** — a repeat is a mistake in your key. The repeat is **not rendered** and a warning is logged; the first occurrence wins, as in React.
- **No `key:`** — the framework fabricated the key from `format!("{:?}", item)`, so a repeated *value* is not your mistake. The fabricated key is made unique by its occurrence ordinal instead, and **every row renders**: `for tag in ["rust", "rust", "gui"]` renders three. Reordering still moves rows rather than rebuilding them, because the ordinal follows the value, not the position.

An index key (`key: i`) is a last resort: it makes identity follow *position*, so inserting anywhere but the end re-renders every row after the insertion point and loses per-row state — the exact failure `key:` exists to prevent.

**Important:** Items with matching keys are **not** re-rendered when the collection changes. Their existing DOM subtree is preserved as-is. For per-item reactivity, use Signals inside each item.

**`match` with multi-branch rendering:**
```rust
let tab = Signal::new(0);

rsx! {
    div {
        match tab.get() {
            0 => div { "Home" },
            1 => div { "About" },
            _ => div { "Not found" },
        }
    }
}
```

Pattern bindings and guards are supported — each arm re-evaluates the scrutinee to extract bound values.

**A braced arm body is decided by how it starts** (issue #395). It holds rsx
children — several nodes, like an `if`/`for` body — when it starts with `let`,
or with an rsx *head* (`Name {`, a string literal, `if`/`for`/`match`, or a
braced interpolation) **followed by another node**:
`0 => { h2 { "Home" } p { "…" } }`, `{ {label} span {} }`. A lone element or
literal is that one node. Everything else is the one braced expression it always
was: `0 => { overview_section(__scope) }` (ui-zoo's routing), `0 => {a.clone()}`,
and a head followed by `.` or an operator (`{ "a".to_string() }`,
`{ if … {} else {} .len() }`). `{ Point { x: 1 } }` is therefore a component, as
unbraced; a path (`geom::Point { … }`), and a struct literal that cannot be an
element (`{ Foo { a } }`, `{ Foo { ..Default::default() } }`, a method called on
one), stay expressions. Lone control flow in braces is still #221's transparent
brace, diagnostic included. In a multi-node arm the rsx parser reports a typo at
the typo, including one inside a leading `if`/`for`/`match`: a head that fails
to parse is skipped **by tokens** to see whether a node follows it — a
diagnostic-only heuristic that a brace in the condition (`if let Foo { a } = x`)
can defeat (`parse_braced_arm_body`, `failed_head_is_rsx`,
`crates/rinch-macros/src/node.rs`).

**Runtime desugaring:** `if` → `show_dom()`, `for` → `for_each_dom_typed()`, `match` → `match_dom()`.

**A brace around control flow is transparent** (issue #221). `div { { match x { … } } }`
renders the same reactive `match_dom` as `div { match x { … } }`, and likewise for
`if` and `for`, at any depth — a braced arm body (`_ => { match y { … } }`) included.
It used to parse as a plain Rust expression and go through `IntoNode::into_node`,
which evaluates **once**: the branch that rendered on mount was the branch you kept,
with nothing at the call site to distinguish it from its unbraced twin.

Braced control flow whose bodies are **not** rsx cannot be made reactive — arms
written as nested `rsx! { … }`, or bodies that are plain calls — so it is now a
compile error naming both rewrites rather than a silent freeze: drop the braces
and write each body as rsx for reactive *markup*, or wrap the whole construct in
`{|| … }` for a reactive *value*. Everything else in braces is unchanged: an
expression (`{ count.to_string() }`), a reactive closure (`{|| count.get()}`) and
a call (`{ section(__scope) }`) never start with a control-flow keyword and are
never touched.

### For Loop Details

The `for` loop variable is **owned** (`T`, not `&T`), so you can capture it directly in `move` closures:

```rust
for todo in todos.get() {
    let id = todo.id;
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: move || todos.update(|t| t.retain(|t| t.id != id)),
            "Delete"
        }
    }
}
```

**Item type requirements:** `Clone + PartialEq + 'static`. The `PartialEq` bound enables selective re-rendering — when the list changes, surviving items (same key) are compared by value. Only items whose data actually changed are re-rendered.

**Keys must be unique** (issue #185). An item whose key repeats one already seen in the same pass is **not rendered** and a warning is logged — first occurrence wins, as in React. The reconcile rests on one key naming one item state and one mounted sibling; a repeat used to leave a row that rendered, swallowed clicks and never updated again. Note the no-`key:` fallback keys by `format!("{:?}", item)`, so two `Debug`-equal items collide: `for n in vec![1, 1, 2]` renders **two** rows, not three. `virtual_list` applies the same rule within a visible range.

When the list changes, `for` uses keyed reconciliation (LIS algorithm) to compute minimal DOM operations:
- **Insert**: New items are rendered and added at the correct position
- **Remove**: Deleted items have their DOM nodes removed
- **Move**: Reordered items are repositioned without re-rendering
- **Changed**: Surviving items with different data (via `PartialEq`) are re-rendered
- **Unchanged**: Items with matching keys and equal data keep their DOM nodes

**Per-item state in for bodies**: Per-item reactive state works inside `for` loop bodies. Each item gets its own isolated scope:

```rust
for todo in todos.get() {
    let editing = Signal::new(false);  // Per-item state
    div { key: todo.id,
        {todo.name.clone()}
        button {
            onclick: move || editing.update(|v| *v = !*v),
            {|| if editing.get() { "Done" } else { "Edit" }}
        }
    }
}
```

### Programmatic Conditional/List Rendering

For cases requiring explicit control, use the runtime functions directly:

- `show_dom()` — conditional rendering (equivalent to `if`/`else`)
- `for_each_dom_typed()` — keyed list rendering (equivalent to `for`)

### Reactive Component Bindings

Some components support reactive value binding via `_fn` props. The `rsx!` macro auto-wraps `_fn` props — just pass a closure:

| Component | Prop | Type | Purpose |
|--------|------|------|---------|
| `Checkbox` | `checked_fn` | `Option<ReactiveBool>` (`Rc<dyn Fn() -> bool>`) | Reactive checked state |
| `TextInput` | `value_fn` | `Option<ReactiveString>` (`Rc<dyn Fn() -> String>`) | Reactive value binding |

**Controlled Input Pattern:** For controlled inputs, use `value_fn` + `oninput` together. `value_fn` keeps the DOM in sync with your signal; `oninput` updates the signal from user input. Without `value_fn`, programmatic `signal.set("")` won't clear the input visually.

A `value_fn` write (any `set_attribute("value")`) to the **focused** field is adopted by the field on both backends (issue #238): it becomes the text the next keystroke edits, the caret keeps its logical position (kept prefix/suffix keeps it, a same-length rewrite leaves it in place, a resized rewrite puts it after the new text), a selection survives, the write is deferred during an IME composition, and it never commits `onchange` by itself. A normalizing or rejecting `oninput` (uppercase, digits-only, max-length) that writes back on every keystroke therefore just works — on desktop, where it used to snap back to the pre-rewrite text on the next key, as well as on the web, where it used to throw the caret to the end.

**The echo is not written back** (issue #238, part 2). `TextInput`, `PasswordInput`, `Textarea` and `NumberInput` share one `value_fn` binding, `rinch_components::value_binding::bind_value_fn`, which writes only a value the field does not already show — read through `NodeHandle::live_value` / `DomDocument::live_value`, the text the field holds *now*. That is the `value` attribute on desktop (the runtime mirrors each edit into it before dispatching `oninput`, so the trait default is honest there) and the `.value` **property** of an `<input>`/`<textarea>`/`<select>` on web, where the attribute keeps the last programmatic write while the user types. **Never compare against `get_attribute("value")` to ask what a field says** — on web that is a fossil, and a guard reading it skips a change back to the fossil's own value. `ColorPicker`/`ColorInput` read `live_value` too; the per-component `typed` record they kept for this (#235) is gone. `MockDomDocument::__type_into` moves a mock control's live text without its attribute, which is how a host test reaches the web's state.

**Undo/redo on a focused field** (issue #288): Ctrl+Z, and Ctrl+Shift+Z / Ctrl+Y (Cmd on macOS; never with Alt — AltGr on Windows — and refused while an IME preedit is shown), reach `EditableState`'s stack through `handle_input_edit_command`, so `oninput` fires with the restored text and `readonly`/`disabled` refuse them like any edit. One key press is one step: `EditableState::execute` folds a command's operations into a `TextOperation::Group` (typing over a selection), and `handle_input_edit_command` folds the rewrite a keystroke's own `oninput` made into that keystroke's step (`adopt_focused_input_rewrite_from_dom`, `UndoStack::mark` / `EditableState::group_undo_since`) — or drops the step outright when the rewrite put the text back as it was (a rejected keystroke). **Every other adopted `value` write clears the undo and redo stacks** (`adopt_focused_input_value_from_dom`), as Chrome does for any script write to `.value`: a clear after submit is not undoable. (A keystroke's own `oninput` rewrite is the one divergence: Chrome clears history for it too, so a normalizing field has no undo on the web.) No typing coalescing, and the text context menu has no Undo row. Pins: `input_commit_tests` (`ctrl_z_*`, `a_normalizing_field_*`, `a_programmatic_write_clears_*`, `a_rejected_keystroke_*`, …).

**Caret keys follow the platform** (issue #951): `app/text_keymap.rs`'s `text_key_action(TextKeymap, KeyCode, Modifiers)` is the one, pure answer for arrows, Home/End, Backspace and Delete in a desktop `<input>`/`<textarea>` and for the rich editor's caret motions (`dispatch_new_editor_key`; its deletes stay the editor keymap's). `TextKeymap::Standard` (Windows/Linux): Ctrl = word, Home/End = visual line, Ctrl+Home/End = value, Ctrl+Backspace/Delete = word delete (a character until #951). `TextKeymap::MacOs`: Option = word (and word delete, the editor keymap's `Alt-Backspace`), Cmd+Left/Right = visual line, Cmd+Up/Down = value, Home/End leave the caret (Shift extends to the value's ends). A field's word delete never crosses a line break (beside one it takes the break alone: `rinch-editable`'s `word_delete_start`/`_end`), a field's words are the editor's (`rinch-words`, the leaf crate both `rinch-editor-core` and `rinch-editable` take `word_segments` from — the field engine does not depend on the editor model; #1619: `foo-bar|` + Ctrl+Backspace → `foo-`, `3.14` one word; a textarea's `\n` is a word of its own, one stop: Ctrl+Right at a line's end lands past it (Chrome 155); Ctrl+Left stays on the caret's line (an indented line: its start, Chrome 155) except from exactly a line's start, where it lands before the break and Chrome 155 leaves the caret (Joe's decision); a word motion segments only the caret's line, since a break joins nothing (one long unbroken CJK run is still superlinear with the dictionary, #1626); the ICU dictionary through `rinch-editable/word-dictionary`, on for desktop and android, off for embed), Ctrl+Alt (AltGr) is no word delete, and in `type=password` a word is the whole value (word delete → to the value's edge, word move → its ends), as in Chrome. The keymap is the field `RinchApp::text_keymap` (`TextKeymap::host()`, a `cfg!`), so the macOS fixtures (`app/text_keymap_951_tests.rs`) run on Linux; a test that means the Windows/Linux keys sets `TextKeymap::Standard`. Not done (#1614): macOS Ctrl+Left/Right (Cocoa line moves) are character moves, Option+Up/Down (paragraph moves) line moves, the Emacs keys are unbound — all pinned as they are; Cmd+Backspace (macOS line delete) is a character delete in a field and nothing in the editor; rinch-web's editor still takes Ctrl-or-Cmd as its word modifier on every platform.

**`onsubmit`:** TextInput supports `onsubmit` which fires when the user presses Enter.

Example - controlled TextInput with submit:
```rust
let input_text = Signal::new(String::new());

rsx! {
    TextInput {
        placeholder: "Type here...",
        value_fn: move || input_text.get(),  // Macro auto-wraps in Some(Rc::new(...))
        oninput: move |value: String| input_text.set(value),
        onsubmit: move || {
            println!("Submitted: {}", input_text.get());
            input_text.set(String::new());  // Clears the input thanks to value_fn
        },
    }
}
```

### `ref:` and `NodeHandle::bounding_rect`

**`ref: target`** (#1779) is pulled out of the props by `html::element_to_dom_html`
and the three component sites, and becomes `rinch::core::dom::bind_node_ref(__scope,
target, &elem)` — after the element's attributes and children, before the caller
appends it; on a component, after `Component::render` on its root (inside the
render closure on a re-rendering component, so each render binds its own root in
its own scope). `target` is a `Signal<Option<NodeHandle>>`; the cleanup that clears
it runs on the building scope's disposal, untracked, and only while the signal
still holds that node (compared by node id and `Weak::ptr_eq` of the document, no
document borrow). The guard matters where an element other than the one going
holds the signal (`for` rows sharing one ref; `a_removed_row_clears_its_ref`); a
branch or component re-render cannot reach it, since `show_dom`, `match_dom` and
`reactive_component_dom` dispose the old scope before building the new output.
`ref` is a keyword: `element.rs` peeks `Token![ref]` beside `Ident` to see it as
a prop (`r#ref:` was already an `Ident`, an attribute named `ref`; it is now the
binding too). Pins: `rinch-macros/tests/rsx_node_ref_1779.rs`.

**`NodeHandle::bounding_rect`** is `DomDocument::bounding_rect` (default `None`):
for a flowed inline element (a 0x0 ghost box) rinch-dom answers
`RinchDocument::inline_element_rect`: the union of its line fragments, from
`out_of_flow::measure_span_fragments` (the #631 measurement, read from
`InlineLayout::span_fragments` when a positioned box already had it measured,
else measured on demand and not cached; `SpanFragments::union_left/right` hold
every fragment's extent less the white space that ends a line), grown by the
element's padding and pushed through the host's painted transform less its own
scroll. A split inline (#513) has no fragments and falls back to the ghost box
(a line `text-overflow: ellipsis` cuts keeps them since #1100). Every other element: rinch-dom answers
`paint::painted_border_box(tree, id, 1.0)` — the conversion
`ClickContext` and the bounds signals use — from the last layout, a zero rect
under `display: none` or for a `display: contents` element (Chrome's answer; the
layout of such a box is zeroed, so a sum up the chain would give its parent's
origin), and `None` for a text node or one not connected; rinch-web answers
`getBoundingClientRect()` for a connected element. Pins:
`rinch-dom/tests/bounding_rect_1779_tests.rs`,
`rinch-web/tests/bounding_rect_and_ref_1779.rs` (Chrome 155, Firefox).

### RSX Prop Transformation Rules (IMPORTANT - Read Before Using Components)

The `rsx!` macro **automatically wraps** component prop values. You must NOT manually wrap them or you'll get confusing type errors from double-wrapping.

| Prop pattern | What you write | What the macro generates |
|---|---|---|
| `oninput` (closure) | `oninput: move \|val\| do_thing(val)` | `(InputCallback::new(move \|val\| do_thing(val))).into()` |
| `oninput` (value) | `oninput: my_callback` | `(my_callback).into()` |
| `on*` (closure) | `onclick: move \|\| do_thing()` | `(Callback::new(move \|\| do_thing())).into()` |
| `on*` (value) | `onclick: my_callback` | `(my_callback).into()` |
| `*_fn` (reactive) | `value_fn: move \|\| text.get()` | `Some(Rc::new(move \|\| text.get()))` |
| `icon`, `*_icon` | `icon: TablerIcon::Check` | `Some(TablerIcon::Check)` |
| bool literal | `disabled: true` | `true` (no wrapping) |
| int literal | `size: 42` | `Some(42)` |
| float literal | `value: 30.0` | `Some(30.0)` |
| string literal | `variant: "filled"` | `String::from("filled")` |
| `Some(...)` or `None` | `tree: Some(state)` | `Some(state)` (pass-through, preserves unsizing coercion) |
| any other expr | `variant: my_var` | `(my_var).into()` (auto-wraps `T` → `Option<T>` via `From`) |

**Common mistakes (DO NOT do these):**

```rust
// WRONG - don't manually wrap callbacks
TextInput { oninput: Some(InputCallback::new(move |val| ...)) }
// RIGHT - macro wraps closures automatically
TextInput { oninput: move |val: String| input_signal.set(val) }

// WRONG - don't manually wrap callbacks
Button { onclick: Some(Callback::new(|| ...)) }
// RIGHT - just pass the closure
Button { onclick: move || do_something() }

// RIGHT - you can also forward an existing Callback directly
Button { onclick: my_callback }

// WRONG - double-wraps into Some(Some(TablerIcon::Check))
Alert { icon: Some(TablerIcon::Check) }
// RIGHT - macro adds Some(...) for you
Alert { icon: TablerIcon::Check }

// WRONG - component expects String, not Option<String>
Button { variant: Some(String::from("filled")) }
// RIGHT - macro generates String::from("filled")
Button { variant: "filled" }
```

**Additional notes:**
- `on*` props accept both closures and existing `Callback` values. The macro uses `.into()` so the field can be either `Callback` or `Option<Callback>`.
- `Callback` and `InputCallback` have built-in defaults (no-op), so custom components can use `on_toggle: Callback` instead of `Option<Callback>`.
- Component text props (e.g., `variant`, `color`, `size`) are now `String` (not `Option<String>`). Empty string means "not set". The macro auto-converts string literals to `String::from(...)`.
- `_fn` suffix props (e.g., `checked_fn`, `value_fn`) are auto-wrapped — just pass the closure directly, don't wrap in `Some(Rc::new(...))`
- **The table above is for components.** On an HTML element, `on*` names are a closed list (`crates/rinch-macros/src/events.rs`: `onclick` → `data-rid`, `oninput`, `onchange`, `onsubmit`, `onscroll`, `onkeydown`/`onkeyup` (#1678, below), the mouse, context-menu, drag and file-drop events), and any other `on*` name — `onfocus`, a typo — is a **compile error** listing them, with a "did you mean" (#1590; it used to fall through to `data-rid` and become a *click* handler, so `input { onsubmit }` ran on a click and never on Enter). An `onsubmit`-, `onkeydown`- or `onkeyup`-only `<input>`/`<textarea>` gets the same desktop-only no-op `data-oninput` shim an `onchange`-only element gets, so it takes typed text; a wrapper `div { onsubmit }` does not. A `*_fn` prop (`value_fn`, `checked_fn`, …) on an HTML element is a compile error saying to write `value: {|| …}` (#1591; it used to be a `value_fn` attribute nothing reads): every HTML attribute already takes a reactive closure and updates surgically, so the component's `_fn` convention has nothing to add there. A raw reactive `value:` writes on every fire, with no #238 echo guard — the components' `bind_value_fn` is the guarded binding.
- **Note:** ThemeProvider props (`primary_color_fn`, `dark_mode_fn`) use a different codegen path and still require manual `Rc::new()` wrapping

**Component Props vs HTML Attributes:**

- **HTML elements** (`div`, `span`, `p`, etc.) accept any attribute as a string: `style:`, `class:`, `id:`, custom `data-*`, etc. — except an unsupported `on*` name and a `*_fn` name, which are compile errors (#1590, #1591; see the note above). They also support reactive closures `{|| expr}` on any attribute. **An attribute *name* is ASCII case-insensitive in HTML content and case-SENSITIVE in SVG content** (issue #688), so `RinchDocument::set_attribute` stores it lowercased unless the element's tag is an SVG one: `<div ID="up">` is `id`, and `#up`, `[id]`, `.x` via `CLASS=` and `STYLE=`'s inline style all work, while `viewBox`, `preserveAspectRatio`, `gradientUnits`, `stdDeviation`, `markerWidth` and `startOffset` keep the author's spelling. The decision is keyed on the **tag**, not on an `<svg>` ancestor, and that is forced rather than chosen: `rsx!` and the `Element::Html` parser both write an element's attributes *before* appending it to its parent, so at the write there is no ancestor to walk and a walk would lowercase every SVG child's name. `crates/rinch-dom/src/attr_name.rs` holds the list and the four SVG names it deliberately omits — `a`, `script`, `style`, `title` are HTML element names too, and the HTML reading is the likely one. That is free for three of them, which carry no camelCase SVG attribute, and **not** free for `a`: SVG's `requiredExtensions` / `systemLanguage` apply to it, so those two fold on an `<a>` inside an `<svg>`. Inert today rather than harmless — nothing in the workspace reads either name and `paint/svg.rs` drops `<a>` through its `_ => {}` arm — but it is the one place the list trades correctness for the ambiguity, and worth weighing again if conditional processing lands. `get_attribute` and `remove_attribute` fold the same way (a fold on the write and not the remove could never turn a boolean attribute off again), and so does `is_boolean_attribute`, or a `CHECKED: {|| false}` would fall through to the literal writer as `checked="false"` — #551 under a different spelling. The **value** is never folded, so an `id` still matches case-sensitively outside quirks mode. `rinch-web` needs none of this fold for the stored **attribute**: the browser folds the name itself, per namespace, because `web_document.rs` creates SVG elements with `createElementNS`. It still needed its own fold for a narrower question the browser's storage fold does not answer — which of a handful of names its own Rust `match` on the literal name treats as a live-property mirror (`checked`/`selected`/`muted`, below): `set_attribute("CHECKED", "")` always set the (browser-folded) content attribute, but fell through every arm of that match and so never called `sync_presence_property`, leaving `.checked` unmirrored on the web alone — desktop already folded via the tag-keyed `fold_attribute_name` above (issue #758). An **HTML boolean attribute** is the exception to "as a string": its *presence* is its value, so `rsx!` writes the bare presence form for a truthy value and **removes** the attribute for a falsey one, through `NodeHandle::write_attribute` rather than `set_attribute` (issue #551). Writing `disabled="false"` would leave the attribute present, which HTML — and therefore the browser, measured — reads as *disabled*; a reactive `disabled: {|| busy.get()}` was disabled from the first render and never recovered, on **both** backends. The set is `rinch_core::dom::is_boolean_attribute`: the 30 rows the WHATWG attributes index marks "Boolean attribute", plus `hidden` (enumerated, but its invalid-value default is the hidden state, so `hidden="false"` hides) and rinch's own `data-disabled` / `data-nofocus` / `data-trap-focus` / `data-backdrop` / `data-scroll-lock-exempt`. **The rule keys on the attribute name, not the value's type**, and that is load-bearing: `draggable` is enumerated (`"true"`/`"false"`, invalid → `auto`) and desktop's drag dispatch matches the literal `"true"`, the ARIA states are tri-valued, and `data-viewport-ready`'s *absence* means ready — presence-mapping any of them loses or inverts it. A *string* yielded into a boolean attribute follows `attr_is_truthy` (on unless `"false"` in any case, or `"0"`), which is the **writer's** rule and nobody's reader: every desktop reader of the HTML set is presence-only, correctly, because a browser is (`:checked` matches on `checked="false"`, `:disabled` on `disabled="false"`, both measured) — which is why #551 reproduced on desktop and why the cure is a writer that removes rather than a reader that learns a falsey string. **`set_attribute` is not that writer.** It is the literal primitive on both backends — including for the `checked` family, which is issue #612's divergence over again and what **#622** closed: the web arm used to presence-map `checked` / `selected` through `attr_is_truthy`, so `set_attribute("checked", "false")` unchecked the box on web and checked it on desktop. It now writes the string, leaving the attribute *present*, which checks the box on both. Writing a `bool` is `write_attribute`'s job; meaning "off" is `remove_attribute`'s. On web the live IDL property (`.checked`, `<option>.selected`) is mirrored from that **presence** and not from the string, because a browser stops mirroring the attribute onto the property once the user has toggled the control (the dirty-checkedness flag) and rinch has no such flag — desktop's reader sees the attribute and nothing else, so the app's write has to win on both. `indeterminate` is the exception and stays truthiness-mapped: HTML has no such content attribute at all, so it has no presence for anyone to read. **The same names are the reason `write_attribute` cannot skip a falsey write** (issue #687, and #754 for the third): its removal is otherwise guarded on the attribute being present, since "already absent" reads as "already off" and costs no style invalidation — but a user's toggle (or, for `muted`, a `<video controls>`'s own mute button) leaves the property on and the attribute absent, so the guard skipped the one write that could have corrected it and the control stayed checked/muted against a binding saying `false`. `checked`, `selected` and `muted` (`rinch_core::dom::is_presence_reflected_attribute`) therefore go to the backend unconditionally; desktop pays nothing for the extra call, because `RinchDocument::remove_attribute` returns before invalidating anything when the node does not carry the attribute — what `removeAttribute` does in a browser. `muted` joined the other two in #754: HTML's `muted` content attribute has no dynamic effect at all (it only seeds `defaultMuted`, once, at the media element's load algorithm), but rinch mirrors its presence onto the live `.muted` IDL property anyway — deliberately more dynamic than plain HTML — because nothing else lets a reactive `muted: {|| m.get()}` binding mute or unmute a `<video>`/`<audio>` element; desktop has no media element that reads `muted` at all, so this is web-only. A `"false"` escape survives for exactly five attributes, rinch's own `data-disabled` / `data-nofocus` / `data-trap-focus` / `data-backdrop` (#1093) / `data-scroll-lock-exempt` (#701), spelled once as `rinch_core::dom::data_attr_is_on`; issue #612 retired it from `disabled` / `readonly`, where it had been desktop-only and so a pure divergence. It is a deliberate convention rather than a leftover, and `data-nofocus` / `data-trap-focus` / `data-backdrop` are what show that — the web reads them the same way, through `[data-nofocus]:not([data-nofocus="false" i])`, `[data-trap-focus]:not([data-trap-focus="false" i])` and `data_attr_is_on`, while `data-disabled` and `data-scroll-lock-exempt` have no web reader at all. Note `data_attr_is_on` is deliberately *narrower* than `attr_is_truthy`: `"0"` is on, matching those selectors, and `"0"` is therefore the only value that distinguishes the two rules — which is why every desktop reader pins it (`computed_style_tests::the_data_escape_excuses_only_false_at_the_reader`, `nofocus_tests::only_false_opts_out_not_zero`, `trap_focus_tests::the_false_escape_opts_out_and_zero_does_not`, `backdrop_any_button_1093_tests::data_backdrop_false_is_not_a_backdrop`, `overlay_scroll_lock_tests::scroll_lock_exempt_false_opts_out_and_zero_does_not`). **`oninput` and `onchange` on `<input>`/`<textarea>` elements** receive the input value as a `String` — use `Fn(String)` closures, not `Fn()`. They are **not aliases** (issue #226): `oninput` fires per keystroke with the live value; `onchange` fires once at the commit boundary — focus leaves the control after a modification, Enter (single-line inputs only; a `<textarea>` commits at blur), or a `<select>` pick — and only if the value actually changed since focus. On Enter, `onchange` fires before `onsubmit`:
  ```rust
  input {
      oninput: move |value: String| name_signal.set(value),
      onchange: move |value: String| autosave(value),
      placeholder: "Type here...",
  }
  ```
  **`onscroll`** takes `Fn(ScrollEvent)` (issue #177). The payload is a
  `#[non_exhaustive]` struct carrying **both** axes — `ev.scroll_top` and
  `ev.scroll_left` — so a horizontal-only scroller reports where it is instead
  of an unchanging zero. It fires once per container that moved, whichever axis
  moved it. Construct one with `ScrollEvent::new(top, left)`; a struct literal
  will not compile downstream. `ScrollEvent` is in the prelude.
- **Components** (`Button`, `TextInput`, `Stack`, etc.) accept their declared struct fields as props. Additionally, all components support these universal props:
  - `style:` — Applied to the component's root DOM element after rendering. Supports static strings and reactive closures. **Merged, not assigned** (issue #647): the declarations are laid over whatever is already on the root, last-wins per property, so every inline declaration the component's own `render` wrote survives — which is how `Modal`, `Drawer`, `Popover`, `DropdownMenu`, `Notification` and `LoadingOverlay` publish `z_index`, `offset`, `overlay_blur` and the rest as custom properties. A write of the whole attribute silently returned all five overlays to their stylesheet defaults. The same is true of `style:` on an HTML element, where the second author is usually a style shorthand: `div { style: {|| …}, p: "md" }` keeps its padding. An element with no second author keeps the author's string verbatim, on every fire and not just the first.

    A **reactive** `style:` takes its own previous declarations back on each re-run: one it no longer makes is removed, or restored to the value it had displaced. Two rules keep that from reaching past the caller's own work, and each is a bug that was there before they were written. It **overwrites a property it still declares where that property stands**, never removing and re-appending it — a declaration that moves to the end of the block changes which of two colliding declarations wins, and `div { style: {|| …}, mt: "8px" }` silently lost its top margin on the first signal change that way. And it **leaves alone a property whose current value is not the one it wrote**, because somebody else has written it since and reverting it would undo a value they never saw.

    What is **not** repaired, deliberately: a genuine same-property collision between `style:` and a style shorthand. `div { style: {|| …}, p: "12px" }` whose closure also names `padding` gets the shorthand at mount, because shorthands are applied last, and the closure's value from its first re-fire onward, because nothing re-asserts a shorthand after the fact. Declaring one property from two props on one element is the mistake; making shorthands reactive is separate work.

    **Every author splits and re-joins the attribute with one parser** — `rinch_core::dom::split_declarations`/`serialize_declarations` (issue #670) — so a `;` or `:` inside `url(…)` or a quoted string is part of that value for all of them, and a property declared twice collapses at the **last** declaration's position, as CSSOM does. Both are behaviour rather than spelling. **Property names are ASCII case-insensitive** and come out lowercased (issue #711): `COLOR` and `color` are one property, a `set_style("COLOR", …)` overwrites an existing `color` rather than declaring it twice, and the rule applies to the name a caller passes as well as the names the parser reads — `rinch_core::dom::normalize_property_name` is the one place it lives, and `rinch-web` needs none of it because the browser's own CSSOM lowercases. A **custom property** is compared exactly, so `--Foo` and `--foo` stay two variables. The collapse is also priority-aware: the surviving **value** is the `!important` one where the two disagree (`color: red !important; color: blue` stays red), while the **slot** is still the last declaration's — a pre-existing #670 residue that the case fold could not be correct without, since two spellings used to reach Stylo as two declarations and its own cascade picked the important one. `split_declarations`'s own collapse is **syntactic** — it has no Stylo and cannot tell a valid declaration from one that will be rejected — so a browser's **post-validity** collapse (Chrome drops an invalid declaration as it parses, so the duplicate never competes) used to be out of reach for `color: notacolor !important; color: blue`, which computed blue in Chrome and black in rinch, and for the plain-vs-plain shape `color: blue; color: notacolor` (older than the priority arm, which only widened it). **Closed for the Stylo-reaching path (issue #722):** `RinchDocument::merged_inline_style` — what `set_style`/`set_styles` route through — no longer feeds Stylo's parser the collapsed form for a property it is not itself writing. It reads the existing attribute with `rinch_core::dom::split_declarations_keeping_duplicates` (the same tokenizer, per-name collapse switched off) instead of `split_declarations`, so an untouched property's duplicates reach Stylo exactly as written and Stylo's own post-validity collapse decides the winner — both shapes above now compute blue, matching Chrome. A property the call *is* writing has no such ambiguity (the caller supplies one unambiguous new value), so every existing occurrence of that name still collapses to it, at the position the pre-existing position rule gives it. Pinned by `inline_style_case_tests::a_duplicate_collapses_before_validity_not_after`, flipped to assert the Chrome-matching values. What the attribute reads back as after a merge is the CSSOM-touched form, lowercased and collapsed — a browser reaches the same string one write later, when something first touches its CSSOM — **except for a property neither this write nor an earlier one has touched, whose duplicates now ride through verbatim** (measured in Chrome 153: a browser's CSSOM collapses a property the moment its attribute is first parsed, markup or a single `setAttribute` alike, so `getAttribute('style')` after any *later* unrelated `style.setProperty` already shows it collapsed too; rinch keeps no persistent, already-resolved declaration block to match that, and re-derives the attribute string from the raw text on every write). The *computed* value is unaffected — Stylo re-resolves the duplicate from that text on every merge — only the literal attribute string can show a duplicate Chrome's `getAttribute` would not — tracked in #1353. `rinch-dom` kept its own `parse_style_string` until then: it split on a bare `;`, so a `background-image: url(data:image/png;base64,…)` already on a node was destroyed by the next unrelated `set_style` on it, and it collapsed at the *first* position, which computes `left: 25px` for `inset: 0px; left: 25px; inset: 4px` where Chrome computes `4px`. `MockDomDocument::set_style` merges the same way now (issue #666), so a component test rendered onto the mock is an oracle for inline-style composition. Two read-only scanners in crates that cannot reach `rinch-core` are deliberately left out and tracked in #705.
  - `class:` — Merged with the component's own CSS classes (additive, not replacing). Supports static strings and reactive closures. A **reactive** `class:` has the same memory the reactive `style:` above does: it takes off the words it last wrote before adding the new ones, so it never accumulates its own old values. It does not reach a class it has **never named** — but naming one is enough to own it: a word the closure emits is the closure's to remove, and on the next re-run that stops naming it the word comes off and nothing puts it back. That includes a class the component itself wrote, **base class included** — `class: {|| if p.get() { "rinch-checkbox marker" } else { "marker" }}` strips `rinch-checkbox` off the element for good (measured). Pre-existing #647 behaviour and the same rule the reactive `style:` follows, not something components can defend against.

    **The merge holds only if the component keeps its side of it** (issue #717). `class:` is applied to the handle `Component::render` *returned*, i.e. after the component has built its tree and registered its effects — so **a component effect must add and remove the one class it is responsible for** (`NodeHandle::add_class` / `remove_class`) and must never write `class` wholesale. Thirteen effects across ten components did, rebuilding the attribute from a base string captured during `render`, and so discarded the caller's class on their first run with no way to get it back short of a re-render: `Checkbox { class: "compact", checked_fn: … }` styled correctly until the user clicked it once. That also means an effect must not be the thing that establishes a node's *base* class — `Tree`'s row was written only by its selection effect, which is the same coupling seen from the other end. `add_class` is **idempotent**, as `DOMTokenList.add` is, which matters because an effect re-runs whenever anything it read changes and `Signal::set` notifies on every write, equal value or not (`set_if_changed` is the other method); without it three `set(true)` calls left three copies of the modifier.

  - **Hyphenated attributes** (issue #433) — `data-nofocus: ""`, `aria-label: "…"`, any name with a `-` — are written on the component's **root DOM element** after `Component::render`, the route `style:`/`class:` take (`component_codegen::is_root_attribute`; `html::generate_attr_code`). A hyphen is not legal in a Rust identifier, so no struct field can collide. Same rules as on an HTML element: through `NodeHandle::write_attribute`, so a boolean attribute (`data-nofocus`, `data-trap-focus`, `hidden`, …) is a presence or a removal (#551); a `{|| …}` value is an effect on the root and re-renders nothing, on a component that re-renders for a struct prop too (#1190, below); it never by itself makes a component a re-rendering one; a component that re-renders for another reactive prop gets every attribute again on its new root. **The caller writes last**, so it wins a collision with an attribute the component wrote on its own root at render, and a falsey boolean removes the component's; after mount the last write wins. That includes rinch's wiring attributes: a caller's `data-rid` on a component replaces the id of the component's own click handler. A bare `id`, `tabindex`, `role` or `title` is unhyphenated and is **not** routed — on a component it is still a struct field. Before this, a hyphenated name on a component panicked the macro and a toolbar needed a wrapper `div` for `data-nofocus`. Pins: `rinch-macros/tests/rsx_component_attrs.rs`, `rinch/src/app/component_attrs_433_tests.rs`.
  - **Quoted names** (issue #1653) — `"title": "Save"`, `"id"`, `"role"`, `"lang"`, `"dir"`, `"data-2x"` — take the same root path, for the same reason: a string literal is not an identifier, so it cannot be a field. `is_root_attribute` is `prop.quoted || prop.name.contains('-')`, and it is checked **first** in both component prop loops, so a quoted name is the attribute even when it spells a style shorthand. A bare `title` could not be routed instead: `Alert`, `Modal`, `Drawer`, `Notification`, `FloatingPanel` and `BorderlessWindow` declare `title` as a heading, and the macro has no type information to tell a heading from a tooltip — the quote is the spelling that tells them apart. Parsing (`element.rs`, `prop.rs`): a `LitStr` followed by a single `:` is a prop (`RsxProp::quoted`, `first_ident: None`, so `name_as_ident` panics for it as for a hyphenated name); a text child is never followed by `:`, so nothing that parsed before changes meaning. `prop.rs::check_quoted_name` refuses, at the literal's span: a name outside the ASCII subset of XML `Name` (letter/`_`/`:` then alphanumerics and `-_.:`, which every browser's `setAttribute` accepts); `style`, `class`, `key`, `ref` (ASCII case-folded), which `rsx!` gives its own meaning and a quote would silently drop — merging, the `for` key, the node ref; and any `on…` name, an event handler content attribute, which is script (and on the web backend a `setAttribute("onclick", …)` would be live). On an **HTML element** (`html.rs`) a quoted prop goes straight to `attr_props`: never an event, a style shorthand (`"p"` is an attribute named `p`) or the #1591 `_fn` error. `ActionIcon` and `CloseButton` also declare a bare `title: String`, written on their root `<button>` when non-empty; no `title_fn` — no label-like prop in the library has one, and `"title": {|| …}` is the surgical spelling. Pins: `rinch-macros/tests/rsx_quoted_attrs_1653.rs` (plus the parser tests in `element.rs`), `rinch/src/app/title_tooltip_tests.rs::component_titles_1653`.

```rust
// style: and class: work on all components
Button {
    variant: "filled",
    style: "margin-top: 8px",           // Static style
    class: "my-custom-button",          // Merged with component classes
    onclick: || do_something(),
    "Click me"
}

// Reactive closures also work on component style/class
Text {
    color: "dimmed",
    style: {|| if highlighted.get() { "background: yellow" } else { "" }},
    class: {|| if active.get() { "active" } else { "" }},
    "Dynamic styling"
}
```

**All component props support reactive closures**, `icon` / `*_icon` included (issue #718 — the icon arm in `generate_component_field_assignments` used to wrap the closure itself in `Some(expr)`, ahead of the arm that would have invoked it, so a reactive icon was a **compile error** naming `TablerIcon`; it now invokes a closure before wrapping, exactly as every other reactive prop does). Pass `{|| expr}` to any component prop (`variant`, `color`, `size`, `disabled`, `icon`, etc.) to make it reactive — when signals change, the component re-renders automatically:

```rust
let active = Signal::new(false);
Button {
    variant: {|| if active.get() { "filled" } else { "light" }},
    onclick: move || active.update(|v| *v = !*v),
    "Toggle"
}
```

For surgical updates without full component re-render, use `_fn` props where available (e.g., `checked_fn`, `value_fn`).

| Prop Type | Accepts `{|| ...}` | Update Strategy |
|---|---|---|
| HTML element attributes | Yes | Surgical DOM update |
| Component `style:`/`class:`, style shorthands, hyphenated and quoted attributes | Yes | Surgical DOM update |
| Component `_fn` props | Yes (auto-wrapped) | Surgical DOM update |
| Component props (all others) | Yes | Full component re-render |

**A component's root bindings stay surgical when the component re-renders** (issue #1190). On a component that re-renders for a reactive struct prop (`reactive_component_dom`, whose render closure runs tracked), the caller's reactive `style:`, `class:`, shorthands and hyphenated attributes are each an effect created in the render's child scope (`component_codegen::RootBindings`), not reads of the render closure. So a change to the signal a `style: {|| …}` reads updates the root in place and keeps the component's local state; a struct-prop change still re-renders, disposes those effects with the old render's scope, and builds fresh ones on the new root, with a fresh `style:`/`class:` memory. They used to be invoked inside the tracked closure, which rebuilt the whole component on every binding change. The whole site is **one** `move` closure, a site bundle (`rinch_core::dom::SiteFn`, `#[doc(hidden)]`), that renders on `SiteCall::Render` and evaluates binding `N` on `SiteCall::Binding(N)`; the render closure and every binding effect hold it by `Rc`. So everything the caller's tokens name is captured exactly once, as when the bindings ran inside the render closure: no new `Clone` requirement (a non-`Clone` value, or a `String` named by a struct prop and a binding, inside `format!` or not, compiles as before), and state a binding keeps in a captured `Cell` lives as long as the site, not one render. A `move` closure capturing a non-`Copy` value on `style:`/`class:` there, E0507 before, compiles. And since the class memory now works on this path, a caller `class:` that names the component's own base class and then drops it strips that class here too, as it always did on the static path (the `class:` bullet above). Pins: `rinch-macros/tests/rsx_rerender_caller_bindings.rs`.
