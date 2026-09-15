# Component Props Reference

This page lists every prop for every component in `rinch-components`. All components use `#[derive(Default)]` unless noted, meaning `Option<T>` defaults to `None` and `bool` defaults to `false`.

**Keeping this page honest:** this file is hand-maintained (prose, worked examples, and "Custom Default" callouts don't survive mechanical regeneration), so it *will* drift from the component structs again. Before trusting or editing a prop table, run `python3 scripts/audit_component_props_doc.py` from the repo root — it cross-references every row here against the real `pub struct` fields in `crates/rinch-components/src/*.rs` and prints every type mismatch, missing row, and stray row it finds (see the script's own docstring for the handful of known non-issues it also reports, e.g. `Callback` vs a fully-qualified `rinch_core::Callback` spelling that source uses in a few files — same type, not a bug).

**String props:** All text/string component props are now `String` type (not `Option<String>`). Empty string `""` means "not set/use default". The RSX macro auto-converts string literals: `variant: "filled"` becomes `String::from("filled")`.

**Float literals:** Float literals are auto-wrapped: `value: 30.0` becomes `Some(30.0)` for `Option<f32>` fields.

**Universal props:** All components support `style:` and `class:` in RSX, which are applied to the component's root DOM element. These support reactive closures `{|| expr}`.

Both **merge** with what the component itself put on that root rather than replacing it: `class:` is added to the component's own classes, and `style:` is laid over the component's own inline declarations, last-wins per property. That matters because a component's inline style is not decoration — it is how a component publishes a prop to its own stylesheet, and every overlay's `z_index` is a custom property written there. `style:` used to write the whole attribute, which erased all of them ([issue #647](https://github.com/joeleaver/rinch/issues/647)).

A reactive `style:` takes its *own* previous declarations back on each re-run, so it never accumulates: a property the closure has stopped naming is removed, or restored to the value it had been overriding. It overwrites a property it still names where that property stands rather than re-appending it, and it leaves alone one whose current value somebody else has since written. The one case it does not settle is a same-property collision with a style shorthand — see [Application Order](./rsx-syntax.md#application-order).

A reactive `class:` has the same memory: it takes off the words it last wrote before adding the new ones, so it never accumulates its own old values. It does not reach a class it has **never named** — but naming one is enough to own it. A word the closure emits is the closure's to remove, so a re-run that stops naming it takes it off and nothing puts it back, and that holds for a class the component itself wrote, its **base class** included: `class: {|| if p.get() { "rinch-checkbox marker" } else { "marker" }}` strips `rinch-checkbox` off the element permanently. That is [issue #647](https://github.com/joeleaver/rinch/issues/647)'s mechanism and the rule the reactive `style:` follows too — don't name a class the component owns.

**The merge only holds if the component keeps its side of it.** A `class:` prop is applied to the handle `Component::render` *returned*, which is after the component has built its tree and registered its effects. A component effect that then rewrote the whole `class` attribute — from a base string captured during `render` — therefore discarded the caller's class on its first run, and there was no way to get it back short of a re-render. `Checkbox { class: "compact", checked_fn: … }` styled correctly until the user clicked it once ([issue #717](https://github.com/joeleaver/rinch/issues/717)). So **a component effect adds and removes the one class it is responsible for** (`NodeHandle::add_class` / `remove_class`) and never writes `class` wholesale. `add_class` is idempotent, as `DOMTokenList.add` is, so an effect that re-runs without changing its answer leaves the attribute alone.

**Style shorthands:** All elements and components support CSS shorthand props like `w`, `h`, `m`, `p`, `maw`, etc. These expand to `set_style()` calls and compose with component styles. Spacing scale values (`xs`, `sm`, `md`, `lg`, `xl`) auto-resolve to `var(--rinch-spacing-{value})`:

```rust
// Shorthands work on both HTML elements and components
div { p: "md", m: "lg", w: "200px", "Styled div" }
Stack { gap: "md", p: "xl", maw: "600px",
    Text { "Content" }
}
```

See [Style Shorthands](./rsx-syntax.md#style-shorthands) for the full list.

**Prop auto-wrapping:** The `rsx!` macro automatically wraps prop values. See [RSX Prop Transformation Rules](./rsx-syntax.md#prop-transformation-rules) — do NOT manually wrap in `Some(...)`.

**All props are reactive:** Every component prop accepts a reactive closure `{|| expr}` in addition to a static value. When any prop uses a closure, the component automatically re-renders when the signals inside change:

```rust
let active = Signal::new(false);

// Static prop value
Button { variant: "filled", "Always filled" }

// Reactive prop value — re-renders when `active` changes
Button { variant: {|| if active.get() { "filled" } else { "outline" }}, "Toggle" }
```

For more efficient surgical updates (no full re-render), use `_fn` suffix props where available (e.g., `value_fn`, `checked_fn`, `opened_fn`).

A `value_fn` write to the text field that currently holds focus is adopted by the field (issue #238): the next keystroke edits the written text, the caret and selection keep their logical position through the rewrite, the write is deferred during an IME composition, and it never commits `onchange` by itself. See [Controlled Input Pattern](components.md#controlled-input-pattern).

---

## Layout

### Stack

Vertical flex container.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `gap` | `String` | `""` | Spacing between children (xs, sm, md, lg, xl or CSS value) |
| `align` | `String` | `""` | CSS `align-items` (e.g., "center", "flex-start") |
| `justify` | `String` | `""` | CSS `justify-content` |

### Group

Horizontal flex container.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `gap` | `String` | `""` | Spacing between children (xs, sm, md, lg, xl or CSS value) |
| `align` | `String` | `""` | CSS `align-items` |
| `justify` | `String` | `""` | CSS `justify-content` |
| `wrap` | `bool` | `false` | Enable flex-wrap |
| `grow` | `bool` | `false` | Children flex-grow: 1 |

### SimpleGrid

Auto-layout grid.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `cols` | `Option<u32>` | `None` | Number of columns (default 1) |
| `min_child_width` | `String` | `""` | Min column width for auto-fill; overrides `cols` |
| `spacing` | `String` | `""` | Gap between items (xs, sm, md, lg, xl or CSS value) |
| `vertical_spacing` | `String` | `""` | Vertical gap (xs, sm, md, lg, xl or CSS value); falls back to `spacing` |

### Container

Centered max-width wrapper.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `String` | `""` | Max-width (xs, sm, md, lg, xl) |
| `fluid` | `bool` | `false` | Full width (no max-width) |

### Center

Centers content horizontally and vertically.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `inline` | `bool` | `false` | Use inline-flex instead of flex |

### Space

Empty spacing element.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `w` | `String` | `""` | Width (spacing scale or CSS value) |
| `h` | `String` | `""` | Height (spacing scale or CSS value) |

---

## Buttons

### Button

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `variant` | `String` | `""` | "filled", "outline", "light", "subtle", "transparent", "white", "default", "gradient" |
| `size` | `String` | `""` | xs, sm, md, lg, xl |
| `color` | `String` | `""` | Theme color name |
| `disabled` | `bool` | `false` | |
| `loading` | `bool` | `false` | |
| `full_width` | `bool` | `false` | |
| `radius` | `String` | `""` | Border radius override |
| `onclick` | `Option<Callback>` | `None` | Click handler |

### ActionIcon

Icon-only button. For text-based action buttons, use `Button` with compact styling.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `icon` | `Option<TablerIcon>` | `None` | Tabler icon to display |
| `variant` | `String` | `""` | Same variants as Button |
| `size` | `String` | `""` | xs, sm, md, lg, xl |
| `color` | `String` | `""` | Theme color name |
| `radius` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `loading` | `bool` | `false` | |
| `onclick` | `Option<Callback>` | `None` | Click handler |

### CloseButton

Draws a stroked X, sized by `icon_size` or by `size`. Until issue #474 it drew
nothing at all: the glyph was an empty `<span class="rinch-icon rinch-icon--close">`
and no stylesheet in the workspace matches that class.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `String` | `""` | xs, sm, md, lg, xl |
| `radius` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `icon_size` | `Option<u32>` | `None` | The X's edge length in px. Unset, it follows `size`: 12 / 14 / 16 / 20 / 24 for xs…xl |
| `onclick` | `Option<Callback>` | `None` | Click handler |

---

## Form Inputs

### TextInput

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `placeholder` | `String` | `""` | |
| `description` | `String` | `""` | Help text below input |
| `error` | `String` | `""` | Error message; shows error styling |
| `size` | `String` | `""` | xs, sm, md, lg, xl |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `radius` | `String` | `""` | One of `xs sm md lg xl`, rounding the field. Empty keeps the theme's default radius |
| `input_type` | `String` | `""` | HTML input type ("text", "email", etc.) |
| `value` | `String` | `""` | Static value |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` |
| `onchange` | `Option<InputCallback>` | `None` | Commit boundary (#226): fires once with the final value when the gesture ends (blur after a modification, or Enter); only if the value changed since focus |
| `onsubmit` | `Option<Callback>` | `None` | Fires on Enter key |

### Textarea

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `error` | `String` | `""` | |
| `placeholder` | `String` | `""` | |
| `size` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `autosize` | `bool` | `false` | Auto-resize textarea |
| `min_rows` | `Option<u32>` | `None` | Visible rows; sizes the control to that many lines. Defaults to 2 (HTML default) when unset. A larger CSS `min-height` wins |
| `max_rows` | `Option<u32>` | `None` | **Declared and inert on desktop (#715).** A textarea has no content height, so its used height is exactly the `min-height` its `rows` and the sheet's 60–120px floor give it — and `min-height` beats `max-height`, so no cap can bind at any value. On the allowlist in `no_dead_props.rs` until a textarea's height can follow its content |
| `value` | `String` | `""` | |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` |
| `onchange` | `Option<InputCallback>` | `None` | Commit boundary (#226): fires once with the final value when the gesture ends (blur after a modification); only if the value changed since focus |

### PasswordInput

Custom Default: `toggle_visibility` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `error` | `String` | `""` | |
| `placeholder` | `String` | `""` | |
| `value` | `String` | `""` | |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `visible` | `bool` | `false` | Password visibility state |
| `visible_fn` | `Option<ReactiveBool>` | `None` | Reactive visibility (auto-wrapped) |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `autofocus` | `bool` | `false` | |
| `size` | `String` | `""` | |
| `radius` | `String` | `""` | One of `xs sm md lg xl`, rounding the field. The visibility toggle inside keeps its own radius |
| `toggle_visibility` | `bool` | **`true`** | Show/hide the eye toggle button |
| `ontoggle` | `Option<Callback>` | `None` | Fires when visibility toggled |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` |
| `onchange` | `Option<InputCallback>` | `None` | Commit boundary (#226): fires once with the final value when the gesture ends (blur after a modification, or Enter); only if the value changed since focus |

### NumberInput

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `error` | `String` | `""` | |
| `placeholder` | `String` | `""` | |
| `value` | `Option<f64>` | `None` | Initial value; uncontrolled, the steppers move the field from here |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped); when set, it is the field's single write path and the steppers are callback-only |
| `default_value` | `Option<f64>` | `None` | Initial value of an uncontrolled field when `value` is absent |
| `min` | `Option<f64>` | `None` | |
| `max` | `Option<f64>` | `None` | |
| `step` | `Option<f64>` | `None` | |
| `decimal_scale` | `Option<u32>` | `None` | Number of decimal places |
| `prefix` | `String` | `""` | e.g., "$" |
| `suffix` | `String` | `""` | e.g., "kg" |
| `disabled` | `bool` | `false` | |
| `hide_controls` | `bool` | `false` | Hide +/- buttons |
| `required` | `bool` | `false` | |
| `size` | `String` | `""` | |
| `radius` | `String` | `""` | One of `xs sm md lg xl`. Rounds the field, a prefix/suffix and the steppers together |
| `onincrement` | `Option<Callback>` | `None` | Notification; uncontrolled, the component steps the field itself and reports through `oninput` (#501) |
| `ondecrement` | `Option<Callback>` | `None` | Notification (see `onincrement`) |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` from direct text entry, and — uncontrolled — each stepper-written value |
| `onchange` | `Option<InputCallback>` | `None` | Commit boundary (#226): fires once with the final value when the gesture ends (blur after a modification, or Enter); only if the value changed since focus |

### Checkbox

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | Secondary line under the label (#707). Puts both in a `.rinch-checkbox__body` column and aligns the box to the first line |
| `size` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `checked` | `bool` | `false` | Static checked state |
| `checked_fn` | `Option<ReactiveBool>` | `None` | Reactive checked binding (auto-wrapped) |
| `indeterminate` | `bool` | `false` | |
| `onchange` | `Option<Callback>` | `None` | |

### Switch

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | Secondary line under the label (#707). Puts both in a `.rinch-switch__body` column and aligns the track to the first line |
| `size` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `checked` | `bool` | `false` | |
| `checked_fn` | `Option<ReactiveBool>` | `None` | Reactive checked binding (auto-wrapped) |
| `label_position` | `String` | `""` | `"start"` puts the label before the track; anything else leaves it after |
| `onchange` | `Option<Callback>` | `None` | |

### Select

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `error` | `String` | `""` | |
| `placeholder` | `String` | `""` | |
| `size` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `value` | `String` | `""` | |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `onchange` | `Option<InputCallback>` | `None` | Receives selected value as `String` |

Options are passed as children: `option { value: "us", "United States" }`

The trigger is a Tab stop (`tabindex="0"`, `role="combobox"`,
`aria-haspopup="listbox"`, and an `aria-expanded` that tracks the open state).
Enter and Space on it toggle the dropdown. Arrow/Enter/Escape navigation of the
**open** option list is issue #434.

### Radio / RadioGroup

**Radio:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `name` | `String` | `""` | Radio group name |
| `value` | `String` | `""` | Radio value |
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `checked` | `bool` | `false` | |
| `checked_fn` | `Option<ReactiveBool>` | `None` | Reactive checked binding (auto-wrapped) |
| `disabled` | `bool` | `false` | |
| `size` | `String` | `""` | |
| `color` | `String` | `""` | |
| `error` | `bool` | `false` | |
| `onchange` | `Option<Callback>` | `None` | |

**RadioGroup:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `error` | `String` | `""` | |
| `size` | `String` | `""` | Default size for this group's radios (#707); a `Radio`'s own `size` wins, and a radio records its ask as `data-size` so an explicit `"md"` is not mistaken for an unset one. A radio that arrives later, or moves in from another group, is sized as it lands (#716) |
| `orientation` | `String` | `""` | "horizontal" or "vertical" |

### Slider

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `min` | `Option<f64>` | `None` | |
| `max` | `Option<f64>` | `None` | |
| `value` | `Option<f64>` | `None` | Static value |
| `value_signal` | `Option<Signal<f64>>` | `None` | Direct signal binding |
| `step` | `Option<f64>` | `None` | |
| `size` | `String` | `""` | |
| `color` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `label` | `String` | `""` | Tooltip label format |
| `show_label_on_hover` | `bool` | `false` | |
| `label_always_on` | `bool` | `false` | |
| `onchange` | `Option<ValueCallback<f64>>` | `None` | Receives `f64` |

---

## Color

### ColorSwatch

A colored square with checkerboard transparency indication.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `color` | `String` | `""` | CSS color value |
| `size` | `String` | `"28px"` | Width and height |
| `radius` | `String` | `"sm"` | Border radius (xs, sm, md, lg, xl or CSS value) |
| `with_shadow` | `bool` | `false` | Show box shadow |
| `onclick` | `Option<Callback>` | `None` | Click handler |

### ColorPicker

Interactive color picker with saturation panel, hue/alpha sliders, hex input, and swatches.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `format` | `String` | `"hex"` | Output format: hex, hexa, rgb, rgba, hsl, hsla |
| `value` | `String` | `""` | Initial color (any parseable CSS color) |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive external value binding |
| `onchange` | `Option<InputCallback>` | `None` | Fires formatted color string on change |
| `alpha` | `bool` | `false` | Show alpha slider |
| `swatches` | `Vec<String>` | `[]` | Preset swatch colors |
| `swatches_per_row` | `Option<usize>` | `7` | Swatches per row |
| `size` | `String` | `"md"` | Size: sm, md, lg, xl |
| `with_input` | `bool` | `false` | Show hex text input |

**`onchange` reports author edits only.** A value arriving through `value_fn` — a
peer's edit, a programmatic set — is adopted silently: the picker applies all
four of its internal components (hue, saturation, value, alpha) as one batched
transition and fires nothing, because the caller already has that value. Only a
drag, a typed hex or a swatch click reports — once per act, with the completed
colour, never a partially-applied mixture. This is what lets a consumer bind
`value_fn` to the same store `onchange` writes without the two chasing each
other.

**The picker's state survives its own round trip.** An echoing binding — a store
that `value_fn` reads and `onchange` writes back into — no longer degrades the
picker's hue, saturation, or alpha. Emitted colour strings quantize — to 8-bit
RGB under the hex and rgb formats, to whole degrees and percents under hsl — so
the echo of the picker's own emission routinely parses to a slightly different
hue/saturation than the internal state it was formatted from (and to *no* hue
at grey, or no alpha under an alpha-less format like `hex`); the picker
recognises such an echo as its own state and leaves the internal signals
untouched, so dragging saturation down to grey and back resumes the original
hue, and the alpha slider works under the default `hex` format. A genuinely
foreign value (one that denotes a different colour, alpha included) still
applies — and even then, channels the value cannot carry are kept rather than
fabricated: a grey keeps the current hue (unless it *states* one, as any
`hsl()` value does — `hsl(240, 0%, 50%)`, the sub-percent
`hsl(205, 0.3%, 49%)`, and `hsl(0, 0%, 50%)` alike — a stated hue is adopted;
note that a store which re-spells an RGB grey as `hsl()` writes hue 0, and the
picker adopts it), a black keeps hue and saturation, judged by what the value
renders at 8-bit rather than by exact parse floats. One corner is deliberately
conceded: under an alpha-dropping display format, an inbound value that
restates the picker's current RGB with an explicitly opaque alpha
(`rgba(r, g, b, 1)`) is indistinguishable from a normalizing store's echo of
the emission and does not apply — bind an alpha-carrying format (`hexa`,
`rgba`, `hsla`) when external writes need to drive alpha. More generally, the
echo test judges identity at the resolution of the picker's own emission in
the notation the inbound value is written in — 8-bit channels for a hex, named
or `rgb()` value, whole degrees and whole percents for an `hsl()` value, alpha
at two decimals under `rgb()`/`hsl()` — never finer than that notation's
serializer writes, whatever finer channels the parser accepts. A difference the
emission could not have spelled folds as an echo, in both directions: on an
`hsl`-format wire a stated-hue move of a whole degree applies even at very low
chroma, where it does not move the rendered colour by an 8-bit step, while an
inbound `hsl()` value that shares a spelling with the picker's current colour
folds even where 8-bit could tell them apart (a normalizing store re-spelling
the picker's own hex emission as `hsl()` is indistinguishable from it). The
text field's write-back guard judges at the same resolution, so field, thumbs
and store stay in step.

**Accepted colour notations** (everywhere a colour string is read — `value`,
`value_fn`, typed text, swatches): hex in 3, 4, 6, or 8 digits (`#rgb`,
`#rgba`, `#rrggbb`, `#rrggbbaa`, with or without the `#`), `rgb()`/`rgba()`
and `hsl()`/`hsla()` in both the legacy comma syntax and the modern
space-separated syntax (`rgb(51 51 102 / 0.5)`; alpha as a number or a
percentage), and the CSS named colours (`red`, `rebeccapurple`,
`transparent`). Named colours and function names are case-insensitive, as
in CSS. Out-of-range channels clamp to their CSS ranges; hue wraps (#243).

**The text field is the author's while they type.** The hex field parses on
every keystroke, and a valid *prefix* of the colour being typed (`#336` on the
way to `#3366cc`) is already a parseable colour — it updates the preview and
reports through `onchange` live, but the picker never rewrites the field under
the author's caret while its text denotes the colour the picker already holds.
The field text is normalized to the output format when the colour actually
moves away from it (a drag, a swatch click, an external `value_fn` change) —
and, since issue #226, when the typed gesture *commits* (focus leaves the
field, or Enter): a committed shorthand like `336` normalizes to `#333366`,
and committed text that parses as no colour reverts to the colour the picker
holds, so an attribute-reading consumer never sees a stale shorthand outlive
the gesture that typed it. `ColorInput`'s text field follows the same
contract.

### ColorInput

Text input with inline color preview and dropdown ColorPicker.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | Input label |
| `description` | `String` | `""` | Description text below the input |
| `error` | `String` | `""` | Error message (shows error styling) |
| `placeholder` | `String` | `""` | Placeholder text |
| `size` | `String` | `""` | Field size: `xs`, `sm`, `md`, `lg`, `xl`. Unset or unrecognised means `md`. Scales the field's height, padding and font size, and the preview swatch alongside them |
| `radius` | `String` | `""` | Field border radius: `xs`, `sm`, `md`, `lg`, `xl`. Unset or unrecognised leaves the default `sm` |
| `disabled` | `bool` | `false` | Disable the input |
| `value` | `String` | `""` | Current color value |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding |
| `onchange` | `Option<InputCallback>` | `None` | Fires the formatted color string for every change made in the dropdown picker — a swatch pick, and each frame of a panel or slider drag — and for a typed edit once, at its commit boundary: blur or Enter (#226). Typing previews live in the swatch and the picker but reports only on commit |
| `format` | `String` | `"hex"` | Output format |
| `alpha` | `bool` | `false` | Show alpha slider in picker |
| `swatches` | `Vec<String>` | `[]` | Preset swatch colors |
| `swatches_per_row` | `Option<usize>` | `7` | Swatches per row |
| `disallow_input` | `bool` | `false` | Disallow typing (picker only) |

`size` and `radius` were declared but unread until #263. **`size` always
resolves to a step, `radius` does not**: an unrecognised `size` falls back to
`md` — matching `TextInput`, whose `size` parses with `unwrap_or_default()` —
and `md` reproduces the geometry the field was hard-coded to before, so nothing
resizes by upgrading. An unrecognised `radius` emits no class at all and leaves
the base `--rinch-radius-sm` standing, which is what `DropdownMenu`, `Modal` and
`Card` do.

**The dropdown is dismissed by clicking the field again.** `ColorInput` has no
click-outside dismissal — it mounts no backdrop and registers no outside-click
handler. A `close_on_click_outside` prop was declared alongside `size` and
`radius` and read by nothing; it was removed in #263 rather than wired, because
unlike those two there was no behaviour behind it to connect, and adding one is
a new interaction rather than a repair.

**The dropdown picker is bound to the input's current colour** (#237). Typed
text previews in it live (a parseable keystroke moves its panel and thumbs,
at the typed notation's own grid — a typed `hsl(205, 3%, 49%)` lands the hue
thumb on 205° whatever the display `format`, and a typed `#228be680` moves
the alpha thumb), an external `value_fn` change moves it, and a slider nudge
derives from the colour the input currently holds — never from the colour it
mounted with. An external arrival is silent all the way through: the picker
applies it without reporting, and the input's `onchange` fires nothing the
caller did not author.

**The field shows the colour in the `format` output spelling.** A `value` or
`value_fn` written in another notation (`red`, `hsl(200, 3%, 49%)`) is
displayed re-spelled (`#ff0000`, `#797e81` under `hex`); the field is
rewritten only when the colour moves away from its text, so the author's
mid-typing text is left alone as before, and an unparseable commit reverts
it to the last committed colour in that same spelling. Under an
alpha-dropping `format` (`hex`, `rgb`, `hsl`) an alpha typed into the field
previews in the picker but is dropped at the commit boundary, and an alpha
arriving through `value_fn` is only half-honoured: the dropdown picker adopts
it (its alpha thumb moves when `alpha` is on) and the swatch renders it
translucent, but the field cannot spell it and every change the input
reports drops it. The corner ColorPicker concedes above applies here too: a
later `value_fn` value restating the colour opaque (`#ff0000`,
`rgba(255, 0, 0, 1)`) is indistinguishable from the input's own echo and does
not apply, so the picker keeps the alpha — and the swatch snaps opaque on the
next reported change while the picker's alpha thumb does not — until a value
carrying a different, non-opaque alpha arrives. Bind `hexa`/`rgba`/`hsla`
when alpha must be externally drivable.

---

## Typography

### Text

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `String` | `""` | xs, sm, md, lg, xl |
| `weight` | `String` | `""` | CSS font-weight |
| `color` | `String` | `""` | Theme color or "dimmed" |
| `align` | `String` | `""` | CSS text-align |
| `inline` | `bool` | `false` | Use `<span>` instead of `<p>` |

### Title

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `order` | `Option<u8>` | `None` | Heading level 1-6 |
| `align` | `String` | `""` | CSS text-align |
| `size` | `String` | `""` | Override size independent of order |

### Code

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `block` | `bool` | `false` | Block display (`<pre>`) vs inline (`<code>`) |
| `color` | `String` | `""` | |

### Kbd

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `String` | `""` | xs, sm, md, lg, xl |

### Anchor

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `href` | `String` | `""` | |
| `target` | `String` | `""` | e.g., "_blank" |
| `size` | `String` | `""` | |
| `underline` | `bool` | `false` | |

### Blockquote

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `cite` | `String` | `""` | Citation source |
| `icon` | `Option<TablerIcon>` | `None` | |
| `color` | `String` | `""` | |
| `radius` | `String` | `""` | |

### Mark

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `color` | `String` | `""` | Highlight background color |

### Highlight

Custom Default: `ignore_case` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `text` | `String` | `""` | Full text to display |
| `highlight` | `String` | `""` | Substring(s) to highlight |
| `color` | `String` | `""` | Highlight color |
| `ignore_case` | `bool` | **`true`** | Case-insensitive matching |

---

## Data Display

### Avatar

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `src` | `String` | `""` | Image URL |
| `alt` | `String` | `""` | |
| `name` | `String` | `""` | For initials fallback |
| `size` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `color` | `String` | `""` | |
| `variant` | `String` | `""` | "filled", "light", "outline" |

**AvatarGroup:** `spacing: String` — overlap spacing.

### Badge

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `variant` | `String` | `""` | "filled", "light", "outline", "dot", "transparent", "white", "default", "gradient" |
| `size` | `String` | `""` | |
| `color` | `String` | `""` | |
| `radius` | `String` | `""` | One of `xs sm md lg xl`. A badge is a pill by default, so this squares it off |
| `full_width` | `bool` | `false` | |

### Card

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `shadow` | `String` | `""` | xs, sm, md, lg, xl |
| `padding` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `with_border` | `bool` | `false` | |

**CardSection:** `inherit_padding: bool`, `with_border: bool`.

### Paper

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `shadow` | `String` | `""` | xs, sm, md, lg, xl |
| `p` | `String` | `""` | Padding (spacing scale) |
| `radius` | `String` | `""` | |
| `with_border` | `bool` | `false` | |

### Divider

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `orientation` | `String` | `""` | "horizontal" or "vertical" |
| `size` | `String` | `""` | |
| `label` | `String` | `""` | Text label in the divider |
| `label_position` | `String` | `""` | "left", "center", "right" |

### Fieldset

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `legend` | `String` | `""` | |
| `variant` | `String` | `""` | "default", "filled", "unstyled" |
| `size` | `String` | `""` | |
| `disabled` | `bool` | `false` | |

### Image

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `src` | `String` | `""` | Image URL |
| `alt` | `String` | `""` | |
| `width` | `String` | `""` | CSS width |
| `height` | `String` | `""` | CSS height |
| `fit` | `String` | `""` | CSS object-fit |
| `radius` | `String` | `""` | |
| `fallback_src` | `String` | `""` | Fallback image URL |
| `caption` | `String` | `""` | Caption text below image |

### List / ListItem

**List:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | `String` | `""` | "ordered" or "unordered" |
| `size` | `String` | `""` | |
| `spacing` | `String` | `""` | |
| `center` | `bool` | `false` | Center items with icons |
| `icon` | `Option<TablerIcon>` | `None` | Default icon for this list's items that set none (#707); a `ListItem`'s own `icon` wins, and an item that arrives later, or moves in from another list, takes it as it lands (#716) |
| `with_padding` | `bool` | `false` | |

**ListItem:** `icon: Option<TablerIcon>` — per-item icon override, which beats
`List::icon`.

A `List` renders *after* its items, so its default cannot reach them as a prop:
it finds each item that built no icon layout of its own and rebuilds that item
into the same markup `ListItem` uses, moving the content it already held into
the content span.

That patch runs at the list's own render **and again on whatever lands beneath
the list afterwards** — a `for` reconcile, a `show_dom` branch, a hand-rolled
`append_child` (issue #716). The second half rides
`rinch_core::dom::on_child_inserted`, a DOM-level hook the four `NodeHandle`
insertion verbs fire: the container registers against its own root and is told,
synchronously, each time a subtree arrives. Synchronously matters — the default
is in place before the frame that shows the new item is laid out, so nothing
flashes.

The same applies to every container default in the library: `RadioGroup::size`
and the four `Stepper` props, `active` included. An item **moved** from one
container into another re-resolves against the one it now belongs to, because a
container marks the values it supplied as its own (`data-list-icon` on a list
item) and will not touch a value the child asked for itself.

The rule a container applies to a *nested* container is the same one its
render-time walk applies downwards, read upwards: every registered ancestor is
told about an insertion, and each declines a node whose chain up to it crosses
one of its own items. So a `List` inside another list's item owns its own rows.

`Stepper` is the one that has to hand something *back* down. A step's state
decides which icon it draws, and a step draws its icon before its stepper
exists — so a step with no `state` of its own renders the icons for the states
it might be moved into and leaves them in its icon box, hidden, for the stepper
to promote (issue #709). Promotion re-parents the glyph out of its wrapper
before the wrapper goes, since a `discard` retires the whole subtree (issue
#719).

**A glyph the step's props supplied is parked, not discarded, when the step
stops drawing it** (issue #716). An insertion moves the steps after it and a
keyed `for` **reorder** moves a step in either direction, so a step this stepper
put into completed can be displaced back out of it — and its `icon` is a
`TablerIcon` in props that no patch of the rendered tree could rebuild, so it
goes back into a hidden wrapper under the content key it serves. A built-in —
the tick, or the step number — is rebuildable from nothing and still leaves by
`discard`. Every parked alternate is kept for as long as the stepper owns the
step's state; a step that named its own `state` is in that state wherever it
moves, so for it they are all dropped.

The markers on a step's icon box are therefore records of the step's **props**
rather than of what it last drew, which is what lets the pass re-run:
`data-icon-has` names the content keys the box holds a real glyph for,
`data-icon-live` names the key showing, and `data-step-derived` on the step says
this stepper wrote its `data-step` rather than the caller.

---

## Feedback

### Alert

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `color` | `String` | `""` | |
| `variant` | `String` | `""` | "filled", "light", "outline", "transparent", "white", "default" |
| `title` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `with_close_button` | `bool` | `false` | |
| `icon` | `Option<TablerIcon>` | `None` | |
| `onclose` | `Option<Callback>` | `None` | |

### Loader

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | `String` | `""` | "oval", "bars", "dots" |
| `size` | `String` | `""` | |
| `color` | `String` | `""` | |

### Progress

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `Option<f32>` | `None` | Percentage 0-100 |
| `value_fn` | `Option<ReactiveF32>` | `None` | Reactive value binding (auto-wrapped) |
| `color` | `String` | `""` | |
| `size` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `striped` | `bool` | `false` | |
| `animated` | `bool` | `false` | |

### Skeleton

Custom Default: `animate` and `visible` default to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `width` | `String` | `""` | |
| `height` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `circle` | `bool` | `false` | |
| `animate` | `bool` | **`true`** | |
| `visible` | `bool` | **`true`** | |

---

## Overlays

**`z_index` on an overlay** (issue #474). `Modal`, `Drawer`, `Popover`,
`DropdownMenu` and `Notification` each paint more than one box, and the boxes
have to stay ordered against each other: a modal's panel above its overlay, a
menu's backdrop below its panel. `z_index` therefore names the level of the
component's *base* layer — the one whose stylesheet number the per-component
table below quotes — and every other layer keeps its existing offset from it.
So `Modal { z_index: 500 }` is overlay 500 / panel 501, and
`DropdownMenu { z_index: 500 }` is panel 500 / backdrop 499; one number moves
the component as a piece. Leaving it unset keeps the stylesheet's own levels.

Every layer that carries a level is a *positioned* box — `fixed` for the
`Modal`/`Drawer` roots, the `DropdownMenu` backdrop and the `Notification`;
`absolute` or `relative` for the panels inside them — so the level orders it
within its nearest ancestor stacking context rather than against the whole
page. See the stacking-context notes in CLAUDE.md if a raised overlay still
sits under something.

One limit worth knowing: the level is published as an inherited custom
property, so an overlay of the **same type** nested inside a raised one inherits
its level instead of falling back to the default — Chromium does exactly the
same with the same CSS, and before this prop existed both were equal anyway.

A component-level `style:` prop is safe to combine with it. It used to replace
the root's whole inline style, which dropped the published level and silently
returned the overlay to its stylesheet default (issue #647, fixed); a `style:`
now merges into the root's own declarations the way `class:` and the style
shorthands always have. See the **Universal props** note at the top of this page.

**Dismissing an overlay** (issue #474). `close_on_escape`, and
`Popover::close_on_click_outside`, and `Notification::auto_close` were declared
and read by nothing until the dismissal PR. All three now work, on desktop and
on the web, and all three are requests rather than actions: they invoke
`onclose`, and **it is the app that closes the overlay** by writing the signal
`opened_fn` reads. An overlay with no `onclose` has nowhere to send the request,
so it registers nothing and leaves the key or the click to the app.

*Escape* goes through the **dismiss stack** — a last-in-first-out list every
open overlay joins while it is mounted. The innermost open overlay answers the
key and the ones beneath it do not, so nesting works; an overlay that is mounted
but **closed** passes the key down rather than swallowing it; and a key nothing
takes reaches the app unchanged. Some things beat it, all pre-existing and all
correct: a document-level
[`set_keyboard_interceptor`](./focus.md#the-document-level-interceptor), which
is the app saying it means to have Escape, and an in-progress drag, which
Escape cancels — both on either backend — plus, on `rinch-web` only, a focused
`RenderSurface`, which swallows every key there. An open native `<select>`
popup takes it too, but by joining this same stack rather than by beating it.
A custom overlay of your own joins the same stack with
[`push_dismiss_handler`](./focus.md#the-dismiss-stack) — reach for that rather
than an interceptor, which is one slot per document and so cannot nest.

*Outside clicks* are caught by an invisible full-viewport backdrop the component
renders while it is open, one stacking level under its own panel, so a click on
the overlay's own content still reaches the content. `Modal` and `Drawer` use
their dimming overlay for this and always have; `Popover` gained one with the
same PR.

*`auto_close`* arms a timer when the notification **opens** and cancels it if
the notification closes — or unmounts — first, so a toast the user dismisses by
hand does not fire `onclose` a second time at its original deadline, and one
that is shown again gets a fresh delay. `0` is off.

*`trap_focus`* contains **Tab** inside the overlay while it is open: the
component stamps `data-trap-focus` on its root, and both backends cycle Tab and
Shift+Tab within that element instead of walking the page. A *click* outside is
untouched — the pointer is not trapped, as a *non-modal* dialog's is not.

It also **moves focus into the overlay when it opens and gives it back when it
closes**, because it is the prop that says the overlay is modal. `Modal` and
`Drawer` focus their first control (or an `autofocus` one); `Popover` moves
focus only for an `autofocus` child, which is the HTML popover API's rule rather
than the dialog's. Closing restores the element that held the keyboard when the
overlay opened, provided that element can still take focus, and unmounting while
open does the same. A claim the user had already moved outside the overlay is
left alone. Details, the nesting rule and the backend timing are in the
[focus guide](focus.md#containing-tab-inside-an-overlay).

*`lock_scroll`* holds the page still while the overlay is open, and this is the
one overlay behaviour that **works differently on the two backends**. Desktop
refuses the *scroll gesture*: a wheel (which is also what a touch scroll and the
MCP `scroll` tool arrive as) and a scrollbar-thumb press are rejected for any
container that is not inside a locking overlay, so the dialog's own
`overflow: auto` body still scrolls while the page behind does not. Nothing is
restyled, so the page does not move and its scroll position is exactly where the
user left it. The web sets `overflow: hidden` on the real `<html>`, because
rinch cannot gate the browser's own wheel — which **also removes the page's
scrollbar**, so a web page with a classic (non-overlay) scrollbar shifts
sideways when an overlay opens, and a desktop one does not. That is the whole of
the divergence.

Two more things follow from "the gesture, not the style" on desktop, and neither
has a web equivalent:

- The page's scrollbar is still **painted** while it is locked. You can see it;
  it will not move. Nothing is drawn differently, only refused.
- Only *input* is gated. Programmatic scrolling — `NodeHandle::set_scroll_top`,
  an app scrolling a list behind the dialog on purpose — still works. Desktop
  has no keyboard page-scrolling (PageDown and the arrows scroll nothing at the
  document level), so there is nothing to gate there.
- A scrollbar drag already **in flight** when the lock arrives is ended, not
  left to keep scrolling the page while the button is held.
- The native `<select>` popup is **exempt**: its option list is appended to
  `<body>`, so it is not inside any overlay's root, and a long list inside a
  dialog would otherwise be unscrollable. See [Focus](./focus.md#locking-the-page-behind-an-overlay).

On the web the lock is the whole page even in island mode — there is one
`<html>` — so a rinch `Modal` inside an island freezes its host page too.

Locks are **counted**: two overlays open and the inner one closing leaves the
page locked, and an overlay that unmounts while still open releases its own.
A custom overlay of your own takes one with
`node.set_scroll_locked(true)` on its root — the node matters, it is the subtree
that stays scrollable — and must release it on close *and* on unmount.

### Tooltip

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | Tooltip text |
| `position` | `String` | `""` | "top", "bottom", "left", "right" |
| `color` | `String` | `""` | |
| `opened` | `bool` | `false` | |
| `disabled` | `bool` | `false` | |
| `with_arrow` | `bool` | `false` | |
| `multiline` | `bool` | `false` | |
| `width` | `String` | `""` | |

### Modal

Custom Default: `with_overlay`, `close_on_click_outside`, `close_on_escape`, `with_close_button`, `lock_scroll`, `trap_focus` all default to `true`.

Positioned with `top: var(--rinch-window-top-inset, 0px)`, so it clears any window chrome rinch draws (the Linux in-app menu bar, the `BorderlessWindow` titlebar) and is flush with the top of a plain window. See [Theming](./theming.md#window-chrome-inset).

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `title` | `String` | `""` | |
| `size` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `with_overlay` | `bool` | **`true`** | |
| `overlay_opacity` | `Option<f32>` | `None` | Backdrop alpha, 0-1 (default 0.75). Applies to the dimming overlay, not the panel |
| `overlay_blur` | `String` | `""` | |
| `centered` | `bool` | `false` | |
| `close_on_click_outside` | `bool` | **`true`** | A click on the dimming overlay invokes `onclose` |
| `close_on_escape` | `bool` | **`true`** | Escape invokes `onclose` while this is the innermost open overlay |
| `with_close_button` | `bool` | **`true`** | |
| `padding` | `String` | `""` | |
| `z_index` | `Option<i32>` | `None` | Stacking level of the whole modal: the full-viewport overlay sits here and the panel one above it (defaults 200 / 201) |
| `lock_scroll` | `bool` | **`true`** | Holds the page still while open. Desktop refuses the gesture, web sets `overflow: hidden` on `<html>` — see [Overlays](#overlays) |
| `trap_focus` | `bool` | **`true`** | Tab cycles inside the overlay while it is open (#474) |
| `onclose` | `Option<Callback>` | `None` | |

### Drawer

Custom Default: `with_overlay`, `close_on_click_outside`, `close_on_escape`, `with_close_button`, `lock_scroll`, `trap_focus` all default to `true`.

Positioned with `top: var(--rinch-window-top-inset, 0px)`, so it clears any window chrome rinch draws (the Linux in-app menu bar, the `BorderlessWindow` titlebar) and is flush with the top of a plain window. See [Theming](./theming.md#window-chrome-inset).

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `title` | `String` | `""` | |
| `position` | `String` | `""` | "left", "right", "top", "bottom" |
| `size` | `String` | `""` | |
| `with_overlay` | `bool` | **`true`** | |
| `overlay_opacity` | `Option<f32>` | `None` | Backdrop alpha, 0-1 (default 0.75). Applies to the dimming overlay, not the panel |
| `close_on_click_outside` | `bool` | **`true`** | A click on the dimming overlay invokes `onclose` |
| `close_on_escape` | `bool` | **`true`** | Escape invokes `onclose` while this is the innermost open overlay |
| `with_close_button` | `bool` | **`true`** | |
| `padding` | `String` | `""` | |
| `z_index` | `Option<i32>` | `None` | Stacking level of the whole drawer: the overlay sits here and the panel one above it (defaults 200 / 201) |
| `lock_scroll` | `bool` | **`true`** | Holds the page still while open. Desktop refuses the gesture, web sets `overflow: hidden` on `<html>` — see [Overlays](#overlays) |
| `trap_focus` | `bool` | **`true`** | Tab cycles inside the overlay while it is open (#474) |
| `onclose` | `Option<Callback>` | `None` | |

### Notification

Custom Default: `with_close_button` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `title` | `String` | `""` | |
| `color` | `String` | `""` | |
| `position` | `String` | `""` | Toast position. The `top-*` variants offset by `--rinch-window-top-inset` so they clear window chrome |
| `radius` | `String` | `""` | |
| `with_close_button` | `bool` | **`true`** | |
| `with_border` | `bool` | `false` | |
| `icon` | `Option<TablerIcon>` | `None` | |
| `auto_close` | `u32` | `0` | Invoke `onclose` this many ms after it opens (0 = disabled). Cancelled if it closes or unmounts first |
| `loading` | `bool` | `false` | |
| `z_index` | `Option<i32>` | `None` | Stacking level of the toast (default 300) |
| `onclose` | `Option<Callback>` | `None` | |

### Popover

Custom Default: `close_on_click_outside` and `close_on_escape` default to `true`.

`Popover` had **no close callback and no reactive open state** until #474, so
neither dismissal prop had anywhere to send a request. Both `onclose` and
`opened_fn` were added then, matching `DropdownMenu`; a `Popover` without
`onclose` still renders, and simply cannot dismiss itself.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `position` | `String` | `""` | |
| `offset` | `Option<i32>` | `None` | |
| `radius` | `String` | `""` | |
| `shadow` | `String` | `""` | |
| `with_arrow` | `bool` | `false` | |
| `arrow_size` | `Option<f32>` | `None` | |
| `arrow_offset` | `Option<f32>` | `None` | |
| `close_on_click_outside` | `bool` | **`true`** | Renders a dismiss backdrop while open; a click on it invokes `onclose` |
| `close_on_escape` | `bool` | **`true`** | Escape invokes `onclose` while this is the innermost open overlay |
| `width` | `String` | `""` | |
| `z_index` | `Option<i32>` | `None` | Stacking level of the dropdown; its click-catching backdrop stays one below (defaults 100 / 99) |
| `trap_focus` | `bool` | `false` | Tab cycles inside the popover (target + dropdown) while it is open (#474) |
| `onclose` | `Option<Callback>` | `None` | Invoked when the popover asks to close |

Sub-components: **PopoverTarget** (no props), **PopoverDropdown** (no props).

### ContextMenu

A right-click context menu. No props on the wrapper — state is managed internally.

Sub-components: **ContextMenuTarget** (no props), **ContextMenuDropdown** (no props).

Use `DropdownMenuItem` and `DropdownMenuDivider` as children of `ContextMenuDropdown`.

### DropdownMenu

Custom Default: `close_on_click_outside` and `close_on_item_click` default to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `position` | `String` | `""` | |
| `offset` | `Option<i32>` | `None` | |
| `radius` | `String` | `""` | |
| `shadow` | `String` | `""` | |
| `close_on_click_outside` | `bool` | **`true`** | |
| `close_on_item_click` | `bool` | **`true`** | |
| `width` | `String` | `""` | |
| `z_index` | `Option<i32>` | `None` | Stacking level of the menu panel; its click-catching backdrop stays one below (defaults 100 / 99) |

**DropdownMenuTarget**, **DropdownMenuDropdown**, **DropdownMenuLabel**, **DropdownMenuDivider**: No props.

**DropdownMenuItem:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `left_section` | `Option<TablerIcon>` | `None` | |
| `right_section` | `Option<TablerIcon>` | `None` | |
| `color` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `onclick` | `Option<Callback>` | `None` | |

### HoverCard

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `position` | `String` | `""` | |
| `offset` | `Option<i32>` | `None` | |
| `radius` | `String` | `""` | |
| `shadow` | `String` | `""` | |
| `width` | `String` | `""` | |
| `open_delay` | `Option<u32>` | `None` | |
| `close_delay` | `Option<u32>` | `None` | |
| `with_arrow` | `bool` | `false` | |

Sub-components: **HoverCardTarget** (no props), **HoverCardDropdown** (no props).

### LoadingOverlay

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `visible` | `bool` | `false` | |
| `overlay_opacity` | `Option<f32>` | `None` | |
| `overlay_blur` | `String` | `""` | |
| `loader_type` | `String` | `""` | |
| `loader_size` | `String` | `""` | |
| `loader_color` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `z_index` | `Option<i32>` | `None` | |
| `transition_duration` | `Option<u32>` | `None` | |

---

## Navigation

### Tabs

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `String` | `""` | Active tab value |
| `default_value` | `String` | `""` | |
| `variant` | `String` | `""` | "default", "outline", "pills" |
| `orientation` | `String` | `""` | "horizontal", "vertical" |
| `position` | `String` | `""` | |
| `grow` | `bool` | `false` | |
| `color` | `String` | `""` | |
| `radius` | `String` | `""` | One of `xs sm md lg xl`, rounding the tab buttons. `outline` tabs keep their bottom corners square; the default underline variant has no visible corners to round at rest, though its hover background does |

**TabsList:** `grow: bool`, `justify: String`.

**Tab:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `String` | `""` | Tab identifier |
| `disabled` | `bool` | `false` | |
| `left_section` | `Option<TablerIcon>` | `None` | |
| `right_section` | `Option<TablerIcon>` | `None` | |
| `onclick` | `Option<Callback>` | `None` | |

**TabsPanel:** `value: String` — matches the Tab value.

### Accordion

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `String` | `""` | Active item value |
| `default_value` | `String` | `""` | |
| `variant` | `String` | `""` | "default", "contained", "filled", "separated" |
| `radius` | `String` | `""` | |
| `multiple` | `bool` | `false` | Allow multiple open items |
| `chevron_position` | `String` | `""` | "left", "right" |
| `disable_chevron_rotation` | `bool` | `false` | Holds the chevron still while an item is open (#707), through `data-disable-chevron-rotation` on the root. The rotation is a class rather than an inline style so this can outrank it |

**AccordionItem:** `value: String`.

**AccordionControl:** `disabled: bool`, `icon: Option<TablerIcon>`, `onclick: Option<Callback>`.

**AccordionPanel:** No props.

### Breadcrumbs

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `separator` | `String` | `""` | Custom separator character |
| `separator_margin` | `String` | `""` | Spacing around separator |

**BreadcrumbsItem:** `href: String`.

### Pagination

Custom Default: `total`, `value`, `siblings`, `boundaries` default to `1`; `with_controls` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `total` | `u32` | **`1`** | Total number of pages |
| `value` | `u32` | **`1`** | Current page |
| `siblings` | `u32` | **`1`** | Pages visible on each side |
| `boundaries` | `u32` | **`1`** | Pages at start/end |
| `size` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `with_edges` | `bool` | `false` | Show first/last page buttons |
| `with_controls` | `bool` | **`true`** | Show prev/next buttons |
| `color` | `String` | `""` | |
| `disabled` | `bool` | `false` | |
| `gap` | `String` | `""` | |
| `onchange` | `Option<ValueCallback<u32>>` | `None` | Receives page number |

### NavLink

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `href` | `String` | `""` | |
| `active` | `bool` | `false` | |
| `active_fn` | `Option<ReactiveBool>` | `None` | Reactive active binding (auto-wrapped) |
| `variant` | `String` | `""` | "light", "filled", "subtle" |
| `color` | `String` | `""` | |
| `left_section` | `Option<TablerIcon>` | `None` | |
| `right_section` | `Option<TablerIcon>` | `None` | |
| `disabled` | `bool` | `false` | |
| `children_offset` | `String` | `""` | Indentation for nested NavLinks |
| `opened` | `bool` | `false` | Nested section expanded |
| `default_opened` | `bool` | `false` | |
| `no_wrap` | `bool` | `false` | |
| `onclick` | `Option<Callback>` | `None` | |

### Stepper

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `active` | `u32` | `0` | Active step index. Every step takes its state from its position against this — before it completed, at it in progress, after it inactive (#709) — unless it named a `state` of its own. A step that arrives later is stated as it lands, and so are the steps it displaced (#716); a step *removed* does not renumber its siblings (#745). As a closure (`active: {\|\| sig.get()}`) it re-renders the stepper and re-derives |
| `size` | `String` | `""` | |
| `orientation` | `String` | `""` | "horizontal", "vertical" |
| `color` | `String` | `""` | |
| `radius` | `String` | `""` | |
| `icon_size` | `String` | `""` | |
| `allow_next_steps_select` | `bool` | `false` | Makes each step past `active` clickable (#707), a step that arrives later included (#716). Grants only: a step's own `allow_step_click` / `allow_step_select` is never taken away. **Decorative** — `Stepper` registers no click handler and takes no callback (#737) |
| `completed_icon` | `Option<TablerIcon>` | `None` | Default completed icon for the steps that set none (#707); the step's own wins, and a step that arrives later takes this one as it lands (#716) |
| `progress_icon` | `Option<TablerIcon>` | `None` | Default in-progress icon for the steps that set none (#707). It stands in for the *`progress_icon`* the step did not set, so it outranks that step's plain `icon`. A step that arrives later takes it as it lands (#716) |

**StepperStep:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | |
| `description` | `String` | `""` | |
| `icon` | `Option<TablerIcon>` | `None` | Default icon |
| `completed_icon` | `Option<TablerIcon>` | `None` | Per-step override |
| `progress_icon` | `Option<TablerIcon>` | `None` | Per-step override |
| `allow_step_click` | `bool` | `false` | |
| `allow_step_select` | `bool` | `false` | |
| `loading` | `bool` | `false` | |
| `state` | `String` | `""` | `"completed"` or `"progress"`; anything else is inactive. Leave it unset and the parent `Stepper` derives it from this step's position against `active` (#709). Set it to override that for this one step: what a step names, it keeps |
| `step` | `Option<u32>` | `None` | Step index, 0-based. Set by the parent `Stepper` from this step's position unless named here, in which case that is the index shown — and the number in the icon box, one higher. An index named here changes the label, **not** the position the state derives from |

**StepperCompleted:** No props.

### Tree

Custom Default: `level_offset` defaults to `"md"`, `expand_on_click` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `data` | `Vec<TreeNodeData>` | `[]` | Tree data |
| `tree` | `Option<UseTreeReturn>` | `None` | State from `UseTreeReturn::new()` |
| `level_offset` | `String` | **`"md"`** | Indentation per level |
| `expand_on_click` | `bool` | **`true`** | Click expands/collapses |
| `select_on_click` | `bool` | `false` | Click selects |
| `render_node` | `Option<RenderTreeNode>` | `None` | Custom node renderer |
| `onselect` | `Option<ValueCallback<String>>` | `None` | |
| `onexpand` | `Option<ValueCallback<String>>` | `None` | |
| `oncollapse` | `Option<ValueCallback<String>>` | `None` | |

**TreeNodeData:** `value: String`, `label: String`, `children: Vec<TreeNodeData>`, `disabled: bool`, `icon: Option<TablerIcon>`, `payload: Option<Rc<dyn Any>>`.

---

## Window

### BorderlessWindow

Custom Default: `show_minimize`, `show_maximize`, `show_close` all default to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `title` | `String` | `""` | Window title in titlebar |
| `radius` | `String` | `""` | Corner radius (none, xs, sm, md, lg, xl) |
| `show_minimize` | `bool` | **`true`** | |
| `show_maximize` | `bool` | **`true`** | |
| `show_close` | `bool` | **`true`** | |
| `left_section` | `Option<SectionRenderer>` | `None` | Custom titlebar left content |
| `right_section` | `Option<SectionRenderer>` | `None` | Custom content before controls |
| `on_minimize` | `Option<Callback>` | `None` | |
| `on_maximize` | `Option<Callback>` | `None` | |
| `on_close` | `Option<Callback>` | `None` | |

---

## Callback Types Reference

| Type | Signature | Used By |
|------|-----------|---------|
| `Callback` | `Rc<dyn Fn()>` | onclick, onclose, onsubmit, onchange (toggle) |
| `InputCallback` | `Rc<dyn Fn(String)>` | oninput, onchange (value) |
| `ValueCallback<T>` | `Rc<dyn Fn(T)>` | Slider (`f64`), Pagination (`u32`), Tree (`String`) |
| `ReactiveBool` | `Rc<dyn Fn() -> bool>` | checked_fn, active_fn, opened_fn, visible_fn |
| `ReactiveString` | `Rc<dyn Fn() -> String>` | value_fn |
| `ReactiveF32` | `Rc<dyn Fn() -> f32>` | Progress value_fn |
| `SectionRenderer` | `Rc<dyn Fn(&mut RenderScope) -> NodeHandle>` | BorderlessWindow sections |

All callback/reactive props are auto-wrapped by the `rsx!` macro — just pass closures directly, do not wrap in `Some(...)` or `Rc::new(...)`.
