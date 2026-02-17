//! Rinch Components - A UI component library for Rinch.
//!
//! Provides styled, themeable UI components that integrate with the Rinch theme system.
//! Inspired by [Mantine](https://mantine.dev/).
//!
//! This crate implements the `Component` trait from `rinch-core`, allowing these components
//! to be used seamlessly in RSX macros. Third-party component libraries can follow the same
//! pattern to create their own component sets (e.g., `rinch-bootstrap`, `rinch-material`).
//!
//! # Available Components
//!
//! ## Layout
//! - [`Stack`] - Vertical flex container with spacing
//! - [`Group`] - Horizontal flex container with spacing
//! - [`SimpleGrid`] - Responsive grid layout with equal-width columns
//! - [`Container`] - Centered max-width container
//! - [`Center`] - Center content horizontally/vertically
//! - [`Space`] - Empty space component
//!
//! ## Buttons
//! - [`Button`] - Clickable button with variants
//! - [`ActionIcon`] - Icon-only button
//! - [`CloseButton`] - Dismiss/close button
//!
//! ## Inputs
//! - [`TextInput`] - Single-line text input
//! - [`Textarea`] - Multi-line text input
//! - [`Checkbox`] - Checkbox input
//! - [`Switch`] - Toggle switch
//! - [`Select`] - Dropdown select
//! - [`Radio`] - Radio button + [`RadioGroup`]
//! - [`NumberInput`] - Numeric input with controls
//! - [`PasswordInput`] - Password field with visibility toggle
//!
//! ## Typography
//! - [`Text`] - Text display with size and color options
//! - [`Title`] - Heading text (h1-h6)
//! - [`Code`] - Inline code
//! - [`Kbd`] - Keyboard key
//! - [`Anchor`] - Styled link
//!
//! ## Feedback
//! - [`Alert`] - User feedback messages
//! - [`Loader`] - Loading spinner/indicator
//! - [`Progress`] - Progress bar
//! - [`Skeleton`] - Loading placeholder
//!
//! ## Data Display
//! - [`Avatar`] - User avatar with image or initials
//! - [`Badge`] - Small status indicator
//! - [`Card`] - Container with sections + [`CardSection`]
//! - [`Paper`] - Card-like container with shadow
//! - [`Divider`] - Horizontal/vertical separator
//! - [`Fieldset`] - Grouped form fields
//! - [`Image`] - Responsive image with fallback
//! - [`List`] - Styled list + [`ListItem`]
//! - [`Blockquote`] - Styled quotation
//! - [`Slider`] - Range input slider
//!
//! ## Text Formatting
//! - [`Mark`] - Highlighted text
//! - [`Highlight`] - Search text highlighting
//!
//! ## Overlays
//! - [`Tooltip`] - CSS-only hover tooltip
//! - [`Modal`] - Dialog overlay
//! - [`Drawer`] - Slide-out panel
//! - [`Notification`] - Toast notification
//! - [`Popover`] - Positioned popup + [`PopoverTarget`], [`PopoverDropdown`]
//! - [`DropdownMenu`] - Dropdown menu + [`DropdownMenuTarget`], [`DropdownMenuDropdown`], [`DropdownMenuItem`]
//! - [`HoverCard`] - Card on hover + [`HoverCardTarget`], [`HoverCardDropdown`]
//! - [`LoadingOverlay`] - Loading overlay for containers
//!
//! ## Navigation
//! - [`Tabs`] - Tab navigation + [`TabsList`], [`Tab`], [`TabsPanel`]
//! - [`Accordion`] - Collapsible sections + [`AccordionItem`], [`AccordionControl`], [`AccordionPanel`]
//! - [`Breadcrumbs`] - Navigation trail + [`BreadcrumbsItem`]
//! - [`Pagination`] - Page navigation
//! - [`NavLink`] - Navigation link with active state
//! - [`Stepper`] - Step progress + [`StepperStep`], [`StepperCompleted`]
//! - [`Tree`] - Hierarchical tree view + [`TreeNodeData`], [`use_tree`]
//!
//! # Example
//!
//! ```ignore
//! use rinch::prelude::*;
//! use rinch_components::*;
//!
//! fn app() -> Element {
//!     rsx! {
//!         ThemeProvider {
//!             primary_color: "cyan",
//!             Window { title: "Components Demo",
//!                 Paper { shadow: "sm", p: "md",
//!                     Stack { gap: "md",
//!                         Text { size: "lg", weight: "bold", "Welcome!" }
//!                         Group { gap: "sm",
//!                             Button { variant: "filled", "Save" }
//!                             Button { variant: "outline", "Cancel" }
//!                         }
//!                     }
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```

pub mod accordion;
pub mod action_icon;
pub mod alert;
pub mod anchor;
pub mod avatar;
pub mod badge;
pub mod blockquote;
pub mod borderless_window;
pub mod breadcrumbs;
pub mod button;
pub mod card;
pub mod center;
pub mod checkbox;
pub mod close_button;
pub mod code;
pub mod container;
pub mod divider;
pub mod drawer;
pub mod dropdown_menu;
pub mod fieldset;
pub mod floating_panel;
pub mod group;
pub mod highlight;
pub mod hover_card;
pub mod icons;
pub mod image;
pub mod kbd;
pub mod list;
pub mod loader;
pub mod loading_overlay;
pub mod mark;
pub mod modal;
pub mod navlink;
pub mod notification;
pub mod number_input;
pub mod pagination;
pub mod paper;
pub mod password_input;
pub mod popover;
pub mod progress;
pub mod radio;
pub mod select;
pub mod simple_grid;
pub mod skeleton;
pub mod slider;
pub mod space;
pub mod stack;
pub mod stepper;
pub mod styles;
pub mod switch;
pub mod tabs;
pub mod text;
pub mod text_input;
pub mod textarea;
pub mod title;
pub mod tooltip;
pub mod tree;

// Re-export component structs
pub use accordion::{Accordion, AccordionControl, AccordionItem, AccordionPanel};
pub use action_icon::ActionIcon;
pub use alert::Alert;
pub use anchor::Anchor;
pub use avatar::Avatar;
pub use badge::Badge;
pub use blockquote::Blockquote;
pub use borderless_window::{BorderlessWindow, SectionRenderer, WindowRadius};
pub use breadcrumbs::{Breadcrumbs, BreadcrumbsItem};
pub use button::Button;
pub use card::{Card, CardSection};
pub use center::Center;
pub use checkbox::Checkbox;
pub use close_button::CloseButton;
pub use code::Code;
pub use container::Container;
pub use divider::Divider;
pub use fieldset::Fieldset;
pub use floating_panel::FloatingPanel;
pub use group::Group;
pub use highlight::Highlight;
pub use image::Image;
pub use kbd::Kbd;
pub use list::{List, ListItem};
pub use loader::Loader;
pub use mark::Mark;
pub use navlink::NavLink;
pub use number_input::NumberInput;
pub use pagination::Pagination;
pub use paper::Paper;
pub use password_input::PasswordInput;
pub use progress::Progress;
pub use radio::{Radio, RadioGroup};
pub use select::Select;
pub use simple_grid::SimpleGrid;
pub use skeleton::Skeleton;
pub use slider::Slider;
pub use space::Space;
pub use stack::Stack;
pub use stepper::{Stepper, StepperCompleted, StepperStep};
pub use switch::Switch;
pub use tabs::{Tab, Tabs, TabsList, TabsPanel};
pub use text::Text;
pub use text_input::{ReactiveString, TextInput};
pub use textarea::Textarea;
pub use title::Title;
pub use tooltip::Tooltip;
pub use tree::{
    RenderTreeNode, RenderTreeNodePayload, Tree, TreeController, TreeNodeData, UseTreeOptions,
    UseTreeReturn, get_tree_expanded_state, use_tree,
};

// Tier 4: Overlays
pub use drawer::Drawer;
pub use dropdown_menu::{
    DropdownMenu, DropdownMenuDivider, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuLabel,
    DropdownMenuTarget,
};
pub use hover_card::{HoverCard, HoverCardDropdown, HoverCardTarget};
pub use loading_overlay::LoadingOverlay;
pub use modal::Modal;
pub use notification::Notification;
pub use popover::{Popover, PopoverDropdown, PopoverTarget};

// Re-export Component trait for convenience
pub use rinch_core::Component;

/// Generate all component CSS classes.
pub fn generate_component_css() -> String {
    styles::generate_all_component_styles()
}
