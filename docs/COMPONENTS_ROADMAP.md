# Rinch Components Roadmap

This document tracks the progress of porting Mantine components to Rinch.

## Status Legend
- ✅ Implemented
- 🚧 In Progress
- ⬚ Not Started

---

## Implemented (52 components)

### Layout
- ✅ Stack - Vertical flex container
- ✅ Group - Horizontal flex container
- ✅ SimpleGrid - Responsive grid layout
- ✅ Container - Centered max-width container
- ✅ Center - Center content horizontally/vertically
- ✅ Space - Adds spacing between elements

### Buttons
- ✅ Button - Clickable button with variants
- ✅ ActionIcon - Icon-only button
- ✅ CloseButton - Dismiss/close button

### Inputs
- ✅ TextInput - Single-line text input
- ✅ Textarea - Multi-line text input
- ✅ Checkbox - Checkbox input
- ✅ Switch - Toggle switch
- ✅ Select - Dropdown select
- ✅ Radio - Radio input + RadioGroup
- ✅ NumberInput - Numeric input with controls
- ✅ PasswordInput - Password field with toggle

### Typography
- ✅ Text - Text display with styling
- ✅ Title - Heading text (h1-h6)
- ✅ Code - Inline code
- ✅ Kbd - Keyboard key
- ✅ Anchor - Styled link

### Feedback
- ✅ Alert - User feedback messages
- ✅ Loader - Loading spinner/indicator
- ✅ Progress - Progress bar
- ✅ Skeleton - Loading placeholder

### Data Display
- ✅ Avatar - User avatar with image/initials fallback
- ✅ Badge - Status indicator
- ✅ Card - Container with sections
- ✅ Paper - Card container with shadow
- ✅ Divider - Horizontal/vertical separator
- ✅ Fieldset - Grouped form fields
- ✅ Image - Responsive image with fallback
- ✅ List - Styled lists
- ✅ Blockquote - Styled quotation
- ✅ Slider - Range input slider

### Text Formatting
- ✅ Mark - Highlighted text
- ✅ Highlight - Search text highlighting

### Overlays
- ✅ Tooltip - CSS-only hover tooltip
- ✅ Modal - Dialog overlay
- ✅ Drawer - Slide-out panel
- ✅ Notification - Toast notification
- ✅ Popover - Positioned popup content
- ✅ DropdownMenu - Dropdown menu
- ✅ HoverCard - Card shown on hover
- ✅ LoadingOverlay - Overlay with loader

### Navigation
- ✅ Tabs - Tab navigation
- ✅ Accordion - Collapsible content sections
- ✅ Breadcrumbs - Navigation trail
- ✅ Pagination - Page navigation
- ✅ NavLink - Navigation link with active state
- ✅ Stepper - Step-by-step progress indicator

---

## Tier 1 - Core Essentials (High Impact) - COMPLETE

| Component | Status | Description |
|-----------|--------|-------------|
| Alert | ✅ | User feedback messages (info, success, warning, error) |
| Loader | ✅ | Loading spinner/indicator (oval, bars, dots) |
| Progress | ✅ | Progress bar (with striped/animated variants) |
| ActionIcon | ✅ | Icon-only button |
| CloseButton | ✅ | Dismiss/close button |
| Radio | ✅ | Radio input (single selection from group) + RadioGroup |
| NumberInput | ✅ | Numeric input with increment/decrement |
| PasswordInput | ✅ | Password field with visibility toggle |

---

## Tier 2 - Enhanced UX - COMPLETE

| Component | Status | Description |
|-----------|--------|-------------|
| Avatar | ✅ | User avatar with image/initials fallback |
| Card | ✅ | Extended Paper with sections |
| Tooltip | ✅ | Hover hints |
| Skeleton | ✅ | Loading placeholder |
| Image | ✅ | Responsive image with fallback |
| List | ✅ | Styled ordered/unordered lists |
| Slider | ✅ | Range input slider |
| Blockquote | ✅ | Styled quotation |
| Mark | ✅ | Highlighted text |
| Highlight | ✅ | Highlight matching text in string |

---

## Tier 3 - Navigation & Organization - COMPLETE

| Component | Status | Description |
|-----------|--------|-------------|
| Tabs | ✅ | Tab navigation |
| Accordion | ✅ | Collapsible content sections |
| Breadcrumbs | ✅ | Navigation trail |
| Pagination | ✅ | Page navigation |
| NavLink | ✅ | Navigation link with active state |
| Stepper | ✅ | Step-by-step progress indicator |

---

## Tier 4 - Overlays - COMPLETE

| Component | Status | Description |
|-----------|--------|-------------|
| Modal | ✅ | Dialog overlay with backdrop |
| Drawer | ✅ | Slide-out panel (left/right/top/bottom) |
| Popover | ✅ | Positioned popup content |
| DropdownMenu | ✅ | Dropdown menu with items |
| Notification | ✅ | Toast notification with positions |
| HoverCard | ✅ | Card shown on hover |
| LoadingOverlay | ✅ | Overlay with loader |

---

## Tier 5 - Specialized

| Component | Status | Description |
|-----------|--------|-------------|
| Table | ⬚ | Data table with styling |
| Timeline | ⬚ | Event timeline |
| Rating | ⬚ | Star rating input |
| ColorInput | ⬚ | Color picker input |
| ColorSwatch | ⬚ | Color display swatch |
| SegmentedControl | ⬚ | Button group selector |
| Chip | ⬚ | Selectable chip/tag |
| ThemeIcon | ⬚ | Icon in colored circle |
| Indicator | ⬚ | Badge indicator on element |
| Spoiler | ⬚ | Show more/less content |
| RingProgress | ⬚ | Circular progress indicator |

---

## Notes

### Implementation Guidelines
1. Each component should implement the `Component` trait from `rinch-core`
2. Use CSS classes following the pattern `rinch-{component}--{modifier}`
3. Support theme CSS variables for colors, spacing, radius
4. Add styles to `styles.rs` via `generate_all_component_styles()`
5. Export from `lib.rs`
6. Update documentation in `docs/src/guide/components.md`

### Overlay Components
Tier 4 overlay components are implemented with:
- ✅ Portal rendering via `Element::Portal` (rendering outside normal DOM hierarchy)
- CSS-only hover states for HoverCard, Popover, DropdownMenu
- Focus trapping support (via `trap_focus` prop on Modal/Drawer)
- Click-outside detection (via overlay click handlers)
- Escape key support (via `close_on_escape` prop)
