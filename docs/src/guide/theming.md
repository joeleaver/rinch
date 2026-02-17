# Theming

Rinch includes a powerful theme system inspired by [Mantine](https://mantine.dev/). Enable it with the `theme` feature:

```toml
[dependencies]
rinch = { path = "...", features = ["theme"] }
```

## Theme Configuration

Configure the theme at the runtime level using `ThemeProviderProps`:

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div {
            h1 { "Themed Application" }
            p { style: "color: var(--rinch-primary-color);",
                "This text uses the primary color."
            }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        default_radius: Some("md".into()),
        dark_mode: false,
        ..Default::default()
    };

    run_with_theme("Themed App", 800, 600, app, theme);
}
```

### ThemeProviderProps

| Prop | Type | Default | Description |
|------|------|---------|-------------|
| `primary_color` | `Option<String>` | `"blue"` | Primary color name (blue, cyan, red, etc.) |
| `default_radius` | `Option<String>` | `"sm"` | Default border radius (xs, sm, md, lg, xl) |
| `font_family` | `Option<String>` | System fonts | Primary font family |
| `font_family_monospace` | `Option<String>` | System mono | Monospace font family |
| `dark_mode` | `bool` | `false` | Enable dark mode |
| `primary_shade` | `Option<u8>` | `6` | Primary color shade index (0-9) |

## CSS Variables

The theme generates CSS custom properties that you can use in your styles:

### Colors

Each color has 10 shades (0 = lightest, 9 = darkest):

```css
/* Color palettes */
var(--rinch-color-blue-0)    /* Lightest blue */
var(--rinch-color-blue-5)    /* Mid blue */
var(--rinch-color-blue-9)    /* Darkest blue */

/* Primary color aliases */
var(--rinch-primary-color)   /* Primary at shade index */
var(--rinch-primary-color-0) /* Lightest primary */
var(--rinch-primary-color-9) /* Darkest primary */

/* Semantic colors */
var(--rinch-color-body)        /* Background */
var(--rinch-color-text)        /* Text */
var(--rinch-color-dimmed)      /* Secondary text */
var(--rinch-color-border)      /* Borders */
var(--rinch-color-placeholder) /* Placeholder text */
```

### Color Palette Reference

All 14 named colors, each with 10 shades (0 = lightest, 9 = darkest):

| Color | Shade 0 | Shade 6 | Notes |
|-------|---------|---------|-------|
| `dark` | `#C1C2C5` | `#1A1B1E` | Dark grays |
| `gray` | `#f8f9fa` | `#868e96` | **gray-0 matches default body background** — use gray-1+ for visible backgrounds |
| `red` | `#fff5f5` | `#fa5252` | |
| `pink` | `#fff0f6` | `#e64980` | |
| `grape` | `#f8f0fc` | `#be4bdb` | |
| `violet` | `#f3f0ff` | `#7950f2` | |
| `indigo` | `#edf2ff` | `#4c6ef5` | |
| `blue` | `#e7f5ff` | `#228be6` | Default primary |
| `cyan` | `#e3fafc` | `#15aabf` | |
| `teal` | `#e6fcf5` | `#12b886` | |
| `green` | `#ebfbee` | `#40c057` | |
| `lime` | `#f4fce3` | `#82c91e` | |
| `yellow` | `#fff9db` | `#fab005` | |
| `orange` | `#fff4e6` | `#fd7e14` | |

> **Tip:** `--rinch-color-body` defaults to `#f8f9fa` (gray-0) in light mode. If you use `background: var(--rinch-color-gray-0)`, it will be invisible against the default body. Use `gray-1` (`#f1f3f5`) or higher for visible card/section backgrounds.

### Spacing

```css
var(--rinch-spacing-xs)  /* 10px */
var(--rinch-spacing-sm)  /* 12px */
var(--rinch-spacing-md)  /* 16px */
var(--rinch-spacing-lg)  /* 20px */
var(--rinch-spacing-xl)  /* 32px */
```

### Border Radius

```css
var(--rinch-radius-xs)      /* 2px */
var(--rinch-radius-sm)      /* 4px */
var(--rinch-radius-md)      /* 8px */
var(--rinch-radius-lg)      /* 16px */
var(--rinch-radius-xl)      /* 32px */
var(--rinch-radius-default) /* Theme default */
```

### Font Sizes

```css
var(--rinch-font-size-xs) /* 12px */
var(--rinch-font-size-sm) /* 14px */
var(--rinch-font-size-md) /* 16px */
var(--rinch-font-size-lg) /* 18px */
var(--rinch-font-size-xl) /* 20px */
```

### Line Heights

```css
var(--rinch-line-height-xs) /* 1.4 */
var(--rinch-line-height-sm) /* 1.45 */
var(--rinch-line-height-md) /* 1.55 */
var(--rinch-line-height-lg) /* 1.6 */
var(--rinch-line-height-xl) /* 1.65 */
```

### Shadows

```css
var(--rinch-shadow-xs)
var(--rinch-shadow-sm)
var(--rinch-shadow-md)
var(--rinch-shadow-lg)
var(--rinch-shadow-xl)
```

### Font Families

```css
var(--rinch-font-family)           /* Primary font */
var(--rinch-font-family-monospace) /* Monospace font */
```

### Headings

```css
/* For each level (h1-h6) */
var(--rinch-h1-font-size)
var(--rinch-h1-line-height)
var(--rinch-h1-font-weight)
```

## Example: Custom Styled Component

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { style: "
            background: var(--rinch-color-body);
            padding: var(--rinch-spacing-md);
            border-radius: var(--rinch-radius-default);
            box-shadow: var(--rinch-shadow-sm);
        ",
            h1 { style: "
                color: var(--rinch-primary-color);
                margin-bottom: var(--rinch-spacing-sm);
            ",
                "Welcome!"
            }
            p { style: "
                color: var(--rinch-color-dimmed);
                font-size: var(--rinch-font-size-sm);
            ",
                "This text uses theme variables."
            }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("violet".into()),
        ..Default::default()
    };

    run_with_theme("Custom Styles", 800, 600, app, theme);
}
```

## Dark Mode

Enable dark mode to automatically adjust semantic colors:

```rust
let theme = ThemeProviderProps {
    dark_mode: true,
    ..Default::default()
};
```

In dark mode:
- `--rinch-color-body` becomes dark gray
- `--rinch-color-text` becomes light
- Other semantic colors adjust accordingly

## Programmatic Theme Access

When the `theme` feature is enabled, you can access theme utilities:

```rust
use rinch::theme::{Theme, ColorName, generate_theme_css};

// Create a custom theme
let theme = Theme::builder()
    .primary_color(ColorName::Cyan)
    .dark_mode(true)
    .build();

// Generate CSS
let css = generate_theme_css(&theme);
```

## Widget Integration

When using the `widgets` feature, all components automatically use theme CSS variables:

```rust
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;

#[component]
fn app() -> NodeHandle {
    rsx! {
        Stack { gap: "md",
            // These widgets automatically use theme colors
            Button { "Primary Button" }
            Badge { variant: "light", "Status" }
            Alert { color: "blue", "Info message" }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        ..Default::default()
    };

    run_with_theme("Themed Widgets", 800, 600, app, theme);
}
```

See the [Widgets Guide](./widgets.md) for the complete list of available components.
