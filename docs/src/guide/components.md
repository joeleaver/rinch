# Components

Rinch provides a comprehensive component library with 60+ styled, themeable UI components inspired by [Mantine](https://mantine.dev/). Enable with the `components` feature (which also enables `theme`):

```toml
[dependencies]
rinch = { workspace = true, features = ["desktop", "components", "theme"] }
```

> **Note:** The workspace dependency uses `default-features = false`, so features must be listed explicitly:
> - `"desktop"` — enables `run()`, window management, and the software renderer (tiny-skia)
> - `"gpu"` — (optional) enables GPU rendering via Vello/wgpu instead of the software renderer
> - `"components"` — enables the component library (Button, TextInput, Stack, etc.)
> - `"theme"` — enables automatic theme CSS loading and CSS variables

## Using Components

Components are used directly in RSX with a declarative syntax:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        Stack { gap: "md",
            Title { order: 1, "Welcome" }
            Text { size: "lg", color: "dimmed", "Get started with Rinch components." }

            Group { gap: "sm",
                Button { variant: "filled", "Get Started" }
                Button { variant: "outline", "Learn More" }
            }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("cyan".into()),
        ..Default::default()
    };
    run_with_theme("My App", 800, 600, app, theme);
}
```

## Component Categories

### Layout Components

| Component | Description |
|-----------|-------------|
| `Stack` | Vertical flex container with spacing |
| `Group` | Horizontal flex container with spacing |
| `Container` | Centered max-width container |
| `Center` | Center content horizontally/vertically |
| `Space` | Empty space component |

```rust
// Stack - vertical layout
Stack { gap: "md",
    Text { "Item 1" }
    Text { "Item 2" }
}

// Group - horizontal layout
Group { gap: "sm", justify: "space-between",
    Button { "Left" }
    Button { "Right" }
}

// Center content
Center {
    Loader {}
}

// Add spacing
Space { h: "xl" }  // height
Space { w: "md" }  // width
```

### Buttons

| Component | Description |
|-----------|-------------|
| `Button` | Clickable button with variants |
| `ActionIcon` | Icon-only button |
| `CloseButton` | Dismiss/close button |

```rust
// Button variants
Button { variant: "filled", "Primary" }
Button { variant: "outline", "Secondary" }
Button { variant: "light", "Light" }
Button { variant: "subtle", "Subtle" }

// Button sizes
Button { size: "xs", "Extra Small" }
Button { size: "sm", "Small" }
Button { size: "md", "Medium" }  // default
Button { size: "lg", "Large" }
Button { size: "xl", "Extra Large" }

// Button colors
Button { color: "red", "Delete" }
Button { color: "green", "Success" }

// Button states
Button { disabled: true, "Disabled" }
Button { loading: true, "Loading..." }
Button { full_width: true, "Full Width" }

// With click handler
Button { onclick: move || count.update(|n| *n += 1), "Click Me" }

// ActionIcon - for icon buttons
ActionIcon { variant: "filled", "+" }
ActionIcon { size: "lg", variant: "light", "?" }

// CloseButton
CloseButton { onclick: move || modal_open.set(false) }
```

### Form Inputs

| Component | Description |
|-----------|-------------|
| `TextInput` | Single-line text input |
| `Textarea` | Multi-line text input |
| `PasswordInput` | Password field with visibility toggle |
| `NumberInput` | Numeric input with controls |
| `Checkbox` | Checkbox input |
| `Switch` | Toggle switch |
| `Select` | Dropdown select |
| `Radio` / `RadioGroup` | Radio buttons |
| `Slider` | Range input slider |
| `Fieldset` | Grouped form fields |

```rust
// TextInput with label and description
TextInput {
    label: "Email",
    placeholder: "you@example.com",
    description: "We'll never share your email."
}

// TextInput with error
TextInput {
    label: "Password",
    input_type: "password",
    error: "Password must be at least 8 characters"
}

// Controlled input (value_fn + oninput)
let input_text = Signal::new(String::new());
TextInput {
    placeholder: "Type here...",
    value_fn: move || input_text.get(),
    oninput: move |value: String| input_text.set(value),
}

// TextInput with onsubmit (fires on Enter key)
let search = Signal::new(String::new());
TextInput {
    placeholder: "Search...",
    value_fn: move || search.get(),
    oninput: move |value: String| search.set(value),
    onsubmit: move || println!("Searching: {}", search.get()),
}

// PasswordInput with visibility toggle
PasswordInput {
    label: "Password",
    placeholder: "Enter password"
}

// NumberInput
NumberInput {
    label: "Quantity",
    placeholder: "0"
}

// Textarea
Textarea {
    label: "Description",
    placeholder: "Enter your message...",
    min_rows: 4          // visible rows; defaults to 2
}

// Checkbox
let checked = Signal::new(false);
Checkbox {
    label: "Accept terms",
    checked: checked.get(),
    onchange: move || checked.update(|v| *v = !*v)
}

// Switch
Switch {
    label: "Enable notifications",
    checked: enabled.get(),
    onchange: move || enabled.update(|v| *v = !*v)
}

// Select
Select {
    label: "Country",
    placeholder: "Select a country",
    option { value: "us", "United States" }
    option { value: "uk", "United Kingdom" }
    option { value: "ca", "Canada" }
}

// RadioGroup
RadioGroup { label: "Plan",
    Radio { value: "free", label: "Free" }
    Radio { value: "pro", label: "Pro" }
    Radio { value: "enterprise", label: "Enterprise" }
}

// Slider
Slider { color: "cyan" }

// Fieldset
Fieldset { legend: "Personal Info",
    Stack { gap: "sm",
        TextInput { placeholder: "First name" }
        TextInput { placeholder: "Last name" }
    }
}
```

### Typography

| Component | Description |
|-----------|-------------|
| `Text` | Text display with styling |
| `Title` | Heading text (h1-h6) |
| `Code` | Inline or block code |
| `Kbd` | Keyboard key |
| `Anchor` | Styled link |
| `Blockquote` | Styled quotation |
| `Mark` | Highlighted text |
| `Highlight` | Search text highlighting |

```rust
// Text sizes and weights
Text { size: "lg", weight: "bold", "Large Bold Text" }
Text { size: "sm", color: "dimmed", "Small dimmed text" }

// Titles (h1-h6)
Title { order: 1, "Main Heading" }
Title { order: 2, "Subheading" }
Title { order: 3, "Section Title" }

// Code
Text { "Run " Code { "npm install" } " to install." }
Code { block: true, "fn main() {\n    println!(\"Hello!\");\n}" }

// Keyboard keys
Group { gap: "xs",
    Kbd { "Ctrl" }
    Text { "+" }
    Kbd { "C" }
}

// Anchor (link)
Anchor { href: "https://example.com", "Visit site" }
Anchor { href: "#", underline: true, "Always underlined" }

// Blockquote
Blockquote { cite: "— Albert Einstein",
    "Imagination is more important than knowledge."
}

// Mark (highlight)
Text { "This has " Mark { "marked text" } " inline." }

// Highlight (search highlighting)
Highlight { highlight: "quick brown", "The quick brown fox jumps" }
```

### Feedback

| Component | Description |
|-----------|-------------|
| `Alert` | User feedback messages |
| `Loader` | Loading spinner/indicator |
| `Progress` | Progress bar |
| `Skeleton` | Loading placeholder |
| `Tooltip` | CSS-only hover tooltip |

```rust
// Alert variants
Alert { color: "blue", title: "Info", "This is informational." }
Alert { color: "green", title: "Success", "Operation completed!" }
Alert { color: "yellow", title: "Warning", "Please review." }
Alert { color: "red", title: "Error", "Something went wrong." }

Alert { variant: "filled", color: "blue", "Filled style" }
Alert { variant: "light", color: "blue", "Light style" }
Alert { variant: "outline", color: "blue", "Outline style" }

// Loader
Loader {}
Loader { size: "lg", color: "cyan" }

// Progress
Progress { color: "blue" }
Progress { color: "green", striped: true }
Progress { striped: true, animated: true }

// Skeleton (loading placeholders)
Skeleton { height: "12px", width: "80%" }
Skeleton { height: "40px", width: "40px", circle: true }

// Tooltip
Tooltip { label: "This is a tooltip!",
    Button { "Hover me" }
}
Tooltip { label: "Top tooltip", position: "top",
    Button { "Top" }
}
```

### Data Display

| Component | Description |
|-----------|-------------|
| `Avatar` | User avatar with image or initials |
| `Badge` | Small status indicator |
| `Card` / `CardSection` | Container with sections |
| `Paper` | Card-like container with shadow |
| `Divider` | Horizontal/vertical separator |
| `Image` | Responsive image with fallback |
| `List` / `ListItem` | Styled list |

```rust
// Avatar
Avatar { name: "John Doe" }  // Shows "JD"
Avatar { size: "lg", color: "cyan", name: "Jane Smith" }
Avatar { src: "https://example.com/photo.jpg" }

// Badge
Badge { "New" }
Badge { variant: "light", color: "green", "Active" }
Badge { variant: "outline", color: "red", "Error" }
Badge { variant: "dot", "With dot" }

// Card
Card { shadow: "sm",
    CardSection { with_border: true,
        div { style: "padding: var(--rinch-spacing-sm);",
            Text { weight: "bold", "Card Title" }
        }
    }
    div { style: "padding: var(--rinch-spacing-md);",
        Text { "Card content goes here." }
    }
}

// Paper
Paper { shadow: "md", p: "lg",
    "Simple paper container"
}
Paper { shadow: "sm", p: "md", with_border: true,
    "Paper with border"
}

// Divider
Divider {}
Divider { label: "OR" }

// Image (local file or URL with image-network feature)
Image {
    src: "photo.png",
    width: "200",
    height: "150",
    fit: "cover",
    radius: "md",
}

// Avatar with image
Avatar { src: "photo.png", size: "lg" }

// List
List {
    ListItem { "First item" }
    ListItem { "Second item" }
    ListItem { "Third item" }
}
```

### Navigation

| Component | Description |
|-----------|-------------|
| `Tabs` / `TabsList` / `Tab` / `TabsPanel` | Tab navigation |
| `Accordion` / `AccordionItem` / `AccordionControl` / `AccordionPanel` | Collapsible sections |
| `Breadcrumbs` / `BreadcrumbsItem` | Navigation trail |
| `Pagination` | Page navigation |
| `NavLink` | Navigation link with active state |
| `Stepper` / `StepperStep` / `StepperCompleted` | Step progress |

```rust
// Tabs
Tabs {
    TabsList {
        Tab { value: "gallery", "Gallery" }
        Tab { value: "messages", "Messages" }
        Tab { value: "settings", "Settings" }
    }
    TabsPanel { value: "gallery",
        Text { "Gallery content" }
    }
    TabsPanel { value: "messages",
        Text { "Messages content" }
    }
}

// Tab variants
Tabs { variant: "outline", /* ... */ }
Tabs { variant: "pills", /* ... */ }

// Accordion
Accordion {
    AccordionItem { value: "item-1",
        AccordionControl { "What is Rinch?" }
        AccordionPanel {
            Text { "A reactive GUI framework for Rust." }
        }
    }
    AccordionItem { value: "item-2",
        AccordionControl { "How do I get started?" }
        AccordionPanel {
            Text { "Add rinch to your Cargo.toml!" }
        }
    }
}

// Breadcrumbs
Breadcrumbs {
    BreadcrumbsItem { "Home" }
    BreadcrumbsItem { "Products" }
    BreadcrumbsItem { "Component" }
}
Breadcrumbs { separator: ">",
    BreadcrumbsItem { "Docs" }
    BreadcrumbsItem { "API" }
}

// Pagination
Pagination {}
Pagination { with_edges: true }

// NavLink
NavLink { label: "Dashboard", active: true }
NavLink { label: "Settings", description: "App configuration" }
NavLink { label: "Disabled", disabled: true }
NavLink { label: "With handler", onclick: move || navigate("/page") }

// Stepper
let step = Signal::new(1u32);
Stepper { active: step.get(),
    StepperStep { label: "Account", description: "Create account" }
    StepperStep { label: "Verify", description: "Verify email" }
    StepperStep { label: "Complete", description: "Get started" }
}
```

### Overlays

| Component | Description |
|-----------|-------------|
| `Modal` | Dialog overlay |
| `Drawer` | Slide-out panel |
| `Notification` | Toast notification |
| `Popover` / `PopoverTarget` / `PopoverDropdown` | Positioned popup |
| `DropdownMenu` / `DropdownMenuTarget` / `DropdownMenuDropdown` / `DropdownMenuItem` | Dropdown menu |
| `HoverCard` / `HoverCardTarget` / `HoverCardDropdown` | Card on hover |
| `LoadingOverlay` | Loading overlay for containers |

`Modal`, `Drawer`, and the top-anchored `Notification` positions automatically
clear any window chrome rinch draws (the Linux in-app menu bar, the
`BorderlessWindow` titlebar) by offsetting with `--rinch-window-top-inset`. The
invisible click-catching backdrops behind `DropdownMenu` and `Select` deliberately
do not — they must catch clicks on the chrome too.

If you hand-roll a `position: fixed` overlay, offset it the same way
(`top: var(--rinch-window-top-inset, 0px)`); fixed resolves against the real
viewport, so a bare `top: 0` renders underneath the chrome. See
[Theming → Window Chrome Inset](./theming.md#window-chrome-inset).

```rust
// Modal
let modal_open = Signal::new(false);

Button { onclick: move || modal_open.set(true), "Open Modal" }

Modal {
    opened: modal_open.get(),
    onclose: move || modal_open.set(false),
    title: "Confirm Action",
    size: "md",

    Text { "Are you sure you want to proceed?" }
    Space { h: "lg" }
    Group { justify: "end", gap: "sm",
        Button { variant: "outline", "Cancel" }
        Button { "Confirm" }
    }
}

// Drawer
Drawer {
    opened: drawer_open.get(),
    onclose: move || drawer_close.set(false),
    title: "Settings",
    position: "right",  // left, right, top, bottom
    size: "sm",

    // Drawer content
}

// Notification
Notification {
    opened: show_notification.get(),
    title: "Success!",
    color: "green",
    position: "top-right",
    onclose: move || close_notification.set(false),
    "Your changes have been saved."
}

// Popover
Popover { position: "bottom",
    PopoverTarget {
        Button { "Click me" }
    }
    PopoverDropdown {
        Text { "Popover content!" }
        Button { size: "xs", "Action" }
    }
}

// DropdownMenu
DropdownMenu {
    DropdownMenuTarget {
        Button { "Menu" }
    }
    DropdownMenuDropdown {
        DropdownMenuLabel { "Application" }
        DropdownMenuItem { "Settings" }
        DropdownMenuItem { "Profile" }
        DropdownMenuDivider {}
        DropdownMenuItem { color: "red", "Delete" }
    }
}

// HoverCard
HoverCard { position: "bottom",
    HoverCardTarget {
        Text { weight: "bold", "@username" }
    }
    HoverCardDropdown {
        Group { gap: "sm",
            Avatar { name: "User Name" }
            Stack { gap: "xs",
                Text { weight: "bold", "User Name" }
                Text { size: "sm", color: "dimmed", "Bio here" }
            }
        }
    }
}

// LoadingOverlay — the `position: relative` on the container is REQUIRED.
// Without it the overlay's containing block is the viewport and it covers the
// whole window (the CSS rule, and what rinch-web has always done).
div { style: "position: relative; height: 200px;",
    // Your content
    LoadingOverlay { visible: is_loading.get() }
}
```

### Window Components

| Component | Description |
|-----------|-------------|
| `BorderlessWindow` | Container for borderless/transparent windows with custom titlebar |

```rust
use rinch::prelude::*;
use std::rc::Rc;

#[component]
fn app() -> NodeHandle {
    // For custom left section (e.g., menu button)
    let menu_open = Signal::new(false);
    let left_section: SectionRenderer = Rc::new(move |__scope| {
        rsx! {
            ActionIcon { onclick: move || menu_open.update(|v| *v = !*v),
                "☰"
            }
        }
    });

    rsx! {
        BorderlessWindow {
            title: "My App",
            radius: "md",  // none, xs, sm, md, lg, xl
            left_section: Some(left_section),
            on_minimize: || minimize_current_window(),
            on_maximize: || toggle_maximize_current_window(),
            on_close: || close_current_window(),

            // Your content here
            Stack { gap: "md",
                Title { order: 2, "Welcome!" }
                Text { "This is a borderless window." }
            }
        }
    }
}

fn main() {
    let window_props = WindowProps {
        title: "My App".into(),
        borderless: true,
        transparent: true,
        resize_inset: Some(8.0),
        ..Default::default()
    };

    run_with_window_props(app, window_props, None);
}
```

**BorderlessWindow Props:**

| Prop | Type | Description |
|------|------|-------------|
| `title` | `Option<String>` | Window title in titlebar |
| `radius` | `Option<String>` | Corner radius: none, xs, sm, md, lg, xl |
| `show_minimize` | `bool` | Show minimize button (default: true) |
| `show_maximize` | `bool` | Show maximize button (default: true) |
| `show_close` | `bool` | Show close button (default: true) |
| `left_section` | `Option<SectionRenderer>` | Custom content for titlebar left |
| `right_section` | `Option<SectionRenderer>` | Custom content before controls |
| `on_minimize` | `Option<Callback>` | Minimize callback |
| `on_maximize` | `Option<Callback>` | Maximize callback |
| `on_close` | `Option<Callback>` | Close callback |

The component provides:
- Rounded corners with configurable radius
- Draggable titlebar (via `app-region: drag`)
- Window control buttons with hover effects
- Left/right custom sections for menu buttons or additional controls
- Proper theming via CSS variables
- `--rinch-window-top-inset` published for overlays (36px for the titlebar, plus
  the menu bar height when one sits below it)

The titlebar is an in-flow element, so your content sits below it automatically.
A `position: fixed` overlay does not — it resolves against the real viewport and
would cover the titlebar and its close button. Offset such overlays by
`var(--rinch-window-top-inset, 0px)`; see
[Theming → Window Chrome Inset](./theming.md#window-chrome-inset).

## Building Custom Components

> **See the dedicated [Writing Components](./writing-components.md) guide** for the recommended approach using `#[component]` PascalCase functions. The manual `Component` trait approach below is only needed for advanced use cases.

## Building Custom Component Crates (Advanced)

You can create your own component library crate that integrates with Rinch's theme system.

### 1. Create the Crate

```toml
# Cargo.toml
[package]
name = "my-components"
version = "0.1.0"
edition = "2021"

[dependencies]
rinch-core = { path = "../rinch-core" }
```

### 2. Implement the Component Trait

Each component must implement the `Component` trait from `rinch-core`. Components receive a `RenderScope` for DOM construction and return a `NodeHandle`:

```rust
use rinch_core::{Component, NodeHandle, RenderScope};

/// A custom card component.
#[derive(Debug, Default)]
pub struct MyCard {
    /// Card title.
    pub title: Option<String>,
    /// Card variant.
    pub variant: Option<String>,
    /// Whether to show shadow.
    pub shadow: bool,
}

impl Component for MyCard {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // Create the card element
        let card = scope.create_element("div");

        // Build CSS classes
        let mut classes = vec!["my-card".to_string()];

        if let Some(ref v) = self.variant {
            match v.as_str() {
                "elevated" => classes.push("my-card--elevated".to_string()),
                "outlined" => classes.push("my-card--outlined".to_string()),
                _ => {}
            }
        }

        if self.shadow {
            classes.push("my-card--shadow".to_string());
        }

        card.set_attribute("class", &classes.join(" "));

        // Add title if provided
        if let Some(ref title) = self.title {
            let title_div = scope.create_element("div");
            title_div.set_attribute("class", "my-card__title");
            title_div.set_text(title);
            card.append_child(&title_div);
        }

        // Append children
        for child in children {
            card.append_child(child);
        }

        card
    }
}
```

### 3. Add Event Handlers

For interactive components, add event listeners directly to DOM nodes:

```rust
use rinch_core::Callback;

pub struct MyButton {
    pub onclick: Option<Callback>,
}

impl Component for MyButton {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let button = scope.create_element("button");
        button.set_attribute("class", "my-button");

        // Add click handler if provided
        if let Some(ref cb) = self.onclick {
            let cb = cb.clone();
            button.add_event_listener("click", move |_| cb.invoke());
        }

        // Append children
        for child in children {
            button.append_child(child);
        }

        button
    }
}
```

### 4. Create CSS Styles

Create a styles module that generates CSS for your components:

```rust
// src/styles/mod.rs
mod card;
mod button;

pub fn generate_all_styles() -> String {
    let mut css = String::new();
    css.push_str(&card::styles());
    css.push_str(&button::styles());
    css
}

// src/styles/card.rs
pub fn styles() -> String {
    r#"
.my-card {
    background: var(--rinch-color-body);
    border-radius: var(--rinch-radius-default);
    padding: var(--rinch-spacing-md);
}

.my-card--elevated {
    box-shadow: var(--rinch-shadow-md);
}

.my-card--outlined {
    border: 1px solid var(--rinch-color-border);
}

.my-card__title {
    font-size: var(--rinch-font-size-lg);
    font-weight: 600;
    margin-bottom: var(--rinch-spacing-sm);
}
"#.to_string()
}
```

### 5. Export Your Components

```rust
// src/lib.rs
pub mod styles;

mod card;
mod button;

pub use card::MyCard;
pub use button::MyButton;

// Re-export Component trait for convenience
pub use rinch_core::Component;

/// Generate all component CSS.
pub fn generate_css() -> String {
    styles::generate_all_styles()
}
```

### 6. Use Theme CSS Variables

Leverage Rinch's theme variables for consistency:

| Variable | Description |
|----------|-------------|
| `--rinch-primary-color` | Primary theme color |
| `--rinch-color-body` | Background color |
| `--rinch-color-text` | Text color |
| `--rinch-color-dimmed` | Dimmed/muted text |
| `--rinch-color-border` | Border color |
| `--rinch-color-{name}-{0-9}` | Color shades (e.g., `--rinch-color-blue-5`) |
| `--rinch-spacing-{xs,sm,md,lg,xl}` | Spacing scale |
| `--rinch-radius-{xs,sm,md,lg,xl}` | Border radius |
| `--rinch-radius-default` | Default radius |
| `--rinch-shadow-{xs,sm,md,lg,xl}` | Box shadows |
| `--rinch-font-size-{xs,sm,md,lg,xl}` | Font sizes |
| `--rinch-font-family` | Default font |
| `--rinch-font-family-monospace` | Monospace font |
| `--rinch-window-top-inset` | Height of the window chrome rinch draws above your content. Not theme-generated — published at runtime by the in-app menu bar / `BorderlessWindow` titlebar. A `position: fixed` overlay should offset by it (`top: var(--rinch-window-top-inset, 0px)`), since fixed resolves against the real viewport and would otherwise slide underneath the chrome. See [Theming](./theming.md#window-chrome-inset) |

### 7. Use Your Component Crate

```rust
use rinch::prelude::*;
use my_components::{MyCard, MyButton};

#[component]
fn app() -> NodeHandle {
    rsx! {
        MyCard { title: "Hello", variant: "elevated", shadow: true,
            Text { "Card content here" }
            MyButton { onclick: move || println!("Clicked!"),
                "Click Me"
            }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        ..Default::default()
    };
    run_with_theme("Custom Components", 800, 600, app, theme);
}
```

## Component CSS Injection

Component CSS is automatically injected when you use `ThemeProvider`. If you need the CSS separately:

```rust
use rinch::components::generate_component_css;

let css = generate_component_css();
```

For custom component crates, inject your CSS alongside the theme:

```rust
use my_components::generate_css;

// Add to your HTML head or inject via ThemeProvider
let custom_css = generate_css();
```

## Custom Components with #[component]

You can create reusable custom components using the `#[component]` macro. Components written in PascalCase become usable components in RSX:

```rust
use rinch::prelude::*;

#[component]
pub fn MyCard(
    title: String,
    color: String,
    children: &[NodeHandle],
) -> NodeHandle {
    rsx! {
        div { class: "my-card",
            h2 { {title.clone()} }
            div { class: "card-body", style: {move || format!("color: {}", color.clone())},
                children
            }
        }
    }
}

// Use in RSX:
#[component]
fn app() -> NodeHandle {
    rsx! {
        MyCard { title: "Hello", color: "blue",
            Text { "Card content here" }
        }
    }
}
```

**Key points:**
- **PascalCase** function names become components in RSX
- `children: &[NodeHandle]` is a special parameter that receives child content
- Use `children` in the body to render child elements
- `#[component]` auto-injects `__scope: &mut RenderScope` as the first parameter
- Props are regular Rust function parameters

**Without children:**
```rust
#[component]
pub fn StatusBadge(label: String, status: String) -> NodeHandle {
    rsx! {
        div { class: "badge",
            style: {move || format!("background: {}",
                if status == "success" { "green" } else { "red" })},
            {label.clone()}
        }
    }
}

// Use:
StatusBadge { label: "Active", status: "success" }
```

**With callback props:**

`Callback` and `InputCallback` have built-in defaults (no-op), so they work as direct props without `Option` wrapping:

```rust
#[component]
pub fn TodoItem(
    label: String,
    completed: bool,
    on_toggle: Callback,
    on_delete: Callback,
) -> NodeHandle {
    rsx! {
        Group { gap: "sm", align: "center",
            Checkbox {
                checked: completed,
                onchange: on_toggle,  // Forward directly — no Option unwrapping needed
            }
            Text { {label.clone()} }
            ActionIcon { variant: "subtle", color: "red",
                onclick: on_delete,
                "X"
            }
        }
    }
}

// Use — closures are auto-wrapped into Callback:
TodoItem {
    label: "Buy groceries",
    completed: false,
    on_toggle: move || toggle(id),
    on_delete: move || delete(id),
}
```

**Prop type defaults:**

The `#[component]` macro generates a `Default` impl for the component struct. Known types get sensible defaults automatically:

| Type | Default |
|------|---------|
| `String` | `String::new()` (empty) |
| `bool` | `false` |
| `Option<T>` | `None` |
| `Vec<T>` | `Vec::new()` |
| Numeric types | `0` / `0.0` |
| `Callback` | No-op (does nothing) |
| `InputCallback` | No-op (does nothing) |

Other types (e.g., custom structs) fall back to `Default::default()`, so they must implement `Default`.

## Icon System

Rinch provides a type-safe icon system built on [Tabler Icons](https://tabler.io/icons). Instead of passing arbitrary HTML strings, components accept `TablerIcon` enum values for discoverability and consistency.

`TablerIcon` lives in the `rinch-tabler-icons` crate and is **not** re-exported through the `rinch` prelude, so add it alongside `rinch`:

```toml
rinch-tabler-icons = { workspace = true }
```

> Older snippets may show a curated `Icon` enum from `rinch-core`. That enum no longer exists — every icon prop takes `Option<TablerIcon>`. A few names were spelled differently: `Icon::CheckCircle` is now `TablerIcon::CircleCheck`, and `Icon::XCircle` is `TablerIcon::CircleX`.

### Basic Usage

```rust
use rinch::prelude::*;
use rinch_tabler_icons::TablerIcon;

#[component]
fn app() -> NodeHandle {
    rsx! {
        Alert { icon: TablerIcon::InfoCircle, color: "blue", title: "Information",
            "This is an informational message."
        }

        Alert { icon: TablerIcon::CircleCheck, color: "green", title: "Success",
            "Operation completed successfully."
        }

        Alert { icon: TablerIcon::AlertTriangle, color: "yellow", title: "Warning",
            "Please review this carefully."
        }

        Alert { icon: TablerIcon::CircleX, color: "red", title: "Error",
            "Something went wrong."
        }
    }
}
```

### Available Icons

All 4,983 Tabler icons are available as enum variants — browse them at
[tabler.io/icons](https://tabler.io/icons), or iterate `ALL_ICONS` (with
`ICON_COUNT`) to build a picker. A sample of the common ones:

| Category | Icons |
|----------|-------|
| **Navigation** | `Home`, `ChevronUp`, `ChevronDown`, `ArrowLeft`, `ArrowRight`, `Menu2` |
| **Actions** | `Plus`, `Minus`, `X`, `Check`, `Search`, `Settings`, `Edit`, `Trash` |
| **Status/Alerts** | `InfoCircle`, `CircleCheck`, `AlertCircle`, `AlertTriangle`, `CircleX` |
| **Content** | `User`, `Mail`, `Phone`, `File`, `Folder`, `Photo`, `Copy` |
| **UI** | `Bell`, `Message`, `Send`, `Share`, `Download`, `Upload` |

Every icon has an Outline form; roughly a thousand also have real Filled
artwork, and asking for `TablerIconStyle::Filled` on one that doesn't falls back
to its outline paths. See [Icon System in CLAUDE.md](https://github.com/joeleaver/rinch/blob/main/CLAUDE.md#icon-system) for rendering icons standalone with `render_tabler_icon`.

### Components with Icon Support

Every prop below is `Option<TablerIcon>`; `rsx!` supplies the `Some(...)`, so
write `icon: TablerIcon::Check`, not `icon: Some(TablerIcon::Check)`.

| Component | Icon Props |
|--------|-----------|
| `ActionIcon` | `icon` |
| `Alert` | `icon` |
| `Notification` | `icon` |
| `AccordionControl` | `icon` |
| `Blockquote` | `icon` |
| `List` | `icon` |
| `ListItem` | `icon` |
| `Stepper` | `completed_icon`, `progress_icon` |
| `StepperStep` | `icon`, `completed_icon`, `progress_icon` |
| `NavLink` | `left_section`, `right_section` |
| `DropdownMenuItem` | `left_section`, `right_section` |
| `Tab` | `left_section`, `right_section` |

`Tree` takes its icons through data instead: `TreeNodeData::icon`, set with the
`with_icon(TablerIcon)` builder.

### Examples

```rust
// NavLink with icons
NavLink {
    label: "Dashboard",
    left_section: TablerIcon::Settings,
    active: true
}

// Tab with icon
Tabs {
    TabsList {
        Tab { value: "home", left_section: TablerIcon::User, "Profile" }
        Tab { value: "settings", left_section: TablerIcon::Settings, "Settings" }
    }
}

// Dropdown menu items with icons
DropdownMenu {
    DropdownMenuTarget {
        Button { "Actions" }
    }
    DropdownMenuDropdown {
        DropdownMenuItem { left_section: TablerIcon::Edit, "Edit" }
        DropdownMenuItem { left_section: TablerIcon::Trash, color: "red", "Delete" }
    }
}

// Blockquote with quote icon
Blockquote { icon: TablerIcon::Quote, cite: "— Unknown",
    "The best code is no code at all."
}

// Stepper with custom icons
Stepper { active: 1, completed_icon: TablerIcon::CircleCheck,
    StepperStep { label: "Account" }
    StepperStep { label: "Verify" }
    StepperStep { label: "Complete" }
}
```

### Benefits of the Icon System

1. **Type Safety**: Compiler catches typos and invalid icon names
2. **Discoverability**: IDE autocomplete shows all available icons
3. **Consistency**: All icons use the same SVG style and sizing
4. **Performance**: Icons render as structured nodes, enabling differential updates instead of full re-renders

## Reactive Component Props

Rinch supports two mechanisms for making component props reactive:

### 1. Reactive Closures on Any Prop (Re-renders Component)

Pass a closure `{|| expr}` to any component prop (like `variant`, `color`, `size`, `disabled`). When signals inside the closure change, the entire component re-renders with the new prop values:

```rust
let active = Signal::new(false);

rsx! {
    Button {
        variant: {|| if active.get() { "filled" } else { "light" }},
        color: {|| if active.get() { "green" } else { "gray" }},
        onclick: move || active.update(|v| *v = !*v),
        "Toggle me"
    }
}
```

This works on **all component props** — the macro detects closures and wraps the component in a reactive scope automatically.

### 2. `_fn` Props (Surgical DOM Updates)

For components that support `_fn` props, these provide more efficient updates that only touch the specific DOM node, without re-rendering the entire component. The `rsx!` macro auto-wraps `_fn` props — just pass a closure directly:

```rust
let is_checked = Signal::new(false);
let input_text = Signal::new(String::new());

rsx! {
    // Auto-wrapped: just pass a closure, no Rc::new() needed
    Checkbox {
        label: "Enable feature",
        checked_fn: move || is_checked.get(),
        onchange: move || is_checked.update(|v| *v = !*v),
    }

    TextInput {
        placeholder: "Type here...",
        value_fn: move || input_text.get(),
        oninput: move |value: String| input_text.set(value),
    }
}
```

### Reactive Props Summary

| Prop Type | Accepts `{|| ...}` | Update Strategy |
|---|---|---|
| HTML element attributes | Yes | Surgical DOM update |
| Component `style:`/`class:` | Yes | Surgical DOM update |
| Component `_fn` props | Yes (auto-wrapped) | Surgical DOM update |
| Component props (all others) | Yes | Full component re-render |

### Components with `_fn` Props

| Component | Reactive Prop | Purpose |
|--------|---------------|---------|
| `Checkbox` | `checked_fn` | Toggle checked class reactively |
| `Switch` | `checked_fn` | Toggle checked class reactively |
| `Radio` | `checked_fn` | Toggle checked class reactively |
| `TextInput` | `value_fn` | Reactive value binding |
| `NavLink` | `active_fn` | Toggle active class reactively |
| `Progress` | `value_fn` | Update progress bar width reactively |
| `Modal` | `opened_fn` | Show/hide modal reactively |
| `Drawer` | `opened_fn` | Show/hide drawer reactively |
| `Notification` | `opened_fn` | Show/hide notification reactively |

### Controlled Input Pattern

For controlled inputs, use `value_fn` + `oninput` together. `value_fn` keeps the DOM in sync with your signal; `oninput` updates the signal from user input. Without `value_fn`, programmatic `signal.set("")` won't clear the input visually:

```rust
let text = Signal::new(String::new());

rsx! {
    TextInput {
        value_fn: move || text.get(),
        oninput: move |value: String| text.set(value),
        onsubmit: move || {
            println!("Submitted: {}", text.get());
            text.set(String::new());  // Clears the input thanks to value_fn
        },
    }
}
```

**Writing to the field the user is typing in.** A `value_fn` write (or any
`set_attribute("value")`) that lands on the *focused* field is adopted by the
field on both backends (issue #238): it becomes the text the next keystroke
edits, the caret keeps its logical position through the rewrite (a kept
prefix or suffix keeps the caret with it, a same-length rewrite such as a case
change leaves it in place, a resized rewrite puts it after the new text), a
selection survives, and the write is deferred while an IME composition is in
flight (applied when the composition ends). A programmatic write never
commits `onchange` by itself — only a user edit in the gesture does. So a
normalizing or rejecting `oninput` is safe to write back on every keystroke:

```rust
let digits = Signal::new(String::new());

rsx! {
    TextInput {
        value_fn: move || digits.get(),
        // Digits only: the rejected character never sticks, and the caret
        // stays put — on desktop as well as on the web.
        oninput: move |value: String| {
            digits.set(value.chars().filter(char::is_ascii_digit).collect())
        },
    }
}
```

Text inputs (`TextInput`, `Textarea`, `PasswordInput`, `NumberInput`) also
take an `onchange` prop — the commit boundary (issue #226): it fires once with
the final value when the typed gesture ends (focus leaves the control after a
modification, or Enter — blur-only for `Textarea`), and only if the user
modified it during the gesture — a programmatic write to a still-untouched
field moves the reference point with it, matching the browser's dirty flag. Keep `oninput` for the controlled-input signal
sync; use `onchange` for validate-on-commit and autosave-on-leave.

## Customization

Components automatically respond to theme settings:

```rust
let theme = ThemeProviderProps {
    primary_color: Some("violet".into()),  // All primary-colored components use violet
    default_radius: Some("lg".into()),     // Larger border radius everywhere
    dark_mode: true,                       // Dark theme colors
    ..Default::default()
};

// Components automatically adapt to the theme
run_with_theme("My App", 800, 600, app, theme);
```
