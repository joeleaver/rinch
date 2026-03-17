//! NavLink component.
//!
//! Navigation link with active state and nested support.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive active state without re-rendering, use the `active_fn` prop:
//!
//! ```ignore
//! let current_page = Signal::new("home".to_string());
//!
//! rsx! {
//!     NavLink {
//!         active_fn: Some(Rc::new(move || current_page.get() == "home")),
//!         onclick: move || current_page.set("home".to_string()),
//!         label: Some("Home".to_string())
//!     }
//! }
//! ```

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::rc::Rc;

/// Reactive callback type for boolean state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// NavLink variant style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavLinkVariant {
    #[default]
    Default,
    Light,
    Filled,
    Subtle,
}

impl NavLinkVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            NavLinkVariant::Default => "rinch-navlink--default",
            NavLinkVariant::Light => "rinch-navlink--light",
            NavLinkVariant::Filled => "rinch-navlink--filled",
            NavLinkVariant::Subtle => "rinch-navlink--subtle",
        }
    }
}

impl std::str::FromStr for NavLinkVariant {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(NavLinkVariant::Default),
            "light" => Ok(NavLinkVariant::Light),
            "filled" => Ok(NavLinkVariant::Filled),
            "subtle" => Ok(NavLinkVariant::Subtle),
            _ => Err(()),
        }
    }
}

/// Navigation link component with active state.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     NavLink {
///         label: "Dashboard",
///         href: "/dashboard",
///         active: true,
///         left_section: Icon::Settings,
///     }
///     NavLink {
///         label: "Settings",
///         href: "/settings",
///         description: "App configuration",
///     }
/// }
/// ```
#[derive(Default)]
pub struct NavLink {
    /// Link label text.
    pub label: String,
    /// Link description (secondary text).
    pub description: String,
    /// Link href.
    pub href: String,
    /// Whether link is active (static, for initial render or non-reactive use).
    pub active: bool,
    /// Reactive active getter - use this for fine-grained updates.
    /// When provided, the navlink class updates automatically when the signal changes.
    pub active_fn: Option<ReactiveBool>,
    /// Visual variant (default, light, filled, subtle).
    pub variant: String,
    /// Color when active.
    pub color: String,
    /// Left section icon.
    pub left_section: Option<TablerIcon>,
    /// Right section icon.
    pub right_section: Option<TablerIcon>,
    /// Whether link is disabled.
    pub disabled: bool,
    /// Whether this navlink has nested children (makes it expandable).
    pub children_offset: String,
    /// Whether children are visible (expanded).
    pub opened: bool,
    /// Default opened state (uncontrolled).
    pub default_opened: bool,
    /// Disable text wrapping.
    pub no_wrap: bool,
    /// Click handler.
    pub onclick: Option<rinch_core::Callback>,
}

impl std::fmt::Debug for NavLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavLink")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("href", &self.href)
            .field("active", &self.active)
            .field("active_fn", &self.active_fn.as_ref().map(|_| "<reactive>"))
            .field("variant", &self.variant)
            .field("color", &self.color)
            .field("left_section", &self.left_section)
            .field("right_section", &self.right_section)
            .field("disabled", &self.disabled)
            .field("children_offset", &self.children_offset)
            .field("opened", &self.opened)
            .field("default_opened", &self.default_opened)
            .field("no_wrap", &self.no_wrap)
            .field("onclick", &self.onclick.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl NavLink {
    /// Generate the base CSS class string (without active state).
    fn base_class_string(&self) -> String {
        let mut classes = vec!["rinch-navlink"];

        if !self.variant.is_empty() {
            if let Ok(variant) = self.variant.parse::<NavLinkVariant>() {
                classes.push(variant.class_name());
            }
        } else {
            classes.push(NavLinkVariant::Default.class_name());
        }

        if self.disabled {
            classes.push("rinch-navlink--disabled");
        }

        if self.no_wrap {
            classes.push("rinch-navlink--no-wrap");
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for this navlink (static version).
    pub fn class_string(&self) -> String {
        let mut class = self.base_class_string();
        if self.active {
            class.push_str(" rinch-navlink--active");
        }
        class
    }
}

impl Component for NavLink {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let base_class = self.base_class_string();

        // Determine initial active state
        let initial_active = if let Some(ref active_fn) = self.active_fn {
            active_fn()
        } else {
            self.active
        };

        // Check if there are children for expandable behavior
        let has_children = !children.is_empty();

        // Build style parts
        let mut style_parts = Vec::new();
        if !self.color.is_empty() {
            let c = &self.color;
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                style_parts.push(format!("--rinch-navlink-color: {}", c));
            } else {
                style_parts.push(format!("--rinch-navlink-color: var(--rinch-color-{}-6)", c));
            }
        }
        if !self.children_offset.is_empty() {
            style_parts.push(format!(
                "--rinch-navlink-children-offset: {}",
                self.children_offset
            ));
        }

        // Create wrapper element
        let wrapper = rinch_macros::rsx! { div { class: "rinch-navlink__wrapper" } };
        if !style_parts.is_empty() {
            wrapper.set_attribute("style", &style_parts.join("; "));
        }

        // Build the inner content
        let inner = rinch_macros::rsx! { span { class: "rinch-navlink__inner" } };

        // Left section icon
        if let Some(icon) = self.left_section {
            let left_span = rinch_macros::rsx! { span { class: "rinch-navlink__left" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            left_span.append_child(&icon_el);
            inner.append_child(&left_span);
        }

        // Body with label and optional description
        if !self.description.is_empty() {
            let body = rinch_macros::rsx! { div { class: "rinch-navlink__body" } };

            if !self.label.is_empty() {
                let label_span = rinch_macros::rsx! { span { class: "rinch-navlink__label", {self.label.clone()} } };
                body.append_child(&label_span);
            }

            let desc = &self.description;
            let desc_span =
                rinch_macros::rsx! { span { class: "rinch-navlink__description", {desc} } };
            body.append_child(&desc_span);

            inner.append_child(&body);
        } else if !self.label.is_empty() {
            let label_span =
                rinch_macros::rsx! { span { class: "rinch-navlink__label", {self.label.clone()} } };
            inner.append_child(&label_span);
        }

        // Right section - chevron for expandable items or custom icon
        if has_children {
            let chevron_class = if self.opened || self.default_opened {
                "rinch-navlink__chevron rinch-navlink__chevron--opened"
            } else {
                "rinch-navlink__chevron"
            };
            let right_span = rinch_macros::rsx! { span { class: "rinch-navlink__right" } };

            let svg = rinch_macros::rsx! {
                svg {
                    viewBox: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor"
                }
            };
            svg.set_attribute("class", chevron_class);
            svg.set_attribute("stroke-width", "2");

            let polyline = rinch_macros::rsx! { polyline { points: "9 18 15 12 9 6" } };
            svg.append_child(&polyline);

            right_span.append_child(&svg);
            inner.append_child(&right_span);
        } else if let Some(icon) = self.right_section {
            let right_span = rinch_macros::rsx! { span { class: "rinch-navlink__right" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            right_span.append_child(&icon_el);
            inner.append_child(&right_span);
        }

        // Build the link or button element
        let link_or_button = if !self.href.is_empty() {
            let href = &self.href;
            let a = rinch_macros::rsx! { a { class: "rinch-navlink" } };
            a.set_attribute("href", href);

            // Set initial class
            let class = if initial_active {
                format!("{} rinch-navlink--active", base_class)
            } else {
                base_class.clone()
            };
            let final_class = if self.disabled {
                format!("{} disabled", class)
            } else {
                class
            };
            a.set_attribute("class", &final_class);

            // Set up reactive effect for active state if active_fn is provided
            if let Some(ref active_fn) = self.active_fn {
                let active_fn = active_fn.clone();
                let base_class = base_class.clone();
                let disabled = self.disabled;
                let a_handle = a.clone();
                __scope.create_effect(move || {
                    let is_active = active_fn();
                    let class = if is_active {
                        format!("{} rinch-navlink--active", base_class)
                    } else {
                        base_class.clone()
                    };
                    let final_class = if disabled {
                        format!("{} disabled", class)
                    } else {
                        class
                    };
                    a_handle.set_attribute("class", &final_class);
                });
            }

            // Click handler
            if let Some(ref cb) = self.onclick {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                a.set_attribute("data-rid", &handler_id.0.to_string());
            }

            a.append_child(&inner);
            a
        } else {
            let btn = rinch_macros::rsx! { button { class: "rinch-navlink" } };

            // Set initial class
            let class = if initial_active {
                format!("{} rinch-navlink--active", base_class)
            } else {
                base_class.clone()
            };
            btn.set_attribute("class", &class);

            // Set up reactive effect for active state if active_fn is provided
            if let Some(ref active_fn) = self.active_fn {
                let active_fn = active_fn.clone();
                let base_class = base_class.clone();
                let btn_handle = btn.clone();
                __scope.create_effect(move || {
                    let is_active = active_fn();
                    let class = if is_active {
                        format!("{} rinch-navlink--active", base_class)
                    } else {
                        base_class.clone()
                    };
                    btn_handle.set_attribute("class", &class);
                });
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

            btn.append_child(&inner);
            btn
        };

        wrapper.append_child(&link_or_button);

        // Render nested children
        if has_children {
            let collapsed_class = if self.opened || self.default_opened {
                "rinch-navlink__children"
            } else {
                "rinch-navlink__children rinch-navlink__children--collapsed"
            };
            let children_container =
                rinch_macros::rsx! { div { class: "rinch-navlink__children" } };
            children_container.set_attribute("class", collapsed_class);

            for child in children {
                children_container.append_child(child);
            }

            wrapper.append_child(&children_container);
        }

        wrapper
    }
}
