//! DropdownMenu widget.
//!
//! A dropdown menu component (distinct from native AppMenu/Menu).

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Dropdown menu position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuPosition {
    #[default]
    Bottom,
    BottomStart,
    BottomEnd,
    Top,
    TopStart,
    TopEnd,
    Left,
    LeftStart,
    LeftEnd,
    Right,
    RightStart,
    RightEnd,
}

impl DropdownMenuPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            DropdownMenuPosition::Bottom => "rinch-dropdown-menu--bottom",
            DropdownMenuPosition::BottomStart => "rinch-dropdown-menu--bottom-start",
            DropdownMenuPosition::BottomEnd => "rinch-dropdown-menu--bottom-end",
            DropdownMenuPosition::Top => "rinch-dropdown-menu--top",
            DropdownMenuPosition::TopStart => "rinch-dropdown-menu--top-start",
            DropdownMenuPosition::TopEnd => "rinch-dropdown-menu--top-end",
            DropdownMenuPosition::Left => "rinch-dropdown-menu--left",
            DropdownMenuPosition::LeftStart => "rinch-dropdown-menu--left-start",
            DropdownMenuPosition::LeftEnd => "rinch-dropdown-menu--left-end",
            DropdownMenuPosition::Right => "rinch-dropdown-menu--right",
            DropdownMenuPosition::RightStart => "rinch-dropdown-menu--right-start",
            DropdownMenuPosition::RightEnd => "rinch-dropdown-menu--right-end",
        }
    }
}

impl std::str::FromStr for DropdownMenuPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "bottom" => Ok(DropdownMenuPosition::Bottom),
            "bottom_start" | "bottomstart" => Ok(DropdownMenuPosition::BottomStart),
            "bottom_end" | "bottomend" => Ok(DropdownMenuPosition::BottomEnd),
            "top" => Ok(DropdownMenuPosition::Top),
            "top_start" | "topstart" => Ok(DropdownMenuPosition::TopStart),
            "top_end" | "topend" => Ok(DropdownMenuPosition::TopEnd),
            "left" => Ok(DropdownMenuPosition::Left),
            "left_start" | "leftstart" => Ok(DropdownMenuPosition::LeftStart),
            "left_end" | "leftend" => Ok(DropdownMenuPosition::LeftEnd),
            "right" => Ok(DropdownMenuPosition::Right),
            "right_start" | "rightstart" => Ok(DropdownMenuPosition::RightStart),
            "right_end" | "rightend" => Ok(DropdownMenuPosition::RightEnd),
            _ => Err(()),
        }
    }
}

/// A dropdown menu component.
///
/// # Example
///
/// ```ignore
/// let show_menu = use_signal(|| false);
///
/// rsx! {
///     DropdownMenu { opened: show_menu.get(),
///         DropdownMenuTarget {
///             Button { onclick: move || show_menu.update(|v| *v = !*v),
///                 "Options"
///             }
///         }
///         DropdownMenuDropdown {
///             DropdownMenuItem { onclick: || println!("Edit"), "Edit" }
///             DropdownMenuItem { onclick: || println!("Delete"), "Delete" }
///             DropdownMenuDivider {}
///             DropdownMenuItem { color: "red", "Delete All" }
///         }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct DropdownMenu {
    /// Whether the menu is open.
    pub opened: bool,
    /// Position relative to target.
    pub position: String,
    /// Offset from target.
    pub offset: Option<i32>,
    /// Border radius.
    pub radius: String,
    /// Shadow size.
    pub shadow: String,
    /// Whether clicking outside closes menu.
    pub close_on_click_outside: bool,
    /// Whether clicking item closes menu.
    pub close_on_item_click: bool,
    /// Width of the dropdown.
    pub width: String,
    /// Z-index.
    pub z_index: Option<i32>,
}

impl Default for DropdownMenu {
    fn default() -> Self {
        Self {
            opened: false,
            position: String::new(),
            offset: None,
            radius: String::new(),
            shadow: String::new(),
            close_on_click_outside: true,
            close_on_item_click: true,
            width: String::new(),
            z_index: None,
        }
    }
}

impl DropdownMenu {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-dropdown-menu"];

        if !self.position.is_empty() {
            if let Ok(pos) = self.position.parse::<DropdownMenuPosition>() {
                classes.push(pos.class_name());
            }
        } else {
            classes.push(DropdownMenuPosition::Bottom.class_name());
        }

        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-dropdown-menu--radius-xs"),
                "sm" => classes.push("rinch-dropdown-menu--radius-sm"),
                "md" => classes.push("rinch-dropdown-menu--radius-md"),
                "lg" => classes.push("rinch-dropdown-menu--radius-lg"),
                "xl" => classes.push("rinch-dropdown-menu--radius-xl"),
                _ => {}
            }
        }

        if !self.shadow.is_empty() {
            match self.shadow.as_str() {
                "xs" => classes.push("rinch-dropdown-menu--shadow-xs"),
                "sm" => classes.push("rinch-dropdown-menu--shadow-sm"),
                "md" => classes.push("rinch-dropdown-menu--shadow-md"),
                "lg" => classes.push("rinch-dropdown-menu--shadow-lg"),
                "xl" => classes.push("rinch-dropdown-menu--shadow-xl"),
                _ => {}
            }
        }

        if self.opened {
            classes.push("rinch-dropdown-menu--opened");
        }

        classes.join(" ")
    }
}

impl Widget for DropdownMenu {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(offset) = self.offset {
            style_parts.push(format!("--rinch-dropdown-menu-offset: {}px", offset));
        }
        if !self.width.is_empty() {
            style_parts.push(format!("--rinch-dropdown-menu-width: {}", self.width));
        }

        let root = rinch_macros::rsx! { div { class: "rinch-dropdown-menu" } };
        root.set_attribute("class", &self.class_string());

        if !style_parts.is_empty() {
            root.set_attribute("style", &style_parts.join("; "));
        }

        for child in children {
            root.append_child(child);
        }

        root
    }
}

/// Target element for the dropdown menu.
#[derive(Debug, Default)]
pub struct DropdownMenuTarget;

impl Widget for DropdownMenuTarget {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let target = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__target" } };

        for child in children {
            target.append_child(child);
        }

        target
    }
}

/// Dropdown content container.
#[derive(Debug, Default)]
pub struct DropdownMenuDropdown;

impl Widget for DropdownMenuDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__dropdown" } };

        for child in children {
            dropdown.append_child(child);
        }

        dropdown
    }
}

/// A menu item in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuItem {
    /// Left section icon.
    pub left_section: Option<TablerIcon>,
    /// Right section icon.
    pub right_section: Option<TablerIcon>,
    /// Color variant.
    pub color: String,
    /// Whether item is disabled.
    pub disabled: bool,
    /// Click callback.
    pub onclick: Option<rinch_core::WidgetCallback>,
}

impl Widget for DropdownMenuItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-dropdown-menu__item"];

        if self.disabled {
            classes.push("rinch-dropdown-menu__item--disabled");
        }

        let color_style = if self.color.is_empty() {
            None
        } else {
            let c = &self.color;
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                Some(format!("--rinch-dropdown-menu-item-color: {}", c))
            } else {
                Some(format!(
                    "--rinch-dropdown-menu-item-color: var(--rinch-color-{}-6)",
                    c
                ))
            }
        };

        let btn = rinch_macros::rsx! { button { class: "rinch-dropdown-menu__item" } };
        btn.set_attribute("class", &classes.join(" "));

        if let Some(ref style) = color_style {
            btn.set_attribute("style", style);
        }

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        // Click handler
        if let Some(ref cb) = self.onclick {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Left section
        if let Some(icon) = self.left_section {
            let left_span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-left" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            left_span.append_child(&icon_el);
            btn.append_child(&left_span);
        }

        // Label
        let label_span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-label" } };
        for child in children {
            label_span.append_child(child);
        }
        btn.append_child(&label_span);

        // Right section
        if let Some(icon) = self.right_section {
            let right_span =
                rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-right" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            right_span.append_child(&icon_el);
            btn.append_child(&right_span);
        }

        btn
    }
}

/// A label/header in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuLabel;

impl Widget for DropdownMenuLabel {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let label = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__label" } };

        for child in children {
            label.append_child(child);
        }

        label
    }
}

/// A divider in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuDivider;

impl Widget for DropdownMenuDivider {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        rinch_macros::rsx! { div { class: "rinch-dropdown-menu__divider" } }
    }
}
