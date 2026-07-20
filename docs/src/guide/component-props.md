# Component Props Reference

This page lists every prop for every component in `rinch-components`. All components use `#[derive(Default)]` unless noted, meaning `Option<T>` defaults to `None` and `bool` defaults to `false`.

**String props:** All text/string component props are now `String` type (not `Option<String>`). Empty string `""` means "not set/use default". The RSX macro auto-converts string literals: `variant: "filled"` becomes `String::from("filled")`.

**Float literals:** Float literals are auto-wrapped: `value: 30.0` becomes `Some(30.0)` for `Option<f32>` fields.

**Universal props:** All components support `style:` and `class:` in RSX, which are applied to the component's root DOM element. These support reactive closures `{|| expr}`.

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

---

## Layout

### Stack

Vertical flex container.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `gap` | `Option<String>` | `None` | Spacing between children (xs, sm, md, lg, xl or CSS value) |
| `align` | `Option<String>` | `None` | CSS `align-items` (e.g., "center", "flex-start") |
| `justify` | `Option<String>` | `None` | CSS `justify-content` |

### Group

Horizontal flex container.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `gap` | `Option<String>` | `None` | Spacing between children (xs, sm, md, lg, xl or CSS value) |
| `align` | `Option<String>` | `None` | CSS `align-items` |
| `justify` | `Option<String>` | `None` | CSS `justify-content` |
| `wrap` | `bool` | `false` | Enable flex-wrap |
| `grow` | `bool` | `false` | Children flex-grow: 1 |

### SimpleGrid

Auto-layout grid.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `cols` | `Option<u32>` | `None` | Number of columns (default 1) |
| `min_child_width` | `Option<String>` | `None` | Min column width for auto-fill; overrides `cols` |
| `spacing` | `Option<String>` | `None` | Gap between items (xs, sm, md, lg, xl or CSS value) |
| `vertical_spacing` | `Option<String>` | `None` | Vertical gap (xs, sm, md, lg, xl or CSS value); falls back to `spacing` |

### Container

Centered max-width wrapper.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `Option<String>` | `None` | Max-width (xs, sm, md, lg, xl) |
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
| `w` | `Option<String>` | `None` | Width (spacing scale or CSS value) |
| `h` | `Option<String>` | `None` | Height (spacing scale or CSS value) |

---

## Buttons

### Button

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `variant` | `Option<String>` | `None` | "filled", "outline", "light", "subtle", "transparent", "white", "default", "gradient" |
| `size` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `color` | `Option<String>` | `None` | Theme color name |
| `disabled` | `bool` | `false` | |
| `loading` | `bool` | `false` | |
| `full_width` | `bool` | `false` | |
| `radius` | `Option<String>` | `None` | Border radius override |
| `onclick` | `Option<Callback>` | `None` | Click handler |

### ActionIcon

Icon-only button. For text-based action buttons, use `Button` with compact styling.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `icon` | `Option<TablerIcon>` | `None` | Tabler icon to display |
| `variant` | `Option<String>` | `None` | Same variants as Button |
| `size` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `color` | `Option<String>` | `None` | Theme color name |
| `radius` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `loading` | `bool` | `false` | |
| `onclick` | `Option<Callback>` | `None` | Click handler |

### CloseButton

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `radius` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `icon_size` | `Option<u32>` | `None` | Custom icon size in pixels |
| `onclick` | `Option<Callback>` | `None` | Click handler |

---

## Form Inputs

### TextInput

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `placeholder` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | Help text below input |
| `error` | `Option<String>` | `None` | Error message; shows error styling |
| `size` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `radius` | `Option<String>` | `None` | |
| `input_type` | `Option<String>` | `None` | HTML input type ("text", "email", etc.) |
| `value` | `Option<String>` | `None` | Static value |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` |
| `onsubmit` | `Option<Callback>` | `None` | Fires on Enter key |

### Textarea

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `error` | `Option<String>` | `None` | |
| `placeholder` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `autosize` | `bool` | `false` | Auto-resize textarea |
| `min_rows` | `Option<u32>` | `None` | Visible rows; sizes the control to that many lines. Defaults to 2 (HTML default) when unset. A larger CSS `min-height` wins |
| `max_rows` | `Option<u32>` | `None` | Upper bound on rows when `autosize` is set |
| `value` | `Option<String>` | `None` | |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` |

### PasswordInput

Custom Default: `toggle_visibility` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `error` | `Option<String>` | `None` | |
| `placeholder` | `Option<String>` | `None` | |
| `value` | `Option<String>` | `None` | |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `visible` | `bool` | `false` | Password visibility state |
| `visible_fn` | `Option<ReactiveBool>` | `None` | Reactive visibility (auto-wrapped) |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `autofocus` | `bool` | `false` | |
| `size` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `toggle_visibility` | `bool` | **`true`** | Show/hide the eye toggle button |
| `ontoggle` | `Option<Callback>` | `None` | Fires when visibility toggled |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` |

### NumberInput

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `error` | `Option<String>` | `None` | |
| `placeholder` | `Option<String>` | `None` | |
| `value` | `Option<f64>` | `None` | |
| `default_value` | `Option<f64>` | `None` | |
| `min` | `Option<f64>` | `None` | |
| `max` | `Option<f64>` | `None` | |
| `step` | `Option<f64>` | `None` | |
| `decimal_scale` | `Option<u32>` | `None` | Number of decimal places |
| `prefix` | `Option<String>` | `None` | e.g., "$" |
| `suffix` | `Option<String>` | `None` | e.g., "kg" |
| `disabled` | `bool` | `false` | |
| `hide_controls` | `bool` | `false` | Hide +/- buttons |
| `required` | `bool` | `false` | |
| `size` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `onincrement` | `Option<Callback>` | `None` | |
| `ondecrement` | `Option<Callback>` | `None` | |
| `oninput` | `Option<InputCallback>` | `None` | Receives `String` from direct text entry |

### Checkbox

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `checked` | `bool` | `false` | Static checked state |
| `checked_fn` | `Option<ReactiveBool>` | `None` | Reactive checked binding (auto-wrapped) |
| `indeterminate` | `bool` | `false` | |
| `onchange` | `Option<Callback>` | `None` | |

### Switch

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `checked` | `bool` | `false` | |
| `checked_fn` | `Option<ReactiveBool>` | `None` | Reactive checked binding (auto-wrapped) |
| `label_position` | `Option<String>` | `None` | "left" or "right" |
| `onchange` | `Option<Callback>` | `None` | |

### Select

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `error` | `Option<String>` | `None` | |
| `placeholder` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `required` | `bool` | `false` | |
| `value` | `Option<String>` | `None` | |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding (auto-wrapped) |
| `onchange` | `Option<InputCallback>` | `None` | Receives selected value as `String` |

Options are passed as children: `option { value: "us", "United States" }`

### Radio / RadioGroup

**Radio:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `name` | `Option<String>` | `None` | Radio group name |
| `value` | `Option<String>` | `None` | Radio value |
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `checked` | `bool` | `false` | |
| `checked_fn` | `Option<ReactiveBool>` | `None` | Reactive checked binding (auto-wrapped) |
| `disabled` | `bool` | `false` | |
| `size` | `Option<String>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `error` | `bool` | `false` | |
| `onchange` | `Option<Callback>` | `None` | |

**RadioGroup:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `error` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `orientation` | `Option<String>` | `None` | "horizontal" or "vertical" |

### Slider

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `min` | `Option<f64>` | `None` | |
| `max` | `Option<f64>` | `None` | |
| `value` | `Option<f64>` | `None` | Static value |
| `value_signal` | `Option<Signal<f64>>` | `None` | Direct signal binding |
| `step` | `Option<f64>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `label` | `Option<String>` | `None` | Tooltip label format |
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

### ColorInput

Text input with inline color preview and dropdown ColorPicker.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `String` | `""` | Input label |
| `description` | `String` | `""` | Description text below the input |
| `error` | `String` | `""` | Error message (shows error styling) |
| `placeholder` | `String` | `""` | Placeholder text |
| `size` | `String` | `""` | Input size |
| `radius` | `String` | `""` | Border radius |
| `disabled` | `bool` | `false` | Disable the input |
| `value` | `String` | `""` | Current color value |
| `value_fn` | `Option<ReactiveString>` | `None` | Reactive value binding |
| `onchange` | `Option<InputCallback>` | `None` | Fires formatted color string on change |
| `format` | `String` | `"hex"` | Output format |
| `alpha` | `bool` | `false` | Show alpha slider in picker |
| `swatches` | `Vec<String>` | `[]` | Preset swatch colors |
| `swatches_per_row` | `Option<usize>` | `7` | Swatches per row |
| `close_on_click_outside` | `bool` | `false` | Close dropdown on outside click |
| `disallow_input` | `bool` | `false` | Disallow typing (picker only) |

---

## Typography

### Text

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `weight` | `Option<String>` | `None` | CSS font-weight |
| `color` | `Option<String>` | `None` | Theme color or "dimmed" |
| `align` | `Option<String>` | `None` | CSS text-align |
| `inline` | `bool` | `false` | Use `<span>` instead of `<p>` |

### Title

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `order` | `Option<u8>` | `None` | Heading level 1-6 |
| `align` | `Option<String>` | `None` | CSS text-align |
| `size` | `Option<String>` | `None` | Override size independent of order |

### Code

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `block` | `bool` | `false` | Block display (`<pre>`) vs inline (`<code>`) |
| `color` | `Option<String>` | `None` | |

### Kbd

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `size` | `Option<String>` | `None` | xs, sm, md, lg, xl |

### Anchor

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `href` | `Option<String>` | `None` | |
| `target` | `Option<String>` | `None` | e.g., "_blank" |
| `size` | `Option<String>` | `None` | |
| `underline` | `bool` | `false` | |

### Blockquote

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `cite` | `Option<String>` | `None` | Citation source |
| `icon` | `Option<Icon>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |

### Mark

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `color` | `Option<String>` | `None` | Highlight background color |

### Highlight

Custom Default: `ignore_case` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `text` | `Option<String>` | `None` | Full text to display |
| `highlight` | `Option<String>` | `None` | Substring(s) to highlight |
| `color` | `Option<String>` | `None` | Highlight color |
| `ignore_case` | `bool` | **`true`** | Case-insensitive matching |

---

## Data Display

### Avatar

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `src` | `Option<String>` | `None` | Image URL |
| `alt` | `Option<String>` | `None` | |
| `name` | `Option<String>` | `None` | For initials fallback |
| `size` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `variant` | `Option<String>` | `None` | "filled", "light", "outline" |

**AvatarGroup:** `spacing: Option<String>` — overlap spacing.

### Badge

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `variant` | `Option<String>` | `None` | "filled", "light", "outline", "dot", "transparent", "white", "default", "gradient" |
| `size` | `Option<String>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `full_width` | `bool` | `false` | |

### Card

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `shadow` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `padding` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `with_border` | `bool` | `false` | |

**CardSection:** `inherit_padding: bool`, `with_border: bool`.

### Paper

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `shadow` | `Option<String>` | `None` | xs, sm, md, lg, xl |
| `p` | `Option<String>` | `None` | Padding (spacing scale) |
| `radius` | `Option<String>` | `None` | |
| `with_border` | `bool` | `false` | |

### Divider

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `orientation` | `Option<String>` | `None` | "horizontal" or "vertical" |
| `size` | `Option<String>` | `None` | |
| `label` | `Option<String>` | `None` | Text label in the divider |
| `label_position` | `Option<String>` | `None` | "left", "center", "right" |

### Fieldset

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `legend` | `Option<String>` | `None` | |
| `variant` | `Option<String>` | `None` | "default", "filled", "unstyled" |
| `size` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |

### Image

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `src` | `Option<String>` | `None` | Image URL |
| `alt` | `Option<String>` | `None` | |
| `width` | `Option<String>` | `None` | CSS width |
| `height` | `Option<String>` | `None` | CSS height |
| `fit` | `Option<String>` | `None` | CSS object-fit |
| `radius` | `Option<String>` | `None` | |
| `fallback_src` | `Option<String>` | `None` | Fallback image URL |
| `caption` | `Option<String>` | `None` | Caption text below image |

### List / ListItem

**List:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | `Option<String>` | `None` | "ordered" or "unordered" |
| `size` | `Option<String>` | `None` | |
| `spacing` | `Option<String>` | `None` | |
| `center` | `bool` | `false` | Center items with icons |
| `icon` | `Option<Icon>` | `None` | Default icon for all items |
| `with_padding` | `bool` | `false` | |

**ListItem:** `icon: Option<Icon>` — per-item icon override.

---

## Feedback

### Alert

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `color` | `Option<String>` | `None` | |
| `variant` | `Option<String>` | `None` | "filled", "light", "outline", "transparent", "white", "default" |
| `title` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `with_close_button` | `bool` | `false` | |
| `icon` | `Option<Icon>` | `None` | |
| `onclose` | `Option<Callback>` | `None` | |

### Loader

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `type` | `Option<String>` | `None` | "oval", "bars", "dots" |
| `size` | `Option<String>` | `None` | |
| `color` | `Option<String>` | `None` | |

### Progress

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `Option<f32>` | `None` | Percentage 0-100 |
| `value_fn` | `Option<ReactiveF32>` | `None` | Reactive value binding (auto-wrapped) |
| `color` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `striped` | `bool` | `false` | |
| `animated` | `bool` | `false` | |

### Skeleton

Custom Default: `animate` and `visible` default to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `width` | `Option<String>` | `None` | |
| `height` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `circle` | `bool` | `false` | |
| `animate` | `bool` | **`true`** | |
| `visible` | `bool` | **`true`** | |

---

## Overlays

### Tooltip

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | Tooltip text |
| `position` | `Option<String>` | `None` | "top", "bottom", "left", "right" |
| `color` | `Option<String>` | `None` | |
| `opened` | `bool` | `false` | |
| `disabled` | `bool` | `false` | |
| `with_arrow` | `bool` | `false` | |
| `multiline` | `bool` | `false` | |
| `width` | `Option<String>` | `None` | |

### Modal

Custom Default: `with_overlay`, `close_on_click_outside`, `close_on_escape`, `with_close_button`, `lock_scroll`, `trap_focus` all default to `true`.

Positioned with `top: var(--rinch-window-top-inset, 0px)`, so it clears any window chrome rinch draws (the Linux in-app menu bar, the `BorderlessWindow` titlebar) and is flush with the top of a plain window. See [Theming](./theming.md#window-chrome-inset).

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `title` | `Option<String>` | `None` | |
| `size` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `with_overlay` | `bool` | **`true`** | |
| `overlay_opacity` | `Option<f32>` | `None` | |
| `overlay_blur` | `Option<String>` | `None` | |
| `centered` | `bool` | `false` | |
| `close_on_click_outside` | `bool` | **`true`** | |
| `close_on_escape` | `bool` | **`true`** | |
| `with_close_button` | `bool` | **`true`** | |
| `padding` | `Option<String>` | `None` | |
| `z_index` | `Option<i32>` | `None` | |
| `lock_scroll` | `bool` | **`true`** | |
| `trap_focus` | `bool` | **`true`** | |
| `onclose` | `Option<Callback>` | `None` | |

### Drawer

Custom Default: `with_overlay`, `close_on_click_outside`, `close_on_escape`, `with_close_button`, `lock_scroll`, `trap_focus` all default to `true`.

Positioned with `top: var(--rinch-window-top-inset, 0px)`, so it clears any window chrome rinch draws (the Linux in-app menu bar, the `BorderlessWindow` titlebar) and is flush with the top of a plain window. See [Theming](./theming.md#window-chrome-inset).

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `title` | `Option<String>` | `None` | |
| `position` | `Option<String>` | `None` | "left", "right", "top", "bottom" |
| `size` | `Option<String>` | `None` | |
| `with_overlay` | `bool` | **`true`** | |
| `overlay_opacity` | `Option<f32>` | `None` | |
| `close_on_click_outside` | `bool` | **`true`** | |
| `close_on_escape` | `bool` | **`true`** | |
| `with_close_button` | `bool` | **`true`** | |
| `padding` | `Option<String>` | `None` | |
| `z_index` | `Option<i32>` | `None` | |
| `lock_scroll` | `bool` | **`true`** | |
| `trap_focus` | `bool` | **`true`** | |
| `onclose` | `Option<Callback>` | `None` | |

### Notification

Custom Default: `with_close_button` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `opened_fn` | `Option<ReactiveBool>` | `None` | Reactive open state (auto-wrapped) |
| `title` | `Option<String>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `position` | `Option<String>` | `None` | Toast position. The `top-*` variants offset by `--rinch-window-top-inset` so they clear window chrome |
| `radius` | `Option<String>` | `None` | |
| `with_close_button` | `bool` | **`true`** | |
| `with_border` | `bool` | `false` | |
| `icon` | `Option<Icon>` | `None` | |
| `auto_close` | `u32` | `0` | Auto-close delay in ms (0 = disabled) |
| `loading` | `bool` | `false` | |
| `z_index` | `Option<i32>` | `None` | |
| `onclose` | `Option<Callback>` | `None` | |

### Popover

Custom Default: `close_on_click_outside` and `close_on_escape` default to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `opened` | `bool` | `false` | |
| `position` | `Option<String>` | `None` | |
| `offset` | `Option<i32>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `shadow` | `Option<String>` | `None` | |
| `with_arrow` | `bool` | `false` | |
| `arrow_size` | `Option<f32>` | `None` | |
| `arrow_offset` | `Option<f32>` | `None` | |
| `close_on_click_outside` | `bool` | **`true`** | |
| `close_on_escape` | `bool` | **`true`** | |
| `width` | `Option<String>` | `None` | |
| `z_index` | `Option<i32>` | `None` | |
| `trap_focus` | `bool` | `false` | |

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
| `position` | `Option<String>` | `None` | |
| `offset` | `Option<i32>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `shadow` | `Option<String>` | `None` | |
| `close_on_click_outside` | `bool` | **`true`** | |
| `close_on_item_click` | `bool` | **`true`** | |
| `width` | `Option<String>` | `None` | |
| `z_index` | `Option<i32>` | `None` | |

**DropdownMenuTarget**, **DropdownMenuDropdown**, **DropdownMenuLabel**, **DropdownMenuDivider**: No props.

**DropdownMenuItem:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `left_section` | `Option<Icon>` | `None` | |
| `right_section` | `Option<Icon>` | `None` | |
| `color` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `onclick` | `Option<Callback>` | `None` | |

### HoverCard

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `position` | `Option<String>` | `None` | |
| `offset` | `Option<i32>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `shadow` | `Option<String>` | `None` | |
| `width` | `Option<String>` | `None` | |
| `open_delay` | `Option<u32>` | `None` | |
| `close_delay` | `Option<u32>` | `None` | |
| `with_arrow` | `bool` | `false` | |

Sub-components: **HoverCardTarget** (no props), **HoverCardDropdown** (no props).

### LoadingOverlay

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `visible` | `bool` | `false` | |
| `overlay_opacity` | `Option<f32>` | `None` | |
| `overlay_blur` | `Option<String>` | `None` | |
| `loader_type` | `Option<String>` | `None` | |
| `loader_size` | `Option<String>` | `None` | |
| `loader_color` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `z_index` | `Option<i32>` | `None` | |
| `transition_duration` | `Option<u32>` | `None` | |

---

## Navigation

### Tabs

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `Option<String>` | `None` | Active tab value |
| `default_value` | `Option<String>` | `None` | |
| `variant` | `Option<String>` | `None` | "default", "outline", "pills" |
| `orientation` | `Option<String>` | `None` | "horizontal", "vertical" |
| `position` | `Option<String>` | `None` | |
| `grow` | `bool` | `false` | |
| `color` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |

**TabsList:** `grow: bool`, `justify: Option<String>`.

**Tab:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `Option<String>` | `None` | Tab identifier |
| `disabled` | `bool` | `false` | |
| `left_section` | `Option<Icon>` | `None` | |
| `right_section` | `Option<Icon>` | `None` | |
| `onclick` | `Option<Callback>` | `None` | |

**TabsPanel:** `value: Option<String>` — matches the Tab value.

### Accordion

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `value` | `Option<String>` | `None` | Active item value |
| `default_value` | `Option<String>` | `None` | |
| `variant` | `Option<String>` | `None` | "default", "contained", "filled", "separated" |
| `radius` | `Option<String>` | `None` | |
| `multiple` | `bool` | `false` | Allow multiple open items |
| `chevron_position` | `Option<String>` | `None` | "left", "right" |
| `disable_chevron_rotation` | `bool` | `false` | |

**AccordionItem:** `value: Option<String>`.

**AccordionControl:** `disabled: bool`, `icon: Option<Icon>`, `onclick: Option<Callback>`.

**AccordionPanel:** No props.

### Breadcrumbs

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `separator` | `Option<String>` | `None` | Custom separator character |
| `separator_margin` | `Option<String>` | `None` | Spacing around separator |

**BreadcrumbsItem:** `href: Option<String>`.

### Pagination

Custom Default: `total`, `value`, `siblings`, `boundaries` default to `1`; `with_controls` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `total` | `u32` | **`1`** | Total number of pages |
| `value` | `u32` | **`1`** | Current page |
| `siblings` | `u32` | **`1`** | Pages visible on each side |
| `boundaries` | `u32` | **`1`** | Pages at start/end |
| `size` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `with_edges` | `bool` | `false` | Show first/last page buttons |
| `with_controls` | `bool` | **`true`** | Show prev/next buttons |
| `color` | `Option<String>` | `None` | |
| `disabled` | `bool` | `false` | |
| `gap` | `Option<String>` | `None` | |
| `onchange` | `Option<ValueCallback<u32>>` | `None` | Receives page number |

### NavLink

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `href` | `Option<String>` | `None` | |
| `active` | `bool` | `false` | |
| `active_fn` | `Option<ReactiveBool>` | `None` | Reactive active binding (auto-wrapped) |
| `variant` | `Option<String>` | `None` | "light", "filled", "subtle" |
| `color` | `Option<String>` | `None` | |
| `left_section` | `Option<Icon>` | `None` | |
| `right_section` | `Option<Icon>` | `None` | |
| `disabled` | `bool` | `false` | |
| `children_offset` | `Option<String>` | `None` | Indentation for nested NavLinks |
| `opened` | `bool` | `false` | Nested section expanded |
| `default_opened` | `bool` | `false` | |
| `no_wrap` | `bool` | `false` | |
| `onclick` | `Option<Callback>` | `None` | |

### Stepper

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `active` | `u32` | `0` | Active step index |
| `size` | `Option<String>` | `None` | |
| `orientation` | `Option<String>` | `None` | "horizontal", "vertical" |
| `color` | `Option<String>` | `None` | |
| `radius` | `Option<String>` | `None` | |
| `icon_size` | `Option<String>` | `None` | |
| `allow_next_steps_select` | `bool` | `false` | |
| `completed_icon` | `Option<Icon>` | `None` | Default completed icon for all steps |
| `progress_icon` | `Option<Icon>` | `None` | Default in-progress icon |

**StepperStep:**

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `label` | `Option<String>` | `None` | |
| `description` | `Option<String>` | `None` | |
| `icon` | `Option<Icon>` | `None` | Default icon |
| `completed_icon` | `Option<Icon>` | `None` | Per-step override |
| `progress_icon` | `Option<Icon>` | `None` | Per-step override |
| `allow_step_click` | `bool` | `false` | |
| `allow_step_select` | `bool` | `false` | |
| `loading` | `bool` | `false` | |
| `state` | `Option<String>` | `None` | "step-progress", "step-completed", "step-inactive" |
| `step` | `Option<u32>` | `None` | Step index |

**StepperCompleted:** No props.

### Tree

Custom Default: `level_offset` defaults to `Some("md")`, `expand_on_click` defaults to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `data` | `Vec<TreeNodeData>` | `[]` | Tree data |
| `tree` | `Option<UseTreeReturn>` | `None` | State from `UseTreeReturn::new()` |
| `level_offset` | `Option<String>` | **`Some("md")`** | Indentation per level |
| `expand_on_click` | `bool` | **`true`** | Click expands/collapses |
| `select_on_click` | `bool` | `false` | Click selects |
| `render_node` | `Option<RenderTreeNode>` | `None` | Custom node renderer |
| `onselect` | `Option<ValueCallback<String>>` | `None` | |
| `onexpand` | `Option<ValueCallback<String>>` | `None` | |
| `oncollapse` | `Option<ValueCallback<String>>` | `None` | |

**TreeNodeData:** `value: String`, `label: String`, `children: Vec<TreeNodeData>`, `disabled: bool`, `icon: Option<Icon>`, `payload: Option<Rc<dyn Any>>`.

---

## Window

### BorderlessWindow

Custom Default: `show_minimize`, `show_maximize`, `show_close` all default to `true`.

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `title` | `Option<String>` | `None` | Window title in titlebar |
| `radius` | `Option<String>` | `None` | Corner radius (none, xs, sm, md, lg, xl) |
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
